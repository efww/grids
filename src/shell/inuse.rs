// Which program has a file open, via Restart Manager (RmStartSession / RmRegisterResources /
// RmGetList / RmEndSession). A sharing or lock failure offers Retry, Switch to program,
// Close program, and Cancel. Close asks the program to exit. It never uses RmForceShutdown,
// TerminateProcess, or another process's file handle.

use std::sync::Arc;

use windows::Win32::System::RestartManager::{
    RmEndSession, RmGetList, RmRegisterResources, RmShutdown, RmStartSession, CCH_RM_SESSION_KEY, RM_APP_TYPE, RM_PROCESS_INFO, RM_UNIQUE_PROCESS, RmConsole,
    RmCritical, RmExplorer, RmForceShutdown, RmMainWindow, RmOtherWindow, RmRebootReasonPermissionDenied, RmService, RmUnknownApp,
};
use windows::Win32::System::Threading::AttachThreadInput;
use windows::Win32::UI::Controls::{
    TaskDialogIndirect, PFTASKDIALOGCALLBACK, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_BUTTON, TASKDIALOG_NOTIFICATIONS, TD_WARNING_ICON, TDF_ALLOW_DIALOG_CANCELLATION,
    TDF_POSITION_RELATIVE_TO_WINDOW, TDF_SIZE_TO_CONTENT, TDN_BUTTON_CLICKED, TDN_RADIO_BUTTON_CLICKED,
};

use crate::imports::*;
use crate::i18n::*;

const ID_SWITCH: i32 = 1101;
const ID_CLOSE_APP: i32 = 1102;
const ID_DISMISS: i32 = 1103;
const RADIO_BASE: i32 = 2100;
const MAX_SHOWN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpKind {
    Delete,
    Rename,
    Move,
    Copy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HolderKind {
    Window,
    OtherWindow,
    Service,
    Explorer,
    Console,
    Critical,
    Unknown,
}

#[derive(Clone, Debug)]
pub(crate) struct Holder {
    pub(crate) app_name: String,
    pub(crate) process_name: String,
    pub(crate) pid: u32,
    start: FILETIME,
    pub(crate) kind: HolderKind,
}

#[derive(Clone, Debug)]
pub(crate) enum Lookup {
    Holders { holders: Vec<Holder>, partial: bool },
    /// `partial` means Restart Manager was not allowed to list every process. The dialog
    /// still uses the "nothing found" sentence, which already names a service or an administrator.
    None {
        #[allow(dead_code)]
        partial: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosePlan {
    NeedsConfirm,
    Blocked,
    PostMessage,
    RestartManager,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Offer {
    pub(crate) retry: bool,
    pub(crate) switch_to: bool,
    pub(crate) close_program: bool,
    pub(crate) dismiss: bool,
}

pub(crate) struct Attempt {
    pub(crate) result: Result<()>,
    pub(crate) locked: Vec<String>,
}

/// `RmShutdown` flags. Zero is a polite request. `RmForceShutdown` is never set.
pub(crate) fn shutdown_flags() -> u32 {
    0
}

pub(crate) fn is_cancel(e: &Error) -> bool {
    let code = e.code();
    code == COPYENGINE_E_USER_CANCELLED || code == ERROR_CANCELLED.to_hresult()
}

pub(crate) fn is_in_use_code(code: HRESULT) -> bool {
    code == ERROR_SHARING_VIOLATION.to_hresult()
        || code == ERROR_LOCK_VIOLATION.to_hresult()
        || code == COPYENGINE_E_SHARING_VIOLATION_SRC
        || code == COPYENGINE_E_SHARING_VIOLATION_DEST
}

/// Robocopy and `std::fs` lines. Win32 32 is sharing, 33 is lock.
pub(crate) fn text_is_in_use(text: &str) -> bool {
    let u = text.to_ascii_uppercase();
    u.contains("ERROR 32")
        || u.contains("ERROR 33")
        || u.contains("0X00000020")
        || u.contains("0X00000021")
        || u.contains("SHARING VIOLATION")
        || u.contains("LOCK VIOLATION")
}

pub(crate) fn message_says_in_use(msg: &str) -> bool {
    let l = msg.to_ascii_lowercase();
    l.contains("being used by another process")
        || l.contains("used by another process")
        || l.contains("sharing violation")
        || l.contains("lock violation")
        || msg.contains("다른 프로세스")
        || msg.contains("別のプロセス")
}

pub(crate) fn should_prompt(code: HRESULT, message: &str, locked: &[String]) -> bool {
    !locked.is_empty() || is_in_use_code(code) || message_says_in_use(message)
}

pub(crate) fn offer(failed_op: bool, holders: usize) -> Offer {
    Offer { retry: failed_op, switch_to: holders > 0, close_program: holders > 0, dismiss: true }
}

/// Close is refused for this process, Explorer, and critical processes.
/// Anything else needs a Yes on the confirm dialog before a window message or Restart Manager.
pub(crate) fn plan_close(confirmed: bool, self_pid: bool, kind: HolderKind, windows: usize) -> ClosePlan {
    if self_pid || matches!(kind, HolderKind::Explorer | HolderKind::Critical) {
        return ClosePlan::Blocked;
    }
    if !confirmed {
        return ClosePlan::NeedsConfirm;
    }
    if windows > 0 {
        ClosePlan::PostMessage
    } else {
        ClosePlan::RestartManager
    }
}

pub(crate) fn holder_line(app: &str, process: &str, pid: u32) -> String {
    let app_owned;
    let process_owned;
    let app = if app.trim().is_empty() {
        app_owned = "—".to_string();
        app_owned.as_str()
    } else {
        app.trim()
    };
    let process = if process.trim().is_empty() {
        process_owned = "—".to_string();
        process_owned.as_str()
    } else {
        process.trim()
    };
    let pid = pid.to_string();
    tf(Key::FileInUseHolder, &[("app", app), ("process", process), ("pid", &pid)])
}

pub(crate) fn rm_path(path: &str) -> String {
    let p = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    };
    if p.len() > 3 {
        p.trim_end_matches(['\\', '/']).to_string()
    } else {
        p
    }
}

pub(crate) fn content_text(paths: &[String], lookup: &Lookup) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(first) = paths.iter().find(|p| !p.is_empty()) {
        lines.push(first.clone());
        let extra = paths.iter().filter(|p| !p.is_empty()).count().saturating_sub(1);
        if extra > 0 {
            lines.push(format!("+{extra}"));
        }
    }
    if !lines.is_empty() {
        lines.push(String::new());
    }
    match lookup {
        Lookup::Holders { holders, partial } if !holders.is_empty() => {
            for h in holders.iter().take(MAX_SHOWN) {
                lines.push(holder_line(&h.app_name, &h.process_name, h.pid));
            }
            if *partial {
                lines.push(String::new());
                lines.push(tr(Key::FileInUsePartial).to_string());
            }
        }
        _ => lines.push(tr(Key::FileInUseNone).to_string()),
    }
    lines.join("\n")
}

/// Run `once` until it succeeds. A sharing or lock failure asks what to do.
/// Cancel comes back as `COPYENGINE_E_USER_CANCELLED`, which callers already skip.
pub(crate) unsafe fn recover(owner: HWND, kind: OpKind, fallback: &[String], mut once: impl FnMut() -> Attempt) -> Result<()> {
    loop {
        let attempt = once();
        // A progress sink can record an in-use item even when PerformOperations returns S_OK.
        if attempt.result.is_ok() && attempt.locked.is_empty() {
            return Ok(());
        }
        let code = match &attempt.result {
            Ok(()) => S_OK,
            Err(e) => e.code(),
        };
        let message = match &attempt.result {
            Ok(()) => String::new(),
            Err(e) => e.message().to_string(),
        };
        if !should_prompt(code, &message, &attempt.locked) {
            return attempt.result;
        }
        let paths = if attempt.locked.is_empty() { fallback.to_vec() } else { attempt.locked };
        if !ask_retry(owner, kind, &paths) {
            return Err(Error::from(COPYENGINE_E_USER_CANCELLED));
        }
    }
}

pub(crate) unsafe fn ask_retry(owner: HWND, kind: OpKind, paths: &[String]) -> bool {
    let lookup = lookup_paths(paths);
    ask(owner, true, kind, paths, lookup)
}

/// Context menu: the same lookup, with no failed operation to retry.
pub(crate) unsafe fn show_who(owner: HWND, path: &str) {
    let paths = [path.to_string()];
    let lookup = lookup_paths(&paths);
    let _ = ask(owner, false, OpKind::Delete, &paths, lookup);
}

/// True when the user wants the operation tried again.
unsafe fn ask(owner: HWND, failed: bool, _kind: OpKind, paths: &[String], lookup: Lookup) -> bool {
    let holders = match &lookup {
        Lookup::Holders { holders, .. } => holders.clone(),
        Lookup::None { .. } => Vec::new(),
    };
    let shown = offer(failed, holders.len());
    ASK.with(|slot| {
        *slot.borrow_mut() = Some(AskCtx { holders, selected: 0 });
    });
    let instruction = if failed { tr(Key::FileInUse) } else { tr(Key::WhoIsUsingThisFile) };
    let content = content_text(paths, &lookup);
    let choice = task_dialog(owner, instruction, &content, &shown);
    ASK.with(|slot| *slot.borrow_mut() = None);
    choice == IDRETRY.0
}

struct AskCtx {
    holders: Vec<Holder>,
    selected: usize,
}

thread_local! {
    static ASK: RefCell<Option<AskCtx>> = const { RefCell::new(None) };
}

fn selected_holder() -> Option<Holder> {
    ASK.with(|slot| {
        let ctx = slot.borrow();
        let ctx = ctx.as_ref()?;
        ctx.holders.get(ctx.selected).cloned()
    })
}

unsafe extern "system" fn ask_cb(hwnd: HWND, msg: TASKDIALOG_NOTIFICATIONS, wp: WPARAM, _lp: LPARAM, _data: isize) -> HRESULT {
    if msg == TDN_RADIO_BUTTON_CLICKED {
        let id = wp.0 as i32;
        if id >= RADIO_BASE {
            ASK.with(|slot| {
                if let Some(ctx) = slot.borrow_mut().as_mut() {
                    ctx.selected = (id - RADIO_BASE) as usize;
                }
            });
        }
        return S_OK;
    }
    if msg != TDN_BUTTON_CLICKED {
        return S_OK;
    }
    let id = wp.0 as i32;
    if id == ID_SWITCH {
        if let Some(h) = selected_holder() {
            let _ = switch_to(h.pid);
        }
        return S_FALSE;
    }
    if id == ID_CLOSE_APP {
        if let Some(h) = selected_holder() {
            on_close(hwnd, &h);
        }
        return S_FALSE;
    }
    S_OK
}

unsafe fn on_close(owner: HWND, holder: &Holder) {
    let windows = top_windows(holder.pid);
    let plan = plan_close(false, holder.pid == std::process::id(), holder.kind, windows.len());
    if plan == ClosePlan::Blocked {
        let name = display_name(holder);
        let text = tf(Key::FileInUseKeep, &[("app", &name)]);
        let _ = MessageBoxW(Some(owner), &HSTRING::from(text), w!("Maguro"), MB_OK | MB_ICONINFORMATION);
        return;
    }
    let name = display_name(holder);
    let text = tf(Key::CloseProgramAsk, &[("app", &name)]);
    if MessageBoxW(Some(owner), &HSTRING::from(text), w!("Maguro"), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
        return;
    }
    match plan_close(true, holder.pid == std::process::id(), holder.kind, windows.len()) {
        ClosePlan::PostMessage => {
            for hwnd in windows {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        ClosePlan::RestartManager => {
            let _ = rm_shutdown_one(holder);
        }
        ClosePlan::Blocked | ClosePlan::NeedsConfirm => {}
    }
}

fn display_name(h: &Holder) -> String {
    if !h.app_name.trim().is_empty() {
        h.app_name.trim().to_string()
    } else if !h.process_name.trim().is_empty() {
        h.process_name.trim().to_string()
    } else {
        format!("PID {}", h.pid)
    }
}

unsafe fn task_dialog(owner: HWND, instruction: &str, content: &str, shown: &Offer) -> i32 {
    let title = HSTRING::from("Maguro");
    let head = HSTRING::from(instruction);
    let body = HSTRING::from(content);
    let mut labels: Vec<(i32, HSTRING)> = Vec::new();
    if shown.retry {
        labels.push((IDRETRY.0, HSTRING::from(tr(Key::Retry))));
    }
    if shown.switch_to {
        labels.push((ID_SWITCH, HSTRING::from(tr(Key::SwitchToProgram))));
    }
    if shown.close_program {
        labels.push((ID_CLOSE_APP, HSTRING::from(tr(Key::CloseProgram))));
    }
    let dismiss = if shown.retry { IDCANCEL.0 } else { ID_DISMISS };
    let dismiss_label = if shown.retry { tr(Key::Cancel) } else { tr(Key::Close) };
    labels.push((dismiss, HSTRING::from(dismiss_label)));
    let buttons: Vec<TASKDIALOG_BUTTON> = labels
        .iter()
        .map(|(id, text)| TASKDIALOG_BUTTON { nButtonID: *id, pszButtonText: PCWSTR(text.as_ptr()) })
        .collect();
    let radios_text: Vec<HSTRING> = ASK.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|ctx| {
                ctx.holders
                    .iter()
                    .take(MAX_SHOWN)
                    .map(|h| HSTRING::from(holder_line(&h.app_name, &h.process_name, h.pid)))
                    .collect()
            })
            .unwrap_or_default()
    });
    let radios: Vec<TASKDIALOG_BUTTON> = if radios_text.len() > 1 {
        radios_text
            .iter()
            .enumerate()
            .map(|(i, text)| TASKDIALOG_BUTTON { nButtonID: RADIO_BASE + i as i32, pszButtonText: PCWSTR(text.as_ptr()) })
            .collect()
    } else {
        Vec::new()
    };
    let default_button = if shown.retry { IDRETRY.0 } else { dismiss };
    let cfg = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW | TDF_SIZE_TO_CONTENT,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        pszMainInstruction: PCWSTR(head.as_ptr()),
        pszContent: PCWSTR(body.as_ptr()),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: default_button,
        cRadioButtons: radios.len() as u32,
        pRadioButtons: if radios.is_empty() { std::ptr::null() } else { radios.as_ptr() },
        nDefaultRadioButton: if radios.is_empty() { 0 } else { RADIO_BASE },
        Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: TD_WARNING_ICON },
        pfCallback: Some(ask_cb),
        ..Default::default()
    };
    let mut pressed = 0i32;
    if TaskDialogIndirect(&cfg, Some(&mut pressed), None, None).is_err() {
        return fallback_box(owner, content, shown.retry);
    }
    pressed
}

unsafe fn fallback_box(owner: HWND, content: &str, retry: bool) -> i32 {
    let text = HSTRING::from(content);
    let flags = if retry { MB_RETRYCANCEL | MB_ICONWARNING } else { MB_OK | MB_ICONINFORMATION };
    let answer = MessageBoxW(Some(owner), &text, w!("Maguro"), flags);
    if retry && answer == IDRETRY {
        IDRETRY.0
    } else if retry {
        IDCANCEL.0
    } else {
        ID_DISMISS
    }
}

pub(crate) unsafe fn lookup_paths(paths: &[String]) -> Lookup {
    let paths: Vec<String> = paths.iter().map(|p| rm_path(p)).filter(|p| !p.is_empty()).collect();
    let mut seen = Vec::new();
    let mut unique = Vec::new();
    for p in paths {
        let key = p.to_lowercase();
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        unique.push(p);
    }
    if unique.is_empty() {
        return Lookup::None { partial: false };
    }
    match rm_list(&unique) {
        Ok((info, reasons)) => holders_from(info, reasons),
        Err(_) => Lookup::None { partial: false },
    }
}

fn holders_from(info: Vec<RM_PROCESS_INFO>, reasons: u32) -> Lookup {
    let partial = reasons & (RmRebootReasonPermissionDenied.0 as u32) != 0;
    let mut holders = Vec::new();
    for row in info {
        let pid = row.Process.dwProcessId;
        if pid == 0 {
            continue;
        }
        let app = wide_field(&row.strAppName);
        let service = wide_field(&row.strServiceShortName);
        let mut process = unsafe { process_exe(pid) };
        if process.is_empty() {
            process = service.clone();
        }
        let app_name = if !app.is_empty() {
            app
        } else if !service.is_empty() {
            service
        } else {
            process.clone()
        };
        let holder = Holder { app_name, process_name: process, pid, start: row.Process.ProcessStartTime, kind: kind_of(row.ApplicationType) };
        if let Some(prev) = holders.iter_mut().find(|h: &&mut Holder| h.pid == pid) {
            if prev.app_name.is_empty() {
                prev.app_name = holder.app_name;
            }
            if prev.process_name.is_empty() {
                prev.process_name = holder.process_name;
            }
        } else {
            holders.push(holder);
        }
    }
    if holders.is_empty() {
        Lookup::None { partial }
    } else {
        Lookup::Holders { holders, partial }
    }
}

fn kind_of(kind: RM_APP_TYPE) -> HolderKind {
    if kind == RmMainWindow {
        HolderKind::Window
    } else if kind == RmOtherWindow {
        HolderKind::OtherWindow
    } else if kind == RmService {
        HolderKind::Service
    } else if kind == RmExplorer {
        HolderKind::Explorer
    } else if kind == RmConsole {
        HolderKind::Console
    } else if kind == RmCritical {
        HolderKind::Critical
    } else if kind == RmUnknownApp {
        HolderKind::Unknown
    } else {
        HolderKind::Unknown
    }
}

fn wide_field(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim().to_string()
}

struct RmSession(u32);

impl Drop for RmSession {
    fn drop(&mut self) {
        unsafe {
            let _ = RmEndSession(self.0);
        }
    }
}

unsafe fn rm_list(paths: &[String]) -> std::result::Result<(Vec<RM_PROCESS_INFO>, u32), u32> {
    let mut key = [0u16; (CCH_RM_SESSION_KEY + 1) as usize];
    let mut session = 0u32;
    let err = RmStartSession(&mut session, Some(0), PWSTR(key.as_mut_ptr()));
    if err != ERROR_SUCCESS {
        return Err(err.0);
    }
    let _end = RmSession(session);
    let wides: Vec<Vec<u16>> = paths.iter().map(|p| p.encode_utf16().chain(std::iter::once(0)).collect()).collect();
    let ptrs: Vec<PCWSTR> = wides.iter().map(|w| PCWSTR(w.as_ptr())).collect();
    let err = RmRegisterResources(session, Some(&ptrs), None, None);
    if err != ERROR_SUCCESS {
        return Err(err.0);
    }
    for _ in 0..4 {
        let mut needed = 0u32;
        let mut count = 0u32;
        let mut reasons = 0u32;
        let err = RmGetList(session, &mut needed, &mut count, None, &mut reasons);
        if err == ERROR_SUCCESS {
            return Ok((Vec::new(), reasons));
        }
        if err != ERROR_MORE_DATA {
            return Err(err.0);
        }
        if needed == 0 {
            return Ok((Vec::new(), reasons));
        }
        let mut buf = vec![RM_PROCESS_INFO::default(); needed as usize];
        count = needed;
        let err = RmGetList(session, &mut needed, &mut count, Some(buf.as_mut_ptr()), &mut reasons);
        if err == ERROR_SUCCESS {
            let n = (count as usize).min(buf.len());
            buf.truncate(n);
            return Ok((buf, reasons));
        }
        if err != ERROR_MORE_DATA {
            return Err(err.0);
        }
    }
    Err(ERROR_MORE_DATA.0)
}

/// Register that one process and ask Restart Manager to shut it down. Flags stay at `shutdown_flags`.
unsafe fn rm_shutdown_one(holder: &Holder) -> bool {
    let mut key = [0u16; (CCH_RM_SESSION_KEY + 1) as usize];
    let mut session = 0u32;
    if RmStartSession(&mut session, Some(0), PWSTR(key.as_mut_ptr())) != ERROR_SUCCESS {
        return false;
    }
    let _end = RmSession(session);
    let proc = RM_UNIQUE_PROCESS { dwProcessId: holder.pid, ProcessStartTime: holder.start };
    if RmRegisterResources(session, None, Some(&[proc]), None) != ERROR_SUCCESS {
        return false;
    }
    let flags = shutdown_flags();
    debug_assert_eq!(flags & (RmForceShutdown.0 as u32), 0);
    RmShutdown(session, flags, None) == ERROR_SUCCESS
}

unsafe fn process_exe(pid: u32) -> String {
    let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
        return String::new();
    };
    let mut buf = [0u16; 520];
    let mut n = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n);
    let _ = CloseHandle(handle);
    if ok.is_err() || n == 0 {
        return String::new();
    }
    let n = n as usize;
    if n > buf.len() {
        return String::new();
    }
    let full = String::from_utf16_lossy(&buf[..n]);
    std::path::Path::new(&full).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or(full)
}

struct WinList {
    pid: u32,
    out: Vec<HWND>,
}

unsafe extern "system" fn enum_win(hwnd: HWND, lp: LPARAM) -> BOOL {
    let list = &mut *(lp.0 as *mut WinList);
    let mut pid = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == list.pid && IsWindowVisible(hwnd).as_bool() && GetWindow(hwnd, GW_OWNER).is_err() {
        list.out.push(hwnd);
    }
    true.into()
}

unsafe fn top_windows(pid: u32) -> Vec<HWND> {
    let mut list = WinList { pid, out: Vec::new() };
    let _ = EnumWindows(Some(enum_win), LPARAM(&mut list as *mut WinList as isize));
    list.out
}

unsafe fn switch_to(pid: u32) -> bool {
    let wins = top_windows(pid);
    let Some(&hwnd) = wins.first() else { return false };
    if IsIconic(hwnd).as_bool() {
        let _ = ShowWindow(hwnd, SW_RESTORE);
    } else {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    let fg = GetForegroundWindow();
    let fg_thread = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
    let target_thread = GetWindowThreadProcessId(hwnd, None);
    let attached = fg_thread != 0 && target_thread != 0 && fg_thread != target_thread && AttachThreadInput(fg_thread, target_thread, true).as_bool();
    let _ = BringWindowToTop(hwnd);
    let _ = SetForegroundWindow(hwnd);
    if attached {
        let _ = AttachThreadInput(fg_thread, target_thread, false);
    }
    true
}

#[implement(IFileOperationProgressSink)]
struct InUseSink {
    recorded: Arc<Mutex<Vec<String>>>,
}

impl InUseSink {
    fn note(&self, hr: HRESULT, item: &windows_core::Ref<IShellItem>, dest_folder: &windows_core::Ref<IShellItem>, new_name: &PCWSTR) -> bool {
        if !is_in_use_code(hr) {
            let msg = Error::from(hr).message();
            if !message_says_in_use(&msg) {
                return false;
            }
        }
        let mut got = Vec::new();
        if let Some(p) = shell_path(item) {
            got.push(p);
        }
        if let Some(folder) = shell_path(dest_folder) {
            let name = pcw_string(new_name);
            let name = if name.is_empty() {
                got.first().and_then(|p| std::path::Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default()
            } else {
                name
            };
            if !name.is_empty() {
                let folder = folder.trim_end_matches(['\\', '/']);
                got.push(format!("{folder}\\{name}"));
            } else if !folder.is_empty() {
                got.push(folder);
            }
        }
        let mut g = self.recorded.lock().unwrap_or_else(|e| e.into_inner());
        for p in got {
            if !p.is_empty() && !g.iter().any(|e| e.eq_ignore_ascii_case(&p)) {
                g.push(p);
            }
        }
        true
    }

    fn push_path(&self, path: &str) {
        if path.is_empty() {
            return;
        }
        let mut g = self.recorded.lock().unwrap_or_else(|e| e.into_inner());
        if !g.iter().any(|e| e.eq_ignore_ascii_case(path)) {
            g.push(path.to_string());
        }
    }
}

fn shell_path(item: &windows_core::Ref<IShellItem>) -> Option<String> {
    let item = item.as_ref()?;
    let p = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH).ok()? };
    let s = unsafe { crate::pwstr(p) };
    (!s.is_empty()).then_some(s)
}

fn pcw_string(p: &PCWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { p.to_string().unwrap_or_default() }
}

impl IFileOperationProgressSink_Impl for InUseSink_Impl {
    fn StartOperations(&self) -> Result<()> {
        Ok(())
    }
    fn FinishOperations(&self, _hrresult: HRESULT) -> Result<()> {
        Ok(())
    }
    fn PreRenameItem(&self, _dwflags: u32, _psiitem: windows_core::Ref<IShellItem>, _psznewname: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostRenameItem(&self, _dwflags: u32, psiitem: windows_core::Ref<IShellItem>, psznewname: &PCWSTR, hrrename: HRESULT, _psinewlycreated: windows_core::Ref<IShellItem>) -> Result<()> {
        // The sink item is the old name. A rename onto an open file is locked at the new path.
        if self.note(hrrename, &psiitem, &windows_core::Ref::default(), psznewname) {
            let name = pcw_string(psznewname);
            if !name.is_empty() {
                if let Some(src) = shell_path(&psiitem) {
                    if let Some(dir) = std::path::Path::new(&src).parent() {
                        self.push_path(&dir.join(name).to_string_lossy());
                    }
                }
            }
        }
        Ok(())
    }
    fn PreMoveItem(&self, _dwflags: u32, _psiitem: windows_core::Ref<IShellItem>, _psidestinationfolder: windows_core::Ref<IShellItem>, _psznewname: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostMoveItem(
        &self,
        _dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
        psidestinationfolder: windows_core::Ref<IShellItem>,
        psznewname: &PCWSTR,
        hrmove: HRESULT,
        _psinewlycreated: windows_core::Ref<IShellItem>,
    ) -> Result<()> {
        self.note(hrmove, &psiitem, &psidestinationfolder, psznewname);
        Ok(())
    }
    fn PreCopyItem(&self, _dwflags: u32, _psiitem: windows_core::Ref<IShellItem>, _psidestinationfolder: windows_core::Ref<IShellItem>, _psznewname: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostCopyItem(
        &self,
        _dwflags: u32,
        psiitem: windows_core::Ref<IShellItem>,
        psidestinationfolder: windows_core::Ref<IShellItem>,
        psznewname: &PCWSTR,
        hrcopy: HRESULT,
        _psinewlycreated: windows_core::Ref<IShellItem>,
    ) -> Result<()> {
        self.note(hrcopy, &psiitem, &psidestinationfolder, psznewname);
        Ok(())
    }
    fn PreDeleteItem(&self, _dwflags: u32, _psiitem: windows_core::Ref<IShellItem>) -> Result<()> {
        Ok(())
    }
    fn PostDeleteItem(&self, _dwflags: u32, psiitem: windows_core::Ref<IShellItem>, hrdelete: HRESULT, _psinewlycreated: windows_core::Ref<IShellItem>) -> Result<()> {
        self.note(hrdelete, &psiitem, &windows_core::Ref::default(), &PCWSTR::null());
        Ok(())
    }
    fn PreNewItem(&self, _dwflags: u32, _psidestinationfolder: windows_core::Ref<IShellItem>, _psznewname: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn PostNewItem(
        &self,
        _dwflags: u32,
        _psidestinationfolder: windows_core::Ref<IShellItem>,
        _psznewname: &PCWSTR,
        _psztemplatename: &PCWSTR,
        _dwfileattributes: u32,
        _hrnew: HRESULT,
        _psinewitem: windows_core::Ref<IShellItem>,
    ) -> Result<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _iworktotal: u32, _iworksofar: u32) -> Result<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> Result<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> Result<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> Result<()> {
        Ok(())
    }
}

pub(crate) unsafe fn perform_watched(op: &IFileOperation) -> (Result<()>, Vec<String>) {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let sink: IFileOperationProgressSink = InUseSink { recorded: recorded.clone() }.into();
    let cookie = op.Advise(&sink).ok();
    let result = op.PerformOperations();
    if let Some(c) = cookie {
        let _ = op.Unadvise(c);
    }
    let locked = recorded.lock().unwrap_or_else(|e| e.into_inner()).clone();
    (result, locked)
}

// Silence the unused import if the callback type alias is only documentation.
#[allow(dead_code)]
fn _callback_type(f: PFTASKDIALOGCALLBACK) -> PFTASKDIALOGCALLBACK {
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharing_and_lock_codes_are_in_use_and_cancel_is_not() {
        assert!(is_in_use_code(ERROR_SHARING_VIOLATION.to_hresult()));
        assert!(is_in_use_code(ERROR_LOCK_VIOLATION.to_hresult()));
        assert!(is_in_use_code(COPYENGINE_E_SHARING_VIOLATION_SRC));
        assert!(is_in_use_code(COPYENGINE_E_SHARING_VIOLATION_DEST));
        assert!(!is_in_use_code(COPYENGINE_E_USER_CANCELLED));
        assert!(!is_in_use_code(ERROR_CANCELLED.to_hresult()));
        assert!(!is_in_use_code(COPYENGINE_E_ACCESS_DENIED_SRC));
        assert!(message_says_in_use("The process cannot access the file because it is being used by another process."));
        assert!(!message_says_in_use("Access is denied."));
        assert!(should_prompt(COPYENGINE_E_USER_CANCELLED, "cancelled", &["C:\\a.txt".into()]));
        assert!(!should_prompt(COPYENGINE_E_USER_CANCELLED, "cancelled", &[]));
        assert!(text_is_in_use("2026/11/01 12:00:00 ERROR 32 (0x00000020) Copying File C:\\a.txt"));
        assert!(text_is_in_use("ERROR 33 (0x00000021)"));
        assert!(!text_is_in_use("ERROR 5 (0x00000005) Access is denied."));
    }

    #[test]
    fn shutdown_never_uses_the_force_flag() {
        assert_eq!(shutdown_flags(), 0);
        assert_eq!(shutdown_flags() & (RmForceShutdown.0 as u32), 0);
    }

    #[test]
    fn close_waits_for_yes_and_refuses_explorer_and_this_process() {
        assert_eq!(plan_close(false, false, HolderKind::Window, 1), ClosePlan::NeedsConfirm);
        assert_eq!(plan_close(true, false, HolderKind::Window, 2), ClosePlan::PostMessage);
        assert_eq!(plan_close(true, false, HolderKind::Console, 0), ClosePlan::RestartManager);
        assert_eq!(plan_close(true, false, HolderKind::Service, 0), ClosePlan::RestartManager);
        assert_eq!(plan_close(true, true, HolderKind::Window, 3), ClosePlan::Blocked);
        assert_eq!(plan_close(true, false, HolderKind::Explorer, 4), ClosePlan::Blocked);
        assert_eq!(plan_close(false, false, HolderKind::Critical, 0), ClosePlan::Blocked);
        assert_eq!(plan_close(true, false, HolderKind::Unknown, 0), ClosePlan::RestartManager);
    }

    #[test]
    fn buttons_follow_whether_a_program_was_found() {
        let found = offer(true, 2);
        assert!(found.retry && found.switch_to && found.close_program && found.dismiss);
        let none = offer(true, 0);
        assert!(none.retry && none.dismiss && !none.switch_to && !none.close_program);
        let who = offer(false, 1);
        assert!(!who.retry && who.switch_to && who.close_program && who.dismiss);
        let who_none = offer(false, 0);
        assert!(!who_none.retry && !who_none.switch_to && !who_none.close_program && who_none.dismiss);
    }

    #[test]
    fn holder_line_names_the_program_the_process_and_the_pid() {
        let line = holder_line("Notepad", "notepad.exe", 4242);
        assert!(line.contains("Notepad"), "{line}");
        assert!(line.contains("notepad.exe"), "{line}");
        assert!(line.contains("4242"), "{line}");
        let blank = holder_line("  ", "", 7);
        assert!(blank.contains('—') && blank.contains('7'), "{blank}");
    }

    #[test]
    fn empty_lookup_says_nothing_was_found() {
        let text = content_text(&[r"C:\WORK\pgtest\lock-test\a.txt".into()], &Lookup::None { partial: false });
        assert!(text.contains(r"C:\WORK\pgtest\lock-test\a.txt"), "{text}");
        assert!(text.contains("No program was found"), "{text}");
        assert!(text.contains("administrator"), "{text}");
        let found = Lookup::Holders {
            holders: vec![Holder {
                app_name: "Notepad".into(),
                process_name: "notepad.exe".into(),
                pid: 9,
                start: FILETIME::default(),
                kind: HolderKind::Window,
            }],
            partial: true,
        };
        let text = content_text(&[r"C:\a.txt".into()], &found);
        assert!(text.contains("notepad.exe") && text.contains("administrator"), "{text}");
    }

    #[test]
    fn rm_path_drops_the_verbatim_prefix() {
        assert_eq!(rm_path(r"\\?\C:\Work\a.txt"), r"C:\Work\a.txt");
        assert_eq!(rm_path(r"\\?\UNC\server\share\a"), r"\\server\share\a");
        assert_eq!(rm_path(r"C:\"), r"C:\");
    }

    #[test]
    fn restart_manager_names_the_process_holding_the_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("maguro-rm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("held.txt");
        std::fs::write(&path, b"held").unwrap();
        let shown = path.to_string_lossy().into_owned();
        let open = Lookup::None { partial: false };
        let idle = unsafe { lookup_paths(&[shown.clone()]) };
        match &idle {
            Lookup::Holders { holders, .. } => assert!(holders.iter().all(|h| h.pid != std::process::id()), "unlocked file listed us: {holders:?}"),
            Lookup::None { .. } => {}
        }
        let _ = open;
        let file = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(&path).unwrap();
        let lookup = unsafe { lookup_paths(&[shown]) };
        let Lookup::Holders { holders, .. } = &lookup else {
            drop(file);
            let _ = std::fs::remove_dir_all(&dir);
            panic!("expected a holder, got {lookup:?}");
        };
        let mine = holders.iter().find(|h| h.pid == std::process::id());
        let Some(mine) = mine else {
            drop(file);
            let _ = std::fs::remove_dir_all(&dir);
            panic!("our pid {} was not listed: {holders:?}", std::process::id());
        };
        let exe = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        assert!(
            mine.process_name.eq_ignore_ascii_case(&exe) || mine.process_name.eq_ignore_ascii_case("maguro.exe"),
            "process {} exe {exe}",
            mine.process_name
        );
        assert!(!mine.app_name.is_empty() || !mine.process_name.is_empty());
        drop(file);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
