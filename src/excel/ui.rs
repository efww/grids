// Options dialog, then the file rewrite. A locked workbook is skipped and offered again
// through the same Restart Manager dialog as a copy that hits a sharing violation.

use std::io::{Read, Write};
use std::path::Path;

use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;

use crate::imports::*;
use crate::i18n::*;

use super::ooxml::{self, TidyOptions};

/// Above the shell range, next to the other Maguro commands (who-is-using is 0x8004).
pub(crate) const ID_CTX_EXCEL: u32 = 0x8007; // tidy A1; REVEAL=0x8005 excelro=0x8006

const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const ID_OK: i32 = 1;
const ID_CANCEL: i32 = 2;
const ID_A1: i32 = 101;
const ID_ZOOM: i32 = 102;
const ID_FIRST: i32 = 103;
const ID_BACKUP: i32 = 104;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Prefs {
    a1: bool,
    zoom: bool,
    first: bool,
    backup: bool,
}

impl Prefs {
    fn from_session(s: &crate::Session) -> Self {
        Self { a1: s.excel_tidy_a1, zoom: s.excel_tidy_zoom, first: s.excel_tidy_first, backup: s.excel_tidy_backup }
    }

    fn apply(self, s: &mut crate::Session) {
        s.excel_tidy_a1 = self.a1;
        s.excel_tidy_zoom = self.zoom;
        s.excel_tidy_first = self.first;
        s.excel_tidy_backup = self.backup;
    }

    fn options(self) -> TidyOptions {
        TidyOptions { selection_a1: self.a1, zoom_100: self.zoom, first_sheet: self.first }
    }
}

struct Skip {
    path: String,
    locked: bool,
    detail: String,
}

struct Report {
    done: usize,
    skipped: Vec<Skip>,
}

pub(crate) fn summary_text(done: usize, skipped: &[(&str, bool, &str)]) -> String {
    let done_s = done.to_string();
    let skipped_s = skipped.len().to_string();
    let mut text = tf(Key::ExcelTidySummary, &[("done", &done_s), ("skipped", &skipped_s)]);
    let shown = skipped.len().min(20);
    if shown > 0 {
        text.push_str("\n\n");
        for (path, locked, detail) in skipped.iter().take(shown) {
            if *locked {
                text.push_str(&tf(Key::ExcelTidySkippedLocked, &[("name", path)]));
            } else {
                text.push_str(&tf(Key::ExcelTidySkippedError, &[("name", path), ("error", detail)]));
            }
            text.push('\n');
        }
        let extra = skipped.len() - shown;
        if extra > 0 {
            text.push_str(&format!("+{extra}"));
        }
    }
    text.trim_end().to_string()
}

/// The command sits with Maguro's own rows, and only when every selected file is `.xlsx` or `.xlsm`.
pub(crate) unsafe fn insert_menu(menu: HMENU, items: &[(String, bool)]) {
    if !ooxml::applies(items) {
        return;
    }
    let Some(at) = crate::menu_index(menu, crate::ID_CTX_WHO as usize)
        .or_else(|| crate::menu_index(menu, crate::ID_CTX_BATCH as usize))
        .or_else(|| crate::menu_index(menu, crate::ID_CTX_FAV as usize))
    else {
        return;
    };
    let label = HSTRING::from(tr(Key::ExcelTidy));
    let _ = InsertMenuW(menu, at, MF_STRING | MF_BYPOSITION, ID_CTX_EXCEL as usize, &label);
}

pub(crate) unsafe fn run(owner: HWND, items: &[(String, bool)]) {
    if !ooxml::applies(items) {
        return;
    }
    let initial = crate::app(|a| Prefs::from_session(&a.s));
    let Some(prefs) = ask(owner, initial) else { return };
    crate::app(|a| prefs.apply(&mut a.s));
    crate::save_session();
    if !prefs.options().changes_files() {
        return;
    }
    let paths: Vec<String> = items.iter().filter(|(_, file)| *file).map(|(p, _)| p.clone()).collect();
    let report = tidy_all(owner, &paths, prefs);
    let rows: Vec<(&str, bool, &str)> = report.skipped.iter().map(|s| (s.path.as_str(), s.locked, s.detail.as_str())).collect();
    let text = HSTRING::from(summary_text(report.done, &rows));
    let title = HSTRING::from(tr(Key::ExcelTidyTitle));
    let icon = if report.skipped.is_empty() { MB_ICONINFORMATION } else { MB_ICONWARNING };
    let _ = MessageBoxW(Some(owner), &text, &title, MB_OK | icon);
}

unsafe fn tidy_all(owner: HWND, paths: &[String], prefs: Prefs) -> Report {
    let wait = LoadCursorW(None, IDC_WAIT).unwrap_or_default();
    let prev = SetCursor(Some(wait));
    let mut done = 0usize;
    let mut skipped = Vec::new();
    let mut pending = paths.to_vec();
    loop {
        let mut locked = Vec::new();
        for path in pending {
            match tidy_one(Path::new(&path), prefs) {
                Ok(()) => done += 1,
                Err(Fail::Locked) => locked.push(path),
                Err(Fail::Message(detail)) => skipped.push(Skip { path, locked: false, detail }),
            }
        }
        if locked.is_empty() {
            break;
        }
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let _ = SetCursor(Some(arrow));
        let retry = crate::shell::inuse::ask_retry(owner, crate::shell::inuse::OpKind::Copy, &locked);
        let _ = SetCursor(Some(wait));
        if !retry {
            for path in locked {
                skipped.push(Skip { path, locked: true, detail: String::new() });
            }
            break;
        }
        pending = locked;
    }
    let _ = SetCursor(Some(prev));
    Report { done, skipped }
}

#[derive(Debug)]
enum Fail {
    Locked,
    Message(String),
}

fn tidy_one(path: &Path, prefs: Prefs) -> std::result::Result<(), Fail> {
    let bytes = read_exclusive(path)?;
    let next = ooxml::tidy_package(&bytes, prefs.options()).map_err(Fail::Message)?;
    if next == bytes {
        return Ok(());
    }
    if prefs.backup {
        let bak = ooxml::backup_path(path);
        // A lock on the backup is not a lock on the workbook. Leave the original alone and say which file.
        write_replace(&bak, &bytes).map_err(|e| match e {
            Fail::Locked => {
                let name = bak.to_string_lossy();
                Fail::Message(tf(Key::ExcelTidySkippedLocked, &[("name", name.as_ref())]))
            }
            other => other,
        })?;
        notify(bak.to_string_lossy().as_ref());
    }
    write_replace(path, &next)?;
    notify(&path.to_string_lossy());
    Ok(())
}

fn read_exclusive(path: &Path) -> std::result::Result<Vec<u8>, Fail> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(path).map_err(map_io)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(map_io)?;
    Ok(bytes)
}

fn map_io(err: std::io::Error) -> Fail {
    if ooxml::is_share_violation(&err) { Fail::Locked } else { Fail::Message(err.to_string()) }
}

fn write_replace(path: &Path, bytes: &[u8]) -> std::result::Result<(), Fail> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(map_io)?;
        }
    }
    let tmp = temp_beside(path);
    let write_tmp = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        Ok::<(), std::io::Error>(())
    })();
    if let Err(e) = write_tmp {
        let _ = std::fs::remove_file(&tmp);
        return Err(map_io(e));
    }
    let moved = replace_move(&tmp, path);
    if moved.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    moved
}

fn temp_beside(path: &Path) -> std::path::PathBuf {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "workbook.xlsx".into());
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(format!("{name}.{}.tidy.tmp", std::process::id()));
    tmp
}

fn replace_move(tmp: &Path, path: &Path) -> std::result::Result<(), Fail> {
    let src = HSTRING::from(tmp.to_string_lossy().as_ref());
    let dst = HSTRING::from(path.to_string_lossy().as_ref());
    match unsafe { MoveFileExW(&src, &dst, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) } {
        Ok(()) => Ok(()),
        Err(e) => {
            if crate::shell::inuse::is_in_use_code(e.code()) {
                Err(Fail::Locked)
            } else {
                Err(Fail::Message(e.message().to_string()))
            }
        }
    }
}

fn notify(path: &str) {
    if path.is_empty() {
        return;
    }
    let mut wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
        if let Some(parent) = Path::new(path).parent() {
            let parent = parent.to_string_lossy();
            if !parent.is_empty() {
                wide = parent.encode_utf16().chain(std::iter::once(0)).collect();
                SHChangeNotify(SHCNE_UPDATEDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
            }
        }
    }
}

thread_local! {
    static ANSWER: Cell<Option<Option<Prefs>>> = const { Cell::new(None) };
}

unsafe fn ask(owner: HWND, initial: Prefs) -> Option<Prefs> {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(proc),
            hInstance: m.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("MaguroExcelTidy"),
            ..Default::default()
        });
    });
    ANSWER.with(|a| a.set(None));
    let dpi = if owner.0.is_null() { 96 } else { GetDpiForWindow(owner).max(96) as i32 };
    let scale = |dip: i32| dip * dpi / 96;
    let mut rect = RECT { left: 0, top: 0, right: scale(420), bottom: scale(220) };
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
    let _ = AdjustWindowRectEx(&mut rect, style, false, WS_EX_DLGMODALFRAME);
    let (ww, hh) = (rect.right - rect.left, rect.bottom - rect.top);
    let (x, y) = if owner.0.is_null() {
        (80, 80)
    } else {
        let mut orc = RECT::default();
        let _ = GetWindowRect(owner, &mut orc);
        (orc.left + (orc.right - orc.left - ww) / 2, orc.top + (orc.bottom - orc.top - hh) / 2)
    };
    let title = HSTRING::from(tr(Key::ExcelTidyTitle));
    let h = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("MaguroExcelTidy"), &title, style, x, y, ww, hh, Some(owner), None, None, None).unwrap_or_default();
    if h.0.is_null() {
        return None;
    }
    style_caption(h);
    let font = dialog_font();
    let tab = 0x0001_0000u32;
    let checks = [
        (ID_A1, tr(Key::ExcelTidySelection), initial.a1),
        (ID_ZOOM, tr(Key::ExcelTidyZoom), initial.zoom),
        (ID_FIRST, tr(Key::ExcelTidyFirst), initial.first),
        (ID_BACKUP, tr(Key::ExcelTidyBackup), initial.backup),
    ];
    for (id, label, on) in checks {
        let ctl = child(h, w!("BUTTON"), label, 3 | tab, id, font);
        theme_ctl(ctl);
        if on {
            let _ = SendMessageW(ctl, BM_SETCHECK, Some(WPARAM(1)), Some(LPARAM(0)));
        }
    }
    let ok = child(h, w!("BUTTON"), tr(Key::Ok), 1 | tab, ID_OK, font);
    let cancel = child(h, w!("BUTTON"), tr(Key::Cancel), tab, ID_CANCEL, font);
    theme_ctl(ok);
    theme_ctl(cancel);
    layout(h);
    let _ = ShowWindow(h, SW_SHOW);
    crate::keys::set_modal(true);
    if !owner.0.is_null() {
        let _ = EnableWindow(owner, false);
    }
    let _ = SetFocus(Some(GetDlgItem(Some(h), ID_A1).unwrap_or_default()));
    let mut msg = MSG::default();
    while ANSWER.with(|a| a.get().is_none()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            PostQuitMessage(0);
            break;
        }
        let mine = msg.hwnd == h || IsChild(h, msg.hwnd).as_bool();
        if msg.message == WM_KEYDOWN && mine {
            let vk = msg.wParam.0 as u16;
            if vk == VK_RETURN.0 {
                let _ = SendMessageW(h, WM_COMMAND, Some(WPARAM(ID_OK as usize)), Some(LPARAM(0)));
                continue;
            }
            if vk == VK_ESCAPE.0 {
                let _ = SendMessageW(h, WM_COMMAND, Some(WPARAM(ID_CANCEL as usize)), Some(LPARAM(0)));
                continue;
            }
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    crate::keys::set_modal(false);
    if !owner.0.is_null() {
        let _ = EnableWindow(owner, true);
        let _ = SetForegroundWindow(owner);
    }
    ANSWER.with(|a| a.get().flatten())
}

fn dialog_font() -> HFONT {
    let font = crate::app(|a| a.font);
    if font.is_invalid() {
        let mut lf = LOGFONTW { lfHeight: -16, lfWeight: 400, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
        for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(crate::ui_face().encode_utf16()) {
            *d = c;
        }
        unsafe { CreateFontIndirectW(&lf) }
    } else {
        font
    }
}

unsafe fn style_caption(h: HWND) {
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = crate::cref(crate::pal().bg);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
}

unsafe fn theme_ctl(h: HWND) {
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let name = if dark { w!("DarkMode_Explorer") } else { w!("Explorer") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

unsafe fn child(parent: HWND, class: PCWSTR, text: &str, style: u32, id: i32, font: HFONT) -> HWND {
    let t = HSTRING::from(text);
    crate::child(parent, class, PCWSTR(t.as_ptr()), style, WINDOW_EX_STYLE(0), id, font)
}

unsafe fn layout(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let s = |dip: i32| dip * dpi / 96;
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let m = s(16);
    let row = s(26);
    let gap = s(8);
    let btn_w = s(96);
    let btn_h = s(28);
    for (i, id) in [ID_A1, ID_ZOOM, ID_FIRST, ID_BACKUP].into_iter().enumerate() {
        let y = m + i as i32 * (row + gap);
        let _ = SetWindowPos(ctl(h, id), None, m, y, rc.right - 2 * m, row, SWP_NOZORDER | SWP_NOACTIVATE);
    }
    let y = rc.bottom - m - btn_h;
    let _ = SetWindowPos(ctl(h, ID_CANCEL), None, rc.right - m - btn_w, y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
    let _ = SetWindowPos(ctl(h, ID_OK), None, rc.right - m - btn_w * 2 - s(8), y, btn_w, btn_h, SWP_NOZORDER | SWP_NOACTIVATE);
}

unsafe fn ctl(h: HWND, id: i32) -> HWND {
    GetDlgItem(Some(h), id).unwrap_or_default()
}

unsafe extern "system" fn proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| proc_inner(h, msg, wp, lp)))
}

unsafe fn proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            let dc = HDC(wp.0 as _);
            let mut rc = RECT::default();
            let _ = GetClientRect(h, &mut rc);
            let brush = CreateSolidBrush(crate::cref(crate::pal().card));
            FillRect(dc, &rc, brush);
            let _ = DeleteObject(brush.into());
            return LRESULT(1);
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
            let dc = HDC(wp.0 as _);
            SetBkMode(dc, TRANSPARENT);
            SetTextColor(dc, crate::cref(crate::pal().text));
            SetBkColor(dc, crate::cref(crate::pal().card));
            SetDCBrushColor(dc, crate::cref(crate::pal().card));
            return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
        }
        WM_SIZE => {
            layout(h);
            return LRESULT(0);
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xffff) as i32;
            if id == ID_OK {
                let prefs = Prefs {
                    a1: checked(h, ID_A1),
                    zoom: checked(h, ID_ZOOM),
                    first: checked(h, ID_FIRST),
                    backup: checked(h, ID_BACKUP),
                };
                ANSWER.with(|a| a.set(Some(Some(prefs))));
                let _ = DestroyWindow(h);
                return LRESULT(0);
            }
            if id == ID_CANCEL {
                ANSWER.with(|a| a.set(Some(None)));
                let _ = DestroyWindow(h);
                return LRESULT(0);
            }
        }
        WM_CLOSE => {
            ANSWER.with(|a| a.set(Some(None)));
            let _ = DestroyWindow(h);
            return LRESULT(0);
        }
        _ => {}
    }
    let _ = lp;
    DefWindowProcW(h, msg, wp, lp)
}

unsafe fn checked(h: HWND, id: i32) -> bool {
    SendMessageW(ctl(h, id), BM_GETCHECK, None, None).0 == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_done_and_names_skipped() {
        let text = summary_text(2, &[("C:\\a.xlsx", true, ""), ("C:\\b.xlsx", false, "not a zip archive")]);
        assert!(text.contains("2"), "{text}");
        assert!(text.contains("2") && text.contains("C:\\a.xlsx"), "{text}");
        assert!(text.contains("in use") || text.contains("사용") || text.contains("使用"), "{text}");
        assert!(text.contains("not a zip archive"), "{text}");
    }

    #[test]
    fn tidy_one_backs_up_then_replaces_and_leaves_a_locked_file() {
        let dir = std::env::temp_dir().join(format!("maguro-tidy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let book = br#"<?xml version="1.0"?><workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><bookViews><workbookView activeTab="2" firstSheet="1"/></bookViews><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
        let rels = br#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let sheet = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView zoomScale="40" topLeftCell="B3" workbookViewId="0"><selection activeCell="C4" sqref="C4"/></sheetView></sheetViews><sheetData/></worksheet>"#;
        let src = mini_zip(&[("xl/workbook.xml", book), ("xl/_rels/workbook.xml.rels", rels), ("xl/worksheets/sheet1.xml", sheet), ("xl/vbaProject.bin", b"vba-bytes")]);
        let path = dir.join("report.xlsm");
        std::fs::write(&path, &src).unwrap();
        let prefs = Prefs { a1: true, zoom: true, first: true, backup: true };
        tidy_one(&path, prefs).unwrap();
        let bak = dir.join("report.bak.xlsm");
        assert_eq!(std::fs::read(&bak).unwrap(), src);
        let edited = std::fs::read(&path).unwrap();
        assert_ne!(edited, src);
        let zip = super::ooxml::tidy_package(&src, prefs.options()).unwrap();
        assert_eq!(edited, zip);
        let again = Prefs { backup: false, ..prefs };
        tidy_one(&path, again).unwrap();
        assert_eq!(std::fs::read(&bak).unwrap(), src, "a second pass that changes nothing does not overwrite the backup");

        let held = dir.join("open.xlsx");
        std::fs::write(&held, &src).unwrap();
        use std::os::windows::fs::OpenOptionsExt;
        let err = {
            let _lock = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&held).unwrap();
            tidy_one(&held, prefs)
        };
        assert!(matches!(err, Err(Fail::Locked)), "{err:?}");
        assert_eq!(std::fs::read(&held).unwrap(), src);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn mini_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data) in files {
            let name_b = name.as_bytes();
            let off = out.len() as u32;
            let crc = {
                let mut c = 0xFFFF_FFFFu32;
                for &b in *data {
                    c ^= b as u32;
                    for _ in 0..8 {
                        let mask = (c & 1).wrapping_neg();
                        c = (c >> 1) ^ (0xEDB8_8320 & mask);
                    }
                }
                !c
            };
            let mut local = Vec::new();
            local.extend(0x0403_4b50u32.to_le_bytes());
            local.extend(20u16.to_le_bytes());
            local.extend(0u16.to_le_bytes());
            local.extend(0u16.to_le_bytes());
            local.extend([0u8; 4]);
            local.extend(crc.to_le_bytes());
            local.extend((data.len() as u32).to_le_bytes());
            local.extend((data.len() as u32).to_le_bytes());
            local.extend((name_b.len() as u16).to_le_bytes());
            local.extend(0u16.to_le_bytes());
            local.extend(name_b);
            out.extend(local);
            out.extend(*data);
            let mut c = Vec::new();
            c.extend(0x0201_4b50u32.to_le_bytes());
            c.extend(20u16.to_le_bytes());
            c.extend(20u16.to_le_bytes());
            c.extend(0u16.to_le_bytes());
            c.extend(0u16.to_le_bytes());
            c.extend([0u8; 4]);
            c.extend(crc.to_le_bytes());
            c.extend((data.len() as u32).to_le_bytes());
            c.extend((data.len() as u32).to_le_bytes());
            c.extend((name_b.len() as u16).to_le_bytes());
            c.extend([0u8; 12]);
            c.extend(off.to_le_bytes());
            c.extend(name_b);
            cd.extend(c);
        }
        let start = out.len() as u32;
        out.extend(&cd);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((cd.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0, 0]);
        out
    }
}
