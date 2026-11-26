"""Title block facts and equipment links (index decisions 1, 2, 5, 15).

title_block(page)      -> {dwg, rev, plant_no, sheet_no}
    rev      : REV cell of the title block. Empty when the cell is empty (decision 15).
    plant_no : 'PLANT NO.' value, e.g. M96. Never copied to Area (decision 1).
    sheet_no : 'SHEET' value, e.g. '1 OF 3'.

attach_equipment(page, geom, syms)
    equip    : equipment tag, ONLY when the instrument's own connection line ends on
               the equipment body / nozzle (decision 5). Pipe instruments stay blank.
    service  : draft service text (decision 2).

Heuristic and fail-safe: any error leaves the fields blank.
"""
from __future__ import annotations

import math
import re

import pymupdf as fitz

EQUIP_TYPES = {"A", "B", "C", "D", "E", "F", "FL", "H", "K", "M", "P", "PK", "R", "S", "T", "TK", "V", "X"}
GEN_EQUIP_RE = re.compile(r"^([A-Z]{0,2}\d{2,3})-([A-Z]{1,2})-(\d{3,4}[A-Z]?)(?:/[A-Z])*$")
DWG_RE = re.compile(r"PID-\d{4}")
REV_VAL = re.compile(r"^[A-Z0-9]{1,3}$")
PLANT_VAL = re.compile(r"^[A-Z]{0,2}\d{1,4}[A-Z]?$")
SKIP = {"NO", "NO.", ":", "NO:", "OF", "REV", "REV.", "DWG.", "DWG", "SHEET", "SHT", "SHT.", "PLANT"}


def _words(page):
    m = page.rotation_matrix
    return [(fitz.Rect(w[:4]) * m, w[4]) for w in page.get_text("words")]


def _value_near(words, lab, ok, max_dx=120, max_dy=22):
    right = [(r.x0 - lab.x1, t) for r, t in words
             if abs((r.y0 + r.y1) / 2 - (lab.y0 + lab.y1) / 2) < 4 and 0 < r.x0 - lab.x1 < max_dx
             and t.upper().strip(":") not in SKIP]
    for _, t in sorted(right):
        if ok(t):
            return t
        break
    below = [(r.y0 - lab.y1, t) for r, t in words
             if abs(r.x0 - lab.x0) < 40 and 0 < r.y0 - lab.y1 < max_dy and t.upper().strip(":") not in SKIP]
    for _, t in sorted(below):
        if ok(t):
            return t
        break
    return ""


def title_block(page) -> dict:
    out = {"dwg": "", "rev": "", "plant_no": "", "sheet_no": ""}
    try:
        W, H = page.rect.width, page.rect.height
        if page.rotation in (90, 270):
            W, H = H, W
        words = _words(page)
        tb = [(r, t) for r, t in words if r.x0 > W * 0.55 and r.y0 > H * 0.55]
        dl = [r for r, t in tb if t.upper() in ("DWG.", "DWG")]
        dlab = max(dl, key=lambda r: r.y0) if dl else None
        if dlab:
            c = [(abs(r.y0 - dlab.y0), r.x0, t) for r, t in tb if DWG_RE.fullmatch(t) and r.x0 > dlab.x1]
            out["dwg"] = min(c)[2] if c else ""
            revs = [r for r, t in tb if t.upper().strip(":") in ("REV", "REV.") and abs(r.y0 - dlab.y0) < 30]
            for lab in sorted(revs, key=lambda r: abs(r.y0 - dlab.y0)):
                v = _value_near(tb, lab, lambda t: bool(REV_VAL.match(t)) and t.upper() not in SKIP, 60, 14)
                if v:
                    out["rev"] = v
                    break
        for r, t in tb:
            u = t.upper().strip(":")
            if u == "PLANT" and not out["plant_no"]:
                lab = r
                nxt = [x for x in tb if abs(x[0].y0 - r.y0) < 4 and 0 < x[0].x0 - r.x1 < 20
                       and x[1].upper().strip(".:") == "NO"]
                if nxt:
                    lab = nxt[0][0]
                out["plant_no"] = _value_near(tb, lab, lambda t: bool(PLANT_VAL.match(t)))
            if u in ("SHEET", "SHT", "SHT.") and not out["sheet_no"]:
                row = sorted([(x[0].x0, x[1]) for x in tb if abs(x[0].y0 - r.y0) < 4 and 0 < x[0].x0 - r.x1 < 110],
                             key=lambda z: z[0])
                parts = []
                for _, t2 in row:
                    tt = t2.strip(":")
                    if tt.upper() in ("NO", "NO.", ""):
                        continue
                    if re.fullmatch(r"\d{1,3}|OF", tt, re.I):
                        parts.append(tt.upper() if tt.upper() == "OF" else tt)
                        if len(parts) >= 3:
                            break
                    else:
                        break
                out["sheet_no"] = " ".join(parts)
    except Exception:
        pass
    return out


def find_all_equipment(words):
    out = []
    for k, (r, t) in enumerate(words):
        m = GEN_EQUIP_RE.match(t)
        if not m or m.group(2) not in EQUIP_TYPES:
            continue
        prev = words[k - 1][1] if k else ""
        out.append((t, r, prev.upper() in ("FROM", "TO")))
    return out


def _equipment_regions(page, tags):
    m = page.rotation_matrix
    W = max(page.rect.width, page.rect.height)
    shapes = []
    for d in page.get_drawings():
        R = d["rect"] * m
        if R.width < 30 or R.height < 30 or R.width > W * 0.5 or R.height > W * 0.5:
            continue
        kinds = {it[0] for it in d["items"]}
        if "c" in kinds or "re" in kinds or "qu" in kinds or len(d["items"]) >= 4:
            shapes.append(R)
    regions = []
    for tag, r, ref in tags:
        if ref:
            continue
        cx, cy = (r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2
        cand = [R for R in shapes if R.contains(fitz.Point(cx, cy))]
        if not cand:
            cand = [R for R in shapes if R.x0 - 10 <= cx <= R.x1 + 10 and 0 <= r.y0 - R.y1 <= 60]
        if cand:
            regions.append((tag, min(cand, key=lambda R: R.width * R.height)))
    return regions


def _connectors(words):
    lines = {}
    for r, t in words:
        lines.setdefault(round(((r.y0 + r.y1) / 2) / 3), []).append((r, t))
    out = []
    for items in lines.values():
        items.sort(key=lambda x: x[0].x0)
        cur = [items[0]]
        phrases = []
        for a in items[1:]:
            if a[0].x0 - cur[-1][0].x1 > 12:
                phrases.append(cur)
                cur = [a]
            else:
                cur.append(a)
        phrases.append(cur)
        for ph in phrases:
            if ph[0][1].upper() not in ("FROM", "TO"):
                continue
            txt = " ".join(t for _, t in ph[1:] if not DWG_RE.fullmatch(t)).strip(" -")
            if not txt:
                continue
            R = fitz.Rect(ph[0][0])
            for x, _ in ph:
                R |= x
            out.append((ph[0][1].upper() + " " + txt[:60], R))
    return out


def _chain_end(geom, s, regions, max_steps=40, max_len=220):
    segs, seen = [], set()
    for g in geom.segs:
        if g.dashed or math.dist(g.a, g.b) <= 0.3:
            continue
        k = tuple(sorted(((round(g.a[0], 1), round(g.a[1], 1)), (round(g.b[0], 1), round(g.b[1], 1)))))
        if k in seen:
            continue
        seen.add(k)
        segs.append(g)
    idx = {}
    cell = 10.0

    def key(p):
        return (int(p[0] // cell), int(p[1] // cell))

    for i, g in enumerate(segs):
        idx.setdefault(key(g.a), []).append((i, 0))
        idx.setdefault(key(g.b), []).append((i, 1))

    def ends_near(p, tol=1.6):
        kx, ky = key(p)
        for ix in (kx - 1, kx, kx + 1):
            for iy in (ky - 1, ky, ky + 1):
                for i, e in idx.get((ix, iy), ()):
                    q = segs[i].a if e == 0 else segs[i].b
                    if math.dist(p, q) <= tol:
                        yield i, e

    def on_equipment(p):
        for tag, R in regions:
            if (R + (-4, -4, 4, 4)).contains(fitz.Point(p)):
                return tag
        return ""

    starts = []
    for i, g in enumerate(segs):
        for e, p in ((0, g.a), (1, g.b)):
            d0 = math.dist(p, (s.cx, s.cy))
            if s.radius * 0.9 <= d0 <= s.radius * 1.6 + 4:
                q = g.b if e == 0 else g.a
                ux, uy = p[0] - s.cx, p[1] - s.cy
                vx, vy = q[0] - p[0], q[1] - p[1]
                nu, nv = math.hypot(ux, uy), math.hypot(vx, vy)
                if nu and nv and (ux * vx + uy * vy) / (nu * nv) > 0.7:
                    starts.append((i, e))
    for i0, e0 in starts:
        visited = {i0}
        i, e = i0, e0
        length = 0.0
        for _ in range(max_steps):
            g = segs[i]
            far = g.b if e == 0 else g.a
            length += math.dist(g.a, g.b)
            if length > max_len:
                break
            nxt = [(j, e2) for j, e2 in ends_near(far) if j not in visited]
            if len(nxt) != 1:
                tag = on_equipment(far)
                if tag:
                    return tag
                break
            i, e = nxt[0]
            e = 0 if math.dist(segs[i].a, far) <= math.dist(segs[i].b, far) else 1
            visited.add(i)
    return ""


def attach_equipment(page, geom, syms):
    try:
        words = _words(page)
        W = max(page.rect.width, page.rect.height)
        tags = find_all_equipment(words)
        regions = _equipment_regions(page, tags)
        anchors = [(t, (r.x0 + r.x1) / 2, (r.y0 + r.y1) / 2) for t, r, ref in tags]
        anchors += [(txt, (R.x0 + R.x1) / 2, (R.y0 + R.y1) / 2) for txt, R in _connectors(words)]
        for s in syms:
            if s.role != "device":
                continue
            try:
                s.equip = _chain_end(geom, s, regions) if regions else ""
            except Exception:
                s.equip = ""
            if s.equip:
                s.service = s.equip
                continue
            left = [(s.cx - x + 2 * abs(y - s.cy), t) for t, x, y in anchors if x < s.cx and s.cx - x < W * 0.6]
            right = [(x - s.cx + 2 * abs(y - s.cy), t) for t, x, y in anchors if x > s.cx and x - s.cx < W * 0.6]
            L = min(left)[1] if left else ""
            Rt = min(right)[1] if right else ""
            clean = lambda t: re.sub(r"^(FROM|TO)\s+", "", t)
            if L and Rt:
                s.service = f"FROM {clean(L)} TO {clean(Rt)}"
            elif L:
                s.service = f"FROM {clean(L)}"
            elif Rt:
                s.service = f"TO {clean(Rt)}"
    except Exception:
        pass
