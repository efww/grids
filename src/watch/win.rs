// Folder Watch window. Owned by the main window. Caption matches Settings: theme background, no cerulean fill.
// A row is a folder or one file. File rows watch the parent directory and hide the subfolder checkbox.
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

use super::model::{format_civil, status_color, Row, Status, CHROME};
use crate::imports::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hover {
    None,
    Add,
    File,
    Active,
    Run(u64),
    Del(u64),
    Row(u64),
}

struct WatchWin {
    hwnd: HWND,
    dpi: i32,
    font: HFONT,
    bold: HFONT,
    small: HFONT,
    scroll: i32,
    drag: bool,
    hover: Hover,
    banner: String,
}

impl WatchWin {
    fn blank() -> Self {
        WatchWin {
            hwnd: HWND::default(),
            dpi: 96,
            font: HFONT::default(),
            bold: HFONT::default(),
            small: HFONT::default(),
            scroll: 0,
            drag: false,
            hover: Hover::None,
            banner: String::new(),
        }
    }
}

thread_local! {
    static WATCH: RefCell<WatchWin> = RefCell::new(WatchWin::blank());
}

struct Hit {
    id: u64,
    row: RECT,
    sub: RECT,
    run: RECT,
    del: RECT,
}

struct Cols {
    folder: i32,
    sub: i32,
    status: i32,
    last: i32,
    actions: i32,
    right: i32,
}

struct Lay {
    hint: RECT,
    add: RECT,
    file: RECT,
    active: RECT,
    banner: RECT,
    frame: RECT,
    head: RECT,
    list: RECT,
    status: RECT,
    cols: Cols,
    rows: Vec<Hit>,
    row_h: i32,
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
    let (ww, hh) = (sc(dpi, 760), sc(dpi, 440));
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
    crate::note_side(h);
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
    let caption = crate::cref(crate::pal().bg);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    let def = COLORREF(0xFFFF_FFFF);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(36), &def as *const _ as _, 4);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(34), &def as *const _ as _, 4);
}

fn face_name() -> String {
    let custom = crate::app(|a| a.s.font_face.clone());
    if custom.is_empty() { crate::ui_face().to_string() } else { custom }
}

fn rebuild_fonts() {
    let dpi = WATCH.with(|w| w.borrow().dpi.max(96));
    let face = face_name();
    let font = make_font(&face, 12, 400, dpi);
    let small = make_font(&face, 11, 400, dpi);
    let bold = make_font(&face, 12, 700, dpi);
    let old = WATCH.with(|w| {
        let mut u = w.borrow_mut();
        let old = (u.font, u.bold, u.small);
        u.font = font;
        u.bold = bold;
        u.small = small;
        old
    });
    drop_font(old.0);
    drop_font(old.1);
    drop_font(old.2);
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
        let one = if is_dir(path) { super::add_folder(path) } else { super::add_file(path) };
        if err.is_none() {
            err = one;
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
                let line = sc(dpi, 22).max(1) * 3;
                WATCH.with(|w| {
                    let mut u = w.borrow_mut();
                    u.scroll -= delta * line;
                });
            }
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_VSCROLL => {
            on_scroll(h, wp);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = crate::xy(lp);
            on_move(h, x, y);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            WATCH.with(|w| w.borrow_mut().hover = Hover::None);
            let _ = InvalidateRect(Some(h), None, false);
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
            m.ptMinTrackSize.x = sc(dpi, 640);
            m.ptMinTrackSize.y = sc(dpi, 360);
            LRESULT(0)
        }
        crate::WM_FAV_ICON => {
            crate::clear_icon_refresh();
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_DESTROY => {
            if WATCH.with(|w| w.borrow().hwnd) == h {
                crate::drop_side(h);
                let _ = RevokeDragDrop(h);
                let (font, bold, small) = WATCH.with(|w| {
                    let mut u = w.borrow_mut();
                    u.hwnd = HWND::default();
                    let fonts = (u.font, u.bold, u.small);
                    u.font = HFONT::default();
                    u.bold = HFONT::default();
                    u.small = HFONT::default();
                    fonts
                });
                drop_font(font);
                drop_font(bold);
                drop_font(small);
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

unsafe fn on_scroll(h: HWND, wp: WPARAM) {
    let code = SCROLLBAR_COMMAND((wp.0 & 0xffff) as i32);
    let dpi = WATCH.with(|w| w.borrow().dpi);
    let line = sc(dpi, 22).max(1);
    let page = line * 8;
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
        } else if code == SB_BOTTOM {
            u.scroll = i32::MAX / 4;
        }
    });
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn on_move(h: HWND, x: i32, y: i32) {
    let rows = super::snapshot();
    let lay = layout(h, &rows);
    let hover = hover_at(&lay, x, y);
    let changed = WATCH.with(|w| {
        let mut u = w.borrow_mut();
        let changed = u.hover != hover;
        u.hover = hover;
        changed
    });
    if changed {
        let _ = InvalidateRect(Some(h), None, false);
    }
    let mut track = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: h,
        dwHoverTime: 0,
    };
    let _ = TrackMouseEvent(&mut track);
}

fn hover_at(lay: &Lay, x: i32, y: i32) -> Hover {
    if inside(lay.add, x, y) {
        return Hover::Add;
    }
    if inside(lay.file, x, y) {
        return Hover::File;
    }
    if inside(lay.active, x, y) {
        return Hover::Active;
    }
    for hit in &lay.rows {
        if inside(hit.run, x, y) {
            return Hover::Run(hit.id);
        }
        if inside(hit.del, x, y) {
            return Hover::Del(hit.id);
        }
        if inside(hit.row, x, y) {
            return Hover::Row(hit.id);
        }
    }
    Hover::None
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
    if inside(lay.file, x, y) {
        let paths = crate::pick_files(h);
        if !paths.is_empty() {
            add_paths(&paths);
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

unsafe fn measure(dc: HDC, font: HFONT, s: &str) -> i32 {
    let old = SelectObject(dc, font.into());
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    let n = if w.is_empty() || !GetTextExtentPoint32W(dc, &w, &mut sz).as_bool() { 0 } else { sz.cx };
    SelectObject(dc, old);
    n
}

unsafe fn layout(h: HWND, rows: &[Row]) -> Lay {
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let dpi = WATCH.with(|w| w.borrow().dpi.max(96));
    let (font, small, scroll, banner_on) = WATCH.with(|w| {
        let u = w.borrow();
        (u.font, u.small, u.scroll, !u.banner.is_empty())
    });
    let dc = GetDC(Some(h));
    let pad = sc(dpi, 8);
    let gap = sc(dpi, 4);
    let w = rc.right - rc.left;
    let height = rc.bottom - rc.top;
    let hint = RECT { left: pad, top: sc(dpi, 4), right: w - pad, bottom: sc(dpi, 26) };
    let btn_h = sc(dpi, 24);
    let btn_y = hint.bottom + sc(dpi, 2);
    let add_w = measure(dc, font, crate::tr(crate::Key::AddFolder)) + sc(dpi, 16);
    let file_w = measure(dc, font, crate::tr(crate::Key::AddFile)) + sc(dpi, 16);
    let active_w = measure(dc, font, crate::tr(crate::Key::AddActivePane)) + sc(dpi, 16);
    let add = RECT { left: pad, top: btn_y, right: pad + add_w, bottom: btn_y + btn_h };
    let file = RECT { left: add.right + gap, top: btn_y, right: add.right + gap + file_w, bottom: btn_y + btn_h };
    let active = RECT { left: file.right + gap, top: btn_y, right: file.right + gap + active_w, bottom: btn_y + btn_h };
    let mut y = btn_y + btn_h + gap;
    let banner = if banner_on {
        let r = RECT { left: pad, top: y, right: w - pad, bottom: y + sc(dpi, 18) };
        y += sc(dpi, 18) + gap;
        r
    } else {
        RECT::default()
    };
    let bar_h = sc(dpi, 22);
    let frame = RECT { left: pad, top: y, right: (w - pad).max(pad + 1), bottom: (height - bar_h - sc(dpi, 4)).max(y) };
    let head_h = sc(dpi, 22);
    let head = RECT { left: frame.left + 1, top: frame.top + 1, right: frame.right - 1, bottom: (frame.top + 1 + head_h).min(frame.bottom) };
    let list = RECT { left: head.left, top: head.bottom, right: head.right, bottom: (frame.bottom - 1).max(head.bottom) };
    let status = RECT { left: 0, top: height - bar_h, right: w, bottom: height };
    let (run_w, del_w) = action_widths(dc, small, dpi);
    let _ = ReleaseDC(Some(h), dc);
    let actions_w = run_w + gap + del_w + sc(dpi, 8);
    let sub_w = sc(dpi, 84);
    let status_w = sc(dpi, 112);
    let inner_l = list.left + sc(dpi, 4);
    let inner_r = list.right - sc(dpi, 4);
    let usable = (inner_r - inner_l - actions_w - sub_w - status_w).max(sc(dpi, 80));
    let folder_w = (usable * 58 / 100).max(sc(dpi, 48)).min(usable);
    let cols = Cols {
        folder: inner_l,
        sub: inner_l + folder_w,
        status: inner_l + folder_w + sub_w,
        last: inner_l + folder_w + sub_w + status_w,
        actions: inner_r - actions_w,
        right: inner_r,
    };
    let row_h = sc(dpi, 22).max(1);
    let content_h = (rows.len() as i32) * row_h;
    let view_h = (list.bottom - list.top).max(1);
    let max_scroll = (content_h - view_h).max(0);
    let scroll = scroll.clamp(0, max_scroll);
    WATCH.with(|w| w.borrow_mut().scroll = scroll);
    let bh = (row_h - sc(dpi, 4)).max(1);
    let mut hits = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let top = list.top - scroll + i as i32 * row_h;
        let row_r = RECT { left: list.left, top, right: list.right, bottom: top + row_h };
        let btn_top = top + (row_h - bh) / 2;
        let run = RECT { left: cols.actions, top: btn_top, right: cols.actions + run_w, bottom: btn_top + bh };
        let del = RECT { left: run.right + gap, top: btn_top, right: run.right + gap + del_w, bottom: btn_top + bh };
        let sub = if row.file {
            RECT::default()
        } else {
            RECT { left: cols.sub, top, right: cols.status, bottom: top + row_h }
        };
        hits.push(Hit { id: row.id, row: row_r, sub, run, del });
    }
    Lay { hint, add, file, active, banner, frame, head, list, status, cols, rows: hits, row_h, view_h, content_h }
}

unsafe fn action_widths(dc: HDC, font: HFONT, dpi: i32) -> (i32, i32) {
    let run = measure(dc, font, crate::tr(crate::Key::WatchStop)).max(measure(dc, font, crate::tr(crate::Key::WatchResume))) + sc(dpi, 12);
    let del = measure(dc, font, crate::tr(crate::Key::Delete)) + sc(dpi, 12);
    (run.max(sc(dpi, 40)), del.max(sc(dpi, 36)))
}

unsafe fn paint(h: HWND, dc: HDC, rc: RECT) {
    let rows = super::snapshot();
    let lay = layout(h, &rows);
    sync_scroll(h, &lay);
    let (font, bold, small, drag, hover, banner) = WATCH.with(|w| {
        let u = w.borrow();
        (u.font, u.bold, u.small, u.drag, u.hover, u.banner.clone())
    });
    let dpi = WATCH.with(|w| w.borrow().dpi.max(96));
    let pal = crate::pal();
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    crate::fill(dc, rc, pal.bg);
    crate::text(dc, lay.hint, crate::tr(crate::Key::FolderWatchHint), small, pal.sub, DT_LEFT);
    tool_btn(dc, lay.add, crate::tr(crate::Key::AddFolder), bold, true, hover == Hover::Add, dpi);
    tool_btn(dc, lay.file, crate::tr(crate::Key::AddFile), font, false, hover == Hover::File, dpi);
    tool_btn(dc, lay.active, crate::tr(crate::Key::AddActivePane), font, false, hover == Hover::Active, dpi);
    if !banner.is_empty() {
        let ink = if dark { 0xF07167 } else { 0x9F1C14 };
        crate::text(dc, lay.banner, &banner, small, ink, DT_LEFT);
    }
    if lay.frame.bottom > lay.frame.top {
        crate::fill(dc, lay.frame, pal.card);
        crate::frame(dc, lay.frame, 1, pal.line);
    }
    if lay.head.bottom > lay.head.top {
        crate::fill(dc, lay.head, pal.pill);
        header_text(dc, lay.head, &lay.cols, small, pal.sub);
        crate::fill(dc, RECT { top: lay.head.bottom - 1, bottom: lay.head.bottom, left: lay.head.left, right: lay.head.right }, pal.line);
    }
    let saved = SaveDC(dc);
    if lay.list.bottom > lay.list.top && lay.list.right > lay.list.left {
        IntersectClipRect(dc, lay.list.left, lay.list.top, lay.list.right, lay.list.bottom);
    }
    if rows.is_empty() {
        crate::text(dc, lay.list, crate::tr(crate::Key::WatchEmpty), font, pal.sub, DT_CENTER);
    }
    let slot = sc(dpi, 16);
    for (row, hit) in rows.iter().zip(&lay.rows) {
        if hit.row.bottom <= lay.list.top || hit.row.top >= lay.list.bottom {
            continue;
        }
        let mut bg = if row.paused { paused_bg(dark) } else { pal.card };
        if hover == Hover::Row(row.id) {
            bg = crate::blend(bg, pal.text, 0.04);
        }
        crate::fill(dc, hit.row, bg);
        let ix = lay.cols.folder;
        let iy = hit.row.top + (lay.row_h - slot).max(0) / 2;
        let missing = row.status == Status::Missing;
        if missing {
            crate::mark_missing(&row.path);
        } else {
            crate::mark_present(&row.path);
        }
        if let Some((list, idx, dim)) = crate::shell_icon(&row.path, row.file) {
            crate::draw_fav_icon(dc, list, idx, ix, iy, slot, dim || missing);
        }
        let path_r = RECT {
            left: ix + slot + sc(dpi, 4),
            top: hit.row.top,
            right: (lay.cols.sub - sc(dpi, 4)).max(ix + slot),
            bottom: hit.row.bottom,
        };
        crate::text(dc, path_r, &row.path, font, pal.text, DT_LEFT);
        if !row.file {
            checkbox(dc, hit.sub, row.subfolders, small, dpi);
        }
        let status_r = RECT { left: lay.cols.status + sc(dpi, 2), top: hit.row.top, right: lay.cols.last - sc(dpi, 4), bottom: hit.row.bottom };
        crate::text(dc, status_r, status_label(row), small, status_color(row.status, dark), DT_LEFT);
        let last_r = RECT { left: lay.cols.last, top: hit.row.top, right: lay.cols.actions - sc(dpi, 4), bottom: hit.row.bottom };
        crate::text(dc, last_r, &last_line(row), small, pal.sub, DT_LEFT);
        let running = !row.paused;
        let run_label = if running { crate::tr(crate::Key::WatchStop) } else { crate::tr(crate::Key::WatchResume) };
        mini_btn(dc, hit.run, run_label, small, false, hover == Hover::Run(row.id), dpi, bg);
        mini_btn(dc, hit.del, crate::tr(crate::Key::Delete), small, true, hover == Hover::Del(row.id), dpi, bg);
        crate::fill(dc, RECT { top: hit.row.bottom - 1, bottom: hit.row.bottom, left: hit.row.left, right: hit.row.right }, pal.line);
    }
    if saved != 0 {
        let _ = RestoreDC(dc, saved);
    }
    crate::fill(dc, RECT { bottom: lay.status.top + 1, ..lay.status }, pal.line);
    let watching = rows.iter().filter(|r| r.status == Status::Watching).count();
    let summary = crate::tf(crate::Key::WatchSummary, &[("n", &rows.len().to_string()), ("w", &watching.to_string())]);
    let mid = lay.status.left + (lay.status.right - lay.status.left) / 2;
    let left = RECT { left: lay.status.left + sc(dpi, 8), right: mid, ..lay.status };
    let right = RECT { left: mid, right: lay.status.right - sc(dpi, 8), ..lay.status };
    crate::text(dc, left, &summary, small, pal.sub, DT_LEFT);
    crate::text(dc, right, crate::tr(crate::Key::WatchFoot), small, pal.sub, DT_RIGHT);
    if drag {
        crate::frame(dc, rc, sc(dpi, 2).max(2), CHROME);
    }
}

fn paused_bg(dark: bool) -> u32 {
    if dark { crate::blend(crate::pal().card, 0xA07820, 0.20) } else { 0xFBF6EA }
}

fn status_label(row: &Row) -> &'static str {
    if row.file && row.status == Status::Missing {
        crate::tr(crate::Key::WatchFileMissing)
    } else {
        crate::tr(row.status.key())
    }
}

unsafe fn header_text(dc: HDC, head: RECT, cols: &Cols, font: HFONT, ink: u32) {
    let cell = |left, right| RECT { left, top: head.top, right, bottom: head.bottom };
    crate::text(dc, cell(cols.folder, cols.sub), crate::tr(crate::Key::Folder), font, ink, DT_LEFT);
    crate::text(dc, cell(cols.sub, cols.status), crate::tr(crate::Key::WatchSub), font, ink, DT_LEFT);
    crate::text(dc, cell(cols.status, cols.last), crate::tr(crate::Key::Status), font, ink, DT_LEFT);
    crate::text(dc, cell(cols.last, cols.actions), crate::tr(crate::Key::LastChange), font, ink, DT_LEFT);
    crate::text(dc, cell(cols.actions, cols.right), crate::tr(crate::Key::WatchActions), font, ink, DT_LEFT);
}

fn last_line(row: &Row) -> String {
    let (Some(ms), Some(kind)) = (row.last_ms, row.last_kind) else {
        if row.status == Status::Missing {
            return "—".to_string();
        }
        return crate::tr(crate::Key::NoChangeYet).to_string();
    };
    let when = civil_from_unix(ms);
    let kind_s = crate::tr(kind.key());
    let name = row.last_name.clone().unwrap_or_default();
    match (when.is_empty(), name.is_empty()) {
        (true, true) => kind_s.to_string(),
        (true, false) => format!("{name} {kind_s}"),
        (false, true) => format!("{when} · {kind_s}"),
        (false, false) => format!("{when} · {name} {kind_s}"),
    }
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

unsafe fn tool_btn(dc: HDC, r: RECT, label: &str, font: HFONT, primary: bool, hot: bool, dpi: i32) {
    if r.right <= r.left {
        return;
    }
    let pal = crate::pal();
    let rad = sc(dpi, 2).max(1);
    if primary {
        let fill = if hot { 0x0786BC } else { CHROME };
        crate::aa_rfill(dc, r, rad, fill, Some(fill), pal.bg);
        crate::text(dc, r, label, font, crate::on_color(fill), DT_CENTER);
    } else {
        let fill = if hot { crate::blend(pal.card, pal.text, 0.06) } else { pal.card };
        crate::aa_rfill(dc, r, rad, fill, Some(pal.line), pal.bg);
        crate::text(dc, r, label, font, pal.text, DT_CENTER);
    }
}

unsafe fn mini_btn(dc: HDC, r: RECT, label: &str, font: HFONT, danger: bool, hot: bool, dpi: i32, ground: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let pal = crate::pal();
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let ink = if danger {
        if dark { 0xF07167 } else { 0x9F1C14 }
    } else {
        pal.text
    };
    let border = if danger { crate::blend(pal.line, ink, 0.55) } else { pal.line };
    let fill = if hot { crate::blend(pal.card, pal.text, 0.06) } else { pal.card };
    crate::aa_rfill(dc, r, sc(dpi, 2).max(1), fill, Some(border), ground);
    crate::text(dc, r, label, font, ink, DT_CENTER);
}

unsafe fn checkbox(dc: HDC, r: RECT, on: bool, font: HFONT, dpi: i32) {
    if r.right <= r.left {
        return;
    }
    let pal = crate::pal();
    let side = sc(dpi, 13);
    let top = r.top + ((r.bottom - r.top) - side).max(0) / 2;
    let mark = RECT { left: r.left, top, right: r.left + side, bottom: top + side };
    if on {
        crate::fill(dc, mark, pal.text);
        let pen = CreatePen(PS_SOLID, 1, crate::cref(pal.card));
        if !pen.is_invalid() {
            let obj = HGDIOBJ(pen.0);
            let old = SelectObject(dc, obj);
            let x1 = mark.left + side / 5;
            let y1 = mark.top + side / 2;
            let x2 = mark.left + side * 2 / 5;
            let y2 = mark.bottom - side / 4;
            let x3 = mark.right - side / 5;
            let y3 = mark.top + side / 4;
            let _ = MoveToEx(dc, x1, y1, None);
            let _ = LineTo(dc, x2, y2);
            let _ = MoveToEx(dc, x2, y2, None);
            let _ = LineTo(dc, x3, y3);
            SelectObject(dc, old);
            let _ = DeleteObject(obj);
        }
    } else {
        crate::fill(dc, mark, pal.card);
        crate::frame(dc, mark, 1, pal.sub);
    }
    let label = RECT { left: mark.right + sc(dpi, 4), ..r };
    crate::text(dc, label, crate::tr(crate::Key::WatchSubShort), font, pal.text, DT_LEFT);
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
        let ok = unsafe { !hdrop_paths(&d).is_empty() };
        offer(effect, ok);
        set_drag(self.hwnd, ok);
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let hot = WATCH.with(|w| w.borrow().drag);
        offer(effect, hot);
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        set_drag(self.hwnd, false);
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let paths = unsafe { hdrop_paths(&d) };
        offer(effect, !paths.is_empty());
        set_drag(self.hwnd, false);
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

fn set_drag(h: HWND, on: bool) {
    WATCH.with(|w| w.borrow_mut().drag = on);
    unsafe { let _ = InvalidateRect(Some(h), None, false); }
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
