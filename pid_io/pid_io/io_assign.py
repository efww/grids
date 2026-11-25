"""Symbols + notes + concept checklist -> index rows.

Rule: one IO point = one index row. Anything a note or a checklist choice
adds (train copies, limit switches, motor IO, package IO, spares) becomes
its own row, with `origin` and `basis` saying where it came from, so that
counting rows in the index always gives the IO total.
"""
from __future__ import annotations

import copy
import math
import re
from dataclasses import dataclass, field

from .concept import Concept
from .extract import Symbol

SYSTEM_NAMES = {"FGS": "F&G", "PLC": "PKG PLC"}       # -> template "Lists" names
IO_NAMES = {"DO-SOV": "DO", "DI(MCC)": "DI", "DO(MCC)": "DO", "COMM": "Serial/Comm"}
LOCATION = {"FIELD": "Field", "DCS_SHARED": "CCR", "PANEL_CR": "CCR",
            "PANEL_BEHIND": "Local Panel", "PLC": "Local Panel"}
HW_IO = ("AI", "AO", "DI", "DO")
SPARE_IDS = {"AI": "H01", "AO": "H02", "DI": "H03", "DO": "H04"}


@dataclass
class Row:
    unit: str
    tag: str
    loop: str
    func: str
    desc: str
    dwg: str
    title: str
    location: str
    system: str
    io: str
    signal: str
    page: int
    coord: str
    conf: float
    status: str
    inform: list = field(default_factory=list)
    symbol_cls: str = ""
    kind: str = "instrument"      # instrument | function (DCS soft tag)
    origin: str = "도면인식"       # 도면인식 / Typical파생 / 트레인복제 / 장비Typical / 패키지 / 스페어
    basis: str = ""
    src_tag: str = ""
    vendor_pkg: str = ""

    @property
    def derived(self):
        return self.origin != "도면인식"


def _rule_for(rules, func):
    for r in rules:
        if re.match(r["match"], func):
            return r
    return rules[-1]


def _io(x):
    return IO_NAMES.get(x, x) if x else "-"


def _status(conf, notes, review_flag, confirm, review):
    if review_flag:
        return "검토필요"
    if conf >= confirm and not notes:
        return "확정"
    return "검토필요" if conf >= review else "불명"


def assign(symbols: list[Symbol], io_proto: dict, concept: Concept, notes: dict | None = None,
           confirm=0.95, review=0.80):
    notes = notes or {}
    rules = io_proto["rules"]
    by_sid = {s.sid: s for s in symbols}
    rows: list[Row] = []
    seen: dict[str, Row] = {}
    log: list[tuple] = []                      # (dwg, text, effect) for the Notes sheet
    keep = str(concept.get("B10", "")).startswith("각각")

    for s in symbols:
        if s.role in ("interlock", "candidate", "ignored"):
            continue           # candidates only count once a person (or AI + person) accepts them

        # ------------------------------------------------ pumps -> motor typical
        if s.func == "EQUIP:P":
            typ = concept.get("D01", "MTR-LV-DOL")
            unit, num = s.number[:2], s.number.split("-")[-1]
            for main, fc, sfx, desc, sys_, io, ids in concept.expand(typ):
                rows.append(Row(unit, f"{unit}-{fc}-{num}{sfx}", f"M-{num}", fc, f"{desc} [{s.number}]",
                                s.dwg, s.title, "MCC", sys_, _io(io), "24VDC", s.page, "", 1.0, "검토필요",
                                [], "EQUIPMENT", origin="장비Typical",
                                basis=f"{s.number} / {typ} ({', '.join(['D01'] + ids)})", src_tag=s.number))
            continue

        # ------------------------------------------------ vendor packages
        if s.func == "EQUIP:PK":
            typ, conn, prow, src = concept.package_typical(s.number)
            units = [u for u in s.notes[0].split(":", 1)[1].split("/") if u] if s.notes else []
            n = int(prow["qty"]) if prow and prow.get("qty") else max(1, len(units))
            unit = s.number[:2]
            later = notes.get(s.dwg).later if notes.get(s.dwg) else ""
            pts = list(concept.expand(typ))
            if not [p for p in pts if p[5] and p[5] != "-"]:
                if later and concept.on("C09"):
                    rows.append(Row(unit, f"{unit}-PKG-XXXX", "", "PKG", f"패키지 물량 미상 [{s.number}]",
                                    s.dwg, s.title, "Skid", "?", "-", "-", s.page, "", 0.0, "불명",
                                    ["패키지 도면 미개발 + 전부 하드와이어: 계기 수를 알 수 없음"], "PACKAGE",
                                    origin="패키지", basis=f"{s.number} / {typ} ({src}, C09)",
                                    src_tag=s.number, vendor_pkg=s.number))
                continue
            for k in range(n):
                letter = units[k] if k < len(units) else str(k + 1)
                pk = f"{s.number}{letter}"
                for main, fc, sfx, desc, sys_, io, ids in concept.expand(typ):
                    rows.append(Row(unit, f"{unit}-{fc}-XXXX{sfx}", "", fc, f"{desc} [{pk}]", s.dwg, s.title,
                                    "Skid", sys_, _io(io), "", s.page, "", 1.0, "검토필요",
                                    [f"연결 방식: {conn}"], "PACKAGE", origin="패키지",
                                    basis=f"{pk} / {typ} ({', '.join([src] + ids)})",
                                    src_tag=s.number, vendor_pkg=pk))
            continue

        # ------------------------------------------------ drawn instruments
        rule = _rule_for(rules, s.func)
        if rule.get("gate") and not concept.on(rule["gate"]):
            continue
        inform = list(s.notes)
        system = rule.get("system", "?")
        esd_links = [by_sid[l] for l in s.links if l in by_sid and by_sid[l].cls == "ESD_INTERLOCK"]
        if system == "link":
            system = "ESD" if esd_links else "DCS"
            if esd_links:
                inform.append("ESD interlock " + ",".join(sorted({f"{i.func}-{i.number}" for i in esd_links})))
        base = dict(unit=s.unit, dwg=s.dwg, title=s.title, location=LOCATION.get(s.cls, ""),
                    page=s.page, coord=f"{round(s.cx)},{round(s.cy)}", conf=round(s.score, 3),
                    symbol_cls=s.cls)
        loop = f"{s.func[0]}-{s.number}" if s.number else ""
        status = _status(s.score, s.notes, rule.get("review") or system == "?", confirm, review)
        if rule.get("review"):
            inform.append("function code not in IO protocol")
        pkg = s.vendor            # from the dash-dot vendor boundary the symbol sits in

        if s.role == "function" and s.system != "FIELD":
            sysn = {"DCS": "DCS", "PLC": "PLC", "LOCAL": "-"}.get(s.system, system)
            row = Row(tag=s.tag, loop=loop, func=s.func, desc=rule.get("desc", ""),
                      system=SYSTEM_NAMES.get(sysn, sysn), io="-", signal="-", status=status,
                      inform=inform, kind="function", basis=s.dwg,
                      vendor_pkg=pkg if concept.on("B06") else "", **base)
            _add(row, rows, seen, keep)
            continue

        if s.override:                      # accepted review / AI decision wins over the rule
            if s.override.get("system") not in (None, "", "-"):
                system = s.override["system"]
            inform.append("검토 승인" + (f": {s.override['reason']}" if s.override.get("reason") else ""))
        if rule.get("ai"):
            inform.append("AI 제안 규칙 (승인됨)")
        typ = rule.get("typical", "").replace("{SYS}", "ESD" if system == "ESD" else "DCS")
        points = list(concept.expand(typ, s.func, SYSTEM_NAMES.get(system, system))) if typ else []
        main_io = next((p[5] for p in points if p[0]), None) if typ else None
        if main_io is None:
            main_io = "/".join(rule.get("io", [])) or "-"
        if s.override and s.override.get("io") not in (None, ""):
            main_io = s.override["io"]
        # vendor PLC package, its instruments not in our IO scope (A03 OFF)
        vendor_out = False
        if pkg and concept.on("B06"):
            inform.append(f"벤더 경계 안 ({pkg})")
            ptyp, conn, _, _ = concept.package_typical(pkg)
            if ptyp == "PKG-COMM" and not concept.on("A03"):
                vendor_out = True
                inform.append(f"벤더 PLC 내부 계기 - IO 제외 ({pkg}, A03 OFF)")
        row = Row(tag=s.tag, loop=loop, func=s.func, desc=rule.get("desc", ""),
                  system=SYSTEM_NAMES.get(system, system), io="-" if vendor_out else _io(main_io),
                  signal=rule.get("signal", ""), status=status, inform=inform,
                  basis=f"{s.dwg}" + (f" / {typ}" if typ else ""),
                  vendor_pkg=pkg if concept.on("B06") else "", **base)
        if not _add(row, rows, seen, keep):
            continue
        for main, fc, sfx, desc, sys_, io, ids in points:
            if main:
                continue
            rows.append(Row(tag=f"{s.unit}-{fc}-{s.number}{sfx}" if s.unit else f"{fc}-{s.number}{sfx}",
                            loop=loop, func=fc, desc=desc, system=sys_,
                            io="-" if vendor_out else _io(io), signal="24VDC" if io in ("DI", "DO") else row.signal,
                            status=status, inform=list(inform[-1:]) if vendor_out else [],
                            origin="Typical파생", basis=f"{s.tag} / {typ} ({', '.join(ids)})",
                            src_tag=s.tag, vendor_pkg=row.vendor_pkg, **base))

    # ------------------------------------------------ implied tags (thermowells ...)
    present = {(r.dwg, r.func, r.tag.split("-")[-1]) for r in rows}
    have = {r.tag for r in rows}
    for imp in io_proto.get("implied", []):
        if imp.get("gate") and not concept.on(imp["gate"]):
            continue
        for r in list(rows):
            if r.kind != "instrument" or r.derived or not re.match(imp["from"], r.func):
                continue
            num = r.tag.split("-")[-1]
            if imp.get("unless") and (r.dwg, imp["unless"], num) in present:
                continue
            tag = f"{r.unit}-{imp['tag']}-{num}" if r.unit else f"{imp['tag']}-{num}"
            if tag in have:
                continue
            have.add(tag)
            rows.append(Row(r.unit, tag, r.loop, imp["tag"], imp["desc"], r.dwg, r.title, r.location, "-", "-",
                            "-", r.page, r.coord, r.conf, "검토필요", [], "IMPLIED", origin="Typical파생",
                            basis=f"{r.tag} ({imp.get('gate', '')})", src_tag=r.tag, vendor_pkg=r.vendor_pkg))

    # ------------------------------------------------ notes: train copies / only-one-shown
    for dwg, pn in notes.items():
        for text, effect in pn.notes:
            log.append((dwg, pn.title, text, effect))
        if pn.copies and concept.on("B01"):
            src_rows = [r for r in rows if r.dwg == dwg]
            for t in pn.copies:
                for r in src_rows:
                    c = copy.deepcopy(r)
                    c.tag = re.sub(rf"^{re.escape(r.unit)}-", f"{t}-", r.tag) if r.unit else f"{t}-{r.tag}"
                    c.vendor_pkg = re.sub(rf"^{re.escape(r.unit)}-", f"{t}-", r.vendor_pkg) if r.vendor_pkg else ""
                    c.unit = t
                    c.origin = "트레인복제"
                    c.basis = f"{dwg} 노트 '{pn.copy_text}' (B01)"
                    c.src_tag = r.tag
                    rows.append(c)
        if pn.only_one and concept.on("B03"):
            for r in rows:
                if r.dwg == dwg and r.origin in ("도면인식", "Typical파생", "장비Typical"):
                    r.inform.append("노트 'ONLY ONE IS SHOWN': 대수 확인 필요 (B03)")
                    if r.status == "확정":
                        r.status = "검토필요"

    # ------------------------------------------------ system-level choices
    if str(concept.get("A01", "")) == "DCS 통합":          # one integrated system
        for r in rows:
            if r.system == "ESD":
                r.system = "DCS"
                r.inform.append("ESD→DCS 통합 (A01)")
    if not concept.on("A02"):                               # no separate F&G system
        for r in rows:
            if r.system == "F&G":
                r.system = "DCS"
                r.inform.append("F&G→DCS (A02 OFF)")
    if str(concept.get("A04", "")).startswith("통신") or str(concept.get("A04", "")).startswith("IMCC"):
        moved = 0
        for r in rows:
            if r.origin == "장비Typical" and r.system == "DCS" and r.io in HW_IO:
                r.io = "-"
                r.inform.append("MCC 통신으로 대체 (A04)")
                moved += 1
        if moved:
            for k in range(2 if concept.on("C04") else 1):
                rows.append(Row("", f"MCC-COMM-{'AB'[k]}", "", "COMM", "MCC 통신 포트", "", "", "MCC", "DCS",
                                "Serial/Comm", str(concept.get("A04")), 0, "", 1.0, "검토필요", [], "MCC",
                                origin="장비Typical", basis=f"A04 {concept.get('A04')} · 모터 신호 {moved}점 대체"))
    if str(concept.get("G05", "")).startswith("RTD"):        # RTD wired straight to the DCS card
        for r in rows:
            if r.func == "TT" and r.io == "AI":
                r.io = "TC/RTD"
                r.inform.append("RTD 직결 (G05)")

    # ------------------------------------------------ spares
    counts = {}
    for r in rows:
        if r.kind == "instrument" and r.io in HW_IO and r.system not in ("-", "?"):
            counts[(r.system, r.io)] = counts.get((r.system, r.io), 0) + 1
    for (sys_, io), n in sorted(counts.items()):
        rate = concept.num(SPARE_IDS[io], 0)
        k = math.ceil(n * rate - 1e-9)
        for i in range(k):
            rows.append(Row("", f"{sys_}-{io}-SPARE-{i + 1:03d}", "", "SPARE", f"Spare {io}", "", "", "", sys_, io,
                            "", 0, "", 1.0, "확정", [], "SPARE", origin="스페어",
                            basis=f"{SPARE_IDS[io]} {rate:.0%} × {n}"))
    return rows, log


def _add(row, rows, seen, keep_dupes=False):
    if row.tag in seen and keep_dupes:
        row.inform.append(f"같은 태그가 {seen[row.tag].dwg}에도 있음 - 각각 집계 (B10)")
        row.status = "검토필요"
        rows.append(row)
        return True
    if row.tag in seen:                  # same tag on another sheet (off-page / repeated)
        first = seen[row.tag]
        first.inform.append(f"same tag also on {row.dwg} p{row.page} - repeat or duplicate number?")
        if first.status == "확정":
            first.status = "검토필요"
        return False
    seen[row.tag] = row
    rows.append(row)
    return True
