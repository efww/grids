"""Company-labelled (encrypted) files.

  xlsx : opened through Excel with the user's own rights (directopen.py). Nothing is
         decrypted on disk.
  pdf  : an AIP-labelled PDF is a cover page + encrypted payload that Python cannot read.
         While the P&ID is readable (right after upload / extraction) pagecache.py saves
         page images, text, note and boundary positions, so every screen keeps working
         from that cache after the file gets labelled again.
"""
from __future__ import annotations

import json
import os
from pathlib import Path

from .directopen import OLE, is_locked  # noqa: F401  (re-exported)

ASK_PDF = ("입력·추출 화면에서 P&ID를 '다른 파일로 바꾸기'로 한 번 다시 올려 주세요. 올리는 즉시 전체 페이지 "
           "이미지를 저장해 두므로 그 뒤로는 파일에 보안이 다시 걸려도 계속 쓸 수 있습니다.")


class Protected(ValueError):
    """'<situation>인데, <ask>' in one sentence."""

    def __init__(self, path, situation: str = "", ask: str = ASK_PDF):
        self.path = Path(path)
        situation = situation or "P&ID PDF에 보안이 걸려 있어 프로그램이 도면을 읽을 수 없는데"
        super().__init__(f"{situation}, {ask}")


def pdf_locked(p) -> bool:
    p = Path(p)
    if not p.exists() or p.stat().st_size == 0:
        return False
    try:
        import pymupdf as fitz
        doc = fitz.open(p)
        try:
            if doc.page_count > 3:
                return False
            txt = " ".join(doc[i].get_text() for i in range(doc.page_count))
            return "Azure Information Protection" in txt or "Microsoft Purview" in txt or "보호된 문서" in txt
        finally:
            doc.close()
    except Exception:
        return True


def require_pdf(p, situation: str = "", ask: str = ASK_PDF):
    if pdf_locked(p):
        raise Protected(p, situation, ask)


def scan(pdir: Path, files: dict) -> dict:
    """{kind: {'file', 'mode'}} - mode 'excel' (read through Excel) or 'locked' (pdf)."""
    out = {}
    for kind, fn in (files or {}).items():
        if not fn:
            continue
        p = Path(pdir) / ("concept.xlsx" if kind == "concept" else f"input/{fn}")
        if p.suffix.lower() == ".pdf":
            if pdf_locked(p):
                out[kind] = {"file": p.name, "mode": "locked"}
        elif is_locked(p):
            out[kind] = {"file": p.name, "mode": "excel"}
    return out


# ------------------------------------------------------------------ checklist values backup
VALUES = "concept_values.json"


def save_values(con, folder: Path):
    data = {"items": {k: v["value"] for k, v in con.items.items()},
            "packages": [{k: p.get(k) for k in ("tag", "dwg", "name", "conn", "typical", "qty", "note")}
                         for p in con.packages]}
    f = Path(folder) / VALUES
    tmp = f.with_suffix(".tmp")
    tmp.write_text(json.dumps(data, ensure_ascii=False, indent=1, default=str), encoding="utf-8")
    os.replace(tmp, f)


def load_values(folder: Path):
    f = Path(folder) / VALUES
    try:
        return json.loads(f.read_text(encoding="utf-8")) if f.exists() else None
    except (OSError, ValueError):
        return None
