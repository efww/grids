use crate::imports::*;
use super::*;
use crate::ui::*;
use crate::pane::*;
use crate::session::*;
use crate::shell::*;
use crate::i18n::*;
use crate::settings;

// ---------- window procs ----------

pub(crate) fn xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

/// Marks the splitter under the pointer. Only the old and new lines are repainted.
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
    if old.is_none() && d.is_some() {
        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
        _ = TrackMouseEvent(&mut t);
    }
}

pub(crate) unsafe fn cursor_for(h: HWND) -> Option<Hit> {
    let mut pt = POINT::default();
    _ = GetCursorPos(&mut pt);
    _ = ScreenToClient(h, &mut pt);
    app(|a| hit(&geo(a), pt.x, pt.y))
}

pub(crate) extern "system" fn main_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| main_proc_inner(h, msg, wp, lp)))
}
fn main_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        if window_count() == 0 {
            return DefWindowProcW(h, msg, wp, lp);
        }
        match msg {
            // While the user drags the window edge, lay the panes out once at the end (same cost reason as splitters).
            WM_SIZE => {
                let kind = wp.0 as u32;
                let sizing = app(|a| {
                    if kind != SIZE_MINIMIZED {
                        a.was_max = kind == SIZE_MAXIMIZED;
                    }
                    a.sizing
                });
                if !sizing {
                    relayout();
                    redraw_all(); // maximize / restore
                }
                // Back from minimized: WM_ACTIVATE arrived while minimized and left focus alone.
                let f = GetFocus();
                if kind != SIZE_MINIMIZED && !app(|a| a.focus_wait) && GetActiveWindow() == h && (f.is_invalid() || f == h) {
                    restore_focus();
                }
            }
            WM_ENTERSIZEMOVE => {
                app(|a| a.sizing = true);
                kids_cancel(); // the '›' is about to move
            }
            WM_EXITSIZEMOVE => {
                app(|a| a.sizing = false);
                refresh_drives(); // one that came or went mid-resize
                relayout();
                redraw_all();
                note_save();
            }
            // Wake from sleep or hibernate. Both resume broadcasts arrive; the second finds
            // the probe already queued. Return immediately: the redirector is not touched here.
            msg if msg == crate::pane::WM_POWERBROADCAST => {
                if power_resume(wp.0 as u32) {
                    resume_shares(h);
                }
                return LRESULT(1);
            }
            // A drive came or went. Only queued: no disk call while the system waits on the reply,
            // and TRUE grants a removal query.
            WM_DEVICECHANGE => {
                _ = PostMessageW(Some(h), WM_DRIVES, WPARAM(0), LPARAM(0));
                return LRESULT(1);
            }
            // A network drive or subst sends no device message. Look again when Maguro comes forward.
            WM_ACTIVATEAPP => {
                if wp.0 != 0 {
                    _ = PostMessageW(Some(h), WM_DRIVES, WPARAM(0), LPARAM(0));
                }
            }
            WM_DRIVES => {
                if !app(|a| a.drag.is_some() || a.sizing) && refresh_drives() {
                    relayout();
                    redraw_all();
                }
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
            WM_TIMER if wp.0 == TIMER_STATUS => {
                update_status();
                check_dirty();
                retry_save();
                poll_preview();
            }
            WM_TIMER if wp.0 == TIMER_PREVIEW => {
                _ = KillTimer(Some(h), TIMER_PREVIEW);
                run_preview(false);
            }
            WM_TIMER if wp.0 == TIMER_SAVE => {
                _ = KillTimer(Some(h), TIMER_SAVE);
                save_session();
            }
            WM_TIMER if wp.0 == TIMER_FVIEWS => {
                _ = KillTimer(Some(h), TIMER_FVIEWS);
                crate::folderview::flush();
            }
            WM_TIMER if wp.0 == TIMER_WINS => {
                _ = KillTimer(Some(h), TIMER_WINS);
                open_stashed();
            }
            WM_TIMER if wp.0 == TIMER_CTX_WARM => crate::shell::warmup::on_timer(h),
            WM_TIMER if wp.0 == TIMER_DROP => on_tree_drop_timer(h),
            WM_TIMER if wp.0 == TIMER_DLG_WATCH || wp.0 == TIMER_DLG_JUMP => crate::dialogjump::on_timer(wp.0),
            WM_SETCURSOR if wp.0 == h.0 as usize && (lp.0 & 0xffff) as u32 == HTCLIENT => {
                let Some(d) = app(|a| a.drag).or_else(|| cursor_for(h)) else {
                    return DefWindowProcW(h, msg, wp, lp);
                };
                let c = match d {
                    Hit::Side | Hit::Col(_) | Hit::Preview => IDC_SIZEWE,
                    Hit::Row(..) | Hit::SideSplit => IDC_SIZENS,
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
                if app(|a| a.drag.is_none()) {
                    gap_hover(h, app(|a| hit(&geo(a), x, y)));
                }
                let moved = app(|a| {
                    let d = a.drag?;
                    let g = geo(a);
                    let min_w = pane_min_w(a.dpi);
                    let min_h = pane_min_h(a.dpi);
                    match d {
                        Hit::Side => a.s.side = x - g.gap / 2,
                        Hit::SideSplit => a.s.side_split = tree_share_at(y - g.gap / 2, g.top, g.bot - g.top, g.gap, side_min(a.dpi), a.s.fav_top),
                        Hit::Col(k) => {
                            let len = g.w - g.pad - g.gx;
                            drag_ratio(&mut a.s.colr, k, x - g.gap / 2, g.gx, len, g.gap, min_w);
                            if let Some(r) = ratios_matching_min(&a.s.colr, len, g.gap, min_w) {
                                a.s.colr = r;
                            }
                        }
                        Hit::Row(c, k) => {
                            let len = g.bot - g.top;
                            drag_ratio(&mut a.s.rowr[c], k, y - g.gap / 2, g.top, len, g.gap, min_h);
                            if let Some(r) = ratios_matching_min(&a.s.rowr[c], len, g.gap, min_h) {
                                a.s.rowr[c] = r;
                            }
                        }
                        Hit::Preview => {
                            let min_left = g.gx + pane_min_w(a.dpi);
                            let max_left = (g.w - g.pad - 1).max(min_left);
                            let left = (x + g.gap / 2).clamp(min_left, max_left);
                            let px = (g.w - g.pad - left).max(1);
                            a.s.preview_w = preview_dip(px, a.dpi);
                        }
                    }
                    a.s.side = kept_side(a.s.tree_on, a.s.side, geo(a).side);
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
                    Some(Hit::SideSplit) => a.s.side_split = 0.5,
                    Some(Hit::Col(_)) => a.s.colr = even(a.s.heights.len()),
                    Some(Hit::Row(c, _)) => a.s.rowr[c] = even(a.s.heights[c]),
                    Some(Hit::Preview) => a.s.preview_w = 0,
                    None => {}
                });
                if matches!(d, Some(Hit::SideSplit)) {
                    note_save();
                }
                if matches!(d, Some(Hit::Preview)) {
                    publish_shared();
                    note_save();
                }
                relayout();
                redraw_all();
                if matches!(d, Some(Hit::Preview)) {
                    across_windows(|| {
                        relayout();
                        redraw_all();
                    });
                }
            }
            WM_COMMAND => match wp.0 & 0xffff {
                id if id == ID_TB_LAYOUT as usize => open_grid(),
                id if id == ID_TB_SAVE as usize => save_current(),
                id if id == ID_TB_LOAD as usize => open_session_list(),
                ID_SESS_SAVE => save_current(),
                ID_SESS_OPEN => open_session_list(),
                id if workspace_command(id) => {}
                id if id == ID_TB_FAV as usize => open_fav_manager(h),
                id if id == ID_TB_SET as usize => popup_menu(app(|a| a.view_menu), ID_TB_SET),
                id if (ID_DRIVE as usize..ID_DRIVE as usize + 26).contains(&id) => {
                    let i = app(|a| a.s.active);
                    if navigate(i, &format!("{}:\\", (b'A' + (id - ID_DRIVE as usize) as u8) as char)) {
                        focus_view(i);
                    } else {
                        _ = MessageBeep(MB_ICONWARNING); // no medium, or gone since the button was drawn
                    }
                }
                ID_HIDDEN => toggle_hidden(),
                ID_PANEL => toggle_tree(),
                ID_PREVIEW => toggle_preview(),
                ID_GATHER => gather_explorer(),
                ID_BASKET => crate::basket::toggle(),
                ID_RECENT => {
                    let i = app(|a| a.s.active);
                    crate::pane::recent::open_recent(i);
                }
                ID_REVEAL => {
                    // Not on the gear menu. The recent-files menu and Ctrl+Enter still reveal.
                    let i = app(|a| a.s.active);
                    crate::pane::recent::show_selection(i);
                }
                ID_HOTKEY => toggle_hotkey(),
                ID_FOLLOW => {
                    let follow = app(|a| {
                        if a.s.theme == 0 {
                            a.s.theme = if a.s.dark { 2 } else { 1 };
                            false
                        } else {
                            a.s.theme = 0;
                            true
                        }
                    });
                    publish_shared();
                    if follow {
                        _ = PostMessageW(Some(h), WM_THEME_SYNC, WPARAM(0), LPARAM(0));
                    }
                    save_session();
                    settings::redraw();
                }
                ID_THEME => {
                    let dark = app(|a| a.s.dark);
                    app(|a| a.s.theme = if dark { 1 } else { 2 });
                    apply_theme();
                    save_session();
                }
                ID_EXT => {
                    let on = app(|a| {
                        a.s.ext ^= true;
                        a.s.ext
                    });
                    apply_ext(on);
                    save_session();
                    settings::redraw();
                }
                ID_DBLUP => {
                    app(|a| a.s.dblup ^= true);
                    settings::redraw();
                }
                ID_SETTINGS => settings::open(h),
                ID_WATCH => crate::watch::open(h),
                ID_LANG_AUTO => set_language(Lang::Auto),
                id if (ID_LANG_FIRST..ID_LANG_FIRST + 64).contains(&id) => {
                    if let Some((lang, _)) = language_choices().get(id - ID_LANG_FIRST) {
                        set_language(*lang);
                    }
                }
                ID_HELP => open_help(),
                // Post so SendMessage(WM_COMMAND, ID_ABOUT) returns before the modal About loop runs.
                ID_ABOUT => {
                    _ = PostMessageW(Some(h), WM_ABOUT, WPARAM(0), LPARAM(0));
                }
                id @ ID_ACCENT..=1405 => set_accent(pal().accents[id - ID_ACCENT].1),
                ID_ACCENT_PICK => pick_accent(),
                ID_KEYS => toggle_keys(),
                id if crate::winmerge::gear(h, id) => {}
                _ => {}
            },
            WM_INITMENUPOPUP => {
                let (menu, dblup, accent, lang, panel, preview, ext, hot) = app(|a| (a.view_menu, a.s.dblup, a.s.accent_now(), a.s.lang, a.s.tree_on, a.s.preview_on, a.s.ext, a.hot_on));
                // Submenus (language, colours, WinMerge panes) also send this. Rebuilding then destroys the one that just opened.
                if HMENU(wp.0 as _) == menu {
                    crate::winmerge::sync_gear(menu);
                    sync_workspace_menu(menu);
                }
                // MF_BYCOMMAND searches submenus, so checks still reach View, Appearance, and Behavior.
                let check = |id: usize, on: bool| CheckMenuItem(menu, id as u32, (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0);
                check(ID_HIDDEN, hidden());
                check(ID_PANEL, panel);
                check(ID_PREVIEW, preview);
                check(ID_EXT, ext);
                check(ID_FOLLOW, app(|a| a.s.theme == 0));
                _ = ModifyMenuW(menu, ID_THEME as u32, MF_BYCOMMAND | MF_STRING, ID_THEME as usize, &HSTRING::from(theme_action_label()));
                check(ID_HOTKEY, hot);
                if let Some(i) = crate::keys::index("app.front") {
                    let label = format!("{}\t{}", tr(crate::Key::BringMaguroFrontFromAnywhere), crate::keys::chord_text(i));
                    _ = ModifyMenuW(menu, ID_HOTKEY as u32, MF_BYCOMMAND | MF_STRING, ID_HOTKEY as usize, &HSTRING::from(label));
                }
                if let Some(i) = crate::keys::index("view.panel") {
                    let label = format!("{}\t{}", tr(crate::Key::LeftPanel), crate::keys::chord_text(i));
                    _ = ModifyMenuW(menu, ID_PANEL as u32, MF_BYCOMMAND | MF_STRING, ID_PANEL as usize, &HSTRING::from(label));
                }
                if let Some(i) = crate::keys::index("view.preview") {
                    let label = format!("{}\t{}", tr(crate::Key::PreviewPane), crate::keys::chord_text(i));
                    _ = ModifyMenuW(menu, ID_PREVIEW as u32, MF_BYCOMMAND | MF_STRING, ID_PREVIEW as usize, &HSTRING::from(label));
                }
                if let Some(i) = crate::keys::index("file.basket") {
                    let label = format!("{}\t{}", tr(crate::Key::FileBasket), crate::keys::chord_text(i));
                    _ = ModifyMenuW(menu, ID_BASKET as u32, MF_BYCOMMAND | MF_STRING, ID_BASKET, &HSTRING::from(label));
                }
                check(ID_BASKET, crate::basket::alive());
                if let Some(i) = crate::keys::index("view.gather") {
                    let label = format!(
                        "{}\t{}",
                        tr(crate::Key::OpenFoldersFromExplorerWindows),
                        crate::keys::chord_text(i)
                    );
                    _ = ModifyMenuW(menu, ID_GATHER as u32, MF_BYCOMMAND | MF_STRING, ID_GATHER as usize, &HSTRING::from(label));
                }
                _ = ModifyMenuW(menu, ID_RECENT as u32, MF_BYCOMMAND | MF_STRING, ID_RECENT, &HSTRING::from(crate::pane::recent::open_label()));
                // Show in folder is not on this menu. Do not refresh ID_REVEAL here: that id used to
                // match another gear row, and a refresh would rename or gray it.
                if let Some(i) = crate::keys::index("sess.save") {
                    let label = format!("{}\t{}", tr(crate::Key::SaveThisTab), crate::keys::chord_text(i));
                    _ = ModifyMenuW(menu, ID_SESS_SAVE as u32, MF_BYCOMMAND | MF_STRING, ID_SESS_SAVE, &HSTRING::from(label));
                }
                if let Some(i) = crate::keys::index("watch.open") {
                    let label = format!("{}\t{}", tr(crate::Key::FolderWatch), crate::keys::chord_text(i));
                    _ = ModifyMenuW(menu, ID_WATCH as u32, MF_BYCOMMAND | MF_STRING, ID_WATCH, &HSTRING::from(label));
                }
                check(ID_DBLUP, dblup);
                check(ID_LANG_AUTO, lang == Lang::Auto);
                for (i, (choice, _)) in language_choices().into_iter().enumerate() {
                    check(ID_LANG_FIRST + i, lang == choice);
                }
                for (k, (_, c)) in pal().accents.iter().enumerate() {
                    check(ID_ACCENT + k, *c == accent);
                }
            }
            WM_COPYDATA => {
                let cd = &*(lp.0 as *const CopyData);
                if cd.dw != COPY_ID {
                    return LRESULT(0);
                }
                // copy first: the sender's buffer is only valid during this call
                let (parent, args) = if cd.cb == 0 || cd.lp.is_null() {
                    (0, Vec::new())
                } else {
                    let text = String::from_utf16_lossy(std::slice::from_raw_parts(cd.lp as *const u16, cd.cb as usize / 2));
                    crate::folderapp::parse_handoff(&text)
                };
                open_forwarded(parent, args);
                return LRESULT(1);
            }
            WM_ACTIVATE => {
                // Back in front: refresh Explorer pins and favourite files if either group is open.
                // Debounced in request_pins. One read fills both lists.
                let (state, min) = ((wp.0 & 0xffff) as u32, (wp.0 >> 16) & 0xffff != 0);
                if state != WA_INACTIVE {
                    app(|a| a.last_front = a.cur);
                    request_pins(false);
                    crate::shell::warmup::on_activate();
                }
                // Keyboard activation restores focus at once. A click waits until that click is handled.
                if state == WA_INACTIVE {
                    keep_focus(h);
                    hide_addr_tip();
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
            WM_SYSCOMMAND => {
                if (wp.0 as u32) & 0xfff0 == SC_MINIMIZE {
                    keep_focus(h);
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
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
            WM_DPICHANGED => {
                kids_cancel();
                let dpi = (wp.0 & 0xffff) as i32;
                app(|a| {
                    a.s.side = a.s.side * dpi / a.dpi;
                    a.s.side_dpi = dpi;
                    a.dpi = dpi;
                });
                apply_font();
                fit_load();
                let r = &*(lp.0 as *const RECT);
                _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            // Restart or sign-out does not send WM_CLOSE. Saving here keeps the last folders.
            WM_ENDSESSION if wp.0 != 0 => {
                crate::remote::unmount_all();
                flush_folder_views();
                save_session();
            }
            // Windows light/dark changed. Queued so nothing is drawn inside the broadcast.
            WM_SETTINGCHANGE => {
                if lp.0 != 0 {
                    let wide = PCWSTR(lp.0 as *const u16);
                    if wide.to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                        _ = PostMessageW(Some(h), WM_THEME_SYNC, WPARAM(0), LPARAM(0));
                    }
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_THEME_SYNC => theme_sync(),
            WM_ABOUT => about(h),
            WM_WATCH => crate::watch::on_ui_event(),
            WM_WATCH_OPEN => crate::watch::open_queued(),
            WM_WATCH_TOAST => crate::watch::show_fallback(),
            WM_THEME_PANE => theme_pane_step(),
            WM_THEME_RELAUNCH => theme_relaunch(),
            WM_UPDATE => on_update(h, wp.0),
            crate::remote::WM_REMOTE => remote_arrived(),
            WM_HOTKEY if wp.0 == HOTKEY_ID as usize => hotkey_pressed(),
            WM_HOTKEY if wp.0 == crate::dialogjump::DIALOG_HOTKEY_ID as usize => crate::dialogjump::on_hotkey(),
            WM_HOTKEY if wp.0 == crate::cliphot::CLIP_HOTKEY_ID as usize => crate::cliphot::on_hotkey(),
            WM_CLOSE => {
                _ = KillTimer(Some(h), TIMER_SAVE);
                _ = KillTimer(Some(h), TIMER_FVIEWS);
                _ = KillTimer(Some(h), TIMER_WINS);
                crate::shell::warmup::disarm(h);
                flush_folder_views();
                if app(|a| a.s.autosave) {
                    autosave_associated_tabs();
                }
                publish_shared();
                let last = window_count() <= 1;
                if last {
                    // The last window stays in the file, so the next launch restores every window that was open.
                    crate::remote::unmount_all();
                    save_session();
                    _ = DestroyWindow(h);
                } else {
                    destroy_every_pane();
                    app(|a| a.tree = None);
                    let handles = drop_current_desk();
                    save_session();
                    _ = DestroyWindow(h);
                    release_desk_handles(handles.0, handles.1, handles.2, handles.3, handles.4);
                }
            }
            WM_GETMINMAXINFO => {
                let m = &mut *(lp.0 as *mut MINMAXINFO);
                let (x, y) = min_track_size(h);
                m.ptMinTrackSize.x = x;
                m.ptMinTrackSize.y = y;
                return LRESULT(0);
            }
            WM_DESTROY => {
                release_hotkey_hwnd(h);
                crate::cliphot::release(h);
                crate::dialogjump::release(h);
                _ = KillTimer(Some(h), TIMER_STATUS);
                _ = KillTimer(Some(h), TIMER_SAVE);
                _ = KillTimer(Some(h), TIMER_WINS);
                crate::shell::warmup::disarm(h);
                // A window that was not the last one already left the desk list in WM_CLOSE.
                if bind_hwnd(h) {
                    destroy_every_pane();
                    app(|a| a.tree = None);
                    let handles = drop_current_desk();
                    release_desk_handles(handles.0, handles.1, handles.2, handles.3, handles.4);
                }
                if window_count() == 0 {
                    crate::watch::shutdown();
                    crate::basket::close();
                    crate::tablist::close();
                    crate::dialogjump::shutdown();
                    PostQuitMessage(0);
                } else {
                    crate::watch::retarget(app(|a| a.main));
                    try_register_hotkey(app(|a| a.main));
                    crate::cliphot::install(app(|a| a.main));
                    crate::dialogjump::install(app(|a| a.main));
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

pub(crate) extern "system" fn pane_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| pane_proc_inner(h, msg, wp, lp)))
}
fn pane_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        // Last desk already dropped; shell/UNC callbacks may still deliver pane messages.
        if window_count() == 0 {
            return DefWindowProcW(h, msg, wp, lp);
        }
        let nav = |flags: u32| {
            if let Some(i) = pane_of(h) {
                nav_pane(i, flags);
            }
        };
        match msg {
            WM_TIMER if wp.0 == TIMER_SETTLE => settle_cols(h),
            WM_TIMER if wp.0 == TIMER_DROP => on_pane_drop_timer(h),
            WM_THEME_SCROLL => apply_theme_scroll(h),
            WM_TIMER if wp.0 == TIMER_RECENT => crate::pane::recent::on_timer(h),
            WM_TIMER if wp.0 == TIMER_NEWFILE => {
                let job = PENDING_FILE.lock().ok().and_then(|mut p| {
                    let cur = p.as_mut()?;
                    if pane_of(h) != Some(cur.pane) {
                        return None;
                    }
                    cur.left = cur.left.saturating_sub(1);
                    let (pane, name, left) = (cur.pane, cur.name.clone(), cur.left);
                    if left == 0 {
                        *p = None;
                    }
                    Some((pane, name, left))
                });
                if let Some((pane, name, left)) = job {
                    if select_new_file(pane, &name) || left == 0 {
                        if let Ok(mut p) = PENDING_FILE.lock() {
                            *p = None;
                        }
                        _ = KillTimer(Some(h), TIMER_NEWFILE);
                    }
                } else {
                    _ = KillTimer(Some(h), TIMER_NEWFILE);
                }
            }
            WM_TIMER if wp.0 == TIMER_CLIPSEL => crate::cliphot::on_select_timer(h),
            WM_TIMER if wp.0 == TIMER_NEWF => {
                let job = PENDING_NEW.lock().ok().and_then(|mut p| {
                    let cur = p.as_mut()?;
                    if pane_of(h) != Some(cur.pane) {
                        return None;
                    }
                    cur.left = cur.left.saturating_sub(1);
                    let (pane, name, left) = (cur.pane, cur.name.clone(), cur.left);
                    if left == 0 {
                        *p = None;
                    }
                    Some((pane, name, left))
                });
                if let Some((pane, name, left)) = job {
                    if select_new_folder(pane, &name) || left == 0 {
                        if let Ok(mut p) = PENDING_NEW.lock() {
                            *p = None;
                        }
                        _ = KillTimer(Some(h), TIMER_NEWF);
                    }
                } else {
                    _ = KillTimer(Some(h), TIMER_NEWF);
                }
            }
            WM_SIZE => layout_pane(h),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                pane_drag_move(h, x, y);
                if app(|a| a.pane_press.is_none() && a.pane_drag.is_none()) {
                    let at = pane_of(h).and_then(|i| crumb_at(h, i, x, y)).and_then(|hit| hit.map(|(k, ..)| k));
                    crumb_hover(h, at);
                    let dpi = app(|a| a.dpi);
                    let addr = addr_rect(h, dpi);
                    addr_tip_at(h, x >= addr.left && x < addr.right && y >= addr.top && y < addr.bottom);
                } else {
                    addr_tip_at(h, false);
                }
            }
            WM_MOUSELEAVE => {
                crumb_hover(h, None);
                refresh_addr_tip(h);
            }
            WM_KIDS => kids_show(h, wp.0),
            WM_NET => net_done(h, wp.0),
            WM_NET_HOLD => net_hold(h, wp.0),
            WM_TIMER if wp.0 == TIMER_NET => net_timeout(h),
            WM_TIMER if wp.0 == TIMER_RESUME => resume_tick(h),
            WM_TIMER if wp.0 == TIMER_KIDS => kids_timeout(h),
            WM_LBUTTONDOWN => {
                hide_addr_tip();
                let (x, y) = xy(lp);
                if let Some(i) = pane_of(h) {
                    if pane_drag_down(h, i, x, y) {
                        return LRESULT(0);
                    }
                    if pane_tab_mouse(h, i, x, y, false) {
                        return LRESULT(0);
                    }
                }
                let on_kind = {
                    let dpi = app(|a| a.dpi);
                    let mut rc = RECT::default();
                    _ = GetClientRect(h, &mut rc);
                    filter_icon_rect(pane_geo_of(h, rc.right, rc.bottom, dpi).filt, 24 * dpi / 96)
                        .is_some_and(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                };
                if let Some(i) = pane_of(h) {
                    set_active(i);
                    if on_kind {
                        kind_menu(i);
                    } else {
                        crumb_click(h, i, x, y);
                    }
                }
            }
            WM_LBUTTONUP => {
                if pane_drag_up(h) {
                    return LRESULT(0);
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_RBUTTONDOWN | WM_MBUTTONDOWN if app(|a| a.pane_drag.is_some() || a.pane_press.is_some()) => {
                cancel_pane_drag();
                return LRESULT(0);
            }
            WM_MBUTTONUP => {
                let (x, y) = xy(lp);
                if let Some(i) = pane_of(h) {
                    if pane_tab_mouse(h, i, x, y, true) {
                        return LRESULT(0);
                    }
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_SETCURSOR if app(|a| a.pane_drag.is_some()) => {
                if let Ok(c) = LoadCursorW(None, IDC_SIZEALL) {
                    SetCursor(Some(c));
                }
                return LRESULT(1);
            }
            WM_CANCELMODE => {
                cancel_pane_drag();
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_CAPTURECHANGED => {
                if app(|a| a.pane_drag.is_some() || a.pane_press.is_some()) {
                    cancel_pane_drag();
                }
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = xy(lp);
                if let Some(i) = pane_of(h) {
                    if on_pane_strip(h, x, y) {
                        let mut rc = RECT::default();
                        _ = GetClientRect(h, &mut rc);
                        let dpi = app(|a| a.dpi);
                        let n = app(|a| a.s.pane_tab_list(i).len());
                        // Empty strip: a new pane tab. On a tab this is just the second click.
                        if matches!(pane_tab_hit(&pane_geo_of(h, rc.right, rc.bottom, dpi), n, x, y, dpi), Some(PaneTabHit::Strip)) {
                            new_pane_tab(i);
                            focus_view(i);
                        } else {
                            pane_tab_mouse(h, i, x, y, false);
                        }
                        return LRESULT(0);
                    }
                }
                let dpi = app(|a| a.dpi);
                let addr = addr_rect(h, dpi);
                let on_addr = x >= addr.left && x < addr.right && y >= addr.top && y < addr.bottom;
                let on_kind = {
                    let mut rc = RECT::default();
                    _ = GetClientRect(h, &mut rc);
                    filter_icon_rect(pane_geo_of(h, rc.right, rc.bottom, dpi).filt, 24 * dpi / 96)
                        .is_some_and(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
                };
                if !on_addr && !on_kind {
                    if let Some(i) = pane_of(h) {
                        set_active(i);
                        toggle_zoom();
                    }
                }
            }
            WM_PAINT => paint_pane(h),
            // The icon's square only. The edit and the crumbs stay where they are.
            WM_PANE_ICON => {
                let (changed, dpi) = app(|a| {
                    let p = a.panes.iter_mut().flatten().find(|p| p.hwnd == h);
                    // A share wait owns the address icon. The previous folder's lookup must not put it back.
                    let new = p.and_then(|p| {
                        if p.net.is_some() {
                            return None;
                        }
                        take_icon(p.ticket, wp.0 as u32, lp.0 as i32).filter(|&n| n != p.icon).map(|n| p.icon = n)
                    });
                    (new.is_some(), a.dpi)
                });
                if let Some(r) = addr_icon(addr_rect(h, dpi), dpi).filter(|_| changed) {
                    _ = InvalidateRect(Some(h), Some(&r), false);
                }
            }
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
            WM_ERASEBKGND => return LRESULT(1), // paint_pane covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            // borderless edits sit on the field colour
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().pill));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().pill));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32) {
                // pill gets an accent outline while its edit has focus
                (ID_EDIT, EN_SETFOCUS | EN_KILLFOCUS) => sync_addr(h),
                (ID_FILTER, EN_SETFOCUS) => {
                    _ = InvalidateRect(Some(h), None, false);
                    _ = KillTimer(Some(h), TIMER_FIND);
                    if let Some(i) = pane_of(h) {
                        find_retarget(i);
                    }
                }
                (ID_FILTER, EN_KILLFOCUS) => {
                    _ = InvalidateRect(Some(h), None, false);
                    SetTimer(Some(h), TIMER_FIND, 120, None);
                }
                (ID_BACK, _) => nav(SBSP_NAVIGATEBACK),
                (ID_FWD, _) => nav(SBSP_NAVIGATEFORWARD),
                (ID_UP, _) => nav(SBSP_PARENT),
                (ID_OPEN_IN, _) => {
                    if let Some(i) = pane_of(h) {
                        open_in_click(h, i);
                    }
                }
                (ID_PANE_ADD, _) => {
                    if let Some(i) = pane_of(h) {
                        set_active(i);
                        new_pane_tab(i);
                        focus_view(i);
                    }
                }
                (ID_FAV, _) => {
                    if let Some(i) = pane_of(h) {
                        toggle_fav(i);
                    }
                }
                // typing in the filter box applies after a short pause
                (ID_FILTER, EN_CHANGE) => {
                    if let Some(i) = pane_of(h) {
                        find_cancel(i);
                    }
                    _ = SetTimer(Some(h), TIMER_FILTER, 250, None);
                }
                _ => {}
            },
            WM_TIMER if wp.0 == TIMER_FILTER => {
                _ = KillTimer(Some(h), TIMER_FILTER);
                if let Some(i) = pane_of(h) {
                    apply_filter(i);
                    refresh_find(i);
                }
            }
            WM_TIMER if wp.0 == TIMER_FIND => {
                _ = KillTimer(Some(h), TIMER_FIND);
                let focus = GetFocus();
                let list = GetDlgItem(Some(h), ID_FILTER).ok();
                let on_filter = list.is_some_and(|e| e == focus);
                let on_list = focus_is_find(h, focus);
                // Quick look opened from a result takes focus off this list. Keep the rows up
                // until that preview closes, or Space just after a click would hide them.
                let peeking = pane_of(h).is_some_and(peek_shows_find);
                if !on_filter && !on_list && !peeking {
                    if let Some(i) = pane_of(h) {
                        hide_find(i);
                    }
                }
            }
            WM_FIND => on_find_msg(h),
            // right-click on ← / →: recent folders; anywhere else as before
            WM_CONTEXTMENU if [ID_BACK, ID_FWD].iter().any(|&id| GetDlgItem(Some(h), id).is_ok_and(|b| b.0 as usize == wp.0)) => {
                if let Some(i) = pane_of(h) {
                    set_active(i);
                    recent_menu(i);
                }
            }
            // Right-click on the header itself (not its buttons or fields): view, colour, split, close.
            WM_CONTEXTMENU if wp.0 == h.0 as usize => {
                let mut p = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: (lp.0 >> 16 & 0xffff) as i16 as i32 };
                let screen = p;
                _ = ScreenToClient(h, &mut p);
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                let pg = pane_geo_of(h, rc.right, rc.bottom, app(|a| a.dpi));
                // The pane-tab strip is not the header: a right-click there does not open the colour menu.
                let header_bottom = if pg.tabs.bottom > pg.tabs.top { pg.tabs.top } else { pg.view.top };
                match pane_of(h) {
                    Some(i) if lp.0 != -1 && p.y < header_bottom => tint_menu(h, i, screen),
                    _ => return DefWindowProcW(h, msg, wp, lp),
                }
            }
            // Back / forward from a mouse driver or a browser key. Over Remote Desktop the
            // loop handles WM_XBUTTONUP first; a mouse-sourced command for that same click
            // is claimed there and must not step history again.
            WM_APPCOMMAND => {
                let Some((dir, path)) = appcommand_event(lp.0) else {
                    return DefWindowProcW(h, msg, wp, lp);
                };
                if side_claim(dir, path) {
                    nav(side_flags(dir));
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// ---------- Win+Shift+E, or whatever chord the keymap now has ----------
// Maguro's hotkey registration. The chord stays Win+Shift+E (`app.front`).

pub(crate) const HOTKEY_ID: i32 = 1;

fn hotkey_busy(chord: &str) -> String {
    tf(Key::HotkeyBusy, &[("chord", chord)])
}

/// Registers or drops the global hotkey. A bare key is refused: it would steal typing in every app.
pub(crate) unsafe fn set_hotkey(main: HWND, on: bool) -> bool {
    if main.0.is_null() {
        return false;
    }
    if !on {
        _ = UnregisterHotKey(Some(main), HOTKEY_ID);
        return false;
    }
    let Some(chord) = crate::keys::hot_chord() else { return false };
    if chord.digits || (!chord.ctrl && !chord.alt && !chord.shift && !chord.win) {
        return false;
    }
    let mut mods = MOD_NOREPEAT;
    if chord.ctrl {
        mods |= MOD_CONTROL;
    }
    if chord.alt {
        mods |= MOD_ALT;
    }
    if chord.shift {
        mods |= MOD_SHIFT;
    }
    if chord.win {
        mods |= MOD_WIN;
    }
    RegisterHotKey(Some(main), HOTKEY_ID, mods, chord.vk as u32).is_ok()
}

/// Startup and a window taking over after the owner closes. A failure leaves the wish on.
pub(crate) unsafe fn try_register_hotkey(main: HWND) {
    if main.0.is_null() || app(|a| !a.s.hotkey || a.hot_on) {
        return;
    }
    let on = set_hotkey(main, true);
    app(|a| {
        a.hot_on = on;
        a.hot_hwnd = if on { main } else { HWND::default() };
    });
}

/// After a rebind or a reset. The wish stays on when the new chord cannot be registered.
pub(crate) unsafe fn apply_hotkey(notify: bool) {
    let (want, old, main) = app(|a| (a.s.hotkey, a.hot_hwnd, a.main));
    if !old.0.is_null() {
        _ = UnregisterHotKey(Some(old), HOTKEY_ID);
    }
    let target = if old.0.is_null() { main } else { old };
    let on = want && set_hotkey(target, true);
    app(|a| {
        a.hot_on = on;
        a.hot_hwnd = if on { target } else { HWND::default() };
    });
    if notify && want && !on {
        let chord = crate::keys::index("app.front").map(crate::keys::chord_text).unwrap_or_default();
        app(|a| {
            a.note = Some((hotkey_busy(&chord), Instant::now() + std::time::Duration::from_secs(3)));
            a.status_key.clear();
        });
    }
    // A rebind can change the dialog chord and the clipboard chord too. ID 1 is left as it is.
    crate::dialogjump::sync();
    crate::cliphot::apply(notify);
}

pub(crate) unsafe fn release_hotkey_hwnd(h: HWND) {
    if app(|a| a.hot_hwnd == h) {
        _ = UnregisterHotKey(Some(h), HOTKEY_ID);
        app(|a| {
            a.hot_on = false;
            a.hot_hwnd = HWND::default();
        });
    }
}

/// The check shows what is really registered, so a failed start is unchecked and one click retries.
pub(crate) unsafe fn toggle_hotkey() {
    let (was, target) = app(|a| (a.hot_on, if a.hot_hwnd.0.is_null() { a.main } else { a.hot_hwnd }));
    let now = crate::keys::hotkey_flip(was, |on| set_hotkey(target, on));
    app(|a| {
        a.hot_on = now;
        a.hot_hwnd = if now { target } else { HWND::default() };
        a.s.hotkey = now;
        if !was && !now {
            let chord = crate::keys::index("app.front").map(crate::keys::chord_text).unwrap_or_default();
            a.note = Some((hotkey_busy(&chord), Instant::now() + std::time::Duration::from_secs(3)));
            a.status_key.clear();
        }
    });
    publish_shared();
    save_session();
}

/// Front, with the file list ready. Pressed again while Maguro is in front: minimize.
/// A dialog that has disabled its owner comes forward instead.
pub(crate) unsafe fn hotkey_pressed() {
    let mains = app(|a| a.desks.iter().map(|d| d.main).filter(|h| !h.0.is_null()).collect::<Vec<_>>());
    let disabled = mains.into_iter().find(|h| !IsWindowEnabled(*h).as_bool());
    if let Some(main) = disabled {
        if IsIconic(main).as_bool() {
            _ = ShowWindow(main, SW_RESTORE);
        }
        let p = GetLastActivePopup(main);
        if !p.0.is_null() && p != main {
            _ = SetForegroundWindow(p);
        }
        return;
    }
    let fg = GetForegroundWindow();
    let root = GetAncestor(fg, GA_ROOTOWNER);
    let ours = app(|a| a.desks.iter().find(|d| d.main == root || d.main == fg).map(|d| d.main));
    if let Some(main) = ours {
        if !IsIconic(main).as_bool() {
            if hotkey_when_front(app(|a| a.s.one_window)) == HotkeyWhenFront::NewTab {
                bind_hwnd(main);
                new_tab();
            } else {
                _ = ShowWindow(main, SW_MINIMIZE);
            }
            return;
        }
    }
    let main = app(|a| a.desks.get(a.last_front).map(|d| d.main).filter(|h| !h.0.is_null()).unwrap_or(a.hot_hwnd));
    if main.0.is_null() {
        return;
    }
    if IsIconic(main).as_bool() {
        _ = ShowWindow(main, SW_RESTORE);
    }
    if SetForegroundWindow(main).as_bool() && GetForegroundWindow() == main {
        bind_hwnd(main);
        app(|a| a.last_front = a.cur);
        focus_view(app(|a| a.s.active));
    }
}

/// F1 / Help. Opens the GitHub Pages Help in the browser. Ctrl+/ still opens the compact shortcut list.
pub(crate) unsafe fn open_help() {
    let code = app(|a| a.s.lang.resolved_code());
    let url = match code.as_str() {
        "ko" => "https://samek86.github.io/maguro/help/",
        "ja" => "https://samek86.github.io/maguro/help/ja/",
        _ => "https://samek86.github.io/maguro/help/en/",
    };
    let url = HSTRING::from(url);
    let _ = ShellExecuteW(None, w!("open"), &url, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
}

pub(crate) unsafe fn about(owner: HWND) {
    crate::settings::show_about(owner);
}

// Tab-row buttons: id, accessible name, tooltip.
pub(crate) fn toolbar_tips() -> [(i32, &'static str, &'static str); 5] {
    [
        (ID_TB_LAYOUT, "", tr(crate::Key::LayoutClickEvenGridDragRowsPer)),
        (ID_TB_SAVE, tr(crate::Key::SaveThisTab), ""), // tooltip set by refresh_save_button
        (ID_TB_LOAD, tr(crate::Key::OpenSession), tr(crate::Key::OpenSessionAsNewTab)),
        (
            ID_TB_FAV,
            tr(crate::Key::Favourites),
            tr(crate::Key::FavouritesSearchEditAndOpenInActive),
        ),
        (ID_TB_SET, tr(crate::Key::SettingsAndView), tr(crate::Key::SettingsAndView)),
    ]
}

/// View-menu label for the hand-picked theme. It names the theme a click will turn on.
pub(crate) fn theme_action_label() -> &'static str {
    if app(|a| a.s.dark) {
        tr(crate::Key::LightTheme)
    } else {
        tr(crate::Key::DarkTheme)
    }
}

// Switches the UI language now: menus, fonts, toolbar tooltips, panes, and an open settings window.
pub(crate) unsafe fn set_language(lang: Lang) {
    app(|a| a.s.choose_lang(lang));
    apply(lang);
    // Vendor titles are chosen in the UI language. The next settings paint reads them again.
    crate::shell::ctxmenu::invalidate();
    publish_shared();
    across_windows(|| {
        let (main, old) = app(|a| (a.main, a.view_menu));
        if main.0.is_null() {
            return;
        }
        if let Ok(m) = make_menus() {
            app(|a| a.view_menu = m);
            if !old.0.is_null() {
                _ = DestroyMenu(old);
            }
        }
        for (id, _, hint) in toolbar_tips() {
            if let Ok(b) = GetDlgItem(Some(main), id) {
                tip_text(b, hint, TTM_UPDATETIPTEXTW);
            }
        }
        refresh_drive_tips();
        refresh_save_button();
        rename_fields();
        app(|a| a.status_key.clear());
    });
    apply_font_all();
    apply_theme();
    crate::basket::refresh();
    save_session();
}

// The popup opened from the gear on the tab row (no classic menu bar).
// Show in folder is not here. It stays on the recent-files menu and on Ctrl+Enter.
pub(crate) unsafe fn make_menus() -> Result<HMENU> {
    let view = CreatePopupMenu()?;
    let item = |menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str| AppendMenuW(menu, flags, id, &HSTRING::from(text));
    let sep = |menu: HMENU| AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let chord = |id: &str| crate::keys::index(id).map(crate::keys::chord_text).unwrap_or_default();
    let with_chord = |name: &str, id: &str| format!("{name}\t{}", chord(id));

    // Session.
    item(view, MF_STRING, ID_SESS_SAVE, &with_chord(tr(crate::Key::SaveThisTab), "sess.save"))?;
    item(view, MF_STRING, ID_SESS_OPEN, tr(crate::Key::OpenSession2))?;
    let workspaces = workspace_menu()?;
    item(view, MF_POPUP, workspaces.0 as usize, tr(crate::Key::Workspaces))?;
    sep(view)?;

    // Tools. WinMerge inserts after Folder watch, before the next separator.
    item(view, MF_STRING, ID_BASKET, &with_chord(tr(crate::Key::FileBasket), "file.basket"))?;
    item(view, MF_STRING, ID_RECENT, &crate::pane::recent::open_label())?;
    item(view, MF_STRING, ID_GATHER, tr(crate::Key::OpenFoldersFromExplorerWindowsCtrlShift))?;
    item(view, MF_STRING, ID_WATCH, &with_chord(tr(crate::Key::FolderWatch), "watch.open"))?;
    sep(view)?;

    // View.
    let sub = CreatePopupMenu()?;
    item(sub, MF_STRING, ID_HIDDEN, tr(crate::Key::ShowHiddenFilesCtrlH))?;
    item(sub, MF_STRING, ID_EXT, tr(crate::Key::ShowFileExtensions))?;
    item(sub, MF_STRING, ID_PANEL, tr(crate::Key::LeftPanelCtrlShiftE))?;
    item(sub, MF_STRING, ID_PREVIEW, &with_chord(tr(crate::Key::PreviewPane), "view.preview"))?;
    item(view, MF_POPUP, sub.0 as usize, tr(crate::Key::View))?;
    sep(view)?;

    // Appearance. Theme names the theme a click will turn on.
    let sub = CreatePopupMenu()?;
    item(sub, MF_STRING, ID_THEME, theme_action_label())?;
    item(sub, MF_STRING, ID_FOLLOW, tr(crate::Key::FollowWindowsLightDark))?;
    let colors = CreatePopupMenu()?;
    for k in 0..LIGHT.accents.len() {
        item(colors, MF_STRING, ID_ACCENT + k, accent_name(k))?;
    }
    sep(colors)?;
    item(colors, MF_STRING, ID_ACCENT_PICK, tr(crate::Key::Custom))?;
    item(sub, MF_POPUP, colors.0 as usize, tr(crate::Key::SelectionColour))?;
    let langs = CreatePopupMenu()?;
    item(langs, MF_STRING, ID_LANG_AUTO, tr(crate::Key::FollowWindows))?;
    for (i, (_, name)) in language_choices().iter().enumerate() {
        item(langs, MF_STRING, ID_LANG_FIRST + i, name)?;
    }
    item(sub, MF_POPUP, langs.0 as usize, tr(crate::Key::Language))?;
    item(view, MF_POPUP, sub.0 as usize, tr(crate::Key::Appearance))?;
    sep(view)?;

    // Behavior.
    let sub = CreatePopupMenu()?;
    item(sub, MF_STRING, ID_DBLUP, tr(crate::Key::DoubleClickEmptySpaceParentFolder))?;
    item(
        sub,
        MF_STRING,
        ID_HOTKEY,
        &format!(
            "{}\t{}",
            tr(crate::Key::BringMaguroFrontFromAnywhere),
            crate::keys::chord_text(crate::keys::index("app.front").unwrap_or(0))
        ),
    )?;
    item(view, MF_POPUP, sub.0 as usize, tr(crate::Key::Behavior))?;
    sep(view)?;

    item(view, MF_STRING, ID_SETTINGS, tr(crate::Key::Settings))?;
    item(view, MF_STRING, ID_KEYS, tr(crate::Key::KeyboardShortcuts))?;
    item(view, MF_STRING, ID_HELP, tr(crate::Key::HelpOnlineF1))?;
    item(view, MF_STRING, ID_ABOUT, tr(crate::Key::AboutMaguro))?;
    crate::winmerge::sync_gear(view);
    Ok(view)
}

