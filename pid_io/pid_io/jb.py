"""STEP 7 - junction boxes: size, spare, place on the plot, specification.

What comes down from STEP 6 (cable): which instruments share a JB (segregation:
system, analogue / digital, DO, RTD, IS, package) and the cable specs.
What is decided here: how big a JB is (40 / 60 / 80P with spare), where it
stands on the plot (rack column or equipment edge, never anywhere), and what
it is (material, Ex, IP, terminals, glands).

Sizing rule (one rule, two spares):
    multi pairs   M >= n x (1 + cable spare)       n = field cables (1 IO = 1 pair)
    JB terminals  T >= M                           every multi pair lands on a terminal
    JB terminals  T >= n x (1 + terminal spare)
With the multi sized like the JB the two spares end up the same number.

Placement: slots first, then choose. A slot is a place a JB can really be
mounted: the outer face of a pipe-rack column, or 1-2 m off an equipment edge.
Slots inside racks, equipment or buildings do not exist, so a JB can never
overlap them. JBs of different systems / signals at the same slot form one
station (one mark on the plot). Users drag a JB onto another slot; the move is
kept by slot name (rack 3 column 7, P-0602 north side), not by coordinates.
"""
from __future__ import annotations

import math
import re

from .schema import _num, _sel, _txt


def _code(d, code):
    d["code"] = code
    return d


SCHEMA = [
    {"id": "jbsize", "title": "크기 · 스페어",
     "lead": "JB에 들어오는 1P 분기 케이블 수(n)로 JB 크기와 멀티를 정합니다. 멀티 페어 M ≥ n × (1+케이블 스페어), "
             "JB 단자 T ≥ M 그리고 T ≥ n × (1+단자 스페어). 멀티를 JB 크기와 같게 하면 두 스페어는 같은 값이 됩니다.",
     "items": [
         _code(_txt("jb_sizes", "JB 크기 (페어)", "40,60,80",
                    "쉼표로 구분. 받을 1P 수에 맞는 가장 작은 크기를 자동으로 고릅니다. RTD JB는 같은 숫자를 Triad(T)로 씁니다"), "J01"),
         _code(_num("jb_spare", "케이블 스페어율 (멀티)", 0.2, "", "멀티 페어 ≥ 사용 × (1 + 스페어율)", 0.05, True), "J02"),
         _code(_num("term_spare", "단자 스페어율 (JB)", 0.2, "",
                    "JB 단자 ≥ 사용 × (1 + 스페어율). 케이블 스페어와 같으면 따로 볼 필요 없습니다", 0.05, True), "J03"),
         _code(_sel("multi_sizing", "멀티 케이블 크기", ["JB 크기와 같게", "사용+스페어 최소 규격"], "JB 크기와 같게",
                    "JB 크기와 같게: 40P JB = 40P 멀티 1가닥. 최소 규격: 아래 규격 중 사용+스페어를 담는 가장 작은 것"), "J04"),
         _code(_txt("multi_std", "멀티 케이블 규격 (최소 규격일 때)", "12,24,40,60,80", "쉼표로 구분 (페어)"), "J05"),
         _code(_num("jb_radius", "JB 반경", 40, "m", "이 거리 안의 계기끼리 한 JB로 묶습니다 (분리 기준은 STEP 6)"), "J06"),
     ]},
    {"id": "jbplace", "title": "배치",
     "lead": "JB를 놓을 수 있는 자리(랙 기둥 바깥면, 장비 가장자리)를 먼저 만들고, 분기 케이블 합 + 멀티 길이가 가장 짧은 자리를 고릅니다. "
             "자리가 아닌 곳(랙 안, 장비·건물 안)에는 놓이지 않습니다.",
     "items": [
         _code(_sel("jb_place", "자동 배치", ["자리(슬롯)에 배치", "계기 중심 (예전 방식)"], "자리(슬롯)에 배치",
                    "끄면 JB를 묶인 계기들의 중심에 둡니다", on="자리(슬롯)에 배치"), "J10"),
         _code(_sel("slot_rack", "랙 기둥 자리", ["사용", "안 씀"], "사용", "랙 기둥 바깥면에 JB를 붙입니다", on="사용"), "J11"),
         _code(_sel("slot_eq", "장비 가장자리 자리", ["사용", "안 씀"], "사용", "장비·패키지 외곽 각 변의 가운데", on="사용"), "J12"),
         _code(_sel("slot_3d_eq", "3D 모델 장비 상자 피하기", ["안 함", "사용 (3D 모델 있을 때)"], "안 함",
                   "배관 3D 모델(STEP 10)의 장비 상자 안 자리는 쓰지 않습니다. 물량이 바뀌므로 STEP 10 '3D 반영 제안'에서 승인하고 켭니다",
                   on="사용 (3D 모델 있을 때)"), "J1A"),
        _code(_num("rack_off", "랙 기둥에서 띄우는 거리", 1.0, "m", "기둥 중심에서 랙 바깥쪽으로", 0.5), "J13"),
         _code(_num("eq_off", "장비에서 띄우는 거리", 1.5, "m", "장비 외곽선에서 바깥쪽으로 (통행·정비 공간)", 0.5), "J14"),
         _code(_num("rack_reach", "랙 우선 거리", 20, "m",
                    "묶인 계기 중심에서 이 거리 안에 랙 자리가 있으면 랙에만 붙입니다. 없을 때만 장비 가장자리를 씁니다 (패키지 Skid JB는 Skid 가장자리 우선)"), "J15"),
         _code(_num("st_max", "스테이션 당 최대 JB", 4, "개", "한 자리에 나란히 붙일 수 있는 JB 수 (DCS·ESD·IS ...)"), "J16"),
         _code(_num("st_bonus", "스테이션 합치기", 15, "m",
                    "이미 JB가 있는 자리를 이만큼 짧은 것으로 봅니다. 클수록 JB가 한 자리에 모여 도면 표시가 줄어듭니다"), "J17"),
         _code(_num("reassign", "계기 다시 배정 횟수", 2, "회", "자리가 정해진 뒤 계기를 가장 가까운 같은 분류 JB로 옮기는 반복"), "J18"),
         _code(_num("small_jb", "작은 JB 합치기", 4, "가닥 이하",
                    "이 수 이하만 받는 JB는 같은 분류(같은 STEP 6 묶음)의 가까운 JB에 합칩니다 (반경 × 2 안, 크기 여유가 있을 때). 0이면 안 합침"), "J19"),
     ]},
    {"id": "jbspec", "title": "JB 사양",
     "lead": "JB 리스트에 들어가는 사양입니다. IS JB는 아래 'IS JB 방폭'을 씁니다.",
     "items": [
         _code(_sel("jb_mat", "재질", ["SUS316", "SUS304", "GRP", "Al (Cu-free)"], "SUS316"), "J20"),
         _code(_sel("jb_ex", "방폭 (Non-IS JB)", ["Ex e", "Ex d", "Ex e/d", "비방폭"], "Ex e"), "J21"),
         _code(_sel("jb_ex_is", "IS JB 방폭", ["Ex i (IS)", "Ex e"], "Ex i (IS)"), "J22"),
         _code(_txt("jb_gas", "가스 그룹 · 온도 등급", "IIC T4", "수소는 IIC"), "J23"),
         _code(_sel("jb_ip", "보호 등급", ["IP66", "IP65", "IP67"], "IP66"), "J24"),
         _code(_sel("term_pp", "페어당 단자", ["3 (+, −, 실드)", "2 (+, −)"], "3 (+, −, 실드)",
                    "개별 차폐 케이블은 페어마다 실드 단자"), "J25"),
         _code(_num("gland_spare", "글랜드 스페어율", 0.2, "", "분기 글랜드 = 사용 × (1 + 스페어율), 멀티 글랜드 1", 0.05, True), "J26"),
         _code(_sel("jb_entry", "케이블 인입", ["하부", "측면", "상부"], "하부"), "J27"),
     ]},
]
ITEMS = {it["id"]: it for g in SCHEMA for it in g["items"]}


def _ints(text, fallback):
    out = sorted({int(x) for x in re.findall(r"\d+", str(text or "")) if int(x) > 0})
    return out or fallback


def sizes(v):
    return _ints(v.get("jb_sizes"), [40, 60, 80])


def capacity(v):
    """most field cables one JB can take (largest size, the stricter spare)."""
    cap = int(math.floor(sizes(v)[-1] / (1 + float(v["term_spare"])) + 1e-9))
    top_m = sizes(v)[-1] if v["multi_sizing"].startswith("JB") else min(sizes(v)[-1], _ints(v.get("multi_std"), [80])[-1])
    cap = min(cap, int(math.floor(top_m / (1 + float(v["jb_spare"])) + 1e-9)))
    return max(1, cap)


def size_for(n, v):
    """n field cables -> (JB size T, multi pairs M). Smallest T that holds both spares."""
    cs, ts = float(v["jb_spare"]), float(v["term_spare"])
    need_m = int(math.ceil(n * (1 + cs) - 1e-9))
    need_t = int(math.ceil(n * (1 + ts) - 1e-9))
    js = sizes(v)
    if v["multi_sizing"].startswith("JB"):
        t = next((s for s in js if s >= max(need_m, need_t)), js[-1])
        return t, t
    ms = _ints(v.get("multi_std"), [12, 24, 40, 60, 80])
    m = next((s for s in ms if s >= need_m), ms[-1])
    t = next((s for s in js if s >= max(m, need_t)), js[-1])
    return t, m


def size_table(v):
    """for the screen: per JB size the most 1P it takes and the multi it gets."""
    out = []
    cs, ts = float(v["jb_spare"]), float(v["term_spare"])
    for t in sizes(v):
        n = 0
        while n < 999:
            tt, mm = size_for(n + 1, v)
            if tt > t or math.ceil((n + 1) * (1 + cs) - 1e-9) > mm or math.ceil((n + 1) * (1 + ts) - 1e-9) > tt:
                break
            n += 1
        tt, m = size_for(n, v) if n else (t, t)
        out.append({"size": t, "max_in": n, "multi": m, "spare_cable": m - n, "spare_term": t - n,
                    "terms": t * (3 if v["term_pp"].startswith("3") else 2)})
    return out


def spec(j, v):
    ex = v["jb_ex_is"] if j["is"] == "IS" else v["jb_ex"]
    ex = ex + (" " + v["jb_gas"] if ex != "비방폭" and v.get("jb_gas") else "")
    n = j["used"]
    tpp = 3 if v["term_pp"].startswith("3") else 2
    return {"material": v["jb_mat"], "ex": ex, "ip": v["jb_ip"], "terminals": _n(j["size"]) * tpp,
            "gl_branch": int(math.ceil(n * (1 + float(v["gland_spare"])) - 1e-9)), "gl_multi": 1,
            "entry": v["jb_entry"]}


def _n(size):
    return int(re.match(r"\d+", size).group())


# ------------------------------------------------------------------ slots
SIDE = {"N": "북쪽", "S": "남쪽", "W": "서쪽", "E": "동쪽"}


def slots(layout, v, width_m=6.0):
    """Places a JB can be mounted, in plot pt: [{id, kind, x, y, label}]. Ids are stable for a plot file."""
    m = layout.m_per_pt
    if not m:
        return []
    W, H = layout.plot["size"]
    rects = layout.rack_rects(width_m)
    obst = layout.plot.get("obstacles") or []
    pad = 0.2 / m
    eq3d = (getattr(layout, "eq_model", None) or []) if str(v.get("slot_3d_eq", "")).startswith("사용") else []
    out = []

    def blocked(x, y, rack_ok=False):
        if not (0 <= x <= W and 0 <= y <= H):
            return True
        if any(o[0] + pad < x < o[2] - pad and o[1] + pad < y < o[3] - pad for o in obst):
            return True
        if any(o[0] - pad < x < o[2] + pad and o[1] - pad < y < o[3] + pad for o in eq3d):   # 3D model equipment (J1A)
            return True
        t = 0.3 / m                                      # rounding of the slot coordinates must not put it on the rack
        return not rack_ok and any(r[0] - t < x < r[2] + t and r[1] - t < y < r[3] + t for r in rects)

    if v["slot_rack"] == "사용":
        off = float(v["rack_off"]) / m
        from .plot import RACK_PAD_M                     # v13.4: PLOT racks reach RACK_PAD_M past the column line
        poff = max(off, (RACK_PAD_M + 0.5) / m)
        cols = layout.plot.get("rack_cols") or []
        for i, r in enumerate(layout.racks_auto):
            if r[-1] or i >= len(cols):
                continue
            x0, y0, x1, y1, ax = r[:5]
            for k, (x, y) in enumerate(sorted(cols[i], key=lambda p: (p[0], p[1])), 1):
                if ax == "H":
                    p = (x, y - poff) if abs(y - y0) <= abs(y - y1) else (x, y + poff)
                else:
                    p = (x - poff, y) if abs(x - x0) <= abs(x - x1) else (x + poff, y)
                if not blocked(*p, rack_ok=False):
                    out.append({"id": f"R{i + 1}-{k}", "kind": "랙", "x": round(p[0], 1), "y": round(p[1], 1),
                                "label": f"랙 {i + 1} 기둥 {k}"})
        bay = 6.0 / m
        if getattr(layout, "rack_src", "") in ("3D 모델 랙", "둘 다"):
            ids = getattr(layout, "racks_model_ids", None) or []
            moff = max(off, 0.5 / m)                    # never on the rack edge itself
            for ri, r in enumerate(getattr(layout, "racks_model", None) or []):   # 3D model racks: a column every bay, both sides
                x0, y0, x1, y1 = r[:4]
                rid = (ids[ri] if ri < len(ids) else "") or f"M{ri + 1}"
                horiz = (x1 - x0) >= (y1 - y0)
                L = (x1 - x0) if horiz else (y1 - y0)
                k = 0
                for t in range(int(L // bay) + 1):
                    for sgn, edge in ((-1, y0 if horiz else x0), (1, y1 if horiz else x1)):
                        p = (x0 + t * bay, edge + sgn * moff) if horiz else (edge + sgn * moff, y0 + t * bay)
                        k += 1
                        if not blocked(*p):
                            out.append({"id": f"3D{rid}-{k}", "kind": "랙", "x": round(p[0], 1), "y": round(p[1], 1),
                                        "label": f"3D 랙 {rid} 기둥 {(k + 1) // 2}"})
        hw = width_m / 2 / m + off
        for i, poly in enumerate(layout.racks_user):
            k = 0
            for (ax0, ay0), (ax1, ay1) in zip(poly, poly[1:]):
                horiz = abs(ax1 - ax0) >= abs(ay1 - ay0)
                L = abs(ax1 - ax0) if horiz else abs(ay1 - ay0)
                for t in range(int(L // bay) + 1):
                    for sgn in (-1, 1):
                        if horiz:
                            p = (min(ax0, ax1) + t * bay, ay0 + sgn * hw)
                        else:
                            p = (ax0 + sgn * hw, min(ay0, ay1) + t * bay)
                        k += 1
                        if not blocked(*p):
                            out.append({"id": f"U{i + 1}-{k}", "kind": "랙", "x": round(p[0], 1), "y": round(p[1], 1),
                                        "label": f"그린 랙 {i + 1} 자리 {k}"})
    if v["slot_eq"] == "사용":
        off = float(v["eq_off"]) / m
        eq = layout.plot.get("equipment") or {}
        for j, o in enumerate(obst):
            x0, y0, x1, y1 = o[:4]
            if not (2 <= (x1 - x0) * m <= 60 and 2 <= (y1 - y0) * m <= 60):
                continue
            name = next((t for t, pts in eq.items() if any(x0 <= p[0] <= x1 and y0 <= p[1] <= y1 for p in pts)), "")
            cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
            for s, p in (("N", (cx, y0 - off)), ("S", (cx, y1 + off)), ("W", (x0 - off, cy)), ("E", (x1 + off, cy))):
                if not blocked(*p):
                    out.append({"id": f"E{j + 1}-{s}", "kind": "장비", "x": round(p[0], 1), "y": round(p[1], 1),
                                "label": f"{name or '장비 ' + str(j + 1)} {SIDE[s]}", "eq": name})
    # two slots closer than 1 m are the same place
    keep = []
    for s in out:
        if not any(abs(s["x"] - k["x"]) * m < 1 and abs(s["y"] - k["y"]) * m < 1 for k in keep):
            keep.append(s)
    return keep


# ------------------------------------------------------------------ placement
def place(jbs, G, v, moves=None):
    """jbs: [{"key": group key, "pkg": bool, "pts": [(x, y) | None per member], ...}] (members in order).
    Fills per JB: pos, slot, slot_label, how. Members may be moved between JBs of the same group
    (returns the new member index lists in jb["take"]). moves: {jb_no: slot_id} applied last by the caller."""
    L = G.L
    if v["jb_place"] != "자리(슬롯)에 배치":
        for j in jbs:
            j["pos"] = _centre(j["pts"])
            j["slot"], j["slot_label"], j["how"] = "", "", f"계기 {len(j['pts'])}개 중심"
        return []
    width = float(v["rack_width"])
    sl = slots(L, v, width)
    R = L.router(float(v["rack_pref"]), width, v["main_route"].startswith("랙"))
    rad = float(v["jb_radius"])
    rf, fmin = float(v["route_factor"]), float(v["field_min"])
    reach, bonus, smax = float(v["rack_reach"]), float(v["st_bonus"]), max(1, int(float(v["st_max"])))
    cap = capacity(v)

    def trunk(p):
        d = R.dist[R.free(p)]
        return d if d != float("inf") else 1e6

    tcache = {}

    def tcost(s):
        if s["id"] not in tcache:
            tcache[s["id"]] = trunk((s["x"], s["y"]))
        return tcache[s["id"]]

    def pick(j, used):
        pts = [p for p in j["pts"] if p]
        if not pts or not sl:
            return None
        c = _centre(pts)
        free = [s for s in sl if used.get(s["id"], 0) < smax]
        near = lambda s: L.dist(c, (s["x"], s["y"]))
        if j["pkg"]:                                   # Skid JB: on the skid edge if there is one near
            pool = [s for s in free if s["kind"] == "장비" and near(s) <= rad] or free
        else:                                          # rack first; equipment edge only where no rack is near
            pool = [s for s in free if s["kind"] == "랙" and near(s) <= reach] or free
        cand = [s for s in pool if near(s) <= rad] or sorted(pool, key=near)[:8]
        best, bc = None, float("inf")
        for s in cand:
            br = sum(max(L.dist(p, (s["x"], s["y"])) * rf, fmin) for p in pts)
            cost = br + tcost(s) - (bonus if used.get(s["id"]) else 0)
            if cost < bc:
                best, bc = s, cost
        return best

    def run():
        used = {}
        for j in sorted(jbs, key=lambda j: -len(j["pts"])):
            s = pick(j, used)
            j["_s"] = s
            if s:
                used[s["id"]] = used.get(s["id"], 0) + 1

    run()
    small = int(float(v.get("small_jb", 0) or 0))
    if small > 0:                                     # fold tiny JBs into a same-group JB nearby
        for j in sorted([j for j in jbs if j["_s"] and len(j["rows"]) <= small], key=lambda j: len(j["rows"])):
            if not j["rows"]:
                continue
            mates = [o for o in jbs if o is not j and o["key"] == j["key"] and o["_s"] and o["rows"]
                     and len(o["rows"]) + len(j["rows"]) <= cap
                     and L.dist((j["_s"]["x"], j["_s"]["y"]), (o["_s"]["x"], o["_s"]["y"])) <= 2 * rad]
            if mates:
                o = min(mates, key=lambda o: L.dist((j["_s"]["x"], j["_s"]["y"]), (o["_s"]["x"], o["_s"]["y"])))
                o["rows"] += j["rows"]
                o["pts"] += j["pts"]
                j["rows"], j["pts"] = [], []
        jbs[:] = [j for j in jbs if j["rows"]]
        run()
    for _ in range(max(0, int(float(v["reassign"])))):
        changed = False
        groups = {}
        for j in jbs:
            if j["_s"]:
                groups.setdefault(j["key"], []).append(j)
        for key, js in groups.items():
            if len(js) < 2:
                continue
            pool = [(r, p) for j in js for r, p in zip(j["rows"], j["pts"]) if p]
            fixed = [(j, [(r, p) for r, p in zip(j["rows"], j["pts"]) if not p]) for j in js]
            new = {id(j): list(f) for j, f in fixed}
            # instruments with the biggest gap between their nearest and second-nearest JB choose first
            def gap(x):
                d = sorted(L.dist(x[1], (j["_s"]["x"], j["_s"]["y"])) for j in js)
                return -(d[1] - d[0]) if len(d) > 1 else 0
            for r, p in sorted(pool, key=gap):
                for j in sorted(js, key=lambda j: L.dist(p, (j["_s"]["x"], j["_s"]["y"]))):
                    if len(new[id(j)]) < cap:
                        new[id(j)].append((r, p))
                        break
            for j in js:
                rows = [x[0] for x in new[id(j)]]
                if [id(r) for r in rows] != [id(r) for r in j["rows"]]:
                    changed = True
                j["rows"], j["pts"] = rows, [x[1] for x in new[id(j)]]
        jbs[:] = [j for j in jbs if j["rows"]]
        if not changed:
            break
        run()
    for j in jbs:
        s = j.pop("_s", None)
        if s:
            j["pos"], j["slot"], j["slot_label"] = (s["x"], s["y"]), s["id"], s["label"]
            j["how"] = f"자리 {s['label']}"
        else:
            j["pos"] = _centre(j["pts"])
            j["slot"], j["slot_label"] = "", ""
            j["how"] = f"계기 {len([p for p in j['pts'] if p])}개 중심 (맞는 자리 없음)" if j["pos"] else ""
    return sl


def _centre(pts):
    pts = [p for p in pts if p]
    if not pts:
        return None
    return (sum(p[0] for p in pts) / len(pts), sum(p[1] for p in pts) / len(pts))


def stations(jbs_geo):
    """JBs at the same slot (or the same point) -> one station mark."""
    st = {}
    for j in jbs_geo:
        k = j.get("slot") or f"{j['x']:.0f},{j['y']:.0f}"
        x = st.setdefault(k, {"id": k, "x": j["x"], "y": j["y"], "label": j.get("slot_label") or "", "jbs": []})
        x["jbs"].append(j["no"])
    return list(st.values())


def checklist(v):
    """rows for the JB checklist sheet / screen: code, group, item, applied, default, note."""
    out = []
    for g in SCHEMA:
        for it in g["items"]:
            val = v.get(it["id"], it["default"])
            out.append({"code": it["code"], "group": g["title"], "name": it["name"], "value": val,
                        "default": it["default"], "changed": str(val) != str(it["default"]), "help": it.get("help", "")})
    return out
