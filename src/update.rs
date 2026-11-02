use crate::*;
use std::io::Cursor;
use windows::Win32::Networking::WinHttp::*;
use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
use windows::Win32::UI::Controls::{TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW};

// ---------- updates: a blue "Update" button at the right of the toolbar when GitHub has a newer release ----------
// A background thread asks for the latest release about a minute after start and then every 6 hours (only while
// "Check for updates" is on; off = no network at all). Failures stay silent and are retried next time. Clicking the
// button asks once, then downloads the zip and SHA256SUMS.txt, checks the zip's SHA-256, takes grids.exe out of it,
// swaps it in beside the running exe (which becomes grids.old.exe, removed on the next start) and restarts.
// Anything going wrong changes nothing and offers the release page instead.
//
// Testing: GRIDS_UPDATE_FEED=<url> points the check at another "latest release" JSON (e.g. a local
// `python -m http.server` serving a fake v9.9.9 release); the first check then comes after 3 s instead of 60.

const FEED: &str = "https://api.github.com/repos/efww/grids/releases/latest";
pub(crate) const WM_UPDATE: u32 = WM_APP + 3210; // wParam: 0 a release was found, 1 the new exe is ready, 2 that failed
pub(crate) const UPDATE_BLUE: u32 = 0x0A6CD6; // white text reads at 5:1 on it, on both themes' toolbars
const EXE_MAX: usize = 64 << 20;
const NEW_EXE: &str = "grids.new.exe";
const OLD_EXE: &str = "grids.old.exe";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UpRelease {
    pub(crate) ver: String, // "0.7.0"
    pub(crate) zip: String, // download URLs
    pub(crate) sums: String,
}

pub(crate) static UPDATES_ON: AtomicBool = AtomicBool::new(true); // the view menu item, for the thread
static FOUND: Mutex<Option<UpRelease>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false); // a download is running
static FAIL: Mutex<Option<UpFail>> = Mutex::new(None);
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new()); // turned on: look now
static RESTART: OnceLock<PathBuf> = OnceLock::new(); // set once the new exe is in place: started when Grids exits

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum UpFail {
    Download,
    Checksum,
    Damaged,
    NoWrite,
}

impl UpFail {
    fn text(self) -> &'static str {
        match self {
            UpFail::Download => tr("the download failed", "내려받기 실패"),
            UpFail::Checksum => tr("the checksum did not match", "체크섬이 맞지 않음"),
            UpFail::Damaged => tr("the package has no usable grids.exe", "패키지에 쓸 수 있는 grids.exe가 없음"),
            UpFail::NoWrite => tr("Grids' folder can't be written to", "Grids 폴더에 쓸 수 없음"),
        }
    }
}

// ---------- pure parts (tested) ----------

/// "v1.2.3" / "1.2" -> [1, 2, 3] / [1, 2, 0]. Anything else (a pre-release suffix too): None.
pub(crate) fn parse_ver(v: &str) -> Option<[u64; 3]> {
    let parts: Vec<&str> = v.trim().trim_start_matches(['v', 'V']).split('.').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut out = [0; 3];
    for (o, p) in out.iter_mut().zip(&parts) {
        *o = p.parse().ok().filter(|_| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))?;
    }
    Some(out)
}

pub(crate) fn newer(tag: &str, cur: &str) -> bool {
    matches!((parse_ver(tag), parse_ver(cur)), (Some(a), Some(b)) if a > b)
}

// Where the value of "key" starts, past the colon.
fn json_at<'a>(json: &'a str, key: &str, from: usize) -> Option<(usize, &'a str)> {
    let k = format!("\"{key}\"");
    let at = from + json.get(from..)?.find(&k)? + k.len();
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    Some((json.len() - rest.len(), rest))
}

// A string value; JSON escapes that matter in URLs and tags (\/ \" \\) are undone.
fn json_str(rest: &str) -> Option<String> {
    let mut out = String::new();
    let mut it = rest.strip_prefix('"')?.chars();
    loop {
        match it.next()? {
            '"' => return Some(out),
            '\\' => match it.next()? {
                'n' => out.push('\n'),
                c => out.push(c),
            },
            c => out.push(c),
        }
    }
}

/// Every string value of `key`, in order.
pub(crate) fn json_strs(json: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some((at, rest)) = json_at(json, key, from) {
        out.extend(json_str(rest));
        from = at;
    }
    out
}

/// The release in GitHub's latest-release JSON, when it is newer than `cur`, final, and carries both
/// grids-v<ver>-x64.zip and SHA256SUMS.txt (asset names are the last part of their download URLs).
pub(crate) fn pick_release(json: &str, cur: &str) -> Option<UpRelease> {
    let flag = |k: &str| json_at(json, k, 0).is_some_and(|(_, r)| r.starts_with("true"));
    if flag("prerelease") || flag("draft") {
        return None;
    }
    let tag = json_strs(json, "tag_name").into_iter().next()?;
    if !newer(&tag, cur) {
        return None;
    }
    let ver = tag.trim_start_matches(['v', 'V']).to_string();
    let urls = json_strs(json, "browser_download_url");
    let find = |name: &str| urls.iter().find(|u| u.rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case(name))).cloned();
    Some(UpRelease { zip: find(&zip_name(&ver))?, sums: find("SHA256SUMS.txt")?, ver })
}

pub(crate) fn zip_name(ver: &str) -> String {
    format!("grids-v{ver}-x64.zip")
}

/// The SHA-256 (lowercase hex) SHA256SUMS.txt gives for `name` ("<hex>  name", or "<hex> *name" for binary mode).
pub(crate) fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.trim().split_once(char::is_whitespace)?;
        let file = file.trim().trim_start_matches('*').trim_start_matches("./");
        let ok = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && file.eq_ignore_ascii_case(name);
        ok.then(|| hash.to_ascii_lowercase())
    })
}

pub(crate) fn sha256_hex(data: &[u8]) -> Option<String> {
    let mut out = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) }.is_ok().then(|| out.iter().map(|b| format!("{b:02x}")).collect())
}

/// Checks the downloaded zip against SHA256SUMS.txt and takes grids.exe out of it (checked too, when listed).
pub(crate) fn unpack(zip: &[u8], sums: &str, ver: &str) -> std::result::Result<Vec<u8>, UpFail> {
    let want = sum_for(sums, &zip_name(ver)).ok_or(UpFail::Checksum)?;
    if sha256_hex(zip).as_deref() != Some(want.as_str()) {
        return Err(UpFail::Checksum);
    }
    let mut f = Cursor::new(zip);
    let es = zip_entries(&mut f).ok_or(UpFail::Damaged)?;
    let e = zip_find(&es, "grids.exe").ok_or(UpFail::Damaged)?;
    let exe = zip_read(&mut f, e, EXE_MAX).filter(|x| x.len() as u64 == e.size && x.starts_with(b"MZ")).ok_or(UpFail::Damaged)?;
    match sum_for(sums, "grids.exe") {
        Some(w) if sha256_hex(&exe).as_deref() != Some(w.as_str()) => Err(UpFail::Checksum),
        _ => Ok(exe),
    }
}

/// grids.new.exe (already written) takes the running exe's place, which becomes grids.old.exe. On failure the
/// running exe is back where it was and the new one is gone.
pub(crate) fn swap_exe(exe: &Path) -> std::io::Result<()> {
    let (new, old) = (exe.with_file_name(NEW_EXE), exe.with_file_name(OLD_EXE));
    _ = std::fs::remove_file(&old);
    let r = std::fs::rename(exe, &old).and_then(|_| {
        std::fs::rename(&new, exe).inspect_err(|_| {
            _ = std::fs::rename(&old, exe);
        })
    });
    if r.is_err() {
        _ = std::fs::remove_file(&new);
    }
    r
}

// ---------- network (the thread's side) ----------

struct Net(*mut core::ffi::c_void);
impl Drop for Net {
    fn drop(&mut self) {
        unsafe { _ = WinHttpCloseHandle(self.0) };
    }
}
fn net(h: *mut core::ffi::c_void) -> Option<Net> {
    (!h.is_null()).then_some(Net(h))
}

/// GET over WinHTTP (system proxy, redirects followed): the body of a 200 answer, at most `max` bytes.
fn http_get(url: &str, max: usize, receive_s: i32) -> Option<Vec<u8>> {
    unsafe {
        let agent = HSTRING::from(format!("Grids/{}", env!("CARGO_PKG_VERSION")));
        let s = net(WinHttpOpen(&agent, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, PCWSTR::null(), PCWSTR::null(), 0))?;
        WinHttpSetTimeouts(s.0, 10_000, 10_000, 15_000, receive_s * 1000).ok()?;
        let wide: Vec<u16> = url.encode_utf16().collect();
        let mut uc = URL_COMPONENTS {
            dwStructSize: size_of::<URL_COMPONENTS>() as u32,
            dwHostNameLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            ..Default::default()
        };
        WinHttpCrackUrl(&wide, 0, &mut uc).ok()?;
        let part = |p: PWSTR, n: u32| if p.is_null() || n == 0 { String::new() } else { String::from_utf16_lossy(std::slice::from_raw_parts(p.0, n as usize)) };
        let host = part(uc.lpszHostName, uc.dwHostNameLength);
        let path = part(uc.lpszUrlPath, uc.dwUrlPathLength) + &part(uc.lpszExtraInfo, uc.dwExtraInfoLength);
        let c = net(WinHttpConnect(s.0, &HSTRING::from(host), uc.nPort, 0))?;
        let secure = if url.starts_with("https:") { WINHTTP_FLAG_SECURE } else { WINHTTP_OPEN_REQUEST_FLAGS(0) };
        let r = net(WinHttpOpenRequest(c.0, w!("GET"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), std::ptr::null(), secure))?;
        let accept: Vec<u16> = "Accept: application/vnd.github+json, */*".encode_utf16().collect();
        _ = WinHttpAddRequestHeaders(r.0, &accept, WINHTTP_ADDREQ_FLAG_ADD);
        WinHttpSendRequest(r.0, None, None, 0, 0, 0).ok()?;
        WinHttpReceiveResponse(r.0, std::ptr::null_mut()).ok()?;
        let (mut code, mut len) = (0u32, 4u32);
        let flags = WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER;
        WinHttpQueryHeaders(r.0, flags, PCWSTR::null(), Some(&mut code as *mut u32 as _), &mut len, std::ptr::null_mut()).ok()?;
        if code != 200 {
            return None;
        }
        let (mut out, mut buf) = (Vec::new(), vec![0u8; 64 << 10]);
        loop {
            let mut n = 0u32;
            WinHttpReadData(r.0, buf.as_mut_ptr() as _, buf.len() as u32, &mut n).ok()?;
            if n == 0 {
                return Some(out);
            }
            out.extend_from_slice(&buf[..n as usize]);
            if out.len() > max {
                return None;
            }
        }
    }
}

fn check() -> Option<UpRelease> {
    let feed = std::env::var("GRIDS_UPDATE_FEED").unwrap_or_else(|_| FEED.into());
    let json = http_get(&feed, 1 << 20, 15)?;
    pick_release(&String::from_utf8_lossy(&json), env!("CARGO_PKG_VERSION"))
}

fn fetch(r: &UpRelease, exe: &Path) -> std::result::Result<(), UpFail> {
    let zip = http_get(&r.zip, EXE_MAX, 60).ok_or(UpFail::Download)?;
    let sums = http_get(&r.sums, 64 << 10, 15).ok_or(UpFail::Download)?;
    let data = unpack(&zip, &String::from_utf8_lossy(&sums), &r.ver)?;
    let new = exe.with_file_name(NEW_EXE);
    let wrote = std::fs::File::create(&new).and_then(|mut f| std::io::Write::write_all(&mut f, &data).and_then(|_| f.sync_all()));
    wrote.map_err(|_| {
        _ = std::fs::remove_file(&new);
        UpFail::NoWrite
    })
}

// ---------- UI side ----------

/// At startup: the (hidden) button, and the thread that looks for releases.
pub(crate) unsafe fn start_updates(main: HWND) {
    let b = child(main, w!("BUTTON"), w!("Update"), BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0), ID_TB_UPDATE, HFONT::default());
    _ = ShowWindow(b, SW_HIDE);
    add_tip(b, "");
    UPDATES_ON.store(app(|a| a.s.updates), Ordering::Relaxed);
    let fast = std::env::var_os("GRIDS_UPDATE_FEED").is_some();
    let main = main.0 as isize;
    std::thread::spawn(move || {
        // left by the last update; the old process may still be on its way out
        if let Ok(old) = std::env::current_exe().map(|e| e.with_file_name(OLD_EXE)) {
            for _ in 0..40 {
                if !old.exists() || std::fs::remove_file(&old).is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        let mut wait = Duration::from_secs(if fast { 3 } else { 60 });
        loop {
            if let Ok(g) = WAKE.0.lock() {
                if let Ok((mut g, _)) = WAKE.1.wait_timeout_while(g, wait, |woke| !*woke) {
                    *g = false;
                }
            }
            wait = Duration::from_secs(6 * 3600);
            if !UPDATES_ON.load(Ordering::Relaxed) {
                continue;
            }
            if let Some(r) = check() {
                *FOUND.lock().unwrap() = Some(r);
                unsafe { _ = PostMessageW(Some(HWND(main as _)), WM_UPDATE, WPARAM(0), LPARAM(0)) };
            }
        }
    });
}

/// For relayout: the button shows while a newer release is known and checking is on.
pub(crate) fn update_shown() -> bool {
    UPDATES_ON.load(Ordering::Relaxed) && FOUND.lock().is_ok_and(|f| f.is_some())
}

pub(crate) unsafe fn update_tip() {
    let ver = FOUND.lock().ok().and_then(|f| f.as_ref().map(|r| r.ver.clone()));
    if let (Some(v), Ok(b)) = (ver, GetDlgItem(Some(app(|a| a.main)), ID_TB_UPDATE)) {
        let tip = match KO.load(Ordering::Relaxed) {
            true => format!("Grids v{v} 업데이트 (지금 v{})", env!("CARGO_PKG_VERSION")),
            false => format!("Update to Grids v{v} (now v{})", env!("CARGO_PKG_VERSION")),
        };
        tip_text(b, &tip, TTM_UPDATETIPTEXTW);
    }
}

/// View menu: checking on / off. On looks right away; off hides the button and stops all network use.
pub(crate) unsafe fn toggle_updates() {
    let on = app(|a| {
        a.s.updates ^= true;
        a.s.updates
    });
    UPDATES_ON.store(on, Ordering::Relaxed);
    if on {
        if let Ok(mut w) = WAKE.0.lock() {
            *w = true;
            WAKE.1.notify_one();
        }
    }
    relayout();
}

pub(crate) unsafe fn draw_update(dc: HDC, rc: RECT, hot: bool, pressed: bool, font: HFONT) {
    let bg = match (pressed, hot) {
        (true, _) => blend(UPDATE_BLUE, 0x000000, 0.2),
        (_, true) => blend(UPDATE_BLUE, 0x000000, 0.1),
        _ => UPDATE_BLUE,
    };
    fill(dc, rc, bg);
    text(dc, rc, tr("Update", "업데이트"), font, 0xFFFFFF, DT_CENTER);
}

// The button: ask once, then download on a worker; WM_UPDATE comes back with the result.
pub(crate) unsafe fn update_click() {
    let Some(r) = FOUND.lock().ok().and_then(|f| f.clone()) else { return };
    if BUSY.load(Ordering::Relaxed) {
        return;
    }
    let main = app(|a| a.main);
    let ko = KO.load(Ordering::Relaxed);
    let title = HSTRING::from(if ko { format!("Grids v{} 업데이트", r.ver) } else { format!("Grids v{} is available", r.ver) });
    let body = HSTRING::from(match ko {
        true => format!("Grids를 재시작해 v{}로 업데이트합니다. 설정, 열린 폴더, 세션은 그대로 유지됩니다.", r.ver),
        false => format!("Grids will restart to update to v{}. Your settings, open folders and sessions are kept.", r.ver),
    });
    let (restart, later) = (HSTRING::from(tr("Restart", "재시작")), HSTRING::from(tr("Later", "나중에")));
    let buttons = [
        TASKDIALOG_BUTTON { nButtonID: IDOK.0, pszButtonText: PCWSTR(restart.as_ptr()) },
        TASKDIALOG_BUTTON { nButtonID: IDCANCEL.0, pszButtonText: PCWSTR(later.as_ptr()) },
    ];
    let cfg = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: main,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
        pszWindowTitle: w!("Grids"),
        pszMainInstruction: PCWSTR(title.as_ptr()),
        pszContent: PCWSTR(body.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: IDOK.0,
        ..Default::default()
    };
    let mut pressed = 0;
    if TaskDialogIndirect(&cfg, Some(&mut pressed), None, None).is_err() || pressed != IDOK.0 {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return update_failed(UpFail::NoWrite) };
    BUSY.store(true, Ordering::Relaxed);
    set_note(format!("{} v{}…", tr("Downloading Grids", "Grids 내려받는 중"), r.ver));
    let main = main.0 as isize;
    std::thread::spawn(move || {
        let res = fetch(&r, &exe);
        if let Err(f) = res {
            *FAIL.lock().unwrap() = Some(f);
        }
        unsafe { _ = PostMessageW(Some(HWND(main as _)), WM_UPDATE, WPARAM(if res.is_ok() { 1 } else { 2 }), LPARAM(0)) };
    });
}

unsafe fn update_failed(f: UpFail) {
    BUSY.store(false, Ordering::Relaxed);
    app(|a| a.note = None);
    let ver = FOUND.lock().ok().and_then(|x| x.as_ref().map(|r| r.ver.clone())).unwrap_or_default();
    let text = match KO.load(Ordering::Relaxed) {
        true => format!("Grids를 업데이트하지 못했습니다 ({}). 바뀐 것은 없습니다.\n\n릴리스 페이지를 열어 직접 내려받을까요?", f.text()),
        false => format!("Grids could not update itself ({}). Nothing was changed.\n\nOpen the release page to download it yourself?", f.text()),
    };
    if MessageBoxW(Some(app(|a| a.main)), &HSTRING::from(text), w!("Grids"), MB_YESNO | MB_ICONWARNING) == IDYES {
        let url = format!("{HOME}/releases/tag/v{ver}");
        ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

/// WM_UPDATE on the main window.
pub(crate) unsafe fn update_msg(main: HWND, what: usize) {
    match what {
        0 => {
            update_tip();
            relayout();
        }
        1 => {
            let Ok(exe) = std::env::current_exe() else { return update_failed(UpFail::NoWrite) };
            if swap_exe(&exe).is_err() {
                return update_failed(UpFail::NoWrite);
            }
            // closing saves grids.ini and the views as always; the new exe starts once this one is gone
            _ = RESTART.set(exe);
            _ = PostMessageW(Some(main), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        _ => update_failed(FAIL.lock().ok().and_then(|f| *f).unwrap_or(UpFail::Download)),
    }
}

/// End of main(): the window is gone (so the new Grids won't hand its start to this one); start it, no arguments,
/// same working folder: it opens the last state from grids.ini.
pub(crate) fn restart_updated() {
    if let Some(exe) = RESTART.get() {
        let dir = std::env::current_dir().map(|d| HSTRING::from(d.as_os_str())).unwrap_or_default();
        unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(exe.as_os_str()), PCWSTR::null(), &dir, SW_SHOWNORMAL) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_versions_and_feed() {
        assert_eq!((parse_ver("v0.6.5"), parse_ver("1.2"), parse_ver("V10.0.1")), (Some([0, 6, 5]), Some([1, 2, 0]), Some([10, 0, 1])));
        for bad in ["", "v1", "1.2.3.4", "1.2.3-beta", "1.x.3", "1..3", "v 1.2"] {
            assert_eq!(parse_ver(bad), None, "{bad}");
        }
        assert!(newer("v0.6.10", "0.6.9") && newer("v0.7.0", "0.6.5") && newer("v1.0", "0.9.9"));
        assert!(!newer("v0.6.5", "0.6.5") && !newer("v0.6.4", "0.6.5") && !newer("v0.7.0-rc1", "0.6.5") && !newer("junk", "0.6.5"));

        // the shape of GitHub's answer, trimmed
        let json = |tag: &str, pre: &str, names: &[&str]| {
            let assets: Vec<String> = names
                .iter()
                .map(|n| format!(r#"{{"name": "{n}", "size": 1, "browser_download_url": "https:\/\/github.com\/efww\/grids\/releases\/download\/{tag}\/{n}"}}"#))
                .collect();
            format!(r#"{{"html_url": "x", "tag_name": "{tag}", "draft": false, "prerelease": {pre}, "assets": [{}]}}"#, assets.join(", "))
        };
        let ok = json("v0.7.0", "false", &["grids-v0.7.0-x64.zip", "SHA256SUMS.txt"]);
        let r = pick_release(&ok, "0.6.5").unwrap();
        assert_eq!(r.ver, "0.7.0");
        assert_eq!(r.zip, "https://github.com/efww/grids/releases/download/v0.7.0/grids-v0.7.0-x64.zip");
        assert_eq!(r.sums, "https://github.com/efww/grids/releases/download/v0.7.0/SHA256SUMS.txt");
        assert_eq!(pick_release(&ok, "0.7.0"), None); // not newer
        assert_eq!(pick_release(&json("v0.7.0", "true", &["grids-v0.7.0-x64.zip", "SHA256SUMS.txt"]), "0.6.5"), None);
        assert_eq!(pick_release(&json("v0.7.0", "false", &["grids-v0.7.0-x64.zip"]), "0.6.5"), None); // no sums
        assert_eq!(pick_release(&json("v0.7.0", "false", &["grids-v0.6.9-x64.zip", "SHA256SUMS.txt"]), "0.6.5"), None);
        assert_eq!(pick_release("{\"message\": \"API rate limit exceeded\"}", "0.6.5"), None);
        assert_eq!(json_strs(r#"{"a": "x\"y\\z", "a" : "2"}"#, "a"), ["x\"y\\z", "2"]);
    }

    #[test]
    fn update_checksums() {
        let sums = "95FCE1B33E8E46C8CFBCEE540C42BDCDF671F6F7786B976D5CA39531B0F18B4C  grids-v0.6.5-x64.zip\n\
                    6979935f421be76172b5bc23339c07cc52912c03611c1c4e3dcdc797c57095e1 *grids.exe\n";
        assert_eq!(sum_for(sums, "grids-v0.6.5-x64.zip").as_deref(), Some("95fce1b33e8e46c8cfbcee540c42bdcdf671f6f7786b976d5ca39531b0f18b4c"));
        assert_eq!(sum_for(sums, "GRIDS.EXE").as_deref(), Some("6979935f421be76172b5bc23339c07cc52912c03611c1c4e3dcdc797c57095e1"));
        assert_eq!((sum_for(sums, "grids.ex"), sum_for("abc  grids.exe", "grids.exe")), (None, None));
        assert_eq!(sha256_hex(b"abc").as_deref(), Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));

        // a stored zip with one grids.exe: right sums unpack it, a wrong or missing line changes nothing
        let exe = b"MZ fake exe".to_vec();
        let mut z = Vec::new();
        let crc = 0u32; // zip_read doesn't look at it
        z.extend(0x0403_4b50u32.to_le_bytes());
        z.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend(crc.to_le_bytes());
        z.extend((exe.len() as u32).to_le_bytes());
        z.extend((exe.len() as u32).to_le_bytes());
        z.extend([9, 0, 0, 0]);
        z.extend(b"grids.exe");
        z.extend(&exe);
        let cd = z.len() as u32;
        z.extend(0x0201_4b50u32.to_le_bytes());
        z.extend([20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend(crc.to_le_bytes());
        z.extend((exe.len() as u32).to_le_bytes());
        z.extend((exe.len() as u32).to_le_bytes());
        z.extend([9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // name length, then extra, comment, disk, attributes
        z.extend(0u32.to_le_bytes());
        z.extend(b"grids.exe");
        let cd_len = z.len() as u32 - cd;
        z.extend(0x0605_4b50u32.to_le_bytes());
        z.extend([0, 0, 0, 0, 1, 0, 1, 0]);
        z.extend(cd_len.to_le_bytes());
        z.extend(cd.to_le_bytes());
        z.extend([0, 0]);
        let good = format!("{}  {}\n{} *grids.exe\n", sha256_hex(&z).unwrap(), zip_name("9.9.9"), sha256_hex(&exe).unwrap());
        assert_eq!(unpack(&z, &good, "9.9.9"), Ok(exe.clone()));
        assert_eq!(unpack(&z, &good.replace(&sha256_hex(&z).unwrap(), &"0".repeat(64)), "9.9.9"), Err(UpFail::Checksum));
        assert_eq!(unpack(&z, &good.replace(&sha256_hex(&exe).unwrap(), &"1".repeat(64)), "9.9.9"), Err(UpFail::Checksum));
        assert_eq!(unpack(&z, &good, "9.9.8"), Err(UpFail::Checksum)); // no line for that zip
        let junk = b"not a zip".to_vec();
        assert_eq!(unpack(&junk, &format!("{}  {}", sha256_hex(&junk).unwrap(), zip_name("1.0.0")), "1.0.0"), Err(UpFail::Damaged));
    }

    #[test]
    fn update_swap() {
        let dir = std::env::temp_dir().join(format!("grids-update-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("grids.exe");
        let read = |n: &str| std::fs::read_to_string(dir.join(n)).ok();
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(dir.join(NEW_EXE), "new").unwrap();
        std::fs::write(dir.join(OLD_EXE), "older").unwrap();
        swap_exe(&exe).unwrap();
        assert_eq!((read("grids.exe"), read(OLD_EXE), read(NEW_EXE)), (Some("new".into()), Some("old".into()), None));
        // nothing downloaded: the running exe stays where it is
        std::fs::remove_file(dir.join(OLD_EXE)).unwrap();
        assert!(swap_exe(&exe).is_err());
        assert_eq!((read("grids.exe"), read(OLD_EXE)), (Some("new".into()), None));
        _ = std::fs::remove_dir_all(&dir);
    }
}
