"""Turn extracted symbols into index rows with IO, using io_rules.yaml."""
from __future__ import annotations

import re
from dataclasses import dataclass, field

from .extract import Symbol

SYSTEM_NAMES = {"FGS": "F&G", "PLC": "PKG PLC"}       # -> template "Lists" names
IO_NAMES = {"DO-SOV": "DO", "DI(MCC)": "DI", "DO(MCC)": "DO"}
LOCATION = {"FIELD": "Field", "DCS_SHARED": "CCR", "PANEL_CR": "CCR",
            "PANEL_BEHIND": "Local Panel", "PLC": "Local Panel"}


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
    io_detail: str
    signal: str
    page: int
    coord: str
    conf: float
    status: str
    inform: list = field(default_factory=list)
    symbol_cls: str = ""
    derived: bool = False
    kind: str = "instrument"      # instrument | function (DCS soft tag) | implied | equipment


def _rule_for(rules, func):
    for r in rules:
        if re.match(r["match"], func):
            return r
    return rules[-1]


def assign(symbols: list[Symbol], io_proto: dict, confirm=0.95, review=0.80) -> list[Row]:
    rules = io_proto["rules"]
    by_sid = {s.sid: s for s in symbols}
    rows: list[Row] = []
    seen: dict[str, Row] = {}

    typicals = io_proto.get("equipment_typicals", {})
    for s in symbols:
        if s.role == "interlock":
            continue                       # interlocks are logic, not instruments
        if s.role == "equipment":
            kind = s.func.split(":")[1]
            for t in typicals.get(kind, []):
                tag = f"{t['tag']}-{s.number}"
                cio = list(t.get("io", []))
                rows.append(Row(unit=s.number[:2], tag=tag, loop=s.number, func=t["tag"], desc=t["desc"],
                                dwg=s.dwg, title=s.title, location="MCC",
                                system=SYSTEM_NAMES.get(t["system"], t["system"]),
                                io="/".join(IO_NAMES.get(x, x) for x in cio) or "-",
                                io_detail="/".join(cio) or "-", signal=t.get("signal", ""),
                                page=s.page, coord="", conf=0.9, status="검토필요",
                                inform=[f"motor typical for pump {s.number}"], symbol_cls="EQUIPMENT",
                                derived=True, kind="equipment"))
            continue
        rule = _rule_for(rules, s.func)
        inform = list(s.notes)

        system = rule.get("system", "?")
        esd_links = [by_sid[l] for l in s.links if l in by_sid and by_sid[l].cls == "ESD_INTERLOCK"]
        if system == "link":
            system = "ESD" if esd_links else "DCS"
            if esd_links:
                inform.append("ESD interlock " + ",".join(sorted({f"{i.func}-{i.number}" for i in esd_links})))
        kind = "instrument"
        if s.role == "function" and s.system not in ("FIELD",):
            # a DCS/PLC balloon decides the system itself; no physical IO
            system = {"DCS": "DCS", "PLC": "PLC", "LOCAL": "-"}.get(s.system, system)
            io = []
            kind = "function"
        else:
            io = list(rule.get("io", []))
        if rule.get("review"):
            inform.append("function code not in IO protocol")
        system = SYSTEM_NAMES.get(system, system)

        conf = s.score
        if rule.get("review") or system == "?":
            status = "검토필요"
        elif conf >= confirm and not s.notes:
            status = "확정"
        elif conf >= review:
            status = "검토필요"
        else:
            status = "불명"

        base = dict(unit=s.unit, func=s.func, dwg=s.dwg, title=s.title,
                    location=LOCATION.get(s.cls, ""), page=s.page,
                    coord=f"{round(s.cx)},{round(s.cy)}", conf=round(conf, 3),
                    symbol_cls=s.cls)
        loop = f"{s.func[0]}-{s.number}" if s.number else ""
        io_main = "/".join(IO_NAMES.get(x, x) for x in io) or "-"
        row = Row(tag=s.tag, loop=loop, desc=rule.get("desc", ""), system=system,
                  io=io_main, io_detail="/".join(io) or "-", signal=rule.get("signal", ""),
                  status=status, inform=inform, kind=kind, **base)

        if s.tag in seen:                  # same tag on another sheet (off-page / repeated)
            first = seen[s.tag]
            first.inform.append(f"same tag also on {s.dwg} p{s.page} - repeat or duplicate number?")
            if first.status == "확정":
                first.status = "검토필요"
            continue
        seen[s.tag] = row
        rows.append(row)

        for ch in rule.get("children", []) if s.role == "device" else []:
            ctag_func = ch["tag"].replace("{L}", s.func[0])
            ctag = f"{s.unit}-{ctag_func}-{s.number}" if s.unit else f"{ctag_func}-{s.number}"
            cio = list(ch.get("io", []))
            rows.append(Row(tag=ctag, loop=loop, desc=ch["desc"], system=system,
                            io="/".join(IO_NAMES.get(x, x) for x in cio) or "-",
                            io_detail="/".join(cio) or "-", signal=ch.get("signal", ""),
                            status=status, inform=[f"typical from {s.tag}"], derived=True,
                            **{**base, "func": ctag_func}))

    # implied tags (e.g. thermowell for every temperature point)
    present = {(r.dwg, r.func, r.loop.split("-")[-1]) for r in rows}
    have = {r.tag for r in rows}
    for imp in io_proto.get("implied", []):
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
            rows.append(Row(unit=r.unit, tag=tag, loop=r.loop, func=imp["tag"], desc=imp["desc"],
                            dwg=r.dwg, title=r.title, location=r.location, system=imp["system"],
                            io="-", io_detail="-", signal=imp["signal"], page=r.page, coord=r.coord,
                            conf=r.conf, status="검토필요", inform=[f"implied by {r.tag}"],
                            symbol_cls="IMPLIED", derived=True, kind="implied"))
    return rows
