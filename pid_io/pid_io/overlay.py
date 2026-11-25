"""Review PDF: draws every recognised symbol on the drawing, coloured by class."""
from __future__ import annotations

from pathlib import Path

import pymupdf as fitz

from .extract import Symbol

COLORS = {
    "FIELD": (0, 0.55, 0),
    "DCS_SHARED": (0, 0.3, 1),
    "PANEL_CR": (0, 0.6, 0.9),
    "PANEL_BEHIND": (0.5, 0.5, 0.5),
    "PLC": (0.6, 0, 0.8),
    "ESD_INTERLOCK": (1, 0, 0),
    "DCS_INTERLOCK": (1, 0.5, 0),
    "HW_INTERLOCK": (0.8, 0.4, 0),
}


def write_overlay(pdf: Path, symbols: list[Symbol], out: Path):
    doc = fitz.open(pdf)
    by_page: dict[int, list[Symbol]] = {}
    for s in symbols:
        if s.role != "equipment":
            by_page.setdefault(s.page, []).append(s)
    keep = sorted(by_page)
    for pno in keep:
        page = doc[pno - 1]
        inv = ~page.rotation_matrix          # display -> unrotated page space
        for s in by_page[pno]:
            col = COLORS.get(s.cls, (0, 0, 0))
            r = s.radius * 1.15
            rect = fitz.Rect(s.cx - r, s.cy - r, s.cx + r, s.cy + r) * inv
            shape = page.new_shape()
            shape.draw_rect(rect)
            shape.finish(color=col, width=1.6)
            shape.commit()
            label = f"{s.cls} {s.score:.2f}"
            if s.links:
                label += " *"
            p = fitz.Point(s.cx - r, s.cy - r - 2) * inv
            page.insert_text(p, label, fontsize=6, color=col, rotate=page.rotation)
    doc.select([p - 1 for p in keep])
    doc.save(out, garbage=3, deflate=True)
