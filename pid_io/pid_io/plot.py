"""Plot plan reader (vector PDF): scale, equipment positions, room / panel labels.

Stage 3 of the cable module starts here. Nothing is final: every value
carries the evidence it came from, so the review screen can ask the
engineer only about what is uncertain.

Scale, in order of trust:
  1. title block text   "1:600" / "1/600"          -> paper scale
  2. dimension texts     "4,500" between extension lines (>= 2 that agree)
  3. reference plot      an older plot with a title-block scale; same equipment
                         drawn in both -> size ratio of the equipment outlines
  4. ask                 two points and a distance (review card)
"""
from __future__ import annotations

import re
import statistics
from dataclasses import dataclass, field
from pathlib import Path

import pymupdf as fitz

PT_MM = 25.4 / 72                      # one PDF point on paper, mm
TAG = re.compile(r"^(\d{2})-([A-Z]{1,4})-(\d{3})([A-Z](?:/[A-Z])*)?$")
BARE = re.compile(r"^([A-Z]{1,4})-(\d{3})([A-Z])?$")
ROOM_WORDS = ("MCB", "CCR", "LER", "FAR", "CONTROL", "SUBSTATION", "SUB-STATION", "ELECTRICAL", "E-HOUSE", "EHOUSE")
PANEL_WORDS = ("MCC", "IRP", "SCADA", "RCP", "PDP", "UPS")


@dataclass
class Scale:
    m_per_pt: float | None
    method: str                         # title / dimension / reference / unknown
    evidence: list = field(default_factory=list)
    confident: bool = False


# ------------------------------------------------------------------ scale
def title_scale(page) -> Scale | None:
    words = page.get_text("words")
    hits = []
    for w in words:
        m = re.fullmatch(r"1\s*[:/]\s*(\d{2,5})", w[4])
        if m:
            hits.append((int(m.group(1)), w))
    if not hits:
        return None
    # several scales on a sheet (details, keys): prefer the one next to a SCALE label
    lab = [w for w in words if "SCALE" in w[4].upper()]

    def near(w):
        return min((abs(w[0] - l[0]) + abs(w[1] - l[1]) for l in lab), default=1e9)

    n, w = min(hits, key=lambda h: near(h[1]))
    return Scale(n * PT_MM / 1000, "title", [f"도면 축척 표기 '1:{n}'"], True)


def _segments(page):
    hv, vv = [], []
    for d in page.get_drawings():
        for it in d["items"]:
            if it[0] != "l":
                continue
            a, b = it[1], it[2]
            dx, dy = abs(a.x - b.x), abs(a.y - b.y)
            if max(dx, dy) < 5:                                # glyph strokes of outlined text
                continue
            if dy <= max(0.3, 0.03 * dx) and dx > 1:          # near-horizontal (plot drivers wobble a little)
                hv.append((min(a.x, b.x), max(a.x, b.x), (a.y + b.y) / 2))
            elif dx <= max(0.3, 0.03 * dy) and dy > 1:
                vv.append((min(a.y, b.y), max(a.y, b.y), (a.x + b.x) / 2))
    return hv, vv


def dimension_scale(page, tol=0.05) -> Scale | None:
    """A dimension value sits centred between two extension lines (perpendicular
    to the reading direction). Value / gap = mm per point; keep the largest group
    of dimensions that agree within tol."""
    hv, vv = _segments(page)
    found = []
    for w in page.get_text("words"):
        if not re.fullmatch(r"\d{1,3}(,\d{3})+|\d{3,6}", w[4]):
            continue
        val = int(w[4].replace(",", ""))
        if not 300 <= val <= 300000:
            continue
        cx, cy = (w[0] + w[2]) / 2, (w[1] + w[3]) / 2
        vertical_text = (w[3] - w[1]) > (w[2] - w[0])
        best = None
        if not vertical_text:            # extension lines are vertical, left and right of the text
            near = [x for (y0, y1, x) in vv if y0 - 30 <= cy <= y1 + 30 or abs(y0 - cy) < 30 or abs(y1 - cy) < 30]
            L = sorted(x for x in near if x < cx - 1)[-6:]
            R = sorted(x for x in near if x > cx + 1)[:6]
        else:
            near = [y for (x0, x1, y) in hv if x0 - 30 <= cx <= x1 + 30 or abs(x0 - cx) < 30 or abs(x1 - cx) < 30]
            L = sorted(y for y in near if y < cy - 1)[-6:]
            R = sorted(y for y in near if y > cy + 1)[:6]
        c = cy if vertical_text else cx
        for a in L:
            for b in R:
                gap = b - a
                if gap < 5 or abs((a + b) / 2 - c) > 0.2 * gap:
                    continue
                if best is None or gap < best:
                    best = gap
        if best:
            found.append((w[4], val, best, val / best / 1000))
    if len(found) < 2:
        return None
    groups = []
    for f in found:
        g = [x for x in found if abs(x[3] - f[3]) / f[3] <= tol]
        groups.append(g)
    g = max(groups, key=len)
    if len(g) < 2:
        return Scale(None, "dimension", [f"치수 {x[0]}: {x[3]:.3f} m/pt" for x in found], False)
    m = statistics.median(x[3] for x in g)
    return Scale(m, "dimension", [f"치수 {x[0]} = 도면 {x[2]:.1f}pt → {x[3]:.3f} m/pt" for x in g], len(g) >= 2)


def _outline_sizes(page, labels: dict[str, tuple]):
    """Largest near-round/square closed shape next to each equipment label -> width in pt."""
    shapes = []
    for d in page.get_drawings():
        r = d["rect"]
        if r.width > 15 and abs(r.width - r.height) / max(r.width, r.height) < 0.05 and len(d["items"]) >= 6:
            shapes.append(r)
    out = {}
    for tag, (x, y) in labels.items():
        near = [r for r in shapes if r.width < 400 and
                abs((r.x0 + r.x1) / 2 - x) < r.width * 1.2 and abs((r.y0 + r.y1) / 2 - y) < r.width * 1.2]
        if near:
            out[tag] = max(r.width for r in near)
    return out


def reference_scale(page, ref_page, ref_scale: Scale) -> Scale | None:
    """Same equipment in an older plot whose scale is known: compare outline sizes."""
    if not ref_scale or not ref_scale.m_per_pt:
        return None
    a = {_bare(t): p for t, p in equipment(page).items()}
    b = {_bare(t): p for t, p in equipment(ref_page).items()}
    common = {t: a[t][0] for t in a if t in b}
    sa = _outline_sizes(page, common)
    sb = _outline_sizes(ref_page, {t: b[t][0] for t in common})
    ratios = sorted((sb[t] / sa[t], t) for t in sa if t in sb and sa[t] > 10)
    if not ratios:
        return None
    groups = [[x for x in ratios if abs(x[0] - r[0]) / r[0] < 0.05] for r in ratios]
    g = max(groups, key=len)
    k = statistics.median(x[0] for x in g)
    return Scale(ref_scale.m_per_pt * k, "reference",
                 [f"{t}: 이전 도면 대비 크기 비 {r:.3f}" for r, t in g] + ref_scale.evidence, len(g) >= 2)


def detect_scale(page, ref_page=None) -> Scale:
    s = title_scale(page)
    if s:
        return s
    d = dimension_scale(page)
    r = None
    if ref_page is not None:
        r = reference_scale(page, ref_page, title_scale(ref_page) or dimension_scale(ref_page))
    if d and d.confident and r and r.m_per_pt:
        agree = abs(d.m_per_pt - r.m_per_pt) / d.m_per_pt < 0.05
        d.evidence += [("이전 도면 비교와 일치: " if agree else "이전 도면 비교와 다름: ") + f"{r.m_per_pt:.3f} m/pt"] + r.evidence
        d.confident = agree
        return d
    if d and d.confident:
        return d
    if r and r.m_per_pt:
        return r
    return d or Scale(None, "unknown", ["축척 표기·치수·참고 도면에서 축척을 찾지 못함"], False)


# ------------------------------------------------------------------ labels
def _bare(tag: str) -> str:
    return re.sub(r"^\d{2}-", "", tag)


def equipment(page) -> dict[str, list]:
    """tag -> [(x, y), ...] centre of each equipment tag text (same tag may repeat)."""
    out = {}
    for w in page.get_text("words"):
        t = w[4]
        if TAG.match(t) or BARE.match(t):
            out.setdefault(t, []).append(((w[0] + w[2]) / 2, (w[1] + w[3]) / 2))
    return out


def labels(page, keys) -> list[dict]:
    out = []
    for b in page.get_text("blocks"):
        txt = " ".join(b[4].split())
        up = txt.upper()
        if any(re.search(rf"\b{re.escape(k)}\b", up) for k in keys):
            out.append({"text": txt, "x": (b[0] + b[2]) / 2, "y": (b[1] + b[3]) / 2,
                        "box": [round(v, 1) for v in b[:4]]})
    return out


def read_plot(pdf: Path, ref_pdf: Path | None = None) -> dict:
    doc = fitz.open(pdf)
    page = doc[0]
    ref = fitz.open(ref_pdf)[0] if ref_pdf else None
    sc = detect_scale(page, ref)
    eq = equipment(page)
    return {
        "pdf": Path(pdf).name, "size": [page.rect.width, page.rect.height],
        "scale": {"m_per_pt": sc.m_per_pt, "method": sc.method, "evidence": sc.evidence, "confident": sc.confident},
        "equipment": {t: [[round(x, 1), round(y, 1)] for x, y in v] for t, v in eq.items()},
        "rooms": labels(page, ROOM_WORDS),
        "panels": labels(page, PANEL_WORDS),
    }


if __name__ == "__main__":                       # python -m pid_io.plot PLOT.pdf [REF.pdf]
    import json
    import sys
    print(json.dumps(read_plot(Path(sys.argv[1]), Path(sys.argv[2]) if len(sys.argv) > 2 else None),
                     ensure_ascii=False, indent=1)[:4000])


# ------------------------------------------------------------------ layout: where each index row sits on the plot
KEY = re.compile(r"^(\d{2})-([A-Z]{1,4})-(\d{3})")


def _keys(tag: str):
    """'10-PK-301A/B' -> ['10-PK-301A', '10-PK-301B', '10-PK-301'];  '10-PK-6021' -> ['10-PK-602'] ...
    Package numbers in the index carry a unit letter or digit after the 3-digit number."""
    tag = (tag or "").strip().replace("PKG-", "PK-")
    m = KEY.match(tag)
    if not m:
        return []
    base = m.group(0)
    rest = tag[len(base):]
    out = [base + s for s in rest.split("/") if s and s.isalpha() and len(s) == 1] if rest else []
    if rest and "/" in rest:
        first = rest.split("/")[0]
        out = [base + first] + [base + s for s in rest.split("/")[1:]]
    elif rest and rest.isalpha() and len(rest) == 1:
        out = [base + rest]
    return out + [base]


def _reading_order(pts, tol=25):
    """top-to-bottom rows (within tol pt), left-to-right inside a row."""
    pts = sorted(pts, key=lambda p: p[1])
    rows, cur = [], []
    for p in pts:
        if cur and abs(p[1] - cur[0][1]) > tol:
            rows.append(cur)
            cur = []
        cur.append(p)
    if cur:
        rows.append(cur)
    return [p for r in rows for p in sorted(r, key=lambda q: q[0])]


class Layout:
    """Plot positions for rooms, panels and index rows. Coordinates in plot pt; m = pt * m_per_pt."""

    def __init__(self, plot: dict, page_texts: dict | None = None, units: list[str] | None = None,
                 decisions: dict | None = None):
        dec = decisions or {}
        self.plot = plot
        self.m_per_pt = dec.get("m_per_pt") or plot["scale"]["m_per_pt"]
        self.pos, self.guessed = {}, set()             # key -> (x, y)
        units = sorted(set(units or []))
        for tag, pts in plot["equipment"].items():
            pts = [tuple(p) for p in pts]
            m = KEY.match(tag.replace("PKG-", "PK-"))
            if len(pts) > 1 and m and units:
                # one label drawn per train with the first train's prefix: spread over the trains in reading order
                u0 = m.group(1)
                later = [u for u in units if u >= u0]
                for u, p in zip(later, _reading_order(pts)):
                    for k in _keys(u + tag[2:]):
                        self.pos.setdefault(k, p)
                        if u != u0:
                            self.guessed.add(k)
            for k in _keys(tag):
                self.pos.setdefault(k, pts[0])
        # P&ID sheet -> equipment printed on it (anchor for instruments without Equipment No)
        self.sheet_eq = {}
        for dwg, val in (page_texts or {}).items():
            text = val[1] if isinstance(val, (list, tuple)) else str(val)
            tags = {t for t in re.findall(r"\b\d{2}-[A-Z]{1,4}-\d{3}[A-Z]?\b", text)}
            self.sheet_eq[dwg] = sorted(tags)
        self.room = tuple(dec["room"]) if dec.get("room") else self._label_pos(plot["rooms"], ("CCR", "MCB", "CONTROL"))
        self.irp = tuple(dec["irp"]) if dec.get("irp") else self._label_pos(plot["panels"], ("IRP",)) or \
            self._label_pos(plot["panels"], ("MCC",))
        self.room_src = "사용자 지정" if dec.get("room") else ("도면 글자" if self.room else "")
        self.irp_src = "사용자 지정" if dec.get("irp") else ("도면 글자" if self.irp else "")

    @staticmethod
    def _label_pos(labels, words):
        for w in words:
            hit = [l for l in labels if re.search(rf"\b{w}\b", l["text"].upper())]
            if hit:
                return (statistics.mean(l["x"] for l in hit), statistics.mean(l["y"] for l in hit))
        return None

    def _find(self, tag, unit=""):
        for k in _keys(tag):
            if k in self.pos:
                return self.pos[k], ("추정(반복 배치)" if k in self.guessed else "장비")
        return None, ""

    def _centroid(self, tags, unit=""):
        pts = []
        for t in tags:
            if unit and KEY.match(t) and t[:2] != unit:
                t = unit + t[2:]                     # sheet drawn for the first train, row belongs to another
            p, _ = self._find(t)
            if p:
                pts.append(p)
        if not pts:
            return None
        return (statistics.mean(p[0] for p in pts), statistics.mean(p[1] for p in pts))

    def unit_anchor(self, unit):
        pts = [p for k, p in self.pos.items() if k[:2] == unit]
        if not pts:
            return None
        return (statistics.mean(p[0] for p in pts), statistics.mean(p[1] for p in pts))

    def locate(self, row):
        """-> ((x, y) or None, source)"""
        for t in (row.equip, row.vendor_pkg, row.src_tag if KEY.match(row.src_tag or "") else ""):
            p, src = self._find(t) if t else (None, "")
            if p:
                return p, src
        p = self._centroid(self.sheet_eq.get(row.dwg, []), row.unit)
        if p:
            return p, f"추정({row.dwg} 시트 장비 중심)"
        p = self.unit_anchor(row.unit)
        if p:
            return p, f"추정(유닛 {row.unit} 장비 중심)"
        return None, "위치 미정"

    def dist(self, a, b):
        """Manhattan distance in m (cables run along orthogonal routes)."""
        return (abs(a[0] - b[0]) + abs(a[1] - b[1])) * self.m_per_pt
