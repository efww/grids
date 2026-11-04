//! Pure decisions for reconnecting a UNC or mapped share after sleep, or on F5.
//! The window proc only snapshots pane paths, then the existing net worker probes them.
//! Nothing here touches the UI-thread `app()` borrow.

use super::{drive_root, share_kind, unc_path};

/// `WM_POWERBROADCAST`. Handled in the main window proc; the work is queued, not done here.
pub(crate) const WM_POWERBROADCAST: u32 = 0x0218;
/// User input resumed the machine. Sent after `PBT_APMRESUMEAUTOMATIC` when a person is back.
pub(crate) const PBT_APMRESUMESUSPEND: u32 = 0x0007;
/// The machine resumed. Always sent, including when nobody is at the keyboard yet.
pub(crate) const PBT_APMRESUMEAUTOMATIC: u32 = 0x0012;

/// Automatic retries after the first resume probe fails. Not used for F5 or a typed path.
pub(crate) const RESUME_RETRIES: u8 = 3;

/// Delay before retry 1, 2, and 3. The first probe runs immediately.
pub(crate) const RESUME_BACKOFF_MS: [u32; 3] = [2_000, 5_000, 10_000];

/// True for the two resume broadcasts. Query and suspend events are not reconnects.
pub(crate) fn power_resume(event: u32) -> bool {
    event == PBT_APMRESUMEAUTOMATIC || event == PBT_APMRESUMESUSPEND
}

/// Delay before automatic retry number `n` (1, 2, or 3).
pub(crate) fn resume_backoff_ms(retry: u32) -> Option<u32> {
    let i = retry.checked_sub(1)? as usize;
    RESUME_BACKOFF_MS.get(i).copied()
}

/// `left` is how many automatic retries are still unused before this failure is counted.
/// Three left waits 2s, two left waits 5s, one left waits 10s.
pub(crate) fn next_resume_wait(left: u8) -> Option<u32> {
    if left == 0 || left > RESUME_RETRIES {
        return None;
    }
    resume_backoff_ms(u32::from(RESUME_RETRIES - left + 1))
}

/// `\\server\share` from a UNC path. A drive letter, a device path, and a bare server are `None`.
pub(crate) fn share_root_of(path: &str) -> Option<String> {
    let p = path.trim();
    let b = p.as_bytes();
    if b.len() < 2 || b[0] != b[1] || (b[0] != b'\\' && b[0] != b'/') {
        return None;
    }
    let rest = if b.len() >= 4 && (b[2] == b'?' || b[2] == b'.') && (b[3] == b'\\' || b[3] == b'/') {
        if b[2] != b'?' {
            return None;
        }
        let tail = p.get(4..)?;
        let r = tail.as_bytes();
        if r.len() < 4 || !r[..3].eq_ignore_ascii_case(b"UNC") || (r[3] != b'\\' && r[3] != b'/') {
            return None;
        }
        tail.get(4..)?
    } else {
        p.get(2..)?
    };
    let mut parts = rest.split(|c| c == '\\' || c == '/').filter(|s| !s.is_empty());
    let server = parts.next()?;
    let share = parts.next()?;
    Some(format!(r"\\{server}\{share}"))
}

/// Win32 codes that mean the redirector refused the session or wants a password.
/// A dead server (bad path, unreachable) is not in this list: that probe stays `SHParseDisplayName`.
pub(crate) fn needs_credentials(err: u32) -> bool {
    matches!(
        err,
        5 |    // ERROR_ACCESS_DENIED
        65 |   // ERROR_NETWORK_ACCESS_DENIED
        86 |   // ERROR_INVALID_PASSWORD
        1219 | // ERROR_SESSION_CREDENTIAL_CONFLICT
        1326 | // ERROR_LOGON_FAILURE
        1330 | // ERROR_PASSWORD_EXPIRED
        1331 | // ERROR_ACCOUNT_DISABLED
        1385 | // ERROR_LOGON_TYPE_NOT_GRANTED
        1793 | // ERROR_ACCOUNT_EXPIRED
        1907 | // ERROR_PASSWORD_MUST_CHANGE
        1909 // ERROR_ACCOUNT_LOCKED_OUT
    )
}

/// Cached drive type for `path`'s letter. `drives` is `(letter, GetDriveTypeW)`. No disk call.
pub(crate) fn cached_kind(drives: &[(u8, u32)], path: &str) -> Option<u32> {
    let root = drive_root(path)?;
    let letter = *root.as_bytes().first()?;
    drives.iter().find(|d| d.0 == letter).map(|d| d.1)
}

/// A folder whose refresh must leave the UI thread. `drive_type` is the cached type, if we have one.
/// `Some(4)` is `DRIVE_REMOTE`. A missing type is not guessed: an unknown letter stays on the local path.
pub(crate) fn folder_is_share(path: &str, drive_type: Option<u32>, live_mount: bool) -> bool {
    share_kind(unc_path(path) || live_mount, drive_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{tr, Key, UI};
    use std::sync::atomic::Ordering;
    use windows::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_ACCOUNT_DISABLED, ERROR_ACCOUNT_EXPIRED, ERROR_ACCOUNT_LOCKED_OUT, ERROR_BAD_NETPATH, ERROR_FILE_NOT_FOUND,
        ERROR_INVALID_PASSWORD, ERROR_LOGON_FAILURE, ERROR_LOGON_TYPE_NOT_GRANTED, ERROR_NETWORK_ACCESS_DENIED, ERROR_PASSWORD_EXPIRED,
        ERROR_PASSWORD_MUST_CHANGE, ERROR_PATH_NOT_FOUND, ERROR_SESSION_CREDENTIAL_CONFLICT,
    };

    #[test]
    fn resume_events_and_backoff() {
        assert!(power_resume(PBT_APMRESUMEAUTOMATIC));
        assert!(power_resume(PBT_APMRESUMESUSPEND));
        assert!(!power_resume(0));
        assert!(!power_resume(4)); // PBT_APMSUSPEND
        assert!(!power_resume(0x8013)); // PBT_POWERSETTINGCHANGE
        assert_eq!(RESUME_RETRIES, 3);
        assert_eq!(resume_backoff_ms(1), Some(2_000));
        assert_eq!(resume_backoff_ms(2), Some(5_000));
        assert_eq!(resume_backoff_ms(3), Some(10_000));
        assert_eq!(resume_backoff_ms(0), None);
        assert_eq!(resume_backoff_ms(4), None);
        assert_eq!(next_resume_wait(3), Some(2_000));
        assert_eq!(next_resume_wait(2), Some(5_000));
        assert_eq!(next_resume_wait(1), Some(10_000));
        assert_eq!(next_resume_wait(0), None);
        assert_eq!(next_resume_wait(4), None);
    }

    #[test]
    fn share_root_is_server_and_share_only() {
        assert_eq!(share_root_of(r"\\server\share").as_deref(), Some(r"\\server\share"));
        assert_eq!(share_root_of(r"\\server\share\dir\file.txt").as_deref(), Some(r"\\server\share"));
        assert_eq!(share_root_of(r"\\server\share\").as_deref(), Some(r"\\server\share"));
        assert_eq!(share_root_of("//server/share/dir").as_deref(), Some(r"\\server\share"));
        assert_eq!(share_root_of(r"\\?\UNC\server\share\dir").as_deref(), Some(r"\\server\share"));
        assert_eq!(share_root_of(r"\\?\unc\SERVER\Share\").as_deref(), Some(r"\\SERVER\Share"));
        assert_eq!(share_root_of(r"\\10.255.255.2\data\a").as_deref(), Some(r"\\10.255.255.2\data"));
        assert_eq!(share_root_of(r"  \\server\share\a  ").as_deref(), Some(r"\\server\share"));
        assert_eq!(share_root_of(r"\\server"), None);
        assert_eq!(share_root_of(r"\\server\"), None);
        assert_eq!(share_root_of(r"\\?\C:\Windows"), None);
        assert_eq!(share_root_of(r"\\.\PIPE\x"), None);
        assert_eq!(share_root_of(r"C:\share"), None);
        assert_eq!(share_root_of(r"Z:\"), None);
        assert_eq!(share_root_of(""), None);
        assert_eq!(share_root_of(r"\server\share"), None);
    }

    #[test]
    fn credential_errors_are_the_interactive_connect() {
        for err in [
            ERROR_ACCESS_DENIED.0,
            ERROR_NETWORK_ACCESS_DENIED.0,
            ERROR_INVALID_PASSWORD.0,
            ERROR_SESSION_CREDENTIAL_CONFLICT.0,
            ERROR_LOGON_FAILURE.0,
            ERROR_PASSWORD_EXPIRED.0,
            ERROR_ACCOUNT_DISABLED.0,
            ERROR_LOGON_TYPE_NOT_GRANTED.0,
            ERROR_ACCOUNT_EXPIRED.0,
            ERROR_PASSWORD_MUST_CHANGE.0,
            ERROR_ACCOUNT_LOCKED_OUT.0,
        ] {
            assert!(needs_credentials(err), "{err}");
        }
        assert!(!needs_credentials(0));
        assert!(!needs_credentials(ERROR_FILE_NOT_FOUND.0));
        assert!(!needs_credentials(ERROR_PATH_NOT_FOUND.0));
        assert!(!needs_credentials(ERROR_BAD_NETPATH.0));
        assert!(!needs_credentials(53)); // same code, named above
        assert!(!needs_credentials(1201)); // ERROR_CONNECTION_UNAVAIL: the parse probe handles it
    }

    #[test]
    fn cached_drive_type_picks_a_share_without_asking_the_disk() {
        let drives = [(b'C', 3u32), (b'Z', 4u32)];
        assert_eq!(cached_kind(&drives, r"Z:\docs\a"), Some(4));
        assert_eq!(cached_kind(&drives, r"c:\docs"), Some(3));
        assert_eq!(cached_kind(&drives, r"\\?\Z:\docs"), Some(4));
        assert_eq!(cached_kind(&drives, r"D:\x"), None);
        assert_eq!(cached_kind(&drives, r"\\server\share"), None);
        assert!(folder_is_share(r"\\server\share\a", None, false));
        assert!(folder_is_share(r"\\?\UNC\server\share", None, false));
        assert!(folder_is_share(r"Z:\docs", Some(4), false));
        assert!(folder_is_share(r"C:\mnt", Some(3), true));
        assert!(!folder_is_share(r"C:\docs", Some(3), false));
        assert!(!folder_is_share(r"Z:\docs", None, false));
        assert!(!folder_is_share(r"D:\docs", Some(2), false));
        assert!(!folder_is_share("", None, false));
    }

    #[test]
    fn reconnecting_is_translated() {
        let prev = UI.load(Ordering::Relaxed);
        UI.store(0, Ordering::Relaxed);
        assert_eq!(tr(Key::Reconnecting), "Reconnecting…");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(tr(Key::Reconnecting), "다시 연결 중…");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(tr(Key::Reconnecting), "再接続中…");
        UI.store(prev, Ordering::Relaxed);
    }
}
