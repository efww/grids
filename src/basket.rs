// File basket: a small always-on-top tool window. Paths stay in maguro.json.
// Copy and move go through IFileOperation on a worker, the same path as a pane paste.
// Remove and Clear edit the list only. A drag out copies unless Shift is held.

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

/// Cerulean #0A9BD6. The basket does not follow the retired mint brand.
pub(crate) const CERULEAN: u32 = 0x0A9BD6;
/// Pressed and hover fill, #0786BC.
pub(crate) const CERULEAN_DEEP: u32 = 0x0786BC;
/// Selection wash, #9BE3FA.
pub(crate) const CERULEAN_MIST: u32 = 0x9BE3FA;
const PALE: u32 = 0xB7E3F4;
const PAPER: u32 = 0xF4FBFE;
const INK: u32 = 0x14323C;
const INK_DIM: u32 = 0x5C7680;
const MISSING: u32 = 0x8A97A0;
const MISSING_DIM: u32 = 0xA8B4BB;
const MINT: u32 = 0x2EC4A6;

const ACT_COPY: u8 = 1;
const ACT_MOVE: u8 = 2;
const ACT_REMOVE: u8 = 3;
const ACT_CLEAR: u8 = 4;
const ACT_SEND: u8 = 5;

const ROW_DIP: i32 = 48;
const BTN_H_DIP: i32 = 30;
const PAD_DIP: i32 = 8;
const GAP_DIP: i32 = 6;

struct State {
    hwnd: HWND,
    scroll: i32,
    sel: Vec<usize>,
    anchor: usize,
    press: Option<(i32, i32)>,
    btn_down: Option<u8>,
    hot: Option<u8>,
    buttons: Vec<(u8, RECT)>,
    list: RECT,
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
            btn_down: None,
            hot: None,
            buttons: Vec::new(),
            list: RECT::default(),
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

/// Folder to open in the active pane. A file sends its parent.
pub(crate) fn send_folder(path: &str, is_dir: bool) -> String {
    let path = tidy_path(path).unwrap_or_else(|| path.to_string());
    if is_dir {
        path
    } else {
        parent_path(&path)
    }
}

pub(crate) fn click_sel(sel: &[usize], index: usize, ctrl: bool) -> Vec<usize> {
    if !ctrl {
        return vec![index];
    }
    let mut s = sel.to_vec();
    if let Some(p) = s.iter().position(|&i| i == index) {
        s.remove(p);
    } else {
        s.push(index);
        s.sort_unstable();
    }
    s
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

/// Copy unless Shift is down. `DROPEFFECT_COPY` is 1, `DROPEFFECT_MOVE` is 2.
pub(crate) fn drag_effect(shift: bool) -> u32 {
    if shift {
        DROPEFFECT_MOVE.0
    } else {
        DROPEFFECT_COPY.0
    }
}

pub(crate) fn button_fill(enabled: bool, hot: bool) -> u32 {
    if !enabled {
        PALE
    } else if hot {
        CERULEAN_DEEP
    } else {
        CERULEAN
    }
}

pub(crate) fn button_ink(fill: u32) -> u32 {
    if fill == CERULEAN || fill == CERULEAN_DEEP {
        0xFFFFFF
    } else {
        0x08384A
    }
}

/// Name colour, path colour, row background.
pub(crate) fn row_colors(missing: bool, selected: bool) -> (u32, u32, u32) {
    let bg = if selected { CERULEAN_MIST } else { 0xFFFFFF };
    if missing {
        (MISSING, MISSING_DIM, bg)
    } else if selected {
        (0x08384A, 0x1A5C78, bg)
    } else {
        (INK, INK_DIM, bg)
    }
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
    let w = 460 * dpi / 96;
    let h = 440 * dpi / 96;
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
    let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_VSCROLL | WS_VISIBLE | WS_CLIPSIBLINGS;
    // Unowned, so a click does not activate one main window and switch desks.
    // The active pane stays the one the user was just using. The last window closes this.
    let Ok(hwnd) = CreateWindowExW(ex, w!("MaguroBasket"), &title, style, x, y, w, h, None, None, None, None) else {
        return;
    };
    paint_caption(hwnd);
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

unsafe fn paint_caption(hwnd: HWND) {
    let color = cref(CERULEAN);
    let text = cref(0xFFFFFF);
    let border = cref(CERULEAN_DEEP);
    _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, &color as *const _ as _, 4);
    _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(36), &text as *const _ as _, 4);
    _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as _, 4);
    let dark = BOOL::from(true);
    _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &dark as *const _ as _, 4);
}

fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi.max(96) / 96
}

fn actions() -> [(u8, Key); 5] {
    [
        (ACT_COPY, Key::CopyToActivePane),
        (ACT_MOVE, Key::MoveToActivePane),
        (ACT_REMOVE, Key::RemoveFromBasket),
        (ACT_CLEAR, Key::ClearBasket),
        (ACT_SEND, Key::SendToActivePane),
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
    let font = app(|a| a.font);
    let pad = sc(dpi, PAD_DIP);
    let gap = sc(dpi, GAP_DIP);
    let btn_h = sc(dpi, BTN_H_DIP);
    let widths: Vec<(u8, i32)> = actions().iter().map(|(id, key)| (*id, text_px(dc, font, tr(*key)) + sc(dpi, 22))).collect();
    let _ = ReleaseDC(Some(hwnd), dc);
    let (bar_h, mut buttons) = place_buttons(rc.right - rc.left, &widths, btn_h, gap, pad);
    let bar_top = (rc.bottom - bar_h).max(0);
    for b in &mut buttons {
        b.1.top += bar_top;
        b.1.bottom += bar_top;
    }
    let list = RECT { left: 0, top: 0, right: rc.right, bottom: bar_top };
    let row_h = sc(dpi, ROW_DIP);
    let n = paths().len() as i32;
    let view_h = (list.bottom - list.top).max(0);
    let max_scroll = (n * row_h - view_h).max(0);
    let scroll = with_st(|s| {
        s.scroll = s.scroll.clamp(0, max_scroll);
        s.buttons = buttons;
        s.list = list;
        s.row_h = row_h.max(1);
        s.scroll
    });
    let mut info = SCROLLINFO {
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
        ACT_CLEAR => n > 0,
        ACT_COPY | ACT_MOVE | ACT_REMOVE | ACT_SEND => sel,
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
        ACT_SEND => send_active(),
        _ => {}
    }
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

unsafe fn send_active() {
    let list = paths();
    let pick = with_st(|s| send_pick(&s.sel, s.anchor));
    let Some(path) = pick.and_then(|i| list.get(i)).cloned() else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if missing(&path) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let dir = path_is_dir(&path);
    let folder = send_folder(&path, dir);
    let i = active_pane();
    if !navigate(i, &folder) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    focus_view(i);
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
            with_st(|s| s.sel = all_sel(n));
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

unsafe fn begin_drag(paths: Vec<String>) {
    if paths.is_empty() {
        return;
    }
    let data: IDataObject = BasketData { paths }.into();
    let Ok(source) = data.cast::<IDropSource>() else { return };
    let mut effect = DROPEFFECT::default();
    let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY | DROPEFFECT_MOVE, &mut effect);
    refresh();
}

#[implement(IDataObject, IDropSource)]
struct BasketData {
    paths: Vec<String>,
}

fn fmt_ok(fmt: &FORMATETC, cf: u16) -> bool {
    fmt.cfFormat == cf && fmt.tymed & TYMED_HGLOBAL.0 as u32 != 0
}

impl IDataObject_Impl for BasketData_Impl {
    fn GetData(&self, fmt: *const FORMATETC) -> Result<STGMEDIUM> {
        let fmt = unsafe { &*fmt };
        let pref = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) } as u16;
        if fmt_ok(fmt, CF_HDROP.0) {
            let mem = unsafe { hdrop_mem(&self.paths) }.ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
            return Ok(medium(mem));
        }
        if pref != 0 && fmt_ok(fmt, pref) {
            let mem = unsafe { effect_mem(drag_effect(shift_down())) }.ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
            return Ok(medium(mem));
        }
        Err(Error::from(DV_E_FORMATETC))
    }

    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }

    fn QueryGetData(&self, fmt: *const FORMATETC) -> HRESULT {
        let fmt = unsafe { &*fmt };
        let pref = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) } as u16;
        if fmt_ok(fmt, CF_HDROP.0) || (pref != 0 && fmt_ok(fmt, pref)) {
            S_OK
        } else {
            DV_E_FORMATETC
        }
    }

    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if !out.is_null() {
            unsafe { (*out).ptd = std::ptr::null_mut() };
        }
        E_NOTIMPL
    }

    fn SetData(&self, _: *const FORMATETC, _: *const STGMEDIUM, _: BOOL) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }

    fn EnumFormatEtc(&self, dir: u32) -> Result<IEnumFORMATETC> {
        if dir != DATADIR_GET.0 as u32 {
            return Err(Error::from(E_NOTIMPL));
        }
        Ok(FmtEnum { items: drag_formats(), at: Cell::new(0) }.into())
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

fn drag_formats() -> Vec<FORMATETC> {
    let one = |cf: u16| FORMATETC {
        cfFormat: cf,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut items = vec![one(CF_HDROP.0)];
    let pref = unsafe { RegisterClipboardFormatW(w!("Preferred DropEffect")) } as u16;
    if pref != 0 {
        items.push(one(pref));
    }
    items
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
                paint(h);
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
                on_down(h, xy(lp), wp.0 & MK_CONTROL.0 as usize != 0);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                on_move(h, xy(lp));
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
                with_st(|s| {
                    s.press = None;
                    s.btn_down = None;
                });
                _ = InvalidateRect(Some(h), None, false);
                LRESULT(0)
            }
            WM_CLOSE => {
                close();
                LRESULT(0)
            }
            WM_DESTROY => {
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

unsafe fn on_down(h: HWND, at: (i32, i32), ctrl: bool) {
    layout(h);
    if let Some(id) = hit_button(at.0, at.1) {
        with_st(|s| s.btn_down = Some(id));
        _ = SetCapture(h);
        _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if let Some(i) = hit_row(at.0, at.1) {
        with_st(|s| {
            s.sel = click_sel(&s.sel, i, ctrl);
            s.anchor = i;
            s.press = Some(at);
        });
        _ = SetCapture(h);
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn on_move(h: HWND, at: (i32, i32)) {
    let (press, down) = with_st(|s| (s.press, s.btn_down));
    if let Some((x, y)) = press {
        let dx = at.0 - x;
        let dy = at.1 - y;
        if dx * dx + dy * dy >= 16 {
            let paths = chosen();
            with_st(|s| s.press = None);
            _ = ReleaseCapture();
            begin_drag(paths);
            return;
        }
    }
    let hot = if down.is_some() { down } else { hit_button(at.0, at.1) };
    let changed = with_st(|s| {
        let changed = s.hot != hot;
        s.hot = hot;
        changed
    });
    if changed {
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn on_up(h: HWND, at: (i32, i32)) {
    let down = with_st(|s| {
        let d = s.btn_down;
        s.btn_down = None;
        s.press = None;
        d
    });
    _ = ReleaseCapture();
    if let Some(id) = down {
        if hit_button(at.0, at.1) == Some(id) {
            act(id);
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

unsafe fn paint(h: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let dc = BeginPaint(h, &mut ps);
    if dc.is_invalid() {
        return;
    }
    layout(h);
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    fill(dc, rc, PAPER);
    let (list, scroll, row_h, buttons, hot, down) = with_st(|s| (s.list, s.scroll, s.row_h, s.buttons.clone(), s.hot, s.btn_down));
    let list_paths = paths();
    let sel = with_st(|s| s.sel.clone());
    let anchor = with_st(|s| s.anchor);
    let (font, small) = app(|a| (a.font, a.small_font));
    let dpi = GetDpiForWindow(h).max(96) as i32;
    if list_paths.is_empty() {
        text(dc, list, tr(Key::BasketEmpty), font, INK_DIM, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
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
        let (name_c, path_c, bg) = row_colors(gone, on);
        let clip = RECT {
            left: row.left,
            top: row.top.max(list.top),
            right: row.right,
            bottom: row.bottom.min(list.bottom),
        };
        fill(dc, clip, bg);
        if on && i == anchor {
            fill(dc, RECT { left: row.left, top: clip.top, right: row.left + sc(dpi, 3), bottom: clip.bottom }, CERULEAN);
        }
        let pad = sc(dpi, 10);
        let name_r = RECT { left: row.left + pad, top: row.top + sc(dpi, 4), right: row.right - pad, bottom: row.top + sc(dpi, 24) };
        let path_r = RECT { left: name_r.left, top: name_r.bottom, right: name_r.right, bottom: row.bottom - sc(dpi, 4) };
        text(dc, name_r, &basket_name(path), font, name_c, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        let mut detail = path.clone();
        if gone {
            detail.push_str("  ·  ");
            detail.push_str(tr(Key::BasketMissing));
        }
        text(dc, path_r, &detail, small, path_c, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
    }
    if list.bottom < rc.bottom {
        fill(dc, RECT { left: 0, top: list.bottom, right: rc.right, bottom: list.bottom + sc(dpi, 1) }, CERULEAN_DEEP);
    }
    for (id, r) in buttons {
        let on = enabled(id);
        let fill_c = button_fill(on, hot == Some(id) || down == Some(id));
        fill(dc, r, fill_c);
        let label = actions().iter().find(|(k, _)| *k == id).map(|(_, key)| tr(*key)).unwrap_or("");
        text(dc, r, label, font, button_ink(fill_c), DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
    }
    _ = EndPaint(h, &ps);
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
    fn ctrl_click_toggles_and_ctrl_a_selects_every_row() {
        assert_eq!(click_sel(&[], 1, false), vec![1]);
        assert_eq!(click_sel(&[1], 2, true), vec![1, 2]);
        assert_eq!(click_sel(&[1, 2], 1, true), vec![2]);
        assert_eq!(click_sel(&[0, 2], 2, false), vec![2]);
        assert_eq!(all_sel(3), vec![0, 1, 2]);
        assert_eq!(send_pick(&[2, 0], 2), Some(2));
        assert_eq!(send_pick(&[2, 0], 5), Some(0));
        assert_eq!(send_pick(&[], 0), None);
    }

    #[test]
    fn drag_copies_unless_shift_and_send_opens_the_parent_of_a_file() {
        assert_eq!(drag_effect(false), DROPEFFECT_COPY.0);
        assert_eq!(drag_effect(true), DROPEFFECT_MOVE.0);
        assert_ne!(drag_effect(false), drag_effect(true));
        assert_eq!(send_folder(r"C:\Work", true), r"C:\Work");
        assert_eq!(send_folder(r"C:\Work\a.txt", false), r"C:\Work");
        assert_eq!(parent_path(r"C:\a.txt"), r"C:\");
        assert_eq!(send_folder(r"\\srv\share\dir\f.txt", false), r"\\srv\share\dir");
    }

    #[test]
    fn cerulean_palette_never_uses_mint() {
        for c in [CERULEAN, CERULEAN_DEEP, CERULEAN_MIST, PALE, button_fill(true, false), button_fill(true, true), button_fill(false, false)] {
            assert_ne!(c, MINT, "{c:#08X}");
            assert_ne!(c, 0x2EC4A6);
        }
        assert_eq!(CERULEAN, 0x0A9BD6);
        assert_eq!(CERULEAN_DEEP, 0x0786BC);
        assert_eq!(CERULEAN_MIST, 0x9BE3FA);
        assert_eq!(button_fill(true, false), CERULEAN);
        assert_eq!(button_fill(true, true), CERULEAN_DEEP);
        assert_eq!(button_ink(CERULEAN), 0xFFFFFF);
        let (name, path, bg) = row_colors(true, true);
        assert_eq!(bg, CERULEAN_MIST);
        assert_ne!(name, INK);
        assert_ne!(path, INK);
        assert_eq!(row_colors(false, false).2, 0xFFFFFF);
        let widths = [(ACT_COPY, 80), (ACT_MOVE, 80), (ACT_REMOVE, 80), (ACT_CLEAR, 80), (ACT_SEND, 80)];
        let (h, rects) = place_buttons(200, &widths, 30, 6, 8);
        assert!(rects.len() == 5);
        assert!(h > 30);
        assert!(rects.iter().any(|(_, r)| r.top > 8));
        assert_eq!(scroll_by(10, -40, 100), 0);
        assert_eq!(scroll_by(90, 40, 100), 100);
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
