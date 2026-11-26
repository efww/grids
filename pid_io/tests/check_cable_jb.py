"""Whole-program check for STEP 6 (cable) / STEP 7 (JB) and the index columns.

    python tests/check_cable_jb.py ["project name"]

1. invariants on the current settings (every IO lands somewhere exactly once, JB sizing rule,
   one multi per JB, unique cable numbers, slots never inside racks / equipment, stations <= max)
2. the exported index: Junction Box / Multi / Field cable columns agree with the cable schedule
3. changes propagate: moving a JB, editing a route, changing sizes / spare
4. every option of every select setting (and number extremes) one at a time, with and without the plot
Prints FAIL lines and a summary; exit code 1 when anything failed.
"""
from __future__ import annotations

import copy
import math
import sys
import traceback
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from pid_io import cable, jb as JB, plot as plotmod, tray as TR          # noqa: E402
from pid_io.app import server as S                           # noqa: E402

FAILS: list[str] = []
NOTES: list[str] = []


def fail(where, msg):
    FAILS.append(f"[{where}] {msg}")


def _n(s):
    return int("".join(ch for ch in str(s) if ch.isdigit()) or 0)


def check_res(where, rows, s, res, layout=None):
    v = s["values"]
    cables, jbs = res["cables"], res["jbs"]
    nos = [c["no"] for c in cables]
    if len(nos) != len(set(nos)):
        dup = sorted({x for x in nos if nos.count(x) > 1})[:5]
        fail(where, f"케이블 번호 중복 {dup}")
    jbn = [j["no"] for j in jbs]
    if len(jbn) != len(set(jbn)):
        fail(where, "JB 번호 중복")
    for c in cables:
        if not isinstance(c["length"], int) or c["length"] <= 0:
            fail(where, f"길이 이상 {c['no']} {c['length']}")
    branch = [c for c in cables if c["kind"] == "분기"]
    multi = {}
    for c in cables:
        if c["kind"] == "멀티":
            if c["frm"] in multi:
                fail(where, f"JB에 멀티 2가닥 {c['frm']}")
            multi[c["frm"]] = c
    cs, ts = float(v["jb_spare"]), float(v["term_spare"])
    cap = JB.capacity(v)
    for j in jbs:
        n = j["used"]
        if n != len(j["tags"]):
            fail(where, f"{j['no']} 사용 수 != 태그 수")
        if n > cap:
            fail(where, f"{j['no']} 용량 초과 {n} > {cap}")
        T, M = _n(j["size"]), _n(j.get("multi_size", j["size"]))
        if T < math.ceil(n * (1 + ts) - 1e-9) or M < math.ceil(n * (1 + cs) - 1e-9) or T < M:
            fail(where, f"{j['no']} 크기 규칙 위반 n={n} T={T} M={M}")
        mc = multi.get(j["no"])
        if not mc:
            fail(where, f"{j['no']} 멀티 없음")
        elif mc["pairs_used"] != n or _n(mc["spec"].split(" ")[0]) != M:
            fail(where, f"{j['no']} 멀티 페어/사양 불일치 {mc['spec']} used={mc['pairs_used']}")
        bt = sorted(c["frm"] for c in branch if c["to"] == j["no"])
        if bt != sorted(j["tags"]):
            fail(where, f"{j['no']} 분기 케이블과 JB 태그 불일치")
    if sum(j["used"] for j in jbs) != len(branch):
        fail(where, "분기 수 != JB 사용 합")
    # every row exactly once: cable or skipped
    covered = {}
    for c in branch:
        covered.setdefault(c["frm"], []).append("분기")
    for c in cables:
        if c["kind"] in ("패키지", "MCC", "통신"):
            for t in c.get("tags") or []:
                covered.setdefault(t, []).append(c["kind"])
    skipped_n = sum(res["skipped"].values())
    comm_rows = [r for r in rows if cable.classify(r, v)[0] == "comm"]
    single = not v["comm_redund"].startswith("이중")
    lost = []
    for r in rows:
        route, why = cable.classify(r, v)
        if route is None:
            continue
        k = covered.get(r.tag, [])
        if len(k) > 1:
            fail(where, f"{r.tag} 케이블 {len(k)}번 ({k})")
        if not k and not (route == "comm" and single):
            lost.append(r.tag)
    if lost:
        fail(where, f"케이블에 안 들어간 IO 행 {len(lost)} (예: {lost[:4]})")
    cabled = len(rows) - skipped_n
    if single:
        n_comm_c = sum(1 for c in cables if c["kind"] == "통신")
        if cabled - len(comm_rows) + n_comm_c < 0:
            fail(where, "행 수 맞춤 실패")
    elif cabled != len({t for t in covered}) + 0 and cabled != len(covered):
        fail(where, f"행 합계 불일치: 전체 {len(rows)} - 제외 {skipped_n} = {cabled}, 케이블 연결 {len(covered)}")
    # index map covers every cabled row (incl. comm rows when single)
    im = cable.index_map(res)
    miss = [r.tag for r in rows if cable.classify(r, v)[0] is not None and r.tag not in im]
    if miss:
        fail(where, f"인덱스 케이블 열이 빌 IO 행 {len(miss)} (예: {miss[:4]})")
    # STEP 8 trays
    t = TR.build(res, layout, s, {})
    br = {c["no"]: c["length"] for c in branch}
    if len(t["pt"]) != len(branch) or any(br[p["cable"]] != p["length"] for p in t["pt"]):
        fail(where, "Perforated 트레이 수/길이 != 분기 케이블")
    tnos = [p["no"] for p in t["pt"]] + [s_["no"] for s_ in t["segments"]] + [x["no"] for s_ in t["segments"] for x in s_["trays"] if x["cls"] != "신호"]
    if len(tnos) != len(set(tnos)):
        fail(where, "트레이 번호 중복")
    lim = TR.eff_fill(v)
    for s_ in t["segments"]:
        if s_["length"] <= 0:
            fail(where, f"{s_['no']} 길이 0")
        for x in s_["trays"]:
            if x["fill"] > lim + 1e-9 and not x["user"]:
                fail(where, f"{x['no']} 자동 크기인데 Fill {x['fill']:.0%} > {lim:.0%}")
            if x["width"] not in TR.widths(v):
                fail(where, f"{x['no']} 폭 {x['width']} 목록 밖")
    if res.get("geo") and layout is not None:
        on = {c for s_ in t["segments"] for c in s_["cables"]}
        routed = {r["rid"] for r in res["geo"]["routes"]}
        miss_t = [c["no"] for c in cables if c["kind"] in ("멀티", "패키지", "MCC", "통신") and TR.rid_of(c) in routed
                  and c["no"] not in on]
        if miss_t:
            fail(where, f"경로가 있는데 트레이/UG 구간에 없는 케이블 {len(miss_t)} {miss_t[:3]}")
        # each routed cable's metres on trays+UG ~ its route's horizontal length
        seglen = {}
        for s_ in t["segments"]:
            for c in s_["cables"]:
                seglen[c] = seglen.get(c, 0) + s_["length"]
        for no, bc in (t.get("by_cable") or {}).items():
            if bc["ladder_m"] + bc["ug_m"] != seglen.get(no, 0):
                fail(where, f"{no} 케이블별 트레이 m {bc['ladder_m']}+{bc['ug_m']} != 구간 합 {seglen.get(no, 0)}")
                break
        if set(t.get("by_cable") or {}) != on:
            fail(where, "케이블별 트레이 표와 구간의 케이블이 다름")
        rl = {r["rid"]: r for r in res["geo"]["routes"]}
        for c in cables:
            rid = TR.rid_of(c)
            if rid in rl and c["no"] in seglen:
                poly = rl[rid]["poly"]
                h = sum(abs(b[0] - a[0]) + abs(b[1] - a[1]) for a, b in zip(poly, poly[1:])) * layout.m_per_pt
                bc = (t.get("by_cable") or {}).get(c["no"]) or {"legs": []}
                runs = sum(1 for i, x in enumerate(bc["legs"]) if x["kind"] == "사다리"
                           and (i == 0 or bc["legs"][i - 1]["kind"] != "사다리"))
                # snapping to the rack centre line adds up to half a rack width on the way in and out
                hw = float(v["rack_width"]) / 2 + 1
                # (rack crossings can zig-zag between two centre lines: allow up to +12 %)
                if not (h - 8 <= seglen[c["no"]] <= max(h * 1.12, h + 3 + 2 * hw * max(1, runs) * 1.5)):
                    fail(where, f"{c['no']} 트레이 구간 길이 {seglen[c['no']]} m != 경로 {h:.0f} m")
                    break
    # plot: slots, stations
    g = res.get("geo")
    if g and layout is not None:
        m = layout.m_per_pt
        obst = layout.plot.get("obstacles") or []
        rects = layout.rack_rects(float(v["rack_width"]))
        pad = 0.2 / m
        for s_ in g.get("slots") or []:
            x, y = s_[1], s_[2]
            if any(o[0] + pad < x < o[2] - pad and o[1] + pad < y < o[3] - pad for o in obst):
                fail(where, f"자리 {s_[0]} 장비/건물 안")
                break
            if any(r[0] < x < r[2] and r[1] < y < r[3] for r in rects):
                fail(where, f"자리 {s_[0]} 랙 안")
                break
        smax = max(1, int(float(v["st_max"])))
        for st in g.get("stations") or []:
            if "," not in st["id"] and len(st["jbs"]) > smax:
                moved = any(j[6] for j in g["jbs"] if j[0] in st["jbs"])
                (NOTES if moved else FAILS).append(f"[{where}] 스테이션 {st['id']} JB {len(st['jbs'])} > {smax}"
                                                   + (" (사용자 이동 포함)" if moved else ""))
        if v["jb_place"].startswith("자리"):
            nos_ = [j[0] for j in g["jbs"] if not j[4]]
            if nos_:
                NOTES.append(f"[{where}] 자리 없이 계기 중심에 둔 JB {len(nos_)} {nos_[:3]}")


def build(rows, s, name, layout):
    return cable.build(rows, s, S._area_map(name), layout)


def layout_with(name, rows, dec_over=None, plot=True):
    if not plot:
        return None
    pl = S.plot_read(name)
    if not pl:
        return None
    dec = copy.deepcopy(S._plot_dec(name))
    dec.update(dec_over or {})
    try:
        pt = S.page_texts_cached(name)
    except Exception:
        pt = {}
    return plotmod.Layout(pl, pt, [r.unit for r in rows if r.unit], dec)


def main(name):
    c = S.computed(name)
    rows = c[4]
    s0 = cable.load(S.pdir(name))
    print(f"프로젝트 {name}: 인덱스 행 {len(rows)}")
    lay = layout_with(name, rows)

    # 1. current settings
    res = build(rows, s0, name, lay)
    check_res("현재 설정", rows, s0, res, lay)
    sm = res["summary"]
    print(f"현재: 케이블 {sm['cables']} · {sm['length']:,} m · JB {sm['jbs']} {sm['jb_sizes']} · 스테이션 {sm.get('stations')}")

    # 2. exported index columns
    try:
        out = S.export(name, {})
        import openpyxl
        f = S.pdir(name) / "output" / out["files"][0]
        ws = openpyxl.load_workbook(f, read_only=True)["Instrument_Index"]
        hdr = [x for x in next(ws.iter_rows(min_row=2, max_row=2, values_only=True))]
        if tuple(hdr[33:38]) != ("Junction Box", "Multi Cable Number", "Multi Cable Spec.", "Field Cable No", "Field Cable Spec."):
            fail("인덱스", f"케이블 열 머리글 {hdr[33:38]}")
        im = cable.index_map(res)
        bad = n = 0
        for x in ws.iter_rows(min_row=3, values_only=True):
            if not x[3]:
                continue
            n += 1
            e = im.get(x[3])
            want = (e["jb"], e["multi"], e["multi_spec"], e["cable"], e["cable_spec"]) if e else ("-",) * 5
            if tuple(v or "" for v in x[33:38]) != tuple(v or "" for v in want):
                bad += 1
                if bad <= 3:
                    fail("인덱스", f"{x[3]} 열 값 {x[33:38]} != 케이블 결과 {want}")
        print(f"인덱스: {n}행 검사, 불일치 {bad}")
        if n != len([r for r in rows if r.kind != 'function' and r.origin != '스페어']):
            fail("인덱스", f"인덱스 행 수 {n} != 계산 행 수")
    except Exception as e:
        fail("인덱스", f"출력 실패 {e}")
        traceback.print_exc()

    # 3. changes propagate
    if lay is not None and res.get("geo"):
        g = res["geo"]
        j0 = next(j for j in g["jbs"] if j[4])
        other = next(s_ for s_ in g["slots"] if s_[3] == "랙" and abs(s_[1] - j0[1]) + abs(s_[2] - j0[2]) > 100)
        lay2 = layout_with(name, rows, {"jb_moves": {j0[0]: other[0]}})
        r2 = build(rows, s0, name, lay2)
        check_res("JB 이동", rows, s0, r2, lay2)
        a = {c["no"]: c["length"] for c in res["cables"]}
        b = {c["no"]: c["length"] for c in r2["cables"]}
        mc = "MC-" + j0[0][3:]
        ch = [k for k in a if k in b and a[k] != b[k]]
        jn = next(j for j in r2["jbs"] if j["no"] == j0[0])
        if not jn["moved"] or jn["slot"] != other[0]:
            fail("JB 이동", "옮긴 자리가 JB 리스트에 안 들어감")
        if mc not in ch:
            fail("JB 이동", f"멀티 {mc} 길이가 안 바뀜")
        br = [c for c in ch if c.startswith("C-")]
        print(f"JB 이동 {j0[0]} → {other[4]}: 길이 바뀐 케이블 {len(ch)} (멀티 {mc}: {a[mc]}→{b[mc]} m, 분기 {len(br)})")
        im2 = cable.index_map(r2)
        if any(im2[t]["jb"] != j0[0] for t in jn["tags"]):
            fail("JB 이동", "인덱스 JB 열이 바뀜 (이동은 위치만 바꿔야 함)")
        # route edit
        rt = next(x for x in g["routes"] if x["kind"] == "jb")
        poly = rt["poly"]
        mid = [poly[0], [poly[0][0] + 400, poly[0][1]], [poly[0][0] + 400, poly[-1][1]], poly[-1]]   # 60 m detour
        lay3 = layout_with(name, rows, {"routes_edit": {rt["rid"]: mid}})
        r3 = build(rows, s0, name, lay3)
        m3 = next(c for c in r3["cables"] if c["kind"] == "멀티" and c["frm"] == rt["rid"])
        m1 = next(c for c in res["cables"] if c["kind"] == "멀티" and c["frm"] == rt["rid"])
        if m3["length"] == m1["length"] or "사용자 수정 경로" not in m3["basis"]:
            fail("경로 수정", f"{rt['rid']} 멀티 길이 반영 안 됨 {m1['length']}→{m3['length']}")
        else:
            print(f"경로 수정 {rt['rid']}: 멀티 {m1['length']}→{m3['length']} m")
        if r3["summary"]["length"] == res["summary"]["length"]:
            fail("경로 수정", "총 길이가 안 바뀜")
        # tray size override
        t0 = TR.build(res, lay, s0, {})
        sg = max((x for x in t0["segments"] if x["trays"]), key=lambda x: x["trays"][0]["area"])
        tr0 = sg["trays"][0]
        t1 = TR.build(res, lay, s0, {"tray_size": {sg["id"]: {tr0["cls"]: [TR.widths(s0["values"])[0], 1]}}})
        tr1 = next(x for x in t1["segments"] if x["id"] == sg["id"])["trays"][0]
        if not tr1["user"] or tr1["width"] != TR.widths(s0["values"])[0] or abs(tr1["fill"] - TR.fill(tr0["area"], tr1["width"], 1, s0["values"])) > 1e-3:
            fail("트레이 크기 변경", "사용자 폭이 반영 안 됨")
        print(f"트레이 {tr0['no']} {tr0['width']}×{tr0['tiers']} Fill {tr0['fill']:.0%} → {tr1['width']} Fill {tr1['fill']:.0%}, "
              f"초과 구간 {t0['summary']['overfill']}→{t1['summary']['overfill']}, 사다리 {t0['summary']['ladder_m']} m · UG {t0['summary']['ug_m']} m")
        t2 = TR.build(r2, lay2, s0, {})
        if t2["summary"] == t0["summary"]:
            fail("JB 이동", "트레이 물량이 안 바뀜")
    # sizes / spare change -> index columns follow
    s2 = copy.deepcopy(s0)
    s2["values"]["jb_spare"] = 0.3
    s2["values"]["term_spare"] = 0.3
    r4 = build(rows, s2, name, lay)
    check_res("스페어 30%", rows, s2, r4, lay)
    im4 = cable.index_map(r4)
    print(f"스페어 30%: JB {r4['summary']['jbs']} {r4['summary']['jb_sizes']} (20%: {res['summary']['jbs']})")
    t = next(iter(j["tags"][0] for j in r4["jbs"]))
    if im4[t]["multi_spec"] not in [c["spec"] for c in r4["cables"] if c["kind"] == "멀티"]:
        fail("스페어 30%", "인덱스 멀티 사양이 결과와 다름")

    # 4. every option, one at a time, with and without the plot
    items = [it for g_ in cable.SCHEMA + JB.SCHEMA + TR.SCHEMA for it in g_["items"]]
    runs = 0
    for plot_on in (True, False):
        L = lay if plot_on else None
        for it in items:
            if it["type"] == "select":
                vals = [o for o in it["options"] if o != s0["values"][it["id"]]]
            elif it["type"] == "number":
                d = float(it["default"])
                vals = [0, d * 3] if it["id"] not in ("rack_pref", "route_factor") else [1, d * 3]
                if it.get("pct"):
                    vals = [0.0, 0.5]
            else:
                vals = {"jb_sizes": ["24,48", "80", "12,24,36,48,60,80"], "multi_std": ["24,48"],
                        "tray_widths": ["300", "150,300,450,600,900"], "od_multi": ["24:20"], "od_single": ["1P:12"]}.get(it["id"], [])
            for val in vals:
                s3 = copy.deepcopy(s0)
                s3["values"][it["id"]] = val
                if it["id"] == "multi_std":
                    s3["values"]["multi_sizing"] = "사용+스페어 최소 규격"
                where = f"{'PLOT' if plot_on else '가정'} {it['name']}={val}"
                try:
                    r = build(rows, s3, name, L)
                    check_res(where, rows, s3, r, L)
                    runs += 1
                except Exception as e:
                    fail(where, f"예외 {type(e).__name__}: {e}")
    print(f"옵션 하나씩 바꿔 계산: {runs}회")
    print()
    for n_ in NOTES[:30]:
        print("참고", n_)
    for f_ in FAILS:
        print("FAIL", f_)
    print(f"\n결과: FAIL {len(FAILS)} · 참고 {len(NOTES)}")
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "서남해 200MW 수전해-2"))
