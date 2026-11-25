# pid_io: 계장 개념설계 I/O 리스트 산출 (P&ID → Instrument Index / IO List)

벡터 PDF P&ID에서 계장 심볼(Balloon)을 인식해 태그를 뽑고, 심볼 종류(FIELD / DCS / ESD / PLC)와 IO 프로토콜에 따라 IO를 붙여 Instrument Index 양식에 채웁니다.

## 화면으로 쓰기 (로컬 웹앱)

`실행.bat`을 더블클릭하거나 `python -m pid_io.app`을 실행하면 브라우저에서 `http://127.0.0.1:8765`가 열립니다. 이 PC 안에서만 동작하고 외부로 나가는 통신은 없습니다.

1. **프로젝트**: 새로 만들면 `projects/<이름>/` 폴더가 생기고 회사 표준 체크리스트가 복사됩니다.
2. **입력·추출**: P&ID PDF와 인덱스 양식(필수), 체크리스트와 레퍼런스(선택)를 올리고 "추출 실행"을 누릅니다. 이 도면 기준 약 1~3분 걸리며, 결과는 `extract.json`으로 저장됩니다.
3. **개념 체크리스트**: 값을 바꾸면 오른쪽 IO가 즉시 다시 계산됩니다(0.1초 이내).
   - 시나리오를 저장하고 비교할 수 있습니다.
   - "영향 큰 가정"은 각 항목을 반대로 바꿨을 때의 IO 변화를 보여줍니다.
   - 패키지별 연결 방식도 여기서 지정합니다.
4. **검토**: 확인이 필요한 행을 사유별로 모아 보여주고, 항목을 누르면 도면에서 위치를 표시합니다.
5. **출력**: 인덱스 엑셀과 (선택) 검토용 오버레이 PDF를 만들고 내려받습니다.

화면 글꼴은 PC에 설치된 현대하모니 L을 쓰고, 없으면 맑은 고딕을 씁니다. 프로젝트 폴더 위치는 환경변수 `PID_IO_HOME`, 포트는 `PID_IO_PORT`로 바꿀 수 있습니다.

## AI 재판정 (GitHub Copilot CLI / 수동 채팅)

규칙이 확정하지 못한 것만 AI에게 묻습니다. 답은 항상 **제안**으로 들어오고, 사람이 승인해야 반영됩니다. 승인한 내용은 프로젝트의 `overrides.json`에 저장되어 다음 계산부터 자동으로 적용됩니다.

**묻는 대상** (검토 화면 → AI 재판정)
- **심볼 재판정**: 규칙에 맞지 않은 태그 모양 글자(미인식 후보), 외곽선 점수가 낮은 심볼, IO 규칙이 없는 기능코드를 가진 심볼. 질문에는 글자, 주변 글자, 외곽선 측정값이 들어가고, 심볼 주변을 자른 이미지가 함께 만들어집니다. 도면 전체는 보내지 않습니다.
- **미분류 기능코드 IO 제안**: EH, PVRV처럼 `io_rules.yaml`에 없는 코드

**연결 방식** (왼쪽 메뉴 → AI 연결 설정, 전체 프로젝트 공통 `ai_settings.json`)
- **수동**: 질문을 복사해 회사 Copilot(M365) 등 채팅 AI에 붙여넣고, 답변을 다시 붙여넣습니다. API가 필요 없습니다.
- **GitHub Copilot CLI**: 이 PC의 `copilot` 명령을 버튼 하나로 실행합니다.
  - 명령은 JSON 배열 템플릿이며 `{prompt}`, `{prompt_file}`, `{model}`이 치환됩니다. 기본값은 `["copilot", "-p", "{prompt}"]`입니다.
  - 설치된 CLI 버전의 옵션(`copilot --help`)에 맞게 조정하고 "연결 테스트"로 확인하세요.
- 나중에 정식 API(Azure OpenAI 등)가 생기면 `pid_io/ai/providers.py`에 연결 방식 하나만 추가하면 됩니다.

## 명령줄로 쓰기

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

## 벤더 경계 인식 (`boundary.py`)

레전드의 "TO BE SUPPLIED/DESIGNED BY (PACKAGE) VENDOR" 샘플이 일점쇄선(긴 선 + 점)이라는 점을 지문으로 씁니다.
- 도면에서 일점쇄선을 찾습니다. 대시 속성으로 그린 것(`[4 3 16 3]`)과 짧은 선 조각으로 하나하나 그린 것을 모두 잡습니다.
- 선을 이어 붙여 닫힌 박스(또는 U자)를 만듭니다. 한 변이 배관에 끊겨 있어도 됩니다.
- 신호선, 트레이싱 같은 일반 점선(`[4 4]`)은 일점쇄선이 아니라서 제외됩니다.
- 박스 안의 패키지 태그(`10-PK-301`)를 영역 이름으로 씁니다. 태그가 없으면 근처 문구로 `VENDOR AREA (BY CI)`처럼 붙입니다.
- 계기 중심이 박스 안에 있으면 인덱스 `Vendor PKG` 열에 기입하고, Inform에 "벤더 경계 안"을 표시합니다.
- 그 패키지가 `벤더PLC+통신`이고 A03이 OFF이면 IO를 `-`로 둡니다(행은 남김).
- `--overlay` PDF에 자주색 일점쇄선으로 표시합니다.

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
