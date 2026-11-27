"""출력 폴더 규칙 (v13.2): LATEST / ISSUED / HISTORY + 지문.

  output/LATEST/   지금 기준 파일 - 종류마다 하나, 이름에 시각 없음, 덮어씀 ("최신이 뭐지?"가 안 생김)
  output/ISSUED/   '발행'한 판 - R0_날짜, R1_날짜 ... 고정 (읽기 전용), 발행할 때 LATEST를 그대로 복사
  output/HISTORY/  만들 때마다 시각이 붙은 원본 - 지우지 않음 (v13.2 이전 출력은 output/ 바로 아래 그대로 둠)

Every workbook gets a '_지문' sheet: Rev, made at, pid_io version, input files (name + SHA-256 12),
3D model hash, settings hash, decisions hash, and a fingerprint id over all of them.
Dropping any file into the app tells whether it is the LATEST, an ISSUED rev, or an older HISTORY file.
The LATEST index marks cells that changed since the last ISSUED rev (fill + comment) and lists them in '_발행대비변경'.
"""
from __future__ import annotations

import hashlib
import json
import shutil
import stat
import time
from pathlib import Path

LATEST, ISSUED, HISTORY = "LATEST", "ISSUED", "HISTORY"
KIND_NAME = {"index": "Instrument_Index", "cable": "Cable_Schedule", "basis": "산출근거", "pid_review": "P&ID_검토용",
             "json3d": "3D_데이터", "twin": "3D인계", "revcmp": "리비전대조"}


def _sha(p: Path, n=12) -> str:
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()[:n]


def _hobj(o) -> str:
    return hashlib.sha256(json.dumps(o, ensure_ascii=False, sort_keys=True, default=str).encode()).hexdigest()[:12]


def dirs(pdir: Path) -> dict:
    o = Path(pdir) / "output"
    d = {k: o / k for k in (LATEST, ISSUED, HISTORY)}
    for p in d.values():
        p.mkdir(parents=True, exist_ok=True)
    return d


def history_path(pdir: Path, kind: str, project: str, ext: str, stamp: str | None = None) -> Path:
    stamp = stamp or time.strftime("%Y%m%d_%H%M")
    base = KIND_NAME.get(kind, kind)
    h = dirs(pdir)[HISTORY]
    n = 1
    while True:                                          # never overwrite a history file (two exports in one minute)
        sfx = "" if n == 1 else f"-{n}"
        name = f"{base}_{project}_{stamp}{sfx}{ext}" if ext else f"{base}_{stamp}{sfx}"
        if not (h / name).exists():
            return h / name
        n += 1


def current_rev(pdir: Path) -> str:
    """'R1+' = changes after issued R1; 'R-' = never issued."""
    iss = issued_list(pdir)
    return (iss[-1]["rev"] + "+") if iss else "미발행"


def fingerprint(pdir: Path, meta: dict, values: dict, dec: dict, version: str) -> dict:
    files = {}
    for k, fn in (meta.get("files") or {}).items():
        p = Path(pdir) / "input" / str(fn)
        if fn and p.exists() and p.is_file():
            files[k] = {"file": fn, "sha": _sha(p)}
    fp = {"pid_io": version, "inputs": files, "settings": _hobj(values), "decisions": _hobj(dec)}
    fp["state"] = _hobj(fp)                                # same inputs + settings + decisions = same state
    fp["rev"] = current_rev(pdir)
    fp["at"] = time.strftime("%Y-%m-%d %H:%M")
    fp["id"] = _hobj([fp["state"], time.time()])           # this one file
    return fp


def stamp_xlsx(path: Path, fp: dict, kind: str):
    import openpyxl
    from openpyxl.styles import Font
    wb = openpyxl.load_workbook(path)
    if "_지문" in wb.sheetnames:
        del wb["_지문"]
    ws = wb.create_sheet("_지문")
    rows = [("이 파일의 지문 (pid_io)", ""), ("지문 ID", fp["id"]), ("상태 ID", fp["state"]), ("Rev", fp["rev"]), ("만든 시각", fp["at"]),
            ("종류", KIND_NAME.get(kind, kind)), ("pid_io", fp["pid_io"]), ("설정 해시", fp["settings"]), ("결정 해시", fp["decisions"])]
    rows += [(f"입력 {k}", f"{v['file']} · SHA-256 {v['sha']}") for k, v in fp["inputs"].items()]
    rows += [("", ""), ("확인 방법", "앱 '출력 파일' 화면에 이 파일을 끌어다 놓으면 최신(LATEST) / 발행본(R?) / 예전 파일인지 알려 줍니다.")]
    for r in rows:
        ws.append(list(r))
    ws["A1"].font = Font(bold=True)
    ws.column_dimensions["A"].width = 18
    ws.column_dimensions["B"].width = 80
    wb.save(path)


def _record(pdir: Path, entry: dict):
    f = dirs(pdir)[HISTORY] / "history.json"
    cur = json.loads(f.read_text(encoding="utf-8")) if f.exists() else []
    cur.append(entry)
    f.write_text(json.dumps(cur, ensure_ascii=False, indent=1), encoding="utf-8")


def publish(pdir: Path, src: Path, kind: str, project: str, fp: dict | None = None) -> Path:
    """HISTORY file / folder -> LATEST/<fixed name>; keeps LATEST/_latest.json and HISTORY/history.json."""
    d = dirs(pdir)
    src = Path(src)
    if src.is_dir():
        dst = d[LATEST] / KIND_NAME.get(kind, kind)
        if dst.exists():
            shutil.rmtree(dst)
        shutil.copytree(src, dst)
        sha = ""
    else:
        dst = d[LATEST] / f"{KIND_NAME.get(kind, kind)}_{project}{src.suffix}"
        shutil.copy2(src, dst)
        sha = _sha(dst, 64)
    lf = d[LATEST] / "_latest.json"
    cur = json.loads(lf.read_text(encoding="utf-8")) if lf.exists() else {}
    cur[kind] = {"file": dst.name, "from": f"HISTORY/{src.name}", "at": time.strftime("%Y-%m-%d %H:%M"),
                 "fid": (fp or {}).get("id", ""), "state": (fp or {}).get("state", ""), "rev": (fp or {}).get("rev", ""), "sha256": sha}
    lf.write_text(json.dumps(cur, ensure_ascii=False, indent=1), encoding="utf-8")
    _record(pdir, {"kind": kind, "file": f"HISTORY/{src.name}", "at": cur[kind]["at"], "fid": cur[kind]["fid"], "state": cur[kind]["state"],
                   "sha256": _sha(src, 64) if src.is_file() else ""})
    return dst


def latest(pdir: Path) -> dict:
    lf = dirs(pdir)[LATEST] / "_latest.json"
    return json.loads(lf.read_text(encoding="utf-8")) if lf.exists() else {}


def issued_list(pdir: Path) -> list:
    out = []
    for p in sorted((Path(pdir) / "output" / ISSUED).glob("R*_*")):
        f = p / "issued.json"
        if f.exists():
            out.append(json.loads(f.read_text(encoding="utf-8")))
    return sorted(out, key=lambda x: int(x["rev"][1:]))


def issue(pdir: Path, note: str = "") -> dict:
    """LATEST -> ISSUED/R<n>_<date>/ (read-only)."""
    d = dirs(pdir)
    lt = latest(pdir)
    if not lt:
        raise ValueError("LATEST가 비어 있습니다. 인덱스·케이블·산출근거를 먼저 만들어 주세요")
    n = len(issued_list(pdir))
    rev = f"R{n}"
    dst = d[ISSUED] / f"{rev}_{time.strftime('%Y%m%d')}"
    shutil.copytree(d[LATEST], dst)
    info = {"rev": rev, "at": time.strftime("%Y-%m-%d %H:%M"), "note": note, "files": lt, "dir": dst.name}
    (dst / "issued.json").write_text(json.dumps(info, ensure_ascii=False, indent=1), encoding="utf-8")
    for p in dst.rglob("*"):
        if p.is_file():
            p.chmod(p.stat().st_mode & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH))
    return info


def read_fid(path: Path) -> tuple:
    """(file id, state id, kind) from the '_지문' sheet."""
    out = {"지문 ID": "", "상태 ID": "", "종류": ""}
    try:
        import openpyxl
        wb = openpyxl.load_workbook(path, read_only=True)
        if "_지문" not in wb.sheetnames:
            return "", "", ""
        for a, b in wb["_지문"].iter_rows(min_row=1, max_row=10, max_col=2, values_only=True):
            if a in out:
                out[a] = str(b or "")
    except Exception:                                      # noqa: BLE001
        pass
    return out["지문 ID"], out["상태 ID"], out["종류"]


def identify(pdir: Path, path: Path) -> dict:
    """which file is this? by fingerprint id, then by exact bytes."""
    fid, state, kname = read_fid(path) if path.suffix.lower() == ".xlsx" else ("", "", "")
    sha = _sha(path, 64)
    lt = latest(pdir)
    same = lambda v: (fid and v.get("fid") == fid) or (v.get("sha256") and v.get("sha256") == sha)
    iss = [(it, v) for it in reversed(issued_list(pdir)) for v in (it.get("files") or {}).values() if same(v)]
    tag = f" · 발행본 {iss[0][0]['rev']}와도 같음" if iss else ""
    for k, v in lt.items():
        if same(v):
            return {"result": "LATEST", "text": f"최신 파일입니다 ({v['file']}, {v['at']}){tag}", "fid": fid}
    if iss:
        it, v = iss[0]
        newer = any(x.get("at", "") > it["at"] for x in lt.values())
        return {"result": it["rev"], "text": f"발행본 {it['rev']} ({it['at']})의 {v['file']}" + (" - 그 뒤에 바뀐 LATEST가 있음" if newer else ""), "fid": fid}
    hf = Path(pdir) / "output" / HISTORY / "history.json"
    for e in reversed(json.loads(hf.read_text(encoding="utf-8")) if hf.exists() else []):
        if same(e):
            cur = lt.get(e["kind"]) or {}
            st = " (내용 상태는 LATEST와 같음)" if state and cur.get("state") == state else " - 최신 아님, LATEST를 쓰세요"
            return {"result": "HISTORY", "text": f"예전 파일입니다 ({e['file']}, {e['at']}){st}", "fid": fid}
    if state:
        for k, v in lt.items():
            if v.get("state") == state:
                return {"result": "COPY", "text": f"LATEST {v['file']}와 같은 입력·설정에서 만든 다른 사본 (직접 고쳤을 수 있음)", "fid": fid}
    import re
    nm = Path(orig_name or path.name).name
    m = re.search(r"_(\d{8})_(\d{4})\.", nm)
    if not fid and (Path(pdir) / "output" / nm).exists():
        return {"result": "OLD", "text": f"v13.2 이전 파일 (지문 없음, {m.group(1)} {m.group(2)}) - 최신 아님, LATEST를 쓰세요" if m else "v13.2 이전 파일 (지문 없음)", "fid": ""}
    return {"result": "UNKNOWN", "text": "이 프로젝트에서 만든 기록이 없는 파일입니다 (직접 고친 파일이거나 다른 프로젝트)", "fid": fid}


def mark_index_changes(latest_xlsx: Path, issued_xlsx: Path, rev: str) -> int:
    """cells that differ from the issued index: yellow fill + comment 'R0: old'; list in '_발행대비변경'."""
    import openpyxl
    from openpyxl.comments import Comment
    from openpyxl.styles import PatternFill
    old = openpyxl.load_workbook(issued_xlsx, read_only=True, data_only=False)["Instrument_Index"]
    wb = openpyxl.load_workbook(latest_xlsx)
    ws = wb["Instrument_Index"]
    head = [c.value for c in next(ws.iter_rows(min_row=2, max_row=2))]

    def keyed(rows):
        out, seen = {}, {}
        for r in rows:
            tag = r[3] if len(r) > 3 else None
            if not tag:
                continue
            k = (r[2], tag)
            seen[k] = seen.get(k, 0) + 1
            out[k + (seen[k],)] = r
        return out
    o = keyed([list(r) for r in old.iter_rows(min_row=3, values_only=True)])
    rows_new = {}
    seen = {}
    for i, r in enumerate(ws.iter_rows(min_row=3, values_only=True), 3):
        if len(r) > 3 and r[3]:
            k = (r[2], r[3])
            seen[k] = seen.get(k, 0) + 1
            rows_new[k + (seen[k],)] = (i, list(r))
    fill = PatternFill("solid", fgColor="FFF2A8")
    newfill = PatternFill("solid", fgColor="D8F5E1")
    changes = []
    for k, (i, r) in rows_new.items():
        if k not in o:
            for c in range(2, min(len(r), 46) + 1):
                ws.cell(i, c).fill = newfill
            changes.append([k[1], "새 행", "", "", ""])
            continue
        orow = o[k]
        for c in range(2, min(len(r), 46) + 1):                # A = row formula, skip
            a = orow[c - 1] if c - 1 < len(orow) else None
            b = r[c - 1]
            if (a or "") != (b or "") and not (isinstance(b, str) and b.startswith("=")):
                cell = ws.cell(i, c)
                cell.fill = fill
                cell.comment = Comment(f"{rev}: {a}", "pid_io")
                changes.append([k[1], head[c - 1] if c - 1 < len(head) else c, str(a), str(b), ""])
    for k in o:
        if k not in rows_new:
            changes.append([k[1], "지워진 행", "", "", ""])
    if "_발행대비변경" in wb.sheetnames:
        del wb["_발행대비변경"]
    sh = wb.create_sheet("_발행대비변경")
    sh.append([f"{rev} 발행본 대비 바뀐 칸 (노랑 = 값 바뀜, 초록 = 새 행)", "", "", "", ""])
    sh.append(["Tag", "열", f"{rev} 값", "지금 값", "비고"])
    for c in changes:
        sh.append(c)
    for col, w in zip("ABCDE", (18, 28, 30, 30, 12)):
        sh.column_dimensions[col].width = w
    wb.save(latest_xlsx)
    return len(changes)
