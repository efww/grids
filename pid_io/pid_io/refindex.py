"""Reference instrument index: read any company's layout, and score our extraction against it (v13.4).

Layouts differ project to project (sheet name, header row, column order, IO columns or one 'I/O Type'
column, merged two-row headers, Korean / English captions). So nothing is read by position:

1. every sheet, rows 1..40: each row is scored by how many cells look like a known caption
   (synonyms below); the best (sheet, row) is the header. A second header row right below
   (sub captions under a merged group, e.g. 'Train 고려 Total' over AI / AO / DI ...) is merged in.
2. each field (tag, description, drawing no, title, system, qty, train qty, IO columns / IO type)
   gets the column whose caption matches best. The result is shown to the user as a table
   ('어떤 열을 무엇으로 읽었나') and can be overridden per project (meta['ref_map']).
3. rows: tag present, not deleted (DEL / DELETED / 삭제 / VOID anywhere in the first cells or a status column).

Scoring (what the screen shows):
  - 태그 재현율: reference tags (placeholders like XXXX excluded) that we also produced
  - 기능코드 개수 일치: per function code, min(ref, ours) / ref over the whole document
    (works even when the reference has XXXX tags)
  - IO 점수 일치: per IO type (AI/AO/DI/DO), min(ref, ours) / ref
  - 초과: tags / counts we have that the reference does not (could be new in this revision)
"""
from __future__ import annotations

import re
from collections import Counter
from pathlib import Path

IO_KEYS = ["AI", "AO", "DI", "DO", "DI(MCC)", "DO(MCC)", "DO-SOV"]

# field -> caption synonyms (normalised: lower case, no spaces / dots / newlines / underscores)
SYN = {
    "tag": ["tagno", "tagnumber", "tag", "instrumenttag", "instrumenttagno", "instrtag", "instrumentno", "instno",
            "태그", "태그번호", "태그no", "계기번호", "계기태그"],
    "desc": ["instrumentdescription", "description", "instrumenttype", "type", "service", "servicedescription",
             "instrumentname", "설명", "계기명", "계기종류", "서비스"],
    "dwg": ["dwgno", "drawingno", "pidno", "p&idno", "p&iddwgno", "piddwgno", "drawing", "dwg", "도면번호", "도면no"],
    "title": ["p&idtitle", "pidtitle", "drawingtitle", "dwgtitle", "title", "도면명", "도면제목"],
    "system": ["to(system)", "tosystem", "system", "controlsystem", "iosystem", "dcs/esd", "시스템", "제어시스템"],
    "qty": ["totalqty", "totalq'ty", "qty", "q'ty", "quantity", "수량"],
    "train": ["train고려qty", "train고려q'ty", "trainqty", "trainq'ty", "noftrain", "트레인수량"],
    "iotype": ["i/otype", "iotype", "signaltype(io)", "io", "i/o", "신호종류", "io종류"],
    "status": ["status", "remark", "remarks", "비고", "상태"],
    "func": ["func", "function", "기능", "기능코드"],
    "loop": ["loopno", "loop", "루프번호"],
}
IO_SYN = {"AI": ["ai", "analoginput", "ai(hart)", "aihart"], "AO": ["ao", "analogoutput"],
          "DI": ["di", "digitalinput"], "DO": ["do", "digitaloutput"],
          "DI(MCC)": ["di(mcc)", "dimcc"], "DO(MCC)": ["do(mcc)", "domcc"], "DO-SOV": ["do-sov", "dosov", "do(sov)", "sov"]}
LABEL = {"tag": "Tag No", "desc": "계기 설명", "dwg": "P&ID No", "title": "P&ID Title", "system": "System",
         "qty": "수량", "train": "트레인 고려 수량", "iotype": "IO 종류 (한 열)", "status": "상태/비고",
         "func": "기능코드", "loop": "루프 번호", **{k: f"IO {k}" for k in IO_KEYS}}
DELETED = re.compile(r"^\s*(DEL|DELETED|삭제|VOID|취소)", re.I)
PLACEHOLDER = re.compile(r"X{3,}|\?{2,}|TBD", re.I)


def _n(v) -> str:
    return re.sub(r"[\s._\-:]+", "", str(v or "")).lower().replace("no.", "no")


def _n_io(v) -> str:
    return re.sub(r"[\s._:]+", "", str(v or "")).lower()


def _match(cap: str, syns) -> float:
    if not cap:
        return 0.0
    if cap in syns:
        return 1.0
    for s in syns:
        if len(s) >= 4 and (cap.startswith(s) or s in cap):
            return 0.6
    return 0.0


def _row_score(cells) -> float:
    sc = 0.0
    for c in cells:
        cap = _n(c)
        best = max([_match(cap, s) for s in SYN.values()] + [1.0 if _n_io(c) in sum(IO_SYN.values(), []) else 0.0])
        sc += best
    return sc


def _cells(ws, r, width=80):
    return [c for c in next(ws.iter_rows(min_row=r, max_row=r, max_col=width, values_only=True), ())]


def _is_sub(cells) -> bool:
    """a second header row (AI / AO / DI / DO under a merged group): two or more distinct IO captions."""
    ios = {_n_io(c) for c in cells if c is not None and _n_io(c) in sum(IO_SYN.values(), [])}
    return len(ios) >= 2


def find_header(wb, sheet=None, row=None):
    """(sheet name, header row, captions per column). captions merge a sub-header row below."""
    best = (0.0, None, None)
    names = [sheet] if sheet else wb.sheetnames
    for n in names:
        ws = wb[n]
        rows = [row] if row else range(1, 41)
        for r in rows:
            sc = _row_score(_cells(ws, r))
            tag_here = any(_match(_n(c), SYN["tag"]) >= 1.0 for c in _cells(ws, r))
            sc += 3 if tag_here else 0
            if sc > best[0]:
                best = (sc, n, r)
    if not best[1] or best[0] < 3:
        raise ValueError("레퍼런스에서 머리글 행(Tag No 같은 열 이름이 있는 줄)을 찾지 못했습니다. "
                         "화면의 '열 지정'에서 시트·머리글 행을 직접 골라 주세요 (시트: " + ", ".join(wb.sheetnames[:12]) + ")")
    ws = wb[best[1]]
    top, sub = _cells(ws, best[2]), _cells(ws, best[2] + 1)
    if not _is_sub(sub):
        sub = []
    caps = []
    for i in range(max(len(top), len(sub))):
        a = str(top[i] or "").strip() if i < len(top) else ""
        b = str(sub[i] or "").strip() if i < len(sub) else ""
        caps.append(a if a and not (b and _n_io(b) in sum(IO_SYN.values(), [])) else (b or a))
    return best[1], best[2], caps


def auto_map(caps) -> dict:
    """field -> column index (0-based) by caption."""
    out, used = {}, set()
    for f in list(SYN) + IO_KEYS:
        syns = SYN.get(f) or IO_SYN[f]
        best = (0.0, None)
        for i, c in enumerate(caps):
            if i in used:
                continue
            s = (1.0 if _n_io(c) in syns else 0.0) if f in IO_KEYS else _match(_n(c), syns)
            if s > best[0]:
                best = (s, i)
        if best[1] is not None:
            out[f] = best[1]
            used.add(best[1])
    return out


def _num(v):
    if isinstance(v, (int, float)):
        return float(v)
    try:
        return float(str(v).strip())
    except (TypeError, ValueError):
        return None


def read(path: Path, override: dict | None = None):
    """rows (same shape as export.load_reference) + how the file was read."""
    from .directopen import load_workbook
    wb = load_workbook(path, data_only=True)
    ov = override or {}
    sheet, hrow, caps = find_header(wb, ov.get("sheet") or None, int(ov["row"]) if ov.get("row") else None)
    mp = auto_map(caps)
    for k, v in (ov.get("cols") or {}).items():
        if v in (None, "", -1, "-1"):
            mp.pop(k, None)
        else:
            mp[k] = int(v)
    if "tag" not in mp and not ("func" in mp and "loop" in mp):
        raise ValueError(f"'{sheet}' 시트 {hrow}행에서 태그 열을 찾지 못했습니다. '열 지정'에서 Tag No 열을 골라 주세요")
    ws = wb[sheet]
    io_cols = [k for k in IO_KEYS if k in mp]
    data_start = hrow + (2 if _is_sub(_cells(ws, hrow + 1)) else 1)
    g = lambda r, k: r[mp[k]] if k in mp and mp[k] < len(r) else None
    out, deleted = [], 0
    for r in ws.iter_rows(min_row=data_start, values_only=True):
        tag = g(r, "tag")
        if not tag and "func" in mp and "loop" in mp and g(r, "func") and g(r, "loop"):
            tag = f"{g(r, 'func')}-{g(r, 'loop')}"
        if not tag or not str(tag).strip():
            continue
        tag = str(tag).strip()
        if any(DELETED.match(str(c)) for c in r[:3] if c) or (g(r, "status") and DELETED.match(str(g(r, "status")))):
            deleted += 1
            continue
        func = str(g(r, "func") or "").strip() or re.split(r"[-\s]", re.sub(r"^\d+-", "", tag))[0]
        qty = _num(g(r, "qty")) or 1
        train = _num(g(r, "train")) or qty
        io_base, io, io_raw = {}, [], {}
        for k in io_cols:
            v = _num(g(r, k))
            if v:
                io.append(k)
                io_raw[k] = v                       # as written (plant total, trains included)
                io_base[k] = v * qty / train if train else v
        if not io_cols and g(r, "iotype"):
            t = str(g(r, "iotype")).upper().replace(" ", "")
            k = next((k for k in IO_KEYS if t == k or t.startswith(k)), None)
            if k:
                io, io_base, io_raw = [k], {k: 1.0}, {k: qty}
        out.append(dict(tag=tag, func=func.upper(), title=g(r, "title") or "", dwg=g(r, "dwg") or "",
                        system=g(r, "system") or "", io=io, io_base=io_base, io_raw=io_raw, qty=qty, desc=g(r, "desc") or ""))
    info = {"sheet": sheet, "row": hrow, "data_row": data_start, "sheets": wb.sheetnames, "captions": caps,
            "map": {k: {"col": i, "caption": caps[i] if i < len(caps) else "", "label": LABEL.get(k, k)} for k, i in mp.items()},
            "missing": [LABEL[k] for k in ("tag", "dwg", "system") if k not in mp] + ([] if io_cols or "iotype" in mp else ["IO 열"]),
            "rows": len(out), "deleted": deleted, "override": bool(ov)}
    return out, info


def _norm_tag(t) -> str:
    return re.sub(r"^\d{2}-", "", str(t).upper().replace(" ", ""))


def score(rows, ref: list) -> dict:
    """our index rows (pid_io Row) vs the reference rows - both as plant totals (trains / units included, spares out)."""
    allr = [r for r in rows if r.kind != "function" and r.origin != "스페어"]
    ours_t = {_norm_tag(r.tag) for r in allr}
    ref_t = {_norm_tag(x["tag"]) for x in ref if not PLACEHOLDER.search(str(x["tag"]))}
    hit_t = ref_t & ours_t
    rf, of = Counter(), Counter()
    for x in ref:
        rf[x["func"]] += int(x.get("qty") or 1)
    for f, t in {(r.func, r.tag) for r in allr}:
        of[f] += 1
    f_hit = sum(min(rf[k], of.get(k, 0)) for k in rf)
    mcc = {"DO-SOV": "DO", "DI(MCC)": "DI", "DO(MCC)": "DO"}
    rio, oio = Counter(), Counter()
    for x in ref:
        for k, v in (x.get("io_raw") or x["io_base"]).items():
            rio[mcc.get(k, k)] += v
    for r in allr:
        if r.io in ("AI", "AO", "DI", "DO"):
            oio[r.io] += 1
    io_hit = sum(min(rio[k], oio[k]) for k in rio)
    pct = lambda a, b: round(100 * a / b, 1) if b else None
    funcs = sorted(set(rf) | set(of), key=lambda k: -max(rf[k], of.get(k, 0)))
    res = {
        "tag": {"ref": len(ref_t), "hit": len(hit_t), "pct": pct(len(hit_t), len(ref_t)),
                "placeholder": sum(1 for x in ref if PLACEHOLDER.search(str(x["tag"]))),
                "missing": sorted(ref_t - ours_t)[:40], "extra": len(ours_t - ref_t)},
        "func": {"ref": sum(rf.values()), "ours": sum(of.values()), "hit": f_hit, "pct": pct(f_hit, sum(rf.values())),
                 "rows": [[k, rf.get(k, 0), of.get(k, 0)] for k in funcs][:40]},
        "io": {"ref": round(sum(rio.values())), "ours": sum(oio.values()), "hit": round(io_hit),
               "pct": pct(io_hit, sum(rio.values())),
               "rows": [[k, round(rio.get(k, 0), 1), oio.get(k, 0)] for k in ("AI", "AO", "DI", "DO")]},
    }
    # tag recall is meaningless when almost every reference tag is a placeholder (XXXX): leave it out of the total
    use_tag = res["tag"]["ref"] >= 0.3 * max(1, len(ref))
    res["tag"]["used"] = use_tag
    main = [v for v in ((res["tag"]["pct"] if use_tag else None), res["func"]["pct"], res["io"]["pct"]) if v is not None]
    res["total"] = round(sum(main) / len(main), 1) if main else None
    res["grade"] = (None if res["total"] is None else "A" if res["total"] >= 95 else "B" if res["total"] >= 85
                    else "C" if res["total"] >= 70 else "D")
    return res
