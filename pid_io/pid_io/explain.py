"""Turn the short Inform codes of a row into plain sentences for the review screen.

ask  : '<situation>인데, <what to do>해 주세요.'  (the reason the row is waiting for a person)
info : background only, nothing to do
"""
from __future__ import annotations

import re


def explain(inform: list[str], row=None) -> list[dict]:
    out = []
    func = getattr(row, "func", "") or ""
    for x in inform or []:
        x = str(x).strip()
        if not x or x.startswith("사용자 판단"):
            continue
        out.append(_one(x, func))
    if row is not None and not any(o["k"] == "ask" for o in out):
        conf = getattr(row, "conf", None)
        if getattr(row, "derived", False):
            out.append({"k": "info", "t": "Typical(표준 IO 세트)로 자동으로 만든 행이라 도면에 직접 그려진 계기는 아닙니다."})
        elif getattr(row, "status", "") != "확정" and isinstance(conf, (int, float)) and conf < 0.95:
            out.append({"k": "ask", "t": f"심볼 모양이 레전드와 덜 비슷한데(일치도 {conf:.2f}), 확대 이미지를 보고 "
                                         f"맞으면 '현행 그대로 확정', 아니면 IO·System을 고쳐 주세요."})
    out.sort(key=lambda o: o["k"] != "ask")
    return out


def _one(x: str, func: str) -> dict:
    m = re.search(r"ONLY ONE IS SHOWN", x)
    if m:
        return {"k": "ask", "t": "이 시트 노트에 'ONLY ONE IS SHOWN'(실제로는 여러 대인데 1대만 그림)이 있는데 실제 대수는 "
                                 "노트에 없습니다. 실제 대수를 '수량'에 넣고 '이대로 수정'을 눌러 주세요 "
                                 "(1대가 맞으면 '현행 그대로 확정')."}
    m = re.match(r"(?:같은 태그가 (\S+) p(\d+)에도 있음 - 반복|same tag also on (\S+) p(\d+))", x)
    if m:
        dwg, pg = (m.group(1), m.group(2)) if m.group(1) else (m.group(3), m.group(4))
        return {"k": "ask", "t": f"같은 태그가 {dwg}(p{pg})에도 있는데, 같은 계기를 두 시트에 그린 것인지 번호가 겹친 "
                                 f"다른 계기인지 도면으로는 알 수 없습니다. 같은 계기면 '현행 그대로 확정'(1개로 집계), "
                                 f"다른 계기면 '수량'을 2로 넣고 '이대로 수정'을 눌러 주세요."}
    if x.startswith("같은 태그가") and "각각 집계" in x:
        return {"k": "info", "t": x.replace(" - 각각 집계 (B10)", "") + " - 체크리스트 B10 설정대로 따로따로 집계했습니다."}
    if x.startswith("IO 판정 불일치"):
        why = x.split(" - ", 1)[-1]
        return {"k": "ask", "t": f"{why} - 규칙과 도면 신호선이 서로 다른데, 맞는 IO를 골라 '이대로 수정'을 눌러 주세요 "
                                 f"(규칙이 맞으면 '현행 그대로 확정')."}
    if x.startswith("신호선 근거 일치"):
        return {"k": "info", "t": f"도면 신호선({x.split(' - ', 1)[-1]})도 이 IO와 맞습니다."}
    if x.startswith("Service 자동 초안"):
        return {"k": "info", "t": "Service 문구는 주변 장비·연결로 자동 작성한 초안입니다 (엑셀에서 다듬으면 됩니다)."}
    if x.startswith("ESD interlock"):
        return {"k": "info", "t": f"ESD 인터록({x.split(' ', 2)[-1]})에 연결돼 있어 ESD로 넣었습니다."}
    if x.startswith("interlock link via actuator"):
        tag = x[x.find("(") + 1:x.rfind(")")] if "(" in x else ""
        return {"k": "ask", "t": f"밸브 구동기를 거쳐 인터록 {tag}에 연결된 것으로 추정했는데 신호선으로는 확인되지 "
                                 f"않았습니다. 맞으면 '현행 그대로 확정', 아니면 System을 고쳐 주세요."}
    if x.startswith("Interlock 연결 확인"):
        return {"k": "info", "t": "인터록 연결을 도면 신호선으로 확인했습니다."}
    if "IO 규칙표에 없음" in x or x.startswith("function code not in"):
        return {"k": "ask", "t": f"기능코드 '{func}'가 IO 규칙표에 없어 IO를 정하지 못했는데, IO·System을 골라 "
                                 f"'이대로 수정'을 눌러 주세요 (계기가 아니면 '인덱스에서 제외')."}
    if x.startswith("no legend rule matched"):
        return {"k": "ask", "t": "레전드의 어떤 심볼 모양과도 맞지 않는데, 확대 이미지를 보고 계기인지 판단해 주세요."}
    m = re.match(r"벤더 PLC 내부 계기 - IO 제외 \((.+?),", x)
    if m:
        return {"k": "info", "t": f"벤더 패키지 {m.group(1)}의 PLC로 가는 계기라 우리 IO에서 뺐습니다 (체크리스트 A03 OFF)."}
    m = re.match(r"벤더 경계 안 \((.+)\)", x)
    if m:
        return {"k": "info", "t": f"벤더 패키지 {m.group(1)} 경계(일점쇄선) 안에 있는 계기입니다."}
    if x.startswith("units:"):
        return {"k": "info", "t": f"패키지 유닛 {x[6:]}"}
    if x.startswith("검토 승인") or x.startswith("AI 제안 규칙"):
        return {"k": "info", "t": x + " - 이미 승인된 판단을 적용했습니다."}
    if x.startswith("연결 방식"):
        return {"k": "info", "t": f"패키지 {x} 기준으로 만든 행입니다."}
    if "물량 미상" in x or "계기 수를 알 수 없음" in x:
        return {"k": "ask", "t": "패키지 도면이 아직 없고 '전부 하드와이어'라 계기 수를 알 수 없는데, 개념 체크리스트 "
                                 "C09에 패키지당 AI/AO/DI/DO 점수를 넣어 주세요."}
    m = re.search(r"\((A\d\d|B\d\d|C\d\d|D\d\d|G\d\d)(?: [A-Z]+)?\)$", x)
    if m:
        return {"k": "info", "t": f"{x} - 체크리스트 {m.group(1)} 설정을 적용했습니다."}
    return {"k": "info", "t": x}
