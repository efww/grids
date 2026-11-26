"""3D overlay data (JSON) for the piping team's three.js model - first part.

Everything the program placed on the plot plan, with x / y / z, in one file:
racks, equipment, control room / IRP, instruments, JBs, trunk cable routes, tray / trench segments,
supports and perforated trays. Coordinates are given in plot points (pt, as read from the PDF)
and in metres; the header says where the origin is, which way the axes run and what z means,
so a person or another AI can line the file up with a model that uses its own origin
(see protocols/3d_protocol.md).

Axes (plot frame):  x_m = x_pt * m_per_pt   (to the right on the plot sheet)
                    y_m = y_pt * m_per_pt   (DOWN the sheet - PDF page coordinates)
                    z_m = elevation above finished grade (EL, m); UG is negative
three.js (Y up):    X = x_m, Y = z_m, Z = y_m   (right-handed, the sheet seen from above)
"""
from __future__ import annotations

import json
import time
from pathlib import Path

FORMAT = "pid_io-3d"
VERSION = "0.2"


def _r(x, n=2):
    return round(float(x), n)


def build(project, res, layout, tray, sup, s, plot=None):
    v = s["values"]
    g = (res or {}).get("geo") or {}
    m = layout.m_per_pt if layout is not None else None
    if not g or not m:
        raise ValueError("PLOT PLAN 좌표가 없습니다. STEP 6에서 PLOT PLAN을 올리고 축척·제어실을 확인해 주세요")
    mode = v["main_route"]
    use_rack = mode.startswith("랙")
    racks_pt = layout.rack_rects(float(v["rack_width"])) if use_rack else []
    rack_el = float(v["rack_el"])
    if mode.startswith("UG") or (use_rack and v["trunk_off"].startswith("UG")):
        off_el = -float(v["ug_depth"])
    else:
        off_el = float(v["sup_el"])
    branch_el = -float(v["ug_depth"]) if v["no_rack"].startswith("UG") else float(v["sup_el"])
    on_rack = lambda x, y: any(r[0] <= x <= r[2] and r[1] <= y <= r[3] for r in racks_pt)

    def P(x, y, z):
        return {"pt": [_r(x, 1), _r(y, 1)], "m": [_r(x * m), _r(y * m), _r(z)]}

    def path3d(poly, start_el, end_el):
        """plot polyline -> 3D points: rise at the start, level per leg (rack top / AG / UG), drop at the end.
        A leg is on the rack when its middle is inside a rack rectangle."""
        pts = []
        lv = []
        for (x0, y0), (x1, y1) in zip(poly, poly[1:]):
            lv.append(rack_el if on_rack((x0 + x1) / 2, (y0 + y1) / 2) else off_el)
        if not lv:
            return pts
        pts.append(P(*poly[0], start_el))
        cur = lv[0]
        pts.append(P(*poly[0], cur))
        for i, (x, y) in enumerate(poly[1:], 1):
            pts.append(P(x, y, cur))
            if i < len(lv) and lv[i] != cur:
                cur = lv[i]
                pts.append(P(x, y, cur))
        pts.append(P(*poly[-1], end_el))
        return pts

    out = {
        "format": FORMAT, "version": VERSION, "project": project,
        "made": time.strftime("%Y-%m-%d %H:%M"),
        "frame": {
            "source": "PLOT PLAN PDF page 1",
            "plot_file": (plot or {}).get("pdf") or "",
            "page_size_pt": [_r(x, 1) for x in ((plot or {}).get("size") or [0, 0])],
            "m_per_pt": m,
            "origin": "top-left corner of the plot sheet (PDF page origin)",
            "x": "right on the sheet (m = pt x m_per_pt)",
            "y": "DOWN the sheet (PDF page coordinates). Flip the sign if the model's north is +y",
            "z": "elevation above grade, m (UG negative). EL values below are settings, not survey",
            "threejs": "X = m[0], Y = m[2] (up), Z = m[1]",
            "north": "not read from the plot - set it when aligning (protocols/3d_protocol.md)",
        },
        "levels": {"rack_top_el": rack_el, "ag_support_el": float(v["sup_el"]), "ug_depth": float(v["ug_depth"]),
                   "instrument_el": float(v["inst_el"]), "jb_el": float(v["jb_el"]), "room_entry_el": float(v["room_el"]),
                   "trunk_off_rack": v["trunk_off"], "branch_level": v["no_rack"]},
        "anchors": [],                    # candidate reference points for alignment: equipment centres, room, IRP
        "racks": [], "equipment": [], "rooms": [], "instruments": [], "jbs": [],
        "cables": [], "trays": [], "supports": [], "joints": [], "perforated": [],
    }
    for i, r in enumerate(racks_pt):
        out["racks"].append({"id": f"RACK-{i + 1:02d}", "rect_pt": [_r(x, 1) for x in r[:4]],
                             "rect_m": [_r(x * m) for x in r[:4]], "top_el": rack_el,
                             "axis": "x" if (r[2] - r[0]) >= (r[3] - r[1]) else "y"})
    for tag, pts in sorted(((plot or {}).get("equipment") or {}).items()):
        for k, (x, y) in enumerate(pts):
            e = {"tag": tag + (f"#{k + 1}" if len(pts) > 1 else ""), **P(x, y, 0), "src": "PLOT 태그 글자 위치"}
            out["equipment"].append(e)
    for k, lab in (("room", "제어실 (CCR)"), ("irp", "IRP")):
        p = getattr(layout, k, None)
        if p:
            out["rooms"].append({"id": k.upper(), "name": lab, **P(p[0], p[1], float(v["room_el"])),
                                 "src": getattr(layout, f"{k}_src", "")})
    out["anchors"] = [{"id": e["tag"], "kind": "equipment", "m": e["m"]} for e in out["equipment"]][:60] + \
                     [{"id": r["id"], "kind": "room", "m": r["m"]} for r in out["rooms"]]
    jb_of = {}
    for c in res["cables"]:
        if c["kind"] == "분기":
            jb_of[c["frm"]] = c["to"]
    for tag, (x, y, src) in sorted((g.get("inst") or {}).items()):
        out["instruments"].append({"tag": tag, **P(x, y, float(v["inst_el"])), "src": src,
                                   "estimated": str(src).startswith("추정"), "jb": jb_of.get(tag, "")})
    jbinfo = {j["no"]: j for j in res["jbs"]}
    for row in g.get("jbs") or []:
        no, x, y = row[0], row[1], row[2]
        j = jbinfo.get(no, {})
        out["jbs"].append({"no": no, **P(x, y, float(v["jb_el"])), "size": j.get("size") or row[9],
                           "kind": j.get("kind", ""), "system": row[3], "signal": row[7], "used": row[8],
                           "slot": row[5], "moved": bool(row[6]), "spec": {k: j.get(k) for k in ("material", "ex", "ip")}})
    routes = {r["rid"]: r for r in g.get("routes") or []}
    from .tray import od as _od, rid_of
    inst = g.get("inst") or {}
    jbpos = {j["no"]: (j["pt"][0], j["pt"][1]) for j in out["jbs"]}
    for c in res["cables"]:
        e = {"no": c["no"], "kind": c["kind"], "from": c["frm"], "to": c["to"], "spec": c["spec"],
             "od_mm": _r(_od(c["spec"], v), 1), "length_m": c["length"]}
        if c["kind"] == "분기":
            a, b = inst.get(c["frm"]), jbpos.get(c["to"])
            if a and b:                    # instrument -> branch level -> JB, x first then y (as measured)
                e["path"] = [P(a[0], a[1], float(v["inst_el"])), P(a[0], a[1], branch_el), P(b[0], a[1], branch_el),
                             P(b[0], b[1], branch_el), P(b[0], b[1], float(v["jb_el"]))]
        else:
            rid = rid_of(c)
            if rid in routes:
                st = float(v["room_el"]) if c["kind"] == "MCC" else float(v["jb_el"])
                e["path"] = path3d(routes[rid]["poly"], st, float(v["room_el"]))
                e["route"] = rid
        trs = ((tray or {}).get("by_cable") or {}).get(c["no"])
        if trs:
            e["trays"] = [x["no"] for x in trs["legs"]]
        out["cables"].append(e)
    for sg in (tray or {}).get("segments") or []:
        a, b = sg["poly"]
        el = rack_el if sg["kind"] == "랙" else off_el
        base = {"seg": sg["no"], "run": sg.get("run"), "kind": {"랙": "ladder_on_rack", "AG": "ladder_on_supports", "UG": "ug_trench"}[sg["kind"]],
                "start": P(a[0], a[1], el), "end": P(b[0], b[1], el), "length_m": sg["length"], "cables": sg["cables"]}
        if sg["kind"] == "UG":
            out["trays"].append({**base, "no": sg["no"], "width_mm": sg.get("width"), "tiers": sg.get("tiers", 1),
                                 "fill": sg.get("fill")})
        for k, t in enumerate(sg.get("trays") or []):
            out["trays"].append({**base, "no": t["no"], "class": t["cls"], "width_mm": t["width"], "tiers": t["tiers"],
                                 "height_mm": float(v["tray_height"]), "fill": t["fill"],
                                 "side_offset_mm": k * (t["width"] + 100)})   # trays of one segment sit side by side
    for x in (sup or {}).get("supports") or []:
        out["supports"].append({"no": x["no"], "type": x["type"], "name": x["name"], "typical": x["typical"],
                                **P(x["x"], x["y"], x["el"] if x["type"] not in ("L", "T", "H") else 0),
                                "top_el": x["el"], "width_mm": x.get("width"), "tiers": x.get("tiers"), "segs": x["segs"],
                                "role": x.get("role", ""), "fix": x.get("fix", ""), "blocked": bool(x.get("blocked"))})
    for j in (sup or {}).get("joints") or []:
        out["joints"].append({"no": j["no"], "seg": j["seg"], **P(j["x"], j["y"], j["el"]), "trays": j["tray_nos"]})
    pt_sup = {p["pt"]: p["count"] for p in (sup or {}).get("pt") or []}
    for p in (tray or {}).get("pt") or []:
        cab = next((c for c in out["cables"] if c["no"] == p["cable"]), None)
        out["perforated"].append({"no": p["no"], "tag": p["to"], "jb": p["frm"], "width_mm": p["width"],
                                  "length_m": p["length"], "brackets": pt_sup.get(p["no"]),
                                  "path": (cab or {}).get("path")})
    out["counts"] = {k: len(out[k]) for k in ("racks", "equipment", "rooms", "instruments", "jbs", "cables", "trays",
                                              "supports", "joints", "perforated")}
    out["counts"]["instruments_estimated"] = sum(1 for x in out["instruments"] if x["estimated"])
    return out


def write(data, out: Path):
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(data, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    return out
