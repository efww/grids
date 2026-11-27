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

from .directopen import load_workbook

PKG_TYPICAL = {"전부 하드와이어": "PKG-HARDWIRED", "벤더PLC+통신": "PKG-COMM", "혼합": "PKG-HYBRID"}

# How far each checklist item is wired into the calculation. Items not listed
# here change the IO. 'info' items are recorded only; 'planned' items need a
# reading the tool cannot make yet and are shown as 미적용 so nobody expects
# the numbers to move.
ITEM_STATUS = {
    "C02": ("info", "기록용 - IO 수량에 영향 없음"),
    "C03": ("info", "케이블로 연결 - STEP 6 '패키지 PLC 통신 케이블'이 이 값을 따름 (IO 수량에는 영향 없음)"),
    "G01": ("info", "기록용 - Fieldbus 세그먼트 집계는 미구현"),
    "B02": ("planned", "미적용 - 노트 없는 시트의 트레인 태그 규칙 정의 필요"),
    "B04": ("planned", "미적용 - HOLD 표기와 계기 연결 인식 필요"),
    "B05": ("planned", "미적용 - FUTURE/SPARE 표기 인식 필요"),
    "D02": ("planned", "미적용 - 장비리스트(모터 kW) 연동 필요"),
    "D05": ("planned", "미적용 - Standby 기동 Typical 정의 필요"),
    "D07": ("planned", "미적용 - 팬 장비 인식 필요"),
    "G04": ("planned", "미적용 - 유량계 통신 Typical 정의 필요"),
    "H06": ("info", "케이블로 연결 - STEP 6 IS / Non-IS(JB·멀티·트레이 분리)가 이 값을 따름. '위험지역 계기 IS'는 "
                    "위험지역 구분도가 없어 전부 Non-IS로 계산 (가정). IO 수량에는 영향 없음"),
}


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
        wb = load_workbook(self.path)                   # literal inputs; labelled files are opened through Excel
        ws = wb["Concept_Checklist"]
        self.items: dict[str, dict] = {}
        group = ""
        for r in ws.iter_rows(min_row=5, values_only=True):
            if r[0] and re.fullmatch(r"[A-Z]\d{2}", str(r[0])):
                choices = [c.strip() for c in str(r[5] or "").split(" / ") if c.strip()]
                st = ITEM_STATUS.get(r[0], ("applied", ""))
                self.items[r[0]] = dict(name=r[1], type=r[2], value=r[3], default=r[4],
                                        choices=choices, effect=r[6] or "", basis=r[7] or "", group=group,
                                        status=st[0], status_note=st[1])
            elif r[0]:
                group = str(r[0])
        self.points: list[Point] = []
        for r in wb["Typical_Points"].iter_rows(min_row=5, values_only=True):
            if r[0]:
                self.points.append(Point(r[0], str(r[2] or ""), str(r[3] or ""), r[4] or "",
                                         str(r[5] or ""), str(r[6] or ""), r[7] if r[7] is not None else 0,
                                         str(r[8] or "")))
        if not any(p.typical == "PKG-HARDWIRED" and p.io not in ("-", "") for p in self.points):
            # older checklists: "전부 하드와이어" had no common points, so it counted fewer IO than "혼합".
            # Fully hard-wired keeps the run/trip/start signals as hard-wired points (C05~C08).
            import dataclasses
            hyb = [p for p in self.points if p.typical == "PKG-HYBRID" and p.io != "COMM"]
            self.points = [p for p in self.points if p.typical != "PKG-HARDWIRED"] + \
                [dataclasses.replace(p, typical="PKG-HARDWIRED") for p in hyb]
        self.packages = []
        for r in wb["Package_Concept"].iter_rows(min_row=5, values_only=True):
            if r[0] and str(r[0]) != "합계":
                self.packages.append(dict(tag=str(r[0]), dwg=r[1], name=r[2], conn=r[3],
                                          typical=r[4], qty=r[5], note=r[6]))

    # ---------------------------------------------------------- editing
    def set(self, iid, value):
        """Change a setting in memory (numbers kept numeric)."""
        if iid not in self.items:
            raise KeyError(iid)
        it = self.items[iid]
        if it["type"] == "숫자":
            value = float(value)
            if value.is_integer() and not isinstance(it["default"], float):
                value = int(value)
        it["value"] = value

    def save(self, path: Path | None = None):
        """Write settings and package rows back into the checklist workbook."""
        path = Path(path or self.path)
        wb = load_workbook(self._save_base())
        ws = wb["Concept_Checklist"]
        for row in ws.iter_rows(min_row=5):
            k = row[0].value
            if k in self.items:
                row[3].value = self.items[k]["value"]
        wp = wb["Package_Concept"]
        rows = [r for r in wp.iter_rows(min_row=5) if str(r[0].value or "") != "합계"]
        total_row = next((r for r in wp.iter_rows(min_row=5) if str(r[0].value or "") == "합계"), None)
        limit = total_row[0].row if total_row else 10 ** 6
        free = [r for r in rows if not r[0].value and r[0].row < limit]
        by_tag = {str(r[0].value): r for r in rows if r[0].value}
        for p in self.packages:
            r = by_tag.get(p["tag"]) or (free.pop(0) if free else None)
            if r is None:
                continue
            for k, col in (("tag", 0), ("dwg", 1), ("name", 2), ("conn", 3), ("typical", 4), ("qty", 5), ("note", 6)):
                r[col].value = p.get(k)
        wb.save(path)
        self.path = path
        try:                                            # plain JSON copy of the inputs
            from .protect import save_values
            save_values(self, path.parent)
        except OSError:
            pass

    def _save_base(self) -> Path:
        """Workbook to write into. A labelled checklist read through Excel keeps the values but not
        the drop-downs/colours; when its Typical_Points match the default, write into the
        formatted default template instead."""
        from .directopen import is_locked
        default = Path(__file__).resolve().parent.parent / "protocols" / "concept_default.xlsx"
        if not is_locked(self.path) or not default.exists() or is_locked(default):
            return self.path
        try:
            if Concept(default).points == self.points:
                return default
        except Exception:
            pass
        return self.path

    def apply_values(self, data: dict) -> int:
        """Put back inputs saved by protect.save_values. Returns how many items differ from default."""
        n = 0
        for k, v in (data.get("items") or {}).items():
            if k in self.items:
                self.items[k]["value"] = v
                n += v != self.items[k]["default"]
        for p in data.get("packages") or []:
            if p.get("tag"):
                q = next((x for x in self.packages if x["tag"] == p["tag"]), None)
                if q is None:
                    self.packages.append(dict(p))
                else:
                    q.update(p)
        return n

    def set_package(self, tag, conn=None, qty=None, typical=None, **info):
        p = next((x for x in self.packages if x["tag"] == tag), None)
        if p is None:
            p = dict(tag=tag, dwg=info.get("dwg"), name=info.get("name"), conn=None, typical=None,
                     qty=None, note=info.get("note"))
            self.packages.append(p)
        if conn is not None:
            p["conn"] = conn
            p["typical"] = typical or PKG_TYPICAL.get(conn)
        if qty is not None:
            p["qty"] = int(qty) if qty not in ("", None) else None

    def alternatives(self, iid):
        it = self.items[iid]
        if it["type"] == "ON/OFF":
            return ["OFF" if str(it["value"]).upper() == "ON" else "ON"]
        if it["type"] == "선택":
            return [c for c in it["choices"] if c != str(it["value"])]
        return []

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
