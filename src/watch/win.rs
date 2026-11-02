// Folder Watch window. Owned by the main window, cerulean caption, client follows the light or dark palette.
// Watching itself does not depend on this window staying open.

use std::sync::Once;

use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWINDOWATTRIBUTE};
use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES};
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::ReleaseStgMedium;
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use super::model::{format_civil, Row, Status, CHROME};
use crate::imports::*;

struct WatchWin {
    hwnd: HWND,
    dpi: i32,
    font: HFONT,
    bold: HFONT,
    scroll: i32,
    hot: bool,
    banner: String,
}

impl WatchWin {
    fn blank() -> Self {
        WatchWin {
            hwnd: HWND::default(),
            dpi: 96,
            font: HFONT::default(),
            bold: HFONT::default(),
            scroll: 0,
            hot: false,
            banner: String::new(),
        }
    }
}

thread_local! {
    static WATCH: RefCell<WatchWin> = RefCell::new(WatchWin::blank());
}

struct Hit {
    id: u64,
    sub: RECT,
    run: RECT,
    del: RECT,
}

struct Lay {
    add: RECT,
    active: RECT,
    banner: RECT,
    rows: Vec<Hit>,
    empty: RECT,
    view_top: i32,
    view_h: i32,
    content_h: i32,
}

fn sc(dpi: i32, dip: i32) -> i32 {
    dip * dpi.max(96) / 96
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

pub(crate) unsafe fn open(owner: HWND) {
    ensure_class();
    let existing = WATCH.with(|w| w.borrow().hwnd);
    if !existing.0.is_null() && IsWindow(Some(existing)).as_bool() {
        let _ = ShowWindow(existing, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(existing);
        redraw();
        return;
    }
    let dpi = if owner.0.is_null() { 96 } else { GetDpiForWindow(owner).max(96) as i32 };
    let (ww, hh) = (sc(dpi, 640), sc(dpi, 520));
    let (x, y) = place(owner, ww, hh);
    let style = WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN | WS_VISIBLE;
    let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroWatch"), w!("Maguro"), style, x, y, ww, hh, Some(owner), None, None, None) else { return };
    WATCH.with(|w| {
        let mut u = w.borrow_mut();
        *u = WatchWin::blank();
        u.hwnd = h;
        u.dpi = GetDpiForWindow(h).max(96) as i32;
    });
    rebuild_fonts();
    style_window(h);
    let _ = SetWindowTextW(h, &HSTRING::from(crate::tr(crate::Key::FolderWatch)));
    let dt: IDropTarget = WatchDrop { hwnd: h }.into();
    let _ = RegisterDragDrop(h, &dt);
    redraw();
}

pub(crate) unsafe fn close() {
    let h = WATCH.with(|w| w.borrow().hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = DestroyWindow(h);
    }
}

pub(crate) unsafe fn redraw() {
    let h = WATCH.with(|w| w.borrow().hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = InvalidateRect(Some(h), None, false);
    }
}

pub(crate) unsafe fn restyle() {
    let h = WATCH.with(|w| w.borrow().hwnd);
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    style_window(h);
    rebuild_fonts();
    let _ = SetWindowTextW(h, &HSTRING::from(crate::tr(crate::Key::FolderWatch)));
    redraw();
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(watch_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(crate::ICON_GROUP_LIGHT as _)).unwrap_or_default(),
            lpszClassName: w!("MaguroWatch"),
            ..Default::default()
        });
    });
}

unsafe fn place(owner: HWND, ww: i32, hh: i32) -> (i32, i32) {
    if owner.0.is_null() {
        return (80, 80);
    }
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    (orc.left + (orc.right - orc.left - ww) / 2, orc.top + (orc.bottom - orc.top - hh) / 2)
}

unsafe fn style_window(h: HWND) {
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = crate::cref(CHROME);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    let text = crate::cref(0xFFFFFF);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(36), &text as *const _ as _, 4);
}

fn rebuild_fonts() {
    let dpi = WATCH.with(|w| w.borrow().dpi.max(96));
    let (face, weight) = crate::title_style();
    let font = make_font(crate::ui_face(), 14, 400, dpi);
    let bold = make_font(face, 14, weight.max(600), dpi);
    let old = WATCH.with(|w| {
        let mut u = w.borrow_mut();
        let old = (u.font, u.bold);
        u.font = font;
        u.bold = bold;
        old
    });
    drop_font(old.0);
    drop_font(old.1);
}

fn drop_font(font: HFONT) {
    if !font.is_invalid() {
        unsafe { let _ = DeleteObject(font.into()); }
    }
}

fn make_font(face: &str, px: i32, weight: i32, dpi: i32) -> HFONT {
    let mut lf = LOGFONTW {
        lfHeight: -(px * dpi / 96).max(1),
        lfWeight: weight,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    unsafe { CreateFontIndirectW(&lf) }
}

fn explain(key: &crate::Key) -> String {
    if *key == crate::Key::WatchLimit {
        crate::tf(crate::Key::WatchLimit, &[("n", &super::limit().to_string())])
    } else {
        crate::tr(*key).to_string()
    }
}

fn set_banner(key: Option<&crate::Key>) {
    WATCH.with(|w| w.borrow_mut().banner = key.map(explain).unwrap_or_default());
}

unsafe fn add_paths(paths: &[String]) {
    let mut err = None;
    for path in paths {
        if let Some(k) = super::add_folder(path) {
            if err.is_none() {
                err = Some(k);
            }
        }
    }
    set_banner(err);
    redraw();
}

unsafe extern "system" fn watch_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            crate::buffered(h, |dc, rc| paint(h, dc, rc));
            LRESULT(0)
        }
        WM_SIZE | WM_MOUSEWHEEL => {
            if msg == WM_MOUSEWHEEL {
                let delta = (((wp.0 >> 16) & 0xffff) as u16 as i16 as i32) / 120;
                let dpi = WATCH.with(|w| w.borrow().dpi);
                WATCH.with(|w| {
                    let mut u = w.borrow_mut();
                    u.scroll -= delta * sc(dpi, 48);
                });
            }
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_VSCROLL => {
            on_scroll(h, wp);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = crate::xy(lp);
            on_click(h, x, y);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            let _ = DestroyWindow(h);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let rc = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(h, None, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOACTIVATE);
            WATCH.with(|w| w.borrow_mut().dpi = GetDpiForWindow(h).max(96) as i32);
            rebuild_fonts();
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let dpi = GetDpiForWindow(h).max(96) as i32;
            let m = &mut *(lp.0 as *mut MINMAXINFO);
            m.ptMinTrackSize.x = sc(dpi, 480);
            m.ptMinTrackSize.y = sc(dpi, 320);
            LRESULT(0)
        }
        WM_DESTROY => {
            if WATCH.with(|w| w.borrow().hwnd) == h {
                let _ = RevokeDragDrop(h);
                let (font, bold) = WATCH.with(|w| {
                    let mut u = w.borrow_mut();
                    u.hwnd = HWND::default();
                    let fonts = (u.font, u.bold);
                    u.font = HFONT::default();
                    u.bold = HFONT::default();
                    fonts
                });
                drop_font(font);
                drop_font(bold);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

unsafe fn on_scroll(h: HWND, wp: WPARAM) {
    let code = SCROLLBAR_COMMAND((wp.0 & 0xffff) as i32);
    let dpi = WATCH.with(|w| w.borrow().dpi);
    let line = sc(dpi, 48);
    let page = sc(dpi, 200);
    let thumb = if code == SB_THUMBTRACK || code == SB_THUMBPOSITION {
        let mut info = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_TRACKPOS, ..Default::default() };
        let _ = GetScrollInfo(h, SB_VERT, &mut info);
        Some(info.nTrackPos)
    } else {
        None
    };
    WATCH.with(|w| {
        let mut u = w.borrow_mut();
        if code == SB_LINEUP {
            u.scroll -= line;
        } else if code == SB_LINEDOWN {
            u.scroll += line;
        } else if code == SB_PAGEUP {
            u.scroll -= page;
        } else if code == SB_PAGEDOWN {
            u.scroll += page;
        } else if let Some(pos) = thumb {
            u.scroll = pos;
        } else if code == SB_TOP {
            u.scroll = 0;
        }
    });
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn on_click(h: HWND, x: i32, y: i32) {
    let rows = super::snapshot();
    let lay = layout(h, &rows);
    if inside(lay.add, x, y) {
        if let Some(path) = crate::pick_folder(h) {
            add_paths(&[path]);
        }
        return;
    }
    if inside(lay.active, x, y) {
        set_banner(super::add_active());
        redraw();
        return;
    }
    for hit in &lay.rows {
        if inside(hit.sub, x, y) {
            super::toggle_subfolders(hit.id);
            return;
        }
        if inside(hit.run, x, y) {
            super::toggle_run(hit.id);
            return;
        }
        if inside(hit.del, x, y) {
            super::delete(hit.id);
            return;
        }
    }
}

unsafe fn layout(h: HWND, rows: &[Row]) -> Lay {
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let dpi = WATCH.with(|w| w.borrow().dpi.max(96));
    let scroll = WATCH.with(|w| w.borrow().scroll);
    let banner_on = WATCH.with(|w| !w.borrow().banner.is_empty());
    let w = rc.right - rc.left;
    let height = rc.bottom - rc.top;
    let pad = sc(dpi, 16);
    let gap = sc(dpi, 8);
    let mut y = pad;
    y += sc(dpi, 28);
    let btn_h = sc(dpi, 32);
    let btn_w = sc(dpi, 150);
    let add = RECT { left: pad, top: y, right: pad + btn_w, bottom: y + btn_h };
    let active = RECT { left: add.right + gap, top: y, right: (add.right + gap + btn_w).min(w - pad), bottom: y + btn_h };
    y += btn_h + gap;
    let banner = if banner_on {
        let r = RECT { left: pad, top: y, right: w - pad, bottom: y + sc(dpi, 28) };
        y += sc(dpi, 28) + gap;
        r
    } else {
        RECT::default()
    };
    let view_top = y;
    let view_h = (height - view_top).max(1);
    let row_h = sc(dpi, 84);
    let content_h = (rows.len() as i32) * row_h;
    let max_scroll = (content_h - view_h).max(0);
    let scroll = scroll.clamp(0, max_scroll);
    WATCH.with(|w| w.borrow_mut().scroll = scroll);
    let mut hits = Vec::new();
    let mut ry = view_top - scroll;
    let bw = sc(dpi, 84);
    let bh = sc(dpi, 28);
    for row in rows {
        let right = w - pad;
        let del = RECT { left: right - bw, top: ry + sc(dpi, 50), right, bottom: ry + sc(dpi, 50) + bh };
        let run = RECT { left: del.left - gap - bw, top: del.top, right: del.left - gap, bottom: del.bottom };
        let sub = RECT { left: pad, top: ry + sc(dpi, 30), right: (pad + sc(dpi, 220)).min(run.left - gap), bottom: ry + sc(dpi, 52) };
        hits.push(Hit { id: row.id, sub, run, del });
        ry += row_h;
    }
    let empty = RECT { left: pad, top: view_top, right: w - pad, bottom: height - pad };
    Lay { add, active, banner, rows: hits, empty, view_top, view_h, content_h }
}

unsafe fn paint(h: HWND, dc: HDC, rc: RECT) {
    let rows = super::snapshot();
    let lay = layout(h, &rows);
    sync_scroll(h, &lay);
    let (font, bold, hot, banner) = WATCH.with(|w| {
        let u = w.borrow();
        (u.font, u.bold, u.hot, u.banner.clone())
    });
    let dpi = WATCH.with(|w| w.borrow().dpi.max(96));
    crate::fill(dc, rc, crate::pal().bg);
    let pad = sc(dpi, 16);
    let hint = RECT { left: pad, top: pad, right: rc.right - pad, bottom: pad + sc(dpi, 28) };
    crate::text(dc, hint, crate::tr(crate::Key::FolderWatchHint), font, crate::pal().sub, DT_LEFT);
    button(dc, lay.add, crate::tr(crate::Key::AddFolder), bold, true);
    button(dc, lay.active, crate::tr(crate::Key::AddActivePane), bold, false);
    if !banner.is_empty() {
        crate::fill(dc, lay.banner, crate::pal().card);
        crate::frame(dc, lay.banner, 1, 0xC45050);
        let inner = RECT { left: lay.banner.left + sc(dpi, 8), right: lay.banner.right - sc(dpi, 8), ..lay.banner };
        crate::text(dc, inner, &banner, font, 0xC45050, DT_LEFT);
    }
    if rows.is_empty() {
        crate::text(dc, lay.empty, crate::tr(crate::Key::WatchEmpty), font, crate::pal().sub, DT_CENTER);
    }
    let row_h = sc(dpi, 84);
    for (row, hit) in rows.iter().zip(&lay.rows) {
        let top = hit.sub.top - sc(dpi, 30);
        if top + row_h <= lay.view_top || top >= rc.bottom {
            continue;
        }
        let card = RECT { left: pad, top, right: rc.right - pad, bottom: top + row_h - sc(dpi, 8) };
        crate::fill(dc, card, crate::pal().card);
        crate::frame(dc, card, 1, crate::pal().line);
        let path_r = RECT { left: card.left + sc(dpi, 12), top: card.top + sc(dpi, 4), right: card.right - sc(dpi, 12), bottom: card.top + sc(dpi, 28) };
        crate::text(dc, path_r, &row.path, bold, crate::pal().text, DT_LEFT);
        checkbox(dc, hit.sub, row.subfolders, font, dpi);
        let status = RECT { left: hit.sub.right + sc(dpi, 8), top: hit.sub.top, right: card.right - sc(dpi, 12), bottom: hit.sub.bottom };
        crate::text(dc, status, crate::tr(row.status.key()), font, status_rgb(row.status), DT_RIGHT);
        let last = RECT { left: card.left + sc(dpi, 12), top: hit.run.top, right: hit.run.left - sc(dpi, 8), bottom: hit.run.bottom };
        crate::text(dc, last, &last_line(row), font, crate::pal().sub, DT_LEFT);
        let running = !row.paused;
        let run_label = if running { crate::tr(crate::Key::WatchStop) } else { crate::tr(crate::Key::WatchResume) };
        button(dc, hit.run, run_label, font, !running);
        button(dc, hit.del, crate::tr(crate::Key::Delete), font, false);
    }
    if hot {
        crate::frame(dc, rc, sc(dpi, 3).max(2), CHROME);
    }
}

fn status_rgb(status: Status) -> u32 {
    match status {
        Status::Watching => CHROME,
        Status::Paused => crate::pal().sub,
        Status::Missing | Status::Failed => 0xC45050,
    }
}

fn last_line(row: &Row) -> String {
    let (Some(ms), Some(kind)) = (row.last_ms, row.last_kind) else {
        return crate::tr(crate::Key::NoChangeYet).to_string();
    };
    let when = civil_from_unix(ms);
    let kind = crate::tr(kind.key());
    if when.is_empty() {
        return format!("{}  {kind}", crate::tr(crate::Key::LastChange));
    }
    format!("{}  {when}  {kind}", crate::tr(crate::Key::LastChange))
}

fn civil_from_unix(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let ticks = (ms as u64).saturating_mul(10_000).saturating_add(11_644_473_600 * 10_000_000);
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    unsafe {
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        if FileTimeToSystemTime(&ft, &mut utc).is_err() {
            return String::new();
        }
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            local = utc;
        }
        format_civil(local.wYear as i32, local.wMonth as u32, local.wDay as u32, local.wHour as u32, local.wMinute as u32)
    }
}

unsafe fn button(dc: HDC, r: RECT, label: &str, font: HFONT, primary: bool) {
    if r.right <= r.left {
        return;
    }
    if primary {
        crate::fill(dc, r, CHROME);
        crate::text(dc, r, label, font, crate::on_color(CHROME), DT_CENTER);
    } else {
        crate::fill(dc, r, crate::pal().card);
        crate::frame(dc, r, 1, CHROME);
        crate::text(dc, r, label, font, CHROME, DT_CENTER);
    }
}

unsafe fn checkbox(dc: HDC, r: RECT, on: bool, font: HFONT, dpi: i32) {
    let side = sc(dpi, 14);
    let top = r.top + ((r.bottom - r.top) - side).max(0) / 2;
    let mark = RECT { left: r.left, top, right: r.left + side, bottom: top + side };
    if on {
        crate::fill(dc, mark, CHROME);
    } else {
        crate::fill(dc, mark, crate::pal().card);
        crate::frame(dc, mark, 1, CHROME);
    }
    let label = RECT { left: mark.right + sc(dpi, 6), ..r };
    crate::text(dc, label, crate::tr(crate::Key::IncludeSubfolders), font, crate::pal().text, DT_LEFT);
}

unsafe fn sync_scroll(h: HWND, lay: &Lay) {
    let mut info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (lay.content_h - 1).max(0),
        nPage: lay.view_h.max(1) as u32,
        nPos: WATCH.with(|w| w.borrow().scroll),
        ..Default::default()
    };
    windows::Win32::UI::Controls::SetScrollInfo(h, SB_VERT, &info, true);
    info.fMask = SIF_POS;
    let _ = GetScrollInfo(h, SB_VERT, &mut info);
    WATCH.with(|w| w.borrow_mut().scroll = info.nPos.max(0));
}

#[implement(IDropTarget)]
struct WatchDrop {
    hwnd: HWND,
}

impl IDropTarget_Impl for WatchDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let ok = unsafe { !dir_paths(&d).is_empty() };
        offer(effect, ok);
        set_hot(self.hwnd, ok);
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let hot = WATCH.with(|w| w.borrow().hot);
        offer(effect, hot);
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        set_hot(self.hwnd, false);
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let paths = unsafe { dir_paths(&d) };
        offer(effect, !paths.is_empty());
        set_hot(self.hwnd, false);
        if !paths.is_empty() {
            unsafe { add_paths(&paths) };
        }
        Ok(())
    }
}

fn offer(effect: *mut DROPEFFECT, ok: bool) {
    if !effect.is_null() {
        unsafe { *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE }; }
    }
}

fn set_hot(h: HWND, on: bool) {
    WATCH.with(|w| w.borrow_mut().hot = on);
    unsafe { let _ = InvalidateRect(Some(h), None, false); }
}

unsafe fn dir_paths(d: &Ref<IDataObject>) -> Vec<String> {
    hdrop_paths(d).into_iter().filter(|p| is_dir(p)).collect()
}

unsafe fn is_dir(path: &str) -> bool {
    let attr = GetFileAttributesW(&HSTRING::from(path));
    if attr == INVALID_FILE_ATTRIBUTES {
        path.starts_with(r"\\")
    } else {
        attr & FILE_ATTRIBUTE_DIRECTORY.0 == FILE_ATTRIBUTE_DIRECTORY.0
    }
}

unsafe fn hdrop_paths(d: &Ref<IDataObject>) -> Vec<String> {
    let Some(obj) = d.as_ref() else { return Vec::new() };
    let fmt = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = obj.GetData(&fmt) else { return Vec::new() };
    let drop = HDROP(medium.u.hGlobal.0);
    let mut paths = Vec::new();
    for k in 0..DragQueryFileW(drop, u32::MAX, None) {
        let need = DragQueryFileW(drop, k, None) as usize;
        let mut buf = vec![0u16; need + 1];
        let got = DragQueryFileW(drop, k, Some(&mut buf)) as usize;
        let path = String::from_utf16_lossy(buf.get(..got).unwrap_or(&[]));
        if !path.is_empty() {
            paths.push(path);
        }
    }
    ReleaseStgMedium(&mut medium);
    paths
}
