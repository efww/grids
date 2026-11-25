"""Write rows into the Instrument Index template and build a reference comparison."""
from __future__ import annotations

import collections
import re
from copy import copy
from pathlib import Path

import openpyxl
from openpyxl.styles import Font, PatternFill

from .io_assign import Row

IO_COLS = ["AI", "AO", "DI", "DO", "DI(MCC)", "DO(MCC)", "DO-SOV"]


def write_template(all_rows: list[Row], template: Path, out: Path, source_file: str, rev: str = ""):
    rows = [r for r in all_rows if r.kind != "function"]
    funcs = [r for r in all_rows if r.kind == "function"]
    wb = openpyxl.load_workbook(template)
    ws = wb["Instrument_Index"]
    first = 3
    style_row = [copy(c._style) for c in ws[first + 3]]      # blank formatted row
    last = max(ws.max_row, first + len(rows) - 1)
    for r in range(first, last + 1):
        for c in range(2, 28):
            ws.cell(r, c).value = None
    for i, row in enumerate(rows):
        r = first + i
        vals = {
            1: f'=IF(D{r}="","",ROW()-2)', 3: row.unit, 4: row.tag, 5: row.loop, 6: row.func,
            7: row.desc, 9: row.dwg, 10: rev, 14: row.location, 15: row.system, 16: row.io,
            17: "", 18: "", 19: source_file, 20: row.page, 21: row.coord, 22: row.conf,
            23: row.status, 24: "; ".join(row.inform),
            27: f"{row.title} | {row.symbol_cls} | {row.signal} | IO={row.io_detail}",
        }
        for c, v in vals.items():
            cell = ws.cell(r, c)
            cell.value = v
            if r >= first + 3 and c - 1 < len(style_row):
                cell._style = copy(style_row[c - 1])
            if c != 1:
                cell.font = Font(color="000000")
    # keep the formulas on Summary sized to the data
    n_last = first + len(rows) - 1
    if n_last > 502:
        for wsn in ("Summary",):
            s = wb[wsn]
            for line in s.iter_rows():
                for c in line:
                    if isinstance(c.value, str) and "502" in c.value:
                        c.value = c.value.replace("502", str(n_last))
        ws.auto_filter.ref = f"A2:AA{n_last}"
    _restore_rules(ws, max(n_last, 502))
    # DCS / panel soft tags (PI, FIC, PAHH ...) are kept for loop checking only
    wf = wb.create_sheet("DCS_Functions")
    wf.append(["Tag No", "Func", "System", "Symbol", "P&ID No", "Title", "Page", "Coordination", "Conf."])
    for c in wf[1]:
        c.font = Font(bold=True)
    for r in funcs:
        wf.append([r.tag, r.func, r.system, r.symbol_cls, r.dwg, r.title, r.page, r.coord, r.conf])
    for col, w in zip("ABCDEFGHI", (16, 8, 8, 14, 11, 34, 6, 12, 7)):
        wf.column_dimensions[col].width = w
    wb.save(out)


def _restore_rules(ws, last):
    """openpyxl drops the template's x14 drop-down lists and the low-confidence
    highlight; put them back as standard rules and stretch every range to the
    last data row."""
    from openpyxl.formatting.formatting import ConditionalFormattingList
    from openpyxl.formatting.rule import FormulaRule
    from openpyxl.styles.differential import DifferentialStyle
    from openpyxl.worksheet.datavalidation import DataValidation

    old = ws.conditional_formatting
    ws.conditional_formatting = ConditionalFormattingList()
    for cf in old:
        rng = " ".join(re.sub(r"\d+$", str(last), str(r)) if str(r).endswith("502") else str(r)
                       for r in cf.sqref.ranges)
        for rule in cf.rules:
            ws.conditional_formatting.add(rng, rule)
    red = DifferentialStyle(font=Font(bold=True, color="FFC00000"))
    ws.conditional_formatting.add(
        f"V3:V{last}", FormulaRule(formula=["AND(ISNUMBER(V3),V3<Settings!$B$3)"], font=red.font))
    for col, src in (("W", "$E$2:$E$5"), ("P", "$B$2:$B$7"), ("N", "$D$2:$D$6"),
                     ("Q", "$D$2:$D$6"), ("O", "$A$2:$A$7")):
        dv = DataValidation(type="list", formula1=f"Lists!{src}", allow_blank=True)
        dv.add(f"{col}3:{col}{last}")
        ws.add_data_validation(dv)


# -------------------------------------------------------------- comparison
def _norm(t):
    t = (t or "").upper()
    t = t.replace("&", " ").replace("/", " ").replace(",", " ")
    t = re.sub(r"\bPKG\b", "PACKAGE", t)
    words = re.findall(r"[A-Z0-9]+", t)
    words = [w[:-1] if len(w) > 3 and w.endswith("S") else w for w in words]   # PUMPS -> PUMP
    return set(w for w in words if w not in {"AND", "THE", "OF", "P", "ID", "REV", "A", "TO", "FROM"})


def _match_title(ref_title, ref_dwg, pages):
    """pages: {dwg: (title, full_text)}. Score = title overlap + share of the
    reference title words found anywhere on the sheet + same sheet number."""
    common = _common_words(pages)
    a = _norm(ref_title)
    a_rare = a - common                   # project-wide words say nothing about the sheet
    best, score = None, 0.0
    for dwg, (t, text) in pages.items():
        if dwg.startswith("PID-9"):
            continue
        b, full = _norm(t), _norm(text)
        if not a:
            continue
        sc = (len(a & b) / len(a | b) if b else 0)
        if a_rare:
            sc += 0.6 * len(a_rare & full) / len(a_rare)
        try:
            if int(str(ref_dwg)) == int(dwg[-4:]):
                sc += 0.25
        except ValueError:
            pass
        if sc > score:
            best, score = dwg, sc
    return (best, score) if score >= 0.75 else (None, score)


_COMMON = {}


def _common_words(pages):
    """Words printed on most sheets (project name, title block) carry no signal."""
    key = id(pages)
    if key not in _COMMON:
        df = collections.Counter()
        sheets = [d for d in pages if not d.startswith("PID-9")]
        for d in sheets:
            df.update(_norm(pages[d][1]))
        _COMMON[key] = {w for w, n in df.items() if n > 0.5 * len(sheets)}
    return _COMMON[key]


def load_reference(path: Path):
    wb = openpyxl.load_workbook(path, data_only=True)
    ws = wb["INDEX"]
    hdr = [str(c.value or "").replace("\n", " ") for c in ws[4]]
    col = {h: i for i, h in enumerate(hdr)}
    out = []
    for r in ws.iter_rows(min_row=5, values_only=True):
        tag = r[col["Tag No."]]
        if not tag or str(r[0]).startswith("DEL"):
            continue
        func = str(tag).split("-")[0].strip()
        io = [k for k, i in zip(IO_COLS, range(13, 20)) if r[i] not in (None, "-", 0, "0")]
        qty = r[5] if isinstance(r[5], (int, float)) else 1
        train = r[6] if isinstance(r[6], (int, float)) and r[6] else qty
        # IO cells hold points x trains; bring them back to one drawing instance
        io_base = {}
        for k, i in zip(IO_COLS, range(13, 20)):
            v = r[i]
            if isinstance(v, (int, float)) and v:
                io_base[k] = v * qty / train if train else v
        out.append(dict(tag=str(tag), func=func, title=r[col["P&ID Title"]], dwg=r[col["DWG. No."]],
                        system=r[col["To (System)"]], io=io, io_base=io_base, qty=qty,
                        desc=r[col["Instrument Description"]]))
    return out


def compare(rows: list[Row], ref: list[dict], out_wb: Path, pages: dict, overrides: dict | None = None):
    pdf_titles = {d: t for d, (t, _) in pages.items()}
    title_map = {}
    overrides = overrides or {}
    for key in {(x["title"], x["dwg"]) for x in ref}:
        ov = overrides.get(f"{key[0]}|{key[1]}", overrides.get(key[0]))
        title_map[key] = (ov, 9.0) if ov else _match_title(key[0], key[1], pages)
        if ov == "-":
            title_map[key] = (None, 0.0)

    ref_cnt = collections.Counter()
    for x in ref:
        dwg = title_map[(x["title"], x["dwg"])][0] or f"(ref) {x['title']}"
        ref_cnt[(dwg, x["func"])] += 1
    rows = [r for r in rows if r.kind != "function"]
    our_cnt = collections.Counter((r.dwg, r.func) for r in rows)

    wb = openpyxl.load_workbook(out_wb)
    if "Ref_Compare" in wb.sheetnames:
        del wb["Ref_Compare"]
    ws = wb.create_sheet("Ref_Compare")
    ws.append(["Drawing", "Title (PDF)", "Func", "Reference", "Extracted", "Diff", "판정"])
    for c in ws[1]:
        c.font = Font(bold=True)
    red = PatternFill("solid", fgColor="FFC7CE")
    yel = PatternFill("solid", fgColor="FFEB9C")
    grn = PatternFill("solid", fgColor="C6EFCE")
    tot = collections.Counter()
    for key in sorted(set(ref_cnt) | set(our_cnt)):
        a, b = ref_cnt.get(key, 0), our_cnt.get(key, 0)
        verdict = "일치" if a == b else ("추출 누락" if b < a else "추출 초과")
        tot[verdict] += 1
        ws.append([key[0], pdf_titles.get(key[0], ""), key[1], a, b, b - a, verdict])
        ws.cell(ws.max_row, 7).fill = grn if a == b else (red if b < a else yel)

    ws2 = wb.create_sheet("Ref_Title_Map") if "Ref_Title_Map" not in wb.sheetnames else wb["Ref_Title_Map"]
    ws2.append(["Reference P&ID Title", "Ref DWG No", "Matched PDF DWG", "PDF Title", "Score"])
    for (t, rd), (dwg, sc) in sorted(title_map.items(), key=lambda x: str(x[0][1]).zfill(4)):
        ws2.append([t, rd, dwg or "(no match)", pdf_titles.get(dwg, ""), round(sc, 2)])
    for w in (ws, ws2):
        for col in "ABCDEFG":
            w.column_dimensions[col].width = 18
    wb.save(out_wb)

    # ---- document-wide view: instrument count and IO points per type
    wsF = wb["Ref_Compare_Func"] if "Ref_Compare_Func" in wb.sheetnames else wb.create_sheet("Ref_Compare_Func")
    wsF.append(["Func", "Reference Q'ty", "Extracted", "Diff"])
    rq = collections.Counter()
    for x in ref:
        if title_map[(x["title"], x["dwg"])][0]:
            rq[x["func"]] += x["qty"]
    oq = collections.Counter(r.func for r in rows)
    for f in sorted(set(rq) | set(oq), key=lambda f: -max(rq[f], oq[f])):
        wsF.append([f, rq[f], oq[f], oq[f] - rq[f]])
    wsI = wb["Ref_Compare_IO"] if "Ref_Compare_IO" in wb.sheetnames else wb.create_sheet("Ref_Compare_IO")
    wsI.append(["System", "IO", "Reference (per instance)", "Extracted", "Diff"])
    rio = collections.Counter()
    for x in ref:
        if title_map[(x["title"], x["dwg"])][0]:
            for k, v in x["io_base"].items():
                rio[(str(x["system"]).replace("FGS", "F&G"), k)] += v
    oio = collections.Counter()
    for r in rows:
        for k in r.io_detail.split("/"):
            if k and k != "-":
                oio[(r.system, k)] += 1
    for key in sorted(set(rio) | set(oio)):
        wsI.append([key[0], key[1], round(rio[key], 1), oio[key], round(oio[key] - rio[key], 1)])
    wb.save(out_wb)
    io_ref, io_our = sum(rio.values()), sum(oio.values())
    io_hit = sum(min(rio[k], oio[k]) for k in rio)

    # recall on matched drawings / func codes
    matched = [k for k in ref_cnt if not k[0].startswith("(ref)")]
    hit = sum(min(ref_cnt[k], our_cnt.get(k, 0)) for k in matched)
    total = sum(ref_cnt[k] for k in matched)
    extra = sum(max(0, our_cnt[k] - ref_cnt.get(k, 0)) for k in our_cnt)
    return dict(io_points_ref=round(io_ref), io_points_extracted=io_our,
                io_recall=round(io_hit / io_ref, 3) if io_ref else 0,
                ref_rows=len(ref), ref_rows_on_matched_dwg=total, matched=hit,
                recall=round(hit / total, 3) if total else 0, extra=extra,
                unmatched_titles=[t for (t, _), (d, _) in title_map.items() if not d], verdicts=dict(tot))
