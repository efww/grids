use crate::*;

// ---------- keyboard ----------

// Screen readers hear where a pane key went ("Pane 2 of 4 · Downloads"), after the list's own item: only when the
// active pane did change (not on a held key's repeats, nor with no pane that way). The mouse says nothing: it is seen.
unsafe fn pane_said(before: usize, repeat: bool) {
    let now = app(|a| a.s.active);
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
    let repeat = m.lParam.0 & (1 << 30) != 0; // a held key's auto-repeat: still acts, but isn't read out
    let typed_ago = LAST_TYPED.get().map(|t| t.elapsed());
    if key_types(vk, ctrl, alt) && list() {
        LAST_TYPED.set(Some(Instant::now()));
    }
    let in_main = || IsChild(app(|x| x.main), m.hwnd).as_bool();
    let ctx = KeyCtx { addr, filt, tabs: tabs_on(), panes: app(|x| x.s.panes()), typed_ago, list: &list, in_main: &in_main };
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
        KeyRoute::AddrGo => go_edit(a),
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
        KeyRoute::CopyPaths => copy_paths(a),
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
    DupTab,
    StepTab(i32),
    MoveTab(i32),
    NthTab(usize),
    Pane(usize),
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
    AddrGo,
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
    Recent,
    NewFolder,
    NewText,
    Group,
    Stamp,
    Paste,
    Shell,
}

// What key_route looks at besides the key: the focus in the address bar / filter, tabs on, the pane count, the last
// typed key's age; and, asked only by the arms that need it, whether the focus is in the file list / the main window.
pub(crate) struct KeyCtx<'a> {
    pub(crate) addr: bool,
    pub(crate) filt: bool,
    pub(crate) tabs: bool,
    pub(crate) panes: usize,
    pub(crate) typed_ago: Option<Duration>,
    pub(crate) list: &'a dyn Fn() -> bool,
    pub(crate) in_main: &'a dyn Fn() -> bool,
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
        VK_W if ctrl && !shift && !alt && tabs => CloseTab,
        VK_K if ctrl && !shift && !alt && tabs => DupTab,
        k @ (VK_NEXT | VK_PRIOR) if ctrl && !shift && !alt && tabs => StepTab(step(k)),
        k @ (VK_NEXT | VK_PRIOR) if ctrl && shift && !alt && tabs => MoveTab(step(k)),
        // Alt+1 … Alt+9: the nth tab, 9 the last (as a browser's Ctrl+1 … 9; Ctrl+digits are the panes). Number pad keys stay out.
        k if alt && !ctrl && !shift && tabs && (VK_1.0..=VK_9.0).contains(&k.0) => NthTab((k.0 - VK_1.0 + 1) as usize),
        k if ctrl && !shift && !alt && (VK_1.0..=VK_9.0).contains(&k.0) && ((k.0 - VK_1.0) as usize) < c.panes => Pane((k.0 - VK_1.0) as usize),
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
        VK_RETURN if addr => AddrGo,
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
        VK_A if ctrl && shift && !alt && list() => Match(false),
        VK_DOWN if alt && !ctrl && !shift => Recent,
        VK_F3 if !ctrl && !shift && !alt && list() => Peek,
        VK_F7 if !ctrl && !shift && !alt && list() => NewFolder,
        VK_F7 if !ctrl && shift && !alt && list() => NewText,
        VK_F7 if ctrl && !shift && !alt && list() => Group,
        // Shift+F5: dated copies next to the selected items (Total Commander's copy-in-place); plain F5 is Refresh above
        VK_F5 if !ctrl && shift && !alt && list() => Stamp,
        VK_V if ctrl && !shift && !alt && list() => Paste,
        _ => Shell,
    }
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

/// Space opens the preview unless a name is being typed (a key within the last second).
pub(crate) fn space_peeks(typed_ago: Option<Duration>) -> bool {
    typed_ago.is_none_or(|d| d >= Duration::from_secs(1))
}
