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
    m = sc.m_per_pt or 0.2                   # size filters only; lengths wait for a confirmed scale
    cols = []
    racks = detect_racks(page, m, cols_out=cols)
    return {
        "racks": racks, "rack_cols": cols, "obstacles": detect_obstacles(page, m),
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


_ROUTERS: dict = {}


def json_key(x):
    import json
    return json.dumps(x, sort_keys=True)


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
        off = set(dec.get("racks_off") or [])
        self.racks_auto = [r + [i in off] for i, r in enumerate(plot.get("racks") or [])]
        self.racks_user = [p for p in (dec.get("racks_add") or []) if len(p) >= 2]
        self.racks_model = [list(r[:4]) for r in (dec.get("racks_model") or [])]   # from the 3D model (STEP 10), pt
        self.racks_model_ids = [str(r[4]) if len(r) > 4 else "" for r in (dec.get("racks_model") or [])]
        self.rack_src = dec.get("rack_src", "PLOT 인식")
        self.eq_model = [list(r[:4]) for r in (dec.get("eq_model") or [])]          # 3D model equipment boxes, pt
        self._router = {}
        self.jb_moves = dict(dec.get("jb_moves") or {})      # STEP 7: {JB no: slot id}
        self.route_edit = {k: orthogonal(v) for k, v in (dec.get("routes_edit") or {}).items() if len(v) >= 2}
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

    def rack_rects(self, width_m=6.0):
        if self.rack_src == "3D 모델 랙" and self.racks_model:
            out = [list(r) for r in self.racks_model]
        else:
            out = [r[:4] for r in self.racks_auto if not r[-1]]
            if self.rack_src == "둘 다":                   # 3D fact > PLOT auto: a PLOT rack only where the model has none
                def cover(r):
                    area = max(1e-6, (r[2] - r[0]) * (r[3] - r[1]))
                    return max((max(0, min(r[2], q[2]) - max(r[0], q[0])) * max(0, min(r[3], q[3]) - max(r[1], q[1])) / area
                                for q in self.racks_model), default=0)
                out = [r for r in out if cover(r) < 0.3] + [list(r) for r in self.racks_model]
        hw = width_m / 2 / self.m_per_pt
        for poly in self.racks_user:
            for (x0, y0), (x1, y1) in zip(poly, poly[1:]):
                if abs(x1 - x0) >= abs(y1 - y0):            # snapped to the nearer axis, like a real rack
                    out.append([min(x0, x1) - hw, y0 - hw, max(x0, x1) + hw, y0 + hw])
                    if abs(y1 - y0) > 1:
                        out.append([x1 - hw, min(y0, y1) - hw, x1 + hw, max(y0, y1) + hw])
                else:
                    out.append([x0 - hw, min(y0, y1) - hw, x0 + hw, max(y0, y1) + hw])
                    if abs(x1 - x0) > 1:
                        out.append([min(x0, x1) - hw, y1 - hw, max(x0, x1) + hw, y1 + hw])
        return out

    def router(self, pref=3.0, width_m=6.0, use_racks=True):
        key = (round(pref, 2), round(width_m, 2), use_racks)
        if key not in self._router:
            k = (str(self.plot.get("_key")), json_key(self.racks_auto), json_key(self.racks_user), self.room,
                 self.m_per_pt, key, self.rack_src, json_key(self.racks_model))
            if k not in _ROUTERS:
                if len(_ROUTERS) > 8:
                    _ROUTERS.clear()
                _ROUTERS[k] = Router(self.plot["size"], self.m_per_pt,
                                     self.rack_rects(width_m) if use_racks else [],
                                     self.plot.get("obstacles") or [], self.room, pref if use_racks else 1.0)
            self._router[key] = _ROUTERS[k]
        return self._router[key]

    def measure(self, poly, width_m=6.0, use_racks=True):
        """A route drawn by hand: walk it every 0.5 m and split rack / off-rack metres like the router does."""
        racks = self.rack_rects(width_m) if use_racks else []
        on = lambda x, y: any(r[0] <= x <= r[2] and r[1] <= y <= r[3] for r in racks)
        g = rk = 0.0
        changes = 0
        prev = None
        first = None
        step = 0.5 / self.m_per_pt
        for (x0, y0), (x1, y1) in zip(poly, poly[1:]):
            seg = ((x1 - x0) ** 2 + (y1 - y0) ** 2) ** 0.5
            n = max(1, int(seg / step))
            for i in range(n):
                x, y = x0 + (x1 - x0) * (i + 0.5) / n, y0 + (y1 - y0) * (i + 0.5) / n
                k = on(x, y)
                if first is None:
                    first = k
                if prev is not None and k != prev:
                    changes += 1
                prev = k
                if k:
                    rk += seg / n * self.m_per_pt
                else:
                    g += seg / n * self.m_per_pt
        return {"ground": g, "rack": rk, "changes": changes, "first_rack": bool(first), "last_rack": bool(prev),
                "poly": [[round(x, 1), round(y, 1)] for x, y in poly]}

    def dist(self, a, b):
        """Manhattan distance in m (cables run along orthogonal routes)."""
        return (abs(a[0] - b[0]) + abs(a[1] - b[1])) * self.m_per_pt


# ------------------------------------------------------------------ pipe racks and obstacles
def detect_racks(page, m_per_pt: float, tol=1.5, cols_out: list | None = None):
    """Pipe racks seen from above look like ladders: two rows of column markers (a ~1 m box
    around an H-beam) at a steady bay (2-13 m), the rows 3-12 m apart. Returns rects in pt:
    [x0, y0, x1, y1, 'H'|'V', bents]. cols_out (if given) gets, per rack, its column centres
    [[x, y], ...] - the faces JBs are mounted on."""
    m = m_per_pt
    cols = []
    for d in page.get_drawings():
        it, r = d["items"], d["rect"]
        if (len(it) == 1 and it[0][0] in ("qu", "re") and 0.5 <= r.width * m <= 1.6 and 0.5 <= r.height * m <= 1.6
                and min(r.width, r.height) / max(r.width, r.height) > 0.65):
            c = ((r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2)
            if not any(abs(c[0] - e[0]) < tol and abs(c[1] - e[1]) < tol for e in cols):
                cols.append(c)
    out = []
    for axis in (0, 1):                 # 0: rack runs along x (bents are vertical pairs)
        a, b = (0, 1) if axis == 0 else (1, 0)
        bents = []
        for c in cols:
            cand = [e for e in cols if e is not c and abs(c[a] - e[a]) < tol and 3 <= (e[b] - c[b]) * m <= 12]
            if cand:
                e = min(cand, key=lambda e: e[b] - c[b])
                bents.append(((c[a] + e[a]) / 2, c[b], e[b]))
        grp = {}
        for x, lo, hi in bents:
            key = next((k for k in grp if abs(k[0] - lo) < tol and abs(k[1] - hi) < tol), (lo, hi))
            grp.setdefault(key, []).append(x)
        for (lo, hi), xs in grp.items():
            xs = sorted(xs)
            run = [xs[0]]
            for x in xs[1:] + [None]:
                if x is not None and (x - run[-1]) * m <= 13:
                    run.append(x)
                    continue
                if len(run) >= 3 and (run[-1] - run[0]) * m >= 10:
                    r = [run[0], lo, run[-1], hi] if axis == 0 else [lo, run[0], hi, run[-1]]
                    out.append([round(v, 1) for v in r] + ["H" if axis == 0 else "V", len(run)])
                    if cols_out is not None:
                        cols_out.append([[round(x, 1), round(lo, 1)] if axis == 0 else [round(lo, 1), round(x, 1)]
                                         for x in run]
                                        + [[round(x, 1), round(hi, 1)] if axis == 0 else [round(hi, 1), round(x, 1)]
                                           for x in run])
                run = [x] if x is not None else []
    return out


def detect_obstacles(page, m_per_pt: float):
    """Equipment, containers and buildings the cable route must go around: boxes (2-80 m)
    with a label or drawing inside, and round tank outlines."""
    m = m_per_pt
    words = [((w[0] + w[2]) / 2, (w[1] + w[3]) / 2) for w in page.get_text("words")]
    pts, cand = [], []
    for d in page.get_drawings():
        it, r = d["items"], d["rect"]
        for x in it:
            if x[0] == "l":
                pts.append(((x[1].x + x[2].x) / 2, (x[1].y + x[2].y) / 2))
        w, h = r.width * m, r.height * m
        if not (2 <= w <= 80 and 2 <= h <= 80):
            continue
        if len(it) == 1 and it[0][0] in ("qu", "re"):
            cand.append((r, "box"))
        elif len(it) >= 6 and abs(r.width - r.height) / max(r.width, r.height) < 0.06 and w <= 40:
            cand.append((r, "round"))
    B = 25.0                                              # spatial buckets: count points inside a box fast
    bins = {}
    for wt, plist in ((10, words), (1, pts)):
        for p in plist:
            bins.setdefault((int(p[0] // B), int(p[1] // B)), []).append((p, wt))

    def inside(rect):
        n = 0
        for bx in range(int(rect.x0 // B), int(rect.x1 // B) + 1):
            for by in range(int(rect.y0 // B), int(rect.y1 // B) + 1):
                for p, wt in bins.get((bx, by), ()):
                    if rect.x0 < p[0] < rect.x1 and rect.y0 < p[1] < rect.y1:
                        n += wt
                        if n >= 8:
                            return n
        return n
    out = []
    for r, kind in cand:
        inner = fitz.Rect(r.x0 + r.width * 0.1, r.y0 + r.height * 0.1, r.x1 - r.width * 0.1, r.y1 - r.height * 0.1)
        if kind == "box" and inside(inner) < 8:
            continue
        q = [round(r.x0, 1), round(r.y0, 1), round(r.x1, 1), round(r.y1, 1), kind]
        if q not in out:
            out.append(q)
    return out


def orthogonal(poly):
    """Cable routes run along x / y only: an inclined step becomes an L (along the longer axis first),
    repeated and collinear points are dropped."""
    pts = [tuple(map(float, p)) for p in poly]
    out = [pts[0]] if pts else []
    for q in pts[1:]:
        a = out[-1]
        if abs(a[0] - q[0]) > 0.05 and abs(a[1] - q[1]) > 0.05:
            out.append((q[0], a[1]) if abs(q[0] - a[0]) >= abs(q[1] - a[1]) else (a[0], q[1]))
        out.append(q)
    clean = []
    for p in out:
        if clean and abs(p[0] - clean[-1][0]) < 0.05 and abs(p[1] - clean[-1][1]) < 0.05:
            continue
        if len(clean) >= 2:
            a, b = clean[-2], clean[-1]
            if (abs(a[0] - b[0]) < 0.05 and abs(b[0] - p[0]) < 0.05) or (abs(a[1] - b[1]) < 0.05 and abs(b[1] - p[1]) < 0.05):
                clean[-1] = p
                continue
        clean.append(p)
    return [[round(x, 1), round(y, 1)] for x, y in clean]


class Router:
    """Shortest cable route to the control room on a 1 m grid: racks cost 1 per metre, open ground
    `pref` per metre (so racks are used when they are near), obstacles are blocked. One Dijkstra from
    the room serves every source."""

    def __init__(self, size_pt, m_per_pt, racks, obstacles, room, pref=3.0, cell=1.0):
        import heapq
        self.m, self.cell = m_per_pt, cell
        W, H = int(size_pt[0] * m_per_pt / cell) + 1, int(size_pt[1] * m_per_pt / cell) + 1
        self.W, self.H = W, H
        self.kind = bytearray(W * H)              # 0 ground, 1 rack, 2 blocked
        f = m_per_pt / cell

        def cells(r, shrink=0.0):
            x0, y0, x1, y1 = (int(r[0] * f + shrink), int(r[1] * f + shrink),
                              int(r[2] * f - shrink), int(r[3] * f - shrink))
            for y in range(max(0, y0), min(H, y1 + 1)):
                for x in range(max(0, x0), min(W, x1 + 1)):
                    yield y * W + x
        for o in obstacles:
            for i in cells(o, 0.3 / cell):
                self.kind[i] = 2
        for rk in racks:
            for i in cells(rk):
                self.kind[i] = 1                   # a rack over an equipment box still passes
        self.goal = self.free(room)
        INF = float("inf")
        dist = [INF] * (W * H)
        self.par = [-1] * (W * H)
        dist[self.goal] = 0.0
        pq = [(0.0, self.goal)]
        while pq:
            d, i = heapq.heappop(pq)
            if d > dist[i]:
                continue
            x, y = i % W, i // W
            for nx, ny in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
                if 0 <= nx < W and 0 <= ny < H:
                    j = ny * W + nx
                    k = self.kind[j]
                    if k == 2:
                        continue
                    nd = d + cell * (1.0 if k == 1 else pref)
                    if nd < dist[j]:
                        dist[j] = nd
                        self.par[j] = i
                        heapq.heappush(pq, (nd, j))
        self.dist = dist

    def free(self, p):
        """grid cell of p; if p is inside an obstacle (IRP in a building, room itself) the nearest open cell."""
        f = self.m / self.cell
        x, y = min(self.W - 1, max(0, int(p[0] * f))), min(self.H - 1, max(0, int(p[1] * f)))
        if self.kind[y * self.W + x] != 2:
            return y * self.W + x
        for rad in range(1, 200):
            for dx in range(-rad, rad + 1):
                for dy in (-rad, rad) if abs(dx) != rad else range(-rad, rad + 1):
                    nx, ny = x + dx, y + dy
                    if 0 <= nx < self.W and 0 <= ny < self.H and self.kind[ny * self.W + nx] != 2:
                        return ny * self.W + nx
        return y * self.W + x

    def route(self, p):
        """-> (metres on ground, metres on rack, level changes ground<->rack, polyline in pt) or None."""
        i = self.free(p)
        if self.dist[i] == float("inf"):
            return None
        g = r = 0.0
        changes = 0
        prev = None
        pts = []
        f = self.cell / self.m
        while i != -1:
            k = self.kind[i]
            if prev is not None:
                if k == 1:
                    r += self.cell
                else:
                    g += self.cell
                if (k == 1) != (prev == 1):
                    changes += 1
            pts.append(((i % self.W + 0.5) * f, (i // self.W + 0.5) * f))
            prev = k
            i = self.par[i]
        first_rack = self.kind[self.free(p)] == 1
        # keep corners only
        poly = [pts[0]]
        for a, b, c in zip(pts, pts[1:], pts[2:]):
            if (b[0] - a[0]) * (c[1] - b[1]) != (b[1] - a[1]) * (c[0] - b[0]):
                poly.append(b)
        poly.append(pts[-1])
        return {"ground": g, "rack": r, "changes": changes, "first_rack": first_rack,
                "last_rack": self.kind[self.goal] == 1, "poly": [[round(x, 1), round(y, 1)] for x, y in poly]}
