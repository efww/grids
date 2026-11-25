"""Extract instrument symbols (balloons) and tags from a vector P&ID PDF."""
from __future__ import annotations

import math
import re
from dataclasses import dataclass, field
from pathlib import Path

import pymupdf as fitz
import yaml

from .geometry import PageGeom, load_page_geom, shape_features

FUNC_RE = re.compile(r"^[A-Z]{1,6}$")
NUM_RE = re.compile(r"^(\d{2,5}[A-Z]?|X{2,4}[A-Z]?)$")
DWG_RE = re.compile(r"PID-\d{4}")
PREFIX_RE = re.compile(r"PREFIXED\s+BY\s+(?:TRAIN|UNIT)\s+NUMBER\s+(\d+)", re.S)


@dataclass
class Symbol:
    page: int
    dwg: str
    title: str
    func: str
    number: str
    cls: str
    system: str
    role: str
    score: float
    cx: float
    cy: float
    radius: float
    feats: dict
    unit: str = ""
    links: set = field(default_factory=set)   # ids of interlocks this symbol is wired to
    notes: list = field(default_factory=list)

    @property
    def tag(self):
        base = f"{self.func}-{self.number}" if self.number else self.func
        return f"{self.unit}-{base}" if self.unit else base

    @property
    def sid(self):
        return f"{self.page}:{self.func}:{self.number}:{round(self.cx)}:{round(self.cy)}"


def load_protocol(path: Path):
    with open(path, encoding="utf-8") as f:
        return yaml.safe_load(f)


def _match(proto: dict, feats: dict, func: str):
    hits = []
    for name, rule in proto["symbols"].items():
        ok = True
        score = 0.0
        for k in ("circle", "square", "diamond"):
            if k in rule:
                ok &= feats[k] >= rule[k]
                score += feats[k]
            if f"no_{k}" in rule:
                ok &= feats[k] < rule[f"no_{k}"]
        ml = rule.get("midline", "any")
        has_ml = feats["midline"] >= 0.55
        if ml == "solid":
            ok &= has_ml and not feats["midline_dashed"]
        elif ml == "dashed":
            ok &= feats["midline"] >= 0.45 and (feats["midline_dashed"] or feats["midline"] < 0.8)
        elif ml == "none":
            ok &= feats["midline"] < 0.45
        if "text_regex" in rule:
            ok &= bool(re.match(rule["text_regex"], func))
        if ok:
            hits.append((rule.get("priority", 0), score, name, rule))
    if not hits:
        return None
    hits.sort(key=lambda h: (-h[0], -h[1]))
    return hits[0]


def _page_meta(page: fitz.Page):
    text = page.get_text()
    m = PREFIX_RE.search(text.replace("\n", " "))
    unit = m.group(1) if m else ""
    words = _display_words(page)
    # drawing number = the PID-xxxx printed in the title block "DWG. NO." cell
    dwg = ""
    d_lbl = [r for r, t in words if t == "DWG."]
    if d_lbl:
        dl = max(d_lbl, key=lambda r: r.y0)
        cands = [(abs(r.y0 - dl.y0), r.x0, t) for r, t in words
                 if DWG_RE.fullmatch(t) and r.x0 > dl.x1 and abs(r.y0 - dl.y0) < 14]
        if cands:
            dwg = min(cands)[2]
    if not dwg:
        dwgs = DWG_RE.findall(text)
        dwg = max(set(dwgs), key=dwgs.count) if dwgs else f"p{page.number + 1}"
    # title = text inside the title block "TITLE :" cell (display coords, lower right)
    title = ""
    t_w = [r for r, t in words if t == "TITLE"]
    d_w = [r for r, t in words if t == "DWG."]
    if t_w and d_w:
        tr = max(t_w, key=lambda r: r.y0)
        below = [r for r in d_w if r.y0 > tr.y0 - 1]
        y_end = min(below, key=lambda r: r.y0).y0 if below else tr.y1 + 40
        x_end = tr.x0 + (page.rect.width * 0.16)
        parts = [(r.y0, r.x0, t) for r, t in words
                 if r.x0 > tr.x1 and r.x1 < x_end and tr.y0 - 4 <= r.y0 < y_end - 1 and t != ":"]
        parts.sort(key=lambda p: (round(p[0] / 4), p[1]))
        title = " ".join(p[2] for p in parts if not DWG_RE.fullmatch(p[2]))
    return dwg, unit, title


def _display_words(page: fitz.Page):
    m = page.rotation_matrix
    out = []
    for w in page.get_text("words"):
        r = fitz.Rect(w[:4]) * m
        out.append((r, w[4]))
    return out


EQUIP_RE = re.compile(r"^(\d{2})-([A-Z]{1,2})-(\d{3})([A-Z](?:/[A-Z])*)?$")


def find_equipment(page: fitz.Page, kinds=("P",)):
    """Equipment drawn on this sheet (not FROM/TO references).
    Returns {base_tag: sorted suffix letters}."""
    words = _display_words(page)
    out: dict[str, set] = {}
    for k, (r, t) in enumerate(words):
        m = EQUIP_RE.match(t)
        if not m or m.group(2) not in kinds:
            continue
        prev = words[k - 1][1] if k else ""
        if prev in ("FROM", "TO"):
            continue
        base = f"{m.group(1)}-{m.group(2)}-{m.group(3)}"
        letters = out.setdefault(base, set())
        if m.group(4):
            letters.update(m.group(4).split("/"))
        else:   # single letters printed next to the tag ("A", "B")
            for r2, t2 in words:
                if len(t2) == 1 and t2.isalpha() and abs(r2.y0 - r.y0) < 3 and 0 < r2.x0 - r.x1 < 12:
                    letters.add(t2)
    return {b: sorted(v) or [""] for b, v in out.items()}


def is_legend_page(page: fitz.Page) -> bool:
    dwg, _, _ = _page_meta(page)
    return dwg.startswith("PID-9")


def extract_page(page: fitz.Page, proto: dict) -> list[Symbol]:
    dwg, unit, title = _page_meta(page)
    words = _display_words(page)
    geom = load_page_geom(page)
    nums = [(r, t) for r, t in words if NUM_RE.match(t)]
    syms: list[Symbol] = []
    seen = []
    ignore = set(proto.get("ignore_words", []))
    for r, t in words:
        if not FUNC_RE.match(t) or t in ignore:
            continue
        cxw, cyw = (r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2
        best = None
        for nr, nt in nums:
            ncx, ncy = (nr.x0 + nr.x1) / 2, (nr.y0 + nr.y1) / 2
            dx, dy = abs(ncx - cxw), ncy - cyw
            if dx < 10 and 5 < dy < 27:
                # numbers of neighbouring balloons can overlap: prefer the one
                # straight below at the usual line spacing
                cost = abs(dy - 13) + 2 * dx
                if best is None or cost < best[0]:
                    best = (cost, nt, ncx, ncy)
        if best:
            cx, cy = (cxw + best[2]) / 2, (cyw + best[3]) / 2
            number = best[1]
        else:
            cx, cy, number = cxw, cyw + 5, ""
        feats = shape_features(geom, cx, cy)
        hit = _match(proto, feats, t)
        if not hit:
            continue
        _, score, name, rule = hit
        if t in ("I", "IS") and rule["role"] != "interlock":
            # interlock letters inside a balloon that the outline test read as
            # something else: trust the letters
            name = "ESD_INTERLOCK" if t == "IS" else "DCS_INTERLOCK"
            rule = proto["symbols"][name]
        # de-duplicate (same symbol seen from two words)
        if any(abs(s.cx - feats["cx"]) < 4 and abs(s.cy - feats["cy"]) < 4 and s.func == t for s in seen):
            continue
        radius = max(feats["r_circle"], feats["r_square"], feats["r_diamond"], 12)
        sym = Symbol(page.number + 1, dwg, title, t, number, name, rule["system"], rule["role"],
                     round(score / max(1, sum(k in rule for k in ("circle", "square", "diamond"))), 3),
                     feats["cx"], feats["cy"], radius, feats, unit)
        if not number and rule["role"] != "interlock":
            continue   # inline element letters such as 'C' (coriolis) - not a tagged balloon
        syms.append(sym)
        seen.append(sym)
    trace_links(geom, syms)
    return syms


# ------------------------------------------------------------ signal tracing
def trace_links(geom: PageGeom, syms: list[Symbol], gap=4.5, max_steps=400):
    """Follow thin signal lines from every interlock symbol and record which
    other symbols they reach. Dashed lines may be stored as separate dash
    pieces, so we allow small collinear gaps."""
    segs = [s for s in geom.segs if s.kind == "l" and s.width < 0.9 and math.dist(s.a, s.b) > 0.3]
    idx = {}
    cell = 10.0

    def key(p):
        return (int(p[0] // cell), int(p[1] // cell))

    for i, s in enumerate(segs):
        idx.setdefault(key(s.a), []).append((i, 0))
        idx.setdefault(key(s.b), []).append((i, 1))

    def ends_near(p, r):
        kx, ky = key(p)
        for ix in (kx - 1, kx, kx + 1):
            for iy in (ky - 1, ky, ky + 1):
                for i, e in idx.get((ix, iy), ()):
                    q = segs[i].a if e == 0 else segs[i].b
                    if math.dist(p, q) <= r:
                        yield i, e

    def inside(p, s, pad=3.0):
        return math.dist(p, (s.cx, s.cy)) <= s.radius * 1.45 + pad

    others = [s for s in syms]
    for src in [s for s in syms if s.role == "interlock"]:
        start = []
        for i, s in enumerate(segs):
            for e, p in ((0, s.a), (1, s.b)):
                d = math.dist(p, (src.cx, src.cy))
                if src.radius * 0.9 <= d <= src.radius * 1.6 + 4:
                    q = s.b if e == 0 else s.a
                    if math.dist(q, (src.cx, src.cy)) > d:   # leaving the symbol
                        start.append((i, e))
        visited = set()
        stack = list(start)
        steps = 0
        terminals = []
        found = False
        while stack and steps < max_steps:
            steps += 1
            i, e = stack.pop()
            if i in visited:
                continue
            visited.add(i)
            s = segs[i]
            far = s.b if e == 0 else s.a
            hit = None
            for o in others:
                if o is src:
                    continue
                if inside(far, o):
                    hit = o
                    break
            if hit:
                hit.links.add(src.sid)
                src.links.add(hit.sid)
                found = True
                continue
            if inside(far, src, pad=0):
                continue
            terminals.append(far)
            dirv = (far[0] - (s.a if e == 0 else s.b)[0], far[1] - (s.a if e == 0 else s.b)[1])
            for j, e2 in ends_near(far, gap):
                if j in visited:
                    continue
                t = segs[j]
                near_pt = t.a if e2 == 0 else t.b
                if math.dist(far, near_pt) > 1.2:
                    # gap jump: only along the same direction (dash pieces)
                    nxt = t.b if e2 == 0 else t.a
                    v2 = (nxt[0] - near_pt[0], nxt[1] - near_pt[1])
                    n1, n2 = math.hypot(*dirv), math.hypot(*v2)
                    if n1 == 0 or n2 == 0 or (dirv[0] * v2[0] + dirv[1] * v2[1]) / (n1 * n2) < 0.95:
                        continue
                stack.append((j, e2))
        if not found and terminals:
            # signal ends on an actuator: attach to the nearest valve balloon
            cands = []
            for o in others:
                if o is src or o.role != "device" or not o.func.endswith("V"):
                    continue
                d = min(math.dist(t, (o.cx, o.cy)) for t in terminals)
                if d < 70:
                    cands.append((d, o))
            if cands:
                o = min(cands, key=lambda c: c[0])[1]
                o.links.add(src.sid)
                src.links.add(o.sid)
                o.notes.append(f"interlock link via actuator ({src.func}-{src.number})")


def page_texts(pdf: Path) -> dict:
    doc = fitz.open(pdf)
    out = {}
    for page in doc:
        dwg, _, title = _page_meta(page)
        out[dwg] = (title, page.get_text())
    return out


def extract_pdf(pdf: Path, proto: dict, pages=None, progress=None):
    doc = fitz.open(pdf)
    out = []
    seen_equipment: set[str] = set()
    for pno in range(doc.page_count):
        if pages and pno + 1 not in pages:
            continue
        page = doc[pno]
        if is_legend_page(page):
            continue
        syms = extract_page(page, proto)
        dwg, unit, title = _page_meta(page)
        for base, letters in find_equipment(page).items():
            for L in letters:
                if base + L in seen_equipment:      # same pump drawn on two sheets
                    continue
                seen_equipment.add(base + L)
                syms.append(Symbol(pno + 1, dwg, title, "EQUIP:P", base + L, "EQUIPMENT", "-",
                                   "equipment", 1.0, 0, 0, 0, {}, ""))
        if progress:
            progress(pno + 1, len(syms))
        out.extend(syms)
    return out
