// Registry, PATH and the disk. Called when a compare runs or Settings paints, not at startup.

use std::sync::Mutex;

use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};
use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW};
use windows::Win32::UI::Shell::ShellExecuteW;

use crate::imports::*;
use crate::tr;

use super::plan::*;

static CACHE: Mutex<Option<(String, Where)>> = Mutex::new(None);

fn cache() -> std::sync::MutexGuard<'static, Option<(String, Where)>> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn forget_status() {
    *cache() = None;
}

/// Cached for the custom path currently stored. A different path looks again.
pub(crate) fn status_now(custom: &str) -> Where {
    if let Some((key, found)) = cache().as_ref() {
        if key == custom {
            return found.clone();
        }
    }
    let found = find(custom);
    *cache() = Some((custom.to_string(), found.clone()));
    found
}

pub(crate) fn find(custom: &str) -> Where {
    let pf = std::env::var("ProgramFiles").unwrap_or_default();
    let pfx = std::env::var("ProgramFiles(x86)").unwrap_or_default();
    let path = std::env::var("PATH").unwrap_or_default();
    let cands = install_candidates(&registry_exes(), &app_path_exes(), &pf, &pfx, &path);
    resolve(custom, &cands, |p| Path::new(p).is_file())
}

/// `(headline, detail)`. Detail is the exe that will run, the missing custom path, or the download page.
pub(crate) fn status_text(found: &Where) -> (String, String) {
    match found {
        Where::Custom(p) => (tr(crate::Key::CustomPath).to_string(), p.clone()),
        Where::CustomMissing { detected: Some(p), .. } => (
            tr(crate::Key::CustomPathWasNotFoundUsingDetected)
            .to_string(),
            p.clone(),
        ),
        Where::CustomMissing { custom, detected: None } => (
            tr(crate::Key::CustomPathWasNotFound).to_string(),
            custom.clone(),
        ),
        Where::Detected(p) => (tr(crate::Key::Detected).to_string(), p.clone()),
        Where::NotInstalled => (tr(crate::Key::NotInstalled).to_string(), DOWNLOAD.to_string()),
    }
}

pub(crate) unsafe fn launch(exe: &str, params: &str) -> bool {
    let file = HSTRING::from(exe);
    let args = HSTRING::from(params);
    let r = ShellExecuteW(None, w!("open"), &file, &args, PCWSTR::null(), SW_SHOWNORMAL);
    r.0 as isize > 32
}

pub(crate) unsafe fn open_download(owner: HWND) {
    let url = HSTRING::from(DOWNLOAD);
    let owner = (!owner.0.is_null()).then_some(owner);
    let _ = ShellExecuteW(owner, w!("open"), &url, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
}

pub(crate) unsafe fn prompt_missing(owner: HWND) {
    let text = tr(crate::Key::WinmergeIsNotInstalledSoFoldersAnd);
    if crate::ask_confirm(owner, crate::Confirm::ok(tr(crate::Key::VendorWinmerge), text)) {
        open_download(owner);
    }
}

pub(crate) unsafe fn prompt_failed(owner: HWND) {
    let text = tr(crate::Key::WinmergeDidNotStart);
    let _ = ask(owner, text, MB_OK | MB_ICONWARNING);
}

pub(crate) unsafe fn prompt_need_folder(owner: HWND) {
    let text = tr(crate::Key::ThisPaneIsNotFolderOpenFolder);
    let _ = ask(owner, text, MB_OK | MB_ICONINFORMATION);
}

pub(crate) unsafe fn prompt_need_two(owner: HWND) {
    let text = tr(crate::Key::CompareNeedsTwoFoldersOpenAnotherPane);
    let _ = ask(owner, text, MB_OK | MB_ICONINFORMATION);
}

pub(crate) unsafe fn prompt_need_file(owner: HWND) {
    let text = tr(crate::Key::SelectOneFileThenCompareItWith);
    let _ = ask(owner, text, MB_OK | MB_ICONINFORMATION);
}

pub(crate) unsafe fn prompt_need_two_files(owner: HWND) {
    let text = tr(crate::Key::SelectTwoFilesCompareThemInWinmerge);
    let _ = ask(owner, text, MB_OK | MB_ICONINFORMATION);
}

unsafe fn ask(owner: HWND, text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let owner = (!owner.0.is_null()).then_some(owner);
    MessageBoxW(owner, &HSTRING::from(text), &HSTRING::from(tr(crate::Key::VendorWinmerge)), flags)
}

pub(crate) unsafe fn pick_exe(owner: HWND, current: &str) -> Option<String> {
    let mut buf = [0u16; 520];
    let start = as_exe(current);
    let seed = if Path::new(&start).is_file() { start } else { current.trim().trim_matches('"').to_string() };
    for (d, c) in buf.iter_mut().zip(seed.encode_utf16()) {
        *d = c;
    }
    let filter = exe_filter();
    let title = HSTRING::from(tr(crate::Key::VendorWinmerge));
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
        ..Default::default()
    };
    GetOpenFileNameW(&mut ofn).as_bool().then(|| {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    })
}

fn exe_filter() -> Vec<u16> {
    let mut s = String::new();
    for (label, spec) in [(tr(crate::Key::VendorWinmerge), "WinMergeU.exe"), (tr(crate::Key::KindPrograms), "*.exe")] {
        s.push_str(label);
        s.push('\0');
        s.push_str(spec);
        s.push('\0');
    }
    s.push('\0');
    s.encode_utf16().collect()
}

fn registry_exes() -> Vec<String> {
    let mut out = Vec::new();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for sub in [REG_SUB, r"Software\WOW6432Node\Thingamahoochie\WinMerge"] {
            let name: Vec<u16> = REG_VALUE.encode_utf16().chain(std::iter::once(0)).collect();
            let v = reg_sz(root, sub, PCWSTR(name.as_ptr()));
            if let Some(p) = clean_exe(&v) {
                push_unique_raw(&mut out, &p);
            }
        }
    }
    out
}

fn app_path_exes() -> Vec<String> {
    let mut out = Vec::new();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for sub in [APP_PATHS, r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\WinMergeU.exe"] {
            let v = reg_sz(root, sub, PCWSTR::null());
            if let Some(p) = clean_exe(&v) {
                push_unique_raw(&mut out, &p);
            }
        }
    }
    out
}

fn push_unique_raw(out: &mut Vec<String>, path: &str) {
    if path.is_empty() || out.iter().any(|e| e.eq_ignore_ascii_case(path)) {
        return;
    }
    out.push(path.to_string());
}

fn reg_sz(key: HKEY, sub: &str, value: PCWSTR) -> String {
    let mut buf = [0u16; 1024];
    let mut n = (buf.len() * 2) as u32;
    let wide: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
    let err = unsafe { RegGetValueW(key, PCWSTR(wide.as_ptr()), value, RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n)) };
    if err != ERROR_SUCCESS || n < 2 {
        return String::new();
    }
    let chars = (n as usize / 2).saturating_sub(1).min(buf.len());
    String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').to_string()
}
