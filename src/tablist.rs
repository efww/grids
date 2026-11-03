//! Tab list: every window tab, its layout, and the folder open in each pane.
//! Folder chips use the same tracking tooltip as tab paths (`tip_enter`, below the chip, 4px gap).

use std::mem::size_of;

use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;

use crate::app::*;
use crate::i18n::*;
use crate::imports::*;
use crate::pane::*;
use crate::session::*;
use crate::ui::*;

const CHIP_TIP: usize = 11;
const ID_EDIT: i32 = 1;
const ID_NAME: i32 = 2;
const GO: u8 = 1;
const CLOSE_TAB: u8 = 2;
const DISMISS: u8 = 3;
const EM_SETCUEBANNER: u32 = 0x1501;
const EN_CHANGE: u32 = 0x0300;

#[derive(Clone)]
struct Chip {
    pane: usize,
    name: String,
    tip: String,
    tint: u8,
}

#[derive(Clone)]
struct Row {
    index: usize,
    name: String,
    locked: bool,
    color: Option<u32>,
    heights: Vec<usize>,
    grid: String,
    panes: usize,
    chips: Vec<Chip>,
    current: bool,
}

struct Btn {
    id: u8,
    rect: RECT,
    on: bool,
}

#[derive(Clone)]
struct RowBox {
    tab: usize,
    y: i32,
    h: i32,
}

#[derive(Clone)]
struct ChipBox {
    tab: usize,
    pane: usize,
    rect: RECT,
}

struct State {
    hwnd: HWND,
    owner: HWND,
    edit: HWND,
    /// Inline rename of the selected row. Hidden until F2.
    name_edit: HWND,
    query: String,
    sel: usize,
    /// Tab index whose name cell is the edit box. None when not renaming.
    renaming: Option<usize>,
    /// Stored `Tab::name` when the edit opened, so a moved tab is still the one that was renamed.
    rename_prev: String,
    /// Label placed in the box. Unchanged text is not written, so an automatic name stays automatic.
    rename_shown: String,
    /// Esc hides the box; the kill-focus that follows must not also save.
    rename_skip: bool,
    scroll: i32,
    hot_row: Option<usize>,
    hot_btn: Option<u8>,
    down_btn: Option<u8>,
    chip_tip: bool,
    dpi: i32,
    font: HFONT,
    bold: HFONT,
    small: HFONT,
    mono: HFONT,
    icon: HFONT,
    hint: RECT,
    search: RECT,
    chord: RECT,
    frame: RECT,
    head: RECT,
    list: RECT,
    status: RECT,
    buttons: Vec<Btn>,
    shown: Vec<Row>,
    rows: Vec<RowBox>,
    chips: Vec<ChipBox>,
    content_h: i32,
}

impl State {
    fn new() -> Self {
        Self {
            hwnd: HWND::default(),
            owner: HWND::default(),
            edit: HWND::default(),
            name_edit: HWND::default(),
            query: String::new(),
            sel: 0,
            renaming: None,
            rename_prev: String::new(),
            rename_shown: String::new(),
            rename_skip: false,
            scroll: 0,
            hot_row: None,
            hot_btn: None,
            down_btn: None,
            chip_tip: false,
            dpi: 96,
            font: HFONT::default(),
            bold: HFONT::default(),
            small: HFONT::default(),
            mono: HFONT::default(),
            icon: HFONT::default(),
            hint: RECT::default(),
            search: RECT::default(),
            chord: RECT::default(),
            frame: RECT::default(),
            head: RECT::default(),
            list: RECT::default(),
            status: RECT::default(),
            buttons: Vec::new(),
            shown: Vec::new(),
            rows: Vec::new(),
            chips: Vec::new(),
            content_h: 0,
        }
    }
}

thread_local! {
    static ST: std::cell::RefCell<State> = std::cell::RefCell::new(State::new());
}

fn with_st<R>(f: impl FnOnce(&mut State) -> R) -> R {
    ST.with(|s| f(&mut s.borrow_mut()))
}

fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi.max(96) / 96
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Columns × rows when every column is the same height (`[2, 2]` is `2×2`). Uneven layouts keep the column heights.
pub(crate) fn grid_label(heights: &[usize]) -> String {
    let Some(&first) = heights.first() else { return String::new() };
    if heights.iter().all(|&n| n == first) {
        format!("{}×{}", heights.len(), first)
    } else {
        heights.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" | ")
    }
}

/// Folder name on a chip. This PC uses the same label as a tab, not the shell CLSID.
pub(crate) fn chip_name(path: &str) -> String {
    if path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        return tr(Key::ThisPc).into();
    }
    folder_display_name(path)
}

/// Tooltip for a chip: the pane's full path. This PC is the localized name. An empty path shows nothing.
pub(crate) fn chip_tip(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    if path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        return tr(Key::ThisPc).into();
    }
    path.to_string()
}

/// Case-insensitive match on the tab name, a folder's display name, or its full path.
pub(crate) fn row_matches(name: &str, chips: &[(&str, &str)], q: &str) -> bool {
    let q = q.trim();
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    let has = |s: &str| s.to_lowercase().contains(&q);
    has(name) || chips.iter().any(|(n, p)| has(n) || has(p))
}

/// Chip rectangles inside the folder cell. `widths` are already capped at the cell width.
pub(crate) fn place_chips(widths: &[i32], avail: i32, chip_h: i32, gap: i32) -> Vec<RECT> {
    let avail = avail.max(1);
    let mut out = Vec::with_capacity(widths.len());
    let mut x = 0;
    let mut y = 0;
    for &w0 in widths {
        let w = w0.max(1).min(avail);
        if x > 0 && x + w > avail {
            x = 0;
            y += chip_h + gap;
        }
        out.push(RECT { left: x, top: y, right: x + w, bottom: y + chip_h });
        x += w + gap;
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KbdSeg {
    Text(String),
    Key(String),
}

/// Split a status line on `{Enter}` / `{F2}` / `{Del}` / `{Esc}` so those words draw as key chips.
pub(crate) fn kbd_segs(s: &str) -> Vec<KbdSeg> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        let (head, tail) = rest.split_at(start);
        if !head.is_empty() {
            out.push(KbdSeg::Text(head.to_string()));
        }
        if let Some(end) = tail.find('}') {
            let key = &tail[1..end];
            if !key.is_empty() && !key.contains('{') {
                out.push(KbdSeg::Key(key.to_string()));
                rest = &tail[end + 1..];
                continue;
            }
        }
        out.push(KbdSeg::Text("{".into()));
        rest = &tail[1..];
    }
    if !rest.is_empty() {
        out.push(KbdSeg::Text(rest.to_string()));
    }
    out
}

/// One wrapped run of key-hint pieces. `start`..`end` indexes the measured widths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FooterRow {
    pub start: usize,
    pub end: usize,
    pub width: i32,
}

/// Greedy rows of piece widths. A piece wider than `width` sits on its own row.
/// Each step takes at least one piece, so an empty remainder cannot loop.
pub(crate) fn wrap_runs(widths: &[i32], width: i32) -> Vec<FooterRow> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut i = 0;
    while i < widths.len() {
        let start = i;
        let mut w = 0;
        loop {
            let piece = widths[i].max(0);
            if i > start && w + piece > width {
                break;
            }
            w += piece;
            i += 1;
            if i == widths.len() || w >= width {
                break;
            }
        }
        rows.push(FooterRow { start, end: i, width: w });
    }
    rows
}

/// How the status line shares its width. The key hint is never clipped to make room for the count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FooterPlan {
    /// Width given to the left-hand count. It ellipsizes inside this.
    pub count_w: i32,
    /// The count shares the key row. Otherwise it is a full-width row above the keys.
    pub count_on_keys: bool,
    pub key_rows: Vec<FooterRow>,
    pub rows: i32,
}

/// `width` is the status bar. `text_w` is the count string. `piece_w` are key-hint segments.
/// `gap` sits between the count and the keys when they share a row.
///
/// Keys that fit on one line stay on that line and the count shrinks. Keys that do not fit wrap,
/// and the count takes the row above them.
pub(crate) fn plan_footer(width: i32, text_w: i32, piece_w: &[i32], gap: i32) -> FooterPlan {
    let width = width.max(1);
    let text_w = text_w.max(0);
    let gap = gap.max(0);
    let keys: i32 = piece_w.iter().map(|w| (*w).max(0)).sum();
    if piece_w.is_empty() || keys == 0 {
        return FooterPlan { count_w: width.min(text_w), count_on_keys: true, key_rows: Vec::new(), rows: 1 };
    }
    if keys <= width {
        let room = if text_w == 0 { 0 } else { width.saturating_sub(keys).saturating_sub(gap) };
        return FooterPlan {
            count_w: room.min(text_w),
            count_on_keys: true,
            key_rows: vec![FooterRow { start: 0, end: piece_w.len(), width: keys }],
            rows: 1,
        };
    }
    let key_rows = wrap_runs(piece_w, width);
    let n = key_rows.len().max(1) as i32;
    FooterPlan { count_w: width.min(text_w), count_on_keys: false, key_rows, rows: n + 1 }
}

fn status_caption(n: usize, q: &str) -> String {
    let n = n.to_string();
    if q.trim().is_empty() {
        tf(Key::TabListStatus, &[("n", n.as_str())])
    } else {
        tf(Key::TabListStatusFilter, &[("n", n.as_str()), ("q", q.trim())])
    }
}

fn row_ok(row: &Row, q: &str) -> bool {
    let chips: Vec<(&str, &str)> = row.chips.iter().map(|c| (c.name.as_str(), c.tip.as_str())).collect();
    row_matches(&row.name, &chips, q)
}

fn snapshot() -> Vec<Row> {
    app(|a| {
        let n = a.s.tabs.len();
        let cur = a.s.tab.min(n.saturating_sub(1));
        let mut rows = Vec::with_capacity(n);
        for i in 0..n {
            let Some(t) = a.s.tabs.get(i) else { continue };
            let live = i == a.s.tab;
            let heights = if live { a.s.heights.clone() } else { t.heights.clone() };
            let paths = if live { a.s.paths.clone() } else { t.paths.clone() };
            let tints = if live { a.s.tints.clone() } else { t.tints.clone() };
            let count = if live { a.s.panes() } else { heights.iter().sum() };
            let count = count.clamp(1, MAXP);
            let active = if live { a.s.active } else { t.active };
            let active = active.min(count - 1);
            let path = paths.get(active).map(String::as_str).unwrap_or("");
            let name = if custom_tab_name(&t.name).is_none() && path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
                tr(Key::ThisPc).to_string()
            } else {
                tab_label(&t.name, path)
            };
            let mut chips = Vec::new();
            for pane in 0..count {
                let p = paths.get(pane).map(String::as_str).unwrap_or("");
                if p.is_empty() {
                    continue;
                }
                let label = chip_name(p);
                if label.is_empty() {
                    continue;
                }
                chips.push(Chip { pane, name: label, tip: chip_tip(p), tint: tints.get(pane).copied().unwrap_or(0) });
            }
            rows.push(Row {
                index: i,
                name,
                locked: t.locked,
                color: t.color,
                grid: grid_label(&heights),
                heights,
                panes: count,
                chips,
                current: i == cur,
            });
        }
        rows
    })
}

fn visible_of(rows: &[Row], q: &str) -> Vec<usize> {
    rows.iter().filter(|r| row_ok(r, q)).map(|r| r.index).collect()
}

fn clamp_sel(rows: &[Row], q: &str, sel: usize) -> usize {
    let vis = visible_of(rows, q);
    if vis.iter().any(|&i| i == sel) {
        sel
    } else {
        vis.first().copied().unwrap_or(0)
    }
}

pub(crate) fn belongs(main: HWND, hwnd: HWND) -> bool {
    let (h, owner) = with_st(|s| (s.hwnd, s.owner));
    owner == main && !h.0.is_null() && (hwnd == h || unsafe { IsChild(h, hwnd).as_bool() })
}

pub(crate) fn captures(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    let b = with_st(|s| s.hwnd);
    !b.0.is_null() && (h == b || unsafe { IsChild(b, h).as_bool() })
}

fn alive_for(main: HWND) -> bool {
    let (h, owner) = with_st(|s| (s.hwnd, s.owner));
    owner == main && !h.0.is_null() && unsafe { IsWindow(Some(h)).as_bool() }
}

pub(crate) unsafe fn refresh() {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) unsafe fn restyle() {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    let title = HSTRING::from(tr(Key::TabList));
    _ = SetWindowTextW(h, &title);
    style_window(h);
    rebuild_fonts();
    theme_edit();
    cue();
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) unsafe fn close() {
    end_name_edit(true);
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        _ = DestroyWindow(h);
    }
}

/// Shortcut and the tab-bar button: open, or close when this window already has the list.
pub(crate) unsafe fn open_toggle() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        return;
    }
    if alive_for(main) {
        close();
        return;
    }
    reopen(main);
}

/// Tab right-click: open, or focus the list when it is already this window's.
pub(crate) unsafe fn open_menu() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        return;
    }
    if alive_for(main) {
        let h = with_st(|s| s.hwnd);
        _ = ShowWindow(h, SW_SHOWNORMAL);
        _ = SetForegroundWindow(h);
        return;
    }
    reopen(main);
}

unsafe fn reopen(main: HWND) {
    let other = with_st(|s| s.hwnd);
    if !other.0.is_null() && IsWindow(Some(other)).as_bool() {
        _ = DestroyWindow(other);
    }
    open(main);
}

/// Keys the list owns. False leaves the key for the filter edit, or for the name box while renaming.
pub(crate) unsafe fn on_key(vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
    let ch = VIRTUAL_KEY(vk);
    if name_editing() {
        return name_edit_key(ch, ctrl, shift, alt);
    }
    match ch {
        VK_F2 if !ctrl && !alt && !shift => {
            let filter = with_st(|s| s.edit);
            if ime_composing(filter) {
                return false;
            }
            begin_name_edit();
            true
        }
        VK_ESCAPE if !ctrl && !alt && !shift => {
            close();
            true
        }
        VK_RETURN if !ctrl && !alt && !shift => {
            go();
            true
        }
        VK_UP if !ctrl && !alt => {
            nudge_sel(-1);
            true
        }
        VK_DOWN if !ctrl && !alt => {
            nudge_sel(1);
            true
        }
        VK_PRIOR if !alt => {
            nudge_sel(-8);
            true
        }
        VK_NEXT if !alt => {
            nudge_sel(8);
            true
        }
        VK_HOME if !ctrl && !alt => {
            nudge_sel(-10_000);
            true
        }
        VK_END if !ctrl && !alt => {
            nudge_sel(10_000);
            true
        }
        VK_DELETE if !ctrl && !alt && !shift => {
            let (edit, q) = with_st(|s| (s.edit, s.query.clone()));
            let focused = GetFocus() == edit;
            if !focused || q.trim().is_empty() {
                close_selected();
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

unsafe fn open(main: HWND) {
    let dpi = GetDpiForWindow(main).max(96) as i32;
    let style = WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN | WS_VISIBLE;
    let mut rc = RECT { left: 0, top: 0, right: sc(dpi, 560), bottom: sc(dpi, 500) };
    _ = AdjustWindowRectExForDpi(&mut rc, style, false, WINDOW_EX_STYLE(0), dpi as u32);
    let (ww, hh) = (rc.right - rc.left, rc.bottom - rc.top);
    let (x, y) = place(main, ww, hh);
    let title = HSTRING::from(tr(Key::TabList));
    let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroTabList"), &title, style, x, y, ww, hh, Some(main), None, None, None) else {
        return;
    };
    let cur = app(|a| a.s.tab);
    with_st(|s| {
        *s = State::new();
        s.hwnd = hwnd;
        s.owner = main;
        s.sel = cur;
        s.dpi = GetDpiForWindow(hwnd).max(96) as i32;
    });
    style_window(hwnd);
    rebuild_fonts();
    let font = with_st(|s| s.font);
    let edit = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("EDIT"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        0,
        0,
        0,
        0,
        Some(hwnd),
        Some(HMENU(ID_EDIT as isize as _)),
        None,
        None,
    )
    .unwrap_or_default();
    with_st(|s| s.edit = edit);
    if !edit.0.is_null() {
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        _ = SetWindowSubclass(edit, Some(edit_proc), 1, 0);
    }
    let _ = ensure_name_edit();
    theme_edit();
    cue();
    layout(hwnd);
    _ = ShowWindow(hwnd, SW_SHOW);
    _ = SetForegroundWindow(hwnd);
    if !edit.0.is_null() {
        _ = SetFocus(Some(edit));
    }
}

fn place(owner: HWND, ww: i32, hh: i32) -> (i32, i32) {
    let mut frame = RECT::default();
    unsafe { _ = GetWindowRect(owner, &mut frame) };
    let mut x = frame.left + (frame.right - frame.left - ww) / 2;
    let mut y = frame.top + (frame.bottom - frame.top - hh) / 2;
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let work = if unsafe { GetMonitorInfoW(MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() } {
        mi.rcWork
    } else {
        RECT { left: 0, top: 0, right: 1920, bottom: 1080 }
    };
    x = x.clamp(work.left, (work.right - ww).max(work.left));
    y = y.clamp(work.top, (work.bottom - hh).max(work.top));
    (x, y)
}

unsafe fn style_window(hwnd: HWND) {
    let dark = DARK_ON.load(Ordering::Relaxed);
    let on = BOOL::from(dark);
    _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = cref(pal().bg);
    _ = DwmSetWindowAttribute(hwnd, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    let def = COLORREF(0xFFFF_FFFF);
    _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(36), &def as *const _ as _, 4);
    _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &def as *const _ as _, 4);
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

unsafe fn rebuild_fonts() {
    let (hwnd, dpi0) = with_st(|s| (s.hwnd, s.dpi));
    let dpi = if hwnd.0.is_null() { dpi0.max(96) } else { GetDpiForWindow(hwnd).max(96) as i32 };
    let face = ui_face();
    let (title_face, title_w) = title_style();
    let new = (
        make_font(face, 12, 400, dpi),
        make_font(title_face, 12, title_w, dpi),
        make_font(face, 11, 400, dpi),
        make_font("Consolas", 10, 400, dpi),
        make_font("Segoe MDL2 Assets", 12, 400, dpi),
    );
    let old = with_st(|s| {
        s.dpi = dpi;
        let old = (s.font, s.bold, s.small, s.mono, s.icon);
        (s.font, s.bold, s.small, s.mono, s.icon) = new;
        old
    });
    for f in [old.0, old.1, old.2, old.3, old.4] {
        delete_font(f);
    }
    let (edit, name, font, bold) = with_st(|s| (s.edit, s.name_edit, s.font, s.bold));
    if !edit.0.is_null() {
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    }
    if !name.0.is_null() {
        SendMessageW(name, WM_SETFONT, Some(WPARAM(bold.0 as usize)), Some(LPARAM(1)));
    }
}

unsafe fn theme_edit() {
    let (edit, name_edit) = with_st(|s| (s.edit, s.name_edit));
    let dark = DARK_ON.load(Ordering::Relaxed);
    let theme = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
    for h in [edit, name_edit] {
        if !h.0.is_null() {
            _ = SetWindowTheme(h, theme, PCWSTR::null());
        }
    }
}

unsafe fn cue() {
    let edit = with_st(|s| s.edit);
    if edit.0.is_null() {
        return;
    }
    let hs = HSTRING::from(tr(Key::TabListFilter));
    SendMessageW(edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(hs.as_ptr() as isize)));
}

unsafe fn text_px(dc: HDC, font: HFONT, s: &str) -> i32 {
    let old = SelectObject(dc, font.into());
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    let n = if w.is_empty() || !GetTextExtentPoint32W(dc, &w, &mut sz).as_bool() { 0 } else { sz.cx };
    SelectObject(dc, old);
    n
}

fn chord_label() -> String {
    crate::keys::index("tab.list").map(crate::keys::chord_text).unwrap_or_default()
}

unsafe fn layout(hwnd: HWND) {
    let dpi = GetDpiForWindow(hwnd).max(96) as i32;
    let mut rc = RECT::default();
    if GetClientRect(hwnd, &mut rc).is_err() {
        return;
    }
    let width = (rc.right - rc.left).max(1);
    let height = (rc.bottom - rc.top).max(1);
    let pad = sc(dpi, 8);
    let gap = sc(dpi, 6);
    let hint_h = sc(dpi, 18);
    let search_h = sc(dpi, 26);
    let btn_h = sc(dpi, 24);
    let y0 = pad;
    let hint = RECT { left: pad, top: y0, right: width - pad, bottom: y0 + hint_h };
    let search = RECT { left: pad, top: hint.bottom + gap, right: width - pad, bottom: hint.bottom + gap + search_h };

    let dc = GetDC(Some(hwnd));
    let (font, bold, mono, small) = with_st(|s| (s.font, s.bold, s.mono, s.small));
    let chord = chord_label();
    let chord_w = if chord.is_empty() { 0 } else { text_px(dc, mono, &chord) + sc(dpi, 10) };
    let chord_r = if chord_w == 0 {
        RECT::default()
    } else {
        let right = search.right - sc(dpi, 6);
        let h = sc(dpi, 16);
        let top = search.top + (search_h - h) / 2;
        RECT { left: right - chord_w, top, right, bottom: top + h }
    };
    let edit_left = search.left + sc(dpi, 8) + sc(dpi, 16);
    let edit_right = if chord_r.right > chord_r.left { chord_r.left - sc(dpi, 4) } else { search.right - sc(dpi, 6) };
    let edit_r = RECT { left: edit_left, top: search.top + sc(dpi, 3), right: edit_right.max(edit_left + 1), bottom: search.bottom - sc(dpi, 3) };

    let rows_all = snapshot();
    let (edit, prev_q, prev_sel) = with_st(|s| (s.edit, s.query.clone(), s.sel));
    let q = if edit.0.is_null() { prev_q } else { text_of(edit) };
    let sel = clamp_sel(&rows_all, &q, prev_sel);
    let shown: Vec<Row> = rows_all.into_iter().filter(|r| row_ok(r, &q)).collect();

    // The key hint stays whole. The count shrinks on the same line, or the keys wrap and the
    // count takes the line above. Measured before the list, so the buttons move up with it.
    let line_h = sc(dpi, 22);
    let status_w = (width - pad * 2).max(1);
    let caption = status_caption(shown.len(), &q);
    let piece_w = kbd_piece_widths(dc, small, mono, dpi);
    let footer = plan_footer(status_w, text_px(dc, small, &caption), &piece_w, sc(dpi, 8));
    let status_h = line_h * footer.rows.max(1);
    let btn_top = height - pad - status_h - gap - btn_h;
    let frame_top = search.bottom + gap;
    let frame_bot = (btn_top - gap).max(frame_top + sc(dpi, 40));
    let frame = RECT { left: pad, top: frame_top, right: width - pad, bottom: frame_bot };
    let head_h = sc(dpi, 22);
    let head = RECT { left: frame.left + 1, top: frame.top + 1, right: frame.right - 1, bottom: (frame.top + 1 + head_h).min(frame.bottom - 1) };
    let list = RECT { left: head.left, top: head.bottom, right: head.right, bottom: (frame.bottom - 1).max(head.bottom) };
    let status = RECT { left: pad, top: height - pad - status_h, right: width - pad, bottom: height - pad };

    let col_name = sc(dpi, 112);
    let col_lay = sc(dpi, 56);
    let col_n = sc(dpi, 32);
    let col_gap = sc(dpi, 6);
    let inset = sc(dpi, 8);
    let folders_left = list.left + inset + col_name + col_gap + col_lay + col_gap + col_n + col_gap;
    let folders_right = (list.right - inset).max(folders_left + 1);
    let avail = (folders_right - folders_left).max(1);
    let chip_h = sc(dpi, 16);
    let chip_gap = sc(dpi, 3);

    let mut y = 0;
    let mut boxes = Vec::new();
    let mut chips = Vec::new();
    for row in &shown {
        let widths: Vec<i32> = row.chips.iter().map(|c| chip_width(dc, small, &c.name, dpi)).collect();
        let placed = place_chips(&widths, avail, chip_h, chip_gap);
        let block = placed.last().map(|r| r.bottom).unwrap_or(0);
        let row_h = (block + sc(dpi, 4)).max(sc(dpi, 24));
        let row_top = list.top + y; // client y before scroll; adjusted below
        let _ = row_top;
        let top_off = if block == 0 { 0 } else { ((row_h - block) / 2).max(sc(dpi, 2)) };
        for (chip, r) in row.chips.iter().zip(placed) {
            chips.push(ChipBox {
                tab: row.index,
                pane: chip.pane,
                rect: RECT {
                    left: folders_left + r.left,
                    top: r.top + top_off, // content-relative; scroll applied after
                    right: folders_left + r.right,
                    bottom: r.bottom + top_off,
                },
            });
        }
        boxes.push(RowBox { tab: row.index, y, h: row_h });
        y += row_h;
    }
    let content_h = y;
    let view_h = (list.bottom - list.top).max(0);
    let max_scroll = (content_h - view_h).max(0);
    let mut scroll = with_st(|s| s.scroll.clamp(0, max_scroll));
    // Keep the row being renamed on screen, so hiding the box to scroll it away does not save.
    if let Some(tab) = with_st(|s| s.renaming) {
        if let Some(row) = boxes.iter().find(|b| b.tab == tab) {
            if row.h < view_h {
                if row.y < scroll {
                    scroll = row.y;
                } else if row.y + row.h > scroll + view_h {
                    scroll = row.y + row.h - view_h;
                }
                scroll = scroll.clamp(0, max_scroll);
            }
        }
    }
    for chip in &mut chips {
        let row_y = boxes.iter().find(|b| b.tab == chip.tab).map(|b| b.y).unwrap_or(0);
        chip.rect.top += list.top + row_y - scroll;
        chip.rect.bottom += list.top + row_y - scroll;
    }

    let go_on = !shown.is_empty();
    let close_on = go_on && app(|a| can_close_tab(&a.s.tabs, sel));
    let labels = [(GO, tr(Key::TabListGo), true), (CLOSE_TAB, tr(Key::CloseTab), false), (DISMISS, tr(Key::Close), false)];
    let mut buttons = Vec::new();
    let mut x = pad;
    for (id, label, primary) in labels {
        if id == DISMISS {
            continue;
        }
        let face = if primary { bold } else { font };
        let w = text_px(dc, face, label) + sc(dpi, 24);
        let on = if id == GO { go_on } else { close_on };
        buttons.push(Btn { id, rect: RECT { left: x, top: btn_top, right: x + w, bottom: btn_top + btn_h }, on });
        x += w + sc(dpi, 4);
    }
    let dismiss = tr(Key::Close);
    let dw = text_px(dc, font, dismiss) + sc(dpi, 24);
    buttons.push(Btn {
        id: DISMISS,
        rect: RECT { left: width - pad - dw, top: btn_top, right: width - pad, bottom: btn_top + btn_h },
        on: true,
    });
    let _ = ReleaseDC(Some(hwnd), dc);

    with_st(|s| {
        s.dpi = dpi;
        s.query = q;
        s.sel = sel;
        s.scroll = scroll;
        s.hint = hint;
        s.search = search;
        s.chord = chord_r;
        s.frame = frame;
        s.head = head;
        s.list = list;
        s.status = status;
        s.buttons = buttons;
        s.shown = shown;
        s.rows = boxes;
        s.chips = chips;
        s.content_h = content_h;
    });
    let edit = with_st(|s| s.edit);
    if !edit.0.is_null() {
        _ = SetWindowPos(edit, None, edit_r.left, edit_r.top, (edit_r.right - edit_r.left).max(1), (edit_r.bottom - edit_r.top).max(1), SWP_NOZORDER | SWP_NOACTIVATE);
    }
    place_name_edit();
    let info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (content_h - 1).max(0),
        nPage: view_h.max(0) as u32,
        nPos: scroll,
        nTrackPos: 0,
    };
    _ = SetScrollInfo(hwnd, SB_VERT, &info, false);
}

fn chip_width(dc: HDC, font: HFONT, name: &str, dpi: i32) -> i32 {
    let text = unsafe { text_px(dc, font, name) };
    let pad = sc(dpi, 4) + sc(dpi, 5) + sc(dpi, 7) + sc(dpi, 3);
    (text + pad).clamp(sc(dpi, 20), sc(dpi, 108))
}

fn chip_dot(tint: u8) -> u32 {
    tint_color(tint).unwrap_or_else(|| pal().accents[1].1)
}

unsafe fn nudge_sel(delta: i32) {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    layout(h);
    let (sel, ids) = with_st(|s| (s.sel, s.shown.iter().map(|r| r.index).collect::<Vec<_>>()));
    if ids.is_empty() {
        return;
    }
    let pos = ids.iter().position(|&i| i == sel).unwrap_or(0);
    let next = (pos as i32 + delta).clamp(0, ids.len() as i32 - 1) as usize;
    with_st(|s| s.sel = ids[next]);
    reveal();
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn reveal() {
    let (list, scroll, sel, rows) = with_st(|s| (s.list, s.scroll, s.sel, s.rows.clone()));
    let Some(row) = rows.iter().find(|r| r.tab == sel) else { return };
    let view = (list.bottom - list.top).max(0);
    let mut next = scroll;
    if row.y < scroll {
        next = row.y;
    } else if row.y + row.h > scroll + view {
        next = row.y + row.h - view;
    }
    let max = with_st(|s| (s.content_h - view).max(0));
    next = next.clamp(0, max);
    if next != scroll {
        hide_chip_tip();
        with_st(|s| s.scroll = next);
    }
}

unsafe fn go() {
    end_name_edit(true);
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() {
        layout(h);
    }
    let (sel, any, owner) = with_st(|s| (s.sel, !s.shown.is_empty(), s.owner));
    if !any {
        return;
    }
    switch_tab(sel);
    close();
    if !owner.0.is_null() && IsWindow(Some(owner)).as_bool() {
        bind_hwnd(owner);
        focus_view(active_pane());
    }
}

unsafe fn close_selected() {
    end_name_edit(true);
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() {
        layout(h);
    }
    let (sel, any) = with_st(|s| (s.sel, s.shown.iter().any(|r| r.index == s.sel)));
    if !any || !app(|a| can_close_tab(&a.s.tabs, sel)) {
        return;
    }
    hide_chip_tip();
    close_tab(sel);
    let n = app(|a| a.s.tabs.len());
    with_st(|s| s.sel = sel.min(n.saturating_sub(1)));
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        layout(h);
        _ = InvalidateRect(Some(h), None, false);
        let edit = with_st(|s| s.edit);
        if !edit.0.is_null() {
            _ = SetFocus(Some(edit));
        }
    }
}

unsafe fn hide_chip_tip() {
    let (h, inst) = with_st(|s| (s.hwnd, s.chip_tip));
    if h.0.is_null() {
        return;
    }
    let (now, _) = tip_leave(h, CHIP_TIP, inst);
    with_st(|s| s.chip_tip = now);
}

fn tip_key(tab: usize, pane: usize, tip: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in tip.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^ ((tab as u64) << 48) ^ ((pane as u64) << 32)
}

unsafe fn show_chip_tip(x: i32, y: i32) {
    let (h, list, inst, chips) = with_st(|s| (s.hwnd, s.list, s.chip_tip, s.chips.clone()));
    if h.0.is_null() {
        return;
    }
    let hit = inside(list, x, y).then(|| chips.into_iter().find(|c| inside(c.rect, x, y))).flatten();
    let Some(chip) = hit else {
        if inst {
            hide_chip_tip();
        }
        return;
    };
    let tip = snapshot()
        .into_iter()
        .find(|r| r.index == chip.tab)
        .and_then(|r| r.chips.into_iter().find(|c| c.pane == chip.pane))
        .map(|c| c.tip)
        .unwrap_or_default();
    let screen = tip_screen_rect(h, chip.rect);
    let (now, _) = tip_enter(h, CHIP_TIP, tip_key(chip.tab, chip.pane, &tip), &tip, screen, TipSide::Below, inst);
    with_st(|s| s.chip_tip = now);
}

fn hit_btn(x: i32, y: i32) -> Option<u8> {
    with_st(|s| s.buttons.iter().find(|b| b.on && inside(b.rect, x, y)).map(|b| b.id))
}

fn hit_row(x: i32, y: i32) -> Option<usize> {
    let (list, scroll, rows) = with_st(|s| (s.list, s.scroll, s.rows.clone()));
    if !inside(list, x, y) {
        return None;
    }
    let y = y - list.top + scroll;
    rows.into_iter().find(|r| y >= r.y && y < r.y + r.h).map(|r| r.tab)
}

fn scroll_to(next: i32) {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    unsafe { hide_chip_tip() };
    with_st(|s| s.scroll = next);
    unsafe {
        layout(h);
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn on_scroll(wp: usize) {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    layout(h);
    let (view, cur, content) = with_st(|s| ((s.list.bottom - s.list.top).max(1), s.scroll, s.content_h));
    let max = (content - view).max(0);
    let mut info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        nMin: 0,
        nMax: 0,
        nPage: 0,
        nPos: 0,
        nTrackPos: 0,
    };
    _ = GetScrollInfo(h, SB_VERT, &mut info);
    let line = sc(with_st(|s| s.dpi), 24);
    let next = match SCROLLBAR_COMMAND((wp & 0xffff) as i32) {
        SB_LINEUP => cur - line,
        SB_LINEDOWN => cur + line,
        SB_PAGEUP => cur - view,
        SB_PAGEDOWN => cur + view,
        SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
        SB_TOP => 0,
        SB_BOTTOM => max,
        _ => cur,
    };
    scroll_to(next.clamp(0, max));
}

fn cols(list: RECT, dpi: i32) -> (i32, i32, i32, i32, i32) {
    let inset = sc(dpi, 8);
    let gap = sc(dpi, 6);
    let name = sc(dpi, 112);
    let lay = sc(dpi, 56);
    let n = sc(dpi, 32);
    let x = list.left + inset;
    (x, x + name + gap, x + name + gap + lay + gap, x + name + gap + lay + gap + n + gap, list.right - inset)
}

unsafe fn paint(dc: HDC, _rc: RECT) {
    let st = with_st(|s| PaintSnap {
        dpi: s.dpi,
        hint: s.hint,
        search: s.search,
        chord: s.chord,
        frame: s.frame,
        head: s.head,
        list: s.list,
        status: s.status,
        buttons: s.buttons.iter().map(|b| (b.id, b.rect, b.on)).collect(),
        shown: s.shown.clone(),
        rows: s.rows.clone(),
        chips: s.chips.clone(),
        scroll: s.scroll,
        sel: s.sel,
        hot_row: s.hot_row,
        hot_btn: s.hot_btn,
        query: s.query.clone(),
        renaming: s.renaming,
        font: s.font,
        bold: s.bold,
        small: s.small,
        mono: s.mono,
        icon: s.icon,
    });
    let p = pal();
    let dpi = st.dpi;
    fill(dc, _rc, p.bg);
    text(dc, st.hint, tr(Key::TabListHint), st.small, p.sub, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX);
    fill(dc, st.search, p.card);
    frame(dc, st.search, 1, p.line);
    let glyph = RECT { left: st.search.left + sc(dpi, 6), top: st.search.top, right: st.search.left + sc(dpi, 22), bottom: st.search.bottom };
    text(dc, glyph, ICON_SEARCH, st.icon, p.sub, DT_CENTER);
    if st.chord.right > st.chord.left {
        aa_rfill(dc, st.chord, sc(dpi, 2).max(1), p.pill, Some(p.line), p.card);
        // Heavier bottom edge, same as the status key chips.
        fill(dc, RECT { top: st.chord.bottom - sc(dpi, 2).max(2), ..st.chord }, p.line);
        text(dc, st.chord, &chord_label(), st.mono, p.sub, DT_CENTER);
    }
    fill(dc, st.frame, p.card);
    frame(dc, st.frame, 1, p.line);
    fill(dc, st.head, p.bg);
    fill(dc, RECT { top: st.head.bottom - 1, bottom: st.head.bottom, ..st.head }, p.line);
    let (x0, x1, x2, x3, x4) = cols(st.head, dpi);
    let head_ink = p.sub;
    text(dc, RECT { left: x0, right: x1 - sc(dpi, 6), ..st.head }, tr(Key::Tab), st.small, head_ink, DT_LEFT);
    text(dc, RECT { left: x1, right: x2 - sc(dpi, 6), ..st.head }, tr(Key::TabListLayout), st.small, head_ink, DT_LEFT);
    text(dc, RECT { left: x2, right: x3 - sc(dpi, 6), ..st.head }, tr(Key::TabListPanes), st.small, head_ink, DT_RIGHT);
    text(dc, RECT { left: x3, right: x4, ..st.head }, tr(Key::Folder), st.small, head_ink, DT_LEFT);

    let saved = SaveDC(dc);
    IntersectClipRect(dc, st.list.left, st.list.top, st.list.right, st.list.bottom);
    let dark = DARK_ON.load(Ordering::Relaxed);
    let current_bg = if dark { 0x1A3F4E } else { 0xD7F4FC };
    let hover_bg = if dark { 0x2A2C30 } else { 0xF7F5F1 };
    for (row, slot) in st.shown.iter().zip(st.rows.iter()) {
        let top = st.list.top + slot.y - st.scroll;
        let r = RECT { left: st.list.left, top, right: st.list.right, bottom: top + slot.h };
        if r.bottom <= st.list.top || r.top >= st.list.bottom {
            continue;
        }
        let selected = row.index == st.sel;
        let hot = st.hot_row == Some(row.index);
        let bg = if row.current {
            current_bg
        } else if selected {
            blend(p.card, crate::basket::CERULEAN, 0.10)
        } else if hot {
            hover_bg
        } else {
            p.card
        };
        fill(dc, r, bg);
        if row.current {
            fill(dc, RECT { right: r.left + sc(dpi, 3).max(3), ..r }, crate::basket::CERULEAN);
        }
        fill(dc, RECT { top: r.bottom - 1, bottom: r.bottom, ..r }, p.line);
        paint_name(dc, r, row, dpi, &st, bg);
        paint_layout(dc, r, row, dpi, &st);
        let count = RECT { left: x2, top: r.top, right: x3 - sc(dpi, 2), bottom: r.bottom };
        text(dc, count, &row.panes.to_string(), st.small, p.sub, DT_RIGHT);
        paint_chips(dc, row, &st, bg);
    }
    let _ = RestoreDC(dc, saved);

    for (id, rect, on) in &st.buttons {
        paint_btn(dc, *rect, *id, *on, st.hot_btn == Some(*id), &st);
    }
    paint_footer(dc, st.status, &status_caption(st.shown.len(), &st.query), &st);
}

struct PaintSnap {
    dpi: i32,
    hint: RECT,
    search: RECT,
    chord: RECT,
    frame: RECT,
    head: RECT,
    list: RECT,
    status: RECT,
    buttons: Vec<(u8, RECT, bool)>,
    shown: Vec<Row>,
    rows: Vec<RowBox>,
    chips: Vec<ChipBox>,
    scroll: i32,
    sel: usize,
    hot_row: Option<usize>,
    hot_btn: Option<u8>,
    query: String,
    renaming: Option<usize>,
    font: HFONT,
    bold: HFONT,
    small: HFONT,
    mono: HFONT,
    icon: HFONT,
}

unsafe fn kbd_piece_widths(dc: HDC, small: HFONT, mono: HFONT, dpi: i32) -> Vec<i32> {
    kbd_segs(tr(Key::TabListKeys))
        .iter()
        .map(|seg| match seg {
            KbdSeg::Text(s) => text_px(dc, small, s),
            KbdSeg::Key(s) => text_px(dc, mono, s) + sc(dpi, 8),
        })
        .collect()
}

/// The count ellipsizes. Every key chip is drawn, on a later row when the bar is too narrow.
unsafe fn paint_footer(dc: HDC, status: RECT, caption: &str, st: &PaintSnap) {
    let dpi = st.dpi;
    let width = (status.right - status.left).max(1);
    let segs = kbd_segs(tr(Key::TabListKeys));
    let widths = kbd_piece_widths(dc, st.small, st.mono, dpi);
    let plan = plan_footer(width, text_px(dc, st.small, caption), &widths, sc(dpi, 8));
    let line_h = sc(dpi, 22);
    let p = pal();
    if plan.count_w > 0 {
        let bottom = if plan.count_on_keys { status.bottom } else { (status.top + line_h).min(status.bottom) };
        let count = RECT { left: status.left, top: status.top, right: status.left + plan.count_w, bottom };
        text(dc, count, caption, st.small, p.sub, DT_LEFT);
    }
    let mut y = if plan.count_on_keys { status.top } else { status.top + line_h };
    let saved = SaveDC(dc);
    IntersectClipRect(dc, status.left, status.top, status.right, status.bottom);
    for row in &plan.key_rows {
        let fits = row.width <= width;
        let mut x = if fits { status.right - row.width } else { status.left };
        let row_bot = y + line_h;
        for i in row.start..row.end {
            let w = widths.get(i).copied().unwrap_or(0);
            let r = RECT { left: x, top: y, right: x + w, bottom: row_bot };
            match segs.get(i) {
                Some(KbdSeg::Text(s)) => text(dc, r, s, st.small, p.sub, DT_LEFT),
                Some(KbdSeg::Key(s)) => {
                    let h = sc(dpi, 14);
                    let top = y + (line_h - h) / 2;
                    let kr = RECT { left: r.left, top, right: r.right, bottom: top + h };
                    aa_rfill(dc, kr, sc(dpi, 2).max(1), p.pill, Some(p.line), p.bg);
                    fill(dc, RECT { top: kr.bottom - sc(dpi, 2).max(2), ..kr }, p.line);
                    text(dc, kr, s, st.mono, p.sub, DT_CENTER);
                }
                None => {}
            }
            x += w;
        }
        y += line_h;
    }
    let _ = RestoreDC(dc, saved);
}

unsafe fn paint_name(dc: HDC, row: RECT, data: &Row, dpi: i32, st: &PaintSnap, ground: u32) {
    let text_r = name_text_rect(row, data.color.is_some(), dpi);
    if let Some(c) = data.color {
        let d = sc(dpi, 8);
        let mid = (row.top + row.bottom) / 2;
        let dot = RECT { left: text_r.left - sc(dpi, 4) - d, top: mid - d / 2, right: text_r.left - sc(dpi, 4), bottom: mid + d / 2 };
        ellipse(dc, dot, c, None);
    }
    if st.renaming == Some(data.index) {
        let field = name_field(row, data.color.is_some(), dpi);
        aa_rfill(dc, field, sc(dpi, crate::settings::FIELD_RADIUS_DIP).max(1), pal().card, Some(pal().line), ground);
        return;
    }
    let mut r = text_r;
    if data.locked {
        let lock_w = sc(dpi, 14);
        let room = (r.right - r.left - lock_w).max(0);
        let tw = text_px(dc, st.bold, &data.name).min(room);
        let lock = RECT { left: r.left + tw, top: r.top, right: r.left + tw + lock_w, bottom: r.bottom };
        r.right = lock.left;
        text(dc, lock, ICON_LOCK, st.icon, pal().sub, DT_CENTER);
    }
    text(dc, r, &data.name, st.bold, pal().text, DT_LEFT);
}

unsafe fn paint_layout(dc: HDC, row: RECT, data: &Row, dpi: i32, st: &PaintSnap) {
    let (_, x1, x2, _, _) = cols(RECT { left: row.left, right: row.right, ..row }, dpi);
    let iw = sc(dpi, 22);
    let ih = sc(dpi, 14);
    let mid = (row.top + row.bottom) / 2;
    let icon = RECT { left: x1, top: mid - ih / 2, right: x1 + iw, bottom: mid - ih / 2 + ih };
    paint_grid_icon(dc, icon, &data.heights);
    let label = RECT { left: icon.right + sc(dpi, 4), top: row.top, right: x2 - sc(dpi, 2), bottom: row.bottom };
    text(dc, label, &data.grid, st.mono, pal().sub, DT_LEFT);
}

unsafe fn paint_grid_icon(dc: HDC, r: RECT, heights: &[usize]) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    fill(dc, r, pal().line);
    let inner = RECT { left: r.left + 1, top: r.top + 1, right: (r.right - 1).max(r.left + 2), bottom: (r.bottom - 1).max(r.top + 2) };
    fill(dc, inner, pal().sub);
    let spec: Vec<usize> = if heights.is_empty() { vec![1] } else { heights.to_vec() };
    let cols_n = spec.len().clamp(1, 8) as i32;
    let gap = 1;
    let iw = inner.right - inner.left;
    let ih = inner.bottom - inner.top;
    if iw < cols_n || ih < 1 {
        return;
    }
    let gaps = gap * (cols_n - 1);
    let base = (iw - gaps) / cols_n;
    let extra = (iw - gaps) - base * cols_n;
    let mut x = inner.left;
    for c in 0..cols_n {
        let rows = spec.get(c as usize).copied().unwrap_or(1).clamp(1, 8) as i32;
        let w = base + if c < extra { 1 } else { 0 };
        let rg = gap * (rows - 1);
        let rh = ((ih - rg) / rows).max(1);
        let mut y = inner.top;
        for _ in 0..rows {
            let bottom = (y + rh).min(inner.bottom);
            if w > 0 && bottom > y {
                fill(dc, RECT { left: x, top: y, right: (x + w).min(inner.right), bottom }, pal().card);
            }
            y = bottom + gap;
        }
        x += w + gap;
    }
}

unsafe fn paint_chips(dc: HDC, row: &Row, st: &PaintSnap, ground: u32) {
    let dpi = st.dpi;
    let p = pal();
    for (chip, spot) in row.chips.iter().zip(st.chips.iter().filter(|c| c.tab == row.index)) {
        let r = spot.rect;
        if r.bottom <= st.list.top || r.top >= st.list.bottom {
            continue;
        }
        aa_rfill(dc, r, sc(dpi, 2).max(1), p.pill, Some(p.line), ground);
        let d = sc(dpi, 7);
        let mid = (r.top + r.bottom) / 2;
        let dot = RECT { left: r.left + sc(dpi, 4), top: mid - d / 2, right: r.left + sc(dpi, 4) + d, bottom: mid + d / 2 };
        ellipse(dc, dot, chip_dot(chip.tint), None);
        let text_r = RECT { left: dot.right + sc(dpi, 3), right: r.right - sc(dpi, 4), ..r };
        text(dc, text_r, &chip.name, st.small, p.text, DT_LEFT);
    }
}

unsafe fn paint_btn(dc: HDC, r: RECT, id: u8, on: bool, hot: bool, st: &PaintSnap) {
    let p = pal();
    let primary = id == GO;
    let (fill_c, ink, border) = if !on {
        (p.card, blend(p.card, p.text, 0.45), blend(p.card, p.line, 0.65))
    } else if primary {
        let c = crate::basket::primary_fill(hot);
        (c, 0xFFFFFFu32, crate::basket::CERULEAN_DEEP)
    } else if hot {
        (p.card, p.text, p.sub)
    } else {
        (p.card, p.text, p.line)
    };
    aa_rfill(dc, r, sc(st.dpi, 2).max(1), fill_c, Some(border), p.bg);
    let label = match id {
        GO => tr(Key::TabListGo),
        CLOSE_TAB => tr(Key::CloseTab),
        _ => tr(Key::Close),
    };
    let font = if primary && on { st.bold } else { st.font };
    text(dc, r, label, font, ink, DT_CENTER);
}

unsafe extern "system" fn edit_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if msg == WM_MOUSEWHEEL {
        let parent = GetParent(h).unwrap_or_default();
        if !parent.0.is_null() {
            return SendMessageW(parent, msg, Some(wp), Some(lp));
        }
    }
    DefSubclassProc(h, msg, wp, lp)
}

pub(crate) extern "system" fn tablist_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| tablist_proc_inner(h, msg, wp, lp)))
}

fn tablist_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        match msg {
            WM_SIZE => {
                layout(h);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                layout(h);
                buffered(h, |dc, rc| paint(dc, rc));
            }
            WM_VSCROLL => on_scroll(wp.0),
            WM_MOUSEWHEEL => {
                let notch = ((wp.0 >> 16) as i16 as i32) / 120;
                let line = sc(with_st(|s| s.dpi), 24);
                let (cur, content, view) = with_st(|s| (s.scroll, s.content_h, (s.list.bottom - s.list.top).max(0)));
                let max = (content - view).max(0);
                scroll_to((cur - notch * line * 3).clamp(0, max));
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let row = hit_row(x, y);
                let btn = hit_btn(x, y);
                let changed = with_st(|s| {
                    let changed = s.hot_row != row || s.hot_btn != btn;
                    s.hot_row = row;
                    s.hot_btn = btn;
                    changed
                });
                show_chip_tip(x, y);
                if changed {
                    _ = InvalidateRect(Some(h), None, false);
                }
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                _ = TrackMouseEvent(&mut t);
            }
            WM_MOUSELEAVE => {
                hide_chip_tip();
                let changed = with_st(|s| {
                    let changed = s.hot_row.is_some() || s.hot_btn.is_some();
                    s.hot_row = None;
                    s.hot_btn = None;
                    changed
                });
                if changed {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some(id) = hit_btn(x, y) {
                    with_st(|s| s.down_btn = Some(id));
                    SetCapture(h);
                } else if let Some(tab) = hit_row(x, y) {
                    with_st(|s| s.sel = tab);
                    _ = InvalidateRect(Some(h), None, false);
                    let edit = with_st(|s| s.edit);
                    if !edit.0.is_null() {
                        _ = SetFocus(Some(edit));
                    }
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let down = with_st(|s| s.down_btn.take());
                if GetCapture() == h {
                    _ = ReleaseCapture();
                }
                if let Some(id) = down {
                    if hit_btn(x, y) == Some(id) {
                        match id {
                            GO => go(),
                            CLOSE_TAB => close_selected(),
                            DISMISS => close(),
                            _ => {}
                        }
                    }
                }
            }
            WM_LBUTTONDBLCLK => {
                if hit_row(xy(lp).0, xy(lp).1).is_some() {
                    go();
                }
            }
            WM_COMMAND => {
                let id = (wp.0 & 0xffff) as i32;
                let code = ((wp.0 >> 16) & 0xffff) as u32;
                if id == ID_EDIT && code == EN_CHANGE {
                    hide_chip_tip();
                    layout(h);
                    _ = InvalidateRect(Some(h), None, false);
                } else if id == ID_NAME && code == EN_KILLFOCUS {
                    let (skip, on) = with_st(|s| {
                        let skip = s.rename_skip;
                        if skip {
                            s.rename_skip = false;
                        }
                        (skip, s.renaming.is_some())
                    });
                    if !skip && on {
                        end_name_edit(true);
                    }
                }
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetTextColor(dc, cref(pal().text));
                SetBkColor(dc, cref(pal().card));
                SetDCBrushColor(dc, cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_GETMINMAXINFO => {
                let dpi = GetDpiForWindow(h).max(96);
                let mut rc = RECT { left: 0, top: 0, right: sc(dpi as i32, 420), bottom: sc(dpi as i32, 280) };
                let style = WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN;
                _ = AdjustWindowRectExForDpi(&mut rc, style, false, WINDOW_EX_STYLE(0), dpi);
                let m = &mut *(lp.0 as *mut MINMAXINFO);
                m.ptMinTrackSize.x = rc.right - rc.left;
                m.ptMinTrackSize.y = rc.bottom - rc.top;
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                rebuild_fonts();
                layout(h);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_CLOSE => {
                end_name_edit(true);
                hide_chip_tip();
                _ = DestroyWindow(h);
            }
            WM_DESTROY => {
                hide_chip_tip();
                let (edit, name) = with_st(|s| (s.edit, s.name_edit));
                if !edit.0.is_null() && IsWindow(Some(edit)).as_bool() {
                    _ = RemoveWindowSubclass(edit, Some(edit_proc), 1);
                }
                if !name.0.is_null() && IsWindow(Some(name)).as_bool() {
                    _ = RemoveWindowSubclass(name, Some(edit_proc), 2);
                }
                let old = with_st(|s| {
                    let old = (s.font, s.bold, s.small, s.mono, s.icon);
                    *s = State::new();
                    old
                });
                for f in [old.0, old.1, old.2, old.3, old.4] {
                    delete_font(f);
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

/// Name column after the colour dot. The lock glyph is drawn in this rect when the row is not being renamed.
fn name_text_rect(row: RECT, color: bool, dpi: i32) -> RECT {
    let (x0, x1, _, _, _) = cols(RECT { left: row.left, right: row.right, ..row }, dpi);
    let mut left = x0;
    if color {
        let d = sc(dpi, 8);
        left += d + sc(dpi, 4);
    }
    RECT { left, top: row.top, right: (x1 - sc(dpi, 6)).max(left + 1), bottom: row.bottom }
}

/// One-line field behind the rename box, centred on the name. A tall chip row does not stretch it.
fn name_field(row: RECT, color: bool, dpi: i32) -> RECT {
    let text = name_text_rect(row, color, dpi);
    let h = sc(dpi, 22).min((text.bottom - text.top).max(1));
    let top = text.top + (text.bottom - text.top - h) / 2;
    RECT { left: text.left, top, right: text.right, bottom: top + h }
}

/// `None` when the box still shows the label it opened with, so leaving it does not pin an automatic folder name.
/// `Some` is the title to store. Empty clears a custom name.
pub(crate) fn inline_rename_result(opened: &str, typed: &str) -> Option<String> {
    if typed == opened { None } else { Some(stored_tab_name(typed)) }
}

fn name_editing() -> bool {
    let (edit, on) = with_st(|s| (s.name_edit, s.renaming.is_some()));
    on && !edit.0.is_null() && unsafe { GetFocus() } == edit
}

unsafe fn name_edit_key(ch: VIRTUAL_KEY, ctrl: bool, shift: bool, alt: bool) -> bool {
    let edit = with_st(|s| s.name_edit);
    match ch {
        VK_RETURN if !ctrl && !alt && !shift => {
            if ime_composing(edit) {
                return false;
            }
            end_name_edit(true);
            true
        }
        VK_ESCAPE if !ctrl && !alt && !shift => {
            if ime_composing(edit) {
                return false;
            }
            end_name_edit(false);
            true
        }
        _ => false,
    }
}

unsafe fn ensure_name_edit() -> HWND {
    let existing = with_st(|s| s.name_edit);
    if !existing.0.is_null() && IsWindow(Some(existing)).as_bool() {
        return existing;
    }
    let (hwnd, bold) = with_st(|s| (s.hwnd, s.bold));
    if hwnd.0.is_null() {
        return HWND::default();
    }
    let edit = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("EDIT"),
        w!(""),
        WS_CHILD | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        0,
        0,
        0,
        0,
        Some(hwnd),
        Some(HMENU(ID_NAME as isize as _)),
        None,
        None,
    )
    .unwrap_or_default();
    with_st(|s| s.name_edit = edit);
    if !edit.0.is_null() {
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(bold.0 as usize)), Some(LPARAM(1)));
        _ = SetWindowSubclass(edit, Some(edit_proc), 2, 0);
        let dark = DARK_ON.load(Ordering::Relaxed);
        let theme = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
        _ = SetWindowTheme(edit, theme, PCWSTR::null());
    }
    edit
}

unsafe fn place_name_edit() {
    let (edit, tab, list, scroll, rows, shown, dpi) = with_st(|s| (s.name_edit, s.renaming, s.list, s.scroll, s.rows.clone(), s.shown.clone(), s.dpi));
    if edit.0.is_null() {
        return;
    }
    let Some(tab) = tab else {
        if IsWindowVisible(edit).as_bool() {
            _ = ShowWindow(edit, SW_HIDE);
        }
        return;
    };
    let Some(slot) = rows.iter().find(|r| r.tab == tab) else {
        if IsWindowVisible(edit).as_bool() {
            _ = ShowWindow(edit, SW_HIDE);
        }
        return;
    };
    let color = shown.iter().find(|r| r.index == tab).is_some_and(|r| r.color.is_some());
    let row = RECT { left: list.left, top: list.top + slot.y - scroll, right: list.right, bottom: list.top + slot.y - scroll + slot.h };
    let field = name_field(row, color, dpi);
    let pad = sc(dpi, crate::settings::FIELD_INSET_DIP);
    let inner = RECT {
        left: field.left + pad,
        top: field.top + pad,
        right: (field.right - pad).max(field.left + pad + 1),
        bottom: (field.bottom - pad).max(field.top + pad + 1),
    };
    place_edit(edit, inner, true);
}

unsafe fn begin_name_edit() {
    if name_editing() {
        let edit = with_st(|s| s.name_edit);
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        return;
    }
    if with_st(|s| s.renaming.is_some()) {
        end_name_edit(true);
    }
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    layout(h);
    let sel = with_st(|s| s.sel);
    let label = with_st(|s| s.shown.iter().find(|r| r.index == sel).map(|r| r.name.clone()));
    let Some(label) = label else { return };
    let Some(prev) = app(|a| a.s.tabs.get(sel).map(|t| t.name.clone())) else { return };
    let edit = ensure_name_edit();
    if edit.0.is_null() {
        return;
    }
    hide_chip_tip();
    with_st(|s| {
        s.renaming = Some(sel);
        s.rename_prev = prev;
        s.rename_shown = label.clone();
        s.rename_skip = false;
    });
    let bold = with_st(|s| s.bold);
    SendMessageW(edit, WM_SETFONT, Some(WPARAM(bold.0 as usize)), Some(LPARAM(1)));
    _ = SetWindowTextW(edit, &HSTRING::from(label.as_str()));
    layout(h);
    _ = SetFocus(Some(edit));
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn end_name_edit(save: bool) {
    let (edit, which, prev, shown) = with_st(|s| {
        s.rename_skip = !save;
        let which = s.renaming.take();
        let prev = std::mem::take(&mut s.rename_prev);
        let shown = std::mem::take(&mut s.rename_shown);
        (s.name_edit, which, prev, shown)
    });
    let Some(i) = which else {
        with_st(|s| s.rename_skip = false);
        return;
    };
    let typed = if save && !edit.0.is_null() { Some(text_of(edit)) } else { None };
    if !edit.0.is_null() && IsWindowVisible(edit).as_bool() {
        _ = ShowWindow(edit, SW_HIDE);
    }
    if let Some(typed) = typed {
        if inline_rename_result(&shown, &typed).is_some() {
            apply_tab_rename(i, &prev, &typed);
        }
    }
    let (h, filter) = with_st(|s| {
        s.rename_skip = false;
        (s.hwnd, s.edit)
    });
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    // A click in another window already moved focus. Put it back on the filter only when it is still ours.
    let focus = GetFocus();
    let ours = focus.0.is_null() || focus == h || focus == edit || IsChild(h, focus).as_bool();
    if ours && !filter.0.is_null() && focus != filter {
        _ = SetFocus(Some(filter));
    }
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_label_is_columns_by_rows_without_spaces() {
        assert_eq!(grid_label(&[2, 2]), "2×2");
        assert_eq!(grid_label(&[2]), "1×2");
        assert_eq!(grid_label(&[1, 1]), "2×1");
        assert_eq!(grid_label(&[1, 1, 1]), "3×1");
        assert_eq!(grid_label(&[1]), "1×1");
        assert_eq!(grid_label(&[3]), "1×3");
        assert_eq!(grid_label(&[3, 3]), "2×3");
        assert_eq!(grid_label(&[2, 2, 2]), "3×2");
        assert_eq!(grid_label(&[1, 3]), "1 | 3");
        assert_eq!(grid_label(&[]), "");
    }

    #[test]
    fn filter_matches_name_folder_and_full_path() {
        let chips = [("docs", r"C:\Work\docs"), ("src", r"D:\Maguro\src")];
        assert!(row_matches("Project", &chips, ""));
        assert!(row_matches("Project", &chips, "proj"));
        assert!(row_matches("Project", &chips, "DOCS"));
        assert!(row_matches("Project", &chips, r"d:\maguro"));
        assert!(!row_matches("Project", &chips, "missing"));
    }

    #[test]
    fn chips_wrap_when_the_next_one_does_not_fit() {
        let placed = place_chips(&[40, 40, 40], 100, 16, 3);
        assert_eq!(placed.len(), 3);
        assert_eq!((placed[0].left, placed[0].top), (0, 0));
        assert_eq!((placed[1].left, placed[1].top), (43, 0));
        assert_eq!((placed[2].left, placed[2].top), (0, 19));
        assert_eq!(placed[2].bottom - placed[2].top, 16);
    }

    #[test]
    fn footer_offers_f2_to_rename() {
        let s = tr(Key::TabListKeys);
        assert!(s.contains("{F2}"), "{s}");
        assert!(s.contains("rename"), "{s}");
        assert!(kbd_segs(&s).iter().any(|seg| matches!(seg, KbdSeg::Key(k) if k == "F2")));
    }

    fn ini_value(text: &str, key: &str) -> String {
        text.lines()
            .find_map(|line| {
                let rest = line.trim().strip_prefix(key)?.trim_start();
                Some(rest.strip_prefix('=')?.trim().to_string())
            })
            .unwrap_or_default()
    }

    #[test]
    fn footer_names_f2_in_korean_and_japanese() {
        let ko = ini_value(include_str!("../lang/lang.ko.ini"), "tab_list_keys");
        let ja = ini_value(include_str!("../lang/lang.ja.ini"), "tab_list_keys");
        assert!(ko.contains("{F2}"), "{ko}");
        assert!(ko.contains("이름 바꾸기"), "{ko}");
        assert!(ja.contains("{F2}"), "{ja}");
        assert!(ja.contains("名前の変更"), "{ja}");
        for line in [&ko, &ja] {
            assert!(kbd_segs(line).iter().any(|seg| matches!(seg, KbdSeg::Key(k) if k == "F2")), "{line}");
        }
    }

    #[test]
    fn footer_shrinks_the_count_before_clipping_keys() {
        // Default client is 560 DIP; the status bar is that minus 8 DIP of padding on each side.
        // The count fits in what the keys leave, so nothing wraps and nothing is ellipsized.
        let plan = plan_footer(544, 192, &[70, 80, 70, 64], 8);
        assert_eq!(plan.rows, 1);
        assert!(plan.count_on_keys);
        assert_eq!(plan.count_w, 192);
        assert_eq!(plan.key_rows[0].width, 284);
        assert_eq!(plan.key_rows[0].end, 4);

        // A long count gives up space. The three key pieces stay on the one row.
        let tight = plan_footer(360, 200, &[100, 100, 100], 8);
        assert_eq!(tight.rows, 1);
        assert!(tight.count_on_keys);
        assert_eq!(tight.count_w, 52);
        assert_eq!(tight.key_rows[0].width, 300);
    }

    #[test]
    fn footer_wraps_keys_instead_of_clipping_them() {
        let plan = plan_footer(150, 80, &[100, 100], 8);
        assert!(!plan.count_on_keys);
        assert!(plan.rows >= 2);
        let placed: Vec<usize> = plan.key_rows.iter().flat_map(|row| row.start..row.end).collect();
        assert_eq!(placed, vec![0, 1]);
        assert!(plan.key_rows.iter().all(|row| row.width <= 150));

        let wide = wrap_runs(&[200, 40], 100);
        assert_eq!(wide.len(), 2);
        assert_eq!(wide[0].width, 200);
        assert_eq!((wide[1].start, wide[1].end), (1, 2));
    }

    #[test]
    fn rename_field_stays_one_line_on_a_tall_row() {
        let row = RECT { left: 0, top: 40, right: 500, bottom: 160 };
        let field = name_field(row, false, 96);
        assert_eq!(field.bottom - field.top, 22);
        assert_eq!(field.top, 40 + (120 - 22) / 2);
        let short = name_field(RECT { left: 0, top: 0, right: 200, bottom: 24 }, true, 96);
        assert_eq!(short.bottom - short.top, 22);
        assert!(short.left > name_text_rect(RECT { left: 0, top: 0, right: 200, bottom: 24 }, false, 96).left);
    }

    #[test]
    fn inline_rename_leaves_an_unedited_label_and_clears_when_emptied() {
        assert_eq!(inline_rename_result("docs", "docs"), None);
        assert_eq!(inline_rename_result("docs", ""), Some(String::new()));
        assert_eq!(inline_rename_result("Work", "Plan"), Some("Plan".into()));
        assert_eq!(inline_rename_result("Work", "  Work  "), Some("Work".into()));
        assert_eq!(inline_rename_result("docs", "1"), Some(String::new()));
        assert_eq!(inline_rename_result("docs", "   "), Some(String::new()));
    }

    #[test]
    fn status_line_splits_key_chips() {
        let segs = kbd_segs("Double-click / {Enter} go · {Del} close");
        assert_eq!(
            segs,
            vec![
                KbdSeg::Text("Double-click / ".into()),
                KbdSeg::Key("Enter".into()),
                KbdSeg::Text(" go · ".into()),
                KbdSeg::Key("Del".into()),
                KbdSeg::Text(" close".into()),
            ]
        );
    }

    #[test]
    fn chip_tip_is_the_full_path_and_this_pc_is_substituted() {
        assert_eq!(chip_name(r"C:\Work\docs\"), "docs");
        assert_eq!(chip_tip(r"C:\Work\docs\"), r"C:\Work\docs\");
        assert!(chip_tip("").is_empty());
        let pc = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
        assert_eq!(chip_name(pc), "This PC");
        assert_eq!(chip_tip(pc), "This PC");
        assert!(!chip_tip(pc).contains("20D04FE0"));
    }
}
