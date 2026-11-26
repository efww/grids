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
    // folder usage: every key is its own while it has the focus (no Ctrl+W, Alt+← … on the pane behind)
    if let Some(used) = usage_key(m) {
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
    // the shortcut list: Esc (after emptying its find box) and Ctrl+/ close it (focus goes back), Tab / Ctrl+F move
    // between the list and the find box; every other key is theirs (no pane shortcuts behind it)
    let keys = app(|a| a.keys_popup);
    if !keys.0.is_null() && (m.hwnd == keys || IsChild(keys, m.hwnd).as_bool()) {
        return keys_key(m, keys, ctrl, shift, alt);
    }
    // the saved sessions popup: Ctrl+O closes it, every other key is its own (load_key); no Ctrl+W, Ctrl+1,
    // Alt+← ... acting on the panes behind it. Its rename box never gets here (pre_translate takes it first).
    let load = app(|a| a.load);
    if !load.0.is_null() && m.hwnd == load {
        if VIRTUAL_KEY(m.wParam.0 as u16) == VK_O && ctrl && !shift && !alt {
            if m.lParam.0 & (1 << 30) == 0 {
                toggle_load();
            }
            return true;
        }
        return false;
    }
    // the getting started tour: Enter, ←/→, Tab, Esc and Ctrl+/ are its own; other keys stay with its buttons
    if let Some(used) = tour_key(m) {
        return used;
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
    if key_types(vk, ctrl, alt) && list() {
        LAST_TYPED.set(Some(Instant::now()));
    }
    match vk {
        // AltGr comes as Ctrl+Alt: a character (ł, ś, ß, [ ...), never a shortcut. Typing in the boxes, the view's otherwise.
        _ if ctrl && key_types(vk, ctrl, alt) => return !(addr || filt) && shell(),
        // Space: preview, as in Finder (F3 too); the preview's own Space closes it
        VK_SPACE if !ctrl && !shift && !alt && list() && space_peeks(typed_ago) => {
            if m.lParam.0 & (1 << 30) == 0 {
                open_peek(a);
            }
        }
        VK_TAB if ctrl => cycle(if shift { -1 } else { 1 }),
        // the keys that have a mouse tip (mouse_tip) mark it learned
        VK_T if ctrl && !shift && !alt && tabs_on() => {
            key_used(TIP_NEW_TAB);
            new_tab(a);
        }
        VK_W if ctrl && !shift && !alt && tabs_on() => {
            key_used(TIP_CLOSE_TAB);
            close_tab(a, app(|x| x.s.tab[a]));
        }
        VK_K if ctrl && !shift && !alt && tabs_on() => dup_tab(a, app(|x| x.s.tab[a])),
        k @ (VK_NEXT | VK_PRIOR) if ctrl && !shift && !alt && tabs_on() => step_tab(a, if k == VK_NEXT { 1 } else { -1 }),
        k @ (VK_NEXT | VK_PRIOR) if ctrl && shift && !alt && tabs_on() => move_cur_tab(a, if k == VK_NEXT { 1 } else { -1 }),
        // Alt+1 … Alt+9: the nth tab, 9 the last (as a browser's Ctrl+1 … 9; Ctrl+digits are the panes). Number pad keys stay out.
        k if alt && !ctrl && !shift && tabs_on() && (VK_1.0..=VK_9.0).contains(&k.0) => pick_nth_tab(a, (k.0 - VK_1.0 + 1) as usize),
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
        k @ (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) if alt && ctrl && !shift => {
            let repeat = m.lParam.0 & (1 << 30) != 0;
            match k {
                VK_LEFT => nudge_split(-1, 0, repeat),
                VK_RIGHT => nudge_split(1, 0, repeat),
                VK_UP => nudge_split(0, -1, repeat),
                _ => nudge_split(0, 1, repeat),
            }
        }
        VK_LEFT if alt => {
            key_used(TIP_BACK);
            browse(a, SBSP_NAVIGATEBACK);
        }
        VK_RIGHT if alt => {
            key_used(TIP_FWD);
            browse(a, SBSP_NAVIGATEFORWARD);
        }
        VK_UP if alt => {
            key_used(TIP_UP);
            browse(a, SBSP_PARENT);
        }
        // the next / previous folder in the same parent; held, it steps once (not into the boxes' own PgDn)
        k @ (VK_NEXT | VK_PRIOR) if alt && !ctrl && !shift && !(addr || filt) => {
            if m.lParam.0 & (1 << 30) == 0 {
                sibling_step(a, if k == VK_NEXT { 1 } else { -1 });
            }
        }
        VK_L if ctrl => return focus_ctl(a, ID_EDIT),
        VK_D if alt => return focus_ctl(a, ID_EDIT),
        VK_F4 if !alt => return focus_ctl(a, ID_EDIT),
        VK_F if ctrl && shift && !alt => {
            key_used(TIP_EV);
            open_ev();
        }
        VK_F if ctrl => return focus_ctl(a, ID_FILTER),
        VK_H if ctrl => toggle_hidden(),
        VK_S if ctrl => save_current(),
        // Ctrl+O: the saved sessions, picked from the keys (held, it opens once)
        VK_O if ctrl && !shift && !alt => {
            if m.lParam.0 & (1 << 30) == 0 {
                key_used(TIP_LOAD);
                toggle_load();
            }
        }
        VK_B if ctrl && !shift && !alt => {
            key_used(TIP_FAVS);
            open_favs();
        }
        // Ctrl+Shift+B: the active pane's ☆+ from the keys (a browser's "bookmark this page"); held, it toggles once.
        // Not while the list is open: it would shrink the favourites under the list's highlight.
        VK_B if ctrl && shift && !alt => {
            if m.lParam.0 & (1 << 30) == 0 && app(|x| x.fav_popup.0.is_null()) {
                fav_key(a);
            }
        }
        // auto-repeat (bit 30) is swallowed: holding Ctrl+/ toggles once
        VK_OEM_2 if ctrl && !shift && !alt => {
            if m.lParam.0 & (1 << 30) == 0 {
                toggle_keys();
            }
        }
        VK_M if ctrl && shift && !alt => {
            key_used(TIP_ZOOM);
            toggle_zoom();
        }
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
            spin_refresh(a);
        }
        _ if addr || filt => return false,
        // Insert: flip the focused item's selection and step down, as in Total Commander; held, it auto-repeats
        // (Ctrl+Insert / Shift+Insert stay Explorer's copy / paste)
        VK_INSERT if !ctrl && !shift && !alt && list() => mark_and_next(a),
        // sort the list by name / date modified / size; again: the other way round (F4 stays the address bar's)
        k @ (VK_F3 | VK_F5 | VK_F6) if ctrl && !shift && !alt && list() => {
            if m.lParam.0 & (1 << 30) == 0 {
                sort_by(a, match k {
                    VK_F3 => 0,
                    VK_F5 => 1,
                    _ => 2,
                });
            }
        }
        VK_C if ctrl && shift && !alt && list() => copy_paths(a),
        VK_U if ctrl && shift && !alt && list() => {
            if m.lParam.0 & (1 << 30) == 0 {
                open_usage(a);
            }
        }
        // Ctrl+M: the ⇄ menu, opened when M is let go, so M's auto-repeats (swallowed while held) can't pick
        // its "&Move" item
        VK_M if ctrl && !shift && !alt && list() => {
            if m.lParam.0 & (1 << 30) == 0 {
                app(|x| x.held = Some(VK_M.0));
                MENU_KEY.set(Some(a));
            }
        }
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
        VK_F7 if ctrl && !shift && !alt && list() => {
            if m.lParam.0 & (1 << 30) == 0 {
                group_key(a);
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
// A key anywhere else drops a mark left over from a tree click that changed nothing (expanding, re-clicking).
pub(crate) fn tree_input_after(_prev: bool, in_tree: bool, used: bool) -> bool {
    !used && in_tree
}

// A left click in the tree marks the next selection change as the user's; a click anywhere else
// (another pane, ...) clears it, so the tree sync that click triggers doesn't navigate that pane again.
pub(crate) fn tree_input_click(prev: bool, in_tree: bool, left: bool) -> bool {
    if in_tree { prev || left } else { false }
}

// Alt pressed and let go with nothing else in between, or F10, with neither Ctrl, Shift nor Win held (nor Alt,
// for F10): the view menu opens when that key is let go, as a hidden menu bar does in Chrome or Notepad. Any
// other key, down or up, disarms it (Alt+←, Alt+1, Alt+Shift, AltGr's Ctrl ...); so do mouse buttons and the
// wheel (pre_translate) and activation (Alt+Tab, main_proc). `mods`: Ctrl, Shift, Alt, Win as held with this
// key. Gives the key now armed and whether this release opens the menu.
pub(crate) fn menu_key_step(armed: Option<u16>, down: bool, vk: u16, repeat: bool, [ctrl, shift, alt, win]: [bool; 4]) -> (Option<u16>, bool) {
    let menu_key = vk == VK_MENU.0 || vk == VK_F10.0;
    match down {
        true if menu_key && repeat => (armed.filter(|&k| k == vk), false),
        true if menu_key => ((!ctrl && !shift && !win && (vk == VK_MENU.0 || !alt)).then_some(vk), false),
        true => (None, false),
        false => (None, armed == Some(vk)),
    }
}

pub(crate) unsafe fn pre_translate(m: &MSG) -> bool {
    let in_tree = || app(|a| a.tree_hwnd == m.hwnd || IsChild(a.tree_hwnd, m.hwnd).as_bool());
    // a click, Alt+drag or Alt+wheel: that Alt was not let go alone
    if matches!(
        m.message,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN | WM_NCMBUTTONDOWN | WM_NCXBUTTONDOWN | WM_MOUSEWHEEL | WM_MOUSEHWHEEL
    ) {
        MENU_ARM.set(None);
    }
    if matches!(m.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN) {
        let t = in_tree();
        app(|a| a.tree_input = tree_input_click(a.tree_input, t, m.message == WM_LBUTTONDOWN));
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
            usage_click_away(m.hwnd);
            if let Some(i) = pane_of(m.hwnd) {
                set_active(i);
            }
            false
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            // hidden in the notification area but still the foreground window (its menu closed with Esc): no shortcuts
            let main = app(|a| a.main);
            if !IsWindowVisible(main).as_bool() && (m.hwnd == main || IsChild(main, m.hwnd).as_bool()) {
                return false;
            }
            // a held Alt+PgDn's repeats leave its folder list coming (a slow share would never get to answer)
            let repeat = m.lParam.0 & (1 << 30) != 0;
            let vk = m.wParam.0 as u16;
            let held = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
            let mods = [held(VK_CONTROL), held(VK_SHIFT), held(VK_MENU), held(VK_LWIN) || held(VK_RWIN)];
            let in_main = m.hwnd == main || IsChild(main, m.hwnd).as_bool();
            let (armed, _) = menu_key_step(MENU_ARM.get(), true, vk, repeat, mods);
            MENU_ARM.set(armed.filter(|_| in_main));
            // F10 opens the view menu when let go: never the view's accelerator nor Windows' hidden menu bar
            // (Shift+F10, the context menu, isn't armed and goes on as before)
            if in_main && vk == VK_F10.0 && armed.is_some() {
                return true;
            }
            if !(repeat && app(|a| a.kids.as_ref().is_some_and(|k| k.step != 0))) {
                kids_cancel();
            }
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
            let armed = app(|a| a.held) == Some(VK_M.0);
            app(|a| a.held = a.held.filter(|&k| k != m.wParam.0 as u16));
            // Ctrl+M let go: the ⇄ menu of the pane it was pressed in, under its ⇄ button as a click opens it
            if VIRTUAL_KEY(m.wParam.0 as u16) == VK_M {
                let pane = MENU_KEY.take().filter(|_| armed);
                if let Some((i, h)) = pane.and_then(|i| app(|a| a.panes.iter().take(a.s.panes()).nth(i)?.as_ref().map(|p| (i, p.hwnd)))) {
                    open_in_menu(h, i);
                    return true;
                }
            }
            // Ctrl+Alt+arrow: the panes take the moved splitter once the arrow, Ctrl or Alt is let go
            if matches!(VIRTUAL_KEY(m.wParam.0 as u16), VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_CONTROL | VK_MENU) {
                end_nudge();
            }
            // Alt let go alone, or F10: the view menu, and the release swallowed so no hidden system menu starts
            let (armed, open) = menu_key_step(MENU_ARM.get(), false, m.wParam.0 as u16, false, [false; 4]);
            MENU_ARM.set(armed);
            let main = app(|a| a.main);
            if open && (m.hwnd == main || IsChild(main, m.hwnd).as_bool()) {
                view_menu_key();
                return true;
            }
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
    // the view as the menu's site: the shell's "New folder" asks its site to select the new folder and start
    // its rename; without one it only creates the folder
    _ = IUnknown_SetSite(&cm, &sv);
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
    // the pane Ctrl+M was pressed in: its ⇄ menu opens when M is released
    pub(crate) static MENU_KEY: Cell<Option<usize>> = const { Cell::new(None) };
    // Alt or F10 pressed alone: the view menu opens when it is let go (menu_key_step)
    pub(crate) static MENU_ARM: Cell<Option<u16>> = const { Cell::new(None) };
}

/// Keys that type into Explorer's find-as-you-type: letters, digits, punctuation, the IME's (Korean) keys.
pub(crate) fn types_char(vk: VIRTUAL_KEY) -> bool {
    matches!(vk.0, 0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE5)
}

/// A character typed: a character key with neither Ctrl nor Alt, or with both (AltGr, which Windows reports as Ctrl+Alt).
pub(crate) fn key_types(vk: VIRTUAL_KEY, ctrl: bool, alt: bool) -> bool {
    ctrl == alt && types_char(vk)
}

/// Space opens the preview unless a name is being typed (a key within the last second).
pub(crate) fn space_peeks(typed_ago: Option<Duration>) -> bool {
    typed_ago.is_none_or(|d| d >= Duration::from_secs(1))
}
