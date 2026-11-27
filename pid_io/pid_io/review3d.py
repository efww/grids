"""AI 3D 리뷰어 (T6): rule pre-checks on the supplemented model data + a review packet for the company AI.

The company AI looks at the supplemented 3D model (or its captures) the way a checker would. To keep it honest:
  1. pid_io runs fixed geometric checks first (this file) -> every finding has an object ID and a model position.
  2. The prompt hands the AI those findings, the data and the rules; the AI confirms / rejects each one and may add
     new ones, always with object IDs from STRUCTURED_MODEL_DATA (no free text without an ID).
  3. The answer comes back as JSON (schema below) and is read into the app as a review list; people decide.
Nothing in the model data is changed by the AI.

Frame: model (X east, Y up = EL, Z south), metres.
"""
from __future__ import annotations

import json
import math
from pathlib import Path

VERSION = "pid_io-3d-review 0.1"

RULES = [
    ("V01", "트레이가 장비를 지나감", "HOLD", "트레이 상자(폭×높이)가 배관 모델 장비 상자(바닥~높이)와 겹침", "옆 차선 / 높이 변경 / 경로 변경"),
    ("V02", "랙 위 트레이가 랙 밖", "HOLD", "ladder_on_rack 트레이 중심이 같은 방향 모델 랙 폭(± half + 1 m) 밖이거나 케이블 단 EL과 0.5 m 넘게 다름", "랙 그리기 확인 (STEP 6) 또는 종류를 AG 서포트로"),
    ("V03", "떠 있는 트레이 (서포트 없음)", "HOLD", "AG 트레이 구간에서 서포트 사이 거리 > 서포트 스팬 × 1.5", "서포트 추가 (STEP 9)"),
    ("V04", "케이블 경사 경로 (라이저 없음)", "INFO", "케이블 경로 한 마디에서 높이와 수평이 같이 바뀜 (수평 > 1 m, 높이 > 0.5 m)", "수직 라이저 + 수평 구간으로 나눔"),
    ("V05", "지그재그 경로", "INFO", "1 m 안 짧은 꺾임이 3번 이상 이어짐", "경로 다듬기 (직선화)"),
    ("V06", "우리 트레이끼리 겹침", "HOLD", "같은 높이의 계장 트레이 두 개가 폭 방향으로 겹침", "차선 간격 (side_offset) 확인"),
    ("V07", "JB가 장비 안", "HOLD", "JB 위치가 장비 상자 안 (바닥~높이)", "JB 자리를 장비 가장자리/랙 기둥으로"),
    ("V08", "다른 공종과 남은 충돌", "HOLD", "계장이 비킨 뒤에도 겹침이 남음 (twin.avoid)", "수동 경로 조정 또는 상대 공종 협의"),
    ("V09", "추정 위치 계기", "INFO", "PLOT에서 장비 위치를 못 찾아 P&ID 시트 평균 위치로 둔 계기", "PLOT 장비 태그 / 수동 위치"),
]
RULE = {r[0]: r for r in RULES}


def _box_tray(t, h_extra=0.0):
    a, b = t["a"], t["b"]
    h = 0.1 + 0.3 * ((t.get("tiers") or 1) - 1) + h_extra
    lo = [min(a[0], b[0]), min(a[1], b[1]) - 0.05, min(a[2], b[2])]
    hi = [max(a[0], b[0]), max(a[1], b[1]) + h, max(a[2], b[2])]
    w = t.get("w") or 0.3
    if abs(a[0] - b[0]) >= abs(a[2] - b[2]):
        lo[2] -= w / 2; hi[2] += w / 2
    else:
        lo[0] -= w / 2; hi[0] += w / 2
    return lo, hi


def _eq_box(e):
    b = e["box"]
    return [b["x0"], 0.0, b["z0"]], [b["x1"], float(e.get("h") or 3.0), b["z1"]]


def _overlap(lo, hi, lo2, hi2, tol=0.0):
    return all(lo[i] < hi2[i] - tol and hi[i] > lo2[i] + tol for i in range(3))


def _run_overlap(t, u):
    """overlap length along the run direction (end-to-end neighbours touch only at a joint -> ~0)."""
    ax = 0 if abs(t["a"][0] - t["b"][0]) >= abs(t["a"][2] - t["b"][2]) else 2
    a0, a1 = sorted((t["a"][ax], t["b"][ax]))
    b0, b1 = sorted((u["a"][ax], u["b"][ax]))
    return min(a1, b1) - max(a0, b0)


def _mid(a, b):
    return [round((a[i] + b[i]) / 2, 2) for i in range(3)]


def precheck(inst: dict, base: dict, v: dict | None = None) -> list:
    v = v or {}
    out = []

    def add(rule, objs, where, msg):
        r = RULE[rule]
        out.append({"issue_id": f"{rule}-{sum(1 for x in out if x['rule'] == rule) + 1:03d}", "rule": rule, "title": r[1],
                    "severity": r[2], "objects": objs, "where": where, "detail": msg, "suggest": r[4], "source": "pid_io precheck"})

    eq = base.get("equipment") or {}
    eqb = {k: _eq_box(e) for k, e in eq.items()}
    trays = [t for t in inst.get("trays") or [] if t["kind"] != "ug_trench"]
    # V01 trays through equipment
    for t in trays:
        lo, hi = _box_tray(t)
        hit = [k for k, (l2, h2) in eqb.items() if _overlap(lo, hi, l2, h2, 0.05)]
        if hit:
            add("V01", [t["id"]] + hit[:5], _mid(t["a"], t["b"]), f"{t['id']} (EL {t['a'][1]} m) ↔ {', '.join(hit[:5])}")
    # V02 rack trays off the rack
    racks = list((base.get("racks") or {}).values())
    for t in trays:
        if t["kind"] != "ladder_on_rack":
            continue
        m = _mid(t["a"], t["b"])
        d_ = "NS" if abs(t["b"][2] - t["a"][2]) >= abs(t["b"][0] - t["a"][0]) else "EW"
        ok = None
        for k in racks:
            if k["dir"] != d_:
                continue
            across = m[0] if k["dir"] == "NS" else m[2]
            along = m[2] if k["dir"] == "NS" else m[0]
            inside = abs(across - k["off"]) <= k["half"] + 1.0 and (k["a0"] is None or k["a0"] - 3 <= along <= k["a1"] + 3)
            if inside:
                ok = k
                break
        if not ok and math.dist([t["a"][0], t["a"][2]], [t["b"][0], t["b"][2]]) <= 11.0:
            for k in racks:                              # short piece across a rack's width (edge to edge) is still on the rack
                across = m[0] if k["dir"] == "NS" else m[2]
                along = m[2] if k["dir"] == "NS" else m[0]
                if abs(across - k["off"]) <= k["half"] + 1.0 and (k["a0"] is None or k["a0"] - 3 <= along <= k["a1"] + 3):
                    ok = k
                    break
        if not ok:
            add("V02", [t["id"]], m, f"{t['id']} ({d_} 방향, 중심 X {m[0]} / Z {m[2]}) - 모델에 이 방향 랙이 없음 (PLOT에서만 인식한 랙?)")
        elif abs(m[1] - ok["cable_el"]) > 0.5 + 0.3 * 3:
            add("V02", [t["id"], "RACK-" + ok["id"]], m, f"{t['id']} EL {m[1]} m, 랙 {ok['id']} 케이블 단 EL {ok['cable_el']} m")
    # V03 floating AG trays (supports along the tray)
    span = float(v.get("sp_span", 3.0) or 3.0)
    sups = [s for s in inst.get("supports") or [] if s["type"] in ("L", "T", "H", "RT", "RF", "RC")]
    for t in trays:
        if t["kind"] != "ladder_on_supports":
            continue
        a, b = t["a"], t["b"]
        L = math.dist([a[0], a[2]], [b[0], b[2]])
        if L < span:
            continue
        ux, uz = (b[0] - a[0]) / L, (b[2] - a[2]) / L
        pos = []
        for s in sups:
            dx, dz = s["p"][0] - a[0], s["p"][2] - a[2]
            along, off = dx * ux + dz * uz, abs(-dx * uz + dz * ux)
            if -0.6 <= along <= L + 0.6 and off <= 1.0:
                pos.append(min(L, max(0.0, along)))
        pos = sorted(set([0.0, L] if not pos else pos + [0.0, L]))
        gaps = [(pos[i + 1] - pos[i], pos[i]) for i in range(len(pos) - 1)]
        big = [g for g in gaps if g[0] > span * 1.5]
        if big:
            g = max(big)
            where = [round(a[0] + ux * (g[1] + g[0] / 2), 2), a[1], round(a[2] + uz * (g[1] + g[0] / 2), 2)]
            add("V03", [t["id"]], where, f"{t['id']} 서포트 사이 최대 {round(g[0], 1)} m (스팬 {span} m)")
    # V04 / V05 cable paths
    for c in inst.get("cables") or []:
        P = c.get("path") or []
        slant = None
        short = run = 0
        for i in range(len(P) - 1):
            p, q = P[i], P[i + 1]
            hz, dy = math.dist([p[0], p[2]], [q[0], q[2]]), abs(q[1] - p[1])
            if hz > 1.0 and dy > 0.5 and slant is None:
                slant = (p, q, hz, dy)
            if 0 < hz + dy < 1.0:
                run += 1
                short = max(short, run)
            else:
                run = 0
        if slant:
            p, q, hz, dy = slant
            add("V04", [c["id"]], _mid(p, q), f"{c['id']} 수평 {round(hz, 1)} m 동안 높이 {round(dy, 1)} m 변함")
        if short >= 3:
            add("V05", [c["id"]], P[0], f"{c['id']} 1 m 안 꺾임 {short}번 연속")
    # V06 own tray overlaps
    boxes = [(t, *_box_tray(t)) for t in trays]
    for i in range(len(boxes)):
        t, lo, hi = boxes[i]
        for j in range(i + 1, len(boxes)):
            u, lo2, hi2 = boxes[j]
            if abs(t["a"][1] - u["a"][1]) > 0.05:
                continue
            if _overlap(lo, hi, lo2, hi2, 0.02) and _run_overlap(t, u) > 0.5:
                add("V06", [t["id"], u["id"]], _mid(t["a"], t["b"]), f"{t['id']} ↔ {u['id']} (EL {t['a'][1]} m)")
    # V07 JBs inside equipment
    for j in inst.get("jbs") or []:
        p = j["p"]
        hit = [k for k, (l2, h2) in eqb.items() if l2[0] + 0.1 < p[0] < h2[0] - 0.1 and l2[2] + 0.1 < p[2] < h2[2] - 0.1 and p[1] < h2[1]]
        if hit:
            add("V07", [j["id"]] + hit[:3], p, f"{j['id']} (EL {p[1]} m) 가 {', '.join(hit[:3])} 상자 안")
    # V08 residual clashes
    for m in inst.get("moves") or []:
        if m["action"].startswith("충돌"):
            t = next((x for x in inst["trays"] if x["id"] == m["tray"]), None)
            add("V08", [m["tray"]] + list(m.get("with") or [])[:5], _mid(t["a"], t["b"]) if t else None, f"{m['tray']} {m['action']}")
    # V09 estimated instruments (one row with the list)
    est = [x["id"] for x in inst.get("instruments") or [] if x.get("est")]
    if est:
        add("V09", est[:200], None, f"추정 위치 계기 {len(est)}개")
    return out


def summary(issues: list) -> dict:
    from collections import Counter
    c = Counter(x["rule"] for x in issues)
    return {"total": len(issues), "hold": sum(1 for x in issues if x["severity"] == "HOLD"),
            "by_rule": {r[0]: {"title": r[1], "n": c.get(r[0], 0)} for r in RULES}}


ANSWER_SCHEMA = {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "title": "pid_io 3D review answer",
    "type": "object",
    "required": ["review_version", "checked", "added"],
    "properties": {
        "review_version": {"const": VERSION},
        "checked": {"type": "array", "description": "precheck 항목마다 하나",
                    "items": {"type": "object", "required": ["issue_id", "verdict", "reason"],
                              "properties": {"issue_id": {"type": "string"},
                                             "verdict": {"enum": ["CONFIRMED", "FALSE_ALARM", "NEEDS_PERSON"]},
                                             "reason": {"type": "string", "description": "한국어, 근거가 된 객체 ID·값"},
                                             "fix": {"type": "string"}}}},
        "added": {"type": "array", "description": "precheck에 없던 새 지적 (객체 ID 필수)",
                  "items": {"type": "object", "required": ["objects", "title", "severity", "reason"],
                            "properties": {"objects": {"type": "array", "minItems": 1, "items": {"type": "string"}},
                                           "title": {"type": "string"},
                                           "severity": {"enum": ["HOLD", "INFO"]},
                                           "where": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3},
                                           "reason": {"type": "string"}, "fix": {"type": "string"}}}},
        "overall": {"type": "string"},
    },
}


def prompt(project: str, issues: list, counts: dict, model_name: str, data_name: str) -> str:
    rules = "\n".join(f"- {r[0]} {r[1]} [{r[2]}]: {r[3]}" for r in RULES)
    lines = "\n".join(json.dumps({k: x[k] for k in ("issue_id", "rule", "objects", "where", "detail")}, ensure_ascii=False)
                      for x in issues if x["rule"] != "V09")
    est = next((x for x in issues if x["rule"] == "V09"), None)
    return f"""# 3D 모델 리뷰 (계장 보강분) — {project}

당신은 계장 3D 개념설계 검토자입니다. 배관팀 3D 모델 위에 pid_io가 얹은 계장 데이터(계기·JB·케이블·트레이·서포트)를 검토합니다.
첨부: `{model_name}` (보강 모델, 레이어 '계장'), `{data_name}` (모든 객체의 ID·좌표·속성), 이 파일의 사전 점검 목록.
좌표: 모델 기준 X 동쪽, Y 위(EL), Z 남쪽, 단위 m. 객체 수: {json.dumps(counts, ensure_ascii=False)}

## 규칙 (pid_io가 먼저 기계적으로 점검한 것)
{rules}

## 할 일
1. 아래 사전 점검 항목마다 모델/데이터를 보고 `CONFIRMED`(맞음) / `FALSE_ALARM`(오탐) / `NEEDS_PERSON`(사람 판단) 중 하나로 답하고, 근거(객체 ID·값)를 한국어로 짧게.
2. 사전 점검에 없는 문제를 찾으면 `added`에 넣되, **반드시 데이터에 있는 객체 ID**를 쓰세요. ID 없는 지적은 받지 않습니다.
   예: 트레이가 배관/구조물 사이로 비정상적으로 지나감, 랙 단을 건너뛰는 경로, 계기에서 JB까지 너무 긴 분기, 서포트가 도로 위.
3. 값은 고치지 말고 제안만 하세요 (사람이 승인). 계장은 항상 비키는 쪽입니다 (다른 공종을 옮기라는 제안은 하지 마세요).
4. 답은 JSON 하나만 (스키마 `3D_REVIEW_ANSWER_SCHEMA.json`, review_version = "{VERSION}").

## 사전 점검 목록 ({len(issues)}건{f", 그중 추정 위치 계기 {len(est['objects'])}개는 목록 생략" if est else ""})
{lines or "(없음)"}
"""


def parse_answer(text: str, issues: list) -> dict:
    """company AI answer -> {"checked": [...], "added": [...], "errors": [...]}; IDs are checked against the data."""
    from .ai.tasks import extract_json
    d = extract_json(text, want=("checked", "added"))
    ids = {x["issue_id"] for x in issues}
    out = {"checked": [], "added": [], "errors": [], "overall": str(d.get("overall", ""))[:500]}
    for c in d.get("checked") or []:
        if c.get("issue_id") not in ids:
            out["errors"].append(f"없는 issue_id: {c.get('issue_id')}")
            continue
        if c.get("verdict") not in ("CONFIRMED", "FALSE_ALARM", "NEEDS_PERSON"):
            out["errors"].append(f"{c['issue_id']}: verdict 값 오류")
            continue
        out["checked"].append({k: str(c.get(k, ""))[:300] for k in ("issue_id", "verdict", "reason", "fix")})
    for a in d.get("added") or []:
        objs = [str(o) for o in a.get("objects") or []]
        if not objs:
            out["errors"].append(f"객체 ID 없는 지적 제외: {str(a.get('title', ''))[:40]}")
            continue
        out["added"].append({"objects": objs[:10], "title": str(a.get("title", ""))[:80], "severity": a.get("severity", "INFO"),
                             "where": a.get("where"), "reason": str(a.get("reason", ""))[:300], "fix": str(a.get("fix", ""))[:200]})
    return out


def known_ids(inst: dict, base: dict) -> set:
    s = set()
    for k in ("instruments", "jbs", "cables", "trays", "supports", "joints", "rooms"):
        s |= {o["id"] for o in inst.get(k) or []}
    s |= set((base.get("equipment") or {}).keys()) | {"RACK-" + r for r in (base.get("racks") or {})}
    return s


def write_packet(outdir: Path, project: str, inst: dict, base: dict, v: dict, model_name: str, data_name: str) -> dict:
    import csv
    issues = precheck(inst, base, v)
    outdir.mkdir(parents=True, exist_ok=True)
    (outdir / "3D_REVIEW_PRECHECK.json").write_text(json.dumps({"version": VERSION, "issues": issues, "summary": summary(issues)},
                                                               ensure_ascii=False, indent=1), encoding="utf-8")
    with open(outdir / "3D_REVIEW_PRECHECK.csv", "w", encoding="utf-8-sig", newline="") as f:
        w = csv.writer(f)
        w.writerow(["issue_id", "규칙", "제목", "등급", "객체", "위치 X,Y,Z (m)", "내용", "제안", "모델에서 찾기"])
        for x in issues:
            w.writerow([x["issue_id"], x["rule"], x["title"], x["severity"], " ".join(x["objects"][:20]),
                        ",".join(map(str, x["where"])) if x["where"] else "", x["detail"], x["suggest"],
                        f"{model_name}#find={x['objects'][0]}" if x["objects"] else ""])
    (outdir / "3D_REVIEW_PROMPT.md").write_text(prompt(project, issues, inst.get("counts") or {}, model_name, data_name), encoding="utf-8")
    (outdir / "3D_REVIEW_ANSWER_SCHEMA.json").write_text(json.dumps(ANSWER_SCHEMA, ensure_ascii=False, indent=1), encoding="utf-8")
    return {"issues": issues, "summary": summary(issues)}
