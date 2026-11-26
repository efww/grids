"""STEP 8 - cable trays from the STEP 6/7 routes.

Every trunk cable (JB multi, package multi, IRP/MCC multi, comm) already has an orthogonal
route on the plot. The routes are cut into 1 m grid edges; each edge knows which cables pass.
Edges in a straight line that carry exactly the same cables form one tray segment, so a tray
is 900 wide near the control room and narrows (600, 300) where cables branch off.

  on a pipe rack           -> ladder tray, width from the cables' cross-section and the fill ratio
  off the rack (trunk)     -> UG trench (STEP 6 'trunk_off' = UG), or a ladder tray on supports (AG)
  instrument -> JB branch  -> perforated tray, length = branch cable length, numbered only

Sizing: sum of cable areas (pi/4 x OD^2) <= width x height x fill. Smallest width that holds it;
beyond the widest, more tiers of the widest (whether they stack or run side by side is decided
later). The user can set any segment's width / tiers; the fill is shown at once.
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
     "lead": "랙 위 간선 케이블(JB→룸 멀티, 패키지·IRP 멀티, 통신)이 지나가는 구간마다 케이블 단면적 합으로 폭을 정합니다. "
             "단면적 합 ≤ 폭 × 높이 × Fill ratio인 가장 작은 폭, 가장 넓은 폭으로도 넘치면 단 수를 늘립니다.",
     "items": [
         _code(_txt("tray_widths", "트레이 폭 (mm)", "300,600,900", "쉼표로 구분. 높이는 아래 값"), "T01"),
         _code(_num("tray_height", "트레이 높이 (레일)", 150, "mm"), "T02"),
         _code(_num("tray_fill", "Fill ratio", 0.4, "", "케이블 단면적 합 / (폭 × 높이)", 0.05, True), "T03"),
         _code(_sel("sep_comm", "통신 케이블 별도 트레이", ["별도", "같이"], "별도", "광·RS-485를 계장 신호와 다른 트레이로", on="별도"), "T04"),
         _code(_sel("sep_is", "IS 케이블 별도 트레이", ["별도", "같이"], "별도", "IS 멀티를 Non-IS와 다른 트레이로", on="별도"), "T05"),
         _code(_num("tray_fit", "피팅·할증", 0.1, "", "엘보·티·리듀서·여유. 물량 = 길이 × (1 + 할증)", 0.05, True), "T06"),
     ]},
    {"id": "od", "title": "케이블 외경",
     "lead": "회사 자료가 없을 때의 일반값입니다 (mm). 사양 글자에서 페어·코어 수를 읽어 이 표로 외경을 찾습니다.",
     "items": [
         _code(_txt("od_single", "분기 케이블 (1P·1T·2C·3C)", "1P:9,1T:10,2P:12,2C:10,3C:11", "구성:외경"), "T10"),
         _code(_txt("od_multi", "멀티페어 (P, Triad는 ×1.15)", "6:15,12:20,24:26,36:30,40:32,48:34,60:38,80:44",
                    "페어 수:외경. 표에 없는 수는 위 값 중 가까운 큰 값"), "T11"),
         _code(_num("od_isos", "IS-OS(개별 차폐) 추가", 2, "mm", "개별 차폐 멀티는 외경에 이만큼 더함"), "T12"),
         _code(_txt("od_core", "멀티코어 (C, MCC 모터당)", "4:12,7:14,10:16,12:17,19:20,24:23,30:25", "코어 수:외경"), "T13"),
         _code(_num("od_fo", "광케이블", 14, "mm"), "T14"),
         _code(_num("od_rs485", "RS-485 통신", 8, "mm"), "T15"),
     ]},
    {"id": "pt", "title": "UG · Perforated",
     "lead": "랙을 타지 않는 간선 구간은 UG(트렌치) 물량으로, JB에서 각 계기까지는 Perforated 트레이(케이블 길이와 같게)로 냅니다.",
     "items": [
         _code(_sel("pt_width", "Perforated 트레이 폭", ["100", "150", "200"], "100", "계기 분기 1~2가닥용"), "T20"),
         _code(_num("ug_fit", "UG 트렌치 할증", 0.05, "", "", 0.05, True), "T21"),
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
    edges = {}                                      # (node, node) -> [set(cable idx), kind]
    for rid, idx in by_rid.items():
        nodes = _grid_nodes(routes[rid]["poly"], m)
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
    room = layout.room
    rn = (room[0] * m, room[1] * m) if room else (0, 0)
    segs.sort(key=lambda sg: abs((sg[0][0][0] + sg[0][-1][0]) / 2 - rn[0]) + abs((sg[0][0][1] + sg[0][-1][1]) / 2 - rn[1]))
    num = Counter()
    used_over = set()
    for chain, (cset, kind) in segs:
        a, b = chain[0], chain[-1]
        if b < a:
            a, b = b, a
        sid = f"{a[0]}_{a[1]}_{b[0]}_{b[1]}"
        cs = [trunk[i] for i in sorted(cset)]
        units = Counter((c.get("unit") or "00") for c in cs)
        u = units.most_common(1)[0][0] if units else "00"
        length = len(chain) - 1
        seg = {"id": sid, "kind": kind, "length": length, "unit": u,
               "poly": [[round(a[0] / m, 1), round(a[1] / m, 1)], [round(b[0] / m, 1), round(b[1] / m, 1)]],
               "cables": [c["no"] for c in cs], "trays": []}
        if kind == "UG":
            num[("UG", u)] += 1
            seg["no"] = f"UG-{u}-{num[('UG', u)]:03d}"
            seg["area"] = round(sum(area(od(c["spec"], v)) for c in cs))
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
    out["lost_overrides"] = len(set(over) - used_over)
    out["summary"] = _summary(out, v)
    return out


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
    ug_cab = sum(s["length"] * len(s["cables"]) for s in t["segments"] if s["kind"] == "UG")
    return {"ladder": [{"cls": k[0], "width": k[1], "length": l_, "order": math.ceil(l_ * (1 + fit))}
                       for k, l_ in sorted(lad.items(), key=lambda kv: (["신호", "IS", "통신"].index(kv[0][0]), kv[0][1]))],
            "ladder_m": sum(lad.values()), "ug_m": ug, "ug_order": math.ceil(ug * (1 + float(v["ug_fit"]))),
            "ug_cable_m": ug_cab, "pt_m": sum(p["length"] for p in t["pt"]), "pt_n": len(t["pt"]),
            "segments": sum(1 for s in t["segments"] if s["kind"] != "UG"),
            "ug_segments": sum(1 for s in t["segments"] if s["kind"] == "UG"), "overfill": over}


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
    data += [["UG 트렌치", "랙 밖 간선", sm["ug_m"], sm["ug_order"]],
             ["Perforated 트레이", f"{v['pt_width']} (계기 분기 {sm['pt_n']}가닥)", sm["pt_m"], sm["pt_m"]]]
    sheet(wb.create_sheet("Tray_Summary"), ["구분", "규격", "길이 (m)", "물량 (할증 포함, m)"], data, [18, 30, 12, 20])
    rows = []
    for s in t["segments"]:
        if s["kind"] == "UG":
            rows.append([s["no"], "UG 트렌치", "", "", "", s["length"], len(s["cables"]), s["area"], "", "", s["unit"],
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
    sheet(wb.create_sheet("Perforated_Tray"), ["PT No", "케이블", "From (JB)", "To (계기)", "폭 (mm)", "길이 (m)"],
          [[p["no"], p["cable"], p["frm"], p["to"], p["width"], p["length"]] for p in t["pt"]], [14, 22, 18, 18, 8, 9])
    sheet(wb.create_sheet("Tray_Checklist"), ["No", "구분", "항목", "적용값", "기본값", "", "설명"],
          [[c["code"], c["group"], c["name"], c["value"], c["default"], "변경" if c["changed"] else "", c["help"]]
           for c in checklist(v)], [6, 14, 30, 34, 34, 6, 70])
