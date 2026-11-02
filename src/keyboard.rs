use crate::*;

// ---------- keyboard ----------

pub(crate) unsafe fn key(m: &MSG) -> bool {
    // the preview takes its own keys; pane shortcuts must not act behind it
    if let Some(used) = peek_key(m) {
        return used;
    }
    // the Everything search box takes its list keys; the rest is typing
    if let Some(used) = ev_key(m) {
        return used;
    }
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    // the pattern box takes Enter / Esc; everything else is typing, so pane shortcuts stay out of it
    if app(|a| a.matcher.as_ref().is_some_and(|x| x.edit == m.hwnd)) {
        match VIRTUAL_KEY(m.wParam.0 as u16) {
            VK_RETURN => {
                app(|x| x.held = Some(VK_RETURN.0));
                run_match(shift);
            }
            VK_ESCAPE => close_match(true),
            _ => return false,
        }
        return true;
    }
    // Alt+↑/↓ in the favourites list reorders it, not the pane behind (Up / recent folders)
    let fav = app(|a| a.fav_popup);
    if !fav.0.is_null() && m.hwnd == fav && fav_step(VIRTUAL_KEY(m.wParam.0 as u16), ctrl, alt, shift).is_some() {
        return false;
    }
    if let Some(i) = pane_of(m.hwnd) {
        set_active(i);
    }
    let a = app(|a| a.s.active);
    let addr = pane_ctl(a, ID_EDIT) == Some(m.hwnd);
    let filt = pane_ctl(a, ID_FILTER) == Some(m.hwnd);
    // F2, Del, Ctrl+C/V ... are the shell view's accelerators: hand them over.
    let shell = || {
        let Some(sv) = view_of::<IShellView>(a) else { return false };
        let svh = sv.GetWindow().unwrap_or_default();
        (svh == m.hwnd || IsChild(svh, m.hwnd).as_bool()) && sv.TranslateAccelerator(m) == S_OK
    };
    let list = || view_of::<IShellView>(a).and_then(|v| v.GetWindow().ok()).is_some_and(|v| in_view_list(v, m.hwnd));
    // Explorer's type-to-find takes a space inside a name: a Space soon after typing stays the list's
    let vk = VIRTUAL_KEY(m.wParam.0 as u16);
    let typed_ago = LAST_TYPED.get().map(|t| t.elapsed());
    if !ctrl && !alt && types_char(vk) && list() {
        LAST_TYPED.set(Some(Instant::now()));
    }
    match vk {
        // Space: preview, as in Finder (F3 too); the preview's own Space closes it
        VK_SPACE if !ctrl && !shift && !alt && list() && space_peeks(typed_ago) => {
            if m.lParam.0 & (1 << 30) == 0 {
                open_peek(a);
            }
        }
        VK_TAB if ctrl => cycle(if shift { -1 } else { 1 }),
        VK_T if ctrl && !shift && !alt && tabs_on() => new_tab(a),
        VK_W if ctrl && !shift && !alt && tabs_on() => close_tab(a, app(|x| x.s.tab[a])),
        VK_K if ctrl && !shift && !alt && tabs_on() => dup_tab(a, app(|x| x.s.tab[a])),
        k @ (VK_NEXT | VK_PRIOR) if ctrl && !shift && !alt && tabs_on() => step_tab(a, if k == VK_NEXT { 1 } else { -1 }),
        k if ctrl && !shift && !alt && (VK_1.0..=VK_9.0).contains(&k.0) && ((k.0 - VK_1.0) as usize) < app(|x| x.s.panes()) => {
            let i = (k.0 - VK_1.0) as usize;
            set_active(i);
            focus_view(i);
        }
        k @ (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) if alt && shift && !ctrl => match k {
            VK_LEFT => move_pane(-1, 0),
            VK_RIGHT => move_pane(1, 0),
            VK_UP => move_pane(0, -1),
            _ => move_pane(0, 1),
        },
        k @ (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) if alt && shift && ctrl => match k {
            VK_LEFT => swap_dir(-1, 0),
            VK_RIGHT => swap_dir(1, 0),
            VK_UP => swap_dir(0, -1),
            _ => swap_dir(0, 1),
        },
        VK_LEFT if alt => browse(a, SBSP_NAVIGATEBACK),
        VK_RIGHT if alt => browse(a, SBSP_NAVIGATEFORWARD),
        VK_UP if alt => browse(a, SBSP_PARENT),
        VK_L if ctrl => return focus_ctl(a, ID_EDIT),
        VK_D if alt => return focus_ctl(a, ID_EDIT),
        VK_F4 if !alt => return focus_ctl(a, ID_EDIT),
        VK_F if ctrl && shift && !alt => open_ev(),
        VK_F if ctrl => return focus_ctl(a, ID_FILTER),
        VK_H if ctrl => toggle_hidden(),
        VK_S if ctrl => save_current(),
        VK_B if ctrl && !shift && !alt => open_favs(),
        // auto-repeat (bit 30) is swallowed: holding Ctrl+/ toggles once
        VK_OEM_2 if ctrl && !shift && !alt => {
            if m.lParam.0 & (1 << 30) == 0 {
                toggle_keys();
            }
        }
        VK_M if ctrl && shift && !alt => toggle_zoom(),
        VK_E if ctrl && shift && !alt => toggle_tree(),
        VK_O if ctrl && shift && !alt => gather_explorer(),
        VK_R if ctrl && shift && !alt => {
            if m.lParam.0 & (1 << 30) == 0 {
                open_rename(a);
            }
        }
        VK_F1 => open_help(),
        VK_RETURN if addr => go_edit(a),
        VK_RETURN if filt => {
            apply_filter(a);
            focus_view(a);
        }
        VK_ESCAPE if addr => addr_reset(a, m.hwnd),
        VK_ESCAPE if filt => {
            _ = SetWindowTextW(m.hwnd, w!(""));
            apply_filter(a);
            focus_view(a);
        }
        VK_F6 if !ctrl && !alt && IsChild(app(|x| x.main), m.hwnd).as_bool() => {
            let th = app(|x| x.tree_hwnd);
            let tv = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()).ok().filter(|_| !th.0.is_null() && IsWindowVisible(th).as_bool());
            let in_tree = th == m.hwnd || IsChild(th, m.hwnd).as_bool();
            let pos = if addr { 1 } else if filt { 2 } else if in_tree { 3 } else { 0 };
            match focus_ring(pos, shift, tv.is_some()) {
                1 => _ = focus_ctl(a, ID_EDIT),
                2 => _ = focus_ctl(a, ID_FILTER),
                3 => {
                    _ = SetFocus(tv);
                    reveal_tree_selection();
                }
                _ => focus_view(a),
            }
        }
        VK_DOWN if alt && !ctrl && !shift && filt => kind_menu(a), // a combo box's Alt+↓
        // the view leaves F5 to its frame, as Explorer's does: reload the active pane from anywhere in the window
        VK_F5 if !ctrl && !alt && !shift => {
            resum_sizes();
            if let Some(sv) = view_of::<IShellView>(a) {
                _ = sv.Refresh();
            }
        }
        _ if addr || filt => return false,
        VK_C if ctrl && shift && !alt && list() => copy_paths(a),
        // held until released: one box / one inversion per press (Ctrl+Num+ stays Explorer's column fit)
        k @ (VK_ADD | VK_SUBTRACT | VK_MULTIPLY) if !ctrl && !alt && list() => {
            app(|x| x.held = Some(k.0));
            match k {
                VK_MULTIPLY => invert_selection(a),
                _ => open_match(a, k == VK_SUBTRACT),
            }
        }
        VK_A if ctrl && shift && !alt && list() => {
            app(|x| x.held = Some(VK_A.0));
            open_match(a, false);
        }
        VK_DOWN if alt && !ctrl && !shift => recent_menu(a),
        VK_F3 if !ctrl && !shift && !alt && list() => {
            if m.lParam.0 & (1 << 30) == 0 {
                open_peek(a);
            }
        }
        VK_F7 if !ctrl && !shift && !alt && list() => {
            if m.lParam.0 & (1 << 30) == 0 {
                new_folder(a);
            }
        }
        VK_F7 if !ctrl && shift && !alt && list() => {
            if m.lParam.0 & (1 << 30) == 0 {
                new_text_file(a);
            }
        }
        VK_V if ctrl && !shift && !alt && list() => {
            if !paste_key(m.lParam.0 & (1 << 30) != 0, || try_paste(a)) {
                return shell();
            }
        }
        _ => return shell(),
    }
    true
}

// F6 ring: 0 file list, 1 address bar, 2 filter, 3 folder tree (skipped without one).
pub(crate) fn focus_ring(pos: usize, back: bool, tree: bool) -> usize {
    let n = if tree { 4 } else { 3 };
    (pos.min(n - 1) + if back { n - 1 } else { 1 }) % n
}

// Keys Grids handles itself (Ctrl+Tab, F6, ...) only sync the tree; tree keys (arrows, typing) still navigate.
pub(crate) fn tree_input_after(prev: bool, in_tree: bool, used: bool) -> bool {
    !used && (prev || in_tree)
}

pub(crate) unsafe fn pre_translate(m: &MSG) -> bool {
    let in_tree = || app(|a| a.tree_hwnd == m.hwnd || IsChild(a.tree_hwnd, m.hwnd).as_bool());
    if m.message == WM_LBUTTONDOWN && in_tree() {
        app(|a| a.tree_input = true);
    }
    match m.message {
        WM_MOUSEMOVE => {
            set_hover(if is_icon_button(m.hwnd) { m.hwnd } else { HWND::default() });
            false
        }
        WM_MOUSELEAVE => {
            if app(|a| a.hover == m.hwnd) {
                set_hover(HWND::default());
            }
            false
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            kids_cancel();
            if let Some(i) = pane_of(m.hwnd) {
                set_active(i);
            }
            false
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            kids_cancel();
            // swallowed whole: no TranslateMessage, so no WM_CHAR either
            let (eat, next) = held_key(app(|a| a.held), m.wParam.0 as u16, m.lParam.0 & (1 << 30) != 0);
            app(|a| a.held = next);
            if eat {
                return true;
            }
            // the rename box on a session card: Enter / Esc are its own, the rest is typing (no Ctrl+S, F1 ... behind it)
            if !m.hwnd.0.is_null() && app(|a| a.load_edit) == m.hwnd {
                match VIRTUAL_KEY(m.wParam.0 as u16) {
                    VK_RETURN if m.message == WM_KEYDOWN => {
                        app(|x| x.held = Some(VK_RETURN.0));
                        commit_rename();
                    }
                    VK_ESCAPE => _ = end_rename(),
                    _ => return false,
                }
                return true;
            }
            // the batch rename window: its own keys, then dialog keys (Tab ...); pane shortcuts stay out of it
            if let Some(used) = rename_key(m) {
                return used;
            }
            let tree = m.message == WM_KEYDOWN && in_tree();
            // cleared while key() runs, so the selection sync it triggers isn't taken for user input
            let prev = app(|a| std::mem::take(&mut a.tree_input));
            let used = key(m);
            app(|a| a.tree_input = tree_input_after(prev, tree, used));
            used
        }
        WM_KEYUP | WM_SYSKEYUP => {
            app(|a| a.held = a.held.filter(|&k| k != m.wParam.0 as u16));
            false
        }
        _ => false,
    }
}

// F7 (Total Commander, FAR): the view's own "New folder" from its background menu, so it is named, selected and
// put into rename like Explorer's Ctrl+Shift+N.
pub(crate) unsafe fn new_folder(i: usize) {
    let Some(sv) = view_of::<IShellView>(i) else { return };
    let Ok(cm) = sv.GetItemObject::<IContextMenu>(SVGIO_BACKGROUND) else { return };
    let Ok(menu) = CreatePopupMenu() else { return };
    if cm.QueryContextMenu(menu, 0, 1, 0x7fff, CMF_NORMAL).is_ok() {
        let ci = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd: sv.GetWindow().unwrap_or_default(),
            lpVerb: s!("NewFolder"),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        if cm.InvokeCommand(&ci).is_err() {
            _ = MessageBeep(MB_ICONWARNING);
        }
    }
    _ = DestroyMenu(menu);
}

thread_local! {
    pub(crate) static LAST_TYPED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Keys that type into Explorer's find-as-you-type: letters, digits, punctuation, the IME's (Korean) keys.
pub(crate) fn types_char(vk: VIRTUAL_KEY) -> bool {
    matches!(vk.0, 0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE5)
}

/// Space opens the preview unless a name is being typed (a key within the last second).
pub(crate) fn space_peeks(typed_ago: Option<Duration>) -> bool {
    typed_ago.is_none_or(|d| d >= Duration::from_secs(1))
}
