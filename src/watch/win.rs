// Folder Watch window. Owned by the main window.
// Caption, buttons, checks, cards, labels, and type come from the shared UI.
// A row is a folder or one file. File rows watch the parent directory and hide the subfolder checkbox.
// Watching itself does not depend on this window staying open.

use std::sync::Once;

use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES};
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::ReleaseStgMedium;
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

use super::model::{format_civil, status_color, Row, Status, CHROME};
use crate::imports::*;
use crate::{px, BTN_GAP, BTN_H_TOOL, BTN_MIN_W, BTN_PAD_X, FONT_META_PX, FONT_PX, PAD_BODY_X, PAD_BODY_Y, RADIUS, SPACE_4, SPACE_8};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hover {
    None,
    Add,
    File,
    Active,
    Run(u64),
    Del(u64),
    Sub(u64),
    Row(u64),
}

struct WatchWin {
    hwnd: HWND,
    scroll: i32,
    drag: bool,
    hover: Hover,
    track: crate::Tracker<Hover>,
    banner: String,
}

impl WatchWin {
    fn blank() -> Self {
        WatchWin {
            hwnd: HWND::default(),
            scroll: 0,
            drag: false,
            hover: Hover::None,
            track: crate::Tracker::new(),
            banner: String::new(),
        }
    }
}

/// Same row, header, and status line as the file basket.
const ROW_DIP: i32 = 22;
const HEAD_DIP: i32 = 22;
const BAR_DIP: i32 = 22;

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
    name: i32,
    path: i32,
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
    let (ww, hh) = (px(dpi, 760), px(dpi, 440));
    let (x, y) = place(owner, ww, hh);
    let style = WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN | WS_VISIBLE;
    let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroWatch"), w!("Maguro"), style, x, y, ww, hh, Some(owner), None, None, None) else { return };
    WATCH.with(|w| {
        let mut u = w.borrow_mut();
        *u = WatchWin::blank();
        u.hwnd = h;
    });
    crate::style_dialog_caption(h);
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
    crate::style_dialog_caption(h);
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

fn dpi_of(h: HWND) -> i32 {
    unsafe { GetDpiForWindow(h).max(96) as i32 }
}

/// Text-button width: the label plus the shared horizontal padding, and at least [`BTN_MIN_W`].
fn button_width(text_w: i32, dpi: i32) -> i32 {
    (text_w + px(dpi, BTN_PAD_X) * 2).max(px(dpi, BTN_MIN_W))
}

/// Hints, column titles, and the footer. The app meta face, or the body face before it exists.
unsafe fn meta_font() -> HFONT {
    if let Some(font) = crate::app_try(|a| a.small_font) {
        if !font.0.is_null() && !font.is_invalid() {
            return font;
        }
    }
    crate::body_font()
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
                let line = px(dpi_of(h), ROW_DIP).max(1) * 3;
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
            WATCH.with(|w| {
                let mut u = w.borrow_mut();
                u.hover = Hover::None;
                u.track.pointer_leave();
            });
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = crate::xy(lp);
            let rows = super::snapshot();
            let lay = layout(h, &rows);
            if let Some(id) = button_of(hover_at(&lay, x, y)) {
                WATCH.with(|w| w.borrow_mut().track.pointer_down(id));
                let _ = SetCapture(h);
                let _ = InvalidateRect(Some(h), None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = crate::xy(lp);
            on_click(h, x, y);
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            let effect = WATCH.with(|w| w.borrow_mut().track.capture_lost());
            if !matches!(effect, crate::TrackEffect::None) {
                let _ = InvalidateRect(Some(h), None, false);
            }
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            let _ = DestroyWindow(h);
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let rc = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(h, None, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOACTIVATE);
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let dpi = dpi_of(h);
            let m = &mut *(lp.0 as *mut MINMAXINFO);
            m.ptMinTrackSize.x = px(dpi, 640);
            m.ptMinTrackSize.y = px(dpi, 360);
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
                WATCH.with(|w| w.borrow_mut().hwnd = HWND::default());
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

unsafe fn on_scroll(h: HWND, wp: WPARAM) {
    let code = SCROLLBAR_COMMAND((wp.0 & 0xffff) as i32);
    let line = px(dpi_of(h), ROW_DIP).max(1);
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
    let at = hover_at(&lay, x, y);
    let btn = button_of(at);
    let changed = WATCH.with(|w| {
        let mut u = w.borrow_mut();
        let row = if btn.is_some() || u.track.down().is_some() {
            Hover::None
        } else {
            match at {
                Hover::Row(id) => Hover::Row(id),
                Hover::Sub(id) => Hover::Sub(id),
                _ => Hover::None,
            }
        };
        let effect = u.track.pointer_move(btn);
        let changed = u.hover != row || !matches!(effect, crate::TrackEffect::None);
        u.hover = row;
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

fn button_of(hover: Hover) -> Option<Hover> {
    match hover {
        Hover::Add | Hover::File | Hover::Active | Hover::Run(_) | Hover::Del(_) => Some(hover),
        _ => None,
    }
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
        if inside(hit.sub, x, y) {
            return Hover::Sub(hit.id);
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
    let at = hover_at(&lay, x, y);
    let effect = WATCH.with(|w| w.borrow_mut().track.pointer_up(button_of(at)));
    if GetCapture() == h {
        let _ = ReleaseCapture();
    }
    match effect {
        crate::TrackEffect::Click(id) => press_button(h, id),
        crate::TrackEffect::None => {
            if let Hover::Sub(id) = at {
                super::toggle_subfolders(id);
            }
        }
        _ => {}
    }
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn press_button(h: HWND, id: Hover) {
    match id {
        Hover::Add => {
            if let Some(path) = crate::pick_folder(h) {
                add_paths(&[path]);
            }
        }
        Hover::File => {
            let paths = crate::pick_files(h);
            if !paths.is_empty() {
                add_paths(&paths);
            }
        }
        Hover::Active => {
            set_banner(super::add_active());
            redraw();
        }
        Hover::Run(id) => super::toggle_run(id),
        Hover::Del(id) => super::delete(id),
        _ => {}
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
    let dpi = dpi_of(h);
    let (scroll, banner_on) = WATCH.with(|w| {
        let u = w.borrow();
        (u.scroll, !u.banner.is_empty())
    });
    let dc = GetDC(Some(h));
    let font = crate::body_font();
    let small = meta_font();
    let pad = px(dpi, PAD_BODY_X);
    let gap = px(dpi, BTN_GAP);
    let tight = px(dpi, SPACE_4);
    let w = rc.right - rc.left;
    let height = rc.bottom - rc.top;
    let hint_top = px(dpi, PAD_BODY_Y);
    let hint = RECT { left: pad, top: hint_top, right: (w - pad).max(pad), bottom: hint_top + px(dpi, FONT_PX + SPACE_8) };
    let btn_h = px(dpi, BTN_H_TOOL);
    let btn_y = hint.bottom + tight;
    let add_w = button_width(measure(dc, font, crate::tr(crate::Key::AddFolder)), dpi);
    let file_w = button_width(measure(dc, font, crate::tr(crate::Key::AddFile)), dpi);
    let active_w = button_width(measure(dc, font, crate::tr(crate::Key::AddActivePane)), dpi);
    let add = RECT { left: pad, top: btn_y, right: pad + add_w, bottom: btn_y + btn_h };
    let file = RECT { left: add.right + gap, top: btn_y, right: add.right + gap + file_w, bottom: btn_y + btn_h };
    let active = RECT { left: file.right + gap, top: btn_y, right: file.right + gap + active_w, bottom: btn_y + btn_h };
    let mut y = btn_y + btn_h + px(dpi, SPACE_8);
    let banner = if banner_on {
        let bh = px(dpi, FONT_META_PX + SPACE_8);
        let r = RECT { left: pad, top: y, right: (w - pad).max(pad), bottom: y + bh };
        y += bh + tight;
        r
    } else {
        RECT::default()
    };
    let bar_h = px(dpi, BAR_DIP);
    let frame = RECT { left: pad, top: y, right: (w - pad).max(pad + 1), bottom: (height - bar_h - tight).max(y) };
    let rad = px(dpi, RADIUS).max(1);
    let head = RECT {
        left: frame.left + 1,
        top: frame.top + rad,
        right: (frame.right - 1).max(frame.left + 2),
        bottom: (frame.top + rad + px(dpi, HEAD_DIP)).min(frame.bottom),
    };
    let list = RECT { left: head.left, top: head.bottom, right: head.right, bottom: (frame.bottom - 1).max(head.bottom) };
    let status = RECT { left: 0, top: height - bar_h, right: w, bottom: height };
    let (run_w, del_w) = action_widths(dc, small, dpi);
    let sub_text = measure(dc, small, crate::tr(crate::Key::WatchSub))
        .max(measure(dc, small, crate::tr(crate::Key::WatchSubShort)) + px(dpi, 16) + px(dpi, SPACE_8));
    let _ = ReleaseDC(Some(h), dc);
    let actions_w = run_w + gap + del_w + px(dpi, SPACE_8);
    let sub_w = sub_text + px(dpi, 10);
    let status_w = px(dpi, 112);
    let inner_l = list.left + tight;
    let inner_r = list.right - tight;
    let usable = (inner_r - inner_l - actions_w - sub_w - status_w).max(0);
    let name_min = px(dpi, 96).min(usable);
    let name_max = (usable * 55 / 100).max(name_min);
    let name_w = (usable * 38 / 100).clamp(name_min, name_max);
    let path_w = ((usable - name_w) * 62 / 100).max(0);
    let cols = Cols {
        name: inner_l,
        path: inner_l + name_w,
        sub: inner_l + name_w + path_w,
        status: inner_l + name_w + path_w + sub_w,
        last: inner_l + name_w + path_w + sub_w + status_w,
        actions: inner_r - actions_w,
        right: inner_r,
    };
    let row_h = px(dpi, ROW_DIP).max(1);
    let content_h = (rows.len() as i32) * row_h;
    let view_h = (list.bottom - list.top).max(1);
    let max_scroll = (content_h - view_h).max(0);
    let scroll = scroll.clamp(0, max_scroll);
    WATCH.with(|w| w.borrow_mut().scroll = scroll);
    let bh = (row_h - tight).max(1);
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
    let pad = px(dpi, BTN_PAD_X) * 2;
    let run = measure(dc, font, crate::tr(crate::Key::WatchStop)).max(measure(dc, font, crate::tr(crate::Key::WatchResume))) + pad;
    let del = measure(dc, font, crate::tr(crate::Key::Delete)) + pad;
    (run.max(px(dpi, 40)), del.max(px(dpi, 36)))
}

unsafe fn paint(h: HWND, dc: HDC, rc: RECT) {
    let rows = super::snapshot();
    let lay = layout(h, &rows);
    sync_scroll(h, &lay);
    let (drag, hover, track, banner) = WATCH.with(|w| {
        let u = w.borrow();
        (u.drag, u.hover, u.track, u.banner.clone())
    });
    let dpi = dpi_of(h);
    let font = crate::body_font();
    let bold = crate::bold_ui_font();
    let small = meta_font();
    let t = crate::ui_tokens();
    let dark = t.dark;
    crate::paint_window(dc, rc);
    crate::paint_meta(dc, lay.hint, crate::tr(crate::Key::FolderWatchHint), small);
    crate::paint_button(dc, lay.add, crate::ButtonKind::Primary, track.visual(Hover::Add, false), crate::tr(crate::Key::AddFolder), font, dpi, t.bg, crate::FocusRing::Outside);
    crate::paint_button(dc, lay.file, crate::ButtonKind::Secondary, track.visual(Hover::File, false), crate::tr(crate::Key::AddFile), font, dpi, t.bg, crate::FocusRing::Outside);
    crate::paint_button(dc, lay.active, crate::ButtonKind::Secondary, track.visual(Hover::Active, false), crate::tr(crate::Key::AddActivePane), font, dpi, t.bg, crate::FocusRing::Outside);
    if !banner.is_empty() {
        crate::text(dc, lay.banner, &banner, small, t.danger, DT_LEFT);
    }
    if lay.frame.bottom > lay.frame.top {
        crate::ui::paint_card(dc, lay.frame, dpi);
    }
    if lay.head.bottom > lay.head.top {
        crate::fill(dc, lay.head, crate::blend(t.card, t.bg, 0.70));
        header_text(dc, lay.head, &lay.cols, small);
        crate::fill(dc, RECT { top: lay.head.bottom - 1, bottom: lay.head.bottom, left: lay.head.left, right: lay.head.right }, t.line);
    }
    let saved = SaveDC(dc);
    if lay.list.bottom > lay.list.top && lay.list.right > lay.list.left {
        IntersectClipRect(dc, lay.list.left, lay.list.top, lay.list.right, lay.list.bottom);
    }
    if rows.is_empty() {
        crate::text(dc, lay.list, crate::tr(crate::Key::WatchEmpty), font, t.sub, DT_CENTER);
    }
    let slot = px(dpi, 16);
    let tight = px(dpi, SPACE_4);
    for (row, hit) in rows.iter().zip(&lay.rows) {
        if hit.row.bottom <= lay.list.top || hit.row.top >= lay.list.bottom {
            continue;
        }
        let mut bg = if row.paused { paused_bg(dark) } else { t.card };
        let row_hot = matches!(hover, Hover::Row(id) | Hover::Sub(id) if id == row.id);
        if row_hot {
            bg = crate::blend(bg, t.text, 0.04);
        }
        crate::fill(dc, hit.row, bg);
        let ix = lay.cols.name;
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
        let name_r = RECT {
            left: ix + slot + tight,
            top: hit.row.top,
            right: (lay.cols.path - tight).max(ix + slot),
            bottom: hit.row.bottom,
        };
        crate::paint_label(dc, name_r, &super::model::leaf_name(&row.path), bold);
        let path_r = RECT {
            left: lay.cols.path,
            top: hit.row.top,
            right: (lay.cols.sub - tight).max(lay.cols.path),
            bottom: hit.row.bottom,
        };
        let parent = super::model::parent_of(&row.path);
        let shown = fit_path(dc, small, &parent, path_r.right - path_r.left);
        crate::paint_meta(dc, path_r, &shown, small);
        if !row.file {
            let box_hot = matches!(hover, Hover::Sub(id) if id == row.id);
            checkbox(dc, hit.sub, row.subfolders, small, dpi, box_hot, bg);
        }
        let status_r = RECT { left: lay.cols.status + px(dpi, 2), top: hit.row.top, right: lay.cols.last - tight, bottom: hit.row.bottom };
        crate::text(dc, status_r, status_label(row), small, status_color(row.status, dark), DT_LEFT);
        let last_r = RECT { left: lay.cols.last, top: hit.row.top, right: lay.cols.actions - tight, bottom: hit.row.bottom };
        crate::paint_meta(dc, last_r, &last_line(row), small);
        let running = !row.paused;
        let run_label = if running { crate::tr(crate::Key::WatchStop) } else { crate::tr(crate::Key::WatchResume) };
        crate::paint_button(dc, hit.run, crate::ButtonKind::Secondary, track.visual(Hover::Run(row.id), false), run_label, small, dpi, bg, crate::FocusRing::Outside);
        crate::paint_button(dc, hit.del, crate::ButtonKind::Danger, track.visual(Hover::Del(row.id), false), crate::tr(crate::Key::Delete), small, dpi, bg, crate::FocusRing::Outside);
        crate::fill(dc, RECT { top: hit.row.bottom - 1, bottom: hit.row.bottom, left: hit.row.left, right: hit.row.right }, t.line);
    }
    if saved != 0 {
        let _ = RestoreDC(dc, saved);
    }
    crate::fill(dc, RECT { bottom: lay.status.top + 1, ..lay.status }, t.line);
    let watching = rows.iter().filter(|r| r.status == Status::Watching).count();
    let summary = crate::tf(crate::Key::WatchSummary, &[("n", &rows.len().to_string()), ("w", &watching.to_string())]);
    let pad = px(dpi, PAD_BODY_X);
    let mid = lay.status.left + (lay.status.right - lay.status.left) / 2;
    let left = RECT { left: lay.status.left + pad, right: mid, ..lay.status };
    let right = RECT { left: mid, right: lay.status.right - pad, ..lay.status };
    crate::paint_meta(dc, left, &summary, small);
    crate::text(dc, right, crate::tr(crate::Key::WatchFoot), small, t.sub, DT_RIGHT);
    if drag {
        crate::frame(dc, rc, px(dpi, 2).max(2), CHROME);
    }
}

fn paused_bg(dark: bool) -> u32 {
    let card = crate::tokens_for(dark).card;
    if dark { crate::blend(card, 0xA07820, 0.20) } else { 0xFBF6EA }
}

fn status_label(row: &Row) -> &'static str {
    if row.file && row.status == Status::Missing {
        crate::tr(crate::Key::WatchFileMissing)
    } else {
        crate::tr(row.status.key())
    }
}

unsafe fn header_text(dc: HDC, head: RECT, cols: &Cols, font: HFONT) {
    let cell = |left, right| RECT { left, top: head.top, right, bottom: head.bottom };
    crate::paint_meta(dc, cell(cols.name, cols.path), crate::tr(crate::Key::Name), font);
    crate::paint_meta(dc, cell(cols.path, cols.sub), crate::tr(crate::Key::Path), font);
    crate::paint_meta(dc, cell(cols.sub, cols.status), crate::tr(crate::Key::WatchSub), font);
    crate::paint_meta(dc, cell(cols.status, cols.last), crate::tr(crate::Key::Status), font);
    crate::paint_meta(dc, cell(cols.last, cols.actions), crate::tr(crate::Key::LastChange), font);
    crate::paint_meta(dc, cell(cols.actions, cols.right), crate::tr(crate::Key::WatchActions), font);
}

/// Keep the tail of a long parent path. `PathCompactPathW` puts the ellipsis in the middle.
unsafe fn fit_path(dc: HDC, font: HFONT, path: &str, width: i32) -> String {
    if path.is_empty() || width <= 8 {
        return path.to_string();
    }
    let mut wide = [0u16; 260];
    let units: Vec<u16> = path.encode_utf16().collect();
    let src = if units.len() > 259 { &units[units.len() - 259..] } else { units.as_slice() };
    wide[..src.len()].copy_from_slice(src);
    let old = SelectObject(dc, font.into());
    let _ = PathCompactPathW(Some(dc), &mut wide, width.max(0) as u32);
    SelectObject(dc, old);
    let n = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..n])
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

unsafe fn checkbox(dc: HDC, r: RECT, on: bool, font: HFONT, dpi: i32, hot: bool, ground: u32) {
    if r.right <= r.left {
        return;
    }
    let side = px(dpi, 16);
    let top = r.top + ((r.bottom - r.top) - side).max(0) / 2;
    let mark = RECT { left: r.left, top, right: r.left + side, bottom: top + side };
    crate::paint_check(dc, mark, on, hot, dpi, ground);
    let label = RECT { left: mark.right + px(dpi, SPACE_8), ..r };
    crate::paint_label(dc, label, crate::tr(crate::Key::WatchSubShort), font);
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
