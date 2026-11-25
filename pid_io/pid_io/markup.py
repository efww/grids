"""Marked-up crops of the drawing around one item to judge.

Red box   : the instrument / text being asked about
Orange    : the signal line the reader followed
Blue box  : the symbol that line reached (DCS balloon, interlock ...)
Purple    : vendor boundary, when the item sits in one
The marks are drawn on an in-memory copy of the page; the PDF on disk is
never changed.
"""
from __future__ import annotations

from pathlib import Path

import pymupdf as fitz

RED, ORANGE, BLUE, PURPLE = (0.88, 0.12, 0.22), (1.0, 0.55, 0.0), (0.18, 0.44, 0.84), (0.49, 0.4, 0.8)


def crop(pdf: Path, page_no: int, cx: float, cy: float, out: Path, sym=None,
         radius: float = 18, half: float = 95, dpi: int = 150, regions=None):
    doc = fitz.open(pdf)                 # fresh copy each time so marks never pile up
    page = doc[page_no - 1]
    inv = ~page.rotation_matrix
    shapes = page.new_shape()

    def box(x, y, r, color, w):
        shapes.draw_rect(fitz.Rect(x - r, y - r, x + r, y + r) * inv)
        shapes.finish(color=color, width=w)

    lo_x, lo_y, hi_x, hi_y = cx - half, cy - half, cx + half, cy + half
    for rg in regions or []:
        if rg.rect.intersects(fitz.Rect(lo_x, lo_y, hi_x, hi_y)):
            shapes.draw_rect(rg.rect * inv)
            shapes.finish(color=PURPLE, width=2.5, dashes="[10 3 2 3] 0")
    for ev in (getattr(sym, "line", None) or []):
        path = ev.get("path") or []
        if len(path) >= 2:
            shapes.draw_polyline([fitz.Point(x, y) * inv for x, y in path])
            shapes.finish(color=ORANGE, width=2.6, closePath=False)
            for x, y in path:          # keep the whole line in view
                lo_x, lo_y, hi_x, hi_y = min(lo_x, x - 25), min(lo_y, y - 25), max(hi_x, x + 25), max(hi_y, y + 25)
        if ev.get("to_xy"):
            x, y, r = ev["to_xy"]
            box(x, y, r * 1.2, BLUE, 2.2)
            lo_x, lo_y, hi_x, hi_y = min(lo_x, x - r - 20), min(lo_y, y - r - 20), max(hi_x, x + r + 20), max(hi_y, y + r + 20)
    box(cx, cy, radius * 1.25, RED, 2.6)
    shapes.commit()
    W, H = page.rect.width, page.rect.height
    clip = fitz.Rect(max(0, lo_x), max(0, lo_y), min(W, hi_x), min(H, hi_y))
    page.get_pixmap(dpi=dpi, clip=clip).save(out)
    return out
