use crate::*;
use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows::Win32::System::RestartManager::*;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Controls::{
    TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDCBF_CANCEL_BUTTON, TDCBF_CLOSE_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
    TD_INFORMATION_ICON, TD_WARNING_ICON,
};

// Small shell extras: Shift+F7 new text file, "which program is using this?", Compare with WinMerge (⇄ menu).
// Ported from Samek86/maguro, re-implemented for Grids.

// ⇄ menu commands (menu-local, returned by TrackPopupMenu)
pub(crate) const ID_INUSE: usize = 1130;
pub(crate) const ID_WM_PAIR: usize = 1131;
pub(crate) const ID_WM_PANE: usize = 3100; // + pane index: compare this pane's folder with that one
const BTN_RETRY: i32 = 3140;
const BTN_SWITCH: i32 = 3141;
const BTN_CLOSE: i32 = 3142;
const RADIO: i32 = 3120; // + holder index
const LVM_GETEDITCONTROL: u32 = 0x1018;
const LVM_GETNEXTITEM: u32 = 0x100C;
const LVM_EDITLABELW: u32 = 0x1076;
const LVNI_FOCUSED: isize = 1;

// ---------- new text file ----------

// "New Text Document.txt", then "New Text Document (2).txt", … The number goes before the extension, like Explorer.
pub(crate) fn fresh_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (base, String::new()),
    };
    std::iter::once(base.to_string()).chain((2..10_000).map(|n| format!("{stem} ({n}){ext}"))).find(|n| !taken(n)).unwrap_or_default()
}

// file the view should select and rename once it lists it: (pane, full path, ticks left)
thread_local! {
    static NEW_FILE: RefCell<Option<(usize, String, u8)>> = const { RefCell::new(None) };
}

// Shift+F7: an empty text file in the active pane's folder, selected and in rename, as F7 does for folders.
// create_new: an existing file is never opened, let alone truncated.
pub(crate) unsafe fn new_text_file(i: usize) {
    let dir = app(|a| a.s.paths[i].clone());
    let name = fresh_name(tr("New Text Document.txt", "새 텍스트 문서.txt"), |n| Path::new(&dir).join(n).exists());
    let full = Path::new(&dir).join(&name).to_string_lossy().into_owned();
    if !is_fs(&dir) || name.is_empty() || std::fs::OpenOptions::new().write(true).create_new(true).open(&full).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let wide = HSTRING::from(full.as_str());
    SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    NEW_FILE.with(|n| *n.borrow_mut() = Some((i, full, 40)));
    // the view lists it a moment later: poll on a thread timer (no window of ours needed)
    SetTimer(None, 0, 50, Some(new_file_tick));
}

unsafe extern "system" fn new_file_tick(_: HWND, _: u32, id: usize, _: u32) {
    let Some((i, full, left)) = NEW_FILE.with(|n| n.borrow().clone()) else {
        _ = KillTimer(None, id);
        return;
    };
    if select_rename(i, &full) || left == 0 {
        NEW_FILE.with(|n| *n.borrow_mut() = None);
        _ = KillTimer(None, id);
    } else {
        NEW_FILE.with(|n| *n.borrow_mut() = Some((i, full, left - 1)));
    }
}

// Select the new file, then open the list's own label edit, as F2 does; true once the box is open.
// Polled: the view lists the file only a moment later. (SVSI_EDIT and the "rename" verb select but open no box here.)
unsafe fn select_rename(i: usize, full: &str) -> bool {
    let (Some(sv), Some(fv)) = (view_of::<IShellView>(i), view_of::<IFolderView2>(i)) else { return true };
    let list = view_list(&fv);
    let editing = || !list.0.is_null() && SendMessageW(list, LVM_GETEDITCONTROL, None, None).0 != 0;
    if editing() {
        return true;
    }
    let sel = fv.GetSelection(false).ok().filter(|a| a.GetCount().is_ok_and(|n| n == 1));
    let got = sel.and_then(|a| a.GetItemAt(0).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)));
    if !got.is_some_and(|p| p.eq_ignore_ascii_case(full)) {
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(full), None, &mut pidl, 0, None).is_err() {
            return true; // gone again
        }
        // a DirectUI list has no label edit to send to: there the view's own SVSI_EDIT is all there is
        let edit = if list.0.is_null() { SVSI_EDIT.0 } else { 0 };
        let flags = edit | SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0;
        _ = sv.SelectItem(ILFindLastID(pidl), flags as u32);
        CoTaskMemFree(Some(pidl as _));
        return false;
    }
    if list.0.is_null() {
        return true;
    }
    let k = SendMessageW(list, LVM_GETNEXTITEM, Some(WPARAM(usize::MAX)), Some(LPARAM(LVNI_FOCUSED))).0;
    if k >= 0 {
        _ = SetFocus(Some(list)); // LVM_EDITLABEL wants the focus in the list
        SendMessageW(list, LVM_EDITLABELW, Some(WPARAM(k as usize)), None);
    }
    editing()
}

// ---------- which program is using this? (Restart Manager) ----------

pub(crate) struct Holder {
    pub(crate) proc: RM_UNIQUE_PROCESS,
    pub(crate) name: String,
    pub(crate) kind: RM_APP_TYPE,
}

// Programs holding any of the files open; None when Restart Manager can't tell.
pub(crate) unsafe fn holders(paths: &[String]) -> Option<Vec<Holder>> {
    let (mut sess, mut key) = (0u32, [0u16; CCH_RM_SESSION_KEY as usize + 1]);
    if RmStartSession(&mut sess, None, PWSTR(key.as_mut_ptr())) != ERROR_SUCCESS {
        return None;
    }
    let wide: Vec<HSTRING> = paths.iter().map(|p| HSTRING::from(p.as_str())).collect();
    let names: Vec<PCWSTR> = wide.iter().map(|h| PCWSTR(h.as_ptr())).collect();
    let mut out = None;
    if RmRegisterResources(sess, Some(&names), None, None) == ERROR_SUCCESS {
        let mut info = vec![RM_PROCESS_INFO::default(); 8];
        for _ in 0..4 {
            let (mut need, mut n, mut why) = (0u32, info.len() as u32, 0u32);
            match RmGetList(sess, &mut need, &mut n, Some(info.as_mut_ptr()), &mut why) {
                e if e == ERROR_MORE_DATA => info.resize(need as usize + 4, RM_PROCESS_INFO::default()),
                e if e == ERROR_SUCCESS => {
                    let list = info[..n as usize].iter().map(|p| {
                        let app = String::from_utf16_lossy(&p.strAppName[..p.strAppName.iter().position(|&c| c == 0).unwrap_or(0)]);
                        let exe = exe_name(p.Process.dwProcessId);
                        Holder { proc: p.Process, name: holder_line(&app, &exe, p.Process.dwProcessId), kind: p.ApplicationType }
                    });
                    out = Some(list.collect());
                    break;
                }
                _ => break,
            }
        }
    }
    _ = RmEndSession(sess);
    out
}

unsafe fn exe_name(pid: u32) -> String {
    let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
    let (mut buf, mut n) = ([0u16; 1024], 1024u32);
    let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
    _ = CloseHandle(p);
    let full = if ok { String::from_utf16_lossy(&buf[..n as usize]) } else { String::new() };
    full.rsplit('\\').next().unwrap_or_default().to_string()
}

// "Notepad  (notepad.exe, PID 1234)"
pub(crate) fn holder_line(app: &str, exe: &str, pid: u32) -> String {
    let app = if app.trim().is_empty() { exe } else { app.trim() };
    match exe {
        "" => format!("{app}  (PID {pid})"),
        e => format!("{app}  ({e}, PID {pid})"),
    }
}

// Never Grids itself, Explorer (the desktop and taskbar) or a critical system process.
pub(crate) fn may_close(kind: RM_APP_TYPE, pid: u32, me: u32) -> bool {
    pid != me && kind != RmExplorer && kind != RmCritical
}

// Visible top-level windows of a process, its main one first.
unsafe fn windows_of(pid: u32) -> Vec<HWND> {
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let (pid, out) = &mut *(lp.0 as *mut (u32, Vec<HWND>));
        let mut p = 0;
        GetWindowThreadProcessId(h, Some(&mut p));
        if p == *pid && IsWindowVisible(h).as_bool() && GetWindow(h, GW_OWNER).is_err() {
            out.push(h);
        }
        true.into()
    }
    let mut st = (pid, Vec::new());
    _ = EnumWindows(Some(each), LPARAM(&mut st as *mut _ as isize));
    st.1
}

// TaskDialogIndirect only exists in common controls v6 (our manifest): looked up at run time so a
// binary without the manifest (the unit tests) still loads.
unsafe fn task_dialog(cfg: &TASKDIALOGCONFIG) -> Option<(i32, i32)> {
    type Tdi = unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut BOOL) -> HRESULT;
    let f = GetProcAddress(LoadLibraryW(w!("comctl32.dll")).ok()?, s!("TaskDialogIndirect"))?;
    let (mut btn, mut radio) = (0, 0);
    let f: Tdi = std::mem::transmute(f);
    f(cfg, &mut btn, &mut radio, std::ptr::null_mut()).is_ok().then_some((btn, radio))
}

// "Which program is using this?" for files; also for later callers (say, after a sharing violation).
// Retry checks again, Switch brings the program forward, Close asks it politely to quit (never forced).
// Returns true when, at the end, no program holds the files any more.
pub(crate) unsafe fn who_uses(owner: HWND, paths: &[String]) -> bool {
    let title = HSTRING::from("Grids");
    let what = match paths {
        [one] => one.rsplit('\\').next().unwrap_or(one).to_string(),
        _ => tr("{n} items", "{n}개 항목").replace("{n}", &paths.len().to_string()),
    };
    let mut cfg = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        ..Default::default()
    };
    let head = HSTRING::from(what);
    loop {
        let Some(list) = holders(paths) else {
            MessageBoxW(Some(owner), &HSTRING::from(tr("Windows could not tell which programs use these files.", "어떤 프로그램이 쓰는지 Windows가 알려 주지 않았습니다.")), &title, MB_ICONWARNING);
            return false;
        };
        if list.is_empty() {
            let free = HSTRING::from(tr("No program is using it.", "사용 중인 프로그램이 없습니다."));
            cfg.pszMainInstruction = PCWSTR(head.as_ptr());
            cfg.pszContent = PCWSTR(free.as_ptr());
            cfg.Anonymous1.pszMainIcon = TD_INFORMATION_ICON;
            cfg.dwCommonButtons = TDCBF_CLOSE_BUTTON;
            (cfg.cButtons, cfg.cRadioButtons) = (0, 0);
            if task_dialog(&cfg).is_none() {
                MessageBoxW(Some(owner), &HSTRING::from(format!("{head}\n\n{free}")), &title, MB_ICONINFORMATION);
            }
            return true;
        }
        let several = list.len() > 1;
        let body = match several {
            true => tr("In use by these programs (pick one for Switch / Close):", "사용 중인 프로그램 (전환·닫기할 것을 고르세요):").to_string(),
            false => format!("{}\n{}", tr("In use by:", "사용 중인 프로그램:"), list[0].name),
        };
        let body = HSTRING::from(body);
        let labels = [
            (BTN_RETRY, tr("Retry", "다시 확인")),
            (BTN_SWITCH, tr("Switch to program", "프로그램으로 전환")),
            (BTN_CLOSE, tr("Close program", "프로그램 닫기")),
        ]
        .map(|(id, t)| (id, HSTRING::from(t)));
        let buttons = labels.each_ref().map(|(id, t)| TASKDIALOG_BUTTON { nButtonID: *id, pszButtonText: PCWSTR(t.as_ptr()) });
        // with several programs, a radio button picks the one Switch / Close act on
        let names: Vec<HSTRING> = list.iter().map(|h| HSTRING::from(h.name.as_str())).collect();
        let radios: Vec<TASKDIALOG_BUTTON> = names.iter().enumerate().map(|(k, t)| TASKDIALOG_BUTTON { nButtonID: RADIO + k as i32, pszButtonText: PCWSTR(t.as_ptr()) }).collect();
        cfg.pszMainInstruction = PCWSTR(head.as_ptr());
        cfg.pszContent = PCWSTR(body.as_ptr());
        cfg.Anonymous1.pszMainIcon = TD_WARNING_ICON;
        cfg.dwCommonButtons = TDCBF_CANCEL_BUTTON;
        (cfg.cButtons, cfg.pButtons, cfg.nDefaultButton) = (3, buttons.as_ptr(), BTN_RETRY);
        (cfg.cRadioButtons, cfg.pRadioButtons) = (if several { radios.len() as u32 } else { 0 }, radios.as_ptr());
        let Some((btn, radio)) = task_dialog(&cfg) else {
            let all = list.iter().map(|h| h.name.as_str()).collect::<Vec<_>>().join("\n");
            MessageBoxW(Some(owner), &HSTRING::from(format!("{head}\n\n{body}\n{all}")), &title, MB_ICONWARNING);
            return false;
        };
        let h = &list[((radio - RADIO).max(0) as usize).min(list.len() - 1)];
        let pid = h.proc.dwProcessId;
        match btn {
            BTN_RETRY => {}
            BTN_SWITCH => {
                let Some(&w) = windows_of(pid).first() else {
                    _ = MessageBeep(MB_ICONWARNING); // no window to show (a service, a hidden helper)
                    continue;
                };
                if IsIconic(w).as_bool() {
                    _ = ShowWindow(w, SW_RESTORE);
                }
                _ = SetForegroundWindow(w);
                return false;
            }
            BTN_CLOSE => {
                if !may_close(h.kind, pid, GetCurrentProcessId()) {
                    MessageBoxW(Some(owner), &HSTRING::from(tr("Grids does not close Windows Explorer, system processes or itself.", "Grids는 Windows 탐색기, 시스템 프로세스, 자기 자신은 닫지 않습니다.")), &title, MB_ICONWARNING);
                    continue;
                }
                let ask = HSTRING::from(format!("{}\n\n{}", tr("Ask this program to close? Unsaved work there may prompt you.", "이 프로그램에 닫기를 요청할까요? 저장하지 않은 작업이 있으면 그 프로그램이 물어봅니다."), h.name));
                if MessageBoxW(Some(owner), &ask, &title, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                    continue;
                }
                close_politely(h);
                std::thread::sleep(Duration::from_millis(700)); // give it a moment, then the list is checked again
            }
            _ => return false,
        }
    }
}

// WM_CLOSE to its windows, as clicking × would; without windows, Restart Manager's shutdown request without the force flag.
unsafe fn close_politely(h: &Holder) {
    let wins = windows_of(h.proc.dwProcessId);
    for w in &wins {
        _ = PostMessageW(Some(*w), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    if wins.is_empty() {
        let (mut sess, mut key) = (0u32, [0u16; CCH_RM_SESSION_KEY as usize + 1]);
        if RmStartSession(&mut sess, None, PWSTR(key.as_mut_ptr())) == ERROR_SUCCESS {
            if RmRegisterResources(sess, None, Some(&[h.proc]), None) == ERROR_SUCCESS {
                _ = RmShutdown(sess, 0, None); // 0: polite; RmForceShutdown is never used
            }
            _ = RmEndSession(sess);
        }
    }
}

// ---------- Compare with WinMerge ----------

// Where WinMerge may be, in order: its registry value (per user, per machine, 32-bit machine), then the usual folders.
pub(crate) fn winmerge_candidates(reg: &[Option<String>], roots: &[String]) -> Vec<String> {
    let clean = |s: &str| s.trim().trim_matches('"').trim().to_string();
    let from_reg = reg.iter().flatten().map(|s| clean(s));
    let from_dirs = roots.iter().map(|r| r.trim_end_matches('\\')).filter(|r| !r.is_empty()).map(|r| format!(r"{r}\WinMerge\WinMergeU.exe"));
    let mut out: Vec<String> = Vec::new();
    for c in from_reg.chain(from_dirs).filter(|c| !c.is_empty()) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&c)) {
            out.push(c);
        }
    }
    out
}

// Quoted for CommandLineToArgvW: backslashes before a quote (or the closing quote) are doubled, so C:\ stays C:\.
pub(crate) fn quote_arg(s: &str) -> String {
    let mut out = String::from("\"");
    let mut bs = 0;
    for c in s.chars() {
        if c == '"' {
            out.push_str(&"\\".repeat(bs + 1));
        }
        bs = if c == '\\' { bs + 1 } else { 0 };
        out.push(c);
    }
    out.push_str(&"\\".repeat(bs));
    out.push('"');
    out
}

// /e: Esc closes it, /u: the paths stay out of WinMerge's recent list, /r: whole folder trees.
pub(crate) fn winmerge_args(left: &str, right: &str, folders: bool) -> String {
    format!("/e /u{} {} {}", if folders { " /r" } else { "" }, quote_arg(left), quote_arg(right))
}

unsafe fn reg_sz(root: HKEY, key: PCWSTR) -> Option<String> {
    let (mut buf, mut n) = ([0u16; 1024], 2048u32);
    RegGetValueW(root, key, w!("Executable"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n)).ok().ok()?;
    let s = String::from_utf16_lossy(&buf[..(n as usize / 2).min(1024)]);
    Some(s.trim_end_matches('\0').to_string())
}

// Looked up each time the ⇄ menu opens (cheap), never at startup.
pub(crate) unsafe fn winmerge_exe() -> Option<String> {
    let key = w!(r"Software\Thingamahoochie\WinMerge");
    let reg = [reg_sz(HKEY_CURRENT_USER, key), reg_sz(HKEY_LOCAL_MACHINE, key), reg_sz(HKEY_LOCAL_MACHINE, w!(r"Software\WOW6432Node\Thingamahoochie\WinMerge"))];
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let roots = [env("ProgramFiles"), env("ProgramFiles(x86)"), format!(r"{}\Programs", env("LOCALAPPDATA"))];
    winmerge_candidates(&reg, &roots).into_iter().find(|p| Path::new(p).is_file())
}

unsafe fn run_winmerge(exe: &str, left: &str, right: &str) {
    let folders = Path::new(left).is_dir() && Path::new(right).is_dir();
    let r = ShellExecuteW(None, w!("open"), &HSTRING::from(exe), &HSTRING::from(winmerge_args(left, right, folders)), PCWSTR::null(), SW_SHOWNORMAL);
    if r.0 as isize <= 32 {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

// ---------- ⇄ menu hooks ----------

// Adds "Which program is using this?" and, with WinMerge found, "Compare with WinMerge" to the ⇄ menu.
// others: (pane, menu label, folder it showed). Returns the WinMerge exe for shell_pick.
pub(crate) unsafe fn shell_menu(menu: HMENU, src: usize, picked: &std::result::Result<Vec<String>, Pick>, others: &[(usize, HSTRING, String)]) -> Option<String> {
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let flags = if picked.is_ok() { MF_STRING } else { MF_STRING | MF_GRAYED };
    _ = AppendMenuW(menu, flags, ID_INUSE, &HSTRING::from(tr("Which program is using this?", "어떤 프로그램이 쓰고 있나요?")));
    let exe = winmerge_exe()?;
    match picked {
        Ok(two) if two.len() == 2 => {
            _ = AppendMenuW(menu, MF_STRING, ID_WM_PAIR, &HSTRING::from(tr("Compare the 2 selected items with WinMerge", "선택한 2개를 WinMerge로 비교")));
        }
        _ => {
            let here = is_fs(&app(|a| a.s.paths[src].clone()));
            if let Ok(sub) = CreatePopupMenu() {
                for (k, label, path) in others {
                    let flags = if here && is_fs(path) { MF_STRING } else { MF_STRING | MF_GRAYED };
                    _ = AppendMenuW(sub, flags, ID_WM_PANE + k, label);
                }
                let flags = if others.is_empty() { MF_POPUP | MF_GRAYED } else { MF_POPUP };
                _ = AppendMenuW(menu, flags, sub.0 as usize, &HSTRING::from(tr("Compare this folder with WinMerge", "이 폴더를 WinMerge로 비교")));
            }
        }
    }
    Some(exe)
}

pub(crate) fn is_shell_pick(p: usize) -> bool {
    p == ID_INUSE || p == ID_WM_PAIR || (ID_WM_PANE..ID_WM_PANE + MAXP).contains(&p)
}

pub(crate) unsafe fn shell_pick(h: HWND, src: usize, p: usize, picked: std::result::Result<Vec<String>, Pick>, exe: Option<String>, others: &[(usize, HSTRING, String)]) {
    match (p, picked, exe) {
        (ID_INUSE, Ok(paths), _) => _ = who_uses(h, &paths),
        (ID_WM_PAIR, Ok(two), Some(exe)) if two.len() == 2 => run_winmerge(&exe, &two[0], &two[1]),
        (p, _, Some(exe)) if p >= ID_WM_PANE => {
            // only the folder the menu showed for that pane
            let k = p - ID_WM_PANE;
            let (here, there) = app(|a| (a.s.paths[src].clone(), a.s.paths[k].clone()));
            match others.iter().find(|(i, ..)| *i == k) {
                Some((_, _, shown)) if *shown == there && is_fs(&here) && is_fs(&there) => run_winmerge(&exe, &here, &there),
                _ => _ = MessageBeep(MB_ICONWARNING),
            }
        }
        _ => _ = MessageBeep(MB_ICONWARNING),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_text_file_names() {
        assert_eq!(fresh_name("New Text Document.txt", |_| false), "New Text Document.txt");
        assert_eq!(fresh_name("New Text Document.txt", |n| n == "New Text Document.txt"), "New Text Document (2).txt");
        let taken = ["새 텍스트 문서.txt", "새 텍스트 문서 (2).txt"];
        assert_eq!(fresh_name("새 텍스트 문서.txt", |n| taken.contains(&n)), "새 텍스트 문서 (3).txt");
        assert_eq!(fresh_name("noext", |n| n == "noext"), "noext (2)");
        assert_eq!(fresh_name(".txt", |n| n == ".txt"), ".txt (2)");
        assert_eq!(fresh_name("a.txt", |_| true), ""); // all taken: nothing, never an existing name
    }

    #[test]
    fn in_use_lines_and_close_rules() {
        assert_eq!(holder_line("Notepad", "notepad.exe", 42), "Notepad  (notepad.exe, PID 42)");
        assert_eq!(holder_line("  ", "pwsh.exe", 7), "pwsh.exe  (pwsh.exe, PID 7)");
        assert_eq!(holder_line("Svc", "", 9), "Svc  (PID 9)");
        assert!(may_close(RmMainWindow, 10, 1) && may_close(RmConsole, 10, 1) && may_close(RmService, 10, 1));
        assert!(!may_close(RmExplorer, 10, 1) && !may_close(RmCritical, 10, 1) && !may_close(RmMainWindow, 1, 1));
    }

    #[test]
    fn winmerge_lookup_order() {
        let reg = [None, Some(r#""D:\Tools\WinMerge\WinMergeU.exe""#.to_string()), Some(r"c:\program files\winmerge\winmergeu.exe".into())];
        let roots = [r"C:\Program Files".to_string(), r"C:\Program Files (x86)\".into(), String::new()];
        assert_eq!(
            winmerge_candidates(&reg, &roots),
            [r"D:\Tools\WinMerge\WinMergeU.exe", r"c:\program files\winmerge\winmergeu.exe", r"C:\Program Files (x86)\WinMerge\WinMergeU.exe"]
        );
        assert!(winmerge_candidates(&[None, None], &[]).is_empty());
    }

    #[test]
    fn winmerge_command_line() {
        assert_eq!(quote_arg(r"C:\a b\x.txt"), r#""C:\a b\x.txt""#);
        assert_eq!(quote_arg(r"C:\"), r#""C:\\""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(winmerge_args(r"C:\", r"D:\x y", true), r#"/e /u /r "C:\\" "D:\x y""#);
        assert_eq!(winmerge_args("a.txt", "b.txt", false), r#"/e /u "a.txt" "b.txt""#);
        assert!(is_shell_pick(ID_INUSE) && is_shell_pick(ID_WM_PANE + 15) && !is_shell_pick(ID_WM_PANE + 16) && !is_shell_pick(301));
    }
}
