use crate::*;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};

// ---------- Ctrl+Alt+G: another app's Open / Save dialog jumps to a folder open in Grids ----------
// The chord is registered only while such a dialog is in front (a foreground WinEvent hook says when),
// so everywhere else Ctrl+Alt+G stays with its app. Off in the View menu: no hook, no hotkey.
// Navigation types the folder into the dialog's file name box and presses Enter: the only way both the
// old and the new (IFileDialog) dialogs move and stay open. WM_SETTEXT alone leaves their state stale.

pub(crate) const DLG_HOTKEY_ID: i32 = 2; // 1 is Win+Alt+E
const FILENAME_ID: i32 = 0x47C; // cmb13: the new dialog's file name combo (its Edit has the same id)
const EDT1_ID: i32 = 0x480; // edt1: older dialogs
pub(crate) const TIMER_DLG_RETRY: usize = 1141; // timers on main (numbers from this feature's id range)
pub(crate) const TIMER_DLG_RESTORE: usize = 1142;
const RETRIES: u8 = 5; // a dialog just shown may not have its file name box yet: look again a few times
const RESTORE_TICKS: u8 = 12;

#[derive(Default)]
struct Watch {
    hook: HWINEVENTHOOK,
    live: bool,     // hotkey registered on main
    retries: u8,
    restore: Option<(HWND, String, String, u8)>, // dialog, name to put back, folder typed, ticks left
}

thread_local! {
    static WATCH: RefCell<Watch> = RefCell::new(Watch::default());
}

fn with<R>(f: impl FnOnce(&mut Watch) -> R) -> R {
    WATCH.with(|w| f(&mut w.borrow_mut()))
}

/// A #32770 with a file name control, in another process (Grids' own dialogs and message boxes don't count).
pub(crate) fn looks_like_file_dialog(class: &str, self_pid: u32, pid: u32, ids: &[i32]) -> bool {
    class == "#32770" && pid != 0 && pid != self_pid && ids.iter().any(|&id| id == FILENAME_ID || id == EDT1_ID)
}

/// What gets typed: backslashes, no trailing one except on a drive root.
pub(crate) fn jump_text(path: &str) -> String {
    let s = path.trim().replace('/', "\\");
    let t = s.trim_end_matches('\\');
    if t.len() == 2 && t.ends_with(':') {
        format!("{t}\\")
    } else {
        t.to_string()
    }
}

/// A typed name or filter goes back into the box afterwards; empty or a folder path does not.
pub(crate) fn should_restore(prev: &str) -> bool {
    let t = prev.trim();
    !t.is_empty() && !is_fs(t)
}

/// (pane, folder) for the menu: active pane first, then the rest in order; shell places skipped, each folder once.
pub(crate) fn jump_list(paths: &[String], active: usize) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let order = std::iter::once(active).chain((0..paths.len()).filter(|&k| k != active));
    for k in order.filter(|&k| k < paths.len()) {
        let p = jump_text(&paths[k]);
        if is_fs(&p) && !out.iter().any(|(_, q)| q.eq_ignore_ascii_case(&p)) {
            out.push((k, p));
        }
    }
    out
}

/// "Pane &2   docs<TAB>C:\work\docs" ('&' in names doubled so it isn't a mnemonic)
pub(crate) fn jump_label(k: usize, path: &str) -> String {
    let full: String = path.chars().take(120).collect();
    let more = if path.chars().count() > 120 { "…" } else { "" };
    format!("{} &{}   {}\t{full}{more}", tr("Pane", "패널"), k + 1, short_name(path)).replace('&', "&&").replacen("&&", "&", 1)
}

unsafe fn class_of(h: HWND) -> String {
    let mut b = [0u16; 64];
    let n = GetClassNameW(h, &mut b);
    String::from_utf16_lossy(&b[..n.max(0) as usize])
}

// WM_GETTEXT, not GetWindowTextW: that one gives another process' edit box its caption, not what is typed in it
unsafe fn text_of(h: HWND) -> String {
    let n = SendMessageW(h, WM_GETTEXTLENGTH, None, None).0.max(0) as usize;
    let mut b = vec![0u16; n + 1];
    let got = SendMessageW(h, WM_GETTEXT, Some(WPARAM(b.len())), Some(LPARAM(b.as_mut_ptr() as isize))).0.max(0) as usize;
    String::from_utf16_lossy(&b[..got.min(n)])
}

unsafe extern "system" fn each_child(h: HWND, lp: LPARAM) -> BOOL {
    (*(lp.0 as *mut Vec<HWND>)).push(h);
    TRUE
}

unsafe fn children(h: HWND) -> Vec<HWND> {
    let mut v: Vec<HWND> = Vec::new();
    _ = EnumChildWindows(Some(h), Some(each_child), LPARAM(&mut v as *mut _ as isize));
    v
}

/// The file name Edit (or its combo when no Edit carries the id).
unsafe fn name_box(dlg: HWND) -> Option<HWND> {
    let hits: Vec<HWND> = children(dlg).into_iter().filter(|&c| matches!(GetDlgCtrlID(c), FILENAME_ID | EDT1_ID)).collect();
    hits.iter().copied().find(|&c| class_of(c) == "Edit").or(hits.first().copied())
}

unsafe fn file_dialog(h: HWND) -> Option<HWND> {
    if h.is_invalid() {
        return None;
    }
    let mut pid = 0;
    GetWindowThreadProcessId(h, Some(&mut pid));
    let ids: Vec<i32> = children(h).into_iter().map(|c| GetDlgCtrlID(c)).collect();
    looks_like_file_dialog(&class_of(h), GetCurrentProcessId(), pid, &ids).then_some(h)
}

/// Hotkey on while a foreign file dialog is in front, off otherwise.
unsafe fn sync() {
    let fg = GetForegroundWindow();
    let (main, on) = app(|a| (a.main, a.s.dlgjump));
    let want = on && file_dialog(fg).is_some();
    let live = with(|w| w.live);
    if want && !live {
        let ok = RegisterHotKey(Some(main), DLG_HOTKEY_ID, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, 'G' as u32).is_ok();
        with(|w| w.live = ok);
    } else if !want && live {
        _ = UnregisterHotKey(Some(main), DLG_HOTKEY_ID);
        with(|w| w.live = false);
    }
    // a #32770 of another app that has no file name box yet: maybe it is still being built
    if on && !want && class_of(fg) == "#32770" && with(|w| w.retries > 0) {
        with(|w| w.retries -= 1);
        SetTimer(Some(main), TIMER_DLG_RETRY, 200, None);
    }
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
    with(|w| w.retries = RETRIES);
    sync();
}

/// Startup and the View menu: hook + hotkey follow the setting.
pub(crate) unsafe fn enable(on: bool) {
    let hook = with(|w| w.hook);
    if on && hook.is_invalid() {
        let hook = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
        with(|w| (w.hook, w.retries) = (hook, RETRIES));
    } else if !on && !hook.is_invalid() {
        _ = UnhookWinEvent(hook);
        with(|w| w.hook = HWINEVENTHOOK::default());
    }
    sync();
}

/// Main is going away: the hotkey is registered on it and the hook would outlive it.
pub(crate) unsafe fn shutdown() {
    let w = with(std::mem::take);
    if !w.hook.is_invalid() {
        _ = UnhookWinEvent(w.hook);
    }
    if w.live {
        _ = UnregisterHotKey(Some(app(|a| a.main)), DLG_HOTKEY_ID);
    }
}

pub(crate) unsafe fn on_timer(id: usize) {
    let main = app(|a| a.main);
    _ = KillTimer(Some(main), id);
    if id == TIMER_DLG_RETRY {
        sync();
    } else if id == TIMER_DLG_RESTORE {
        restore_tick(main);
    }
}

pub(crate) unsafe fn on_hotkey() {
    let fg = GetForegroundWindow();
    let list = app(|a| jump_list(&a.s.paths[..a.s.panes()], a.s.active));
    if let (Some(dlg), false) = (file_dialog(fg), list.is_empty()) {
        if let Some(path) = pick(dlg, &list) {
            go(dlg, &path);
        }
    }
    sync();
}

unsafe fn pick(dlg: HWND, list: &[(usize, String)]) -> Option<String> {
    let mut r = RECT::default();
    _ = GetWindowRect(dlg, &mut r);
    let dpi = GetDpiForWindow(dlg).max(96) as i32;
    let (x, y) = (r.left + 16 * dpi / 96, r.top + 48 * dpi / 96);
    let menu = CreatePopupMenu().ok()?;
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Go to a folder open in Grids:", "Grids에 열린 폴더로 이동:")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for (n, (k, p)) in list.iter().enumerate() {
        _ = AppendMenuW(menu, MF_STRING, n + 1, &HSTRING::from(jump_label(*k, p)));
    }
    // A tiny window of ours owns the menu: the hotkey lets it take the foreground (the menu needs that for
    // its keys) without bringing the Grids window over the dialog. The menu's dark/light follows Grids' theme.
    let host = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST, w!("STATIC"), PCWSTR::null(), WS_POPUP, x, y, 1, 1, None, None, None, None).ok();
    let got = host.map(|host| {
        _ = ShowWindow(host, SW_SHOW);
        _ = SetForegroundWindow(host);
        let n = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN, x, y, None, host, None).0;
        _ = DestroyWindow(host);
        n
    });
    _ = DestroyMenu(menu);
    let n = got.unwrap_or(0) as usize;
    (n > 0).then(|| list.get(n - 1).map(|(_, p)| p.clone())).flatten()
}

fn key(vk: u16, unit: u16, up: bool) -> INPUT {
    let mut f = if vk == 0 { KEYEVENTF_UNICODE } else { KEYBD_EVENT_FLAGS(0) };
    if up {
        f |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: unit, dwFlags: f, time: 0, dwExtraInfo: 0 } },
    }
}

/// Folder typed into the file name box + Enter: the dialog moves and stays open; the name typed before comes back.
unsafe fn go(dlg: HWND, path: &str) {
    let Some(edit) = name_box(dlg).filter(|_| IsWindow(Some(dlg)).as_bool()) else { return };
    let prev = text_of(edit);
    let text = jump_text(path);
    let (me, them) = (GetCurrentThreadId(), GetWindowThreadProcessId(dlg, None));
    let joined = AttachThreadInput(me, them, true).as_bool(); // SetFocus across threads needs the joined input
    _ = SetForegroundWindow(dlg);
    _ = SetFocus(Some(edit));
    let ok = GetForegroundWindow() == dlg;
    if ok {
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        let keys: Vec<INPUT> = text.encode_utf16().flat_map(|u| [key(0, u, false), key(0, u, true)]).collect();
        SendInput(&keys, size_of::<INPUT>() as i32);
        std::thread::sleep(Duration::from_millis(60)); // the characters first, then Enter
        SendInput(&[key(VK_RETURN.0, 0, false), key(VK_RETURN.0, 0, true)], size_of::<INPUT>() as i32);
    }
    if joined {
        _ = AttachThreadInput(me, them, false);
    }
    if !ok {
        _ = MessageBeep(MB_ICONWARNING); // nothing typed anywhere else
    } else if should_restore(&prev) {
        with(|w| w.restore = Some((dlg, prev, text, RESTORE_TICKS)));
        SetTimer(Some(app(|a| a.main)), TIMER_DLG_RESTORE, 150, None);
    }
}

// The dialog clears the box (or leaves the folder in it) once it has moved: put the name back, for a while,
// unless the user has typed something else since.
unsafe fn restore_tick(main: HWND) {
    let Some((dlg, prev, folder, left)) = with(|w| w.restore.take()) else { return };
    let Some(edit) = name_box(dlg).filter(|_| IsWindow(Some(dlg)).as_bool()) else { return };
    let now = text_of(edit);
    let ours = now.trim().is_empty() || jump_text(&now).eq_ignore_ascii_case(&folder);
    if now != prev && ours {
        let w = HSTRING::from(&prev); // WM_SETTEXT: SetWindowTextW can't set another process' control
        SendMessageW(edit, WM_SETTEXT, None, Some(LPARAM(w.as_ptr() as isize)));
    }
    if (now == prev || ours) && left > 1 {
        with(|w| w.restore = Some((dlg, prev, folder, left - 1)));
        SetTimer(Some(main), TIMER_DLG_RESTORE, 150, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_detection() {
        assert!(looks_like_file_dialog("#32770", 4, 9, &[1, FILENAME_ID]));
        assert!(looks_like_file_dialog("#32770", 4, 9, &[EDT1_ID]));
        assert!(!looks_like_file_dialog("#32770", 4, 4, &[FILENAME_ID])); // Grids' own
        assert!(!looks_like_file_dialog("#32770", 4, 9, &[1, 2])); // a message box
        assert!(!looks_like_file_dialog("Notepad", 4, 9, &[FILENAME_ID]));
        assert!(!looks_like_file_dialog("#32770", 4, 0, &[FILENAME_ID]));
        assert_ne!(DLG_HOTKEY_ID, HOTKEY_ID);
    }

    #[test]
    fn typed_text_and_restore() {
        assert_eq!(jump_text(r"C:\work\a\"), r"C:\work\a");
        assert_eq!(jump_text("C:/work/b/"), r"C:\work\b");
        assert_eq!(jump_text(r"C:\"), r"C:\");
        assert_eq!(jump_text("D:"), r"D:\");
        assert_eq!(jump_text(r"\\srv\share\"), r"\\srv\share");
        assert!(should_restore("notes.txt") && should_restore("*.txt"));
        assert!(!should_restore("  ") && !should_restore(r"C:\work") && !should_restore(r"\\srv\share"));
    }

    #[test]
    fn ini_setting() {
        assert!(Session::parse("side=300\n").dlgjump); // older ini: on
        assert!(!Session::parse("dialog_jump=0\n").dlgjump);
        for on in [false, true] {
            assert_eq!(Session::parse(&Session { dlgjump: on, ..Session::default() }.to_ini()).dlgjump, on);
        }
        // app-wide: no unsaved-layout dot, a loaded session doesn't change it
        let base = Session::default();
        assert!(!layout_differs(&Session { dlgjump: false, ..base.clone() }, &base));
        let mut cur = base.clone();
        cur.take_layout(&Session::parse("dialog_jump=0\n"));
        assert!(cur.dlgjump);
    }

    #[test]
    fn menu_list_and_labels() {
        let p = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let paths = p(&[r"C:\a", r"D:\b\", THIS_PC, r"c:\A", "", r"\\srv\x"]);
        let want = vec![(1, r"D:\b".to_string()), (0, r"C:\a".into()), (5, r"\\srv\x".into())];
        assert_eq!(jump_list(&paths, 1), want);
        assert_eq!(jump_list(&paths, 9)[0], (0, r"C:\a".to_string())); // stale active: plain order
        assert!(jump_list(&p(&[THIS_PC, ""]), 0).is_empty());
        assert_eq!(jump_label(1, r"C:\A & B"), "Pane &2   A && B\tC:\\A && B");
    }
}
