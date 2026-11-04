// "Open in Terminal (Administrator)", directly under the shell's Open in Terminal row.
//
// The row is Maguro's. It is not a settings switch. It is shown only when that
// shell row is on the menu, and it copies that row's enabled state. The launch
// is ShellExecuteExW with the runas verb: wt.exe -d <folder> when wt.exe (or
// WindowsTerminal.exe) can be found, otherwise the inbox command-window-here
// command. Cancelling the UAC prompt returns ERROR_CANCELLED and shows nothing.

use std::cell::RefCell;

use windows::Win32::Storage::FileSystem::SearchPathW;

use crate::imports::*;

/// Above the shell id range, next to the other Maguro context commands.
pub(crate) const ID_CTX_TERM: u32 = 0x8008;

thread_local! {
    static FOLDER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Remember the folder this popup would open. `None` grays the row.
pub(crate) fn arm(folder: Option<String>) {
    FOLDER.with(|f| *f.borrow_mut() = folder);
}

pub(crate) fn folder() -> Option<String> {
    FOLDER.with(|f| f.borrow().clone())
}

pub(crate) struct ClearTerm;
impl Drop for ClearTerm {
    fn drop(&mut self) {
        arm(None);
    }
}

/// What ShellExecuteExW is given. `mask` is `fMask` (no error UI, wait for UAC).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AdminTerm {
    pub file: String,
    pub verb: &'static str,
    pub parameters: String,
    pub directory: String,
    pub mask: u32,
}

/// One menu row, in order, separators omitted. `text` is the raw item label.
#[derive(Clone, Debug)]
pub(crate) struct TermSnap {
    pub index: u32,
    pub verb: String,
    pub text: String,
    pub enabled: bool,
}

/// Where the admin row is inserted, and whether it is enabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TermAnchor {
    pub at: u32,
    pub enabled: bool,
}

/// `host` is wt.exe or WindowsTerminal.exe when one was found. `None` is the
/// inbox command window (`cmd.exe /s /k pushd`).
pub(crate) fn admin_term(folder: &str, host: Option<&str>) -> Option<AdminTerm> {
    let directory = normalize_dir(folder)?;
    let host = host.map(str::trim).filter(|s| !s.is_empty());
    let (file, parameters) = match host {
        Some(exe) => (exe.to_string(), format!("-d {}", quote_arg(&directory))),
        None => ("cmd.exe".to_string(), format!("/s /k pushd {}", quote_arg(&directory))),
    };
    Some(AdminTerm {
        file,
        verb: "runas",
        parameters,
        directory,
        mask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
    })
}

/// UAC cancel. Any other ShellExecuteEx failure is a real error.
pub(crate) fn uac_cancelled_code(code: HRESULT) -> bool {
    code == ERROR_CANCELLED.to_hresult()
}

/// Folder the existing Open in Terminal item starts in.
/// Background: the folder on display. A folder: itself. A file: its parent.
pub(crate) fn terminal_folder(background: bool, shown: &str, items: &[(String, bool)]) -> Option<String> {
    let raw = if background {
        shown.to_string()
    } else {
        let (path, file) = items.first()?;
        if *file { parent_dir(path)? } else { path.clone() }
    };
    normalize_dir(&raw)
}

/// Directly under the shell's Open in Terminal row. A preview or canary row
/// loses to the stable title when both are present. The admin label is not a hit.
pub(crate) fn terminal_anchor(rows: &[TermSnap]) -> Option<TermAnchor> {
    let mut best: Option<(u8, u32, bool)> = None;
    for row in rows {
        let label = normalize_title(&row.text);
        if !terminal_verb(&row.verb) && !terminal_label(&label) {
            continue;
        }
        let rank = if preview_title(&label) { 1 } else { 0 };
        let replace = match best {
            None => true,
            Some((rank_have, index, _)) => rank < rank_have || (rank == rank_have && row.index < index),
        };
        if replace {
            best = Some((rank, row.index, row.enabled));
        }
    }
    best.map(|(_, index, enabled)| TermAnchor { at: index.saturating_add(1), enabled })
}

/// Delete a previous row, then insert directly under Open in Terminal.
pub(crate) unsafe fn insert(menu: HMENU, verb_of: impl Fn(u32) -> String) {
    if let Some(i) = super::fav::menu_index(menu, ID_CTX_TERM as usize) {
        let _ = DeleteMenu(menu, i, MF_BYPOSITION);
    }
    let Some(anchor) = terminal_anchor(&snaps(menu, &verb_of)) else { return };
    let enabled = anchor.enabled && folder().is_some();
    let flags = if enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
    let label = HSTRING::from(crate::tr(crate::Key::OpenInTerminalAdministrator));
    let _ = InsertMenuW(menu, anchor.at, flags | MF_BYPOSITION, ID_CTX_TERM as usize, &label);
}

pub(crate) unsafe fn launch(owner: HWND, folder: &str) {
    let host = find_host();
    let Some(spec) = admin_term(folder, host.as_deref()) else { return };
    let file = HSTRING::from(spec.file.as_str());
    let verb = HSTRING::from(spec.verb);
    let params = HSTRING::from(spec.parameters.as_str());
    let dir = HSTRING::from(spec.directory.as_str());
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: spec.mask,
        hwnd: owner,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        lpDirectory: PCWSTR(dir.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    if let Err(e) = ShellExecuteExW(&mut sei) {
        if !uac_cancelled_code(e.code()) {
            let _ = MessageBeep(MB_ICONWARNING);
        }
    }
}

fn find_host() -> Option<String> {
    search_exe("wt.exe").or_else(|| windowsapps_wt()).or_else(|| search_exe("WindowsTerminal.exe"))
}

fn search_exe(name: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let file = HSTRING::from(name);
    let n = unsafe { SearchPathW(PCWSTR::null(), PCWSTR(file.as_ptr()), PCWSTR::null(), Some(&mut buf), None) };
    if n == 0 || n as usize >= buf.len() {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..n as usize]);
    (!s.is_empty()).then_some(s)
}

fn windowsapps_wt() -> Option<String> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let p = std::path::Path::new(&local).join("Microsoft").join("WindowsApps").join("wt.exe");
    p.is_file().then(|| p.to_string_lossy().into_owned())
}

unsafe fn snaps(menu: HMENU, verb_of: &impl Fn(u32) -> String) -> Vec<TermSnap> {
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    let mut out = Vec::new();
    for i in 0..n {
        if super::fav::menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        if id == ID_CTX_TERM {
            continue;
        }
        let verb = if id == u32::MAX { String::new() } else { verb_of(id) };
        out.push(TermSnap {
            index: i,
            verb,
            text: super::ctxmenu::item_text(menu, i),
            enabled: row_enabled(menu, i),
        });
    }
    out
}

unsafe fn row_enabled(menu: HMENU, pos: u32) -> bool {
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STATE,
        ..Default::default()
    };
    if GetMenuItemInfoW(menu, pos, true, &mut info).is_err() {
        return false;
    }
    info.fState.0 & MFS_DISABLED.0 == 0
}

fn normalize_dir(path: &str) -> Option<String> {
    let mut s = path.trim().trim_matches('"').replace('/', "\\");
    if s.is_empty() {
        return None;
    }
    while s.len() > 3 && (s.ends_with('\\') || s.ends_with('/')) {
        s.pop();
    }
    crate::is_fs(&s).then_some(s)
}

fn parent_dir(path: &str) -> Option<String> {
    let path = path.trim().trim_end_matches(['\\', '/']);
    let i = path.rfind(['\\', '/'])?;
    let parent = path.get(..i).unwrap_or("");
    if parent.is_empty() {
        return None;
    }
    if parent.len() == 2 && parent.as_bytes().get(1) == Some(&b':') {
        return Some(format!("{parent}\\"));
    }
    Some(parent.to_string())
}

/// Quote one command-line argument the way Windows Terminal quotes `-d`.
fn quote_arg(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
            out.push(c);
            continue;
        }
        if c == '"' || c == ';' {
            out.extend(std::iter::repeat('\\').take(backslashes + 1));
        }
        backslashes = 0;
        out.push(c);
    }
    out.extend(std::iter::repeat('\\').take(backslashes));
    out.push('"');
    out
}

fn normalize_title(raw: &str) -> String {
    let head = raw.split('\t').next().unwrap_or(raw);
    strip_single_accel(&super::ctxmenu::fold_menu_label(head))
}

fn strip_single_accel(s: &str) -> String {
    let t = s.trim();
    let Some(open) = t.rfind('(') else { return t.to_string() };
    if !t.ends_with(')') || open == 0 {
        return t.to_string();
    }
    let inner = t[open + 1..t.len() - 1].trim();
    let one = inner.chars().count() == 1 && inner.chars().next().is_some_and(|c| c.is_ascii_alphanumeric());
    if !one {
        return t.to_string();
    }
    t[..open].trim().to_string()
}

fn terminal_verb(verb: &str) -> bool {
    matches!(
        verb.trim().to_ascii_lowercase().as_str(),
        "windowsterminalopenhere" | "openterminalhere" | "windowsterminal" | "wt"
    )
}

fn terminal_label(label: &str) -> bool {
    const TITLES: &[&str] = &[
        "open in terminal",
        "open in windows terminal",
        "open windows terminal here",
        "open in terminal preview",
        "open in terminal canary",
        "ターミナルで開く",
        "ターミナル プレビューで開く",
        "터미널에서 열기",
        "터미널 미리 보기에서 열기",
        "in terminal öffnen",
        "in terminal vorschau öffnen",
        "ouvrir dans le terminal",
        "ouvrir dans la préversion du terminal",
    ];
    TITLES.contains(&label)
}

fn preview_title(label: &str) -> bool {
    label.contains("preview")
        || label.contains("canary")
        || label.contains("vorschau")
        || label.contains("préversion")
        || label.contains("미리 보기")
        || label.contains("プレビュー")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_terminal_shell_execute_uses_runas_wt_and_folder() {
        let spec = admin_term(r"C:\work\proj", Some("wt.exe")).expect("folder");
        assert_eq!(spec.file, "wt.exe");
        assert_eq!(spec.verb, "runas");
        assert_eq!(spec.directory, r"C:\work\proj");
        assert_eq!(spec.parameters, r#"-d "C:\work\proj""#);
        assert_eq!(spec.mask, SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC);

        let fallback = admin_term(r"C:\work\proj", Some(r"C:\Program Files\WindowsApps\WindowsTerminal.exe")).unwrap();
        assert_eq!(fallback.file, r"C:\Program Files\WindowsApps\WindowsTerminal.exe");
        assert_eq!(fallback.verb, "runas");
        assert_eq!(fallback.directory, r"C:\work\proj");
        assert_eq!(fallback.parameters, r#"-d "C:\work\proj""#);
        assert_eq!(fallback.mask, spec.mask);

        let cmd = admin_term(r"D:\docs", None).unwrap();
        assert_eq!(cmd.file, "cmd.exe");
        assert_eq!(cmd.verb, "runas");
        assert_eq!(cmd.directory, r"D:\docs");
        assert_eq!(cmd.parameters, r#"/s /k pushd "D:\docs""#);

        let root = admin_term(r"C:\", Some("wt.exe")).unwrap();
        assert_eq!(root.directory, r"C:\");
        assert_eq!(root.parameters, r#"-d "C:\\""#);
        assert!(admin_term("This PC", Some("wt.exe")).is_none());
        assert_eq!(admin_term(r"D:\docs", Some("  ")).unwrap().file, "cmd.exe");
    }

    #[test]
    fn uac_cancel_is_silent_and_other_errors_are_not() {
        assert!(uac_cancelled_code(ERROR_CANCELLED.to_hresult()));
        assert!(!uac_cancelled_code(ERROR_FILE_NOT_FOUND.to_hresult()));
    }

    #[test]
    fn anchor_sits_under_open_in_terminal_and_mirrors_enabled() {
        let rows = vec![
            snap(0, "open", "Open", true),
            snap(1, "", "Open in &Terminal", true),
            snap(2, "properties", "Properties", true),
        ];
        let hit = terminal_anchor(&rows).unwrap();
        assert_eq!(hit.at, 2);
        assert!(hit.enabled);

        let mut gray = rows.clone();
        gray[1].enabled = false;
        assert!(!terminal_anchor(&gray).unwrap().enabled);

        let ja = vec![snap(3, "WindowsTerminalOpenHere", "ターミナルで開く(&T)", true)];
        assert_eq!(terminal_anchor(&ja).unwrap().at, 4);

        let both = vec![
            snap(1, "", "Open in Terminal Preview", true),
            snap(4, "", "Open in Terminal", true),
        ];
        assert_eq!(terminal_anchor(&both).unwrap().at, 5);

        let ours = vec![snap(0, "", "Open in Terminal (Administrator)", true)];
        assert!(terminal_anchor(&ours).is_none());
        assert!(terminal_anchor(&[]).is_none());
    }

    #[test]
    fn target_folder_follows_the_click() {
        assert_eq!(terminal_folder(true, r"D:\work\", &[]).as_deref(), Some(r"D:\work"));
        assert_eq!(terminal_folder(false, "", &[(r"C:\a".into(), false)]).as_deref(), Some(r"C:\a"));
        assert_eq!(terminal_folder(false, "", &[(r"C:\a\b.txt".into(), true)]).as_deref(), Some(r"C:\a"));
        assert_eq!(terminal_folder(false, "", &[(r"C:\notes.txt".into(), true)]).as_deref(), Some(r"C:\"));
        assert_eq!(
            terminal_folder(false, "", &[(r"\\server\share\a.txt".into(), true)]).as_deref(),
            Some(r"\\server\share")
        );
        assert!(terminal_folder(true, "This PC", &[]).is_none());
        assert!(terminal_folder(false, "", &[]).is_none());
    }

    fn snap(index: u32, verb: &str, text: &str, enabled: bool) -> TermSnap {
        TermSnap { index, verb: verb.into(), text: text.into(), enabled }
    }
}
