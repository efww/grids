use crate::*;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteTreeW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ, RRF_RT_REG_SZ,
};
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

// file the view should select and rename once it lists it: (pane, full path, ticks left, folder and view
// the pane must still show with the focus in its list, else the rename is dropped: set for Ctrl+F7 only)
type NewFile = (usize, String, u8, Option<(String, usize)>);
thread_local! {
    static NEW_FILE: RefCell<Option<NewFile>> = const { RefCell::new(None) };
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
    rename_soon((i, full, 40, None));
}

// the view lists it a moment later: poll on a thread timer (no window of ours needed)
unsafe fn rename_soon(f: NewFile) {
    NEW_FILE.with(|n| *n.borrow_mut() = Some(f));
    SetTimer(None, 0, 50, Some(new_file_tick));
}

unsafe extern "system" fn new_file_tick(_: HWND, _: u32, id: usize, _: u32) {
    let Some((i, full, left, guard)) = NEW_FILE.with(|n| n.borrow().clone()) else {
        _ = KillTimer(None, id);
        return;
    };
    let gone = guard.as_ref().is_some_and(|(dir, view)| !rename_ok(i, dir) || view_id(i) != *view);
    if gone || select_rename(i, &full) || left == 0 {
        NEW_FILE.with(|n| *n.borrow_mut() = None);
        _ = KillTimer(None, id);
    } else {
        NEW_FILE.with(|n| *n.borrow_mut() = Some((i, full, left - 1, guard)));
    }
}

// ---------- Ctrl+F7: the selected items into a new folder ----------

// The new folder's name: one item gives its name without the extension; several give the start of the
// name they all share, once trailing ' ', '-', '_', '.' and '(' are cut, if 3 characters or more are left.
// Otherwise `fallback`.
pub(crate) fn group_base(names: &[&str], fallback: &str) -> String {
    let cut = |s: &str| s.trim_end_matches([' ', '-', '_', '.', '(']).to_string();
    let base = match names {
        [] => String::new(),
        [one] => cut(match one.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem,
            _ => one,
        }),
        [first, rest @ ..] => {
            let n = rest.iter().map(|r| first.chars().zip(r.chars()).take_while(|(a, b)| a == b).count()).min().unwrap_or(0);
            Some(cut(&first.chars().take(n).collect::<String>())).filter(|b| b.chars().count() >= 3).unwrap_or_default()
        }
    };
    if base.trim().is_empty() { fallback.to_string() } else { base }
}

// "Name", then "Name (2)", "Name (3)" … A folder: the number goes at the end, not before a '.'.
pub(crate) fn group_name(base: &str, k: u32) -> String {
    if k < 2 { base.to_string() } else { format!("{base} ({k})") }
}

// Only a whole selection that sits right in the pane's own (file system) folder.
pub(crate) fn group_ok(srcs: &[String], dir: &str) -> bool {
    is_fs(dir) && paste_mode(srcs, dir) == Some(true)
}

pub(crate) const ID_GROUP: usize = 1133;

// a finished grouping, for the UI thread (WM_GROUPED)
struct Grouped {
    pane: usize,
    dir: String,
    new: String,
    n: usize,
    view: usize, // the pane's view when it started: another one (navigated away and back) gets no rename
}

static GROUPED: Mutex<Vec<Grouped>> = Mutex::new(Vec::new());

// a grouping is under way: a second Ctrl+F7 or ⇄ click meanwhile only beeps (it would race the first one
// for the same items into a "Name (2)")
static GROUPING: AtomicBool = AtomicBool::new(false);

struct GroupingDone;
impl Drop for GroupingDone {
    fn drop(&mut self) {
        GROUPING.store(false, Ordering::SeqCst);
    }
}

fn base_of(p: &str) -> &str {
    p.trim_end_matches('\\').rsplit('\\').next().unwrap_or(p)
}

unsafe fn view_id(i: usize) -> usize {
    view_of::<IShellView>(i).map_or(0, |v| v.as_raw() as usize)
}

// The pane still shows `dir` and nothing else is being typed into: no focus in this thread (another
// program, or the move's own progress window, is in front), the focus in that pane's file list, or on
// one of its top bar buttons (the ⇄ button keeps it after its menu).
unsafe fn rename_ok(i: usize, dir: &str) -> bool {
    let pane = app(|a| a.panes.get(i)?.as_ref().filter(|_| i < a.s.panes() && same_dir(&a.s.paths[i], dir)).map(|p| p.hwnd));
    let (Some(pane), Some(sv)) = (pane, view_of::<IShellView>(i)) else { return false };
    let f = GetFocus();
    if f.0.is_null() || in_view_list(sv.GetWindow().unwrap_or_default(), f) {
        return true;
    }
    let mut c = [0u16; 16];
    let n = GetClassNameW(f, &mut c) as usize;
    IsChild(pane, f).as_bool() && String::from_utf16_lossy(&c[..n]).eq_ignore_ascii_case("Button")
}

// Ctrl+F7 in a file list: the pane's whole selection, as the ⇄ menu counts it.
pub(crate) unsafe fn group_key(i: usize) {
    let arr = view_of::<IFolderView2>(i).and_then(|fv| fv.GetSelection(false).ok());
    let count = arr.as_ref().and_then(|a| a.GetCount().ok()).unwrap_or(0);
    let picked = collect_all(count, |k| unsafe { arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)) });
    group_into_folder(i, picked);
}

// Makes the folder and moves the items on a worker thread (one undoable shell move, never replacing
// anything); the rename starts only after every item is in. Ctrl+F7 and the ⇄ menu both end up here.
pub(crate) unsafe fn group_into_folder(i: usize, picked: std::result::Result<Vec<String>, Pick>) {
    let (dir, main) = app(|a| (a.s.paths[i].clone(), a.main.0 as usize));
    let Some(srcs) = picked.ok().filter(|s| group_ok(s, &dir)) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if GROUPING.swap(true, Ordering::SeqCst) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let done = GroupingDone; // released when the worker ends, or right away if it can't start
    let names: Vec<&str> = srcs.iter().map(|s| base_of(s)).collect();
    let base = group_base(&names, tr("New folder", "새 폴더"));
    let view = view_id(i);
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let _done = done;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        if let Some(new) = group_work(&srcs, &dir, &base) {
            GROUPED.lock().unwrap_or_else(|e| e.into_inner()).push(Grouped { pane: i, dir, new, n: srcs.len(), view });
            _ = PostMessageW(Some(HWND(main as _)), WM_GROUPED, WPARAM(0), LPARAM(0));
        }
        if com {
            CoUninitialize();
        }
    });
    match job {
        Ok(job) => keep_job(job),
        Err(_) => _ = MessageBeep(MB_ICONWARNING),
    }
}

// Worker thread. The new folder's path once every item is in it; None after a failure or a cancel, with
// the folder removed again if it is still empty, and said where things are if only some got moved.
unsafe fn group_work(srcs: &[String], dir: &str, base: &str) -> Option<String> {
    let warn = |text: String| _ = MessageBoxW(None, &HSTRING::from(text), w!("Grids"), MB_ICONWARNING);
    let mut new = None;
    for k in 1..=999 {
        let p = Path::new(dir).join(group_name(base, k));
        match std::fs::create_dir(&p) {
            Ok(()) => {
                new = Some(p.to_string_lossy().into_owned());
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                warn(format!("{}{e}", tr("Couldn't make the new folder: ", "새 폴더를 만들지 못했습니다: ")));
                return None;
            }
        }
    }
    let Some(new) = new else {
        _ = MessageBeep(MB_ICONWARNING);
        return None;
    };
    let wide = HSTRING::from(new.as_str());
    SHChangeNotify(SHCNE_MKDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    // FOF_RENAMEONCOLLISION: whatever turns up in the new folder meanwhile is never replaced or merged
    let op = run_op(srcs, &new, true, true);
    let aborted = op.as_ref().map_or(true, |op| op.GetAnyOperationsAborted().map_or(true, |b| b.as_bool()));
    // what actually sits in the new folder now (made empty just above), not just what left the old one
    let moved = std::fs::read_dir(&new).map_or(0, |d| d.count()).min(srcs.len());
    let left = srcs.iter().any(|s| std::fs::symlink_metadata(s).is_ok());
    if !aborted && !left && moved == srcs.len() {
        return Some(new);
    }
    if let Err(e) = &op {
        if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
            warn(format!("{}{}", tr("Moving failed: ", "이동하지 못했습니다: "), e.message()));
        }
    }
    // remove_dir only takes an empty folder: anything that did get moved stays put
    if std::fs::remove_dir(&new).is_err() && moved > 0 {
        let text = tr("{m} of {n} items were moved into\n{new}\n\nThe rest are still in\n{dir}", "{n}개 중 {m}개만 아래 폴더로 옮겨졌습니다.\n{new}\n\n나머지는 그대로 있습니다.\n{dir}");
        warn(text.replace("{m}", &moved.to_string()).replace("{n}", &srcs.len().to_string()).replace("{new}", &new).replace("{dir}", dir));
    }
    None
}

// WM_GROUPED on the main window: says so on the status line, then selects the new folder and opens its
// rename, as F7 does, only while that pane still shows the same view of that folder and nothing else
// has the keyboard.
pub(crate) unsafe fn grouped_msg() {
    let done: Vec<Grouped> = std::mem::take(&mut *GROUPED.lock().unwrap_or_else(|e| e.into_inner()));
    let Some(g) = done.into_iter().last() else { return };
    let note = tr("Moved {n} items into “{name}”", "{n}개를 ‘{name}’ 폴더로 묶음");
    set_note(note.replace("{n}", &g.n.to_string()).replace("{name}", base_of(&g.new)));
    if rename_ok(g.pane, &g.dir) && view_id(g.pane) == g.view {
        rename_soon((g.pane, g.new, 40, Some((g.dir, g.view))));
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
pub(crate) unsafe fn task_dialog(cfg: &TASKDIALOGCONFIG) -> Option<(i32, i32)> {
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

// ---------- "Open in Grids" in Explorer's right-click menu ----------
// Per user only (HKCU\Software\Classes): no admin rights. The registry is the only record, nothing in grids.ini.

const CTX_BASE: &str = r"Software\Classes";
// fixed names: only these three verbs are ever written or deleted
const CTX_KEYS: [&str; 3] = [r"Directory\shell\Grids", r"Directory\Background\shell\Grids", r"Drive\shell\Grids"];
// value on Grids' own keys: a "Grids" verb without it belongs to something else and is left alone
const CTX_MARK: &str = "GridsMenu";

/// The menu's command: folder mode (never a session), and `\.` after %V so a drive root "D:\"
/// does not end in `\"` and eat its closing quote. menu_args takes the `\.` off again.
pub(crate) fn ctx_command(exe: &str) -> String {
    format!(r#""{exe}" {FOLDER_FLAG} "%V\.""#)
}

/// True = the registered command runs this exe (paths compared without case).
pub(crate) fn ctx_owned(cmd: &str, exe: &str) -> bool {
    cmd.trim().to_lowercase() == ctx_command(exe).to_lowercase()
}

/// Arguments of a folder-mode launch lose the `\.` the menu command adds: "C:\x\." → "C:\x", "D:\\." → "D:\".
pub(crate) fn menu_args(mut args: Vec<String>) -> Vec<String> {
    if args.first().is_some_and(|a| a == FOLDER_FLAG) {
        for a in args.iter_mut().skip(1) {
            if let Some(p) = a.strip_suffix(r"\.") {
                *a = if p.len() == 2 && p.ends_with(':') { format!(r"{p}\") } else { p.to_string() };
            }
        }
    }
    args
}

unsafe fn reg_text(key: &str, name: Option<&str>) -> Option<String> {
    let (mut buf, mut n) = ([0u16; 1024], 2048u32);
    let name = name.map(HSTRING::from);
    let v = name.as_ref().map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr()));
    RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(key), v, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n)).ok().ok()?;
    Some(String::from_utf16_lossy(&buf[..(n as usize / 2).min(1024)]).trim_end_matches('\0').to_string())
}

unsafe fn reg_put(key: &str, name: Option<&str>, text: &str) -> bool {
    let data: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let name = name.map(HSTRING::from);
    let v = name.as_ref().map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr()));
    RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), v, REG_SZ.0, Some(data.as_ptr() as _), (data.len() * 2) as u32).is_ok()
}

unsafe fn reg_has(key: &str) -> bool {
    let mut k = HKEY::default();
    let found = RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key), None, KEY_READ, &mut k).is_ok();
    if found {
        _ = RegCloseKey(k);
    }
    found
}

// absent, or carrying Grids' mark
unsafe fn ctx_free(key: &str) -> bool {
    !reg_has(key) || reg_text(key, Some(CTX_MARK)).is_some()
}

// on = all three carry Grids' mark and run this exe (same test ctx_remove uses, so "on" can always be removed)
unsafe fn ctx_on_at(base: &str, exe: &str) -> bool {
    CTX_KEYS.iter().all(|k| {
        let key = format!(r"{base}\{k}");
        reg_text(&key, Some(CTX_MARK)).is_some() && reg_text(&format!(r"{key}\command"), None).is_some_and(|c| ctx_owned(&c, exe))
    })
}

unsafe fn ctx_write(base: &str, exe: &str, label: &str) -> bool {
    CTX_KEYS.iter().all(|k| {
        let key = format!(r"{base}\{k}");
        // the mark first, so a half-written key is still Grids' to clean up
        reg_put(&key, Some(CTX_MARK), "1")
            && reg_put(&key, None, label)
            && reg_put(&key, Some("Icon"), &format!(r#""{exe}",0"#))
            && reg_put(&format!(r"{key}\command"), None, &ctx_command(exe))
    })
}

// deletes Grids' own keys (a foreign "Grids" verb stays); true = none of Grids' is left
unsafe fn ctx_remove(base: &str) -> bool {
    CTX_KEYS.iter().fold(true, |ok, k| {
        let key = format!(r"{base}\{k}");
        let gone = !reg_has(&key) || reg_text(&key, Some(CTX_MARK)).is_none() || RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key.as_str())).is_ok();
        ok && gone
    })
}

// All three or none: a failed half is undone (on: removed again; off: written back for this exe).
unsafe fn set_ctx_at(base: &str, exe: &str, label: &str, on: bool) -> bool {
    if on {
        if !CTX_KEYS.iter().all(|k| ctx_free(&format!(r"{base}\{k}"))) {
            return false; // another program's "Grids" verb: nothing touched
        }
        if ctx_write(base, exe, label) {
            return true;
        }
        ctx_remove(base);
        false
    } else {
        if ctx_remove(base) {
            return true;
        }
        ctx_write(base, exe, label);
        false
    }
}

/// Checked in the View menu only when the menu runs this exe: after moving Grids, one click registers the new path.
pub(crate) unsafe fn ctx_menu_on() -> bool {
    this_exe().is_some_and(|e| ctx_on_at(CTX_BASE, &e))
}

/// Adds or removes the three verbs (in the UI language) and tells Explorer. False = failed, nothing half-done.
pub(crate) unsafe fn set_ctx_menu(on: bool) -> bool {
    let Some(exe) = this_exe() else { return false };
    let ok = set_ctx_at(CTX_BASE, &exe, tr("Open in Grids", "Grids에서 열기"), on);
    SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    ok
}

// View menu click: flips what is really registered, with a note either way.
pub(crate) unsafe fn toggle_ctx_menu() {
    let on = !ctx_menu_on();
    let ok = set_ctx_menu(on);
    if !ok {
        _ = MessageBeep(MB_ICONWARNING);
    }
    let text = match (ok, on) {
        (true, true) => tr(
            "\"Open in Grids\" added to Explorer's right-click menu (Windows 11: under Show more options)",
            "탐색기 오른쪽 클릭 메뉴에 \"Grids에서 열기\"를 넣음 (Windows 11: '더 많은 옵션 표시' 안)",
        ),
        (true, false) => tr("\"Open in Grids\" removed from Explorer's right-click menu", "탐색기 오른쪽 클릭 메뉴에서 \"Grids에서 열기\"를 뺌"),
        (false, true) => tr("Could not add \"Open in Grids\" to Explorer's menu", "탐색기 메뉴에 \"Grids에서 열기\"를 등록하지 못함"),
        (false, false) => tr("Could not remove \"Open in Grids\" from Explorer's menu", "탐색기 메뉴에서 \"Grids에서 열기\"를 빼지 못함"),
    };
    app(|a| a.note = Some((text.into(), Instant::now() + Duration::from_secs(3))));
    update_status();
}

// ---------- ⇄ menu hooks ----------

// Adds "Which program is using this?" and, with WinMerge found, "Compare with WinMerge" to the ⇄ menu.
// others: (pane, menu label, folder it showed). Returns the WinMerge exe for shell_pick.
// no_dirs: none of the selected items is a folder (checksum, up to HASH_MAX, and date folders).
pub(crate) unsafe fn shell_menu(menu: HMENU, src: usize, picked: &std::result::Result<Vec<String>, Pick>, no_dirs: bool, others: &[(usize, HSTRING, String)]) -> Option<String> {
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let flags = if picked.is_ok() { MF_STRING } else { MF_STRING | MF_GRAYED };
    _ = AppendMenuW(menu, flags, ID_INUSE, &HSTRING::from(tr2(M_INUSE)));
    // SHA-256: up to HASH_MAX picked files, no folders; while one runs, the same place stops it
    let (flags, label) = match hash_busy() {
        Some(pct) => (MF_STRING, tr2(M_HASH_STOP).replace("{p}", &pct.to_string())),
        None => {
            let ok = no_dirs && picked.as_ref().is_ok_and(|p| p.len() <= HASH_MAX);
            (if ok { MF_STRING } else { MF_STRING | MF_GRAYED },tr2(M_HASH).to_string())
        }
    };
    _ = AppendMenuW(menu, flags, ID_HASH, &HSTRING::from(label));
    // before the WinMerge lookup: it ends this function early on a PC without WinMerge
    let here = app(|a| a.s.paths[src].clone());
    org_menu(menu, org_ok(picked, no_dirs, &here));
    let label = match picked {
        Ok(p) if group_ok(p, &here) => {
            let t = tr2(M_GROUP_N);
            (MF_STRING, t.replace("{n}", &p.len().to_string()))
        }
        _ => (MF_STRING | MF_GRAYED, tr2(M_GROUP).to_string()),
    };
    _ = AppendMenuW(menu, label.0, ID_GROUP, &HSTRING::from(label.1));
    empty_menu(menu, is_fs(&here)); // whatever is selected
    let exe = winmerge_exe()?;
    match picked {
        Ok(two) if two.len() == 2 => {
            _ = AppendMenuW(menu, MF_STRING, ID_WM_PAIR, &HSTRING::from(tr2(M_WM_PAIR)));
        }
        _ => {
            let here = is_fs(&app(|a| a.s.paths[src].clone()));
            if let Ok(sub) = CreatePopupMenu() {
                for (k, label, path) in others {
                    let flags = if here && is_fs(path) { MF_STRING } else { MF_STRING | MF_GRAYED };
                    _ = AppendMenuW(sub, flags, ID_WM_PANE + k, label);
                }
                let flags = if others.is_empty() { MF_POPUP | MF_GRAYED } else { MF_POPUP };
                _ = AppendMenuW(menu, flags, sub.0 as usize, &HSTRING::from(tr2(M_WM_PANE)));
            }
        }
    }
    Some(exe)
}

pub(crate) fn is_shell_pick(p: usize) -> bool {
    p == ID_INUSE || p == ID_WM_PAIR || p == ID_HASH || p == ID_GROUP || p == ID_EMPTY || is_org_pick(p) || (ID_WM_PANE..ID_WM_PANE + MAXP).contains(&p)
}

pub(crate) unsafe fn shell_pick(h: HWND, src: usize, p: usize, picked: std::result::Result<Vec<String>, Pick>, exe: Option<String>, others: &[(usize, HSTRING, String)]) {
    match (p, picked, exe) {
        (ID_INUSE, Ok(paths), _) => _ = who_uses(h, &paths),
        (ID_HASH, picked, _) => hash_pick(h, picked),
        // the selection the menu counted, checked again against the folder the pane shows now
        (ID_GROUP, picked, _) => group_into_folder(src, picked),
        (p, picked, _) if is_org_pick(p) => organize(src, p - ID_ORG, picked),
        // no selection needed: the folder the pane shows now, checked again there
        (ID_EMPTY, _, _) => remove_empty(src),
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

    // A whole command line split by Windows' own CommandLineToArgvW; the first token is the program name.
    fn split(cmd: &str) -> Vec<String> {
        unsafe {
            let mut n = 0;
            let v = CommandLineToArgvW(&HSTRING::from(cmd), &mut n);
            let out: Vec<String> = (0..n as usize).map(|k| (*v.add(k)).to_string().unwrap()).collect();
            _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(v as _)));
            out
        }
    }

    // The one quote_arg (jump list and WinMerge) read back by the real Windows rules, not a look-alike.
    #[test]
    fn quote_arg_matches_windows() {
        // "x " in front: the program name has its own rules, and an empty line returns this exe's path
        let one = |p: &str| split(&format!("x {}", quote_arg(p))).into_iter().skip(1).collect::<Vec<_>>();
        for p in [r"D:\", r"C:\a b\", r"\\srv\s", r"E:\x\\", r#"a"b"#, r#"C:\q\"x"#, "", r"C:\한글 폴더", r"C:\x\\\"] {
            assert_eq!(one(p), [p], "{p}");
        }
        assert_eq!(split(&format!("x {}", winmerge_args(r"D:\", r"C:\a b\", true)))[1..], ["/e", "/u", "/r", r"D:\", r"C:\a b\"]);
    }

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
    fn group_folder_names() {
        let nf = "New folder";
        // one item: its name without the extension
        assert_eq!(group_base(&["Report 2026.docx"], nf), "Report 2026");
        assert_eq!(group_base(&["photos"], nf), "photos");
        assert_eq!(group_base(&["archive.tar.gz"], nf), "archive.tar");
        // several: the shared start, trailing separators cut
        assert_eq!(group_base(&["Trip - day 1.jpg", "Trip - day 2.jpg", "Trip - map.png"], nf), "Trip");
        assert_eq!(group_base(&["IMG_2031.jpg", "IMG_2045.jpg"], nf), "IMG_20");
        assert_eq!(group_base(&["invoice (1).pdf", "invoice (2).pdf"], nf), "invoice");
        // too short a start, or none: the fallback
        assert_eq!(group_base(&["ab1.txt", "ab2.txt"], nf), nf);
        assert_eq!(group_base(&["a.txt", "b.txt"], nf), nf);
        assert_eq!(group_base(&["--_.x", "--_.y"], nf), nf);
        assert_eq!(group_base(&[], nf), nf);
        // Korean names: compared by character, not byte
        assert_eq!(group_base(&["회의록 1월.hwp", "회의록 2월.hwp"], "새 폴더"), "회의록");
        assert_eq!(group_base(&["사진1.jpg", "사진2.jpg"], "새 폴더"), "새 폴더");
        // a taken name gets a number at the end, never before a '.'
        assert_eq!(group_name("Trip", 1), "Trip");
        assert_eq!(group_name("Trip", 2), "Trip (2)");
        assert_eq!(group_name("archive.tar", 3), "archive.tar (3)");
        // only items that sit right in the pane's folder
        let srcs = [r"C:\w\a.txt".to_string(), r"C:\w\b.txt".into()];
        assert!(group_ok(&srcs, r"C:\w") && group_ok(&srcs, r"c:\W\"));
        assert!(!group_ok(&srcs, r"C:\x") && !group_ok(&[r"C:\w\a.txt".into(), r"C:\x\b.txt".into()], r"C:\w") && !group_ok(&[], r"C:\w"));
        assert!(is_shell_pick(ID_GROUP));
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
    fn explorer_menu_command() {
        let exe = r"C:\Program Files\My Tools\grids.exe";
        assert_eq!(ctx_command(exe), r#""C:\Program Files\My Tools\grids.exe" --folder "%V\.""#);
        assert!(ctx_owned(&ctx_command(exe), exe));
        assert!(ctx_owned(&format!("  {}  ", ctx_command(&exe.to_uppercase())), exe)); // case and stray spaces
        assert!(!ctx_owned(&ctx_command(r"D:\old\grids.exe"), exe)); // moved exe: not checked, one click re-registers
        assert!(!ctx_owned(r#""C:\Program Files\My Tools\grids.exe" "%V""#, exe));
        // what Explorer passes after %V is filled in, split by Windows' own rules, then cleaned and resolved
        let run = |v: &str| menu_args(split(&ctx_command(exe).replace("%V", v)).into_iter().skip(1).collect());
        assert_eq!(run(r"D:\"), [FOLDER_FLAG, r"D:\"]);
        assert_eq!(run(r"C:\Users\Public"), [FOLDER_FLAG, r"C:\Users\Public"]);
        assert_eq!(run(r"\\srv\share\a b"), [FOLDER_FLAG, r"\\srv\share\a b"]);
        assert_eq!(resolve(run(r"C:\Users\Public")[1].clone()), r"C:\Users\Public");
        assert_eq!(resolve(run(r"C:\")[1].clone()), r"C:\");
        // only folder mode is touched
        assert_eq!(menu_args(vec![r"C:\x\.".into()]), [r"C:\x\."]);
    }

    #[test]
    fn explorer_menu_registry_round_trip() {
        // a scratch branch of HKCU, never the real Software\Classes
        let (root, base) = (r"Software\GridsTest", r"Software\GridsTest\Classes");
        let exe = r"C:\Tools Dir\grids.exe";
        unsafe {
            _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(root));
            assert!(!ctx_on_at(base, exe));
            assert!(set_ctx_at(base, exe, "Open in Grids", true));
            assert!(ctx_on_at(base, exe) && !ctx_on_at(base, r"D:\other\grids.exe"));
            for k in CTX_KEYS {
                let key = format!(r"{base}\{k}");
                assert_eq!(reg_text(&key, None).as_deref(), Some("Open in Grids"));
                assert_eq!(reg_text(&key, Some("Icon")).as_deref(), Some(r#""C:\Tools Dir\grids.exe",0"#));
                assert_eq!(reg_text(&format!(r"{key}\command"), None), Some(ctx_command(exe)));
            }
            // moved exe: the marked keys are rewritten in place
            assert!(set_ctx_at(base, r"D:\new\grids.exe", "Grids에서 열기", true));
            assert!(ctx_on_at(base, r"D:\new\grids.exe") && !ctx_on_at(base, exe));
            assert!(set_ctx_at(base, exe, "x", false));
            assert!(CTX_KEYS.iter().all(|k| !reg_has(&format!(r"{base}\{k}"))));
            assert!(reg_has(&format!(r"{base}\Directory\shell"))); // only the Grids verbs go
            // someone else's "Grids" verb (no mark): neither taken over nor deleted
            let foreign = format!(r"{base}\{}", CTX_KEYS[2]);
            assert!(reg_put(&foreign, None, "theirs"));
            assert!(!set_ctx_at(base, exe, "Open in Grids", true));
            assert!(!reg_has(&format!(r"{base}\{}", CTX_KEYS[0])));
            assert!(set_ctx_at(base, exe, "x", false));
            assert_eq!(reg_text(&foreign, None).as_deref(), Some("theirs"));
            // same command but no mark: not "on" (off could not delete it either)
            for k in CTX_KEYS {
                assert!(reg_put(&format!(r"{base}\{k}\command"), None, &ctx_command(exe)));
            }
            assert!(!ctx_on_at(base, exe));
            _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(root));
        }
    }

    #[test]
    fn winmerge_command_line() {
        assert_eq!(quote_arg(r"C:\a b\x.txt"), r#""C:\a b\x.txt""#);
        assert_eq!(quote_arg(r"C:\"), r#""C:\\""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(winmerge_args(r"C:\", r"D:\x y", true), r#"/e /u /r "C:\\" "D:\x y""#);
        assert_eq!(winmerge_args("a.txt", "b.txt", false), r#"/e /u "a.txt" "b.txt""#);
        assert!(is_shell_pick(ID_INUSE) && is_shell_pick(ID_HASH) && is_shell_pick(ID_WM_PANE + 15) && !is_shell_pick(ID_WM_PANE + 16) && !is_shell_pick(301));
    }
}
