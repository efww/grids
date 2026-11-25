# pid_io: 계장 개념설계 I/O 리스트 산출 (P&ID → Instrument Index / IO List)

벡터 PDF P&ID에서 계장 심볼(Balloon)을 인식해 태그를 뽑고, 심볼 종류(FIELD / DCS / ESD / PLC)와 IO 프로토콜에 따라 IO를 붙여 Instrument Index 양식에 채웁니다.

## 사용법

```bash
pip install -r requirements.txt
python -m pid_io run "P&ID.pdf" \
    --template Instrument_Index_Template.xlsx \
    --out result.xlsx \
    --ref reference_index.xlsx      # (선택) 정확도 비교
    --overlay review.pdf            # (선택) 인식 결과를 도면 위에 표시
    --concept my_concept.xlsx       # (선택) 개념설계 체크리스트. 기본: protocols/concept_default.xlsx
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

## 개념설계 체크리스트 (`--concept`)

도면 인식은 항상 전부 수행하고, IO 물량을 바꾸는 것은 체크리스트(엑셀)뿐입니다.
`python -m pid_io.make_concept_template`로 빈 템플릿을 다시 만들 수 있습니다.

- **Concept_Checklist**: 설계 가정 53항목 (ON/OFF, 선택, 숫자)
- **Typical_Points**: Typical 정의. IO 1점 = 1행이며, 수량과 조건에 체크리스트 ID를 씁니다. 엔진은 이 표대로 인덱스 행을 만듭니다.
- **Package_Concept**: 패키지별 연결 방식 (없으면 C01 기본값)
- **적용_원리**: 선택에 따라 인덱스에 어떤 행이 생기는지 예시

**원칙: IO 1점 = 인덱스 1행.** 노트 배수(트레인), 파생 IO(리밋스위치, 솔레노이드), 모터 IO, 패키지 IO, 스페어까지 모두 행으로 생성합니다. 인덱스 끝에 세 열을 추가합니다.

| 열 | 내용 |
|---|---|
| 산출구분 | 도면인식 / Typical파생 / 트레인복제 / 장비Typical / 패키지 / 스페어 |
| 근거 | 예: `11-FV-0301 / CV (F01)`, `PID-0001 노트 'TRAIN 12~18 …' (B01)` |
| 원본 Tag | 파생·복제된 행의 출처 계기 |

번호가 정해지지 않은 행은 `10-XS-XXXX`처럼 표기합니다. 벤더 PLC 패키지 안의 계기(A03 OFF)는 행을 남기고 IO만 `-`로 둡니다.

출력 시트:
- `IO_Summary`: 산출구분 × System × IO를 인덱스 행에서 COUNTIFS로 셉니다. IO 카드 개략 수량도 포함합니다.
- `Concept_Basis`: 적용한 가정
- `Notes_Review`: 읽은 노트와 적용 효과

## 정확도를 올리는 방법 (프로토콜 수정)

코드를 고치지 않고 YAML만 바꾸면 됩니다.

| 상황 | 수정 위치 |
|---|---|
| 새 프로젝트 레전드에서 심볼 모양이 다름 | `symbols.yaml`의 coverage 임계값, `midline`, `text_regex` |
| 원 안의 글자가 태그가 아님 (GRADE, NOTE 등) | `symbols.yaml` → `ignore_words` |
| 기능코드의 IO가 다름 | `io_rules.yaml` → `rules` (위에서부터 첫 매칭 적용) |
| 모터, 밸브, 패키지 typical | 체크리스트 `Typical_Points` (코드 수정 없음) |
| 레퍼런스 도면 매칭이 틀림 | `dwg_map.yaml` |

## Status 기준

- **확정**: 도형 일치도 ≥ 0.95, 특이사항 없음
- **검토필요**: 파생 태그(LS, TW, 모터), Interlock 연결 추정, 같은 태그가 여러 시트에 있음, 프로토콜에 없는 기능코드
- `Coordination` 열 = 화면 기준 좌표. `--overlay` PDF에서 위치를 바로 확인할 수 있습니다.

## 한계 (v1)

- 스캔(래스터) PDF는 지원하지 않습니다. 벡터 PDF 전용입니다.
- 계기 1대에 대해 시스템(DCS/ESD)이 갈리는 경우는 신호선 추적 결과에 의존합니다.
- Line No, Equipment No, Service 열은 아직 비워 둡니다.
