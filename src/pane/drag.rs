use crate::imports::*;
use crate::app::*;
use crate::session::*;
use crate::ui::*;
use super::{apply_pane_tab_chrome, arm_tab_left, commit_layout, focus_view, go_near, hide_pane_tab_tip, pick_pane_tab, redraw_all, relayout, rename_fields, repaint_pane_tabs, save_view, set_active, update_status, NAV_GEN, SEL_REV};

// Grab a pane by its header or pane-tab strip and drop it on another pane.
// The file list is not a grab: OLE drag-and-drop from the view is untouched.
// Window-tab reorder is a different window and a different capture.
// A drag that stays on a pane-tab body reorders that pane's tabs. Leaving the
// strip before the slack is a pane move, and Ctrl then copies. Releasing Ctrl
// mid-move returns to the move. A copy never leaves this window tab.

/// Drop preview when a copy would pass the grid caps. The move preview stays the accent.
pub(crate) const COPY_BLOCK: u32 = 0xE23B3B;

fn ctrl_copy() -> bool {
    unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 }
}

fn drag_cursor() {
    unsafe {
        if let Ok(c) = LoadCursorW(None, IDC_SIZEALL) {
            SetCursor(Some(c));
        }
    }
}

fn overlay_hwnd(main: HWND) -> HWND {
    let cur = app(|a| a.pane_over);
    if !cur.0.is_null() && unsafe { IsWindow(Some(cur)).as_bool() } {
        return cur;
    }
    let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    let h = unsafe { CreateWindowExW(ex, w!("MaguroBar"), w!(""), WS_POPUP, 0, 0, 0, 0, Some(main), None, None, None).unwrap_or_default() };
    if h.0.is_null() {
        return h;
    }
    // Same class brush as the splitter guides (the accent). Half opacity, so the files stay readable.
    unsafe { _ = SetLayeredWindowAttributes(h, COLORREF(0), 128, LWA_ALPHA) };
    app(|a| a.pane_over = h);
    h
}

fn hide_overlay() {
    let h = app(|a| a.pane_over);
    if !h.0.is_null() {
        unsafe { _ = ShowWindow(h, SW_HIDE) };
    }
}

fn show_overlay(main: HWND, rect: Option<RECT>) {
    let Some(r) = rect else {
        hide_overlay();
        return;
    };
    let h = overlay_hwnd(main);
    if h.0.is_null() {
        return;
    }
    let mut p = POINT { x: r.left, y: r.top };
    unsafe {
        _ = ClientToScreen(main, &mut p);
        let w = (r.right - r.left).max(1);
        let hgt = (r.bottom - r.top).max(1);
        _ = SetWindowPos(h, Some(HWND_TOP), p.x, p.y, w, hgt, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        _ = InvalidateRect(Some(h), None, true);
        _ = UpdateWindow(h);
    }
}

struct DropHot {
    onto: usize,
    zone: DropZone,
    preview: RECT,
    copy: bool,
    blocked: bool,
}

fn hit_drop(a: &App) -> Option<DropHot> {
    let from = a.pane_drag.as_ref()?.from;
    if a.zoom.is_some() {
        return None;
    }
    let copy = ctrl_copy();
    let mut pt = POINT::default();
    unsafe { _ = GetCursorPos(&mut pt) };
    let mut c = pt;
    unsafe { _ = ScreenToClient(a.main, &mut c) };
    let g = geo(a);
    let n = a.s.panes();
    let heights = &a.s.heights;
    // A move still ignores the pane it came from. A copy may land on that pane's own edge or centre.
    let onto = (0..n).find(|&i| (copy || i != from) && inside(cell_rect(&g, heights, i), c.x, c.y))?;
    let zone = pane_zone(cell_rect(&g, heights, onto), c.x, c.y)?;
    let plan = drag_plan(copy, heights, &a.s.colr, &a.s.rowr, from, onto, zone)?;
    let cells: Vec<RECT> = (0..n).map(|i| cell_rect(&g, heights, i)).collect();
    let preview = drop_preview(heights, &cells, onto, zone)?;
    let blocked = matches!(plan, DragPlan::Copy(CopyDrop::Blocked));
    Some(DropHot { onto, zone, preview, copy, blocked })
}

fn copy_hint(copy: bool, blocked: bool) -> String {
    if !copy {
        return String::new();
    }
    crate::tr(if blocked { crate::Key::CopyPaneBlocked } else { crate::Key::CopyPaneDrag }).to_string()
}

fn track_pane_drag() {
    let placed = app(|a| {
        if a.pane_drag.is_none() {
            return None;
        }
        let hot = hit_drop(a);
        let copy = hot.as_ref().is_some_and(|h| h.copy) || (hot.is_none() && ctrl_copy());
        let blocked = hot.as_ref().is_some_and(|h| h.blocked);
        if let Some(d) = a.pane_drag.as_mut() {
            d.copy = copy;
            d.blocked = blocked;
            d.hot = hot.as_ref().filter(|h| !h.blocked).map(|h| (h.onto, h.zone));
        }
        let hint = copy_hint(copy, blocked);
        let hint_changed = a.drag_hint != hint;
        if hint_changed {
            a.drag_hint = hint;
        }
        Some((a.main, hot.map(|h| h.preview), hint_changed))
    });
    let Some((main, rect, hint_changed)) = placed else { return };
    show_overlay(main, rect);
    if hint_changed {
        unsafe { update_status() };
    }
}

/// Ctrl went down or up while a pane is moving. The next mouse move would notice too.
pub(crate) unsafe fn refresh_pane_drag() {
    if app(|a| a.pane_drag.is_none()) {
        return;
    }
    track_pane_drag();
}

enum PressStart {
    Reorder(usize, usize),
    Move,
}

/// True when this press is the start of a pane drag (header or strip, not a button or the list).
/// A pane-tab body can also start a reorder when this window has only one pane.
pub(crate) unsafe fn pane_drag_down(h: HWND, i: usize, x: i32, y: i32) -> bool {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let copying = ctrl_copy();
    let grab = app(|a| {
        // Splitter drag, zoom, and a window-tab press each already own the mouse.
        let panes = a.s.panes();
        if a.drag.is_some() || a.zoom.is_some() || panes == 0 || a.tab_drag.is_some() || a.tab_press.is_some() || a.pane_drag.is_some() || a.pane_tab_drag.is_some() {
            return None;
        }
        let n = a.s.pane_tab_list(i).len();
        let locks = a.s.pane_tab_locks(i);
        let strip = a.s.pane_strip_on(i);
        let grab = pane_grab(&pane_geo(rc.right, rc.bottom, a.dpi, strip), n, &locks, x, y, a.dpi)?;
        // One pane still cannot start a move. Ctrl can start a copy of that pane.
        // A tab body can reorder inside its own strip either way.
        let can_move = copying || panes >= 2;
        match grab {
            PaneGrab::Tab(_) | PaneGrab::Strip => Some(grab),
            PaneGrab::Header if can_move => Some(grab),
            _ => None,
        }
    });
    let Some(grab) = grab else { return false };
    app(|a| a.pane_press = Some(PanePress { hwnd: h, pane: i, x, y, grab, past: false }));
    SetCapture(h);
    true
}

pub(crate) unsafe fn pane_drag_move(h: HWND, x: i32, y: i32) {
    if app(|a| a.pane_tab_drag.is_some()) {
        drag_pane_tabs(h, x, y);
        return;
    }
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let started = app(|a| {
        let p = a.pane_press.as_ref()?;
        if p.hwnd != h || p.past {
            return None;
        }
        let slack = drag_slack(a.dpi);
        if (x - p.x).abs() + (y - p.y).abs() <= slack {
            return None;
        }
        let grab = p.grab;
        let pane = p.pane;
        let strip = a.s.pane_strip_on(pane);
        let pg = pane_geo(rc.right, rc.bottom, a.dpi, strip);
        let in_strip = pg.tabs.bottom > pg.tabs.top && y >= pg.tabs.top && y < pg.tabs.bottom;
        if let PaneGrab::Tab(k) = grab {
            if in_strip {
                a.pane_press = None;
                a.pane_tab_drag = Some(k);
                return Some(PressStart::Reorder(pane, k));
            }
        }
        let can_move = ctrl_copy() || a.s.panes() >= 2;
        if can_move {
            a.pane_press = None;
            a.pane_drag = Some(PaneMove { hwnd: h, from: pane, hot: None, copy: false, blocked: false });
            return Some(PressStart::Move);
        }
        if let Some(p) = a.pane_press.as_mut() {
            p.past = true;
        }
        None
    });
    match started {
        Some(PressStart::Reorder(pane, index)) => {
            hide_pane_tab_tip(h);
            let cur = app(|a| {
                let list = a.s.pane_tab_list(pane);
                if list.len() > 1 { a.s.pane_cur.get(pane).copied().unwrap_or(0).min(list.len() - 1) } else { 0 }
            });
            if pane_reorder_switches(cur, index) {
                pick_pane_tab(pane, index);
            }
            // The press already captured this pane. Take it back if hiding the tip let it go.
            app(|a| a.pane_tab_drag = Some(index));
            SetCapture(h);
            drag_pane_tabs(h, x, y);
        }
        Some(PressStart::Move) => {
            drag_cursor();
            track_pane_drag();
        }
        None => {
            if app(|a| a.pane_drag.as_ref().is_some_and(|d| d.hwnd == h)) {
                drag_cursor();
                track_pane_drag();
            }
        }
    }
}

/// Reorder stays inside this pane until the button comes up, even if the pointer leaves the strip.
/// The index changes only while the pointer is still on the strip's row.
unsafe fn drag_pane_tabs(h: HWND, x: i32, y: i32) {
    let Some(from) = app(|a| a.pane_tab_drag) else { return };
    let Some(pane) = pane_of(h) else { return };
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let (dpi, strip, n) = app(|a| (a.dpi, a.s.pane_strip_on(pane), a.s.pane_tab_list(pane).len()));
    let pg = pane_geo(rc.right, rc.bottom, dpi, strip);
    if y < pg.tabs.top || y >= pg.tabs.bottom {
        return;
    }
    let (rects, _) = pane_tab_rects(pg.tabs, n, dpi);
    let to = tab_drag_index(&rects, x);
    if to == from || to >= n {
        return;
    }
    let moved = app(|a| {
        let ok = a.s.reorder_pane_tab(pane, from, to);
        if ok {
            a.pane_tab_drag = Some(to);
        }
        ok
    });
    if moved {
        repaint_pane_tabs(pane);
        note_save();
    }
}

/// Drop, or a click that never crossed the slack. True when this window owned the press.
pub(crate) unsafe fn pane_drag_up(h: HWND) -> bool {
    if app(|a| a.pane_tab_drag.is_some()) {
        app(|a| {
            a.pane_tab_drag = None;
            a.pane_press = None;
        });
        hide_overlay();
        _ = ReleaseCapture();
        return true;
    }
    if app(|a| a.pane_drag.as_ref().is_some_and(|d| d.hwnd == h)) {
        track_pane_drag();
    }
    let (drag, press) = app(|a| {
        let mine = a.pane_drag.as_ref().is_some_and(|d| d.hwnd == h) || a.pane_press.as_ref().is_some_and(|p| p.hwnd == h);
        if !mine {
            return (None, None);
        }
        (a.pane_drag.take(), a.pane_press.take())
    });
    if drag.is_none() && press.is_none() {
        return false;
    }
    hide_overlay();
    _ = ReleaseCapture();
    if let Some(d) = drag {
        clear_drag_hint();
        if let Some((onto, zone)) = d.hot {
            if d.copy {
                commit_pane_copy(d.from, onto, zone);
            } else {
                commit_pane_move(d.from, onto, zone);
            }
        }
        return true;
    }
    if let Some(p) = press {
        // The press already made this pane active (pre_translate). A tab or the empty strip
        // still has to do what a click did before the drag threshold existed.
        match p.grab {
            PaneGrab::Tab(k) => {
                pick_pane_tab(p.pane, k);
                focus_view(p.pane);
            }
            PaneGrab::Strip => focus_view(p.pane),
            PaneGrab::Header => {}
        }
    }
    true
}

/// Esc, or a capture stolen by something else. False when nothing was being dragged.
pub(crate) unsafe fn cancel_pane_drag() -> bool {
    let had = app(|a| {
        let had = a.pane_drag.is_some() || a.pane_press.is_some() || a.pane_tab_drag.is_some();
        a.pane_drag = None;
        a.pane_press = None;
        a.pane_tab_drag = None;
        a.drag_hint.clear();
        had
    });
    if !had {
        return false;
    }
    hide_overlay();
    _ = ReleaseCapture();
    update_status();
    true
}

fn clear_drag_hint() {
    let changed = app(|a| {
        if a.drag_hint.is_empty() {
            return false;
        }
        a.drag_hint.clear();
        true
    });
    if changed {
        unsafe { update_status() };
    }
}

/// Move the live panes, then the active tab's grid. Other tabs and other windows stay as they are.
unsafe fn commit_pane_move(from: usize, onto: usize, zone: DropZone) {
    let Some(plan) = app(|a| {
        if a.zoom.is_some() {
            return None;
        }
        match drag_plan(false, &a.s.heights, &a.s.colr, &a.s.rowr, from, onto, zone) {
            Some(DragPlan::Move(shift)) => Some(shift),
            _ => None,
        }
    }) else {
        return;
    };
    app(|a| a.focus_req = HWND::default());
    let n = app(|a| a.s.panes().min(MAXP));
    for i in 0..n {
        save_view(i);
    }
    let perm = plan.perm;
    let slots = app(|a| perm.map(|k| a.panes.get(k).and_then(|p| p.as_ref()).map(|p| p.slot.clone())));
    {
        let mut f = FILTERS.lock().unwrap_or_else(|e| e.into_inner());
        let old = f.clone();
        for (j, &k) in perm.iter().enumerate() {
            f[j] = old[k].clone();
            if let Some(s) = &slots[j] {
                s.store(j, Ordering::SeqCst);
            }
        }
    }
    let revs = SEL_REV.each_ref().map(|r| r.load(Ordering::Relaxed));
    for (j, &k) in perm.iter().enumerate() {
        SEL_REV[j].store(revs[k], Ordering::Relaxed);
    }
    let focus = app(|a| {
        let old_f = a.live_filters.clone();
        for (j, &k) in perm.iter().enumerate() {
            a.live_filters[j] = old_f[k].clone();
        }
        let mut taken: Vec<Option<Pane>> = a.panes.iter_mut().map(Option::take).collect();
        a.panes = perm.iter().map(|&k| taken.get_mut(k).and_then(Option::take)).collect();
        let mut base = a.base.take();
        permute_in(&mut a.s, base.as_mut(), &perm);
        a.base = base;
        let tab = a.s.tab;
        let order: [u8; MAXP] = std::array::from_fn(|i| perm[i] as u8);
        if let Some(b) = a.tab_base.get_mut(tab) {
            reorder_tab_slots(&mut b.grid, &order);
        }
        let to = |k: &mut usize| *k = perm.iter().position(|&p| p == *k).unwrap_or(*k);
        if let Some(d) = a.drop_open.as_mut() {
            to(&mut d.1);
        }
        if let Some(m) = a.matcher.as_mut() {
            to(&mut m.pane);
        }
        if let Some(k) = a.kids.as_mut() {
            to(&mut k.i);
        }
        to(&mut a.peek_pane);
        a.zoom = None;
        a.status_key.clear();
        a.s.heights = plan.heights;
        a.s.colr = plan.colr;
        a.s.rowr = plan.rowr;
        let count = a.s.panes().max(1);
        a.s.active = plan.focus.min(count - 1);
        // The active tab stores this grid. A later switch or save reads it back.
        a.s.anchor();
        let _ = pull_dirty(a);
        a.s.active
    });
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    relayout();
    rename_fields();
    redraw_all();
    refresh_save_button();
    note_save();
    set_active(focus);
    focus_view(focus);
}

/// Copy the current pane tab. An edge splits, including this pane's own edge. The centre adds a pane tab in that cell.
unsafe fn commit_pane_copy(from: usize, onto: usize, zone: DropZone) {
    let drop = app(|a| {
        if a.zoom.is_some() {
            return None;
        }
        match drag_plan(true, &a.s.heights, &a.s.colr, &a.s.rowr, from, onto, zone) {
            Some(DragPlan::Copy(d)) => Some(d),
            _ => None,
        }
    });
    match drop {
        Some(CopyDrop::Split { heights, colr, rowr, slots }) => commit_split_copy(from, heights, colr, rowr, slots),
        Some(CopyDrop::Tab) => commit_tab_copy(from, onto),
        _ => {}
    }
}

unsafe fn commit_split_copy(from: usize, heights: Vec<usize>, colr: Vec<f64>, rowr: Vec<Vec<f64>>, slots: Vec<Option<usize>>) {
    let Some((perm, home, fresh, focus)) = app(|a| {
        let placed = place_grid(&slots, None, &a.home)?;
        let fresh = placed.fresh_old?;
        let focus = slots.iter().position(|s| s.is_none())?;
        Some((placed.perm, placed.home, fresh, focus))
    }) else {
        return;
    };
    // The new cell is a new Explorer browser: same folder, view, sort and columns, empty history.
    commit_layout(heights, Some((perm, home)), Some((from, fresh)), None, Some(focus), Some((colr, rowr)));
}

/// The folder on display becomes a pane tab on `dst`. That cell keeps its view. A failed browse puts the tabs back.
unsafe fn commit_tab_copy(src: usize, dst: usize) {
    save_view(src);
    // The destination cell is the one leaving its folder. Save it before `copy_pane_tab`
    // points `paths` at the copied folder.
    save_view(dst);
    let Some((before_on, before_f, before_c, before_l, before_m, before_p, armed, to)) = app(|a| {
        if src >= a.s.panes() || dst >= a.s.panes() {
            return None;
        }
        a.s.ensure_pane_tabs();
        let before_on = a.s.pane_strip_on(dst);
        let before_f = a.s.pane_folders[dst].clone();
        let before_c = a.s.pane_cur[dst];
        let before_l = a.s.pane_locks[dst].clone();
        let before_m = a.s.pane_names[dst].clone();
        let before_p = a.s.paths.get(dst).cloned().unwrap_or_default();
        let to = a.s.copy_pane_tab(src, dst);
        let armed = to.is_some() && a.panes.get_mut(dst).and_then(|p| p.as_mut()).is_some_and(|p| arm_tab_left(p, &before_p));
        a.s.anchor();
        let _ = pull_dirty(a);
        Some((before_on, before_f, before_c, before_l, before_m, before_p, armed, to))
    }) else {
        return;
    };
    if let Some(path) = to {
        if !go_near(dst, &path) {
            app(|a| {
                if dst < a.s.pane_folders.len() {
                    a.s.pane_folders[dst] = before_f;
                }
                if dst < a.s.pane_cur.len() {
                    a.s.pane_cur[dst] = before_c;
                }
                if dst < a.s.pane_locks.len() {
                    a.s.pane_locks[dst] = before_l;
                }
                if dst < a.s.pane_names.len() {
                    a.s.pane_names[dst] = before_m;
                }
                if dst < a.s.paths.len() {
                    a.s.paths[dst] = before_p;
                }
                if armed {
                    if let Some(p) = a.panes.get_mut(dst).and_then(|p| p.as_mut()) {
                        p.tab_left = None;
                    }
                }
                a.s.anchor();
                let _ = pull_dirty(a);
            });
            _ = MessageBeep(MB_ICONWARNING);
        }
    }
    apply_pane_tab_chrome(dst, before_on);
    refresh_save_button();
    note_save();
    set_active(dst);
    focus_view(dst);
}
