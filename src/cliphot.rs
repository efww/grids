//! Global hotkey that opens a clipboard path in a new tab.
//! Registered the same way as Win+Shift+E (`app.front`): `RegisterHotKey` on one window,
//! only while Maguro is running. The chord is `clip.open` (Win+Shift+V unless rebound).
//! The on/off switch is `Session::clip_hotkey`, not an unbound chord.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES};
use windows::Win32::UI::Shell::IShellFolder;

use crate::clippath::{self, ClipPlan, Probe};
use crate::imports::*;

/// Not `HOTKEY_ID` (1, Win+Shift+E) and not `DIALOG_HOTKEY_ID` (2).
pub(crate) const CLIP_HOTKEY_ID: i32 = 3;

const _: () = assert!(CLIP_HOTKEY_ID != crate::HOTKEY_ID && CLIP_HOTKEY_ID != crate::dialogjump::DIALOG_HOTKEY_ID);

link!("kernel32.dll" "system" fn ExpandEnvironmentStringsW(src: PCWSTR, dst: PWSTR, n: u32) -> u32);

struct Pending {
    pane: usize,
    name: String,
    left: u8,
}

static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

fn expand_env(src: &str) -> String {
    if !src.contains('%') {
        return src.to_string();
    }
    let wide: Vec<u16> = src.encode_utf16().chain([0]).collect();
    let mut buf = vec![0u16; 32768];
    let n = unsafe { ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), PWSTR(buf.as_mut_ptr()), buf.len() as u32) };
    if n == 0 || n as usize > buf.len() {
        return src.to_string();
    }
    let n = (n as usize).saturating_sub(1);
    String::from_utf16_lossy(&buf[..n])
}

fn chord_mods(chord: crate::keys::Chord) -> Option<HOT_KEY_MODIFIERS> {
    if chord.digits || (!chord.ctrl && !chord.alt && !chord.shift && !chord.win) {
        return None;
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
    Some(mods)
}

/// Registers or drops the clipboard hotkey. A bare key is refused: it would steal typing in every app.
pub(crate) unsafe fn set_hotkey(main: HWND, on: bool) -> bool {
    if main.0.is_null() {
        return false;
    }
    if !on {
        let _ = UnregisterHotKey(Some(main), CLIP_HOTKEY_ID);
        return false;
    }
    let Some(chord) = crate::keys::clip_chord() else { return false };
    let Some(mods) = chord_mods(chord) else { return false };
    RegisterHotKey(Some(main), CLIP_HOTKEY_ID, mods, chord.vk as u32).is_ok()
}

/// Startup and a window taking over after the owner closes. A failure leaves the wish on and does not toast.
pub(crate) unsafe fn install(main: HWND) {
    if main.0.is_null() || crate::app(|a| !a.s.clip_hotkey || a.clip_on) {
        return;
    }
    let on = set_hotkey(main, true);
    crate::app(|a| {
        a.clip_on = on;
        a.clip_hwnd = if on { main } else { HWND::default() };
    });
}

/// After a rebind, a reset, or the settings switch. The wish stays on when the chord cannot be registered.
pub(crate) unsafe fn apply(notify: bool) {
    let (want, old, main) = crate::app(|a| (a.s.clip_hotkey, a.clip_hwnd, a.main));
    if !old.0.is_null() {
        let _ = UnregisterHotKey(Some(old), CLIP_HOTKEY_ID);
    }
    let target = if old.0.is_null() { main } else { old };
    let on = want && set_hotkey(target, true);
    crate::app(|a| {
        a.clip_on = on;
        a.clip_hwnd = if on { target } else { HWND::default() };
    });
    if notify && want && !on {
        let chord = crate::keys::index("clip.open").map(crate::keys::chord_text).unwrap_or_default();
        crate::app(|a| {
            a.note = Some((crate::tf(crate::Key::HotkeyBusy, &[("chord", &chord)]), Instant::now() + Duration::from_secs(3)));
            a.status_key.clear();
        });
        crate::update_status();
    }
}

pub(crate) unsafe fn release(h: HWND) {
    if crate::app(|a| a.clip_hwnd == h) {
        let _ = UnregisterHotKey(Some(h), CLIP_HOTKEY_ID);
        crate::app(|a| {
            a.clip_on = false;
            a.clip_hwnd = HWND::default();
        });
    }
}

/// Shown on the General card and the Shortcuts page when the chord is wanted but not registered.
/// A null main window (unit tests, before the first window) does not count: nothing was refused yet.
pub(crate) fn registration_notice() -> Option<String> {
    let show = crate::app(|a| a.s.clip_hotkey && !a.clip_on && !a.main.0.is_null());
    if !show {
        return None;
    }
    let chord = crate::keys::index("clip.open").map(crate::keys::chord_text).unwrap_or_default();
    Some(crate::tf(crate::Key::HotkeyBusy, &[("chord", &chord)]))
}

unsafe fn modal_forward() -> bool {
    let mains = crate::app(|a| a.desks.iter().map(|d| d.main).filter(|h| !h.0.is_null()).collect::<Vec<_>>());
    let Some(main) = mains.into_iter().find(|h| !IsWindowEnabled(*h).as_bool()) else {
        return false;
    };
    if IsIconic(main).as_bool() {
        let _ = ShowWindow(main, SW_RESTORE);
    }
    let p = GetLastActivePopup(main);
    if !p.0.is_null() && p != main {
        let _ = SetForegroundWindow(p);
    }
    true
}

fn front_target() -> HWND {
    crate::app(|a| {
        a.desks
            .get(a.last_front)
            .map(|d| d.main)
            .filter(|h| !h.0.is_null())
            .or_else(|| (!a.clip_hwnd.0.is_null()).then_some(a.clip_hwnd))
            .unwrap_or(a.main)
    })
}

unsafe fn read_clip() -> (Option<String>, Option<String>) {
    let main = crate::app(|a| a.main);
    if OpenClipboard(Some(main)).is_err() {
        return (None, None);
    }
    let drop = clip_drop_first();
    let text = clip_text();
    let _ = CloseClipboard();
    (drop, text)
}

unsafe fn clip_drop_first() -> Option<String> {
    let Ok(h) = GetClipboardData(CF_HDROP.0 as u32) else { return None };
    let drop = HDROP(h.0);
    if DragQueryFileW(drop, u32::MAX, None) == 0 {
        return None;
    }
    let mut buf = vec![0u16; DragQueryFileW(drop, 0, None) as usize + 1];
    let n = DragQueryFileW(drop, 0, Some(&mut buf)) as usize;
    let s = String::from_utf16_lossy(&buf[..n]);
    (!s.is_empty()).then_some(s)
}

unsafe fn clip_text() -> Option<String> {
    let Ok(h) = GetClipboardData(CF_UNICODETEXT.0 as u32) else { return None };
    let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    while n < 1_000_000 && *p.add(n) != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
    let _ = GlobalUnlock(HGLOBAL(h.0));
    Some(s)
}

unsafe fn probe_path(path: &str) -> Probe {
    if crate::unc_path(path) || crate::share_path(path) {
        return Probe::Skip;
    }
    let attr = GetFileAttributesW(&HSTRING::from(path));
    if attr == INVALID_FILE_ATTRIBUTES {
        Probe::Missing
    } else if attr & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
        Probe::Dir
    } else {
        Probe::File
    }
}

fn miss() {
    crate::app(|a| {
        a.note = Some((crate::tr(crate::Key::ClipboardPathMissing).to_string(), Instant::now() + Duration::from_secs(3)));
        a.status_key.clear();
    });
    unsafe { crate::update_status() };
}

/// Open already happened. Select `name` in the new tab once the shell view fills.
pub(crate) unsafe fn select_soon(name: &str) {
    arm_select(name);
}

unsafe fn arm_select(name: &str) {
    let pane = crate::app(|a| a.s.active);
    if let Ok(mut p) = PENDING.lock() {
        *p = Some(Pending { pane, name: name.to_string(), left: 40 });
    }
    let hwnd = crate::app(|a| a.panes.get(pane).and_then(|p| p.as_ref()).map(|p| p.hwnd)).unwrap_or_default();
    if !hwnd.0.is_null() {
        let _ = SetTimer(Some(hwnd), crate::TIMER_CLIPSEL, 50, None);
    }
}

/// Win+Shift+V, or whatever chord the keymap now has. A dialog that has disabled its owner comes forward instead.
pub(crate) unsafe fn on_hotkey() {
    if modal_forward() {
        return;
    }
    let (drop, text) = read_clip();
    crate::reveal_window(front_target());
    let Some(path) = clippath::choose_clip(drop.as_deref(), text.as_deref(), expand_env) else {
        miss();
        return;
    };
    match clippath::plan(&path, probe_path(&path)) {
        Some(ClipPlan::Folder(folder)) => crate::new_tab_at(Some(&folder)),
        Some(ClipPlan::File { folder, name }) => {
            crate::new_tab_at(Some(&folder));
            arm_select(&name);
        }
        None => miss(),
    }
}

/// The new tab's shell view fills a moment later. Select the file without starting a rename.
pub(crate) unsafe fn on_select_timer(h: HWND) {
    let job = PENDING.lock().ok().and_then(|mut p| {
        let cur = p.as_mut()?;
        if crate::pane_of(h) != Some(cur.pane) {
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
        if select_named(pane, &name) || left == 0 {
            if let Ok(mut p) = PENDING.lock() {
                *p = None;
            }
            let _ = KillTimer(Some(h), crate::TIMER_CLIPSEL);
        }
    } else {
        let _ = KillTimer(Some(h), crate::TIMER_CLIPSEL);
    }
}

unsafe fn select_named(i: usize, name: &str) -> bool {
    let Some(fv) = crate::view_of::<IFolderView2>(i) else { return false };
    if fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0) <= 0 {
        return false;
    }
    let Ok(folder) = fv.GetFolder::<IShellFolder>() else { return false };
    let mut pidl = std::ptr::null_mut();
    let parsed = folder.ParseDisplayName(HWND::default(), None, &HSTRING::from(name), None, &mut pidl, std::ptr::null_mut());
    if parsed.is_err() || pidl.is_null() {
        return false;
    }
    let Some(sv) = crate::view_of::<IShellView>(i) else {
        CoTaskMemFree(Some(pidl as _));
        return false;
    };
    // No SVSI_EDIT: selecting the clipboard file must not start a rename.
    let flags = (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    let ok = sv.SelectItem(pidl, flags).is_ok();
    CoTaskMemFree(Some(pidl as _));
    if ok {
        crate::focus_view(i);
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_stays_hidden_until_a_window_exists() {
        assert!(crate::app(|a| a.main.0.is_null()));
        assert!(registration_notice().is_none());
    }
}
