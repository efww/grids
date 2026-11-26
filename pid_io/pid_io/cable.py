"""Index rows -> field cables, junction boxes and JB-to-room multi-pair cables.

Stage 1 of the cable module: no plot plan yet, so every length is an
assumption from the settings (field run, JB-to-room trunk per area, margin).
Same rule as the IO side: the index is the fact, the settings are the
assumptions, and every cable row says which setting produced it.

Settings live per project in cable.json. A lead engineer's set can be saved
as a named preset and loaded into other projects.
"""
from __future__ import annotations

import json
import math
import re
from pathlib import Path

HW_IO = ("AI", "AO", "DI", "DO", "TC/RTD")
SYS_LETTER = {"DCS": "D", "ESD": "E", "F&G": "F"}
CLS_NAME = {"A": "아날로그", "D": "디지털", "O": "디지털(DO)", "R": "RTD", "X": "혼합"}
MULTI_STD = [6, 12, 24, 36, 48]
CORE_STD = [4, 7, 10, 12, 19, 24, 30]
SQ = ["0.75", "1.0", "1.25", "1.5", "2.0", "2.5"]
SHIELD = ["OS (전체 차폐)", "IS+OS (개별+전체 차폐)", "비차폐"]
SHIELD_CODE = {"OS (전체 차폐)": "OS", "IS+OS (개별+전체 차폐)": "IS-OS", "비차폐": ""}


def _sel(id, name, options, default, help=""):
    return {"id": id, "name": name, "type": "select", "options": options, "default": default, "help": help}


def _num(id, name, default, unit="", help="", step=1):
    return {"id": id, "name": name, "type": "number", "default": default, "unit": unit, "help": help, "step": step}


def _txt(id, name, default, help=""):
    return {"id": id, "name": name, "type": "text", "default": default, "help": help}


def _io_items(io, form, sq, shield, forms):
    return [_sel(f"{io}_form", f"{io} 구성", forms, form),
            _sel(f"{io}_sq", f"{io} 굵기 (SQ)", SQ, sq),
            _sel(f"{io}_shield", f"{io} 차폐", SHIELD, shield)]


SCHEMA = [
    {"id": "branch", "title": "IO별 케이블 (계기 → JB)",
     "lead": "계기 하나에서 JB까지 가는 분기 케이블입니다. IO 1점 = 1가닥.",
     "items": (_io_items("AI", "1P", "1.25", "OS (전체 차폐)", ["1P", "1T"])
               + _io_items("AO", "1P", "1.25", "OS (전체 차폐)", ["1P", "1T"])
               + _io_items("DI", "1P", "1.25", "OS (전체 차폐)", ["1P", "2C", "3C"])
               + _io_items("DO", "2C", "2.0", "OS (전체 차폐)", ["1P", "2C", "3C"])
               + _io_items("RTD", "1T", "1.25", "OS (전체 차폐)", ["1T", "2P"])
               + [_txt("COMM_spec", "통신 케이블 사양", "RS-485 1P x 0.75SQ IS-OS",
                       "패키지·MCC 통신 포트에서 룸까지 직접 가는 케이블")])},
    {"id": "jb", "title": "JB · 멀티 케이블 (JB → 룸)",
     "lead": "JB에 모인 페어가 멀티페어 케이블 하나로 랙을 타고 룸까지 갑니다.",
     "items": [
         _sel("jb_size_A", "아날로그 JB 크기", [f"{n}P" for n in MULTI_STD], "24P"),
         _sel("jb_size_D", "디지털 JB 크기", [f"{n}P" for n in MULTI_STD], "24P"),
         _sel("jb_size_R", "RTD JB 크기", [f"{n}T" for n in MULTI_STD], "12T"),
         _num("jb_spare", "JB 스페어율", 0.2, "", "사용 페어 × (1 + 스페어율) ≤ JB 크기. 20%면 24P JB에 20페어까지", 0.05),
         _sel("multi_sizing", "멀티 케이블 크기", ["JB 크기와 같게", "사용+스페어 최소 규격"], "JB 크기와 같게",
              "최소 규격: 사용 페어 × (1+스페어율)을 담는 가장 작은 표준 규격 (6/12/24/36/48)"),
         _sel("multi_sq", "멀티 케이블 굵기 (SQ)", SQ, "1.25"),
         _sel("multi_shield_A", "아날로그 멀티 차폐", SHIELD, "IS+OS (개별+전체 차폐)"),
         _sel("multi_shield_D", "디지털 멀티 차폐", SHIELD, "OS (전체 차폐)"),
     ]},
    {"id": "seg", "title": "JB 분리 기준",
     "lead": "같은 JB에 섞지 않는 기준입니다. 분리할수록 JB 수가 늘고 페어당 채움이 줄어듭니다.",
     "items": [
         _sel("group_by", "JB 묶는 단위", ["유닛(트레인)별", "P&ID 시트별", "Area 매핑표"], "유닛(트레인)별",
              "같은 단위 안에서 도면 순서(시트·위치)대로 채웁니다. Area 매핑표가 없으면 유닛별로 묶습니다"),
         _sel("seg_system", "시스템 분리 (DCS / ESD / F&G)", ["분리", "혼합"], "분리"),
         _sel("seg_ad", "아날로그 / 디지털 분리", ["분리", "혼합"], "분리"),
         _sel("seg_do", "DO 별도 JB", ["DI와 같이", "별도"], "DI와 같이", "솔레노이드 24VDC 전원선을 분리할 때"),
         _sel("rtd_mode", "RTD", ["별도 JB (Triad)", "아날로그 JB에 포함"], "별도 JB (Triad)"),
         _sel("is_rule", "IS / Non-IS", ["전부 Non-IS", "아날로그(AI·AO·RTD)는 IS", "전부 IS"], "전부 Non-IS",
              "인덱스 결정사항 7번이 정해지면 그 기준으로 바꿉니다. IS는 JB와 케이블을 따로 씁니다"),
         _sel("pkg_if", "패키지 인터페이스 신호 (운전·Trip 등)", ["제어반(UCP) → 룸 직결", "Skid JB 경유"],
              "제어반(UCP) → 룸 직결", "체크리스트 패키지 Typical로 생긴 행. 직결이면 패키지·시스템마다 멀티 1가닥"),
         _sel("pkg_route", "패키지(벤더 경계) 안 계기", ["패키지마다 Skid JB", "일반 필드 JB에 포함"],
              "패키지마다 Skid JB", "도면에 그려진 경계 안 계기. 인덱스 결정사항 16번 ★ (경계 안 = Skid)"),
         _sel("mcc_mode", "MCC 신호 (모터 IO)", ["MCC→룸 멀티페어 집합", "모터당 1가닥 멀티코어"],
              "MCC→룸 멀티페어 집합", "MCC 신호는 필드 JB를 거치지 않고 MCC에서 룸으로 바로 갑니다"),
         _sel("mcc_sq", "MCC 케이블 굵기 (SQ)", SQ, "1.25"),
     ]},
    {"id": "len", "title": "길이 가정 (PLOT PLAN 전)",
     "lead": "PLOT PLAN이 없어서 길이는 전부 가정입니다. 유닛별 간선 길이는 아래 표에서 따로 줄 수 있습니다.",
     "items": [
         _num("len_field", "계기 → JB", 30, "m", "분기 케이블 평균 길이"),
         _num("len_trunk", "JB → 룸 (기본)", 200, "m", "랙을 타는 간선. 유닛별 값이 있으면 그 값"),
         _num("len_mcc", "MCC → 룸", 50, "m"),
         _num("len_margin", "길이 여유율", 0.1, "", "단말·처짐·우회", 0.05),
         _num("len_round", "길이 올림 단위", 5, "m"),
         _txt("room_DCS", "DCS 도착 룸", "CCR"),
         _txt("room_ESD", "ESD 도착 룸", "CCR"),
         _txt("room_F&G", "F&G 도착 룸", "CCR"),
     ]},
]
ITEMS = {it["id"]: it for g in SCHEMA for it in g["items"]}
DEFAULTS = {k: v["default"] for k, v in ITEMS.items()}


# ------------------------------------------------------------------ settings
def load(pdir: Path) -> dict:
    f = Path(pdir) / "cable.json"
    s = json.loads(f.read_text(encoding="utf-8")) if f.exists() else {}
    return {"values": {**DEFAULTS, **{k: v for k, v in (s.get("values") or {}).items() if k in ITEMS}},
            "trunk": dict(s.get("trunk") or {}), "preset": s.get("preset", "")}


def save(pdir: Path, s: dict):
    (Path(pdir) / "cable.json").write_text(json.dumps(s, ensure_ascii=False, indent=1), encoding="utf-8")


def set_value(s: dict, k: str, v):
    it = ITEMS.get(k)
    if not it:
        raise ValueError(f"알 수 없는 케이블 설정: {k}")
    if it["type"] == "number":
        v = float(v)
        if v < 0:
            raise ValueError(f"{it['name']}: 0 이상이어야 합니다")
        v = int(v) if v == int(v) and it.get("step", 1) >= 1 else v
    elif it["type"] == "select" and v not in it["options"]:
        raise ValueError(f"{it['name']}: 선택지에 없는 값 '{v}'")
    s["values"][k] = v


def changed(s: dict):
    return [(k, ITEMS[k]["name"], v, ITEMS[k]["default"]) for k, v in s["values"].items() if v != ITEMS[k]["default"]]


def presets_dir(home: Path) -> Path:
    return Path(home) / "_cable_presets"


def list_presets(home: Path):
    d = presets_dir(home)
    return sorted(p.stem for p in d.glob("*.json")) if d.exists() else []


def save_preset(home: Path, name: str, s: dict):
    name = re.sub(r'[\\/:*?"<>|]+', "_", name).strip(" .")[:60]
    if not name:
        raise ValueError("프리셋 이름이 비어 있습니다")
    d = presets_dir(home)
    d.mkdir(parents=True, exist_ok=True)
    (d / f"{name}.json").write_text(json.dumps({"values": s["values"]}, ensure_ascii=False, indent=1),
                                    encoding="utf-8")
    return name


def load_preset(home: Path, name: str, s: dict):
    f = presets_dir(home) / f"{Path(name).name}.json"
    if not f.exists():
        raise ValueError("프리셋을 찾지 못했습니다")
    vals = json.loads(f.read_text(encoding="utf-8")).get("values") or {}
    s["values"] = {**DEFAULTS, **{k: v for k, v in vals.items() if k in ITEMS}}
    s["preset"] = name
    return s


# ------------------------------------------------------------------ helpers
def _n(size: str) -> int:
    return int(re.match(r"\d+", size).group())


def _spec(form: str, sq: str, shield: str) -> str:
    code = SHIELD_CODE.get(shield, "")
    return f"{form} x {sq}SQ" + (f" {code}" if code else "")


def _round_len(m: float, v: dict) -> int:
    step = max(1, int(v["len_round"] or 1))
    return int(math.ceil(m * (1 + float(v["len_margin"])) / step - 1e-9) * step)


def _std(need: int) -> int:
    return next((n for n in MULTI_STD if n >= need), MULTI_STD[-1])


def _cores(need: int) -> int:
    return next((n for n in CORE_STD if n >= need), CORE_STD[-1])


def _xy(r):
    try:
        x, y = (float(t) for t in (r.coord or "").split(","))
        return y, x
    except ValueError:
        return 0.0, 0.0


def classify(r, v: dict):
    """-> (route, signal class) or (None, reason) for rows that get no cable."""
    if r.kind != "instrument":
        return None, "DCS 소프트 태그"
    if r.origin == "스페어":
        return None, "스페어 IO (JB 스페어 페어로 반영)"
    if r.io == "Serial/Comm":
        return "comm", ""
    if r.io not in HW_IO:
        return None, "IO 없음"
    if r.system in ("-", "?", "", None):
        return None, "시스템 미정"
    if r.system == "PKG PLC":
        return None, "벤더 PLC IO (벤더 범위)"
    io = "RTD" if r.io == "TC/RTD" else r.io
    if io == "RTD":
        cls = "R" if v["rtd_mode"].startswith("별도") else "A"
    elif io in ("AI", "AO"):
        cls = "A"
    else:
        cls = "O" if io == "DO" and v["seg_do"] == "별도" else "D"
    if v["seg_ad"] == "혼합" and cls in ("A", "D", "O"):
        cls = "X"
    if (r.location or "") == "MCC":
        return "mcc", cls
    if r.location == "CCR":
        return None, "룸 안 계기 (필드 케이블 없음)"
    if r.symbol_cls == "PACKAGE":                 # package Typical rows (and their train copies)
        return ("ucp" if v["pkg_if"].startswith("제어반") else "pkg"), cls
    if (r.vendor_pkg or r.location == "Skid") and v["pkg_route"].startswith("패키지"):
        return "pkg", cls
    return "field", cls


def _is(io: str, v: dict) -> bool:
    rule = v["is_rule"]
    return rule == "전부 IS" or (rule.startswith("아날로그") and io in ("AI", "AO", "TC/RTD"))


def _area(r, v: dict, area_map: dict) -> str:
    g = v["group_by"]
    if g.startswith("P&ID"):
        return (r.dwg or "NODWG").replace("-", "")
    if g.startswith("Area") and area_map:
        a = area_map.get(r.dwg) or area_map.get(getattr(r, "plant", "") or "")
        if a:
            return str(a)
    return r.unit or "00"


def _trunk(area: str, v: dict, trunk: dict) -> float:
    t = trunk.get(area)
    return float(t) if t not in (None, "") else float(v["len_trunk"])


# ------------------------------------------------------------------ build
def build(rows, s: dict, area_map: dict | None = None):
    v, trunk = s["values"], s.get("trunk") or {}
    area_map = area_map or {}
    spare = float(v["jb_spare"])
    room = lambda sys_: v.get(f"room_{sys_}") or "CCR"
    cables, jbs, skipped = [], [], {}
    used_no = {}

    def uniq(no):
        k = used_no.get(no, 0) + 1
        used_no[no] = k
        return no if k == 1 else f"{no}-{k}"

    groups: dict[tuple, list] = {}
    mcc, comm = [], []
    for r in rows:
        route, cls = classify(r, v)
        if route is None:
            skipped[cls] = skipped.get(cls, 0) + 1
            continue
        if route == "comm":
            comm.append(r)
            continue
        if route == "mcc":
            mcc.append((r, cls))
            continue
        sys_ = r.system if v["seg_system"] == "분리" else "MIX"
        is_ = _is(r.io, v)
        area = r.vendor_pkg if route in ("pkg", "ucp") and r.vendor_pkg else _area(r, v, area_map)
        groups.setdefault((route, area, sys_, cls, is_), []).append(r)

    # ---------------- package control panel straight to the room (no JB)
    for (route, area, sys_, cls, is_), members in sorted(groups.items(), key=lambda kv: [str(x) for x in kv[0]]):
        if route != "ucp":
            continue
        unit = "T" if cls == "R" else "P"
        cap = max(1, int(math.floor(MULTI_STD[-1] / (1 + spare) + 1e-9)))
        for k, i in enumerate(range(0, len(members), cap), 1):
            ch = members[i:i + cap]
            size = _std(int(math.ceil(len(ch) * (1 + spare) - 1e-9)))
            mshield = v["multi_shield_A"] if cls in ("A", "R", "X") else v["multi_shield_D"]
            spec = _spec(f"{size}{unit}", v["multi_sq"], mshield)
            tl = _trunk(ch[0].unit or "00", v, trunk)
            sl = SYS_LETTER.get(sys_, "M")
            cables.append({"no": uniq(f"MC-{area}-{sl}{cls}" + (f"-{k:02d}" if len(members) > cap else "")),
                           "kind": "패키지", "frm": f"{area} UCP", "frm_type": "패키지 제어반",
                           "to": room(sys_) if sys_ != "MIX" else room("DCS"), "to_type": "룸", "spec": spec,
                           "io": CLS_NAME[cls], "system": sys_, "is": "IS" if is_ else "Non-IS",
                           "length": _round_len(tl, v), "area": area, "dwg": ch[0].dwg, "coord": "",
                           "unit": ch[0].unit, "pairs_used": len(ch),
                           "basis": f"패키지 인터페이스 {len(ch)}점 ({', '.join(r.tag for r in ch[:6])}"
                                    f"{' …' if len(ch) > 6 else ''}) · 스페어 {spare:.0%} · 제어반→룸 {tl:g}m 가정"})

    # ---------------- field / package JBs, chunked in drawing order
    for (route, area, sys_, cls, is_), members in sorted(groups.items(), key=lambda kv: [str(x) for x in kv[0]]):
        if route == "ucp":
            continue
        members.sort(key=lambda r: (r.dwg or "", r.page or 0, *_xy(r), r.tag))
        unit = "T" if cls == "R" else "P"
        size = _n(v[f"jb_size_{'R' if cls == 'R' else ('A' if cls in ('A', 'X') else 'D')}"])
        cap = max(1, int(math.floor(size / (1 + spare) + 1e-9)))
        chunks = [members[i:i + cap] for i in range(0, len(members), cap)]
        sl = SYS_LETTER.get(sys_, "M")
        base = f"JB-{area}-{sl}{cls}{'I' if is_ else ''}"
        for k, ch in enumerate(chunks, 1):
            jb = base + (f"-{k:02d}" if route != "pkg" or len(chunks) > 1 else "")
            if v["multi_sizing"].startswith("JB"):
                msize = size
            else:
                msize = _std(int(math.ceil(len(ch) * (1 + spare) - 1e-9)))
            mshield = v["multi_shield_A"] if cls in ("A", "R", "X") else v["multi_shield_D"]
            room_ = room(sys_) if sys_ != "MIX" else room("DCS")
            mc = "MC-" + jb[3:]
            tl = _trunk(area if route != "pkg" else (ch[0].unit or "00"), v, trunk)
            jbs.append({"no": jb, "kind": "패키지 Skid" if route == "pkg" else "필드", "area": area, "system": sys_,
                        "cls": CLS_NAME[cls], "is": "IS" if is_ else "Non-IS", "size": f"{size}{unit}",
                        "used": len(ch), "spare": size - len(ch), "fill": round(len(ch) / size, 3),
                        "multi": mc, "dwgs": ", ".join(sorted({r.dwg for r in ch if r.dwg})),
                        "units": ", ".join(sorted({r.unit for r in ch if r.unit})),
                        "tags": [r.tag for r in ch]})
            for r in ch:
                io = "RTD" if r.io == "TC/RTD" else r.io
                spec = _spec(v[f"{io}_form"], v[f"{io}_sq"], v[f"{io}_shield"])
                cables.append({"no": uniq(f"C-{r.tag}"), "kind": "분기", "frm": r.tag, "frm_type": "계기",
                               "to": jb, "to_type": "JB", "spec": spec, "io": r.io, "system": r.system,
                               "is": "IS" if is_ else "Non-IS", "length": _round_len(float(v["len_field"]), v),
                               "area": area, "dwg": r.dwg, "coord": r.coord, "unit": r.unit,
                               "basis": f"{io} → {spec} (케이블 설정) · 계기→JB {v['len_field']}m 가정"})
            spec = _spec(f"{msize}{unit}", v["multi_sq"], mshield)
            cables.append({"no": uniq(mc), "kind": "멀티", "frm": jb, "frm_type": "JB", "to": room_, "to_type": "룸",
                           "spec": spec, "io": CLS_NAME[cls], "system": sys_, "is": "IS" if is_ else "Non-IS",
                           "length": _round_len(tl, v), "area": area, "dwg": "", "coord": "",
                           "unit": ch[0].unit, "pairs_used": len(ch),
                           "basis": f"사용 {len(ch)}{unit} · 스페어 {spare:.0%} · {v['multi_sizing']} · "
                                    f"JB→룸 {tl:g}m 가정" + (" (유닛별 값)" if area in trunk else "")})

    # ---------------- MCC signals straight to the room
    if mcc:
        if v["mcc_mode"].startswith("MCC"):
            by = {}
            for r, cls in mcc:
                sys_ = r.system if v["seg_system"] == "분리" else "MIX"
                by.setdefault((sys_, "A" if cls in ("A", "R") else "D"), []).append(r)
            for (sys_, cls), members in sorted(by.items()):
                cap = max(1, int(math.floor(MULTI_STD[-1] / (1 + spare) + 1e-9)))
                for k, i in enumerate(range(0, len(members), cap), 1):
                    ch = members[i:i + cap]
                    size = _std(int(math.ceil(len(ch) * (1 + spare) - 1e-9)))
                    spec = _spec(f"{size}P", v["mcc_sq"], v["multi_shield_D"])
                    cables.append({"no": uniq(f"MC-MCC-{SYS_LETTER.get(sys_, 'M')}{cls}-{k:02d}"), "kind": "MCC",
                                   "frm": "MCC", "frm_type": "MCC", "to": room(sys_) if sys_ != "MIX" else room("DCS"),
                                   "to_type": "룸", "spec": spec, "io": CLS_NAME[cls], "system": sys_, "is": "Non-IS",
                                   "length": _round_len(float(v["len_mcc"]), v), "area": "MCC", "dwg": "",
                                   "coord": "", "unit": "", "pairs_used": len(ch),
                                   "basis": f"모터 신호 {len(ch)}점 · 스페어 {spare:.0%} · MCC→룸 {v['len_mcc']}m 가정"})
        else:
            by = {}
            for r, cls in mcc:
                by.setdefault((r.src_tag or r.equip or r.tag, r.system), []).append(r)
            for (motor, sys_), members in sorted(by.items()):
                cores = _cores(int(math.ceil(2 * len(members) * (1 + spare) - 1e-9)))
                spec = _spec(f"{cores}C", v["mcc_sq"], "비차폐")
                cables.append({"no": uniq(f"CC-{motor}-{SYS_LETTER.get(sys_, 'M')}"), "kind": "MCC", "frm": "MCC",
                               "frm_type": "MCC", "to": room(sys_), "to_type": "룸", "spec": spec,
                               "io": "/".join(sorted({r.io for r in members})), "system": sys_, "is": "Non-IS",
                               "length": _round_len(float(v["len_mcc"]), v), "area": "MCC", "dwg": "", "coord": "",
                               "unit": members[0].unit, "pairs_used": len(members),
                               "basis": f"{motor} 신호 {len(members)}점 × 2심 · 스페어 {spare:.0%}"})

    # ---------------- comm ports: home run
    for r in comm:
        src = r.vendor_pkg or r.equip or ("MCC" if r.location == "MCC" else r.tag)
        area = r.unit or "00"
        tl = float(v["len_mcc"]) if r.location == "MCC" else _trunk(area, v, trunk)
        cables.append({"no": uniq(f"CB-{r.tag}"), "kind": "통신", "frm": src,
                       "frm_type": "MCC" if r.location == "MCC" else "패키지", "to": room(r.system if r.system in SYS_LETTER else "DCS"),
                       "to_type": "룸", "spec": v["COMM_spec"], "io": r.io, "system": r.system, "is": "Non-IS",
                       "length": _round_len(tl, v), "area": area, "dwg": r.dwg, "coord": r.coord, "unit": r.unit,
                       "basis": f"통신 포트 직결 · {tl:g}m 가정"})

    order = {"분기": 0, "멀티": 0, "패키지": 1, "MCC": 2, "통신": 3}   # JB branches stay next to their multi
    cables.sort(key=lambda c: order[c["kind"]])
    return {"cables": cables, "jbs": jbs, "skipped": skipped, "summary": summarize(cables, jbs, skipped)}


def summarize(cables, jbs, skipped):
    spec = {}
    for c in cables:
        k = (c["kind"], c["spec"])
        x = spec.setdefault(k, {"kind": c["kind"], "spec": c["spec"], "count": 0, "length": 0})
        x["count"] += 1
        x["length"] += c["length"]
    kinds = {}
    for c in cables:
        x = kinds.setdefault(c["kind"], {"count": 0, "length": 0})
        x["count"] += 1
        x["length"] += c["length"]
    jb_sizes = {}
    for j in jbs:
        jb_sizes[j["size"]] = jb_sizes.get(j["size"], 0) + 1
    fill = sum(j["used"] for j in jbs) / max(1, sum(_n(j["size"]) for j in jbs))
    return {"cables": len(cables), "length": sum(c["length"] for c in cables), "jbs": len(jbs),
            "jb_sizes": dict(sorted(jb_sizes.items(), key=lambda kv: (kv[0][-1], _n(kv[0])))),
            "jb_fill": round(fill, 3), "kinds": kinds,
            "specs": sorted(spec.values(), key=lambda x: (["분기", "멀티", "패키지", "MCC", "통신"].index(x["kind"]), x["spec"])),
            "skipped": skipped}


# ------------------------------------------------------------------ excel
def write_xlsx(res: dict, s: dict, out: Path, project: str = ""):
    from openpyxl import Workbook
    from openpyxl.styles import Alignment, Font, PatternFill
    from openpyxl.utils import get_column_letter

    wb = Workbook()
    head = PatternFill("solid", fgColor="1C5438")
    hfont = Font(bold=True, color="FFFFFF")

    def sheet(ws, cols, data, widths=None):
        ws.append(cols)
        for c in ws[1]:
            c.fill, c.font = head, hfont
            c.alignment = Alignment(vertical="center")
        for row in data:
            ws.append(row)
        ws.freeze_panes = "A2"
        ws.auto_filter.ref = ws.dimensions
        for i, w in enumerate(widths or [], 1):
            ws.column_dimensions[get_column_letter(i)].width = w

    ws = wb.active
    ws.title = "Cable_Summary"
    sm = res["summary"]
    ws.append([f"케이블 물량 (개념설계, PLOT PLAN 전 가정 길이) - {project}"])
    ws["A1"].font = Font(bold=True, size=13)
    ws.append([f"케이블 {sm['cables']}가닥 · 총 {sm['length']:,} m · JB {sm['jbs']}개 · JB 평균 채움 {sm['jb_fill']:.0%}"])
    ws.append([])
    ws.append(["구분", "사양", "가닥", "길이 (m)"])
    for c in ws[4]:
        c.fill, c.font = head, hfont
    for x in sm["specs"]:
        ws.append([x["kind"], x["spec"], x["count"], x["length"]])
    ws.append([])
    ws.append(["JB 크기", "개수"])
    for c in ws[ws.max_row]:
        c.fill, c.font = head, hfont
    for k, n in sm["jb_sizes"].items():
        ws.append([k, n])
    ws.append([])
    ws.append(["케이블 없는 행", "행 수"])
    for c in ws[ws.max_row]:
        c.fill, c.font = head, hfont
    for k, n in sm["skipped"].items():
        ws.append([k, n])
    for col, w in zip("ABCD", (12, 34, 10, 12)):
        ws.column_dimensions[col].width = w

    sheet(wb.create_sheet("Cable_Schedule"),
          ["Cable No", "구분", "From", "From 종류", "To", "To 종류", "Cable Spec", "IO / 신호", "System", "IS",
           "Length (m)", "Unit", "Area/JB 그룹", "P&ID", "도면 좌표", "근거 (가정)"],
          [[c["no"], c["kind"], c["frm"], c["frm_type"], c["to"], c["to_type"], c["spec"], c["io"], c["system"],
            c["is"], c["length"], c.get("unit", ""), c["area"], c["dwg"], c["coord"], c["basis"]] for c in res["cables"]],
          [24, 7, 22, 9, 22, 8, 26, 10, 8, 8, 10, 6, 14, 11, 11, 60])

    sheet(wb.create_sheet("JB_List"),
          ["JB No", "구분", "그룹", "System", "신호", "IS", "크기", "사용", "스페어", "채움률", "멀티 케이블", "Unit",
           "P&ID", "계기 태그"],
          [[j["no"], j["kind"], j["area"], j["system"], j["cls"], j["is"], j["size"], j["used"], j["spare"],
            j["fill"], j["multi"], j["units"], j["dwgs"], ", ".join(j["tags"])] for j in res["jbs"]],
          [20, 10, 12, 8, 10, 8, 7, 6, 7, 8, 20, 8, 24, 80])
    for r in wb["JB_List"].iter_rows(min_row=2, min_col=10, max_col=10):
        r[0].number_format = "0%"

    v = s["values"]
    data = []
    for g in SCHEMA:
        for it in g["items"]:
            val = v[it["id"]]
            data.append([g["title"], it["name"], val, it["default"], "변경" if val != it["default"] else "",
                         it.get("help", "")])
    for a, m in sorted((s.get("trunk") or {}).items()):
        data.append(["길이 가정 (PLOT PLAN 전)", f"JB → 룸 ({a})", m, v["len_trunk"], "유닛별 값", ""])
    sheet(wb.create_sheet("Cable_Settings"), ["구분", "항목", "적용값", "기본값", "", "설명"], data,
          [26, 30, 26, 26, 8, 70])
    out.parent.mkdir(parents=True, exist_ok=True)
    wb.save(out)
    return out
