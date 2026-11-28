use crate::*;

// ---------- keyboard ----------

// Screen readers hear where a pane key went ("Pane 2 of 4 · Downloads"), after the list's own item: only when the
// active pane did change (not on a held key's repeats, nor with no pane that way). The mouse says nothing: it is seen.
// Seen too: the pane reached gets a fading ring (flash_pane), again on each repeat that lands on another pane.
unsafe fn pane_said(before: usize, repeat: bool) {
    let now = app(|a| a.s.active);
    if now != before {
        flash_pane(now);
    }
    if !repeat && now != before {
        say_pane(now);
    }
}

// A tab key on pane i (change: true when it started a browse). Read out only when the tab number or count changed
// (not one tab, the last tab's beep, a failed browse, no such tab, an end): a browse is read when its folder has
// opened (on_nav: the new view takes the focus, which would cut the words off), anything else at once.
pub(crate) unsafe fn tab_said(i: usize, repeat: bool, change: impl FnOnce() -> bool) {
    let state = || {
        app(|a| {
            let n = a.s.tab_list(i).len();
            (a.s.tab[i].min(n - 1), n)
        })
    };
    if repeat {
        change();
        return;
    }
    let before = state();
    app(|a| a.say_nav = Some(i)); // set first: a folder may open inside the browse call itself
    let browsed = change();
    // on_nav has read it out already
    if !app(|a| a.say_nav == Some(i)) {
        return;
    }
    if state() == before || !browsed {
        app(|a| a.say_nav = None);
        if state() != before {
            say_pane(i);
        }
    }
}

pub(crate) unsafe fn key(m: &MSG) -> bool {
    // the preview takes its own keys; pane shortcuts must not act behind it
    if let Some(used) = peek_key(m) {
        return used;
    }
    // the Everything search box takes its list keys; the rest is typing
    if let Some(used) = ev_key(m) {
        return used;
    }
    // Ctrl+P's popup: its list keys, Enter, Esc and Ctrl+P; the rest is typing (no Ctrl+W, Alt+← … behind it)
    if let Some(used) = sw_key(m) {
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
    // a focused top toolbar button: ←/→, Home / End, Enter and Esc (with no Ctrl, Alt or Shift: Alt+← stays Back,
    // Alt+Shift+→ the pane move); other keys go on to the shortcuts below
    if !ctrl && !shift && !alt && is_toolbar_button(m.hwnd) {
        if let Some(used) = toolbar_key(m) {
            return used;
        }
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
    // Ctrl+`: the key left of 1 by its place (scan code 0x29, not extended), whatever the layout or an IME calls it
    let vk = match VIRTUAL_KEY(m.wParam.0 as u16) {
        _ if ctrl && !shift && !alt && (m.lParam.0 >> 16) & 0x1FF == 0x29 => VK_OEM_3,
        k => k,
    };
    let repeat = m.lParam.0 & (1 << 30) != 0; // a held key's auto-repeat: still acts, but isn't read out
    let typed_ago = LAST_TYPED.get().map(|t| t.elapsed());
    if key_types(vk, ctrl, alt) && list() {
        LAST_TYPED.set(Some(Instant::now()));
    }
    let in_main = || IsChild(app(|x| x.main), m.hwnd).as_bool();
    // the name box of an item being renamed in the active pane's list (an Edit inside its shell view)
    let renaming = || {
        let mut c = [0u16; 16];
        let n = GetClassNameW(m.hwnd, &mut c) as usize;
        String::from_utf16_lossy(&c[..n]).eq_ignore_ascii_case("Edit")
            && view_of::<IShellView>(a).and_then(|v| v.GetWindow().ok()).is_some_and(|v| !v.0.is_null() && IsChild(v, m.hwnd).as_bool())
    };
    let ctx = KeyCtx { addr, filt, tabs: tabs_on(), panes: app(|x| x.s.panes()), typed_ago, list: &list, in_main: &in_main, renaming: &renaming };
    // key_route picks where the key goes; the arms here do it (and say whether it was used)
    match key_route(vk, ctrl, shift, alt, &ctx) {
        KeyRoute::Typing => return !(addr || filt) && shell(),
        KeyRoute::Peek => {
            if !repeat {
                open_peek(a);
            }
        }
        KeyRoute::CyclePane(d) => {
            cycle(d);
            pane_said(a, repeat);
        }
        // the keys that have a mouse tip (mouse_tip) mark it learned
        KeyRoute::NewTab => {
            key_used(TIP_NEW_TAB);
            tab_said(a, repeat, || {
                new_tab(a);
                false
            });
        }
        KeyRoute::CloseTab => {
            key_used(TIP_CLOSE_TAB);
            tab_said(a, repeat, || close_tab(a, app(|x| x.s.tab[a])));
        }
        KeyRoute::ReopenTab => {
            if !repeat {
                tab_said(a, false, || reopen_closed_tab(a));
            }
        }
        KeyRoute::DupTab => tab_said(a, repeat, || dup_tab(a, app(|x| x.s.tab[a]))),
        KeyRoute::StepTab(d) => tab_said(a, repeat, || step_tab(a, d)),
        KeyRoute::MoveTab(d) => tab_said(a, repeat, || {
            move_cur_tab(a, d);
            false
        }),
        KeyRoute::NthTab(n) => tab_said(a, repeat, || pick_nth_tab(a, n)),
        KeyRoute::Pane(i) => {
            set_active(i);
            focus_view(i);
            pane_said(a, repeat);
        }
        // the pane active before this one (tmux's last-pane); again, back here. Held, it goes once (no flicker).
        KeyRoute::LastPane => {
            if !repeat {
                match app(|x| x.prev_active.filter(|&k| k != x.s.active && k < x.s.panes() && x.panes[k].is_some())) {
                    Some(k) => {
                        set_active(k);
                        focus_view(k);
                        pane_said(a, false);
                    }
                    None => {
                        _ = MessageBeep(MB_ICONWARNING);
                        set_note(no_prev_pane_note(KO.load(Ordering::Relaxed)).into());
                    }
                }
            }
        }
        KeyRoute::MovePane(dx, dy) => {
            move_pane(dx, dy);
            pane_said(a, repeat);
        }
        KeyRoute::Swap(dx, dy) => swap_dir(dx, dy),
        KeyRoute::Nudge(dx, dy) => nudge_split(dx, dy, repeat),
        KeyRoute::Back => {
            key_used(TIP_BACK);
            browse(a, SBSP_NAVIGATEBACK);
        }
        KeyRoute::Forward => {
            key_used(TIP_FWD);
            browse(a, SBSP_NAVIGATEFORWARD);
        }
        KeyRoute::Up => {
            key_used(TIP_UP);
            browse(a, SBSP_PARENT);
        }
        KeyRoute::Sibling(d) => {
            if !repeat {
                sibling_step(a, d);
            }
        }
        KeyRoute::FocusAddr => return focus_ctl(a, ID_EDIT),
        KeyRoute::Everything => {
            key_used(TIP_EV);
            open_ev();
        }
        KeyRoute::FocusFilter => return focus_ctl(a, ID_FILTER),
        KeyRoute::Hidden => toggle_hidden(),
        KeyRoute::Save => save_current(),
        KeyRoute::Load => {
            if !repeat {
                key_used(TIP_LOAD);
                toggle_load();
            }
        }
        KeyRoute::Switch => {
            if !repeat {
                open_switcher();
            }
        }
        KeyRoute::Favs => {
            key_used(TIP_FAVS);
            open_favs();
        }
        // not while the list is open: it would shrink the favourites under the list's highlight
        KeyRoute::FavToggle => {
            if !repeat && app(|x| x.fav_popup.0.is_null()) {
                fav_key(a);
            }
        }
        KeyRoute::Keys => {
            if !repeat {
                toggle_keys();
            }
        }
        KeyRoute::Zoom => {
            key_used(TIP_ZOOM);
            toggle_zoom();
        }
        KeyRoute::Tree => toggle_tree(),
        KeyRoute::Gather => gather_explorer(),
        KeyRoute::Rename => {
            if !repeat {
                open_rename(a);
            }
        }
        KeyRoute::Help => open_help(),
        // held, Ctrl+Shift+Enter runs once: after it the address is this folder again, and a repeat would ask again
        KeyRoute::AddrGo(up) => {
            if !(up && repeat) {
                go_edit(a, up);
            }
        }
        KeyRoute::FilterApply => {
            apply_filter(a);
            focus_view(a);
        }
        KeyRoute::AddrCancel => addr_reset(a, m.hwnd),
        KeyRoute::FilterClear => {
            _ = SetWindowTextW(m.hwnd, w!(""));
            apply_filter(a);
            focus_view(a);
        }
        KeyRoute::FocusRing => {
            let th = app(|x| x.tree_hwnd);
            let tv = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()).ok().filter(|_| !th.0.is_null() && IsWindowVisible(th).as_bool());
            let in_tree = th == m.hwnd || IsChild(th, m.hwnd).as_bool();
            let on_bar = is_toolbar_button(m.hwnd);
            let pos = if addr { 1 } else if filt { 2 } else if in_tree { 3 } else if on_bar { 4 } else { 0 };
            let to = focus_ring(pos, shift, tv.is_some());
            if to != 4 {
                toolbar_ring_off();
            }
            match to {
                1 => _ = focus_ctl(a, ID_EDIT),
                2 => _ = focus_ctl(a, ID_FILTER),
                3 => {
                    flush_tree_sync();
                    _ = SetFocus(tv);
                    reveal_tree_selection();
                }
                4 => focus_toolbar(),
                _ => focus_view(a),
            }
        }
        KeyRoute::KindMenu => kind_menu(a),
        KeyRoute::GoMenu => {
            if !repeat {
                go_menu(a);
            }
        }
        KeyRoute::Refresh => {
            resum_sizes();
            if let Some(sv) = view_of::<IShellView>(a) {
                _ = sv.Refresh();
            }
            spin_refresh(a);
        }
        KeyRoute::BoxKey => return false,
        // Held, it opens once: the Enter still down doesn't open an item in the list that comes.
        KeyRoute::OpenTabs(go) => {
            let paths = if repeat { Vec::new() } else { tab_targets(a, go) };
            if paths.is_empty() {
                return shell();
            }
            app(|x| x.held = Some(VK_RETURN.0));
            if go {
                tab_said(a, false, || open_in_tabs(a, &paths, true));
            } else {
                open_in_tabs(a, &paths, false);
                set_note(opened_tabs_note(paths.len().min(OPEN_TABS_MAX), KO.load(Ordering::Relaxed)));
            }
        }
        KeyRoute::Mark => mark_and_next(a),
        KeyRoute::Sort(kind) => {
            if !repeat {
                sort_by(a, kind);
            }
        }
        KeyRoute::CopyPaths => copy_paths(a, repeat),
        // held, it opens once (and the repeats don't reach the view)
        KeyRoute::Edit => {
            if !repeat {
                edit_files(a);
            }
        }
        KeyRoute::Usage => {
            if !repeat {
                open_usage(a);
            }
        }
        // the ⇄ menu opens when M is let go, so M's auto-repeats (swallowed while held) can't pick its "&Move" item
        KeyRoute::MoveMenu => {
            if !repeat {
                app(|x| x.held = Some(VK_M.0));
                MENU_KEY.set(Some(a));
            }
        }
        // held until released, as the key pressed (Num+, Num− or Ctrl+Shift+A's A): one box / inversion per press
        KeyRoute::Match(desel) => {
            app(|x| x.held = Some(vk.0));
            open_match(a, desel);
        }
        KeyRoute::Invert => {
            app(|x| x.held = Some(vk.0));
            invert_selection(a);
        }
        // held, it restores once (a repeat would go straight back)
        KeyRoute::RestoreSel => {
            app(|x| x.held = Some(vk.0));
            restore_selection(a);
        }
        // held, it compares once
        KeyRoute::Diff => {
            app(|x| x.held = Some(vk.0));
            diff_key(a);
        }
        // held, it steps once (no flicker); the key never reaches the view
        KeyRoute::RenameCycle => {
            if !repeat {
                rename_cycle_key(a, m.hwnd);
            }
        }
        KeyRoute::Recent => recent_menu(a),
        KeyRoute::NewFolder => {
            if !repeat {
                new_folder(a);
            }
        }
        KeyRoute::NewText => {
            if !repeat {
                new_text_file(a);
            }
        }
        KeyRoute::Group => {
            if !repeat {
                group_key(a);
            }
        }
        // held, it copies once
        KeyRoute::Stamp => {
            if !repeat {
                stamp_key(a);
            }
        }
        KeyRoute::Paste => {
            if !paste_key(repeat, || try_paste(a)) {
                return shell();
            }
        }
        // held, it goes once
        KeyRoute::PasteGo => {
            if !repeat {
                paste_go(a);
            }
        }
        KeyRoute::Shell => return shell(),
    }
    true
}

// Where a key goes (Grids' own action, a box's, or the shell view's), as key() then does it. The arms' order is the
// behaviour: Ctrl+Shift+F before Ctrl+F, the filter's Alt+↓ before the list's, F5 before Ctrl+F5 ... (key_routes test).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum KeyRoute {
    Typing,
    Peek,
    CyclePane(i32),
    NewTab,
    CloseTab,
    ReopenTab,
    DupTab,
    StepTab(i32),
    MoveTab(i32),
    NthTab(usize),
    Pane(usize),
    LastPane,
    MovePane(i32, i32),
    Swap(i32, i32),
    Nudge(i32, i32),
    Back,
    Forward,
    Up,
    Sibling(i32),
    FocusAddr,
    Everything,
    FocusFilter,
    Hidden,
    Save,
    Load,
    Switch,
    Favs,
    FavToggle,
    Keys,
    Zoom,
    Tree,
    Gather,
    Rename,
    Help,
    AddrGo(bool),
    FilterApply,
    AddrCancel,
    FilterClear,
    FocusRing,
    KindMenu,
    GoMenu,
    Refresh,
    BoxKey,
    OpenTabs(bool),
    Mark,
    Sort(usize),
    CopyPaths,
    Edit,
    Usage,
    MoveMenu,
    Match(bool),
    Invert,
    RestoreSel,
    Diff,
    RenameCycle,
    Recent,
    NewFolder,
    NewText,
    Group,
    Stamp,
    Paste,
    PasteGo,
    Shell,
}

// What key_route looks at besides the key: the focus in the address bar / filter, tabs on, the pane count, the last
// typed key's age; and, asked only by the arms that need it, whether the focus is in the file list / the main window /
// a pane's rename box.
pub(crate) struct KeyCtx<'a> {
    pub(crate) addr: bool,
    pub(crate) filt: bool,
    pub(crate) tabs: bool,
    pub(crate) panes: usize,
    pub(crate) typed_ago: Option<Duration>,
    pub(crate) list: &'a dyn Fn() -> bool,
    pub(crate) in_main: &'a dyn Fn() -> bool,
    pub(crate) renaming: &'a dyn Fn() -> bool,
}

pub(crate) fn key_route(vk: VIRTUAL_KEY, ctrl: bool, shift: bool, alt: bool, c: &KeyCtx) -> KeyRoute {
    use KeyRoute::*;
    let (addr, filt, tabs, list) = (c.addr, c.filt, c.tabs, c.list);
    let dir = |k: VIRTUAL_KEY| match k {
        VK_LEFT => (-1, 0),
        VK_RIGHT => (1, 0),
        VK_UP => (0, -1),
        _ => (0, 1),
    };
    let step = |k: VIRTUAL_KEY| if k == VK_NEXT { 1 } else { -1 };
    match vk {
        // AltGr comes as Ctrl+Alt: a character (ł, ś, ß, [ ...), never a shortcut. Typing in the boxes, the view's otherwise.
        _ if ctrl && key_types(vk, ctrl, alt) => Typing,
        // Space: preview, as in Finder (F3 too); the preview's own Space closes it
        VK_SPACE if !ctrl && !shift && !alt && list() && space_peeks(c.typed_ago) => Peek,
        VK_TAB if ctrl => CyclePane(if shift { -1 } else { 1 }),
        VK_T if ctrl && !shift && !alt && tabs => NewTab,
        // Ctrl+Shift+T: the last closed tab back (a browser's); held, it reopens one
        VK_T if ctrl && shift && !alt && tabs => ReopenTab,
        VK_W if ctrl && !shift && !alt && tabs => CloseTab,
        VK_K if ctrl && !shift && !alt && tabs => DupTab,
        k @ (VK_NEXT | VK_PRIOR) if ctrl && !shift && !alt && tabs => StepTab(step(k)),
        k @ (VK_NEXT | VK_PRIOR) if ctrl && shift && !alt && tabs => MoveTab(step(k)),
        // Alt+1 … Alt+9: the nth tab, 9 the last (as a browser's Ctrl+1 … 9; Ctrl+digits are the panes). Number pad keys stay out.
        k if alt && !ctrl && !shift && tabs && (VK_1.0..=VK_9.0).contains(&k.0) => NthTab((k.0 - VK_1.0 + 1) as usize),
        k if ctrl && !shift && !alt && (VK_1.0..=VK_9.0).contains(&k.0) && ((k.0 - VK_1.0) as usize) < c.panes => Pane((k.0 - VK_1.0) as usize),
        // Ctrl+`: back to the pane used before (key() reads the key left of 1 as `); from the boxes too
        VK_OEM_3 if ctrl && !shift && !alt => LastPane,
        k @ (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) if alt && shift && !ctrl => MovePane(dir(k).0, dir(k).1),
        k @ (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) if alt && shift && ctrl => Swap(dir(k).0, dir(k).1),
        k @ (VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) if alt && ctrl && !shift => Nudge(dir(k).0, dir(k).1),
        VK_LEFT if alt => Back,
        VK_RIGHT if alt => Forward,
        VK_UP if alt => Up,
        // the next / previous folder in the same parent; held, it steps once (not into the boxes' own PgDn)
        k @ (VK_NEXT | VK_PRIOR) if alt && !ctrl && !shift && !(addr || filt) => Sibling(step(k)),
        VK_L if ctrl => FocusAddr,
        VK_D if alt => FocusAddr,
        VK_F4 if !alt => FocusAddr,
        VK_F if ctrl && shift && !alt => Everything,
        VK_F if ctrl => FocusFilter,
        VK_H if ctrl => Hidden,
        VK_S if ctrl => Save,
        // Ctrl+O: the saved sessions, picked from the keys (held, it opens once)
        VK_O if ctrl && !shift && !alt => Load,
        // Ctrl+P: go to an open pane or tab by part of its folder's name, from the boxes too (held, it opens once)
        VK_P if ctrl && !shift && !alt => Switch,
        VK_B if ctrl && !shift && !alt => Favs,
        // Ctrl+Shift+B: the active pane's ☆+ from the keys (a browser's "bookmark this page"); held, it toggles once
        VK_B if ctrl && shift && !alt => FavToggle,
        // auto-repeat (bit 30) is swallowed: holding Ctrl+/ toggles once
        VK_OEM_2 if ctrl && !shift && !alt => Keys,
        VK_M if ctrl && shift && !alt => Zoom,
        VK_E if ctrl && shift && !alt => Tree,
        VK_O if ctrl && shift && !alt => Gather,
        VK_R if ctrl && shift && !alt => Rename,
        VK_F1 => Help,
        // Ctrl+Shift+Enter: as administrator (Win+R's); any other Enter as typed
        VK_RETURN if addr => AddrGo(ctrl && shift && !alt),
        VK_RETURN if filt => FilterApply,
        VK_ESCAPE if addr => AddrCancel,
        VK_ESCAPE if filt => FilterClear,
        VK_F6 if !ctrl && !alt && (c.in_main)() => FocusRing,
        VK_DOWN if alt && !ctrl && !shift && filt => KindMenu, // a combo box's Alt+↓
        // Ctrl+G: Windows' own folders, from the list, the address bar, the filter or the tree; held, it opens once
        VK_G if ctrl && !shift && !alt => GoMenu,
        // the view leaves F5 to its frame, as Explorer's does: reload the active pane from anywhere in the window
        VK_F5 if !ctrl && !alt && !shift => Refresh,
        _ if addr || filt => BoxKey,
        // Ctrl+Enter: the chosen folders as tabs behind this one, Ctrl+Shift+Enter: the focused folder in a new tab,
        // gone to (a browser's Ctrl+click / Ctrl+Shift+click). Files, no folder or tabs off: the view's own Enter.
        VK_RETURN if ctrl && !alt && tabs && list() => OpenTabs(shift),
        // Insert: flip the focused item's selection and step down, as in Total Commander; held, it auto-repeats
        // (Ctrl+Insert / Shift+Insert stay Explorer's copy / paste)
        VK_INSERT if !ctrl && !shift && !alt && list() => Mark,
        // sort the list by name / date modified / size; again: the other way round (F4 stays the address bar's)
        k @ (VK_F3 | VK_F5 | VK_F6) if ctrl && !shift && !alt && list() => Sort(match k {
            VK_F3 => 0,
            VK_F5 => 1,
            _ => 2,
        }),
        VK_C if ctrl && shift && !alt && list() => CopyPaths,
        // Ctrl+E: the chosen files in an editor, never run (Total Commander's F4, which is the address bar's here);
        // a pane has no Explorer search box for it to reach. Ctrl+Shift+E stays the tree's (above).
        VK_E if ctrl && !shift && !alt && list() => Edit,
        VK_U if ctrl && shift && !alt && list() => Usage,
        // Ctrl+M: the ⇄ menu, opened when M is let go
        VK_M if ctrl && !shift && !alt && list() => MoveMenu,
        // (Ctrl+Num+ stays Explorer's column fit)
        k @ (VK_ADD | VK_SUBTRACT | VK_MULTIPLY) if !ctrl && !alt && list() => match k {
            VK_MULTIPLY => Invert,
            _ => Match(k == VK_SUBTRACT),
        },
        // Num/: the selection lost last in this folder back, as in Total Commander (Ctrl+Num/ stays the view's)
        VK_DIVIDE if !ctrl && !alt && list() => RestoreSel,
        // Shift+F2: select the files that differ from another pane's (Total Commander's Compare Directories); only
        // in the list, so a rename box's Shift+F2 stays its own
        VK_F2 if shift && !ctrl && !alt && list() => Diff,
        // F2 again while renaming in the list: the selection steps name → whole name → extension (Directory Opus,
        // XYplorer); Explorer does nothing with it there
        VK_F2 if !ctrl && !shift && !alt && (c.renaming)() => RenameCycle,
        VK_A if ctrl && shift && !alt && list() => Match(false),
        VK_DOWN if alt && !ctrl && !shift => Recent,
        VK_F3 if !ctrl && !shift && !alt && list() => Peek,
        VK_F7 if !ctrl && !shift && !alt && list() => NewFolder,
        VK_F7 if !ctrl && shift && !alt && list() => NewText,
        VK_F7 if ctrl && !shift && !alt && list() => Group,
        // Shift+F5: dated copies next to the selected items (Total Commander's copy-in-place); plain F5 is Refresh above
        VK_F5 if !ctrl && shift && !alt && list() => Stamp,
        VK_V if ctrl && !shift && !alt && list() => Paste,
        // Ctrl+Shift+V: go to the path on the clipboard, from the list only (an edit box's Ctrl+Shift+V stays a paste)
        VK_V if ctrl && shift && !alt && list() => PasteGo,
        _ => Shell,
    }
}

// F6 ring: 0 file list, 1 address bar, 2 filter, 3 folder tree (skipped without one), 4 top toolbar.
pub(crate) fn focus_ring(pos: usize, back: bool, tree: bool) -> usize {
    let ring: &[usize] = if tree { &[0, 1, 2, 3, 4] } else { &[0, 1, 2, 4] };
    let n = ring.len();
    // a place not in the ring (the tree's, with the tree gone) counts as the one before it
    let k = ring.iter().rposition(|&p| p <= pos).unwrap_or(0);
    ring[(k + if back { n - 1 } else { 1 }) % n]
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

// ---------- Ctrl held alone: each visible button's shortcut as a keycap ----------
// Office's KeyTips and macOS CheatSheet: the keys of what is on screen, without hunting tooltips or the Ctrl+/ list.

pub(crate) const TIMER_CTRL_HINT: usize = 1116; // on main: Ctrl held long enough, then the shown keycaps' watch
const HINT_WAIT_MS: u32 = 800;
const HINT_WATCH_MS: u32 = 100;

thread_local! {
    // 0 nothing, 1 Ctrl down and waiting (HINT_WAIT_MS), 2 keycaps shown
    pub(crate) static CTRL_HINT: Cell<u8> = const { Cell::new(0) };
    static HINT_AT: Cell<POINT> = const { Cell::new(POINT { x: 0, y: 0 }) }; // the pointer when Ctrl went down
    static HINT_WND: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

/// Ctrl pressed with neither Shift, Alt nor Win arms the keycaps (a fresh press starts the wait); its own repeats
/// keep them armed; any other key, down or up, disarms them (AltGr's Alt, Ctrl+C ...), as menu_key_step does.
/// Mouse buttons, the wheel and deactivation disarm them too (pre_translate, main_proc). Gives (armed, start).
pub(crate) fn ctrl_hint_step(armed: bool, down: bool, vk: u16, repeat: bool, [_, shift, alt, win]: [bool; 4]) -> (bool, bool) {
    let ctrl = [VK_CONTROL.0, VK_LCONTROL.0, VK_RCONTROL.0].contains(&vk);
    match down {
        true if ctrl && repeat => (armed, false),
        true if ctrl => (!shift && !alt && !win, !shift && !alt && !win),
        _ => (false, false),
    }
}

/// The key a tooltip names in brackets: the first "( … )" that starts with a key, its keys joined " / ".
/// "View (Alt or F10): …" gives "Alt / F10"; "(filled star: remove)  (Ctrl+Shift+B)" gives "Ctrl+Shift+B".
pub(crate) fn tip_key(tip: &str) -> Option<String> {
    let is_key = |w: &str| {
        ["Ctrl+", "Alt+", "Shift+", "Win+"].iter().any(|m| w.len() > m.len() && w.starts_with(m))
            || w == "Alt"
            || w.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=12).contains(&n))
    };
    tip.split('(').skip(1).find_map(|p| {
        let words: Vec<&str> = p.split(')').next()?.split([' ', ',', ';']).filter(|w| !w.is_empty()).collect();
        words.first().is_some_and(|w| is_key(w)).then(|| words.into_iter().filter(|w| is_key(w)).collect::<Vec<_>>().join(" / "))
    })
}

/// Keycaps laid out: each (control, cap width) centred under its control and kept inside `area`, `h` tall. One that
/// would touch a cap already placed (or a `taken` one) tries one row lower, else it is left out; so is one that
/// doesn't fit in `area`.
pub(crate) fn hint_rects(wants: &[(RECT, i32)], h: i32, gap: i32, area: RECT) -> Vec<Option<RECT>> {
    let mut placed: Vec<RECT> = Vec::new();
    let mut out = Vec::new();
    for &(c, w) in wants {
        let left = ((c.left + c.right - w) / 2).min(area.right - w).max(area.left);
        let at = |row: i32| {
            let top = c.bottom + gap + row * (h + gap);
            RECT { left, top, right: left + w, bottom: top + h }
        };
        let free = |r: &RECT| !placed.iter().any(|p| r.left < p.right + gap && p.left < r.right + gap && r.top < p.bottom + gap && p.top < r.bottom + gap);
        let fits = |r: &RECT| r.right <= area.right && r.top >= area.top && r.bottom <= area.bottom;
        let r = [at(0), at(1)].into_iter().find(|r| fits(r) && free(r));
        placed.extend(r);
        out.push(r);
    }
    out
}

// A colour for the overlay's see-through background that none of the caps' colours is (they are drawn opaque).
pub(crate) fn hint_key_colour(used: &[u32]) -> u32 {
    [0xFF00FF, 0x00FF01, 0x010203].into_iter().find(|k| !used.contains(k)).unwrap_or(0xFF00FF)
}

// Ctrl went down alone in the main window: the wait starts (nothing while switched off in the View menu).
unsafe fn ctrl_hint_arm(main: HWND) {
    ctrl_hint_off();
    if !app(|a| a.s.ctrl_hints) {
        return;
    }
    let mut at = POINT::default();
    _ = GetCursorPos(&mut at);
    HINT_AT.set(at);
    CTRL_HINT.set(1);
    SetTimer(Some(main), TIMER_CTRL_HINT, HINT_WAIT_MS, None);
}

// The keycaps gone, the wait or watch stopped. Cheap when there is nothing (called on every click and key-up).
pub(crate) unsafe fn ctrl_hint_off() {
    if CTRL_HINT.replace(0) == 0 {
        return;
    }
    _ = KillTimer(Some(app(|a| a.main)), TIMER_CTRL_HINT);
    let w = HINT_WND.get();
    if !w.is_invalid() {
        _ = ShowWindow(w, SW_HIDE);
    }
}

// Sticky Keys leaves Ctrl down after a tap: the keycaps would stay up, so they never come.
unsafe fn sticky_keys() -> bool {
    use windows::Win32::UI::Accessibility::{SKF_STICKYKEYSON, STICKYKEYS};
    let mut sk = STICKYKEYS { cbSize: size_of::<STICKYKEYS>() as u32, ..Default::default() };
    SystemParametersInfoW(SPI_GETSTICKYKEYS, sk.cbSize, Some(&mut sk as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok() && (sk.dwFlags & SKF_STICKYKEYSON).0 != 0
}

// TIMER_CTRL_HINT: after the wait the keycaps show if Ctrl alone is still down, the pointer stayed put (no Ctrl+drag
// or Ctrl+click on its way), Grids itself is in front (no switcher, Everything, shortcut list, preview) and no menu,
// drag or resize runs; then the same check every HINT_WATCH_MS hides them as soon as one fails, as a Ctrl let go
// inside a menu's loop or in another app never reaches pre_translate.
pub(crate) unsafe fn ctrl_hint_tick(main: HWND) {
    let down = |k: VIRTUAL_KEY| GetAsyncKeyState(k.0 as i32) < 0;
    let alone = down(VK_CONTROL) && ![VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN, VK_LBUTTON, VK_RBUTTON, VK_MBUTTON, VK_XBUTTON1, VK_XBUTTON2].into_iter().any(down);
    let (enabled, menu, busy) = hold_state(main);
    let front = GetForegroundWindow() == main && IsWindowVisible(main).as_bool() && !IsIconic(main).as_bool();
    let ok = alone && front && enabled && !menu && !busy && app(|a| a.s.ctrl_hints) && !sticky_keys();
    let mut at = POINT::default();
    _ = GetCursorPos(&mut at);
    let from = HINT_AT.get();
    match CTRL_HINT.get() {
        1 if ok && (at.x - from.x).abs() <= 4 && (at.y - from.y).abs() <= 4 => {
            if show_ctrl_hints(main) {
                CTRL_HINT.set(2);
                SetTimer(Some(main), TIMER_CTRL_HINT, HINT_WATCH_MS, None);
            } else {
                ctrl_hint_off();
            }
        }
        2 if ok => {}
        _ => ctrl_hint_off(),
    }
}

// The keycaps of what shows now, in main's client coordinates: (control, key, from its left edge rather than centred).
unsafe fn hint_targets(main: HWND, dpi: i32) -> Vec<(RECT, String, bool)> {
    let s = |v: i32| v * dpi / 96;
    // a visible control's rectangle in main's client area
    let place = |h: HWND| -> Option<RECT> {
        if h.is_invalid() || !IsWindowVisible(h).as_bool() {
            return None;
        }
        let mut r = RECT::default();
        _ = GetWindowRect(h, &mut r);
        let (mut a, mut b) = (POINT { x: r.left, y: r.top }, POINT { x: r.right, y: r.bottom });
        _ = ScreenToClient(main, &mut a);
        _ = ScreenToClient(main, &mut b);
        (b.x > a.x && b.y > a.y).then_some(RECT { left: a.x, top: a.y, right: b.x, bottom: b.y })
    };
    let mut out = Vec::new();
    // the toolbar, left to right (Save's tooltip is made elsewhere, with the session's name: its key is given here)
    let mut keyed: Vec<(i32, String)> = vec![(ID_TB_SAVE, "Ctrl+S".into())];
    keyed.extend(toolbar_tips().into_iter().filter_map(|(id, _, tip)| Some((id, tip_key(tip)?))));
    let mut bar: Vec<(RECT, String, bool)> = keyed.into_iter().filter_map(|(id, k)| Some((place(GetDlgItem(Some(main), id).ok()?)?, k, false))).collect();
    bar.sort_by_key(|t| t.0.left);
    out.extend(bar);
    // the folder tree, at its top left: the key of its View menu item
    if let (Some(r), Some(k)) = (place(app(|a| a.tree_hwnd)), VIEW_TOP[0].1.split('\t').nth(1)) {
        out.push((RECT { left: r.left + s(4), right: r.left + s(4), top: r.top, bottom: r.top }, k.into(), true));
    }
    // each pane's header, left to right: number (Ctrl+1 … 9), back, forward, up, address, ⇄, filter, ★
    let panes: Vec<HWND> = app(|a| a.panes.iter().take(a.s.panes()).map(|p| p.as_ref().map_or(HWND::default(), |p| p.hwnd)).collect());
    for (i, &h) in panes.iter().enumerate() {
        let Some(pr) = place(h) else { continue };
        let pg = pane_geo_of(h, dpi);
        let at = |r: RECT| RECT { left: r.left + pr.left, top: r.top + pr.top, right: r.right + pr.left, bottom: r.bottom + pr.top };
        let tip = |k: usize| tip_key(tr(PANE_TIPS[k].2 .0, PANE_TIPS[k].2 .1));
        let field = crumb_field(pg.addr, dpi);
        let wants = [
            (pg.num, (i < 9).then(|| format!("Ctrl+{}", i + 1)), false),
            (pg.btn[0], tip(0), false),
            (pg.btn[1], tip(1), false),
            (pg.btn[2], tip(2), false),
            (RECT { right: field.left, ..field }, (pg.addr.right > pg.addr.left).then(|| "Ctrl+L".into()), true),
            (pg.btn[3], tip(3), false),
            (RECT { right: pg.filt.left, ..pg.filt }, tip_key(tr(FILTER_TIP.0, FILTER_TIP.1)).filter(|_| pg.filt.right > pg.filt.left), true),
            (pg.fav, tip(4), false),
        ];
        for (r, k, left) in wants {
            if let Some(k) = k.filter(|_| left || r.right > r.left) {
                out.push((at(r), k, left));
            }
        }
    }
    out
}

// The keycaps drawn on a see-through popup over main's client area that takes no clicks and no focus (as the drag
// guide bars). False: nothing to show.
unsafe fn show_ctrl_hints(main: HWND) -> bool {
    let (dpi, font) = app(|a| (a.dpi, a.small_font));
    let s = |v: i32| v * dpi / 96;
    let mut area = RECT::default();
    _ = GetClientRect(main, &mut area);
    if area.right <= 0 || area.bottom <= 0 {
        return false;
    }
    let mut w = HINT_WND.get();
    if w.is_invalid() {
        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let Ok(made) = CreateWindowExW(ex, w!("GridsBar"), w!(""), WS_POPUP, 0, 0, 0, 0, Some(main), None, None, None) else { return false };
        w = made;
        HINT_WND.set(made);
    }
    let screen = GetDC(None);
    let dc = CreateCompatibleDC(Some(screen));
    let bmp = CreateCompatibleBitmap(screen, area.right, area.bottom);
    let old = SelectObject(dc, bmp.into());
    let old_font = SelectObject(dc, font.into());
    let width = |t: &str| {
        let mut sz = SIZE::default();
        _ = GetTextExtentPoint32W(dc, &t.encode_utf16().collect::<Vec<_>>(), &mut sz);
        sz
    };
    let (pad, gap, h) = (s(5), s(2), width("Ctrl").cy + s(4));
    let mut targets = hint_targets(main, dpi);
    // "All shortcuts: Ctrl+/" centred just above the status line, placed first so it always shows
    if let Some(st) = GetDlgItem(Some(main), ID_STATUS).ok().filter(|&h| IsWindowVisible(h).as_bool()) {
        let mut r = RECT::default();
        _ = GetWindowRect(st, &mut r);
        let mut p = POINT { x: r.left, y: r.top };
        _ = ScreenToClient(main, &mut p);
        let y = p.y - h - 2 * gap; // hint_rects puts a cap `gap` under its control
        targets.insert(0, (RECT { left: area.right / 2, right: area.right / 2, top: y, bottom: y }, tr("All shortcuts: Ctrl+/", "모든 단축키: Ctrl+/").into(), false));
    }
    // a left-aligned cap starts at its control's left edge
    let wants: Vec<(RECT, i32)> = targets
        .iter()
        .map(|(r, k, left)| {
            let w = width(k).cx + 2 * pad;
            (if *left { RECT { right: r.left + w, ..*r } } else { *r }, w)
        })
        .collect();
    SelectObject(dc, old_font);
    let p = pal();
    let key = hint_key_colour(&[p.pill, p.line, p.text]);
    fill(dc, area, key);
    let mut any = false;
    for ((_, k, _), r) in targets.iter().zip(hint_rects(&wants, h, gap, area)) {
        let Some(r) = r else { continue };
        fill(dc, r, p.pill);
        frame(dc, r, 1, p.line);
        text(dc, r, k, font, p.text, DT_CENTER);
        any = true;
    }
    let mut org = POINT::default();
    _ = ClientToScreen(main, &mut org);
    let size = SIZE { cx: area.right, cy: area.bottom };
    let done = any && UpdateLayeredWindow(w, Some(screen), Some(&org), Some(&size), Some(dc), Some(&POINT::default()), cref(key), None, ULW_COLORKEY).is_ok();
    SelectObject(dc, old);
    _ = DeleteObject(bmp.into());
    _ = DeleteDC(dc);
    ReleaseDC(None, screen);
    if done {
        _ = ShowWindow(w, SW_SHOWNA);
    }
    done
}

pub(crate) unsafe fn pre_translate(m: &MSG) -> bool {
    let in_tree = || app(|a| a.tree_hwnd == m.hwnd || IsChild(a.tree_hwnd, m.hwnd).as_bool());
    // a click, Alt+drag or Alt+wheel: that Alt was not let go alone
    if matches!(
        m.message,
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN | WM_NCLBUTTONDOWN | WM_NCRBUTTONDOWN | WM_NCMBUTTONDOWN | WM_NCXBUTTONDOWN | WM_MOUSEWHEEL | WM_MOUSEHWHEEL
    ) {
        MENU_ARM.set(None);
        DIFF_KEY.set(None);
        ctrl_hint_off(); // Ctrl+click, Ctrl+drag, Ctrl+wheel: no keycaps, and the input goes on as ever
        kbd_tip_hide(); // nor the tooltip of a toolbar button picked from the keys
    }
    if matches!(m.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN) {
        let t = in_tree();
        if app(|a| {
            a.tree_input = tree_input_click(a.tree_input, t, m.message == WM_LBUTTONDOWN);
            a.tree_input
        }) {
            cancel_tree_sync(); // a waiting follow must not undo the pick
        }
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
            toolbar_ring_off(); // a click on a toolbar button looks as it always has: no keyboard ring
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
            kbd_tip_key(m, VIRTUAL_KEY(vk), mods == [false; 4]);
            // Shift+F2's pane list waits for F2 and Shift to be let go; any other key (or Shift pressed anew) drops it,
            // and so does a fresh F2 without Shift (a missed key-up must not turn a rename into the compare list)
            if !((vk == VK_F2.0 && (repeat || mods[1])) || (vk == VK_SHIFT.0 && repeat)) {
                DIFF_KEY.set(None);
            }
            let (armed, _) = menu_key_step(MENU_ARM.get(), true, vk, repeat, mods);
            MENU_ARM.set(armed.filter(|_| in_main));
            // Ctrl alone: the keycaps wait for it to stay held; anything else drops them (the key goes on as ever)
            match ctrl_hint_step(CTRL_HINT.get() != 0, true, vk, repeat, mods) {
                (_, true) if in_main => ctrl_hint_arm(main),
                (true, false) => {}
                _ => ctrl_hint_off(),
            }
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
            if app(|a| {
                a.tree_input = tree_input_after(prev, tree, used);
                a.tree_input
            }) {
                cancel_tree_sync(); // an arrow in the tree: its pick, not a waiting follow
            }
            used
        }
        WM_KEYUP | WM_SYSKEYUP => {
            ctrl_hint_off(); // Ctrl let go (or any key): the keycaps go at once
            let armed = app(|a| a.held) == Some(VK_M.0);
            app(|a| a.held = a.held.filter(|&k| k != m.wParam.0 as u16));
            // Enter let go on the toolbar button it went down on: that button pressed
            if VIRTUAL_KEY(m.wParam.0 as u16) == VK_RETURN && toolbar_enter_up(m) {
                return true;
            }
            // Ctrl+M let go: the ⇄ menu of the pane it was pressed in, under its ⇄ button as a click opens it
            if VIRTUAL_KEY(m.wParam.0 as u16) == VK_M {
                let pane = MENU_KEY.take().filter(|_| armed);
                if let Some((i, h)) = pane.and_then(|i| app(|a| a.panes.iter().take(a.s.panes()).nth(i)?.as_ref().map(|p| (i, p.hwnd)))) {
                    open_in_menu(h, i);
                    return true;
                }
            }
            // Shift+F2 with several panes to compare: their list once both F2 and Shift are up, so the pane digit
            // isn't typed as '@'
            let up = VIRTUAL_KEY(m.wParam.0 as u16);
            let down = |k: VIRTUAL_KEY| k != up && GetKeyState(k.0 as i32) < 0;
            if matches!(up, VK_F2 | VK_SHIFT) && !down(VK_F2) && !down(VK_SHIFT) {
                if let Some((i, h)) = DIFF_KEY.take().and_then(|i| app(|a| a.panes.iter().take(a.s.panes()).nth(i)?.as_ref().map(|p| (i, p.hwnd)))) {
                    diff_menu(h, i);
                    return up == VK_F2;
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
            fail(Fail::NewFolder, "");
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

/// Ctrl+` with no pane to go back to (none used before in this layout, or its cell went).
pub(crate) fn no_prev_pane_note(ko: bool) -> &'static str {
    if ko { "돌아갈 이전 패널이 없음 — 다른 패널로 옮긴 뒤 Ctrl+`로 오갈 수 있음" } else { "No previous pane to go back to — after you move to another pane, Ctrl+` goes back and forth" }
}

/// Space opens the preview unless a name is being typed (a key within the last second).
pub(crate) fn space_peeks(typed_ago: Option<Duration>) -> bool {
    typed_ago.is_none_or(|d| d >= Duration::from_secs(1))
}

/// F2 again in a rename box: the next selection (start, end) in UTF-16 units, as the edit counts them. The name
/// without its extension → the whole name → the extension alone (after the dot) → the name again; any other
/// selection, or just a caret (after typing), starts at the name. `whole`: no extension step (a folder, a dot
/// file, an extension the box doesn't show), so the whole name stays selected.
pub(crate) fn rename_cycle(name: &str, whole: bool, sel: (usize, usize)) -> (usize, usize) {
    let n = name.encode_utf16().count();
    let s = stem_len(name, whole);
    let sel = (sel.0.min(sel.1), sel.0.max(sel.1));
    let (stem, all, ext) = ((0, s), (0, n), (s + 1, n));
    if s + 1 >= n {
        return all; // no extension (or a name ending in a dot): nothing to step through
    }
    match sel {
        x if x == stem => all,
        x if x == all => ext,
        _ => stem,
    }
}

// F2 in the active pane's rename box (`edit`): its selection steps on (rename_cycle). The extension step only when
// the box shows the focused item's extension: a file whose editing name is its parsing name (not a .lnk, nor with
// extensions hidden); a folder, or an item that can't be read, selects the whole name.
unsafe fn rename_cycle_key(i: usize, edit: HWND) {
    use windows::Win32::UI::Input::Ime::{ImmGetCompositionStringW, ImmGetContext, ImmNotifyIME, ImmReleaseContext, CPS_COMPLETE, GCS_COMPSTR, NI_COMPOSITIONSTR};
    // a Korean syllable still being composed is finished first, and this F2 does only that: a selection moved under
    // it would put the syllable somewhere else
    let imc = ImmGetContext(edit);
    if !imc.is_invalid() {
        let composing = ImmGetCompositionStringW(imc, GCS_COMPSTR, None, 0) > 0;
        if composing {
            _ = ImmNotifyIME(imc, NI_COMPOSITIONSTR, CPS_COMPLETE, 0);
        }
        _ = ImmReleaseContext(edit, imc);
        if composing {
            return;
        }
    }
    let mut buf = vec![0u16; GetWindowTextLengthW(edit).max(0) as usize + 1];
    let got = GetWindowTextW(edit, &mut buf).max(0) as usize;
    let name = String::from_utf16_lossy(&buf[..got.min(buf.len())]);
    let (mut s, mut e) = (0u32, 0u32);
    SendMessageW(edit, EM_GETSEL, Some(WPARAM(&mut s as *mut u32 as usize)), Some(LPARAM(&mut e as *mut u32 as isize)));
    let shows_ext = || -> Option<bool> {
        let fv = view_of::<IFolderView2>(i)?;
        let it = fv.GetItem::<IShellItem>(fv.GetFocusedItem().ok()?).ok()?;
        let attr = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0)).ok()?.0;
        let editing = pwstr(it.GetDisplayName(SIGDN_PARENTRELATIVEEDITING).ok()?);
        let parsing = pwstr(it.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).ok()?);
        Some(!real_folder(attr) && editing == parsing)
    };
    let (a, b) = rename_cycle(&name, !shows_ext().unwrap_or(false), (s as usize, e as usize));
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(a)), Some(LPARAM(b as isize)));
    SendMessageW(edit, EM_SCROLLCARET, None, None);
}
