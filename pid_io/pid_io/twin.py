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

    def P(p, el=None, snap=False, side=0.0):
        X, Z = xz(p["pt"])
        Y = p["m"][2] if el is None else el
        if snap or abs(Y - rack_el) < 1e-6:
            k = on_rack(X, Z)
            if k:
                if k["dir"] == "NS":
                    X = k["off"] + side
                else:
                    Z = k["off"] + side
                Y = k["cable_el"] + (Y - rack_el if not snap else 0)
        return [round(X, 2), round(Y, 2), round(Z, 2)]

    out = {"frame": "model (X east, Y up EL, Z south) m", "transform": tf, "instruments": [], "jbs": [], "cables": [],
           "trays": [], "supports": [], "joints": [], "rooms": []}
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
        pts = [P(p) for p in c["path"]]
        out["cables"].append({"id": c["no"], "kind": c["kind"], "od": c["od_mm"], "path": pts,
                              "props": {"Cable No": c["no"], "구분": c["kind"], "From": c["from"], "To": c["to"], "Spec": c["spec"],
                                        "OD (mm)": c["od_mm"], "길이 (m)": c["length_m"], "트레이": " → ".join(c.get("trays") or [])}})
    for t in d3["trays"]:
        snap = t["kind"] == "ladder_on_rack"
        side = (t.get("side_offset_mm") or 0) / 1000
        a, b = P(t["start"], snap=snap, side=side), P(t["end"], snap=snap, side=side)
        out["trays"].append({"id": t["no"], "kind": t["kind"], "a": a, "b": b, "w": (t.get("width_mm") or 300) / 1000,
                             "tiers": t.get("tiers") or 1,
                             "props": {"Tray No": t["no"], "구간": t["seg"], "트렌치 줄": t.get("run") or "", "종류": t["kind"],
                                       "구분": t.get("class", ""), "폭 (mm)": t.get("width_mm"), "단": t.get("tiers"),
                                       "Fill": f"{round((t.get('fill') or 0) * 100)}%", "길이 (m)": t["length_m"],
                                       "케이블 수": len(t["cables"]), "케이블": ", ".join(t["cables"][:40]) + (" …" if len(t["cables"]) > 40 else "")}})
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
    out["counts"] = {k: len(out[k]) for k in ("instruments", "jbs", "cables", "trays", "supports", "joints")}
    return out


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
  for (const t of INST.trays) {
    const m = t.kind === 'ug_trench' ? M.ug : t.kind === 'ladder_on_rack' ? M.lad : M.ladAg;
    for (let i = 0; i < (t.tiers || 1); i++) {
      const a = [t.a[0], t.a[1] + i * 0.3, t.a[2]], b = [t.b[0], t.b[1] + i * 0.3, t.b[2]];
      bar(a, b, t.w, t.kind === 'ug_trench' ? 0.3 : 0.1, m, G.inst_tray, '트레이', t);
    }
  }
  for (const c of INST.cables) {
    const m = c.kind === '분기' ? M.branch : c.kind === '통신' ? M.comm : M.multi, r = Math.max(0.012, (c.od || 15) / 2000);
    for (let i = 1; i < c.path.length; i++) bar(c.path[i - 1], c.path[i], r * 2, r * 2, m, G.inst_cable, '케이블', c);
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
    popup.classList.add('show'); popup.querySelector('.close').addEventListener('click', hidePopup);
  };
  console.log('[pid_io] INST layer', INST.counts);
}
"""


def build_html(base_path: Path, inst: dict) -> str:
    s = Path(base_path).read_text(encoding="utf-8")
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
                   data_file: Path, check_summary: str):
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
        "settings": rules, "exceptions": ["현장 지시계·PSV·소프트 태그 = 케이블 없음", "벤더 PLC 내부 IO = 벤더 범위 (A03)",
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
        "changes": inst["counts"], "transform": {k: tf[k] for k in ("s", "tx", "tz", "anchors", "rack_anchors", "median_m", "max_m", "formula")},
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
