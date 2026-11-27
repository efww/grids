"""'판단 요청서' PDF: one card per item that needs an engineer's decision.

Each card = marked-up crop of the drawing + what the tool decided + why it
asks + a question with tick boxes, so the packet can be reviewed on paper
or in a meeting without the tool.
"""
from __future__ import annotations

import os
import time
from pathlib import Path

import pymupdf as fitz

QUESTIONS = {
    "IO 판정 불일치": ("규칙과 도면 중 어느 쪽 IO가 맞습니까?", ["규칙이 맞음", "도면이 맞음", "기타: ______"]),
    "미인식 후보": ("이 글자는 계기 태그입니까?", ["예 (종류: ______ / IO: ____)", "아니오 (태그 아님)"]),
    "ESD 연결 추정": ("이 계기가 표시된 ESD Interlock에 연결된 것이 맞습니까?", ["예", "아니오 (DCS)", "기타: ______"]),
    "중복 태그": ("다른 시트의 같은 태그와 같은 계기입니까?", ["같은 계기 (1개로 집계)", "다른 계기 (각각 집계)"]),
    "대수 확인 필요": ("노트 'ONLY ONE IS SHOWN' - 실제 대수는?", ["대수: ____ 대"]),
    "미분류 기능코드": ("이 기능코드의 계기 종류와 IO는?", ["종류: ______  IO: ____"]),
    "벤더 경계 안": ("벤더 범위가 맞습니까? 연결 방식은?", ["하드와이어", "벤더PLC+통신", "혼합", "범위 아님"]),
    "벤더 PLC 내부": ("벤더 PLC 내부 계기로 보고 IO를 제외해도 됩니까?", ["예", "아니오 (하드와이어)"]),
    "도형 일치도 낮음": ("인식 결과가 맞습니까?", ["예", "아니오: ______"]),
    "패키지 물량 미상": ("패키지 IO를 어떻게 잡을까요?", ["Typical 적용: ______", "벤더 자료 대기"]),
}
SKIP = {"파생/가정 행", "ESD 연결"}      # informational, not questions
MARK_LEGEND = "빨강: 판단 대상   주황: 따라간 신호선   파랑: 신호선 도착 심볼   보라: 벤더 경계"


# TrueType first: it can be subset (Malgun on Windows); CFF fonts such as Noto CJK embed whole (~13MB)
FONT_FILES = (r"C:\Windows\Fonts\malgun.ttf", "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
              "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc")


def _font(page):
    """Register a Korean font on the page; returns (name, Font for measuring)."""
    for f in FONT_FILES:
        if os.path.exists(f):
            page.insert_font(fontname="kr", fontfile=f)
            return "kr", fitz.Font(fontfile=f)
    return "korea", fitz.Font("korea")


def build(items: list, out: Path, project: str, card_png):
    """items: review items (dicts from the review view); card_png(item) -> Path of the marked-up crop."""
    asks = [i for i in items if any(r in QUESTIONS for r in i["reasons"]) and not set(i["reasons"]) <= SKIP]
    doc = fitz.open()
    W, H = 842, 595                    # A4 landscape (pt)
    per = 4
    pages = max(1, (len(asks) + per - 1) // per)
    for pno in range(pages):
        page = doc.new_page(width=W, height=H)
        fn, font = _font(page)
        page.draw_rect(fitz.Rect(0, 0, W, 6), color=None, fill=(0.08, 0.68, 0.38))
        page.insert_text((28, 30), f"판단 요청서 · {project}", fontname=fn, fontsize=14)
        page.insert_text((28, 46), f"{time.strftime('%Y-%m-%d')}   {pno + 1}/{pages}   총 {len(asks)}건   ·   {MARK_LEGEND}",
                         fontname=fn, fontsize=7.5, color=(0.4, 0.4, 0.4))
        for k, it in enumerate(asks[pno * per:(pno + 1) * per]):
            col, row = k % 2, k // 2
            x0, y0 = 28 + col * 400, 58 + row * 262
            card = fitz.Rect(x0, y0, x0 + 386, y0 + 252)
            page.draw_rect(card, color=(0.8, 0.8, 0.8), width=0.6)
            n = pno * per + k + 1
            page.draw_rect(fitz.Rect(x0, y0, x0 + 22, y0 + 18), color=None, fill=(0.88, 0.12, 0.22))
            page.insert_text((x0 + 6, y0 + 13), str(n), fontname=fn, fontsize=9, color=(1, 1, 1))
            page.insert_text((x0 + 28, y0 + 13), f"{it['tag']}", fontname=fn, fontsize=11)
            reason = next(r for r in it["reasons"] if r in QUESTIONS)
            page.insert_text((x0 + 200, y0 + 13), reason, fontname=fn, fontsize=8.5, color=(0.88, 0.12, 0.22))
            try:
                img = card_png(it)
                jpg = fitz.Pixmap(str(img)).tobytes(output="jpeg", jpg_quality=80)   # keeps the packet small
                page.insert_image(fitz.Rect(x0 + 6, y0 + 22, x0 + 186, y0 + 202), stream=jpg, keep_proportion=True)
            except Exception:
                page.insert_text((x0 + 10, y0 + 100), "(도면 위치 없음)", fontname=fn, fontsize=8)
            info = (f"{it['desc']}\n결과: {it['system']} {it['io']}  ({it['origin']})\n"
                    f"위치: {it['dwg']} p{it['page'] or '-'}\n\n근거:\n{(it.get('inform') or '-')[:260]}")
            page.insert_textbox(fitz.Rect(x0 + 192, y0 + 24, x0 + 380, y0 + 204), info, fontname=fn, fontsize=7.2,
                                color=(0.15, 0.15, 0.15))
            q, opts = QUESTIONS[reason]
            page.insert_text((x0 + 8, y0 + 218), "Q. " + q, fontname=fn, fontsize=8.5)
            ox = x0 + 10
            for o in opts:
                page.draw_rect(fitz.Rect(ox, y0 + 228, ox + 8, y0 + 236), color=(0.2, 0.2, 0.2), width=0.6)
                page.insert_text((ox + 11, y0 + 235.5), o, fontname=fn, fontsize=7.8)
                ox += 22 + font.text_length(o, fontsize=7.8)
            page.insert_text((x0 + 8, y0 + 248), "검토자: ________   메모: __________________________", fontname=fn,
                             fontsize=7, color=(0.5, 0.5, 0.5))
    try:
        doc.subset_fonts()                 # embed only the glyphs used (the Korean font is large)
    except Exception:
        pass
    doc.save(out, garbage=4, deflate=True)
    return len(asks)
