"""Two stages, kept apart so the slow one runs once:

  extract  (minutes) : PDF -> symbols + notes, saved as extract.json
  compute  (ms)      : extract.json + concept checklist -> index rows + totals

Changing the checklist only re-runs `compute`.
"""
from __future__ import annotations

import json
import math
from dataclasses import asdict
from pathlib import Path

import pymupdf as fitz

from .concept import Concept
from .extract import Symbol, extract_pdf, load_protocol
from .io_assign import assign
from .notes import PageNotes, read_notes

PROTO = Path(__file__).resolve().parent.parent / "protocols"
ORIGINS = ["도면인식", "Typical파생", "트레인복제", "장비Typical", "패키지", "스페어"]
HW = ("AI", "AO", "DI", "DO")


def run_extract(pdf: Path, out: Path, progress=None, symbols_yaml: Path = PROTO / "symbols.yaml"):
    proto = load_protocol(symbols_yaml)
    doc = fitz.open(pdf)
    n = doc.page_count
    syms = extract_pdf(pdf, proto, progress=(lambda p, k: progress(p, n, k)) if progress else None)
    notes = read_notes(pdf)
    pages = {}
    for page in doc:
        pages[page.number + 1] = [page.rect.width, page.rect.height]
    data = {
        "pdf": Path(pdf).name, "page_count": n, "page_size": pages,
        "symbols": [{**asdict(s), "links": sorted(s.links)} for s in syms],
        "notes": {d: asdict(v) for d, v in notes.items()},
    }
    Path(out).write_text(json.dumps(data, ensure_ascii=False), encoding="utf-8")
    return data


def load_extract(path: Path):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    syms = []
    for d in data["symbols"]:
        d = dict(d)
        d["links"] = set(d.get("links", []))
        syms.append(Symbol(**d))
    notes = {}
    for k, v in data["notes"].items():
        v = dict(v)
        v["train_range"] = tuple(v.get("train_range") or ())
        v["notes"] = [tuple(x) for x in v.get("notes", [])]
        notes[k] = PageNotes(**v)
    return data, syms, notes


ROLE_OF = {"FIELD": "device", "DCS_SHARED": "function", "PANEL_CR": "function", "PANEL_BEHIND": "function",
           "PLC": "function", "ESD_INTERLOCK": "interlock", "DCS_INTERLOCK": "interlock", "HW_INTERLOCK": "interlock"}
SYS_OF = {"FIELD": "FIELD", "DCS_SHARED": "DCS", "PANEL_CR": "DCS", "PANEL_BEHIND": "LOCAL", "PLC": "PLC",
          "ESD_INTERLOCK": "ESD", "DCS_INTERLOCK": "DCS", "HW_INTERLOCK": "HARDWIRED"}


def apply_overrides(syms, rules: dict, overrides: dict | None):
    """Accepted review/AI decisions: per-symbol class/tag/IO and extra function-code rules."""
    if not overrides:
        return syms, rules
    import copy
    import re
    out = []
    so = overrides.get("symbols", {})
    for s in syms:
        o = so.get(s.sid)
        if not o:
            out.append(s)
            continue
        s = copy.copy(s)
        if o["class"] == "NOT_A_TAG":
            s.role = "ignored"
        else:
            s.cls, s.role, s.system = o["class"], ROLE_OF[o["class"]], SYS_OF[o["class"]]
            m = re.match(r"^\s*(?:\d{2}-)?([A-Z]{1,6})-?(\w+)\s*$", o.get("tag") or "")
            if m:
                s.func, s.number = m.group(1), m.group(2)
            s.override = {"io": o.get("io"), "system": o.get("system"), "reason": o.get("reason", "")}
            s.score = max(s.score, 0.95)
            s.notes = []
        out.append(s)
    extra = [{"match": f"^{re.escape(r['func'])}$", "desc": r["desc"], "io": r["io"],
              "system": {"F&G": "FGS"}.get(r["system"], r["system"]), "signal": "", "ai": True}
             for r in overrides.get("rules", [])]
    return out, {**rules, "rules": extra + rules["rules"]}


def compute(syms, notes, concept: Concept, io_rules: Path = PROTO / "io_rules.yaml", overrides: dict | None = None):
    syms, rules = apply_overrides(syms, load_protocol(io_rules), overrides)
    rows, log = assign(syms, rules, concept, notes)
    return rows, log


def summarize(rows, concept: Concept):
    inst = [r for r in rows if r.kind != "function"]
    by = {}
    for r in inst:
        key = (r.origin, r.system, r.io)
        by[key] = by.get(key, 0) + 1
    hw = sum(v for (o, s, io), v in by.items() if io in HW and s not in ("-", "?"))
    comm = sum(v for (o, s, io), v in by.items() if io == "Serial/Comm")
    per_origin = {o: {"rows": sum(1 for r in inst if r.origin == o),
                      "hw": sum(v for (oo, s, io), v in by.items() if oo == o and io in HW and s not in ("-", "?")),
                      "comm": sum(v for (oo, s, io), v in by.items() if oo == o and io == "Serial/Comm")}
                  for o in ORIGINS}
    per_sys = {}
    for (o, s, io), v in by.items():
        if io in HW and s not in ("-", "?"):
            per_sys.setdefault(s, {k: 0 for k in HW})[io] += v
    ch = concept.num("H05", 16) or 16
    cards = {s: sum(math.ceil(v[k] / ch) for k in HW) for s, v in per_sys.items()}
    status = {}
    for r in inst:
        status[r.status] = status.get(r.status, 0) + 1
    return {"rows": len(inst), "functions": len(rows) - len(inst), "hw": hw, "comm": comm,
            "per_origin": per_origin, "per_system": per_sys, "cards": cards,
            "cards_total": sum(cards.values()), "status": status}


def impact(syms, notes, concept: Concept, io_rules: Path = PROTO / "io_rules.yaml", top=8):
    """Flip each ON/OFF or choice item to its alternatives and report how much the
    hard-wired IO total moves - shows which assumptions to settle first."""
    proto = load_protocol(io_rules)
    base_rows, _ = assign(syms, proto, concept, notes)
    base = summarize(base_rows, concept)["hw"]
    out = []
    for iid, it in concept.items.items():
        if iid.startswith("I"):
            continue
        orig = it["value"]
        best = None
        for alt in concept.alternatives(iid):
            it["value"] = alt
            try:
                rows, _ = assign(syms, proto, concept, notes)
                d = summarize(rows, concept)["hw"] - base
            finally:
                it["value"] = orig
            if best is None or abs(d) > abs(best[1]):
                best = (alt, d)
        if best and best[1] != 0:
            out.append({"id": iid, "name": it["name"], "current": orig, "alt": best[0], "delta": best[1]})
    out.sort(key=lambda x: -abs(x["delta"]))
    return {"base": base, "items": out[:top]}
