"""리비전 대조: snapshots of the whole result and a comparison that says WHY each quantity changed.

A snapshot (projects/<p>/revisions/<stamp>.json) is saved on every export (cable / basis / 3D handoff) and by hand.
It holds the inputs (plot file + key, 3D model hash, settings), equipment and instrument positions, and every
JB, cable, tray, trench run and the support totals.

Comparing two snapshots gives:
  inputs     plot / model file changed, settings changed (name: old -> new), equipment moved (m)
  changes    every JB / cable / tray / trench added, removed or changed, with the before -> after values and a
             cause: the equipment or instrument move behind it, a JB move, a setting, or a route / rack change
  summary    totals before -> after (%), same words as the 산출근거 workbook
"""
from __future__ import annotations

import json
import math
import time
from pathlib import Path


def snapshot(project: str, rows, s: dict, res: dict, tr: dict, sp: dict, layout, plot: dict | None,
             model: dict | None, metrics: dict, label: str = "") -> dict:
    m = layout.m_per_pt if layout is not None else None
    g = res.get("geo") or {}
    eq = {}
    if layout is not None:
        for k, p in (layout.pos or {}).items():
            eq[k] = [round(p[0] * m, 2), round(p[1] * m, 2)] if m else list(p)
    inst = {t: {"xy": [round(x[0] * m, 2), round(x[1] * m, 2)] if m else [x[0], x[1]], "src": x[2]}
            for t, x in (g.get("inst") or {}).items()}
    jbs = {}
    for j in res["jbs"]:
        jbs[j["no"]] = {"size": j["size"], "used": j["used"], "xy": [j.get("x_m"), j.get("y_m")], "tags": j["tags"],
                        "place": j.get("place", "")}
    cables = {c["no"]: {"kind": c["kind"], "spec": c["spec"], "len": c["length"], "frm": c["frm"], "to": c["to"]}
              for c in res["cables"]}
    trays = {}
    for sg in tr.get("segments", []):
        if sg["kind"] == "UG":
            trays[sg["no"]] = {"kind": "UG", "w": sg.get("width"), "n": sg.get("tiers"), "len": sg["length"],
                               "cables": len(sg["cables"]), "run": sg.get("run")}
        for t in sg.get("trays") or []:
            trays[t["no"]] = {"kind": sg["kind"], "w": t["width"], "n": t["tiers"], "len": sg["length"], "cables": t["count"]}
    by_cable = {k: [x["no"] for x in v["legs"]] for k, v in (tr.get("by_cable") or {}).items()}
    return {"project": project, "made": time.strftime("%Y-%m-%d %H:%M"), "label": label,
            "inputs": {"plot": (plot or {}).get("pdf") or "", "plot_key": str((plot or {}).get("_key")),
                       "model": (model or {}).get("file"), "model_sha": (model or {}).get("sha256"),
                       "settings": dict(s["values"])},
            "equipment": eq, "instruments": inst, "jbs": jbs, "cables": cables, "trays": trays, "by_cable": by_cable,
            "supports": {x["type"]: x["count"] for x in sp["summary"]["rows"]}, "metrics": metrics}


def save(pdir: Path, snap: dict) -> Path:
    d = Path(pdir) / "revisions"
    d.mkdir(exist_ok=True)
    f = d / f"{time.strftime('%Y%m%d_%H%M%S')}.json"
    f.write_text(json.dumps(snap, ensure_ascii=False), encoding="utf-8")
    return f


def listing(pdir: Path):
    d = Path(pdir) / "revisions"
    out = []
    for f in sorted(d.glob("*.json"), reverse=True) if d.exists() else []:
        try:
            h = json.loads(f.read_text(encoding="utf-8"))
            out.append({"id": f.stem, "made": h.get("made"), "label": h.get("label", ""), "plot": h["inputs"].get("plot"),
                        "model": h["inputs"].get("model"), "cable_m": h["metrics"].get("케이블 m"), "jbs": h["metrics"].get("JB")})
        except Exception:                                  # noqa: BLE001
            continue
    return out


def load(pdir: Path, sid: str) -> dict:
    return json.loads((Path(pdir) / "revisions" / f"{Path(sid).name}.json").read_text(encoding="utf-8"))


def _d(a, b):
    return math.hypot(a[0] - b[0], a[1] - b[1]) if a and b and None not in (a[0], a[1], b[0], b[1]) else None


def compare(old: dict, new: dict, names: dict | None = None) -> dict:
    names = names or {}
    # ---- inputs
    inputs = []
    for k, lab in (("plot", "PLOT PLAN"), ("model", "3D 모델")):
        if old["inputs"].get(k) != new["inputs"].get(k):
            inputs.append([lab, old["inputs"].get(k) or "-", new["inputs"].get(k) or "-"])
    if old["inputs"].get("model") == new["inputs"].get("model") and old["inputs"].get("model_sha") != new["inputs"].get("model_sha"):
        inputs.append(["3D 모델 내용", (old["inputs"].get("model_sha") or "")[:12], (new["inputs"].get("model_sha") or "")[:12]])
    so, sn = old["inputs"]["settings"], new["inputs"]["settings"]
    changed_settings = sorted(k for k in set(so) | set(sn) if so.get(k) != sn.get(k))
    for k in changed_settings:
        inputs.append([f"설정: {names.get(k, k)}", so.get(k), sn.get(k)])
    moved_eq = {}
    for k in set(old["equipment"]) & set(new["equipment"]):
        d = _d(old["equipment"][k], new["equipment"][k])
        if d is not None and d >= 1.0:
            moved_eq[k] = round(d, 1)
    for k in set(new["equipment"]) - set(old["equipment"]):
        moved_eq[k] = "새로 생김"
    for k in set(old["equipment"]) - set(new["equipment"]):
        moved_eq[k] = "없어짐"
    for k, d in sorted(moved_eq.items(), key=lambda kv: str(kv[1])):
        inputs.append([f"장비 {k}", "", f"{d} m 이동" if isinstance(d, float) else d])
    moved_inst = {}
    for t in set(old["instruments"]) & set(new["instruments"]):
        d = _d(old["instruments"][t]["xy"], new["instruments"][t]["xy"])
        if d is not None and d >= 1.0:
            moved_inst[t] = round(d, 1)
    rack_change = any(k in changed_settings for k in ("rack_src", "rack_el", "rack_el_src", "rack_width", "rack_pref", "main_route", "trunk_off")) \
        or old["inputs"].get("plot_key") != new["inputs"].get("plot_key") or old["inputs"].get("model_sha") != new["inputs"].get("model_sha")
    setting_txt = ", ".join(names.get(k, k) for k in changed_settings[:6]) + (" …" if len(changed_settings) > 6 else "")

    def inst_cause(tags):
        hits = [f"{t} {moved_inst[t]} m" for t in tags if t in moved_inst]
        if hits:
            src = {new["instruments"].get(t.split()[0], {}).get("src", "") for t in hits}
            return f"계기 위치 이동 ({', '.join(hits[:3])}{' …' if len(hits) > 3 else ''})" + (f" ← {', '.join(sorted(src))[:80]}" if src else "")
        return ""

    def cause_jb(no, oj, nj):
        c = []
        d = _d(oj.get("xy"), nj.get("xy"))
        if d is not None and d >= 1.0:
            c.append(f"JB 자리 이동 {d:.0f} m ({oj.get('place')} → {nj.get('place')})"
                     + (" ← 랙·PLOT·모델 변경 (자리 후보가 바뀜)" if rack_change else ""))
        if set(oj["tags"]) != set(nj["tags"]):
            c.append(f"담긴 계기 바뀜 (+{len(set(nj['tags']) - set(oj['tags']))} / −{len(set(oj['tags']) - set(nj['tags']))})")
        ic = inst_cause(nj["tags"])
        if ic:
            c.append(ic)
        if changed_settings and not c:
            c.append(f"설정 변경 ({setting_txt})")
        return " · ".join(c) or "재계산 결과 (입력 같음)"

    changes = []
    # ---- JB
    for no in sorted(set(old["jbs"]) | set(new["jbs"])):
        oj, nj = old["jbs"].get(no), new["jbs"].get(no)
        if not oj:
            changes.append(["JB", no, "추가", "", f"{nj['size']} · {nj['used']}가닥", "",
                            inst_cause(nj["tags"]) or (f"설정·묶음 변경 ({setting_txt})" if changed_settings else "묶음 바뀜")])
        elif not nj:
            changes.append(["JB", no, "삭제", f"{oj['size']} · {oj['used']}가닥", "", "", inst_cause(oj["tags"]) or (f"설정 변경 ({setting_txt})" if changed_settings else "묶음 바뀜")])
        elif (oj["size"], oj["used"], oj.get("place")) != (nj["size"], nj["used"], nj.get("place")) or (_d(oj.get("xy"), nj.get("xy")) or 0) >= 1:
            changes.append(["JB", no, "변경", f"{oj['size']} · {oj['used']}가닥 · {oj.get('place')}", f"{nj['size']} · {nj['used']}가닥 · {nj.get('place')}",
                            "", cause_jb(no, oj, nj)])
    # ---- cables
    for no in sorted(set(old["cables"]) | set(new["cables"])):
        oc, nc = old["cables"].get(no), new["cables"].get(no)
        if not oc:
            changes.append(["케이블", no, "추가", "", f"{nc['spec']} {nc['len']} m", nc["len"], f"{nc['frm']} → {nc['to']} 새로 생김"])
            continue
        if not nc:
            changes.append(["케이블", no, "삭제", f"{oc['spec']} {oc['len']} m", "", -oc["len"], f"{oc['frm']} → {oc['to']} 없어짐"])
            continue
        if (oc["spec"], oc["len"], oc["to"]) == (nc["spec"], nc["len"], nc["to"]):
            continue
        why = []
        if oc["to"] != nc["to"]:
            why.append(f"도착 JB 바뀜 {oc['to']} → {nc['to']}")
        if nc["kind"] == "분기":
            ic = inst_cause([nc["frm"]])
            if ic:
                why.append(ic)
            jo, jn = old["jbs"].get(nc["to"]), new["jbs"].get(nc["to"])
            if jo and jn and (_d(jo.get("xy"), jn.get("xy")) or 0) >= 1:
                why.append(f"JB {nc['to']} 이동 {_d(jo['xy'], jn['xy']):.0f} m")
        else:
            jo, jn = old["jbs"].get(nc["frm"]), new["jbs"].get(nc["frm"])
            if jo and jn and (_d(jo.get("xy"), jn.get("xy")) or 0) >= 1:
                why.append(f"출발 JB 이동 {_d(jo['xy'], jn['xy']):.0f} m")
            if old["by_cable"].get(no) != new["by_cable"].get(no):
                why.append("경로(트레이 순서) 바뀜" + (" ← 랙·PLOT·모델 변경" if rack_change else ""))
        if oc["spec"] != nc["spec"]:
            why.append(f"사양 {oc['spec']} → {nc['spec']}")
        if changed_settings and not why:
            why.append(f"설정 변경 ({setting_txt})")
        changes.append(["케이블", no, "변경", f"{oc['spec']} {oc['len']} m", f"{nc['spec']} {nc['len']} m", nc["len"] - oc["len"],
                        " · ".join(why) or "재계산 결과"])
    # ---- trays / trench
    for no in sorted(set(old["trays"]) | set(new["trays"])):
        ot, nt = old["trays"].get(no), new["trays"].get(no)
        fmt = lambda t: (f"트렌치 {t['w']}" if t["kind"] == "UG" else f"{t['w']}×{t['n']}단") + f" {t['len']} m · 케이블 {t['cables']}"
        if not ot:
            changes.append(["트레이", no, "추가", "", fmt(nt), nt["len"] * (nt.get("n") or 1), "경로가 새로 지나감" + (" ← 랙·PLOT·모델 변경" if rack_change else "")])
        elif not nt:
            changes.append(["트레이", no, "삭제", fmt(ot), "", -ot["len"] * (ot.get("n") or 1), "경로가 더 이상 안 지나감" + (" ← 랙·PLOT·모델 변경" if rack_change else "")])
        elif (ot["w"], ot["n"], ot["len"], ot["cables"]) != (nt["w"], nt["n"], nt["len"], nt["cables"]):
            why = []
            if ot["cables"] != nt["cables"]:
                why.append(f"지나가는 케이블 {ot['cables']} → {nt['cables']}")
            if ot["len"] != nt["len"]:
                why.append(f"구간 길이 {ot['len']} → {nt['len']} m")
            if (ot["w"], ot["n"]) != (nt["w"], nt["n"]) and ot["cables"] == nt["cables"]:
                why.append(f"폭 규칙·외경 설정 ({setting_txt})" if changed_settings else "폭 재계산")
            changes.append(["트레이", no, "변경", fmt(ot), fmt(nt), nt["len"] * (nt.get("n") or 1) - ot["len"] * (ot.get("n") or 1), " · ".join(why)])
    sup = [[k, old["supports"].get(k, 0), new["supports"].get(k, 0), new["supports"].get(k, 0) - old["supports"].get(k, 0)]
           for k in sorted(set(old["supports"]) | set(new["supports"]))]
    from .basis import words, NUM_KEYS
    summary = [[k, old["metrics"].get(k, 0), new["metrics"].get(k, 0), new["metrics"].get(k, 0) - old["metrics"].get(k, 0),
                (f"{(new['metrics'].get(k, 0) - old['metrics'].get(k, 0)) / old['metrics'][k] * 100:+.1f}%" if old["metrics"].get(k) else "")]
               for k in NUM_KEYS]
    return {"old": {"made": old["made"], "label": old.get("label")}, "new": {"made": new["made"], "label": new.get("label")},
            "inputs": inputs, "changes": changes, "supports": sup, "summary": summary,
            "words": words(old["metrics"], new["metrics"]), "moved_equipment": len(moved_eq), "moved_instruments": len(moved_inst)}


def write_xlsx(path: Path, cmp: dict):
    from openpyxl import Workbook
    from .basis import _sheet
    wb = Workbook()
    wb.remove(wb.active)
    head = [["이전", f"{cmp['old']['made']} {cmp['old'].get('label') or ''}"], ["이후", f"{cmp['new']['made']} {cmp['new'].get('label') or ''}"],
            ["한 줄 요약", cmp["words"] or "변화 없음"], ["움직인 장비 / 계기", f"{cmp['moved_equipment']} / {cmp['moved_instruments']}"],
            ["변경 건수", len(cmp["changes"])], ["", ""]]
    _sheet(wb, "0_요약", ["항목", "값"], head + [["", ""]] + [["== 물량", "이전 / 이후 / Δ / %"]] +
           [[r[0], f"{r[1]:,} → {r[2]:,} ({r[3]:+,}) {r[4]}"] for r in cmp["summary"] if r[3]], [26, 110], wrap=(2,))
    _sheet(wb, "1_입력변화", ["항목", "이전", "이후"], cmp["inputs"], [36, 40, 40])
    _sheet(wb, "2_변경목록", ["구분", "번호", "변화", "이전", "이후", "Δ (m)", "원인"], cmp["changes"], [8, 22, 7, 34, 34, 8, 90], wrap=(7,))
    _sheet(wb, "3_서포트", ["형식", "이전", "이후", "Δ"], cmp["supports"], [10, 10, 10, 10])
    path.parent.mkdir(parents=True, exist_ok=True)
    wb.save(path)
    return path
