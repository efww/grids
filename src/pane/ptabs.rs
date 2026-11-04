// Pane tabs: folders one Explorer view switches between. Separate from the window tabs
// (one tab = a whole grid) in ui/tabs.rs. One plain folder hides the strip and shows the
// add button on the bar. Two or more, or one folder that is locked or renamed, show the
// strip and hide that button.

use crate::imports::*;
use crate::app::*;
use crate::ui::*;
use crate::session::*;
use crate::i18n::*;
use super::{focus_view, go_near, layout_pane, save_view, set_active};

/// Path tooltip for a pane tab. The lock glyph uses 15. Window tabs use 7 and 8.
const PANE_TAB_TIP: usize = 14;
const PANE_TAB_LOCK_TIP: usize = 15;

/// The bar and the list move when the strip appears or disappears. Otherwise only the strip repaints.
pub(crate) unsafe fn apply_pane_tab_chrome(i: usize, before_on: bool) {
    let after_on = app(|a| a.s.pane_strip_on(i));
    let Some(h) = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd)) else { return };
    if before_on != after_on {
        layout_pane(h);
        _ = InvalidateRect(Some(h), None, false);
    } else if after_on {
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
    let (before_on, before_f, before_c, before_l, before_m, before_p, armed, to) = app(|a| {
        a.s.ensure_pane_tabs();
        let before_on = a.s.pane_strip_on(i);
        let before_f = a.s.pane_folders[i].clone();
        let before_c = a.s.pane_cur[i];
        let before_l = a.s.pane_locks[i].clone();
        let before_m = a.s.pane_names[i].clone();
        let before_p = a.s.paths.get(i).cloned().unwrap_or_default();
        let to = change(&mut a.s);
        let armed = to.is_some() && a.panes.get_mut(i).and_then(|p| p.as_mut()).is_some_and(|p| arm_tab_left(p, &before_p));
        (before_on, before_f, before_c, before_l, before_m, before_p, armed, to)
    });
    if let Some(to) = to {
        if !go_near(i, &to) {
            app(|a| {
                a.s.pane_folders[i] = before_f;
                a.s.pane_cur[i] = before_c;
                a.s.pane_locks[i] = before_l;
                a.s.pane_names[i] = before_m;
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
    apply_pane_tab_chrome(i, before_on);
    note_save();
    focus_view(i);
}

pub(crate) unsafe fn new_pane_tab(i: usize) {
    let before_on = app(|a| a.s.pane_strip_on(i));
    app(|a| a.s.add_pane_tab(i));
    apply_pane_tab_chrome(i, before_on);
    note_save();
}

pub(crate) unsafe fn pick_pane_tab(i: usize, k: usize) {
    pane_tab_go(i, |s| s.pick_pane_tab(i, k));
}

pub(crate) unsafe fn close_pane_tab(i: usize, k: usize) {
    let (n, locked) = app(|a| {
        let n = a.s.pane_tab_list(i).len();
        let locked = a.s.pane_tab_locks(i).get(k).copied().unwrap_or(false);
        (n, locked)
    });
    if n < 2 || locked {
        _ = MessageBeep(MB_ICONWARNING); // the last pane tab stays, and so does a locked one
        return;
    }
    pane_tab_go(i, |s| s.close_pane_tab(i, k));
}

pub(crate) unsafe fn close_other_pane_tabs(i: usize, keep: usize) {
    pane_tab_go(i, |s| s.close_pane_tabs_other(i, keep));
}

pub(crate) unsafe fn close_pane_tabs_to_the_right(i: usize, k: usize) {
    pane_tab_go(i, |s| s.close_pane_tabs_right(i, k));
}

pub(crate) unsafe fn duplicate_pane_tab(i: usize, k: usize) {
    pane_tab_go(i, |s| s.duplicate_pane_tab(i, k));
}

pub(crate) unsafe fn toggle_pane_tab_lock(i: usize, k: usize) {
    let before_on = app(|a| a.s.pane_strip_on(i));
    let changed = app(|a| {
        let locked = a.s.pane_tab_locks(i).get(k).copied().unwrap_or(false);
        a.s.set_pane_tab_lock(i, k, !locked)
    });
    if changed {
        apply_pane_tab_chrome(i, before_on);
        note_save();
    }
}

/// Custom title for this pane tab. Empty goes back to the folder name. Digits are kept.
pub(crate) unsafe fn rename_pane_tab(i: usize, k: usize) {
    hide_pane_tab_tip_here(i);
    let (owner, initial, before_on) = app(|a| {
        let initial = a.s.pane_tab_names(i).get(k).cloned().unwrap_or_default();
        (a.main, initial, a.s.pane_strip_on(i))
    });
    if owner.0.is_null() {
        return;
    }
    let Some(text) = crate::settings::ask(owner, tr(Key::RenameTab), &initial) else { return };
    let changed = app(|a| a.s.set_pane_tab_name(i, k, &text));
    if changed {
        apply_pane_tab_chrome(i, before_on);
        note_save();
    }
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
    /// The close slot of a locked pane tab. It does not close.
    Lock(usize),
    Plus,
    Strip,
}

/// What is under (x, y) in a pane's strip. None outside it, or while the strip is hidden.
pub(crate) fn pane_tab_hit(pg: &PaneGeo, n: usize, locks: &[bool], x: i32, y: i32, dpi: i32) -> Option<PaneTabHit> {
    if !inside(pg.tabs, x, y) {
        return None;
    }
    let (tabs, plus) = pane_tab_rects(pg.tabs, n, dpi);
    if inside(plus, x, y) {
        return Some(PaneTabHit::Plus);
    }
    let Some(k) = tabs.iter().position(|r| inside(*r, x, y)) else { return Some(PaneTabHit::Strip) };
    let locked = locks.get(k).copied().unwrap_or(false);
    match pane_tab_close_rect(tabs[k], dpi) {
        Some(c) if locked && inside(c, x, y) => Some(PaneTabHit::Lock(k)),
        Some(c) if n > 1 && inside(c, x, y) => Some(PaneTabHit::Close(k)),
        _ => Some(PaneTabHit::Tab(k)),
    }
}

pub(crate) unsafe fn on_pane_strip(h: HWND, x: i32, y: i32) -> bool {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    inside(pane_geo_of(h, rc.right, rc.bottom, app(|a| a.dpi)).tabs, x, y)
}

fn hit_at(h: HWND, i: usize, x: i32, y: i32) -> Option<PaneTabHit> {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(h, &mut rc) };
    let (dpi, n, locks) = app(|a| (a.dpi, a.s.pane_tab_list(i).len(), a.s.pane_tab_locks(i)));
    pane_tab_hit(&pane_geo(rc.right, rc.bottom, dpi, app(|a| a.s.pane_strip_on(i))), n, &locks, x, y, dpi)
}

/// Left click picks, closes or adds. Middle click closes an unlocked tab. False when the point is not on the strip.
pub(crate) unsafe fn pane_tab_mouse(h: HWND, i: usize, x: i32, y: i32, middle: bool) -> bool {
    let Some(hit) = hit_at(h, i, x, y) else { return false };
    let locks = app(|a| a.s.pane_tab_locks(i));
    set_active(i);
    match (hit, middle) {
        (PaneTabHit::Tab(k), true) if !locks.get(k).copied().unwrap_or(false) => close_pane_tab(i, k),
        (PaneTabHit::Close(k), _) => close_pane_tab(i, k),
        (PaneTabHit::Tab(k), false) => pick_pane_tab(i, k),
        (PaneTabHit::Plus, false) => new_pane_tab(i),
        _ => {}
    }
    focus_view(i);
    true
}

/// Right-click menu for the pane tab under the pointer. `+` does not open it.
/// An empty part of the strip uses the tab on display. The click does not switch tabs.
pub(crate) unsafe fn pane_tab_menu(h: HWND, i: usize, x: i32, y: i32) {
    let Some(hit) = hit_at(h, i, x, y) else { return };
    let k = match hit {
        PaneTabHit::Tab(k) | PaneTabHit::Close(k) | PaneTabHit::Lock(k) => k,
        PaneTabHit::Strip => app(|a| {
            let n = a.s.pane_tab_list(i).len();
            if n > 1 { a.s.pane_cur[i].min(n - 1) } else { 0 }
        }),
        PaneTabHit::Plus => return,
    };
    hide_pane_tab_tip(h);
    set_active(i);
    let (locked, n, others, right, path) = app(|a| {
        let list = a.s.pane_tab_list(i);
        let locks = a.s.pane_tab_locks(i);
        let n = list.len();
        let locked = locks.get(k).copied().unwrap_or(false);
        let others = n > 1 && (0..n).any(|j| j != k && !locks.get(j).copied().unwrap_or(false));
        let right = ((k + 1)..n).any(|j| !locks.get(j).copied().unwrap_or(false));
        let path = list.get(k).cloned().unwrap_or_default();
        (locked, n, others, right, path)
    });
    let Ok(menu) = CreatePopupMenu() else { return };
    let add = |id: usize, text: &str, on: bool| {
        let flags = if on { MF_STRING } else { MF_STRING | MF_GRAYED };
        _ = AppendMenuW(menu, flags, id, &HSTRING::from(text));
    };
    add(1, tr(Key::ClosePaneTab), !locked && n > 1);
    add(2, tr(Key::CloseOtherTabs), others);
    add(3, tr(Key::CloseTabsToTheRight), right);
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    add(4, tr(Key::DuplicateTab), true);
    add(5, if locked { tr(Key::UnlockTab) } else { tr(Key::LockTab) }, true);
    add(6, tr(Key::RenameTab), true);
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    add(7, tr(Key::CopyFullPath), !path.is_empty());
    add(8, tr(Key::OpenInExplorer), !path.is_empty());
    let mut pt = POINT { x, y };
    _ = ClientToScreen(h, &mut pt);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, h, None).0 as usize;
    _ = DestroyMenu(menu);
    match pick {
        1 => close_pane_tab(i, k),
        2 => close_other_pane_tabs(i, k),
        3 => close_pane_tabs_to_the_right(i, k),
        4 => duplicate_pane_tab(i, k),
        5 => toggle_pane_tab_lock(i, k),
        6 => rename_pane_tab(i, k),
        7 => copy_pane_tab_path(i, k),
        8 => open_pane_tab(i, k),
        _ => {}
    }
}

unsafe fn tab_path(i: usize, k: usize) -> String {
    let (list, cur) = app(|a| {
        let list = a.s.pane_tab_list(i);
        let cur = if list.len() > 1 { a.s.pane_cur.get(i).copied().unwrap_or(0).min(list.len() - 1) } else { 0 };
        (list, cur)
    });
    if k == cur {
        let live = super::nav::pane_parsing_name(i);
        if !live.is_empty() {
            return live;
        }
    }
    list.get(k).cloned().unwrap_or_default()
}

unsafe fn copy_pane_tab_path(i: usize, k: usize) {
    let path = tab_path(i, k);
    if path.is_empty() || !crate::copy_plain(&path) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    app(|a| a.note = Some((tr(Key::CopiedPath).to_string(), Instant::now() + std::time::Duration::from_secs(3))));
    crate::update_status();
}

unsafe fn open_pane_tab(i: usize, k: usize) {
    let path = tab_path(i, k);
    if path.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    crate::folderapp::open_in_explorer(&path);
}

/// Full path under the tab, or "Locked" on the lock glyph. False when the point is not on the strip.
pub(crate) unsafe fn pane_tab_hover(h: HWND, i: usize, x: i32, y: i32) -> bool {
    if app(|a| a.pane_drag.is_some() || a.pane_tab_drag.is_some() || a.tab_drag.is_some()) {
        hide_pane_tab_tip(h);
        return false;
    }
    let Some(hit) = hit_at(h, i, x, y) else {
        hide_pane_tab_tip(h);
        return false;
    };
    match hit {
        PaneTabHit::Lock(k) => lock_tip(h, i, k),
        PaneTabHit::Tab(k) | PaneTabHit::Close(k) => path_tip(h, i, k),
        PaneTabHit::Plus | PaneTabHit::Strip => hide_pane_tab_tip(h),
    }
    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
    _ = TrackMouseEvent(&mut t);
    true
}

pub(crate) unsafe fn hide_pane_tab_tip(owner: HWND) {
    let (path, lock) = app(|a| (a.pane_tab_tip, a.pane_tab_lock_tip));
    let (path, _) = tip_leave(owner, PANE_TAB_TIP, path);
    let (lock, _) = tip_leave(owner, PANE_TAB_LOCK_TIP, lock);
    app(|a| {
        a.pane_tab_tip = path;
        a.pane_tab_lock_tip = lock;
    });
}

unsafe fn hide_pane_tab_tip_here(i: usize) {
    let Some(h) = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd)) else { return };
    hide_pane_tab_tip(h);
}

unsafe fn tab_screen(h: HWND, i: usize, k: usize, slot: bool) -> Option<RECT> {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let (dpi, n) = app(|a| (a.dpi, a.s.pane_tab_list(i).len()));
    let pg = pane_geo_of(h, rc.right, rc.bottom, dpi);
    let (rects, _) = pane_tab_rects(pg.tabs, n, dpi);
    let r = rects.get(k).copied().filter(|r| r.right > r.left)?;
    let r = if slot { pane_tab_close_rect(r, dpi).unwrap_or(r) } else { r };
    Some(tip_screen_rect(h, r))
}

unsafe fn path_tip(h: HWND, i: usize, k: usize) {
    let (added_path, added_lock) = app(|a| (a.pane_tab_tip, a.pane_tab_lock_tip));
    let (lock, _) = tip_leave(h, PANE_TAB_LOCK_TIP, added_lock);
    let text = app(|a| a.s.pane_tab_list(i).get(k).cloned().unwrap_or_default());
    let Some(item) = (!text.is_empty()).then(|| tab_screen(h, i, k, false)).flatten() else {
        let (path, _) = tip_leave(h, PANE_TAB_TIP, added_path);
        app(|a| {
            a.pane_tab_tip = path;
            a.pane_tab_lock_tip = lock;
        });
        return;
    };
    let key = 0xE000_0000_0000_0000u64 | ((i as u64) << 32) | k as u64;
    let (path, _) = tip_enter(h, PANE_TAB_TIP, key, &text, item, TipSide::Below, added_path);
    app(|a| {
        a.pane_tab_tip = path;
        a.pane_tab_lock_tip = lock;
    });
}

unsafe fn lock_tip(h: HWND, i: usize, k: usize) {
    let (added_path, added_lock) = app(|a| (a.pane_tab_tip, a.pane_tab_lock_tip));
    let (path, _) = tip_leave(h, PANE_TAB_TIP, added_path);
    let Some(item) = tab_screen(h, i, k, true) else {
        let (lock, _) = tip_leave(h, PANE_TAB_LOCK_TIP, added_lock);
        app(|a| {
            a.pane_tab_tip = path;
            a.pane_tab_lock_tip = lock;
        });
        return;
    };
    let key = 0xF000_0000_0000_0000u64 | ((i as u64) << 32) | k as u64;
    let (lock, _) = tip_enter(h, PANE_TAB_LOCK_TIP, key, tr(Key::Locked2), item, TipSide::Below, added_lock);
    app(|a| {
        a.pane_tab_tip = path;
        a.pane_tab_lock_tip = lock;
    });
}

/// The strip under the header. The current folder is raised, with an accent edge.
/// A locked tab shows a lock in the close slot and cannot be closed. The current unlocked
/// tab shows × when there is another tab. '+' sits after the last.
/// `hot` is the tab under a resting file drag. Mouse hover on the strip is left alone.
pub(crate) unsafe fn paint_pane_tabs(
    dc: HDC,
    pg: &PaneGeo,
    tabs: &[String],
    locks: &[bool],
    names: &[String],
    cur: usize,
    dpi: i32,
    font: HFONT,
    icon: HFONT,
    accent: u32,
    hot: Option<usize>,
) {
    let s = |v: i32| v * dpi / 96;
    if pg.tabs.bottom <= pg.tabs.top || tabs.is_empty() {
        return;
    }
    fill(dc, pg.tabs, pal().bg);
    let (rects, plus) = pane_tab_rects(pg.tabs, tabs.len(), dpi);
    for (k, (r, path)) in rects.iter().zip(tabs).enumerate() {
        let on = k == cur;
        let locked = locks.get(k).copied().unwrap_or(false);
        let custom = names.get(k).map(String::as_str).unwrap_or("");
        let name = if custom.trim().is_empty() {
            match short_name(path) {
                n if n.is_empty() => "…".to_string(),
                n => n,
            }
        } else {
            custom.trim().chars().take(80).collect()
        };
        let slot = pane_tab_close_rect(*r, dpi).filter(|_| locked || (on && tabs.len() > 1));
        if on {
            fill(dc, *r, pal().card);
            fill(dc, RECT { bottom: r.top + s(2), ..*r }, accent);
        } else if hot == Some(k) {
            fill(dc, *r, blend(pal().card, pal().bg, 0.45));
        } else if k + 1 != cur && k + 1 < tabs.len() {
            fill(dc, RECT { left: r.right - 1, top: r.top + s(5), right: r.right, bottom: r.bottom - s(5) }, pal().line);
        }
        let right = slot.map_or(r.right - s(8), |c| c.left);
        let label = RECT { left: r.left + s(8), right, ..*r };
        text(dc, label, &name, font, if on { pal().text } else { pal().sub }, DT_LEFT);
        if let Some(c) = slot {
            let ink = close_ink(locked, false, accent, pal().sub);
            text(dc, c, close_glyph(locked), icon, ink, DT_CENTER);
        }
    }
    text(dc, plus, "+", font, pal().sub, DT_CENTER);
}
