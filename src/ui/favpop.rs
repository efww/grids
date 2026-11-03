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
    blend, buffered, clear_icon_refresh, drop_pop_fav_tip, drop_side, fav_chevron_rect, fav_editable_indices, fav_head_h, fav_popup_can_drop, fav_row_editable, fav_row_h, files_section_label, fill,
    hide_pop_fav_tip, hit_fav_board, layout_fav_board, note_side, paint_fav_head, paint_fav_row, pal, pins_section_label, place_edit, popup_rows, show_pop_fav_tip, text, FavBoard, FavHit,
};

const ID_SEARCH: i32 = 1;
const ID_GROUP: i32 = 2;
const ID_FOLDER: i32 = 3;
const ID_REN: i32 = 1000;
const ID_DEL: i32 = 2000;
const BTN_MAX: i32 = 512;
/// Movement before a press becomes a reorder. A click must not move the row.
const DRAG_SLOP: i32 = 4;

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
    small: HFONT,
    icon: HFONT,
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
            small: HFONT::default(),
            icon: HFONT::default(),
        }
    }
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
    static PENDING_OWNER: Cell<HWND> = Cell::new(HWND::default());
}

struct PopLay {
    search: RECT,
    edit: RECT,
    group: RECT,
    folder: RECT,
    list: RECT,
}

fn px(dpi: i32, v: i32) -> i32 {
    v * dpi.max(96) / 96
}

/// Search frame, toolbar and the list viewport inside the client.
/// The search frame is the settings search frame (`search_frame_h`).
fn pop_lay(w: i32, h: i32, dpi: i32) -> PopLay {
    let dpi = dpi.max(96);
    let s = |v: i32| px(dpi, v);
    let margin = s(16);
    let gap = s(8);
    let frame_h = settings::search_frame_h(dpi).max(1);
    let right = (w - margin).max(margin + 1);
    let search = RECT { left: margin, top: margin, right, bottom: margin + frame_h };
    let inset = s(settings::FIELD_INSET_DIP).min(frame_h / 2);
    let edit = RECT {
        left: search.left + inset,
        top: search.top + inset,
        right: (search.right - inset).max(search.left + inset),
        bottom: (search.bottom - inset).max(search.top + inset),
    };
    let btn_h = s(32);
    let by = search.bottom + gap;
    let avail = (right - margin).max(1);
    let group_w = s(132).min(avail / 2);
    let folder_w = s(168).min((avail - group_w - gap).max(s(40)));
    let group = RECT { left: margin, top: by, right: margin + group_w, bottom: by + btn_h };
    let folder = RECT { left: group.right + gap, top: by, right: group.right + gap + folder_w, bottom: by + btn_h };
    let list_top = folder.bottom + gap;
    let list = RECT { left: margin, top: list_top, right, bottom: (h - margin).max(list_top + s(40)) };
    PopLay { search, edit, group, folder, list }
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
    let new = (make_font(face, 14, 400, dpi), make_font(face, 12, 400, dpi), make_font("Segoe MDL2 Assets", 14, 400, dpi));
    let old = ST.with(|s| {
        let mut st = s.borrow_mut();
        let old = (st.font, st.small, st.icon);
        (st.font, st.small, st.icon) = new;
        old
    });
    unsafe {
        for f in [old.0, old.1, old.2] {
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

unsafe fn make_btn(parent: HWND, id: i32, label: &str, font: HFONT) -> HWND {
    let t = HSTRING::from(label);
    crate::child(parent, w!("BUTTON"), PCWSTR(t.as_ptr()), BS_OWNERDRAW as u32 | 0x00010000, WINDOW_EX_STYLE(0), id, font)
}

fn each_child(h: HWND, mut f: impl FnMut(HWND)) {
    for id in [ID_SEARCH, ID_GROUP, ID_FOLDER] {
        let c = ctl(h, id);
        if !c.0.is_null() {
            f(c);
        }
    }
    for i in 0..BTN_MAX {
        for base in [ID_REN, ID_DEL] {
            let c = ctl(h, base + i);
            if c.0.is_null() {
                break;
            }
            f(c);
        }
    }
}

unsafe fn retitle(h: HWND) {
    let _ = SetWindowTextW(h, &HSTRING::from(tr(crate::Key::Favourites)));
    let _ = SetWindowTextW(ctl(h, ID_GROUP), &HSTRING::from(tr(crate::Key::NewGroup)));
    let _ = SetWindowTextW(ctl(h, ID_FOLDER), &HSTRING::from(tr(crate::Key::AddFolder)));
    let rename = tr(crate::Key::Rename2);
    let delete = tr(crate::Key::Delete);
    for i in 0..BTN_MAX {
        let r = ctl(h, ID_REN + i);
        let d = ctl(h, ID_DEL + i);
        if r.0.is_null() && d.0.is_null() {
            break;
        }
        if !r.0.is_null() {
            let _ = SetWindowTextW(r, &HSTRING::from(rename));
        }
        if !d.0.is_null() {
            let _ = SetWindowTextW(d, &HSTRING::from(delete));
        }
    }
    cue(ctl(h, ID_SEARCH), tr(crate::Key::SearchFavourites));
}

unsafe fn cue(edit: HWND, text: &str) {
    if edit.0.is_null() {
        return;
    }
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let hs = HSTRING::from(text);
    SendMessageW(edit, 0x1501, Some(WPARAM(1)), Some(LPARAM(hs.as_ptr() as isize)));
    let m = px(dpi, 8) as isize;
    SendMessageW(edit, 0x00D3, Some(WPARAM(3)), Some(LPARAM(m | (m << 16))));
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
    let lay = pop_lay(rc.right, rc.bottom, dpi);
    let view = (lay.list.bottom - lay.list.top).max(1);
    (view / fav_row_h(dpi)).max(1) as usize
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
    let lay = pop_lay(rc.right, rc.bottom, dpi);
    let view = (lay.list.bottom - lay.list.top).max(1);
    let y = fav_head_h(dpi) + index as i32 * fav_row_h(dpi);
    let rh = fav_row_h(dpi);
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

fn intersects(a: RECT, b: RECT) -> bool {
    a.right > a.left && a.bottom > a.top && a.bottom > b.top && a.top < b.bottom && a.right > b.left && a.left < b.right
}

unsafe fn move_to(h: HWND, r: RECT, show: bool) {
    if h.0.is_null() {
        return;
    }
    if !show {
        let _ = ShowWindow(h, SW_HIDE);
        return;
    }
    let _ = SetWindowPos(h, None, r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1), SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

fn board_at(n: usize, lay: &PopLay, scroll: i32, dpi: i32) -> FavBoard {
    let keep: Vec<usize> = (0..n).collect();
    layout_fav_board(&keep, lay.list.left, lay.list.right, lay.list.top - scroll, dpi)
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
    let lay = pop_lay(rc.right, rc.bottom, dpi);
    let q = ST.with(|s| s.borrow().query.clone());
    let rows = shown();
    let n = rows.len();
    let hint = if show_fav_hint(&rows, &q) { fav_row_h(dpi) } else { 0 };
    let content = layout_fav_board(&(0..n).collect::<Vec<_>>(), lay.list.left, lay.list.right, 0, dpi).bottom + hint;
    let view = (lay.list.bottom - lay.list.top).max(1);
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
    let board = board_at(n, &lay, scroll, dpi);
    let font = ST.with(|s| s.borrow().font);
    place_edit(ctl(h, ID_SEARCH), lay.edit, true);
    cue(ctl(h, ID_SEARCH), tr(crate::Key::SearchFavourites));
    move_to(ctl(h, ID_GROUP), lay.group, true);
    move_to(ctl(h, ID_FOLDER), lay.folder, true);
    let rename = tr(crate::Key::Rename2);
    let delete = tr(crate::Key::Delete);
    let edit = fav_editable_indices(&rows);
    let mut placed = 0i32;
    for (n, &row_i) in edit.iter().enumerate() {
        if n as i32 >= BTN_MAX {
            break;
        }
        let Some(slot) = board.slots.get(row_i) else { continue };
        let show_r = intersects(slot.rename, lay.list);
        let show_d = intersects(slot.delete, lay.list);
        let mut rb = ctl(h, ID_REN + n as i32);
        if rb.0.is_null() && show_r {
            rb = make_btn(h, ID_REN + n as i32, rename, font);
            theme_ctl(rb);
        }
        if !rb.0.is_null() {
            let _ = SetWindowTextW(rb, &HSTRING::from(rename));
            move_to(rb, slot.rename, show_r);
        }
        let mut db = ctl(h, ID_DEL + n as i32);
        if db.0.is_null() && show_d {
            db = make_btn(h, ID_DEL + n as i32, delete, font);
            theme_ctl(db);
        }
        if !db.0.is_null() {
            let _ = SetWindowTextW(db, &HSTRING::from(delete));
            move_to(db, slot.delete, show_d);
        }
        placed = n as i32 + 1;
    }
    for i in placed..BTN_MAX {
        let r = ctl(h, ID_REN + i);
        let d = ctl(h, ID_DEL + i);
        if r.0.is_null() && d.0.is_null() {
            break;
        }
        if !r.0.is_null() {
            let _ = ShowWindow(r, SW_HIDE);
        }
        if !d.0.is_null() {
            let _ = ShowWindow(d, SW_HIDE);
        }
    }
}

unsafe fn paint(h: HWND) {
    crate::note_pins_hwnd(h);
    crate::pins_on_paint();
    let (dpi, scroll, query, sel, font, small, icon) = ST.with(|s| {
        let st = s.borrow();
        (st.dpi.max(96), st.scroll, st.query.clone(), st.sel.clone(), st.font, st.small, st.icon)
    });
    bind_owner();
    let rows = shown();
    let mark = shown_mark();
    ST.with(|s| s.borrow_mut().rows_mark = Some(mark));
    buffered(h, |dc, rc| {
        let lay = pop_lay(rc.right, rc.bottom, dpi);
        let accent = settings::chrome_color();
        fill(dc, rc, pal().bg);
        let focused = unsafe { search_focused(h) };
        crate::aa_rfill(dc, lay.search, px(dpi, settings::FIELD_RADIUS_DIP), pal().card, Some(if focused { accent } else { pal().line }), pal().bg);
        crate::aa_rfill(dc, lay.list, px(dpi, 6), pal().card, Some(pal().line), pal().bg);
        let board = board_at(rows.len(), &lay, scroll, dpi);
        let _ = SaveDC(dc);
        let _ = IntersectClipRect(dc, lay.list.left, lay.list.top, lay.list.right, lay.list.bottom);
        paint_fav_head(dc, board.head, dpi, small);
        if let Some(r) = board.empty {
            text(dc, r, empty_msg(&query), small, pal().sub, DT_CENTER);
        } else if show_fav_hint(&rows, &query) {
            let r = RECT { left: board.table.left, top: board.bottom, right: board.table.right, bottom: board.bottom + fav_row_h(dpi) };
            text(dc, r, empty_msg(""), small, pal().sub, DT_CENTER);
        }
        for slot in &board.slots {
            let row = rows.get(slot.index);
            let on = row.is_some_and(|r| sel.as_ref().is_some_and(|s| r.kind == s.kind && r.id == s.id));
            if on {
                fill(dc, slot.rect, blend(pal().card, accent, 0.16));
                let bar = RECT {
                    left: slot.rect.left,
                    top: slot.rect.top + px(dpi, 8),
                    right: slot.rect.left + px(dpi, 3),
                    bottom: slot.rect.bottom - px(dpi, 8),
                };
                fill(dc, bar, accent);
            }
            paint_fav_row(dc, row, slot.rect, dpi, font, small, icon, false, true);
        }
        let _ = RestoreDC(dc, -1);
    });
}

unsafe fn draw_btn(d: &DRAWITEMSTRUCT) {
    let dis = d.itemState.0 & 4 != 0;
    let pressed = d.itemState.0 & 1 != 0;
    let hot = app(|a| a.hover) == d.hwndItem;
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let font = ST.with(|s| s.borrow().font);
    let accent = settings::chrome_color();
    let ground = pal().bg;
    fill(d.hDC, d.rcItem, ground);
    let (fill_c, border, ink) = if dis {
        (blend(ground, pal().sub, 0.12), pal().line, pal().sub)
    } else if pressed {
        (blend(ground, pal().text, 0.08), pal().line, pal().text)
    } else if hot {
        (blend(ground, accent, 0.12), accent, pal().text)
    } else {
        (ground, pal().line, pal().text)
    };
    crate::aa_rfill(d.hDC, d.rcItem, px(dpi, 4), fill_c, Some(border), ground);
    text(d.hDC, d.rcItem, &crate::text_of(d.hwndItem), font, ink, DT_CENTER);
}

struct HitRow {
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
    let lay = pop_lay(rc.right, rc.bottom, dpi);
    if x < lay.list.left || x >= lay.list.right || y < lay.list.top || y >= lay.list.bottom {
        return None;
    }
    let rows = shown();
    let board = board_at(rows.len(), &lay, scroll, dpi);
    let index = match hit_fav_board(&board, x, y)? {
        FavHit::Row(i) | FavHit::Rename(i) | FavHit::Delete(i) => i,
    };
    let rect = board.slots.iter().find(|s| s.index == index)?.rect;
    let row = rows.get(index).cloned()?;
    Some(HitRow { row, rect })
}

unsafe fn on_command(h: HWND, id: i32, code: u32) {
    if code == EN_CHANGE && id == ID_SEARCH {
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
        return;
    }
    if code != 0 {
        return;
    }
    bind_owner();
    if id == ID_GROUP {
        on_add_group(h);
        return;
    }
    if id == ID_FOLDER {
        add_folder(h);
        return;
    }
    let rows = shown();
    let edit = fav_editable_indices(&rows);
    if (ID_REN..ID_REN + BTN_MAX).contains(&id) {
        if let Some(&row_i) = edit.get((id - ID_REN) as usize) {
            if let Some(row) = rows.get(row_i).filter(|r| fav_row_editable(r)).cloned() {
                rename_id(h, &row);
            }
        }
        return;
    }
    if (ID_DEL..ID_DEL + BTN_MAX).contains(&id) {
        if let Some(&row_i) = edit.get((id - ID_DEL) as usize) {
            if let Some(row) = rows.get(row_i).filter(|r| fav_row_editable(r)).cloned() {
                delete_id(&row.id);
            }
        }
    }
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
                let _ = make_btn(h, ID_GROUP, tr(crate::Key::NewGroup), font);
                let _ = make_btn(h, ID_FOLDER, tr(crate::Key::AddFolder), font);
                theme_ctl(ctl(h, ID_GROUP));
                theme_ctl(ctl(h, ID_FOLDER));
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
            WM_DRAWITEM => {
                draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
                let dc = HDC(wp.0 as _);
                SetBkMode(dc, TRANSPARENT);
                SetBkColor(dc, crate::cref(pal().card));
                SetTextColor(dc, crate::cref(pal().text));
                SetDCBrushColor(dc, crate::cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => on_command(h, (wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32),
            WM_LBUTTONDOWN => {
                // Gone before the click is handled, so the tip cannot take the click or the double-click.
                pop_tip_leave(h);
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                let _ = SetFocus(Some(h));
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
                let (from, ready, at) = ST.with(|s| {
                    let st = s.borrow();
                    (st.drag.clone(), st.drag_ready, st.drag_at)
                });
                let Some(from) = from else {
                    pop_tip_at(h, x, y);
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut t);
                    return LRESULT(0);
                };
                // A press that can become a reorder must not keep a tip under the pointer.
                pop_tip_leave(h);
                if !ready {
                    if (x - at.0).abs() + (y - at.1).abs() <= DRAG_SLOP {
                        return LRESULT(0);
                    }
                    ST.with(|s| s.borrow_mut().drag_ready = true);
                }
                let (dpi, scroll) = ST.with(|s| (s.borrow().dpi.max(96), s.borrow().scroll));
                let mut rc = RECT::default();
                let _ = GetClientRect(h, &mut rc);
                let lay = pop_lay(rc.right, rc.bottom, dpi);
                let rows = shown();
                let board = board_at(rows.len(), &lay, scroll, dpi);
                let in_list = x >= lay.list.left && x < lay.list.right && y >= lay.list.top && y < lay.list.bottom;
                let hit = in_list.then(|| {
                    board.slots.iter().find(|s| y >= s.rect.top && y < s.rect.bottom).and_then(|s| rows.get(s.index).map(|r| (r.clone(), s.rect)))
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
            }
            WM_MOUSELEAVE => pop_tip_leave(h),
            WM_LBUTTONUP => {
                let moved = finish_drag();
                let _ = ReleaseCapture();
                if moved {
                    commit();
                }
            }
            WM_CAPTURECHANGED => {
                if finish_drag() {
                    commit();
                }
            }
            WM_MOUSEWHEEL => {
                pop_tip_hold(h);
                let delta = ((wp.0 >> 16) as i16 as i32) / 120;
                if delta != 0 {
                    let dpi = ST.with(|s| s.borrow().dpi.max(96));
                    let step = fav_row_h(dpi) * 3;
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
                    SB_LINEUP => cur - fav_row_h(dpi),
                    SB_LINEDOWN => cur + fav_row_h(dpi),
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
                    let old = (st.font, st.small, st.icon);
                    *st = St::default();
                    old
                });
                for f in [old.0, old.1, old.2] {
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
    fn search_frame_matches_settings() {
        for dpi in [96, 120, 144, 192] {
            let lay = pop_lay(900, 640, dpi);
            let frame = lay.search.bottom - lay.search.top;
            assert_eq!(frame, settings::search_frame_h(dpi));
            assert_eq!(frame, settings::search_frame_h(dpi.max(96)));
            let inset = px(dpi, settings::FIELD_INSET_DIP);
            assert_eq!(lay.edit.top, lay.search.top + inset);
            assert_eq!(lay.edit.bottom, lay.search.bottom - inset);
            assert_eq!(lay.edit.left, lay.search.left + inset);
            assert_eq!(lay.edit.right, lay.search.right - inset);
            let line = (16 * dpi / 96).max(1);
            let text = crate::centered_edit_rect(lay.edit, line);
            assert_eq!(text.bottom - text.top, line.min(lay.edit.bottom - lay.edit.top));
            assert!(text.top >= lay.search.top && text.bottom <= lay.search.bottom);
            assert!(lay.list.top >= lay.folder.bottom);
            assert!(lay.group.right <= lay.folder.left);
            assert_eq!(lay.search.left, lay.list.left);
        }
        assert_eq!(settings::search_frame_h(96), 32);
    }
}
