// "Open read-only" on the shell menu for Excel workbooks and CSV.
//
// The row is Maguro's, like Fast paste: it is not a shell verb and it is not a
// settings switch. Excel is found with a read-only App Paths lookup when the
// row is chosen. Nothing here writes the registry or the settings file.

use std::cell::RefCell;

use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};

use crate::imports::*;
use crate::tr;

/// Extensions that open in Excel. Compared without regard to case.
pub(crate) const EXTS: &[&str] = &["xls", "xlsx", "xlsm", "csv"];

/// `HKLM` and `HKCU`, native and `WOW6432Node`. The default value is the exe.
pub(crate) const APP_PATHS: &[&str] = &[
    r"Software\Microsoft\Windows\CurrentVersion\App Paths\excel.exe",
    r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths\excel.exe",
];

thread_local! {
    static PATHS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Remember the files for this popup. A background click or a mixed selection stores nothing.
pub(crate) fn arm(background: bool, items: &[(String, bool)]) {
    let paths = if background { Vec::new() } else { selection_paths(items) };
    PATHS.with(|p| *p.borrow_mut() = paths);
}

pub(crate) fn clear() {
    PATHS.with(|p| p.borrow_mut().clear());
}

pub(crate) fn armed() -> bool {
    PATHS.with(|p| !p.borrow().is_empty())
}

/// Every selected item is one of `EXTS`. Anything else, including a folder, hides the row.
pub(crate) fn selection_paths(items: &[(String, bool)]) -> Vec<String> {
    if items.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(items.len());
    for (path, file) in items {
        if !*file || !is_excel_file(path) {
            return Vec::new();
        }
        out.push(path.clone());
    }
    out
}

pub(crate) fn is_excel_file(path: &str) -> bool {
    let path = path.trim().trim_end_matches(['\\', '/']);
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    !ext.is_empty() && EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e))
}

/// `excel.exe /r "path"`. `/r` opens that workbook read-only.
pub(crate) fn readonly_args(path: &str) -> String {
    format!("/r \"{path}\"")
}

/// Unquote a registry path. A trailing `,0` icon index is dropped.
pub(crate) fn clean_exe(raw: &str) -> Option<String> {
    let s = raw.trim().trim_matches('\0');
    if s.is_empty() {
        return None;
    }
    let s = if let Some(rest) = s.strip_prefix('"') { rest.split('"').next().unwrap_or(rest) } else { s };
    let s = s.split(',').next().unwrap_or(s).trim().trim_matches('"');
    (!s.is_empty()).then(|| s.to_string())
}

/// First absolute `excel.exe` that exists. Later duplicates are ignored.
pub(crate) fn pick_excel(candidates: &[String], exists: impl Fn(&str) -> bool) -> Option<String> {
    let mut seen = Vec::new();
    for raw in candidates {
        let Some(path) = clean_exe(raw) else { continue };
        if !is_absolute(&path) || !is_excel_exe(&path) {
            continue;
        }
        if seen.iter().any(|e: &String| e.eq_ignore_ascii_case(&path)) {
            continue;
        }
        seen.push(path.clone());
        if exists(&path) {
            return Some(path);
        }
    }
    None
}

fn is_excel_exe(path: &str) -> bool {
    path.rsplit(['\\', '/']).next().unwrap_or(path).eq_ignore_ascii_case("excel.exe")
}

fn is_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || path.starts_with(r"\\")
}

/// Open each armed file read-only. A missing Excel shows one message and does not launch anything.
pub(crate) unsafe fn open_readonly(owner: HWND) {
    let paths = PATHS.with(|p| p.borrow().clone());
    if paths.is_empty() {
        return;
    }
    let Some(exe) = find_excel() else {
        tell(owner, crate::Key::ExcelIsNotInstalled, MB_ICONINFORMATION);
        return;
    };
    let mut failed = false;
    for path in &paths {
        if !launch(&exe, &readonly_args(path)) {
            failed = true;
        }
    }
    if failed {
        tell(owner, crate::Key::ExcelDidNotStart, MB_ICONWARNING);
    }
}

/// Read-only App Paths lookup. `RegGetValueW` does not create or change a key.
fn find_excel() -> Option<String> {
    let mut raw = Vec::new();
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for sub in APP_PATHS {
            let v = reg_default(root, sub);
            if !v.is_empty() {
                raw.push(v);
            }
        }
    }
    pick_excel(&raw, |p| Path::new(p).is_file())
}

fn reg_default(root: HKEY, sub: &str) -> String {
    let mut buf = [0u16; 1024];
    let mut n = (buf.len() * 2) as u32;
    let wide: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
    // The default value of `...\App Paths\excel.exe`. Read types only.
    let err = unsafe { RegGetValueW(root, PCWSTR(wide.as_ptr()), PCWSTR::null(), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n)) };
    if err != ERROR_SUCCESS || n < 2 {
        return String::new();
    }
    let chars = (n as usize / 2).saturating_sub(1).min(buf.len());
    String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').trim().to_string()
}

unsafe fn launch(exe: &str, params: &str) -> bool {
    let file = HSTRING::from(exe);
    let args = HSTRING::from(params);
    let r = ShellExecuteW(None, w!("open"), &file, &args, PCWSTR::null(), SW_SHOWNORMAL);
    r.0 as isize > 32
}

unsafe fn tell(owner: HWND, key: crate::Key, icon: MESSAGEBOX_STYLE) {
    let owner = (!owner.0.is_null()).then_some(owner);
    let _ = MessageBoxW(owner, &HSTRING::from(tr(key)), w!("Maguro"), MB_OK | icon);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_all_excel_selection_is_armed() {
        let all = vec![
            (r"C:\work\a.xlsx".to_string(), true),
            (r"C:\work\b.XLS".to_string(), true),
            (r"D:\c.xlsm".to_string(), true),
            (r"E:\sheet.CSV".to_string(), true),
        ];
        assert_eq!(selection_paths(&all).len(), 4);
        assert!(selection_paths(&[]).is_empty());
        assert!(selection_paths(&[(r"C:\a.xlsx".into(), true), (r"C:\a.txt".into(), true)]).is_empty());
        assert!(selection_paths(&[(r"C:\a.xlsx".into(), true), (r"C:\dir".into(), false)]).is_empty());
        assert!(selection_paths(&[(r"C:\a.xlsx".into(), false)]).is_empty());
        assert!(!is_excel_file(r"C:\a\file.xlsx.txt"));
        assert!(!is_excel_file(r"C:\a\file.xlsb"));
        assert!(!is_excel_file(r"C:\a\file"));
        assert!(is_excel_file(r"C:\a\file.XlSx"));
    }

    #[test]
    fn the_command_is_excel_read_only_switch() {
        assert_eq!(readonly_args(r"C:\Work\Book.xlsx"), r#"/r "C:\Work\Book.xlsx""#);
        assert_eq!(readonly_args(r"D:\a b\c.csv"), r#"/r "D:\a b\c.csv""#);
    }

    #[test]
    fn app_paths_name_excel_and_the_first_real_exe_wins() {
        assert!(APP_PATHS.iter().all(|p| p.ends_with(r"App Paths\excel.exe")));
        assert!(APP_PATHS.iter().any(|p| p.contains("WOW6432Node")));
        assert!(APP_PATHS.iter().any(|p| !p.contains("WOW6432Node")));

        let quoted = pick_excel(&[r#""D:\Office\EXCEL.EXE""#.into(), r"D:\Office\EXCEL.EXE".into()], |p| p.eq_ignore_ascii_case(r"D:\Office\EXCEL.EXE"));
        assert_eq!(quoted.as_deref(), Some(r"D:\Office\EXCEL.EXE"));

        let skipped = pick_excel(&[r"C:\missing\EXCEL.EXE".into(), r"C:\Windows\notepad.exe".into(), r"C:\Office\EXCEL.EXE".into()], |p| p.ends_with("EXCEL.EXE") && p.contains("Office"));
        assert_eq!(skipped.as_deref(), Some(r"C:\Office\EXCEL.EXE"));

        assert!(pick_excel(&["excel.exe".into()], |_| true).is_none());
        assert!(pick_excel(&[r"C:\gone\EXCEL.EXE".into()], |_| false).is_none());
        assert_eq!(clean_exe(r#""C:\Office\EXCEL.EXE",0"#).as_deref(), Some(r"C:\Office\EXCEL.EXE"));
    }

    #[test]
    fn menu_label_matches_korean_japanese_and_english() {
        let prev = crate::i18n::UI.load(std::sync::atomic::Ordering::Relaxed);
        crate::i18n::UI.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(tr(crate::Key::OpenReadOnly), "Open read-only");
        crate::i18n::UI.store(1, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(tr(crate::Key::OpenReadOnly), "읽기 모드로 열기");
        crate::i18n::UI.store(2, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(tr(crate::Key::OpenReadOnly), "読み取り専用で開く");
        crate::i18n::UI.store(prev, std::sync::atomic::Ordering::Relaxed);
    }
}
