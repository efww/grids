"""산출근거 workbook: everything the program counted, how it is tied together, and why.

  0 요약            totals and the audit verdict
  1 누락점검         every quantity (IO, cable, JB, tray, trench, perforated, support, joint) is in the outputs
  2 관계점검         tag -> field cable -> perforated -> JB -> multi -> ladder / trench -> supports hang together
  3 IO 근거          IO per origin with the formula (symbols x rule, trains x, spares = ceil(n x rate))
  4 케이블 근거      branch = IO x 1, multi = JB x 1, lengths (horizontal x detour + height) x margin, rounding
  5 JB 근거          capacity, size rule, groups, fill
  6 트레이 근거      sum of pi/4 OD^2, width by fill, tiers; trench widths and runs
  7 서포트 근거      span, fittings (NEMA VE 2), expansion joints, members
  8 자재            bill of quantities
  9 체크리스트 영향   each checklist / setting option: what moves, by how much, in words; verdict
  10 검토 의견       findings (odd results explained or fixed, items not wired, duplicates)
  Tag_Chain, Tray_Chain  per tag / per segment relations
"""
from __future__ import annotations

import copy
import math
import time
from collections import Counter
from pathlib import Path

from . import cable
from .io_assign import assign
from .pipeline import summarize

KINDS_TRUNK = ("멀티", "패키지", "MCC", "통신")


# ------------------------------------------------------------------ one full calculation
def chain(rows, s, area_map, layout, dec, obstacles, concept=None):
    if concept is not None:                                # checklist C03 / C04 / H06 drive the linked cable settings
        s = {**s, "values": dict(s["values"])}
        cable.apply_concept(s["values"], concept)
    res = cable.build(rows, s, area_map, layout)
    tr = cable.TR.build(res, layout, s, dec)
    sp = cable.SP.build(tr, s["values"], layout.m_per_pt if layout else None, dec, obstacles)
    return res, tr, sp


def metrics(rows, res, tr, sp, concept):
    inst = [r for r in rows if r.kind != "function"]
    io = Counter(r.io for r in inst if r.system not in ("-", "?"))
    sm = summarize(rows, concept)
    jbsz = Counter(j["size"] for j in res["jbs"])
    area = sum(s["length"] * t["width"] * t["tiers"] for s in tr["segments"] for t in s.get("trays") or []) / 1000
    by = Counter()
    for c in res["cables"]:
        by[c["kind"]] += c["length"]
    return {"IO 행": len(inst), "HW IO": sm["hw"], "AI": io["AI"], "AO": io["AO"], "DI": io["DI"], "DO": io["DO"],
            "RTD": io["TC/RTD"], "통신 IO": io["Serial/Comm"],
            "케이블 수": len(res["cables"]), "케이블 m": sum(c["length"] for c in res["cables"]),
            "분기 m": by["분기"], "멀티 m": by["멀티"] + by["패키지"] + by["MCC"], "통신 m": by["통신"],
            "JB": len(res["jbs"]), "JB 크기": " ".join(f"{k}×{v}" for k, v in sorted(jbsz.items())),
            "사다리 m": tr["summary"]["ladder_m"], "트레이 면적 m²": round(area, 1), "UG m": tr["summary"]["ug_m"],
            "트렌치 줄": len(tr["summary"].get("ug_runs") or []), "Perforated m": tr["summary"]["pt_m"],
            "서포트(포스트)": sp["summary"]["supports"], "강재 kg": round(sp["summary"]["kg"]),
            "조인트": sp["summary"]["joints"],
            "사다리 발주 m": sum(x["order"] for x in tr["summary"]["ladder"]),
            "트렌치 면적 m²": round(sum(sg["length"] * sg.get("width", 0) * sg.get("tiers", 1) for sg in tr["segments"]
                                     if sg["kind"] == "UG") / 1000, 1),
            "UG 발주 m": tr["summary"]["ug_order"], "강재 발주 kg": sp["summary"]["kg_order"],
            "클램프": sp["summary"]["clamps"], "앵커": sum(x["anchors"] for x in sp["summary"]["rows"]),
            "케이블 사양": " | ".join(f"{k} {v}" for k, v in sorted(Counter(c["spec"] for c in res["cables"]).items())),
            "JB 사양": " | ".join(f"{k} {v}" for k, v in sorted(Counter(f"{j.get('material', '')} {j.get('ex', '')} {j.get('ip', '')} "
                                                                      f"단자 {j.get('terminals', '')}" for j in res["jbs"]).items())),
            "Perforated 폭": tr["pt"][0]["width"] if tr["pt"] else ""}


NUM_KEYS = ["HW IO", "AI", "AO", "DI", "DO", "RTD", "통신 IO", "케이블 수", "케이블 m", "분기 m", "멀티 m", "통신 m", "JB",
            "사다리 m", "사다리 발주 m", "트레이 면적 m²", "UG m", "UG 발주 m", "트렌치 줄", "트렌치 면적 m²", "Perforated m",
            "서포트(포스트)", "강재 kg", "강재 발주 kg", "클램프", "앵커", "조인트"]
TXT_KEYS = ["JB 크기", "케이블 사양", "JB 사양", "Perforated 폭"]


def words(base, m):
    """changes in words: 'JB 29 → 21 (-27.6%) · 케이블 414 → 398 (-3.9%)'."""
    out = []
    for k in NUM_KEYS:
        a, b = base.get(k, 0), m.get(k, 0)
        if a != b:
            pct = f" ({(b - a) / a * 100:+.1f}%)" if a else ""
            out.append(f"{k} {a:,} → {b:,}{pct}")
    if base.get("JB 크기") != m.get("JB 크기") and m.get("JB 크기"):
        out.append(f"JB 크기 {base['JB 크기']} → {m['JB 크기']}")
    for k in ("케이블 사양", "JB 사양", "Perforated 폭"):
        if base.get(k) != m.get(k):
            a = set(str(base.get(k, "")).split(" | "))
            b = set(str(m.get(k, "")).split(" | "))
            out.append(f"{k}: [{' | '.join(sorted(a - b))[:160]}] → [{' | '.join(sorted(b - a))[:160]}]")
    return " · ".join(out)


# ------------------------------------------------------------------ audit
def audit(rows, res, tr, sp, s, idx_map):
    v = s["values"]
    out = []

    def add(group, name, ok, count, detail, ex=""):
        out.append({"구분": group, "점검": name, "결과": ok, "수량": count, "내용": detail, "예시": ex})
    inst = [r for r in rows if r.kind == "instrument"]
    cabled = [r for r in inst if cable.classify(r, v)[0] is not None]
    skipped = Counter(cable.classify(r, v)[1] for r in inst if cable.classify(r, v)[0] is None)
    cables = res["cables"]
    by_no = {c["no"]: c for c in cables}
    # ---- 1. coverage
    miss_idx = [r.tag for r in cabled if r.tag not in idx_map]
    add("누락", "케이블이 필요한 IO 행 → 인덱스 JB·케이블 열", "OK" if not miss_idx else "NG", f"{len(cabled)}행",
        f"케이블 대상 {len(cabled)}행 중 인덱스 열이 빈 행 {len(miss_idx)}. 케이블 없는 행 {sum(skipped.values())} = "
        + ", ".join(f"{k} {n}" for k, n in skipped.most_common()), ", ".join(miss_idx[:5]))
    tags_in_cables = Counter()
    for c in cables:
        if c["kind"] == "분기":
            tags_in_cables[c["frm"]] += 1
        else:
            for t in c.get("tags") or []:
                tags_in_cables[t] += 1
    dup = [t for t, n in tags_in_cables.items() if n > 1 and not t.endswith("XXXX")]
    lost = [r.tag for r in cabled if r.tag not in tags_in_cables]
    add("누락", "IO 1점 = 케이블 하나에 정확히 한 번", "OK" if not lost and not dup else "주의",
        f"{len(tags_in_cables)}태그", f"케이블에 없는 IO {len(lost)}, 두 케이블에 들어간 태그 {len(dup)} "
        "(같은 태그 번호를 쓰는 트레인 복제·XXXX 태그는 제외)", ", ".join((lost + dup)[:5]))
    br = [c for c in cables if c["kind"] == "분기"]
    pt = {p["cable"]: p for p in tr["pt"]}
    nopt = [c["no"] for c in br if c["no"] not in pt]
    badlen = [c["no"] for c in br if c["no"] in pt and pt[c["no"]]["length"] != c["length"]]
    add("누락", "분기 케이블 → Perforated 트레이 (길이 같음)", "OK" if not nopt and not badlen else "NG", f"{len(br)}가닥",
        f"Perforated 없는 분기 {len(nopt)}, 길이 다른 것 {len(badlen)}", ", ".join((nopt + badlen)[:5]))
    ptbr = {p["pt"]: p for p in sp["pt"]}
    add("누락", "Perforated → 브래킷 (태그별)", "OK" if all(p["no"] in ptbr for p in tr["pt"]) else "NG", f"{len(tr['pt'])}개",
        f"브래킷 합 {sum(p['count'] for p in sp['pt']):,}개 = Σ ceil(PT 길이 ÷ {v['sp_pt_span']} m)")
    trunk = [c for c in cables if c["kind"] in KINDS_TRUNK]
    routed = [c for c in trunk if c["no"] in (tr.get("by_cable") or {})]
    add("누락", "간선 케이블 → 트레이·트렌치 경로", "OK" if len(routed) == len(trunk) else "주의",
        f"{len(routed)}/{len(trunk)}", f"경로 없는 간선 {len(trunk) - len(routed)} (PLOT 위치 못 찾음 = 가정 길이)",
        ", ".join(c["no"] for c in trunk if c not in routed)[:200])
    multi = Counter(c["frm"] for c in cables if c["kind"] == "멀티")
    jb_bad = [j["no"] for j in res["jbs"] if multi.get(j["no"]) != 1]
    add("누락", "JB 하나에 멀티 하나", "OK" if not jb_bad else "NG", f"JB {len(res['jbs'])}", f"멀티가 0 또는 2 이상인 JB {len(jb_bad)}",
        ", ".join(jb_bad[:5]))
    sup_segs = {g["seg"] for g in sp["segments"] if g["count"] > 0}
    noseg = [x["no"] for x in tr["segments"] if x["no"] not in sup_segs]
    add("누락", "트레이·트렌치 구간 → 서포트", "OK" if not noseg else "NG", f"{len(tr['segments'])}구간",
        f"서포트가 없는 구간 {len(noseg)}", ", ".join(noseg[:5]))
    # ---- 2. relations
    on_ladder = Counter()
    for sg in tr["segments"]:
        for no in sg["cables"]:
            on_ladder[by_no[no]["kind"] if no in by_no else "없음"] += 1
    add("관계", "사다리 트레이·트렌치에 분기(싱글) 케이블 없음", "OK" if not on_ladder.get("분기") else "NG",
        sum(on_ladder.values()), "구간에 실린 케이블 종류: " + ", ".join(f"{k} {n}" for k, n in on_ladder.items())
        + " (분기는 Perforated에만)")
    add("관계", "Perforated에는 분기 케이블만", "OK" if all(by_no.get(p["cable"], {}).get("kind") == "분기" for p in tr["pt"]) else "NG",
        len(tr["pt"]), "PT 하나 = 분기 케이블 하나 (JB → 계기)")
    lonely = [(sg["no"], t["no"], t["count"], t["cls"], sg["length"]) for sg in tr["segments"] for t in sg.get("trays") or []
              if t["count"] <= 2]
    lc = Counter(x[3] for x in lonely)
    add("관계", "사다리 트레이에 케이블 1~2가닥뿐인 트레이", "주의" if lonely else "OK", f"{len(lonely)}개 · {sum(x[4] for x in lonely)} m",
        "케이블 1~2가닥 때문에 사다리 트레이(300)를 따로 까는 곳: " + ", ".join(f"{k} {n}" for k, n in lc.items())
        + ". 통신(-C)이면 신호 트레이 칸막이(divider), 신호면 Perforated·컨듀잇이 맞을 수 있음 (결정 필요)",
        ", ".join(f"{b}({n})" for a, b, n, c, L in lonely[:8]))
    tray_cls = {t["no"]: t["cls"] for sg in tr["segments"] for t in sg.get("trays") or []}
    comm_legs = [(no, x["no"]) for no, bc in (tr.get("by_cable") or {}).items() if bc["kind"] == "통신"
                 for x in bc["legs"] if x["kind"] == "사다리"]
    wrong = [f"{a}→{b}" for a, b in comm_legs if v["sep_comm"] == "별도" and tray_cls.get(b) != "통신"]
    sig_wrong = [f"{no}→{x['no']}" for no, bc in (tr.get("by_cable") or {}).items() if bc["kind"] != "통신"
                 for x in bc["legs"] if x["kind"] == "사다리" and tray_cls.get(x["no"]) == "통신"]
    add("관계", "통신 케이블 ↔ 통신 트레이(-C), 신호 케이블 ↔ 신호·IS 트레이", "OK" if not wrong and not sig_wrong else "NG",
        f"{len(comm_legs)}구간", f"통신이 신호 트레이에 {len(wrong)}, 신호가 통신 트레이에 {len(sig_wrong)}", ", ".join((wrong + sig_wrong)[:5]))
    is_wrong = [f"{no}→{x['no']}" for no, bc in (tr.get("by_cable") or {}).items()
                for x in bc["legs"] if x["kind"] == "사다리" and v["sep_is"] == "별도"
                and (tray_cls.get(x["no"]) == "IS") != (by_no[no].get("is") == "IS")]
    add("관계", "IS 케이블 ↔ IS 트레이(-I) (별도일 때)", "OK" if not is_wrong else "NG", "", f"어긋난 것 {len(is_wrong)}",
        ", ".join(is_wrong[:5]))
    rack_types = {"RT", "RF", "RC"}
    seg_kind = {x["no"]: x["kind"] for x in tr["segments"]}
    bad_t = [x["no"] for x in sp["supports"] if (x["type"] in rack_types and all(seg_kind.get(q) != "랙" for q in x["segs"]))
             or (x["type"] in ("L", "T", "H") and all(seg_kind.get(q) != "AG" for q in x["segs"]))
             or (x["type"] in ("UB", "UM", "UP") and all(seg_kind.get(q) != "UG" for q in x["segs"]))]
    add("관계", "서포트 형식 ↔ 구간 종류 (랙 위 / AG / UG)", "OK" if not bad_t else "NG", len(sp["supports"]),
        f"형식과 구간 종류가 안 맞는 서포트 {len(bad_t)}", ", ".join(bad_t[:5]))
    j_bad = [j["no"] for j in sp["joints"] if seg_kind.get(j["seg"]) == "UG"]
    add("관계", "익스펜션 조인트는 사다리 트레이에만", "OK" if not j_bad else "NG", len(sp["joints"]), "")
    seglen = Counter()
    for sg in tr["segments"]:
        for no in sg["cables"]:
            seglen[no] += sg["length"]
    over = [no for no, bc in (tr.get("by_cable") or {}).items() if bc["ladder_m"] + bc["ug_m"] > by_no[no]["length"] + 1]
    add("관계", "케이블 길이 ≥ 탄 트레이·트렌치 길이", "OK" if not over else "NG", len(tr.get("by_cable") or {}),
        f"트레이 m가 케이블 길이보다 긴 케이블 {len(over)}", ", ".join(over[:5]))
    idx_bad = [t for t, m in idx_map.items() if m.get("multi") and m["multi"] not in by_no]
    add("관계", "인덱스 Multi Cable Number = 케이블 스케줄", "OK" if not idx_bad else "NG", len(idx_map), "", ", ".join(idx_bad[:5]))
    jb_tags = {t: j["no"] for j in res["jbs"] for t in j["tags"]}
    jb_mis = [t for t, m in idx_map.items() if m.get("jb") not in ("-", "", None) and jb_tags.get(t) != m["jb"]]
    add("관계", "인덱스 Junction Box = JB_List 태그", "OK" if not jb_mis else "NG", len(jb_tags), "", ", ".join(jb_mis[:5]))
    # ---- 3. materials add up
    spec_m = Counter()
    for c in cables:
        spec_m[c["spec"]] += c["length"]
    add("자재", "케이블 사양별 m 합 = 스케줄 합", "OK" if sum(spec_m.values()) == sum(c["length"] for c in cables) else "NG",
        f"{len(spec_m)}사양", f"{sum(spec_m.values()):,} m")
    lad = sum(x["length"] for x in tr["summary"]["ladder"])
    lad2 = sum(sg["length"] * t["tiers"] for sg in tr["segments"] for t in sg.get("trays") or [])
    add("자재", "사다리 트레이 폭별 m 합 = 구간 길이 × 단 수 합", "OK" if lad == lad2 else "NG", f"{lad:,} m", "")
    sc = sum(x["count"] for x in sp["summary"]["rows"] if x["type"] != "PB")
    add("자재", "서포트 형식별 개수 합 = 서포트 목록", "OK" if sc == len(sp["supports"]) else "NG", f"{sc:,}", "")
    return out


# ------------------------------------------------------------------ sensitivity
CONCEPT_EXPECT = {   # id -> (expected effect, note shown when the result matches / does not)
    "A01": "IO 거의 같음 (시스템별 스페어 반올림만), JB·패키지 멀티가 시스템 분리 없이 합쳐져 케이블 감소",
    "A02": "F&G 계기가 있으면 DCS/ESD로 이동",
    "A03": "벤더 경계 안 계기 IO 증가",
    "A04": "MCC 하드와이어 IO 감소, 통신 포트 증가, IRP 멀티 케이블이 통신 케이블로",
    "A05": "소프트 태그 행만 (HW IO 영향 없음)",
    "B01": "트레인 복제 IO 크게 감소",
    "B03": "ONLY ONE SHOWN 복제 IO 감소",
    "B06": "벤더 경계 안 계기가 일반 계기로 → IO 증가",
    "B07": "현장 지시계 행만 (IO 없음)", "B08": "PSV 행만 (IO 없음)", "B09": "써모웰 행만 (IO 없음)",
    "B10": "여러 시트 같은 태그가 각각 → IO 증가",
    "C01": "패키지 연결 방식 기본값 (Package_Concept에 행이 있는 패키지는 영향 없음)",
    "C04": "통신 포트 B 제거 → 통신 IO·광케이블 감소",
    "C05": "패키지당 ESD Trip DO", "C06": "패키지당 ESD Trip DI", "C07": "패키지당 DCS 상태 DI", "C08": "패키지당 DCS 명령 DO",
    "C09": "도면 미개발 패키지 Allowance / 물량 미상 행",
    "D01": "모터 Typical: MV·VFD는 AI(전류)·AO(속도) 추가",
    "D06": "모터당 Space Heater DI", "E01": "밸브당 LS DI 감소", "E02": "밸브당 SOV DO 증가", "E03": "PST DO 증가",
    "E04": "LS가 ESD로 → 시스템만 이동 (스페어 반올림 ±1)", "E05": "Reset DI 증가",
    "F01": "제어밸브 LS DI 감소", "F02": "제어밸브 AI 증가", "F03": "제어밸브 SOV DO 증가 (ESD JB 따로)",
    "G02": "SIS 전송기 이중·삼중 → AI 증가", "G03": "분석기 DI 감소",
    "G05": "TT AI가 RTD 채널로 (개수 같음)",
    "H01": "AI 스페어", "H02": "AO 스페어", "H03": "DI 스페어", "H04": "DO 스페어", "H05": "카드 수만 (IO 개수 영향 없음)",
}
REVIEW = {   # findings from the v12 audit (2026-11-26), keyed by checklist id
    "C01": ("검토 필요 (설계 판단)",
            "'전부 하드와이어'로 바꿔도 HW IO는 +11 뿐. 이유: ① 이 프로젝트 패키지 12개 중 6개는 Package_Concept 시트에 이미 "
            "'전부 하드와이어'로 행이 있어 C01(기본값)이 안 먹고, 나머지 6개(603·701·702·703·801×2)만 바뀜. ② PKG-HARDWIRED Typical이 "
            "혼합(PKG-HYBRID)과 같은 하드와이어 6점(ESD Trip DO·DI, DCS 상태 DI×3, 명령 DO)에서 통신 포트 2개만 뺀 것. "
            "그래서 통신 IO −12, 대신 늘어나는 건 벤더 경계 안에 그려진 계기(AI 7·DI 4)뿐. "
            "실무에서 '전부 하드와이어'는 통신으로 가던 상태·알람·아날로그 값이 전부 하드와이어로 바뀌는 것이라 IO가 크게 늘어야 함. "
            "제안: 체크리스트에 'C10 하드와이어 대체 신호 (패키지당 AI/DI/DO)'를 추가하거나 PKG-HARDWIRED Typical에 점수를 더 넣기 (결정 필요)"),
    "A01": ("정상 (설명)", "IO −1은 시스템별 스페어 반올림(ESD·DCS 따로 ceil → 합쳐서 ceil). 케이블 −63·−20.9 km는 패키지 제어반 멀티가 "
                           "시스템(ESD/DCS)별로 2가닥 → 1가닥으로 합쳐진 것 (118 → 59가닥, 39.6 → 19.8 km)과 ESD JB 4개가 DCS JB에 합쳐진 것"),
    "E04": ("정상 (설명)", "IO +1은 스페어 반올림: DCS DI 스페어 1 감소, ESD DI 스페어 2 증가 (시스템별 ceil(n × 20%))"),
    "G05": ("수정함 (v12.1)", "RTD로 바꾸면 HW IO가 −5 되던 것: 스페어가 AI/AO/DI/DO에만 붙고 RTD 채널에는 안 붙었음. RTD에도 AI 스페어율(H01)을 적용하도록 고침"),
    "A04": ("정상 (설명)", "MCC 하드와이어 120점(DI 60·DO 60)이 통신 2포트로. 케이블 수는 같음: IRP→룸 멀티페어 2가닥이 통신 케이블 2가닥으로 바뀜"),
    "C05": ("정상 (설명)", "패키지 단위 71개(트레인 복제 포함) × 1점. 케이블 수는 그대로: 패키지 제어반 멀티의 스페어 페어 안에 들어감 (멀티 규격만 커질 수 있음)"),
    "C07": ("정상 (설명)", "패키지 단위 71개 × 3점 = 213. 케이블 수 그대로 (멀티 규격 증가), 트레이 폭 약간 증가"),
    "F03": ("정상 (설명)", "제어밸브 SOV 25점이 ESD로 → ESD 분리 때문에 ESD JB 9개 새로 (멀티 9가닥 추가). 분기는 PLOT 위치가 있는 21가닥"),
    "A02": ("해당 없음", "이 도면에서 F&G 시스템 계기가 인식되지 않아 변화 없음 (시스템: DCS·ESD뿐)"),
    "D04": ("해당 없음", "비상정지 HS가 붙은 모터가 없음"), "D03": ("조건부", "VFD 모터(D01=MTR-VFD)일 때만 영향"),
    "H06": ("정상 (합침 v13.3)", "STEP 6 IS / Non-IS가 H06을 따름. '위험지역 계기 IS'는 위험지역 구분도가 없어 전부 Non-IS로 계산(가정) → 변화 없음이 맞음. "
                       "'아날로그는 IS' / '전부 IS'로 바꾸면 IS JB·멀티·트레이가 따로 생김"),
    "C03": ("정상 (합침 v13.3)", "STEP 6 패키지 PLC 통신 케이블이 C03을 따름 (광 / RS-485 / Cu Ethernet). IO는 그대로, 통신 케이블 사양·외경만 바뀜"),
    "C04": ("정상 (합침 v13.3)", "통신 포트 B(이중화) IO 제거 → 광케이블도 같이 감소. STEP 6 '통신 이중화'도 C04를 따름 (따로 바꾸는 곳 없음)"),
}


def concept_sensitivity(syms, proto, concept, notes, decs, s, am, lay, dec, obst, progress=None):
    r0, _ = assign(syms, proto, concept, notes, decisions=decs)
    base = metrics(r0, *chain(r0, s, am, lay, dec, obst, concept), concept)
    out = []
    items = [(iid, it) for iid, it in concept.items.items() if not iid.startswith("I")]
    for k, (iid, it) in enumerate(items):
        orig = it["value"]
        alts = list(concept.alternatives(iid))
        if not alts and isinstance(orig, (int, float)):
            alts = [0.0, 0.4] if iid in ("H01", "H02", "H03", "H04") else ([orig * 2, 0] if orig else [1])
            if iid == "H05":
                alts = [8, 32]
        for alt in alts:
            it["value"] = alt
            try:
                rr, _ = assign(syms, proto, concept, notes, decisions=decs)
                m = metrics(rr, *chain(rr, s, am, lay, dec, obst, concept), concept)
            except Exception as e:                                   # noqa: BLE001
                m = {"오류": repr(e)}
            finally:
                it["value"] = orig
            out.append({"id": iid, "name": it["name"], "cur": orig, "alt": alt, "m": m})
        if progress:
            progress(k + 1, len(items))
    return base, out


CABLE_ALTS = {"jb_sizes": ["60", "80", "24,48"], "multi_std": ["24,48"], "tray_widths": ["150,300,450,600,900"],
              "od_multi": ["2:10,4:12,6:15,10:18,12:20,16:22,20:24,24:26,30:28,36:30,40:32,48:34,60:38,80:44"],
              "od_single": ["1P:9,1T:10,2P:12,2C:10,3C:11"], "ug_widths": ["600,1200"],
              "sp_rules": ["L:300:1:2,T:600:1:2,H:1500:4:2"]}
SKIP_CABLE = {"FO_spec", "COMM_spec", "ETH_spec", "room_DCS", "room_ESD", "room_F&G", "jb_gas", "jb_material", "sp_member",
              "sp_typ_L", "sp_typ_T", "sp_typ_H", "sp_typ_RT", "sp_typ_UB", "sp_typ_PB", "sp_typ_RF"}
CABLE_EXPECT = {
    "jb_sizes": "JB 크기를 키우면 JB·멀티 수 감소, 멀티 규격 증가", "jb_spare": "스페어를 늘리면 JB 크기·수 증가",
    "term_spare": "단자 스페어 증가 → JB 크기 증가", "jb_radius": "반경을 키우면 JB 수 감소, 분기 길이 증가",
    "seg_system": "시스템 분리를 끄면 JB·멀티 감소", "seg_ad": "아날로그/디지털 혼합 → JB 감소",
    "tray_fill": "Fill을 올리면 트레이 폭(면적) 감소", "sp_span": "간격을 늘리면 서포트 감소", "sp_bay": "랙 빔 간격 → 클램프·T 서포트",
    "len_margin": "여유율 → 케이블 m 비례", "rack_el": "랙 높이 → 케이블 m (오르내림)", "trunk_off": "랙 밖 간선 AG ↔ UG",
    "main_route": "간선 방식", "sp_dt": "온도차를 키우면 조인트 증가", "sp_exp": "익스펜션 끄면 조인트 0",
}


def cable_sensitivity(rows, s, am, lay, dec, obst, concept, progress=None):
    base = metrics(rows, *chain(rows, s, am, lay, dec, obst), concept)
    out = []
    items = [it for g in cable.SCHEMA + cable.JB.SCHEMA + cable.TR.SCHEMA + cable.SP.SCHEMA for it in g["items"]
             if it["id"] not in SKIP_CABLE and not it.get("from_concept")]
    for k, it in enumerate(items):
        cur = s["values"][it["id"]]
        if it["type"] == "select":
            alts = [o for o in it["options"] if o != cur]
        elif it["type"] == "number":
            d = float(cur)
            alts = [0.5, 0.3] if it.get("pct") and d == 0.4 else ([round(d * 1.5, 3), round(d * 0.5, 3)] if d else [1])
            if it.get("pct") and d != 0.4:
                alts = [round(d + 0.1, 2), max(0.0, round(d - 0.1, 2))]
        else:
            alts = CABLE_ALTS.get(it["id"], [])
        for alt in alts:
            s2 = copy.deepcopy(s)
            s2["values"][it["id"]] = alt
            try:
                m = metrics(rows, *chain(rows, s2, am, lay, dec, obst), concept)
            except Exception as e:                                   # noqa: BLE001
                m = {"오류": repr(e)}
            out.append({"id": it["id"], "code": it.get("code", ""), "name": it["name"], "cur": cur, "alt": alt, "m": m})
        if progress:
            progress(k + 1, len(items))
    # supports off the rack only exist when the trunk runs AG off the rack: repeat those items on that basis
    if s["values"]["trunk_off"].startswith("UG"):
        sa = copy.deepcopy(s)
        sa["values"]["trunk_off"] = "AG 서포트"
        base_a = metrics(rows, *chain(rows, sa, am, lay, dec, obst), concept)
        out.append({"id": "trunk_off", "code": "(기준)", "name": "== 아래는 간선(랙 밖) AG 기준 ==", "cur": "UG 트렌치",
                    "alt": "AG 서포트", "m": base_a, "base": base})
        for it in [it for g in cable.SP.SCHEMA for it in g["items"] if it["id"] in ("sp_rules", "sp_type", "sp_span", "sp_tier_gap", "sp_arm_add")]:
            cur = sa["values"][it["id"]]
            alts = ([o for o in it["options"] if o != cur] if it["type"] == "select"
                    else CABLE_ALTS.get(it["id"], []) if it["type"] == "text" else [round(float(cur) * 1.5, 3), round(float(cur) * 0.5, 3)])
            for alt in alts:
                s3 = copy.deepcopy(sa)
                s3["values"][it["id"]] = alt
                m = metrics(rows, *chain(rows, s3, am, lay, dec, obst), concept)
                out.append({"id": it["id"], "code": it.get("code", "") + " (AG)", "name": it["name"] + " [간선 AG 기준]",
                            "cur": cur, "alt": alt, "m": m, "base": base_a})
    return base, out


# ------------------------------------------------------------------ the workbook
def _sheet(wb, title, cols, data, widths=None, wrap=()):
    from openpyxl.styles import Alignment, Font, PatternFill
    ws = wb.create_sheet(title)
    ws.append(cols)
    for c in ws[1]:
        c.font = Font(bold=True, color="FFFFFF")
        c.fill = PatternFill("solid", fgColor="0B3D91")
        c.alignment = Alignment(vertical="center", wrap_text=True)
    for r in data:
        ws.append(r)
    for i, w in enumerate(widths or [], 1):
        ws.column_dimensions[ws.cell(1, i).column_letter].width = w
    for col in wrap:
        for row in ws.iter_rows(min_row=2, min_col=col, max_col=col):
            row[0].alignment = Alignment(wrap_text=True, vertical="top")
    ws.freeze_panes = "A2"
    for row in ws.iter_rows(min_row=2):
        for c in row:
            if c.value in ("NG", "이상", "수정함 (v12.1)"):
                c.fill = PatternFill("solid", fgColor="FDE8EA")
            elif c.value in ("주의", "검토 필요 (설계 판단)", "미적용 + 중복", "기록용 + 중복", "정상 + 중복", "기준 바꿈"):
                c.fill = PatternFill("solid", fgColor="FFF4CC")
    return ws


def _verdict_concept(iid, base, m, alt):
    if "오류" in m:
        return "이상", m["오류"]
    if iid in REVIEW:
        return REVIEW[iid]
    from .concept import ITEM_STATUS
    st = ITEM_STATUS.get(iid)
    moved = any(base[k] != m[k] for k in NUM_KEYS)
    if st and not moved:
        return ("미적용 (표시됨)" if st[0] == "planned" else "기록용"), st[1]
    if not moved and iid in CONCEPT_EXPECT:
        return "정상 (물량 영향 없음)", CONCEPT_EXPECT[iid]
    if not moved and m["IO 행"] == base["IO 행"]:
        return "확인", "이 도면에 해당 계기·조건이 없어 변화 없음 (해당 계기가 있는 프로젝트에서 다시 확인)"
    return "정상", CONCEPT_EXPECT.get(iid, "")


COND = {   # settings that only act under another condition (why '변화 없음' is right here)
    "rack_width": "PLOT에 직접 그린 랙이 있을 때만 (지금 없음)",
    "len_field": "PLOT 위치를 못 찾은 계기만 (지금 전부 PLOT 위치)", "len_trunk": "PLOT 경로가 없는 간선만 (지금 전부 PLOT 경로)",
    "len_mcc": "IRP 위치가 없을 때만", "rtd_mode": "RTD 계기가 있을 때만 (G05 = TT 4-20mA)",
    "pkg_route": "벤더 경계 안 계기가 케이블 대상일 때 (지금 A03 OFF로 IO 제외)",
    "mcc_from": "IRP와 MCC 위치가 같은 건물이라 길이 차이 없음",
    "jb_spare": "JB 평균 채움 25%라 스페어를 줄여도 크기가 안 바뀜", "term_spare": "JB 평균 채움 25%라 스페어를 바꿔도 크기가 안 바뀜",
    "multi_std": "'멀티 = JB 크기'일 때는 안 씀", "st_max": "스테이션에 JB가 이미 최대 4개 이하",
    "reassign": "다시 배정해도 같은 결과로 수렴",
    "sep_is": "IS 케이블이 있을 때만 (지금 '전부 Non-IS')",
    "od_single": "분기 케이블은 Perforated(태그별 물량)라 외경을 쓰지 않음",
    "od_isos": "외경 차이가 트레이 폭 단계(300/600/900)를 넘지 못함", "od_fo": "외경 차이가 트레이 폭 단계를 넘지 못함",
    "od_rs485": "RS-485 케이블 없음 (광케이블)", "ug_widths": "트렌치 폭만 바뀜 (면적 열 참고)",
    "sp_rules": "간선이 AG일 때만 (아래 '간선 AG 기준' 행)", "sp_type": "간선이 AG일 때만 (아래 '간선 AG 기준' 행)",
    "sp_span": "간선이 AG일 때만 (아래 '간선 AG 기준' 행)", "sp_ask_kg": "질문 카드 표시 기준 (물량 영향 없음)",
    "sp_marker": "UG 방식 = 직매설일 때만", "sp_pit": "UG 방식 = 덕트 뱅크일 때만", "sp_clamp": "클램프 열 참고",
    "sp_anchor": "앵커 열 참고", "sp_fit_pct": "강재 발주 kg 열 참고", "tray_fit": "사다리 발주 m 열 참고",
    "ug_fit": "UG 발주 m 열 참고", "pt_width": "Perforated 폭만 바뀜", "group_by": "Area 매핑표가 유닛과 같게 되어 있음", "is_rule": "전부 IS면 모두 같은 IS 트레이라 분리 수가 같음 (JB 번호에 I만 붙음)",
    "jb_ex_is": "IS JB가 없음 (전부 Non-IS)", "jb_entry": "JB_List 표기만 (물량 영향 없음)", 
    "comm_pkg": "케이블 종류만 바뀜 (사양 열)", "plot_use": "",
    "RTD_form": "RTD 계기가 없음 (G05 = TT 4-20mA)", "RTD_sq": "RTD 계기가 없음 (G05 = TT 4-20mA)",
    "RTD_shield": "RTD 계기가 없음 (G05 = TT 4-20mA)", "gland_spare": "글랜드 수 표기만 (JB_List)",
}


def _verdict_cable(it_id, base, m, ag=False):
    if "오류" in m:
        return "이상", m["오류"]
    moved = any(base[k] != m[k] for k in NUM_KEYS)
    txt = [k for k in TXT_KEYS if base.get(k) != m.get(k)]
    if it_id == "jb_sizes" and base["JB"] == m["JB"] and "JB 크기" in txt:
        return "정상 (설명)", ("JB 수는 반경·분리 기준으로 묶을 때 정해지고(평균 채움 25%), 크기만 커짐 → 멀티도 같은 크기로 커져 "
                             "트레이 단면적·폭이 늘어남. JB 수를 줄이려면 'JB 반경'·'작은 JB 합치기'를 키워야 함")
    if not moved and txt:
        return "사양만 바뀜", "수량은 같고 " + ", ".join(txt) + "만 바뀜"
    if not moved:
        if ag:
            return "변화 없음", "형식 규칙표의 스팬(3 m)이 이 상한보다 작아 그대로 (작게 할 때만 영향)"
        return "변화 없음", COND.get(it_id) or "이 프로젝트 조건에서는 영향 없음"
    return "정상", CABLE_EXPECT.get(it_id, "")


def explain_io(rows, concept):
    inst = [r for r in rows if r.kind != "function"]
    hw = [r for r in inst if r.io in ("AI", "AO", "DI", "DO", "TC/RTD") and r.system not in ("-", "?")]
    org = Counter(r.origin for r in hw)
    out = []
    txt = {"도면인식": "P&ID에서 인식한 계기 심볼 × IO 규칙표(io_rules.yaml) 1점",
           "Typical파생": "밸브·계기 Typical 전개 (예: 차단밸브 1개 = SOV DO 1 + LS DI 2(E01) …)",
           "트레인복제": "트레인 노트(B01) 시트의 IO × (트레인 수 − 1)",
           "대수복제": "'ONLY ONE IS SHOWN' 노트(B03) × (대수 − 1)",
           "장비Typical": "모터 Typical (D01: DOL = Run DI·Fault DI·Start DO·Stop DO …) × 모터 수",
           "패키지": "패키지 Typical (C01/Package_Concept) × 패키지 단위 수 + C09 Allowance",
           "스페어": "시스템·IO 종류별 ceil(IO 수 × 스페어율 H01~H04)"}
    for o, n in org.most_common():
        io = Counter(r.io for r in hw if r.origin == o)
        out.append([o, n, " · ".join(f"{k} {v}" for k, v in io.most_common()), txt.get(o, "")])
    out.append(["합계", len(hw), "", f"HW IO = {len(hw)}"])
    sp = []
    cnt = Counter((r.system, r.io) for r in hw if r.origin != "스페어")
    rates = {"AI": "H01", "AO": "H02", "DI": "H03", "DO": "H04", "TC/RTD": "H01"}
    for (sy, io), n in sorted(cnt.items()):
        rate = concept.num(rates.get(io, "H01"), 0)
        sp.append([f"{sy} {io}", n, f"{rate:.0%}", math.ceil(n * rate - 1e-9),
                   f"ceil({n} × {rate:.0%}) = ceil({n * rate:.2f}) = {math.ceil(n * rate - 1e-9)}"])
    return out, sp


def explain_cable(res, s):
    v = s["values"]
    cables = res["cables"]
    out = []
    by = Counter(c["kind"] for c in cables)
    m = Counter()
    for c in cables:
        m[c["kind"]] += c["length"]
    rule = {"분기": f"IO 1점 = 1가닥 (계기 → JB). 사양 = IO별 설정 (AI {v['AI_form']} × {v['AI_sq']}SQ …)",
            "멀티": "JB 1개 = 멀티 1가닥 (JB → 룸). 페어 수 = JB 크기 (멀티 = JB 크기 설정)",
            "패키지": "패키지 제어반(UCP) → 룸: 패키지·시스템·신호 묶음마다, 페어 = ceil(점수 × (1+스페어)) 위 표준 규격",
            "MCC": "IRP → 룸 멀티페어 (모터 신호 묶음)", "통신": "통신 포트마다 1가닥 (이중화) 또는 패키지당 1가닥"}
    for k in ("분기", "멀티", "패키지", "MCC", "통신"):
        if by[k]:
            out.append([k, by[k], m[k], round(m[k] / by[k], 1), rule[k]])
    out.append(["합계", len(cables), sum(m.values()), "", ""])
    ex = []
    marg, step = float(v["len_margin"]), int(v["len_round"])
    for kind in ("분기", "멀티", "패키지", "통신"):
        c = next((c for c in cables if c["kind"] == kind and "PLOT" in c["basis"]), None)
        if not c:
            continue
        import re
        nums = re.findall(r"([0-9.]+)m", c["basis"])
        raw = sum(float(x) for x in nums[:1] + nums[-1:]) if kind == "분기" else None
        if kind == "분기" and len(nums) >= 2:
            h, z = float(nums[0]), float(nums[1])
            calc = f"(수평 {h:g} + 높이 {z:g}) × (1 + 여유 {marg:.0%}) = {(h + z) * (1 + marg):.1f} → {step} m 올림 = {c['length']} m"
        else:
            hm = re.search(r"([0-9.]+)m \(", c["basis"])
            zm = re.search(r"높이 ([0-9.]+)m", c["basis"])
            if hm and zm:
                h, z = float(hm.group(1)), float(zm.group(1))
                calc = f"(경로 {h:g} + 높이 {z:g}) × (1 + {marg:.0%}) = {(h + z) * (1 + marg):.1f} → {step} m 올림 = {c['length']} m"
            else:
                calc = ""
        _ = raw
        ex.append([kind, c["no"], c["spec"], c["length"], calc, c["basis"]])
    hz = []
    hz.append(["분기 높이", f"계기 EL {v['inst_el']} → {v['no_rack']} 높이 → JB EL {v['jb_el']}",
               "|계기 EL − 경로 높이| + |경로 높이 − JB EL|"])
    hz.append(["간선 높이", f"JB EL {v['jb_el']} → 랙 EL {v['rack_el']} (랙 구간) / UG −{v['ug_depth']} → 룸 EL {v['room_el']}",
               "출발 오르내림 + 랙↔랙 밖 바뀔 때마다 |랙 EL − 랙 밖 높이| + 도착 오르내림"])
    hz.append(["수평", f"분기 = 직각 거리 × 우회계수 {v['route_factor']} (최소 {v['field_min']} m), 간선 = PLOT 1 m 격자 경로 (랙 1/m, 랙 밖 {v['rack_pref']}/m)", ""])
    hz.append(["여유·올림", f"× (1 + {marg:.0%}) 후 {step} m 단위 올림", "단말·처짐·우회 여유"])
    return out, ex, hz


def explain_jb(res, s):
    from . import jb as JB
    v = s["values"]
    tab = JB.size_table(v)
    rows = [[f"{t['size']}P", t["max_in"], t["multi"], t["spare_cable"], t["spare_term"],
             f"계기 n가닥 → 멀티 ≥ ceil(n × (1+{float(v['jb_spare']):.0%})), 단자 ≥ ceil(n × (1+{float(v['term_spare']):.0%})) → "
             f"{t['size']}P에 최대 {t['max_in']}가닥"] for t in tab]
    grp = Counter((j["kind"], j["system"], j["cls"], j["is"]) for j in res["jbs"])
    g = [[k[0], k[1], k[2], k[3], n, sum(j["used"] for j in res["jbs"] if (j["kind"], j["system"], j["cls"], j["is"]) == k),
          " · ".join(f"{j['no']} {j['used']}/{j['size']}" for j in res["jbs"] if (j["kind"], j["system"], j["cls"], j["is"]) == k)[:300]]
         for k, n in grp.most_common()]
    return rows, g


def explain_tray(tr, s):
    v = s["values"]
    h, f = float(v["tray_height"]), float(v["tray_fill"])
    out = []
    for w in cable.TR.widths(v):
        out.append([f"사다리 {w}", f"{w} × {h:g} × {f:.0%} = {w * h * f:,.0f} mm²", "케이블 단면적 합(Σ π/4·OD²)이 이 값 이하인 가장 작은 폭"])
    ws = cable.TR.ug_widths(v)
    for w in ws:
        out.append([f"트렌치 {w}", f"{w} × {float(v['ug_height']):g} × {float(v['ug_fill']):.0%} = {w * float(v['ug_height']) * float(v['ug_fill']):,.0f} mm²", "트렌치 단당"])
    ex = []
    segs = sorted((sg for sg in tr["segments"] if sg.get("trays")), key=lambda sg: -max(t["area"] for t in sg["trays"]))[:6]
    for sg in segs:
        for t in sg["trays"]:
            cap = t["width"] * h * f * t["tiers"]
            ex.append([t["no"], sg["kind"], t["count"], t["area"],
                       f"Σ π/4·OD² = {t['area']:,} mm² → {t['width']}{' × ' + str(t['tiers']) + '단' if t['tiers'] > 1 else ''} "
                       f"(용량 {cap:,.0f}) · Fill {t['fill']:.0%} · 길이 {sg['length']} m × {t['tiers']}단 = {sg['length'] * t['tiers']} m"])
    return out, ex


def explain_support(sp, tr, s):
    v = s["values"]
    from . import support as SPm
    out = [["AG 서포트 간격", f"형식 규칙표 스팬과 상한 {v['sp_span']} m 중 작은 값",
            "구간마다: 고정점(구간 끝 / 피팅 서포트 / 조인트 서포트) 사이를 간격 이하로 균등 → 개수 = Σ ceil(고정점 간 거리 ÷ 간격) − 1 + 고정점"],
           ["피팅 (NEMA VE 2)", f"반경 {v['sp_fit_r']} mm + 최대 {v['sp_fit_d']} mm",
            f"꺾임·분기·랙 출입 점에서 {float(v['sp_fit_r']) / 1000 + float(v['sp_fit_d']) / 2000:.1f} m 떨어진 곳에 1개씩 (점 자체에는 없음)"],
           ["익스펜션 조인트", f"{float(v['sp_gap']):g} mm ÷ ({SPm.ALPHA.get(v['sp_mat'], 11.7e-6) * 1e6:g}e-6 × {v['sp_dt']}°C) = {SPm.exp_spacing(v):.1f} m",
            "직선 구간 길이 L > 최대 간격이면 조인트 = ceil(L ÷ 최대 간격) − 1, 양쪽 0.3 m에 서포트, 조인트 사이 가운데 = 앵커"],
           ["랙 위", f"랙 빔 {v['sp_bay']} m마다", "1단 = 클램프 (트레이 1단당 2개), 2단 이상 = 랙 위 T 서포트"],
           ["트렌치 브래킷", f"{v['sp_ug_span']} m마다", "구간 양 끝 포함: ceil(L ÷ 간격) + 1 (이웃 구간과 끝 공유)"],
           ["Perforated 브래킷", f"{v['sp_pt_span']} m마다", "태그마다 ceil(PT 길이 ÷ 간격)"]]
    mem = []
    for t in ("L", "T", "H", "RT", "RF", "UB"):
        w = 300 if t in ("L", "UB") else 600
        mm, bp, an, cl = SPm.members(t, w, 1 if t != "RT" else 2, float(v["sup_el"]) if t in ("L", "T", "H") else 0, v)
        mem.append([SPm.TYPES[t], f"폭 {w}{' 2단' if t == 'RT' else ''}", round(mm, 2), round(mm * float(v["sp_kgm"]), 1), bp, an, cl,
                    {"L": "포스트(EL) + 팔(폭+여유)", "T": "포스트 + 팔 ×2", "H": "포스트 ×2 + 빔", "RT": "랙 빔 위 포스트 + 위 단 팔",
                     "RF": "빔 사이 보조 빔 + 0.3 m", "UB": "브래킷 길이 × 단"}[t]])
    ex = []
    segs = sorted(sp["segments"], key=lambda g: -g["length"])[:8]
    for g in segs:
        ex.append([g["seg"], g["kind"], g["length"], g["span"], g["count"], g["joints"],
                   f"{g['length']} m ÷ {g['span']} m 간격" + (f" + 조인트 {g['joints']}" if g["joints"] else "")
                   + f" → {g['count']}개 ({', '.join(f'{SPm.TYPES[k]} {n}' for k, n in g['types'].items())})"])
    return out, mem, ex


def bom(res, tr, sp, s):
    v = s["values"]
    out = []
    spec = Counter()
    for c in res["cables"]:
        spec[(c["kind"], c["spec"])] += c["length"]
    for (k, sp_), m in sorted(spec.items()):
        n = sum(1 for c in res["cables"] if c["spec"] == sp_ and c["kind"] == k)
        out.append(["케이블", f"{sp_} ({k})", "m", m, f"{n}가닥 길이 합 (여유 {float(v['len_margin']):.0%}·{v['len_round']} m 올림 포함)"])
    jb = Counter((j["size"], j.get("material", ""), j.get("ex", "")) for j in res["jbs"])
    for (sz, mat, ex), n in sorted(jb.items()):
        out.append(["JB", f"{sz} {mat} {ex}".strip(), "EA", n, "JB_List"])
    for x in tr["summary"]["ladder"]:
        out.append(["사다리 트레이", f"{x['cls']} {x['width']}×{int(float(v['tray_height']))}", "m", x["order"],
                    f"구간 길이 × 단 합 {x['length']} m × (1 + 피팅·할증 {float(v['tray_fit']):.0%})"])
    for x in tr["summary"].get("ug_by_width") or []:
        out.append(["UG 트렌치", f"폭 {x['width']}" + (f" × {x['tiers']}단" if x["tiers"] > 1 else ""), "m", x["order"],
                    f"{x['length']} m × (1 + {float(v['ug_fit']):.0%})"])
    out.append(["Perforated 트레이", f"{v['pt_width']}", "m", tr["summary"]["pt_m"], f"분기 케이블 {tr['summary']['pt_n']}가닥 길이 = PT 길이"])
    for x in sp["summary"]["rows"]:
        out.append(["서포트", x["name"] + (f" ({x['typical']})" if x["typical"] else ""), "EA", x["count"],
                    f"강재 {x['kg']:,} kg (할증 {x['kg_order']:,} kg)"])
    smm = sp["summary"]
    out.append(["서포트 자재", "강재 합계 (할증 포함)", "kg", smm["kg_order"], f"{v['sp_member']} {v['sp_kgm']} kg/m 등"])
    out.append(["서포트 자재", "앵커 볼트", "EA", sum(x["anchors"] for x in smm["rows"]), ""])
    out.append(["서포트 자재", "클램프 / 가이드", "EA", smm["clamps"], f"앵커 지점 {smm['anchor_pts']} · 가이드 지점 {smm['guide_pts']}"])
    out.append(["트레이 부속", "익스펜션 스플라이스 플레이트 세트", "SET", smm["joint_plates"], f"조인트 {smm['joints']}개 × 트레이 단 수"])
    return out


def write(path: Path, project: str, rows, concept, s, res, tr, sp, idx_map, sens_c=None, sens_k=None):
    from openpyxl import Workbook
    wb = Workbook()
    wb.remove(wb.active)
    base = metrics(rows, res, tr, sp, concept)
    au = audit(rows, res, tr, sp, s, idx_map)
    ng = sum(1 for a in au if a["결과"] == "NG")
    warn = sum(1 for a in au if a["결과"] == "주의")
    summ = [["프로젝트", project], ["만든 시각", time.strftime("%Y-%m-%d %H:%M")],
            ["점검 결과", f"NG {ng} · 주의 {warn} · OK {len(au) - ng - warn} (1·2 시트)"]]
    summ += [[k, base[k]] for k in ["IO 행", "HW IO", "AI", "AO", "DI", "DO", "RTD", "통신 IO", "케이블 수", "케이블 m", "분기 m",
                                    "멀티 m", "통신 m", "JB", "JB 크기", "사다리 m", "트레이 면적 m²", "UG m", "트렌치 줄",
                                    "Perforated m", "서포트(포스트)", "강재 kg", "조인트"]]
    summ += [["", ""], ["읽는 법", "1 누락점검 → 2 관계점검 → 3~7 항목별 산출식(실제 숫자 대입) → 8 자재 → 9 체크리스트를 바꾸면 무엇이 얼마나 → 10 검토 의견"]]
    _sheet(wb, "0_요약", ["항목", "값"], summ, [22, 90])
    _sheet(wb, "1_누락점검", ["구분", "점검", "결과", "수량", "내용", "예시"],
           [[a["구분"], a["점검"], a["결과"], a["수량"], a["내용"], a["예시"]] for a in au if a["구분"] in ("누락", "자재")],
           [8, 42, 7, 12, 80, 40], wrap=(5,))
    _sheet(wb, "2_관계점검", ["구분", "점검", "결과", "수량", "내용", "예시"],
           [[a["구분"], a["점검"], a["결과"], a["수량"], a["내용"], a["예시"]] for a in au if a["구분"] == "관계"],
           [8, 42, 7, 12, 80, 40], wrap=(5,))
    io_rows, spare = explain_io(rows, concept)
    ws = _sheet(wb, "3_IO근거", ["산출구분", "HW IO", "IO 종류별", "산출식"], io_rows, [14, 8, 40, 70], wrap=(4,))
    ws.append([])
    ws.append(["스페어 (시스템·IO별)", "IO 수", "스페어율", "스페어", "계산"])
    for r in spare:
        ws.append(r)
    c_rows, c_ex, c_hz = explain_cable(res, s)
    ws = _sheet(wb, "4_케이블근거", ["구분", "가닥", "m", "평균 m", "산출식"], c_rows, [10, 8, 10, 9, 90], wrap=(5,))
    ws.append([])
    ws.append(["길이 계산 예", "케이블", "사양", "m", "계산 (숫자 대입)", "프로그램 근거 문구"])
    for r in c_ex:
        ws.append(r)
    ws.append([])
    ws.append(["길이 규칙", "값", "식"])
    for r in c_hz:
        ws.append(r)
    j_tab, j_grp = explain_jb(res, s)
    ws = _sheet(wb, "5_JB근거", ["JB 크기", "최대 계기", "멀티", "케이블 스페어", "단자 스페어", "식"], j_tab, [9, 10, 8, 12, 12, 90], wrap=(6,))
    ws.append([])
    ws.append(["구분", "시스템", "신호", "IS", "JB 수", "계기 합", "JB별 사용/크기"])
    for r in j_grp:
        ws.append(r)
    ws.append([])
    ws.append(["묶는 법", "STEP 6 분리 기준(유닛·시스템·아날로그/디지털·RTD·IS)으로 나눈 묶음 안에서, PLOT에 위치가 있으면 반경 "
               f"{s['values']['jb_radius']} m 안 계기끼리 (서쪽·북쪽부터), JB 크기 = 위 표에서 담는 가장 작은 크기"])
    t_rule, t_ex = explain_tray(tr, s)
    ws = _sheet(wb, "6_트레이근거", ["폭", "용량 (폭 × 높이 × Fill)", "규칙"], t_rule, [14, 36, 70])
    ws.append([])
    ws.append(["Tray No", "종류", "케이블", "단면적 mm²", "계산 (숫자 대입)"])
    for r in t_ex:
        ws.append(r)
    ws.append([])
    ws.append(["외경 (mm)", "사양 글자에서 페어·코어 수를 읽어 STEP 8 '케이블 외경' 표 (외장형 카탈로그 일반값)"])
    for spec_ in sorted({c["spec"] for c in res["cables"]})[:30]:
        d = cable.TR.od(spec_, s["values"])
        ws.append([spec_, f"OD {d:.1f} mm → π/4 × {d:.1f}² = {cable.TR.area(d):,.0f} mm²"])
    ws.append([])
    ws.append(["트렌치 번호", "분기하거나 폭이 바뀔 때만 새 번호 (트렌치 한 줄) - Trench_Runs"])
    for r in (tr["summary"].get("ug_runs") or [])[:40]:
        ws.append([r["run"], f"폭 {r['width']} · {r['length']} m · 구간 {r['parts']} · 케이블 {r['cables']}"])
    s_rule, s_mem, s_ex = explain_support(sp, tr, s)
    ws = _sheet(wb, "7_서포트근거", ["항목", "값", "식"], s_rule, [18, 40, 90], wrap=(3,))
    ws.append([])
    ws.append(["형식", "예", "주 부재 m", "kg", "베이스", "앵커", "클램프", "구성"])
    for r in s_mem:
        ws.append(r)
    ws.append([])
    ws.append(["구간 예 (긴 순)", "종류", "길이", "간격", "서포트", "조인트", "계산"])
    for r in s_ex:
        ws.append(r)
    _sheet(wb, "8_자재", ["구분", "규격", "단위", "수량", "근거"], bom(res, tr, sp, s), [16, 40, 6, 12, 70], wrap=(5,))
    if sens_c:
        b, items = sens_c
        data = []
        for x in items:
            m = x["m"]
            vd, why = _verdict_concept(x["id"], b, m, x["alt"])
            data.append([x["id"], x["name"], str(x["cur"]), str(x["alt"]), vd,
                         words(b, m) if "오류" not in m else m["오류"], why,
                         *[(m.get(k, 0) - b[k]) if "오류" not in m else "" for k in ("HW IO", "케이블 수", "케이블 m", "JB", "사다리 m", "서포트(포스트)")]])
        _sheet(wb, "9_체크리스트영향_IO", ["No", "항목", "현재", "바꾸면", "판정", "무엇이 얼마나 (전 → 후, %)", "설명",
                                       "Δ HW IO", "Δ 케이블", "Δ 케이블 m", "Δ JB", "Δ 사다리 m", "Δ 서포트"],
               data, [6, 34, 16, 18, 14, 80, 70, 8, 8, 10, 6, 10, 8], wrap=(6, 7))
    if sens_k:
        b, items = sens_k
        data = []
        for x in items:
            m, bb = x["m"], x.get("base") or b
            vd, why = _verdict_cable(x["id"], bb, m, "(AG)" in str(x["code"]))
            if x["code"] == "(기준)":
                vd, why = "기준 바꿈", "랙 밖 간선을 AG로 두면 (아래 행들은 이 값에서의 변화)"
            data.append([x["code"], x["name"], str(x["cur"]), str(x["alt"]), vd,
                         words(bb, m) if "오류" not in m else m["오류"], why])
        _sheet(wb, "9_설정영향_케이블~서포트", ["No", "항목", "현재", "바꾸면", "판정", "무엇이 얼마나 (전 → 후, %)", "설명"],
               data, [7, 34, 22, 22, 10, 90, 50], wrap=(6, 7))
    rv = [[k, *REVIEW[k]] for k in REVIEW]
    pk = [c for c in res["cables"] if c["kind"] == "패키지"]
    tot = sum(c["length"] for c in res["cables"]) or 1
    if pk:
        avg_pts = sum(c.get("pairs_used", 0) for c in pk) / len(pk)
        rv.append(["물량", "검토 필요 (물량 영향 큼)",
                   f"패키지 제어반 → 룸 멀티 {len(pk)}가닥 · {sum(c['length'] for c in pk):,} m = 전체 케이블 m의 "
                   f"{sum(c['length'] for c in pk) / tot:.0%}. 패키지 단위(트레인 복제 포함)마다 시스템·신호별로 한 가닥씩이고 "
                   f"가닥당 평균 {avg_pts:.1f}점만 씀 ({pk[0]['spec'].split(' x')[0]} 최소 규격). 실무에서는 패키지 근처 마샬링 JB에 모아 "
                   "큰 멀티로 보내는 경우가 많음 → STEP 6 '패키지 인터페이스 신호 = Skid JB 경유'(JB가 크게 늘어남) 외에 "
                   "'여러 패키지 단위를 한 멀티로 묶기' 옵션이 필요할 수 있음 (결정 필요)"])
    lon = [(t["cls"], sg["length"]) for sg in tr["segments"] for t in sg.get("trays") or [] if t["count"] <= 2]
    rv += [["관계", "주의", f"케이블 1~2가닥뿐인 사다리 트레이 {len(lon)}개 · {sum(L for _, L in lon)} m "
                         f"(통신 -C {sum(1 for c, _ in lon if c == '통신')}개). 통신 별도 트레이(T04)가 광케이블 2~4가닥 때문에 300 사다리를 "
                         "따로 까는 결과 (사다리 통신 300 = 115 m). 칸막이(divider)로 신호 트레이에 같이 넣는 옵션, "
                         "또는 'n가닥 이하는 Perforated/컨듀잇' 규칙을 넣을지 결정 필요 (T04 '같이'로 바꾸면 사다리 −115 m)"],
           ["누락", "보완함 (v12.1)", "인덱스 파일에 트레이·트렌치·Perforated·서포트·길이·근거가 없었음 → 39~46열 추가 "
                                   "(Field/Multi Cable Length, Perforated Tray·브래킷, Ladder Tray·UG Trench 경로, 사다리/UG m, 케이블 산출 근거)"]]
    _sheet(wb, "10_검토의견", ["항목", "판정", "내용"], rv, [8, 22, 140], wrap=(3,))
    from . import protocol
    protocol.write_sheet(wb, protocol.build(concept, s["values"]), _sheet)
    # relations per tag / per segment
    tc = []
    for t, mm in sorted(idx_map.items()):
        tc.append([t, mm.get("cable", ""), mm.get("cable_spec", ""), mm.get("cable_len", ""), mm.get("pt", ""), mm.get("pt_br", ""),
                   mm.get("jb", ""), mm.get("multi", ""), mm.get("multi_spec", ""), mm.get("multi_len", ""),
                   mm.get("ladder", ""), mm.get("trench", ""), mm.get("lad_ug", "")])
    _sheet(wb, "Tag_Chain", ["Tag", "Field Cable", "Spec", "m", "Perforated", "브래킷", "JB", "Multi", "Multi Spec", "Multi m",
                             "Ladder Tray (순서)", "UG Trench (순서)", "사다리/UG m"],
           tc, [18, 18, 20, 6, 16, 6, 16, 18, 22, 7, 60, 40, 10])
    supc = Counter()
    for x in sp["supports"]:
        for q in x["segs"]:
            supc[(q, x["type"])] += 1
    jc = Counter(j["seg"] for j in sp["joints"])
    by_no = {c["no"]: c for c in res["cables"]}
    ch = []
    for sg in tr["segments"]:
        kinds = Counter(by_no[c]["kind"] for c in sg["cables"] if c in by_no)
        ch.append([sg["no"], sg.get("run", ""), sg["kind"],
                   ", ".join(f"{t['no']} {t['width']}×{t['tiers']}" for t in sg.get("trays") or []) or (f"트렌치 {sg.get('width')}" if sg["kind"] == "UG" else ""),
                   sg["length"], len(sg["cables"]), " · ".join(f"{k} {n}" for k, n in kinds.items()),
                   ", ".join(f"{cable.SP.TYPES[t]} {n}" for (q, t), n in supc.items() if q == sg["no"]), jc.get(sg["no"], 0),
                   ", ".join(sg["cables"])[:400]])
    _sheet(wb, "Tray_Chain", ["구간", "트렌치 줄", "종류", "트레이", "m", "케이블 수", "케이블 종류", "서포트", "조인트", "케이블"],
           ch, [14, 12, 6, 30, 6, 8, 24, 40, 6, 80])
    path.parent.mkdir(parents=True, exist_ok=True)
    wb.save(path)
    return {"ng": ng, "warn": warn, "checks": len(au)}
