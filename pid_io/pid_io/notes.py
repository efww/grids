"""Read P&ID notes that change quantities or scope.

Every note is reduced to a typed effect so the engine can turn it into
index rows (never just a multiplier on a total):
  prefix      tag prefix / train range       "PREFIXED BY TRAIN NUMBER 11~18"
  copies      identical trains not drawn     "TRAIN 12~18 ARE IDENTICAL TO TRAIN 11"
  only_one    one of several drawn, count ?  "ONLY ONE IS SHOWN ON THE P&ID"
  later       package drawing not developed  "P&ID TO BE DEVELOPED LATER"
  vendor      vendor scope                   "BY VENDOR", "VENDOR'S DETAILED DESIGN"
  hold        HOLD items
Unrecognised notes are still listed so a person can decide.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path

import pymupdf as fitz

from .extract import _page_meta, is_legend_page

RANGE_RE = re.compile(r"PREFIXED\s+BY\s+(?:TRAIN|UNIT)\s+NUMBER\s+(\d+)\s*(?:[~\-]\s*(\d+))?")
COPY_RE = re.compile(r"TRAINS?\s+(\d+)\s*[~\-]\s*(\d+)\s+(?:ARE|IS)\s+IDENTICAL\s+TO\s+TRAIN\s+(\d+)")
ONLY_ONE_RE = re.compile(r"ONLY\s+ONE\s+(?:IS\s+)?SHOWN")
LATER_RE = re.compile(r"(?:P&ID'?S?\s+)?TO\s+BE\s+DEVELOPED\s+(?:LATER|IN\s+A\s+FUTURE)")
VENDOR_RE = re.compile(r"(?:BY\s+VENDOR|VENDOR[’']?S\s+(?:DETAILED\s+)?(?:DESIGN|P&ID|SCOPE))")
NOTE_RE = re.compile(r"(?:^|\s)(\d{1,2})\.\s+([A-Z][^\n]{8,}?(?:\.(?=\s|$)|$))")


@dataclass
class PageNotes:
    dwg: str
    page: int
    title: str
    unit: str = ""
    train_range: tuple = ()
    copies: list = field(default_factory=list)     # extra train numbers to copy to
    copy_text: str = ""
    only_one: str = ""
    later: str = ""
    vendor: list = field(default_factory=list)
    holds: list = field(default_factory=list)
    notes: list = field(default_factory=list)       # (text, effect)


def read_notes(pdf: Path) -> dict[str, PageNotes]:
    out = {}
    doc = fitz.open(pdf)
    for page in doc:
        if is_legend_page(page):
            continue
        dwg, unit, title = _page_meta(page)
        flat = " ".join(page.get_text().split())
        pn = PageNotes(dwg, page.number + 1, title, unit)
        m = RANGE_RE.search(flat)
        if m:
            a = int(m.group(1)); b = int(m.group(2) or m.group(1))
            pn.train_range = (a, b)
            pn.unit = str(a)
            pn.notes.append((m.group(0), f"태그 접두 = {a}" + (f" (트레인 {a}~{b})" if b != a else "")))
        m = COPY_RE.search(flat)
        if m:
            a, b, src = int(m.group(1)), int(m.group(2)), m.group(3)
            pn.copies = [str(x) for x in range(a, b + 1) if str(x) != src]
            pn.copy_text = m.group(0)
            pn.notes.append((m.group(0), f"트레인 {src} 행을 {a}~{b}로 복제 (B01)"))
        m = ONLY_ONE_RE.search(flat)
        if m:
            ctx = flat[max(0, m.start() - 80): m.end() + 20]
            pn.only_one = ctx
            pn.notes.append((ctx, "대수 미기재: 배수 입력 필요 (B03)"))
        m = LATER_RE.search(flat)
        if m:
            pn.later = flat[max(0, m.start() - 40): m.end() + 10]
            pn.notes.append((pn.later, "패키지 도면 미개발: 패키지 Typical로 IO 추정 (C09)"))
        for m in VENDOR_RE.finditer(flat):
            ctx = flat[max(0, m.start() - 60): m.end() + 20]
            pn.vendor.append(ctx)
        if pn.vendor:
            pn.notes.append((pn.vendor[0], "벤더 범위 언급 (B06/C01)"))
        pn.holds = sorted(set(re.findall(r"HOLD\s+(\d+)", flat)))
        out[dwg] = pn
    return out
