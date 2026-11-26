"""Open label-encrypted Excel files the same way a person does: through Excel itself.

The company DRM/sensitivity label encrypts xlsx files on disk (the file becomes an OLE
container). Excel opens them with the signed-in user's rights. This module asks Excel
(COM, invisible, read-only) for the sheet contents and builds an in-memory openpyxl
workbook from them.

  * Protection is never removed and no decrypted copy is written to disk.
  * Plain xlsx files still go straight to openpyxl (fast path, full fidelity).
  * Values and formulas come across in full. Formatting is copied only for the first
    header rows of each sheet and for column widths (enough for the index template);
    conditional formats and drop-downs are rebuilt by export._restore_rules.

Requires Windows + desktop Excel + pywin32 (requirements.txt installs it on Windows).
"""
from __future__ import annotations

import threading
from pathlib import Path

import openpyxl
from openpyxl.styles import Alignment, Font, PatternFill

OLE = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"
STYLE_ROWS = 6          # header rows whose formatting is copied
STYLE_COLS = 40
_LOCK = threading.Lock()
_DUMPS: dict = {}       # (path, mtime, size) -> dump


class DirectOpenError(ValueError):
    pass


def is_locked(path) -> bool:
    p = Path(path)
    try:
        with open(p, "rb") as f:
            return f.read(8) == OLE
    except OSError:
        return False


def load_workbook(path, data_only: bool = False, read_only: bool = False):
    """Drop-in replacement for openpyxl.load_workbook(path)."""
    p = Path(path)
    if not is_locked(p):
        return openpyxl.load_workbook(p, data_only=data_only, read_only=read_only)
    return _build(_dump(p), data_only)


def available() -> str:
    """'' if Excel automation can be used here, else the reason."""
    try:
        import win32com.client  # noqa: F401
        import pythoncom  # noqa: F401
    except ImportError:
        return "pywin32가 설치되어 있지 않습니다 (실행.bat을 다시 실행하면 설치됩니다)"
    return ""


# --------------------------------------------------------------------------- Excel side
def _dump(p: Path) -> dict:
    st = p.stat()
    key = (str(p.resolve()), st.st_mtime_ns, st.st_size)
    if key in _DUMPS:
        return _DUMPS[key]
    why = available()
    if why:
        raise DirectOpenError(f"{p.name}: 보안 엑셀을 Excel로 열어야 하는데 {why}")
    with _LOCK:
        if key not in _DUMPS:
            if len(_DUMPS) > 8:
                _DUMPS.clear()
            _DUMPS[key] = _excel_dump(p)
    return _DUMPS[key]


def _excel_dump(p: Path) -> dict:
    import pythoncom
    import win32com.client

    pythoncom.CoInitialize()
    xl = wb = None
    try:
        xl = win32com.client.DispatchEx("Excel.Application")
        xl.Visible = False
        xl.DisplayAlerts = False
        xl.ScreenUpdating = False
        try:
            xl.AutomationSecurity = 3            # msoAutomationSecurityForceDisable: no macros
        except Exception:
            pass
        try:
            wb = xl.Workbooks.Open(str(p.resolve()), UpdateLinks=0, ReadOnly=True, IgnoreReadOnlyRecommended=True,
                                   AddToMru=False)
        except Exception as e:
            raise DirectOpenError(f"{p.name}: Excel에서 열리지 않는데, 이 PC에서 파일을 더블클릭해 열리는지 "
                                  f"확인해 주세요 ({_com_msg(e)})")
        sheets = []
        for ws in wb.Worksheets:
            ur = ws.UsedRange
            r0, c0 = ur.Row, ur.Column
            nr, nc = ur.Rows.Count, ur.Columns.Count
            vals = _grid(ur.Value2, nr, nc)
            forms = _grid(ur.Formula, nr, nc)
            fmt = {}
            for r in range(1, min(STYLE_ROWS, r0 + nr - 1) + 1):
                for c in range(1, min(STYLE_COLS, c0 + nc - 1) + 1):
                    cell = ws.Cells(r, c)
                    try:
                        fi = cell.Font
                        it = cell.Interior
                        fmt[(r, c)] = dict(bold=bool(fi.Bold), color=_bgr(fi.Color), size=fi.Size,
                                           fill=_bgr(it.Color) if it.Pattern not in (-4142, 0) else None,
                                           wrap=bool(cell.WrapText), numfmt=str(cell.NumberFormat))
                    except Exception:
                        pass
            widths = {}
            for c in range(1, min(STYLE_COLS, c0 + nc - 1) + 1):
                try:
                    widths[c] = float(ws.Columns(c).ColumnWidth)
                except Exception:
                    pass
            merged = set()
            for r in range(1, min(STYLE_ROWS, r0 + nr - 1) + 1):
                for c in range(1, min(STYLE_COLS, c0 + nc - 1) + 1):
                    try:
                        cell = ws.Cells(r, c)
                        if cell.MergeCells:
                            merged.add(cell.MergeArea.Address.replace("$", ""))
                    except Exception:
                        pass
            sheets.append(dict(title=ws.Name, r0=r0, c0=c0, vals=vals, forms=forms, fmt=fmt, widths=widths,
                               merged=sorted(merged), hidden=ws.Visible != -1))
        return {"sheets": sheets}
    finally:
        try:
            if wb is not None:
                wb.Close(SaveChanges=False)
        except Exception:
            pass
        try:
            if xl is not None:
                xl.Quit()
        except Exception:
            pass
        pythoncom.CoUninitialize()


def _grid(v, nr, nc):
    if nr == 1 and nc == 1:
        return [[v]]
    return [list(row) for row in (v or ())]


def _bgr(v):
    try:
        v = int(v)
    except (TypeError, ValueError):
        return None
    if v < 0:
        return None
    r, g, b = v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF
    return f"FF{r:02X}{g:02X}{b:02X}"


def _com_msg(e) -> str:
    try:
        return str(e.excepinfo[2] or e)
    except Exception:
        return str(e)


# --------------------------------------------------------------------------- openpyxl side
def _build(dump: dict, data_only: bool):
    wb = openpyxl.Workbook()
    wb.remove(wb.active)
    for s in dump["sheets"]:
        ws = wb.create_sheet(s["title"][:31])
        for i, row in enumerate(s["vals"]):
            frow = s["forms"][i] if i < len(s["forms"]) else []
            for j, v in enumerate(row):
                f = frow[j] if j < len(frow) else None
                if not data_only and isinstance(f, str) and f.startswith("="):
                    v = f
                if v is None or v == "":
                    continue
                if isinstance(v, float) and v.is_integer() and abs(v) < 1e15:
                    v = int(v)
                ws.cell(s["r0"] + i, s["c0"] + j, v)
        for (r, c), f in s["fmt"].items():
            cell = ws.cell(r, c)
            cell.font = Font(bold=f["bold"], color=f["color"], size=f["size"] or None)
            if f["fill"]:
                cell.fill = PatternFill("solid", fgColor=f["fill"])
            if f["wrap"]:
                cell.alignment = Alignment(wrap_text=True, vertical="center")
            if f["numfmt"] and f["numfmt"] != "General":
                cell.number_format = f["numfmt"]
        for c, w in s["widths"].items():
            ws.column_dimensions[openpyxl.utils.get_column_letter(c)].width = w
        for rng in s["merged"]:
            try:
                ws.merge_cells(rng)
            except Exception:
                pass
        if s.get("hidden"):
            ws.sheet_state = "hidden"
    return wb
