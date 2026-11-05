// File basket: a small always-on-top window. Paths stay in maguro.json.
// Copy and move go through IFileOperation on a worker, the same path as a pane paste.
// Remove and Clear edit the list only.
// A drag out is an OLE drop: CF_HDROP plus a shell data object when every path parses.
// It copies unless Shift is held. Ctrl forces a copy. Alt and Ctrl+Shift are the
// Windows shortcut chord, and this window does not create links. A drop on a Maguro
// pane or the folder tree runs IFileOperation on a worker, with the Windows progress.

use std::cell::{Cell, RefCell};

use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES};
use windows::Win32::System::Com::{FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL};
use windows::Win32::UI::Controls::SetScrollInfo;

use crate::app::*;
use crate::i18n::*;
use crate::imports::*;
use crate::pane::*;
use crate::session::*;
use crate::shell::*;
use crate::ui::*;

/// Cerulean #0A9BD6. Primary buttons and the tab-list accent bar. The basket does not follow the retired mint brand.
pub(crate) const CERULEAN: u32 = ACCENT;
/// Pressed primary fill, #0786BC. Hover is a separate step in the shared button tokens.
pub(crate) const CERULEAN_DEEP: u32 = ACCENT_PRESSED;
/// Light cerulean the tab list still uses for search ink. Basket rows use the shared selection mist.
pub(crate) const CERULEAN_MIST: u32 = 0x9BE3FA;

const ACT_COPY: u8 = 1;
const ACT_MOVE: u8 = 2;
const ACT_REMOVE: u8 = 3;
const ACT_CLEAR: u8 = 4;
const ACT_ALL: u8 = 5;

/// Same row and header as the favourites manager list.
const ROW_DIP: i32 = 22;
const HEAD_DIP: i32 = 22;
/// Status line. One meta line plus a little air.
const BAR_DIP: i32 = 22;

/// Mouse-down on a row. `keep` means the row was already selected and Shift is up,
/// so the selection stays until mouse-up. A drag then carries every selected file.
struct Press {
    x: i32,
    y: i32,
    index: usize,
    keep: bool,
    ctrl: bool,
}

struct State {
    hwnd: HWND,
    scroll: i32,
    sel: Vec<usize>,
    anchor: usize,
    press: Option<Press>,
    track: Tracker<u8>,
    hot_row: Option<usize>,
    buttons: Vec<(u8, RECT)>,
    list: RECT,
    hint: RECT,
    frame: RECT,
    head: RECT,
    status: RECT,
    row_h: i32,
}

impl State {
    fn new() -> Self {
        State {
            hwnd: HWND::default(),
            scroll: 0,
            sel: Vec::new(),
            anchor: 0,
            press: None,
            track: Tracker::new(),
            hot_row: None,
            buttons: Vec::new(),
            list: RECT::default(),
            hint: RECT::default(),
            frame: RECT::default(),
            head: RECT::default(),
            status: RECT::default(),
            row_h: ROW_DIP,
        }
    }
}

thread_local! {
    static ST: RefCell<State> = RefCell::new(State::new());
}

fn with_st<R>(f: impl FnOnce(&mut State) -> R) -> R {
    ST.with(|s| f(&mut s.borrow_mut()))
}

pub(crate) fn alive() -> bool {
    let h = with_st(|s| s.hwnd);
    !h.0.is_null() && unsafe { IsWindow(Some(h)).as_bool() }
}

pub(crate) fn captures(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    let b = with_st(|s| s.hwnd);
    !b.0.is_null() && (h == b || unsafe { IsChild(b, h).as_bool() })
}

/// Drop a trailing slash, keep a drive root, and turn slashes into backslashes.
pub(crate) fn tidy_path(path: &str) -> Option<String> {
    let mut s = path.trim().replace('/', "\\");
    if s.is_empty() {
        return None;
    }
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    is_fs(&s).then_some(s)
}

pub(crate) fn same_path(a: &str, b: &str) -> bool {
    match (tidy_path(a), tidy_path(b)) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(&b),
        _ => false,
    }
}

/// Keep order. Skip blanks, non-files, and paths already in the list.
pub(crate) fn tidy_list(paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in paths {
        let Some(shown) = tidy_path(p) else { continue };
        if out.iter().any(|e| same_path(e, &shown)) {
            continue;
        }
        out.push(shown);
        if out.len() == 4000 {
            break;
        }
    }
    out
}

/// How many new paths were appended. Duplicates stay out.
pub(crate) fn push_paths(list: &mut Vec<String>, incoming: &[String]) -> usize {
    let mut n = 0;
    for p in incoming {
        let Some(shown) = tidy_path(p) else { continue };
        if list.iter().any(|e| same_path(e, &shown)) {
            continue;
        }
        list.push(shown);
        n += 1;
        if list.len() == 4000 {
            break;
        }
    }
    n
}

pub(crate) fn basket_name(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    t.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()).unwrap_or(path).to_string()
}

pub(crate) fn parent_path(path: &str) -> String {
    let t = path.trim_end_matches('\\');
    let Some(i) = t.rfind('\\') else { return t.to_string() };
    if i == 2 && t.as_bytes().get(1) == Some(&b':') {
        return format!("{}\\", &t[..2]);
    }
    if i == 0 {
        return t.to_string();
    }
    t[..i].to_string()
}

/// Selection after a click, and the anchor to keep.
/// Shift selects the inclusive range from `anchor` and leaves the anchor.
/// Ctrl toggles one row and moves the anchor there. Shift+Ctrl adds that range.
/// A plain click selects only `index` and moves the anchor there.
pub(crate) fn click_sel(sel: &[usize], anchor: usize, index: usize, ctrl: bool, shift: bool) -> (Vec<usize>, usize) {
    if shift {
        let anchor = if sel.is_empty() { index } else { anchor };
        let (lo, hi) = if anchor <= index { (anchor, index) } else { (index, anchor) };
        let mut s = if ctrl { sel.iter().copied().filter(|&i| i < lo || i > hi).collect::<Vec<_>>() } else { Vec::new() };
        s.extend(lo..=hi);
        s.sort_unstable();
        s.dedup();
        return (s, anchor);
    }
    if ctrl {
        let mut s = sel.to_vec();
        if let Some(p) = s.iter().position(|&i| i == index) {
            s.remove(p);
        } else {
            s.push(index);
            s.sort_unstable();
        }
        return (s, index);
    }
    (vec![index], index)
}

pub(crate) fn all_sel(n: usize) -> Vec<usize> {
    (0..n).collect()
}

pub(crate) fn send_pick(sel: &[usize], anchor: usize) -> Option<usize> {
    if sel.is_empty() {
        return None;
    }
    if sel.contains(&anchor) {
        Some(anchor)
    } else {
        sel.iter().copied().min()
    }
}

/// Copy by default. Ctrl copies. Shift moves.
/// Alt, or Ctrl with Shift, is the Windows shortcut chord. The basket does not
/// create links, so that chord is no effect (`0`).
/// `DROPEFFECT_COPY` is 1 and `DROPEFFECT_MOVE` is 2.
pub(crate) fn drag_effect(ctrl: bool, shift: bool, alt: bool) -> u32 {
    if alt || (ctrl && shift) {
        0
    } else if shift {
        DROPEFFECT_MOVE.0
    } else {
        DROPEFFECT_COPY.0
    }
}

pub(crate) fn drag_modifiers(key: MODIFIERKEYS_FLAGS) -> (bool, bool, bool) {
    (key.0 & MK_CONTROL.0 != 0, key.0 & MK_SHIFT.0 != 0, key.0 & MK_ALT != 0)
}

/// Start a copy or move of `srcs` into `dst` on a worker thread.
/// IFileOperation shows the standard progress window. The value returned to the
/// drag source is copy even for a move: MoveItem runs after Drop returns, and a
/// reported move would let the source delete the files first.
pub(crate) unsafe fn drop_transfer(srcs: Vec<String>, dst: String, ctrl: bool, shift: bool, alt: bool) -> u32 {
    let want = drag_effect(ctrl, shift, alt);
    if want == 0 || srcs.is_empty() || !is_fs(&dst) {
        return DROPEFFECT_NONE.0;
    }
    let mv = want == DROPEFFECT_MOVE.0;
    let Some(rename) = send_mode(&srcs, &dst, mv) else {
        return DROPEFFECT_NONE.0;
    };
    let started = if mv { start_drop_move(srcs, dst) } else { start_copy(srcs, dst, rename, false) };
    if started { DROPEFFECT_COPY.0 } else { DROPEFFECT_NONE.0 }
}

/// Name colour, path colour, row background. A missing source uses the danger token.
/// Selection is the shared mist, the same wash as the favourites manager.
pub(crate) fn row_colors(missing: bool, selected: bool, dark: bool) -> (u32, u32, u32) {
    let t = tokens_for(dark);
    let bg = if selected { t.sel_mist() } else { t.card };
    if missing {
        (t.danger, t.danger, bg)
    } else {
        (t.text, t.sub, bg)
    }
}

/// Text-button width: the label plus the shared horizontal padding, and at least [`BTN_MIN_W`].
pub(crate) fn button_width(text_w: i32, dpi: i32) -> i32 {
    (text_w + px(dpi, BTN_PAD_X) * 2).max(px(dpi, BTN_MIN_W))
}

pub(crate) fn place_buttons(width: i32, widths: &[(u8, i32)], btn_h: i32, gap: i32, pad: i32) -> (i32, Vec<(u8, RECT)>) {
    let mut rows: Vec<Vec<(u8, i32)>> = vec![Vec::new()];
    let inner = (width - pad * 2).max(btn_h);
    let mut used = 0i32;
    for &(id, w) in widths {
        let w = w.max(btn_h);
        let need = if used == 0 { w } else { gap + w };
        if used > 0 && used + need > inner {
            rows.push(Vec::new());
            used = 0;
        }
        let add = if used == 0 { w } else { gap + w };
        rows.last_mut().unwrap().push((id, w));
        used += add;
    }
    let mut y = pad;
    let mut out = Vec::new();
    for row in rows {
        let mut x = pad;
        for (id, w) in row {
            out.push((id, RECT { left: x, top: y, right: x + w, bottom: y + btn_h }));
            x += w + gap;
        }
        y += btn_h + gap;
    }
    let h = if out.is_empty() { pad * 2 } else { y - gap + pad };
    (h, out)
}

pub(crate) fn scroll_by(pos: i32, delta: i32, max: i32) -> i32 {
    (pos + delta).clamp(0, max.max(0))
}

fn paths() -> Vec<String> {
    app(|a| a.s.file_basket.clone())
}

fn commit(next: Vec<String>) {
    let next = tidy_list(&next);
    app(|a| a.s.file_basket = next);
    let n = app(|a| a.s.file_basket.len());
    with_st(|s| {
        s.sel.retain(|&i| i < n);
        if n == 0 {
            s.anchor = 0;
        } else if s.anchor >= n {
            s.anchor = n - 1;
        }
    });
    unsafe {
        save_session();
        refresh();
    }
}

const TIMER_POLL: usize = 1;

fn append(incoming: &[String]) -> usize {
    let mut list = paths();
    let n = push_paths(&mut list, incoming);
    if n > 0 {
        commit(list);
    }
    n
}

pub(crate) unsafe fn toggle() {
    if alive() {
        close();
    } else {
        open();
    }
}

pub(crate) unsafe fn close() {
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        _ = DestroyWindow(h);
    }
    with_st(|s| *s = State::new());
}

pub(crate) unsafe fn refresh() {
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let title = HSTRING::from(tr(Key::FileBasket));
        _ = SetWindowTextW(h, &title);
        style_dialog_caption(h);
        layout(h);
        _ = InvalidateRect(Some(h), None, false);
    }
}

pub(crate) unsafe fn add_active_selection() {
    let i = active_pane();
    let got = selection_paths(i);
    if got.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let n = append(&got);
    if n == 0 {
        _ = MessageBeep(MB_ICONWARNING);
    }
    if !alive() {
        open();
    } else {
        refresh();
    }
}

unsafe fn selection_paths(i: usize) -> Vec<String> {
    let Some(fv) = view_of::<IFolderView2>(i) else { return Vec::new() };
    let Ok(arr) = fv.GetSelection(false) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0).min(10_000);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { continue };
        let Ok(name) = it.GetDisplayName(SIGDN_FILESYSPATH) else { continue };
        let path = pwstr(name);
        if tidy_path(&path).is_some() {
            out.push(path);
        }
    }
    out
}

unsafe fn open() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        return;
    }
    let dpi = GetDpiForWindow(main).max(96) as i32;
    let w = 560 * dpi / 96;
    let h = 520 * dpi / 96;
    let mut frame = RECT::default();
    _ = GetWindowRect(main, &mut frame);
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let work = if GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
        mi.rcWork
    } else {
        RECT { left: 0, top: 0, right: 1920, bottom: 1080 }
    };
    let mut x = frame.right - w - 16 * dpi / 96;
    let mut y = frame.top + 40 * dpi / 96;
    x = x.clamp(work.left, (work.right - w).max(work.left));
    y = y.clamp(work.top, (work.bottom - h).max(work.top));
    let title = HSTRING::from(tr(Key::FileBasket));
    // Same caption as Folder Watch and Settings: app icon, standard min/max/close.
    // Still unowned and topmost, so a click does not activate one main window and switch desks.
    let ex = WS_EX_TOPMOST;
    let style = WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN | WS_VISIBLE | WS_CLIPSIBLINGS;
    let Ok(hwnd) = CreateWindowExW(ex, w!("MaguroBasket"), &title, style, x, y, w, h, None, None, None, None) else {
        return;
    };
    style_dialog_caption(hwnd);
    note_side(hwnd);
    with_st(|s| {
        *s = State::new();
        s.hwnd = hwnd;
    });
    let target: IDropTarget = BasketDrop.into();
    if RegisterDragDrop(hwnd, &target).is_err() {
        // The window still takes the hotkey and the buttons. Drag-in needs the target.
    }
    layout(hwnd);
    _ = SetTimer(Some(hwnd), TIMER_POLL, 1000, None);
    _ = ShowWindow(hwnd, SW_SHOW);
    _ = SetForegroundWindow(hwnd);
}

fn actions() -> [(u8, Key); 5] {
    [
        (ACT_COPY, Key::CopyToActivePane),
        (ACT_MOVE, Key::MoveToActivePane),
        (ACT_ALL, Key::BasketSelectAll),
        (ACT_REMOVE, Key::RemoveFromBasket),
        (ACT_CLEAR, Key::ClearBasket),
    ]
}

unsafe fn text_px(dc: HDC, font: HFONT, s: &str) -> i32 {
    let old = SelectObject(dc, font.into());
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    let n = if w.is_empty() || !GetTextExtentPoint32W(dc, &w, &mut sz).as_bool() { 0 } else { sz.cx };
    SelectObject(dc, old);
    n
}

unsafe fn layout(hwnd: HWND) {
    let dpi = GetDpiForWindow(hwnd).max(96) as i32;
    let mut rc = RECT::default();
    if GetClientRect(hwnd, &mut rc).is_err() {
        return;
    }
    let dc = GetDC(Some(hwnd));
    let font = body_font();
    let pad_x = px(dpi, PAD_BODY_X);
    let gap = px(dpi, BTN_GAP);
    let btn_h = px(dpi, BTN_H_FOOTER);
    let widths: Vec<(u8, i32)> = actions().iter().map(|(id, key)| (*id, button_width(text_px(dc, font, tr(*key)), dpi))).collect();
    let _ = ReleaseDC(Some(hwnd), dc);
    let hint = RECT { left: 0, top: px(dpi, PAD_BODY_Y), right: rc.right, bottom: px(dpi, PAD_BODY_Y) + px(dpi, FONT_PX + SPACE_8) };
    let (_, mut buttons) = place_buttons(rc.right - rc.left, &widths, btn_h, gap, pad_x);
    let space = px(dpi, SPACE_8);
    let want_top = hint.bottom + space;
    let cur_top = buttons.first().map(|(_, r)| r.top).unwrap_or(pad_x);
    let dy = want_top - cur_top;
    let mut buttons_bottom = want_top;
    for (_, r) in &mut buttons {
        r.top += dy;
        r.bottom += dy;
        buttons_bottom = buttons_bottom.max(r.bottom);
    }
    let bar_h = px(dpi, BAR_DIP);
    let list_top = buttons_bottom + space;
    let list_bottom = (rc.bottom - bar_h - space).max(list_top);
    let frame = RECT { left: pad_x, top: list_top, right: (rc.right - pad_x).max(pad_x + 1), bottom: list_bottom };
    let rad = px(dpi, RADIUS).max(1);
    let head = RECT {
        left: frame.left + 1,
        top: frame.top + rad,
        right: (frame.right - 1).max(frame.left + 2),
        bottom: (frame.top + rad + px(dpi, HEAD_DIP)).min(frame.bottom),
    };
    let list = RECT { left: head.left, top: head.bottom, right: head.right, bottom: (frame.bottom - 1).max(head.bottom) };
    let status = RECT { left: 0, top: rc.bottom - bar_h, right: rc.right, bottom: rc.bottom };
    let row_h = px(dpi, ROW_DIP).max(1);
    let n = paths().len() as i32;
    let view_h = (list.bottom - list.top).max(0);
    let max_scroll = (n * row_h - view_h).max(0);
    let scroll = with_st(|s| {
        s.scroll = s.scroll.clamp(0, max_scroll);
        s.buttons = buttons;
        s.list = list;
        s.hint = hint;
        s.frame = frame;
        s.head = head;
        s.status = status;
        s.row_h = row_h;
        s.scroll
    });
    let info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (n * row_h - 1).max(0),
        nPage: view_h.max(0) as u32,
        nPos: scroll,
        nTrackPos: 0,
    };
    let _ = SetScrollInfo(hwnd, SB_VERT, &info, true);
}

fn hit_button(x: i32, y: i32) -> Option<u8> {
    with_st(|s| s.buttons.iter().find(|(_, r)| inside(*r, x, y)).map(|(id, _)| *id))
}

fn hit_row(x: i32, y: i32) -> Option<usize> {
    let (list, scroll, row_h) = with_st(|s| (s.list, s.scroll, s.row_h));
    if row_h <= 0 || !inside(list, x, y) {
        return None;
    }
    let i = (y - list.top + scroll) / row_h;
    let n = paths().len();
    (i >= 0 && (i as usize) < n).then_some(i as usize)
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn enabled(id: u8) -> bool {
    let (n, sel) = (paths().len(), with_st(|s| !s.sel.is_empty()));
    match id {
        ACT_CLEAR | ACT_ALL => n > 0,
        ACT_COPY | ACT_MOVE | ACT_REMOVE => sel,
        _ => false,
    }
}

unsafe fn act(id: u8) {
    if !enabled(id) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    match id {
        ACT_COPY => transfer(false),
        ACT_MOVE => transfer(true),
        ACT_REMOVE => remove_sel(),
        ACT_CLEAR => commit(Vec::new()),
        ACT_ALL => {
            select_all_rows();
            refresh();
        }
        _ => {}
    }
}

fn select_all_rows() {
    let n = paths().len();
    with_st(|s| s.sel = all_sel(n));
}

fn chosen() -> Vec<String> {
    let sel = with_st(|s| s.sel.clone());
    let list = paths();
    sel.iter().filter_map(|&i| list.get(i).cloned()).collect()
}

unsafe fn transfer(mv: bool) {
    let srcs: Vec<String> = chosen().into_iter().filter(|p| !missing(p)).collect();
    if srcs.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let i = active_pane();
    let dst = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    let Some(rename) = send_mode(&srcs, &dst, mv) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if !start_copy(srcs, dst, rename, mv) {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

fn remove_sel() {
    let sel = with_st(|s| s.sel.clone());
    let mut list = paths();
    let mut drop_at = sel;
    drop_at.sort_unstable();
    drop_at.dedup();
    for i in drop_at.into_iter().rev() {
        if i < list.len() {
            list.remove(i);
        }
    }
    commit(list);
}

unsafe fn path_is_dir(path: &str) -> bool {
    if unc_path(path) {
        return false;
    }
    let attr = GetFileAttributesW(&HSTRING::from(path));
    attr != INVALID_FILE_ATTRIBUTES && attr & FILE_ATTRIBUTE_DIRECTORY.0 != 0
}

fn missing(path: &str) -> bool {
    unsafe { local_path_gone(path) }
}

unsafe fn open_original(path: &str) {
    if missing(path) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let file = HSTRING::from(path);
    let r = ShellExecuteW(Some(with_st(|s| s.hwnd)), w!("open"), &file, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    if (r.0 as isize) <= 32 {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

pub(crate) unsafe fn on_key(vk: u16, ctrl: bool, shift: bool) -> bool {
    let _ = shift;
    if !alive() {
        return false;
    }
    let n = paths().len();
    match VIRTUAL_KEY(vk) {
        VK_ESCAPE if !ctrl => close(),
        VK_A if ctrl => {
            select_all_rows();
            refresh();
        }
        VK_C if ctrl => copy_selected(),
        VK_RETURN if !ctrl => {
            let list = paths();
            let pick = with_st(|s| send_pick(&s.sel, s.anchor));
            if let Some(path) = pick.and_then(|i| list.get(i)) {
                open_original(path);
            }
        }
        VK_UP | VK_DOWN if !ctrl => {
            if n == 0 {
                return true;
            }
            let delta = if VIRTUAL_KEY(vk) == VK_UP { -1 } else { 1 };
            with_st(|s| {
                let cur = if s.sel.contains(&s.anchor) { s.anchor } else { s.sel.first().copied().unwrap_or(0) };
                let next = (cur as i32 + delta).clamp(0, n as i32 - 1) as usize;
                s.sel = vec![next];
                s.anchor = next;
                reveal(s, next);
            });
            refresh();
        }
        _ => return false,
    }
    true
}

fn reveal(s: &mut State, index: usize) {
    let view = (s.list.bottom - s.list.top).max(0);
    let y = index as i32 * s.row_h;
    if y < s.scroll {
        s.scroll = y;
    } else if y + s.row_h > s.scroll + view {
        s.scroll = y + s.row_h - view;
    }
}

unsafe fn copy_selected() {
    let srcs: Vec<String> = chosen().into_iter().filter(|p| !missing(p)).collect();
    if srcs.is_empty() || !copy_files(&srcs) {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

unsafe fn copy_files(paths: &[String]) -> bool {
    let Some(mem) = hdrop_mem(paths) else { return false };
    let Some(eff) = effect_mem(DROPEFFECT_COPY.0) else {
        let _ = GlobalFree(Some(mem));
        return false;
    };
    let owner = with_st(|s| s.hwnd);
    if OpenClipboard(Some(owner)).is_err() {
        let _ = GlobalFree(Some(mem));
        let _ = GlobalFree(Some(eff));
        return false;
    }
    // SetClipboardData takes ownership. Free only a handle the clipboard did not accept.
    let mut hdrop_set = false;
    let mut effect_set = false;
    if EmptyClipboard().is_ok() {
        hdrop_set = SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(mem.0))).is_ok();
        if hdrop_set {
            let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
            if fmt != 0 {
                effect_set = SetClipboardData(fmt, Some(HANDLE(eff.0))).is_ok();
            }
        }
    }
    let _ = CloseClipboard();
    if !hdrop_set {
        let _ = GlobalFree(Some(mem));
    }
    if !effect_set {
        let _ = GlobalFree(Some(eff));
    }
    hdrop_set
}

fn hdrop_bytes(paths: &[String]) -> Vec<u8> {
    let mut wide: Vec<u16> = Vec::new();
    for p in paths {
        wide.extend(p.encode_utf16());
        wide.push(0);
    }
    wide.push(0);
    let mut buf = vec![0u8; 20 + wide.len() * 2];
    buf[..4].copy_from_slice(&20u32.to_le_bytes());
    buf[16..20].copy_from_slice(&1u32.to_le_bytes());
    let bytes = unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
    buf[20..].copy_from_slice(bytes);
    buf
}

unsafe fn hdrop_mem(paths: &[String]) -> Option<HGLOBAL> {
    let buf = hdrop_bytes(paths);
    let mem = GlobalAlloc(GMEM_MOVEABLE, buf.len()).ok()?;
    let p = GlobalLock(mem);
    if p.is_null() {
        let _ = GlobalFree(Some(mem));
        return None;
    }
    std::ptr::copy_nonoverlapping(buf.as_ptr(), p as *mut u8, buf.len());
    let _ = GlobalUnlock(mem);
    Some(mem)
}

unsafe fn effect_mem(effect: u32) -> Option<HGLOBAL> {
    let mem = GlobalAlloc(GMEM_MOVEABLE, 4).ok()?;
    let p = GlobalLock(mem) as *mut u32;
    if p.is_null() {
        let _ = GlobalFree(Some(mem));
        return None;
    }
    *p = effect;
    let _ = GlobalUnlock(mem);
    Some(mem)
}

fn shift_down() -> bool {
    unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 }
}

fn ctrl_down() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

fn alt_down() -> bool {
    unsafe { GetKeyState(VK_MENU.0 as i32) < 0 }
}

fn current_effect() -> u32 {
    drag_effect(ctrl_down(), shift_down(), alt_down())
}

fn basket_cf() -> u16 {
    unsafe { RegisterClipboardFormatW(w!("MaguroBasket")) as u16 }
}

fn pref_cf() -> u16 {
    unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16 }
}

/// True when `obj` is a drag that started in the file basket.
pub(crate) unsafe fn is_basket_data(obj: &IDataObject) -> bool {
    let cf = basket_cf();
    if cf == 0 {
        return false;
    }
    let fmt = FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    obj.QueryGetData(&fmt) == S_OK
}

/// Shell data object for every path. None when a path does not parse, so the
/// drag still offers our own CF_HDROP for the full list.
unsafe fn shell_data(paths: &[String]) -> Option<IDataObject> {
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    for p in paths {
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(p), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            pidls.push(pidl);
        }
    }
    if pidls.len() != paths.len() {
        for p in pidls {
            CoTaskMemFree(Some(p as _));
        }
        return None;
    }
    let refs: Vec<*const ITEMIDLIST> = pidls.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let arr = SHCreateShellItemArrayFromIDLists(&refs).ok();
    for p in pidls {
        CoTaskMemFree(Some(p as _));
    }
    arr?.BindToHandler::<_, IDataObject>(None, &BHID_DataObject).ok()
}

unsafe fn begin_drag(paths: Vec<String>) {
    if paths.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let shell = shell_data(&paths);
    let data: IDataObject = BasketData { paths, shell }.into();
    let Ok(source) = data.cast::<IDropSource>() else { return };
    let mut effect = DROPEFFECT::default();
    let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE, &mut effect);
    refresh();
}

#[implement(IDataObject, IDropSource)]
struct BasketData {
    paths: Vec<String>,
    shell: Option<IDataObject>,
}

fn fmt_ok(fmt: &FORMATETC, cf: u16) -> bool {
    fmt.cfFormat == cf && fmt.tymed & TYMED_HGLOBAL.0 as u32 != 0
}

impl IDataObject_Impl for BasketData_Impl {
    fn GetData(&self, fmt: *const FORMATETC) -> Result<STGMEDIUM> {
        let asked = unsafe { &*fmt };
        let pref = pref_cf();
        let mark = basket_cf();
        // Preferred effect follows the keys now, not a value baked in at drag start.
        if pref != 0 && fmt_ok(asked, pref) {
            let mem = unsafe { effect_mem(current_effect()) }.ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
            return Ok(medium(mem));
        }
        if mark != 0 && fmt_ok(asked, mark) {
            let mem = unsafe { effect_mem(1) }.ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
            return Ok(medium(mem));
        }
        if let Some(shell) = &self.shell {
            if let Ok(got) = unsafe { shell.GetData(fmt) } {
                return Ok(got);
            }
        }
        if fmt_ok(asked, CF_HDROP.0) {
            let mem = unsafe { hdrop_mem(&self.paths) }.ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
            return Ok(medium(mem));
        }
        Err(Error::from(DV_E_FORMATETC))
    }

    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }

    fn QueryGetData(&self, fmt: *const FORMATETC) -> HRESULT {
        let asked = unsafe { &*fmt };
        let pref = pref_cf();
        let mark = basket_cf();
        if fmt_ok(asked, CF_HDROP.0) || (pref != 0 && fmt_ok(asked, pref)) || (mark != 0 && fmt_ok(asked, mark)) {
            return S_OK;
        }
        if let Some(shell) = &self.shell {
            return unsafe { shell.QueryGetData(fmt) };
        }
        DV_E_FORMATETC
    }

    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if !out.is_null() {
            unsafe { (*out).ptd = std::ptr::null_mut() };
        }
        E_NOTIMPL
    }

    fn SetData(&self, fmt: *const FORMATETC, medium: *const STGMEDIUM, release: BOOL) -> Result<()> {
        // The shell object accepts Performed DropEffect. A move we finish ourselves
        // still reports copy, so the source does not delete ahead of MoveItem.
        if let Some(shell) = &self.shell {
            return unsafe { shell.SetData(fmt, medium, release.as_bool()) };
        }
        Err(Error::from(E_NOTIMPL))
    }

    fn EnumFormatEtc(&self, dir: u32) -> Result<IEnumFORMATETC> {
        if dir != DATADIR_GET.0 as u32 {
            return Err(Error::from(E_NOTIMPL));
        }
        Ok(FmtEnum { items: listed_formats(&self.shell), at: Cell::new(0) }.into())
    }

    fn DAdvise(&self, _: *const FORMATETC, _: u32, _: Ref<IAdviseSink>) -> Result<u32> {
        Err(Error::from(E_NOTIMPL))
    }

    fn DUnadvise(&self, _: u32) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }

    fn EnumDAdvise(&self) -> Result<IEnumSTATDATA> {
        Err(Error::from(E_NOTIMPL))
    }
}

fn one_fmt(cf: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

fn drag_formats() -> Vec<FORMATETC> {
    let mut items = vec![one_fmt(CF_HDROP.0)];
    let pref = pref_cf();
    if pref != 0 {
        items.push(one_fmt(pref));
    }
    let mark = basket_cf();
    if mark != 0 {
        items.push(one_fmt(mark));
    }
    items
}

fn listed_formats(shell: &Option<IDataObject>) -> Vec<FORMATETC> {
    let mut items = drag_formats();
    let Some(shell) = shell else { return items };
    for fmt in shell_formats(shell) {
        if items.iter().any(|e| e.cfFormat == fmt.cfFormat) {
            continue;
        }
        items.push(fmt);
    }
    items
}

fn shell_formats(shell: &IDataObject) -> Vec<FORMATETC> {
    let Ok(en) = (unsafe { shell.EnumFormatEtc(DATADIR_GET.0 as u32) }) else { return Vec::new() };
    let mut out = Vec::new();
    loop {
        let mut one = [FORMATETC {
            cfFormat: 0,
            ptd: std::ptr::null_mut(),
            dwAspect: 0,
            lindex: -1,
            tymed: 0,
        }];
        let mut n = 0u32;
        let hr = unsafe { en.Next(&mut one, Some(&mut n)) };
        if n == 0 {
            break;
        }
        let mut fmt = one[0];
        if !fmt.ptd.is_null() {
            unsafe { CoTaskMemFree(Some(fmt.ptd as _)) };
            fmt.ptd = std::ptr::null_mut();
        }
        if hr == S_OK || hr == S_FALSE {
            out.push(fmt);
        }
        if hr != S_OK || out.len() == 64 {
            break;
        }
    }
    out
}

#[implement(IEnumFORMATETC)]
struct FmtEnum {
    items: Vec<FORMATETC>,
    at: Cell<usize>,
}

impl IEnumFORMATETC_Impl for FmtEnum_Impl {
    fn Next(&self, celt: u32, out: *mut FORMATETC, fetched: *mut u32) -> HRESULT {
        if out.is_null() || celt == 0 {
            return E_INVALIDARG;
        }
        let mut n = 0u32;
        let mut i = self.at.get();
        while n < celt && i < self.items.len() {
            let src = &self.items[i];
            unsafe {
                out.add(n as usize).write(FORMATETC {
                    cfFormat: src.cfFormat,
                    ptd: src.ptd,
                    dwAspect: src.dwAspect,
                    lindex: src.lindex,
                    tymed: src.tymed,
                });
            }
            i += 1;
            n += 1;
        }
        self.at.set(i);
        if !fetched.is_null() {
            unsafe { *fetched = n };
        }
        if n == celt { S_OK } else { S_FALSE }
    }

    fn Skip(&self, celt: u32) -> Result<()> {
        let next = self.at.get().saturating_add(celt as usize);
        if next > self.items.len() {
            self.at.set(self.items.len());
            return Err(Error::from(S_FALSE));
        }
        self.at.set(next);
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        self.at.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumFORMATETC> {
        Ok(FmtEnum { items: self.items.clone(), at: Cell::new(self.at.get()) }.into())
    }
}

fn medium(mem: HGLOBAL) -> STGMEDIUM {
    STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: mem },
        pUnkForRelease: std::mem::ManuallyDrop::new(None),
    }
}

impl IDropSource_Impl for BasketData_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, key: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            return DRAGDROP_S_CANCEL;
        }
        if key.0 & MK_LBUTTON.0 != 0 {
            S_OK
        } else {
            DRAGDROP_S_DROP
        }
    }

    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[implement(IDropTarget)]
struct BasketDrop;

thread_local! {
    static HOVER: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn set_offer(effect: *mut DROPEFFECT) {
    if effect.is_null() {
        return;
    }
    let empty = HOVER.with(|h| h.borrow().is_empty());
    unsafe { *effect = if empty { DROPEFFECT_NONE } else { DROPEFFECT_COPY } };
}

impl IDropTarget_Impl for BasketDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let paths = unsafe { drop_fs_paths(&d) };
        HOVER.with(|h| *h.borrow_mut() = paths);
        set_offer(effect);
        Ok(())
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        set_offer(effect);
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        HOVER.with(|h| h.borrow_mut().clear());
        Ok(())
    }

    fn Drop(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let mut paths = unsafe { drop_fs_paths(&d) };
        if paths.is_empty() {
            paths = HOVER.with(|h| h.borrow().clone());
        }
        HOVER.with(|h| h.borrow_mut().clear());
        if paths.is_empty() {
            if !effect.is_null() {
                unsafe { *effect = DROPEFFECT_NONE };
            }
            return Ok(());
        }
        append(&paths);
        if !effect.is_null() {
            unsafe { *effect = DROPEFFECT_COPY };
        }
        Ok(())
    }
}

pub(crate) extern "system" fn basket_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| basket_proc_inner(h, msg, wp, lp)))
}

fn basket_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => LRESULT(0),
            WM_SIZE => {
                layout(h);
                _ = InvalidateRect(Some(h), None, false);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                buffered(h, |dc, rc| paint(h, dc, rc));
                LRESULT(0)
            }
            WM_VSCROLL => {
                on_scroll(h, wp.0);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == TIMER_POLL => {
                _ = InvalidateRect(Some(h), None, false);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let notch = ((wp.0 >> 16) as i16 as i32) / 120;
                let row = with_st(|s| s.row_h);
                nudge(-notch * row.max(1) * 3);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                _ = SetFocus(Some(h));
                on_down(h, xy(lp), wp.0 & MK_CONTROL.0 as usize != 0, wp.0 & MK_SHIFT.0 as usize != 0);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                on_move(h, xy(lp));
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                let changed = with_st(|s| {
                    let row = s.hot_row.take().is_some();
                    let effect = s.track.pointer_leave();
                    row || !matches!(effect, TrackEffect::None)
                });
                if changed {
                    _ = InvalidateRect(Some(h), None, false);
                }
                LRESULT(0)
            }
            WM_FAV_ICON => {
                clear_icon_refresh();
                _ = InvalidateRect(Some(h), None, false);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                on_up(h, xy(lp));
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                if let Some(i) = hit_row(xy(lp).0, xy(lp).1) {
                    if let Some(path) = paths().get(i).cloned() {
                        open_original(&path);
                    }
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                let effect = with_st(|s| {
                    s.press = None;
                    s.track.capture_lost()
                });
                if !matches!(effect, TrackEffect::None) {
                    _ = InvalidateRect(Some(h), None, false);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                close();
                LRESULT(0)
            }
            WM_DESTROY => {
                drop_side(h);
                let _ = RevokeDragDrop(h);
                with_st(|s| {
                    if s.hwnd == h {
                        s.hwnd = HWND::default();
                    }
                });
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let m = &mut *(lp.0 as *mut MINMAXINFO);
                let dpi = GetDpiForWindow(h).max(96) as i32;
                m.ptMinTrackSize.x = 360 * dpi / 96;
                m.ptMinTrackSize.y = 280 * dpi / 96;
                LRESULT(0)
            }
            _ => DefWindowProcW(h, msg, wp, lp),
        }
    }
}

fn xy(lp: LPARAM) -> (i32, i32) {
    (lp.0 as i16 as i32, (lp.0 >> 16) as i16 as i32)
}

unsafe fn on_down(h: HWND, at: (i32, i32), ctrl: bool, shift: bool) {
    layout(h);
    if let Some(id) = hit_button(at.0, at.1) {
        with_st(|s| {
            s.track.pointer_down(id);
        });
        _ = SetCapture(h);
        _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if let Some(i) = hit_row(at.0, at.1) {
        with_st(|s| {
            let n = paths().len();
            let already = s.sel.contains(&i);
            // Shift builds a new range immediately. Ctrl or a plain click on a
            // selected row waits for mouse-up, so a drag keeps the whole selection.
            let keep = already && !shift;
            if !keep {
                let anchor = if n == 0 { i } else { s.anchor.min(n - 1) };
                let (sel, anchor) = click_sel(&s.sel, anchor, i, ctrl, shift);
                s.sel = sel;
                s.anchor = anchor;
            }
            s.press = Some(Press { x: at.0, y: at.1, index: i, keep, ctrl });
        });
        _ = SetCapture(h);
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn on_move(h: HWND, at: (i32, i32)) {
    let (press, down) = with_st(|s| (s.press.as_ref().map(|p| (p.x, p.y)), s.track.down()));
    if let Some((x, y)) = press {
        let dx = at.0 - x;
        let dy = at.1 - y;
        if dx * dx + dy * dy >= 16 {
            let paths: Vec<String> = chosen().into_iter().filter(|p| !missing(p)).collect();
            with_st(|s| s.press = None);
            _ = ReleaseCapture();
            begin_drag(paths);
            return;
        }
    }
    let over = hit_button(at.0, at.1);
    let row = if down.is_some() { None } else { hit_row(at.0, at.1) };
    let changed = with_st(|s| {
        let row_changed = s.hot_row != row;
        s.hot_row = row;
        let effect = s.track.pointer_move(over);
        row_changed || !matches!(effect, TrackEffect::None)
    });
    if changed {
        _ = InvalidateRect(Some(h), None, false);
    }
    let mut track = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: h,
        dwHoverTime: 0,
    };
    _ = TrackMouseEvent(&mut track);
}

unsafe fn on_up(h: HWND, at: (i32, i32)) {
    let over = hit_button(at.0, at.1);
    let (effect, press) = with_st(|s| {
        let p = s.press.take();
        (s.track.pointer_up(over), p)
    });
    _ = ReleaseCapture();
    if let TrackEffect::Click(id) = effect {
        act(id);
    } else if let Some(p) = press {
        if p.keep {
            with_st(|s| {
                if p.ctrl {
                    let (sel, anchor) = click_sel(&s.sel, s.anchor, p.index, true, false);
                    s.sel = sel;
                    s.anchor = anchor;
                } else {
                    s.sel = vec![p.index];
                    s.anchor = p.index;
                }
            });
        }
    }
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn nudge(delta: i32) {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    let row = with_st(|s| s.row_h);
    let n = paths().len() as i32;
    let view = with_st(|s| (s.list.bottom - s.list.top).max(0));
    let max = (n * row - view).max(0);
    with_st(|s| s.scroll = scroll_by(s.scroll, delta, max));
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn on_scroll(h: HWND, wp: usize) {
    let row = with_st(|s| s.row_h).max(1);
    let view = with_st(|s| (s.list.bottom - s.list.top).max(row));
    let n = paths().len() as i32;
    let max = (n * row - view).max(0);
    let mut info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        nMin: 0,
        nMax: 0,
        nPage: 0,
        nPos: 0,
        nTrackPos: 0,
    };
    let _ = GetScrollInfo(h, SB_VERT, &mut info);
    let cur = with_st(|s| s.scroll);
    let next = match SCROLLBAR_COMMAND((wp & 0xffff) as i32) {
        SB_LINEUP => cur - row,
        SB_LINEDOWN => cur + row,
        SB_PAGEUP => cur - view,
        SB_PAGEDOWN => cur + view,
        SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
        SB_TOP => 0,
        SB_BOTTOM => max,
        _ => cur,
    };
    with_st(|s| s.scroll = scroll_by(next, 0, max));
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

fn row_cols(row: RECT, dpi: i32) -> (RECT, RECT, RECT, RECT) {
    let gutter = px(dpi, SPACE_8);
    let icon_w = px(dpi, 22);
    let kind_w = px(dpi, 76);
    let gap = px(dpi, SPACE_8);
    let icon = RECT { left: row.left + gutter, top: row.top, right: row.left + gutter + icon_w, bottom: row.bottom };
    let kind_left = (row.right - kind_w).max(icon.right);
    let kind = RECT { left: kind_left, top: row.top, right: row.right - px(dpi, SPACE_4), bottom: row.bottom };
    let mid_r = (kind.left - gap).max(icon.right);
    let mid = (mid_r - icon.right).max(0);
    let name_w = (mid * 2 / 5).max(px(dpi, 48).min(mid));
    let name = RECT { left: icon.right, top: row.top, right: icon.right + name_w, bottom: row.bottom };
    let path = RECT { left: (name.right + gap).min(mid_r), top: row.top, right: mid_r, bottom: row.bottom };
    (icon, name, path, kind)
}

fn entry_is_file(path: &str, gone: bool) -> bool {
    if gone {
        return basket_name(path).contains('.');
    }
    !unsafe { path_is_dir(path) }
}

unsafe fn paint(h: HWND, dc: HDC, rc: RECT) {
    layout(h);
    let (list, scroll, row_h, buttons, track, hot_row, hint, frame_r, head, status) =
        with_st(|s| (s.list, s.scroll, s.row_h, s.buttons.clone(), s.track, s.hot_row, s.hint, s.frame, s.head, s.status));
    let list_paths = paths();
    let sel = with_st(|s| s.sel.clone());
    let font = body_font();
    let small = app(|a| a.small_font);
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let t = ui_tokens();
    let dark = t.dark;
    paint_window(dc, rc);
    let pad_x = px(dpi, PAD_BODY_X);
    let hint_r = RECT { left: hint.left + pad_x, right: (hint.right - pad_x).max(hint.left + pad_x), ..hint };
    paint_meta(dc, hint_r, tr(Key::BasketHint), small);
    for (id, r) in &buttons {
        let on = enabled(*id);
        let label = actions().iter().find(|(k, _)| k == id).map(|(_, key)| tr(*key)).unwrap_or("");
        let kind = if *id == ACT_COPY { ButtonKind::Primary } else { ButtonKind::Secondary };
        paint_button(dc, *r, kind, track.visual(*id, !on), label, font, dpi, t.bg, FocusRing::Outside);
    }
    if frame_r.bottom > frame_r.top {
        crate::ui::paint_card(dc, frame_r, dpi);
    }
    if head.bottom > head.top {
        fill(dc, head, blend(t.card, t.bg, 0.70));
        let (_, name, path, kind) = row_cols(head, dpi);
        paint_meta(dc, name, tr(Key::Name), small);
        paint_meta(dc, path, tr(Key::Path), small);
        paint_meta(dc, kind, tr(Key::BasketKind), small);
        fill(dc, RECT { top: head.bottom - 1, bottom: head.bottom, left: head.left, right: head.right }, t.line);
    }
    let saved = SaveDC(dc);
    if list.bottom > list.top && list.right > list.left {
        IntersectClipRect(dc, list.left, list.top, list.right, list.bottom);
    }
    if list_paths.is_empty() {
        text(dc, list, tr(Key::BasketEmpty), font, t.sub, DT_CENTER);
    }
    let mut y = list.top - scroll;
    for (i, path) in list_paths.iter().enumerate() {
        let row = RECT { left: list.left, top: y, right: list.right, bottom: y + row_h };
        y += row_h;
        if row.bottom <= list.top || row.top >= list.bottom {
            continue;
        }
        let on = sel.contains(&i);
        let gone = missing(path);
        let (name_c, path_c, mut bg) = row_colors(gone, on, dark);
        if !on && hot_row == Some(i) {
            bg = blend(t.card, t.text, 0.04);
        }
        fill(dc, row, bg);
        if on {
            let inset = px(dpi, 3).min((row.bottom - row.top) / 4);
            let bar = RECT {
                left: row.left,
                top: row.top + inset,
                right: row.left + px(dpi, 3).max(2),
                bottom: row.bottom - inset,
            };
            fill(dc, bar, ACCENT);
        }
        let (icon, name_r, path_r, kind_r) = row_cols(row, dpi);
        let file = entry_is_file(path, gone);
        if gone {
            mark_missing(path);
        } else {
            mark_present(path);
        }
        let slot = px(dpi, 16);
        if let Some((ilist, idx, dim)) = shell_icon(path, file) {
            let ix = icon.left + ((icon.right - icon.left) - slot).max(0) / 2;
            let iy = row.top + (row_h - slot).max(0) / 2;
            draw_fav_icon(dc, ilist, idx, ix, iy, slot, dim || gone);
        }
        let shown = basket_name(path);
        if gone {
            text(dc, name_r, &shown, font, name_c, DT_LEFT);
        } else {
            paint_label(dc, name_r, &shown, font);
        }
        let mut detail = parent_path(path);
        if gone {
            detail.push_str("  ·  ");
            detail.push_str(tr(Key::BasketMissing));
        }
        if gone {
            text(dc, path_r, &detail, small, path_c, DT_LEFT);
        } else {
            paint_meta(dc, path_r, &detail, small);
        }
        let kind = if file { tr(Key::File) } else { tr(Key::Folder) };
        if gone {
            text(dc, kind_r, kind, small, path_c, DT_LEFT);
        } else {
            paint_meta(dc, kind_r, kind, small);
        }
        fill(dc, RECT { top: row.bottom - 1, bottom: row.bottom, left: row.left, right: row.right }, t.line);
    }
    if saved != 0 {
        let _ = RestoreDC(dc, saved);
    }
    fill(dc, RECT { bottom: status.top + 1, ..status }, t.line);
    let n = list_paths.len();
    let sel_n = sel.len();
    let miss_n = list_paths.iter().filter(|p| missing(p)).count();
    let count = tf(Key::BasketCount, &[("n", &n.to_string()), ("s", &sel_n.to_string())]);
    let status_l = RECT { left: status.left + pad_x, right: (status.right - pad_x).max(status.left + pad_x), ..status };
    paint_meta(dc, status_l, &count, small);
    if miss_n > 0 {
        let label = tf(Key::BasketMissingCount, &[("n", &miss_n.to_string())]);
        text(dc, status_l, &label, small, t.danger, DT_RIGHT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_and_slashes_collapse() {
        let mut list = vec![r"C:\Work\A.txt".into()];
        assert_eq!(push_paths(&mut list, &[r"c:\work\a.txt".into(), r"C:\Work\A.txt\".into()]), 0);
        assert_eq!(push_paths(&mut list, &[r"D:\new".into(), r"D:/new/".into(), "not-a-path".into()]), 1);
        assert_eq!(list, vec![r"C:\Work\A.txt".to_string(), r"D:\new".to_string()]);
        assert_eq!(tidy_list(&[r"C:\a".into(), r"c:\a\".into(), r"\\srv\share\b".into(), "".into()]).len(), 2);
        assert!(same_path(r"C:\A", r"c:/a"));
        assert_eq!(basket_name(r"D:\Photos\cat.jpg"), "cat.jpg");
        assert_eq!(basket_name(r"\\srv\share"), "share");
    }

    #[test]
    fn ctrl_click_toggles_shift_click_ranges_and_ctrl_a_selects_every_row() {
        assert_eq!(click_sel(&[], 0, 1, false, false), (vec![1], 1));
        assert_eq!(click_sel(&[1], 1, 2, true, false), (vec![1, 2], 2));
        assert_eq!(click_sel(&[1, 2], 2, 1, true, false), (vec![2], 1));
        assert_eq!(click_sel(&[0, 2], 0, 2, false, false), (vec![2], 2));
        assert_eq!(click_sel(&[0], 0, 3, false, true), (vec![0, 1, 2, 3], 0));
        assert_eq!(click_sel(&[0, 1, 2, 3], 0, 1, false, true), (vec![0, 1], 0));
        assert_eq!(click_sel(&[5], 5, 2, false, true), (vec![2, 3, 4, 5], 5));
        assert_eq!(click_sel(&[0, 4], 0, 2, true, true), (vec![0, 1, 2, 4], 0));
        assert_eq!(all_sel(3), vec![0, 1, 2]);
        assert_eq!(send_pick(&[2, 0], 2), Some(2));
        assert_eq!(send_pick(&[2, 0], 5), Some(0));
        assert_eq!(send_pick(&[], 0), None);
    }

    #[test]
    fn drag_copies_by_default_and_shift_moves() {
        assert_eq!(drag_effect(false, false, false), DROPEFFECT_COPY.0);
        assert_eq!(drag_effect(true, false, false), DROPEFFECT_COPY.0);
        assert_eq!(drag_effect(false, true, false), DROPEFFECT_MOVE.0);
        assert_eq!(drag_effect(true, true, false), 0);
        assert_eq!(drag_effect(false, false, true), 0);
        assert_ne!(drag_effect(false, false, false), drag_effect(false, true, false));
        assert_eq!(parent_path(r"C:\a.txt"), r"C:\");
        assert_eq!(parent_path(r"\\srv\share\dir\f.txt"), r"\\srv\share\dir");
        let buf = hdrop_bytes(&[r"C:\a.txt".into(), r"D:\b".into()]);
        assert_eq!(u32::from_le_bytes(buf[0..4].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(buf[16..20].try_into().unwrap()), 1);
        let wide: Vec<u16> = buf[20..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let text = String::from_utf16_lossy(&wide);
        assert_eq!(text, "C:\\a.txt\0D:\\b\0\0");
    }

    #[test]
    fn cerulean_palette_never_uses_mint() {
        let hover = button_colors(ButtonKind::Primary, ButtonState::Hover, false, BG_LIGHT).fill;
        let pressed = button_colors(ButtonKind::Primary, ButtonState::Pressed, false, BG_LIGHT).fill;
        for c in [CERULEAN, CERULEAN_DEEP, CERULEAN_MIST, hover.unwrap_or(0), pressed.unwrap_or(0)] {
            assert_ne!(c, 0x2EC4A6, "{c:#08X}");
        }
        assert_eq!(CERULEAN, 0x0A9BD6);
        assert_eq!(CERULEAN_DEEP, 0x0786BC);
        assert_eq!(CERULEAN_MIST, 0x9BE3FA);
        assert_eq!(hover, Some(0x0890C7));
        assert_eq!(pressed, Some(CERULEAN_DEEP));
        assert_ne!(hover, pressed);
        assert_eq!(button_colors(ButtonKind::Primary, ButtonState::Normal, false, BG_LIGHT).ink, 0xFFFFFF);
        let (name, path, bg) = row_colors(true, true, false);
        assert_eq!(bg, LIGHT_TOKENS.sel_mist());
        assert_eq!(name, DANGER_LIGHT);
        assert_eq!(path, DANGER_LIGHT);
        assert_ne!(name, INK);
        assert_ne!(path, INK);
        assert_ne!(bg, CERULEAN_MIST);
        assert_eq!(row_colors(false, false, false), (TEXT_LIGHT, SUB_LIGHT, CARD_LIGHT));
        assert_eq!(row_colors(false, true, true).2, DARK_TOKENS.sel_mist());
        assert_eq!(row_colors(true, false, true).0, DANGER_DARK);
        assert_eq!(row_colors(false, true, false).0, TEXT_LIGHT);
        assert_eq!(button_width(10, 96), BTN_MIN_W);
        assert_eq!(button_width(80, 96), 80 + BTN_PAD_X * 2);
        assert_eq!(button_width(10, 192), BTN_MIN_W * 2);
        let widths = [(ACT_COPY, 80), (ACT_MOVE, 80), (ACT_ALL, 80), (ACT_REMOVE, 80), (ACT_CLEAR, 80)];
        let (h, rects) = place_buttons(200, &widths, 30, 6, 8);
        assert!(rects.len() == 5);
        assert!(h > 30);
        assert!(rects.iter().any(|(_, r)| r.top > 8));
        assert_eq!(scroll_by(10, -40, 100), 0);
        assert_eq!(scroll_by(90, 40, 100), 100);
        assert_eq!(actions().map(|(id, _)| id), [ACT_COPY, ACT_MOVE, ACT_ALL, ACT_REMOVE, ACT_CLEAR]);
    }

    #[test]
    fn remove_and_clear_only_edit_the_list() {
        let mut list = vec![r"C:\a".to_string(), r"C:\b".to_string(), r"C:\c".to_string()];
        let sel = [0usize, 2];
        let mut drop_at = sel.to_vec();
        drop_at.sort_unstable();
        for i in drop_at.into_iter().rev() {
            list.remove(i);
        }
        assert_eq!(list, vec![r"C:\b".to_string()]);
        list.clear();
        assert!(list.is_empty());
    }
}
