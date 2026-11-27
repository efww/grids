"""AI tasks: build a prompt, parse and validate the answer, turn it into suggestions.

AI never edits the index directly. Every answer becomes a suggestion that a
person accepts or rejects; accepted ones are stored as overrides in the
project (overrides.json) and applied on the next calculation.

Tasks
  symbols : re-judge symbols the rules could not settle (unmatched tag-shaped
            text, low outline score, function code without an IO rule)
  funcs   : propose IO for function codes missing from io_rules.yaml
"""
from __future__ import annotations

import json
import re
import uuid
from pathlib import Path

import pymupdf as fitz

CLASSES = ["FIELD", "DCS_SHARED", "PANEL_CR", "PANEL_BEHIND", "PLC", "ESD_INTERLOCK", "DCS_INTERLOCK",
           "HW_INTERLOCK", "NOT_A_TAG"]
IOS = ["AI", "AO", "DI", "DO", "-"]
SYSTEMS = ["DCS", "ESD", "F&G", "PKG PLC", "-"]
LOW_SCORE = 0.8

CLASS_HELP = """FIELD         field-mounted instrument (plain circle)
DCS_SHARED    DCS display/control (circle inside a square) - no physical IO
PANEL_CR      control-room panel instrument (circle + solid midline)
PANEL_BEHIND  behind-panel instrument (circle + dashed midline)
PLC           PLC function (square with chamfered corners)
ESD_INTERLOCK ESD interlock (square + diamond, 'IS')
DCS_INTERLOCK DCS interlock (square, 'I')
HW_INTERLOCK  hardwired interlock (diamond, 'I')
NOT_A_TAG     text that is not an instrument tag (line number, connector, note)"""


# ------------------------------------------------------------------ helpers
ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")


def _balanced(text: str, start: int):
    """End index of the JSON object starting at text[start] == '{' (string-aware), or -1."""
    depth, ins, esc = 0, False, False
    for i in range(start, len(text)):
        ch = text[i]
        if ins:
            if esc:
                esc = False
            elif ch == "\\":
                esc = True
            elif ch == '"':
                ins = False
            continue
        if ch == '"':
            ins = True
        elif ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return i
    return -1


def extract_json(text: str, want=("items", "rules", "ok", "action")):
    """Take the JSON object out of a chat / CLI answer. Tries every '{' (CLI output often
    has logs, echoed prompts or '{placeholders}' before the answer), prefers an object
    with an expected key, tolerates code fences, single quotes and trailing commas."""
    import ast
    text = ANSI.sub("", text or "")
    found = []
    for m in re.finditer(r"\{", text):
        end = _balanced(text, m.start())
        if end < 0:
            continue
        raw = text[m.start():end + 1]
        obj = None
        for cand in (raw, re.sub(r",\s*([}\]])", r"\1", raw)):
            try:
                obj = json.loads(cand)
                break
            except ValueError:
                pass
        if obj is None:
            try:
                obj = ast.literal_eval(re.sub(r"\btrue\b", "True", re.sub(r"\bfalse\b", "False",
                                                                           re.sub(r"\bnull\b", "None", raw))))
            except Exception:
                obj = None
        if isinstance(obj, dict):
            if any(k in obj for k in want):
                return obj
            found.append(obj)
    if found:
        return found[-1]
    snippet = " ".join(text.split())[:240]
    raise ValueError("답변에서 JSON을 찾지 못했습니다. 받은 답변 앞부분: " + (snippet or "(비어 있음)"))


def nearby_words(page: fitz.Page, cx, cy, r=70):
    m = page.rotation_matrix
    out = []
    for w in page.get_text("words"):
        q = fitz.Rect(w[:4]) * m
        if abs((q.x0 + q.x1) / 2 - cx) < r and abs((q.y0 + q.y1) / 2 - cy) < r:
            out.append(w[4])
    return " ".join(out)[:200]


def crop(page: fitz.Page, cx, cy, out: Path, r=70):
    page.get_pixmap(dpi=150, clip=fitz.Rect(cx - r, cy - r, cx + r, cy + r)).save(out)


# ------------------------------------------------------------------ task: symbols
def symbol_items(syms, rules_unknown):
    """Symbols worth a second opinion."""
    items = []
    for s in syms:
        if s.override or s.role == "equipment" or s.role == "interlock":
            continue
        why = []
        if s.role == "candidate":
            why.append("no legend rule matched")
        elif s.score < LOW_SCORE:
            why.append(f"low outline score {s.score}")
        if s.func in rules_unknown and s.role != "function":
            why.append("function code without IO rule")
        if why:
            items.append((s, why))
    return items


def build_symbols(pdf: Path, syms, rules_unknown, crop_dir: Path, limit=40, pd: Path | None = None, examples: str = ""):
    from .. import pagecache
    cached = pd is not None and pagecache.ready(pd)       # labelled P&ID: work from the saved page images
    doc = None if cached else fitz.open(pdf)
    crop_dir.mkdir(parents=True, exist_ok=True)
    items = symbol_items(syms, rules_unknown)[:limit]
    lines, files = [], []
    for k, (s, why) in enumerate(items, 1):
        img = crop_dir / f"{k:02d}_{s.dwg}_{s.func}-{s.number}.png"
        if cached:
            pagecache.card(pd, s.page, s.cx, s.cy, img, sym=s, radius=s.radius)
            near = pagecache.nearby_words(pd, s.page, s.cx, s.cy)
        else:
            from ..markup import crop as mark_crop          # red box on the item, traced line if any
            mark_crop(pdf, s.page, s.cx, s.cy, img, sym=s, radius=s.radius)
            near = nearby_words(doc[s.page - 1], s.cx, s.cy)
        files.append(img)
        f = s.feats or {}
        lines.append(json.dumps({
            "id": s.sid, "image": img.name, "sheet": s.dwg, "text": f"{s.func} {s.number}".strip(),
            "outline": {k2: f.get(k2) for k2 in ("circle", "square", "diamond", "midline")},
            "rule_result": s.cls, "why_asked": why, "nearby_text": near,
        }, ensure_ascii=False))
    prompt = f"""You are an instrumentation engineer reviewing symbols read from a P&ID (ISA 5.1 legend).
A rule-based reader could not settle the items below. For each item decide what it is.

Classes:
{CLASS_HELP}

"outline" = share (0..1) of a circle / square / diamond / midline found around the tag text.
Images (if you can see them) are crops around each item, file names given in "image": the item is in a
RED box, a traced signal line is ORANGE, the symbol that line reaches is in a BLUE box.

{examples}
Items (one JSON per line):
{chr(10).join(lines)}


Write every "desc" and "reason" value in Korean (한국어), short and plain.
Answer with ONE JSON object only, no other text:
{{"items": [{{"id": "<id as given>", "class": "<one of the classes>", "tag": "<func-number, e.g. PT-0602>",
  "io": "<AI|AO|DI|DO|->", "system": "<DCS|ESD|F&G|PKG PLC|->", "confidence": <0..1>, "reason": "<short>"}}]}}
"""
    return prompt, files, [s.sid for s, _ in items]


def parse_symbols(answer: str, asked_ids):
    data = extract_json(answer)
    out = []
    for it in data.get("items", []):
        if it.get("id") not in asked_ids:
            continue
        cls = str(it.get("class", "")).upper()
        io = str(it.get("io", "-")).upper()
        sy = str(it.get("system", "-")).upper().replace("FGS", "F&G")
        if cls not in CLASSES or io not in IOS or sy not in SYSTEMS:
            continue
        out.append({"id": it["id"], "class": cls, "tag": str(it.get("tag", ""))[:40], "io": io,
                    "system": sy, "confidence": float(it.get("confidence") or 0), "reason": str(it.get("reason", ""))[:200]})
    return out


# ------------------------------------------------------------------ task: IO conflicts
def build_ioconf(rows, limit=40):
    """Rows where the concept rule and the drawn signal line disagree."""
    items = [r for r in rows if r.line == "conflict" and r.sid][:limit]
    lines = [json.dumps({"id": r.sid, "tag": r.tag, "instrument": r.desc, "rule_io": r.io, "rule_system": r.system,
                         "drawing_evidence": next((x for x in r.inform if x.startswith("IO 판정 불일치")), ""),
                         "sheet": f"{r.dwg} {r.title}"}, ensure_ascii=False) for r in items]
    prompt = f"""You are an instrumentation engineer doing conceptual-design IO counting.
For each instrument, the concept rule (function code: transmitter = AI, control valve = AO, on/off valve
solenoid = DO, switch = DI) and the drawn signal line disagree. Conceptual P&IDs often have incomplete
arrows and line types, so the rule is usually right unless the drawing clearly shows otherwise (for
example a software/serial link means communication, not hard-wired IO).
Decide the IO for each item.

{chr(10).join(lines)}


Write every "desc" and "reason" value in Korean (한국어), short and plain.
Answer with ONE JSON object only:
{{"items": [{{"id": "<id as given>", "class": "FIELD", "tag": "<tag>", "io": "<AI|AO|DI|DO|->",
  "system": "<DCS|ESD|F&G|PKG PLC|->", "confidence": <0..1>, "reason": "<short: rule or drawing, why>"}}]}}
"""
    return prompt, [r.sid for r in items]


# ------------------------------------------------------------------ task: function codes
def unknown_funcs(syms, rules):
    from ..io_assign import _rule_for
    out = {}
    for s in syms:
        if s.role in ("device", "candidate") and s.number and _rule_for(rules, s.func).get("review"):
            out.setdefault(s.func, []).append(f"{s.dwg} {s.func}-{s.number}")
    return out


def build_funcs(codes: dict):
    lines = [json.dumps({"func": k, "examples": v[:5]}, ensure_ascii=False) for k, v in codes.items()]
    prompt = f"""You are an instrumentation engineer. These tag-like codes were read from a P&ID but our IO rules do not
cover them. Some are real ISA 5.1 function codes (e.g. EH = electric heater), some are just ordinary
words picked up from notes or titles (e.g. TRAIN, PUMPS) - set "instrument": false for those.
For each real code give the instrument type and the physical IO one such instrument needs
(conceptual design; [] if it has no IO, e.g. local gauges or relief devices).

{chr(10).join(lines)}


Write every "desc" and "reason" value in Korean (한국어), short and plain.
Answer with ONE JSON object only:
{{"rules": [{{"func": "<code>", "instrument": <true|false>, "desc": "<instrument type, Korean>", "io": ["AI"|"AO"|"DI"|"DO", ...],
  "system": "<DCS|ESD|F&G|->", "confidence": <0..1>, "reason": "<short>"}}]}}
"""
    return prompt, list(codes)


def parse_funcs(answer: str, asked):
    data = extract_json(answer)
    out = []
    for r in data.get("rules", []):
        f = str(r.get("func", "")).upper()
        io = [str(x).upper() for x in (r.get("io") or [])]
        sy = str(r.get("system", "DCS")).upper().replace("FGS", "F&G")
        if r.get("instrument") is False:
            io, sy = [], "-"
            r["desc"] = "계기 기능코드 아님 (도면의 일반 단어)"
        if f not in asked or any(x not in IOS[:4] for x in io) or sy not in SYSTEMS:
            continue
        out.append({"func": f, "desc": str(r.get("desc", ""))[:60], "io": io, "system": sy,
                    "confidence": float(r.get("confidence") or 0), "reason": str(r.get("reason", ""))[:200]})
    return out


# ------------------------------------------------------------------ storage
def load(path: Path, default):
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else default


def save(path: Path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=1), encoding="utf-8")


def add_suggestions(pdir: Path, kind: str, items: list, source: str):
    f = pdir / "ai_suggestions.json"
    allx = load(f, [])
    for it in items:
        allx.append({"sid": uuid.uuid4().hex[:10], "kind": kind, "status": "pending", "source": source, **it})
    save(f, allx)
    return allx


def decide(pdir: Path, sid: str, accept: bool):
    f = pdir / "ai_suggestions.json"
    allx = load(f, [])
    ov = load(pdir / "overrides.json", {"symbols": {}, "rules": []})
    for s in allx:
        if s["sid"] != sid:
            continue
        s["status"] = "accepted" if accept else "rejected"
        if accept and s["kind"] == "symbols":
            ov["symbols"][s["id"]] = {k: s[k] for k in ("class", "tag", "io", "system", "reason")}
        if accept and s["kind"] == "missing":
            ov.setdefault("added", [])
            ov["added"] = [a for a in ov["added"] if a["id"] != s["id"]]
            ov["added"].append({k: s[k] for k in ("id", "page", "cx", "cy", "tag", "class", "io", "system", "reason")})
        if accept and s["kind"] == "legend":
            ov.setdefault("legend", []).append({k: s.get(k, "") for k in ("shape", "class", "system", "note")})
        if accept and s["kind"] in ("funcs", "legend_rule"):
            ov["rules"] = [r for r in ov["rules"] if r["func"] != s["func"]]
            ov["rules"].append({k: s[k] for k in ("func", "desc", "io", "system", "reason")})
    save(f, allx)
    save(pdir / "overrides.json", ov)
    return allx


# ------------------------------------------------------------------ v13: examples, legend, missed symbols, agreement
def examples_block(syms, overrides: dict, limit=12) -> str:
    """symbols people already approved on this project -> few-shot examples for the next question."""
    so = (overrides or {}).get("symbols") or {}
    by = {s.sid: s for s in syms}
    lines = []
    for sid, o in list(so.items())[-limit:]:
        s = by.get(sid)
        if not s:
            continue
        f = s.feats or {}
        lines.append(json.dumps({"text": f"{s.func} {s.number}".strip(), "outline": {k: f.get(k) for k in ("circle", "square", "diamond", "midline")},
                                 "approved": {"class": o["class"], "io": o.get("io"), "system": o.get("system")}}, ensure_ascii=False))
    leg = (overrides or {}).get("legend") or []
    out = ""
    if leg:
        out += "\nProject legend (approved, read from the legend sheets):\n" + "\n".join(
            f"- {x.get('shape', '')}: {x.get('class', '')} / {x.get('system', '')} {x.get('note', '')}" for x in leg[:30]) + "\n"
    if lines:
        out += "\nExamples a person approved on this project (follow the same judgement):\n" + "\n".join(lines) + "\n"
    return out


def legend_pages(pd: Path, syms) -> list:
    """pages with 'SYMBOL' / 'LEGEND' words and no instrument found = legend sheets."""
    info = json.loads((Path(pd) / "cache" / "raster" / "info.json").read_text(encoding="utf-8")) if (Path(pd) / "cache" / "raster" / "info.json").exists() else {}
    found = {s.page for s in syms}
    out = []
    for p, v in (info.get("pages") or {}).items():
        txt = " ".join(str(w[4]) if isinstance(w, (list, tuple)) and len(w) > 4 else str(w) for w in v.get("words") or []).upper()
        if int(p) not in found and ("SYMBOL" in txt or "LEGEND" in txt):
            out.append(int(p))
    return sorted(out)


def build_legend(pd: Path, pages: list, out_dir: Path):
    out_dir.mkdir(parents=True, exist_ok=True)
    files = []
    for p in pages[:6]:
        src = Path(pd) / "cache" / "raster" / f"p{p}.png"
        if src.exists():
            dst = out_dir / f"legend_p{p}.png"
            dst.write_bytes(src.read_bytes())
            files.append(dst)
    prompt = f"""You are an instrumentation engineer. The attached images are the LEGEND sheets of a P&ID set (ISA 5.1 style).
Read them and build this project's symbol dictionary.

1) Instrument symbol shapes -> class. Classes:
{CLASS_HELP}
2) Function letter codes used on this project (first + succeeding letters, e.g. PT, FIT, XV, ZSO) with their IO.

Write every "desc", "note" and "shape" value in Korean (한국어), short.
Answer with ONE JSON object only:
{{"symbols": [{{"shape": "<shape in words>", "class": "<class>", "system": "<DCS|ESD|F&G|PKG PLC|->", "note": "<short>"}}],
  "rules": [{{"func": "<code>", "desc": "<instrument type>", "io": ["AI"|"AO"|"DI"|"DO"], "system": "<DCS|ESD|F&G|PKG PLC|->",
             "confidence": <0..1>, "reason": "<legend evidence>"}}]}}
Images: {", ".join(f.name for f in files)}
"""
    return prompt, files, [f"legend_p{p}" for p in pages[:6]]


def parse_legend(answer: str):
    data = extract_json(answer, want=("symbols", "rules"))
    sym = []
    for it in data.get("symbols", []) or []:
        cls = str(it.get("class", "")).upper()
        if cls in CLASSES:
            sym.append({"shape": str(it.get("shape", ""))[:80], "class": cls,
                        "system": str(it.get("system", "-")).upper().replace("FGS", "F&G"), "note": str(it.get("note", ""))[:120]})
    rules = []
    for r in data.get("rules", []) or []:
        f = str(r.get("func", "")).upper().strip()
        io = [x for x in (r.get("io") or []) if x in IOS[:4]]
        sy = str(r.get("system", "DCS")).upper().replace("FGS", "F&G")
        if re.fullmatch(r"[A-Z]{1,6}", f) and sy in SYSTEMS:
            rules.append({"func": f, "desc": str(r.get("desc", ""))[:60], "io": io, "system": sy,
                          "confidence": float(r.get("confidence") or 0), "reason": str(r.get("reason", ""))[:200]})
    return sym, rules


def build_missing(pd: Path, syms, pages: list, out_dir: Path, grid=2):
    """each page cut into grid x grid tiles; the AI lists instrument balloons NOT in our list for that tile."""
    from PIL import Image
    out_dir.mkdir(parents=True, exist_ok=True)
    info = json.loads((Path(pd) / "cache" / "raster" / "info.json").read_text(encoding="utf-8"))
    dpi = info.get("dpi", 150)
    k = dpi / 72.0
    files, tiles = [], []
    for p in pages:
        src = Path(pd) / "cache" / "raster" / f"p{p}.png"
        if not src.exists():
            continue
        im = Image.open(src)
        W, H = im.size
        for gy in range(grid):
            for gx in range(grid):
                box = (gx * W // grid, gy * H // grid, (gx + 1) * W // grid, (gy + 1) * H // grid)
                tid = f"p{p}_{gy}{gx}"
                f = out_dir / f"{tid}.png"
                im.crop(box).save(f)
                x0, y0, x1, y1 = (b / k for b in box)
                have = sorted({f"{s.func}-{s.number}" for s in syms if s.page == p and x0 <= s.cx < x1 and y0 <= s.cy < y1
                               and s.role not in ("ignored", "candidate")})
                files.append(f)
                tiles.append({"tile": tid, "image": f.name, "page": p, "box_pt": [round(x0), round(y0), round(x1), round(y1)], "found": have})
    lines = [json.dumps(t, ensure_ascii=False) for t in tiles]
    prompt = f"""You are checking a P&ID for instrument balloons (ISA 5.1 circles / squares with a tag inside) that a reader MISSED.
For each tile image the list "found" holds the tags already read there. Look at the image and list every instrument
balloon whose tag is NOT in "found". Ignore line numbers, equipment tags, notes and off-page connectors.
Give its position inside the tile as fractions (0..1 from the tile's top-left) and the tag text exactly as drawn.

Tiles (one JSON per line):
{chr(10).join(lines)}

Write "reason" in Korean (한국어). Answer with ONE JSON object only:
{{"items": [{{"tile": "<tile id>", "tag": "<e.g. PT-0602>", "fx": <0..1>, "fy": <0..1>, "class": "<class>",
  "io": "<AI|AO|DI|DO|->", "system": "<DCS|ESD|F&G|PKG PLC|->", "confidence": <0..1>, "reason": "<short>"}}]}}
Classes:
{CLASS_HELP}
"""
    return prompt, files, tiles


def parse_missing(answer: str, tiles: list):
    data = extract_json(answer)
    by = {t["tile"]: t for t in tiles}
    out = []
    for it in data.get("items", []) or []:
        t = by.get(it.get("tile"))
        if not t:
            continue
        tag = str(it.get("tag", "")).upper().strip()
        m = re.match(r"^(?:\d{2}-)?([A-Z]{1,6})-?(\w+)$", tag)
        cls = str(it.get("class", "FIELD")).upper()
        io = str(it.get("io", "-")).upper()
        sy = str(it.get("system", "-")).upper().replace("FGS", "F&G")
        if not m or cls not in CLASSES or cls == "NOT_A_TAG" or io not in IOS or sy not in SYSTEMS:
            continue
        if f"{m.group(1)}-{m.group(2)}" in t["found"]:
            continue
        x0, y0, x1, y1 = t["box_pt"]
        fx, fy = min(1, max(0, float(it.get("fx") or 0.5))), min(1, max(0, float(it.get("fy") or 0.5)))
        out.append({"id": f"{t['page']}:{m.group(1)}:{m.group(2)}", "page": t["page"], "cx": round(x0 + fx * (x1 - x0), 1),
                    "cy": round(y0 + fy * (y1 - y0), 1), "tag": f"{m.group(1)}-{m.group(2)}", "class": cls, "io": io, "system": sy,
                    "confidence": float(it.get("confidence") or 0), "reason": str(it.get("reason", ""))[:200]})
    return out


def agree(a: list, b: list, key="id"):
    """two independent answers -> items where class / io / system agree (kept), the rest flagged."""
    bi = {x[key]: x for x in b}
    out = []
    for x in a:
        y = bi.get(x[key])
        same = y is not None and all(x.get(k) == y.get(k) for k in ("class", "io", "system"))
        out.append({**x, "agree": bool(same), "confidence": x.get("confidence", 0) if same else min(x.get("confidence", 0), 0.4),
                    "reason": x.get("reason", "") + ("" if same else " · AI 두 답이 다름 → 사람 판단")})
    return out
