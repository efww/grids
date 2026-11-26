use crate::*;

// ---------- tabs: one Explorer view per pane, the tabs are folders it switches between ----------

// View menu: show / hide every pane's strip. The tabs stay put while hidden.
pub(crate) unsafe fn toggle_tabs() {
    let on = app(|a| {
        a.s.tabs_on ^= true;
        a.s.tabs_on
    });
    TABS_ON.store(on, Ordering::Relaxed);
    for h in app(|a| a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>()) {
        layout_pane(h);
        _ = InvalidateRect(Some(h), None, false);
    }
}

pub(crate) unsafe fn repaint_tabs(i: usize) {
    let Some(h) = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd)) else { return };
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let strip = pane_geo(rc.right, rc.bottom, app(|a| a.dpi)).tabs;
    if strip.bottom > strip.top {
        _ = InvalidateRect(Some(h), Some(&strip), false);
    }
}

// Folder path → system image list index, filled from the address bar's own lookups (WM_PANE_ICON) so a tab
// that is not current keeps its folder's icon; nothing is looked up for it. Case-insensitive, runtime only,
// at most MAX entries (one more empties it).
#[derive(Default)]
pub(crate) struct IconMemo(std::collections::HashMap<String, i32>);

impl IconMemo {
    pub(crate) const MAX: usize = 256;

    // Only real icons: a failed lookup (-1) leaves the tab to the drive / plain folder icon.
    // True when the stored icon is new or different (other panes' tabs of that folder need a repaint).
    pub(crate) fn put(&mut self, path: &str, ix: i32) -> bool {
        if ix < 0 || path.is_empty() {
            return false;
        }
        let k = path.to_lowercase();
        if self.0.len() >= Self::MAX && !self.0.contains_key(&k) {
            self.0.clear();
        }
        self.0.insert(k, ix) != Some(ix)
    }

    pub(crate) fn get(&self, path: &str) -> Option<i32> {
        self.0.get(&path.to_lowercase()).copied()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

// "X:\" exactly (a drive's root) → its letter.
pub(crate) fn drive_root(path: &str) -> Option<u8> {
    match path.as_bytes() {
        [c, b':', b'\\'] if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

// A tab's icon (-1: none). The current tab: the address bar's icon, or while that is still on its way the one
// this folder had before (no blink on switching). Then: last seen this run → its drive kind's (a root) → plain folder.
pub(crate) fn tab_icon_pick(current: bool, pane: i32, saved: Option<i32>, drive: i32, folder: i32) -> i32 {
    if current && pane >= 0 {
        return pane;
    }
    saved.filter(|&n| n >= 0).or(Some(drive).filter(|&n| n >= 0)).unwrap_or(folder.max(-1))
}

// Each tab's icon for the painter (inside app(): reads only what is cached, calls nothing).
pub(crate) fn tab_icons_of(a: &App, i: usize, list: &[String], cur: usize) -> Vec<i32> {
    let pane = a.panes.get(i).and_then(|p| p.as_ref()).map_or(-1, |p| p.icon);
    let folder = folder_icon();
    list.iter()
        .enumerate()
        .map(|(k, path)| {
            let kind = drive_root(path).and_then(|c| a.drives.iter().find(|d| d.0 == c)).map(|d| d.1);
            tab_icon_pick(k == cur, pane, a.tab_icons.get(path), kind.map_or(-1, drive_icon), folder)
        })
        .collect()
}

// A tab change that browses: undone when the folder can't be opened (go_near already tried its parent).
unsafe fn tab_go(i: usize, change: impl FnOnce(&mut Session) -> Option<String>) {
    let (before, to) = app(|a| {
        let before = (a.s.tabs[i].clone(), a.s.tab[i]);
        (before, change(&mut a.s))
    });
    if let Some(to) = to {
        if !go_near(i, &to) {
            app(|a| (a.s.tabs[i], a.s.tab[i]) = before);
            _ = MessageBeep(MB_ICONWARNING);
        }
    }
    repaint_tabs(i);
    rehover_pane(i);
}

pub(crate) unsafe fn new_tab(i: usize) {
    app(|a| a.s.add_tab(i));
    repaint_tabs(i);
    rehover_pane(i);
}

// The tabs moved under a still pointer (picked, closed, added, by mouse, key or menu): shade what is there now.
unsafe fn rehover_pane(i: usize) {
    if let Some(h) = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd)) {
        head_rehover(h);
    }
}

pub(crate) unsafe fn pick_tab(i: usize, k: usize) {
    tab_go(i, |s| s.pick_tab(i, k));
}

pub(crate) unsafe fn dup_tab(i: usize, k: usize) {
    tab_go(i, |s| s.dup_tab(i, k));
}

// Tab k one place left (d = -1) or right (1); the folder on display stays. At either end: nothing (no wrap).
unsafe fn move_tab(i: usize, k: usize, d: i32) {
    if app(|a| a.s.move_tab(i, k, d)) {
        repaint_tabs(i);
        rehover_pane(i);
    }
}

// Ctrl+Shift+PgUp / Ctrl+Shift+PgDn: the current tab.
pub(crate) unsafe fn move_cur_tab(i: usize, d: i32) {
    move_tab(i, app(|a| a.s.tab[i]), d);
}

// Alt+1 … Alt+8: the nth tab, Alt+9: the last one. No such tab: nothing.
pub(crate) unsafe fn pick_nth_tab(i: usize, n: usize) {
    let len = app(|a| a.s.tab_list(i).len());
    let k = if n >= 9 { len - 1 } else { n - 1 };
    if k < len {
        pick_tab(i, k);
    }
}

// Right-click on a tab: duplicate, move or close that tab.
pub(crate) unsafe fn tab_menu(h: HWND, i: usize, k: usize, at: POINT) {
    let n = app(|a| a.s.tab_list(i).len());
    let Ok(menu) = CreatePopupMenu() else { return };
    let on = |yes: bool| if yes { MF_STRING } else { MF_STRING | MF_GRAYED };
    _ = AppendMenuW(menu, MF_STRING, 1, &HSTRING::from(tr("Duplicate tab\tCtrl+K", "탭 복제\tCtrl+K")));
    _ = AppendMenuW(menu, on(k > 0 && k < n), 3, &HSTRING::from(tr("Move left\tCtrl+Shift+PgUp", "왼쪽으로 옮기기\tCtrl+Shift+PgUp")));
    _ = AppendMenuW(menu, on(k + 1 < n), 4, &HSTRING::from(tr("Move right\tCtrl+Shift+PgDn", "오른쪽으로 옮기기\tCtrl+Shift+PgDn")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    _ = AppendMenuW(menu, on(n > 1), 2, &HSTRING::from(tr("Close tab\tCtrl+W", "탭 닫기\tCtrl+W"))); // the last tab stays
    set_active(i);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, at.x, at.y, None, h, None).0;
    _ = DestroyMenu(menu);
    match pick {
        1 => dup_tab(i, k),
        2 => close_tab(i, k),
        3 => move_tab(i, k, -1),
        4 => move_tab(i, k, 1),
        _ => {}
    }
    focus_view(i);
}

pub(crate) unsafe fn close_tab(i: usize, k: usize) {
    if app(|a| a.s.tab_list(i).len()) < 2 {
        _ = MessageBeep(MB_ICONWARNING); // the last tab stays
        return;
    }
    tab_go(i, |s| s.close_tab(i, k));
}

// Ctrl+PgDn / Ctrl+PgUp, wrapping around.
pub(crate) unsafe fn step_tab(i: usize, d: i32) {
    let (n, cur) = app(|a| (a.s.tab_list(i).len() as i32, a.s.tab[i] as i32));
    if n > 1 {
        pick_tab(i, (cur + d).rem_euclid(n) as usize);
    }
}

// A header-window mouse message landing in the tab strip.
pub(crate) unsafe fn on_strip(h: HWND, _i: usize, lp: LPARAM) -> bool {
    let (x, y) = ((lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    inside(pane_geo(rc.right, rc.bottom, app(|a| a.dpi)).tabs, x, y)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TabHit {
    Tab(usize),
    Close(usize),
    Plus,
    Strip,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TabClick {
    Left,
    Middle,
    // the second press of a double click
    Double,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TabAct {
    Pick(usize),
    Close(usize),
    New,
    Nothing,
}

// Where tab k's × is, if it has one: only on the current tab, and only while there are several.
// The painter and the hit test both ask this, so a click closes only where a × is drawn.
pub(crate) fn tab_close_at(tab: RECT, k: usize, cur: usize, n: usize, dpi: i32) -> Option<RECT> {
    tab_close_rect(tab, dpi).filter(|_| k == cur && n > 1)
}

// What is under (x, y) in a pane's strip; None outside it (or with tabs off: a strip of no height).
pub(crate) fn tab_hit(strip: RECT, n: usize, cur: usize, x: i32, y: i32, dpi: i32) -> Option<TabHit> {
    if !inside(strip, x, y) {
        return None;
    }
    let (tabs, plus) = tab_rects(strip, n, dpi);
    if inside(plus, x, y) {
        return Some(TabHit::Plus);
    }
    let Some(k) = tabs.iter().position(|r| inside(*r, x, y)) else { return Some(TabHit::Strip) };
    match tab_close_at(tabs[k], k, cur, n, dpi) {
        Some(c) if inside(c, x, y) => Some(TabHit::Close(k)),
        _ => Some(TabHit::Tab(k)),
    }
}

// What the mouse shades in a pane's header: a tab, the current tab's ×, '+', or the filter pill's file-type icon.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HeadHot {
    Tab(usize),
    Close(usize),
    Plus,
    Kind,
}

// Asks the clicks' own hit tests (tab_hit, filter_icon_rect), so what is shaded is what a click does there.
pub(crate) fn head_hot_at(pg: &PaneGeo, n: usize, cur: usize, x: i32, y: i32, dpi: i32) -> Option<HeadHot> {
    if filter_icon_rect(pg.filt, 24 * dpi / 96).is_some_and(|r| inside(r, x, y)) {
        return Some(HeadHot::Kind);
    }
    match tab_hit(pg.tabs, n, cur, x, y, dpi)? {
        TabHit::Tab(k) => Some(HeadHot::Tab(k)),
        TabHit::Close(k) => Some(HeadHot::Close(k)),
        TabHit::Plus => Some(HeadHot::Plus),
        TabHit::Strip => None,
    }
}

// Left click picks / closes / adds, middle click closes. A double click's second press never closes:
// the first press already did its part (picking a tab puts a × under the pointer).
pub(crate) fn tab_action(hit: TabHit, c: TabClick) -> TabAct {
    match (c, hit) {
        (TabClick::Left, TabHit::Tab(k)) => TabAct::Pick(k),
        (TabClick::Left | TabClick::Middle, TabHit::Close(k)) | (TabClick::Middle, TabHit::Tab(k)) => TabAct::Close(k),
        (TabClick::Left | TabClick::Double, TabHit::Plus) | (TabClick::Double, TabHit::Strip) => TabAct::New,
        (TabClick::Double, TabHit::Tab(k) | TabHit::Close(k)) => TabAct::Pick(k),
        _ => TabAct::Nothing,
    }
}

// Mouse in a pane's header window. False: not the strip.
pub(crate) unsafe fn tab_mouse(h: HWND, i: usize, x: i32, y: i32, click: TabClick) -> bool {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let dpi = app(|a| a.dpi);
    let (n, cur) = app(|a| (a.s.tab_list(i).len(), a.s.tab[i]));
    let Some(hit) = tab_hit(pane_geo(rc.right, rc.bottom, dpi).tabs, n, cur, x, y, dpi) else { return false };
    set_active(i);
    let act = tab_action(hit, click);
    match act {
        TabAct::Pick(k) => pick_tab(i, k),
        TabAct::Close(k) => close_tab(i, k),
        TabAct::New => new_tab(i),
        TabAct::Nothing => {}
    }
    focus_view(i);
    // shortcut tip when it did happen: Ctrl+W closes the current tab only, and a middle click is quick already
    let now = app(|a| a.s.tab_list(i).len());
    match act {
        TabAct::New if now > n => mouse_tip(TIP_NEW_TAB),
        TabAct::Close(k) if click == TabClick::Left && k == cur && now < n => mouse_tip(TIP_CLOSE_TAB),
        _ => {}
    }
    true
}

// The strip under the header bar: the current tab raised on the list's colour with an accent edge on top,
// the others as plain names; a × on the current one when there are several; '+' after the last.
// Under the mouse (as browser tabs): another tab lightens, the × or '+' gets a shaded square.
// Each name follows its folder's icon (icons[k], from tab_icons_of) when the tabs are wide enough for one.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn paint_tabs(
    dc: HDC,
    pg: &PaneGeo,
    tabs: &[String],
    cur: usize,
    dpi: i32,
    font: HFONT,
    accent: u32,
    hot: Option<HeadHot>,
    icons: Option<(&IImageList, &[i32])>,
) {
    let s = |v: i32| v * dpi / 96;
    if pg.tabs.bottom <= pg.tabs.top {
        return;
    }
    fill(dc, pg.tabs, pal().bg);
    let (rects, plus) = tab_rects(pg.tabs, tabs.len(), dpi);
    // icons or not, once for the whole strip (tab_icon_rect), so no single tab drops its icon
    let icons = icons.filter(|_| rects.first().is_some_and(|r| tab_icon_rect(*r, tabs.len() > 1, dpi).is_some()));
    // a hot × / '+': a square centred on the glyph, shaded as a pressed header button
    let square = |c: RECT, under: u32| {
        let (d, x, y) = (s(16), (c.left + c.right) / 2 - s(8), (c.top + c.bottom) / 2 - s(8));
        fill(dc, RECT { left: x, top: y, right: x + d, bottom: y + d }, blend(under, pal().text, press_tint(false, true)));
    };
    let lit = |k: usize| k != cur && hot == Some(HeadHot::Tab(k)); // hot may be stale (k >= n): then nothing matches
    for (k, (r, path)) in rects.iter().zip(tabs).enumerate() {
        let on = k == cur;
        let name = match place_label(path) {
            n if n.is_empty() => "…".to_string(),
            n => n,
        };
        let close = tab_close_at(*r, k, cur, tabs.len(), dpi);
        if on {
            fill(dc, *r, pal().card);
            fill(dc, RECT { bottom: r.top + s(2), ..*r }, accent);
        } else if lit(k) {
            fill(dc, *r, blend(pal().bg, pal().text, press_tint(false, false)));
        }
        if !on && k + 1 != cur && k + 1 < tabs.len() && !lit(k) && !lit(k + 1) {
            // hairline between two plain tabs (not against a lightened one)
            fill(dc, RECT { left: r.right - 1, top: r.top + s(5), right: r.right, bottom: r.bottom - s(5) }, pal().line);
        }
        // no icon to draw (none known, GetIcon failed): the name stays where it was without icons
        let mut left = r.left + s(8);
        if let (Some((il, ixs)), Some(ir)) = (icons, tab_icon_rect(*r, tabs.len() > 1, dpi)) {
            if let Some(icon) = ixs.get(k).filter(|&&n| n >= 0).and_then(|&n| il.GetIcon(n, ILD_NORMAL.0).ok()) {
                _ = DrawIconEx(dc, ir.left, ir.top, icon, ir.right - ir.left, ir.bottom - ir.top, 0, None, DI_NORMAL);
                _ = DestroyIcon(icon);
                left = r.left + s(28);
            }
        }
        let right = close.map_or(r.right - s(8), |c| c.left);
        let label = RECT { left, right, ..*r };
        text(dc, label, &name, font, if on || lit(k) { pal().text } else { pal().sub }, DT_LEFT);
        if let Some(c) = close {
            let hot_x = hot == Some(HeadHot::Close(k));
            if hot_x {
                square(c, pal().card);
            }
            text(dc, c, "×", font, if hot_x { pal().text } else { pal().sub }, DT_CENTER);
        }
    }
    let hot_plus = hot == Some(HeadHot::Plus);
    if hot_plus {
        square(plus, pal().bg);
    }
    text(dc, plus, "+", font, if hot_plus { pal().text } else { pal().sub }, DT_CENTER);
}
