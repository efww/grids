use crate::*;
use windows::Win32::System::Recovery::{RegisterApplicationRestart, UnregisterApplicationRestart, REGISTER_APPLICATION_RESTART_FLAGS, RESTART_NO_CRASH, RESTART_NO_HANG};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ};

// ---------- View > Start with Windows (minimized) ----------
// HKCU Run value only (no admin rights). The registry is the only record, nothing in grids.ini.

/// The Run value's argument: start minimized and without taking the focus.
pub(crate) const STARTUP_FLAG: &str = "--startup";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
// Task Manager's Startup apps on/off switch for the same name
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const RUN_NAME: &str = "Grids";
// Windows skips Run commands longer than this
const RUN_MAX: usize = 260;

/// A start from the Run value, minimized: not yet placed, minimized and waiting for the first restore, or done.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Quiet {
    No,
    Placing,
    Waiting,
}

/// Takes a leading STARTUP_FLAG off; true = started by Windows at sign-in.
pub(crate) fn take_startup_flag(args: &mut Vec<String>) -> bool {
    let on = args.first().is_some_and(|a| a.eq_ignore_ascii_case(STARTUP_FLAG));
    if on {
        args.remove(0);
    }
    on
}

pub(crate) fn run_command(exe: &str) -> String {
    format!(r#""{exe}" {STARTUP_FLAG}"#)
}

/// True = the Run value starts this exe (paths compared without case).
pub(crate) fn run_owned(cmd: &str, exe: &str) -> bool {
    cmd.trim().to_lowercase() == run_command(exe).to_lowercase()
}

pub(crate) fn run_fits(cmd: &str) -> bool {
    cmd.encode_utf16().count() <= RUN_MAX
}

/// Task Manager's "Disabled": the first byte of the StartupApproved value is odd (02 on, 03 off).
pub(crate) fn approved_off(data: &[u8]) -> bool {
    data.first().is_some_and(|b| b & 1 == 1)
}

unsafe fn run_value() -> Option<String> {
    let (mut buf, mut n) = ([0u16; 1024], 2048u32);
    RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(RUN_NAME), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n)).ok().ok()?;
    Some(String::from_utf16_lossy(&buf[..(n as usize / 2).min(1024)]).trim_end_matches('\0').to_string())
}

unsafe fn approved_value() -> Option<Vec<u8>> {
    let (mut buf, mut n) = ([0u8; 64], 64u32);
    RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(APPROVED_KEY), &HSTRING::from(RUN_NAME), RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as _), Some(&mut n)).ok().ok()?;
    Some(buf[..(n as usize).min(64)].to_vec())
}

/// Checked in the View menu only when Windows will really start this exe: moved Grids or switched off in
/// Task Manager shows unchecked, and one click registers it again.
pub(crate) unsafe fn autostart_on() -> bool {
    this_exe().is_some_and(|e| run_value().is_some_and(|c| run_owned(&c, &e))) && !approved_value().is_some_and(|d| approved_off(&d))
}

unsafe fn delete_value(key: &str) -> bool {
    let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(RUN_NAME));
    r.is_ok() || r == ERROR_FILE_NOT_FOUND
}

/// On: the Run value for this exe, and Task Manager's "Disabled" cleared. Off: the Run value removed.
/// True = Windows now does what was asked.
pub(crate) unsafe fn set_autostart(on: bool) -> bool {
    if !on {
        return delete_value(RUN_KEY);
    }
    let Some(cmd) = this_exe().map(|e| run_command(&e)).filter(|c| run_fits(c)) else { return false };
    let data: Vec<u16> = cmd.encode_utf16().chain([0]).collect();
    let set = RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(RUN_NAME), REG_SZ.0, Some(data.as_ptr() as _), (data.len() * 2) as u32);
    set.is_ok() && delete_value(APPROVED_KEY) && autostart_on()
}

// View menu click: flips what is really registered; a failure says so in the status line.
pub(crate) unsafe fn toggle_autostart() {
    let on = !autostart_on();
    if set_autostart(on) {
        return;
    }
    _ = MessageBeep(MB_ICONWARNING);
    let text = if on { tr("Could not add to startup", "시작 프로그램 등록 실패") } else { tr("Could not remove from startup", "시작 프로그램 해제 실패") };
    app(|a| a.note = Some((text.into(), Instant::now() + Duration::from_secs(3))));
    update_status();
}

// ---------- Windows' app restart (update reboot, "restartable apps" sign-in option) ----------
// Windows keeps the exe path itself and adds these arguments: the same quiet start as the Run value.

pub(crate) fn restart_args() -> &'static str {
    STARTUP_FLAG
}

/// Only after a reboot or an update: never after a crash or a hang, so no restart loop.
pub(crate) fn restart_flags() -> REGISTER_APPLICATION_RESTART_FLAGS {
    RESTART_NO_CRASH | RESTART_NO_HANG
}

/// The instance that owns the window, once it is up. A failure only means no restart: nothing to say.
pub(crate) unsafe fn register_restart() {
    _ = RegisterApplicationRestart(&HSTRING::from(restart_args()), restart_flags());
}

/// The window is closed: a reboot while copies still finish (or the renamed old exe after an update)
/// must not bring Grids back.
pub(crate) unsafe fn unregister_restart() {
    _ = UnregisterApplicationRestart();
}

/// Placed and laid out: still minimized, wait for the first restore; shown some other way, wake now.
pub(crate) unsafe fn quiet_placed(main: HWND) {
    if IsIconic(main).as_bool() {
        app(|a| a.quiet = Quiet::Waiting);
    } else {
        quiet_wake(main, if IsZoomed(main).as_bool() { SIZE_MAXIMIZED } else { SIZE_RESTORED });
    }
}

/// WM_SIZE: the first time the minimized start is shown, it comes up as saved (maximized too, even where
/// Windows ignored WPF_RESTORETOMAXIMIZED), and panes whose folder was not reachable at sign-in
/// (a network or USB drive not there yet) try it again.
pub(crate) unsafe fn quiet_size(main: HWND, kind: u32) {
    if kind != SIZE_MINIMIZED && app(|a| a.quiet == Quiet::Waiting) {
        quiet_wake(main, kind);
    }
}

unsafe fn quiet_wake(main: HWND, kind: u32) {
    let (max, late) = app(|a| {
        a.quiet = Quiet::No;
        (std::mem::take(&mut a.quiet_max), std::mem::take(&mut a.late))
    });
    if max && kind != SIZE_MAXIMIZED {
        _ = ShowWindow(main, SW_MAXIMIZE);
    }
    for (i, p) in late {
        // only a pane still on This PC (or not there yet): anything opened there since is the user's
        if app(|a| i < a.s.panes() && a.panes[i].is_some() && still_late(&a.s.paths[i])) {
            go_near(i, &p);
        }
    }
}

/// A late pane's path before anything else opened there: This PC, or empty (go_near clears a missing
/// folder, so before This PC has opened, or if it failed, the path is "").
fn still_late(path: &str) -> bool {
    path.is_empty() || path == THIS_PC
}

/// grids.ini keeps the folders of panes still waiting for their retry, not This PC.
pub(crate) fn keep_late(s: &Session, late: &[(usize, String)]) -> Session {
    let mut s = s.clone();
    for (i, p) in late {
        if s.paths.get(*i).is_some_and(|c| still_late(c)) {
            s.paths[*i] = p.clone();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_value_text() {
        let exe = r"C:\Tools\Grids\grids.exe";
        assert_eq!(run_command(exe), r#""C:\Tools\Grids\grids.exe" --startup"#);
        assert!(run_owned(&run_command(exe), exe));
        assert!(run_owned(r#"  "c:\tools\GRIDS\Grids.EXE" --STARTUP "#, exe)); // case and stray spaces
        assert!(!run_owned(r#""D:\Old\grids.exe" --startup"#, exe)); // moved: another path
        assert!(!run_owned(r"C:\Tools\Grids\grids.exe --startup", exe)); // no quotes
        assert!(!run_owned(r#""C:\Tools\Grids\grids.exe""#, exe)); // no flag
        assert!(run_owned(&run_command(r"C:\사용자\문서\grids.exe"), r"C:\사용자\문서\grids.exe"));
    }

    #[test]
    fn run_length_limit() {
        let ok = format!(r"C:\{}\grids.exe", "a".repeat(260 - 25));
        assert_eq!(run_command(&ok).encode_utf16().count(), 260);
        assert!(run_fits(&run_command(&ok)));
        assert!(!run_fits(&run_command(&format!(r"C:\a{}", &ok[2..]))));
    }

    #[test]
    fn task_manager_switch() {
        assert!(!approved_off(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(approved_off(&[3, 0, 0, 0, 0x10, 0x20, 0, 0, 0, 0, 0, 0]));
        assert!(!approved_off(&[6, 0, 0, 0]));
        assert!(approved_off(&[7]));
        assert!(!approved_off(&[]));
    }

    #[test]
    fn startup_flag_leaves_the_folders_in_order() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut args = v(&["--startup", r"C:\a", r"D:\b", r"E:\c"]);
        assert!(take_startup_flag(&mut args));
        assert_eq!(args, v(&[r"C:\a", r"D:\b", r"E:\c"]));
        let mut s = Session::default();
        apply_args(&mut s, args);
        assert_eq!(&s.paths[..3], &v(&[r"C:\a", r"D:\b", r"E:\c"])[..]);
        let mut only = v(&["--startup"]);
        assert!(take_startup_flag(&mut only) && only.is_empty());
        // not leading: a folder argument, left alone
        let mut later = v(&[r"C:\a", "--startup"]);
        assert!(!take_startup_flag(&mut later));
        assert_eq!(later.len(), 2);
    }

    #[test]
    fn restart_is_the_quiet_start() {
        let mut args = vec![restart_args().to_string()];
        assert!(take_startup_flag(&mut args) && args.is_empty());
        assert!(restart_flags().contains(RESTART_NO_CRASH) && restart_flags().contains(RESTART_NO_HANG));
    }

    #[test]
    fn late_panes_keep_their_folder() {
        let mut s = Session::default();
        s.paths[0] = THIS_PC.into();
        s.paths[1] = r"C:\x".into(); // opened by hand since: kept
        s.paths[2] = THIS_PC.into();
        s.paths[3] = String::new(); // cleared by go_near, This PC not open yet: still waiting
        let kept = keep_late(&s, &[(0, r"\\nas\share".into()), (1, r"Z:\old".into()), (3, r"U:\usb".into()), (40, r"Q:\".into())]);
        assert_eq!((kept.paths[0].as_str(), kept.paths[1].as_str(), kept.paths[2].as_str()), (r"\\nas\share", r"C:\x", THIS_PC));
        assert_eq!(kept.paths[3], r"U:\usb");
        assert_eq!(s.paths[0], THIS_PC); // the live session is untouched
    }
}
