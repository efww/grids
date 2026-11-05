// Jump from another app's Open or Save dialog to a folder Maguro already has open.
// The chord (Ctrl+Alt+G by default) is registered only while a foreign #32770 file dialog
// is the foreground window, so it never takes those keys in other places. Win+Shift+E
// stays the separate "bring Maguro forward" hotkey.
//
// Navigation types the folder into the filename edit and presses Enter. SetWindowText and
// WM_COMMAND do not update the dialog's own navigation state. A directory path moves the
// dialog and leaves it open. A timer then puts the previous filename back when it was a
// name or a filter, not a folder path.

use std::collections::HashSet;

use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Controls::EM_SETSEL;

use crate::imports::*;

/// Common-item dialog filename combo (cmb13). The nested Edit carries the same id.
const FILENAME_ID: i32 = 0x47C;
/// Older #32770 dialogs use edt1.
const EDT1_ID: i32 = 0x480;
/// Save dialog filename Edit. It sits in an id-less ComboBox inside a FloatNotifySink.
/// The address-bar toolbar uses this id too, so the id alone is not enough.
const SAVE_NAME_ID: i32 = 0x3E9;
/// Not `HOTKEY_ID` (1). That one is Win+Shift+E.
pub(crate) const DIALOG_HOTKEY_ID: i32 = 2;

const WATCH_MS: u32 = 400;
const RETRY_MS: u32 = 200;
const RESTORE_MS: u32 = 200;
const RESTORE_TICKS: u8 = 12;
/// A dialog just shown may not have its filename box yet. Look again this many times.
pub(crate) const DLG_RETRIES: u8 = 5;

struct Best {
    rank: u8,
    hwnd: HWND,
}

struct Pending {
    dialog: HWND,
    previous: String,
    folder: String,
    left: u8,
}

struct Watch {
    hook: HWINEVENTHOOK,
    owner: HWND,
    timer_hwnd: HWND,
    live: bool,
    vk: u32,
    mods: u32,
    pending: Option<Pending>,
    /// Window the last foreground event named. `GetForegroundWindow` can lag behind it.
    seen: HWND,
    /// Looks left for a `#32770` whose filename box is not built yet.
    retries: u8,
}

impl Watch {
    fn new() -> Self {
        Watch {
            hook: HWINEVENTHOOK::default(),
            owner: HWND::default(),
            timer_hwnd: HWND::default(),
            live: false,
            vk: 0,
            mods: 0,
            pending: None,
            seen: HWND::default(),
            retries: 0,
        }
    }
}

thread_local! {
    static WATCH: std::cell::RefCell<Watch> = std::cell::RefCell::new(Watch::new());
}

fn with<R>(f: impl FnOnce(&mut Watch) -> R) -> R {
    WATCH.with(|w| f(&mut w.borrow_mut()))
}

/// Register the dialog chord only when the setting is on, the foreground window is a file
/// dialog of another process, and the chord has a modifier. A bare key is refused.
pub(crate) fn should_register_hotkey(setting_on: bool, file_dialog: bool, bare_chord: bool) -> bool {
    setting_on && file_dialog && !bare_chord
}

/// `#32770` with a filename box, belonging to another process. A plain message box has no name box.
pub(crate) fn looks_like_file_dialog(class: &str, self_pid: u32, window_pid: u32, has_name_box: bool) -> bool {
    class == "#32770" && window_pid != 0 && window_pid != self_pid && has_name_box
}

/// How good a filename box this control is (lower is better), from its class, id, and the
/// parent and grandparent class names. Open's Edit is best. Save's Edit counts only in its
/// ComboBox / FloatNotifySink place, so the address-bar toolbar's id 1001 does not.
pub(crate) fn name_box_rank(class: &str, id: i32, parent: &str, grand: &str) -> Option<u8> {
    match (class, id) {
        ("Edit", FILENAME_ID | EDT1_ID) => Some(0),
        ("Edit", SAVE_NAME_ID) if parent == "ComboBox" && grand == "FloatNotifySink" => Some(1),
        (_, FILENAME_ID | EDT1_ID) => Some(2),
        _ => None,
    }
}

/// A foreground event always remembers that hwnd and gives the filename box five looks.
pub(crate) fn note_foreground_retries() -> u8 {
    DLG_RETRIES
}

/// Which window the hotkey should type into. The foreground window wins. `seen` covers the
/// gap where the event named the dialog before `GetForegroundWindow` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogSource {
    Foreground,
    Seen,
    None,
}

pub(crate) fn dialog_source(fg_dialog: bool, seen_dialog: bool) -> DialogSource {
    if fg_dialog {
        DialogSource::Foreground
    } else if seen_dialog {
        DialogSource::Seen
    } else {
        DialogSource::None
    }
}

/// Facts `reconcile` can decide from, without a window handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DialogSnap {
    pub setting_on: bool,
    pub chord_ok: bool,
    /// `GetForegroundWindow` is a foreign file dialog (name box present).
    pub fg_dialog: bool,
    /// That window's class is `#32770`.
    pub fg_32770: bool,
    pub seen_set: bool,
    /// `IsWindow(seen)`.
    pub seen_alive: bool,
    pub seen_32770: bool,
    /// Name-box recheck: `seen` currently has a filename box and is a foreign dialog.
    pub seen_name: bool,
    pub retries: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DialogPlan {
    /// Register or keep the dialog hotkey.
    pub want_hotkey: bool,
    /// `seen` is closed, not a dialog, or the retries ran out with no name box.
    pub clear_seen: bool,
    pub retries: u8,
    /// Store `retries` (a foreground event or a retry tick). The 400 ms watch does not.
    pub write_retries: bool,
    /// Arm `TIMER_DLG_RETRY` at 200 ms.
    pub arm_retry: bool,
}

/// `count_retry` is false for the 400 ms watch. That pass still keeps a hotkey when `seen`
/// is a live file dialog whose foreground report has not caught up, and it still clears a
/// dead hwnd, but it does not spend one of the five looks.
pub(crate) fn plan_watch(snap: DialogSnap, count_retry: bool) -> DialogPlan {
    let seen_dialog = snap.seen_set && snap.seen_alive && snap.seen_name;
    let seen_building = snap.seen_set && snap.seen_alive && snap.seen_32770 && !snap.seen_name;
    let fg_building = snap.fg_32770 && !snap.fg_dialog;
    let want = should_register_hotkey(snap.setting_on, snap.fg_dialog || seen_dialog, !snap.chord_ok);
    let clear_seen = if !snap.seen_set {
        false
    } else if !snap.seen_alive {
        true
    } else if seen_dialog {
        false
    } else if seen_building && snap.retries > 0 {
        false
    } else {
        true
    };
    let pending = !want && (fg_building || (seen_building && !clear_seen));
    let retry = snap.setting_on && snap.chord_ok && pending && snap.retries > 0;
    let retries = if retry && count_retry { snap.retries - 1 } else { snap.retries };
    DialogPlan { want_hotkey: want, clear_seen, retries, write_retries: count_retry, arm_retry: count_retry && retry }
}

/// What gets typed. A drive root keeps its slash. Every other folder drops a trailing slash.
pub(crate) fn jump_text(path: &str) -> String {
    let s = path.trim().replace('/', "\\");
    let bytes = s.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\' {
        let rest = s[3..].trim_start_matches('\\').trim_end_matches('\\');
        if rest.is_empty() {
            return s[..3].to_string();
        }
        return format!("{}{}", &s[..3], rest);
    }
    s.trim_end_matches('\\').to_string()
}

pub(crate) fn dedupe_key(path: &str) -> String {
    jump_text(path).to_ascii_lowercase()
}

/// A typed name or a filter goes back into the box. An empty box, or a folder path, stays.
pub(crate) fn should_restore(previous: &str) -> bool {
    let t = previous.trim();
    !t.is_empty() && !crate::is_fs(t)
}

pub(crate) fn menu_label(path: &str) -> String {
    let leaf = crate::folder_display_name(path);
    let full: String = path.chars().take(200).collect();
    let full = if path.chars().count() > 200 { format!("{full}…") } else { full };
    let shown = if leaf.is_empty() { full } else { format!("{leaf}\t{full}") };
    shown.replace('&', "&&")
}

fn push_path(out: &mut Vec<String>, seen: &mut HashSet<String>, path: &str) {
    if !crate::is_fs(path) {
        return;
    }
    let shown = jump_text(path);
    if !crate::is_fs(&shown) || !seen.insert(dedupe_key(&shown)) {
        return;
    }
    out.push(shown);
}

fn tab_panes(tab: &crate::Tab) -> usize {
    let n = tab.heights.iter().sum::<usize>();
    if n == 0 { 1 } else { n.min(crate::MAXP) }
}

fn tab_pane_paths(tab: &crate::Tab, i: usize) -> Vec<String> {
    if i >= crate::MAXP {
        return Vec::new();
    }
    let mut folders = tab.pane_folders.get(i).cloned().unwrap_or_default();
    let cur = tab.pane_cur.get(i).copied().unwrap_or(0);
    let path = tab.paths.get(i).cloned().unwrap_or_default();
    if folders.len() > 1 && cur < folders.len() {
        folders[cur] = path;
        folders
    } else {
        vec![path]
    }
}

fn push_pane(out: &mut Vec<String>, seen: &mut HashSet<String>, tab: &crate::Tab, i: usize) {
    let list = tab_pane_paths(tab, i);
    if list.is_empty() {
        return;
    }
    let cur = tab.pane_cur.get(i).copied().unwrap_or(0);
    let cur = if list.len() > 1 { cur.min(list.len() - 1) } else { 0 };
    push_path(out, seen, &list[cur]);
    for (k, path) in list.iter().enumerate() {
        if k != cur {
            push_path(out, seen, path);
        }
    }
}

fn push_tab(out: &mut Vec<String>, seen: &mut HashSet<String>, tab: &crate::Tab) {
    let n = tab_panes(tab);
    let active = tab.active.min(n.saturating_sub(1));
    push_pane(out, seen, tab, active);
    for i in 0..n {
        if i != active {
            push_pane(out, seen, tab, i);
        }
    }
}

fn push_session(out: &mut Vec<String>, seen: &mut HashSet<String>, session: &crate::Session) {
    let tabs = session.view_tabs();
    if tabs.is_empty() {
        return;
    }
    let active = session.tab.min(tabs.len() - 1);
    push_tab(out, seen, &tabs[active]);
    for (i, tab) in tabs.iter().enumerate() {
        if i != active {
            push_tab(out, seen, tab);
        }
    }
}

/// Active window first, then its active tab's active pane, that pane's other folders,
/// the other panes, the other tabs, then the other windows. Duplicates collapse.
pub(crate) fn ordered_folders(sessions: &[crate::Session], front: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if sessions.is_empty() {
        return out;
    }
    let front = if front < sessions.len() { front } else { 0 };
    push_session(&mut out, &mut seen, &sessions[front]);
    for (i, session) in sessions.iter().enumerate() {
        if i != front {
            push_session(&mut out, &mut seen, session);
        }
    }
    out
}

fn current_folders() -> Vec<String> {
    crate::app(|a| {
        let sessions: Vec<crate::Session> = a.desks.iter().map(|d| d.s.clone()).collect();
        ordered_folders(&sessions, a.last_front)
    })
}

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn window_text(hwnd: HWND) -> String {
    let n = unsafe { GetWindowTextLengthW(hwnd) };
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..got.max(0) as usize])
}

unsafe extern "system" fn enum_filename(hwnd: HWND, lp: LPARAM) -> BOOL {
    let best = &mut *(lp.0 as *mut Option<Best>);
    let parent = GetParent(hwnd).unwrap_or_default();
    let grand = if parent.0.is_null() { HWND::default() } else { GetParent(parent).unwrap_or_default() };
    let parent_class = if parent.0.is_null() { String::new() } else { class_of(parent) };
    let grand_class = if grand.0.is_null() { String::new() } else { class_of(grand) };
    if let Some(rank) = name_box_rank(&class_of(hwnd), GetDlgCtrlID(hwnd), &parent_class, &grand_class) {
        // Lowest rank wins: Open's Edit, then Save's Edit, then a combo that carries the id.
        if best.as_ref().map(|b| rank < b.rank).unwrap_or(true) {
            *best = Some(Best { rank, hwnd });
        }
    }
    TRUE
}

/// The filename box: Open's Edit, Save's Edit, or an older dialog's combo when no Edit carries the id.
unsafe fn filename_edit(dialog: HWND) -> Option<HWND> {
    let mut best: Option<Best> = None;
    let _ = EnumChildWindows(Some(dialog), Some(enum_filename), LPARAM(&mut best as *mut Option<Best> as isize));
    best.map(|b| b.hwnd)
}

#[derive(Clone, Copy)]
struct WinLook {
    alive: bool,
    class_32770: bool,
    foreign_name: bool,
}

unsafe fn look_at(hwnd: HWND) -> WinLook {
    if hwnd.0.is_null() || !IsWindow(Some(hwnd)).as_bool() {
        return WinLook { alive: false, class_32770: false, foreign_name: false };
    }
    let mut pid = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    let class = class_of(hwnd);
    let class_32770 = class == "#32770";
    let foreign_name = looks_like_file_dialog(&class, GetCurrentProcessId(), pid, filename_edit(hwnd).is_some());
    WinLook { alive: true, class_32770, foreign_name }
}

fn chord_usable(chord: &crate::keys::Chord) -> bool {
    !chord.digits && (chord.ctrl || chord.alt || chord.shift || chord.win)
}

fn chord_mods(chord: &crate::keys::Chord) -> (u32, u32) {
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
    (mods.0, chord.vk as u32)
}

fn pick_owner() -> HWND {
    let (hot, last, mains) = crate::app(|a| {
        let last = a.desks.get(a.last_front).map(|d| d.main).unwrap_or_default();
        let mains: Vec<HWND> = a.desks.iter().map(|d| d.main).filter(|h| !h.0.is_null()).collect();
        (a.hot_hwnd, last, mains)
    });
    unsafe {
        if !hot.0.is_null() && IsWindow(Some(hot)).as_bool() {
            return hot;
        }
        if !last.0.is_null() && IsWindow(Some(last)).as_bool() {
            return last;
        }
        mains.into_iter().find(|h| IsWindow(Some(*h)).as_bool()).unwrap_or_default()
    }
}

unsafe fn drop_hotkey() {
    let (live, owner) = with(|w| (w.live, w.owner));
    if live && !owner.0.is_null() {
        let _ = UnregisterHotKey(Some(owner), DIALOG_HOTKEY_ID);
    }
    with(|w| {
        w.live = false;
        w.owner = HWND::default();
        w.vk = 0;
        w.mods = 0;
    });
}

unsafe fn apply_dialog_hotkey(want: bool) {
    if !want {
        if with(|w| w.live) {
            drop_hotkey();
        }
        return;
    }
    let Some(chord) = crate::keys::dialog_chord().filter(chord_usable) else {
        if with(|w| w.live) {
            drop_hotkey();
        }
        return;
    };
    let (mods, vk) = chord_mods(&chord);
    let owner = pick_owner();
    let (live, cur_owner, cur_vk, cur_mods) = with(|w| (w.live, w.owner, w.vk, w.mods));
    if live && cur_owner == owner && cur_vk == vk && cur_mods == mods && IsWindow(Some(owner)).as_bool() {
        return;
    }
    if live {
        drop_hotkey();
    }
    if owner.0.is_null() {
        return;
    }
    let ok = RegisterHotKey(Some(owner), DIALOG_HOTKEY_ID, HOT_KEY_MODIFIERS(mods), vk).is_ok();
    with(|w| {
        w.live = ok;
        w.owner = if ok { owner } else { HWND::default() };
        w.vk = if ok { vk } else { 0 };
        w.mods = if ok { mods } else { 0 };
    });
}

/// Foreground window and the last event hwnd, together. `count_retry` spends one of the
/// five looks and arms the 200 ms timer. The 400 ms watch passes false: a hotkey registered
/// for `seen` stays while `GetForegroundWindow` is still the previous window.
unsafe fn reconcile(count_retry: bool) {
    let fg = GetForegroundWindow();
    let (seen, retries) = with(|w| (w.seen, w.retries));
    let fg_look = look_at(fg);
    let seen_set = !seen.0.is_null();
    let seen_look = if !seen_set {
        WinLook { alive: false, class_32770: false, foreign_name: false }
    } else if seen == fg {
        fg_look
    } else {
        look_at(seen)
    };
    let setting = crate::app(|a| a.s.dialog_jump);
    let chord_ok = crate::keys::dialog_chord().filter(chord_usable).is_some();
    let plan = plan_watch(
        DialogSnap {
            setting_on: setting,
            chord_ok,
            fg_dialog: fg_look.foreign_name,
            fg_32770: fg_look.class_32770,
            seen_set,
            seen_alive: seen_look.alive,
            seen_32770: seen_look.class_32770,
            seen_name: seen_look.foreign_name,
            retries,
        },
        count_retry,
    );
    with(|w| {
        if plan.clear_seen && w.seen == seen {
            w.seen = HWND::default();
        }
        if plan.write_retries {
            w.retries = plan.retries;
        }
    });
    apply_dialog_hotkey(plan.want_hotkey);
    if plan.arm_retry {
        let host = with(|w| w.timer_hwnd);
        if !host.0.is_null() {
            let _ = SetTimer(Some(host), crate::TIMER_DLG_RETRY, RETRY_MS, None);
        }
    }
}

/// Foreground changed, the shortcut was edited, or a timer fired.
pub(crate) unsafe fn sync() {
    reconcile(true);
}

unsafe extern "system" fn on_foreground(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    let seen = if hwnd.0.is_null() { GetForegroundWindow() } else { hwnd };
    with(|w| {
        w.seen = seen;
        w.retries = note_foreground_retries();
    });
    reconcile(true);
}

fn instance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|h| h.into()).unwrap_or_default() }
}

fn ensure_class() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(jump_proc),
            hInstance: instance(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("MaguroDlgJump"),
            ..Default::default()
        });
    });
}

unsafe extern "system" fn jump_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(h, msg, wp, lp)
}

unsafe fn ensure_hook() {
    let invalid = with(|w| w.hook.is_invalid());
    if !invalid {
        return;
    }
    let hook = SetWinEventHook(
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_FOREGROUND,
        None,
        Some(on_foreground),
        0,
        0,
        WINEVENT_OUTOFCONTEXT,
    );
    with(|w| w.hook = hook);
}

/// One watcher for the process. A new window takes the timer only when the previous owner is gone.
pub(crate) unsafe fn install(main: HWND) {
    if main.0.is_null() {
        return;
    }
    ensure_class();
    ensure_hook();
    let timer = with(|w| w.timer_hwnd);
    if timer.0.is_null() || !IsWindow(Some(timer)).as_bool() {
        let _ = SetTimer(Some(main), crate::TIMER_DLG_WATCH, WATCH_MS, None);
        with(|w| w.timer_hwnd = main);
    }
    sync();
}

/// The window is going away. The hotkey is registered on one window; destroying it does not unregister.
pub(crate) unsafe fn release(h: HWND) {
    let _ = KillTimer(Some(h), crate::TIMER_DLG_WATCH);
    let _ = KillTimer(Some(h), crate::TIMER_DLG_JUMP);
    let _ = KillTimer(Some(h), crate::TIMER_DLG_RETRY);
    let (live, owner, timer) = with(|w| (w.live, w.owner, w.timer_hwnd));
    if live && owner == h {
        let _ = UnregisterHotKey(Some(h), DIALOG_HOTKEY_ID);
        with(|w| {
            w.live = false;
            w.owner = HWND::default();
            w.vk = 0;
            w.mods = 0;
        });
    }
    if timer == h {
        with(|w| w.timer_hwnd = HWND::default());
    }
}

pub(crate) unsafe fn shutdown() {
    drop_hotkey();
    let (hook, timer) = with(|w| (w.hook, w.timer_hwnd));
    if !timer.0.is_null() {
        let _ = KillTimer(Some(timer), crate::TIMER_DLG_WATCH);
        let _ = KillTimer(Some(timer), crate::TIMER_DLG_JUMP);
        let _ = KillTimer(Some(timer), crate::TIMER_DLG_RETRY);
    }
    if !hook.is_invalid() {
        let _ = UnhookWinEvent(hook);
    }
    with(|w| *w = Watch::new());
}

pub(crate) unsafe fn on_timer(id: usize) {
    if id == crate::TIMER_DLG_RETRY {
        let host = with(|w| w.timer_hwnd);
        if !host.0.is_null() {
            let _ = KillTimer(Some(host), crate::TIMER_DLG_RETRY);
        }
        reconcile(true);
    } else if id == crate::TIMER_DLG_WATCH {
        // Foreground can still name the previous window. Do not spend a retry, and do not
        // drop a hotkey `seen` just registered.
        reconcile(false);
    } else if id == crate::TIMER_DLG_JUMP {
        restore_tick();
    }
}

fn menu_origin(dialog: HWND) -> POINT {
    let mut rc = RECT::default();
    if unsafe { GetWindowRect(dialog, &mut rc) }.is_err() {
        return POINT { x: 40, y: 40 };
    }
    let mut x = rc.left + 16;
    let mut y = rc.top + 40;
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        if GetMonitorInfoW(MonitorFromWindow(dialog, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            let work = info.rcWork;
            if x > work.right - 80 {
                x = work.right - 80;
            }
            if y > work.bottom - 80 {
                y = work.bottom - 80;
            }
            if x < work.left {
                x = work.left;
            }
            if y < work.top {
                y = work.top;
            }
        }
    }
    POINT { x, y }
}

unsafe fn popup(dialog: HWND, folders: &[String]) -> Option<String> {
    if folders.is_empty() || dialog.0.is_null() {
        return None;
    }
    ensure_class();
    let pt = menu_origin(dialog);
    let Ok(menu) = CreatePopupMenu() else { return None };
    for (i, path) in folders.iter().enumerate() {
        let _ = AppendMenuW(menu, MF_STRING, i as usize + 1, &HSTRING::from(menu_label(path)));
    }
    // The host must be allowed to activate. A no-activate owner shows the menu
    // but does not take keyboard input, so arrows and Enter never choose a folder.
    let created = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
        w!("MaguroDlgJump"),
        w!(""),
        WS_POPUP,
        pt.x,
        pt.y,
        1,
        1,
        None,
        None,
        Some(instance()),
        None,
    )
    .unwrap_or_default();
    let (host, owned) = if created.0.is_null() {
        (pick_owner(), false)
    } else {
        let _ = ShowWindow(created, SW_SHOW);
        (created, true)
    };
    if host.0.is_null() {
        let _ = DestroyMenu(menu);
        return None;
    }
    // WM_HOTKEY gave this thread the right to take the foreground for the menu.
    let _ = SetForegroundWindow(host);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, host, None).0;
    let _ = PostMessageW(Some(host), WM_NULL, WPARAM(0), LPARAM(0));
    let _ = DestroyMenu(menu);
    if owned {
        let _ = DestroyWindow(created);
    }
    (pick > 0).then(|| folders.get(pick as usize - 1).cloned()).flatten()
}

fn key_unicode(unit: u16, up: bool) -> INPUT {
    let mut flags = KEYEVENTF_UNICODE;
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    let mut input = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0::default() };
    input.Anonymous.ki = KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    input
}

fn key_vk(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let mut input = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0::default() };
    input.Anonymous.ki = KEYBDINPUT {
        wVk: vk,
        wScan: 0,
        dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
        time: 0,
        dwExtraInfo: 0,
    };
    input
}

unsafe fn send_inputs(inputs: &[INPUT]) -> bool {
    let sent = SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    sent == inputs.len() as u32
}

unsafe fn type_text(path: &str) -> bool {
    let mut inputs = Vec::with_capacity(path.encode_utf16().count() * 2);
    for unit in path.encode_utf16() {
        inputs.push(key_unicode(unit, false));
        inputs.push(key_unicode(unit, true));
    }
    send_inputs(&inputs)
}

unsafe fn tap_enter() -> bool {
    send_inputs(&[key_vk(VK_RETURN, false), key_vk(VK_RETURN, true)])
}

unsafe fn navigate(dialog: HWND, path: &str) {
    if dialog.0.is_null() || !IsWindow(Some(dialog)).as_bool() {
        return;
    }
    let Some(edit) = filename_edit(dialog) else { return };
    let previous = window_text(edit);
    let typed = jump_text(path);
    if typed.is_empty() {
        return;
    }
    let thread = GetWindowThreadProcessId(dialog, None);
    let mine = GetCurrentThreadId();
    let attached = thread != 0 && thread != mine && AttachThreadInput(mine, thread, true).as_bool();
    // The menu host is foreground. A blank key lets this thread move the foreground
    // back to the dialog, so both the path and Enter land in its filename edit.
    let nudge = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0::default() };
    let _ = SendInput(&[nudge], std::mem::size_of::<INPUT>() as i32);
    let _ = SetForegroundWindow(dialog);
    let _ = SetFocus(Some(edit));
    let _ = SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    let typed_ok = type_text(&typed);
    if typed_ok {
        // Let the dialog take the characters before Enter. Same-batch Enter can
        // hit the menu host instead of the edit.
        std::thread::sleep(std::time::Duration::from_millis(60));
        let _ = SetForegroundWindow(dialog);
        let _ = SetFocus(Some(edit));
        let _ = tap_enter();
    }
    if attached {
        let _ = AttachThreadInput(mine, thread, false);
    }
    if typed_ok && should_restore(&previous) {
        schedule_restore(dialog, previous, typed);
    }
}

fn schedule_restore(dialog: HWND, previous: String, folder: String) {
    let host = with(|w| w.timer_hwnd);
    if host.0.is_null() {
        return;
    }
    with(|w| {
        w.pending = Some(Pending { dialog, previous, folder, left: RESTORE_TICKS });
    });
    unsafe {
        let _ = SetTimer(Some(host), crate::TIMER_DLG_JUMP, RESTORE_MS, None);
    }
}

fn same_dir(now: &str, folder: &str) -> bool {
    let now = dedupe_key(now);
    let folder = dedupe_key(folder);
    !now.is_empty() && now == folder
}

unsafe fn restore_tick() {
    let Some(mut pending) = with(|w| w.pending.clone()) else {
        let host = with(|w| w.timer_hwnd);
        if !host.0.is_null() {
            let _ = KillTimer(Some(host), crate::TIMER_DLG_JUMP);
        }
        return;
    };
    let mut stop = !IsWindow(Some(pending.dialog)).as_bool();
    if !stop {
        if let Some(edit) = filename_edit(pending.dialog) {
            let now = window_text(edit);
            if now.trim() != pending.previous.trim() {
                if now.trim().is_empty() || same_dir(&now, &pending.folder) {
                    let _ = SetWindowTextW(edit, &HSTRING::from(&pending.previous));
                } else {
                    stop = true;
                }
            }
        }
        if !stop {
            pending.left = pending.left.saturating_sub(1);
            stop = pending.left == 0;
        }
    }
    if stop {
        with(|w| w.pending = None);
        let host = with(|w| w.timer_hwnd);
        if !host.0.is_null() {
            let _ = KillTimer(Some(host), crate::TIMER_DLG_JUMP);
        }
    } else {
        with(|w| w.pending = Some(pending));
    }
}

/// Hotkey, or the same chord pressed while Maguro itself is in front. No folders: no menu.
/// The event hwnd counts when `GetForegroundWindow` has not caught up to it.
pub(crate) unsafe fn on_hotkey() {
    if !crate::app(|a| a.s.dialog_jump) {
        return;
    }
    let fg = GetForegroundWindow();
    let seen = with(|w| w.seen);
    let fg_look = look_at(fg);
    let seen_look = if seen.0.is_null() {
        WinLook { alive: false, class_32770: false, foreign_name: false }
    } else {
        look_at(seen)
    };
    let dlg = match dialog_source(fg_look.foreign_name, seen_look.alive && seen_look.foreign_name) {
        DialogSource::Foreground => Some(fg),
        DialogSource::Seen => Some(seen),
        DialogSource::None => None,
    };
    let Some(dlg) = dlg else {
        // Closed, or the name box is gone: do not leave the hotkey on that hwnd.
        reconcile(true);
        return;
    };
    let folders = current_folders();
    if folders.is_empty() {
        return;
    }
    if let Some(path) = popup(dlg, &folders) {
        navigate(dlg, &path);
    }
}

impl Clone for Pending {
    fn clone(&self) -> Self {
        Pending { dialog: self.dialog, previous: self.previous.clone(), folder: self.folder.clone(), left: self.left }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_follows_the_dialog_and_never_a_bare_key() {
        assert_ne!(DIALOG_HOTKEY_ID, crate::HOTKEY_ID);
        assert!(!should_register_hotkey(false, true, false));
        assert!(!should_register_hotkey(true, false, false));
        assert!(!should_register_hotkey(true, true, true));
        assert!(!looks_like_file_dialog("#32770", 4, 4, true));
        assert!(!looks_like_file_dialog("#32770", 4, 9, false));
        assert!(!looks_like_file_dialog("Notepad", 4, 9, true));
        assert!(!looks_like_file_dialog("#32770", 4, 0, true));
        assert!(looks_like_file_dialog("#32770", 4, 9, true));
        let dialog = looks_like_file_dialog("#32770", 4, 9, true);
        assert!(should_register_hotkey(true, dialog, false));
    }

    #[test]
    fn name_box_rank_picks_open_then_save_and_skips_the_address_bar() {
        let open = name_box_rank("Edit", FILENAME_ID, "ComboBox", "ComboBoxEx32");
        let older = name_box_rank("Edit", EDT1_ID, "#32770", "");
        let save = name_box_rank("Edit", SAVE_NAME_ID, "ComboBox", "FloatNotifySink");
        let combo = name_box_rank("ComboBox", FILENAME_ID, "ComboBoxEx32", "#32770");
        // id 1001 on the address-bar toolbar, and an Edit with that id in the wrong place
        let toolbar = name_box_rank("ToolbarWindow32", SAVE_NAME_ID, "Breadcrumb Parent", "msctls_progress32");
        let loose = name_box_rank("Edit", SAVE_NAME_ID, "#32770", "");
        let address = name_box_rank("Edit", 0xA205, "ComboBox", "ComboBoxEx32");
        assert_eq!(open, Some(0));
        assert_eq!(older, Some(0));
        assert_eq!(save, Some(1));
        assert_eq!(combo, Some(2));
        assert_eq!(toolbar, None);
        assert_eq!(loose, None);
        assert_eq!(address, None);
        // The lookup keeps the lowest rank, so the toolbar's 1001 never beats Save or Open.
        let ranks = [toolbar, address, loose, save, combo, open];
        assert_eq!(ranks.into_iter().flatten().min(), Some(0));
        assert_eq!([toolbar, loose, save].into_iter().flatten().min(), Some(1));
        assert_eq!([toolbar, address, loose].into_iter().flatten().min(), None);
    }

    #[test]
    fn seen_and_retries_follow_the_foreground_event() {
        assert_eq!(note_foreground_retries(), DLG_RETRIES);
        assert_eq!(dialog_source(true, true), DialogSource::Foreground);
        assert_eq!(dialog_source(false, true), DialogSource::Seen);
        assert_eq!(dialog_source(false, false), DialogSource::None);

        let building = |retries| DialogSnap {
            setting_on: true,
            chord_ok: true,
            fg_dialog: false,
            fg_32770: true,
            seen_set: true,
            seen_alive: true,
            seen_32770: true,
            seen_name: false,
            retries,
        };
        let mut retries = note_foreground_retries();
        for i in 0..DLG_RETRIES {
            let plan = plan_watch(building(retries), true);
            assert!(plan.arm_retry, "look {i}");
            assert!(plan.write_retries);
            assert!(!plan.want_hotkey);
            assert!(!plan.clear_seen);
            assert_eq!(plan.retries, retries - 1);
            retries = plan.retries;
        }
        assert_eq!(retries, 0);
        let done = plan_watch(building(0), true);
        assert!(!done.arm_retry);
        assert!(done.clear_seen);
        assert!(!done.want_hotkey);

        // The event hwnd is already a file dialog; GetForegroundWindow still says the old window.
        // The 400 ms watch must keep the hotkey and must not spend a retry.
        let lagged = DialogSnap {
            setting_on: true,
            chord_ok: true,
            fg_dialog: false,
            fg_32770: false,
            seen_set: true,
            seen_alive: true,
            seen_32770: true,
            seen_name: true,
            retries: DLG_RETRIES,
        };
        let kept = plan_watch(lagged, false);
        assert!(kept.want_hotkey);
        assert!(!kept.clear_seen);
        assert!(!kept.arm_retry);
        assert!(!kept.write_retries);
        assert_eq!(kept.retries, DLG_RETRIES);
        let registered = plan_watch(lagged, true);
        assert!(registered.want_hotkey && !registered.arm_retry);
        assert_eq!(registered.retries, DLG_RETRIES);

        // Closed window: IsWindow failed. The hotkey must not stay.
        let closed = plan_watch(
            DialogSnap { seen_alive: false, seen_32770: false, seen_name: false, fg_dialog: false, fg_32770: false, ..lagged },
            false,
        );
        assert!(closed.clear_seen);
        assert!(!closed.want_hotkey);
        assert!(!closed.arm_retry);

        // Name-box recheck failed and it is not a dialog we are still waiting on.
        let notepad = plan_watch(
            DialogSnap { seen_alive: true, seen_32770: false, seen_name: false, fg_dialog: false, fg_32770: false, ..lagged },
            true,
        );
        assert!(notepad.clear_seen);
        assert!(!notepad.want_hotkey);

        // Foreground is the dialog even if `seen` is already dead.
        let fg = plan_watch(DialogSnap { fg_dialog: true, fg_32770: true, seen_alive: false, seen_name: false, ..lagged }, true);
        assert!(fg.want_hotkey);
        assert!(fg.clear_seen);
        assert!(!fg.arm_retry);

        // Setting off, or a bare chord: no hotkey and no retry, even on a real dialog.
        let off = plan_watch(DialogSnap { setting_on: false, ..lagged }, true);
        assert!(!off.want_hotkey && !off.arm_retry && !off.clear_seen);
        let bare = plan_watch(DialogSnap { chord_ok: false, ..lagged }, true);
        assert!(!bare.want_hotkey && !bare.arm_retry);
    }

    #[test]
    fn jump_text_keeps_a_drive_root_and_drops_other_slashes() {
        assert_eq!(jump_text(r"C:\WORK\pgtest\jump-a\"), r"C:\WORK\pgtest\jump-a");
        assert_eq!(jump_text(r"C:\"), r"C:\");
        assert_eq!(jump_text(r"C:\\"), r"C:\");
        assert_eq!(jump_text("C:/WORK/pgtest/jump-b/"), r"C:\WORK\pgtest\jump-b");
        assert_eq!(dedupe_key(r"C:\WORK\pgtest\jump-a\"), dedupe_key(r"c:\work\pgtest\jump-a"));
        assert_eq!(jump_text(r"\\server\share\"), r"\\server\share");
    }

    #[test]
    fn restore_keeps_names_and_filters_not_folders() {
        assert!(!should_restore(""));
        assert!(!should_restore("   "));
        assert!(should_restore("notes.txt"));
        assert!(should_restore("*.txt"));
        assert!(!should_restore(r"C:\WORK\pgtest\jump-a"));
        assert!(!should_restore(r"\\server\share"));
        assert_eq!(menu_label(r"C:\A & B"), "A && B\tC:\\A && B");
    }

    #[test]
    fn folders_start_at_the_active_pane_and_skip_virtual_places() {
        let mut front = crate::Session::default();
        front.set_heights(vec![2]);
        front.active = 1;
        front.paths[0] = r"C:\WORK\pgtest\jump-a\".into();
        front.paths[1] = r"C:\WORK\pgtest\jump-b".into();
        front.ensure_pane_tabs();
        front.pane_folders[1] = vec![r"C:\WORK\pgtest\jump-b".into(), "shell:Desktop".into(), r"C:\WORK\pgtest\jump-d".into()];
        front.pane_cur[1] = 0;
        front.anchor();
        let mut side = crate::Session::default();
        side.set_heights(vec![1]);
        side.paths[0] = r"D:\only".into();
        front.tabs.push(crate::Tab::from_live(&side, "Other".into(), false, Vec::new()));

        let mut other = crate::Session::default();
        other.set_heights(vec![2]);
        other.active = 0;
        other.paths[0] = r"c:\work\pgtest\jump-b".into();
        other.paths[1] = r"\\server\share".into();

        assert_eq!(
            ordered_folders(&[front.clone(), other.clone()], 0),
            vec![
                r"C:\WORK\pgtest\jump-b".to_string(),
                r"C:\WORK\pgtest\jump-d".into(),
                r"C:\WORK\pgtest\jump-a".into(),
                r"D:\only".into(),
                r"\\server\share".into(),
            ]
        );
        assert_eq!(
            ordered_folders(&[front.clone(), other], 1),
            vec![
                r"c:\work\pgtest\jump-b".to_string(),
                r"\\server\share".into(),
                r"C:\WORK\pgtest\jump-d".into(),
                r"C:\WORK\pgtest\jump-a".into(),
                r"D:\only".into(),
            ]
        );
        assert!(ordered_folders(&[], 3).is_empty());
        let mut virtual_only = crate::Session::default();
        virtual_only.paths[0] = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}".into();
        assert!(ordered_folders(&[virtual_only], 0).is_empty());
        let _ = front;
    }
}
