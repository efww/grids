// Pane tabs: folders one Explorer view switches between. Separate from the window tabs
// (one tab = a whole grid) in ui/tabs.rs. One folder hides the strip and shows the add
// button on the bar. Two or more show the strip and hide that button. The folders stay.

use crate::imports::*;
use crate::app::*;
use crate::ui::*;
use crate::session::*;
use super::{focus_view, go_near, layout_pane, save_view, set_active};

/// The bar and the list move when the strip appears or disappears. Otherwise only the strip repaints.
pub(crate) unsafe fn apply_pane_tab_chrome(i: usize, before: usize) {
    let after = app(|a| a.s.pane_tab_list(i).len());
    let Some(h) = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd)) else { return };
    if (before > 1) != (after > 1) {
        layout_pane(h);
        _ = InvalidateRect(Some(h), None, false);
    } else if after > 1 {
        repaint_pane_tabs(i);
    }
}

pub(crate) unsafe fn repaint_pane_tabs(i: usize) {
    let Some(h) = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd)) else { return };
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let strip = pane_geo_of(h, rc.right, rc.bottom, app(|a| a.dpi)).tabs;
    if strip.bottom > strip.top {
        _ = InvalidateRect(Some(h), Some(&strip), false);
    }
}

/// Remember the folder still on screen when a pane-tab switch stores the destination in `paths`
/// before the browse. False when one is already remembered: a second click must not replace
/// the folder the view is actually showing.
pub(crate) fn arm_tab_left(pane: &mut Pane, from: &str) -> bool {
    if pane.tab_left.is_some() {
        return false;
    }
    pane.tab_left = Some(from.to_string());
    true
}

/// A change that browses. Undone when the folder cannot be opened (go_near already tried its parent).
unsafe fn pane_tab_go(i: usize, change: impl FnOnce(&mut Session) -> Option<String>) {
    // Columns belong to the folder still on screen. `change` points `paths` at the
    // destination, and the pending callback then skips the save because the watch key no longer matches.
    save_view(i);
    let (before_n, before_f, before_c, before_p, armed, to) = app(|a| {
        a.s.ensure_pane_tabs();
        let before_n = a.s.pane_tab_list(i).len();
        let before_f = a.s.pane_folders[i].clone();
        let before_c = a.s.pane_cur[i];
        let before_p = a.s.paths.get(i).cloned().unwrap_or_default();
        let to = change(&mut a.s);
        let armed = to.is_some() && a.panes.get_mut(i).and_then(|p| p.as_mut()).is_some_and(|p| arm_tab_left(p, &before_p));
        (before_n, before_f, before_c, before_p, armed, to)
    });
    if let Some(to) = to {
        if !go_near(i, &to) {
            app(|a| {
                a.s.pane_folders[i] = before_f;
                a.s.pane_cur[i] = before_c;
                if i < a.s.paths.len() {
                    a.s.paths[i] = before_p;
                }
                if armed {
                    if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
                        p.tab_left = None;
                    }
                }
            });
            _ = MessageBeep(MB_ICONWARNING);
        }
    }
    apply_pane_tab_chrome(i, before_n);
    note_save();
    focus_view(i);
}

pub(crate) unsafe fn new_pane_tab(i: usize) {
    let before = app(|a| a.s.pane_tab_list(i).len());
    app(|a| a.s.add_pane_tab(i));
    apply_pane_tab_chrome(i, before);
    note_save();
}

pub(crate) unsafe fn pick_pane_tab(i: usize, k: usize) {
    pane_tab_go(i, |s| s.pick_pane_tab(i, k));
}

pub(crate) unsafe fn close_pane_tab(i: usize, k: usize) {
    if app(|a| a.s.pane_tab_list(i).len()) < 2 {
        _ = MessageBeep(MB_ICONWARNING); // the last pane tab stays
        return;
    }
    pane_tab_go(i, |s| s.close_pane_tab(i, k));
}

/// Next or previous pane tab, wrapping around.
pub(crate) unsafe fn step_pane_tab(i: usize, d: i32) {
    let (n, cur) = app(|a| {
        let list = a.s.pane_tab_list(i);
        let cur = if list.len() > 1 { a.s.pane_cur[i].min(list.len() - 1) } else { 0 };
        (list.len() as i32, cur as i32)
    });
    if n > 1 {
        pick_pane_tab(i, (cur + d).rem_euclid(n) as usize);
    }
}

pub(crate) enum PaneTabHit {
    Tab(usize),
    Close(usize),
    Plus,
    Strip,
}

/// What is under (x, y) in a pane's strip. None outside it, or while this pane has one tab.
pub(crate) fn pane_tab_hit(pg: &PaneGeo, n: usize, x: i32, y: i32, dpi: i32) -> Option<PaneTabHit> {
    if !inside(pg.tabs, x, y) {
        return None;
    }
    let (tabs, plus) = pane_tab_rects(pg.tabs, n, dpi);
    if inside(plus, x, y) {
        return Some(PaneTabHit::Plus);
    }
    let Some(k) = tabs.iter().position(|r| inside(*r, x, y)) else { return Some(PaneTabHit::Strip) };
    match pane_tab_close_rect(tabs[k], dpi) {
        Some(c) if n > 1 && inside(c, x, y) => Some(PaneTabHit::Close(k)),
        _ => Some(PaneTabHit::Tab(k)),
    }
}

pub(crate) unsafe fn on_pane_strip(h: HWND, x: i32, y: i32) -> bool {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    inside(pane_geo_of(h, rc.right, rc.bottom, app(|a| a.dpi)).tabs, x, y)
}

/// Left click picks, closes or adds. Middle click closes. False when the point is not on the strip.
pub(crate) unsafe fn pane_tab_mouse(h: HWND, i: usize, x: i32, y: i32, middle: bool) -> bool {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let dpi = app(|a| a.dpi);
    let n = app(|a| a.s.pane_tab_list(i).len());
    let Some(hit) = pane_tab_hit(&pane_geo_of(h, rc.right, rc.bottom, dpi), n, x, y, dpi) else { return false };
    set_active(i);
    match (hit, middle) {
        (PaneTabHit::Tab(k) | PaneTabHit::Close(k), true) => close_pane_tab(i, k),
        (PaneTabHit::Close(k), false) => close_pane_tab(i, k),
        (PaneTabHit::Tab(k), false) => pick_pane_tab(i, k),
        (PaneTabHit::Plus, false) => new_pane_tab(i),
        _ => {}
    }
    focus_view(i);
    true
}

/// The strip under the header: the current folder raised on the list colour with an accent edge,
/// the others as plain names, a × on the current one when there are several, and '+' after the last.
pub(crate) unsafe fn paint_pane_tabs(dc: HDC, pg: &PaneGeo, tabs: &[String], cur: usize, dpi: i32, font: HFONT, accent: u32, hot: Option<usize>) {
    let s = |v: i32| v * dpi / 96;
    if pg.tabs.bottom <= pg.tabs.top || tabs.is_empty() {
        return;
    }
    fill(dc, pg.tabs, pal().bg);
    let (rects, plus) = pane_tab_rects(pg.tabs, tabs.len(), dpi);
    for (k, (r, path)) in rects.iter().zip(tabs).enumerate() {
        let on = k == cur;
        let name = match short_name(path) {
            n if n.is_empty() => "…".to_string(),
            n => n,
        };
        let close = pane_tab_close_rect(*r, dpi).filter(|_| on && tabs.len() > 1);
        if on {
            fill(dc, *r, pal().card);
            fill(dc, RECT { bottom: r.top + s(2), ..*r }, accent);
        } else if hot == Some(k) {
            fill(dc, *r, blend(pal().card, pal().bg, 0.45));
        } else if k + 1 != cur && k + 1 < tabs.len() {
            fill(dc, RECT { left: r.right - 1, top: r.top + s(5), right: r.right, bottom: r.bottom - s(5) }, pal().line);
        }
        let right = close.map_or(r.right - s(8), |c| c.left);
        let label = RECT { left: r.left + s(8), right, ..*r };
        text(dc, label, &name, font, if on { pal().text } else { pal().sub }, DT_LEFT);
        if let Some(c) = close {
            text(dc, c, "×", font, pal().sub, DT_CENTER);
        }
    }
    text(dc, plus, "+", font, pal().sub, DT_CENTER);
}
