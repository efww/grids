"""Vendor package boundaries (dash-dot / dashed outlines).

The legend teaches what a boundary looks like: the sample drawn next to
"TO BE SUPPLIED/DESIGNED BY (PACKAGE) VENDOR" is a dash-dot line (long and
short dashes mixed) and gives the stroke width. On the sheets we take every
axis-aligned dash-dot line at least that heavy, merge pieces into long runs,
group runs whose ends touch, and keep groups that close into a box. Plain
dashed lines (instrument signals, tracing) are not dash-dot and stay out.
"""
from __future__ import annotations

import re
from dataclasses import dataclass

import pymupdf as fitz

LEGEND_RE = re.compile(r"SUPPLIED.*BY.*VENDOR|BY\s+(?:PACKAGE\s+|PACAKGE\s+)?VENDOR")
PK_RE = re.compile(r"^\d{2}-PK-\d{3}")


@dataclass
class Region:
    rect: fitz.Rect          # display coordinates
    package: str             # package tag found inside / on the outline, or ''
    runs: int


def is_dash_dot(dashes) -> bool:
    """Dash-dot / phantom line: the dash array mixes long and short dashes
    (e.g. '[ 4 3 16 3 ] 0'), unlike plain dashed signal lines ('[ 4 4 ] 0')."""
    if not dashes:
        return False
    nums = [float(x) for x in re.findall(r"[\d.]+", dashes.split("]")[0])]
    on = nums[0::2]
    return len(on) >= 2 and max(on) >= 2 * max(min(on), 0.1)


def _pieces(page: fitz.Page, min_width: float):
    """Axis-aligned line pieces in display coords: (orient, fixed, a, b, kind).
    kind 'attr'  : drawn with a dash-dot dash array (heavy enough)
    kind 'piece' : a short solid stroke that may be one dash of a dash-dot line
                   drawn piece by piece (checked for the long/short rhythm later)"""
    m = page.rotation_matrix
    out = []
    for d in page.get_drawings():
        w = d.get("width") or 0
        dd = is_dash_dot(d.get("dashes"))
        if dd and w < min_width:
            continue
        items = []
        for it in d["items"]:
            if it[0] == "l":
                items.append(it)
            elif it[0] in ("re", "qu") and dd:          # whole outline as one rectangle
                q = it[1].quad if it[0] == "re" else it[1]
                cs = [q.ul, q.ur, q.lr, q.ll]
                items += [("l", cs[k], cs[(k + 1) % 4]) for k in range(4)]
        if not items or (not dd and (len(items) > 2 or w <= 0)):
            continue
        for it in items:
            p, q = it[1] * m, it[2] * m
            if not dd and abs(p.x - q.x) + abs(p.y - q.y) > 30:
                continue                                  # long solid line: pipe, frame
            kind = "attr" if dd else "piece"
            if abs(p.y - q.y) < 0.6:
                out.append(("h", round((p.y + q.y) / 2, 0), min(p.x, q.x), max(p.x, q.x), kind))
            elif abs(p.x - q.x) < 0.6:
                out.append(("v", round((p.x + q.x) / 2, 0), min(p.y, q.y), max(p.y, q.y), kind))
    return out


def _merge(pieces, gap=14.0, min_len=40.0):
    """Merge collinear pieces into runs. Runs made of solid pieces only count
    when they show the dash-dot rhythm: several long dashes and short dots."""
    runs = []
    by = {}
    for o, f, a, b, k in pieces:
        by.setdefault((o, f, k), []).append((a, b))
    for (o, f, k), segs in by.items():
        segs.sort()
        g = gap if k == "attr" else 9.0
        cur = [segs[0][0], segs[0][1], [segs[0][1] - segs[0][0]]]
        groups = []
        for a, b in segs[1:]:
            if a - cur[1] <= g:
                cur[1] = max(cur[1], b)
                cur[2].append(b - a)
            else:
                groups.append(cur)
                cur = [a, b, [b - a]]
        groups.append(cur)
        for a, b, lens in groups:
            if b - a < min_len:
                continue
            if k == "piece":
                longs = sum(1 for v in lens if v >= 9)
                dots = sum(1 for v in lens if v <= 6)
                if longs < 3 or dots < 2 or not (0.3 <= dots / longs <= 3):
                    continue
            runs.append((o, f, a, b))
    return runs


def _rect_of(run):
    o, f, a, b = run
    return fitz.Rect(a, f, b, f) if o == "h" else fitz.Rect(f, a, f, b)


def legend_width(doc: fitz.Document) -> float:
    """Stroke width of the vendor-boundary sample in the legend (fallback 0.9)."""
    for page in doc:
        words = page.get_text("words")
        m = page.rotation_matrix
        for w in words:
            if w[4].startswith("VENDOR"):
                r = fitz.Rect(w[:4]) * m
                line = " ".join(x[4] for x in words if abs((fitz.Rect(x[:4]) * m).y0 - r.y0) < 3)
                if not LEGEND_RE.search(line):
                    continue
                band = fitz.Rect(r.x0 - 320, r.y0 - 14, r.x0 - 5, r.y1 + 14)
                ws = [d.get("width") or 0 for d in page.get_drawings()
                      if (d["rect"] * m).intersects(band) and (d["rect"] * m).width < 300]
                if ws:
                    return max(0.6, min(ws) * 0.85)
    return 0.9


def find_regions(page: fitz.Page, min_width: float) -> list[Region]:
    runs = _merge(_pieces(page, min_width))
    # group runs whose ends touch (tolerance 8pt) -> outlines
    n = len(runs)
    parent = list(range(n))

    def find(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    rects = [_rect_of(r) for r in runs]
    for i in range(n):
        for j in range(i + 1, n):
            if runs[i][0] == runs[j][0]:
                continue
            a, b = rects[i], rects[j]
            if (a.x0 - 8 <= b.x1 and b.x0 - 8 <= a.x1 and a.y0 - 8 <= b.y1 and b.y0 - 8 <= a.y1):
                parent[find(i)] = find(j)
    groups = {}
    for i in range(n):
        groups.setdefault(find(i), []).append(i)
    W = page.rect.width
    words = [(fitz.Rect(w[:4]) * page.rotation_matrix, w[4]) for w in page.get_text("words")]
    out = []
    for idx in groups.values():
        hs = [runs[i] for i in idx if runs[i][0] == "h"]
        vs = [runs[i] for i in idx if runs[i][0] == "v"]
        if not hs or not vs or len(hs) + len(vs) < 3:
            continue          # needs at least a U shape (an edge is often broken by pipes)
        # degenerate (zero-height/width) rects are ignored by Rect union: do it by hand
        box = fitz.Rect(min(rects[i].x0 for i in idx), min(rects[i].y0 for i in idx),
                        max(rects[i].x1 for i in idx), max(rects[i].y1 for i in idx))
        if box.width < 60 or box.height < 40 or box.width > W * 0.9:
            continue
        if box.x0 > W * 0.82:                         # legend / title block column
            continue
        pk = ""
        cands = [(t, r) for r, t in words if PK_RE.match(t) and (box + (-30, -30, 30, 30)).contains(r)]
        if cands:   # prefer a tag inside, else the closest on the outline
            inside = [t for t, r in cands if box.contains(r)]
            pk = PK_RE.match((inside or [cands[0][0]])[0]).group(0)
        if not pk:   # no package tag: name it from a caption on/near the outline
            near = [(r, t) for r, t in words if (box + (-6, -40, 6, 40)).contains(r)
                    and not box.contains(fitz.Rect(r.x0, r.y0 + 2, r.x1, r.y1 - 2))]
            line = " ".join(t for r, t in sorted(near, key=lambda x: (round(x[0].y0 / 5), x[0].x0)))
            m = re.search(r"BY\s+[A-Z][\w&]*|VENDOR|OTHERS", line)
            pk = f"VENDOR AREA ({m.group(0)})" if m else "VENDOR AREA"
        out.append(Region(box, pk, len(idx)))
    # boxes nested inside another outline are equipment inside the package, not packages
    out = [r for r in out if not any(o is not r and o.rect.contains(r.rect) for o in out)]
    return out


def regions_for_pdf(pdf, pages=None) -> dict[int, list[Region]]:
    doc = fitz.open(pdf)
    wmin = legend_width(doc)
    res = {}
    for page in doc:
        if pages and page.number + 1 not in pages:
            continue
        res[page.number + 1] = find_regions(page, wmin)
    return res
