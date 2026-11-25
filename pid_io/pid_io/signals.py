"""Signal-line evidence for field instruments (supporting evidence only).

In conceptual P&IDs the signal lines and arrows are often incomplete, so the
IO type stays decided by the function-code concept (transmitter = AI,
control valve = AO ...). This module only collects what the drawing says
about each instrument's signal line, so the result can be marked as
agreeing / disagreeing with the concept rule:

  line type   electric (dashed)            -> hard-wired
              software / serial link (o-o) -> communication
              pneumatic (// ticks)         -> no electrical IO
  direction   filled arrowhead at the far end  -> signal goes to the system (input)
              filled arrowhead at the device   -> signal comes to the device (output)
  far end     the symbol the line reaches (DCS balloon, PLC, interlock, ...)

The legend (PID-9001 'INSTRUMENT LINES') draws these marks the same way:
small circles ~5pt across for software links, short diagonal ticks for
pneumatic, a dash pattern for electric.
"""
from __future__ import annotations

import math

import pymupdf as fitz

from .geometry import PageGeom


def _markers(page: fitz.Page):
    """Small marks that tell the line type apart, plus filled arrowheads (display coords)."""
    m = page.rotation_matrix
    circles, ticks, arrows = [], [], []
    for d in page.get_drawings():
        R = d["rect"] * m
        its = d["items"]
        if R.width < 8 and R.height < 8 and len(its) >= 8 and all(x[0] in ("l", "c") for x in its):
            circles.append(((R.x0 + R.x1) / 2, (R.y0 + R.y1) / 2))
        elif R.width < 12 and R.height < 12 and d.get("fill") is not None and 1 <= len(its) <= 4:
            pts = []
            for x in its:
                if x[0] == "l":
                    pts += [fitz.Point(x[1]) * m, fitz.Point(x[2]) * m]
            if len(pts) >= 3:
                cx, cy = sum(p.x for p in pts) / len(pts), sum(p.y for p in pts) / len(pts)
                tip = max(pts, key=lambda p: math.dist((p.x, p.y), (cx, cy)))
                arrows.append(((cx, cy), (tip.x, tip.y)))
        elif len(its) == 1 and its[0][0] == "l":
            a, b = fitz.Point(its[0][1]) * m, fitz.Point(its[0][2]) * m
            L = math.dist((a.x, a.y), (b.x, b.y))
            dx, dy = abs(a.x - b.x), abs(a.y - b.y)
            if 3 < L < 6.5 and dx > 1 and dy > 1 and 0.4 < dx / dy < 2.5:
                ticks.append(((a.x + b.x) / 2, (a.y + b.y) / 2))
    return circles, ticks, arrows


def _near(pts, cands, r):
    return any(math.dist(p, c) <= r for p in pts for c in cands)


def collect(page: fitz.Page, geom: PageGeom, syms, gap=4.5, max_steps=300):
    """Attach `line` evidence to every field device symbol on the page."""
    circles, ticks, arrows = _markers(page)
    segs = [s for s in geom.segs if s.kind == "l" and s.width < 0.9 and math.dist(s.a, s.b) > 0.3]
    cell = 10.0
    idx = {}

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

    others = [s for s in syms if s.role in ("device", "function", "interlock")]
    SYSTEM = ("DCS_SHARED", "PLC", "ESD_INTERLOCK", "DCS_INTERLOCK", "HW_INTERLOCK", "PANEL_CR", "PANEL_BEHIND")
    for src in [s for s in syms if s.role == "device"]:
        chains = []
        for i, s in enumerate(segs):
            for e, p in ((0, s.a), (1, s.b)):
                d0 = math.dist(p, (src.cx, src.cy))
                if src.radius * 0.9 <= d0 <= src.radius * 1.6 + 4:
                    q = s.b if e == 0 else s.a
                    # must leave the balloon radially (not run along its outline)
                    ux, uy = p[0] - src.cx, p[1] - src.cy
                    vx, vy = q[0] - p[0], q[1] - p[1]
                    nu, nv = math.hypot(ux, uy), math.hypot(vx, vy)
                    if nu and nv and (ux * vx + uy * vy) / (nu * nv) > 0.7:
                        chains.append([(i, e)])
        results = []
        for start in chains:
            visited, stack, steps = set(), list(start), 0
            pts, dashed_n, jumps, dest, far_end = [], 0, 0, None, None
            while stack and steps < max_steps:
                steps += 1
                i, e = stack.pop()
                if i in visited:
                    continue
                visited.add(i)
                s = segs[i]
                far = s.b if e == 0 else s.a
                near_pt = s.a if e == 0 else s.b
                pts += [near_pt, far]
                dashed_n += s.dashed
                hit = next((o for o in others if o is not src and inside(far, o)), None)
                if hit:
                    dest, far_end = hit, far
                    break
                dirv = (far[0] - near_pt[0], far[1] - near_pt[1])
                for j, e2 in ends_near(far, gap):
                    if j in visited:
                        continue
                    t = segs[j]
                    np_ = t.a if e2 == 0 else t.b
                    if math.dist(far, np_) > 1.2:
                        nxt = t.b if e2 == 0 else t.a
                        v2 = (nxt[0] - np_[0], nxt[1] - np_[1])
                        n1, n2 = math.hypot(*dirv), math.hypot(*v2)
                        if n1 == 0 or n2 == 0 or (dirv[0] * v2[0] + dirv[1] * v2[1]) / (n1 * n2) < 0.95:
                            continue
                        jumps += 1
                    stack.append((j, e2))
            if not pts or dest is None or dest.cls not in SYSTEM:
                continue            # only lines that reach a system symbol tell us about IO
            sample = pts[::3]
            if _near(sample, circles, 4.5):
                kind = "software"
            elif _near(sample, ticks, 4.5):
                kind = "pneumatic"
            elif dashed_n or jumps >= 2:
                kind = "electric"
            else:
                continue            # solid line: process / impulse connection, not a signal
            # only arrowheads sitting on this line count (dimension arrows etc. nearby do not)
            on_line = [(c, tip) for (c, tip) in arrows if _near([c], pts, 6)]
            start_pt = pts[0]
            at_sys = any(math.dist(tip, far_end) < 9 or inside(tip, dest, pad=6) for c, tip in on_line)
            at_dev = any(math.dist(tip, start_pt) < 9 or inside(tip, src, pad=6) for c, tip in on_line)
            flow = "to_system" if at_sys and not at_dev else "to_device" if at_dev and not at_sys else "?"
            r = {"type": kind, "to": dest.cls, "to_tag": f"{dest.func}-{dest.number}".strip("-"), "flow": flow}
            same = next((x for x in results if x["to_tag"] == r["to_tag"] and x["type"] == r["type"]), None)
            if same is None:
                results.append(r)
            elif same["flow"] == "?":
                same["flow"] = flow
        if results:
            src.line = results


def judge(io: str, line: list | None):
    """Compare the concept-rule IO with the drawn signal line.
    Returns (verdict, text): verdict in 'agree' | 'conflict' | 'none'."""
    if not line:
        return "none", ""
    kinds = {x["type"] for x in line}
    to_sys = [x for x in line if x["to"] in ("DCS_SHARED", "PLC", "ESD_INTERLOCK", "DCS_INTERLOCK", "PANEL_CR")]
    desc = ", ".join(f"{ {'electric': '전기 점선', 'software': '통신선', 'pneumatic': '공압선'}[x['type']] }→{x['to_tag']}"
                     + ({"to_system": " (화살표: 시스템 방향)", "to_device": " (화살표: 계기 방향)"}.get(x["flow"], ""))
                     for x in line)
    hard = io in ("AI", "AO", "DI", "DO", "TC/RTD")
    if "software" in kinds and hard and not any(x["type"] == "electric" for x in line):
        return "conflict", f"규칙은 하드와이어 {io}, 도면은 통신선: {desc}"
    if "pneumatic" in kinds and hard and not any(x["type"] == "electric" for x in line):
        return "conflict", f"규칙은 {io}, 도면은 공압선: {desc}"
    if not hard and io in ("-", "") and any(x["type"] == "electric" for x in to_sys):
        return "conflict", f"규칙은 IO 없음, 도면은 시스템으로 가는 전기 신호선: {desc}"
    if io in ("AI", "DI") and any(x["flow"] == "to_device" for x in to_sys) and not any(x["flow"] == "to_system" for x in to_sys):
        return "conflict", f"규칙은 입력 {io}, 화살표는 계기 방향(출력): {desc}"
    if io in ("AO", "DO") and any(x["flow"] == "to_system" for x in to_sys) and not any(x["flow"] == "to_device" for x in to_sys):
        return "conflict", f"규칙은 출력 {io}, 화살표는 시스템 방향(입력): {desc}"
    return "agree", desc
