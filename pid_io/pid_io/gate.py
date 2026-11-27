"""결정 보호 규칙 + 3D 반영 관문 (v13.2).

Who wins when data disagree (high -> low):
  1. 사람 결정   - racks drawn on the PLOT, routes dragged, JB moves, tray sizes, support overrides, confirmed answers
  2. 3D 팩트     - piping model racks / cable tier EL / equipment boxes, other-discipline objects
  3. PLOT 자동   - racks, equipment and obstacles read from the PLOT PDF
  4. 체크리스트 기본값
3D never edits a person's decision. A 3D change that moves quantities is first a PROPOSAL:
  preview (numbers + PLOT picture before/after + which person decisions it touches) -> person approves item by item.
On approval the settings change, every approved/kept choice is written to decision_log.json with its source,
and a revision snapshot is taken. Nothing is applied on cancel.
"""
from __future__ import annotations

import json
import time
from pathlib import Path

PRIORITY = ["사람 결정", "3D 팩트", "PLOT 자동 인식", "체크리스트 기본값"]

DECISION_KINDS = {
    "racks_add": "PLOT에 직접 그린 랙", "routes_edit": "끌어서 고친 경로", "jb_moves": "옮긴 JB",
    "tray_size": "바꾼 트레이 크기", "tray_kind": "트렌치 ↔ 트레이 바꾼 구간", "sup_over": "구간별 서포트 형식·간격", "racks_off": "끈 자동 인식 랙",
    "room": "제어실 위치", "irp": "IRP 위치", "m_per_pt": "축척",
}


def person_decisions(dd: dict) -> list:
    out = []
    for k, label in DECISION_KINDS.items():
        v = dd.get(k)
        if not v:
            continue
        if isinstance(v, dict):
            out += [{"kind": k, "key": str(x), "label": label} for x in v]
        elif isinstance(v, list) and k in ("racks_add", "racks_off"):
            out += [{"kind": k, "key": str(i if k == "racks_add" else x), "label": label} for i, x in enumerate(v)]
        else:
            out.append({"kind": k, "key": k, "label": label})
    return out


def _inside(p, rects, pad=0.0):
    return next((r for r in rects if r[0] - pad < p[0] < r[2] + pad and r[1] - pad < p[1] < r[3] + pad), None)


def affected(dd: dict, res_new: dict, lay_new, tr_new=None) -> list:
    """person decisions the proposal touches. Each: {kind, key, label, problem, default ('keep'), choices}."""
    out = []
    geo = (res_new or {}).get("geo") or {}
    jbs = {j["no"]: j for j in (res_new or {}).get("jbs") or []}
    slots = {s[0]: s for s in geo.get("slots") or []}
    eq3d = getattr(lay_new, "eq_model", None) or []
    racks3d = getattr(lay_new, "racks_model", None) or []
    m = getattr(lay_new, "m_per_pt", None) or 1.0
    for no, slot in (dd.get("jb_moves") or {}).items():
        if no not in jbs:
            out.append({"kind": "jb_moves", "key": no, "label": "옮긴 JB", "problem": f"{no}: 새 계산에 이 JB 번호가 없음 (묶음이 바뀜)",
                        "choices": ["내 결정 유지", "3D 따르기 (이동 지움)"]})
        elif slot not in slots:
            out.append({"kind": "jb_moves", "key": no, "label": "옮긴 JB", "problem": f"{no}: 옮긴 자리 {slot}가 새 계산에서 막힘 (3D 장비·랙 안)",
                        "choices": ["내 결정 유지", "3D 따르기 (이동 지움)"]})
    ids = getattr(lay_new, "racks_model_ids", None) or []
    racks3d = [list(r[:4]) + [ids[i] if i < len(ids) else ""] for i, r in enumerate(racks3d)]
    for i, poly in enumerate(dd.get("racks_add") or []):
        mid = poly[len(poly) // 2]
        r = _inside(mid, racks3d, 3.0 / m)
        if r:
            out.append({"kind": "racks_add", "key": str(i), "label": "PLOT에 직접 그린 랙",
                        "problem": f"그린 랙 {i + 1}이 3D 모델 랙 {r[4] if len(r) > 4 else ''}과 겹침 (둘 다 쓰면 이중)",
                        "choices": ["내 결정 유지", "3D 따르기 (그린 랙 지움)"]})
    rids = {r["rid"] for r in geo.get("routes") or []}
    for rid, poly in (dd.get("routes_edit") or {}).items():
        hit = [_inside(p, eq3d) for p in poly]
        hit = [h for h in hit if h]
        if rid not in rids:
            out.append({"kind": "routes_edit", "key": rid, "label": "끌어서 고친 경로", "problem": f"{rid}: 새 계산에 이 경로가 없음",
                        "choices": ["내 결정 유지", "3D 따르기 (수정 지움)"]})
        elif hit:
            out.append({"kind": "routes_edit", "key": rid, "label": "끌어서 고친 경로",
                        "problem": f"{rid}: 고친 경로 꺾임점이 3D 장비 {hit[0][4] if len(hit[0]) > 4 else ''} 안",
                        "choices": ["내 결정 유지", "3D 따르기 (수정 지움)"]})
    if tr_new is not None:
        segs = {str(sg.get("id")) for sg in tr_new.get("segments") or []}
        for k, label in (("tray_size", "바꾼 트레이 크기"), ("tray_kind", "트렌치 ↔ 트레이 바꾼 구간"), ("sup_over", "구간별 서포트 형식·간격")):
            for seg in (dd.get(k) or {}):
                if segs and seg not in segs:
                    out.append({"kind": k, "key": seg, "label": label, "problem": f"{seg}: 새 계산에 이 구간이 없음 (경로가 바뀜)",
                                "choices": ["내 결정 유지", "3D 따르기 (지움)"]})
    return out


def jb_changes(res0: dict, res1: dict, m_per_pt: float | None) -> list:
    a = {j["no"]: j for j in res0.get("jbs") or []}
    b = {j["no"]: j for j in res1.get("jbs") or []}
    pa = {j[0]: (j[1], j[2], j[5] if len(j) > 5 else "") for j in ((res0.get("geo") or {}).get("jbs") or [])}
    pb = {j[0]: (j[1], j[2], j[5] if len(j) > 5 else "") for j in ((res1.get("geo") or {}).get("jbs") or [])}
    out = []
    for no in sorted(set(a) | set(b)):
        if no not in b:
            out.append({"jb": no, "change": "없어짐"})
        elif no not in a:
            out.append({"jb": no, "change": f"새로 생김 ({b[no].get('size', '')})"})
        elif no in pa and no in pb:
            d = ((pa[no][0] - pb[no][0]) ** 2 + (pa[no][1] - pb[no][1]) ** 2) ** 0.5 * (m_per_pt or 1)
            if d > 1.0:
                out.append({"jb": no, "change": f"{round(d, 1)} m 이동: {pa[no][2] or '-'} → {pb[no][2] or '-'}"})
    return out


def preview_png(plot_png: Path, pl_size, out: Path, before, after):
    """PLOT picture (50 dpi) with the current markup in grey and the proposal in colour.
    before / after: (layout, res). Racks = boxes, routes = lines, JBs = squares."""
    from PIL import Image, ImageDraw
    im = Image.open(plot_png).convert("RGB")
    W, H = im.size
    k = W / pl_size[0]
    im = Image.blend(im, Image.new("RGB", im.size, "white"), 0.55)
    d = ImageDraw.Draw(im, "RGBA")

    def draw(lay, res, col, w):
        for r in lay.rack_rects() if lay else []:
            d.rectangle([r[0] * k, r[1] * k, r[2] * k, r[3] * k], outline=col + (200,), width=w)
        geo = (res or {}).get("geo") or {}
        for rt in geo.get("routes") or []:
            pts = [(p[0] * k, p[1] * k) for p in rt["poly"]]
            if len(pts) >= 2:
                d.line(pts, fill=col + (150,), width=w)
        for j in geo.get("jbs") or []:
            x, y = j[1] * k, j[2] * k
            d.rectangle([x - 4, y - 4, x + 4, y + 4], fill=col + (230,))
    draw(*before, (150, 150, 150), 2)
    draw(*after, (230, 0, 60), 2)
    d.rectangle([8, 8, 330, 52], fill=(255, 255, 255, 230))
    d.text((16, 14), "grey = now (current markup)", fill=(110, 110, 110))
    d.text((16, 32), "red  = 3D proposal (not applied yet)", fill=(230, 0, 60))
    out.parent.mkdir(parents=True, exist_ok=True)
    im.save(out)
    return out


def log(pdir: Path, entries: list):
    f = Path(pdir) / "decision_log.json"
    cur = json.loads(f.read_text(encoding="utf-8")) if f.exists() else []
    stamp = time.strftime("%Y-%m-%d %H:%M")
    cur += [{"at": stamp, **e} for e in entries]
    f.write_text(json.dumps(cur, ensure_ascii=False, indent=1), encoding="utf-8")
    return cur


def drop_decision(dd: dict, kind: str, key: str):
    if kind == "racks_add":
        i = int(key)
        dd["racks_add"] = [r for n, r in enumerate(dd.get("racks_add") or []) if n != i]
    elif isinstance(dd.get(kind), dict):
        dd[kind].pop(key, None)
    return dd
