# pid_io: P&ID → Instrument Index / IO List

벡터 PDF P&ID에서 계장 심볼(Balloon)을 인식해 태그를 뽑고, 심볼 종류(FIELD / DCS / ESD / PLC)와 IO 프로토콜에 따라 IO를 붙여 Instrument Index 양식에 채웁니다.

## 사용법

```bash
pip install -r requirements.txt
python -m pid_io run "P&ID.pdf" \
    --template Instrument_Index_Template.xlsx \
    --out result.xlsx \
    --ref reference_index.xlsx      # (선택) 정확도 비교
    --overlay review.pdf            # (선택) 인식 결과를 도면 위에 표시
    --pages 10-17,31                # (선택) 일부 페이지만
```

레전드 시트(`PID-9xxx`)는 자동으로 건너뜁니다.

## 동작 방식

1. **도형 수집** (`geometry.py`): PDF의 선, 곡선, 사각형을 점으로 펼칩니다. CAD 출력은 원 하나가 여러 조각으로 나뉘어 있는 경우가 많아, 경로 단위가 아니라 "태그 글자 주변에 어떤 외곽선이 있는가"로 판정합니다.
2. **심볼 판정** (`protocols/symbols.yaml`): 태그 글자(PT) 아래의 번호(0601)를 짝짓고, 주변에서 원, 사각, 다이아몬드, 가운데선의 존재 비율(coverage)을 측정해 레전드 규칙에 대입합니다.
   - 원 → FIELD
   - 사각 안에 원 → DCS
   - 사각 + 다이아몬드 + `IS` → ESD Interlock
   - 사각 + `I` → DCS Interlock
   - 다이아몬드 + `I` → Hardwired Interlock
3. **신호선 추적** (`extract.py`): Interlock 심볼에서 얇은 신호선(점선 조각 포함)을 따라가 연결된 계기를 찾습니다. 밸브 액추에이터에서 끊기면 가장 가까운 밸브에 연결합니다. ESD Interlock에 물린 계기는 System = ESD가 됩니다.
4. **IO 할당** (`protocols/io_rules.yaml`):
   - 기능코드 → IO 매핑 (예: `xT` → AI, `FV` → AO + FLSO/FLSC DI, `XV` → DO-SOV + XLSO/XLSC DI)
   - 암묵 태그 생성 (TG/TE → TW)
   - 장비 typical (펌프 → MHS, MXS, MHSR, MHSS, HS)
5. **출력** (`export.py`): 템플릿의 `Instrument_Index`를 채우고, 드롭다운과 조건부서식을 복구합니다. 시트 구성은 다음과 같습니다.
   - `DCS_Functions`: PI, FIC, PAHH 같은 DCS 소프트 태그 (물리 IO 없음)
   - `Ref_Compare*`: 레퍼런스 인덱스와 도면별, 기능코드별, IO별 비교
   - `Ref_Title_Map`: 레퍼런스 도면명 ↔ PDF 시트 자동 매칭 결과. 틀리면 `protocols/dwg_map.yaml`에서 고칩니다.

## 정확도를 올리는 방법 (프로토콜 수정)

코드를 고치지 않고 YAML만 바꾸면 됩니다.

| 상황 | 수정 위치 |
|---|---|
| 새 프로젝트 레전드에서 심볼 모양이 다름 | `symbols.yaml`의 coverage 임계값, `midline`, `text_regex` |
| 원 안의 글자가 태그가 아님 (GRADE, NOTE 등) | `symbols.yaml` → `ignore_words` |
| 기능코드의 IO가 다름 | `io_rules.yaml` → `rules` (위에서부터 첫 매칭 적용) |
| 펌프 외 장비(팬, 압축기)의 모터 IO | `io_rules.yaml` → `equipment_typicals`에 장비 문자 추가, `extract.find_equipment(kinds=...)` |
| 레퍼런스 도면 매칭이 틀림 | `dwg_map.yaml` |

## Status 기준

- **확정**: 도형 일치도 ≥ 0.95, 특이사항 없음
- **검토필요**: 파생 태그(LS, TW, 모터), Interlock 연결 추정, 같은 태그가 여러 시트에 있음, 프로토콜에 없는 기능코드
- `Coordination` 열 = 화면 기준 좌표. `--overlay` PDF에서 위치를 바로 확인할 수 있습니다.

## 한계 (v1)

- 스캔(래스터) PDF는 지원하지 않습니다. 벡터 PDF 전용입니다.
- 계기 1대에 대해 시스템(DCS/ESD)이 갈리는 경우는 신호선 추적 결과에 의존합니다.
- Line No, Equipment No, Service 열은 아직 비워 둡니다.
