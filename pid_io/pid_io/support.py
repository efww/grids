"""STEP 9 - tray / trench supports from the STEP 8 segments (NEMA VE 2 based general rules).

Nothing new is routed here: every support stands on a STEP 8 segment (or a perforated tray).

  ladder tray off the rack (AG)  -> supports every span (from the type rule table), type by width / tiers
  ladder tray on the rack        -> the rack carries it: one tier = hold-down clamps every rack beam,
                                    stacked tiers = a T support (post on the rack beam) every beam
  fittings (NEMA VE 2)           -> a support within 600 mm of each fitting end: corners, tees and the
                                    vertical elbows where a tray leaves / joins the rack (no support on the
                                    fitting itself unless the 45-degree option is on)
  expansion splices (NEMA VE 2)  -> straight runs longer than movement / (alpha x dT) get expansion splice
                                    joints; a support within 600 mm on each side; the support nearest the
                                    middle between joints is the anchor, the others are expansion guides
  UG trench                      -> brackets / route markers / pull pits
  perforated tray (JB -> instr.) -> quantity only, tied to the instrument tag

The user can override the type and span of any segment (plot_decisions.json 'sup_over' by tray segment id).
"""
from __future__ import annotations

import math
from collections import Counter

from .schema import _num, _sel, _txt


def _code(d, code):
    d["code"] = code
    return d


TYPES = {
    "L": "L형 (캔틸레버)",
    "T": "T형 (포스트 + 양팔)",
    "H": "문형 (포스트 2 + 빔)",
    "RT": "랙 위 T 서포트 (적층)",
    "RF": "랙 위 피팅 보조 서포트",
    "RC": "랙 위 클램프",
    "UB": "트렌치 브래킷",
    "UM": "직매설 표지",
    "UP": "덕트 풀 피트",
    "PB": "Perforated 브래킷",
}
POSTS = ("L", "T", "H", "RT", "RF")
ALPHA = {"강 (아연도금)": 11.7e-6, "알루미늄": 23.4e-6, "SUS": 17.3e-6}

SCHEMA = [
    {"id": "sp_ag", "title": "AG 서포트 (랙 밖 사다리)",
     "lead": "랙을 타지 않는 사다리 트레이 구간(STEP 6 '랙 없는 구간 (간선)' = AG일 때)에 간격마다 서포트를 세웁니다. "
             "형식은 아래 규칙표(타피컬)에서 폭·단 수를 담을 수 있는 첫 형식, 간격은 그 형식의 최대 스팬.",
     "items": [
         _code(_txt("sp_rules", "형식 규칙표 (타피컬)", "L:300:1:3,T:600:1:3,H:1500:4:3",
                    "형식:최대 폭(mm):최대 단:최대 스팬(m). 위에서부터 담을 수 있는 첫 형식. 타피컬 드로잉 값으로 바꾸세요"), "S01"),
         _code(_sel("sp_type", "서포트 형식", ["자동 (규칙표)", "L형", "T형", "문형"], "자동 (규칙표)",
                    "자동이 아니면 모든 AG 구간을 이 형식으로 (구간별 변경은 PLOT 화면에서)"), "S02"),
         _code(_num("sp_span", "최대 서포트 간격 (전체 상한)", 3.0, "m", "규칙표 스팬과 이 값 중 작은 값. 일반값 2~3 m", 0.5), "S03"),
         _code(_num("sp_tier_gap", "적층 단 간격", 300, "mm", "2단 이상일 때 단과 단 사이"), "S05"),
         _code(_num("sp_arm_add", "팔(빔) 여유 길이", 150, "mm", "트레이 폭 + 이 값 = 팔 길이 (문형은 양쪽)"), "S06"),
         _code(_num("sp_ask_kg", "질문 카드 기준", 100, "kg",
                    "규칙 경계(폭이 형식 최대 폭의 80% 이상)에서 다음 형식과 강재 차이가 이 값 이상이면 물어봄"), "S07"),
     ]},
    {"id": "sp_nema", "title": "피팅 · 익스펜션 (NEMA VE 2)",
     "lead": "NEMA VE 2: 피팅(엘보·티·크로스·리듀서, 랙 출입 수직 엘보) 끝에서 600 mm 안에 서포트, "
             "익스펜션 스플라이스 양쪽 600 mm 안에 서포트, 조인트 사이 가운데에 가장 가까운 서포트는 앵커, 나머지는 익스펜션 가이드. "
             "조인트 간격 = 이동량 ÷ (선팽창계수 × 온도차) (강 ΔT 70°C → 약 31 m).",
     "items": [
         _code(_sel("sp_fit", "피팅 서포트", ["NEMA VE 2 (피팅 끝 600 mm 안)", "구간 끝만"], "NEMA VE 2 (피팅 끝 600 mm 안)",
                    "구간 끝만: 꺾임·분기점에 서포트 1개 (v11 방식)"), "S50"),
         _code(_num("sp_fit_r", "피팅(엘보) 반경", 600, "mm", "피팅 끝 = 꺾임점에서 이 거리"), "S51"),
         _code(_num("sp_fit_d", "피팅 끝 → 서포트 최대 거리", 600, "mm", "NEMA VE 2: 2 ft (600 mm). 서포트는 이 범위 가운데"), "S52"),
         _code(_sel("sp_fit_mid", "수평 엘보 45° 지점 서포트", ["안 함", "추가"], "안 함",
                    "판본·제조사에 따라 큰 반경 엘보의 45° 지점 서포트를 요구 (확인 후 켜기)", on="추가"), "S53"),
         _code(_sel("sp_exp", "익스펜션 스플라이스", ["사용", "안 함"], "사용", "", on="사용"), "S54"),
         _code(_sel("sp_mat", "트레이 재질", list(ALPHA), "강 (아연도금)", "선팽창계수 강 11.7, 알루미늄 23.4, SUS 17.3 (×10⁻⁶/°C)"), "S55"),
         _code(_num("sp_dt", "온도차 ΔT", 70, "°C", "연중 최고·최저 (트레이 표면) 차. 예: −15 ~ 55°C → 70"), "S56"),
         _code(_num("sp_gap", "조인트 이동량", 25.4, "mm", "익스펜션 스플라이스 한 개가 받는 길이 변화 (1 in)", 0.1), "S57"),
     ]},
    {"id": "sp_rack", "title": "랙 위",
     "lead": "랙 최상단을 타는 사다리 트레이는 랙이 받칩니다. 1단이면 랙 빔마다 클램프만, 단을 쌓으면(900 × n단) 랙 빔 위에 T 서포트를 세웁니다. "
             "랙 위 피팅(꺾임·분기·랙 출입)과 익스펜션 조인트 근처에 빔이 없으면 보조 서포트를 둡니다.",
     "items": [
         _code(_sel("sp_rack_stack", "랙 위 적층 트레이", ["T 서포트", "서포트 없음"], "T 서포트",
                    "2단 이상이면 랙 빔 위에 포스트를 세워 위 단을 받침", on="T 서포트"), "S10"),
         _code(_num("sp_bay", "랙 빔 간격 (베이)", 6.0, "m", "클램프·T 서포트가 서는 간격", 0.5), "S11"),
         _code(_num("sp_clamp", "빔당 클램프 (트레이 1단당)", 2, "개", "홀드다운 클램프 / 익스펜션 가이드. 양쪽 레일"), "S12"),
         _code(_sel("sp_rack_fit", "랙 위 피팅 보조 서포트", ["빔에서 멀면 추가", "안 함 (빔이 받침)"], "빔에서 멀면 추가",
                    "피팅 끝·익스펜션 조인트에서 600 mm 안에 랙 빔이 없을 때 보조 빔. 빔 위치는 구간 시작부터 베이 간격으로 가정",
                    on="빔에서 멀면 추가"), "S13"),
     ]},
    {"id": "sp_ug", "title": "UG · Perforated",
     "lead": "UG 구간은 방식에 따라 트렌치 브래킷 / 직매설 표지 / 덕트 풀 피트를, Perforated는 길이로 브래킷 수만 냅니다 (계기 태그별).",
     "items": [
         _code(_sel("sp_ug_type", "UG 방식", ["콘크리트 트렌치 (브래킷)", "직매설", "덕트 뱅크"], "콘크리트 트렌치 (브래킷)"), "S20"),
         _code(_num("sp_ug_span", "트렌치 브래킷 간격", 1.5, "m", "트렌치 벽 케이블 브래킷. 단이 여러 개면 단마다", 0.5), "S21"),
         _code(_num("sp_marker", "직매설 표지 간격", 30, "m", "꺾임점에도 1개"), "S22"),
         _code(_num("sp_pit", "덕트 풀 피트 간격", 50, "m", "꺾임점에도 1개"), "S23"),
         _code(_num("sp_pt_span", "Perforated 브래킷 간격", 1.5, "m", "JB → 계기 Perforated 트레이", 0.5), "S24"),
     ]},
    {"id": "sp_mat", "title": "자재 · 타피컬",
     "lead": "서포트 1개의 구성 자재 일반값입니다. 타피컬 드로잉(Typical Support Drawing)이 오면 도면 번호와 값을 넣고, "
             "'형식 카드'에서 형식마다 타피컬 그림을 올린 뒤 프리셋으로 저장하세요.",
     "items": [
         _code(_txt("sp_member", "주 부재", "C100×50×5/7.5", "포스트·팔·빔"), "S30"),
         _code(_num("sp_kgm", "주 부재 단위 중량", 9.36, "kg/m", "C100×50×5/7.5 = 9.36 kg/m", 0.01), "S31"),
         _code(_num("sp_pb_kgm", "Perforated 브래킷 단위 중량", 4.43, "kg/m", "L50×50×6 = 4.43 kg/m", 0.01), "S32"),
         _code(_num("sp_pb_len", "Perforated 브래킷 1개 길이", 0.5, "m", "", 0.1), "S33"),
         _code(_num("sp_ub_len", "트렌치 브래킷 1개 길이", 0.4, "m", "", 0.1), "S34"),
         _code(_num("sp_anchor", "베이스 플레이트당 앵커 볼트", 4, "개"), "S35"),
         _code(_num("sp_fit_pct", "서포트 자재 할증", 0.1, "", "", 0.05, True), "S36"),
         _code(_txt("sp_typ_L", "타피컬 도면 번호 - L형", "TS-01 (일반값)"), "S40"),
         _code(_txt("sp_typ_T", "타피컬 도면 번호 - T형", "TS-02 (일반값)"), "S41"),
         _code(_txt("sp_typ_H", "타피컬 도면 번호 - 문형", "TS-03 (일반값)"), "S42"),
         _code(_txt("sp_typ_RT", "타피컬 도면 번호 - 랙 위 T", "TS-04 (일반값)"), "S43"),
         _code(_txt("sp_typ_UB", "타피컬 도면 번호 - 트렌치 브래킷", "TS-05 (일반값)"), "S44"),
         _code(_txt("sp_typ_PB", "타피컬 도면 번호 - Perforated 브래킷", "TS-06 (일반값)"), "S45"),
         _code(_txt("sp_typ_RF", "타피컬 도면 번호 - 랙 위 피팅 보조", "TS-07 (일반값)"), "S46"),
     ]},
]
ITEMS = {it["id"]: it for g in SCHEMA for it in g["items"]}
MIN_SPAN = 0.5                                     # a spacing of 0 would mean a support every millimetre
RANK = {"L": 0, "T": 1, "H": 2}


def span_of(x):
    return max(float(x), MIN_SPAN)


def typical(t, v):
    return v.get(f"sp_typ_{t}", "") or ""


def rules(v):
    """type rule table -> [(type, max width mm, max tiers, max span m)] in order."""
    out = []
    for part in str(v.get("sp_rules") or "").split(","):
        f = [x.strip() for x in part.split(":")]
        if len(f) >= 2 and f[0].upper() in RANK:
            try:
                out.append((f[0].upper(), float(f[1]), int(float(f[2])) if len(f) > 2 else 1,
                            float(f[3]) if len(f) > 3 else float(v["sp_span"])))
            except ValueError:
                continue
    return out or [("L", 300, 1, 3.0), ("T", 600, 1, 3.0), ("H", 1e9, 99, 3.0)]


def ag_choice(width, tiers, v):
    """(auto type, its span, next heavier candidate or None)."""
    rl = rules(v)
    forced = {"L형": "L", "T형": "T", "문형": "H"}.get(v["sp_type"])

    def span(t):
        return min(next((r[3] for r in rl if r[0] == t), float(v["sp_span"])), float(v["sp_span"]))
    if forced:
        return forced, span(forced), None
    ok = [r for r in rl if width <= r[1] + 1e-9 and tiers <= r[2]]
    if not ok:
        r = max(rl, key=lambda r: RANK[r[0]])
        return r[0], span(r[0]), None
    nxt = next((r for r in ok[1:] if r[0] != ok[0][0]), None)
    return ok[0][0], span(ok[0][0]), (nxt[0] if nxt else None)


def max_width(t, v):
    return next((r[1] for r in rules(v) if r[0] == t), 0)


def members(t, width, tiers, el, v):
    """(main member m, base plates, anchors, clamps) for one support. el = height of the lowest tier (m)."""
    arm = (width + float(v["sp_arm_add"])) / 1000
    gap = float(v["sp_tier_gap"]) / 1000
    top = el + (tiers - 1) * gap
    cl = float(v["sp_clamp"]) * tiers
    anc = float(v["sp_anchor"])
    if t == "L":
        return top + arm * tiers, 1, anc, cl
    if t == "T":
        return top + 2 * arm * tiers, 1, anc, cl
    if t == "H":
        return 2 * top + (arm + float(v["sp_arm_add"]) / 1000) * tiers, 2, 2 * anc, cl
    if t == "RT":                                  # post on the rack beam, arms for the upper tiers
        return (tiers - 1) * gap + 0.1 + 2 * arm * (tiers - 1), 1, anc, cl
    if t == "RF":                                  # short auxiliary beam between rack beams
        return arm * tiers + 0.3, 2, 2 * anc, cl
    if t == "RC":
        return 0.0, 0, 0, cl
    if t == "UB":
        return float(v["sp_ub_len"]) * tiers, 0, 2 * tiers, 0
    return 0.0, 0, 0, 0


def exp_spacing(v):
    """max straight length between expansion splices (m): movement / (alpha x dT)."""
    a = ALPHA.get(v["sp_mat"], 11.7e-6)
    return max(1.0, float(v["sp_gap"]) / 1000 / (a * max(1.0, float(v["sp_dt"]))))


def _fill(fixed, span):
    """positions of extra supports so no gap between consecutive fixed positions exceeds span."""
    out = []
    pts = sorted(set(round(x, 3) for x in fixed))
    for p, q in zip(pts, pts[1:]):
        n = math.ceil((q - p) / span - 1e-9)
        out += [p + (q - p) * i / n for i in range(1, n)]
    return out


def build(tray, v, m_per_pt=None, dec=None, obstacles=None):
    """tray = tray.build(...) output -> supports, joints, per-segment rows, perforated per tag, questions, summary.
    obstacles (plot pt rectangles): a post standing inside one is slid along its segment to the nearest free
    point (within half a span), else flagged 'blocked'."""
    dec = dec or {}
    over = dec.get("sup_over") or {}
    out = {"supports": [], "segments": [], "pt": [], "joints": [], "questions": [], "summary": {}, "lost_overrides": 0}
    gl_el, rack_el = float(v["sup_el"]), float(v["rack_el"])
    m = m_per_pt or 0
    segs = tray.get("segments", [])
    nema = v["sp_fit"].startswith("NEMA")
    r_fit = float(v["sp_fit_r"]) / 1000
    d_fit = float(v["sp_fit_d"]) / 1000

    def nk(p):                                     # node key: 0.1 m
        return (round(p[0] * m * 10), round(p[1] * m * 10)) if m else (round(p[0] * 10), round(p[1] * 10))

    # ---- node graph: which segment ends meet where, and in which direction they leave
    at = {}
    for i, s in enumerate(segs):
        a, b = s["poly"]
        L_ = math.hypot(b[0] - a[0], b[1] - a[1]) or 1.0
        at.setdefault(nk(a), []).append((i, ((b[0] - a[0]) / L_, (b[1] - a[1]) / L_)))
        at.setdefault(nk(b), []).append((i, ((a[0] - b[0]) / L_, (a[1] - b[1]) / L_)))

    def node_kind(p, i):
        """segment i's end at p: 'free' (nothing else), 'splice' (straight on, same kind), else 'fitting'."""
        here = at.get(nk(p), [])
        others = [(j, d) for j, d in here if j != i]
        if not others:
            return "free"
        mine = next(d for j, d in here if j == i)
        if len(others) == 1:
            j, d = others[0]
            if abs(mine[0] + d[0]) < 1e-3 and abs(mine[1] + d[1]) < 1e-3 and segs[j]["kind"] == segs[i]["kind"]:
                return "splice"
        return "fitting"

    pos = {}

    def add(kind, p, seg, info):
        k = (nk(p), kind in ("UB", "UM", "UP"))
        i = pos.get(k)
        if i is not None:                          # shared point: one support, the bigger type wins
            s = out["supports"][i]
            if kind in RANK and s["type"] in RANK and RANK[kind] > RANK[s["type"]]:
                s.update(type=kind, **info)
            elif kind in POSTS and s["type"] == "RC":
                s.update(type=kind, **info)
            elif s["type"] == kind:
                for f in ("width", "tiers"):
                    if f in info:
                        s[f] = max(s.get(f, 0), info[f])
            if info.get("role") and not s.get("role"):
                s["role"] = info["role"]
            if seg not in s["segs"]:
                s["segs"].append(seg)
            return i
        pos[k] = len(out["supports"])
        out["supports"].append({"type": kind, "x": round(p[0], 1), "y": round(p[1], 1), "segs": [seg],
                                **{"role": ""}, **info})
        return pos[k]

    # ---- straight ladder runs (through 'splice' nodes only) -> expansion splice joints
    lad = [i for i, s in enumerate(segs) if s["kind"] != "UG"]
    parent = {i: i for i in lad}

    def find(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i
    for i in lad:
        for p in segs[i]["poly"]:
            if node_kind(p, i) == "splice":
                j = next(j for j, _ in at[nk(p)] if j != i)
                if j in parent:
                    parent[find(i)] = find(j)
    groups = {}
    for i in lad:
        groups.setdefault(find(i), []).append(i)
    joints_at = {}                                  # seg index -> [distance from its poly[0], m]
    exp_on = v["sp_exp"] == "사용"
    Lx = exp_spacing(v)
    out["exp_spacing"] = round(Lx, 1)
    runs = []
    for rid, mem in groups.items():
        start = next((i for i in mem if any(node_kind(p, i) != "splice" for p in segs[i]["poly"])), mem[0])
        p0, p1 = segs[start]["poly"]
        entry = p0 if node_kind(p0, start) != "splice" else p1
        order, seen, cur = [], set(), start
        while cur is not None and cur not in seen:
            seen.add(cur)
            a, b = segs[cur]["poly"]
            fwd = nk(a) == nk(entry)
            order.append((cur, fwd))
            ex = b if fwd else a
            nxt = None
            if node_kind(ex, cur) == "splice":
                nxt = next((j for j, _ in at[nk(ex)] if j != cur and j in parent and find(j) == rid), None)
            cur, entry = nxt, ex
        total = sum(segs[i]["length"] for i, _ in order)
        runs.append({"segs": [i for i, _ in order], "length": total, "joints": 0})
        if not exp_on or total <= Lx + 1e-9:
            continue
        n = math.ceil(total / Lx - 1e-9) - 1
        runs[-1]["joints"] = n
        acc = 0.0
        for i, fwd in order:
            L_ = segs[i]["length"]
            for k in range(1, n + 1):
                c = total * k / (n + 1)
                if acc - 1e-9 <= c < acc + L_ - 1e-9:
                    d = c - acc
                    joints_at.setdefault(i, []).append(d if fwd else L_ - d)
            acc += L_

    # ---- place per segment
    used_over = set()
    seg_rows = []
    for i, s in enumerate(segs):
        a, b = s["poly"]
        L = s["length"]
        trs = s.get("trays") or []
        idx = []

        def at_d(d, a=a, b=b, L=L):
            return (a[0] + (b[0] - a[0]) * d / L, a[1] + (b[1] - a[1]) * d / L) if L else a
        o = over.get(s["id"]) or {}
        if o:
            used_over.add(s["id"])
        if s["kind"] == "UG":
            ut = v["sp_ug_type"]
            kind, span = (("UB", float(v["sp_ug_span"])) if ut.startswith("콘크리트")
                          else ("UM", float(v["sp_marker"])) if ut.startswith("직매설") else ("UP", float(v["sp_pit"])))
            span = span_of(o.get("span") or span)
            for d in [0, L] + _fill([0, L], span):
                idx.append(add(kind, at_d(d), s["no"], {"el": -float(v["ug_depth"]), "width": s.get("width", 0),
                                                        "tiers": s.get("tiers", 1)}))
            seg_rows.append({"seg": s["no"], "id": s["id"], "kind": s["kind"], "length": L, "idx": idx, "trays": [],
                             "span": span, "type": kind, "auto": kind, "alt": None, "user": bool(o)})
            continue
        n = max((t_["tiers"] for t_ in trs), default=1)
        wsum = sum(t_["width"] for t_ in trs) + 100 * max(0, len(trs) - 1)   # trays side by side on one support
        info = {"width": wsum, "tiers": n, "trays": sum(t_["tiers"] for t_ in trs)}
        if s["kind"] == "랙":
            kind = "RT" if (n > 1 and v["sp_rack_stack"] == "T 서포트") else "RC"
            auto, nxt = kind, None
            span = span_of(o.get("span") or v["sp_bay"])
            info["el"] = rack_el
            fit_kind = "RF"
        else:
            auto, span0, nxt = ag_choice(wsum, n, v)
            kind = o.get("type") or auto
            span = span_of(o.get("span") or span0)
            info["el"] = gl_el
            fit_kind = kind
        fixed, extra = [], []
        for d, p in ((0.0, a), (float(L), b)):
            k_ = node_kind(p, i)
            if k_ == "fitting" and nema and L > 0:
                off = min(r_fit + d_fit / 2, L / 2)
                extra.append((off if d == 0 else L - off, fit_kind, "피팅"))
                here = at.get(nk(p), [])
                if len(here) == 2 and v["sp_fit_mid"] == "추가" and all(segs[j]["kind"] == s["kind"] for j, _ in here):
                    idx.append(add(fit_kind, p, s["no"], dict(info, role="엘보 45°")))
            else:
                fixed.append(d)
                idx.append(add(kind, p, s["no"], dict(info, role="피팅" if k_ == "fitting" else "")))
        for d in joints_at.get(i, []):
            for dd in (max(0.0, d - d_fit / 2), min(float(L), d + d_fit / 2)):
                extra.append((dd, kind if s["kind"] != "랙" else "RF", "익스펜션"))
            out["joints"].append({"seg": s["no"], "id": s["id"], "at_m": round(d, 2), "x": round(at_d(d)[0], 1),
                                  "y": round(at_d(d)[1], 1), "el": info["el"],
                                  "trays": sum(t_["tiers"] for t_ in trs), "tray_nos": [t_["no"] for t_ in trs]})
        bay = span_of(v["sp_bay"])
        for dd, k_, role in extra:
            if s["kind"] == "랙" and k_ == "RF" and (v["sp_rack_fit"].startswith("안 함")
                                                     or min(dd % bay, bay - dd % bay) <= d_fit / 2 + 1e-9):
                fixed.append(dd)                   # a rack beam is within reach: clamp / T support there
                idx.append(add(kind, at_d(dd), s["no"], dict(info, role=role)))
                continue
            fixed.append(dd)
            idx.append(add(k_, at_d(dd), s["no"], dict(info, role=role)))
        if not fixed:
            fixed = [0.0, float(L)]
        lo, hi = min(fixed), max(fixed)
        # beyond the outermost fixed support the fitting itself spans to the next segment
        for dd in _fill(fixed, span):
            if lo < dd < hi:
                idx.append(add(kind, at_d(dd), s["no"], dict(info)))
        if s["kind"] == "AG" and nxt and not o and wsum >= 0.8 * max_width(auto, v):
            cnt = len(set(idx))
            c_auto = members(auto, wsum, n, gl_el, v)[0] * float(v["sp_kgm"])
            c_nxt = members(nxt, wsum, n, gl_el, v)[0] * float(v["sp_kgm"])
            if (c_nxt - c_auto) * cnt >= float(v["sp_ask_kg"]):
                out["questions"].append({"seg": s["no"], "id": s["id"], "auto": auto, "alt": nxt, "width": wsum,
                                         "tiers": n, "count": cnt, "kg_auto": round(c_auto * cnt),
                                         "kg_alt": round(c_nxt * cnt),
                                         "why": f"폭 합 {wsum:.0f} mm가 {TYPES[auto]} 최대 폭 {max_width(auto, v):.0f} mm에 가깝습니다"})
        seg_rows.append({"seg": s["no"], "id": s["id"], "kind": s["kind"], "length": L, "idx": idx,
                         "trays": [t_["no"] for t_ in trs], "span": span, "type": kind, "auto": auto, "alt": nxt,
                         "user": bool(o)})
    out["lost_overrides"] = len(set(over) - used_over)
    _avoid(out, segs, obstacles or [], m, v)

    # ---- anchor / guides between expansion splices of one straight run
    for r in runs:
        if not r["joints"]:
            continue
        ids = {segs[i]["no"] for i in r["segs"]}
        a0 = segs[r["segs"][0]]["poly"][0]
        ax = segs[r["segs"][0]]["poly"]
        horiz = abs(ax[1][0] - ax[0][0]) >= abs(ax[1][1] - ax[0][1])
        t_of = (lambda p: p[0]) if horiz else (lambda p: p[1])
        cuts = sorted(t_of((j["x"], j["y"])) for j in out["joints"] if j["seg"] in ids)
        sup = [k for k, x in enumerate(out["supports"]) if set(x["segs"]) & ids and x["type"] not in ("UB", "UM", "UP")]
        edges = [-1e18] + cuts + [1e18]
        for lo, hi in zip(edges, edges[1:]):
            grp = [k for k in sup if lo < t_of((out["supports"][k]["x"], out["supports"][k]["y"])) < hi]
            if not grp:
                continue
            ts = [t_of((out["supports"][k]["x"], out["supports"][k]["y"])) for k in grp]
            mid = (max(ts) + min(ts)) / 2
            anc = min(grp, key=lambda k: abs(t_of((out["supports"][k]["x"], out["supports"][k]["y"])) - mid))
            for k in grp:
                out["supports"][k]["fix"] = "앵커" if k == anc else "가이드"
        _ = a0
    out["runs"] = [{"segs": [segs[i]["no"] for i in r["segs"]], "length": r["length"], "joints": r["joints"]}
                   for r in runs]

    # ---- numbering: nearest the control room first = segment order from tray.build
    num = Counter()
    pre = {"L": "SP", "T": "SP", "H": "SP", "RT": "SP", "RF": "SP", "RC": "CL", "UB": "UB", "UM": "UM", "UP": "UP"}
    for s in out["supports"]:
        p_ = s["segs"][0].split("-")
        u = p_[1] if len(p_) >= 3 else "00"
        p = pre[s["type"]]
        num[(p, u)] += 1
        s["no"] = f"{p}-{u}-{num[(p, u)]:03d}"
        s["typical"] = typical(s["type"], v) if s["type"] not in ("RC", "UM", "UP") else ""
        mm, bp, an, cl = members(s["type"], s.get("width", 300), s.get("tiers", 1),
                                 s["el"] if s["type"] in ("L", "T", "H") else 0, v)
        s.update(steel_m=round(mm, 2), base=bp, anchors=an, clamps=cl,
                 kg=round(mm * float(v["sp_kgm"]), 1), name=TYPES[s["type"]])
        s.setdefault("fix", "")
        if m:
            s["x_m"], s["y_m"] = round(s["x"] * m, 2), round(s["y"] * m, 2)
    for k, j in enumerate(out["joints"], 1):
        j["no"] = f"EJ-{k:03d}"
        if m:
            j["x_m"], j["y_m"] = round(j["x"] * m, 2), round(j["y"] * m, 2)
    for sg in seg_rows:
        idx = list(dict.fromkeys(sg.pop("idx")))
        nos = [out["supports"][i]["no"] for i in idx]
        own = [out["supports"][i]["no"] for i in idx if out["supports"][i]["segs"][0] == sg["seg"]]
        sg["count"] = len(nos)
        sg["shared"] = len(nos) - len(own)          # ends numbered with the neighbouring segment
        sg["types"] = dict(Counter(out["supports"][i]["type"] for i in idx))
        own_s = sorted(own)
        sg["range"] = (f"{own_s[0]} ~ {own_s[-1]}" if len(own_s) > 1 else (own_s[0] if own_s else "")) + \
                      (f" (+공유 {sg['shared']})" if sg["shared"] else "")
        sg["nos"] = nos
        sg["joints"] = sum(1 for j in out["joints"] if j["seg"] == sg["seg"])
    out["segments"] = seg_rows
    # perforated: quantity per instrument tag
    span = span_of(v["sp_pt_span"])
    for p in tray.get("pt", []):
        n = max(1, math.ceil(p["length"] / span - 1e-9))
        out["pt"].append({"tag": p["to"], "pt": p["no"], "jb": p["frm"], "length": p["length"], "count": n,
                          "steel_m": round(n * float(v["sp_pb_len"]), 2),
                          "kg": round(n * float(v["sp_pb_len"]) * float(v["sp_pb_kgm"]), 1),
                          "typical": typical("PB", v)})
    out["summary"] = _summary(out, v)
    return out


def _avoid(out, segs, obstacles, m, v):
    """posts (L/T/H) inside an equipment / building outline: slide along the segment, else flag."""
    if not obstacles or not m:
        return
    pad = 0.2 / m                                   # on the outline itself is outside (routes run along edges)
    inside = lambda x, y: any(o[0] + pad < x < o[2] - pad and o[1] + pad < y < o[3] - pad for o in obstacles)
    by_no = {s["no"]: s for s in segs}
    taken = {(round(x["x"], 1), round(x["y"], 1)) for x in out["supports"]}
    for x in out["supports"]:
        if x["type"] not in ("L", "T", "H") or not inside(x["x"], x["y"]):
            continue
        sg = by_no.get(x["segs"][0])
        if not sg or not sg["length"]:
            x["blocked"] = True
            continue
        a, b = sg["poly"]
        L = sg["length"]
        ux, uy = (b[0] - a[0]) / L, (b[1] - a[1]) / L       # pt per metre along the segment
        lim = span_of(v["sp_span"]) / 2
        done = False
        for k in range(1, int(lim / 0.25) + 1):
            for sgn in (1, -1):
                d = sgn * k * 0.25
                nx, ny = x["x"] + ux * d, x["y"] + uy * d
                t = ((nx - a[0]) * ux + (ny - a[1]) * uy) / ((ux * ux + uy * uy) or 1)
                if not (0 <= t <= L) or inside(nx, ny) or (round(nx, 1), round(ny, 1)) in taken:
                    continue
                x["x"], x["y"] = round(nx, 1), round(ny, 1)
                x["moved"] = round(abs(d), 2)
                done = True
                break
            if done:
                break
        if not done:
            x["blocked"] = True


def _summary(sp, v):
    fit = float(v["sp_fit_pct"])
    by = {}
    for s in sp["supports"]:
        k = s["type"]
        x = by.setdefault(k, {"type": k, "name": TYPES[k], "count": 0, "steel_m": 0.0, "kg": 0.0, "base": 0,
                              "anchors": 0, "clamps": 0, "typical": s.get("typical", "")})
        x["count"] += 1
        for f in ("steel_m", "kg", "base", "anchors", "clamps"):
            x[f] += s[f]
    if sp["pt"]:
        by["PB"] = {"type": "PB", "name": TYPES["PB"], "count": sum(p["count"] for p in sp["pt"]),
                    "steel_m": sum(p["steel_m"] for p in sp["pt"]), "kg": sum(p["kg"] for p in sp["pt"]),
                    "base": 0, "anchors": 2 * sum(p["count"] for p in sp["pt"]), "clamps": 0,
                    "typical": typical("PB", v)}
    order = list(TYPES)
    rows = sorted(by.values(), key=lambda x: order.index(x["type"]))
    for x in rows:
        x["steel_m"] = round(x["steel_m"], 1)
        x["kg"] = round(x["kg"], 1)
        x["kg_order"] = math.ceil(x["kg"] * (1 + fit))
        x["anchors"] = int(x["anchors"])
        x["clamps"] = int(x["clamps"])
    fixc = Counter(s.get("fix") for s in sp["supports"] if s.get("fix"))
    return {"rows": rows, "supports": sum(x["count"] for x in rows if x["type"] in POSTS),
            "clamps": sum(x["clamps"] for x in rows), "kg": round(sum(x["kg"] for x in rows), 1),
            "kg_order": sum(x["kg_order"] for x in rows), "pt_n": by.get("PB", {}).get("count", 0),
            "ug": sum(x["count"] for x in rows if x["type"] in ("UB", "UM", "UP")),
            "joints": len(sp["joints"]), "joint_plates": sum(j["trays"] for j in sp["joints"]),
            "exp_spacing": sp.get("exp_spacing"), "anchor_pts": fixc.get("앵커", 0), "guide_pts": fixc.get("가이드", 0),
            "fitting": sum(1 for s in sp["supports"] if s.get("role") in ("피팅", "엘보 45°")),
            "questions": len(sp["questions"]), "moved": sum(1 for s in sp["supports"] if s.get("moved")),
            "blocked": sum(1 for s in sp["supports"] if s.get("blocked"))}


def checklist(v):
    out = []
    for g in SCHEMA:
        for it in g["items"]:
            val = v.get(it["id"], it["default"])
            out.append({"code": it["code"], "group": g["title"], "name": it["name"], "value": val,
                        "default": it["default"], "changed": str(val) != str(it["default"]), "help": it.get("help", "")})
    return out


def write_sheets(wb, sp, v, sheet):
    sm = sp["summary"]
    rows = [[x["name"], x["typical"], x["count"], x["steel_m"], x["kg"], x["kg_order"], x["base"], x["anchors"], x["clamps"]]
            for x in sm["rows"]]
    rows += [["", "", "", "", "", "", "", "", ""],
             ["익스펜션 스플라이스 조인트", f"최대 간격 {sm['exp_spacing']} m", sm["joints"], "", "", "",
              f"플레이트 세트 {sm['joint_plates']}", "", ""],
             ["앵커 / 가이드 지점 (조인트 있는 직선 구간)", "", f"{sm['anchor_pts']} / {sm['guide_pts']}", "", "", "", "", "", ""],
             ["피팅 서포트 (NEMA VE 2)", "", sm["fitting"], "", "", "", "", "", ""],
             ["장비·건물 피해 옮긴 서포트 / 못 옮김", "", f"{sm['moved']} / {sm['blocked']}", "", "", "", "", "", ""],
             ["(일반값 기준. 타피컬 드로잉이 오면 STEP 9 '자재 · 타피컬' 값과 형식 규칙표를 바꾸세요)", "", "", "", "", "", "", "", ""]]
    sheet(wb.create_sheet("Support_Summary"),
          ["형식", "타피컬", "개수", "주 부재 (m)", "중량 (kg)", "중량 (할증, kg)", "베이스", "앵커", "클램프"],
          rows, [34, 18, 8, 12, 12, 14, 14, 8, 8])
    sheet(wb.create_sheet("Support_List"),
          ["Support No", "형식", "타피컬", "역할", "고정", "장애물", "구간 (Tray / UG)", "폭 합 (mm)", "단 수", "EL (m)", "X (m)", "Y (m)",
           "X (pt)", "Y (pt)", "주 부재 (m)", "중량 (kg)", "앵커", "클램프"],
          [[s["no"], s["name"], s["typical"], s.get("role", ""), s.get("fix", ""),
            ("장비 안 (확인 필요)" if s.get("blocked") else f"피해 {s['moved']} m 이동" if s.get("moved") else ""), ", ".join(s["segs"]), s.get("width", ""),
            s.get("tiers", ""), s["el"], s.get("x_m", ""), s.get("y_m", ""), s["x"], s["y"], s["steel_m"], s["kg"],
            s["anchors"], s["clamps"]] for s in sp["supports"]], [14, 20, 16, 9, 7, 16, 26, 9, 6, 7, 9, 9, 9, 9, 10, 9, 6, 7])
    sheet(wb.create_sheet("Support_by_Segment"),
          ["구간", "종류", "트레이", "길이 (m)", "간격 (m)", "형식 (자동)", "결정", "서포트 수", "조인트", "형식별", "번호"],
          [[g["seg"], g["kind"], ", ".join(g["trays"]), g["length"], g["span"], TYPES.get(g["auto"], g["auto"]),
            "사용자" if g["user"] else "자동", g["count"], g["joints"],
            ", ".join(f"{TYPES[k]} {n}" for k, n in g["types"].items()), g["range"]] for g in sp["segments"]],
          [14, 8, 28, 9, 8, 18, 7, 9, 7, 34, 30])
    sheet(wb.create_sheet("Expansion_Joints"),
          ["EJ No", "구간", "구간 시작에서 (m)", "트레이 수 (단 포함)", "트레이", "EL", "X (m)", "Y (m)"],
          [[j["no"], j["seg"], j["at_m"], j["trays"], ", ".join(j["tray_nos"]), j["el"], j.get("x_m", ""), j.get("y_m", "")]
           for j in sp["joints"]], [10, 14, 14, 14, 30, 6, 9, 9])
    sheet(wb.create_sheet("Support_Perforated"),
          ["Tag (계기)", "PT No", "From (JB)", "길이 (m)", "브래킷 수", "부재 (m)", "중량 (kg)", "타피컬"],
          [[p["tag"], p["pt"], p["jb"], p["length"], p["count"], p["steel_m"], p["kg"], p["typical"]] for p in sp["pt"]],
          [20, 14, 18, 9, 9, 9, 9, 16])
    sheet(wb.create_sheet("Support_Checklist"), ["No", "구분", "항목", "적용값", "기본값", "", "설명"],
          [[c["code"], c["group"], c["name"], c["value"], c["default"], "변경" if c["changed"] else "", c["help"]]
           for c in checklist(v)], [6, 18, 30, 26, 26, 6, 70])
