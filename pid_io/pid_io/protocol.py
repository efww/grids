"""규칙 프로토콜: the fixed protocol the design office wants - FACT data + checklist (design concept) -> rules.

Every checklist item (STEP 3, concept.xlsx) and every setting (STEP 6~10, cable.json) becomes one rule row:
  rule_id   stable id (checklist code, or group code + setting id)
  reads     which fact data it works on (P&ID symbols, index rows, PLOT, 3D model, catalogue ...)
  changes   which outputs move when the value changes (IO, cable, JB, tray, trench, support, 3D)
  formula   how (the same formula the 산출근거 workbook shows with numbers)
  status    적용 / 기록용 / 미적용(구현 전) / 조건부
  value, default, options
The 산출근거 workbook's sheet 9 gives, for each rule, what actually moved on this project and by how much.
A company AI (or a person) can apply the rules to the same facts and must get the same quantities.
"""
from __future__ import annotations

FACTS = [
    {"id": "F-PID", "name": "P&ID (벡터 PDF)", "fields": "계기 심볼·태그·기능코드·신호선·벤더 경계·노트·시트 번호", "kind": "팩트 (도면 인식)"},
    {"id": "F-IDX", "name": "Instrument Index 양식", "fields": "열 순서·머리글 (A~AA) + 34~46열 케이블·트레이", "kind": "회사 표준"},
    {"id": "F-PLOT", "name": "PLOT PLAN (벡터 PDF)", "fields": "장비 태그 위치·축척·랙(기둥)·장애물·제어실·IRP", "kind": "팩트 (도면 인식, 일부 추정)"},
    {"id": "F-3D", "name": "배관 3D 모델 (Three.js)", "fields": "장비 박스·랙(방향·위치·단 EL·케이블 단 EL)·참조 객체", "kind": "팩트 (배관팀 모델, 개념)"},
    {"id": "F-CAT", "name": "케이블 카탈로그 외경", "fields": "사양별 외경 (BS 5308 / EN 50288-7 외장형)", "kind": "공개 자료 일반값"},
    {"id": "F-NEMA", "name": "NEMA VE 2", "fields": "피팅 600 mm, 익스펜션 스플라이스 간격", "kind": "표준"},
    {"id": "F-OTHER", "name": "다른 공종 3D (전기 등)", "fields": "트레이 중심선·박스", "kind": "팩트 (있을 때)"},
]

# setting group -> (rule prefix, reads, changes, formula)
GROUP_RULE = {
    "branch": ("R-CB-SPEC", "F-PID (IO 종류)", "케이블 사양 · 트레이 단면적", "IO 종류별 구성 × 굵기 × 차폐 = 분기 케이블 사양"),
    "multi": ("R-CB-MULTI", "STEP 7 JB", "멀티 사양 · 트레이 단면적", "멀티 페어 = JB 크기 (또는 사용+스페어 최소 규격) × 굵기 × 차폐"),
    "seg": ("R-SEG", "F-PID (시스템·신호·IS·벤더)", "JB 수 · 멀티 수 · 케이블 m · 트레이", "분리 기준 조합마다 따로 묶음 → 묶음 안에서 JB"),
    "route": ("R-RT", "F-PLOT · F-3D", "케이블 길이 · 트레이/트렌치 종류 · 높이", "PLOT 1 m 격자 경로(랙 1/m, 랙 밖 '랙 우선도'/m) + 높이 오르내림"),
    "len": ("R-LEN", "-", "케이블 길이 (PLOT 위치 없을 때)", "가정 길이 × (1 + 여유) → 올림"),
    "jbsize": ("R-JB-SIZE", "STEP 6 묶음", "JB 크기 · 멀티 크기 · JB 수", "ceil(n×(1+케이블 스페어)) ≤ M, ceil(n×(1+단자 스페어)) ≤ T, 가장 작은 T"),
    "jbplace": ("R-JB-PLACE", "F-PLOT (랙 기둥·장비 외곽)", "JB 위치 · 분기·멀티 길이", "반경 안 계기 묶음 → 랙 기둥/장비 가장자리 자리, 비용 = 분기 합 + 멀티"),
    "jbspec": ("R-JB-SPEC", "-", "JB 사양 (재질·방폭·IP·단자·글랜드)", "선택값 그대로 (단자 = 페어 × 단자 수)"),
    "tray": ("R-TR", "STEP 6·7 경로", "사다리 트레이 폭·단·m", "Σ π/4·OD² ≤ 폭 × 높이 × Fill → 가장 작은 폭, 넘치면 단"),
    "od": ("R-OD", "F-CAT", "트레이·트렌치 폭", "사양 글자 → 페어/코어 수 → 외경 표"),
    "pt": ("R-UG-PT", "STEP 6 경로", "트렌치 폭·줄 번호 · Perforated", "트렌치 = 단면적 ÷ (폭 × 유효 높이 × Fill), 분기·폭 변경 때 새 번호; Perforated = 분기 케이블 1:1"),
    "sp_ag": ("R-SP-AG", "STEP 8 구간", "AG 서포트 형식·개수·강재", "형식 규칙표에서 담을 수 있는 첫 형식, 고정점 사이 ≤ 스팬"),
    "sp_nema": ("R-SP-NEMA", "F-NEMA", "피팅·익스펜션 서포트 · 조인트 · 앵커/가이드", "피팅 끝 600 mm 안 서포트; 조인트 간격 = 이동량 ÷ (α × ΔT)"),
    "sp_rack": ("R-SP-RACK", "F-PLOT / F-3D 랙", "랙 위 클램프·T 서포트·보조 서포트", "1단 = 빔마다 클램프, 2단 이상 = T 서포트, 빔 600 mm 밖 피팅 = 보조"),
    "sp_ug": ("R-SP-UG", "STEP 8 구간", "트렌치 브래킷·표지·피트 · Perforated 브래킷", "구간 양 끝 + 간격; Perforated = 태그마다 ceil(길이 ÷ 간격)"),
    "sp_mat": ("R-SP-MAT", "타피컬 드로잉 (오면)", "강재 kg · 앵커 · 타피컬 번호", "부재 길이 × 단위 중량, 할증"),
    "tw": ("R-3D", "F-3D · F-OTHER", "3D 배치 (트레이 차선·높이) · 충돌 회피 기록", "계장 차선 선점, 충돌 시 계장이 옆 → 위로 비킴"),
}

CONCEPT_CHANGES = {"A": "IO 시스템·캐비닛 · JB 분리 · 케이블", "B": "IO 행 (도면 인식·복제)", "C": "패키지 IO · 패키지 멀티·광케이블",
                   "D": "모터 IO · IRP 멀티", "E": "차단밸브 IO · 분기 케이블", "F": "제어밸브 IO · 분기 케이블",
                   "G": "전송기·분석기 IO · 분기 케이블", "H": "스페어 IO · 카드 수", "I": "표기·출력 (물량 영향 없음)"}


def build(concept, values: dict) -> dict:
    from . import cable as C
    rows = []
    for iid, it in concept.items.items():
        st = it.get("status", "applied")
        status = {"applied": "적용", "info": "기록용", "planned": "미적용 (구현 전)"}.get(st, st)
        rows.append({"rule_id": iid, "source": "STEP 3 체크리스트 (concept.xlsx)", "group": it.get("group", ""), "name": it["name"],
                     "value": it["value"], "default": it.get("default"),
                     "options": it.get("choices") or (["ON", "OFF"] if it.get("type") == "ON/OFF" else []),
                     "reads": "F-PID (심볼·노트·패키지) + 체크리스트", "changes": CONCEPT_CHANGES.get(iid[:1], "IO"),
                     "formula": it.get("effect") or "", "basis": it.get("basis") or "", "status": status,
                     "note": it.get("status_note", "")})
    for g in C.SCHEMA + C.JB.SCHEMA + C.TR.SCHEMA + C.SP.SCHEMA + C.TW.SCHEMA:
        pre, reads, changes, formula = GROUP_RULE.get(g["id"], ("R-" + g["id"].upper(), "", "", ""))
        for it in g["items"]:
            rows.append({"rule_id": it.get("code") or f"{pre}:{it['id']}", "source": f"STEP 6~10 설정 · {g['title']}",
                         "group": pre, "name": it["name"], "key": it["id"], "value": values.get(it["id"]), "default": it["default"],
                         "options": it.get("options") or [], "unit": it.get("unit", ""), "reads": reads, "changes": changes,
                         "formula": formula, "basis": it.get("help") or "", "status": "적용", "note": ""})
    return {"version": "규칙 프로토콜 v1", "principle": "팩트(데이터) + 체크리스트(설계 컨셉) → 규칙 → 물량. 같은 팩트·같은 값이면 같은 물량.",
            "facts": FACTS, "rules": rows}


def write_sheet(wb, proto: dict, sheet):
    sheet(wb, "11_규칙프로토콜", ["Rule ID", "출처", "그룹", "항목", "현재 값", "기본값", "선택지", "읽는 데이터", "바뀌는 결과", "식 / 효과", "근거", "상태"],
          [[r["rule_id"], r["source"], r["group"], r["name"], str(r["value"]), str(r["default"]), " / ".join(map(str, r["options"]))[:120],
            r["reads"], r["changes"], r["formula"], r["basis"], r["status"]] for r in proto["rules"]],
          [10, 22, 12, 30, 18, 18, 30, 22, 26, 60, 40, 12], wrap=(10, 11))
    sheet(wb, "12_팩트데이터", ["ID", "데이터", "필드", "성격"], [[f["id"], f["name"], f["fields"], f["kind"]] for f in proto["facts"]],
          [8, 26, 60, 26])
