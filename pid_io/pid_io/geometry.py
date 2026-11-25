"""Vector primitive handling for P&ID PDFs.

Every drawing on a page is flattened into sample points (in *display*
coordinates, i.e. after the page rotation is applied) so that a symbol
can be judged by "which outline shapes surround this tag text".
CAD exports often split one circle or diamond into many separate paths,
so we never rely on a single path being a complete shape.
"""
from __future__ import annotations

import math
from dataclasses import dataclass, field

import pymupdf as fitz


@dataclass
class Seg:
    pts: list[tuple[float, float]]      # sampled points along the segment
    a: tuple[float, float]               # start point
    b: tuple[float, float]               # end point
    dashed: bool
    width: float
    kind: str                            # 'l' line, 'c' curve


@dataclass
class PageGeom:
    segs: list[Seg]
    cell: float = 20.0
    grid: dict = field(default_factory=dict)   # (ix,iy) -> list[(x,y,seg_idx)]

    def build(self):
        for i, s in enumerate(self.segs):
            for x, y in s.pts:
                self.grid.setdefault((int(x // self.cell), int(y // self.cell)), []).append((x, y, i))
        return self

    def points_near(self, cx, cy, r):
        out = []
        c = self.cell
        for ix in range(int((cx - r) // c), int((cx + r) // c) + 1):
            for iy in range(int((cy - r) // c), int((cy + r) // c) + 1):
                for x, y, i in self.grid.get((ix, iy), ()):
                    if abs(x - cx) <= r and abs(y - cy) <= r:
                        out.append((x - cx, y - cy, i))
        return out


def _bezier(p0, p1, p2, p3, n=10):
    out = []
    for k in range(n + 1):
        t = k / n
        mt = 1 - t
        x = mt**3 * p0[0] + 3 * mt * mt * t * p1[0] + 3 * mt * t * t * p2[0] + t**3 * p3[0]
        y = mt**3 * p0[1] + 3 * mt * mt * t * p1[1] + 3 * mt * t * t * p2[1] + t**3 * p3[1]
        out.append((x, y))
    return out


def _line_pts(a, b, step=1.0):
    L = math.dist(a, b)
    n = max(1, int(L / step))
    return [(a[0] + (b[0] - a[0]) * k / n, a[1] + (b[1] - a[1]) * k / n) for k in range(n + 1)]


def load_page_geom(page: fitz.Page) -> PageGeom:
    m = page.rotation_matrix
    segs: list[Seg] = []

    def T(p):
        q = fitz.Point(p) * m
        return (q.x, q.y)

    for d in page.get_drawings():
        dashed = bool(d.get("dashes")) and d.get("dashes") not in ("[] 0", "")
        w = d.get("width") or 0.0
        for it in d["items"]:
            op = it[0]
            if op == "l":
                a, b = T(it[1]), T(it[2])
                segs.append(Seg(_line_pts(a, b), a, b, dashed, w, "l"))
            elif op == "c":
                pts = [T(p) for p in it[1:5]]
                segs.append(Seg(_bezier(*pts), pts[0], pts[3], dashed, w, "c"))
            elif op == "re":
                r = it[1]
                corners = [T(r.tl), T(r.tr), T(r.br), T(r.bl)]
                for k in range(4):
                    a, b = corners[k], corners[(k + 1) % 4]
                    segs.append(Seg(_line_pts(a, b), a, b, dashed, w, "l"))
            elif op == "qu":
                q = it[1]
                corners = [T(q.ul), T(q.ur), T(q.lr), T(q.ll)]
                for k in range(4):
                    a, b = corners[k], corners[(k + 1) % 4]
                    segs.append(Seg(_line_pts(a, b), a, b, dashed, w, "l"))
    return PageGeom(segs).build()


# ---------------------------------------------------------------- shape tests
NBINS = 36


def _coverage(angles):
    bins = {int(((a + math.pi) / (2 * math.pi)) * NBINS) % NBINS for a in angles}
    return len(bins) / NBINS


def shape_features(geom: PageGeom, cx: float, cy: float, rmin=9.0, rmax=26.0, search=8):
    """Return best-fit circle / square / diamond / midline evidence around (cx,cy).

    The tag text is rarely at the exact symbol centre, so the centre is
    searched coarse-to-fine within +-`search` points.
    """
    pts = geom.points_near(cx, cy, rmax + search + 4)
    best = {"circle": (0.0, 0, 0, 0), "square": (0.0, 0, 0, 0), "diamond": (0.0, 0, 0, 0)}
    if not pts:
        return _pack(best, None, geom, cx, cy)
    metrics = (
        ("circle", lambda x, y: math.hypot(x, y)),
        ("square", lambda x, y: max(abs(x), abs(y))),
        ("diamond", lambda x, y: (abs(x) + abs(y)) / math.sqrt(2)),
    )

    def evaluate(ox, oy, only=None):
        rel = [(x - ox, y - oy) for x, y, _ in pts]
        for name, metric in metrics:
            if only and name != only:
                continue
            hist = {}
            for x, y in rel:
                v = metric(x, y)
                if rmin <= v <= rmax:
                    hist.setdefault(int(v), []).append(math.atan2(y, x))
            for r0, angs in hist.items():
                cov = _coverage(angs + hist.get(r0 + 1, []))
                if cov > best[name][0]:
                    best[name] = (cov, r0 + 0.5, ox, oy)

    for ox in range(-search, search + 1, 2):
        for oy in range(-search, search + 1, 2):
            evaluate(ox, oy)
    for name in ("circle", "square", "diamond"):
        _, _, bx, by = best[name]
        for ox in (bx - 1, bx, bx + 1):
            for oy in (by - 1, by, by + 1):
                if (ox, oy) != (bx, by):
                    evaluate(ox, oy, only=name)
    return _pack(best, pts, geom, cx, cy)


def _vertex_hits(pts, ox, oy, verts, tol=2.2):
    n = 0
    for vx, vy in verts:
        if any(abs(x - ox - vx) <= tol and abs(y - oy - vy) <= tol for x, y, _ in pts):
            n += 1
    return n


def _pack(best, pts, geom, cx, cy):
    # a circle also scores on the square / diamond metric near the axes, so a
    # square must show its 4 corners and a diamond its 4 vertices
    if pts:
        cov, h, ox, oy = best["square"]
        if cov and _vertex_hits(pts, ox, oy, [(h, h), (-h, h), (h, -h), (-h, -h)]) < 3:
            best["square"] = (min(cov, 0.3), h, ox, oy)
        cov, k, ox, oy = best["diamond"]
        v = k * math.sqrt(2)
        if cov and _vertex_hits(pts, ox, oy, [(v, 0), (-v, 0), (0, v), (0, -v)]) < 3:
            best["diamond"] = (min(cov, 0.3), k, ox, oy)
    f = {k: round(v[0], 3) for k, v in best.items()}
    f["r_circle"] = best["circle"][1]
    f["r_square"] = best["square"][1]
    f["r_diamond"] = best["diamond"][1]
    # symbol centre = best circle centre if strong, else square
    key = "circle" if best["circle"][0] >= 0.6 else ("square" if best["square"][0] >= 0.6 else "diamond")
    ox, oy = best[key][2], best[key][3]
    f["cx"], f["cy"] = cx + ox, cy + oy
    f["midline"], f["midline_dashed"] = 0.0, False
    if pts:
        R = best[key][1] or 15
        xs, dashed_n, n = set(), 0, 0
        for x, y, i in pts:
            x -= ox
            y -= oy
            if abs(y) < 1.2 and abs(x) < R * 0.95:
                xs.add(int((x + R) / (2 * R) * 20))
                n += 1
                dashed_n += geom.segs[i].dashed
        f["midline"] = round(len(xs) / 20, 3)
        f["midline_dashed"] = n > 0 and dashed_n / n > 0.5
    return f
