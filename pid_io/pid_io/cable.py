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


from .schema import _num, _sel, _txt  # noqa: E402


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
               + [_sel("comm_pkg", "패키지 PLC 통신 케이블", ["광케이블 (Fiber Optic)", "RS-485 동선", "Cu Ethernet 동선"],
                       "광케이블 (Fiber Optic)", "벤더PLC+통신 / 혼합 패키지의 통신 포트 → DCS. 포트(이중화 A/B)마다 1가닥"),
                  _sel("comm_redund", "통신 이중화", ["이중화 (포트마다 1가닥)", "단일 (패키지당 1가닥)"],
                       "이중화 (포트마다 1가닥)", "이중화면 A/B 포트마다 1가닥, 단일이면 패키지(유닛)마다 1가닥", on="이중화 (포트마다 1가닥)"),
                  _txt("FO_spec", "광케이블 사양", "FO 12C SM Armoured", "코어 수·모드·외장"),
                  _txt("COMM_spec", "RS-485 통신 케이블 사양", "RS-485 1P x 0.75SQ IS-OS",
                       "패키지를 동선으로 할 때, 그리고 MCC 통신 포트"),
                  _txt("ETH_spec", "Cu Ethernet 통신 케이블 사양", "CAT6A S/FTP 4P x 23AWG Armoured",
                       "체크리스트 C03 = Cu Ethernet일 때 패키지 통신 포트")])},
    {"id": "multi", "title": "멀티 케이블 (JB → 룸)",
     "lead": "JB에 모인 페어가 멀티페어 케이블 하나로 랙을 타고 룸까지 갑니다. 페어 수(크기)와 스페어는 STEP 7 JB에서 정합니다.",
     "items": [
         _sel("multi_sq", "멀티 케이블 굵기 (SQ)", SQ, "1.25"),
         _sel("multi_shield_A", "아날로그 멀티 차폐", SHIELD, "IS+OS (개별+전체 차폐)"),
         _sel("multi_shield_D", "디지털 멀티 차폐", SHIELD, "OS (전체 차폐)"),
     ]},
    {"id": "seg", "title": "분리 기준 (케이블 · JB 공통)",
     "lead": "같은 JB·멀티에 섞지 않는 기준입니다. 여기서 정한 묶음이 STEP 7 JB로 내려갑니다. 분리할수록 JB 수가 늘고 채움이 줄어듭니다.",
     "items": [
         _sel("group_by", "JB 묶는 단위", ["유닛(트레인)별", "P&ID 시트별", "Area 매핑표"], "유닛(트레인)별",
              "같은 단위 안에서 도면 순서(시트·위치)대로 채웁니다. Area 매핑표가 없으면 유닛별로 묶습니다"),
         _sel("seg_system", "시스템 분리 (DCS / ESD / F&G)", ["분리", "혼합"], "분리", "끄면 시스템을 섞어 한 JB에 담습니다", on="분리"),
         _sel("seg_ad", "아날로그 / 디지털 분리", ["분리", "혼합"], "분리", "끄면 AI·AO·DI·DO를 한 JB에 담습니다", on="분리"),
         _sel("seg_do", "DO 별도 JB", ["DI와 같이", "별도"], "DI와 같이", "솔레노이드 24VDC 전원선을 분리할 때", on="별도"),
         _sel("rtd_mode", "RTD 별도 JB (Triad)", ["별도 JB (Triad)", "아날로그 JB에 포함"], "별도 JB (Triad)",
              "끄면 RTD를 아날로그 JB에 넣습니다", on="별도 JB (Triad)"),
         _sel("is_rule", "IS / Non-IS", ["전부 Non-IS", "아날로그(AI·AO·RTD)는 IS", "전부 IS"], "전부 Non-IS",
              "인덱스 결정사항 7번: IS 열은 만들지 않기로 함. 프로젝트에서 IS를 나눌 때만 바꾸세요 (IS는 JB·케이블을 따로 씀)"),
         _sel("pkg_if", "패키지 인터페이스 신호 (운전·Trip 등)", ["제어반(UCP) → 룸 직결", "Skid JB 경유"],
              "제어반(UCP) → 룸 직결", "체크리스트 패키지 Typical로 생긴 행. 직결이면 패키지·시스템마다 멀티 1가닥"),
         _sel("pkg_route", "패키지(벤더 경계) 안 계기", ["패키지마다 Skid JB", "일반 필드 JB에 포함"],
              "패키지마다 Skid JB", "도면에 그려진 경계 안 계기. 인덱스 결정사항 16번 ★ (경계 안 = Skid)"),
         _sel("mcc_from", "MCC 신호 출발점", ["IRP (인터포징 릴레이 판넬)", "MCC 직접"], "IRP (인터포징 릴레이 판넬)",
              "MCC↔IRP는 전기실 안 배선(전기 범위)이라 집계하지 않고, IRP → DCS/ESD 캐비닛 케이블만 셉니다"),
         _sel("mcc_mode", "MCC 신호 케이블", ["멀티페어 집합", "모터당 1가닥 멀티코어"],
              "멀티페어 집합", "MCC 신호는 필드 JB를 거치지 않고 전기실에서 룸으로 바로 갑니다"),
         _sel("mcc_sq", "MCC 케이블 굵기 (SQ)", SQ, "1.25"),
     ]},
    {"id": "route", "title": "경로 · 높이 (Z)",
     "lead": "PLOT PLAN이 있으면 수평 길이는 도면 좌표(직각 경로)로, 높이는 아래 값으로 더합니다. 없으면 다음 탭의 가정 길이를 씁니다.",
     "items": [
         _sel("plot_use", "PLOT PLAN 좌표로 길이 계산", ["사용", "가정 길이만"], "사용",
              "PLOT PLAN을 올리고 축척·제어실 위치가 잡히면 적용됩니다", on="사용"),
         _sel("main_route", "간선 (JB → 룸)", ["랙 (AG)", "UG 트렌치", "AG 서포트"], "랙 (AG)"),
         _num("rack_el", "랙 최상단 높이 (트레이 EL)", 6.0, "m", "케이블 트레이는 파이프랙 최상단을 탑니다. 그 단의 높이", 0.5),
         _sel("rack_src", "랙 출처", ["PLOT 인식", "3D 모델 랙", "둘 다"], "PLOT 인식",
              "3D 모델(STEP 10에 올린 배관 Three.js)이 있으면 그 랙 위치·길이를 PLOT 좌표로 옮겨 경로에 씁니다. PLOT 인식은 일부 랙만 잡힘"),
         _sel("rack_el_src", "랙 트레이 EL 출처", ["체크리스트 값", "3D 모델 케이블 단"], "체크리스트 값",
              "3D 모델 케이블 단: 모델의 랙별 케이블 단 EL(RK.cable)을 위 '랙 최상단 높이' 대신 씀 (랙이 여럿이면 가장 긴 랙 값)"),
         _sel("no_rack", "랙 없는 구간 (계기 → JB)", ["AG 서포트", "UG 트렌치"], "AG 서포트",
              "분기 케이블. 수소 등 가스 체류 우려가 있으면 보통 AG"),
         _sel("trunk_off", "랙 없는 구간 (간선: JB·패키지·IRP → 룸)", ["UG 트렌치", "AG 서포트"], "UG 트렌치",
              "랙을 타지 않는 간선 구간. UG면 STEP 8에 UG(트렌치) 물량으로, AG면 사다리 트레이로 잡힙니다"),
         _num("sup_el", "AG 서포트 높이 (EL)", 3.0, "m", "", 0.5),
         _num("ug_depth", "UG 매설 깊이", 0.8, "m", "", 0.1),
         _num("inst_el", "계기 높이 (EL)", 1.5, "m", "", 0.1),
         _num("jb_el", "JB 높이 (EL)", 1.2, "m", "", 0.1),
         _num("room_el", "룸 케이블 인입 높이 (EL)", 0.0, "m", "케이블 피트·바닥이면 0", 0.1),
         _num("rack_pref", "랙 우선도", 3, "", "랙 밖 1m를 랙 몇 m로 볼지. 클수록 돌아가더라도 랙을 탑니다", 0.5),
         _num("rack_width", "직접 그린 랙 폭", 6, "m", "PLOT 화면에서 다각선으로 그린 랙의 폭"),
         _num("route_factor", "분기 우회계수", 1.1, "", "계기 → JB 직각 거리 × 계수 (짧은 구간이라 경로 탐색 생략)", 0.05),
         _num("field_min", "계기 → JB 최소 수평 길이", 5, "m", "같은 장비 옆이라 도면 거리가 0에 가까울 때"),
     ]},
    {"id": "len", "title": "길이 가정",
     "lead": "PLOT PLAN이 없거나 위치를 못 찾은 케이블에 쓰는 가정 길이입니다. 유닛별 간선 길이는 아래 표에서 따로 줄 수 있습니다.",
     "items": [
         _num("len_field", "계기 → JB", 30, "m", "분기 케이블 평균 길이"),
         _num("len_trunk", "JB → 룸 (기본)", 200, "m", "랙을 타는 간선. 유닛별 값이 있으면 그 값"),
         _num("len_mcc", "전기실(IRP/MCC) → 룸", 50, "m", "PLOT PLAN이 들어오면 전기실과 룸 위치로 계산"),
         _num("len_margin", "길이 여유율", 0.1, "", "단말·처짐·우회", 0.05, True),
         _num("len_round", "길이 올림 단위", 5, "m"),
         _txt("room_DCS", "DCS 도착 룸", "CCR"),
         _txt("room_ESD", "ESD 도착 룸", "CCR"),
         _txt("room_F&G", "F&G 도착 룸", "CCR"),
     ]},
]
from . import jb as JB  # noqa: E402  (STEP 7 settings live in the same cable.json)
from . import tray as TR  # noqa: E402  (STEP 8 too)
from . import support as SP  # noqa: E402  (and STEP 9)
from . import twin as TW  # noqa: E402  (and STEP 10 lanes / clash)

# v13.3 (사용자 결정 2026-11-27 '합쳐'): one value per decision. These STEP 6 items follow the concept checklist
# (C03 통신 매체, C04 통신 이중화, H06 IS / Non-IS) - the checklist is the single place to change them.
CONCEPT_LINK = {
    "comm_pkg": ("C03", {"광케이블": "광케이블 (Fiber Optic)", "RS-485": "RS-485 동선", "Cu Ethernet": "Cu Ethernet 동선"}),
    "comm_redund": ("C04", {"ON": "이중화 (포트마다 1가닥)", "OFF": "단일 (패키지당 1가닥)"}),
    "is_rule": ("H06", {"전부 Non-IS": "전부 Non-IS", "아날로그(AI·AO·RTD)는 IS": "아날로그(AI·AO·RTD)는 IS",
                        "전부 IS": "전부 IS",
                        # no hazardous-area drawing yet -> counted as 전부 Non-IS (index decision 7), marked 가정
                        "위험지역 계기 IS": "전부 Non-IS"}),
}
for _g in SCHEMA:
    for _it in _g["items"]:
        if _it["id"] in CONCEPT_LINK:
            _it["from_concept"] = CONCEPT_LINK[_it["id"]][0]


def apply_concept(values: dict, concept) -> list:
    """overwrite the linked STEP 6 values with the checklist (C03 / C04 / H06). Returns notes for the screen."""
    notes = []
    if concept is None:
        return notes
    for k, (iid, mp) in CONCEPT_LINK.items():
        raw = concept.get(iid)
        if raw is None:
            continue
        raw = str(raw).strip()
        if raw.upper() in ("ON", "OFF"):
            raw = raw.upper()
        val = mp.get(raw)
        if val is None:
            notes.append(f"체크리스트 {iid} '{raw}'는 케이블 선택지에 없어 '{values.get(k)}' 유지")
            continue
        values[k] = val
        if iid == "H06" and raw == "위험지역 계기 IS":
            notes.append("H06 '위험지역 계기 IS': 위험지역 구분도가 없어 전부 Non-IS로 계산 (가정)")
    return notes


ITEMS = {it["id"]: it for g in SCHEMA + JB.SCHEMA + TR.SCHEMA + SP.SCHEMA + TW.SCHEMA for it in g["items"]}
DEFAULTS = {k: v["default"] for k, v in ITEMS.items()}


# ------------------------------------------------------------------ settings
def load(pdir: Path) -> dict:
    f = Path(pdir) / "cable.json"
    s = json.loads(f.read_text(encoding="utf-8")) if f.exists() else {}
    ok = lambda k, v: k in ITEMS and (ITEMS[k]["type"] != "select" or v in ITEMS[k]["options"])   # renamed options -> default
    return {"values": {**DEFAULTS, **{k: v for k, v in (s.get("values") or {}).items() if ok(k, v)}},
            "trunk": dict(s.get("trunk") or {}), "preset": s.get("preset", "")}


def save(pdir: Path, s: dict):
    (Path(pdir) / "cable.json").write_text(json.dumps(s, ensure_ascii=False, indent=1), encoding="utf-8")


def set_value(s: dict, k: str, v):
    it = ITEMS.get(k)
    if not it:
        raise ValueError(f"알 수 없는 케이블 설정: {k}")
    if it.get("from_concept"):
        raise ValueError(f"{it['name']}: 체크리스트 {it['from_concept']}에서 정합니다 (STEP 4 개념 체크리스트에서 바꿔 주세요)")
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
    s["values"] = {**DEFAULTS, **{k: v for k, v in vals.items()
                                  if k in ITEMS and (ITEMS[k]["type"] != "select" or v in ITEMS[k]["options"])}}
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
    return max(step, int(math.ceil(m * (1 + float(v["len_margin"])) / step - 1e-9) * step))   # no 0 m cable


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
class _Geo:
    """Lengths from the plot plan: horizontal = orthogonal distance x detour factor, vertical from the Z settings."""

    def __init__(self, layout, v):
        self.L, self.v = layout, v
        self.cache = {}
        self.seen = set()
        self.geo = {"inst": {}, "jbs": [], "pkgs": {}, "routes": [], "room": layout.room, "irp": layout.irp,
                    "unlocated": 0, "guessed": 0}

    def loc(self, r):
        if r.tag not in self.cache:
            p, src = self.L.locate(r)
            self.cache[r.tag] = (p, src)
            if p is None:
                self.geo["unlocated"] += 1
            else:
                if src.startswith("추정"):
                    self.geo["guessed"] += 1
                self.geo["inst"][r.tag] = [round(p[0], 1), round(p[1], 1), src]
        return self.cache[r.tag]

    def zf(self, start, end):
        """vertical metres for a run whose ends sit at `start` / `end` EL, routed per main_route."""
        v = self.v
        mode = v["main_route"]
        if mode.startswith("랙"):
            lvl = float(v["rack_el"])
        elif mode.startswith("UG"):
            lvl = -float(v["ug_depth"])
        else:
            lvl = float(v["sup_el"])
        return abs(start - lvl) + abs(lvl - end)

    def run(self, p, start_el, end_el, kind, rid=""):
        """Routed run to the control room: -> (metres incl. height, basis text).
        rid names the route (JB / package / IRP / comm tag) so a hand-edited polyline can replace it."""
        v = self.v
        mode = v["main_route"]
        use_r = mode.startswith("랙")
        edited = rid in self.L.route_edit
        if edited:
            r = self.L.measure(self.L.route_edit[rid], float(v["rack_width"]), use_r)
        else:
            r = self.L.router(float(v["rack_pref"]), float(v["rack_width"]), use_r).route(p)
        if not r:
            hh, zz = self.h(p, self.L.room), self.zf(start_el, end_el)
            return hh + zz, f"PLOT 직각 {hh:.0f}m (경로 못 찾음) + 높이 {zz:.1f}m"
        if mode.startswith("UG") or (use_r and v["trunk_off"].startswith("UG")):
            gl = -float(v["ug_depth"])
        else:
            gl = float(v["sup_el"])
        rl = float(v["rack_el"])
        first = rl if (use_r and r["first_rack"]) else gl
        last = rl if (use_r and r["last_rack"]) else gl
        z = abs(start_el - first) + (r["changes"] * abs(rl - gl) if use_r else 0) + abs(last - end_el)
        horiz = r["ground"] + r["rack"]
        if rid not in self.seen:
            self.seen.add(rid)
            self.geo["routes"].append({"rid": rid, "poly": r["poly"], "kind": kind, "len": round(horiz + z),
                                       "rack": round(r["rack"]), "edited": edited, "cables": 0})
        for x in self.geo["routes"]:
            if x["rid"] == rid:
                x["cables"] += 1
        lvl = "UG" if gl < 0 else "AG 서포트"
        tag = "사용자 수정 경로" if edited else "PLOT 경로"
        return horiz + z, (f"{tag} {horiz:.0f}m (랙 {r['rack']:.0f} · {lvl} {r['ground']:.0f}) + 높이 {z:.1f}m"
                           if use_r else f"{tag} {horiz:.0f}m ({mode}) + 높이 {z:.1f}m")

    def zb(self):
        v = self.v
        lvl = -float(v["ug_depth"]) if v["no_rack"].startswith("UG") else float(v["sup_el"])
        return abs(float(v["inst_el"]) - lvl) + abs(lvl - float(v["jb_el"]))

    def h(self, a, b):
        return self.L.dist(a, b) * float(self.v["route_factor"])


def _spatial(members, cap, G):
    """Greedy clusters on the plot: seed = west-/north-most free instrument, then its nearest
    neighbours within the JB radius until the JB is full. Unlocated rows keep drawing order."""
    rad = float(G.v["jb_radius"])
    located = [(r, G.loc(r)[0]) for r in members]
    free = [x for x in located if x[1]]
    out = []
    while free:
        free.sort(key=lambda x: (x[1][0] + x[1][1]))
        seed = free[0][1]
        near = sorted(free, key=lambda x: G.L.dist(seed, x[1]))
        ch = [x for x in near if G.L.dist(seed, x[1]) <= rad][:cap] or near[:1]
        ids = {id(x[0]) for x in ch}
        free = [x for x in free if id(x[0]) not in ids]
        out.append([x[0] for x in ch])
    lost = [r for r, p in located if not p]
    out += [lost[i:i + cap] for i in range(0, len(lost), cap)]
    return out


def _centre(pts):
    pts = [p for p in pts if p]
    if not pts:
        return None
    return (sum(p[0] for p in pts) / len(pts), sum(p[1] for p in pts) / len(pts))


def build(rows, s: dict, area_map: dict | None = None, layout=None):
    v, trunk = s["values"], s.get("trunk") or {}
    G = _Geo(layout, v) if (layout is not None and v.get("plot_use") == "사용" and layout.m_per_pt
                            and layout.room) else None
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
            how = f"제어반→룸 {tl:g}m 가정"
            if G:
                p = _centre([G.loc(r)[0] for r in ch])
                if p:
                    tl, how = G.run(p, float(v["jb_el"]), float(v["room_el"]), "pkg", f"PKG {area}")
                    how += f" ({G.loc(ch[0])[1]})"
                    G.geo["pkgs"][area] = [round(p[0], 1), round(p[1], 1)]
            sl = SYS_LETTER.get(sys_, "M")
            cables.append({"no": uniq(f"MC-{area}-{sl}{cls}" + (f"-{k:02d}" if len(members) > cap else "")),
                           "kind": "패키지", "frm": f"{area} UCP", "frm_type": "패키지 제어반",
                           "to": room(sys_) if sys_ != "MIX" else room("DCS"), "to_type": "룸", "spec": spec,
                           "io": CLS_NAME[cls], "system": sys_, "is": "IS" if is_ else "Non-IS",
                           "length": _round_len(tl, v), "area": area, "dwg": ch[0].dwg, "coord": "",
                           "unit": ch[0].unit, "pairs_used": len(ch), "tags": [r.tag for r in ch],
                           "basis": f"패키지 인터페이스 {len(ch)}점 ({', '.join(r.tag for r in ch[:6])}"
                                    f"{' …' if len(ch) > 6 else ''}) · 스페어 {spare:.0%} · {how}"})

    # ---------------- field / package JBs: group (STEP 6 segregation) -> cluster -> place (STEP 7) -> cables
    cap = JB.capacity(v)
    boxes = []
    for (route, area, sys_, cls, is_), members in sorted(groups.items(), key=lambda kv: [str(x) for x in kv[0]]):
        if route == "ucp":
            continue
        members.sort(key=lambda r: (r.dwg or "", r.page or 0, *_xy(r), r.tag))
        chunks = _spatial(members, cap, G) if G else [members[i:i + cap] for i in range(0, len(members), cap)]
        for ch in chunks:
            boxes.append({"key": (route, area, sys_, cls, is_), "pkg": route == "pkg", "rows": list(ch),
                          "pts": [G.loc(r)[0] for r in ch] if G else [None] * len(ch)})
    slots = JB.place(boxes, G, v) if G else []
    moves = (getattr(layout, "jb_moves", None) or {}) if G else {}
    slot_by = {x["id"]: x for x in slots}
    # numbering per group: nearest the control room first (plot), else drawing order
    if G:
        R = layout.router(float(v["rack_pref"]), float(v["rack_width"]), v["main_route"].startswith("랙"))
        far = lambda b: R.dist[R.free(b["pos"])] if b.get("pos") else float("inf")
    else:
        far = lambda b: 0
    by_key = {}
    for bx in boxes:
        by_key.setdefault(bx["key"], []).append(bx)
    for key, bl in sorted(by_key.items(), key=lambda kv: [str(x) for x in kv[0]]):
        route, area, sys_, cls, is_ = key
        bl.sort(key=far)
        unit = "T" if cls == "R" else "P"
        sl = SYS_LETTER.get(sys_, "M")
        base = f"JB-{area}-{sl}{cls}{'I' if is_ else ''}"
        for k, bx in enumerate(bl, 1):
            ch = bx["rows"]
            jb = base + (f"-{k:02d}" if route != "pkg" or len(bl) > 1 else "")
            size, msize = JB.size_for(len(ch), v)
            moved = False
            if jb in moves and moves[jb] in slot_by:
                s_ = slot_by[moves[jb]]
                bx["pos"], bx["slot"], bx["slot_label"] = (s_["x"], s_["y"]), s_["id"], s_["label"]
                bx["how"], moved = f"사용자 이동: {s_['label']}", True
            mshield = v["multi_shield_A"] if cls in ("A", "R", "X") else v["multi_shield_D"]
            room_ = room(sys_) if sys_ != "MIX" else room("DCS")
            mc = "MC-" + jb[3:]
            tl = _trunk(area if route != "pkg" else (ch[0].unit or "00"), v, trunk)
            how = f"JB→룸 {tl:g}m 가정" + (" (유닛별 값)" if area in trunk else "")
            pj = bx.get("pos") if G else None
            if pj:
                tl, how = G.run(pj, float(v["jb_el"]), float(v["room_el"]), "jb", jb)
                how += f" (JB {bx.get('how') or ''})"
                G.geo["jbs"].append([jb, round(pj[0], 1), round(pj[1], 1), sys_, bx.get("slot", ""),
                                     bx.get("slot_label", ""), moved, CLS_NAME[cls], len(ch), f"{size}{unit}"])
            jrow = {"no": jb, "kind": "패키지 Skid" if route == "pkg" else "필드", "area": area, "system": sys_,
                    "cls": CLS_NAME[cls], "is": "IS" if is_ else "Non-IS", "size": f"{size}{unit}",
                    "used": len(ch), "spare": size - len(ch), "fill": round(len(ch) / size, 3),
                    "multi": mc, "multi_size": f"{msize}{unit}", "dwgs": ", ".join(sorted({r.dwg for r in ch if r.dwg})),
                    "units": ", ".join(sorted({r.unit for r in ch if r.unit})),
                    "tags": [r.tag for r in ch], "slot": bx.get("slot", ""), "place": bx.get("slot_label", "") or
                    ("계기 중심" if pj else "PLOT 없음"), "moved": moved,
                    "x_m": round(pj[0] * layout.m_per_pt, 1) if pj else None,
                    "y_m": round(pj[1] * layout.m_per_pt, 1) if pj else None}
            jrow.update(JB.spec(jrow, v))
            jbs.append(jrow)
            for r in ch:
                io = "RTD" if r.io == "TC/RTD" else r.io
                spec = _spec(v[f"{io}_form"], v[f"{io}_sq"], v[f"{io}_shield"])
                fl, how_f = float(v["len_field"]), f"계기→JB {v['len_field']}m 가정"
                pi = G.loc(r) if pj else (None, "")
                if pi[0]:
                    hh = max(G.h(pi[0], pj), float(v["field_min"]))
                    zz = G.zb()
                    fl, how_f = hh + zz, f"PLOT 수평 {hh:.0f}m + 높이 {zz:.1f}m (계기 위치: {pi[1]})"
                cables.append({"no": uniq(f"C-{r.tag}"), "kind": "분기", "frm": r.tag, "frm_type": "계기",
                               "to": jb, "to_type": "JB", "spec": spec, "io": r.io, "system": r.system,
                               "is": "IS" if is_ else "Non-IS", "length": _round_len(fl, v),
                               "area": area, "dwg": r.dwg, "coord": r.coord, "unit": r.unit,
                               "basis": f"{io} → {spec} (케이블 설정) · {how_f}"})
            spec = _spec(f"{msize}{unit}", v["multi_sq"], mshield)
            cables.append({"no": uniq(mc), "kind": "멀티", "frm": jb, "frm_type": "JB", "to": room_, "to_type": "룸",
                           "spec": spec, "io": CLS_NAME[cls], "system": sys_, "is": "IS" if is_ else "Non-IS",
                           "length": _round_len(tl, v), "area": area, "dwg": "", "coord": "",
                           "unit": ch[0].unit, "pairs_used": len(ch),
                           "basis": f"사용 {len(ch)}{unit} · 케이블 스페어 {spare:.0%} · {v['multi_sizing']} · {how}"})
    if G:
        G.geo["slots"] = [[s_["id"], s_["x"], s_["y"], s_["kind"], s_["label"]] for s_ in slots]
        G.geo["stations"] = JB.stations([{"no": j[0], "x": j[1], "y": j[2], "slot": j[4], "slot_label": j[5]}
                                         for j in G.geo["jbs"]])

    # ---------------- MCC signals straight to the room
    if mcc:
        src = "IRP" if v["mcc_from"].startswith("IRP") else "MCC"
        mlen, mhow = float(v["len_mcc"]), f"{src}→룸 {v['len_mcc']}m 가정"
        if G and G.L.irp:
            mlen, mhow = G.run(G.L.irp, float(v["room_el"]), float(v["room_el"]), "irp", src)
            mhow += f" ({src} 위치: {G.L.irp_src})"
        if v["mcc_mode"].startswith("멀티"):
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
                    cables.append({"no": uniq(f"MC-{src}-{SYS_LETTER.get(sys_, 'M')}{cls}-{k:02d}"), "kind": "MCC",
                                   "frm": src, "frm_type": src, "to": room(sys_) if sys_ != "MIX" else room("DCS"),
                                   "to_type": "룸", "spec": spec, "io": CLS_NAME[cls], "system": sys_, "is": "Non-IS",
                                   "length": _round_len(mlen, v), "area": "MCC", "dwg": "",
                                   "coord": "", "unit": "", "pairs_used": len(ch), "tags": [r.tag for r in ch],
                                   "basis": f"모터 신호 {len(ch)}점 · 스페어 {spare:.0%} · {mhow}"
                                            + (" · MCC↔IRP는 전기 범위 (미집계)" if src == "IRP" else "")})
        else:
            by = {}
            for r, cls in mcc:
                by.setdefault((r.src_tag or r.equip or r.tag, r.system), []).append(r)
            for (motor, sys_), members in sorted(by.items()):
                cores = _cores(int(math.ceil(2 * len(members) * (1 + spare) - 1e-9)))
                spec = _spec(f"{cores}C", v["mcc_sq"], "비차폐")
                cables.append({"no": uniq(f"CC-{motor}-{SYS_LETTER.get(sys_, 'M')}"), "kind": "MCC", "frm": src,
                               "frm_type": src, "to": room(sys_), "to_type": "룸", "spec": spec,
                               "io": "/".join(sorted({r.io for r in members})), "system": sys_, "is": "Non-IS",
                               "length": _round_len(mlen, v), "area": "MCC", "dwg": "", "coord": "",
                               "unit": members[0].unit, "pairs_used": len(members), "tags": [r.tag for r in members],
                               "basis": f"{motor} 신호 {len(members)}점 × 2심 · 스페어 {spare:.0%} · {mhow}"})

    # ---------------- comm ports: home run (one per port when redundant, else one per package / MCC)
    share = {}                                       # single: one cable per package carries all its port rows
    if not v["comm_redund"].startswith("이중"):
        one = {}
        for r in comm:
            k = (r.vendor_pkg or r.equip or ("MCC" if r.location == "MCC" else r.tag), r.system)
            one.setdefault(k, r)
            share.setdefault(id(one[k]), []).append(r.tag)
        comm = list(one.values())
    for r in comm:
        src = r.vendor_pkg or r.equip or ("MCC" if r.location == "MCC" else r.tag)
        area = r.unit or "00"
        tl = float(v["len_mcc"]) if r.location == "MCC" else _trunk(area, v, trunk)
        how = f"{tl:g}m 가정"
        if G:
            p = G.L.irp if r.location == "MCC" else G.loc(r)[0]
            if p:
                tl, how = G.run(p, float(v["jb_el"]), float(v["room_el"]), "comm", f"COMM {src}")
        fo = r.location != "MCC" and v["comm_pkg"].startswith("광")
        eth = r.location != "MCC" and v["comm_pkg"].startswith("Cu")
        cables.append({"no": uniq(f"{'FO' if fo else 'CB'}-{r.tag}"), "kind": "통신", "frm": src,
                       "frm_type": "MCC" if r.location == "MCC" else "패키지", "to": room(r.system if r.system in SYS_LETTER else "DCS"),
                       "to_type": "룸", "spec": v["FO_spec"] if fo else v["ETH_spec"] if eth else v["COMM_spec"], "io": r.io, "system": r.system, "is": "Non-IS",
                       "length": _round_len(tl, v), "area": area, "dwg": r.dwg, "coord": r.coord, "unit": r.unit,
                       "tags": share.get(id(r), [r.tag]),
                       "basis": f"{'패키지 PLC 통신 (광)' if fo else '패키지 PLC 통신 (Cu Ethernet)' if eth else '통신 포트 직결'} · {v['comm_redund'].split(' ')[0]} · {how}"})

    order = {"분기": 0, "멀티": 0, "패키지": 1, "MCC": 2, "통신": 3}   # JB branches stay next to their multi
    cables.sort(key=lambda c: order[c["kind"]])
    res = {"cables": cables, "jbs": jbs, "skipped": skipped, "summary": summarize(cables, jbs, skipped)}
    res["summary"]["by_plot"] = sum(1 for c in cables if "PLOT" in c["basis"] or "사용자 수정 경로" in c["basis"])
    res["summary"]["edited_routes"] = len(layout.route_edit) if (G and layout is not None) else 0
    res["geo"] = G.geo if G else None
    res["summary"]["stations"] = len(G.geo.get("stations") or []) if G else 0
    res["summary"]["moved"] = sum(1 for j in jbs if j.get("moved"))
    return res


INDEX_COLS = [  # (column, header, key, width) - index columns after the template (decision 8: appended)
    (34, "Junction Box", "jb", 16), (35, "Multi Cable Number", "multi", 18), (36, "Multi Cable Spec.", "multi_spec", 24),
    (37, "Field Cable No", "cable", 18), (38, "Field Cable Spec.", "cable_spec", 20),
    (39, "Field Cable Length (m)", "cable_len", 10), (40, "Multi Cable Length (m)", "multi_len", 10),
    (41, "Perforated Tray", "pt", 16), (42, "Perforated Bracket (개)", "pt_br", 10),
    (43, "Ladder Tray (멀티 경로)", "ladder", 40), (44, "UG Trench (멀티 경로)", "trench", 30),
    (45, "Ladder m / UG m", "lad_ug", 12), (46, "케이블 산출 근거", "basis", 70),
]


def index_map(res, tray=None, sup=None):
    """tag -> index columns 34-46: JB, multi, field cable, lengths, perforated tray + brackets,
    the ladder trays / trench runs its multi rides (in order JB -> room) and the cable basis.
    Field instruments: their JB and its multi. Package interface / MCC rows: no JB, their multi.
    Comm ports: their home-run cable (and its trays)."""
    multi = {c["frm"]: c for c in res["cables"] if c["kind"] == "멀티"}
    by_cable = (tray or {}).get("by_cable") or {}
    pt = {p["cable"]: p for p in (tray or {}).get("pt") or []}
    br = {p["pt"]: p["count"] for p in (sup or {}).get("pt") or []}

    def route(no):
        bc = by_cable.get(no)
        if not bc:
            return {"ladder": "", "trench": "", "lad_ug": ""}
        lad = [f"{x['no']} ({x['size']}) {x['m']}m" for x in bc["legs"] if x["kind"] == "사다리"]
        ug = [f"{x['no']} {x['m']}m" for x in bc["legs"] if x["kind"] == "UG"]
        return {"ladder": " → ".join(lad), "trench": " → ".join(ug), "lad_ug": f"{bc['ladder_m']} / {bc['ug_m']}"}
    out = {}
    for c in res["cables"]:
        if c["kind"] == "분기":
            m = multi.get(c["to"]) or {}
            p = pt.get(c["no"]) or {}
            out.setdefault(c["frm"], {"jb": c["to"], "multi": m.get("no", ""), "multi_spec": m.get("spec", ""),
                                      "cable": c["no"], "cable_spec": c["spec"], "cable_len": c["length"],
                                      "multi_len": m.get("length", ""), "pt": f"{p['no']} ({p['width']})" if p else "",
                                      "pt_br": br.get(p.get("no"), "") if p else "", **route(m.get("no", "")),
                                      "basis": f"[분기] {c['basis']}" + (f" / [멀티] {m['basis']}" if m else "")})
        elif c["kind"] in ("패키지", "MCC"):
            for t in c.get("tags") or []:
                out.setdefault(t, {"jb": "-", "multi": c["no"], "multi_spec": c["spec"], "cable": "", "cable_spec": "",
                                   "cable_len": "", "multi_len": c["length"], "pt": "", "pt_br": "", **route(c["no"]),
                                   "basis": f"[{c['kind']}] {c['basis']}"})
        elif c["kind"] == "통신":
            for t in c.get("tags") or []:
                out.setdefault(t, {"jb": "-", "multi": "", "multi_spec": "", "cable": c["no"], "cable_spec": c["spec"],
                                   "cable_len": c["length"], "multi_len": "", "pt": "", "pt_br": "", **route(c["no"]),
                                   "basis": f"[통신] {c['basis']}"})
    return out


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
def write_xlsx(res: dict, s: dict, out: Path, project: str = "", tray: dict | None = None, sup: dict | None = None):
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
          ["JB No", "구분", "그룹", "System", "신호", "IS", "크기", "사용 (1P)", "스페어", "채움률", "멀티 케이블", "멀티 크기",
           "재질", "방폭", "보호등급", "단자 수", "분기 글랜드", "멀티 글랜드", "인입", "위치 (자리)", "X (m)", "Y (m)",
           "Unit", "P&ID", "계기 태그"],
          [[j["no"], j["kind"], j["area"], j["system"], j["cls"], j["is"], j["size"], j["used"], j["spare"],
            j["fill"], j["multi"], j.get("multi_size", ""), j.get("material", ""), j.get("ex", ""), j.get("ip", ""),
            j.get("terminals", ""), j.get("gl_branch", ""), j.get("gl_multi", ""), j.get("entry", ""),
            (j.get("place", "") + (" (사용자 이동)" if j.get("moved") else "")), j.get("x_m"), j.get("y_m"),
            j["units"], j["dwgs"], ", ".join(j["tags"])] for j in res["jbs"]],
          [20, 10, 12, 8, 10, 8, 7, 8, 7, 8, 20, 8, 9, 16, 8, 7, 8, 8, 6, 22, 8, 8, 8, 24, 80])
    for r in wb["JB_List"].iter_rows(min_row=2, min_col=10, max_col=10):
        r[0].number_format = "0%"
    jsum = {}
    for j in res["jbs"]:
        k = (j["size"], j.get("material", ""), j.get("ex", ""), j.get("ip", ""))
        jsum[k] = jsum.get(k, 0) + 1
    sheet(wb.create_sheet("JB_Summary"), ["크기", "재질", "방폭", "보호등급", "개수"],
          [[*k, n] for k, n in sorted(jsum.items())] + [[], ["크기표 (현재 스페어 기준)"],
           ["JB 크기", "받는 1P 최대", "멀티", "케이블 스페어 (P)", "단자 스페어 (P)", "단자 수"]]
          + [[x["size"], x["max_in"], x["multi"], x["spare_cable"], x["spare_term"], x["terms"]]
             for x in JB.size_table(s["values"])], [10, 14, 22, 18, 16, 10])
    sheet(wb.create_sheet("JB_Checklist"), ["No", "구분", "항목", "적용값", "기본값", "", "설명"],
          [[c["code"], c["group"], c["name"], c["value"], c["default"], "변경" if c["changed"] else "", c["help"]]
           for c in JB.checklist(s["values"])], [6, 14, 30, 22, 22, 6, 80])

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
    if tray is not None:
        TR.write_sheets(wb, tray, s["values"], sheet)
    if sup is not None:
        SP.write_sheets(wb, sup, s["values"], sheet)
    out.parent.mkdir(parents=True, exist_ok=True)
    wb.save(out)
    return out
