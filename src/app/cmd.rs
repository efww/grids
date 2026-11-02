use std::time::Duration;

use crate::imports::*;
use super::*;
use crate::ui::*;
use crate::pane::*;
use crate::session::*;
use crate::shell::*;
use crate::settings;

// ---------- keyboard ----------

// "New Text Document.txt", then "New Text Document (2).txt", … The number sits before the extension.
pub(crate) fn fresh_file(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (base, String::new()),
    };
    let mut n = 2u32;
    loop {
        let name = format!("{stem} ({n}){ext}");
        if !taken(&name) || n == 9999 {
            return name;
        }
        n += 1;
    }
}

// File created by Ctrl+Alt+N, waiting for the view to list it so it can be selected and renamed.
pub(crate) struct PendingFile {
    pub(crate) pane: usize,
    pub(crate) name: String,
    pub(crate) left: u8,
}
pub(crate) static PENDING_FILE: Mutex<Option<PendingFile>> = Mutex::new(None);

// Ctrl+Alt+N: an empty text file in the active pane, then the shell's own rename (F2) once it shows up.
pub(crate) unsafe fn new_file(i: usize) {
    if crate::pane::recent::locked(i) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let dir = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    if !is_fs(&dir) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let base = app(|a| if a.s.new_name.trim().is_empty() { default_new_name() } else { a.s.new_name.clone() });
    let name = fresh_file(&base, |n| Path::new(&dir).join(n).exists());
    let full = Path::new(&dir).join(&name);
    if std::fs::OpenOptions::new().write(true).create_new(true).open(&full).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let wide: Vec<u16> = full.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
    if let Ok(mut p) = PENDING_FILE.lock() {
        *p = Some(PendingFile { pane: i, name, left: 8 });
    }
    if let Some(h) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)) {
        SetTimer(Some(h), TIMER_NEWFILE, 40, None);
    }
    focus_view(i);
}

// The view lists the new file a moment after Refresh. Select it and start the shell rename.
pub(crate) unsafe fn select_new_file(i: usize, name: &str) -> bool {
    let Some(fv) = view_of::<IFolderView2>(i) else { return false };
    let Ok(folder) = fv.GetFolder::<IShellFolder>() else { return false };
    let mut pidl = std::ptr::null_mut();
    let parsed = folder.ParseDisplayName(HWND::default(), None, &HSTRING::from(name), None, &mut pidl, std::ptr::null_mut());
    if parsed.is_err() || pidl.is_null() {
        return false;
    }
    let Some(sv) = view_of::<IShellView>(i) else {
        CoTaskMemFree(Some(pidl as _));
        return false;
    };
    // SVSI_EDIT is the shell's rename (the same one F2 uses). If this view won't edit, selecting is enough.
    let edit = (SVSI_EDIT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    let ok = sv.SelectItem(pidl, edit).is_ok()
        || sv.SelectItem(pidl, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

thread_local! {
    static LAST_TYPED: RefCell<Option<Instant>> = const { RefCell::new(None) };
}

fn last_typed_ago() -> Option<Duration> {
    LAST_TYPED.with(|c| c.borrow().map(|t| t.elapsed()))
}

fn note_typed() {
    LAST_TYPED.with(|c| *c.borrow_mut() = Some(Instant::now()));
}

/// Keys that type into Explorer's find-as-you-type: letters, digits, punctuation, and the IME process key
/// (Korean and Japanese composition arrives as VK_PROCESSKEY, 0xE5).
pub(crate) fn types_char(vk: VIRTUAL_KEY) -> bool {
    matches!(vk.0, 0x30..=0x39 | 0x41..=0x5A | 0x60..=0x6F | 0xBA..=0xC0 | 0xDB..=0xDF | 0xE5)
}

/// Space opens the preview unless a name is being typed (a key within the last second).
pub(crate) fn space_peeks(typed_ago: Option<Duration>) -> bool {
    typed_ago.is_none_or(|d| d >= Duration::from_secs(1))
}

/// Space opens quick look when the setting is on, no name is being typed, and no IME composition is open.
pub(crate) fn space_opens(typed_ago: Option<Duration>, composing: bool, enabled: bool) -> bool {
    enabled && !composing && space_peeks(typed_ago)
}

/// What a plain F5 does. A failed network folder is retried before any binding.
/// A hold is eaten so the refresh or the retry does not run again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum F5Act {
    /// The pane is showing a network-folder error: try that path again.
    Retry,
    /// Reload the active pane.
    Refresh,
    /// Handled, but a key repeat: do nothing.
    Eat,
    /// Not this key, or a binding owns the chord.
    Pass,
}

/// `plain` is F5 with no Ctrl, Alt, or Shift. `repeat` is the key-down repeat bit.
/// `failed` is the active pane's network-folder error. `bound` means the keymap owns this chord.
/// The error wins over a binding. Otherwise an unbound F5 reloads the pane, once per press.
pub(crate) fn f5_act(plain: bool, repeat: bool, failed: bool, bound: bool) -> F5Act {
    if !plain {
        return F5Act::Pass;
    }
    if failed {
        return if repeat { F5Act::Eat } else { F5Act::Retry };
    }
    if bound {
        return F5Act::Pass;
    }
    if repeat {
        F5Act::Eat
    } else {
        F5Act::Refresh
    }
}

/// F5: reload the active pane and sum selected folders again. Missing icons are rechecked.
unsafe fn refresh_pane(a: usize) {
    resum_sizes();
    crate::ui::forget_missing();
    if let Some(side) = app(|x| (!x.side_hwnd.is_invalid()).then_some(x.side_hwnd)) {
        _ = InvalidateRect(Some(side), None, false);
    }
    // The view leaves F5 to its frame, as Explorer's does, so this works when the list
    // does not have focus (address bar, filter, folder tree, favourites).
    if let Some(sv) = view_of::<IShellView>(a) {
        _ = sv.Refresh();
    }
}

/// Korean and Japanese IMEs keep a composition string while a syllable or a reading is open.
/// Space then belongs to the IME (commit or convert), not to the preview.
pub(crate) fn ime_composing(hwnd: HWND) -> bool {
    unsafe { composition_pending(hwnd) || composition_pending(GetFocus()) }
}

unsafe fn composition_pending(start: HWND) -> bool {
    use windows::Win32::UI::Input::Ime::{ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext, GCS_COMPSTR};
    let mut h = start;
    for _ in 0..12 {
        if h.0.is_null() {
            return false;
        }
        let himc = ImmGetContext(h);
        if !himc.0.is_null() {
            let n = ImmGetCompositionStringW(himc, GCS_COMPSTR, None, 0);
            let _ = ImmReleaseContext(h, himc);
            if n > 0 {
                return true;
            }
        }
        h = GetParent(h).unwrap_or_default();
    }
    false
}

pub(crate) unsafe fn key(m: &MSG) -> bool {
    let vk = m.wParam.0 as u16;
    // A pane drag in progress: Esc drops it and does not also clear a filter or close a dialog.
    if VIRTUAL_KEY(vk) == VK_ESCAPE && cancel_pane_drag() {
        return true;
    }
    // The preview takes its own keys. Pane shortcuts must not act behind it.
    let peek = app(|a| a.peek);
    if !peek.0.is_null() && m.hwnd == peek {
        return false;
    }
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    let win = down(VK_LWIN) || down(VK_RWIN);
    // A conflict dialog is up. Let it see the key.
    if crate::keys::modal() {
        return false;
    }
    // The key that just finished a capture must not also run the command.
    if crate::keys::swallows(vk) {
        return true;
    }
    if crate::keys::capturing() {
        // The Windows key is a modifier, so a chord such as Win+Shift+E can be recorded.
        if matches!(vk, 0x5B | 0x5C) || m.lParam.0 & (1 << 30) != 0 {
            return true;
        }
        return finish_capture(vk, ctrl, shift, alt, win);
    }
    // Typing in Settings must not fire pane shortcuts. Esc closes the window.
    if settings::captures(m.hwnd) {
        if !ctrl && !alt && VIRTUAL_KEY(vk) == VK_ESCAPE {
            settings::close();
            return true;
        }
        return false;
    }
    // Batch rename is its own window. Esc closes it; Ctrl+Enter applies. Other keys are the edits.
    if crate::rename::captures(m.hwnd) {
        return crate::rename::on_key(vk, ctrl, alt, shift);
    }
    // The favourites window: Esc closes, Enter opens the selected item, arrows move through it.
    if fav_manager_captures(m.hwnd) {
        return fav_manager_on_key(vk, ctrl, alt);
    }
    if app(|a| a.matcher.as_ref().map(|b| b.edit)) == Some(m.hwnd) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => {
                app(|a| a.held = Some(VK_RETURN.0));
                run_match(shift);
            }
            VK_ESCAPE => close_match(true),
            _ => return false,
        }
        return true;
    }
    // The rename box on a session card. Enter and Esc are its own; the rest is typing.
    if !m.hwnd.0.is_null() && m.hwnd == app(|a| a.load_edit) && app(|a| a.load_editing.is_some()) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => {
                app(|x| x.held = Some(VK_RETURN.0));
                commit_load_rename();
            }
            VK_ESCAPE => _ = end_load_rename(),
            _ => return false,
        }
        return true;
    }
    if m.hwnd == app(|a| a.tab_edit) && app(|a| a.tab_renaming.is_some()) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => end_rename(true),
            VK_ESCAPE => end_rename(false),
            _ => return false,
        }
        return true;
    }
    if m.hwnd == app(|a| a.side_edit) && !app(|a| a.side_edit_at.is_empty()) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN => end_side_rename(true),
            VK_ESCAPE => end_side_rename(false),
            _ => return false,
        }
        return true;
    }
    // The file basket is its own window. Its keys stay here, including the toggle and add chords.
    if crate::basket::captures(m.hwnd) {
        if let Some(i) = crate::keys::lookup(vk, ctrl, shift, alt, win) {
            let id = crate::keys::id_of(i);
            if matches!(id, "file.basket" | "file.basket.add") {
                if m.lParam.0 & (1 << 30) != 0 {
                    return true;
                }
                return dispatch(id, vk, active_pane(), false, || false);
            }
        }
        let _ = crate::basket::on_key(vk, ctrl, shift);
        return true;
    }
    // Plain Alt+↑/↓ in the favourites panel or its list reorders that row.
    // It must not send the pane behind it to the parent folder.
    if fav_step(vk, ctrl, alt, shift).is_some() {
        let (side, fav) = app(|a| (a.side_hwnd, a.fav_popup));
        let on = |root: HWND| !root.0.is_null() && (m.hwnd == root || IsChild(root, m.hwnd).as_bool());
        if on(side) || on(fav) {
            return false;
        }
    }
    if let Some(i) = pane_of(m.hwnd) {
        set_active(i);
    }
    let a = active_pane();
    let addr = pane_ctl(a, ID_EDIT) == Some(m.hwnd);
    let filt = pane_ctl(a, ID_FILTER) == Some(m.hwnd);
    // Focus often sits on MaguroPane (the host) rather than DefView. Ctrl+A/C/V and other
    // Explorer accelerators must still reach the shell view, or copy/paste creates nothing.
    let shell = || {
        let Some(sv) = view_of::<IShellView>(a) else { return false };
        let svh = sv.GetWindow().unwrap_or_default();
        if svh.0.is_null() {
            return false;
        }
        let on_view = svh == m.hwnd || IsChild(svh, m.hwnd).as_bool();
        // Address/filter return before this closure runs. Pane chrome is fine to forward.
        let on_pane = pane_of(m.hwnd) == Some(a);
        (on_view || on_pane) && sv.TranslateAccelerator(m) == S_OK
    };
    // view_list is the items surface: SysListView32, or the DirectUIHWND Windows 10 uses instead.
    // Focus on a child of the shell view (a scrollbar, an in-place edit) still counts by class.
    // Click often leaves focus on MaguroPane rather than DefView — still the items surface for
    // paste/delete/Space (address/filter return before this runs).
    let list = view_of::<IFolderView2>(a).is_some_and(|fv| {
        if view_list(&fv) == m.hwnd {
            return true;
        }
        let svh = fv.cast::<IShellView>().and_then(|v| v.GetWindow()).unwrap_or_default();
        let mut c = [0u16; 32];
        let n = GetClassNameW(m.hwnd, &mut c) as usize;
        if matches!(String::from_utf16_lossy(&c[..n]).as_str(), "DirectUIHWND" | "SysListView32")
            && !svh.0.is_null()
            && (svh == m.hwnd || IsChild(svh, m.hwnd).as_bool())
        {
            return true;
        }
        pane_of(m.hwnd) == Some(a)
    });
    // A failed share keeps its path. F5 retries that path; the shell's refresh would reload the
    // previous folder and leave the failure behind. The error view's Retry button does the same.
    // A hold does not retry again. This runs before key bindings, so a rebound F5 still retries.
    let repeat = m.lParam.0 & (1 << 30) != 0;
    let plain_f5 = VIRTUAL_KEY(vk) == VK_F5 && !ctrl && !alt && !shift;
    if net_failed(a) && plain_f5 {
        if f5_act(true, repeat, true, false) == F5Act::Retry {
            retry_failed(a);
        }
        return true;
    }
    if pane_ctl(a, ID_NETFAIL) == Some(m.hwnd) {
        match VIRTUAL_KEY(vk) {
            VK_RETURN if !ctrl && !alt => retry_failed(a),
            VK_ESCAPE if !ctrl && !alt => leave_net_fail(a),
            _ => return false,
        }
        return true;
    }
    // Explorer's find-as-you-type: a letter, digit, punctuation or IME process key in the list
    // starts the one-second window in which Space stays part of the name.
    if list && !ctrl && !alt && types_char(VIRTUAL_KEY(vk)) {
        note_typed();
    }
    // F6 leaves the address and the filter. Every other chord stays out of a name being typed.
    if let Some(i) = crate::keys::lookup(vk, ctrl, shift, alt, win) {
        let id = crate::keys::id_of(i);
        if id == "app.keys" && m.lParam.0 & (1 << 30) != 0 {
            return true; // holding Ctrl+/ toggles once
        }
        if matches!(id, "file.basket" | "file.basket.add") && m.lParam.0 & (1 << 30) != 0 {
            return true;
        }
        if matches!(id, "focus.next" | "focus.prev" | "app.keys" | "file.basket" | "file.basket.add") {
            return dispatch(id, vk, a, list, shell);
        }
    }
    // Plain F5 reloads the active pane from anywhere in the window: file list, address bar,
    // filter, folder tree, favourites. A hold does not refresh again. A rebound F5 keeps that
    // command. A failed share was retried above.
    let bound = crate::keys::lookup(vk, ctrl, shift, alt, win).is_some();
    match f5_act(plain_f5, repeat, false, bound) {
        F5Act::Refresh => {
            refresh_pane(a);
            return true;
        }
        F5Act::Eat => return true,
        _ => {}
    }
    // Address, filter, and tab rename keep every key except Enter and Esc.
    // A rebound letter must not land in the middle of a name.
    if addr || filt {
        match VIRTUAL_KEY(vk) {
            VK_RETURN if addr => go_edit(a),
            VK_RETURN if filt => {
                if !open_find_sel(a) {
                    apply_filter(a);
                    focus_view(a);
                }
            }
            VK_ESCAPE if addr => addr_reset(a, m.hwnd),
            VK_ESCAPE if filt => {
                find_clear(a);
                _ = SetWindowTextW(m.hwnd, w!(""));
                apply_filter(a);
                focus_view(a);
            }
            // Alt+↓ in the filter is the file-kind menu. Recent folders keeps Alt+↓ everywhere else.
            VK_DOWN if filt && alt && !ctrl && !shift => {
                if m.lParam.0 & (1 << 30) == 0 {
                    kind_menu(a);
                }
            }
            VK_DOWN if filt && !alt && !ctrl && !shift => {
                if !nudge_find(a, 1) {
                    return false;
                }
            }
            VK_UP if filt && !alt && !ctrl && !shift => {
                if !nudge_find(a, -1) {
                    return false;
                }
            }
            VK_NEXT if filt && !alt => {
                if !nudge_find(a, 8) {
                    return false;
                }
            }
            VK_PRIOR if filt && !alt => {
                if !nudge_find(a, -8) {
                    return false;
                }
            }
            _ => return false,
        }
        return true;
    }
    if let Some(i) = crate::keys::lookup(vk, ctrl, shift, alt, win) {
        let id = crate::keys::id_of(i);
        if matches!(id, "app.keys" | "view.peek" | "view.preview" | "app.front" | "dialog.jump" | "view.gather" | "view.recentfiles" | "file.mkdir" | "file.winmerge" | "file.reveal" | "file.basket" | "file.basket.add" | "ws.load" | "ws.save") && m.lParam.0 & (1 << 30) != 0 {
            return true;
        }
        // Pane-tab chords stay out of the address bar (already returned) and out of every other focus,
        // so they never take Ctrl+T, Ctrl+W or Ctrl+Page Down from the window tabs and the panes.
        if matches!(id, "pane.tab.new" | "pane.tab.close" | "pane.tab.next" | "pane.tab.prev") {
            let in_pane = list || pane_of(m.hwnd).is_some();
            if !in_pane {
                return shell();
            }
        }
        return dispatch(id, vk, a, list, shell);
    }
    // Recent files is read-only. These keys would delete, rename, cut, or paste a shortcut.
    if list && crate::pane::recent::locked(a) && crate::pane::recent::blocks_key(vk, ctrl, alt, shift) {
        if m.lParam.0 & (1 << 30) == 0 {
            _ = MessageBeep(MB_ICONWARNING);
        }
        return true;
    }
    // Not an app shortcut. Del and Ctrl+V still belong to the file list
    // unless the user has bound that chord above. F5 was handled above.
    // Read the typing clock before a composing Space refreshes it.
    let ch = VIRTUAL_KEY(vk);
    let typed_ago = last_typed_ago();
    let composing = ch == VK_SPACE && list && !ctrl && !alt && ime_composing(m.hwnd);
    if composing {
        note_typed();
    }
    match ch {
        VK_DELETE if !ctrl && !alt && list => {
            delete_selection(a, shift);
            true
        }
        VK_V if ctrl && !shift && !alt && list => {
            if !paste_key(m.lParam.0 & (1 << 30) != 0, || try_paste(a)) {
                shell()
            } else {
                true
            }
        }
        // Space: quick look, as in Finder. The preview's own Space still closes it.
        // A space soon after typing, or while an IME is composing, stays with the list.
        VK_SPACE if !ctrl && !shift && !alt && list && space_opens(typed_ago, composing, app(|x| x.s.space_preview)) => {
            if m.lParam.0 & (1 << 30) == 0 {
                open_peek(a);
            }
            true
        }
        // A click on a result row focuses that list. Space then previews the selected row
        // at once: this is not the file list's type-ahead. The setting still turns it off,
        // and an open IME composition keeps the space.
        VK_SPACE if !ctrl && !shift && !alt && find_focused(a) && app(|x| x.s.space_preview) && !ime_composing(m.hwnd) => {
            if m.lParam.0 & (1 << 30) == 0 {
                preview_find_sel(a);
            }
            true
        }
        _ => shell(),
    }
}

unsafe fn finish_capture(vk: u16, ctrl: bool, shift: bool, alt: bool, win: bool) -> bool {
    match crate::keys::offer(vk, ctrl, shift, alt, win) {
        crate::keys::Offer::Ignore => true,
        crate::keys::Offer::Cancel | crate::keys::Offer::Same => {
            crate::keys::swallow(vk);
            settings::redraw();
            true
        }
        crate::keys::Offer::Bind { row, chord, other } => {
            if let Some(other) = other {
                let name = crate::keys::action(other);
                if !settings::confirm_swap(&crate::keys::format_chord(&chord), &name) {
                    crate::keys::swallow(vk);
                    settings::redraw();
                    return true;
                }
            }
            crate::keys::rebind(row, chord);
            crate::keys::swallow(vk);
            apply_hotkey(true);
            crate::dialogjump::sync();
            save_session();
            settings::refresh();
            true
        }
    }
}

unsafe fn dispatch(id: &str, vk: u16, a: usize, list: bool, shell: impl Fn() -> bool) -> bool {
    match id {
        "tab.new" => new_tab(),
        "tab.close" => close_tab(app(|x| x.s.tab)),
        "tab.next" => switch_rel(1),
        "tab.prev" => switch_rel(-1),
        "tab.reopen" => reopen_closed_at(0),
        "tab.dup" => duplicate_tab(app(|x| x.s.tab)),
        "pane.next" => cycle(1),
        "pane.prev" => cycle(-1),
        "pane.goto" => {
            let n = if (0x31..=0x39).contains(&vk) { (vk - 0x31) as usize } else { 0 };
            if n >= app(|x| x.s.panes()) {
                return shell();
            }
            set_active(n);
            focus_view(n);
        }
        "pane.left" => move_pane(-1, 0),
        "pane.right" => move_pane(1, 0),
        "pane.up" => move_pane(0, -1),
        "pane.down" => move_pane(0, 1),
        "pane.swap.left" => swap_dir(-1, 0),
        "pane.swap.right" => swap_dir(1, 0),
        "pane.swap.up" => swap_dir(0, -1),
        "pane.swap.down" => swap_dir(0, 1),
        "nav.back" => browse(a, SBSP_NAVIGATEBACK),
        "nav.fwd" => browse(a, SBSP_NAVIGATEFORWARD),
        "nav.up" => browse(a, SBSP_PARENT),
        "nav.recent" => recent_menu(a),
        "nav.addr" | "nav.addr2" | "nav.addr3" => return focus_ctl(a, ID_EDIT),
        "view.filter" => return focus_ctl(a, ID_FILTER),
        "view.peek" if list => open_peek(a),
        "view.peek" if find_focused(a) => preview_find_sel(a),
        "view.peek" => return shell(),
        "view.preview" => toggle_preview(),
        "focus.next" => cycle_focus(false),
        "focus.prev" => cycle_focus(true),
        "view.hidden" => toggle_hidden(),
        "view.panel" => toggle_tree(),
        "view.gather" => gather_explorer(),
        "view.recentfiles" => crate::pane::recent::open_recent(a),
        "view.zoom" => toggle_zoom(),
        "file.sel" | "file.sel2" if list => {
            app(|a| a.held = Some(vk));
            open_match(a, false);
        }
        "file.desel" if list => {
            app(|a| a.held = Some(vk));
            open_match(a, true);
        }
        "file.invert" if list => {
            app(|a| a.held = Some(vk));
            invert_selection(a);
        }
        "file.sel" | "file.sel2" | "file.desel" | "file.invert" => return shell(),
        "file.path" => {
            if !list {
                return shell();
            }
            copy_paths(a);
        }
        "file.rename" => {
            focus_view(a);
            crate::rename::open_pane(a);
        }
        "file.new" => new_file(a),
        "file.dir" => new_folder(a),
        "file.mkdir" if list => explorer_new_folder(a),
        "file.mkdir" => return shell(),
        "file.winmerge" => crate::winmerge::run_folders(app(|x| x.main), None),
        "file.basket" => crate::basket::toggle(),
        "file.basket.add" => crate::basket::add_active_selection(),
        "file.reveal" => {
            if !crate::pane::recent::locked(a) {
                return shell();
            }
            crate::pane::recent::show_selection(a);
        }
        "pane.tab.new" => {
            new_pane_tab(a);
            focus_view(a);
        }
        "pane.tab.close" => {
            let k = app(|x| {
                let n = x.s.pane_tab_list(a).len();
                if n > 1 { x.s.pane_cur[a].min(n - 1) } else { 0 }
            });
            close_pane_tab(a, k);
        }
        "pane.tab.next" => step_pane_tab(a, 1),
        "pane.tab.prev" => step_pane_tab(a, -1),
        "sess.save" => save_current(),
        "ws.load" => {
            if let Some(n) = workspace_digit(vk) {
                load_workspace(n);
            }
        }
        "ws.save" => {
            if let Some(n) = workspace_digit(vk) {
                save_workspace(n);
            }
        }
        "fav.list" => open_favs(),
        "app.settings" => settings::open(app(|x| x.main)),
        "app.help" => open_help(),
        "app.keys" => toggle_keys(),
        "app.front" => hotkey_pressed(),
        // Eat the chord inside Maguro too, so it does not type the letter. The menu opens
        // only when another app's file dialog is actually in front.
        "dialog.jump" => crate::dialogjump::on_hotkey(),
        "win.new" => new_window(),
        _ => return shell(),
    }
    true
}

// ---------- mouse side buttons ----------
//
// A local mouse driver posts WM_APPCOMMAND, or a VK_BROWSER_BACK / VK_BROWSER_FORWARD key.
// The pane window handles the command and walks history. Over Remote Desktop the virtual
// channel posts WM_XBUTTON* at the window under the cursor instead. That window is the shell
// view hosted by IExplorerBrowser (the list, DirectUI, or its tree). The view consumes the
// message, DefWindowProc never synthesises WM_APPCOMMAND, and the pane handler never runs.
// Explorer itself handles WM_XBUTTON*, which is why a remote Explorer window still goes back.
//
// The main loop takes the button before TranslateMessage / DispatchMessage, for any window
// of this process that sits in a pane, and calls the same nav_pane path as WM_APPCOMMAND.
// Down, double-click, and the non-client copies are swallowed so the view cannot handle
// them too. One physical click still moves once when the driver also posts WM_APPCOMMAND:
// the two deliveries share a short claim and the second one does not navigate. Quick look,
// settings, and the other popups are not panes, so a side button there is left alone.

/// Two deliveries of one click can sit a few messages apart. A later click is a new one.
pub(crate) const SIDE_PAIR: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SideDir {
    Back,
    Forward,
}

/// Which queue entry carried the click. Client and non-client button-up are one click,
/// and so are a button and a mouse command. A browser key and a key command are one press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SidePath {
    Button,
    ButtonNc,
    AppMouse,
    AppKey,
    Key,
}

#[derive(Clone, Copy)]
struct SideMark {
    dir: SideDir,
    path: SidePath,
    at: Instant,
}

thread_local! {
    static SIDE_MARK: Cell<Option<SideMark>> = const { Cell::new(None) };
}

pub(crate) fn side_flags(dir: SideDir) -> u32 {
    match dir {
        SideDir::Back => SBSP_NAVIGATEBACK,
        SideDir::Forward => SBSP_NAVIGATEFORWARD,
    }
}

/// Back, forward, and parent: make that pane current, walk, and leave the file list focused.
pub(crate) unsafe fn nav_pane(i: usize, flags: u32) {
    set_active(i);
    browse(i, flags);
    focus_view(i);
}

pub(crate) fn xbutton_dir(wparam: usize) -> Option<SideDir> {
    match ((wparam >> 16) & 0xffff) as u16 {
        XBUTTON1 => Some(SideDir::Back),
        XBUTTON2 => Some(SideDir::Forward),
        _ => None,
    }
}

/// Back / forward from `WM_APPCOMMAND`. Mouse and OEM share the button claim. A key
/// command shares the browser-key claim. Every other command stays with DefWindowProc.
pub(crate) fn appcommand_event(lp: isize) -> Option<(SideDir, SidePath)> {
    let hi = (lp >> 16) as u32;
    let dir = match hi & 0x0fff {
        1 => SideDir::Back,
        2 => SideDir::Forward,
        _ => return None,
    };
    let path = if hi & FAPPCOMMAND_MASK == FAPPCOMMAND_KEY { SidePath::AppKey } else { SidePath::AppMouse };
    Some((dir, path))
}

fn side_pair(a: SidePath, b: SidePath) -> bool {
    use SidePath::*;
    matches!(
        (a, b),
        (Button, ButtonNc) | (ButtonNc, Button)
            | (Button, AppMouse) | (AppMouse, Button)
            | (ButtonNc, AppMouse) | (AppMouse, ButtonNc)
            | (Key, AppKey) | (AppKey, Key)
    )
}

/// The first delivery navigates. The paired delivery of the same direction, inside
/// [`SIDE_PAIR`], does not. The mark stays so a third copy of that click is dropped too.
fn side_admit(prev: Option<SideMark>, dir: SideDir, path: SidePath, now: Instant) -> (bool, Option<SideMark>) {
    if let Some(mark) = prev {
        if mark.dir == dir && side_pair(mark.path, path) && now.saturating_duration_since(mark.at) <= SIDE_PAIR {
            return (false, Some(mark));
        }
    }
    (true, Some(SideMark { dir, path, at: now }))
}

pub(crate) fn side_claim(dir: SideDir, path: SidePath) -> bool {
    let now = Instant::now();
    SIDE_MARK.with(|cell| {
        let (ok, next) = side_admit(cell.get(), dir, path, now);
        cell.set(next);
        ok
    })
}

fn xbutton_up(msg: u32) -> Option<bool> {
    match msg {
        WM_XBUTTONUP | WM_NCXBUTTONUP => Some(true),
        WM_XBUTTONDOWN | WM_NCXBUTTONDOWN | WM_XBUTTONDBLCLK | WM_NCXBUTTONDBLCLK => Some(false),
        _ => None,
    }
}

unsafe fn same_process(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(h, Some(&mut pid));
    pid == std::process::id()
}

/// Popups that are not a folder pane. A side button there keeps whatever it did before.
unsafe fn side_popup(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    if settings::captures(h) || crate::rename::captures(h) || fav_manager_captures(h) || crate::basket::captures(h) {
        return true;
    }
    let (peek, load, grid, keys, help, fav, match_hwnd) = app(|a| {
        (a.peek, a.load, a.grid, a.keys_popup, a.help_popup, a.fav_popup, a.matcher.as_ref().map(|m| m.hwnd).unwrap_or_default())
    });
    let hit = |root: HWND| !root.0.is_null() && (h == root || IsChild(root, h).as_bool());
    hit(peek) || hit(load) || hit(grid) || hit(keys) || hit(help) || hit(fav) || hit(match_hwnd)
}

unsafe fn pane_from_hwnd(h: HWND) -> Option<usize> {
    let mut cur = h;
    for _ in 0..16 {
        if cur.0.is_null() {
            return None;
        }
        if let Some(i) = pane_of(cur) {
            return Some(i);
        }
        let parent = GetParent(cur).unwrap_or_default();
        if parent.0.is_null() || parent == cur {
            return None;
        }
        cur = parent;
    }
    None
}

unsafe fn pane_under_cursor() -> Option<usize> {
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() {
        return None;
    }
    let under = WindowFromPoint(pt);
    if !same_process(under) {
        return None;
    }
    let cur = app(|a| a.cur);
    let switched = bind_hwnd(under);
    let found = pane_from_hwnd(under);
    if found.is_none() && switched {
        bind_index(cur);
    }
    found
}

unsafe fn history_allowed() -> bool {
    let main = app(|a| a.main);
    !main.0.is_null() && IsWindowEnabled(main).as_bool()
}

unsafe fn pane_for_button(hwnd: HWND) -> Option<usize> {
    let cur = app(|a| a.cur);
    let found = pane_from_hwnd(hwnd).or_else(|| pane_under_cursor());
    if found.is_some() && history_allowed() {
        found
    } else {
        bind_index(cur);
        None
    }
}

/// `true` when this message was a side button on one of our panes and must not be dispatched.
unsafe fn take_side_button(m: &MSG) -> bool {
    let Some(up) = xbutton_up(m.message) else { return false };
    if !same_process(m.hwnd) || side_popup(m.hwnd) {
        return false;
    }
    let Some(i) = pane_for_button(m.hwnd) else { return false };
    kids_cancel();
    if up {
        if let Some(dir) = xbutton_dir(m.wParam.0) {
            let path = if m.message == WM_NCXBUTTONUP { SidePath::ButtonNc } else { SidePath::Button };
            if side_claim(dir, path) {
                nav_pane(i, side_flags(dir));
            }
        }
    }
    true
}

fn browser_key_dir(vk: u16) -> Option<SideDir> {
    match VIRTUAL_KEY(vk) {
        VK_BROWSER_BACK => Some(SideDir::Back),
        VK_BROWSER_FORWARD => Some(SideDir::Forward),
        _ => None,
    }
}

/// Browser back / forward keys, when focus is already inside a pane. Swallowing the key
/// keeps DefWindowProc from also posting WM_APPCOMMAND for the same press.
unsafe fn take_browser_key(m: &MSG) -> bool {
    if m.message != WM_KEYDOWN {
        return false;
    }
    let Some(dir) = browser_key_dir(m.wParam.0 as u16) else { return false };
    if !same_process(m.hwnd) || side_popup(m.hwnd) {
        return false;
    }
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    if down(VK_CONTROL) || down(VK_MENU) || down(VK_SHIFT) {
        return false;
    }
    let Some(i) = pane_from_hwnd(m.hwnd) else { return false };
    if !history_allowed() {
        return false;
    }
    if side_claim(dir, SidePath::Key) {
        nav_pane(i, side_flags(dir));
    }
    true
}

pub(crate) unsafe fn pre_translate(m: &MSG) -> bool {
    bind_hwnd(m.hwnd);
    if take_side_button(m) {
        return true;
    }
    let in_tree = || app(|a| !a.tree_hwnd.0.is_null() && (a.tree_hwnd == m.hwnd || IsChild(a.tree_hwnd, m.hwnd).as_bool()));
    match m.message {
        WM_MOUSEMOVE => {
            set_hover(if is_icon_button(m.hwnd) { m.hwnd } else { HWND::default() });
            // The folder list does not deliver WM_MOUSELEAVE to the favourites panel.
            // A move anywhere else takes the location tip down.
            hide_fav_tip_if_pointer_left(m.hwnd);
            // Same gap as favourites: the folder list may not deliver WM_MOUSELEAVE to the pane.
            hide_addr_tip_if_pointer_left(m.hwnd);
            false
        }
        WM_MOUSELEAVE => {
            if app(|a| a.hover == m.hwnd) {
                set_hover(HWND::default());
            }
            false
        }
        WM_MOUSEWHEEL => {
            let tabs = app(|a| a.tabs_hwnd);
            if tabs.0.is_null() {
                return false;
            }
            let pt = POINT { x: (m.lParam.0 & 0xffff) as i16 as i32, y: ((m.lParam.0 >> 16) & 0xffff) as i16 as i32 };
            let under = WindowFromPoint(pt);
            if under == tabs || IsChild(tabs, under).as_bool() {
                tab_wheel((m.wParam.0 >> 16) as i16 as i32);
                return true;
            }
            false
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            kids_cancel();
            if m.message == WM_LBUTTONDOWN && in_tree() {
                app(|a| a.tree_input = true);
            }
            if let Some(i) = pane_of(m.hwnd) {
                set_active(i);
            }
            false
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = m.wParam.0 as u16;
            if vk == VK_CONTROL.0 || vk == VK_LCONTROL.0 || vk == VK_RCONTROL.0 {
                refresh_pane_drag();
            }
            kids_cancel();
            let (eat, next) = held_key(app(|a| a.held), vk, m.lParam.0 & (1 << 30) != 0);
            app(|a| a.held = next);
            if eat {
                return true;
            }
            if take_browser_key(m) {
                return true;
            }
            // Cleared before key() so a selection the tree reports while we handle Ctrl+Tab or F6
            // is not taken for a click in the tree (that would reload the pane and drop Forward).
            let tree = m.message == WM_KEYDOWN && in_tree();
            let prev = app(|a| std::mem::take(&mut a.tree_input));
            let used = key(m);
            app(|a| a.tree_input = tree_input_after(prev, tree, used));
            used
        }
        WM_KEYUP | WM_SYSKEYUP => {
            let vk = m.wParam.0 as u16;
            if vk == VK_CONTROL.0 || vk == VK_LCONTROL.0 || vk == VK_RCONTROL.0 {
                refresh_pane_drag();
            }
            app(|a| a.held = a.held.filter(|&k| k != vk));
            crate::keys::clear_swallow(vk);
            false
        }
        _ => false,
    }
}

/// F6 ring: 0 file list, 1 address, 2 filter, then the upper left-panel half and the lower one.
/// Favourites lead when they are on top. A hidden panel drops the last two stops.
pub(crate) fn focus_ring(pos: usize, back: bool, panel: bool) -> usize {
    let n = if panel { 5 } else { 3 };
    (pos.min(n - 1) + if back { n - 1 } else { 1 }) % n
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum PanelStop {
    Tree,
    Fav,
}

/// Upper half, then lower half.
pub(crate) fn panel_stops(fav_top: bool) -> [PanelStop; 2] {
    if fav_top { [PanelStop::Fav, PanelStop::Tree] } else { [PanelStop::Tree, PanelStop::Fav] }
}

/// 0 file list, 1 address, 2 filter, 3 upper left-panel half, 4 lower half.
pub(crate) fn focus_at(addr: bool, filt: bool, in_tree: bool, in_fav: bool, fav_top: bool) -> usize {
    if addr {
        return 1;
    }
    if filt {
        return 2;
    }
    let [top, _] = panel_stops(fav_top);
    if in_tree {
        return if top == PanelStop::Tree { 3 } else { 4 };
    }
    if in_fav {
        return if top == PanelStop::Fav { 3 } else { 4 };
    }
    0
}

/// A key Maguro handled (Ctrl+Tab, F6, …) must not count as input in the tree.
/// An unhandled key in the tree still does, so the arrows navigate the pane.
pub(crate) fn tree_input_after(prev: bool, in_tree: bool, used: bool) -> bool {
    !used && (prev || in_tree)
}

unsafe fn cycle_focus(back: bool) {
    let a = active_pane();
    let f = GetFocus();
    let (panel, th, side, fav_top) = app(|x| (x.s.tree_on, x.tree_hwnd, x.side_hwnd, x.s.fav_top));
    let shown = panel && !th.0.is_null() && IsWindowVisible(th).as_bool();
    let addr = pane_ctl(a, ID_EDIT) == Some(f);
    let filt = pane_ctl(a, ID_FILTER) == Some(f);
    let in_tree = !th.0.is_null() && (th == f || IsChild(th, f).as_bool());
    let in_fav = !side.0.is_null() && (side == f || IsChild(side, f).as_bool());
    let pos = focus_at(addr, filt, in_tree, in_fav, fav_top);
    match focus_ring(pos, back, shown) {
        1 => {
            _ = focus_ctl(a, ID_EDIT);
        }
        2 => {
            _ = focus_ctl(a, ID_FILTER);
        }
        n @ 3 | n @ 4 => match panel_stops(fav_top)[n - 3] {
            PanelStop::Tree => {
                let tv = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()).unwrap_or(th);
                _ = SetFocus(Some(tv));
                reveal_tree_selection();
            }
            PanelStop::Fav => {
                if app(|x| x.side_sel.is_none()) {
                    app(|x| x.side_sel = Some(0));
                }
                _ = SetFocus(Some(side));
                _ = InvalidateRect(Some(side), None, false);
            }
        },
        _ => focus_view(a),
    }
}

#[cfg(test)]
mod focus_tests {
    use super::{focus_at, focus_ring, panel_stops, tree_input_after, PanelStop};

    #[test]
    fn focus_rings() {
        let walk = |back, panel| (0..5).map(|p| focus_ring(p, back, panel)).collect::<Vec<_>>();
        assert_eq!(walk(false, true), [1, 2, 3, 4, 0]);
        assert_eq!(walk(true, true), [4, 0, 1, 2, 3]);
        assert_eq!(walk(false, false)[..3], [1, 2, 0]);
        assert_eq!(walk(true, false)[..3], [2, 0, 1]);
        // A stop that only exists while the panel is open falls back into the short ring.
        assert_eq!(focus_ring(4, false, false), 0);
        assert_eq!(focus_ring(3, true, false), 1);
    }

    #[test]
    fn focus_follows_the_panel_order() {
        assert_eq!(panel_stops(true), [PanelStop::Fav, PanelStop::Tree]);
        assert_eq!(panel_stops(false), [PanelStop::Tree, PanelStop::Fav]);
        assert_eq!(focus_at(false, false, false, true, true), 3);
        assert_eq!(focus_at(false, false, true, false, true), 4);
        assert_eq!(focus_at(false, false, true, false, false), 3);
        assert_eq!(focus_at(false, false, false, true, false), 4);
        assert_eq!(focus_at(true, false, true, false, true), 1);
        assert_eq!(focus_at(false, true, false, true, false), 2);
        assert_eq!(focus_at(false, false, false, false, true), 0);
        // F6 from the filter lands on whichever half is on top, then the other half.
        assert_eq!(focus_ring(2, false, true), 3);
        assert_eq!(panel_stops(true)[focus_ring(2, false, true) - 3], PanelStop::Fav);
        assert_eq!(panel_stops(false)[focus_ring(2, false, true) - 3], PanelStop::Tree);
        assert_eq!(focus_ring(3, false, true), 4);
        assert_eq!(panel_stops(true)[focus_ring(3, false, true) - 3], PanelStop::Tree);
        assert_eq!(focus_ring(4, true, true), 3);
        assert_eq!(panel_stops(false)[focus_ring(4, true, true) - 3], PanelStop::Tree);
    }

    #[test]
    fn tree_input_kept_only_for_tree_keys() {
        assert!(tree_input_after(false, true, false)); // arrow in the tree
        assert!(!tree_input_after(true, true, true)); // Ctrl+Tab / F6 in the tree
        assert!(tree_input_after(true, false, false) && !tree_input_after(false, false, false));
    }
}

#[cfg(test)]
mod tests {
    use super::{space_opens, space_peeks, types_char};
    use std::time::Duration;
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_7, VK_A, VK_DOWN, VK_F3, VK_RETURN, VK_SPACE, VIRTUAL_KEY};

    #[test]
    fn space_previews() {
        assert!(space_peeks(None) && space_peeks(Some(Duration::from_millis(1500))));
        assert!(!space_peeks(Some(Duration::from_millis(300)))); // "my f|": the space is part of the name
        assert!(types_char(VK_A) && types_char(VK_7) && types_char(VIRTUAL_KEY(0xE5))); // 0xE5: Korean / Japanese IME
        assert!(!types_char(VK_SPACE) && !types_char(VK_DOWN) && !types_char(VK_F3) && !types_char(VK_RETURN));
        assert!(space_opens(None, false, true));
        assert!(!space_opens(Some(Duration::from_millis(300)), false, true));
        assert!(!space_opens(None, true, true)); // IME composition: the space stays in the name
        assert!(!space_opens(None, false, false)); // space_preview off
    }

    #[test]
    fn f5_retries_a_failed_share_and_refreshes_once_everywhere_else() {
        use super::{f5_act, F5Act};
        // The error view: first press retries that path, a hold does not, and a binding cannot steal it.
        assert_eq!(f5_act(true, false, true, false), F5Act::Retry);
        assert_eq!(f5_act(true, false, true, true), F5Act::Retry);
        assert_eq!(f5_act(true, true, true, false), F5Act::Eat);
        assert_eq!(f5_act(true, true, true, true), F5Act::Eat);
        // Anywhere else, with the list or not: one refresh. A hold is eaten.
        assert_eq!(f5_act(true, false, false, false), F5Act::Refresh);
        assert_eq!(f5_act(true, true, false, false), F5Act::Eat);
        // Ctrl/Alt/Shift, and a rebound F5, stay with their own command (including its repeats).
        assert_eq!(f5_act(false, false, true, false), F5Act::Pass);
        assert_eq!(f5_act(false, false, false, false), F5Act::Pass);
        assert_eq!(f5_act(true, false, false, true), F5Act::Pass);
        assert_eq!(f5_act(true, true, false, true), F5Act::Pass);
    }
}

#[cfg(test)]
mod side_tests {
    use super::{appcommand_event, side_admit, xbutton_dir, SideDir, SidePath, SIDE_PAIR};
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::{FAPPCOMMAND_KEY, FAPPCOMMAND_MASK, FAPPCOMMAND_MOUSE, FAPPCOMMAND_OEM, XBUTTON1, XBUTTON2};

    fn pack(cmd: u32, dev: u32) -> isize {
        let hi = (cmd & 0x0fff) | (dev & FAPPCOMMAND_MASK);
        ((hi << 16) as u32 as i32) as isize
    }

    fn run(events: &[(SideDir, SidePath, u64)]) -> Vec<bool> {
        let t0 = Instant::now();
        let mut prev = None;
        let mut out = Vec::new();
        for &(dir, path, ms) in events {
            let (ok, next) = side_admit(prev, dir, path, t0 + Duration::from_millis(ms));
            out.push(ok);
            prev = next;
        }
        out
    }

    #[test]
    fn xbutton_word_is_back_or_forward() {
        assert_eq!(xbutton_dir((XBUTTON1 as usize) << 16 | 0x0008), Some(SideDir::Back));
        assert_eq!(xbutton_dir((XBUTTON2 as usize) << 16), Some(SideDir::Forward));
        assert_eq!(xbutton_dir(XBUTTON1 as usize), None);
        assert_eq!(xbutton_dir(3usize << 16), None);
    }

    #[test]
    fn appcommand_keeps_only_back_and_forward() {
        // The mouse device flag sets the top bit of lParam. It arrives sign-extended.
        assert_eq!(appcommand_event(pack(1, FAPPCOMMAND_MOUSE)), Some((SideDir::Back, SidePath::AppMouse)));
        assert_eq!(appcommand_event(pack(2, FAPPCOMMAND_KEY)), Some((SideDir::Forward, SidePath::AppKey)));
        assert_eq!(appcommand_event(pack(1, FAPPCOMMAND_OEM)), Some((SideDir::Back, SidePath::AppMouse)));
        assert_eq!(appcommand_event(pack(8, FAPPCOMMAND_KEY)), None);
        assert!(pack(1, FAPPCOMMAND_MOUSE) < 0);
    }

    #[test]
    fn one_click_navigates_once_from_either_order() {
        let back = SideDir::Back;
        let fwd = SideDir::Forward;
        // Local: button then the driver's mouse command, or the command first.
        assert_eq!(
            run(&[(back, SidePath::Button, 0), (back, SidePath::AppMouse, 5), (back, SidePath::AppMouse, 10)]),
            [true, false, false]
        );
        assert_eq!(run(&[(back, SidePath::AppMouse, 0), (back, SidePath::Button, 5)]), [true, false]);
        // Remote Desktop posts only the button. A second click is a second step.
        assert_eq!(run(&[(back, SidePath::Button, 0), (back, SidePath::Button, 20)]), [true, true]);
        // Driver that only posts the command still moves.
        assert_eq!(run(&[(fwd, SidePath::AppMouse, 0)]), [true]);
        // Non-client up is the same click as the client up, and as the mouse command.
        assert_eq!(run(&[(back, SidePath::ButtonNc, 0), (back, SidePath::Button, 5)]), [true, false]);
        assert_eq!(run(&[(fwd, SidePath::AppMouse, 0), (fwd, SidePath::ButtonNc, 5)]), [true, false]);
        // After the pairing window, the other path is a new click.
        let later = SIDE_PAIR.as_millis() as u64 + 1;
        assert_eq!(run(&[(back, SidePath::Button, 0), (back, SidePath::AppMouse, later)]), [true, true]);
        // Back does not eat forward. A browser key pairs with a key command, not with the mouse.
        assert_eq!(run(&[(back, SidePath::Button, 0), (fwd, SidePath::AppMouse, 5)]), [true, true]);
        assert_eq!(
            run(&[(back, SidePath::Key, 0), (back, SidePath::AppKey, 5), (back, SidePath::Key, 40)]),
            [true, false, true]
        );
        assert_eq!(run(&[(back, SidePath::Key, 0), (back, SidePath::AppMouse, 5)]), [true, true]);
    }
}

