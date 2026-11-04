//! Startup update check against the latest GitHub release, and the self-replace that installs it.
//! No token and no Actions: one unauthenticated GET to the public releases API, then the raw
//! `maguro.exe` asset and its `maguro.exe.sha256`. The exe is replaced only when the
//! SHA-256 matches. A missing checksum, a mismatch, or a zip-only release is reported and not installed.

use crate::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use windows::Win32::Networking::WinHttp::*;

/// Test builds can point at a local mock or pretend to be older; release builds use the real values.
const API: &str = match option_env!("MAGURO_UPDATE_API") {
    Some(u) => u,
    None => "https://api.github.com/repos/Samek86/maguro/releases/latest",
};
pub(crate) const APP_VERSION: &str = match option_env!("MAGURO_FAKE_VERSION") {
    Some(v) => v,
    None => env!("CARGO_PKG_VERSION"),
};
/// Largest maguro.exe the updater will download.
const EXE_MAX: usize = 64 << 20;
/// Largest maguro.exe.sha256 the updater will read. A sha256sum line is under 100 bytes.
const SHA_MAX: usize = 4096;
/// GitHub release asset the updater downloads. Releases upload this raw exe, not a zip.
const EXE_ASSET: &str = "maguro.exe";
/// sha256sum checksum published next to the exe. `<64 hex>  maguro.exe`.
const SHA_ASSET: &str = "maguro.exe.sha256";
/// Downloaded build, next to the running exe, until the swap moves it into place.
const NEW_EXE: &str = "maguro.new.exe";
const OLD_EXE: &str = "maguro.old.exe";

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Release {
    /// Raw `maguro.exe` plus a usable `maguro.exe.sha256`.
    Exe { tag: String, url: String, size: u64, sha_url: String, sha_size: u64 },
    /// `maguro.exe` was published without a usable checksum file. Shown, never installed.
    MissingSha { tag: String },
    /// Newer release that published a zip and no `maguro.exe`. Shown, never extracted.
    ZipOnly { tag: String },
}

impl Release {
    fn tag(&self) -> &str {
        match self {
            Release::Exe { tag, .. } | Release::MissingSha { tag } | Release::ZipOnly { tag } => tag,
        }
    }
}

/// Why `install` left the running exe in place.
enum InstallFail {
    /// Checksum asset missing, or the checksum body could not be downloaded.
    MissingSha,
    /// Checksum text was not a sha256sum line for maguro.exe, or the digest differed.
    Mismatch,
    Other,
}

static FOUND: Mutex<Option<Release>> = Mutex::new(None);
static INSTALLING: AtomicBool = AtomicBool::new(false);

fn version_parts(v: &str) -> Vec<u64> {
    let core = v.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next().unwrap_or("");
    core.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

/// True when `tag` (v1.2.3) is a higher version than `current`. Missing parts count as 0.
pub(crate) fn is_newer(tag: &str, current: &str) -> bool {
    let (a, b) = (version_parts(tag), version_parts(current));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

fn asset_file_name(asset: &serde_json::Value) -> Option<&str> {
    let name = asset.get("name")?.as_str()?;
    Some(name.rsplit(['/', '\\']).next().unwrap_or(name))
}

/// The tag plus the `maguro.exe` and `maguro.exe.sha256` assets.
/// The exe wins over a zip. An exe with no usable checksum is `MissingSha`, never a download.
/// A zip is never returned as something to download.
pub(crate) fn parse_release(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v.get("tag_name")?.as_str()?.to_string();
    let assets = v.get("assets")?.as_array()?;
    if let Some(asset) = assets.iter().find(|a| asset_file_name(a).is_some_and(|n| n.eq_ignore_ascii_case(EXE_ASSET))) {
        let url = asset.get("browser_download_url")?.as_str()?.to_string();
        let size = asset.get("size")?.as_u64()?;
        if let Some((sha_url, sha_size)) = sha_asset(assets) {
            return Some(Release::Exe { tag, url, size, sha_url, sha_size });
        }
        return Some(Release::MissingSha { tag });
    }
    let zip_only = assets.iter().any(|a| asset_file_name(a).is_some_and(|n| n.to_ascii_lowercase().ends_with(".zip")));
    zip_only.then_some(Release::ZipOnly { tag })
}

/// `maguro.exe.sha256` download URL and size, when the asset is small enough to be a sha256sum file.
fn sha_asset(assets: &[serde_json::Value]) -> Option<(String, u64)> {
    let sha = assets.iter().find(|a| asset_file_name(a).is_some_and(|n| n.eq_ignore_ascii_case(SHA_ASSET)))?;
    let sha_url = sha.get("browser_download_url")?.as_str()?.to_string();
    let sha_size = sha.get("size")?.as_u64()?;
    (1..=SHA_MAX as u64).contains(&sha_size).then_some((sha_url, sha_size))
}

/// First sha256sum line naming `maguro.exe`: `<64 hex>  maguro.exe`.
/// Binary mode (`hash *maguro.exe`) and a directory prefix on the name are accepted.
/// A bare 64-hex line (no filename) is also accepted so a mis-uploaded checksum asset still verifies;
/// a named `maguro.exe` line wins when both appear.
/// The hex is returned as published so the compare can be case-insensitive.
pub(crate) fn parse_sha256sum(text: &str) -> Option<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut bare: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else { continue };
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        match parts.next() {
            None => {
                if bare.is_none() {
                    bare = Some(hash.to_string());
                }
            }
            Some(name) => {
                if parts.next().is_some() {
                    continue;
                }
                let name = name.trim_start_matches('*');
                let name = name.rsplit(['/', '\\']).next().unwrap_or(name);
                if name.eq_ignore_ascii_case(EXE_ASSET) {
                    return Some(hash.to_string());
                }
            }
        }
    }
    bare
}

/// Case-insensitive match of two SHA-256 hex digests. A value that is not 64 hex digits does not match.
pub(crate) fn hashes_match(expected: &str, actual: &str) -> bool {
    fn hex64(s: &str) -> bool {
        s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
    }
    hex64(expected) && hex64(actual) && expected.eq_ignore_ascii_case(actual)
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn sha256_bytes(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex_lower(&Sha256::digest(data))
}

/// SHA-256 of `path`, lowercase hex. Streams the file so a 64 MB exe is not hashed from a second copy.
fn sha256_file(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 << 10];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(hex_lower(&hasher.finalize()))
}

/// Closes a WinHTTP handle when dropped.
struct Hnd(*mut core::ffi::c_void);
impl Drop for Hnd {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { _ = WinHttpCloseHandle(self.0); }
        }
    }
}

/// One GET over WinHTTP (no cache layer; redirects followed). Some(body) only on HTTP 200
/// and when the body is at most `max` bytes. Blocking: callers run it on the update worker thread.
fn http_get(url: &str, max: usize) -> Option<Vec<u8>> {
    let wide: Vec<u16> = url.encode_utf16().collect();
    let mut host = [0u16; 256];
    let mut path = vec![0u16; 4096];
    let mut uc = URL_COMPONENTS {
        dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
        lpszHostName: PWSTR(host.as_mut_ptr()),
        dwHostNameLength: host.len() as u32,
        lpszUrlPath: PWSTR(path.as_mut_ptr()),
        dwUrlPathLength: path.len() as u32,
        dwSchemeLength: 1,
        dwExtraInfoLength: 1,
        ..Default::default()
    };
    unsafe {
        WinHttpCrackUrl(&wide, 0, &mut uc).ok()?;
        // Path and query are cracked separately; the request needs both.
        let mut object: Vec<u16> = path[..uc.dwUrlPathLength as usize].to_vec();
        if !uc.lpszExtraInfo.is_null() {
            object.extend_from_slice(std::slice::from_raw_parts(uc.lpszExtraInfo.0, uc.dwExtraInfoLength as usize));
        }
        object.push(0);
        host[uc.dwHostNameLength as usize] = 0;
        let secure = uc.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;
        let ua = HSTRING::from(format!("Maguro/{APP_VERSION}"));
        let ses = Hnd(WinHttpOpen(&ua, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0));
        if ses.0.is_null() {
            return None;
        }
        _ = WinHttpSetTimeouts(ses.0, 10_000, 15_000, 30_000, 60_000);
        let con = Hnd(WinHttpConnect(ses.0, PCWSTR(host.as_ptr()), uc.nPort, 0));
        if con.0.is_null() {
            return None;
        }
        let flags = if secure { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let req = Hnd(WinHttpOpenRequest(con.0, w!("GET"), PCWSTR(object.as_ptr()), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), flags));
        if req.0.is_null() {
            return None;
        }
        let hdr: Vec<u16> = "Cache-Control: no-cache\r\nPragma: no-cache\r\n".encode_utf16().collect();
        WinHttpSendRequest(req.0, Some(&hdr), None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).ok()?;
        let mut status = 0u32;
        let mut len = 4u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut len,
            std::ptr::null_mut(),
        )
        .ok()?;
        if status != 200 {
            return None;
        }
        let mut body = Vec::new();
        let mut buf = vec![0u8; 64 << 10];
        loop {
            let mut got = 0u32;
            WinHttpReadData(req.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut got).ok()?;
            if got == 0 {
                break;
            }
            body.extend_from_slice(&buf[..got as usize]);
            if body.len() > max {
                return None;
            }
        }
        Some(body)
    }
}

fn download(url: &str, dest: &Path) -> bool {
    _ = std::fs::remove_file(dest);
    http_get(url, EXE_MAX).is_some_and(|b| std::fs::write(dest, b).is_ok())
}

fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Startup: remove the exe a previous update left behind, then ask GitHub when the setting is on.
pub(crate) fn startup(main: HWND, check: bool) {
    let h = main.0 as isize;
    _ = std::thread::Builder::new().name("maguro-update".into()).spawn(move || {
        if let Some(old) = exe_path().and_then(|e| e.parent().map(|d| d.join(OLD_EXE))) {
            // The previous process may still be closing for a moment.
            for _ in 0..20 {
                if !old.exists() || std::fs::remove_file(&old).is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        }
        if !check {
            return;
        }
        let text = http_get(API, 1 << 20).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let Some(rel) = parse_release(&text) else { return };
        if is_newer(rel.tag(), APP_VERSION) {
            let stage = match &rel {
                Release::Exe { .. } => 0,
                Release::MissingSha { .. } => 4,
                Release::ZipOnly { .. } => 3,
            };
            *FOUND.lock().unwrap_or_else(|e| e.into_inner()) = Some(rel);
            unsafe { _ = PostMessageW(Some(HWND(h as _)), WM_UPDATE, WPARAM(stage), LPARAM(0)); }
        }
    });
}

/// Download the raw exe and its checksum, then swap only when the SHA-256 matches.
/// On a missing checksum or a mismatch, `maguro.new.exe` is deleted and the running exe stays.
fn install(rel: &Release) -> std::result::Result<(), InstallFail> {
    let Release::Exe { url, size, sha_url, sha_size, .. } = rel else { return Err(InstallFail::MissingSha) };
    let exe = exe_path().ok_or(InstallFail::Other)?;
    let dir = exe.parent().ok_or(InstallFail::Other)?.to_path_buf();
    let new = dir.join(NEW_EXE);
    let res = (|| {
        if *size == 0 || *size > EXE_MAX as u64 {
            return Err(InstallFail::Other);
        }
        if !download(url, &new) {
            return Err(InstallFail::Other);
        }
        if std::fs::metadata(&new).map(|m| m.len()).ok() != Some(*size) {
            return Err(InstallFail::Other);
        }
        let mut magic = [0u8; 2];
        {
            let mut f = std::fs::File::open(&new).map_err(|_| InstallFail::Other)?;
            if std::io::Read::read(&mut f, &mut magic).ok() != Some(2) || &magic != b"MZ" {
                return Err(InstallFail::Other);
            }
        }
        let sha_body = http_get(sha_url, SHA_MAX).ok_or(InstallFail::MissingSha)?;
        if sha_body.len() as u64 != *sha_size {
            return Err(InstallFail::MissingSha);
        }
        let expected = parse_sha256sum(&String::from_utf8_lossy(&sha_body)).ok_or(InstallFail::Mismatch)?;
        let actual = sha256_file(&new).ok_or(InstallFail::Other)?;
        if !hashes_match(&expected, &actual) {
            return Err(InstallFail::Mismatch);
        }
        swap(&exe, &new, &dir.join(OLD_EXE)).map_err(|_| InstallFail::Other)
    })();
    // Success renamed `new` onto the exe. A mismatch, a missing checksum, or any other failure
    // leaves the temp file behind; drop it and do not install.
    _ = std::fs::remove_file(&new);
    res
}

/// Running exe -> maguro.old.exe, new -> exe. Puts the old one back if the second move fails.
fn swap(exe: &Path, new: &Path, old: &Path) -> std::result::Result<(), &'static str> {
    if old.exists() && std::fs::remove_file(old).is_err() {
        return Err("old exe busy");
    }
    std::fs::rename(exe, old).map_err(|_| "rename running exe")?;
    if std::fs::rename(new, exe).is_err() {
        _ = std::fs::rename(old, exe);
        return Err("move new exe");
    }
    Ok(())
}

fn rollback(exe: &Path) {
    let old = exe.with_file_name(OLD_EXE);
    if old.exists() {
        _ = std::fs::remove_file(exe);
        _ = std::fs::rename(&old, exe);
    }
}

/// WM_UPDATE on the main window.
pub(crate) unsafe fn on_update(h: HWND, stage: usize) {
    match stage {
        0 => {
            let Some(rel) = FOUND.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
            let Release::Exe { tag, .. } = &rel else { return };
            let text = tr(crate::Key::MaguroNewIsAvailableYouHaveCur)
            .replace("{new}", tag)
            .replace("{cur}", &format!("v{APP_VERSION}"));
            let title = tr(crate::Key::MaguroUpdate);
            if !ask_confirm(h, Confirm::ok(title, text).default_cancel()) {
                return;
            }
            if INSTALLING.swap(true, Ordering::SeqCst) {
                return;
            }
            let hv = h.0 as isize;
            let spawned = std::thread::Builder::new().name("maguro-update-install".into()).spawn(move || {
                let stage = match install(&rel) {
                    Ok(()) => 1,
                    Err(InstallFail::MissingSha) => 4,
                    Err(InstallFail::Mismatch) => 5,
                    Err(InstallFail::Other) => 2,
                };
                _ = PostMessageW(Some(HWND(hv as _)), WM_UPDATE, WPARAM(stage), LPARAM(0));
            });
            if spawned.is_err() {
                INSTALLING.store(false, Ordering::SeqCst);
            }
        }
        3 => {
            let Some(Release::ZipOnly { tag }) = FOUND.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
            zip_only(h, &tag);
        }
        4 => {
            INSTALLING.store(false, Ordering::SeqCst);
            let Some(tag) = FOUND.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|r| r.tag().to_string()) else { return };
            sha_missing(h, &tag);
        }
        5 => {
            INSTALLING.store(false, Ordering::SeqCst);
            sha_mismatch(h);
        }
        1 => {
            let Some(exe) = exe_path() else { return };
            // current_exe still names the original path, which now holds the new build.
            let started = std::process::Command::new(&exe)
                .arg("--updated")
                .arg(std::process::id().to_string())
                .current_dir(exe.parent().unwrap_or(Path::new(".")))
                .spawn()
                .is_ok();
            if !started {
                rollback(&exe);
                INSTALLING.store(false, Ordering::SeqCst);
                failed(h);
                return;
            }
            close_all_windows();
        }
        _ => {
            INSTALLING.store(false, Ordering::SeqCst);
            failed(h);
        }
    }
}

/// Zip-only notice. Versions only: no address and no site name.
pub(crate) fn zip_notice(tag: &str, current: &str) -> String {
    tr(crate::Key::MaguroNewIsAvailableYouHaveCur2).replace("{new}", tag).replace("{cur}", current)
}

/// A newer release published only a zip. Tell the user; do not download or extract it.
unsafe fn zip_only(h: HWND, tag: &str) {
    let text = zip_notice(tag, &format!("v{APP_VERSION}"));
    let title = tr(crate::Key::MaguroUpdate);
    MessageBoxW(Some(h), &HSTRING::from(text), &HSTRING::from(title), MB_OK | MB_ICONINFORMATION);
}

/// The release has no `maguro.exe.sha256`, or that file could not be downloaded. Nothing was installed.
unsafe fn sha_missing(h: HWND, tag: &str) {
    let text = tr(crate::Key::MaguroNewIsAvailableYouHaveCur3)
    .replace("{new}", tag)
    .replace("{cur}", &format!("v{APP_VERSION}"));
    let title = tr(crate::Key::MaguroUpdate);
    MessageBoxW(Some(h), &HSTRING::from(text), &HSTRING::from(title), MB_OK | MB_ICONWARNING);
}

/// The downloaded exe does not match `maguro.exe.sha256`. The temp file is already deleted.
unsafe fn sha_mismatch(h: HWND) {
    let text = tr(crate::Key::DownloadedMaguroExeDoesNotMatchPanegri);
    let title = tr(crate::Key::MaguroUpdate);
    MessageBoxW(Some(h), &HSTRING::from(text), &HSTRING::from(title), MB_OK | MB_ICONWARNING);
}

unsafe fn failed(h: HWND) {
    let text = tr(crate::Key::UpdateCouldNotBeInstalledMaguroWas);
    let title = tr(crate::Key::MaguroUpdate);
    MessageBoxW(Some(h), &HSTRING::from(text), &HSTRING::from(title), MB_OK | MB_ICONWARNING);
}

/// Every Maguro main window of this process gets WM_CLOSE, so each saves as on a normal exit.
unsafe fn close_all_windows() {
    unsafe extern "system" fn each(h: HWND, _a1: LPARAM) -> BOOL {
        crate::crash::catch_bool(std::panic::AssertUnwindSafe(|| unsafe { each_inner(h, _a1) }))
    }
    unsafe fn each_inner(h: HWND, _a1: LPARAM) -> BOOL {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let mut cls = [0u16; 32];
        let n = GetClassNameW(h, &mut cls) as usize;
        if pid == std::process::id() && String::from_utf16_lossy(&cls[..n]) == "MaguroMain" {
            _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        TRUE
    }
    _ = EnumWindows(Some(each), LPARAM(0));
}

/// Theme restart. The new process waits out this one, then opens the saved tabs instead of the startup layout.
pub(crate) const KEEP_TABS: &str = "--keep-tabs";

/// Drop `--keep-tabs` from the launch arguments. True when it was present.
pub(crate) fn take_keep_tabs(args: &mut Vec<String>) -> bool {
    let keep = args.iter().any(|a| a == KEEP_TABS);
    args.retain(|a| a != KEEP_TABS);
    keep
}

/// Start this exe again and close every window of this process. The child waits until we exit, then reads the session just saved.
pub(crate) unsafe fn relaunch_same_session() -> bool {
    let Some(exe) = exe_path() else { return false };
    let started = std::process::Command::new(&exe)
        .arg("--updated")
        .arg(std::process::id().to_string())
        .arg(KEEP_TABS)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .spawn()
        .is_ok();
    if started {
        close_all_windows();
    }
    started
}

/// `--updated <pid>` from the old process: wait for it to save and exit before reading settings.
pub(crate) fn wait_for_previous(args: &mut Vec<String>) {
    if args.first().map(String::as_str) != Some("--updated") {
        return;
    }
    let pid = args.get(1).and_then(|p| p.parse::<u32>().ok());
    args.drain(..args.len().min(2));
    if let Some(pid) = pid {
        unsafe {
            if let Ok(p) = OpenProcess(windows::Win32::System::Threading::PROCESS_SYNCHRONIZE, false, pid) {
                windows::Win32::System::Threading::WaitForSingleObject(p, 20_000);
                _ = CloseHandle(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_tabs_flag_is_removed_from_the_launch_args() {
        let mut args = vec!["--keep-tabs".into(), r"C:\Work".into()];
        assert!(take_keep_tabs(&mut args));
        assert_eq!(args, vec![r"C:\Work".to_string()]);
        assert!(!take_keep_tabs(&mut args));
        assert_eq!(args, vec![r"C:\Work".to_string()]);
    }

    #[test]
    fn zip_notice_has_versions_and_no_address() {
        let text = zip_notice("v0.2.0", "v0.1.77");
        assert!(text.contains("v0.2.0"), "{text}");
        assert!(text.contains("v0.1.77"), "{text}");
        let low = text.to_ascii_lowercase();
        assert!(!low.contains("github"), "{text}");
        assert!(!text.contains("http"), "{text}");
        assert!(!text.contains("{url}"), "{text}");
        assert!(!text.contains('{'), "{text}");
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.1", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn release_json_picks_exe_and_sha_assets() {
        let j = r#"{"tag_name":"v0.2.0","assets":[{"name":"notes.txt","size":3,"browser_download_url":"x"},
            {"name":"maguro-v0.2.0-x64.zip","size":99,"browser_download_url":"https://e/p.zip"},
            {"name":"maguro.exe.SHA256","size":80,"browser_download_url":"https://e/maguro.exe.sha256"},
            {"name":"Maguro.EXE","size":1234,"browser_download_url":"https://e/maguro.exe"}]}"#;
        assert_eq!(
            parse_release(j),
            Some(Release::Exe {
                tag: "v0.2.0".into(),
                url: "https://e/maguro.exe".into(),
                size: 1234,
                sha_url: "https://e/maguro.exe.sha256".into(),
                sha_size: 80,
            })
        );
        assert_eq!(parse_release(r#"{"tag_name":"v1","assets":[]}"#), None);
        assert_eq!(parse_release("not json"), None);
        // A checksum with no exe is not an update.
        assert_eq!(
            parse_release(r#"{"tag_name":"v9","assets":[{"name":"maguro.exe.sha256","size":80,"browser_download_url":"https://e/s"}]}"#),
            None
        );
    }

    #[test]
    fn release_json_exe_without_sha_is_not_installed() {
        let j = r#"{"tag_name":"v0.2.0","assets":[
            {"name":"maguro.exe","size":1234,"browser_download_url":"https://e/maguro.exe"},
            {"name":"maguro-v0.2.0-x64.zip","size":99,"browser_download_url":"https://e/p.zip"}]}"#;
        assert_eq!(parse_release(j), Some(Release::MissingSha { tag: "v0.2.0".into() }));
        // An empty or oversized checksum file cannot be used.
        let empty = r#"{"tag_name":"v0.2.0","assets":[
            {"name":"maguro.exe","size":10,"browser_download_url":"https://e/maguro.exe"},
            {"name":"maguro.exe.sha256","size":0,"browser_download_url":"https://e/s"}]}"#;
        assert_eq!(parse_release(empty), Some(Release::MissingSha { tag: "v0.2.0".into() }));
        let big = r#"{"tag_name":"v0.2.0","assets":[
            {"name":"maguro.exe","size":10,"browser_download_url":"https://e/maguro.exe"},
            {"name":"maguro.exe.sha256","size":4097,"browser_download_url":"https://e/s"}]}"#;
        assert_eq!(parse_release(big), Some(Release::MissingSha { tag: "v0.2.0".into() }));
    }

    #[test]
    fn sha256sum_parses_and_compares_case_insensitively() {
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        // sha256sum text mode: 64 hex, two spaces, maguro.exe.
        assert_eq!(parse_sha256sum(&format!("{hash}  maguro.exe\n")).as_deref(), Some(hash));
        assert_eq!(parse_sha256sum(&format!("{hash}  maguro.exe\r\n")).as_deref(), Some(hash));
        let upper = hash.to_ascii_uppercase();
        let parsed = parse_sha256sum(&format!("\u{feff}{upper}  maguro.exe")).unwrap();
        assert!(hashes_match(&parsed, hash));
        assert!(hashes_match(hash, &upper));
        assert!(!hashes_match(&parsed, &hash.replace('b', "c")));
        // Binary mode, a directory prefix, and a case-different file name.
        assert_eq!(parse_sha256sum(&format!("{hash} *maguro.exe")).as_deref(), Some(hash));
        assert_eq!(parse_sha256sum(&format!("{hash}  build/maguro.exe")).as_deref(), Some(hash));
        assert_eq!(parse_sha256sum(&format!("{hash}  MAGURO.EXE")).as_deref(), Some(hash));
        assert_eq!(parse_sha256sum(&format!("# comment\n\n{hash}  maguro.exe")).as_deref(), Some(hash));
        assert!(parse_sha256sum("deadbeef  maguro.exe").is_none());
        assert!(parse_sha256sum(&format!("{hash}  other.exe")).is_none());
        assert!(parse_sha256sum(&format!("{hash}  maguro.exe extra")).is_none());
        // Bare hash (no filename) — tolerant of a mis-uploaded sha256 asset.
        assert_eq!(parse_sha256sum(&format!("{hash}")).as_deref(), Some(hash));
        assert_eq!(parse_sha256sum(&format!("{hash}\n")).as_deref(), Some(hash));
        // Named line wins over a bare hash earlier in the file.
        assert_eq!(parse_sha256sum(&format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n{hash}  maguro.exe")).as_deref(), Some(hash));
        assert!(parse_sha256sum("").is_none());
        assert!(parse_sha256sum("SHA256 (maguro.exe) = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad").is_none());
        assert!(!hashes_match("gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg", hash));
        assert!(!hashes_match(hash, &hash[..63]));
        assert!(!hashes_match("", ""));
    }

    #[test]
    fn sha256_of_abc_matches_the_published_digest() {
        // echo -n abc | sha256sum
        let digest = sha256_bytes(b"abc");
        assert_eq!(digest, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let line = format!("{}  maguro.exe", digest.to_ascii_uppercase());
        assert!(hashes_match(parse_sha256sum(&line).as_deref().unwrap(), &digest));
        let path = std::env::temp_dir().join(format!("pg-sha-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(sha256_file(&path).as_deref(), Some(digest.as_str()));
        _ = std::fs::remove_file(&path);
    }

    #[test]
    fn release_json_zip_only_is_not_an_exe() {
        let j = r#"{"tag_name":"v0.1.4","assets":[
            {"name":"maguro-v0.1.4-x64.zip","size":1252190,"browser_download_url":"https://e/p.zip"}]}"#;
        assert_eq!(parse_release(j), Some(Release::ZipOnly { tag: "v0.1.4".into() }));
        // Some other exe is not the update asset. A zip beside it is still zip-only.
        let other = r#"{"tag_name":"v0.2.0","assets":[
            {"name":"setup.exe","size":10,"browser_download_url":"https://e/setup.exe"},
            {"name":"maguro-v0.2.0-x64.zip","size":99,"browser_download_url":"https://e/p.zip"}]}"#;
        assert_eq!(parse_release(other), Some(Release::ZipOnly { tag: "v0.2.0".into() }));
        assert_eq!(
            parse_release(r#"{"tag_name":"v9","assets":[{"name":"readme.txt","size":1,"browser_download_url":"x"}]}"#),
            None
        );
    }

    #[test]
    fn swap_moves_and_rolls_back() {
        let d = std::env::temp_dir().join(format!("pg-swap-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (exe, new, old) = (d.join("maguro.exe"), d.join("maguro.new.exe"), d.join(OLD_EXE));
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        assert!(swap(&exe, &new, &old).is_ok());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "old");
        rollback(&exe);
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        assert!(!old.exists());
        // A missing new file leaves the running exe in place.
        assert!(swap(&exe, &new, &old).is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        _ = std::fs::remove_dir_all(&d);
    }
}
