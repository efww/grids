"""Write rows into the Instrument Index template and build a reference comparison."""
from __future__ import annotations

import collections
import re
from copy import copy
from pathlib import Path

import openpyxl

from .directopen import load_workbook
from openpyxl.styles import Font, PatternFill

from .io_assign import Row

IO_COLS = ["AI", "AO", "DI", "DO", "DI(MCC)", "DO(MCC)", "DO-SOV"]


ORIGINS = ["도면인식", "Typical파생", "트레인복제", "대수복제", "장비Typical", "패키지", "스페어"]
SYSTEMS = ["DCS", "ESD", "F&G", "PKG PLC"]
HW = ["AI", "AO", "DI", "DO"]


# The index template's row-2 headers (A..AA). The program writes by column position, so an
# upload whose headers differ would put values in the wrong columns: refuse it instead.
TEMPLATE_HEADERS = ["No", "Area", "Unit", "Tag No", "Loop No", "Func. Code", "Instrument Type", "Service Description",
                    "P&ID No", "P&ID Rev", "Line No", "Equipment No", "Line Size", "Inst. Location", "System",
                    "I/O Type", "I/O Location", "Vendor PKG", "Source File", "Page", "Coordination", "OCR Conf.",
                    "Status", "Inform 사유", "Reviewer", "Review Date", "Remarks"]


def template_problems(ws) -> list[str]:
    """[] when row 2 of the Instrument_Index sheet matches the standard template, else what differs."""
    from openpyxl.utils import get_column_letter
    norm = lambda x: re.sub(r"\s+", " ", str(x or "")).strip().lower()
    row = next(ws.iter_rows(min_row=2, max_row=2, values_only=True), ())
    row = list(row) + [None] * (len(TEMPLATE_HEADERS) - len(row))
    out = []
    for i, want in enumerate(TEMPLATE_HEADERS):
        if norm(row[i]) != norm(want):
            out.append(f"{get_column_letter(i + 1)}열이 '{want}'여야 하는데 '{row[i] or '(빈 칸)'}'입니다")
    return out


def write_template(all_rows: list[Row], template: Path, out: Path, source_file: str, sheets: dict | None = None,
                   concept=None, log=None, scenarios=None, impact=None, area_map: dict | None = None,
                   cable_map: dict | None = None):
    """sheets: {page: {rev, plant_no, sheet_no}} from the title blocks (extract.json).
    area_map: {dwg or plant_no: Area} - Area is filled only from this map (decision 1)."""
    sheets = {str(k): v for k, v in (sheets or {}).items()}
    area_map = area_map or {}
    spares = [r for r in all_rows if r.kind != "function" and r.origin == "스페어"]
    rows = [r for r in all_rows if r.kind != "function" and r.origin != "스페어"]   # decision 21
    funcs = [r for r in all_rows if r.kind == "function"]
    if concept is not None and str(concept.get("I01", "")) == "접두 없음":
        import copy as _c
        rows = [_c.copy(r) for r in rows]
        for r in rows:
            if r.unit and r.tag.startswith(r.unit + "-"):
                r.tag = r.tag[len(r.unit) + 1:]
    if concept is not None and not concept.on("A05"):
        funcs = None
    wb = load_workbook(template)
    ws = wb["Instrument_Index"]
    bad = template_problems(ws)
    if bad:
        raise ValueError("인덱스 양식이 표준과 다릅니다 (" + "; ".join(bad[:4]) + (" …" if len(bad) > 4 else "")
                         + "). 2 입력 화면의 '표준 양식 받기'로 받은 양식에 맞춰 다시 올려 주세요")
    first = 3
    style_row = [copy(c._style) for c in ws[first + 3]]      # blank formatted row
    last = max(ws.max_row, first + len(rows) - 1)
    for r in range(first, last + 1):
        for c in range(2, 39):
            ws.cell(r, c).value = None
    # extra columns: where each row came from
    for c, h in ((28, "산출구분"), (29, "근거 (도면/노트/체크리스트)"), (30, "원본 Tag"), (31, "Plant No"),
                 (32, "Sheet No"), (33, "Signal Type")):                     # 33: decision 8
        cell = ws.cell(2, c, h)
        cell._style = copy(ws.cell(2, 27)._style)
        ws.column_dimensions[cell.column_letter].width = (12, 46, 16, 10, 10, 14)[c - 28]
    try:
        from openpyxl.comments import Comment
        ws.cell(2, 22).comment = Comment("값 = 심볼 인식 신뢰도 (0~1). OCR은 사용하지 않음.", "pid_io")
    except Exception:
        pass
    ws.cell(1, 28, "산출 근거")._style = copy(ws.cell(1, 19)._style)
    if cable_map is not None:                          # STEP 6 cable / STEP 7 JB results
        from .cable import INDEX_COLS
        for c, h, _, w in INDEX_COLS:
            cell = ws.cell(2, c, h)
            cell._style = copy(ws.cell(2, 27)._style)
            ws.column_dimensions[cell.column_letter].width = w
        ws.cell(1, 34, "케이블 · JB (STEP 6·7)")._style = copy(ws.cell(1, 19)._style)
        ws.cell(1, 39, "길이 · 트레이 · 서포트 (STEP 6·8·9)")._style = copy(ws.cell(1, 19)._style)
    for i, row in enumerate(rows):
        r = first + i
        sh = sheets.get(str(row.page), {}) if row.page else {}
        area = area_map.get(row.dwg) or area_map.get(sh.get("plant_no", "")) or ""
        vals = {
            1: f'=IF(D{r}="","",ROW()-2)', 2: area, 3: row.unit, 4: row.tag, 5: row.loop, 6: row.func,
            7: row.desc, 8: getattr(row, "service", ""), 9: row.dwg, 10: sh.get("rev", ""),
            12: getattr(row, "equip", ""), 14: row.location, 15: row.system, 16: row.io,
            17: getattr(row, "ioloc", ""), 18: row.vendor_pkg, 19: source_file, 20: row.page or None, 21: row.coord,
            22: row.conf, 23: row.status, 24: "; ".join(row.inform),
            27: " | ".join(x for x in (row.title, row.symbol_cls) if x),
            28: row.origin, 29: row.basis, 30: row.src_tag, 31: sh.get("plant_no", ""), 32: sh.get("sheet_no", ""),
            33: signal_type(row),
        }
        if cable_map is not None:
            cm = cable_map.get(row.tag) or cable_map.get(f"{row.unit}-{row.tag}")
            from .cable import INDEX_COLS
            for c, _, k, _w in INDEX_COLS:
                vals[c] = (cm.get(k, "") if cm else ("-" if c <= 38 else ""))
        for c, v in vals.items():
            cell = ws.cell(r, c)
            cell.value = v
            if r >= first + 3 and c - 1 < len(style_row):
                cell._style = copy(style_row[c - 1])
            elif c >= 28 or c in (2, 8, 12, 17):
                cell._style = copy(ws.cell(r, 27)._style)
            if c != 1:
                cell.font = Font(color="000000")
    n_last = first + len(rows) - 1
    if n_last > 502:
        for line in wb["Summary"].iter_rows():
            for c in line:
                if isinstance(c.value, str) and "502" in c.value:
                    c.value = c.value.replace("502", str(n_last))
    ws.auto_filter.ref = f"A2:{'AT' if cable_map is not None else 'AG'}{max(n_last, 502)}"
    _restore_rules(ws, max(n_last, 502))
    _io_summary(wb, max(n_last, 502), concept, spares)
    if "Settings" in wb.sheetnames:                    # decision 19
        wb["Settings"]["B2"] = 0.95
        wb["Settings"]["B3"] = 0.80
    if concept is not None:
        _concept_basis(wb, concept, all_rows, impact)
    if log:
        _notes_sheet(wb, log)
    if scenarios and concept is not None and concept.on("I02"):
        wsc = wb.create_sheet("Scenarios", 2)
        wsc.append(["시나리오", "하드와이어 IO", "통신 포트", "인덱스 행", "IO 카드", "저장", "기본값과 다른 가정"])
        for c in wsc[1]:
            c.font = Font(bold=True)
        for sc in scenarios:
            wsc.append([sc["name"], sc["hw"], sc["comm"], sc["rows"], sc.get("cards"), sc.get("saved"),
                        "; ".join(f"{c[0]} {c[1]}={c[2]}" for c in sc.get("changed", []))])
        for col, w in zip("ABCDEFG", (18, 12, 10, 10, 9, 12, 90)):
            wsc.column_dimensions[col].width = w
    if funcs is None:
        wb.save(out)
        return
    # DCS / panel soft tags (PI, FIC, PAHH ...) are kept for loop checking only
    wf = wb.create_sheet("DCS_Functions")
    wf.append(["Tag No", "Func", "System", "Symbol", "P&ID No", "Title", "Page", "Coordination", "Conf.", "산출구분"])
    for c in wf[1]:
        c.font = Font(bold=True)
    for r in funcs:
        wf.append([r.tag, r.func, r.system, r.symbol_cls, r.dwg, r.title, r.page, r.coord, r.conf, r.origin])
    for col, w in zip("ABCDEFGHIJ", (16, 8, 8, 14, 11, 34, 6, 12, 7, 12)):
        wf.column_dimensions[col].width = w
    wb.save(out)


SIGNAL_BY_IO = {"AI": "4-20mA (HART)", "AO": "4-20mA (HART)", "DI": "24VDC", "DO": "24VDC",
                "TC/RTD": "RTD", "Serial/Comm": "COMM"}


def signal_type(row) -> str:
    """Signal Type column (decision 8): the IO rule's signal text, else by IO type."""
    if row.io in ("Serial/Comm", "TC/RTD"):
        return SIGNAL_BY_IO[row.io]
    s = (row.signal or "").strip()
    if s and s not in ("-", "?") and row.io not in ("-", "", None):
        return s
    return SIGNAL_BY_IO.get(row.io, "")


def _io_summary(wb, last, concept, spares=None):
    """IO points counted from the index rows themselves (COUNTIFS), split by
    where the rows came from. Spares are not index rows (decision 21)."""
    spare_cnt = collections.Counter((r.system, r.io) for r in (spares or []))
    ws = wb.create_sheet("IO_Summary", 1)
    ix = "Instrument_Index"
    rng = lambda col: f"{ix}!${col}$3:${col}${last}"
    ws["A1"] = "IO 집계 (인덱스 행을 직접 카운트)"
    ws["A1"].font = Font(bold=True, size=14)
    ws["A2"] = ("IO는 인덱스 행을 직접 셉니다(수식). 스페어는 인덱스 본문에 넣지 않고 이 시트의 '스페어' 행에만 "
                "스페어율 계산값으로 넣습니다.")
    cols = [(s, io) for s in ("DCS", "ESD", "F&G") for io in HW] + [("*", "Serial/Comm")]
    hdr = ["산출구분"] + [f"{s} {io}" for s, io in cols[:-1]] + ["통신 포트", "하드와이어 합계", "인덱스 행 수"]
    ws.append([])
    ws.append(hdr)
    for c in ws[4]:
        c.font = Font(bold=True, color="FFFFFF")
        c.fill = PatternFill("solid", fgColor="1F3864")
    for i, org in enumerate(ORIGINS + ["합계"]):
        r = 5 + i
        ws.cell(r, 1, org).font = Font(bold=org == "합계")
        for k, (sy, io) in enumerate(cols):
            col = 2 + k
            if org == "합계":
                L = ws.cell(5, col).column_letter
                f = f"=SUM({L}5:{L}{r - 1})"
            elif org == "스페어":
                f = spare_cnt.get((sy, io), 0) if sy != "*" else 0
            elif sy == "*":
                f = f'=COUNTIFS({rng("AB")},$A{r},{rng("P")},"{io}")'
            else:
                f = f'=COUNTIFS({rng("AB")},$A{r},{rng("O")},"{sy}",{rng("P")},"{io}")'
            ws.cell(r, col, f)
        n = len(cols)
        a, b = ws.cell(r, 2).column_letter, ws.cell(r, 1 + n - 1).column_letter
        ws.cell(r, 2 + n, f"=SUM({a}{r}:{b}{r})").font = Font(bold=True)
        ws.cell(r, 3 + n, f"=SUM({ws.cell(5, 3 + n).column_letter}5:{ws.cell(5, 3 + n).column_letter}{r - 1})"
                if org == "합계" else ("(본문 제외)" if org == "스페어" else f'=COUNTIF({rng("AB")},$A{r})'))
    tot = 5 + len(ORIGINS)
    # cards per system / IO type
    ch = concept.num("H05", 16) if concept is not None else 16
    r0 = tot + 3
    ws.cell(r0, 1, "IO 카드 개략 수량 (스페어 포함, 채널 수 = 체크리스트 H05)").font = Font(bold=True, size=12)
    ws.cell(r0 + 1, 1, "채널/카드")
    ws.cell(r0 + 1, 2, ch)
    ws.cell(r0 + 1, 3, "← Concept H05")
    ws.append([])
    hr = r0 + 2
    for k, h in enumerate(["System"] + HW + ["카드 합계"]):
        c = ws.cell(hr, 1 + k, h)
        c.font = Font(bold=True, color="FFFFFF")
        c.fill = PatternFill("solid", fgColor="1F3864")
    for j, sy in enumerate(("DCS", "ESD", "F&G")):
        r = hr + 1 + j
        ws.cell(r, 1, sy)
        for k, io in enumerate(HW):
            tot_cell = ws.cell(tot, 2 + cols.index((sy, io))).coordinate
            ws.cell(r, 2 + k, f'=ROUNDUP({tot_cell}/$B${r0 + 1},0)')
        ws.cell(r, 6, f"=SUM(B{r}:E{r})").font = Font(bold=True)
    for col, w in zip("ABCDEFGHIJKLMNOPQ", [14] + [9] * 16):
        ws.column_dimensions[col].width = w


def _concept_basis(wb, concept, rows=None, impact=None):
    """'적용_체크리스트' sheet: every checklist item, what was applied, and what it did
    to this index (rows / IO points it produced, IO change if switched)."""
    from openpyxl.styles import Alignment, Border, Side
    ws = wb.create_sheet("적용_체크리스트", 1)
    thin = Side(style="thin", color="D0D0D0")
    box = Border(bottom=thin)
    green, chg = "15AD60", "FFF5D6"
    rows = [r for r in (rows or []) if r.kind != "function"]
    hw = {"AI", "AO", "DI", "DO", "TC/RTD"}
    imp = {x["id"]: x for x in (impact or {}).get("items", [])}

    def used(iid):
        pat = re.compile(rf"\b{iid}\b")
        rs = [r for r in rows if pat.search(r.basis or "") or any(pat.search(x) for x in r.inform)]
        return len(rs), sum(1 for r in rs if r.io in hw)

    changed = [k for k, v in concept.items.items() if v["value"] != v["default"]]
    st = {"applied": 0, "info": 0, "planned": 0}
    for v in concept.items.values():
        st[v.get("status", "applied")] += 1
    ws["A1"] = "적용 체크리스트 - 이 인덱스에 쓰인 개념설계 가정"
    ws["A1"].font = Font(bold=True, size=15)
    ws["A2"] = (f"체크리스트 파일: {concept.path.name}   ·   항목 {len(concept.items)}개 (계산 반영 {st['applied']} · "
                f"기록용 {st['info']} · 미적용 {st['planned']})   ·   기본값에서 바꾼 항목 {len(changed)}개"
                + (f": {', '.join(changed)}" if changed else ""))
    ws["A2"].font = Font(size=10, color="555555")
    if impact:
        ws["A3"] = (f"하드와이어 IO {impact['base']}점 기준. '다른 선택 시 IO 변화'는 그 항목만 바꿨을 때의 증감입니다 "
                    "(회의에서 먼저 정할 항목 판단용).")
        ws["A3"].font = Font(size=10, color="555555")
    hdr = ["ID", "항목", "적용값", "기본값", "변경", "계산 반영", "이 설정으로 생긴 행", "그중 IO 점수",
           "다른 선택 시", "IO 변화", "IO 영향 (설명)", "근거 / 비고"]
    ws.append([])
    ws.append(hdr)
    hr = ws.max_row
    for c in ws[hr]:
        c.font = Font(bold=True, color="FFFFFF")
        c.fill = PatternFill("solid", fgColor=green)
        c.alignment = Alignment(vertical="center", wrap_text=True)
    label = {"applied": "반영", "info": "기록용", "planned": "미적용"}
    group = None
    for k, v in concept.items.items():
        if v.get("group") != group:
            group = v.get("group")
            ws.append([group])
            gr = ws.max_row
            ws.cell(gr, 1).font = Font(bold=True, color="1C5438")
            for col in range(1, len(hdr) + 1):
                ws.cell(gr, col).fill = PatternFill("solid", fgColor="E7F6EE")
        n_rows, n_io = used(k)
        im = imp.get(k)
        val = v["value"]
        if isinstance(val, float) and val < 1 and v.get("type") == "숫자":
            val = f"{val:.0%}"
        dflt = v["default"]
        if isinstance(dflt, float) and dflt < 1 and v.get("type") == "숫자":
            dflt = f"{dflt:.0%}"
        status = v.get("status", "applied")
        ws.append([k, v["name"], val, dflt, "변경" if v["value"] != v["default"] else "",
                   label[status] if status == "applied" else (v.get("status_note") or label[status]),
                   n_rows or "", n_io or "", im["alt"] if im else "", (f"{im['delta']:+d}" if im else ""),
                   v.get("effect", ""), v.get("basis", "")])
        r = ws.max_row
        for col in range(1, len(hdr) + 1):
            c = ws.cell(r, col)
            c.border = box
            c.alignment = Alignment(vertical="center", wrap_text=col in (2, 6, 11, 12))
            if v["value"] != v["default"]:
                c.fill = PatternFill("solid", fgColor=chg)
            elif status == "planned":
                c.font = Font(color="999999")
        if im:
            ws.cell(r, 10).font = Font(bold=True, color="1C5438" if im["delta"] < 0 else "D1335D")
    for col, w in zip("ABCDEFGHIJKL", (6, 40, 18, 18, 6, 26, 11, 10, 18, 9, 44, 30)):
        ws.column_dimensions[col].width = w
    ws.freeze_panes = ws.cell(hr + 1, 3)
    ws.auto_filter.ref = f"A{hr}:L{ws.max_row}"


def _notes_sheet(wb, log):
    ws = wb.create_sheet("Notes_Review")
    ws.append(["P&ID No", "Title", "노트 원문 (발췌)", "적용 효과"])
    for c in ws[1]:
        c.font = Font(bold=True)
    for row in log:
        ws.append(list(row))
    for col, w in zip("ABCD", (11, 30, 90, 46)):
        ws.column_dimensions[col].width = w


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


def find_reference_sheet(wb):
    """(sheet, header row): 'INDEX' if present, else the first sheet with a 'Tag No.' header in rows 1-10."""
    order = (["INDEX"] if "INDEX" in wb.sheetnames else []) + [n for n in wb.sheetnames if n != "INDEX"]
    for n in order:
        ws = wb[n]
        for i, row in enumerate(ws.iter_rows(min_row=1, max_row=10, values_only=True), 1):
            if any(str(v or "").replace("\n", " ").strip() == "Tag No." for v in row):
                return ws, i
    raise ValueError("레퍼런스에서 'Tag No.' 머리글이 있는 시트를 찾지 못했습니다 (시트: "
                     + ", ".join(wb.sheetnames[:12]) + ")")


def load_reference(path: Path, override: dict | None = None):
    """v13.4: any layout - header / columns found by caption (refindex). Returns the reference rows."""
    from .refindex import read
    return read(path, override)[0]


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
    rows = [r for r in rows if r.kind != "function" and r.origin not in ("트레인복제", "대수복제", "스페어")]
    our_cnt = collections.Counter((r.dwg, r.func) for r in rows)

    wb = load_workbook(out_wb)
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
                k = {"DO-SOV": "DO", "DI(MCC)": "DI", "DO(MCC)": "DO"}.get(k, k)
                rio[(str(x["system"]).replace("FGS", "F&G"), k)] += v
    oio = collections.Counter()
    for r in rows:
        if r.io and r.io != "-":
            oio[(r.system, r.io)] += 1
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
