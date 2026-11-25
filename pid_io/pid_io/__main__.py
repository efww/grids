"""P&ID -> Instrument Index / IO list.

    python -m pid_io run  P&ID.pdf --template Instrument_Index_Template.xlsx \
                          --out result.xlsx [--ref reference_index.xlsx] [--pages 15,16]
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

from .export import compare, load_reference, write_template
from .extract import extract_pdf, load_protocol, page_texts
from .concept import Concept
from .io_assign import assign
from .notes import read_notes

HERE = Path(__file__).resolve().parent.parent / "protocols"


def _pages(s):
    if not s:
        return None
    out = set()
    for part in s.split(","):
        if "-" in part:
            a, b = part.split("-")
            out.update(range(int(a), int(b) + 1))
        else:
            out.add(int(part))
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(prog="pid_io")
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="extract tags + IO into the index template")
    r.add_argument("pdf", type=Path)
    r.add_argument("--template", type=Path, required=True)
    r.add_argument("--out", type=Path, required=True)
    r.add_argument("--ref", type=Path, help="reference index for accuracy check")
    r.add_argument("--pages", help="e.g. 10-17,31")
    r.add_argument("--overlay", type=Path, help="write a review PDF with recognised symbols marked")
    r.add_argument("--symbols", type=Path, default=HERE / "symbols.yaml")
    r.add_argument("--io-rules", type=Path, default=HERE / "io_rules.yaml")
    r.add_argument("--concept", type=Path, default=HERE / "concept_default.xlsx",
                   help="concept checklist workbook (engineering assumptions)")
    r.add_argument("--dwg-map", type=Path, default=HERE / "dwg_map.yaml",
                   help="manual reference-title -> PDF drawing overrides")
    a = ap.parse_args(argv)

    sym_proto = load_protocol(a.symbols)
    io_proto = load_protocol(a.io_rules)
    syms = extract_pdf(a.pdf, sym_proto, _pages(a.pages),
                       progress=lambda p, n: print(f"  page {p}: {n} symbols", file=sys.stderr))
    concept = Concept(a.concept)
    notes = read_notes(a.pdf)
    if a.pages:
        notes = {d: n for d, n in notes.items() if n.page in _pages(a.pages)}
    rows, log = assign(syms, io_proto, concept, notes)
    m = re.search(r"Rev\.?\s*([A-Z0-9]+)", a.pdf.name, re.I)
    from .pipeline import impact
    write_template(rows, a.template, a.out, a.pdf.name, m.group(1) if m else "", concept, log,
                   impact=impact(syms, notes, concept, a.io_rules, top=None))
    print(f"symbols: {len(syms)}  index rows: {len(rows)}  -> {a.out}")
    if a.overlay:
        from .overlay import write_overlay
        write_overlay(a.pdf, syms, a.overlay)
        print(f"review overlay -> {a.overlay}")
    if a.ref:
        ov = load_protocol(a.dwg_map).get("map", {}) if a.dwg_map.exists() else {}
        res = compare(rows, load_reference(a.ref), a.out, page_texts(a.pdf), ov)
        for k, v in res.items():
            print(f"  {k}: {v}")


if __name__ == "__main__":
    main()
