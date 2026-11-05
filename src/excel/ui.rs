// Options dialog, then the file rewrite. A locked workbook is skipped and offered again
// through the same Restart Manager dialog as a copy that hits a sharing violation.

use std::io::{Read, Write};
use std::path::Path;

use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;

use crate::imports::*;
use crate::i18n::*;

use super::ooxml::{self, TidyOptions};

/// Above the shell range, next to the other Maguro commands (who-is-using is 0x8004).
pub(crate) const ID_CTX_EXCEL: u32 = 0x8007; // tidy A1; REVEAL=0x8005 excelro=0x8006

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
    static PREFS: Cell<Prefs> = const { Cell::new(Prefs { a1: true, zoom: true, first: true, backup: false }) };
    static HOT: Cell<i32> = const { Cell::new(0) }; // hovered checkbox id, or 0
}

const MARGIN_DIP: i32 = crate::PAD_BODY_X;
const ROW_DIP: i32 = 28;
const ROW_GAP_DIP: i32 = crate::SPACE_8;
const BOX_DIP: i32 = 16;
const BOX_TEXT_GAP_DIP: i32 = crate::SPACE_8;
const BTN_W_DIP: i32 = 80;
const BTN_H_DIP: i32 = crate::BTN_H_FOOTER;
const BTN_GAP_DIP: i32 = crate::BTN_GAP;
const CLIENT_W_DIP: i32 = 420;

fn check_ids() -> [i32; 4] {
    [ID_A1, ID_ZOOM, ID_FIRST, ID_BACKUP]
}

fn check_label(id: i32) -> &'static str {
    match id {
        ID_A1 => tr(Key::ExcelTidySelection),
        ID_ZOOM => tr(Key::ExcelTidyZoom),
        ID_FIRST => tr(Key::ExcelTidyFirst),
        ID_BACKUP => tr(Key::ExcelTidyBackup),
        _ => "",
    }
}

fn pref_get(p: Prefs, id: i32) -> bool {
    match id {
        ID_A1 => p.a1,
        ID_ZOOM => p.zoom,
        ID_FIRST => p.first,
        ID_BACKUP => p.backup,
        _ => false,
    }
}

fn pref_set(p: &mut Prefs, id: i32, on: bool) {
    match id {
        ID_A1 => p.a1 = on,
        ID_ZOOM => p.zoom = on,
        ID_FIRST => p.first = on,
        ID_BACKUP => p.backup = on,
        _ => {}
    }
}

fn sc(dpi: i32, dip: i32) -> i32 {
    crate::px(dpi, dip)
}

fn ask_style() -> WINDOW_STYLE {
    WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN
}

fn client_h(dpi: i32) -> i32 {
    let rows = 4 * sc(dpi, ROW_DIP) + 3 * sc(dpi, ROW_GAP_DIP);
    rows + sc(dpi, BTN_H_DIP) + 2 * sc(dpi, MARGIN_DIP) + sc(dpi, 16)
}

fn outer_size(dpi: i32) -> (i32, i32) {
        let dpi = dpi.max(96);
    let cw = sc(dpi, CLIENT_W_DIP);
    let ch = client_h(dpi);
    let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
    unsafe {
        if AdjustWindowRectExForDpi(&mut r, ask_style(), false, WS_EX_DLGMODALFRAME, dpi as u32).is_ok() {
            ((r.right - r.left).max(cw), (r.bottom - r.top).max(ch))
        } else {
            (cw, ch + sc(dpi, 48))
        }
    }
}

struct Lay {
    marks: [RECT; 4],
    labels: [RECT; 4],
    rows: [RECT; 4],
    ok: RECT,
    cancel: RECT,
}

fn lay(client_w: i32, client_h: i32, dpi: i32) -> Lay {
    let m = sc(dpi, MARGIN_DIP);
    let row_h = sc(dpi, ROW_DIP);
    let gap = sc(dpi, ROW_GAP_DIP);
    let box_s = sc(dpi, BOX_DIP);
    let text_gap = sc(dpi, BOX_TEXT_GAP_DIP);
    let btn_w = sc(dpi, BTN_W_DIP);
    let btn_h = sc(dpi, BTN_H_DIP);
    let btn_gap = sc(dpi, BTN_GAP_DIP);
    let cw = client_w.max(1);
    let ch = client_h.max(1);
    let mut marks = [RECT::default(); 4];
    let mut labels = [RECT::default(); 4];
    let mut rows = [RECT::default(); 4];
    for i in 0..4 {
        let y = m + i as i32 * (row_h + gap);
        let row = RECT { left: m, top: y, right: cw - m, bottom: y + row_h };
        let mark_y = y + (row_h - box_s) / 2;
        let mark = RECT { left: m, top: mark_y, right: m + box_s, bottom: mark_y + box_s };
        let label = RECT { left: mark.right + text_gap, top: y, right: row.right, bottom: row.bottom };
        rows[i] = row;
        marks[i] = mark;
        labels[i] = label;
    }
    let by = ch - m - btn_h;
    let cancel = RECT { left: (cw - m - btn_w).max(0), top: by, right: (cw - m).max(1), bottom: by + btn_h };
    let ok = RECT {
        left: (cancel.left - btn_gap - btn_w).max(0),
        top: by,
        right: (cancel.left - btn_gap).max(1),
        bottom: by + btn_h,
    };
    Lay { marks, labels, rows, ok, cancel }
}

unsafe fn dialog_font() -> HFONT {
    crate::body_font()
}

unsafe fn child(parent: HWND, class: PCWSTR, text: &str, style: u32, id: i32, font: HFONT) -> HWND {
    let t = HSTRING::from(text);
    crate::child(parent, class, PCWSTR(t.as_ptr()), style, WINDOW_EX_STYLE(0), id, font)
}

unsafe fn ctl(h: HWND, id: i32) -> HWND {
    GetDlgItem(Some(h), id).unwrap_or_default()
}

unsafe fn layout(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let l = lay(rc.right - rc.left, rc.bottom - rc.top, dpi);
    let _ = SetWindowPos(ctl(h, ID_OK), None, l.ok.left, l.ok.top, l.ok.right - l.ok.left, l.ok.bottom - l.ok.top, SWP_NOZORDER | SWP_NOACTIVATE);
    let _ = SetWindowPos(ctl(h, ID_CANCEL), None, l.cancel.left, l.cancel.top, l.cancel.right - l.cancel.left, l.cancel.bottom - l.cancel.top, SWP_NOZORDER | SWP_NOACTIVATE);
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn paint(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let prefs = PREFS.with(|p| p.get());
    let hot = HOT.with(|c| c.get());
    let font = dialog_font();
    crate::buffered(h, |dc, rc| {
        crate::paint_window(dc, rc);
        let l = lay(rc.right - rc.left, rc.bottom - rc.top, dpi);
        let ground = crate::ui_tokens().bg;
        for (i, id) in check_ids().into_iter().enumerate() {
            let on = pref_get(prefs, id);
            crate::paint_check(dc, l.marks[i], on, hot == id, dpi, ground);
            crate::paint_label(dc, l.labels[i], check_label(id), font);
        }
    });
}

unsafe fn draw_btn(d: &DRAWITEMSTRUCT) {
    let parent = GetParent(d.hwndItem).unwrap_or(d.hwndItem);
    let dpi = GetDpiForWindow(parent).max(96) as i32;
    let kind = if d.CtlID as i32 == ID_OK { crate::ButtonKind::Primary } else { crate::ButtonKind::Secondary };
    let visual = crate::visual_from_item(d.itemState.0, crate::app(|a| a.hover) == d.hwndItem);
    crate::paint_button(d.hDC, d.rcItem, kind, visual, &crate::text_of(d.hwndItem), dialog_font(), dpi, crate::ui_tokens().bg, crate::FocusRing::Inside);
}

fn hit_check(l: &Lay, x: i32, y: i32) -> i32 {
    for (i, id) in check_ids().into_iter().enumerate() {
        let r = l.rows[i];
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            return id;
        }
    }
    0
}

unsafe fn ask(owner: HWND, initial: Prefs) -> Option<Prefs> {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(hinst), PCWSTR(crate::ICON_GROUP_LIGHT as _)).unwrap_or_default(),
            lpszClassName: w!("MaguroExcelTidy"),
            ..Default::default()
        });
    });
    ANSWER.with(|a| a.set(None));
    PREFS.with(|p| p.set(initial));
    HOT.with(|c| c.set(0));
    let dpi = if owner.0.is_null() { 96 } else { GetDpiForWindow(owner).max(96) as i32 };
    let (ww, hh) = outer_size(dpi);
    let (x, y) = if owner.0.is_null() {
        (80, 80)
    } else {
        let mut orc = RECT::default();
        let _ = GetWindowRect(owner, &mut orc);
        (orc.left + (orc.right - orc.left - ww) / 2, orc.top + (orc.bottom - orc.top - hh) / 2)
    };
    let title = HSTRING::from(tr(Key::ExcelTidyTitle));
    // Shown only after the dark caption attributes are set, or the first frame stays light.
    let h = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("MaguroExcelTidy"), &title, ask_style(), x, y, ww, hh, Some(owner), None, None, None).unwrap_or_default();
    if h.0.is_null() {
        return None;
    }
    crate::style_dialog_caption(h);
    let font = dialog_font();
    let tab = 0x0001_0000u32;
    let ok = child(h, w!("BUTTON"), tr(Key::Ok), 0xB | tab, ID_OK, font);
    let cancel = child(h, w!("BUTTON"), tr(Key::Cancel), 0xB | tab, ID_CANCEL, font);
    let _ = (ok, cancel);
    layout(h);
    let _ = ShowWindow(h, SW_SHOW);
    crate::keys::set_modal(true);
    if !owner.0.is_null() {
        let _ = EnableWindow(owner, false);
    }
    let _ = SetFocus(Some(ctl(h, ID_OK)));
    let mut msg = MSG::default();
    while ANSWER.with(|a| a.get().is_none()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            PostQuitMessage(0);
            break;
        }
        let mine = msg.hwnd == h || IsChild(h, msg.hwnd).as_bool();
        if msg.message == WM_MOUSEMOVE {
            crate::set_hover(if crate::is_icon_button(msg.hwnd) { msg.hwnd } else { HWND::default() });
        } else if msg.message == WM_MOUSELEAVE && crate::app(|a| a.hover) == msg.hwnd {
            crate::set_hover(HWND::default());
        }
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
    crate::set_hover(HWND::default());
    crate::keys::set_modal(false);
    if !owner.0.is_null() {
        let _ = EnableWindow(owner, true);
        let _ = SetForegroundWindow(owner);
    }
    ANSWER.with(|a| a.get().flatten())
}

unsafe extern "system" fn proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { proc_inner(h, msg, wp, lp) }))
}

unsafe fn proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(h);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_DRAWITEM => {
            draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
            return LRESULT(1);
        }
        WM_MOUSEMOVE => {
            let dpi = GetDpiForWindow(h).max(96) as i32;
            let mut rc = RECT::default();
            let _ = GetClientRect(h, &mut rc);
            let l = lay(rc.right - rc.left, rc.bottom - rc.top, dpi);
            // SIGNED coords for multi-monitor / negative; LOWORD/HIWORD as i16
            let x = lp.0 as i16 as i32;
            let y = (lp.0 >> 16) as i16 as i32;
            let next = hit_check(&l, x, y);
            let prev = HOT.with(|c| c.replace(next));
            if prev != next {
                let _ = InvalidateRect(Some(h), None, false);
            }
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            let dpi = GetDpiForWindow(h).max(96) as i32;
            let mut rc = RECT::default();
            let _ = GetClientRect(h, &mut rc);
            let l = lay(rc.right - rc.left, rc.bottom - rc.top, dpi);
            let x = lp.0 as i16 as i32;
            let y = (lp.0 >> 16) as i16 as i32;
            let id = hit_check(&l, x, y);
            if id != 0 {
                PREFS.with(|p| {
                    let mut prefs = p.get();
                    let on = !pref_get(prefs, id);
                    pref_set(&mut prefs, id, on);
                    p.set(prefs);
                });
                let _ = InvalidateRect(Some(h), None, false);
            }
            return LRESULT(0);
        }
        WM_SIZE => {
            layout(h);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            let font = dialog_font();
            if !font.is_invalid() {
                for id in [ID_OK, ID_CANCEL] {
                    SendMessageW(ctl(h, id), WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                }
            }
            layout(h);
            return LRESULT(0);
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xffff) as i32;
            if id == ID_OK {
                let prefs = PREFS.with(|p| p.get());
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
    DefWindowProcW(h, msg, wp, lp)
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
    fn tidy_dialog_uses_shared_footer_metrics() {
        let dpi = 96;
        let l = super::lay(super::sc(dpi, super::CLIENT_W_DIP), super::client_h(dpi), dpi);
        assert_eq!(l.ok.bottom - l.ok.top, crate::BTN_H_FOOTER);
        assert_eq!(l.cancel.left - l.ok.right, crate::BTN_GAP);
        assert_eq!(l.rows[0].left, crate::PAD_BODY_X);
        assert_eq!(l.marks[0].bottom - l.marks[0].top, 16);
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
