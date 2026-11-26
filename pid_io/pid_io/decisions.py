"""User decisions made in the review step and the checklist (decisions.json).

{
  "only_one":  {"PID-0015": 2},              # B03: real count for 'ONLY ONE IS SHOWN' sheets
  "allowance": {"AI": 2, "AO": 0, "DI": 4, "DO": 2},   # C09: IO per undeveloped package unit
  "rows": {"<row key>": {"action": "ok|exclude|edit", "io": "DI", "system": "ESD",
                         "tag": "...", "desc": "...", "qty": 2, "note": "...", "text": "<what the user typed>"}}
}
Row keys are 'tag|desc' taken before the XXXX numbering, so they survive re-computation.
A decided row is final: status 확정, the decision is written to Remarks, and it leaves
the review queue.
"""
from __future__ import annotations

import copy
import json
import re
from pathlib import Path

IOS = ("AI", "AO", "DI", "DO")


def load(pdir: Path) -> dict:
    f = Path(pdir) / "decisions.json"
    d = json.loads(f.read_text(encoding="utf-8")) if f.exists() else {}
    d.setdefault("only_one", {})
    d.setdefault("allowance", {})
    d.setdefault("rows", {})
    return d


def save(pdir: Path, d: dict):
    (Path(pdir) / "decisions.json").write_text(json.dumps(d, ensure_ascii=False, indent=1), encoding="utf-8")


def row_key(r) -> str:
    return f"{r.tag}|{r.desc}"


def summary(e: dict) -> str:
    if e.get("action") == "exclude":
        return "인덱스에서 제외"
    parts = []
    for k, lab in (("tag", "Tag"), ("io", "IO"), ("system", "System"), ("desc", "설명"), ("qty", "수량")):
        if e.get(k) not in (None, ""):
            parts.append(f"{lab}={e[k]}")
    if e.get("note"):
        parts.append(f"메모: {e['note']}")
    return ", ".join(parts) if parts else "현행 확정"


def apply_rows(rows, decided: dict):
    if not decided:
        for r in rows:
            r.key = r.key or row_key(r)
        return rows
    out = []
    for r in rows:
        r.key = r.key or row_key(r)
        e = decided.get(r.key)
        if not e and r.origin in ("트레인복제", "대수복제") and r.src_tag:
            src = decided.get(f"{r.src_tag}|{r.desc}")          # copies follow their source row
            if src:
                e = {k: v for k, v in src.items() if k not in ("tag", "desc")}
        if not e or r.kind == "function":
            out.append(r)
            continue
        if e.get("action") == "exclude":
            continue
        if e.get("io"):
            r.io = e["io"]
        if e.get("system"):
            r.system = e["system"]
        if e.get("desc"):
            r.desc = e["desc"]
        if e.get("tag"):
            r.tag = e["tag"]
        r.status = "확정"
        r.decided = summary(e)
        r.inform.append(f"사용자 판단: {r.decided}")
        out.append(r)
        q = int(e.get("qty") or 1)
        for k in range(2, q + 1):
            c = copy.deepcopy(r)
            c.tag = f"{r.tag}-{k}"
            c.key = f"{r.key}#{k}"
            out.append(c)
    return out


# ------------------------------------------------------------------ plain-language instructions
SYS_WORDS = [(r"ESD|SIS", "ESD"), (r"F\s*&\s*G|FGS|화재|가스", "F&G"), (r"DCS", "DCS"),
             (r"벤더\s*PLC|PKG\s*PLC|패키지\s*PLC", "PKG PLC")]


def parse(text: str) -> dict:
    """Turn a short Korean/English instruction into an edit. Unknown words become the memo.
    e.g. 'DI로 바꾸고 ESD로' -> io=DI, system=ESD ; '빼줘' -> exclude ; '2대' -> qty=2"""
    t = (text or "").strip()
    e: dict = {}
    if not t:
        return {"action": "ok"}
    u = t.upper()
    if re.search(r"제외|삭제|빼|없애|지워|EXCLUDE|DELETE|REMOVE", u):
        return {"action": "exclude", "note": t}
    m = re.search(r"(?:태그|TAG)\s*(?:는|을|를|:|=|→|->)?\s*([0-9A-Z][0-9A-Z\-/]{2,})", u)
    if m:
        e["tag"] = m.group(1)
    if re.search(r"IO\s*(?:없|NONE|-)|IO\s*제외|하드와이어\s*아님|통신으로", u):
        e["io"] = "-"
    else:
        ios = [x for x in re.findall(r"(?<![A-Z])(AI|AO|DI|DO)(?![A-Z])", u)]
        if ios:
            e["io"] = ios[-1]
    for pat, val in SYS_WORDS:
        if re.search(pat, u):
            e["system"] = val
            break
    m = re.search(r"(\d+)\s*(?:대|개|EA|SET|배|점)", u)
    if m and int(m.group(1)) >= 1:
        e["qty"] = int(m.group(1))
    m = re.search(r"(?:설명|DESC(?:RIPTION)?)\s*(?:은|는|을|를|:|=)?\s*(.+)$", t, re.I)
    if m:
        e["desc"] = m.group(1).strip()[:80]
    e["action"] = "edit" if any(k in e for k in ("tag", "io", "system", "qty", "desc")) else "ok"
    e["note"] = t
    e["understood"] = e["action"] != "ok" or bool(re.search(r"확정|맞|OK|그대로|유지|정상|문제\s*없|그냥|둬|놔", u))
    return e


AI_PROMPT = """You convert an instrument engineer's instruction about ONE instrument index row into JSON.
Row: tag={tag}; description={desc}; system={system}; io={io}; drawing={dwg}
Instruction (Korean or English): {text}
Answer with ONE JSON object only, no other text:
{{"action": "ok|exclude|edit", "tag": "<new tag or empty>", "io": "<AI|AO|DI|DO|-|empty>",
  "system": "<DCS|ESD|F&G|PKG PLC|empty>", "desc": "<new description or empty>", "qty": <integer, 1 if not said>}}"""


def from_ai(answer: dict, text: str) -> dict:
    e = {"action": str(answer.get("action") or "ok").lower(), "note": text}
    if e["action"] not in ("ok", "exclude", "edit"):
        e["action"] = "edit"
    io = str(answer.get("io") or "").upper()
    if io in IOS or io == "-":
        e["io"] = io
    sy = str(answer.get("system") or "").upper().replace("FGS", "F&G")
    if sy in ("DCS", "ESD", "F&G", "PKG PLC"):
        e["system"] = sy
    if answer.get("tag"):
        e["tag"] = str(answer["tag"])[:40]
    if answer.get("desc"):
        e["desc"] = str(answer["desc"])[:80]
    try:
        q = int(answer.get("qty") or 1)
        if q > 1:
            e["qty"] = q
    except (TypeError, ValueError):
        pass
    e["understood"] = True
    return e
