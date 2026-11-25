"""Concept checklist (engineering assumptions) -> extra index rows.

The checklist workbook is the single source of truth for typicals: the
`Typical_Points` sheet lists one row per IO point, with an optional quantity
taken from a checklist ID and a condition on checklist values. The engine
expands exactly those rows, so whatever the workbook's Typicals summary
shows is what lands in the index - one IO point = one index row.
"""
from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

import openpyxl

PKG_TYPICAL = {"전부 하드와이어": "PKG-HARDWIRED", "벤더PLC+통신": "PKG-COMM", "혼합": "PKG-HYBRID"}


@dataclass
class Point:
    typical: str
    func: str          # pattern: {F} own function code, {L} first letter
    suffix: str
    desc: str
    system: str        # DCS / ESD / F&G / @E04 (take checklist value)
    io: str
    qty: object        # number or checklist ID
    cond: str


class Concept:
    def __init__(self, path: Path):
        self.path = Path(path)
        wb = openpyxl.load_workbook(self.path)          # literal inputs, not formulas
        ws = wb["Concept_Checklist"]
        self.items: dict[str, dict] = {}
        for r in ws.iter_rows(min_row=5, values_only=True):
            if r[0] and re.fullmatch(r"[A-Z]\d{2}", str(r[0])):
                self.items[r[0]] = dict(name=r[1], value=r[3], default=r[4])
        self.points: list[Point] = []
        for r in wb["Typical_Points"].iter_rows(min_row=5, values_only=True):
            if r[0]:
                self.points.append(Point(r[0], str(r[2] or ""), str(r[3] or ""), r[4] or "",
                                         str(r[5] or ""), str(r[6] or ""), r[7] if r[7] is not None else 0,
                                         str(r[8] or "")))
        self.packages = []
        for r in wb["Package_Concept"].iter_rows(min_row=5, values_only=True):
            if r[0] and str(r[0]) != "합계":
                self.packages.append(dict(tag=str(r[0]), dwg=r[1], name=r[2], conn=r[3],
                                          typical=r[4], qty=r[5], note=r[6]))

    # ---------------------------------------------------------- values
    def get(self, iid, default=None):
        it = self.items.get(iid)
        return default if it is None or it["value"] is None else it["value"]

    def on(self, iid) -> bool:
        return str(self.get(iid, "OFF")).upper() == "ON"

    def num(self, iid, default=0.0) -> float:
        try:
            return float(self.get(iid, default))
        except (TypeError, ValueError):
            return default

    def _sval(self, iid):
        v = self.get(iid, "")
        if isinstance(v, float) and v.is_integer():
            v = int(v)
        return str(v)

    def cond_ok(self, cond: str) -> bool:
        if not cond:
            return True
        for c in cond.split("&"):
            c = c.strip()
            op = "<>" if "<>" in c else "="
            k, v = (x.strip() for x in c.split(op, 1))
            if (self._sval(k) == v) != (op == "="):
                return False
        return True

    def qty(self, q) -> int:
        if isinstance(q, (int, float)):
            return int(q)
        return int(self.num(str(q), 0))

    def system(self, s: str) -> str:
        return self._sval(s[1:]) if s.startswith("@") else s

    def changed(self):
        return [(k, v["name"], v["value"], v["default"]) for k, v in self.items.items()
                if v["value"] != v["default"]]

    # ---------------------------------------------------------- expansion
    def expand(self, typical: str, func: str = "", main_system: str | None = None):
        """Yield (is_main, func_code, suffix, desc, system, io, ids) for every
        IO point of a typical under the current checklist. `ids` lists the
        checklist IDs that decided the point (for the index 'basis' column)."""
        pts = [p for p in self.points if p.typical == typical]
        for p in pts:
            if not self.cond_ok(p.cond):
                continue
            n = self.qty(p.qty)
            ids = sorted(set(re.findall(r"[A-Z]\d{2}", p.cond + " " + str(p.qty) + " " + p.system)))
            fc = p.func.replace("{F}", func).replace("{L}", func[:1] if func else "")
            is_main = p.func == "{F}" and p.suffix == ""
            sys_ = main_system if (is_main and main_system) else self.system(p.system)
            for k in range(n):
                sfx = p.suffix + (f"-{k + 1}" if n > 1 else "")
                yield is_main, fc, sfx, p.desc, sys_, p.io, ids

    def package_typical(self, pkg_tag: str):
        """Package_Concept row for this package (matched on the base tag), else C01 default."""
        base = re.match(r"\d{2}-PK-\d{3}", pkg_tag)
        base = base.group(0) if base else pkg_tag
        for p in self.packages:
            if p["tag"].startswith(base):
                t = p["typical"] or PKG_TYPICAL.get(p["conn"], "PKG-HYBRID")
                return t, p["conn"], p, "Package_Concept"
        conn = self.get("C01", "혼합")
        return PKG_TYPICAL.get(conn, "PKG-HYBRID"), conn, None, "C01"
