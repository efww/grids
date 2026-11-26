"""STEP 9 - tray / trench supports from the STEP 8 segments.

Nothing new is routed here: every support stands on a STEP 8 segment (or a perforated tray).

  ladder tray off the rack (AG)  -> a support every `sp_span` m along the segment, both ends included,
                                    so corners and tees get one; type by width / tiers (L, T, portal)
  ladder tray on the rack        -> the rack carries it. One tier: hold-down clamps only, every rack bay.
                                    Stacked tiers (900 x n): a T support (post on the rack beam) every bay
  UG trench                      -> concrete trench: cable brackets every `sp_ug_span` m;
                                    direct buried: route markers; duct bank: pull pits
  perforated tray (JB -> instr.) -> quantity only, tied to the instrument tag: ceil(length / span) brackets

Members (post / arm / beam lengths, steel kg/m, anchors, clamps) are general values. When the user's
typical support drawing arrives its numbers go into the same settings (and a preset); the typical
drawing number per type is a setting too, so every support row carries it.
"""
from __future__ import annotations

import math
from collections import Counter

from .schema import _num, _sel, _txt


def _code(d, code):
    d["code"] = code
    return d


TYPES = {
    "L": "L형 (캔틸레버)",
    "T": "T형 (포스트 + 양팔)",
    "H": "문형 (포스트 2 + 빔)",
    "RT": "랙 위 T 서포트 (적층)",
    "RC": "랙 위 클램프",
    "UB": "트렌치 브래킷",
    "UM": "직매설 표지",
    "UP": "덕트 풀 피트",
    "PB": "Perforated 브래킷",
}

SCHEMA = [
    {"id": "sp_ag", "title": "AG 서포트 (랙 밖 사다리)",
     "lead": "랙을 타지 않는 사다리 트레이 구간(STEP 6 '랙 없는 구간 (간선)' = AG일 때)에 간격마다 서포트를 세웁니다. "
             "구간 양 끝을 포함하므로 꺾임·분기점에는 항상 서포트가 있습니다. 형식은 폭·단 수로 자동 선택합니다.",
     "items": [
         _code(_num("sp_span", "사다리 트레이 서포트 간격", 3.0, "m", "일반값 2~3 m. 타피컬 드로잉의 최대 스팬으로 바꾸세요", 0.5), "S01"),
         _code(_sel("sp_type", "서포트 형식", ["자동 (폭·단 기준)", "L형", "T형", "문형"], "자동 (폭·단 기준)",
                    "자동: 폭 ≤ L형 최대 폭이고 1단이면 L형, ≤ T형 최대 폭이고 1단이면 T형, 그 밖(넓거나 적층)은 문형"), "S02"),
         _code(_num("sp_l_max", "L형 최대 폭", 300, "mm"), "S03"),
         _code(_num("sp_t_max", "T형 최대 폭", 600, "mm"), "S04"),
         _code(_num("sp_tier_gap", "적층 단 간격", 300, "mm", "2단 이상일 때 단과 단 사이"), "S05"),
         _code(_num("sp_arm_add", "팔(빔) 여유 길이", 150, "mm", "트레이 폭 + 이 값 = 팔 길이 (문형은 양쪽)"), "S06"),
     ]},
    {"id": "sp_rack", "title": "랙 위",
     "lead": "랙 최상단을 타는 사다리 트레이는 랙이 받칩니다. 1단이면 랙 빔마다 클램프만, 단을 쌓으면(900 × n단) 랙 빔 위에 T 서포트를 세웁니다.",
     "items": [
         _code(_sel("sp_rack_stack", "랙 위 적층 트레이", ["T 서포트", "서포트 없음"], "T 서포트",
                    "2단 이상이면 랙 빔 위에 포스트를 세워 위 단을 받침", on="T 서포트"), "S10"),
         _code(_num("sp_bay", "랙 빔 간격 (베이)", 6.0, "m", "클램프·T 서포트가 서는 간격", 0.5), "S11"),
         _code(_num("sp_clamp", "빔당 클램프 (트레이 1단당)", 2, "개", "홀드다운 클램프. 양쪽 레일"), "S12"),
     ]},
    {"id": "sp_ug", "title": "UG · Perforated",
     "lead": "UG 구간은 방식에 따라 트렌치 브래킷 / 직매설 표지 / 덕트 풀 피트를, Perforated는 길이로 브래킷 수만 냅니다 (계기 태그별).",
     "items": [
         _code(_sel("sp_ug_type", "UG 방식", ["콘크리트 트렌치 (브래킷)", "직매설", "덕트 뱅크"], "콘크리트 트렌치 (브래킷)"), "S20"),
         _code(_num("sp_ug_span", "트렌치 브래킷 간격", 1.5, "m", "트렌치 벽 케이블 브래킷", 0.5), "S21"),
         _code(_num("sp_marker", "직매설 표지 간격", 30, "m", "꺾임점에도 1개"), "S22"),
         _code(_num("sp_pit", "덕트 풀 피트 간격", 50, "m", "꺾임점에도 1개"), "S23"),
         _code(_num("sp_pt_span", "Perforated 브래킷 간격", 1.5, "m", "JB → 계기 Perforated 트레이", 0.5), "S24"),
     ]},
    {"id": "sp_mat", "title": "자재 · 타피컬",
     "lead": "서포트 1개의 구성 자재 일반값입니다. 타피컬 드로잉(Typical Support Drawing)이 오면 도면 번호와 값을 넣고 프리셋으로 저장하세요.",
     "items": [
         _code(_txt("sp_member", "주 부재", "C100×50×5/7.5", "포스트·팔·빔"), "S30"),
         _code(_num("sp_kgm", "주 부재 단위 중량", 9.36, "kg/m", "C100×50×5/7.5 = 9.36 kg/m", 0.01), "S31"),
         _code(_num("sp_pb_kgm", "Perforated 브래킷 단위 중량", 4.43, "kg/m", "L50×50×6 = 4.43 kg/m", 0.01), "S32"),
         _code(_num("sp_pb_len", "Perforated 브래킷 1개 길이", 0.5, "m", "", 0.1), "S33"),
         _code(_num("sp_ub_len", "트렌치 브래킷 1개 길이", 0.4, "m", "", 0.1), "S34"),
         _code(_num("sp_anchor", "베이스 플레이트당 앵커 볼트", 4, "개"), "S35"),
         _code(_num("sp_fit", "서포트 자재 할증", 0.1, "", "", 0.05, True), "S36"),
         _code(_txt("sp_typ_L", "타피컬 도면 번호 - L형", "TS-01 (일반값)"), "S40"),
         _code(_txt("sp_typ_T", "타피컬 도면 번호 - T형", "TS-02 (일반값)"), "S41"),
         _code(_txt("sp_typ_H", "타피컬 도면 번호 - 문형", "TS-03 (일반값)"), "S42"),
         _code(_txt("sp_typ_RT", "타피컬 도면 번호 - 랙 위 T", "TS-04 (일반값)"), "S43"),
         _code(_txt("sp_typ_UB", "타피컬 도면 번호 - 트렌치 브래킷", "TS-05 (일반값)"), "S44"),
         _code(_txt("sp_typ_PB", "타피컬 도면 번호 - Perforated 브래킷", "TS-06 (일반값)"), "S45"),
     ]},
]
ITEMS = {it["id"]: it for g in SCHEMA for it in g["items"]}


def typical(t, v):
    return v.get(f"sp_typ_{t}", "") or ""


def ag_type(width, tiers, v):
    f = v["sp_type"]
    if f.startswith("L"):
        return "L"
    if f.startswith("T"):
        return "T"
    if f.startswith("문"):
        return "H"
    if tiers <= 1 and width <= float(v["sp_l_max"]):
        return "L"
    if tiers <= 1 and width <= float(v["sp_t_max"]):
        return "T"
    return "H"


RANK = {"L": 0, "T": 1, "H": 2}


def members(t, width, tiers, el, v):
    """(main member m, base plates, anchors, clamps) for one support. el = height of the lowest tier (m)."""
    arm = (width + float(v["sp_arm_add"])) / 1000
    gap = float(v["sp_tier_gap"]) / 1000
    top = el + (tiers - 1) * gap
    cl = float(v["sp_clamp"]) * tiers
    anc = float(v["sp_anchor"])
    if t == "L":
        return top + arm * tiers, 1, anc, cl
    if t == "T":
        return top + 2 * arm * tiers, 1, anc, cl
    if t == "H":
        return 2 * top + (arm + float(v["sp_arm_add"]) / 1000) * tiers, 2, 2 * anc, cl
    if t == "RT":                                  # post on the rack beam, arms for the upper tiers
        return (tiers - 1) * gap + 0.1 + 2 * arm * (tiers - 1), 1, anc, cl
    if t == "RC":
        return 0.0, 0, 0, cl
    if t == "UB":
        return float(v["sp_ub_len"]), 0, 2, 0
    return 0.0, 0, 0, 0


MIN_SPAN = 0.5                                     # a spacing of 0 would mean a support every millimetre


def span_of(x):
    return max(float(x), MIN_SPAN)


def _along(a, b, length, span):
    n = max(1, math.ceil(length / span_of(span) - 1e-9))
    return [(a[0] + (b[0] - a[0]) * i / n, a[1] + (b[1] - a[1]) * i / n) for i in range(n + 1)]


def build(tray, v, m_per_pt=None):
    """tray = tray.build(...) output. -> supports, per-segment rows, perforated per tag, summary."""
    out = {"supports": [], "segments": [], "pt": [], "summary": {}}
    gl_el = float(v["sup_el"])
    rack_el = float(v["rack_el"])
    m = m_per_pt or 0
    pos = {}                                       # node key -> support index (AG and rack posts share corners)

    def key(p):
        return (round(p[0] * m * 10), round(p[1] * m * 10)) if m else (round(p[0] * 10), round(p[1] * 10))

    def add(kind, p, seg, info):
        k = (key(p), kind in ("UB", "UM", "UP"))
        i = pos.get(k)
        if i is not None:                          # shared corner / tee: one support, the bigger type wins
            s = out["supports"][i]
            if kind in RANK and s["type"] in RANK and RANK[kind] > RANK[s["type"]]:
                s.update(type=kind, **info)
            elif kind in ("H", "T", "L", "RT") and s["type"] == "RC":
                s.update(type=kind, **info)
            elif s["type"] == kind:
                for f in ("width", "tiers"):
                    if f in info:
                        s[f] = max(s.get(f, 0), info[f])
            if seg not in s["segs"]:
                s["segs"].append(seg)
            return i
        pos[k] = len(out["supports"])
        out["supports"].append({"type": kind, "x": round(p[0], 1), "y": round(p[1], 1), "segs": [seg], **info})
        return pos[k]

    for s in tray.get("segments", []):
        a, b = s["poly"]
        trs = s.get("trays") or []
        idx = []
        if s["kind"] == "UG":
            ut = v["sp_ug_type"]
            kind, span = (("UB", float(v["sp_ug_span"])) if ut.startswith("콘크리트")
                          else ("UM", float(v["sp_marker"])) if ut.startswith("직매설") else ("UP", float(v["sp_pit"])))
            for p in _along(a, b, s["length"], span):
                idx.append(add(kind, p, s["no"], {"el": -float(v["ug_depth"]), "width": 0, "tiers": 1}))
        else:
            w = max((t_["width"] for t_ in trs), default=300)
            n = max((t_["tiers"] for t_ in trs), default=1)
            wsum = sum(t_["width"] for t_ in trs) + 100 * max(0, len(trs) - 1)   # trays side by side on one support
            if s["kind"] == "랙":
                kind = "RT" if (n > 1 and v["sp_rack_stack"] == "T 서포트") else "RC"
                for p in _along(a, b, s["length"], float(v["sp_bay"])):
                    idx.append(add(kind, p, s["no"], {"el": rack_el, "width": wsum, "tiers": n,
                                                       "trays": sum(t_["tiers"] for t_ in trs)}))
            else:
                kind = ag_type(wsum, n, v)
                for p in _along(a, b, s["length"], float(v["sp_span"])):
                    idx.append(add(kind, p, s["no"], {"el": gl_el, "width": wsum, "tiers": n,
                                                       "trays": sum(t_["tiers"] for t_ in trs)}))
        out["segments"].append({"seg": s["no"], "kind": s["kind"], "length": s["length"], "idx": idx,
                                "trays": [t_["no"] for t_ in trs]})
    # numbering: per type group, nearest the control room first = segment order from tray.build
    num = Counter()
    pre = {"L": "SP", "T": "SP", "H": "SP", "RT": "SP", "RC": "CL", "UB": "UB", "UM": "UM", "UP": "UP"}
    for s in out["supports"]:
        u = s["segs"][0].split("-")[1] if s["segs"][0].count("-") >= 2 else "00"
        p = pre[s["type"]]
        num[(p, u)] += 1
        s["no"] = f"{p}-{u}-{num[(p, u)]:03d}"
        s["typical"] = typical(s["type"], v) if s["type"] not in ("RC", "UM", "UP") else ""
        mm, bp, an, cl = members(s["type"], s.get("width", 300), s.get("tiers", 1),
                                 s["el"] if s["type"] in ("L", "T", "H") else 0, v)
        s.update(steel_m=round(mm, 2), base=bp, anchors=an, clamps=cl,
                 kg=round(mm * float(v["sp_kgm"]), 1), name=TYPES[s["type"]])
        if m:
            s["x_m"], s["y_m"] = round(s["x"] * m, 2), round(s["y"] * m, 2)
    for sg in out["segments"]:
        nos = [out["supports"][i]["no"] for i in sg["idx"]]
        own = [out["supports"][i]["no"] for i in sg["idx"] if out["supports"][i]["segs"][0] == sg["seg"]]
        sg["count"] = len(nos)
        sg["shared"] = len(nos) - len(own)          # corners / tees numbered with the neighbouring segment
        sg["types"] = dict(Counter(out["supports"][i]["type"] for i in sg["idx"]))
        sg["range"] = (f"{own[0]} ~ {own[-1]}" if len(own) > 1 else (own[0] if own else "")) + \
                      (f" (+공유 {sg['shared']})" if sg["shared"] else "")
        sg["nos"] = nos
        del sg["idx"]
    # perforated: quantity per instrument tag
    span = span_of(v["sp_pt_span"])
    for p in tray.get("pt", []):
        n = max(1, math.ceil(p["length"] / span - 1e-9))
        out["pt"].append({"tag": p["to"], "pt": p["no"], "jb": p["frm"], "length": p["length"], "count": n,
                          "steel_m": round(n * float(v["sp_pb_len"]), 2),
                          "kg": round(n * float(v["sp_pb_len"]) * float(v["sp_pb_kgm"]), 1),
                          "typical": typical("PB", v)})
    out["summary"] = _summary(out, v)
    return out


def _summary(sp, v):
    fit = float(v["sp_fit"])
    by = {}
    for s in sp["supports"]:
        k = s["type"]
        x = by.setdefault(k, {"type": k, "name": TYPES[k], "count": 0, "steel_m": 0.0, "kg": 0.0, "base": 0,
                              "anchors": 0, "clamps": 0, "typical": s.get("typical", "")})
        x["count"] += 1
        for f in ("steel_m", "kg", "base", "anchors", "clamps"):
            x[f] += s[f]
    if sp["pt"]:
        by["PB"] = {"type": "PB", "name": TYPES["PB"], "count": sum(p["count"] for p in sp["pt"]),
                    "steel_m": sum(p["steel_m"] for p in sp["pt"]), "kg": sum(p["kg"] for p in sp["pt"]),
                    "base": 0, "anchors": 2 * sum(p["count"] for p in sp["pt"]), "clamps": 0,
                    "typical": typical("PB", v)}
    order = list(TYPES)
    rows = sorted(by.values(), key=lambda x: order.index(x["type"]))
    for x in rows:
        x["steel_m"] = round(x["steel_m"], 1)
        x["kg"] = round(x["kg"], 1)
        x["kg_order"] = math.ceil(x["kg"] * (1 + fit))
        x["anchors"] = int(x["anchors"])
        x["clamps"] = int(x["clamps"])
    return {"rows": rows, "supports": sum(x["count"] for x in rows if x["type"] in ("L", "T", "H", "RT")),
            "clamps": sum(x["clamps"] for x in rows), "kg": round(sum(x["kg"] for x in rows), 1),
            "kg_order": sum(x["kg_order"] for x in rows), "pt_n": by.get("PB", {}).get("count", 0),
            "ug": sum(x["count"] for x in rows if x["type"] in ("UB", "UM", "UP"))}


def checklist(v):
    out = []
    for g in SCHEMA:
        for it in g["items"]:
            val = v.get(it["id"], it["default"])
            out.append({"code": it["code"], "group": g["title"], "name": it["name"], "value": val,
                        "default": it["default"], "changed": str(val) != str(it["default"]), "help": it.get("help", "")})
    return out


def write_sheets(wb, sp, v, sheet):
    sm = sp["summary"]
    sheet(wb.create_sheet("Support_Summary"),
          ["형식", "타피컬", "개수", "주 부재 (m)", "중량 (kg)", "중량 (할증, kg)", "베이스", "앵커", "클램프"],
          [[x["name"], x["typical"], x["count"], x["steel_m"], x["kg"], x["kg_order"], x["base"], x["anchors"], x["clamps"]]
           for x in sm["rows"]] + [["", "", "", "", "", "", "", "", ""],
                                   ["(일반값 기준. 타피컬 드로잉이 오면 STEP 9 '자재 · 타피컬' 값을 바꾸세요)", "", "", "", "", "", "", "", ""]],
          [26, 18, 8, 12, 12, 14, 8, 8, 8])
    sheet(wb.create_sheet("Support_List"),
          ["Support No", "형식", "타피컬", "구간 (Tray / UG)", "폭 합 (mm)", "단 수", "EL (m)", "X (m)", "Y (m)",
           "X (pt)", "Y (pt)", "주 부재 (m)", "중량 (kg)", "앵커", "클램프"],
          [[s["no"], s["name"], s["typical"], ", ".join(s["segs"]), s.get("width", ""), s.get("tiers", ""), s["el"],
            s.get("x_m", ""), s.get("y_m", ""), s["x"], s["y"], s["steel_m"], s["kg"], s["anchors"], s["clamps"]]
           for s in sp["supports"]], [14, 20, 16, 26, 9, 6, 7, 9, 9, 9, 9, 10, 9, 6, 7])
    sheet(wb.create_sheet("Support_by_Segment"),
          ["구간", "종류", "트레이", "길이 (m)", "서포트 수", "형식", "번호"],
          [[g["seg"], g["kind"], ", ".join(g["trays"]), g["length"], g["count"],
            ", ".join(f"{TYPES[k]} {n}" for k, n in g["types"].items()), g["range"]] for g in sp["segments"]],
          [14, 8, 28, 9, 9, 30, 30])
    sheet(wb.create_sheet("Support_Perforated"),
          ["Tag (계기)", "PT No", "From (JB)", "길이 (m)", "브래킷 수", "부재 (m)", "중량 (kg)", "타피컬"],
          [[p["tag"], p["pt"], p["jb"], p["length"], p["count"], p["steel_m"], p["kg"], p["typical"]] for p in sp["pt"]],
          [20, 14, 18, 9, 9, 9, 9, 16])
    sheet(wb.create_sheet("Support_Checklist"), ["No", "구분", "항목", "적용값", "기본값", "", "설명"],
          [[c["code"], c["group"], c["name"], c["value"], c["default"], "변경" if c["changed"] else "", c["help"]]
           for c in checklist(v)], [6, 18, 30, 26, 26, 6, 70])
