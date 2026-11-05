// Favourites window opened from the star on the tab row.
// One window at a time. It edits the same tree as Settings and the left panel.
// Explorer pins and favourite files lead the list, with the same labels and collapse as the left panel.

use crate::app::{app, bind_hwnd, ime_composing};
use crate::open_fav_row;
use crate::i18n::tr;
use crate::imports::*;
use crate::session::*;
use crate::settings;

use super::{
    blend, buffered, button_colors, clear_icon_refresh, drop_pop_fav_tip, drop_side, fav_chevron_rect, fav_popup_can_drop, fav_row_editable, files_section_label, fill, hide_pop_fav_tip, note_side,
    paint_button, paint_card, paint_fav_head, paint_fav_row, paint_field, paint_window, pal, pins_section_label, place_edit, popup_rows, show_pop_fav_tip, text, tip_drop, tip_enter, tip_leave,
    draw_line_icon, LineIcon,
    tip_screen_rect, ui_tokens, ACCENT, BTN_GAP, BTN_H_FOOTER, BTN_H_TOOL, BTN_MIN_W, ButtonKind, ButtonVisual, FIELD_H, FIELD_PAD_X, FONT_META_PX, FONT_PX, FocusRing, PAD_BODY_X, PAD_BODY_Y, RADIUS,
    TrackEffect, Tracker, TipSide,
};

const ID_SEARCH: i32 = 1;
/// Movement before a press becomes a reorder. A click must not move the row.
const DRAG_SLOP: i32 = 4;
/// Compact card row, DIP. The settings table keeps the taller button row.
const MGR_ROW: i32 = 22;
const MGR_HEAD: i32 = 22;
const ICON_N: usize = 6;
/// Tracking tip for the toolbar glyphs. Not the row tip (11) or the find tip (13).
const MGR_TIP: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Add,
    Group,
    Up,
    Down,
    Rename,
    Del,
    Expand,
    Close,
}

const TOOL: [Act; ICON_N] = [Act::Add, Act::Group, Act::Up, Act::Down, Act::Rename, Act::Del];

/// Which row the keyboard and the highlight are on. Kind is part of the identity:
/// a pin and a Maguro favourite can share the same index.
#[derive(Clone, Debug, PartialEq)]
struct Sel {
    kind: RowKind,
    id: Vec<usize>,
}

struct St {
    hwnd: HWND,
    owner: HWND,
    dpi: i32,
    query: String,
    scroll: i32,
    sel: Option<Sel>,
    drag: Option<Vec<usize>>,
    drag_at: (i32, i32),
    drag_ready: bool,
    drag_moved: bool,
    /// Client point where a scroll or a content change hid the location tip.
    tip_hold: Option<(i32, i32)>,
    /// Identity of the rows last painted. Unchanged pin refreshes keep the tip.
    rows_mark: Option<u64>,
    font: HFONT,
    semi: HFONT,
    small: HFONT,
    icon: HFONT,
    track: Tracker<Act>,
    hot_row: Option<usize>,
    expand_w: i32,
    tip_on: bool,
}

impl Default for St {
    fn default() -> Self {
        St {
            hwnd: HWND::default(),
            owner: HWND::default(),
            dpi: 96,
            query: String::new(),
            scroll: 0,
            sel: None,
            drag: None,
            drag_at: (0, 0),
            drag_ready: false,
            drag_moved: false,
            tip_hold: None,
            rows_mark: None,
            font: HFONT::default(),
            semi: HFONT::default(),
            small: HFONT::default(),
            icon: HFONT::default(),
            track: Tracker::new(),
            hot_row: None,
            expand_w: 0,
            tip_on: false,
        }
    }
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
    static PENDING_OWNER: Cell<HWND> = Cell::new(HWND::default());
}

struct PopLay {
    hint: RECT,
    icons: [RECT; ICON_N],
    sep: RECT,
    search: RECT,
    edit: RECT,
    list: RECT,
    head: RECT,
    body: RECT,
    expand: RECT,
    close: RECT,
}

fn px(dpi: i32, v: i32) -> i32 {
    v * dpi.max(96) / 96
}

/// Hint, icon toolbar, search field, card, and the expand / close footer.
/// `expand_w` is the measured link width in pixels.
fn pop_lay(w: i32, h: i32, dpi: i32, expand_w: i32) -> PopLay {
    let dpi = dpi.max(96);
    let s = |v: i32| px(dpi, v);
    let margin = s(PAD_BODY_X);
    let gap = s(10);
    let right = (w - margin).max(margin + 1);
    let mut y = s(PAD_BODY_Y);
    let hint = RECT { left: margin, top: y, right, bottom: y + s(16) };
    y = hint.bottom + gap;
    let tool_h = s(FIELD_H).max(s(BTN_H_TOOL));
    let icon = s(BTN_H_TOOL);
    let iy = y + (tool_h - icon) / 2;
    let ig = s(4);
    let mut x = margin;
    let mut icons = [RECT::default(); ICON_N];
    icons[0] = RECT { left: x, top: iy, right: x + icon, bottom: iy + icon };
    x += icon + ig;
    icons[1] = RECT { left: x, top: iy, right: x + icon, bottom: iy + icon };
    x += icon + s(6);
    let sep = RECT { left: x, top: iy + s(4), right: x + 1, bottom: (iy + icon - s(4)).max(iy + s(8)) };
    x = sep.right + s(6);
    for i in 2..ICON_N {
        icons[i] = RECT { left: x, top: iy, right: x + icon, bottom: iy + icon };
        x += icon + ig;
    }
    let field_h = s(FIELD_H);
    let search_top = y + (tool_h - field_h) / 2;
    let search_w = s(180).min((right - x - s(8)).max(s(72)));
    let search = RECT { left: (right - search_w).max(x), top: search_top, right, bottom: search_top + field_h };
    let inset = s(2);
    let pad = s(FIELD_PAD_X);
    let edit = RECT {
        left: search.left + pad,
        top: search.top + inset,
        right: (search.right - pad).max(search.left + pad + 1),
        bottom: (search.bottom - inset).max(search.top + inset + 1),
    };
    y += tool_h + gap;
    let foot_h = s(BTN_H_FOOTER);
    let foot_top = (h - s(PAD_BODY_Y) - foot_h).max(y + s(48));
    let list = RECT { left: margin, top: y, right, bottom: (foot_top - gap).max(y + 1) };
    let rad = s(RADIUS).max(1);
    let head = RECT {
        left: list.left + 1,
        top: list.top + rad,
        right: (list.right - 1).max(list.left + 2),
        bottom: list.top + rad + s(MGR_HEAD),
    };
    let body = RECT { left: head.left, top: head.bottom, right: head.right, bottom: (list.bottom - 1).max(head.bottom) };
    let close_w = s(88).max(s(BTN_MIN_W));
    let close = RECT { left: (right - close_w).max(margin), top: foot_top, right, bottom: foot_top + foot_h };
    let exp_w = expand_w.max(s(48)).min((close.left - s(BTN_GAP) - margin).max(s(48)));
    let expand = RECT { left: margin, top: foot_top, right: margin + exp_w, bottom: foot_top + foot_h };
    PopLay { hint, icons, sep, search, edit, list, head, body, expand, close }
}

fn lay_client(w: i32, h: i32, dpi: i32) -> PopLay {
    let expand_w = ST.with(|s| s.borrow().expand_w);
    pop_lay(w, h, dpi, expand_w)
}

fn client_lay(h: HWND) -> PopLay {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    lay_client(rc.right, rc.bottom, dpi)
}

fn hwnd() -> HWND {
    ST.with(|s| s.borrow().hwnd)
}

fn bind_owner() {
    let owner = ST.with(|s| s.borrow().owner);
    if !owner.0.is_null() {
        crate::bind_hwnd(owner);
    }
}

fn remember_on_desk(h: HWND) {
    app(|a| a.fav_mgr = h);
}

pub(crate) unsafe fn fav_manager_captures(h: HWND) -> bool {
    let w = hwnd();
    !w.0.is_null() && IsWindow(Some(w)).as_bool() && (h == w || IsChild(w, h).as_bool())
}

pub(crate) unsafe fn open_fav_manager(owner: HWND) {
    ensure_class();
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = ShowWindow(h, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(h);
        return;
    }
    let dpi = GetDpiForWindow(owner).max(96) as i32;
    let (ww, hh) = (px(dpi, 880), px(dpi, 560));
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    PENDING_OWNER.with(|c| c.set(owner));
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VSCROLL;
    let created = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroFavMgr"), w!(""), style, x, y, ww, hh, Some(owner), None, None, None).unwrap_or_default();
    if created.0.is_null() {
        return;
    }
    style_window(created);
    let _ = ShowWindow(created, SW_SHOW);
    let _ = SetForegroundWindow(created);
    if let Ok(edit) = GetDlgItem(Some(created), ID_SEARCH) {
        let _ = SetFocus(Some(edit));
    }
}

pub(crate) unsafe fn close_fav_manager() {
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = DestroyWindow(h);
    }
}

pub(crate) unsafe fn refresh_fav_manager() {
    let h = hwnd();
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    bind_owner();
    // The rows on screen may have changed. The tip waits until the pointer moves.
    pop_tip_hold(h);
    let dpi = GetDpiForWindow(h).max(96) as i32;
    ST.with(|s| s.borrow_mut().dpi = dpi);
    style_window(h);
    rebuild_fonts();
    let font = ST.with(|s| s.borrow().font);
    each_child(h, |c| {
        SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        theme_ctl(c);
    });
    retitle(h);
    layout(h);
    let _ = InvalidateRect(Some(h), None, true);
}

pub(crate) unsafe fn fav_manager_on_key(vk: u16, ctrl: bool, alt: bool) -> bool {
    if ctrl || alt {
        return false;
    }
    bind_owner();
    let h = hwnd();
    let search = search_focused(h);
    let button = is_button(GetFocus());
    match VIRTUAL_KEY(vk) {
        VK_ESCAPE => {
            close_fav_manager();
            true
        }
        VK_RETURN if !button => {
            fav_manager_activate();
            true
        }
        VK_UP => {
            fav_manager_move(false, 1);
            true
        }
        VK_DOWN => {
            fav_manager_move(true, 1);
            true
        }
        VK_PRIOR => {
            fav_manager_move(false, page_step(h));
            true
        }
        VK_NEXT => {
            fav_manager_move(true, page_step(h));
            true
        }
        VK_HOME if !search => {
            fav_manager_edge(false);
            true
        }
        VK_END if !search => {
            fav_manager_edge(true);
            true
        }
        VK_DELETE if !search => {
            fav_manager_delete_sel();
            true
        }
        VK_F2 if !search => {
            fav_manager_rename_sel();
            true
        }
        _ => false,
    }
}

fn ensure_class() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(fav_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(hinst), PCWSTR(crate::ICON_GROUP_LIGHT as _)).unwrap_or_default(),
            lpszClassName: w!("MaguroFavMgr"),
            ..Default::default()
        });
    });
}

unsafe fn style_window(h: HWND) {
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = crate::cref(pal().bg);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
}

fn make_font(face: &str, size: i32, weight: i32, dpi: i32) -> HFONT {
    let mut lf = LOGFONTW {
        lfHeight: -px(dpi, size).max(1),
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

fn rebuild_fonts() {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let custom = app(|a| a.s.font_face.clone());
    let fallback = crate::ui_face();
    let face = if custom.is_empty() { fallback } else { custom.as_str() };
    let new = (
        make_font(face, FONT_PX, 400, dpi),
        make_font(face, FONT_PX, 600, dpi),
        make_font(face, FONT_META_PX, 400, dpi),
        make_font("Segoe MDL2 Assets", 14, 400, dpi),
    );
    let old = ST.with(|s| {
        let mut st = s.borrow_mut();
        let old = (st.font, st.semi, st.small, st.icon);
        (st.font, st.semi, st.small, st.icon) = new;
        old
    });
    unsafe {
        for f in [old.0, old.1, old.2, old.3] {
            crate::delete_font(f);
        }
    }
}

unsafe fn theme_ctl(h: HWND) {
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let name = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

fn ctl(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

fn each_child(h: HWND, mut f: impl FnMut(HWND)) {
    let c = ctl(h, ID_SEARCH);
    if !c.0.is_null() {
        f(c);
    }
}

unsafe fn retitle(h: HWND) {
    let _ = SetWindowTextW(h, &HSTRING::from(tr(crate::Key::Favourites)));
    cue(ctl(h, ID_SEARCH), tr(crate::Key::SearchFavourites));
}

fn act_on(id: Act) -> bool {
    match id {
        Act::Add | Act::Group | Act::Expand | Act::Close => true,
        Act::Up | Act::Down | Act::Rename | Act::Del => {
            let rows = shown();
            sel_index(&rows).and_then(|i| rows.get(i)).is_some_and(fav_row_editable)
        }
    }
}

fn act_icon(id: Act) -> Option<LineIcon> {
    Some(match id {
        Act::Add => LineIcon::Add,
        Act::Group => LineIcon::Group,
        Act::Up => LineIcon::Up,
        Act::Down => LineIcon::Down,
        Act::Rename => LineIcon::Rename,
        Act::Del => LineIcon::Del,
        Act::Expand | Act::Close => return None,
    })
}

fn act_tip(id: Act) -> &'static str {
    match id {
        Act::Add => tr(crate::Key::AddFolder),
        Act::Group => tr(crate::Key::NewGroup),
        Act::Up => tr(crate::Key::FavMoveUp),
        Act::Down => tr(crate::Key::FavMoveDown),
        Act::Rename => tr(crate::Key::Rename2),
        Act::Del => tr(crate::Key::Delete),
        Act::Expand => tr(crate::Key::FavExpandAll),
        Act::Close => tr(crate::Key::Close),
    }
}

fn hit_act(lay: &PopLay, x: i32, y: i32) -> Option<Act> {
    for (i, r) in lay.icons.iter().enumerate() {
        if inside(*r, x, y) {
            return Some(TOOL[i]);
        }
    }
    if inside(lay.expand, x, y) {
        return Some(Act::Expand);
    }
    if inside(lay.close, x, y) {
        return Some(Act::Close);
    }
    None
}

fn act_rect(lay: &PopLay, id: Act) -> RECT {
    match id {
        Act::Add => lay.icons[0],
        Act::Group => lay.icons[1],
        Act::Up => lay.icons[2],
        Act::Down => lay.icons[3],
        Act::Rename => lay.icons[4],
        Act::Del => lay.icons[5],
        Act::Expand => lay.expand,
        Act::Close => lay.close,
    }
}

unsafe fn paint_icon(dc: HDC, r: RECT, id: Act, visual: ButtonVisual, font: HFONT, dpi: i32, ground: u32) {
    paint_button(dc, r, ButtonKind::Icon, visual, "", font, dpi, ground, FocusRing::Outside);
    let ink = button_colors(ButtonKind::Icon, visual.state, ui_tokens().dark, ground).ink;
    if let Some(kind) = act_icon(id) {
        draw_line_icon(dc, r, kind, ink, dpi);
    }
}

unsafe fn button_tip(h: HWND, id: Option<Act>, rect: RECT) {
    let inst = ST.with(|s| s.borrow().tip_on);
    let Some(id) = id else {
        let (now, _) = tip_leave(h, MGR_TIP, inst);
        ST.with(|s| s.borrow_mut().tip_on = now);
        return;
    };
    let (now, _) = tip_enter(h, MGR_TIP, id as u64, act_tip(id), tip_screen_rect(h, rect), TipSide::Below, inst);
    ST.with(|s| s.borrow_mut().tip_on = now);
}

unsafe fn measure_label(font: HFONT, s: &str) -> i32 {
    if font.is_invalid() || s.is_empty() {
        return 0;
    }
    let dc = GetDC(None);
    if dc.is_invalid() {
        return 0;
    }
    let w: Vec<u16> = s.encode_utf16().collect();
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    let ok = GetTextExtentPoint32W(dc, &w, &mut sz).as_bool();
    SelectObject(dc, old);
    let _ = ReleaseDC(None, dc);
    if ok { sz.cx } else { 0 }
}

unsafe fn cue(edit: HWND, text: &str) {
    if edit.0.is_null() {
        return;
    }
    let hs = HSTRING::from(text);
    SendMessageW(edit, 0x1501, Some(WPARAM(1)), Some(LPARAM(hs.as_ptr() as isize)));
    // The edit window is already inset by FIELD_PAD_X. A second margin would double the pad.
    SendMessageW(edit, 0x00D3, Some(WPARAM(3)), Some(LPARAM(0)));
}

fn shown() -> Vec<FavRow> {
    let q = ST.with(|s| s.borrow().query.clone());
    let pins_label = pins_section_label();
    let files_label = files_section_label();
    app(|a| {
        a.s.ensure_fav_tree();
        let pins = crate::pin_list();
        let files = crate::pin_files();
        popup_rows(&a.s.tree, a.s.pins_open, &pins, pins_label, a.s.files_open, &files, files_label, &q)
    })
}

fn sel_from(row: &FavRow) -> Sel {
    Sel { kind: row.kind, id: row.id.clone() }
}

fn sel_index(rows: &[FavRow]) -> Option<usize> {
    let sel = ST.with(|s| s.borrow().sel.clone())?;
    rows.iter().position(|r| r.kind == sel.kind && r.id == sel.id)
}

fn show_fav_hint(rows: &[FavRow], query: &str) -> bool {
    query.trim().is_empty() && !rows.iter().any(|r| r.in_tree())
}

fn empty_msg(query: &str) -> &'static str {
    if query.trim().is_empty() {
        tr(crate::Key::NoFavouritesYetAddOneWithIn)
    } else {
        tr(crate::Key::NoMatches)
    }
}

unsafe fn search_focused(h: HWND) -> bool {
    GetFocus() == ctl(h, ID_SEARCH)
}

unsafe fn is_button(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    let mut c = [0u16; 32];
    let n = GetClassNameW(h, &mut c) as usize;
    String::from_utf16_lossy(&c[..n]).eq_ignore_ascii_case("Button")
}

fn page_step(h: HWND) -> usize {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    let lay = lay_client(rc.right, rc.bottom, dpi);
    let view = (lay.body.bottom - lay.body.top).max(1);
    (view / px(dpi, MGR_ROW)).max(1) as usize
}

unsafe fn fav_manager_move(down: bool, step: usize) {
    let h = hwnd();
    if h.0.is_null() {
        return;
    }
    bind_owner();
    let rows = shown();
    if rows.is_empty() {
        return;
    }
    let step = step.max(1);
    let cur = sel_index(&rows);
    let next = match cur {
        None => {
            if down {
                0
            } else {
                rows.len() - 1
            }
        }
        Some(i) if down => (i + step).min(rows.len() - 1),
        Some(i) => i.saturating_sub(step),
    };
    ST.with(|s| s.borrow_mut().sel = Some(sel_from(&rows[next])));
    reveal(h, next);
    layout(h);
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn fav_manager_edge(last: bool) {
    let h = hwnd();
    if h.0.is_null() {
        return;
    }
    bind_owner();
    let rows = shown();
    if rows.is_empty() {
        return;
    }
    let next = if last { rows.len() - 1 } else { 0 };
    ST.with(|s| s.borrow_mut().sel = Some(sel_from(&rows[next])));
    reveal(h, next);
    layout(h);
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn reveal(h: HWND, index: usize) {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let lay = lay_client(rc.right, rc.bottom, dpi);
    let view = (lay.body.bottom - lay.body.top).max(1);
    let rh = px(dpi, MGR_ROW);
    let y = index as i32 * rh;
    let mut scroll = ST.with(|s| s.borrow().scroll);
    let prev = scroll;
    if y < scroll {
        scroll = y;
    }
    if y + rh > scroll + view {
        scroll = y + rh - view;
    }
    scroll = scroll.max(0);
    ST.with(|s| s.borrow_mut().scroll = scroll);
    if scroll != prev {
        pop_tip_hold(h);
    }
}

unsafe fn fav_manager_activate() {
    let h = hwnd();
    if h.0.is_null() {
        return;
    }
    bind_owner();
    let rows = shown();
    let row = sel_index(&rows).and_then(|i| rows.get(i)).cloned().or_else(|| rows.first().cloned());
    let Some(row) = row else { return };
    activate_row(&row);
}

/// A group toggles. A leaf navigates the active pane, or launches a file, and the window closes.
unsafe fn activate_row(row: &FavRow) {
    if row.group {
        finish_drag();
        let _ = ReleaseCapture();
        if row.in_tree() {
            toggle_id(&row.id);
        } else {
            toggle_home(row.kind);
        }
        return;
    }
    let row = row.clone();
    let moved = finish_drag();
    let _ = ReleaseCapture();
    if moved {
        commit();
    }
    close_fav_manager();
    open_fav_row(&row);
}

unsafe fn fav_manager_delete_sel() {
    bind_owner();
    let rows = shown();
    let Some(row) = sel_index(&rows).and_then(|i| rows.get(i)).filter(|r| fav_row_editable(r)).cloned() else { return };
    delete_id(&row.id);
}

unsafe fn fav_manager_rename_sel() {
    let h = hwnd();
    bind_owner();
    let rows = shown();
    let Some(row) = sel_index(&rows).and_then(|i| rows.get(i)).filter(|r| fav_row_editable(r)).cloned() else { return };
    rename_id(h, &row);
}

fn group_parent(rows: &[FavRow]) -> Vec<usize> {
    let Some(i) = sel_index(rows) else { return Vec::new() };
    let row = &rows[i];
    if !row.in_tree() {
        return Vec::new();
    }
    if row.group {
        row.id.clone()
    } else if row.id.len() <= 1 {
        Vec::new()
    } else {
        row.id[..row.id.len() - 1].to_vec()
    }
}

unsafe fn toggle_id(id: &[usize]) {
    app(|a| {
        a.s.ensure_fav_tree();
        toggle_open(&mut a.s.tree, id);
        a.s.sync_favs();
    });
    commit();
}

/// Collapse state is the same flag the left panel uses, so both lists open and close together.
unsafe fn toggle_home(kind: RowKind) {
    let open = app(|a| match kind {
        RowKind::Pins => {
            a.s.pins_open = !a.s.pins_open;
            a.s.pins_open
        }
        RowKind::Files => {
            a.s.files_open = !a.s.files_open;
            a.s.files_open
        }
        RowKind::Tree => false,
    });
    if open {
        crate::request_pins(false);
    }
    commit();
}

unsafe fn delete_id(id: &[usize]) {
    let links = crate::servers_at(id);
    app(|a| {
        a.s.ensure_fav_tree();
        delete_at(&mut a.s.tree, id);
        a.s.sync_favs();
        a.fav_hot = None;
    });
    crate::release_servers(links);
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if st.sel.as_ref().is_some_and(|s| s.kind == RowKind::Tree && s.id.starts_with(id)) {
            st.sel = None;
        }
        if st.drag.as_ref().is_some_and(|d| d.starts_with(id)) {
            st.drag = None;
        }
    });
    commit();
}

unsafe fn rename_id(owner: HWND, row: &FavRow) {
    let Some(name) = settings::ask(owner, tr(crate::Key::Rename2), &row.name) else { return };
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let id = row.id.clone();
    app(|a| {
        a.s.ensure_fav_tree();
        rename_at(&mut a.s.tree, &id, &name);
        a.s.sync_favs();
    });
    commit();
}

unsafe fn on_add_group(owner: HWND) {
    bind_owner();
    let parent = group_parent(&shown());
    let name = tr(crate::Key::Group).to_string();
    let id = app(|a| {
        a.s.ensure_fav_tree();
        if !parent.is_empty() {
            set_open(&mut a.s.tree, &parent, true);
        }
        if !add_group(&mut a.s.tree, &parent, &name) {
            return None;
        }
        a.s.sync_favs();
        tree_rows(&a.s.tree).into_iter().rev().find(|r| r.group && r.name == name && r.id.len() == parent.len() + 1 && r.id.starts_with(&parent)).map(|r| r.id)
    });
    let Some(id) = id else {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.query.clear();
        st.sel = Some(Sel { kind: RowKind::Tree, id: id.clone() });
        st.scroll = 0;
    });
    let _ = SetWindowTextW(ctl(owner, ID_SEARCH), w!(""));
    if let Some(i) = shown().iter().position(|r| r.in_tree() && r.id == id) {
        reveal(owner, i);
    }
    layout(owner);
    let _ = InvalidateRect(Some(owner), None, false);
    if let Some(typed) = settings::ask(owner, tr(crate::Key::GroupName), &name) {
        let typed = typed.trim().to_string();
        if !typed.is_empty() && typed != name {
            app(|a| {
                a.s.ensure_fav_tree();
                rename_at(&mut a.s.tree, &id, &typed);
                a.s.sync_favs();
            });
        }
    }
    commit();
}

/// Toolbar up / down. Same sibling move as Alt+↑/↓ in the dropdown.
unsafe fn nudge_sel(down: bool) {
    bind_owner();
    let rows = shown();
    let Some(row) = sel_index(&rows).and_then(|i| rows.get(i)).filter(|r| fav_row_editable(r)).cloned() else { return };
    let moved = app(|a| {
        a.s.ensure_fav_tree();
        let next = nudge_node(&mut a.s.tree, &row.id, down)?;
        a.s.sync_favs();
        Some(next)
    });
    let Some(next) = moved else { return };
    ST.with(|s| s.borrow_mut().sel = Some(Sel { kind: RowKind::Tree, id: next.clone() }));
    let h = hwnd();
    if let Some(i) = shown().iter().position(|r| r.in_tree() && r.id == next) {
        reveal(h, i);
    }
    commit();
}

/// Open every group, and the pins and files sections the list shares with the side panel.
unsafe fn expand_everything() {
    app(|a| {
        a.s.ensure_fav_tree();
        expand_all(&mut a.s.tree);
        a.s.pins_open = true;
        a.s.files_open = true;
        a.s.sync_favs();
    });
    crate::request_pins(false);
    commit();
}

unsafe fn run_act(h: HWND, id: Act) {
    if !act_on(id) {
        return;
    }
    pop_tip_leave(h);
    button_tip(h, None, RECT::default());
    match id {
        Act::Add => add_folder(h),
        Act::Group => on_add_group(h),
        Act::Up => nudge_sel(false),
        Act::Down => nudge_sel(true),
        Act::Rename => fav_manager_rename_sel(),
        Act::Del => fav_manager_delete_sel(),
        Act::Expand => expand_everything(),
        Act::Close => close_fav_manager(),
    }
}

unsafe fn add_folder(owner: HWND) {
    bind_owner();
    let Some(path) = crate::pick_folder(owner) else { return };
    let id = app(|a| {
        a.s.ensure_fav_tree();
        if !add_item(&mut a.s.tree, &path, false) {
            return None;
        }
        a.s.sync_favs();
        tree_rows(&a.s.tree).into_iter().rev().find(|r| !r.group && r.path.eq_ignore_ascii_case(&path)).map(|r| r.id)
    });
    let Some(id) = id else {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.query.clear();
        st.sel = Some(Sel { kind: RowKind::Tree, id: id.clone() });
        st.scroll = 0;
    });
    let _ = SetWindowTextW(ctl(owner, ID_SEARCH), w!(""));
    if let Some(i) = shown().iter().position(|r| r.in_tree() && r.id == id) {
        reveal(owner, i);
    }
    layout(owner);
    let _ = InvalidateRect(Some(owner), None, false);
    commit();
}

unsafe fn commit() {
    crate::commit_favs();
}

fn row_h(dpi: i32) -> i32 {
    px(dpi, MGR_ROW)
}

fn row_rects(n: usize, body: RECT, scroll: i32, dpi: i32) -> Vec<RECT> {
    let rh = row_h(dpi);
    let mut y = body.top - scroll;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(RECT { left: body.left, top: y, right: body.right, bottom: y + rh });
        y += rh;
    }
    out
}

fn content_h(n: usize, hint: bool, dpi: i32) -> i32 {
    let rh = row_h(dpi);
    n as i32 * rh + if hint { rh } else { 0 }
}

unsafe fn layout(h: HWND) {
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    place(h, rc);
    let mut again = RECT::default();
    let _ = GetClientRect(h, &mut again);
    if again.right != rc.right || again.bottom != rc.bottom {
        place(h, again);
    }
}

unsafe fn place(h: HWND, rc: RECT) {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let font = ST.with(|s| s.borrow().font);
    let ew = measure_label(font, tr(crate::Key::FavExpandAll)) + px(dpi, 8);
    ST.with(|s| s.borrow_mut().expand_w = ew.max(px(dpi, 48)));
    let lay = lay_client(rc.right, rc.bottom, dpi);
    let q = ST.with(|s| s.borrow().query.clone());
    let rows = shown();
    let hint = show_fav_hint(&rows, &q);
    let content = content_h(rows.len(), hint, dpi);
    let view = (lay.body.bottom - lay.body.top).max(1);
    let max_scroll = (content - view).max(0);
    let prev_scroll = ST.with(|s| s.borrow().scroll);
    let scroll = ST.with(|s| {
        let mut st = s.borrow_mut();
        st.scroll = st.scroll.clamp(0, max_scroll);
        st.scroll
    });
    if scroll != prev_scroll {
        pop_tip_hold(h);
    }
    let info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (content - 1).max(0),
        nPage: view as u32,
        nPos: scroll,
        nTrackPos: 0,
    };
    let _ = windows::Win32::UI::Controls::SetScrollInfo(h, SB_VERT, &info, true);
    place_edit(ctl(h, ID_SEARCH), lay.edit, true);
    cue(ctl(h, ID_SEARCH), tr(crate::Key::SearchFavourites));
}

unsafe fn paint(h: HWND) {
    crate::note_pins_hwnd(h);
    crate::pins_on_paint();
    let (dpi, scroll, query, sel, font, semi, small, icon, track, hot_row) = ST.with(|s| {
        let st = s.borrow();
        (st.dpi.max(96), st.scroll, st.query.clone(), st.sel.clone(), st.font, st.semi, st.small, st.icon, st.track, st.hot_row)
    });
    bind_owner();
    let rows = shown();
    let mark = shown_mark();
    ST.with(|s| s.borrow_mut().rows_mark = Some(mark));
    buffered(h, |dc, rc| {
        let lay = lay_client(rc.right, rc.bottom, dpi);
        let t = ui_tokens();
        let ground = t.bg;
        paint_window(dc, rc);
        text(dc, lay.hint, tr(crate::Key::FavMgrHint), small, t.sub, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
        fill(dc, lay.sep, t.line);
        for (i, r) in lay.icons.iter().enumerate() {
            let id = TOOL[i];
            paint_icon(dc, *r, id, track.visual(id, !act_on(id)), icon, dpi, ground);
        }
        let focused = unsafe { search_focused(h) };
        paint_field(dc, lay.search, dpi, focused);
        paint_card(dc, lay.list, dpi);
        let head_bg = blend(t.card, t.bg, 0.70);
        fill(dc, lay.head, head_bg);
        paint_fav_head(dc, lay.head, dpi, small);
        let saved = SaveDC(dc);
        if lay.body.bottom > lay.body.top && lay.body.right > lay.body.left {
            let _ = IntersectClipRect(dc, lay.body.left, lay.body.top, lay.body.right, lay.body.bottom);
        }
        let rects = row_rects(rows.len(), lay.body, scroll, dpi);
        if rows.is_empty() {
            text(dc, lay.body, empty_msg(&query), small, t.sub, DT_CENTER);
        }
        for (i, rect) in rects.iter().enumerate() {
            let row = rows.get(i);
            let on = row.is_some_and(|r| sel.as_ref().is_some_and(|s| r.kind == s.kind && r.id == s.id));
            let group = row.is_some_and(|r| r.group);
            if on {
                fill(dc, *rect, t.sel_mist());
                let inset = px(dpi, 3).min((rect.bottom - rect.top) / 4);
                let bar = RECT {
                    left: rect.left,
                    top: rect.top + inset,
                    right: rect.left + px(dpi, 3).max(2),
                    bottom: rect.bottom - inset,
                };
                fill(dc, bar, ACCENT);
            } else if group {
                fill(dc, *rect, blend(t.card, t.bg, 0.55));
            } else if hot_row == Some(i) {
                fill(dc, *rect, blend(t.card, t.text, 0.04));
            }
            let face = if group { semi } else { font };
            paint_fav_row(dc, row, *rect, dpi, face, small, icon, false, true, false);
        }
        if show_fav_hint(&rows, &query) && !rows.is_empty() {
            let rh = row_h(dpi);
            let top = lay.body.top - scroll + rows.len() as i32 * rh;
            let r = RECT { left: lay.body.left, top, right: lay.body.right, bottom: top + rh };
            text(dc, r, empty_msg(""), small, t.sub, DT_CENTER);
        }
        if saved != 0 {
            let _ = RestoreDC(dc, saved);
        }
        paint_button(dc, lay.expand, ButtonKind::Link, track.visual(Act::Expand, false), tr(crate::Key::FavExpandAll), font, dpi, ground, FocusRing::Outside);
        paint_button(dc, lay.close, ButtonKind::Primary, track.visual(Act::Close, false), tr(crate::Key::Close), font, dpi, ground, FocusRing::Outside);
    });
}

struct HitRow {
    index: usize,
    row: FavRow,
    rect: RECT,
}

/// Hide the location tip and remember the pointer, so a scroll does not immediately show it again.
unsafe fn pop_tip_hold(h: HWND) {
    bind_owner();
    hide_pop_fav_tip(h);
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() || !ScreenToClient(h, &mut pt).as_bool() {
        ST.with(|s| s.borrow_mut().tip_hold = None);
        return;
    }
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    if pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom {
        ST.with(|s| s.borrow_mut().tip_hold = Some((pt.x, pt.y)));
    } else {
        ST.with(|s| s.borrow_mut().tip_hold = None);
    }
}

unsafe fn pop_tip_at(h: HWND, x: i32, y: i32) {
    // The search edit, or an IME composition on it, owns the screen. The tracking tip
    // re-enters this window and stalls the candidate the same way the side panel does.
    if search_focused(h) || ime_composing(h) {
        bind_owner();
        hide_pop_fav_tip(h);
        return;
    }
    if ST.with(|s| s.borrow().tip_hold) == Some((x, y)) {
        return;
    }
    ST.with(|s| s.borrow_mut().tip_hold = None);
    bind_owner();
    let hit = hit_row(h, x, y);
    let item = hit.as_ref().map(|hit| crate::ui::tip_screen_rect(h, hit.rect));
    show_pop_fav_tip(h, hit.as_ref().map(|hit| &hit.row), item);
}

fn shown_mark() -> u64 {
    let list = shown();
    let mut h = 0x9E37_79B9u64.wrapping_add(list.len() as u64);
    for r in &list {
        h = h.wrapping_mul(31).wrapping_add(r.path.chars().count() as u64);
        for c in r.path.chars() {
            h = h.wrapping_mul(31).wrapping_add(c as u64);
        }
        h = h.wrapping_mul(31).wrapping_add(r.name.chars().count() as u64);
        for c in r.name.chars() {
            h = h.wrapping_mul(31).wrapping_add(c as u64);
        }
        h = h.wrapping_mul(31).wrapping_add(r.id.len() as u64);
        h = h.wrapping_mul(31).wrapping_add(u64::from(r.group));
        h = h.wrapping_mul(31).wrapping_add(match r.kind {
            RowKind::Tree => 1,
            RowKind::Pins => 2,
            RowKind::Files => 3,
        });
    }
    h
}

unsafe fn pop_tip_leave(h: HWND) {
    ST.with(|s| s.borrow_mut().tip_hold = None);
    bind_owner();
    hide_pop_fav_tip(h);
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn hit_row(h: HWND, x: i32, y: i32) -> Option<HitRow> {
    let (dpi, scroll) = ST.with(|s| (s.borrow().dpi.max(96), s.borrow().scroll));
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    let lay = lay_client(rc.right, rc.bottom, dpi);
    if !inside(lay.body, x, y) {
        return None;
    }
    let rows = shown();
    let rects = row_rects(rows.len(), lay.body, scroll, dpi);
    let index = rects.iter().position(|r| inside(*r, x, y))?;
    let rect = rects[index];
    let row = rows.get(index).cloned()?;
    Some(HitRow { index, row, rect })
}

unsafe fn on_command(h: HWND, id: i32, code: u32) {
    if code != EN_CHANGE || id != ID_SEARCH {
        return;
    }
    let q = crate::text_of(ctl(h, ID_SEARCH)).trim().to_string();
    let same = ST.with(|s| s.borrow().query == q);
    if same {
        return;
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.query = q;
        st.scroll = 0;
    });
    pop_tip_hold(h);
    layout(h);
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn finish_drag() -> bool {
    ST.with(|s| {
        let mut st = s.borrow_mut();
        let had = st.drag.take().is_some();
        let moved = st.drag_moved;
        st.drag_moved = false;
        st.drag_ready = false;
        had && moved
    })
}

extern "system" fn fav_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| fav_proc_inner(h, msg, wp, lp)))
}
fn fav_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg != WM_CREATE {
            bind_owner();
        }
        match msg {
            WM_CREATE => {
                let owner = PENDING_OWNER.with(|c| c.get());
                let dpi = GetDpiForWindow(h).max(96) as i32;
                ST.with(|s| {
                    let mut st = s.borrow_mut();
                    *st = St::default();
                    st.hwnd = h;
                    st.owner = owner;
                    st.dpi = dpi;
                });
                if !owner.0.is_null() {
                    bind_hwnd(owner);
                }
                remember_on_desk(h);
                rebuild_fonts();
                let font = ST.with(|s| s.borrow().font);
                let edit = crate::child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32 | 0x00010000, WINDOW_EX_STYLE(0), ID_SEARCH, font);
                theme_ctl(edit);
                note_side(h);
                crate::note_pins_hwnd(h);
                retitle(h);
                layout(h);
                return LRESULT(0);
            }
            WM_SIZE => {
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as i32;
                ST.with(|s| s.borrow_mut().dpi = dpi.max(96));
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                refresh_fav_manager();
            }
            WM_GETMINMAXINFO => {
                let dpi = ST.with(|s| s.borrow().dpi.max(96));
                let m = &mut *(lp.0 as *mut MINMAXINFO);
                m.ptMinTrackSize.x = px(dpi, 640);
                m.ptMinTrackSize.y = px(dpi, 420);
                return LRESULT(0);
            }
            WM_PAINT => paint(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
                let dc = HDC(wp.0 as _);
                let t = ui_tokens();
                SetBkMode(dc, TRANSPARENT);
                SetBkColor(dc, crate::cref(t.field_bg));
                SetTextColor(dc, crate::cref(t.text));
                SetDCBrushColor(dc, crate::cref(t.field_bg));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => on_command(h, (wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32),
            WM_LBUTTONDOWN => {
                // Gone before the click is handled, so the tip cannot take the click or the double-click.
                pop_tip_leave(h);
                button_tip(h, None, RECT::default());
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                let _ = SetFocus(Some(h));
                let lay = client_lay(h);
                if let Some(id) = hit_act(&lay, x, y) {
                    // A disabled glyph still occupies the toolbar. Do not clear the row selection.
                    if act_on(id) {
                        ST.with(|s| s.borrow_mut().track.pointer_down(id));
                        let _ = SetCapture(h);
                        let _ = InvalidateRect(Some(h), None, false);
                    }
                    return LRESULT(0);
                }
                if let Some(hit) = hit_row(h, x, y) {
                    let dpi = ST.with(|s| s.borrow().dpi.max(96));
                    let chevron = hit.row.group && inside(fav_chevron_rect(hit.rect, hit.row.depth, dpi), x, y);
                    let editable = hit.row.in_tree();
                    ST.with(|s| {
                        let mut st = s.borrow_mut();
                        st.sel = Some(sel_from(&hit.row));
                        st.drag = None;
                        st.drag_ready = false;
                        st.drag_moved = false;
                        if !chevron && editable {
                            st.drag = Some(hit.row.id.clone());
                            st.drag_at = (x, y);
                        }
                    });
                    if chevron {
                        if editable {
                            toggle_id(&hit.row.id);
                        } else {
                            toggle_home(hit.row.kind);
                        }
                    } else if editable {
                        let _ = SetCapture(h);
                        let _ = InvalidateRect(Some(h), None, false);
                    } else {
                        let _ = InvalidateRect(Some(h), None, false);
                    }
                } else {
                    ST.with(|s| s.borrow_mut().sel = None);
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONDBLCLK => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                let Some(hit) = hit_row(h, x, y) else { return LRESULT(0) };
                activate_row(&hit.row);
            }
            WM_MOUSEMOVE => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                let (from, ready, at, pressed) = ST.with(|s| {
                    let st = s.borrow();
                    (st.drag.clone(), st.drag_ready, st.drag_at, st.track.down())
                });
                if let Some(from) = from {
                    // A press that can become a reorder must not keep a tip under the pointer.
                    pop_tip_leave(h);
                    button_tip(h, None, RECT::default());
                    if !ready {
                        if (x - at.0).abs() + (y - at.1).abs() <= DRAG_SLOP {
                            return LRESULT(0);
                        }
                        ST.with(|s| s.borrow_mut().drag_ready = true);
                    }
                    let (dpi, scroll) = ST.with(|s| (s.borrow().dpi.max(96), s.borrow().scroll));
                    let mut rc = RECT::default();
                    let _ = GetClientRect(h, &mut rc);
                    let lay = lay_client(rc.right, rc.bottom, dpi);
                    let rows = shown();
                    let rects = row_rects(rows.len(), lay.body, scroll, dpi);
                    let hit = inside(lay.body, x, y).then(|| {
                        rects.iter().enumerate().find(|(_, r)| y >= r.top && y < r.bottom).and_then(|(i, r)| rows.get(i).map(|row| (row.clone(), *r)))
                    }).flatten();
                    if let Some((target, rect)) = hit {
                        if fav_popup_can_drop(true, &from, &target) {
                            let zone = drop_zone(target.group, y - rect.top, (rect.bottom - rect.top).max(1));
                            let moved = app(|a| {
                                a.s.ensure_fav_tree();
                                let new_id = apply_drop(&mut a.s.tree, &from, &target.id, zone)?;
                                if new_id == from {
                                    return None;
                                }
                                reveal_drop(&mut a.s.tree, &from, &target.id, zone);
                                a.s.sync_favs();
                                Some(new_id)
                            });
                            if let Some(new_id) = moved {
                                ST.with(|s| {
                                    let mut st = s.borrow_mut();
                                    st.drag = Some(new_id.clone());
                                    st.sel = Some(Sel { kind: RowKind::Tree, id: new_id });
                                    st.drag_moved = true;
                                });
                                layout(h);
                                let _ = InvalidateRect(Some(h), None, false);
                            }
                        }
                    }
                } else {
                    let lay = client_lay(h);
                    let over = hit_act(&lay, x, y).filter(|id| act_on(*id) || pressed == Some(*id));
                    let row = if pressed.is_some() { None } else { hit_row(h, x, y).map(|hit| hit.index) };
                    let changed = ST.with(|s| {
                        let mut st = s.borrow_mut();
                        let row_changed = st.hot_row != row;
                        st.hot_row = row;
                        let effect = st.track.pointer_move(over);
                        row_changed || !matches!(effect, TrackEffect::None)
                    });
                    if changed {
                        let _ = InvalidateRect(Some(h), None, false);
                    }
                    if let Some(id) = over {
                        pop_tip_leave(h);
                        button_tip(h, Some(id), act_rect(&lay, id));
                    } else {
                        button_tip(h, None, RECT::default());
                        if pressed.is_none() {
                            pop_tip_at(h, x, y);
                        }
                    }
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut t);
                }
            }
            WM_MOUSELEAVE => {
                pop_tip_leave(h);
                button_tip(h, None, RECT::default());
                let changed = ST.with(|s| {
                    let mut st = s.borrow_mut();
                    let row = st.hot_row.take().is_some();
                    let effect = st.track.pointer_leave();
                    row || !matches!(effect, TrackEffect::None)
                });
                if changed {
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                let lay = client_lay(h);
                let over = hit_act(&lay, x, y).filter(|id| act_on(*id));
                let effect = ST.with(|s| s.borrow_mut().track.pointer_up(over));
                let moved = finish_drag();
                let _ = ReleaseCapture();
                if let TrackEffect::Click(id) = effect {
                    run_act(h, id);
                } else if moved {
                    commit();
                }
                let _ = InvalidateRect(Some(h), None, false);
            }
            WM_CAPTURECHANGED => {
                if finish_drag() {
                    commit();
                }
                let effect = ST.with(|s| s.borrow_mut().track.capture_lost());
                if !matches!(effect, TrackEffect::None) {
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_MOUSEWHEEL => {
                pop_tip_hold(h);
                let delta = ((wp.0 >> 16) as i16 as i32) / 120;
                if delta != 0 {
                    let dpi = ST.with(|s| s.borrow().dpi.max(96));
                    let step = row_h(dpi) * 3;
                    ST.with(|s| {
                        let mut st = s.borrow_mut();
                        st.scroll -= delta * step;
                    });
                    layout(h);
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_VSCROLL => {
                pop_tip_hold(h);
                let mut si = SCROLLINFO { cbSize: size_of::<SCROLLINFO>() as u32, fMask: SIF_ALL, ..Default::default() };
                let _ = GetScrollInfo(h, SB_VERT, &mut si);
                let dpi = ST.with(|s| s.borrow().dpi.max(96));
                let cur = ST.with(|s| s.borrow().scroll);
                let want = match SCROLLBAR_COMMAND((wp.0 & 0xFFFF) as i32) {
                    SB_LINEUP => cur - row_h(dpi),
                    SB_LINEDOWN => cur + row_h(dpi),
                    SB_PAGEUP => cur - si.nPage as i32,
                    SB_PAGEDOWN => cur + si.nPage as i32,
                    SB_THUMBTRACK | SB_THUMBPOSITION => si.nTrackPos,
                    SB_TOP => 0,
                    SB_BOTTOM => i32::MAX,
                    _ => cur,
                };
                ST.with(|s| s.borrow_mut().scroll = want);
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
            }
            crate::WM_FAV_ICON => {
                clear_icon_refresh();
                let _ = InvalidateRect(Some(h), None, false);
                // A row may have just been marked missing. Refresh the note if the pointer is still on it.
                let idle = ST.with(|s| {
                    let st = s.borrow();
                    st.tip_hold.is_none() && st.drag.is_none()
                });
                if idle {
                    let mut pt = POINT::default();
                    if GetCursorPos(&mut pt).is_ok() && ScreenToClient(h, &mut pt).as_bool() {
                        pop_tip_at(h, pt.x, pt.y);
                    }
                }
            }
            WM_TIMER if wp.0 == crate::TIMER_PINS => {
                let _ = KillTimer(Some(h), crate::TIMER_PINS);
                crate::request_pins(true);
            }
            crate::WM_PINS => {
                // Pin reads retry until Explorer answers. Only a real change of the rows hides the tip.
                let mark = shown_mark();
                let changed = ST.with(|s| {
                    let mut st = s.borrow_mut();
                    let changed = st.rows_mark.is_some_and(|prev| prev != mark);
                    st.rows_mark = Some(mark);
                    changed
                });
                if changed {
                    pop_tip_hold(h);
                }
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
                if !crate::pins_loaded() {
                    let _ = SetTimer(Some(h), crate::TIMER_PINS, 700, None);
                }
            }
            WM_CLOSE => {
                let _ = DestroyWindow(h);
            }
            WM_DESTROY => {
                bind_owner();
                ST.with(|s| s.borrow_mut().tip_hold = None);
                button_tip(h, None, RECT::default());
                tip_drop(h, MGR_TIP);
                drop_pop_fav_tip(h);
                drop_side(h);
                crate::drop_pins_hwnd(h);
                let _ = KillTimer(Some(h), crate::TIMER_PINS);
                app(|a| {
                    if a.fav_mgr == h {
                        a.fav_mgr = HWND::default();
                    }
                });
                let old = ST.with(|s| {
                    let mut st = s.borrow_mut();
                    let old = (st.font, st.semi, st.small, st.icon);
                    *st = St::default();
                    old
                });
                for f in [old.0, old.1, old.2, old.3] {
                    crate::delete_font(f);
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_has_rename_between_down_and_delete() {
        let i = TOOL.iter().position(|a| *a == Act::Rename).expect("rename button");
        assert_eq!(TOOL[i - 1], Act::Down);
        assert_eq!(TOOL[i + 1], Act::Del);
        assert_eq!(act_icon(Act::Rename), Some(LineIcon::Rename));
        assert_eq!(
            TOOL.map(act_icon),
            [
                Some(LineIcon::Add),
                Some(LineIcon::Group),
                Some(LineIcon::Up),
                Some(LineIcon::Down),
                Some(LineIcon::Rename),
                Some(LineIcon::Del),
            ]
        );
        let lay = pop_lay(900, 640, 96, px(96, 96));
        assert_eq!(act_rect(&lay, Act::Rename), lay.icons[i]);
        assert_eq!(act_rect(&lay, Act::Del), lay.icons[i + 1]);
    }

    #[test]
    fn toolbar_uses_token_metrics() {
        for dpi in [96, 120, 144, 192] {
            let lay = pop_lay(900, 640, dpi, px(dpi, 96));
            assert_eq!(lay.search.bottom - lay.search.top, px(dpi, FIELD_H));
            assert_eq!(lay.edit.top, lay.search.top + px(dpi, 2));
            assert_eq!(lay.edit.bottom, lay.search.bottom - px(dpi, 2));
            assert_eq!(lay.edit.left, lay.search.left + px(dpi, FIELD_PAD_X));
            assert_eq!(lay.edit.right, lay.search.right - px(dpi, FIELD_PAD_X));
            assert_eq!(lay.close.bottom - lay.close.top, px(dpi, BTN_H_FOOTER));
            assert_eq!(lay.icons[0].bottom - lay.icons[0].top, px(dpi, BTN_H_TOOL));
            assert!(lay.expand.right <= lay.close.left);
            assert!(lay.icons[0].right <= lay.icons[1].left);
            assert!(lay.icons[1].right <= lay.sep.left);
            assert!(lay.sep.right <= lay.icons[2].left);
            assert!(lay.icons[4].right <= lay.search.left);
            assert!(lay.list.top >= lay.search.bottom);
            assert_eq!(lay.head.bottom - lay.head.top, px(dpi, MGR_HEAD));
            assert!(lay.body.top >= lay.head.bottom);
            assert_eq!(row_h(dpi), px(dpi, MGR_ROW));
        }
    }
}
