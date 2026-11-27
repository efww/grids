"""Page images + positions saved while the P&ID is readable.

The company label turns the P&ID on disk into a cover-page PDF that Python cannot read.
Everything the screens need from the drawing is therefore saved once, when the file is
readable (upload / extraction), as PNG + JSON (the label does not touch these):

  cache/raster/p<N>.png      whole page at DPI
  cache/raster/info.json     per page: size, vendor boundaries, note hit boxes, words
  cache/page_texts.json      {dwg: [title, text]} for the reference comparison

Review crops, page overviews, note crops and AI crops are drawn from these images.
Coordinates are page points in display orientation (as stored in extract.json).
"""
from __future__ import annotations

import json
import os
import threading
from pathlib import Path

from PIL import Image, ImageDraw

DPI = 150
S = DPI / 72.0
NOTE_SEARCH = {"B01": ("IDENTICAL TO TRAIN", "PREFIXED BY"), "B03": ("ONLY ONE",), "C09": ("DEVELOPED LATER", "DEVELOPED IN")}
RED, ORANGE, BLUE, PURPLE = (224, 31, 56), (255, 140, 0), (46, 112, 214), (125, 102, 204)
Image.MAX_IMAGE_PIXELS = None
_IMG: dict = {}
_LOCK = threading.Lock()


def rdir(pd: Path) -> Path:
    return Path(pd) / "cache" / "raster"


def info(pd: Path) -> dict:
    f = rdir(pd) / "info.json"
    try:
        return json.loads(f.read_text(encoding="utf-8")) if f.exists() else {}
    except (OSError, ValueError):
        return {}


def ready(pd: Path) -> bool:
    i = info(pd)
    return bool(i.get("complete"))


def has_page(pd: Path, page: int) -> bool:
    return (rdir(pd) / f"p{page}.png").exists() and str(page) in info(pd).get("pages", {})


# ------------------------------------------------------------------ build (needs a readable PDF)
def build(pdf: Path, pd: Path, progress=None):
    import pymupdf as fitz
    from .boundary import find_regions, legend_width
    from .extract import page_texts

    out = rdir(pd)
    out.mkdir(parents=True, exist_ok=True)
    doc = fitz.open(pdf)
    lw = legend_width(doc)
    meta = {"dpi": DPI, "pdf": Path(pdf).name, "pages": {}, "complete": False}
    n = doc.page_count
    for i, pg in enumerate(doc, 1):
        tmp = out / f"p{i}.tmp.png"
        pg.get_pixmap(dpi=DPI).save(tmp)
        os.replace(tmp, out / f"p{i}.png")
        m = pg.rotation_matrix
        try:
            regions = [[round(v, 1) for v in r.rect] for r in find_regions(pg, lw)]
        except Exception:
            regions = []
        notes = {k: [[round(v, 1) for v in h] for q in qs for h in pg.search_for(q)] for k, qs in NOTE_SEARCH.items()}
        words = [[round(v, 1) for v in fitz.Rect(w[:4]) * m] + [w[4]] for w in pg.get_text("words")]
        meta["pages"][str(i)] = {"w": pg.rect.width, "h": pg.rect.height, "regions": regions, "notes": notes,
                                 "words": words}
        if progress:
            progress(i, n)
    (Path(pd) / "cache" / "page_texts.json").write_text(json.dumps(page_texts(pdf), ensure_ascii=False),
                                                        encoding="utf-8")
    meta["complete"] = True
    tmp = out / "info.tmp"
    tmp.write_text(json.dumps(meta, ensure_ascii=False), encoding="utf-8")
    os.replace(tmp, out / "info.json")
    _IMG.clear()


# ------------------------------------------------------------------ drawing from the cache
def _page(pd: Path, page: int) -> Image.Image:
    f = rdir(pd) / f"p{page}.png"
    key = str(f)
    with _LOCK:
        im = _IMG.get(key)
        if im is None:
            if len(_IMG) > 3:
                _IMG.clear()
            im = Image.open(f).convert("RGB")
            im.load()
            _IMG[key] = im
    return im


def _crop(pd, page, x0, y0, x1, y1):
    im = _page(pd, page)
    box = (max(0, int(x0 * S)), max(0, int(y0 * S)), min(im.width, int(x1 * S)), min(im.height, int(y1 * S)))
    return im.crop(box), box


def card(pd: Path, page: int, cx: float, cy: float, out: Path, sym=None, radius: float = 18, half: float = 95):
    """Same picture as markup.crop, drawn on the cached page image."""
    pinfo = info(pd)["pages"][str(page)]
    lo_x, lo_y, hi_x, hi_y = cx - half, cy - half, cx + half, cy + half
    for ev in (getattr(sym, "line", None) or []):
        for x, y in ev.get("path") or []:
            lo_x, lo_y, hi_x, hi_y = min(lo_x, x - 25), min(lo_y, y - 25), max(hi_x, x + 25), max(hi_y, y + 25)
        if ev.get("to_xy"):
            x, y, r = ev["to_xy"]
            lo_x, lo_y, hi_x, hi_y = min(lo_x, x - r - 20), min(lo_y, y - r - 20), max(hi_x, x + r + 20), max(hi_y, y + r + 20)
    img, box = _crop(pd, page, lo_x, lo_y, hi_x, hi_y)
    img = img.copy()
    d = ImageDraw.Draw(img)
    P = lambda x, y: (x * S - box[0], y * S - box[1])  # noqa: E731

    def rect(x0, y0, x1, y1, color, w):
        d.rectangle([P(x0, y0), P(x1, y1)], outline=color, width=max(1, int(w * S)))

    for x0, y0, x1, y1 in pinfo.get("regions", []):
        if x0 < hi_x and x1 > lo_x and y0 < hi_y and y1 > lo_y:
            rect(x0, y0, x1, y1, PURPLE, 2)
    for ev in (getattr(sym, "line", None) or []):
        path = ev.get("path") or []
        if len(path) >= 2:
            d.line([P(x, y) for x, y in path], fill=ORANGE, width=max(1, int(2.4 * S)))
        if ev.get("to_xy"):
            x, y, r = ev["to_xy"]
            r *= 1.2
            rect(x - r, y - r, x + r, y + r, BLUE, 2.2)
    r = radius * 1.25
    rect(cx - r, cy - r, cx + r, cy + r, RED, 2.6)
    img.save(out)
    return out


def overview(pd: Path, page: int, out: Path, boxes, dpi: int = 60):
    """Whole page, small, with vendor boundaries and symbol boxes. boxes: [(cx, cy, r, rgb)]."""
    im = _page(pd, page)
    k = dpi / DPI
    img = im.resize((int(im.width * k), int(im.height * k)), Image.LANCZOS)
    d = ImageDraw.Draw(img)
    s = dpi / 72.0
    for x0, y0, x1, y1 in info(pd)["pages"][str(page)].get("regions", []):
        d.rectangle([x0 * s, y0 * s, x1 * s, y1 * s], outline=PURPLE, width=3)
    for cx, cy, r, col in boxes:
        d.rectangle([(cx - r) * s, (cy - r) * s, (cx + r) * s, (cy + r) * s], outline=col, width=2)
    img.save(out)
    return out


def note(pd: Path, page: int, which: str) -> bytes:
    import io
    hits = info(pd)["pages"][str(page)]["notes"].get(which) or []
    if not hits:
        raise ValueError("도면에서 이 노트 위치를 찾지 못했는데, 노트 문구는 위 표에 그대로 적혀 있으니 표를 참고해 주세요")
    x0, y0, x1, y1 = hits[0]
    for h in hits[1:3]:
        if abs(h[1] - y0) < 200:
            x0, y0, x1, y1 = min(x0, h[0]), min(y0, h[1]), max(x1, h[2]), max(y1, h[3])
    dpi_out = 130
    img, box = _crop(pd, page, x0 - 330, y0 - 60, x1 + 330, y1 + 90)
    img = img.convert("RGBA")
    ov = Image.new("RGBA", img.size, (0, 0, 0, 0))
    d = ImageDraw.Draw(ov)
    for h in hits:
        a = (h[0] * S - box[0] - 3, h[1] * S - box[1] - 3, h[2] * S - box[0] + 3, h[3] * S - box[1] + 3)
        d.rectangle(a, fill=(255, 217, 51, 90), outline=RED, width=2)
    img = Image.alpha_composite(img, ov).convert("RGB")
    k = dpi_out / DPI
    img = img.resize((int(img.width * k), int(img.height * k)), Image.LANCZOS)
    b = io.BytesIO()
    img.save(b, "PNG")
    return b.getvalue()


def nearby_words(pd: Path, page: int, cx, cy, r=70) -> str:
    ws = info(pd)["pages"][str(page)].get("words", [])
    out = [w[4] for w in ws if abs((w[0] + w[2]) / 2 - cx) < r and abs((w[1] + w[3]) / 2 - cy) < r]
    return " ".join(out)[:200]
