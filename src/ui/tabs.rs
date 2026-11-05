use crate::imports::*;
use super::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;


/// Widest a tab grows, at 96 DPI. A longer title is drawn with `DT_END_ELLIPSIS`.
const TAB_MAX_W: i32 = 135;
/// Tracking tooltip for the paths under one tab. The lock glyph uses id 7.
const TAB_TIP_ID: usize = 8;
const ICON_LIST: &str = "\u{E8FD}";

// Segoe Fluent Icons / MDL2 Assets code points
pub(crate) const ICON_SEARCH: &str = "\u{E721}";
pub(crate) const ICON_FILTER: &str = "\u{E71C}";
pub(crate) const ICON_LOCK: &str = "\u{E72E}";
pub(crate) const ICON_CLOSE: &str = "\u{E711}";
pub(crate) const ICON_PLUS: &str = "\u{E710}";
pub(crate) const ICON_CHEVRON_DOWN: &str = "\u{E70D}";
/// Tab stripe colours, 0xRRGGBB. Chosen so a thin bar and a light tint read on both themes.
pub(crate) const TAB_COLORS: [(u32, crate::Key); 8] = [
    (0xE15A4A, crate::Key::ColorRed),
    (0xE8943A, crate::Key::ColorOrange),
    (0xC49212, crate::Key::ColorYellow),
    (0x3EAE6E, crate::Key::ColorGreen),
    (0x2A9D9A, crate::Key::ColorTeal),
    (0x3D7EE0, crate::Key::ColorBlue),
    (0x8A5CD0, crate::Key::ColorPurple),
    (0xD45A8C, crate::Key::ColorPink),
];
pub(crate) const CLOSED_KEEP: usize = 10;
pub(crate) const ID_TAB_EDIT: i32 = 208;

pub(crate) fn glyph(id: i32) -> &'static str {
    match id {
        ID_BACK => "\u{E72B}",
        ID_FWD => "\u{E72A}",
        ID_UP => "\u{E74A}",
        ID_OPEN_IN => "\u{E8AB}",
        ID_PANE_ADD => ICON_PLUS,
        ID_TB_LAYOUT => "\u{E8A9}",
        // Tab-row save/load are drawn in sicon.rs. These points remain for glyph().
        ID_TB_SAVE => "\u{E74E}",
        ID_TB_LOAD => "\u{E838}",
        ID_TB_SET => "\u{E713}",
        ID_TB_LIST => ICON_LIST,
        _ => "\u{E890}",
    }
}

// ---------- tabs ----------
// The strip is its own child so splitter hit-testing stays below it. Every hidden tab keeps its
// grid on the session. Only the most recently hidden tab also keeps its Explorer windows.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TabHit {
    Tab(usize),
    Close(usize),
    /// The close slot of a locked tab. It does not close.
    Lock(usize),
    Plus,
    /// The ▾ that lists every tab when they no longer fit.
    Overflow,
    Empty,
}

pub(crate) struct TabLay {
    pub(crate) body: Vec<RECT>,
    pub(crate) close: Vec<Option<RECT>>,
    pub(crate) lock: Vec<bool>,
    pub(crate) plus: RECT,
    /// Set when the tabs do not fit at `min_w`. Sits at the right of the tab area, left of the icon group.
    pub(crate) overflow: Option<RECT>,
    /// First tab that has a rect. Clamped so the window never runs past the last tab.
    pub(crate) first: usize,
    pub(crate) count: usize,
}

impl TabLay {
    /// Tab under a drag, among the ones on screen. Past the last visible tab, that tab.
    pub(crate) fn drag_target(&self, x: i32) -> usize {
        tab_drag_index(&self.body, x)
    }

    /// Where a tab dragged in from another window is inserted.
    pub(crate) fn insert_at(&self, x: i32) -> usize {
        self.body
            .iter()
            .enumerate()
            .find(|(_, r)| r.right > r.left && x < (r.left + r.right) / 2)
            .map(|(i, _)| i)
            .unwrap_or_else(|| self.body.iter().enumerate().rev().find(|(_, r)| r.right > r.left).map(|(i, _)| i + 1).unwrap_or(self.body.len()))
    }
}

/// Index a horizontal drag lands on. The first body whose midpoint is to the right of `x`, else the last one with a width.
pub(crate) fn tab_drag_index(body: &[RECT], x: i32) -> usize {
    body.iter()
        .enumerate()
        .find(|(_, r)| r.right > r.left && x < (r.left + r.right) / 2)
        .map(|(i, _)| i)
        .unwrap_or_else(|| body.iter().enumerate().rev().find(|(_, r)| r.right > r.left).map(|(i, _)| i).unwrap_or(0))
}

/// X uses the accent while the pointer is on it. A lock stays the quiet colour, hot or not.
pub(crate) fn close_ink(locked: bool, hot: bool, accent: u32, sub: u32) -> u32 {
    if locked || !hot { sub } else { accent }
}

pub(crate) fn close_glyph(locked: bool) -> &'static str {
    if locked { ICON_LOCK } else { ICON_CLOSE }
}

/// Middle-click closes an unlocked tab. A lock, and the body of a locked tab, do not.
pub(crate) fn middle_closes(lay: &TabLay, hit: TabHit) -> bool {
    match hit {
        TabHit::Close(i) | TabHit::Tab(i) => !lay.lock.get(i).copied().unwrap_or(false),
        _ => false,
    }
}

/// Right-hand group on the tab row. Drive letters are a prefix of this, left of Layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct IconLay {
    /// Width the tabs may use. A gap sits between this and the group.
    pub(crate) tab_w: i32,
    pub(crate) icons: [(i32, RECT); 5],
    /// Drive buttons that fit, left to right. A prefix of the A→Z list; the rest are hidden.
    pub(crate) drives: [RECT; 26],
    pub(crate) drive_n: usize,
    /// Tab-list button, immediately left of the drive letters or the icon group.
    /// Same size and vertical span as the glyph toolbar buttons (gear, star, save, load).
    pub(crate) list: RECT,
}

/// pad, gap, plus, max tab, min tab, overflow chevron. Shared with the drive-button reserve.
/// The chevron width matches the +, so both buttons are the same square.
fn tab_px(dpi: i32) -> (i32, i32, i32, i32, i32, i32) {
    let s = |v: i32| v * dpi / 96;
    let plus = s(28);
    (s(6), s(2), plus, s(TAB_MAX_W), s(44), plus)
}

/// Width `tab_lay` needs for one minimum tab, the +, and the ▾. Drive buttons stop before this.
fn tab_reserve(dpi: i32) -> i32 {
    let (pad, gap, plus_w, _, min_w, over_w) = tab_px(dpi);
    pad + min_w + gap + plus_w + gap + over_w + pad
}

/// How many drive buttons the tab row reserves. Off reserves none, so the tabs take that width.
pub(crate) fn drive_slots(show: bool, n: usize) -> usize {
    if show { n } else { 0 }
}

/// Glyph button on the main-window tool row, width in DIP.
/// Save, load, the star, the gear, drive letters, and the tab list share it.
/// The layout button is wider because it also shows the grid. Height is [`BTN_H_TOOL`].
pub(crate) const TOOL_ICON_W: i32 = 32;

pub(crate) fn tool_icon_px(dpi: i32) -> (i32, i32) {
    let s = |v: i32| v * dpi / 96;
    (s(TOOL_ICON_W), s(BTN_H_TOOL))
}

/// Chrome ids [`draw_button`] paints as one MDL2 glyph on the shared icon face.
/// The tab-list button is this kind, same as the gear: same hover, press, ink, and glyph font.
pub(crate) fn glyph_tool_button(id: i32) -> bool {
    matches!(id, ID_TB_SET | ID_TB_LIST)
}

fn icon_widths(dpi: i32) -> [i32; 5] {
    let s = |v: i32| v * dpi / 96;
    let icon = tool_icon_px(dpi).0;
    [s(70), icon, icon, icon, icon]
}

/// The gear stays when the toolbar is hidden. It is the mouse path to settings.
fn icon_on(bar: bool) -> [bool; 5] {
    if bar { [true; 5] } else { [false, false, false, false, true] }
}

/// Narrowest tab row that still keeps one minimum tab clear of the icon group.
/// Drive letters are not part of this: they hide before the group does.
pub(crate) fn icon_group_min(dpi: i32, bar: bool) -> i32 {
    let s = |v: i32| v * dpi / 96;
    let (pad, gap) = (s(8), s(4));
    let on = icon_on(bar);
    let n_on = on.iter().filter(|v| **v).count() as i32;
    let icons_w = icon_widths(dpi).iter().zip(&on).filter(|(_, v)| **v).map(|(w, _)| *w).sum::<i32>() + gap * (n_on - 1).max(0);
    let list_w = tool_icon_px(dpi).0;
    tab_reserve(dpi) + gap + list_w + gap + icons_w + pad
}

/// Layout, save, load, the favourites star, the gear, and `drives` letter buttons, right-aligned on the tab row.
/// Letters sit to the left of Layout. The last ones drop when the row cannot also keep `tab_reserve`.
/// `drives == 0` (see `drive_slots`) hides every letter and gives that width to the tabs.
/// `bar == false` keeps the gear and the tab-list button. The other icons and the letters are not placed, and the tabs use that width.
/// The star is the icon immediately left of the gear.
pub(crate) fn icon_slots(row_w: i32, top: i32, dpi: i32, drives: usize, bar: bool) -> IconLay {
    let s = |v: i32| v * dpi / 96;
    let (icon_w, bh) = tool_icon_px(dpi);
    let (pad, gap) = (s(8), s(4));
    let specs = [ID_TB_LAYOUT, ID_TB_SAVE, ID_TB_LOAD, ID_TB_FAV, ID_TB_SET];
    let widths = icon_widths(dpi);
    // The gear is the mouse path to settings, so a hidden toolbar still reserves its slot.
    let on = icon_on(bar);
    let n_on = on.iter().filter(|v| **v).count() as i32;
    let icons_w = widths.iter().zip(&on).filter(|(_, v)| **v).map(|(w, _)| *w).sum::<i32>() + gap * (n_on - 1).max(0);
    let icons_left = (row_w - pad - icons_w).max(0);
    let dw = icon_w;
    let stride = (dw + gap).max(1);
    // Same width and height as the gear, on the same top the caller used for that row.
    let list_w = icon_w;
    let list_h = bh;
    // The first drive must start far enough right that one minimum tab and the list button still fit.
    let min_left = tab_reserve(dpi) + gap + list_w + gap;
    let room = icons_left - min_left;
    let fit = if !bar || room < stride { 0 } else { (room / stride) as usize };
    let drive_n = drives.min(26).min(fit);
    let mut drive_rects = [RECT::default(); 26];
    let mut x = icons_left - drive_n as i32 * stride;
    let anchor = x;
    for i in 0..drive_n {
        drive_rects[i] = RECT { left: x, top, right: x + dw, bottom: top + bh };
        x += stride;
    }
    let mut icons = [(0, RECT::default()); 5];
    for i in 0..specs.len() {
        if !on[i] {
            icons[i] = (specs[i], RECT { left: x, top, right: x, bottom: top });
            continue;
        }
        let w = widths[i];
        icons[i] = (specs[i], RECT { left: x, top, right: x + w, bottom: top + bh });
        x += w + gap;
    }
    let list_right = (anchor - gap).max(0);
    let list_left = (list_right - list_w).max(0);
    let list = if list_right > list_left {
        RECT { left: list_left, top, right: list_right, bottom: top + list_h }
    } else {
        RECT { left: 0, top, right: 0, bottom: top }
    };
    let tab_w = if list.right > list.left { (list.left - gap).max(1) } else { (anchor - gap).max(1) };
    IconLay { tab_w, icons, drives: drive_rects, drive_n, list }
}

/// `scroll` moved so `index` is inside a window of `shown` tabs. Pure.
pub(crate) fn scroll_to_show(scroll: usize, index: usize, shown: usize, n: usize) -> usize {
    let n = n.max(1);
    let shown = shown.clamp(1, n);
    let index = index.min(n - 1);
    let max_first = n - shown;
    let scroll = scroll.min(max_first);
    if index < scroll {
        index
    } else if index >= scroll + shown {
        index + 1 - shown
    } else {
        scroll
    }
}

/// Width the tabs may use. The icon group and the drive buttons that fit keep the rest while the bar is on.
/// Hidden drive buttons (`drives == 0`) reserve nothing, so the tabs take that width.
/// A hidden bar still reserves the gear. The other icons are not on screen, and the tabs use that width.
pub(crate) fn tab_area_width(client_w: i32, dpi: i32, bar: bool, drives: usize) -> i32 {
    icon_slots(client_w.max(1), 0, dpi, drives, bar).tab_w
}

pub(crate) fn tab_lay(width: i32, n: usize, locked: &[bool], dpi: i32, scroll: usize) -> TabLay {
    let s = |v: i32| v * dpi / 96;
    let n = n.max(1);
    let width = width.max(1);
    let (pad, gap, plus_w, max_w, min_w, over_w) = tab_px(dpi);
    let y = s(4);
    let h = s(26);
    let used = |w: i32| n as i32 * w + gap * (n as i32 - 1) + plus_w + pad * 2;
    let slot = |r: RECT| {
        let cw = s(16);
        RECT {
            left: r.right - s(4) - cw,
            top: r.top + (h - cw) / 2,
            right: r.right - s(4),
            bottom: r.top + (h - cw) / 2 + cw,
        }
    };
    let mut body = vec![RECT::default(); n];
    let mut close = vec![None; n];
    let lock: Vec<bool> = (0..n).map(|i| locked.get(i).copied().unwrap_or(false)).collect();
    if used(min_w) <= width {
        let mut w = max_w;
        if used(w) > width {
            w = ((width - pad * 2 - plus_w - gap * (n as i32 - 1)) / n as i32).max(min_w);
        }
        let mut x = pad;
        for i in 0..n {
            let r = RECT { left: x, top: y, right: x + w, bottom: y + h };
            close[i] = Some(slot(r));
            body[i] = r;
            x += w + gap;
        }
        return TabLay {
            body,
            close,
            lock,
            plus: RECT { left: x, top: y, right: x + plus_w, bottom: y + h },
            overflow: None,
            first: 0,
            count: n,
        };
    }
    // Chevron at the right. The + stays pinned just to its left, still plus_w wide.
    let over_right = (width - pad).max(over_w.min(width));
    let over_left = (over_right - over_w).max(0);
    let over = RECT { left: over_left, top: y, right: over_right, bottom: y + h };
    let plus_right = (over.left - gap).max(plus_w.min(width));
    let plus_left = (plus_right - plus_w).max(0);
    let plus = RECT { left: plus_left, top: y, right: plus_right, bottom: y + h };
    let avail = (plus.left - gap - pad).max(0);
    let stride = (min_w + gap).max(1);
    let mut count = ((avail + gap) / stride).max(1) as usize;
    count = count.min(n);
    let max_first = n - count;
    let first = scroll.min(max_first);
    let mut x = pad;
    for i in first..first + count {
        let r = RECT { left: x, top: y, right: x + min_w, bottom: y + h };
        close[i] = Some(slot(r));
        body[i] = r;
        x += min_w + gap;
    }
    TabLay { body, close, lock, plus, overflow: Some(over), first, count }
}

pub(crate) fn in_rect(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

pub(crate) fn tab_hit(lay: &TabLay, x: i32, y: i32) -> TabHit {
    if in_rect(lay.plus, x, y) {
        return TabHit::Plus;
    }
    if lay.overflow.is_some_and(|r| in_rect(r, x, y)) {
        return TabHit::Overflow;
    }
    for (i, r) in lay.body.iter().enumerate() {
        if r.right <= r.left {
            continue;
        }
        if in_rect(*r, x, y) {
            if lay.close[i].is_some_and(|c| in_rect(c, x, y)) {
                return if lay.lock.get(i).copied().unwrap_or(false) { TabHit::Lock(i) } else { TabHit::Close(i) };
            }
            return TabHit::Tab(i);
        }
    }
    TabHit::Empty
}

pub(crate) fn put_filters(f: &[String]) {
    if let Ok(mut g) = FILTERS.lock() {
        for i in 0..MAXP {
            g[i] = f.get(i).cloned().unwrap_or_default();
        }
    }
    unsafe { sync_filter_hwnds() };
}

pub(crate) fn fit_park(a: &mut App) {
    let n = a.s.tabs.len();
    if a.park.len() < n {
        a.park.resize_with(n, no_panes);
    } else if a.park.len() > n {
        a.park.truncate(n);
    }
    let npark = a.park.len();
    a.park_lru.retain(|&i| i < npark);
}

pub(crate) fn lru_note(a: &mut App, i: usize) {
    a.park_lru.retain(|&x| x != i);
    a.park_lru.push(i);
}

pub(crate) fn lru_forget(a: &mut App, i: usize) {
    a.park_lru.retain(|&x| x != i);
}

pub(crate) fn lru_insert_shift(a: &mut App, at: usize) {
    for x in &mut a.park_lru {
        if *x >= at {
            *x += 1;
        }
    }
}

pub(crate) fn lru_remove_shift(a: &mut App, at: usize) {
    a.park_lru.retain(|&x| x != at);
    for x in &mut a.park_lru {
        if *x > at {
            *x -= 1;
        }
    }
}

pub(crate) fn park_has_browsers(set: &[Option<Pane>]) -> bool {
    set.iter().any(|p| p.is_some())
}

/// Drop hidden Explorer windows past PARK_CACHE. Their folders, columns and filters stay on the tab.
pub(crate) unsafe fn evict_park() {
    loop {
        let victim = app(|a| {
            let live = a.park.iter().filter(|s| park_has_browsers(s)).count();
            if live <= PARK_CACHE {
                return None;
            }
            a.park_lru.iter().copied().find(|&i| a.park.get(i).is_some_and(|s| park_has_browsers(s))).or_else(|| {
                a.park.iter().position(|s| park_has_browsers(s))
            })
        });
        let Some(i) = victim else { break };
        let set = app(|a| {
            lru_forget(a, i);
            if i < a.park.len() {
                std::mem::replace(&mut a.park[i], no_panes())
            } else {
                no_panes()
            }
        });
        destroy_set(set);
    }
}

pub(crate) unsafe fn pull_filters() -> Vec<String> {
    let (n, stored) = app(|a| {
        let stored = a.s.tabs.get(a.s.tab).map(|t| t.filters.clone()).unwrap_or_default();
        (a.s.panes(), stored)
    });
    let mut f = pad_filters(stored);
    for i in 0..n {
        if let Some(e) = pane_ctl(i, ID_FILTER) {
            f[i] = filter_spec(&text_of(e));
        }
    }
    put_filters(&f);
    f
}

/// Fold the live grid and filters into the active tab so a switch or a save keeps them.
pub(crate) unsafe fn remember_tabs() {
    let n = app(|a| a.s.panes());
    for i in 0..n {
        save_view(i);
    }
    let filters = pull_filters();
    app(|a| {
        let tab = a.s.tab;
        if tab < a.s.tabs.len() {
            a.s.tabs[tab].filters = pad_filters(filters);
        }
        a.s.anchor();
    });
}

pub(crate) unsafe fn detach_live() {
    hide_addr_tip();
    for i in 0..MAXP {
        let got = app(|a| {
            a.panes[i].as_mut().map(|p| {
                let eb = p.eb.clone();
                let cookie = std::mem::take(&mut p.cookie);
                (eb, cookie, p.hwnd)
            })
        });
        if let Some((eb, cookie, hwnd)) = got {
            if cookie != 0 {
                _ = eb.Unadvise(cookie);
            }
            _ = IUnknown_SetSite(&eb, None);
            _ = KillTimer(Some(hwnd), TIMER_SETTLE);
            _ = KillTimer(Some(hwnd), TIMER_FILTER);
            _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

pub(crate) unsafe fn attach_live() {
    for i in 0..MAXP {
        let eb = app(|a| a.panes[i].as_ref().map(|p| p.eb.clone()));
        let Some(eb) = eb else { continue };
        let slot = app(|a| a.panes[i].as_ref().map(|p| p.slot.clone()));
        let Some(slot) = slot else { continue };
        slot.store(i, Ordering::SeqCst);
        let site: IServiceProvider = Host(slot.clone()).into();
        _ = IUnknown_SetSite(&eb, &site);
        let ev: IExplorerBrowserEvents = Events(slot).into();
        if let Ok(c) = eb.Advise(&ev) {
            app(|a| {
                if let Some(p) = a.panes[i].as_mut() {
                    p.cookie = c;
                }
            });
        }
    }
    // A tab that was hidden through the theme switch still has the old shell view.
    // Headers and scrollbars follow now. The list itself waits for a restart.
    recolor_due_panes();
}

pub(crate) unsafe fn sync_filter_edits() {
    let n = app(|a| a.s.panes());
    for i in 0..n {
        let spec = filter_of(i);
        if let Some(e) = pane_ctl(i, ID_FILTER) {
            if text_of(e) != spec {
                _ = SetWindowTextW(e, &HSTRING::from(spec.as_str()));
            }
        }
    }
}

pub(crate) unsafe fn destroy_set(set: Vec<Option<Pane>>) {
    if set.iter().all(|p| p.is_none()) {
        return;
    }
    let (back, keep) = app(|a| {
        let keep = FILTERS.lock().map(|f| f.clone()).unwrap_or_else(|_| [const { String::new() }; MAXP]);
        (std::mem::replace(&mut a.panes, set), keep)
    });
    // These browsers belong to a tab that is not on screen. Their grid is already on that tab.
    KEEP_VIEWS.with(|c| c.set(true));
    for i in 0..MAXP {
        destroy_pane(i);
    }
    KEEP_VIEWS.with(|c| c.set(false));
    app(|a| a.panes = back);
    put_filters(&keep);
}

pub(crate) unsafe fn destroy_every_pane() {
    for i in 0..MAXP {
        destroy_pane(i);
    }
    let sets = app(|a| a.park.drain(..).collect::<Vec<_>>());
    for set in sets {
        destroy_set(set);
    }
}

pub(crate) fn tabs_invalidate() {
    let h = app(|a| a.tabs_hwnd);
    if !h.0.is_null() {
        unsafe { _ = InvalidateRect(Some(h), None, false) };
    }
    unsafe { crate::tablist::refresh() };
}

pub(crate) unsafe fn switch_tab(j: usize) {
    let (cur, n) = app(|a| (a.s.tab, a.s.tabs.len()));
    if j >= n {
        return;
    }
    if j == cur {
        show_active_tab();
        tabs_invalidate();
        return;
    }
    end_rename(true);
    remember_tabs();
    detach_live();
    let filters = app(|a| {
        fit_park(a);
        let live = std::mem::replace(&mut a.panes, no_panes());
        let tab = a.s.tab;
        a.park[tab] = live;
        lru_note(a, tab);
        a.panes = std::mem::take(&mut a.park[j]);
        if a.panes.len() != MAXP {
            a.panes = no_panes();
        }
        lru_forget(a, j);
        let old = a.s.tab;
        stash_tab_order(a, old);
        a.s.tab = j;
        a.s.publish();
        restore_tab_order(a, j);
        let _ = pull_dirty(a);
        a.zoom = None;
        a.s.tabs[j].filters.clone()
    });
    put_filters(&filters);
    attach_live();
    show_active_tab();
    relayout();
    sync_filter_edits();
    redraw_all();
    focus_view(active_pane());
    tabs_invalidate();
    evict_park();
    note_save();
    refresh_save_button();
}

pub(crate) unsafe fn switch_rel(d: i32) {
    let (i, n) = app(|a| (a.s.tab as i32, a.s.tabs.len() as i32));
    if n <= 0 {
        return;
    }
    switch_tab((i + d).rem_euclid(n) as usize);
}

pub(crate) unsafe fn new_tab() {
    new_tab_at(None);
}

/// A new window tab. `path` is pane 0's folder. `None` leaves every path empty, as `new_tab` does.
pub(crate) unsafe fn new_tab_at(path: Option<&str>) {
    end_rename(true);
    remember_tabs();
    detach_live();
    let first = path.unwrap_or("").to_string();
    app(|a| {
        fit_park(a);
        let live = std::mem::replace(&mut a.panes, no_panes());
        let tab = a.s.tab;
        a.park[tab] = live;
        lru_note(a, tab);
        // Settings pane count, with even splits. Not the tab on screen.
        // Paths stay empty unless the caller named a folder for the first pane.
        // Pane colours start with no label.
        let fresh = session_for_new_tab(&a.s, &first);
        let old = a.s.tab;
        stash_tab_order(a, old);
        let name = next_tab_name(&a.s.tabs);
        let tab = Tab::from_live(&fresh, name, false, vec![String::new(); MAXP]);
        a.s.tabs.push(tab.clone());
        a.park.push(no_panes());
        let at = a.s.tabs.len() - 1;
        insert_tab_base(a, at, &tab);
        a.s.tab = a.s.tabs.len() - 1;
        a.s.publish();
        a.s.order.clear();
        let _ = pull_dirty(a);
        a.zoom = None;
    });
    put_filters(&vec![String::new(); MAXP]);
    attach_live();
    show_active_tab();
    relayout();
    redraw_all();
    focus_view(active_pane());
    tabs_invalidate();
    evict_park();
    note_save();
    refresh_save_button();
}

pub(crate) unsafe fn duplicate_tab(i: usize) {
    end_rename(true);
    remember_tabs();
    let j = app(|a| {
        if i >= a.s.tabs.len() {
            return None;
        }
        fit_park(a);
        let Some(t) = duplicated_tab(&a.s, i) else {
            return None;
        };
        let j = (i + 1).min(a.s.tabs.len());
        a.s.tabs.insert(j, t.clone());
        a.park.insert(j, no_panes());
        insert_tab_base(a, j, &t);
        lru_insert_shift(a, j);
        if j <= a.s.tab {
            a.s.tab += 1;
        }
        Some(j)
    });
    if let Some(j) = j {
        switch_tab(j);
    }
}

/// Insert `tabs` immediately after the tab on screen and show the first of them.
/// Tabs that were already open are not changed.
pub(crate) unsafe fn open_tabs_beside(tabs: Vec<Tab>) {
    if tabs.is_empty() {
        return;
    }
    end_rename(true);
    remember_tabs();
    let at = app(|a| {
        fit_park(a);
        let at = if a.s.tabs.is_empty() { 0 } else { (a.s.tab + 1).min(a.s.tabs.len()) };
        for (k, t) in tabs.iter().enumerate() {
            let j = at + k;
            a.s.tabs.insert(j, t.clone());
            a.park.insert(j, no_panes());
            insert_tab_base(a, j, t);
            lru_insert_shift(a, j);
        }
        at
    });
    if at == app(|a| a.s.tab) {
        let filters = app(|a| {
            let i = at.min(a.s.tabs.len().saturating_sub(1));
            a.s.tab = i;
            a.s.publish();
            a.s.order.clear();
            mark_tab_clean(a, i);
            let _ = pull_dirty(a);
            a.s.tabs[i].filters.clone()
        });
        put_filters(&filters);
        show_active_tab();
        relayout();
        sync_filter_edits();
        redraw_all();
        focus_view(active_pane());
        tabs_invalidate();
        note_save();
        refresh_save_button();
    } else {
        switch_tab(at);
    }
}

pub(crate) unsafe fn close_tab(i: usize) {
    end_rename(true);
    if !app(|a| can_close_tab(&a.s.tabs, i)) {
        return;
    }
    let active = app(|a| a.s.tab);
    if i == active {
        remember_tabs();
        detach_live();
    }
    let (doomed, filters, switched) = app(|a| {
        fit_park(a);
        let tab = a.s.tabs.remove(i);
        let mut parked = a.park.remove(i);
        remove_tab_base(a, i);
        lru_remove_shift(a, i);
        a.closed.insert(0, tab);
        a.closed.truncate(CLOSED_KEEP);
        let was = a.s.tab;
        let switched = i == was;
        if switched {
            let j = i.min(a.s.tabs.len() - 1);
            a.s.tab = j;
            let next = std::mem::replace(&mut a.park[j], no_panes());
            lru_forget(a, j);
            parked = std::mem::replace(&mut a.panes, next);
            a.s.publish();
            restore_tab_order(a, j);
            let _ = pull_dirty(a);
        } else if i < was {
            a.s.tab = was - 1;
        }
        let filters = a.s.tabs[a.s.tab].filters.clone();
        (parked, filters, switched)
    });
    destroy_set(doomed);
    show_active_tab();
    if switched {
        put_filters(&filters);
        attach_live();
        relayout();
        sync_filter_edits();
        redraw_all();
        focus_view(active_pane());
    }
    tabs_invalidate();
    note_save();
    refresh_save_button();
}

pub(crate) unsafe fn close_other_tabs(keep: usize) {
    let victims = app(|a| close_other_indexes(&a.s.tabs, keep));
    for i in victims.into_iter().rev() {
        close_tab(i);
    }
}

/// Close every unlocked tab. Locked tabs stay. If that would leave none, open one fresh tab and close the rest.
pub(crate) unsafe fn close_all_tabs() {
    if app(|a| close_all_needs_fresh(&a.s.tabs)) {
        new_tab();
        let keep = app(|a| a.s.tabs.len().saturating_sub(1));
        close_other_tabs(keep);
        return;
    }
    let victims = app(|a| close_all_indexes(&a.s.tabs));
    for i in victims.into_iter().rev() {
        close_tab(i);
    }
}

pub(crate) unsafe fn reopen_closed_at(k: usize) {
    end_rename(true);
    let j = app(|a| {
        if k >= a.closed.len() {
            return None;
        }
        let t = a.closed.remove(k);
        fit_park(a);
        let j = (a.s.tab + 1).min(a.s.tabs.len());
        a.s.tabs.insert(j, t.clone());
        a.park.insert(j, no_panes());
        insert_tab_base(a, j, &t);
        lru_insert_shift(a, j);
        Some(j)
    });
    if let Some(j) = j {
        switch_tab(j);
    }
}

pub(crate) unsafe fn reorder_tab(from: usize, to: usize) {
    if from == to {
        return;
    }
    app(|a| {
        if from >= a.s.tabs.len() || to >= a.s.tabs.len() {
            return;
        }
        fit_park(a);
        reorder_vec(&mut a.s.tabs, from, to);
        reorder_vec(&mut a.park, from, to);
        reorder_vec(&mut a.tab_base, from, to);
        a.park_lru = a.park_lru.iter().copied().map(|i| moved_active(i, from, to)).collect();
        a.s.tab = moved_active(a.s.tab, from, to);
    });
    tabs_invalidate();
    note_save();
}

pub(crate) unsafe fn end_rename(save: bool) {
    let (edit, which) = app(|a| {
        a.tab_rename_skip = !save;
        (a.tab_edit, a.tab_renaming.take())
    });
    if edit.0.is_null() || which.is_none() {
        return;
    }
    if save {
        if let Some(i) = which {
            let name = text_of(edit);
            app(|a| {
                if let Some(t) = a.s.tabs.get_mut(i) {
                    t.name = stored_tab_name(&name);
                }
            });
        }
    }
    _ = ShowWindow(edit, SW_HIDE);
    tabs_invalidate();
    if save {
        note_save();
    }
}

/// Store a typed tab title the way the rename dialog does.
/// Empty, and a legacy all-digit title, clear the custom name so the tab follows its folder again.
/// The tab is the one still called `prev` when the index has moved. False when nothing changed.
pub(crate) unsafe fn apply_tab_rename(i: usize, prev: &str, typed: &str) -> bool {
    let next = stored_tab_name(typed);
    if stored_tab_name(prev) == next {
        return false;
    }
    // The prompt runs a message loop; tabs may have moved. Rename the tab still named `prev`.
    let done = app(|a| {
        let at = if a.s.tabs.get(i).is_some_and(|t| t.name == prev) {
            Some(i)
        } else {
            let mut m = a.s.tabs.iter().enumerate().filter(|(_, t)| t.name == prev).map(|(k, _)| k);
            m.next().filter(|_| m.next().is_none())
        };
        let Some(t) = at.and_then(|k| a.s.tabs.get_mut(k)) else { return false };
        t.name = next;
        true
    });
    if !done {
        return false;
    }
    tabs_invalidate();
    note_save();
    true
}

pub(crate) unsafe fn begin_rename(i: usize) {
    path_tooltip(None);
    let (owner, prev) = app(|a| {
        let prev = a.s.tabs.get(i).map(|t| t.name.clone()).unwrap_or_default();
        (a.main, prev)
    });
    if owner.0.is_null() || app(|a| i >= a.s.tabs.len()) {
        return;
    }
    // Empty when the tab is automatic, so confirming without typing leaves the folder name in charge.
    let initial = custom_tab_name(&prev).unwrap_or("").to_string();
    let title = tr(crate::Key::RenameTab);
    let Some(text) = crate::settings::ask(owner, title, &initial) else { return };
    let _ = apply_tab_rename(i, &prev, &text);
}

pub(crate) unsafe fn set_tab_color(i: usize, color: Option<u32>) {
    app(|a| {
        if let Some(t) = a.s.tabs.get_mut(i) {
            t.color = color;
        }
    });
    tabs_invalidate();
    note_save();
}

pub(crate) unsafe fn tab_menu(i: usize, x: i32, y: i32) {
    path_tooltip(None);
    let (hwnd, locked, others, any_unlocked, color) = app(|a| {
        let tab = a.s.tabs.get(i);
        let locked = tab.is_some_and(|t| t.locked);
        let others = !close_other_indexes(&a.s.tabs, i).is_empty();
        let any_unlocked = !close_all_indexes(&a.s.tabs).is_empty();
        (a.tabs_hwnd, locked, others, any_unlocked, tab.and_then(|t| t.color))
    });
    let Ok(menu) = CreatePopupMenu() else { return };
    let add = |id: usize, text: &str, on: bool| {
        let flags = if on { MF_STRING } else { MF_STRING | MF_GRAYED };
        _ = AppendMenuW(menu, flags, id, &HSTRING::from(text));
    };
    add(1, tr(crate::Key::RenameTab), true);
    add(2, if locked { tr(crate::Key::UnlockTab) } else { tr(crate::Key::LockTab) }, true);
    let dup_key = crate::keys::index("tab.dup").map(crate::keys::chord_text).unwrap_or_default();
    let dup = if dup_key.is_empty() {
        tr(crate::Key::DuplicateTab).to_string()
    } else {
        format!("{}\t{dup_key}", tr(crate::Key::DuplicateTab))
    };
    add(3, &dup, true);
    add(6, tr(crate::Key::MoveNewWindow), true);
    let list_key = crate::keys::index("tab.list").map(crate::keys::chord_text).unwrap_or_default();
    let list_item = if list_key.is_empty() {
        tr(crate::Key::TabListMenu).to_string()
    } else {
        format!("{}\t{list_key}", tr(crate::Key::TabListMenu))
    };
    add(7, &list_item, true);
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    add(8, tr(crate::Key::CloseAllTabs), any_unlocked);
    add(4, tr(crate::Key::CloseOtherTabs), others);
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    add(5, tr(crate::Key::SaveThisTab3), true);
    let Ok(colors) = CreatePopupMenu() else {
        _ = DestroyMenu(menu);
        return;
    };
    let mark = |menu: HMENU, id: usize, on: bool| {
        if on {
            _ = CheckMenuItem(menu, id as u32, (MF_BYCOMMAND | MF_CHECKED).0);
        }
    };
    _ = AppendMenuW(colors, MF_STRING, 19, &HSTRING::from(tr(crate::Key::None)));
    mark(colors, 19, color.is_none());
    for (k, &(c, label)) in TAB_COLORS.iter().enumerate() {
        let id = 20 + k;
        _ = AppendMenuW(colors, MF_STRING, id, &HSTRING::from(tr(label)));
        mark(colors, id, color == Some(c));
    }
    _ = AppendMenuW(menu, MF_POPUP, colors.0 as usize, &HSTRING::from(tr(crate::Key::TabColor)));
    let mut pt = POINT { x, y };
    _ = ClientToScreen(hwnd, &mut pt);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, hwnd, None).0 as usize;
    _ = DestroyMenu(menu);
    match pick {
        1 => begin_rename(i),
        2 => {
            app(|a| {
                if let Some(t) = a.s.tabs.get_mut(i) {
                    t.locked ^= true;
                }
            });
            tabs_invalidate();
            note_save();
        }
        3 => duplicate_tab(i),
        6 => move_tab_to_new(i),
        7 => crate::tablist::open_menu(),
        8 => close_all_tabs(),
        4 => close_other_tabs(i),
        5 => save_tab_as_session(i),
        19 => set_tab_color(i, None),
        n if (20..20 + TAB_COLORS.len()).contains(&n) => set_tab_color(i, Some(TAB_COLORS[n - 20].0)),
        _ => {}
    }
}

pub(crate) unsafe fn install_session(s: Session) {
    destroy_every_pane();
    let filters = s.tabs.get(s.tab.min(s.tabs.len().saturating_sub(1))).map(|t| t.filters.clone()).unwrap_or_default();
    let n = s.tabs.len().max(1);
    put_filters(&filters);
    app(|a| {
        a.s = s;
        a.park = (0..n).map(|_| no_panes()).collect();
        a.park_lru.clear();
        a.zoom = None;
        let order = a.s.order.clone();
        let tab = a.s.tab;
        a.tab_base = a.s.tabs.iter().enumerate().map(|(i, t)| TabBase {
            grid: t.clone(),
            order: if i == tab { order.clone() } else { Vec::new() },
        }).collect();
        let _ = pull_dirty(a);
    });
    show_active_tab();
}

/// Scroll so `index` is on screen. Uses the main window's width, so it is valid before the strip is moved.
pub(crate) fn show_tab(index: usize) {
    app(|a| {
        if a.main.0.is_null() {
            return;
        }
        let mut rc = RECT::default();
        unsafe { _ = GetClientRect(a.main, &mut rc) };
        if rc.right <= 0 {
            return;
        }
        let locked: Vec<bool> = a.s.tabs.iter().map(|t| t.locked).collect();
        let n = locked.len().max(1);
        let w = tab_area_width(rc.right, a.dpi, a.s.bar, drive_slots(a.s.drive_buttons, a.drives.len()));
        let lay = tab_lay(w, n, &locked, a.dpi, a.tab_scroll);
        let index = index.min(n - 1);
        a.tab_scroll = if lay.overflow.is_none() { 0 } else { scroll_to_show(lay.first, index, lay.count.max(1), n) };
    });
}

pub(crate) fn show_active_tab() {
    let i = app(|a| a.s.tab);
    show_tab(i);
}

/// One wheel notch moves one tab. Positive delta (wheel up) reveals earlier tabs.
pub(crate) fn tab_wheel(delta: i32) {
    if delta == 0 {
        return;
    }
    let mut steps = delta / 120;
    if steps == 0 {
        steps = delta.signum();
    }
    app(|a| {
        if steps > 0 {
            a.tab_scroll = a.tab_scroll.saturating_sub(steps as usize);
        } else {
            a.tab_scroll = a.tab_scroll.saturating_add((-steps) as usize);
        }
    });
    let _ = current_tab_lay();
    tabs_invalidate();
}

/// Every window tab, by name. The current one is checked and a locked one is marked.
pub(crate) unsafe fn open_tab_list() {
    let (hwnd, names, locked, cur) = app(|a| {
        (
            a.tabs_hwnd,
            (0..a.s.tabs.len()).map(|i| tab_text(a, i).0).collect::<Vec<_>>(),
            a.s.tabs.iter().map(|t| t.locked).collect::<Vec<_>>(),
            a.s.tab,
        )
    });
    if hwnd.0.is_null() || names.is_empty() {
        return;
    }
    let Ok(menu) = CreatePopupMenu() else { return };
    for (i, name) in names.iter().enumerate() {
        let shown = name.replace('&', "&&");
        let label = if locked.get(i).copied().unwrap_or(false) {
            format!("{shown} — {}", tr(crate::Key::Locked))
        } else {
            shown
        };
        _ = AppendMenuW(menu, MF_STRING, i + 1, &HSTRING::from(label));
        if i == cur {
            _ = CheckMenuItem(menu, (i + 1) as u32, (MF_BYCOMMAND | MF_CHECKED).0);
        }
    }
    let lay = current_tab_lay();
    let (x, y) = lay.overflow.map(|r| (r.left, r.bottom)).unwrap_or((0, 0));
    let mut pt = POINT { x, y };
    _ = ClientToScreen(hwnd, &mut pt);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, hwnd, None).0 as usize;
    _ = DestroyMenu(menu);
    if (1..=names.len()).contains(&pick) {
        switch_tab(pick - 1);
    }
}

pub(crate) fn current_tab_lay() -> TabLay {
    app(|a| {
        let mut rc = RECT::default();
        unsafe { _ = GetClientRect(a.tabs_hwnd, &mut rc) };
        let locked: Vec<bool> = a.s.tabs.iter().map(|t| t.locked).collect();
        let strip_h = 32 * a.dpi / 96;
        let btn_top = (strip_h - tool_icon_px(a.dpi).1) / 2;
        let icons = icon_slots(rc.right.max(1), btn_top, a.dpi, drive_slots(a.s.drive_buttons, a.drives.len()), a.s.bar);
        let lay = tab_lay(icons.tab_w, locked.len().max(1), &locked, a.dpi, a.tab_scroll);
        a.tab_scroll = lay.first;
        lay
    })
}

/// Title and path tooltip for tab `i`. The tab on screen uses the live grid; the others use what was saved on the tab.
fn tab_text(a: &App, i: usize) -> (String, String) {
    let (name, parked_paths, parked_n, parked_active, live) = {
        let Some(t) = a.s.tabs.get(i) else { return (String::new(), String::new()) };
        let n = t.heights.iter().sum::<usize>().clamp(1, MAXP);
        (t.name.clone(), t.paths.clone(), n, t.active.min(n - 1), i == a.s.tab)
    };
    let (paths, n, active) = if live {
        let n = a.s.panes().clamp(1, MAXP);
        (a.s.paths.clone(), n, a.s.active.min(n - 1))
    } else {
        (parked_paths, parked_n, parked_active)
    };
    let path = paths.get(active).map(String::as_str).unwrap_or("");
    let label = if custom_tab_name(&name).is_none() && path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        tr(crate::Key::ThisPc).into()
    } else {
        tab_label(&name, path)
    };
    let pc: String = tr(crate::Key::ThisPc).into();
    let shown: Vec<String> = paths.iter().take(n).map(|p| if p.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") { pc.clone() } else { p.clone() }).collect();
    let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
    (label, tab_tip_text(&refs))
}

pub(crate) unsafe fn paint_tabs(h: HWND) {
    let (active, names, colors, mut hot, mut hot_close, hot_plus, hot_over, dpi, font, icon, accent) = app(|a| {
        (
            a.s.tab,
            (0..a.s.tabs.len()).map(|i| tab_text(a, i).0).collect::<Vec<_>>(),
            a.s.tabs.iter().map(|t| t.color).collect::<Vec<_>>(),
            a.tab_hot,
            a.tab_hot_close,
            a.tab_hot_plus,
            a.tab_hot_over,
            a.dpi,
            a.font,
            a.icon_font,
            a.s.accent_now(),
        )
    });
    // A file drag warms the tab under the pointer without touching mouse-hover state.
    if let Some(i) = crate::file_window_hot(h) {
        hot = Some(i);
        hot_close = false;
    }
    let lay = current_tab_lay();
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        fill(dc, rc, pal().bg);
        fill(dc, RECT { top: rc.bottom - 1, ..rc }, pal().line);
        for (i, r) in lay.body.iter().enumerate() {
            if r.right <= r.left {
                continue;
            }
            let on = i == active;
            let over = hot == Some(i);
            let col = colors.get(i).copied().flatten();
            let bg = match (on, over, col) {
                (true, _, Some(c)) => blend(pal().card, c, 0.18),
                (true, _, None) => pal().card,
                (_, true, Some(c)) => blend(blend(pal().card, pal().bg, 0.45), c, 0.16),
                (_, true, None) => blend(pal().card, pal().bg, 0.45),
                (_, _, Some(c)) => blend(pal().bg, c, 0.18),
                _ => pal().bg,
            };
            fill(dc, *r, bg);
            if !on {
                frame(dc, *r, 1, pal().line);
            }
            if on {
                fill(dc, RECT { bottom: r.top + s(2), ..*r }, col.unwrap_or(accent));
                fill(dc, RECT { top: r.bottom - 1, bottom: r.bottom, ..*r }, pal().card);
            }
            if let Some(c) = col {
                let inset = if on { 0 } else { 1 };
                fill(dc, RECT { left: r.left + inset, top: r.top + inset, right: r.left + inset + s(4), bottom: r.bottom - inset }, c);
            }
            let lock = lay.lock.get(i).copied().unwrap_or(false);
            let nudge = if col.is_some() { s(5) } else { 0 };
            let mut label = *r;
            label.left += s(8) + nudge;
            if let Some(c) = lay.close[i] {
                label.right = c.left;
                let ink = close_ink(lock, hot_close && hot == Some(i), accent, pal().sub);
                text(dc, c, close_glyph(lock), icon, ink, DT_CENTER);
            } else {
                label.right -= s(8);
            }
            // At the minimum width a locked tab keeps the lock and drops the name.
            let room = label.right - label.left;
            if room >= s(8) && !(lock && room < s(18)) {
                text(dc, label, names.get(i).map(|n| n.as_str()).unwrap_or(""), font, if on { pal().text } else { pal().sub }, DT_LEFT | DT_END_ELLIPSIS);
            }
        }
        // Same hover and the same icon ink for the + and the chevron.
        let hot_bg = blend(pal().card, pal().bg, 0.45);
        let ink = pal().text;
        fill(dc, lay.plus, if hot_plus { hot_bg } else { pal().bg });
        text(dc, lay.plus, ICON_PLUS, icon, ink, DT_CENTER);
        if let Some(r) = lay.overflow {
            fill(dc, r, if hot_over { hot_bg } else { pal().bg });
            text(dc, r, ICON_CHEVRON_DOWN, icon, ink, DT_CENTER);
        }
    });
}

/// "Tab list" plus its chord. The button is a tool-row control, so this is its tooltip text.
pub(crate) fn tab_list_tip() -> String {
    let chord = crate::keys::index("tab.list").map(crate::keys::chord_text).unwrap_or_default();
    let name = tr(crate::Key::TabList);
    if chord.is_empty() { name.to_string() } else { format!("{name} ({chord})") }
}

/// Put the current tab-list name and chord on that button. No-op before the button exists.
pub(crate) unsafe fn refresh_tab_list_tip() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        return;
    }
    let Ok(b) = GetDlgItem(Some(main), ID_TB_LIST) else { return };
    _ = SetWindowTextW(b, &HSTRING::from(tr(crate::Key::TabList)));
    tip_text(b, &tab_list_tip(), TTM_UPDATETIPTEXTW);
}

pub(crate) unsafe fn tab_mouse(x: i32, y: i32) -> TabHit {
    tab_hit(&current_tab_lay(), x, y)
}

/// Screen rect of window-tab body `i`, or None when off-screen.
unsafe fn tab_body_screen(i: usize) -> Option<RECT> {
    let tabs = app(|a| a.tabs_hwnd);
    if tabs.0.is_null() {
        return None;
    }
    let lay = current_tab_lay();
    let r = lay.body.get(i).copied().filter(|r| r.right > r.left)?;
    Some(crate::ui::tip_screen_rect(tabs, r))
}

/// Screen rect of the lock glyph on tab `i` (drawn in the close-box slot).
unsafe fn tab_lock_screen(i: usize) -> Option<RECT> {
    let tabs = app(|a| a.tabs_hwnd);
    if tabs.0.is_null() {
        return None;
    }
    let lay = current_tab_lay();
    if !lay.lock.get(i).copied().unwrap_or(false) {
        return None;
    }
    let r = lay.close.get(i).copied().flatten().filter(|r| r.right > r.left)?;
    Some(crate::ui::tip_screen_rect(tabs, r))
}

/// "Locked" beside the lock glyph. Anchored below the glyph with a 4px gap.
unsafe fn lock_tooltip(show: bool, tab: Option<usize>) {
    let (tip, tabs, added) = app(|a| (a.tip, a.tabs_hwnd, a.tab_lock_tip));
    if tip.0.is_null() || tabs.0.is_null() {
        return;
    }
    const LOCK_ID: usize = 7;
    if !show {
        let (now, _) = crate::ui::tip_leave(tabs, LOCK_ID, added);
        app(|a| a.tab_lock_tip = now);
        return;
    }
    let Some(i) = tab else { return };
    let Some(item) = tab_lock_screen(i) else { return };
    let label = tr(crate::Key::Locked2);
    let key = 0x7000_0000_0000_0000u64 | (i as u64);
    let (now, _) = crate::ui::tip_enter(tabs, LOCK_ID, key, label, item, crate::ui::TipSide::Below, added);
    app(|a| a.tab_lock_tip = now);
}

/// Full paths of the panes in `tab`, one per line. `None` hides it.
/// Anchored below the tab body with a 4px gap; never covers the tab title.
unsafe fn path_tooltip(tab: Option<usize>) {
    let (tip, tabs, added) = app(|a| (a.tip, a.tabs_hwnd, a.tab_path_tip));
    if tip.0.is_null() || tabs.0.is_null() {
        return;
    }
    let text = tab.filter(|_| app(|a| a.tab_drag.is_none())).and_then(|i| {
        let body = app(|a| tab_text(a, i).1);
        (!body.is_empty()).then_some((i, body))
    });
    let Some((i, text)) = text else {
        let (now, _) = crate::ui::tip_leave(tabs, TAB_TIP_ID, added);
        app(|a| a.tab_path_tip = now);
        return;
    };
    let Some(item) = tab_body_screen(i) else {
        let (now, _) = crate::ui::tip_leave(tabs, TAB_TIP_ID, added);
        app(|a| a.tab_path_tip = now);
        return;
    };
    let key = 0x8000_0000_0000_0000u64 | (i as u64);
    let (now, _) = crate::ui::tip_enter(
        tabs,
        TAB_TIP_ID,
        key,
        &text,
        item,
        crate::ui::TipSide::Below,
        added,
    );
    app(|a| a.tab_path_tip = now);
}

/// Inline tab rename, centred in the same rectangle the title is drawn in. No-op while hidden.
pub(crate) unsafe fn place_tab_edit() {
    let (edit, which, dpi, color) = app(|a| {
        let which = a.tab_renaming;
        let color = which.and_then(|i| a.s.tabs.get(i).and_then(|t| t.color));
        (a.tab_edit, which, a.dpi, color)
    });
    let Some(i) = which else { return };
    if edit.0.is_null() || !IsWindowVisible(edit).as_bool() {
        return;
    }
    let lay = current_tab_lay();
    let Some(r) = lay.body.get(i).copied().filter(|r| r.right > r.left) else { return };
    let s = |v: i32| v * dpi / 96;
    let nudge = if color.is_some() { s(5) } else { 0 };
    let left = r.left + s(8) + nudge;
    let right = lay.close.get(i).and_then(|c| *c).map(|c| c.left).unwrap_or(r.right - s(8)).max(left + 1);
    place_edit(edit, RECT { left, top: r.top, right, bottom: r.bottom }, true);
}

pub(crate) extern "system" fn tabs_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| tabs_proc_inner(h, msg, wp, lp)))
}
fn tabs_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        match msg {
            WM_PAINT => paint_tabs(h),
            WM_DESTROY => {
                crate::revoke_tab_drop(h);
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_SIZE => place_tab_edit(),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hit = tab_mouse(x, y);
                let (hot, close, plus, over) = match hit {
                    TabHit::Tab(i) | TabHit::Lock(i) => (Some(i), false, false, false),
                    TabHit::Close(i) => (Some(i), true, false, false),
                    TabHit::Plus => (None, false, true, false),
                    TabHit::Overflow => (None, false, false, true),
                    TabHit::Empty => (None, false, false, false),
                };
                lock_tooltip(matches!(hit, TabHit::Lock(_)), match hit { TabHit::Lock(i) => Some(i), _ => None });
                let path_tab = match hit {
                    TabHit::Tab(i) | TabHit::Close(i) => Some(i),
                    _ => None,
                };
                let (from, paint) = app(|a| {
                    if let Some((pi, px, py, _)) = a.tab_press {
                        if a.tab_drag.is_none() && (x - px).abs() + (y - py).abs() > (4 * a.dpi / 96).max(4) {
                            a.tab_drag = Some(pi);
                        }
                    }
                    let from = a.tab_drag;
                    let same = a.tab_hot == hot && a.tab_hot_close == close && a.tab_hot_plus == plus && a.tab_hot_over == over;
                    if !same {
                        a.tab_hot = hot;
                        a.tab_hot_close = close;
                        a.tab_hot_plus = plus;
                        a.tab_hot_over = over;
                    }
                    (from, !same)
                });
                if let Some(from) = from {
                    let mut rc = RECT::default();
                    _ = GetClientRect(h, &mut rc);
                    // Reorder only while the pointer stays in this strip, and only across tabs that are on screen.
                    if y >= 0 && y < rc.bottom {
                        let lay = current_tab_lay();
                        let edge = lay.overflow.map(|r| r.left).unwrap_or(lay.plus.right);
                        if x < edge {
                            let to = lay.drag_target(x);
                            if to != from && to < app(|a| a.s.tabs.len()) {
                                reorder_tab(from, to);
                                app(|a| a.tab_drag = Some(to));
                            }
                        }
                    }
                }
                if paint || from.is_some() {
                    tabs_invalidate();
                }
                path_tooltip(path_tab);
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                _ = TrackMouseEvent(&mut t);
            }
            WM_MOUSELEAVE => {
                lock_tooltip(false, None);
                path_tooltip(None);
                app(|a| {
                    a.tab_hot = None;
                    a.tab_hot_close = false;
                    a.tab_hot_plus = false;
                    a.tab_hot_over = false;
                });
                tabs_invalidate();
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                end_rename(true);
                match tab_mouse(x, y) {
                    TabHit::Tab(i) | TabHit::Close(i) | TabHit::Lock(i) => {
                        // The lock occupies the close slot and does not arm a close.
                        let on_close = matches!(tab_mouse(x, y), TabHit::Close(_));
                        app(|a| a.tab_press = Some((i, x, y, on_close)));
                        SetCapture(h);
                        if !on_close {
                            switch_tab(i);
                        }
                    }
                    TabHit::Plus => new_tab(),
                    TabHit::Overflow => open_tab_list(),
                    TabHit::Empty => {}
                }
            }
            WM_MOUSEWHEEL => tab_wheel((wp.0 >> 16) as i16 as i32),
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let press = app(|a| a.tab_press.take());
                let dragged = app(|a| a.tab_drag.take());
                _ = ReleaseCapture();
                if let Some(from) = dragged {
                    finish_tab_drag(h, from);
                } else if let Some((i, _, _, on_close)) = press {
                    if on_close {
                        if let TabHit::Close(j) = tab_mouse(x, y) {
                            if j == i {
                                close_tab(i);
                            }
                        }
                    }
                }
            }
            WM_LBUTTONDBLCLK => {
                if let TabHit::Tab(i) = tab_mouse(xy(lp).0, xy(lp).1) {
                    begin_rename(i);
                }
            }
            WM_MBUTTONUP => {
                let hit = tab_mouse(xy(lp).0, xy(lp).1);
                if middle_closes(&current_tab_lay(), hit) {
                    if let TabHit::Tab(i) | TabHit::Close(i) = hit {
                        close_tab(i);
                    }
                }
            }
            WM_RBUTTONUP => {
                let (x, y) = xy(lp);
                let i = match tab_mouse(x, y) {
                    TabHit::Tab(i) | TabHit::Close(i) | TabHit::Lock(i) => i,
                    _ => app(|a| a.s.tab),
                };
                tab_menu(i, x, y);
            }
            WM_CAPTURECHANGED => {
                app(|a| {
                    a.tab_press = None;
                    a.tab_drag = None;
                });
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().card));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32) {
                (ID_TAB_EDIT, EN_KILLFOCUS) => {
                    let skip = app(|a| a.tab_rename_skip);
                    if !skip {
                        end_rename(true);
                    } else {
                        app(|a| a.tab_rename_skip = false);
                    }
                }
                _ => {}
            },
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(lay: &TabLay) -> i32 {
        lay.overflow.map(|r| r.right).unwrap_or(lay.plus.right)
    }

    #[test]
    fn icon_slots_stay_right_aligned_without_drives() {
        let lay = icon_slots(1000, 6, 96, 0, true);
        assert_eq!(lay.drive_n, 0);
        assert_eq!(lay.tab_w, 738);
        assert_eq!(lay.list, RECT { left: 742, top: 6, right: 774, bottom: 30 });
        assert_eq!(lay.list.right + 4, lay.icons[0].1.left);
        assert_eq!(lay.icons.map(|s| s.0), [ID_TB_LAYOUT, ID_TB_SAVE, ID_TB_LOAD, ID_TB_FAV, ID_TB_SET]);
        assert_eq!(lay.icons[0].1, RECT { left: 778, top: 6, right: 848, bottom: 30 });
        assert_eq!(lay.icons[1].1.left, 852);
        assert_eq!(lay.icons[4].1.right, 992);
        // The star is the 32px button immediately left of the gear, 4px gap.
        assert_eq!(lay.icons[3].0, ID_TB_FAV);
        assert_eq!(lay.icons[3].1.right + 4, lay.icons[4].1.left);
        assert_eq!(lay.icons[3].1.right - lay.icons[3].1.left, 32);
        assert_eq!((lay.icons[3].1.top, lay.icons[3].1.bottom), (lay.icons[4].1.top, lay.icons[4].1.bottom));
        assert!(lay.icons.windows(2).all(|w| w[0].1.right + 4 == w[1].1.left));
        assert_eq!(tab_area_width(1000, 96, true, 0), lay.tab_w);
        // Hidden toolbar keeps the gear (right 992, 32px) and the gap in front of it.
        assert_eq!(tab_area_width(1000, 96, false, 8), 920);
    }

    #[test]
    fn icon_group_min_keeps_one_tab_clear_of_the_icons() {
        for (dpi, bar) in [(96, true), (96, false), (144, true), (192, false)] {
            let w = icon_group_min(dpi, bar);
            let lay = icon_slots(w, 0, dpi, 26, bar);
            assert!(lay.tab_w >= tab_reserve(dpi), "{dpi} bar={bar} tab {} reserve {}", lay.tab_w, tab_reserve(dpi));
            assert_eq!(lay.drive_n, 0, "{dpi} bar={bar}");
            let tight = icon_slots((w - 1).max(1), 0, dpi, 0, bar);
            assert!(tight.tab_w < tab_reserve(dpi), "{dpi} bar={bar} tab {} reserve {}", tight.tab_w, tab_reserve(dpi));
        }
    }

    #[test]
    fn icon_slots_with_n_drives() {
        // Letters sit in the group, immediately left of Layout, and do not move the icons.
        let bare = icon_slots(1000, 4, 96, 0, true);
        let n = 5;
        let lay = icon_slots(1000, 4, 96, n, true);
        assert_eq!(lay.drive_n, n);
        assert_eq!(lay.icons, bare.icons);
        assert_eq!(lay.drives[n - 1].right + 4, lay.icons[0].1.left);
        assert_eq!(lay.list.right + 4, lay.drives[0].left);
        assert_eq!(lay.tab_w, lay.list.left - 4);
        for i in 0..n {
            assert_eq!(lay.drives[i].right - lay.drives[i].left, 32);
            assert_eq!((lay.drives[i].top, lay.drives[i].bottom), (4, 28));
            if i > 0 {
                assert_eq!(lay.drives[i - 1].right + 4, lay.drives[i].left);
            }
        }
        // One minimum tab, +, and ▾ (116px) stay clear of the buttons. Tabs paint only in tab_w.
        let reserve = 116;
        assert!(lay.tab_w >= reserve && lay.drives[0].left >= reserve);
        let tabs = tab_lay(lay.tab_w, 30, &[false; 30], 96, 0);
        assert!(edge(&tabs) <= lay.drives[0].left);

        // A narrow row hides the last letters first. The ones that remain are the prefix.
        let tight = icon_slots(420, 4, 96, 8, true);
        assert!(tight.drive_n > 0 && tight.drive_n < 8);
        assert_eq!(tight.icons[0].1.left, icon_slots(420, 4, 96, 0, true).icons[0].1.left);
        assert!(tight.tab_w >= reserve && tight.drives[0].left >= reserve);
        assert_eq!(icon_slots(420, 4, 96, tight.drive_n + 1, true).drive_n, tight.drive_n);
        let shown = tab_lay(tight.tab_w, 12, &[false; 12], 96, 0);
        assert!(edge(&shown) <= tight.drives[0].left);

        // Too narrow for even one letter: the icons stay, every drive is hidden.
        let tiny = icon_slots(220, 0, 96, 4, true);
        assert_eq!(tiny.drive_n, 0);
        assert!(tiny.tab_w >= 1 && tiny.icons[0].1.left < reserve);

        // The same row at double DPI is the 96dpi layout scaled by two.
        let hi = icon_slots(2000, 8, 192, n, true);
        assert_eq!(hi.drive_n, n);
        assert_eq!(hi.tab_w, lay.tab_w * 2);
        assert_eq!(hi.drives[0].right - hi.drives[0].left, 64);
        assert_eq!(hi.drives[n - 1].right + 8, hi.icons[0].1.left);
        assert_eq!(tab_area_width(2000, 192, true, n), hi.tab_w);
        assert_eq!(tab_area_width(2000, 192, false, n), 1840);
    }

    #[test]
    fn icon_slots_with_drives_hidden() {
        let n = 5;
        let shown = icon_slots(1000, 4, 96, drive_slots(true, n), true);
        let hidden = icon_slots(1000, 4, 96, drive_slots(false, n), true);
        let bare = icon_slots(1000, 4, 96, 0, true);
        assert_eq!(drive_slots(false, n), 0);
        assert_eq!(drive_slots(true, n), n);
        assert_eq!(hidden, bare);
        assert_eq!(hidden.drive_n, 0);
        assert!(hidden.tab_w > shown.tab_w);
        assert_eq!(hidden.icons, shown.icons);
        // The width the letters occupied is now part of the tab strip. The icons stay put.
        assert_eq!(hidden.tab_w - shown.tab_w, shown.icons[0].1.left - shown.drives[0].left);
        assert_eq!(tab_area_width(1000, 96, true, drive_slots(false, n)), hidden.tab_w);
        assert_eq!(tab_area_width(1000, 96, true, drive_slots(true, n)), shown.tab_w);
    }

    #[test]
    fn hidden_bar_keeps_the_gear_and_gives_the_rest_to_the_tabs() {
        let on = icon_slots(1000, 6, 96, 5, true);
        let off = icon_slots(1000, 6, 96, 5, false);
        assert_eq!(off.drive_n, 0);
        assert_eq!(off.icons[4].0, ID_TB_SET);
        // Same gear slot as with the toolbar on. The other icons, including the star, collapse to nothing.
        assert_eq!(off.icons[4].1, on.icons[4].1);
        assert!(off.icons[4].1.right > off.icons[4].1.left);
        for i in 0..4 {
            assert_eq!(off.icons[i].1.left, off.icons[4].1.left);
            assert_eq!(off.icons[i].1.right, off.icons[i].1.left);
        }
        assert_eq!(off.list.right + 4, off.icons[4].1.left);
        assert_eq!(off.tab_w, off.list.left - 4);
        assert!(off.tab_w > on.tab_w);
        assert_eq!(tab_area_width(1000, 96, false, 5), off.tab_w);
        assert_eq!(tab_area_width(1000, 96, true, 5), on.tab_w);

        let hi_on = icon_slots(2000, 8, 192, 5, true);
        let hi_off = icon_slots(2000, 8, 192, 5, false);
        assert_eq!(hi_off.icons[4].1, hi_on.icons[4].1);
        assert_eq!(hi_off.list.right + 8, hi_off.icons[4].1.left);
        assert_eq!(hi_off.tab_w, hi_off.list.left - 8);
        assert_eq!(tab_area_width(2000, 192, false, 5), hi_off.tab_w);
    }

    #[test]
    fn tab_lay_plus_width_matches_when_overflowing() {
        let fit = tab_lay(800, 3, &[false; 3], 96, 0);
        assert!(fit.overflow.is_none());
        assert_eq!(fit.plus.right - fit.plus.left, 28);
        assert_eq!(fit.plus.bottom - fit.plus.top, 26);

        let over = tab_lay(240, 12, &[false; 12], 96, 0);
        let o = over.overflow.expect("overflow");
        assert_eq!(over.plus.right - over.plus.left, fit.plus.right - fit.plus.left);
        assert_eq!(over.plus.bottom - over.plus.top, fit.plus.bottom - fit.plus.top);
        assert_eq!(over.plus.top, fit.plus.top);
        // Same square as the +. Wide enough for the icon, and it does not cover the +.
        assert_eq!(o.right - o.left, over.plus.right - over.plus.left);
        assert_eq!(o.bottom - o.top, over.plus.bottom - over.plus.top);
        assert!(over.plus.right <= o.left && o.right <= 240);
        assert!(o.right - o.left >= 20);

        let hi_fit = tab_lay(1600, 3, &[false; 3], 192, 0);
        let hi = tab_lay(480, 12, &[false; 12], 192, 0);
        let ho = hi.overflow.expect("overflow");
        assert_eq!(hi.plus.right - hi.plus.left, hi_fit.plus.right - hi_fit.plus.left);
        assert_eq!(hi.plus.right - hi.plus.left, 56);
        assert_eq!(ho.right - ho.left, 56);
        assert_eq!(ho.bottom - ho.top, hi.plus.bottom - hi.plus.top);
        assert!(hi.plus.right <= ho.left);
    }

    #[test]
    fn tab_list_button_matches_the_toolbar_icon_buttons() {
        // Same glyph kind as the gear. draw_button paints both on the shared icon face.
        assert!(glyph_tool_button(ID_TB_LIST));
        assert!(glyph_tool_button(ID_TB_SET));
        assert!(!glyph_tool_button(ID_TB_LAYOUT));
        assert!(!glyph_tool_button(ID_TB_FAV));
        assert!(!glyph_tool_button(ID_TB_SAVE));
        assert_eq!(glyph(ID_TB_LIST), ICON_LIST);
        assert_ne!(glyph(ID_TB_LIST), glyph(ID_TB_SET));
        for (dpi, top, bar) in [(96, 4, true), (96, 4, false), (144, 6, true), (192, 8, false)] {
            let lay = icon_slots(2400, top, dpi, 4, bar);
            let gear = lay.icons[4].1;
            let list = lay.list;
            let (w, h) = tool_icon_px(dpi);
            assert!(list.right > list.left, "dpi {dpi} bar {bar}");
            assert_eq!(list.right - list.left, gear.right - gear.left, "dpi {dpi}");
            assert_eq!(list.bottom - list.top, gear.bottom - gear.top, "dpi {dpi}");
            assert_eq!((list.top, list.bottom), (gear.top, gear.bottom), "dpi {dpi}");
            assert_eq!((list.right - list.left, list.bottom - list.top), (w, h), "dpi {dpi}");
            assert_eq!(h, BTN_H_TOOL * dpi / 96);
            assert_eq!(w, TOOL_ICON_W * dpi / 96);
            if bar {
                // Same row as the layout button. That one is wider because it shows the grid.
                assert_eq!((lay.icons[0].1.top, lay.icons[0].1.bottom), (list.top, list.bottom));
                assert!(lay.icons[0].1.right - lay.icons[0].1.left > w);
                let star = lay.icons[3].1;
                assert_eq!((star.right - star.left, star.top, star.bottom), (w, list.top, list.bottom));
                assert_eq!((lay.drives[0].right - lay.drives[0].left, lay.drives[0].top, lay.drives[0].bottom), (w, list.top, list.bottom));
            }
            // Not the tab-row + (28×26 DIP).
            let plus = tab_lay(900, 2, &[false; 2], dpi, 0).plus;
            assert_ne!(list.bottom - list.top, plus.bottom - plus.top);
            assert_ne!(list.right - list.left, plus.right - plus.left);
        }
    }
}

