"""STEP 10 - 3D 인계: put the instrumentation layer on the piping team's Three.js model, and write the
company AI platform's standard handoff set (Task 1 개념설계 / T1-CI 계장).

Base model = the piping team's Three.js HTML (Seed, e.g. 02-AI-Assisted.Conceptual.3D.Model_Final.html,
Three.js 0.164.1 + OrbitControls via importmap). It is never changed; a supplemented copy is written.

1. read the base model: EQUIPMENT boxes, racks (RK: direction, offset, half width, cable tier EL), and the
   load-time east shift (DX_E) the model applies to equipment / Rack C
2. fit our plot points (pt) to the model frame (X, Z m; Y up): X = s*x + tx, Z = s*y + tz (north up on both,
   no rotation) by least squares on shared equipment tags and rack centre lines (racks weigh more)
3. our 3D data (threed.build) in the model frame: trays on a rack snap to that rack's centre line at its cable
   tier EL (their RK.cable), the rest keeps our EL settings
4. supplemented model = base HTML + one block appended to its module script: layers 계장 · 트레이 / 케이블 /
   JB / 계기 / 서포트 with every object clickable (popup shows the properties: tray no, JB no, tags ...)
5. standard 7 files (00_manifest ... 06_model_handoff) + STRUCTURED_MODEL_DATA (full)

The standard file contents follow the company guide pictures (AI_3D_Platform_Copilot_Appendix_Rev6) as far as
they were readable; fields are drafts until the guide HTML itself is in the repo.
"""
from __future__ import annotations

import csv
import hashlib
import io
import json
import math
import re
import time
from pathlib import Path

SKILL = "ai3d-t1-ci-r9"            # registered skill id seen in the guide (T1-CI, R9-G6)
PROFILE = "T1-CI"

from .schema import _num, _sel  # noqa: E402

SCHEMA = [
    {"id": "tw", "title": "3D 차선 · 충돌 회피",
     "lead": "랙 케이블 단을 공종별 차선으로 나눠 계장이 먼저 자리를 잡습니다(선점). 다른 공종(전기 등) 3D 파일이 오면 겹치는 곳은 "
             "아래 순서로 자동 회피합니다: 같은 단 안에서 옆으로 → 위로 한 단씩 → 그래도 겹치면 '충돌 남음' 표시. 모든 이동은 기록됩니다.",
     "items": [
         {**_sel("tw_lane", "계장 차선 (랙 케이블 단)", ["+쪽 절반 (남북 랙 = 동, 동서 랙 = 남)", "−쪽 절반 (남북 랙 = 서, 동서 랙 = 북)", "가운데"],
                 "+쪽 절반 (남북 랙 = 동, 동서 랙 = 남)", "다른 공종(전기 전력)은 반대쪽 절반을 쓰도록 협의"), "code": "W01"},
         {**_num("tw_margin", "랙 가장자리 여유", 0.3, "m", "", 0.1), "code": "W02"},
         {**_num("tw_gap", "다른 공종과 이격", 0.3, "m", "계장-전력 트레이 일반값 300 mm", 0.1), "code": "W03"},
         {**_sel("tw_yield", "충돌 시 누가 비키나", ["계장이 비킴 (항상)", "비키지 않음 (충돌만 표시)"], "계장이 비킴 (항상)",
                 "사용자 방침: 계장은 항상 비키는 공종"), "code": "W04"},
         {**_num("tw_step", "위로 비킬 때 한 단 높이", 0.3, "m", "", 0.1), "code": "W05"},
         {**_num("tw_max_up", "위로 비킬 수 있는 단 수", 2, "단"), "code": "W06"},
     ]},
]
ITEMS = {it["id"]: it for g in SCHEMA for it in g["items"]}


# ------------------------------------------------------------------ base model
def _obj(src, name):
    i = src.find(f"const {name} = ")
    if i < 0:
        return None
    try:
        return json.JSONDecoder().raw_decode(src[i + len(name) + 9:])[0]
    except ValueError:
        return None


def parse_base(path: Path) -> dict:
    raw = Path(path).read_bytes()
    s = raw.decode("utf-8", errors="replace")
    mod = s[s.find('<script type="module">'):] if '<script type="module">' in s else s
    meta = _obj(mod, "PFD_META") or {}
    eq = _obj(mod, "EQUIPMENT") or []
    refs = _obj(mod, "REFS") or []
    three = (re.search(r"three@([\d.]+)", s) or [None, ""])[1]
    racks_meta = dict(meta.get("racks") or {})
    # load-time east shift (Elec update): equipment / refs with centre x > 42 and Rack C move +DX_E
    m = re.search(r"const DX_E\s*=\s*([\d.]+)", mod)
    dx = float(m.group(1)) if m else 0.0
    if dx and "C_off" in racks_meta:
        racks_meta["C_off"] = racks_meta["C_off"] + dx
        if "EW" in racks_meta:
            racks_meta["EW"] = {**racks_meta["EW"], "a1": racks_meta["EW"]["a1"] + dx}
    equipment = {}
    for e in eq:
        b = dict(e["box"])
        cx = (b["x0"] + b["x1"]) / 2
        if dx and cx > 42:
            b["x0"] += dx
            b["x1"] += dx
        equipment[e["tag"]] = {"x": (b["x0"] + b["x1"]) / 2, "z": (b["z0"] + b["z1"]) / 2, "box": b, "h": e.get("h"),
                               "name": e.get("name", ""), "rack": e.get("rack", "")}
    # racks: RK literal (dir, off, half, lv, cable); offsets / ends from PFD_META.racks
    racks = {}
    rk = re.search(r"const RK = \{(.*?)\n\};", mod, re.S)
    if rk:
        for rid, body in re.findall(r"(\w+):\s*\{\s*id:\s*'\w+',(.*?)\}", rk.group(1), re.S):
            g = lambda k, d=None: (re.search(k + r":\s*([^,]+)", body) or [None, d])[1]
            direction = (g("dir", "'NS'") or "'NS'").strip().strip("'")
            off_txt = (g("off", "0") or "0").strip()
            off = racks_meta.get("C_off", 0) - 1.5 if "C_off" in off_txt else float(re.sub(r"[^\d.\-]", "", off_txt) or 0)
            lv = [float(x) for x in re.findall(r"[\d.]+", (re.search(r"lv:\s*\[([^\]]*)\]", body) or [None, ""])[1])]
            rm = racks_meta.get(rid) or {}
            racks[rid] = {"id": rid, "dir": direction, "off": off, "half": float(g("half", "4.5") or 4.5),
                          "a0": rm.get("a0"), "a1": rm.get("a1"), "levels": lv,
                          "cable_el": float(g("cable", "11") or 11),
                          "name": ((re.search(r"name:\s*'([^']*)'", body) or [None, rid])[1])}
    return {"file": Path(path).name, "sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw), "three": three,
            "title": (re.search(r"<title>(.*?)</title>", s) or [None, ""])[1], "meta": meta, "dx_e": dx,
            "equipment": equipment, "racks": racks, "refs": len(refs)}


# ------------------------------------------------------------------ alignment
def fit(plot: dict, our_racks: list, base: dict, w_rack: float = 3.0) -> dict:
    """least squares for [s, tx, tz]: X = s*x + tx, Z = s*y + tz (pt -> model m, both north up).
    anchors: shared equipment tags (2 equations each), our racks matched to model racks (1 equation: the
    centre line across the rack). Two passes: equipment only, then racks matched by nearest centre line."""
    import numpy as np
    eqs, rhs, wts, tags = [], [], [], []
    for tag, pts in (plot.get("equipment") or {}).items():
        e = base["equipment"].get(tag)
        if not e or len(pts) != 1:
            continue
        x, y = pts[0]
        eqs += [[x, 1, 0], [y, 0, 1]]
        rhs += [e["x"], e["z"]]
        wts += [1.0, 1.0]
        tags.append((tag, x, y, e["x"], e["z"]))
    if len(tags) < 2:
        raise ValueError("3D 모델과 PLOT PLAN에 같은 장비 태그가 2개 이상 있어야 좌표를 맞출 수 있습니다")

    def solve(E, R, W):
        A = np.array(E) * np.array(W)[:, None]
        return np.linalg.lstsq(A, np.array(R) * np.array(W), rcond=None)[0]
    s, tx, tz = solve(eqs, rhs, wts)
    matched = []
    for _ in range(2):
        E2, R2, W2 = list(eqs), list(rhs), list(wts)
        matched = []
        for r in our_racks:
            x0, y0, x1, y1 = r["rect_pt"]
            if r["axis"] == "y":                 # N-S rack: centre x
                xc = (x0 + x1) / 2
                X = s * xc + tx
                cand = [k for k in base["racks"].values() if k["dir"] == "NS"]
                best = min(cand, key=lambda k: abs(k["off"] - X), default=None)
                if best and abs(best["off"] - X) < 15:
                    E2.append([xc, 1, 0]); R2.append(best["off"]); W2.append(w_rack)
                    matched.append((r["id"], best["id"], round(X - best["off"], 2)))
            else:                                # E-W rack: centre z
                yc = (y0 + y1) / 2
                Z = s * yc + tz
                cand = [k for k in base["racks"].values() if k["dir"] == "EW"]
                best = min(cand, key=lambda k: abs(k["off"] - Z), default=None)
                if best and abs(best["off"] - Z) < 15:
                    E2.append([yc, 0, 1]); R2.append(best["off"]); W2.append(w_rack)
                    matched.append((r["id"], best["id"], round(Z - best["off"], 2)))
        s, tx, tz = solve(E2, R2, W2)
    res = []
    for tag, x, y, X, Z in tags:
        dx, dz = s * x + tx - X, s * y + tz - Z
        res.append({"tag": tag, "dx": round(dx, 2), "dz": round(dz, 2), "d": round(math.hypot(dx, dz), 2)})
    res.sort(key=lambda r: -r["d"])
    racks_res = []
    for our_id, rid, _ in matched:
        r = next(x for x in our_racks if x["id"] == our_id)
        k = base["racks"][rid]
        x0, y0, x1, y1 = r["rect_pt"]
        c = s * ((x0 + x1) / 2) + tx if k["dir"] == "NS" else s * ((y0 + y1) / 2) + tz
        racks_res.append({"ours": our_id, "model": rid, "across_m": round(c - k["off"], 2)})
    d = [r["d"] for r in res]
    return {"s": float(s), "tx": float(tx), "tz": float(tz), "rotation_deg": 0.0,
            "anchors": len(tags), "rack_anchors": len(matched),
            "median_m": round(sorted(d)[len(d) // 2], 2), "max_m": round(max(d), 2), "equipment": res, "racks": racks_res,
            "formula": f"X = {s:.5f}·x_pt + {tx:.2f} · Z = {s:.5f}·y_pt + {tz:.2f} · Y = EL (cable tier on racks = model RK.cable)"}


# ------------------------------------------------------------------ our data in the model frame
def to_model(d3: dict, tf: dict, base: dict, v: dict) -> dict:
    s, tx, tz = tf["s"], tf["tx"], tf["tz"]
    racks = base["racks"]
    rack_el = float(v["rack_el"])

    def xz(pt):
        return s * pt[0] + tx, s * pt[1] + tz

    def on_rack(X, Z, pad=1.0):
        for k in racks.values():
            if k["dir"] == "NS" and abs(X - k["off"]) <= k["half"] + pad and (k["a0"] is None or k["a0"] - 3 <= Z <= k["a1"] + 3):
                return k
            if k["dir"] == "EW" and abs(Z - k["off"]) <= k["half"] + pad and (k["a0"] is None or k["a0"] - 3 <= X <= k["a1"] + 3):
                return k
        return None

    lane = v.get("tw_lane", "+")
    sign = 1 if lane.startswith("+") else -1 if lane.startswith("−") else 0
    margin = float(v.get("tw_margin", 0.3))

    def lane_c(k, width_m, side):
        """centre across the rack for a tray of width_m, `side` m further in from the lane edge."""
        if not sign:
            return k["off"] + side
        edge = k["half"] - margin - width_m / 2 - side
        return k["off"] + sign * max(0.0, edge)

    def P(p, el=None, snap=False, side=0.0, width_m=0.0):
        X, Z = xz(p["pt"])
        Y = p["m"][2] if el is None else el
        if snap or abs(Y - rack_el) < 1e-6:
            k = on_rack(X, Z)
            if k:
                c = lane_c(k, width_m or 0.3, side)
                if k["dir"] == "NS":
                    X = c
                else:
                    Z = c
                Y = k["cable_el"] + (Y - rack_el if not snap else 0)
        return [round(X, 2), round(Y, 2), round(Z, 2)]

    out = {"frame": "model (X east, Y up EL, Z south) m", "transform": tf, "instruments": [], "jbs": [], "cables": [],
           "trays": [], "supports": [], "joints": [], "rooms": [], "perforated": []}
    for x in d3["instruments"]:
        out["instruments"].append({"id": x["tag"], "p": P(x), "est": x["estimated"],
                                   "props": {"Tag No": x["tag"], "JB": x["jb"], "위치 근거": x["src"],
                                             "추정 위치": "예" if x["estimated"] else "아니오", **(x.get("index") or {})}})
    for j in d3["jbs"]:
        out["jbs"].append({"id": j["no"], "p": P(j), "props": {"JB No": j["no"], "크기": j["size"], "종류": j["kind"],
                                                                "시스템": j["system"], "신호": j["signal"], "사용": j["used"],
                                                                "자리": j["slot"], **{k: v_ for k, v_ in (j.get("spec") or {}).items() if v_},
                                                                **(j.get("extra") or {})}})
    for c in d3["cables"]:
        if not c.get("path"):
            continue
        pts = []
        for q in (P(p) for p in c["path"]):          # v13.1: no slanted legs - horizontal at the lower EL, then a vertical riser
            if pts:
                p0 = pts[-1]
                if abs(q[1] - p0[1]) > 0.05 and (abs(q[0] - p0[0]) > 0.05 or abs(q[2] - p0[2]) > 0.05):
                    pts.append([q[0], p0[1], q[2]] if p0[1] < q[1] else [p0[0], q[1], p0[2]])
            pts.append(q)
        out["cables"].append({"id": c["no"], "kind": c["kind"], "od": c["od_mm"], "path": pts,
                              "props": {"Cable No": c["no"], "구분": c["kind"], "From": c["from"], "To": c["to"], "Spec": c["spec"],
                                        "OD (mm)": c["od_mm"], "길이 (m)": c["length_m"], "트레이": " → ".join(c.get("trays") or [])}})
    # v13.1 lane packing: a rack tray belongs to the rack running the same way as the tray; every tray on that rack
    # gets its own slot across the rack (trays of different segments over the same stretch no longer share a lane).
    slots = {}                                           # rack id -> [(along0, along1, side0, side1)]
    gap_m = 0.1

    def rack_for(t):
        (ax_, az_), (bx_, bz_) = xz(t["start"]["pt"]), xz(t["end"]["pt"])
        d_ = "NS" if abs(bz_ - az_) >= abs(bx_ - ax_) else "EW"
        mx, mz = (ax_ + bx_) / 2, (az_ + bz_) / 2
        for k in racks.values():
            if k["dir"] != d_:
                continue
            across, along = (mx, mz) if d_ == "NS" else (mz, mx)
            if abs(across - k["off"]) <= k["half"] + 1.0 and (k["a0"] is None or k["a0"] - 3 <= along <= k["a1"] + 3):
                return k, (ax_, az_), (bx_, bz_)
        return None, (ax_, az_), (bx_, bz_)

    def place(k, A, B, wm):
        i0 = 1 if k["dir"] == "NS" else 0              # along index in (X, Z)
        lo_, hi_ = sorted((A[i0], B[i0]))
        side = 0.0
        taken = sorted((s0, s1) for (a0, a1, s0, s1) in slots.setdefault(k["id"], []) if min(hi_, a1) - max(lo_, a0) > 0.5)
        for s0, s1 in taken:
            if side + wm + gap_m <= s0 or side >= s1 + gap_m:
                continue
            side = max(side, s1 + gap_m)
        slots[k["id"]].append((lo_, hi_, side, side + wm))
        c = lane_c(k, wm, side)
        el = k["cable_el"]
        if k["dir"] == "NS":
            return [round(c, 2), el, round(A[1], 2)], [round(c, 2), el, round(B[1], 2)], side
        return [round(A[0], 2), el, round(c, 2)], [round(B[0], 2), el, round(c, 2)], side

    for t in sorted(d3["trays"], key=lambda t: -(t.get("width_mm") or 300)):
        snap = t["kind"] == "ladder_on_rack"
        side = (t.get("side_offset_mm") or 0) / 1000
        wm = (t.get("width_mm") or 300) / 1000
        k = None
        if snap:
            k, A, B = rack_for(t)
        if k:
            a, b, side = place(k, A, B, wm)
        else:
            a, b = P(t["start"], snap=snap, side=side, width_m=wm), P(t["end"], snap=snap, side=side, width_m=wm)
            if snap and side:                            # no model rack here: still keep side-by-side trays apart
                i_ = 0 if abs(b[2] - a[2]) >= abs(b[0] - a[0]) else 2
                a[i_] = round(a[i_] + side, 2)
                b[i_] = round(b[i_] + side, 2)
        ug = t["kind"] == "ug_trench"
        out["trays"].append({"id": t["no"], "kind": t["kind"], "a": a, "b": b, "w": (t.get("width_mm") or 300) / 1000,
                             "tiers": t.get("tiers") or 1,
                             # v13.3 real shape + cables laid in the cross-section (a / b = floor of tier 1, centre line)
                             "h": round((t.get("height_mm") or (200 if ug else 150)) / 1000, 3),
                             "pitch": round((float(v.get("ug_height", 200)) + 100 if ug else float(v.get("sp_tier_gap", 300))) / 1000, 3),
                             "lay": [[c["no"], c["tier"], round(c["x"] / 1000, 4), round(c["y"] / 1000, 4), c["od"]]
                                     for c in t.get("lay") or []],
                             "props": {"Tray No": t["no"], "구간": t["seg"], "트렌치 줄": t.get("run") or "", "종류": t["kind"],
                                       "구분": t.get("class", ""), "폭 (mm)": t.get("width_mm"), "단": t.get("tiers"),
                                       "Fill": f"{round((t.get('fill') or 0) * 100)}%", "길이 (m)": t["length_m"],
                                       "랙 / 차선 (m)": f"{k['id']} / {round(side, 2)}" if k else "",
                                       "케이블 수": len(t["cables"]), "케이블": ", ".join(t["cables"][:40]) + (" …" if len(t["cables"]) > 40 else "")}})
    order = {t["no"]: i for i, t in enumerate(d3["trays"])}
    out["trays"].sort(key=lambda x: order.get(x["id"], 0))
    for x in d3.get("perforated") or []:                 # v13.3 perforated tray under each branch cable (JB -> instrument)
        if not x.get("path"):
            continue
        pts = []
        for q in (P(p) for p in x["path"]):
            if pts:
                p0 = pts[-1]
                if abs(q[1] - p0[1]) > 0.05 and (abs(q[0] - p0[0]) > 0.05 or abs(q[2] - p0[2]) > 0.05):
                    pts.append([q[0], p0[1], q[2]] if p0[1] < q[1] else [p0[0], q[1], p0[2]])
            pts.append(q)
        out["perforated"].append({"id": x["no"], "w": (x.get("width_mm") or 100) / 1000, "path": pts,
                                  "props": {"PT No": x["no"], "계기": x.get("tag", ""), "JB": x.get("jb", ""),
                                            "폭 (mm)": x.get("width_mm"), "길이 (m)": x.get("length_m"),
                                            "브래킷": x.get("brackets") or ""}})
    for x in d3["supports"]:
        p = P(x, el=x["top_el"])
        out["supports"].append({"id": x["no"], "type": x["type"], "p": p, "base": 0 if x["type"] in ("L", "T", "H") else None,
                                "props": {"Support No": x["no"], "형식": x["name"], "타피컬": x["typical"], "역할": x.get("role", ""),
                                          "고정": x.get("fix", ""), "폭 합 (mm)": x.get("width_mm"), "단": x.get("tiers"),
                                          "EL (m)": p[1], "구간": ", ".join(x["segs"]), "장비 안": "예 (확인)" if x.get("blocked") else ""}})
    for j in d3.get("joints") or []:
        out["joints"].append({"id": j["no"], "p": P(j), "props": {"EJ No": j["no"], "구간": j["seg"], "트레이": ", ".join(j["trays"])}})
    for r in d3["rooms"]:
        out["rooms"].append({"id": r["id"], "p": P(r), "props": {"이름": r["name"], "근거": r.get("src", "")}})
    out["counts"] = {k: len(out[k]) for k in ("instruments", "jbs", "cables", "trays", "supports", "joints", "perforated")}
    out["counts"]["cables_in_trays"] = sum(len(t["lay"]) for t in out["trays"])
    return out


# ------------------------------------------------------------------ clash avoidance (instrumentation yields)
def load_other(path: Path) -> dict:
    """other discipline objects in the model frame (JSON):
    {"discipline": "전기", "objects": [{"id": "ET-001", "kind": "tray", "a": [x,y,z], "b": [x,y,z], "w": 0.6, "h": 0.15},
                                       {"id": "EB-01", "kind": "box", "min": [x,y,z], "max": [x,y,z]}]}"""
    d = json.loads(Path(path).read_text(encoding="utf-8"))
    boxes = []
    for o in d.get("objects") or []:
        if "min" in o and "max" in o:
            lo, hi = o["min"], o["max"]
        elif "a" in o and "b" in o:
            w, h = float(o.get("w", 0.3)), float(o.get("h", 0.15))
            lo = [min(o["a"][0], o["b"][0]), min(o["a"][1], o["b"][1]), min(o["a"][2], o["b"][2])]
            hi = [max(o["a"][0], o["b"][0]), max(o["a"][1], o["b"][1]) + h, max(o["a"][2], o["b"][2])]
            if abs(o["a"][0] - o["b"][0]) >= abs(o["a"][2] - o["b"][2]):
                lo[2] -= w / 2; hi[2] += w / 2
            else:
                lo[0] -= w / 2; hi[0] += w / 2
        else:
            continue
        boxes.append({"id": str(o.get("id", "")), "kind": o.get("kind", "box"), "lo": lo, "hi": hi})
    return {"discipline": d.get("discipline", "다른 공종"), "file": Path(path).name, "boxes": boxes}


def _tray_box(t, gap):
    a, b = t["a"], t["b"]
    h = t.get("h", 0.1) + t.get("pitch", 0.3) * (t.get("tiers", 1) - 1)
    lo = [min(a[0], b[0]), min(a[1], b[1]) - 0.05, min(a[2], b[2])]
    hi = [max(a[0], b[0]), max(a[1], b[1]) + h, max(a[2], b[2])]
    if abs(a[0] - b[0]) >= abs(a[2] - b[2]):
        lo[2] -= t["w"] / 2; hi[2] += t["w"] / 2
    else:
        lo[0] -= t["w"] / 2; hi[0] += t["w"] / 2
    return [x - gap for x in lo], [x + gap for x in hi]


def _hit(lo, hi, B):
    return all(lo[i] < B["hi"][i] and hi[i] > B["lo"][i] for i in range(3))


def avoid(inst: dict, other: dict, base: dict, v: dict) -> list:
    """move our trays off other-discipline objects: sideways inside the rack (lane), then up one step at a time.
    Returns the move log; trays get "moved" in their props."""
    gap = float(v.get("tw_gap", 0.3))
    yield_ = v.get("tw_yield", "").startswith("계장이")
    step, max_up = float(v.get("tw_step", 0.3)), int(float(v.get("tw_max_up", 2)))
    log = []

    def mine(t):                                     # our own trays of the same segment (signal / comm / IS side by side)
        out = []
        for u in inst["trays"]:
            if u is t or u["props"].get("구간") != t["props"].get("구간"):
                continue
            lo_, hi_ = _tray_box(u, 0.0)
            out.append({"id": u["id"], "lo": lo_, "hi": hi_})
        return out
    for t in inst["trays"]:
        if t["kind"] == "ug_trench":
            continue
        lo, hi = _tray_box(t, gap)
        hits = [B for B in other["boxes"] if _hit(lo, hi, B)]
        if not hits:
            continue
        if not yield_:
            log.append({"tray": t["id"], "with": [B["id"] for B in hits], "action": "충돌 (비키지 않음 설정)", "dx": 0, "dy": 0})
            t["props"]["충돌"] = ", ".join(B["id"] for B in hits)
            continue
        along_x = abs(t["a"][0] - t["b"][0]) >= abs(t["a"][2] - t["b"][2])
        ax = 2 if along_x else 0                       # across the tray run
        rack = None
        for k in base["racks"].values():
            if (k["dir"] == "NS") == (not along_x):
                c = t["a"][0] if k["dir"] == "NS" else t["a"][2]
                if abs(c - k["off"]) <= k["half"] + 0.5:
                    rack = k
        lim = (rack["off"] - rack["half"] + t["w"] / 2, rack["off"] + rack["half"] - t["w"] / 2) if rack else None
        done = None
        for up in range(0, max_up + 1):
            for d in [0.0] + [s_ * k_ * 0.1 for k_ in range(1, 61) for s_ in (1, -1)]:
                if up == 0 and d == 0:
                    continue
                c0 = t["a"][ax] + d
                if lim and not (lim[0] <= c0 <= lim[1]):
                    continue
                if not lim and abs(d) > 2.0:
                    continue
                a = list(t["a"]); b = list(t["b"]); a[ax] += d; b[ax] += d; a[1] += up * step; b[1] += up * step
                lo2, hi2 = _tray_box({**t, "a": a, "b": b}, gap)
                lo3, hi3 = _tray_box({**t, "a": a, "b": b}, 0.05)
                if not any(_hit(lo2, hi2, B) for B in other["boxes"]) and not any(_hit(lo3, hi3, B) for B in mine(t)):
                    done = (a, b, d, up * step)
                    break
            if done:
                break
        if done:
            t["a"], t["b"] = done[0], done[1]
            act = ("옆으로 " + f"{done[2]:+.1f} m" if done[2] else "") + (" · " if done[2] and done[3] else "") + \
                  (f"위로 {done[3]:+.1f} m" if done[3] else "")
            t["props"]["회피"] = f"{act} ({other['discipline']} {', '.join(B['id'] for B in hits)} 피함)"
            log.append({"tray": t["id"], "with": [B["id"] for B in hits], "action": act, "dx": round(done[2], 2), "dy": round(done[3], 2)})
        else:
            t["props"]["충돌"] = f"남음: {', '.join(B['id'] for B in hits)} (사람 판단)"
            log.append({"tray": t["id"], "with": [B["id"] for B in hits], "action": "충돌 남음 (사람 판단)", "dx": 0, "dy": 0})
    return log


# ------------------------------------------------------------------ supplemented model
INJECT = r"""
/* ===================================================================================
   pid_io 계장 레이어 (INST) - appended by pid_io STEP 10 (3D 인계). Base model unchanged above this block.
   Layers: 계장 · 트레이/트렌치 · 케이블 · JB · 계기 · 서포트/조인트. Click → properties.
   =================================================================================== */
{
  const INST = __INST__;
  const DEF = [['inst_tray', '계장 · 트레이 / 트렌치'], ['inst_cable', '계장 · 케이블'], ['inst_jb', '계장 · JB'],
               ['inst_instr', '계장 · 계기'], ['inst_support', '계장 · 서포트 / 조인트']];
  const hd = document.createElement('div'); hd.className = 'row'; hd.innerHTML = '<b style="color:#fbbf24;margin-top:6px">계장 (pid_io)</b>'; layerBox.appendChild(hd);
  for (const [k, txt] of DEF) {
    const g = new THREE.Group(); g.name = k; scene.add(g); G[k] = g;
    const row = document.createElement('div'); row.className = 'row';
    row.innerHTML = `<input type="checkbox" id="ly_${k}" checked><label for="ly_${k}">${txt}</label>`;
    layerBox.appendChild(row);
    row.querySelector('input').addEventListener('change', ev => { g.visible = ev.target.checked; if (typeof requestShadow === 'function') requestShadow(); });
  }
  const M = {
    lad: mat(0x38bdf8, { metal: 0.6, rough: 0.4 }), ladAg: mat(0x60a5fa, { metal: 0.6, rough: 0.4 }), ug: mat(0xa16207, { metal: 0, rough: 0.9, transparent: true, opacity: 0.55 }),
    multi: mat(0xc084fc, { metal: 0.1, rough: 0.6 }), branch: mat(0x94a3b8, { metal: 0.1, rough: 0.6 }), comm: mat(0xf472b6, { metal: 0.1, rough: 0.6 }),
    jb: mat(0xf59e0b, { metal: 0.3, rough: 0.5, emissive: 0x3b2500 }), inst: mat(0x22c55e, { metal: 0.1, rough: 0.5 }), est: mat(0xfbbf24, { metal: 0.1, rough: 0.5 }),
    post: mat(0x10b981, { metal: 0.5, rough: 0.5 }), rack: mat(0xf43f5e, { metal: 0.5, rough: 0.5 }), clamp: mat(0x94a3b8, { metal: 0.5, rough: 0.5 }),
    joint: mat(0xe11d48, { metal: 0.3, rough: 0.5, emissive: 0x400010 }), room: mat(0xef4444, { metal: 0, rough: 0.8, transparent: true, opacity: 0.45 }) };
  const V = p => new THREE.Vector3(p[0], p[1], p[2]);
  function pick(o, kind, d) { o.userData.pick = { kind: 'inst', ref: { cat: kind, ...d } }; clickables.push(o); return o; }
  function bar(a, b, w, h, m, g, kind, d) {
    const A = V(a), B = V(b), len = Math.max(A.distanceTo(B), 0.05);
    const o = new THREE.Mesh(new THREE.BoxGeometry(w, h, len), m);
    o.position.copy(A).add(B).multiplyScalar(0.5); o.lookAt(B); g.add(o); if (kind) pick(o, kind, d); return o;
  }
  // ---- v13.3 real shapes: ladder = 2 C-rails + rungs @300, trench = concrete U + cover, perforated = channel;
  //      cables laid in each tray cross-section (lay = [cable, tier, x from left wall m, y above tier floor m, OD mm])
  const Y0 = new THREE.Vector3(0, 1, 0);
  M.lad.side = THREE.DoubleSide; M.ladAg.side = THREE.DoubleSide;
  M.multi.side = THREE.DoubleSide; M.comm.side = THREE.DoubleSide; M.branch.side = THREE.DoubleSide;
  M.conc = mat(0x9ca3af, { metal: 0, rough: 0.95, side: THREE.DoubleSide });
  M.cover = mat(0x6b7280, { metal: 0.2, rough: 0.7, transparent: true, opacity: 0.35, depthWrite: false });
  M.perf = mat(0x7dd3fc, { metal: 0.6, rough: 0.4, side: THREE.DoubleSide });
  const G2 = new THREE.Group(); G2.name = 'inst_cpath'; G2.visible = false; scene.add(G2); G.inst_cpath = G2;
  { const row = document.createElement('div'); row.className = 'row';
    row.innerHTML = '<input type="checkbox" id="ly_inst_cpath"><label for="ly_inst_cpath">계장 · 간선 케이블 중심선 (경로)</label>';
    layerBox.appendChild(row); row.querySelector('input').addEventListener('change', ev => { G2.visible = ev.target.checked; }); }
  function basis(dir) {                                   // across, up, along for a run in direction dir
    const d = dir.clone().normalize(); let ac = new THREE.Vector3().crossVectors(d, Y0);
    if (ac.lengthSq() < 1e-6) ac = new THREE.Vector3(1, 0, 0); ac.normalize();
    const up = new THREE.Vector3().crossVectors(ac, d).normalize();
    return { d, ac, up, q: new THREE.Quaternion().setFromRotationMatrix(new THREE.Matrix4().makeBasis(ac, up, d)) };
  }
  function merged(parts, m) {                             // parts: [[w, h, len, centre, quat]] -> one mesh
    const pos = [], nor = [], mx = new THREE.Matrix4();
    for (const [w, h, l, c, q] of parts) {
      const g = new THREE.BoxGeometry(w, h, l).toNonIndexed(); mx.compose(c, q, new THREE.Vector3(1, 1, 1)); g.applyMatrix4(mx);
      pos.push(...g.attributes.position.array); nor.push(...g.attributes.normal.array); g.dispose();
    }
    const bg = new THREE.BufferGeometry();
    bg.setAttribute('position', new THREE.Float32BufferAttribute(pos, 3)); bg.setAttribute('normal', new THREE.Float32BufferAttribute(nor, 3));
    bg.computeBoundingSphere(); return new THREE.Mesh(bg, m);
  }
  const at = (A, B, k) => A.clone().add(B.clone().sub(A).multiplyScalar(k));
  const kindOf = {}; for (const c of INST.cables) kindOf[c.id] = c;
  const cylG = new THREE.CylinderGeometry(1, 1, 1, 12, 1, false);
  const TRAY_BY = {};
  for (const t of INST.trays) {
    TRAY_BY[t.id] = t;
    const A = V(t.a), B = V(t.b), len = Math.max(A.distanceTo(B), 0.05), F = basis(B.clone().sub(A)), mid = at(A, B, 0.5);
    const ug = t.kind === 'ug_trench', w = t.w, h = t.h || 0.15, pitch = t.pitch || 0.3, n = t.tiers || 1;
    const P0 = (x, y, k = 0.5) => at(A, B, k).add(F.ac.clone().multiplyScalar(x)).add(F.up.clone().multiplyScalar(y));
    const parts = [];
    if (ug) {
      const depth = Math.max(0.3, -A.y + 0.05) ;                       // walls up to grade
      parts.push([w + 0.3, 0.1, len, P0(0, -0.05), F.q]);              // floor slab
      for (const sx of [-1, 1]) parts.push([0.1, depth, len, P0(sx * (w / 2 + 0.05), depth / 2), F.q]);
      for (let i = 1; i < n; i++) parts.push([w, 0.01, len, P0(0, i * pitch), F.q]);   // bracket shelf
      const o = merged(parts, M.conc); G.inst_tray.add(o); pick(o, '트레이', t);
      const cv = new THREE.Mesh(new THREE.BoxGeometry(w + 0.3, 0.05, len), M.cover); cv.position.copy(P0(0, depth + 0.025)); cv.quaternion.copy(F.q);
      G.inst_tray.add(cv);
    } else {
      for (let i = 0; i < n; i++) {
        const y = i * pitch;
        for (const sx of [-1, 1]) {                                   // C-rail: web + top / bottom flange (lips inward)
          parts.push([0.004, h, len, P0(sx * w / 2, y + h / 2), F.q]);
          parts.push([0.03, 0.004, len, P0(sx * (w / 2 - 0.015), y + h), F.q]);
          parts.push([0.03, 0.004, len, P0(sx * (w / 2 - 0.015), y + 0.002), F.q]);
        }
        const nr = Math.max(1, Math.floor(len / 0.3));               // rungs @300 mm
        for (let k = 0; k < nr; k++) parts.push([w, 0.02, 0.04, P0(0, y + 0.012, (k + 0.5) / nr), F.q]);
      }
      const o = merged(parts, t.kind === 'ladder_on_rack' ? M.lad : M.ladAg); G.inst_tray.add(o); pick(o, '트레이', t);
    }
    // cables in the cross-section: one instanced mesh per tray and kind (click = the tray, popup shows the section)
    const byM = new Map();
    for (const [no, tier, x, y, od] of t.lay || []) {
      const c = kindOf[no], m = c && c.kind === '통신' ? M.comm : M.multi;
      if (!byM.has(m)) byM.set(m, []); byM.get(m).push([x, y + (tier - 1) * pitch + (ug ? 0 : 0.022), od]);
    }
    const qc = new THREE.Quaternion().setFromUnitVectors(Y0, F.d), mx = new THREE.Matrix4();
    for (const [m, arr] of byM) {
      const im = new THREE.InstancedMesh(cylG, m, arr.length);
      arr.forEach(([x, y, od], k) => { const r = od / 2000; mx.compose(P0(x - w / 2, y), qc, new THREE.Vector3(r, len, r)); im.setMatrixAt(k, mx); });
      im.instanceMatrix.needsUpdate = true; im.computeBoundingSphere(); G.inst_cable.add(im); pick(im, '트레이', t);
    }
  }
  for (const c of INST.cables) {          // branch cables in full; trunk cables: risers only (the tray part is laid above)
    const m = c.kind === '분기' ? M.branch : c.kind === '통신' ? M.comm : M.multi, r = Math.max(0.006, (c.od || 15) / 2000);
    for (let i = 1; i < c.path.length; i++) {
      const a = c.path[i - 1], b = c.path[i], vert = Math.abs(a[1] - b[1]) > 0.05 && Math.abs(a[0] - b[0]) < 0.05 && Math.abs(a[2] - b[2]) < 0.05;
      if (c.kind === '분기' || vert) bar(a, b, r * 2, r * 2, m, G.inst_cable, '케이블', c);
      if (c.kind !== '분기') bar(a, b, r * 2, r * 2, m, G2, '케이블', c);
    }
  }
  for (const x of INST.perforated || []) {  // perforated channel under the branch cable
    const parts = [];
    for (let i = 1; i < x.path.length; i++) {
      const A = V(x.path[i - 1]), B = V(x.path[i]), len = A.distanceTo(B); if (len < 0.05) continue;
      const F = basis(B.clone().sub(A)), C = at(A, B, 0.5).add(F.up.clone().multiplyScalar(-0.012));
      parts.push([x.w, 0.002, len, C, F.q]);
      for (const sx of [-1, 1]) parts.push([0.002, 0.05, len, C.clone().add(F.ac.clone().multiplyScalar(sx * x.w / 2)).add(F.up.clone().multiplyScalar(0.025)), F.q]);
    }
    if (parts.length) { const o = merged(parts, M.perf); G.inst_tray.add(o); pick(o, 'Perforated', x); }
  }
  // ---- vertical section (Section view): cut across a tray, or at an X / Z position; combines with the base Y section
  const VS = { on: false, axis: 'x', pos: 0, flip: false, tray: null };
  const vPlane = new THREE.Plane(new THREE.Vector3(1, 0, 0), 0);
  const bb = new THREE.Box3(); INST.trays.forEach(t => { bb.expandByPoint(V(t.a)); bb.expandByPoint(V(t.b)); });
  function syncClip() {
    if (VS.tray) { const t = VS.tray, A = V(t.a), B = V(t.b), d = B.clone().sub(A).normalize(); if (VS.flip) d.negate();
      vPlane.setFromNormalAndCoplanarPoint(d, at(A, B, 0.5)); }
    else { const n = VS.axis === 'x' ? new THREE.Vector3(1, 0, 0) : new THREE.Vector3(0, 0, 1); if (VS.flip) n.negate();
      vPlane.setFromNormalAndCoplanarPoint(n, VS.axis === 'x' ? new THREE.Vector3(VS.pos, 0, 0) : new THREE.Vector3(0, 0, VS.pos)); }
    renderer.clippingPlanes = [...(state.section ? [clipPlane] : []), ...(VS.on ? [vPlane] : [])];
    if (typeof requestShadow === 'function') requestShadow();
  }
  window.__pidClip = p => !VS.on || vPlane.distanceToPoint(p) >= -0.02;
  const _aa = applyAppearance; applyAppearance = function () { _aa(); syncClip(); };
  const sr = document.createElement('div'); sr.className = 'row'; sr.style.flexWrap = 'wrap';
  sr.innerHTML = `<input type="checkbox" id="vsOn"><label for="vsOn">수직 단면 (Section view)</label>
    <select id="vsAx" style="background:rgba(0,0,0,.25);color:#e6eefc;border:1px solid rgba(125,211,252,.35);border-radius:6px"><option value="x">X 위치 (동서)</option><option value="z">Z 위치 (남북)</option></select>
    <input type="range" id="vsPos" style="width:100%" step="0.1"><span id="vsV" style="font-size:11px"></span>
    <button type="button" class="btn" id="vsFlip">반대쪽</button><button type="button" class="btn" id="vsFree">트레이 단면 풀기</button>
    <div style="font-size:11px;opacity:.8;width:100%">트레이를 클릭 → 팝업의 '이 트레이 단면 보기'로 트레이 직각 단면 (케이블 배열이 보임)</div>`;
  layerBox.insertBefore(sr, hd.nextSibling);
  const vsPos = sr.querySelector('#vsPos'), vsAx = sr.querySelector('#vsAx');
  const range = () => { const lo = VS.axis === 'x' ? bb.min.x : bb.min.z, hi = VS.axis === 'x' ? bb.max.x : bb.max.z;
    vsPos.min = Math.floor(lo - 5); vsPos.max = Math.ceil(hi + 5); };
  const label = () => { sr.querySelector('#vsV').textContent = VS.tray ? `트레이 ${VS.tray.id} 직각 단면` : `${VS.axis.toUpperCase()} = ${(+VS.pos).toFixed(1)} m`; };
  range(); VS.pos = +vsPos.value; label();
  sr.querySelector('#vsOn').addEventListener('change', ev => { VS.on = ev.target.checked; CAPS.visible = VS.on; if (!VS.on) labelsOff(false); syncClip(); });
  vsAx.addEventListener('change', () => { VS.axis = vsAx.value; VS.tray = null; CAPS.clear(); range(); VS.pos = +vsPos.value; label(); syncClip(); });
  vsPos.addEventListener('input', () => { VS.tray = null; CAPS.clear(); VS.pos = +vsPos.value; label(); syncClip(); });
  sr.querySelector('#vsFlip').addEventListener('click', () => { VS.flip = !VS.flip; if (VS.tray) cutTray(VS.tray); else syncClip(); });
  sr.querySelector('#vsFree').addEventListener('click', () => { VS.tray = null; CAPS.clear(); labelsOff(false); label(); syncClip(); });
  let labelsWas = null;                              // labels hide the cut face: off while a tray section is shown
  function labelsOff(on) {
    if (!G.labels) return;
    if (on && labelsWas === null) { labelsWas = G.labels.visible; G.labels.visible = false; }
    if (!on && labelsWas !== null) { G.labels.visible = labelsWas; labelsWas = null; }
    const cb = document.getElementById('ly_labels'); if (cb) cb.checked = G.labels.visible;
  }
  const CAPS = new THREE.Group(); CAPS.name = 'inst_caps'; scene.add(CAPS);
  const capG = new THREE.CircleGeometry(1, 20);
  const capM = { [M.multi.uuid]: new THREE.MeshBasicMaterial({ color: 0xa855f7 }), [M.comm.uuid]: new THREE.MeshBasicMaterial({ color: 0xec4899 }) };
  function caps(t, d) {                              // filled discs on the cut face so the cables read as solid circles
    CAPS.clear();
    const A = V(t.a), B = V(t.b), F = basis(B.clone().sub(A)), pitch = t.pitch || 0.3, ug = t.kind === 'ug_trench';
    const C = at(A, B, 0.5).add(d.clone().multiplyScalar(0.003)), q = new THREE.Quaternion().setFromUnitVectors(new THREE.Vector3(0, 0, 1), d.clone().negate());
    for (const [no, tier, x, y, od] of t.lay || []) {
      const c = kindOf[no], m = capM[(c && c.kind === '통신' ? M.comm : M.multi).uuid], r = od / 2000;
      const o = new THREE.Mesh(capG, m); o.scale.set(r, r, 1); o.quaternion.copy(q);
      o.position.copy(C).add(F.ac.clone().multiplyScalar(x - t.w / 2)).add(F.up.clone().multiplyScalar(y + (tier - 1) * pitch + (ug ? 0 : 0.022)));
      CAPS.add(o);
    }
  }
  function cutTray(t) {
    VS.tray = t; VS.on = true; CAPS.visible = true; sr.querySelector('#vsOn').checked = true; label(); syncClip(); hidePopup();
    const A = V(t.a), B = V(t.b), M_ = at(A, B, 0.5), d = B.clone().sub(A).normalize(); if (VS.flip) d.negate();
    caps(t, d);
    const top = M_.clone().add(new THREE.Vector3(0, (t.h || 0.15) / 2 + (t.pitch || 0.3) * ((t.tiers || 1) - 1) / 2, 0));
    const eye = top.clone().sub(d.clone().multiplyScalar(Math.max(1.2, t.w * 1.6)));
    labelsOff(true);
    tween(eye, top);
  }
  function sectionSvg(t) {                         // 2D cross-section drawing of one tray: tiers, rails, numbered cables
    const lay = t.lay || [], n = t.tiers || 1, w = t.w, h = t.h || 0.15, pitch = t.pitch || 0.3, ug = t.kind === 'ug_trench';
    const top = Math.max(h, ...lay.map(c => c[3] + c[4] / 2000)) ;
    const H = (n - 1) * pitch + top + 0.04, S = Math.min(280 / w, 150 / H), PW = w * S + 40, PH = H * S + 20;
    const X = x => 20 + x * S, Yp = y => PH - 10 - y * S;
    let g = '';
    for (let i = 0; i < n; i++) { const y = i * pitch;
      g += ug ? `<rect x="${X(0)}" y="${Yp(y)}" width="${w * S}" height="3" fill="#9ca3af"/>`
              : `<rect x="${X(0)}" y="${Yp(y) - 3}" width="${w * S}" height="3" fill="#38bdf8"/><rect x="${X(0) - 2}" y="${Yp(y + h)}" width="2" height="${h * S}" fill="#38bdf8"/><rect x="${X(w)}" y="${Yp(y + h)}" width="2" height="${h * S}" fill="#38bdf8"/>`; }
    if (ug) g += `<rect x="${X(0) - 6}" y="${Yp(H)}" width="6" height="${H * S + 3}" fill="#9ca3af"/><rect x="${X(w)}" y="${Yp(H)}" width="6" height="${H * S + 3}" fill="#9ca3af"/>`;
    lay.forEach(([no, tier, x, y, od], k) => { const c = kindOf[no], col = c && c.kind === '통신' ? '#f472b6' : '#c084fc';
      const cy = (tier - 1) * pitch + y + (ug ? 0 : 0.02), r = Math.max(1.5, od / 2000 * S);
      g += `<circle cx="${X(x)}" cy="${Yp(cy)}" r="${r}" fill="${col}" stroke="#1e1b4b" stroke-width=".6"><title>${k + 1}. ${esc(no)} · OD ${od} mm</title></circle>`;
      if (r > 5) g += `<text x="${X(x)}" y="${Yp(cy) + 3}" font-size="8" text-anchor="middle" fill="#111">${k + 1}</text>`; });
    return `<svg width="${PW}" height="${PH}" style="background:rgba(255,255,255,.92);border-radius:6px;display:block;margin:6px 0">${g}</svg>`;
  }
  for (const j of INST.jbs) { const o = new THREE.Mesh(new THREE.BoxGeometry(0.8, 0.8, 0.35), M.jb); o.position.copy(V(j.p)); G.inst_jb.add(o); pick(o, 'JB', j); }
  const sg = new THREE.SphereGeometry(0.28, 12, 8);
  for (const x of INST.instruments) { const o = new THREE.Mesh(sg, x.est ? M.est : M.inst); o.position.copy(V(x.p)); G.inst_instr.add(o); pick(o, '계기', x); }
  for (const x of INST.supports) {
    const p = V(x.p);
    if (x.base === 0) { const h = Math.max(0.3, p.y); const o = new THREE.Mesh(new THREE.BoxGeometry(0.12, h, 0.12), M.post); o.position.set(p.x, h / 2, p.z); G.inst_support.add(o); pick(o, '서포트', x); }
    else if (x.type === 'RC') { const o = new THREE.Mesh(new THREE.BoxGeometry(0.15, 0.08, 0.15), M.clamp); o.position.copy(p); G.inst_support.add(o); pick(o, '서포트', x); }
    else if (x.type === 'UB' || x.type === 'UM' || x.type === 'UP') { const o = new THREE.Mesh(new THREE.BoxGeometry(0.4, 0.05, 0.05), M.clamp); o.position.copy(p); G.inst_support.add(o); pick(o, '서포트', x); }
    else { const o = new THREE.Mesh(new THREE.BoxGeometry(0.15, 0.6, 0.15), M.rack); o.position.set(p.x, p.y + 0.3, p.z); G.inst_support.add(o); pick(o, '서포트', x); }
  }
  for (const j of INST.joints) { const o = new THREE.Mesh(new THREE.BoxGeometry(0.5, 0.5, 0.5), M.joint); o.position.copy(V(j.p)); G.inst_support.add(o); pick(o, '익스펜션 조인트', j); }
  for (const r of INST.rooms) { const o = new THREE.Mesh(new THREE.BoxGeometry(3, 3, 3), M.room); o.position.copy(V(r.p)).add(new THREE.Vector3(0, 1.5, 0)); G.inst_jb.add(o); pick(o, '룸', r); }
  // 계장 찾기: tag / JB / cable / tray / support number -> fly there and open the properties
  const FINDX = [];
  const reg = (cat, d, p) => FINDX.push({ id: String(d.id).toUpperCase(), cat, d, p });
  INST.instruments.forEach(x => reg('계기', x, x.p)); INST.jbs.forEach(x => reg('JB', x, x.p));
  INST.trays.forEach(x => reg('트레이', x, [(x.a[0] + x.b[0]) / 2, (x.a[1] + x.b[1]) / 2, (x.a[2] + x.b[2]) / 2]));
  INST.cables.forEach(x => reg('케이블', x, x.path[Math.floor(x.path.length / 2)])); INST.supports.forEach(x => reg('서포트', x, x.p));
  const fr = document.createElement('div'); fr.className = 'row';
  fr.innerHTML = '<input id="instFind" placeholder="계장 찾기: 태그 / JB / 트레이 / 케이블" style="flex:1;min-width:0;background:rgba(0,0,0,.25);color:#e6eefc;border:1px solid rgba(125,211,252,.35);border-radius:8px;padding:4px 7px;font:inherit">';
  layerBox.insertBefore(fr, hd.nextSibling);
  const goFind = q => { q = q.trim().toUpperCase(); if (!q) return; const h = FINDX.find(x => x.id === q) || FINDX.find(x => x.id.includes(q));
    if (!h) { fr.title = '없음'; return; } focusOn(V(h.p), 28); showPopup('inst', { cat: h.cat, ...h.d }); };
  fr.querySelector('input').addEventListener('keydown', ev => { if (ev.key === 'Enter') { ev.preventDefault(); goFind(ev.target.value); } });
  if (location.hash.startsWith('#find=')) setTimeout(() => goFind(decodeURIComponent(location.hash.slice(6))), 1500);
  const _showPopup = showPopup;
  showPopup = function (kind, d) {
    if (kind !== 'inst') return _showPopup(kind, d);
    const rows = Object.entries(d.props || {}).filter(([, v]) => v !== '' && v !== null && v !== undefined);
    popup.innerHTML = `<div class="ph"><div><div class="k">계장 (pid_io) · ${esc(d.cat)}</div><div class="t">${esc(d.id)}</div></div><button class="close" type="button" aria-label="Close">✕</button></div>
      <table>${rows.map(([a, b]) => `<tr><td>${esc(a)}</td><td>${esc(String(b))}</td></tr>`).join('')}
      <tr><td>좌표 (X / Y / Z m)</td><td>${(d.p || d.a || [0, 0, 0]).map(n => (+n).toFixed(1)).join(' / ')}</td></tr>
      <tr><td>출처</td><td>pid_io ${esc(INST.version)} · ${esc(INST.project)} · 좌표 맞춤 ${esc(INST.transform.formula)}</td></tr></table>`;
    if (d.cat === '트레이' && TRAY_BY[d.id]) {
      const t = TRAY_BY[d.id];
      popup.insertAdjacentHTML('beforeend', `<div class="ph" style="margin-top:6px"><div class="k">단면 (케이블 ${(t.lay || []).length}가닥 · 폭 ${Math.round(t.w * 1000)} · ${t.tiers || 1}단)</div>
        <button type="button" class="btn" id="cutHere">이 트레이 단면 보기</button></div>${sectionSvg(t)}
        <details><summary style="cursor:pointer">케이블 배열 목록</summary><table>${(t.lay || []).map(([no, tier, x, y, od], k) =>
          `<tr><td>${k + 1}. ${esc(no)}</td><td>${tier}단 · X ${Math.round(x * 1000)} · Y ${Math.round(y * 1000)} mm · OD ${od}</td></tr>`).join('')}</table></details>`);
      popup.querySelector('#cutHere').addEventListener('click', () => cutTray(t));
    }
    popup.classList.add('show'); popup.querySelector('.close').addEventListener('click', hidePopup);
  };
  console.log('[pid_io] INST layer', INST.counts);
}
"""


def build_html(base_path: Path, inst: dict) -> str:
    s = Path(base_path).read_text(encoding="utf-8")
    s = s.replace("(!state.section || h.point.y <= state.sectionH)",          # v13.3: clicks ignore what the vertical section hides
                  "(!state.section || h.point.y <= state.sectionH) && (!window.__pidClip || window.__pidClip(h.point))", 1)
    i = s.find('<script type="module">')
    if i < 0:
        raise ValueError("3D 모델에 <script type=\"module\">이 없습니다 (Three.js 모듈 형식이 아님)")
    j = s.find("</script>", i)
    block = INJECT.replace("__INST__", json.dumps(inst, ensure_ascii=False, separators=(",", ":")))
    out = s[:j] + block + s[j:]
    return out.replace("<title>", "<title>[계장 보강] ", 1)


# ------------------------------------------------------------------ standard handoff files
def _sha(p: Path) -> str:
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def standard_files(outdir: Path, project: str, version: str, base: dict, base_path: Path, inst: dict, tf: dict,
                   s: dict, res, tr, sp, rows_n: int, sources: dict, validation: list, model_file: Path,
                   data_file: Path, check_summary: str, proto: dict | None = None):
    v = s["values"]
    outdir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y-%m-%d %H:%M")
    rev = time.strftime("R%Y%m%d")
    files = {}

    def wjson(name, obj):
        (outdir / name).write_text(json.dumps(obj, ensure_ascii=False, indent=1), encoding="utf-8")
        files[name] = outdir / name

    # 01 scope: confirmed quantity scope (owner, class, spec, unit, qty, rule)
    scope = []
    from collections import Counter
    spec = Counter()
    for c in res["cables"]:
        spec[(c["kind"], c["spec"])] += c["length"]
    k = 0
    def add(cls, sp_, unit, qty, rule):
        nonlocal k
        k += 1
        scope.append([f"CI-{k:03d}", "계장 (T1-CI)", cls, sp_, unit, qty, rule])
    add("IO", "HW IO (AI/AO/DI/DO/RTD)", "점", sum(1 for _ in range(0)) or None, "R-IO-*")
    for (kind, sp_), m in sorted(spec.items()):
        add("케이블", f"{sp_} ({kind})", "m", m, "R-CB-LEN")
    for sz, n in sorted(Counter(j["size"] for j in res["jbs"]).items()):
        add("JB", sz, "EA", n, "R-JB-SIZE")
    for x in tr["summary"]["ladder"]:
        add("사다리 트레이", f"{x['cls']} {x['width']}x{int(float(v['tray_height']))}", "m", x["order"], "R-TR-WIDTH")
    for x in tr["summary"].get("ug_by_width") or []:
        add("UG 트렌치", f"W{x['width']}" + (f" x{x['tiers']}" if x["tiers"] > 1 else ""), "m", x["order"], "R-UG-WIDTH")
    add("Perforated 트레이", str(v["pt_width"]), "m", tr["summary"]["pt_m"], "R-PT")
    for x in sp["summary"]["rows"]:
        add("서포트", x["name"], "EA", x["count"], "R-SP-" + x["type"])
    add("서포트 강재", "강재 (할증 포함)", "kg", sp["summary"]["kg_order"], "R-SP-KG")
    add("트레이 부속", "익스펜션 스플라이스 플레이트", "SET", sp["summary"]["joint_plates"], "R-SP-EXP")
    scope[0][5] = rows_n
    with open(outdir / "01_scope.csv", "w", encoding="utf-8-sig", newline="") as f:
        w = csv.writer(f)
        w.writerow(["scope_id", "담당", "분류", "규격", "단위", "수량", "rule_id"])
        w.writerows(scope)
    files["01_scope.csv"] = outdir / "01_scope.csv"

    # 02 input / model contract: entities, fields (minimum / derived / approved), coordinates, interfaces
    wjson("02_input_model_contract.json", {
        "profile": PROFILE, "skill": SKILL, "rev": rev,
        "inputs": [{"id": "P&ID", "file": sources.get("pdf"), "kind": "minimum"},
                   {"id": "PLOT_PLAN", "file": sources.get("plot"), "kind": "minimum"},
                   {"id": "INDEX_TEMPLATE", "file": sources.get("template"), "kind": "minimum"},
                   {"id": "CHECKLIST", "file": "concept.xlsx + cable.json", "kind": "approved (설계 가정)"},
                   {"id": "BASE_MODEL", "file": base["file"], "sha256": base["sha256"], "kind": "seed"}],
        "entities": {
            "instrument": {"id": "instrument_tag", "fields": {"instrument_tag": "minimum (P&ID)", "io_type": "derived (rule)", "system": "derived (rule)",
                                                              "jb_id": "derived", "field_cable": "derived", "xyz": "derived (PLOT, 추정 표시)"}},
            "io": {"id": "io_id = instrument_tag + io_type (IO 1점 = 인덱스 1행)", "fields": {"origin": "도면인식/Typical파생/트레인복제/대수복제/장비Typical/패키지/스페어"}},
            "jb": {"id": "jb_id (JB-유닛-시스템신호[I]-순번)", "fields": {"size": "derived (spare rule)", "slot": "derived (rack column / equipment edge)", "xyz": "derived"}},
            "cable": {"id": "circuit / cable_no (C-태그, MC-JB꼬리, FO-태그)", "fields": {"spec": "approved (STEP 6)", "od_mm": "approved (catalogue)", "length_m": "derived", "path": "derived (route)"}},
            "route_segment": {"id": "route_segment_id (TR-/UG-/PT-)", "fields": {"kind": "ladder_on_rack / ladder_on_supports / ug_trench / perforated", "width": "derived (fill)", "tiers": "derived", "el": "derived (model cable tier on racks)"}},
            "support": {"id": "support_id (SP-/CL-/UB-)", "fields": {"type": "derived (rule table)", "typical": "approved (TS-xx)", "role": "fitting / expansion", "fix": "anchor / guide"}}},
        "coordinates": {"model": "X east, Y up (EL m), Z south (base model frame)", "plot": "pt, PDF page top-left origin, y down",
                        "transform": tf["formula"], "anchors": tf["anchors"], "rack_anchors": tf["rack_anchors"],
                        "residual_median_m": tf["median_m"], "residual_max_m": tf["max_m"]},
        "interfaces": {"piping": "racks / equipment from base model (read only)", "electrical": "MCC↔IRP 전기 범위 (미집계)",
                       "package": "패키지 UCP → 룸 (vendor boundary)", "dedupe": "instrument_tag / cable_no unique"}})

    # 03 engineering rules: formulas, judgements, exceptions, outputs, units, sources
    from . import cable as C
    rules = []
    for g in C.SCHEMA + C.JB.SCHEMA + C.TR.SCHEMA + C.SP.SCHEMA:
        for it in g["items"]:
            rules.append({"id": it.get("code") or it["id"], "group": g["title"], "name": it["name"], "value": v.get(it["id"]),
                          "default": it["default"], "unit": it.get("unit", ""), "note": it.get("help", "")})
    wjson("03_engineering_rules.json", {
        "profile": PROFILE, "rev": rev, "source": "pid_io " + version + " (STEP 3 체크리스트 + STEP 6~9 설정)",
        "formulas": {
            "R-IO-SPARE": "spare = ceil(n(system, io) × rate H01~H04; RTD = H01)",
            "R-CB-BRANCH": "IO 1점 = 분기 1가닥; length = round_up((수평 × 우회 + 높이) × (1 + 여유), 올림)",
            "R-CB-MULTI": "JB 1개 = 멀티 1가닥; pairs = JB size",
            "R-JB-SIZE": "smallest T: ceil(n×(1+cable spare)) ≤ M, ceil(n×(1+terminal spare)) ≤ T",
            "R-TR-WIDTH": "Σ π/4·OD² ≤ W × H × Fill → smallest W; else tiers of the widest",
            "R-UG-WIDTH": "same with trench height / fill; run number changes at a branch or size change",
            "R-SP-SPAN": "fixed points (ends / fitting / joint) + fill at ≤ span (rule table)",
            "R-SP-FIT": "NEMA VE 2: support within 600 mm of each fitting end",
            "R-SP-EXP": "joint spacing = movement / (α × ΔT); anchor at the middle support, guides elsewhere",
            "R-3D-EL": "trays on a rack at model RK.cable EL; off-rack AG / UG at checklist EL"},
        "settings": rules, "protocol": proto, "exceptions": ["현장 지시계·PSV·소프트 태그 = 케이블 없음", "벤더 PLC 내부 IO = 벤더 범위 (A03)",
                                          "MCC↔IRP = 전기 범위", "PLOT 위치 없는 계기 = 가정 길이"]})

    # 04 test inputs (no expected values)
    wjson("04_test_inputs.json", {"profile": PROFILE, "rev": rev, "cases": [
        {"id": "T-NORMAL", "type": "NORMAL", "input": "현재 체크리스트·설정 + PLOT"},
        {"id": "T-CHANGED-JB60", "type": "CHANGED", "input": "JB 크기 = 60P"},
        {"id": "T-CHANGED-AG", "type": "CHANGED", "input": "랙 밖 간선 = AG 서포트"},
        {"id": "T-CHANGED-SPARE30", "type": "CHANGED", "input": "JB 스페어 30%"},
        {"id": "T-MISSING-PLOT", "type": "MISSING", "input": "PLOT 좌표 사용 안 함 (가정 길이)"},
        {"id": "T-HOLDOUT-P1", "type": "HOLDOUT", "input": "다른 프로젝트 폴더 (서남해 200MW 수전해, PLOT 없음)"}]})

    # 05 validation
    with open(outdir / "05_validation.csv", "w", encoding="utf-8-sig", newline="") as f:
        w = csv.writer(f)
        w.writerow(["case_id", "항목", "기대값/허용차 (사전 잠금)", "실행값", "판정", "확인자", "비고"])
        w.writerows(validation)
    files["05_validation.csv"] = outdir / "05_validation.csv"

    # 06 model handoff
    wjson("06_model_handoff.json", {
        "profile": PROFILE, "rev": rev, "seed": {"file": base["file"], "sha256": base["sha256"], "bytes": base["bytes"],
                                                 "three": base["three"], "title": base["title"], "source_id": "piping 3D (3d-rev0)"},
        "applied_scope": "계장 개략 물량·배치 (계기·JB·케이블·트레이·트렌치·서포트)",
        "SUPPLEMENTED_MODEL": {"file": model_file.name, "sha256": _sha(model_file), "bytes": model_file.stat().st_size,
                               "how": "base HTML unchanged + one block appended to its module script (layers 계장 · ...)",
                               "layers": ["inst_tray", "inst_cable", "inst_jb", "inst_instr", "inst_support"]},
        "STRUCTURED_MODEL_DATA": {"file": data_file.name, "sha256": _sha(data_file), "kind": "FULL"},
        "changes": inst["counts"], "clash": {"other": inst.get("other"), "moves": inst.get("moves") or []}, "transform": {k: tf[k] for k in ("s", "tx", "tz", "anchors", "rack_anchors", "median_m", "max_m", "formula")},
        "verification": check_summary})

    # 00 manifest last (lists everything)
    wjson("00_manifest.json", {
        "profile": PROFILE, "skill": SKILL, "project": project, "made": stamp, "pid_io": version,
        "rev": {"scope": rev, "rule": rev, "model": rev},
        "sources": [{"id": k, "file": f} for k, f in sources.items() if f] + [{"id": "BASE_MODEL", "file": base["file"], "sha256": base["sha256"]}],
        "files": {n: {"sha256": _sha(p)} for n, p in sorted(files.items())},
        "model_required_scope_ids": [r[0] for r in scope],
        "gate_evidence": {"G1~G6": "HOLD - 회사 Agent 실행·공종 승인 전 (pid_io 자체 점검: " + check_summary + ")"},
        "open": ["좌표 맞춤 잔차 (06 transform) 검토", "계기 위치 다수 추정", "회사 가이드 원문(HTML) 받은 뒤 필드 이름 맞춤"],
        "status": "DRAFT_NOT_READY"})
    return files
