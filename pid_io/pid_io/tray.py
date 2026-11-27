"""STEP 8 - cable trays from the STEP 6/7 routes.

Every trunk cable (JB multi, package multi, IRP/MCC multi, comm) already has an orthogonal
route on the plot. The routes are cut into 1 m grid edges; each edge knows which cables pass.
Edges in a straight line that carry exactly the same cables form one tray segment, so a tray
is 900 wide near the control room and narrows (600, 300) where cables branch off.

  on a pipe rack           -> ladder tray, width from the cables' cross-section and the fill ratio
  off the rack (trunk)     -> UG trench (STEP 6 'trunk_off' = UG), or a ladder tray on supports (AG)
  instrument -> JB branch  -> perforated tray, length = branch cable length, numbered only

Sizing: sum of cable areas (pi/4 x OD^2) <= width x height x fill. Smallest width that holds it;
beyond the widest, tiers of the widest are stacked (user decision 2026-11-26). The user can set any segment's width / tiers; the fill is shown at once.
"""
from __future__ import annotations

import math
import re
from collections import Counter

from .schema import _num, _sel, _txt


def _code(d, code):
    d["code"] = code
    return d


SCHEMA = [
    {"id": "tray", "title": "사다리 트레이",
     "lead": "랙 최상단 위 간선 케이블(JB→룸 멀티, 패키지·IRP 멀티, 통신)이 지나가는 구간마다 케이블 단면적 합으로 폭을 정합니다. "
             "단면적 합 ≤ 폭 × 높이 × Fill ratio인 가장 작은 폭, 가장 넓은 폭(900)으로도 넘치면 단을 쌓습니다 (900 × n단).",
     "items": [
         _code(_txt("tray_widths", "트레이 폭 (mm)", "300,600,900", "쉼표로 구분. 높이는 아래 값"), "T01"),
         _code(_num("tray_height", "트레이 높이 (레일)", 150, "mm"), "T02"),
         _code(_num("tray_fill", "Fill ratio", 0.4, "", "케이블 단면적 합 / (폭 × 높이)", 0.05, True), "T03"),
         _code(_sel("sep_comm", "통신 케이블 별도 트레이", ["별도", "같이"], "별도", "광·RS-485를 계장 신호와 다른 트레이로", on="별도"), "T04"),
         _code(_sel("sep_is", "IS 케이블 별도 트레이", ["별도", "같이"], "별도", "IS 멀티를 Non-IS와 다른 트레이로", on="별도"), "T05"),
         _code(_num("tray_fit", "피팅·할증", 0.1, "", "엘보·티·리듀서·여유. 물량 = 길이 × (1 + 할증)", 0.05, True), "T06"),
     ]},
    {"id": "od", "title": "케이블 외경",
     "lead": "회사 데이터시트가 없을 때의 일반값입니다 (mm). 기본값은 공개 제조사 카탈로그(BS 5308 / EN 50288-7 외장형 SWA, "
             "1.5SQ: Battcables, Huadong, CSDS, Caledonian, B3 등)의 외경을 1.25SQ에 보수적으로 쓴 것입니다. "
             "LS전선·대한전선 표는 이 환경에서 받지 못했습니다 → 회사 데이터시트가 오면 바꾸세요. 사양 글자에서 페어·코어 수를 읽어 이 표로 외경을 찾습니다.",
     "items": [
         _code(_txt("od_single", "분기 케이블 (1P·1T·2C·3C)", "1P:12.5,1T:13.5,2P:14,2C:13,3C:14",
                    "구성:외경. 근거: 외장 1P OS 12.0 / IS-OS 12.9, 2P OS 13.2~14.2 (1.5SQ). 2C·3C는 추정. 비외장 일반값(예전 기본) 1P:9,1T:10,2P:12,2C:10,3C:11"), "T10"),
         _code(_txt("od_multi", "멀티페어 (P, Triad는 ×1.15)",
                    "2:14,4:18,6:21,10:26,12:28,16:31,20:34,24:37,30:40,36:43,40:47,48:51,60:57,80:64",
                    "페어 수:외경 (OS 외장). 근거: 4P 17.7, 10P 26~28.4, 20P 34.7~35.7, 30P 36.8 (1.5SQ 카탈로그). "
                    "40P 이상은 √(페어 수)로 늘린 추정. 표에 없는 수는 가까운 큰 값. 비외장 예전 기본 6:15,12:20,24:26,36:30,40:32,48:34,60:38,80:44"), "T11"),
         _code(_num("od_isos", "IS-OS(개별 차폐) 추가", 3, "mm", "개별 차폐 멀티는 외경에 이만큼 더함. 근거: 10P 28.4 / 20P 35.7 (IS-OS) vs OS 26 / 34"), "T12"),
         _code(_txt("od_core", "멀티코어 (C, MCC 모터당)", "4:16,7:18,10:21,12:22,19:25,24:28,30:30",
                    "코어 수:외경 (외장 추정 = 비외장 + 4 mm). 비외장 예전 기본 4:12,7:14,10:16,12:17,19:20,24:23,30:25"), "T13"),
         _code(_num("od_fo", "광케이블", 15, "mm", "외장(SWA) 12C 14.5~16.2 mm, 철 테이프 외장 7.5~8.3 mm"), "T14"),
         _code(_num("od_rs485", "RS-485 통신", 10, "mm", "1P 외장 추정 (차폐 1P 외장 12 mm 안팎보다 조금 작게)"), "T15"),
     ]},
    {"id": "pt", "title": "UG · Perforated",
     "lead": "랙을 타지 않는 간선 구간은 UG(트렌치) 물량으로, JB에서 각 계기까지는 Perforated 트레이(케이블 길이와 같게)로 냅니다.",
     "items": [
         _code(_sel("pt_width", "Perforated 트레이 폭", ["100", "150", "200"], "100", "계기 분기 1~2가닥용"), "T20"),
         _code(_num("ug_fit", "UG 트렌치 할증", 0.05, "", "", 0.05, True), "T21"),
         _code(_txt("ug_widths", "트렌치 폭 (mm)", "300,450,600,900,1200",
                    "케이블 단면적 합 ≤ 폭 × 유효 높이 × Fill인 가장 작은 폭. 넘치면 단(브래킷 단) 추가"), "T22"),
         _code(_num("ug_height", "트렌치 케이블 유효 높이 (단당)", 200, "mm"), "T23"),
         _code(_num("ug_fill", "트렌치 Fill ratio", 0.4, "", "", 0.05, True), "T24"),
         _code(_sel("ug_runno", "트렌치 번호", ["트렌치 한 줄 (분기·폭 변경 때만 새 번호)", "구간마다"],
                    "트렌치 한 줄 (분기·폭 변경 때만 새 번호)",
                    "한 줄: 물리적으로 이어지고 폭이 같은 트렌치 = 번호 하나 (꺾여도 같은 번호). 구간: 케이블이 바뀔 때마다 번호"), "T25"),
     ]},
]
ITEMS = {it["id"]: it for g in SCHEMA for it in g["items"]}


def _table(text, cast=int):
    out = {}
    for k, v in re.findall(r"([0-9A-Za-z]+)\s*:\s*([0-9.]+)", str(text or "")):
        out[k.upper()] = float(v)
    return out


def widths(v):
    w = sorted({int(x) for x in re.findall(r"\d+", str(v.get("tray_widths") or "")) if int(x) > 0})
    return w or [300, 600, 900]


def od(spec: str, v) -> float:
    """outer diameter (mm) from a cable spec text like '40P x 1.25SQ IS-OS', 'FO 12C SM Armoured'."""
    s = spec.upper()
    if s.startswith("FO"):
        return float(v["od_fo"])
    if s.startswith("RS-485"):
        return float(v["od_rs485"])
    m = re.match(r"(\d+)\s*([PTC])", s)
    if not m:
        return 10.0
    n, u = int(m.group(1)), m.group(2)
    single = _table(v["od_single"])
    add = float(v["od_isos"]) if "IS-OS" in s else 0.0
    if f"{n}{u}" in single and n <= 3:
        return single[f"{n}{u}"]
    tab = _table(v["od_core"] if u == "C" else v["od_multi"])
    keys = sorted(int(k) for k in tab)
    if not keys:
        return 20.0
    k = next((k for k in keys if k >= n), keys[-1])
    d = tab[str(k)]
    if k < n:                                 # beyond the table: grow with the square root of the count
        d *= math.sqrt(n / k)
    return d * (1.15 if u == "T" else 1.0) + (add if u != "C" else 0.0)


def area(d_mm):
    return math.pi / 4 * d_mm * d_mm


def eff_fill(v):
    return max(0.01, float(v["tray_fill"]))


def size_for(a, v):
    """cable area (mm2) -> (width, tiers) by the fill rule."""
    ws, h, f = widths(v), max(1.0, float(v["tray_height"])), eff_fill(v)   # 0 would divide by 0
    for w in ws:
        if a <= w * h * f + 1e-9:
            return w, 1
    w = ws[-1]
    return w, max(1, math.ceil(a / (w * h * f) - 1e-9))


def ug_widths(v):
    w = sorted({int(x) for x in re.findall(r"\d+", str(v.get("ug_widths") or "")) if int(x) > 0})
    return w or [300, 450, 600, 900, 1200]


def ug_size(a, v):
    """trench: cable area (mm2) -> (width, tiers of brackets)."""
    ws, h, f = ug_widths(v), max(1.0, float(v["ug_height"])), max(0.01, float(v["ug_fill"]))
    for w in ws:
        if a <= w * h * f + 1e-9:
            return w, 1
    return ws[-1], max(1, math.ceil(a / (ws[-1] * h * f) - 1e-9))


def fill(a, w, n, v):
    return a / (max(1, w) * max(1.0, float(v["tray_height"])) * max(1, n))


def rid_of(c):
    k = c["kind"]
    if k == "멀티":
        return c["frm"]
    if k == "패키지":
        return f"PKG {c['area']}"
    if k == "MCC":
        return c["frm"]
    if k == "통신":
        return f"COMM {c['frm']}"
    return None


def tray_class(c, v):
    if v["sep_comm"] == "별도" and c["kind"] == "통신":
        return "통신"
    if v["sep_is"] == "별도" and c.get("is") == "IS":
        return "IS"
    return "신호"


CLS_SUFFIX = {"신호": "", "통신": "-C", "IS": "-I"}


# ------------------------------------------------------------------ build
def build(res, layout, s, dec=None):
    v = s["values"]
    g = res.get("geo")
    dec = dec or {}
    over = dec.get("tray_size") or {}
    out = {"segments": [], "pt": [], "summary": {}, "lost_overrides": 0}
    # perforated trays: one per branch cable (JB -> instrument), numbered per unit
    pt_n = Counter()
    for c in sorted((c for c in res["cables"] if c["kind"] == "분기"), key=lambda c: (c["unit"] or "", c["to"], c["frm"])):
        u = c.get("unit") or "00"
        pt_n[u] += 1
        out["pt"].append({"no": f"PT-{u}-{pt_n[u]:03d}", "cable": c["no"], "frm": c["to"], "to": c["frm"],
                          "width": int(v["pt_width"]), "length": c["length"]})
    if not g or layout is None or not layout.m_per_pt:
        out["summary"] = _summary(out, v)
        return out
    m = layout.m_per_pt
    routes = {r["rid"]: r for r in g.get("routes") or []}
    mode = v["main_route"]
    racks = layout.rack_rects(float(v["rack_width"])) if mode.startswith("랙") else []
    on_rack = lambda x, y: any(r[0] <= x <= r[2] and r[1] <= y <= r[3] for r in racks)
    off = "UG" if (mode.startswith("UG") or (mode.startswith("랙") and v["trunk_off"].startswith("UG"))) else "AG"
    if mode.startswith("AG"):
        off = "AG"
    trunk = [c for c in res["cables"] if c["kind"] in ("멀티", "패키지", "MCC", "통신") and rid_of(c) in routes]
    by_rid = {}
    for i, c in enumerate(trunk):
        by_rid.setdefault(rid_of(c), []).append(i)
    # inside a rack every cable rides the one tray on its top tier: snap to the rack centre line
    rk_m = [(r[0] * m, r[1] * m, r[2] * m, r[3] * m) for r in racks]
    snap = lambda nodes: _snap(nodes, rk_m)
    edges = {}                                      # (node, node) -> [set(cable idx), kind]
    for rid, idx in by_rid.items():
        nodes = snap(_grid_nodes(routes[rid]["poly"], m))
        for a, b in zip(nodes, nodes[1:]):
            k = (a, b) if a <= b else (b, a)
            e = edges.get(k)
            if e is None:
                mx, my = (a[0] + b[0]) / 2 / m, (a[1] + b[1]) / 2 / m
                e = edges[k] = [set(), "랙" if on_rack(mx, my) else off]
            e[0].update(idx)
    # chain straight edges carrying the same cables into segments
    nb = {}
    for k in edges:
        for n in k:
            nb.setdefault(n, []).append(k)
    seen = set()
    segs = []
    for k in edges:
        if k in seen:
            continue
        seen.add(k)
        key = (frozenset(edges[k][0]), edges[k][1])
        chain = [k[0], k[1]]
        for end in (1, 0):                          # grow from both ends
            while True:
                n = chain[-1] if end == 1 else chain[0]
                prev = chain[-2] if end == 1 else chain[1]
                es = nb.get(n, [])
                if len(es) != 2:
                    break
                nxt = es[0] if es[1] == ((prev, n) if prev <= n else (n, prev)) else es[1]
                if nxt in seen or (frozenset(edges[nxt][0]), edges[nxt][1]) != key:
                    break
                far = nxt[0] if nxt[1] == n else nxt[1]
                if (far[0] - n[0], far[1] - n[1]) != (n[0] - prev[0], n[1] - prev[1]):
                    break                           # a corner ends the segment
                seen.add(nxt)
                if end == 1:
                    chain.append(far)
                else:
                    chain.insert(0, far)
        segs.append((chain, edges[k]))
    seg_of = {}                                      # edge -> index into segs
    for i, (chain, _) in enumerate(segs):
        for a, b in zip(chain, chain[1:]):
            seg_of[(a, b) if a <= b else (b, a)] = i
    room = layout.room
    rn = (room[0] * m, room[1] * m) if room else (0, 0)
    order = sorted(range(len(segs)), key=lambda i: abs((segs[i][0][0][0] + segs[i][0][-1][0]) / 2 - rn[0])
                   + abs((segs[i][0][0][1] + segs[i][0][-1][1]) / 2 - rn[1]))
    num = Counter()
    used_over = set()
    seg_out = {}
    for i in order:
        chain, (cset, kind) = segs[i]
        a, b = chain[0], chain[-1]
        if b < a:
            a, b = b, a
        sid = f"{a[0]}_{a[1]}_{b[0]}_{b[1]}"
        cs = [trunk[i] for i in sorted(cset)]
        units = Counter((c.get("unit") or "00") for c in cs)
        u = units.most_common(1)[0][0] if units else "00"
        length = len(chain) - 1
        seg = {"id": sid, "kind": kind, "length": length, "unit": u, "_ends": (a, b),
               "poly": [[round(a[0] / m, 1), round(a[1] / m, 1)], [round(b[0] / m, 1), round(b[1] / m, 1)]],
               "cables": [c["no"] for c in cs], "trays": []}
        if kind == "UG":
            seg["area"] = round(sum(area(od(c["spec"], v)) for c in cs))
            seg["width"], seg["tiers"] = ug_size(seg["area"], v)
            seg["fill"] = round(seg["area"] / (seg["width"] * max(1.0, float(v["ug_height"])) * seg["tiers"]), 3)
        else:
            num[("TR", u)] += 1
            seg["no"] = f"TR-{u}-{num[('TR', u)]:03d}"
            cls = {}
            for c in cs:
                cls.setdefault(tray_class(c, v), []).append(c)
            for name in ("신호", "IS", "통신"):
                if name not in cls:
                    continue
                a_ = sum(area(od(c["spec"], v)) for c in cls[name])
                w, n = size_for(a_, v)
                auto = (w, n)
                o = (over.get(sid) or {}).get(name)
                if o:
                    used_over.add(sid)
                    w, n = int(o[0]), int(o[1])
                seg["trays"].append({"no": seg["no"] + CLS_SUFFIX[name], "cls": name, "count": len(cls[name]),
                                     "area": round(a_), "width": w, "tiers": n, "auto": list(auto), "user": bool(o),
                                     "fill": round(fill(a_, w, n, v), 3)})
        out["segments"].append(seg)
        seg_out[i] = seg
    _number_ug(out["segments"], v)
    # each trunk cable: the trays / UG it runs in, in order from its start (JB, panel) to the room
    out["by_cable"] = {}
    for rid, idx in by_rid.items():
        nodes = snap(_grid_nodes(routes[rid]["poly"], m))
        walk = []
        for a, b in zip(nodes, nodes[1:]):
            i = seg_of.get((a, b) if a <= b else (b, a))
            if i is None:
                continue
            if walk and walk[-1][0] == i:
                walk[-1][1] += 1
            else:
                walk.append([i, 1])
        for ci in idx:
            c = trunk[ci]
            cls = tray_class(c, v)
            legs = []
            for i, run in walk:
                sg = seg_out[i]
                if sg["kind"] == "UG":
                    if legs and legs[-1]["kind"] == "UG" and legs[-1]["no"] == sg["run"]:
                        legs[-1]["m"] += run            # one trench run, one leg
                    else:
                        legs.append({"no": sg["run"], "kind": "UG", "m": run,
                                     "size": f"트렌치 {sg['width']}" + (f" {sg['tiers']}단" if sg["tiers"] > 1 else "")})
                else:
                    tr = next((x for x in sg["trays"] if x["cls"] == cls), None)
                    if tr:
                        legs.append({"no": tr["no"], "kind": "사다리", "size": f"{tr['width']}×{int(v['tray_height'])}"
                                     + (f" {tr['tiers']}단 적층" if tr["tiers"] > 1 else ""), "m": run})
            out["by_cable"][c["no"]] = {"cable": c["no"], "kind": c["kind"], "frm": c["frm"], "to": c["to"],
                                        "spec": c["spec"], "od": round(od(c["spec"], v), 1), "cls": cls,
                                        "ladder_m": sum(x["m"] for x in legs if x["kind"] == "사다리"),
                                        "ug_m": sum(x["m"] for x in legs if x["kind"] == "UG"), "legs": legs}
    out["lost_overrides"] = len(set(over) - used_over)
    out["summary"] = _summary(out, v)
    return out


def _number_ug(segs, v):
    """UG numbers. 'run': segments joined end to end where nothing else meets (no branch) and the trench
    keeps its size forms one trench run = one number (corners stay in the run); parts get /1, /2 ...
    Numbered per unit, nearest the control room first (segments come in that order)."""
    deg = Counter()
    for s in segs:
        for n in s["_ends"]:
            deg[n] += 1
    ug = [s for s in segs if s["kind"] == "UG"]
    parent = list(range(len(ug)))

    def find(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i
    if v.get("ug_runno", "").startswith("트렌치"):
        at = {}
        for i, s in enumerate(ug):
            for n in s["_ends"]:
                at.setdefault(n, []).append(i)
        for n, ids in at.items():
            if len(ids) == 2 and deg[n] == 2:
                a, b = (ug[i] for i in ids)
                if (a["width"], a["tiers"]) == (b["width"], b["tiers"]):
                    parent[find(ids[0])] = find(ids[1])
    groups = {}
    for i in range(len(ug)):
        groups.setdefault(find(i), []).append(i)
    num = Counter()
    for r in sorted(groups.values(), key=lambda g: min(g)):
        u = Counter(ug[i]["unit"] for i in r).most_common(1)[0][0]
        num[u] += 1
        run = f"UG-{u}-{num[u]:03d}"
        for k, i in enumerate(sorted(r), 1):
            ug[i]["run"] = run
            ug[i]["no"] = run if len(r) == 1 else f"{run}/{k}"
            ug[i]["parts"] = len(r)
    for s in segs:
        s.pop("_ends", None)


def _snap(nodes, racks):
    """grid nodes inside a rack move onto the rack's centre line (across the rack), then the path is
    made continuous again with unit steps (x first)."""
    if not racks or not nodes:
        return nodes
    out = []
    for x, y in nodes:
        for x0, y0, x1, y1 in racks:
            if x0 <= x <= x1 and y0 <= y <= y1:
                if (x1 - x0) >= (y1 - y0):
                    y = round((y0 + y1) / 2)
                else:
                    x = round((x0 + x1) / 2)
                break
        if not out or out[-1] != (x, y):
            out.append((x, y))
    full = [out[0]]
    for q in out[1:]:
        x, y = full[-1]
        while (x, y) != q:
            if x != q[0]:
                x += 1 if q[0] > x else -1
            else:
                y += 1 if q[1] > y else -1
            full.append((x, y))
    # drop immediate back-and-forth (A, B, A) left by the snap
    clean = []
    for p in full:
        if len(clean) >= 2 and clean[-2] == p:
            clean.pop()
            continue
        clean.append(p)
    return clean


def _grid_nodes(poly, m):
    """orthogonal polyline in pt -> consecutive 1 m grid nodes (ints, metres)."""
    nodes = []
    for (x0, y0), (x1, y1) in zip(poly, poly[1:]):
        a = (round(x0 * m), round(y0 * m))
        b = (round(x1 * m), round(y1 * m))
        if a[0] != b[0] and a[1] != b[1]:          # not orthogonal: go along x first, then y
            pts = [a, (b[0], a[1]), b]
        else:
            pts = [a, b]
        for p, q in zip(pts, pts[1:]):
            dx = (q[0] > p[0]) - (q[0] < p[0])
            dy = (q[1] > p[1]) - (q[1] < p[1])
            x, y = p
            if not nodes or nodes[-1] != (x, y):
                nodes.append((x, y))
            while (x, y) != q:
                x, y = x + dx, y + dy
                nodes.append((x, y))
    return nodes


def _summary(t, v):
    fit = float(v["tray_fit"])
    lad = {}
    over = 0
    for s in t["segments"]:
        for tr in s["trays"]:
            k = (tr["cls"], tr["width"])
            lad[k] = lad.get(k, 0) + s["length"] * tr["tiers"]
            if tr["fill"] > eff_fill(v) + 1e-9:
                over += 1
    ug = sum(s["length"] for s in t["segments"] if s["kind"] == "UG")
    ugw = {}
    for s in t["segments"]:
        if s["kind"] == "UG":
            k = (s["width"], s["tiers"])
            ugw[k] = ugw.get(k, 0) + s["length"]
    runs = {}
    for s in t["segments"]:
        if s["kind"] == "UG":
            r = runs.setdefault(s["run"], {"run": s["run"], "width": s["width"], "tiers": s["tiers"], "length": 0,
                                           "parts": 0, "cables": set(), "unit": s["unit"]})
            r["length"] += s["length"]
            r["parts"] += 1
            r["cables"].update(s["cables"])
    ug_cab = sum(s["length"] * len(s["cables"]) for s in t["segments"] if s["kind"] == "UG")
    return {"ladder": [{"cls": k[0], "width": k[1], "length": l_, "order": math.ceil(l_ * (1 + fit))}
                       for k, l_ in sorted(lad.items(), key=lambda kv: (["신호", "IS", "통신"].index(kv[0][0]), kv[0][1]))],
            "ladder_m": sum(lad.values()), "ug_m": ug, "ug_order": math.ceil(ug * (1 + float(v["ug_fit"]))),
            "ug_cable_m": ug_cab, "pt_m": sum(p["length"] for p in t["pt"]), "pt_n": len(t["pt"]),
            "segments": sum(1 for s in t["segments"] if s["kind"] != "UG"),
            "ug_segments": sum(1 for s in t["segments"] if s["kind"] == "UG"), "overfill": over,
            "ug_by_width": [{"width": k[0], "tiers": k[1], "length": l_, "order": math.ceil(l_ * (1 + float(v["ug_fit"])))}
                            for k, l_ in sorted(ugw.items())],
            "ug_runs": [{**r, "cables": len(r["cables"])} for r in runs.values()]}


def checklist(v):
    out = []
    for g in SCHEMA:
        for it in g["items"]:
            val = v.get(it["id"], it["default"])
            out.append({"code": it["code"], "group": g["title"], "name": it["name"], "value": val,
                        "default": it["default"], "changed": str(val) != str(it["default"]), "help": it.get("help", "")})
    return out


def write_sheets(wb, t, v, sheet):
    sm = t["summary"]
    data = [["사다리 트레이", f"{x['cls']} {x['width']}×{int(v['tray_height'])}", x["length"], x["order"]] for x in sm["ladder"]]
    data += [["", "(900으로 넘치는 구간은 단을 쌓아 단 수만큼 길이에 더함)", "", ""]]
    data += [["UG 트렌치", f"폭 {x['width']}" + (f" × {x['tiers']}단" if x["tiers"] > 1 else ""), x["length"], x["order"]]
             for x in sm["ug_by_width"]]
    data += [["UG 트렌치 합계", f"트렌치 {len(sm['ug_runs'])}줄 · 구간 {sm['ug_segments']}", sm["ug_m"], sm["ug_order"]],
             ["Perforated 트레이", f"{v['pt_width']} (계기 분기 {sm['pt_n']}가닥)", sm["pt_m"], sm["pt_m"]]]
    sheet(wb.create_sheet("Tray_Summary"), ["구분", "규격", "길이 (m)", "물량 (할증 포함, m)"], data, [18, 30, 12, 20])
    rows = []
    for s in t["segments"]:
        if s["kind"] == "UG":
            rows.append([s["no"], f"UG 트렌치 ({s['run']})", "", s["width"], s["tiers"], s["length"], len(s["cables"]), s["area"],
                         s["fill"], "자동 (트렌치 Fill)", s["unit"],
                         f"{s['poly'][0]} → {s['poly'][1]}", ", ".join(s["cables"])])
        for tr in s["trays"]:
            rows.append([tr["no"], "사다리 (랙)" if s["kind"] == "랙" else "사다리 (AG 서포트)", tr["cls"], tr["width"],
                         tr["tiers"], s["length"], tr["count"], tr["area"], tr["fill"],
                         ("사용자 " if tr["user"] else "자동 ") + f"(자동 {tr['auto'][0]}×{tr['auto'][1]}단)", s["unit"],
                         f"{s['poly'][0]} → {s['poly'][1]}", ", ".join(c for c in s["cables"])])
    sheet(wb.create_sheet("Tray_Segments"), ["Tray No", "종류", "트레이 구분", "폭 (mm)", "단 수", "길이 (m)", "케이블 수",
                                             "단면적 합 (mm²)", "Fill", "크기 결정", "Unit", "구간 (PLOT pt)", "케이블"],
          rows, [14, 16, 10, 8, 6, 8, 8, 12, 7, 22, 6, 30, 80])
    for r in wb["Tray_Segments"].iter_rows(min_row=2, min_col=9, max_col=9):
        r[0].number_format = "0%"
    cr = []
    for c in t.get("by_cable", {}).values():
        route = " → ".join(f"{x['no']} ({x['size']}) {x['m']}m" for x in c["legs"])
        cr.append([c["cable"], c["kind"], c["frm"], c["to"], c["spec"], c["od"], c["cls"], c["ladder_m"], c["ug_m"],
                   len(c["legs"]), route])
    sheet(wb.create_sheet("Cable_Tray_Route"), ["Cable No", "구분", "From", "To", "Cable Spec", "외경 (mm)", "트레이 구분",
                                                "사다리 (m)", "UG (m)", "구간 수", "트레이 경로 (출발 → 룸)"],
          cr, [22, 7, 18, 8, 26, 9, 9, 9, 8, 7, 120])
    sheet(wb.create_sheet("Trench_Runs"), ["Trench No", "폭 (mm)", "브래킷 단", "길이 (m)", "구간 수", "케이블 수", "Unit"],
          [[r["run"], r["width"], r["tiers"], r["length"], r["parts"], r["cables"], r["unit"]] for r in sm["ug_runs"]],
          [14, 8, 8, 9, 8, 9, 6])
    sheet(wb.create_sheet("Perforated_Tray"), ["PT No", "케이블", "From (JB)", "To (계기)", "폭 (mm)", "길이 (m)"],
          [[p["no"], p["cable"], p["frm"], p["to"], p["width"], p["length"]] for p in t["pt"]], [14, 22, 18, 18, 8, 9])
    sheet(wb.create_sheet("Tray_Checklist"), ["No", "구분", "항목", "적용값", "기본값", "", "설명"],
          [[c["code"], c["group"], c["name"], c["value"], c["default"], "변경" if c["changed"] else "", c["help"]]
           for c in checklist(v)], [6, 14, 30, 34, 34, 6, 70])
