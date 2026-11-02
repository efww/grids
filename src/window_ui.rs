use crate::*;

// ---------- window procs ----------

pub(crate) fn xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

pub(crate) unsafe fn cursor_for(h: HWND) -> Option<Hit> {
    let mut pt = POINT::default();
    _ = GetCursorPos(&mut pt);
    _ = ScreenToClient(h, &mut pt);
    app(|a| hit(&geo(a), pt.x, pt.y))
}

// Marks the splitter under the mouse (paint_main); only the old and new lines are repainted.
pub(crate) unsafe fn gap_hover(h: HWND, d: Option<Hit>) {
    let (old, lines) = app(|a| {
        let old = std::mem::replace(&mut a.gap_hot, d);
        let g = geo(a);
        (old, [old, d].map(|k| k.and_then(|k| gap_line(&g, k, 2 * a.dpi / 96))))
    });
    if old == d {
        return;
    }
    for r in lines.iter().flatten() {
        _ = InvalidateRect(Some(h), Some(r), false);
    }
    if old.is_none() {
        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
        _ = TrackMouseEvent(&mut t);
    }
}

pub(crate) extern "system" fn main_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // While the user drags the window edge, lay the panes out once at the end (same cost reason as splitters).
            WM_SIZE => {
                let sizing = app(|a| {
                    if wp.0 as u32 != SIZE_MINIMIZED {
                        a.was_max = wp.0 as u32 == SIZE_MAXIMIZED;
                    }
                    a.sizing
                });
                if !sizing {
                    relayout();
                    redraw_all(); // maximize / restore
                }
                // back from minimized: WM_ACTIVATE came while still minimized and left focus alone
                let f = GetFocus();
                if wp.0 as u32 != SIZE_MINIMIZED && !app(|a| a.focus_wait) && GetActiveWindow() == h && (f.is_invalid() || f == h) {
                    restore_focus();
                }
            }
            // WA_ACTIVE (Alt+Tab, taskbar, a popup closing, a second grids.exe): focus back where it was.
            // WA_CLICKACTIVE: as before, so the click lands on what was under it (crumbs stay crumbs); TIMER_FOCUS
            // looks again once that click is handled.
            WM_ACTIVATE => {
                let (state, min) = ((wp.0 & 0xffff) as u32, wp.0 >> 16 & 0xffff != 0);
                if state == WA_INACTIVE {
                    keep_focus(h);
                } else if state == WA_ACTIVE && !min {
                    restore_focus();
                    return LRESULT(0);
                } else if state == WA_CLICKACTIVE && !min {
                    app(|a| a.focus_wait = true);
                    SetTimer(Some(h), TIMER_FOCUS, 10, None);
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_MOUSEACTIVATE => {
                app(|a| a.click_nc = (lp.0 & 0xffff) as u32 != HTCLIENT);
                return DefWindowProcW(h, msg, wp, lp);
            }
            // minimizing may drop focus before WA_INACTIVE comes
            WM_SYSCOMMAND => {
                if wp.0 as u32 & 0xfff0 == SC_MINIMIZE {
                    keep_focus(h);
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            // a list, edit, button or the sidebar took the click: nothing to do. Title bar: back where it was;
            // empty client space (crumb gaps, top bar, status line): the active list. Waits out drags and menus.
            WM_TIMER if wp.0 == TIMER_FOCUS => {
                if !GetCapture().is_invalid() || GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 {
                    return LRESULT(0);
                }
                _ = KillTimer(Some(h), TIMER_FOCUS);
                let nc = app(|a| {
                    a.focus_wait = false;
                    std::mem::take(&mut a.click_nc)
                });
                let f = GetFocus();
                if GetActiveWindow() == h && (f.is_invalid() || f == h) {
                    if nc {
                        restore_focus();
                    } else {
                        focus_view(app(|a| a.s.active));
                    }
                }
            }
            WM_ENTERSIZEMOVE => {
                app(|a| a.sizing = true);
                kids_cancel(); // its '›' is about to move
            }
            WM_EXITSIZEMOVE => {
                app(|a| a.sizing = false);
                refresh_drives(); // one that came or went mid-resize
                relayout();
                redraw_all();
            }
            WM_PAINT => paint_main(h),
            WM_ERASEBKGND => return LRESULT(1), // paint_main covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_TIMER if wp.0 == TIMER_REVEAL => {
                _ = KillTimer(Some(h), TIMER_REVEAL);
                reveal_tree_selection();
                let left = app(|a| {
                    a.reveal_tries = a.reveal_tries.saturating_sub(1);
                    a.reveal_tries
                });
                if left > 0 {
                    SetTimer(Some(h), TIMER_REVEAL, if left == 2 { 330 } else { 550 }, None);
                }
            }
            // A drive came or went (any kind). Only queued: no disk calls while the system waits on the reply,
            // and TRUE grants a removal query.
            WM_DEVICECHANGE => {
                _ = PostMessageW(Some(h), WM_DRIVES, WPARAM(0), LPARAM(0));
                return LRESULT(1);
            }
            // network drives and subst send no device message: look again whenever Grids comes to the front
            WM_ACTIVATEAPP => {
                if wp.0 != 0 {
                    _ = PostMessageW(Some(h), WM_DRIVES, WPARAM(0), LPARAM(0));
                    update_nudge(); // and a release check an hour or more old is due again
                }
            }
            // Windows light/dark (or the accent) changed. Only queued: nothing is drawn inside the broadcast.
            WM_SETTINGCHANGE => {
                if lp.0 != 0 && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                    _ = PostMessageW(Some(h), WM_THEME_SYNC, WPARAM(0), LPARAM(0));
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_THEME_SYNC => theme_sync(),
            WM_UPDATE => update_msg(h, wp.0),
            // mid-drag or mid-resize, end_drag / WM_EXITSIZEMOVE pick it up instead
            WM_DRIVES => {
                if !app(|a| a.drag.is_some() || a.sizing) && refresh_drives() {
                    relayout();
                    redraw_all();
                }
            }
            WM_TIMER if wp.0 == TIMER_EV_SELECT => ev_pick_tick(h),
            WM_TIMER if wp.0 == TIMER_STATUS => {
                ev_watch(); // Everything started or quit: its button comes or goes
                update_status();
                check_dirty();
                autosave();
            }
            WM_SETCURSOR if wp.0 == h.0 as usize && (lp.0 & 0xffff) as u32 == HTCLIENT => {
                let Some(d) = app(|a| a.drag).or_else(|| cursor_for(h)) else {
                    return DefWindowProcW(h, msg, wp, lp);
                };
                let c = match d {
                    Hit::Side | Hit::Col(_) => IDC_SIZEWE,
                    Hit::Row(..) => IDC_SIZENS,
                };
                SetCursor(LoadCursorW(None, c).ok());
                return LRESULT(1);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| hit(&geo(a), x, y)) {
                    gap_hover(h, None);
                    app(|a| a.drag = Some(d));
                    SetCapture(h);
                }
            }
            WM_MOUSELEAVE => gap_hover(h, None),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| a.drag.is_none().then(|| hit(&geo(a), x, y))) {
                    gap_hover(h, d);
                }
                let moved = app(|a| {
                    let d = a.drag?;
                    let g = geo(a);
                    let min = 60 * a.dpi / 96;
                    match d {
                        Hit::Side => a.s.side = x - g.pad - g.gap / 2,
                        Hit::Col(k) => drag_ratio(&mut a.s.colr, k, x - g.gap / 2, g.gx, g.w - g.pad - g.gx, g.gap, min),
                        Hit::Row(c, k) => drag_ratio(&mut a.s.rowr[c], k, y - g.gap / 2, g.top, g.bot - g.top, g.gap, min),
                    }
                    a.s.side = kept_side(a.s.tree, a.s.side, geo(a).side);
                    Some(())
                });
                if moved.is_some() {
                    show_bars(); // panes stay put until release: relaying out 16 Explorer views per mouse move is ~120ms
                }
            }
            WM_LBUTTONUP => {
                _ = ReleaseCapture();
                end_drag();
            }
            WM_CAPTURECHANGED => end_drag(),
            WM_LBUTTONDBLCLK => {
                let (x, y) = xy(lp);
                let d = app(|a| hit(&geo(a), x, y));
                app(|a| match d {
                    Some(Hit::Side) => a.s.side = 240 * a.dpi / 96,
                    Some(Hit::Col(_)) => a.s.colr = even(a.s.heights.len()),
                    Some(Hit::Row(c, _)) => a.s.rowr[c] = even(a.s.heights[c]),
                    None => {}
                });
                relayout();
                redraw_all();
            }
            WM_COMMAND => match wp.0 & 0xffff {
                id if id == ID_TB_LAYOUT as usize => open_grid(),
                id if id == ID_TB_SAVE as usize => save_current(),
                id if id == ID_TB_LOAD as usize => open_load(),
                id if id == ID_TB_FAV as usize => open_favs(),
                id if id == ID_TB_EV as usize => open_ev(),
                id if id == ID_TB_UPDATE as usize => update_click(),
                ID_UPDATES => toggle_updates(),
                id if id == ID_TB_THEME as usize => {
                    app(|a| (a.s.dark, a.s.follow) = (!a.s.dark, false)); // picked by hand: stop following Windows
                    retheme();
                }
                id if id == ID_TB_VIEW as usize => popup_menu(app(|a| a.view_menu), ID_TB_VIEW),
                id if (ID_DRIVE as usize..ID_DRIVE as usize + 26).contains(&id) => {
                    let i = app(|a| a.s.active);
                    if navigate(i, &format!("{}:\\", (b'A' + (id - ID_DRIVE as usize) as u8) as char)) {
                        focus_view(i);
                    } else {
                        _ = MessageBeep(MB_ICONWARNING); // no medium, or gone since
                    }
                }
                ID_TREE => toggle_tree(),
                ID_HIDDEN => toggle_hidden(),
                ID_EXT => toggle_extensions(),
                ID_DBLUP => app(|a| a.s.dblup ^= true),
                ID_TABS => toggle_tabs(),
                ID_FOLLOW => {
                    if app(|a| {
                        a.s.follow ^= true;
                        a.s.follow
                    }) {
                        _ = PostMessageW(Some(h), WM_THEME_SYNC, WPARAM(0), LPARAM(0));
                    }
                }
                // flips what is really registered, so after a failed start one click retries
                ID_HOTKEY => {
                    let was = app(|a| a.hot_on);
                    let now = hotkey_flip(was, |on| set_hotkey(h, on));
                    app(|a| {
                        (a.hot_on, a.s.hotkey) = (now, now);
                        if !was && !now {
                            let text = tr("Win+Alt+E is already used by another app", "Win+Alt+E는 다른 앱이 이미 쓰고 있음");
                            a.note = Some((text.into(), Instant::now() + Duration::from_secs(3)));
                        }
                    });
                }
                ID_DLGJUMP => {
                    let on = app(|a| {
                        a.s.dlgjump ^= true;
                        a.s.dlgjump
                    });
                    dialog_jump::enable(on);
                }
                ID_LANG => set_language(!KO.load(Ordering::Relaxed)),
                ID_HELP => open_help(),
                ID_ABOUT => about(h),
                id @ ID_ACCENT..=1405 => set_accent(pal().accents[id - ID_ACCENT].1),
                ID_ACCENT_PICK => pick_accent(),
                ID_KEYS => open_keys(),
                ID_GATHER => gather_explorer(),
                _ => {}
            },
            WM_INITMENUPOPUP => {
                let (menu, dblup, tree, accent, hot, follow, jump) =
                    app(|a| (a.view_menu, a.s.dblup, a.s.tree, a.s.accent_now(), a.hot_on, a.s.follow, a.s.dlgjump));
                let check = |id: usize, on: bool| CheckMenuItem(menu, id as u32, (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0);
                check(ID_TREE, tree);
                check(ID_FOLLOW, follow);
                check(ID_HIDDEN, hidden());
                check(ID_EXT, extensions());
                check(ID_DBLUP, dblup);
                check(ID_TABS, TABS_ON.load(Ordering::Relaxed));
                check(ID_HOTKEY, hot);
                check(ID_DLGJUMP, jump);
                check(ID_UPDATES, app(|a| a.s.updates));
                check(ID_LANG, KO.load(Ordering::Relaxed));
                for (k, (_, c)) in pal().accents.iter().enumerate() {
                    check(ID_ACCENT + k, *c == accent);
                }
            }
            WM_COPYDATA => {
                let cd = &*(lp.0 as *const CopyData);
                if cd.dw != COPY_ID || cd.lp.is_null() {
                    return LRESULT(0);
                }
                // copy first: the sender's buffer is only valid during this call
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(cd.lp as *const u16, cd.cb as usize / 2));
                open_args(text.split('\n').map(String::from).collect());
                return LRESULT(1);
            }
            WM_DPICHANGED => {
                kids_cancel();
                let dpi = (wp.0 & 0xffff) as i32;
                app(|a| {
                    a.s.side = a.s.side * dpi / a.dpi;
                    a.dpi = dpi;
                });
                apply_font();
                fit_load();
                let r = &*(lp.0 as *const RECT);
                _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            WM_CLOSE => {
                save_session();
                _ = DestroyWindow(h);
            }
            // Windows restart / sign-out: no WM_CLOSE comes. WM_QUERYENDSESSION stays with DefWindowProcW (never blocks).
            WM_ENDSESSION if wp.0 != 0 => save_session(),
            WM_HOTKEY if wp.0 == HOTKEY_ID as usize => hotkey_pressed(h),
            WM_HOTKEY if wp.0 == dialog_jump::DLG_HOTKEY_ID as usize => dialog_jump::on_hotkey(),
            WM_TIMER if wp.0 == dialog_jump::TIMER_DLG_RETRY || wp.0 == dialog_jump::TIMER_DLG_RESTORE => dialog_jump::on_timer(wp.0),
            WM_DESTROY => {
                _ = KillTimer(Some(h), TIMER_STATUS);
                dialog_jump::shutdown();
                if app(|a| a.hot_on) {
                    set_hotkey(h, false);
                }
                for i in 0..MAXP {
                    destroy_pane(i);
                }
                app(|a| a.tree = None);
                PostQuitMessage(0);
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

pub(crate) extern "system" fn pane_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let nav = |flags: u32| {
            if let Some(i) = pane_of(h) {
                set_active(i);
                browse(i, flags);
                focus_view(i);
            }
        };
        let on_kind_icon = |x: i32, y: i32| {
            let dpi = app(|a| a.dpi);
            let mut rc = RECT::default();
            _ = GetClientRect(h, &mut rc);
            filter_icon_rect(pane_geo(rc.right, rc.bottom, dpi).filt, 24 * dpi / 96)
                .is_some_and(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
        };
        match msg {
            WM_TIMER if wp.0 == TIMER_SETTLE => settle_cols(h),
            WM_SIZE => layout_pane(h),
            WM_LBUTTONDBLCLK if pane_of(h).is_some_and(|i| on_strip(h, i, lp)) => {
                // empty strip: a new tab; on a tab it is just a second click
                let (x, y) = ((lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
                if let Some(i) = pane_of(h) {
                    let mut rc = RECT::default();
                    _ = GetClientRect(h, &mut rc);
                    let dpi = app(|a| a.dpi);
                    let n = app(|a| a.s.tab_list(i).len());
                    if matches!(tab_hit(&pane_geo(rc.right, rc.bottom, dpi), n, x, y, dpi), Some(TabHit::Strip)) {
                        new_tab(i);
                    } else {
                        tab_mouse(h, i, x, y, false);
                    }
                }
            }
            WM_MBUTTONUP if pane_of(h).is_some_and(|i| on_strip(h, i, lp)) => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
                if let Some(i) = pane_of(h) {
                    tab_mouse(h, i, x, y, true);
                }
            }
            WM_LBUTTONDOWN if pane_of(h).is_some_and(|i| on_strip(h, i, lp)) => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
                if let Some(i) = pane_of(h) {
                    tab_mouse(h, i, x, y, false);
                }
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
                let addr = addr_rect(h, app(|a| a.dpi));
                let on_addr = x >= addr.left && x < addr.right && y >= addr.top && y < addr.bottom;
                if let Some(i) = pane_of(h).filter(|_| !on_addr && !on_kind_icon(x, y)) {
                    set_active(i);
                    toggle_zoom();
                }
            }
            // filter icon: file types; address bar crumbs (set_active already ran in pre_translate)
            WM_LBUTTONDOWN => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
                match pane_of(h) {
                    Some(i) if on_kind_icon(x, y) => kind_menu(i),
                    Some(i) => crumb_click(h, i, x, y),
                    None => {}
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
                let k = pane_of(h).and_then(|i| crumb_at(h, i, x, y)).flatten().map(|(k, ..)| k);
                crumb_hover(h, k);
            }
            WM_MOUSELEAVE => {
                if app(|a| a.crumb_hot.is_some_and(|(w, _)| w == h)) {
                    crumb_hover(h, None);
                }
            }
            WM_PAINT => paint_pane(h),
            // folder dropped on the top bar: still this pane's? then open it here, as ⇄ does
            WM_DROP_OPEN => {
                let queued = app(|a| {
                    let mine = a.drop_open.as_ref().is_some_and(|(w, i, _)| *w == h && a.panes[*i].as_ref().is_some_and(|p| p.hwnd == h));
                    if mine {
                        a.drop_open.take()
                    } else {
                        None
                    }
                });
                if let Some((_, i, item)) = queued {
                    browse_item(i, &item);
                    set_active(i);
                    focus_view(i);
                }
            }
            WM_KIDS => kids_show(h, wp.0),
            // folder icon looked up: only the icon's square is repainted, nothing moves
            WM_PANE_ICON => {
                let (changed, dpi) = app(|a| {
                    let p = a.panes.iter_mut().flatten().find(|p| p.hwnd == h);
                    let new = p.and_then(|p| take_icon(p.ticket, wp.0 as u32, lp.0 as i32).filter(|&n| n != p.icon).map(|n| p.icon = n));
                    (new.is_some(), a.dpi)
                });
                if let Some(r) = addr_icon(addr_rect(h, dpi), dpi).filter(|_| changed) {
                    _ = InvalidateRect(Some(h), Some(&r), false);
                }
            }
            WM_ERASEBKGND => return LRESULT(1), // paint_pane covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32) {
                // pill gets an accent outline while its edit has focus
                (ID_EDIT, EN_SETFOCUS | EN_KILLFOCUS) => sync_addr(h),
                (ID_FILTER, EN_SETFOCUS | EN_KILLFOCUS) => _ = InvalidateRect(Some(h), None, false),
                (ID_BACK, _) => nav(SBSP_NAVIGATEBACK),
                (ID_FWD, _) => nav(SBSP_NAVIGATEFORWARD),
                (ID_UP, _) => nav(SBSP_PARENT),
                (ID_OPEN_IN, _) => {
                    if let Some(i) = pane_of(h) {
                        open_in_menu(h, i);
                    }
                }
                (ID_FAV, _) => {
                    if let Some(i) = pane_of(h) {
                        toggle_fav(i);
                    }
                }
                // typing in the filter box applies after a short pause
                (ID_FILTER, EN_CHANGE) => _ = SetTimer(Some(h), TIMER_FILTER, 250, None),
                _ => {}
            },
            WM_TIMER if wp.0 == TIMER_FILTER => {
                _ = KillTimer(Some(h), TIMER_FILTER);
                if let Some(i) = pane_of(h) {
                    apply_filter(i);
                }
            }
            // right-click on ← / →: recent folders; anywhere else as before
            WM_CONTEXTMENU if [ID_BACK, ID_FWD].iter().any(|&id| GetDlgItem(Some(h), id).is_ok_and(|b| b.0 as usize == wp.0)) => {
                if let Some(i) = pane_of(h) {
                    set_active(i);
                    recent_menu(i);
                }
            }
            // right-click on the header itself (not its buttons or fields): colour label; on a tab: that tab's menu
            WM_CONTEXTMENU if wp.0 == h.0 as usize => {
                let mut p = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: (lp.0 >> 16 & 0xffff) as i16 as i32 };
                let screen = p;
                _ = ScreenToClient(h, &mut p);
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                let dpi = app(|a| a.dpi);
                let pg = pane_geo(rc.right, rc.bottom, dpi);
                let tab = |i: usize| match tab_hit(&pg, app(|a| a.s.tab_list(i).len()), p.x, p.y, dpi) {
                    Some(TabHit::Tab(k) | TabHit::Close(k)) => Some(k),
                    _ => None,
                };
                match pane_of(h).filter(|_| lp.0 != -1).map(|i| (i, tab(i))) {
                    Some((i, Some(k))) => tab_menu(h, i, k, screen),
                    Some((i, None)) if p.y < pg.view.top => tint_menu(h, i, screen),
                    _ => return DefWindowProcW(h, msg, wp, lp),
                }
            }
            // mouse side buttons
            WM_APPCOMMAND => match (lp.0 >> 16) & 0x0fff {
                1 => nav(SBSP_NAVIGATEBACK),
                2 => nav(SBSP_NAVIGATEFORWARD),
                _ => return DefWindowProcW(h, msg, wp, lp),
            },
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// ---------- Win+Alt+E: Grids to the front from any app ----------

pub(crate) const HOTKEY_ID: i32 = 1;

// The new real state after a menu click: reg(wanted) registers / unregisters and says whether it is on now.
pub(crate) fn hotkey_flip(on: bool, reg: impl FnOnce(bool) -> bool) -> bool {
    reg(!on)
}

pub(crate) unsafe fn set_hotkey(main: HWND, on: bool) -> bool {
    if on {
        RegisterHotKey(Some(main), HOTKEY_ID, MOD_WIN | MOD_ALT | MOD_NOREPEAT, 'E' as u32).is_ok()
    } else {
        _ = UnregisterHotKey(Some(main), HOTKEY_ID);
        false
    }
}

// Front with the list ready for the keyboard; pressed while Grids is already in front: minimize.
// A modal dialog owned by main (save, colour, About) has disabled it: bring that forward, never minimize.
pub(crate) unsafe fn hotkey_pressed(main: HWND) {
    let iconic = IsIconic(main).as_bool();
    if !IsWindowEnabled(main).as_bool() {
        if iconic {
            _ = ShowWindow(main, SW_RESTORE);
        }
        let p = GetLastActivePopup(main);
        if p != main {
            _ = SetForegroundWindow(p);
        }
    } else if GetAncestor(GetForegroundWindow(), GA_ROOTOWNER) == main && !iconic {
        keep_focus(main);
        _ = ShowWindow(main, SW_MINIMIZE);
    } else {
        if iconic {
            _ = ShowWindow(main, SW_RESTORE);
        }
        // refused (the taskbar button flashes instead): leave focus where it is
        if SetForegroundWindow(main).as_bool() && GetForegroundWindow() == main {
            focus_view(app(|a| a.s.active));
        }
    }
}

// The README on GitHub, in the UI language.
pub(crate) unsafe fn open_help() {
    let url = format!("{HOME}/blob/main/{}", tr("README.md", "README.ko.md"));
    ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
}

pub(crate) unsafe fn about(owner: HWND) {
    let text = format!(
        "Grids {}\n{}\n\n{}\n{HOME}\n\n{}",
        env!("CARGO_PKG_VERSION"),
        tr("Fast multi-pane file manager for Windows", "Windows용 빠른 다중 패널 파일 관리자"),
        tr("MIT License · © 2026 efww", "MIT 라이선스 · © 2026 efww"),
        tr("Open the project page?", "프로젝트 페이지를 열까요?"),
    );
    if MessageBoxW(Some(owner), &HSTRING::from(text), &HSTRING::from(tr("About Grids", "Grids 정보")), MB_YESNO | MB_ICONINFORMATION) == IDYES {
        ShellExecuteW(None, w!("open"), &HSTRING::from(HOME), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

// Toolbar buttons: id, accessible name, tooltip.
pub(crate) fn toolbar_tips() -> [(i32, &'static str, &'static str); 7] {
    [
        (ID_TB_LAYOUT, "", tr("Layout: click = even grid, drag = rows per column", "레이아웃: 클릭 = 균등, 끌기 = 열마다 칸 수")),
        (ID_TB_SAVE, tr("Save session", "세션 저장"), ""), // tooltip set by refresh_save_button
        (ID_TB_LOAD, tr("Load", "불러오기"), tr("Load a session", "세션 불러오기")),
        (ID_TB_VIEW, tr("View", "보기"), tr("View: hidden files · extensions · colours · language · help", "보기: 숨김 파일 · 확장명 · 선택 색 · 언어 · 도움말")),
        (ID_TB_FAV, tr("Favourites", "즐겨찾기"), tr("Favourite folders (Ctrl+B; open in the active pane)", "즐겨찾기 폴더 (Ctrl+B, 선택한 패널에서 열기)")),
        (ID_TB_THEME, tr("Theme", "테마"), tr("Light / dark theme", "라이트 / 다크 테마")),
        (ID_TB_EV, tr("Search with Everything", "Everything으로 검색"), tr("Search with Everything (Ctrl+Shift+F)", "Everything으로 검색 (Ctrl+Shift+F)")),
    ]
}

// ---------- drive buttons (top toolbar) ----------

pub(crate) fn mask_letters(mask: u32) -> Vec<u8> {
    (0..26u8).filter(|k| mask >> k & 1 != 0).map(|k| b'A' + k).collect()
}

// Drive letter of a folder path ("d:\x" -> D); UNC, shell and search locations have none.
pub(crate) fn drive_of(path: &str) -> Option<u8> {
    match path.as_bytes() {
        [c, b':', ..] if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

// The drive's kind only: reading a volume label can wait on a sleeping disk or a dead share.
pub(crate) fn drive_tip(c: u8, kind: u32) -> String {
    let name = match kind {
        2 => tr("Removable drive", "이동식 드라이브"),
        3 => tr("Local disk", "로컬 디스크"),
        4 => tr("Network drive", "네트워크 드라이브"),
        5 => tr("CD/DVD drive", "CD/DVD 드라이브"),
        _ => tr("Drive", "드라이브"),
    };
    format!("{name} ({}:) — {}", c as char, tr("open in the active pane", "지금 패널에서 열기"))
}

// Makes / removes the buttons of drive letters that came or went; true if any did. The caller lays them out.
pub(crate) unsafe fn refresh_drives() -> bool {
    let mask = GetLogicalDrives();
    let Some((main, hover, gone, added)) = app(|a| {
        let old = std::mem::replace(&mut a.drive_mask, mask);
        if old == mask {
            return None;
        }
        let (keep, gone): (Vec<_>, Vec<_>) = a.drives.drain(..).partition(|d| mask >> (d.0 - b'A') & 1 != 0);
        a.drives = keep;
        Some((a.main, a.hover, gone, mask_letters(mask & !old)))
    }) else {
        return false;
    };
    for (_, _, h) in gone {
        if hover == h {
            set_hover(HWND::default());
        }
        tip_text(h, "", TTM_DELTOOLW);
        _ = DestroyWindow(h);
    }
    for c in added {
        let kind = GetDriveTypeW(&HSTRING::from(format!("{}:\\", c as char)));
        let name = HSTRING::from(format!("{}:", c as char)); // also the accessible name
        let h = child(main, w!("BUTTON"), PCWSTR(name.as_ptr()), BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0), ID_DRIVE + (c - b'A') as i32, HFONT::default());
        add_tip(h, &drive_tip(c, kind));
        app(|a| {
            a.drives.push((c, kind, h));
            a.drives.sort_by_key(|d| d.0);
        });
    }
    true
}

// The underline follows the active pane's drive: on the 300ms status tick, and at once on a pane switch.
pub(crate) unsafe fn mark_drive() {
    let Some(hs) = app(|a| {
        let m = drive_of(&a.s.paths[a.s.active]);
        (a.drive_mark != m).then(|| {
            a.drive_mark = m;
            a.drives.iter().map(|d| d.2).collect::<Vec<_>>()
        })
    }) else {
        return;
    };
    for h in hs {
        _ = InvalidateRect(Some(h), None, false);
    }
}

// Switches the UI language now: menu, toolbar tooltips, and the panes (rebuilt with their tooltips).
pub(crate) unsafe fn set_language(ko: bool) {
    KO.store(ko, Ordering::Relaxed);
    let (main, old) = app(|a| {
        a.s.ko = ko;
        (a.main, a.view_menu)
    });
    if let Ok(m) = make_menus() {
        app(|a| a.view_menu = m);
        _ = DestroyMenu(old);
    }
    for (id, _, hint) in toolbar_tips() {
        if let Ok(b) = GetDlgItem(Some(main), id) {
            tip_text(b, hint, TTM_UPDATETIPTEXTW);
        }
    }
    for (c, kind, h) in app(|a| a.drives.clone()) {
        tip_text(h, &drive_tip(c, kind), TTM_UPDATETIPTEXTW);
    }
    refresh_save_button();
    update_tip();
    app(|a| a.status_key.clear());
    apply_theme();
}

// The view popup opened from the top toolbar (no classic menu bar).
pub(crate) unsafe fn make_menus() -> Result<HMENU> {
    let view = CreatePopupMenu()?;
    let item = |menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str| AppendMenuW(menu, flags, id, &HSTRING::from(text));
    item(view, MF_STRING, ID_TREE, tr("Folder tree\tCtrl+Shift+E", "폴더 트리\tCtrl+Shift+E"))?;
    item(view, MF_STRING, ID_TABS, tr("Tabs in panes", "패널 탭"))?;
    item(view, MF_STRING, ID_HIDDEN, tr("Show hidden files\tCtrl+H", "숨김 파일 표시\tCtrl+H"))?;
    item(view, MF_STRING, ID_GATHER, tr("Open folders from Explorer windows\tCtrl+Shift+O", "탐색기 창의 폴더 가져오기\tCtrl+Shift+O"))?;
    item(view, MF_STRING, ID_EXT, tr("Show file name extensions", "파일 확장명 표시"))?;
    item(view, MF_STRING, ID_DBLUP, tr("Double-click empty space: parent folder", "빈 곳 더블클릭: 위 폴더로"))?;
    item(view, MF_STRING, ID_HOTKEY, tr("Bring Grids to front from anywhere\tWin+Alt+E", "어디서나 Grids 불러오기\tWin+Alt+E"))?;
    let jump = tr("Jump to Grids folders from file dialogs\tCtrl+Alt+G", "파일 대화 상자에서 Grids 폴더로 이동\tCtrl+Alt+G");
    item(view, MF_STRING, ID_DLGJUMP, jump)?;
    item(view, MF_STRING, ID_FOLLOW, tr("Follow Windows light/dark", "Windows 라이트·다크 따르기"))?;
    let colors = CreatePopupMenu()?;
    for (k, (name, _)) in LIGHT.accents.iter().enumerate() {
        item(colors, MF_STRING, ID_ACCENT + k, tr(ACCENT_EN[k], name))?;
    }
    AppendMenuW(colors, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(colors, MF_STRING, ID_ACCENT_PICK, tr("Custom...", "직접 선택..."))?;
    item(view, MF_POPUP, colors.0 as usize, tr("Selection colour", "선택 색"))?;
    AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(view, MF_STRING, ID_LANG, "한국어 (Korean)")?;
    AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(view, MF_STRING, ID_KEYS, tr("Keyboard shortcuts\tCtrl+/", "단축키\tCtrl+/"))?;
    item(view, MF_STRING, ID_HELP, tr("Help (online)\tF1", "도움말 (온라인)\tF1"))?;
    item(view, MF_STRING, ID_UPDATES, tr("Check for updates", "업데이트 확인"))?;
    item(view, MF_STRING, ID_ABOUT, tr("About Grids", "Grids 정보"))?;
    Ok(view)
}


pub(crate) unsafe fn popup_menu(menu: HMENU, id: i32) {
    let main = app(|a| a.main);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), id) {
        _ = GetWindowRect(b, &mut rc);
    }
    _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom + 4, None, main, None);
}
