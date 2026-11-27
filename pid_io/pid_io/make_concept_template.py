"""Builds protocols/concept_default.xlsx - the concept checklist template.

Run:  python -m pid_io.make_concept_template   (OUT=path to write elsewhere)
"""
from pathlib import Path
import openpyxl
from openpyxl.styles import Font, PatternFill, Alignment, Border, Side
from openpyxl.worksheet.datavalidation import DataValidation
from openpyxl.utils import get_column_letter

import os
OUT = os.environ.get("OUT") or str(Path(__file__).resolve().parent.parent / "protocols" / "concept_default.xlsx")

F = "Arial"
HDR_FILL = PatternFill("solid", fgColor="1F3864")
GRP_FILL = PatternFill("solid", fgColor="D9E1F2")
IN_FILL = PatternFill("solid", fgColor="FFFF00")
EX_FILL = PatternFill("solid", fgColor="FFF2CC")
thin = Side(style="thin", color="BFBFBF")
BOX = Border(left=thin, right=thin, top=thin, bottom=thin)


def hdr(ws, row, values, widths=None):
    for i, v in enumerate(values, 1):
        c = ws.cell(row, i, v)
        c.font = Font(name=F, bold=True, color="FFFFFF")
        c.fill = HDR_FILL
        c.alignment = Alignment(horizontal="center", vertical="center", wrap_text=True)
        c.border = BOX
    if widths:
        for i, w in enumerate(widths, 1):
            ws.column_dimensions[get_column_letter(i)].width = w


def body(c, wrap=True):
    c.font = Font(name=F, size=10)
    c.alignment = Alignment(vertical="center", wrap_text=wrap)
    c.border = BOX


def inp(c):
    c.font = Font(name=F, size=10, bold=True, color="0000FF")
    c.fill = IN_FILL
    c.alignment = Alignment(horizontal="center", vertical="center", wrap_text=True)
    c.border = BOX


wb = openpyxl.Workbook()

# ------------------------------------------------------------------ checklist
ON, OFF = "ON", "OFF"
items = [
    # (group, id, item, input type, default, choices, IO effect, basis)
    ("A. 시스템 구성", None),
    ("A", "A01", "ESD(SIS) 시스템 구성", "선택", "독립 SIS", "독립 SIS,DCS 통합",
     "독립이면 ESD IO를 별도 캐비닛 물량으로 집계", "일반 관행 (확정 필요)"),
    ("A", "A02", "F&G 시스템 별도 구성", "ON/OFF", ON, "",
     "ON: GD/FD/경보 IO를 F&G로 분리 집계", "레퍼런스 인덱스 System=FGS 사용"),
    ("A", "A03", "벤더 PLC 내부 IO를 물량에 포함", "ON/OFF", OFF, "",
     "OFF: 벤더 PLC 통신 패키지 내부 계기는 우리 IO에서 제외", "개념설계 기본"),
    ("A", "A04", "MCC 인터페이스 방식", "선택", "하드와이어", "하드와이어,통신(Modbus/Profibus),IMCC(IEC 61850)",
     "통신이면 모터 DI/DO가 통신 포트로 대체", "레퍼런스: DI(MCC)/DO(MCC) 하드와이어"),
    ("A", "A05", "DCS 소프트 태그(PI, FIC, PAHH 등) 출력", "ON/OFF", ON, "",
     "ON: 별도 시트로 출력 (IO 영향 없음)", "루프 검증용"),

    ("B. 도면 해석 (노트·범위)", None),
    ("B", "B01", "트레인 배수 노트 적용 (예: 8 TRAINS)", "ON/OFF", ON, "",
     "ON: 해당 시트 IO × 트레인 수", "PID-0001~0003 노트"),
    ("B", "B02", "트레인 수 기본값 (노트 미검출 시)", "숫자", 1, "",
     "노트가 없는 시트에 적용할 배수", "사용자 입력"),
    ("B", "B03", "'ONLY ONE IS SHOWN' 류 노트 배수 적용", "ON/OFF", ON, "",
     "ON: 노트의 대수만큼 IO 배수", "PID-0015 노트"),
    ("B", "B04", "HOLD 항목 처리", "선택", "포함(검토필요)", "포함(검토필요),제외",
     "HOLD 계기의 IO 포함 여부", "도면 HOLD 표기"),
    ("B", "B05", "FUTURE / SPARE 표기 계기 포함", "ON/OFF", OFF, "",
     "ON: Future 계기도 IO 집계", "확정 필요"),
    ("B", "B06", "벤더 경계(1점쇄선/점선) 판정 사용", "ON/OFF", ON, "",
     "경계 안 계기는 C그룹 패키지 연결방식을 따름", "레전드 'TO BE SUPPLIED BY VENDOR'"),
    ("B", "B07", "현장 지시계(PG, TG, LG) 인덱스 포함", "ON/OFF", ON, "",
     "인덱스 행만 추가, IO 없음", "레퍼런스 인덱스 포함"),
    ("B", "B08", "PSV/PRV/PCV 인덱스 포함", "ON/OFF", ON, "",
     "인덱스 행만 추가, IO 없음", "레퍼런스 인덱스 포함"),
    ("B", "B09", "써모웰(TW) 자동 생성 (TG/TE/TT)", "ON/OFF", ON, "",
     "인덱스 행만 추가, IO 없음", "레퍼런스 인덱스 관행"),
    ("B", "B10", "같은 태그가 여러 시트에 있을 때", "선택", "1개로 병합(검토필요)", "1개로 병합(검토필요),각각 집계",
     "Off-page 반복 표기 중복 제거", "도면 연결 표기"),

    ("C. 벤더 패키지 (기본값, 패키지별 지정은 Package_Concept 시트)", None),
    ("C", "C01", "패키지 기본 연결 방식", "선택", "혼합", "전부 하드와이어,벤더PLC+통신,혼합",
     "혼합: ESD·주요 상태만 하드와이어, 나머지 통신", "개념설계 관행 (확정 필요)"),
    ("C", "C02", "통신 프로토콜", "선택", "Modbus TCP", "Modbus TCP,Modbus RTU,OPC UA,Profinet,EtherNet/IP",
     "통신 포트 타입", "확정 필요"),
    ("C", "C03", "통신 매체", "선택", "광케이블", "광케이블,Cu Ethernet,RS-485",
     "STEP 6 패키지 통신 케이블이 이 값을 따름", "사용자 언급(파이버 옵틱)"),
    ("C", "C04", "통신 이중화", "ON/OFF", ON, "",
     "ON: 패키지당 통신 포트 2", "확정 필요"),
    ("C", "C05", "혼합·하드와이어 패키지 ESD Trip 명령 (DO)", "숫자", 1, "",
     "패키지당 ESD DO 점수", "Typicals PKG-HYBRID"),
    ("C", "C06", "혼합·하드와이어 패키지 ESD Trip 상태 (DI)", "숫자", 1, "",
     "패키지당 ESD DI 점수", "Typicals PKG-HYBRID"),
    ("C", "C07", "혼합·하드와이어 패키지 DCS 상태 (Run/Fault/Common Alarm, DI)", "숫자", 3, "",
     "패키지당 DCS DI 점수", "Typicals PKG-HYBRID"),
    ("C", "C08", "혼합·하드와이어 패키지 DCS 명령 (Start/Permissive, DO)", "숫자", 1, "",
     "패키지당 DCS DO 점수", "Typicals PKG-HYBRID"),
    ("C", "C09", "'P&ID TO BE DEVELOPED LATER' 패키지에 Allowance 적용", "ON/OFF", ON, "",
     "도면 내용이 없는 패키지는 Package_Concept의 Typical로 추정", "PID-0005/0008 노트"),

    ("D. 모터 (Typicals 시트 참조)", None),
    ("D", "D01", "LV 모터 기본 Typical", "선택", "MTR-LV-DOL", "MTR-LV-DOL,MTR-MV,MTR-VFD",
     "펌프 1대당 IO", "레퍼런스 IO typical 시트"),
    ("D", "D02", "MV 모터 적용 기준 (kW 이상)", "숫자", 200, "",
     "정격이 이 값 이상이면 MTR-MV 적용 (장비리스트 연동 시)", "확정 필요"),
    ("D", "D03", "VFD 속도 신호 방식", "선택", "하드와이어 AI/AO", "하드와이어 AI/AO,통신",
     "MTR-VFD의 AI/AO 포함 여부", "확정 필요"),
    ("D", "D04", "비상정지 HS를 ESD 하드와이어로", "ON/OFF", ON, "",
     "ON: 모터당 ESD DO 1", "레퍼런스: HS → ESD DO(MCC)"),
    ("D", "D05", "Standby 자동기동 신호 별도 IO", "ON/OFF", OFF, "",
     "OFF: DCS 로직(소프트)으로 처리", "PID-0031 노트 4"),
    ("D", "D06", "모터 Space Heater 상태 DI", "ON/OFF", OFF, "",
     "ON: 모터당 DI 1 추가", "확정 필요"),
    ("D", "D07", "쿨링타워 팬 Typical (2-Speed)", "ON/OFF", ON, "",
     "ON: 팬마다 MTR-FAN-2SPD 적용", "레퍼런스 IO typical 시트"),

    ("E. On/Off 밸브 (XV/SDV/BDV)", None),
    ("E", "E01", "리밋스위치 개수", "선택", "2 (Open/Close)", "2 (Open/Close),1 (Close만),0",
     "밸브당 DI 개수", "레전드 PID-9006 Typical"),
    ("E", "E02", "솔레노이드 구성", "선택", "단일", "단일,이중(Double)",
     "밸브당 DO 1 또는 2", "레전드 PID-9006 Typical 3"),
    ("E", "E03", "PST (Partial Stroke Test)", "ON/OFF", OFF, "",
     "ON: SIS 밸브에 PST 솔레노이드 DO 1 추가", "레전드 Typical 1P"),
    ("E", "E04", "ESD 밸브 리밋스위치 연결 시스템", "선택", "DCS", "DCS,ESD",
     "ESD 밸브의 DI를 어느 시스템에 둘지", "레퍼런스: XLSO/XLSC → DCS"),
    ("E", "E05", "Local Reset 스위치 IO", "ON/OFF", OFF, "",
     "ON: 래칭 솔레노이드 리셋 DI 1", "레전드 Typical 1R/2R"),

    ("F. 제어밸브 (FV/LV/PV/TV)", None),
    ("F", "F01", "제어밸브 리밋스위치", "ON/OFF", ON, "",
     "ON: 밸브당 DI 2", "레퍼런스: FLSO/FLSC"),
    ("F", "F02", "위치 피드백 AI", "ON/OFF", OFF, "",
     "ON: 밸브당 AI 1", "확정 필요"),
    ("F", "F03", "제어밸브 ESD 트립 솔레노이드", "ON/OFF", OFF, "",
     "ON: ESD 연결 제어밸브에 DO 1", "확정 필요"),

    ("G. 계측기", None),
    ("G", "G01", "전송기 신호 방식", "선택", "4-20mA HART", "4-20mA HART,4-20mA,Fieldbus",
     "Fieldbus면 세그먼트 단위 집계", "레퍼런스 Signal Type"),
    ("G", "G02", "SIS 전송기 이중화", "선택", "1oo1", "1oo1,1oo2,2oo3",
     "ESD 연결 전송기 수량 × 1/2/3", "확정 필요"),
    ("G", "G03", "분석기 Fault/Maintenance DI", "ON/OFF", ON, "",
     "ON: 분석기당 DI 1", "레퍼런스 AT 일부 DI 포함"),
    ("G", "G04", "유량계 적산/진단 통신", "ON/OFF", OFF, "",
     "ON: 통신 포트로 집계", "확정 필요"),
    ("G", "G05", "온도 측정 방식", "선택", "TT 4-20mA", "TT 4-20mA,RTD 직결(DCS RTD 카드)",
     "RTD면 AI 대신 RTD 채널로 집계", "확정 필요"),

    ("H. 물량 마진 / 카드", None),
    ("H", "H01", "AI 스페어율", "숫자", 0.2, "", "AI 합계 × (1+율)", "일반 관행 20%"),
    ("H", "H02", "AO 스페어율", "숫자", 0.2, "", "AO 합계 × (1+율)", "일반 관행 20%"),
    ("H", "H03", "DI 스페어율", "숫자", 0.2, "", "DI 합계 × (1+율)", "일반 관행 20%"),
    ("H", "H04", "DO 스페어율", "숫자", 0.2, "", "DO 합계 × (1+율)", "일반 관행 20%"),
    ("H", "H05", "IO 카드 채널 수 (반올림 단위)", "숫자", 16, "", "카드 수 = 올림(점수/채널)", "확정 필요"),
    ("H", "H06", "IS / Non-IS 구분 기준", "선택", "위험지역 계기 IS", "위험지역 계기 IS,전부 Non-IS,아날로그(AI·AO·RTD)는 IS,전부 IS",
     "IS 배리어/카드 물량 분리", "수소 설비 (확정 필요)"),

    ("I. 출력", None),
    ("I", "I01", "태그 표기", "선택", "트레인 접두 포함", "트레인 접두 포함,접두 없음",
     "예: 10-PT-0602 / PT-0602", "노트 'PREFIXED BY TRAIN NUMBER'"),
    ("I", "I02", "시나리오 비교 시트 생성", "ON/OFF", ON, "",
     "C01 3가지 연결방식별 IO 합계를 나란히 출력", "개념설계 의사결정용"),
    ("I", "I03", "검토용 오버레이 PDF 생성", "ON/OFF", ON, "",
     "인식 결과를 도면 위에 색으로 표시", "검토 효율"),
]

ws = wb.active
ws.title = "Concept_Checklist"
ws["A1"] = "계장 개념설계 I/O 리스트 산출 - Concept Checklist"
ws["A1"].font = Font(name=F, bold=True, size=14)
ws["A2"] = "노란 칸(파란 글씨)만 수정하세요. 기본값은 제안값이며 프로젝트별로 확정이 필요합니다. 선택형은 드롭다운을 사용합니다."
ws["A2"].font = Font(name=F, size=10, italic=True)
H = ["ID", "항목", "입력 형식", "설정값", "기본값(제안)", "선택지", "IO 영향", "근거 / 출처", "비고 (사용자 메모)"]
hdr(ws, 4, H, [7, 44, 9, 20, 18, 36, 46, 30, 26])
ws.freeze_panes = "C5"
r = 5
first_item_row = None
for it in items:
    if it[1] is None:
        ws.merge_cells(start_row=r, start_column=1, end_row=r, end_column=len(H))
        c = ws.cell(r, 1, it[0])
        c.font = Font(name=F, bold=True, size=11)
        c.fill = GRP_FILL
        r += 1
        continue
    _, iid, name, typ, dflt, choices, effect, basis = it
    first_item_row = first_item_row or r
    vals = [iid, name, typ, dflt, dflt, choices.replace(",", " / "), effect, basis, ""]
    for col, v in enumerate(vals, 1):
        c = ws.cell(r, col, v)
        body(c)
    inp(ws.cell(r, 4))
    if isinstance(dflt, float) and dflt < 1:
        ws.cell(r, 4).number_format = "0%"
        ws.cell(r, 5).number_format = "0%"
    if typ == "ON/OFF":
        dv = DataValidation(type="list", formula1='"ON,OFF"', allow_blank=False)
        ws.add_data_validation(dv); dv.add(ws.cell(r, 4))
    elif typ == "선택":
        dv = DataValidation(type="list", formula1=f'"{choices}"', allow_blank=False)
        ws.add_data_validation(dv); dv.add(ws.cell(r, 4))
    elif typ == "숫자":
        dv = DataValidation(type="decimal", operator="greaterThanOrEqual", formula1="0")
        ws.add_data_validation(dv); dv.add(ws.cell(r, 4))
    ws.row_dimensions[r].height = 30
    r += 1
last_item_row = r - 1
# changed-from-default flag via conditional formatting
from openpyxl.formatting.rule import FormulaRule
ws.conditional_formatting.add(
    f"D{first_item_row}:D{last_item_row}",
    FormulaRule(formula=[f'AND($A{first_item_row}<>"",$D{first_item_row}<>$E{first_item_row})'],
                fill=PatternFill("solid", fgColor="F8CBAD")))

# ------------------------------------------------------------------ typical points
# Single source of truth for typicals: one row per IO point. The engine reads
# this sheet; the Typicals summary below is computed from it with formulas.
CL = "Concept_Checklist"
POINTS = [
    # typical, func, suffix, desc, system, io, qty, condition
    ("TX-DCS", "{F}", "", "전송기 본체", "DCS", "AI", 1, ""),
    ("TX-ESD", "{F}", "", "전송기 본체 (A)", "ESD", "AI", 1, ""),
    ("TX-ESD", "{F}", "B", "SIS 이중화 전송기 B", "ESD", "AI", 1, "G02<>1oo1"),
    ("TX-ESD", "{F}", "C", "SIS 이중화 전송기 C", "ESD", "AI", 1, "G02=2oo3"),
    ("ANALYZER", "{F}", "", "분석기 본체", "DCS", "AI", 1, ""),
    ("ANALYZER", "XA", "", "분석기 Fault / Maintenance", "DCS", "DI", 1, "G03=ON"),
    ("XV-DCS", "{F}", "", "On/Off 밸브 솔레노이드", "DCS", "DO", 1, ""),
    ("XV-DCS", "{F}", "-2", "2차 솔레노이드", "DCS", "DO", 1, "E02=이중(Double)"),
    ("XV-DCS", "{L}LSO", "", "Limit Switch (Open)", "DCS", "DI", 1, "E01=2 (Open/Close)"),
    ("XV-DCS", "{L}LSC", "", "Limit Switch (Close)", "DCS", "DI", 1, "E01<>0"),
    ("XV-DCS", "XS", "", "Local Reset", "DCS", "DI", 1, "E05=ON"),
    ("XV-ESD", "{F}", "", "On/Off 밸브 솔레노이드 (ESD)", "ESD", "DO", 1, ""),
    ("XV-ESD", "{F}", "-2", "2차 솔레노이드", "ESD", "DO", 1, "E02=이중(Double)"),
    ("XV-ESD", "{F}", "-PST", "PST 솔레노이드", "ESD", "DO", 1, "E03=ON"),
    ("XV-ESD", "{L}LSO", "", "Limit Switch (Open)", "@E04", "DI", 1, "E01=2 (Open/Close)"),
    ("XV-ESD", "{L}LSC", "", "Limit Switch (Close)", "@E04", "DI", 1, "E01<>0"),
    ("XV-ESD", "XS", "", "Local Reset", "ESD", "DI", 1, "E05=ON"),
    ("CV", "{F}", "", "제어밸브 (Positioner)", "DCS", "AO", 1, ""),
    ("CV", "{L}LSO", "", "Limit Switch (Open)", "DCS", "DI", 1, "F01=ON"),
    ("CV", "{L}LSC", "", "Limit Switch (Close)", "DCS", "DI", 1, "F01=ON"),
    ("CV", "ZT", "", "Position Feedback", "DCS", "AI", 1, "F02=ON"),
    ("CV", "{L}Y", "", "ESD Trip Solenoid", "ESD", "DO", 1, "F03=ON"),
    ("MTR-LV-DOL", "MHS", "", "Motor Local/Remote", "DCS", "DI", 1, ""),
    ("MTR-LV-DOL", "MXS", "", "Motor Running/Fault", "DCS", "DI", 1, ""),
    ("MTR-LV-DOL", "MHSR", "", "Motor Start Command", "DCS", "DO", 1, ""),
    ("MTR-LV-DOL", "MHSS", "", "Motor Stop Command", "DCS", "DO", 1, ""),
    ("MTR-LV-DOL", "HS", "", "Emergency Stop (ESD)", "ESD", "DO", 1, "D04=ON"),
    ("MTR-LV-DOL", "HS", "", "Emergency Stop (DCS)", "DCS", "DO", 1, "D04=OFF"),
    ("MTR-LV-DOL", "MXH", "", "Space Heater Status", "DCS", "DI", 1, "D06=ON"),
    ("MTR-MV", "MHS", "", "Motor Local/Remote", "DCS", "DI", 1, ""),
    ("MTR-MV", "MXS", "", "Motor Running", "DCS", "DI", 1, ""),
    ("MTR-MV", "MXA", "", "Motor Fault", "DCS", "DI", 1, ""),
    ("MTR-MV", "MHSR", "", "Motor Start Command", "DCS", "DO", 1, ""),
    ("MTR-MV", "MHSS", "", "Motor Stop Command", "DCS", "DO", 1, ""),
    ("MTR-MV", "IT", "", "Motor Current", "DCS", "AI", 1, ""),
    ("MTR-MV", "HS", "", "Emergency Stop (ESD)", "ESD", "DO", 1, "D04=ON"),
    ("MTR-MV", "HS", "", "Emergency Stop (DCS)", "DCS", "DO", 1, "D04=OFF"),
    ("MTR-MV", "MXH", "", "Space Heater Status", "DCS", "DI", 1, "D06=ON"),
    ("MTR-VFD", "MHS", "", "Motor Local/Remote", "DCS", "DI", 1, ""),
    ("MTR-VFD", "MXS", "", "Motor Running", "DCS", "DI", 1, ""),
    ("MTR-VFD", "MXA", "", "VFD Fault", "DCS", "DI", 1, ""),
    ("MTR-VFD", "MHSR", "", "Motor Start Command", "DCS", "DO", 1, ""),
    ("MTR-VFD", "MHSS", "", "Motor Stop Command", "DCS", "DO", 1, ""),
    ("MTR-VFD", "SI", "", "Speed Feedback", "DCS", "AI", 1, "D03=하드와이어 AI/AO"),
    ("MTR-VFD", "SC", "", "Speed Reference", "DCS", "AO", 1, "D03=하드와이어 AI/AO"),
    ("MTR-VFD", "COMM", "", "VFD 통신 포트", "DCS", "COMM", 1, "D03=통신"),
    ("MTR-VFD", "HS", "", "Emergency Stop (ESD)", "ESD", "DO", 1, "D04=ON"),
    ("MTR-VFD", "HS", "", "Emergency Stop (DCS)", "DCS", "DO", 1, "D04=OFF"),
    ("MTR-FAN-2SPD", "MHA", "", "Fan Remote Selected", "DCS", "DI", 2, ""),
    ("MTR-FAN-2SPD", "MXA", "", "Fan Fault", "DCS", "DI", 1, ""),
    ("MTR-FAN-2SPD", "MXL", "", "Fan Status", "DCS", "DI", 1, ""),
    ("MTR-FAN-2SPD", "VAHH", "", "Fan Vibration High High", "DCS", "DI", 1, ""),
    ("MTR-FAN-2SPD", "MHS", "", "Fan Fast/Slow/Stop Command", "DCS", "DO", 2, ""),
    ("PKG-HARDWIRED", "XS", "", "ESD Trip Command (하드와이어)", "ESD", "DO", "C05", ""),
    ("PKG-HARDWIRED", "XL", "", "ESD Trip Status (하드와이어)", "ESD", "DI", "C06", ""),
    ("PKG-HARDWIRED", "XL", "", "Running / Fault / Common Alarm (하드와이어)", "DCS", "DI", "C07", ""),
    ("PKG-HARDWIRED", "HS", "", "Start / Permissive (하드와이어)", "DCS", "DO", "C08", ""),
    ("PKG-COMM", "COMM", "A", "패키지 통신 포트 A", "DCS", "COMM", 1, ""),
    ("PKG-COMM", "COMM", "B", "패키지 통신 포트 B (이중화)", "DCS", "COMM", 1, "C04=ON"),
    ("PKG-HYBRID", "XS", "", "ESD Trip Command (하드와이어)", "ESD", "DO", "C05", ""),
    ("PKG-HYBRID", "XL", "", "ESD Trip Status (하드와이어)", "ESD", "DI", "C06", ""),
    ("PKG-HYBRID", "XL", "", "Running / Fault / Common Alarm (하드와이어)", "DCS", "DI", "C07", ""),
    ("PKG-HYBRID", "HS", "", "Start / Permissive (하드와이어)", "DCS", "DO", "C08", ""),
    ("PKG-HYBRID", "COMM", "A", "패키지 통신 포트 A", "DCS", "COMM", 1, ""),
    ("PKG-HYBRID", "COMM", "B", "패키지 통신 포트 B (이중화)", "DCS", "COMM", 1, "C04=ON"),
]


def _cl(id_):
    return f'INDEX({CL}!$D:$D,MATCH("{id_}",{CL}!$A:$A,0))'


def cond_formula(cond):
    if not cond:
        return "TRUE"
    parts = []
    for c in cond.split("&"):
        c = c.strip()
        op = "<>" if "<>" in c else "="
        k, v = c.split(op)
        parts.append(f'{_cl(k.strip())}&""{op}"{v.strip()}"')
    return parts[0] if len(parts) == 1 else "AND(" + ",".join(parts) + ")"


wq = wb.create_sheet("Typical_Points")
wq["A1"] = "Typical 상세 (IO 1점 = 1행) - 엔진이 이 표대로 인덱스 행을 만듭니다"
wq["A1"].font = Font(name=F, bold=True, size=14)
wq["A2"] = ("{F}=원 계기 기능코드, {L}=첫 글자, 수량에 체크리스트 ID(C05 등)를 쓰면 그 설정값을 사용. "
            "System '@E04' = 체크리스트 E04 값. 조건 예: E01<>0, D04=ON, G02<>1oo1 & G03=ON. 적용 수량은 체크리스트에 따라 자동 계산.")
wq["A2"].font = Font(name=F, size=10, italic=True)
QH = ["Typical ID", "순번", "태그 기능코드", "번호 접미사", "설명", "System", "IO", "수량", "적용 조건", "System (적용)", "적용 수량"]
hdr(wq, 4, QH, [16, 6, 13, 10, 40, 9, 7, 7, 26, 12, 10])
seq = {}
for i, (t, f_, sfx, d, sy, io, q, cnd) in enumerate(POINTS):
    rr = 5 + i
    seq[t] = seq.get(t, 0) + 1
    vals = [t, seq[t], f_, sfx, d]
    for col, v in enumerate(vals, 1):
        body(wq.cell(rr, col, v))
    for col, v in ((6, sy), (7, io), (8, q), (9, cnd)):
        inp(wq.cell(rr, col, v))
    sysf = f'=IF(F{rr}="@E04",{_cl("E04")},F{rr})'
    qty_expr = (f"H{rr}" if isinstance(q, (int, float)) else _cl(q))
    for col, v in ((10, sysf), (11, f"=IF({cond_formula(cnd)},{qty_expr},0)")):
        c = wq.cell(rr, col, v); body(c); c.alignment = Alignment(horizontal="center", vertical="center")
pts_last = 4 + len(POINTS)
wq.freeze_panes = "C5"
wq.auto_filter.ref = f"A4:K{pts_last}"

# ------------------------------------------------------------------ typicals summary
wt = wb.create_sheet("Typicals")
wt["A1"] = "IO Typical 요약 (1대당 점수, Typical_Points에서 자동 계산)"
wt["A1"].font = Font(name=F, bold=True, size=14)
wt["A2"] = "수정은 Typical_Points 시트에서 하세요. 이 표는 현재 체크리스트 설정이 반영된 결과입니다. COMM = 통신 포트 수 (하드와이어 합계에서 제외)."
wt["A2"].font = Font(name=F, size=10, italic=True)
TH = ["Typical ID", "설명", "DCS AI", "DCS AO", "DCS DI", "DCS DO", "ESD AI", "ESD DI", "ESD DO", "COMM", "하드와이어 합계", "근거 / 출처"]
hdr(wt, 4, TH, [18, 50, 8, 8, 8, 8, 8, 8, 8, 8, 14, 34])
TDESC = [
    ("TX-DCS", "전송기 (DCS)", "일반"),
    ("TX-ESD", "전송기 (ESD 연결, G02 이중화 반영)", "레전드 Interlock 연결 + G02"),
    ("ANALYZER", "분석기 (G03 Fault DI)", "레퍼런스 AT 일부 DI"),
    ("XV-DCS", "On/Off 밸브 DCS", "레전드 PID-9006 Typical 2"),
    ("XV-ESD", "On/Off 밸브 ESD", "레전드 PID-9006 Typical 1/1P/3"),
    ("CV", "제어밸브", "레퍼런스: FV + FLSO/FLSC"),
    ("MTR-LV-DOL", "LV 모터 직입기동", "레퍼런스 IO typical: MHS, MXS, MHSR, MHSS, HS"),
    ("MTR-MV", "MV 모터", "제안값 (확정 필요)"),
    ("MTR-VFD", "VFD 모터", "제안값 (확정 필요)"),
    ("MTR-FAN-2SPD", "쿨링타워 팬 2-Speed", "레퍼런스 IO typical: MHA, MXA, MXL, VAHH, MHS"),
    ("PKG-HARDWIRED", "패키지 전부 하드와이어 (경계 안 계기 개별 집계 + 운전/Trip 하드와이어)", "Concept C05~C08"),
    ("PKG-COMM", "패키지 벤더PLC + 통신", "Concept C02~C04"),
    ("PKG-HYBRID", "패키지 혼합 (ESD/주요 상태 하드와이어 + 통신)", "Concept C05~C08"),
]
COLS = [("DCS", "AI"), ("DCS", "AO"), ("DCS", "DI"), ("DCS", "DO"), ("ESD", "AI"), ("ESD", "DI"), ("ESD", "DO"), (None, "COMM")]
P = "Typical_Points"
for i, (tid, d, src) in enumerate(TDESC):
    rr = 5 + i
    body(wt.cell(rr, 1, tid)); body(wt.cell(rr, 2, d))
    for k, (sy, io) in enumerate(COLS):
        if sy:
            f = f'=SUMIFS({P}!$K$5:$K${pts_last},{P}!$A$5:$A${pts_last},$A{rr},{P}!$J$5:$J${pts_last},"{sy}",{P}!$G$5:$G${pts_last},"{io}")'
        else:
            f = f'=SUMIFS({P}!$K$5:$K${pts_last},{P}!$A$5:$A${pts_last},$A{rr},{P}!$G$5:$G${pts_last},"COMM")'
        c = wt.cell(rr, 3 + k, f); body(c); c.alignment = Alignment(horizontal="center", vertical="center")
    c = wt.cell(rr, 11, f"=SUM(C{rr}:I{rr})")
    body(c); c.alignment = Alignment(horizontal="center", vertical="center"); c.font = Font(name=F, size=10, bold=True)
    body(wt.cell(rr, 12, src))
typ_last = 4 + len(TDESC)
wt.freeze_panes = "C5"


# ------------------------------------------------------------------ package concept
wp = wb.create_sheet("Package_Concept")
wp["A1"] = "벤더 패키지별 연결 방식"
wp["A1"].font = Font(name=F, bold=True, size=14)
wp["A2"] = ("도구가 도면에서 패키지 태그·경계를 찾아 이 표를 채웁니다. 연결방식/Typical/수량만 확인·수정하세요. "
            "주황 행은 이번 도면에서 확인된 예시입니다 (IO는 Typicals 시트에서 자동 계산).")
wp["A2"].font = Font(name=F, size=10, italic=True)
PH = ["Package Tag", "P&ID No", "패키지명", "연결 방식", "Typical ID", "수량(대)", "도면 상태",
      "DCS AI", "DCS AO", "DCS DI", "DCS DO", "ESD AI", "ESD DI", "ESD DO", "COMM", "하드와이어 합계", "비고"]
hdr(wp, 4, PH, [16, 11, 30, 16, 16, 9, 22, 8, 8, 8, 8, 8, 8, 8, 8, 12, 30])
EXAMPLES = False   # project-specific example rows are not kept in the repo
pk = []   # example rows live only in project copies
N_PK_ROWS = len(pk) + 12            # example rows + empty input rows
for i in range(N_PK_ROWS):
    rr = 5 + i
    p = pk[i] if i < len(pk) else ("",) * 7
    for col, v in enumerate(p, 1):
        c = wp.cell(rr, col, v if v != "" else None); body(c)
        if i < len(pk):
            c.fill = EX_FILL
    for col in (1, 2, 3, 4, 5, 6, 7):
        if i >= len(pk) or col in (4, 5, 6):
            inp(wp.cell(rr, col))
    for k in range(9):  # H..P from Typicals C..K
        tcol = get_column_letter(3 + k)
        f = f'=IF($E{rr}="",0,IFERROR(INDEX(Typicals!${tcol}$5:${tcol}${typ_last},MATCH($E{rr},Typicals!$A$5:$A${typ_last},0))*$F{rr},0))'
        c = wp.cell(rr, 8 + k, f); body(c); c.alignment = Alignment(horizontal="center")
    body(wp.cell(rr, 17, "예시 행 (자동 인식 대상)" if i < len(pk) else None))
tot = 5 + N_PK_ROWS
wp.cell(tot, 1, "합계").font = Font(name=F, bold=True)
for k in range(9):
    col = get_column_letter(8 + k)
    c = wp.cell(tot, 8 + k, f"=SUM({col}5:{col}{tot - 1})")
    c.font = Font(name=F, bold=True); c.border = BOX; c.alignment = Alignment(horizontal="center")
dv = DataValidation(type="list", formula1='"전부 하드와이어,벤더PLC+통신,혼합"'); wp.add_data_validation(dv); dv.add(f"D5:D{tot+30}")
dv2 = DataValidation(type="list", formula1=f"=Typicals!$A$5:$A${typ_last}"); wp.add_data_validation(dv2); dv2.add(f"E5:E{tot+30}")
wp.freeze_panes = "D5"

# ------------------------------------------------------------------ explanation
wx = wb.create_sheet("적용_원리")
wx["A1"] = "체크리스트가 인덱스에 반영되는 방식"
wx["A1"].font = Font(name=F, bold=True, size=14)
wx["A2"] = ("원칙: IO 1점 = 인덱스 1행. 곱하기로 끝내지 않고, 늘어나는 IO는 모두 행으로 생성하며 "
            "'산출구분'과 '근거' 열에 어떤 도면/노트/체크리스트 때문에 생긴 행인지 기록합니다. "
            "번호가 정해지지 않은 행은 태그를 XXXX로 표기합니다 (예: 10-XS-XXXX).")
wx["A2"].font = Font(name=F, size=10, italic=True)
wx["A2"].alignment = Alignment(wrap_text=True)
wx.merge_cells("A2:F2"); wx.row_dimensions[2].height = 45
XH = ["체크리스트", "이런 경우 / 이렇게 선택하면", "인덱스에 생기는 일", "예시 행 (Tag | System | IO)", "산출구분", "근거 열 예시"]
hdr(wx, 4, XH, [12, 40, 52, 34, 14, 34])
ex = [
    ("(기본)", "도면에서 계기 심볼을 인식", "계기 1개 = 1행. 심볼 종류로 System 판정, 기능코드로 IO 판정",
     "10-PT-0602 | ESD | AI", "도면인식", "PID-0006 / IS-002 연결"),
    ("B01 = ON", "노트 'TRAIN 12~18 ARE IDENTICAL TO TRAIN 11'", "11번 트레인의 모든 행(파생 행 포함)을 12~18로 복제하고 태그 앞번호를 교체",
     "12-PT-0101 | DCS | AI", "트레인복제", "PID-0001 노트: TRAIN 12~18 (B01)"),
    ("B01 = OFF", "같은 노트", "도면에 그려진 11번 트레인만 집계 (IO 약 1/8)", "11-PT-0101만", "도면인식", "-"),
    ("B03", "노트 'ONLY ONE IS SHOWN ON THE P&ID' (대수 미기재)", "Notes 시트에 '대수 입력 필요'로 표시. 입력한 대수만큼 행 복제",
     "10-MHS-XXXX (2/3)", "노트배수", "PID-0015 노트 11 (B03)"),
    ("B09 = ON", "TG / TE 계기가 있음", "같은 번호의 써모웰 행 추가 (IO 없음)", "11-TW-0103 | - | -", "Typical파생", "11-TG-0103 (B09)"),
    ("E01 = 2", "On/Off 밸브 XV-0602", "솔레노이드(본체) + 리밋스위치 Open/Close 2행", "10-XLSO-0602 | DCS | DI", "Typical파생", "10-XV-0602 / XV-ESD (E01)"),
    ("E01 = 1 (Close만)", "같은 밸브", "XLSO 행이 생기지 않음 (DI 1점 감소)", "10-XLSC-0602만", "Typical파생", "XV-ESD (E01)"),
    ("E02 = 이중", "같은 밸브", "2차 솔레노이드 행 추가", "10-XV-0602-2 | ESD | DO", "Typical파생", "XV-ESD (E02)"),
    ("E03 = ON", "ESD 연결 XV", "PST 솔레노이드 행 추가", "10-XV-0602-PST | ESD | DO", "Typical파생", "XV-ESD (E03)"),
    ("E04 = ESD", "ESD 밸브의 리밋스위치", "리밋스위치 행의 System이 DCS → ESD", "10-XLSC-0602 | ESD | DI", "Typical파생", "XV-ESD (E04)"),
    ("F01 = OFF", "제어밸브 FV-0301", "FLSO / FLSC 행이 생기지 않음", "11-FV-0301 | DCS | AO만", "도면인식", "CV (F01)"),
    ("G02 = 2oo3", "ESD Interlock에 연결된 PT-0602", "B, C 전송기 행 추가", "10-PT-0602B, 10-PT-0602C | ESD | AI", "Typical파생", "TX-ESD (G02)"),
    ("D01 = MTR-LV-DOL", "도면에 펌프 10-P-801A", "모터 IO 5행 (Remote, Running, Start, Stop, 비상정지)", "10-MHSR-801A | DCS | DO", "장비Typical", "10-P-801A / MTR-LV-DOL (D01)"),
    ("D04 = OFF", "같은 펌프", "비상정지 HS 행의 System이 ESD → DCS", "10-HS-801A | DCS | DO", "장비Typical", "MTR-LV-DOL (D04)"),
    ("C01 = 혼합", "패키지 10-PK-301 (P&ID 추후 개발)", "패키지 공통 IO 행 생성: ESD Trip 명령/상태, DCS 상태/명령, 통신 포트",
     "10-XS-XXXX | ESD | DO", "패키지", "10-PK-301 / PKG-HYBRID (C01, C05~C08)"),
    ("C01 = 벤더PLC+통신", "같은 패키지", "통신 포트 행만 생성. 경계 안 계기는 행은 남기고 IO를 '-'로 (A03 OFF, 혼합도 동일)",
     "10-COMM-XXXXA | DCS | COMM", "패키지", "PKG-COMM (C01, C04)"),
    ("C01 = 전부 하드와이어", "같은 패키지", "경계 안 계기 각각이 자기 IO를 가짐 + 운전/Trip 하드와이어 행 (C05~C08)", "(계기 행 그대로)", "도면인식", "PKG-HARDWIRED"),
    ("C04 = OFF", "통신 패키지", "통신 포트 B 행이 생기지 않음", "COMM-XXXXA만", "패키지", "(C04)"),
    ("H01~H04", "AI 120점, 스페어율 20%", "스페어 행 24개 생성 (태그 SPARE)", "DCS-AI-SPARE-001 | DCS | AI", "스페어", "H01 20%"),
    ("B04 = 포함", "HOLD 표기 계기", "행은 포함하되 Status = 검토필요", "10-PSV-0601 (HOLD 1)", "도면인식", "PID-0006 HOLD 1"),
]
for i, e in enumerate(ex):
    rr = 5 + i
    for col, v in enumerate(e, 1):
        body(wx.cell(rr, col, v))
    wx.row_dimensions[rr].height = 32
rr = 6 + len(ex)
wx.cell(rr, 1, "산출구분 값").font = Font(name=F, bold=True, size=11)
for j, (k, v) in enumerate([
        ("도면인식", "P&ID 심볼에서 직접 읽은 계기"),
        ("Typical파생", "계기 1개에 딸린 IO (리밋스위치, 솔레노이드, 이중화 전송기, 써모웰 등)"),
        ("트레인복제", "노트에 따라 다른 트레인으로 복제한 행"),
        ("노트배수", "'ONLY ONE IS SHOWN' 류 노트로 복제한 행"),
        ("장비Typical", "펌프·팬 등 장비에서 생성한 모터 IO"),
        ("패키지", "벤더 패키지 공통 IO (연결 방식에 따름)"),
        ("스페어", "스페어율로 생성한 예비 IO")]):
    body(wx.cell(rr + 1 + j, 1, k)); body(wx.cell(rr + 1 + j, 2, v))
    wx.merge_cells(start_row=rr + 1 + j, start_column=2, end_row=rr + 1 + j, end_column=4)
wx.freeze_panes = "A5"

# ------------------------------------------------------------------ guide
wg = wb.create_sheet("안내", 0)
wg.column_dimensions["A"].width = 22
wg.column_dimensions["B"].width = 90
rows = [
    ("계장 개념설계 I/O 리스트 산출", None),
    ("버전", "Concept Checklist v0.1 (초안, 기본값은 제안값)"),
    ("목적", "도면에서 읽은 '사실'(계기, 노트, 패키지 경계)에 이 시트의 '설계 가정'을 적용해 개념설계 단계 IO 물량을 산출합니다."),
    ("원칙", "도면 인식은 항상 전부 수행하고, 결과를 바꾸는 것은 이 체크리스트뿐입니다. 체크리스트만 바꿔서 재계산(PDF 재인식 없음)."),
    ("", ""),
    ("사용 순서", "1) 프로젝트 선택 후 도면 인식 실행  2) Concept_Checklist 확정  3) Package_Concept 확인  4) IO 리스트 재계산"),
    ("", ""),
    ("색 구분", None),
    ("노란 칸 / 파란 글씨", "사용자 입력값 (수정 가능)"),
    ("분홍 칸", "기본값에서 변경된 설정 (Concept_Checklist 설정값 열)"),
    ("주황 행", "예시 행 (이번 프로젝트 도면에서 확인된 값)"),
    ("검은 글씨", "계산식 / 설명 (수정 금지)"),
    ("", ""),
    ("시트", None),
    ("Concept_Checklist", "설계 가정 체크리스트 (ON/OFF, 선택, 숫자)"),
    ("Typical_Points", "Typical 상세: IO 1점 = 1행. 엔진이 이 표대로 인덱스 행을 생성 (수정은 여기서)"),
    ("Typicals", "Typical별 1대당 IO 점수 요약 (자동 계산)"),
    ("적용_원리", "체크리스트 선택이 인덱스에 어떻게 반영되는지 사례 설명"),
    ("Package_Concept", "벤더 패키지별 연결 방식과 IO (Typicals 자동 참조)"),
    ("", ""),
    ("통계", None),
    ("체크리스트 항목 수", f'=COUNTA(Concept_Checklist!C{first_item_row}:C{last_item_row})'),
    ("기본값에서 변경된 항목", f'=SUMPRODUCT((Concept_Checklist!A{first_item_row}:A{last_item_row}<>"")*(Concept_Checklist!D{first_item_row}:D{last_item_row}<>Concept_Checklist!E{first_item_row}:E{last_item_row}))'),
    ("패키지 하드와이어 IO 합계", f"=Package_Concept!P{tot}"),
]
for i, (a, b) in enumerate(rows, 1):
    ca = wg.cell(i, 1, a); cb = wg.cell(i, 2, b)
    ca.font = Font(name=F, bold=True, size=14 if i == 1 else 10)
    cb.font = Font(name=F, size=10)
    cb.alignment = Alignment(wrap_text=True, vertical="center")
    if b is None and a:
        ca.fill = GRP_FILL if i > 1 else PatternFill()
wg["A9"].fill = IN_FILL; wg["A9"].font = Font(name=F, bold=True, color="0000FF")
wg["A10"].fill = PatternFill("solid", fgColor="F8CBAD")
wg["A11"].fill = EX_FILL
for rr in (20, 21, 22):
    wg.cell(rr, 2).alignment = Alignment(horizontal="left")

for sh in wb.worksheets:
    sh.page_setup.orientation = "landscape"
    sh.page_setup.paperSize = sh.PAPERSIZE_A3
    sh.page_setup.fitToWidth = 1
    sh.page_setup.fitToHeight = 0
    sh.sheet_properties.pageSetUpPr.fitToPage = True
wb.save(OUT)
print(OUT, first_item_row, last_item_row, tot)
