//! Crash reports for a silent exit.
//!
//! An access violation in a shell extension reaches the vectored handler and the unhandled
//! filter; the report records the faulting module and offset while that DLL is loaded.
//! Fail-fast (`0xC0000409`, `0xC0000602`) skips both, so a detached watcher reads the exit
//! code and the last flushed trace. A killed process (`0xFFFFFFFF`, `0xC000013A`) is not
//! a fail-fast. Reports live in the settings folder (`crashes\`).

use std::cell::Cell;
use std::cell::RefCell;
use std::cell::UnsafeCell;
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, Once, OnceLock, TryLockError};

use windows::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, SetUnhandledExceptionFilter, EXCEPTION_EXECUTE_HANDLER, EXCEPTION_POINTERS,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::ProcessStatus::EnumProcessModules;
use windows::Win32::System::Threading::{GetCurrentProcess, GetExitCodeProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};

use crate::imports::*;

const LOG_CAP: usize = 40;

static DIR: OnceLock<PathBuf> = OnceLock::new();
static LAST: Mutex<String> = Mutex::new(String::new());
static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
static SHELL: Mutex<Vec<String>> = Mutex::new(Vec::new());
static WROTE: AtomicBool = AtomicBool::new(false);
static INSTALLED: Once = Once::new();

struct Cap {
    code: AtomicU32,
    addr: AtomicU64,
    off: AtomicU64,
    len: AtomicUsize,
    bytes: UnsafeCell<[u8; 1024]>,
}
unsafe impl Sync for Cap {}

static CAP: Cap = Cap {
    code: AtomicU32::new(0),
    addr: AtomicU64::new(0),
    off: AtomicU64::new(0),
    len: AtomicUsize::new(0),
    bytes: UnsafeCell::new([0; 1024]),
};

thread_local! {
    static GUARD_DEPTH: Cell<u32> = const { Cell::new(0) };
    static WND_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Panic payload for a release-build `app()` re-entered from a window proc.
/// `catch_ui` returns the fallback. The hook writes no crash report.
pub(crate) struct QuietReentry;

pub(crate) fn skip_reenter(msg: &str) -> ! {
    log(msg);
    std::panic::panic_any(QuietReentry);
}

struct WndDepth;
impl Drop for WndDepth {
    fn drop(&mut self) {
        WND_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// True while a window procedure wrapped by `catch_wnd` is on the stack.
pub(crate) fn in_wndproc() -> bool {
    WND_DEPTH.with(|d| d.get() > 0)
}

struct PanicNote {
    message: String,
    location: String,
}

thread_local! {
    static PANIC_NOTE: RefCell<PanicNote> = const { RefCell::new(PanicNote { message: String::new(), location: String::new() }) };
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        GUARD_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// `catch_unwind` that tells the panic hook this panic is contained.
pub(crate) fn guard<R>(f: impl FnOnce() -> R + std::panic::UnwindSafe) -> std::thread::Result<R> {
    GUARD_DEPTH.with(|d| d.set(d.get().saturating_add(1)));
    let _g = Guard;
    std::panic::catch_unwind(f)
}

pub(crate) fn guarded() -> bool {
    GUARD_DEPTH.with(|d| d.get() > 0)
}

static SHOWING_PANIC: AtomicBool = AtomicBool::new(false);

/// A panic on the UI thread. The fallback is returned and an error is shown.
/// Unwinding out of a window procedure is undefined, and aborting is a silent exit.
pub(crate) fn catch_ui<R>(fallback: R, f: impl FnOnce() -> R + std::panic::UnwindSafe) -> R {
    match guard(f) {
        Ok(v) => v,
        Err(payload) => {
            // A window proc re-entered `app()` in a release build. Keep the process up.
            if payload.downcast_ref::<QuietReentry>().is_some() {
                return fallback;
            }
            // The hook already stored file:line. `&*payload` is the panic value.
            // `&payload` is the Box; downcast then misses `&str` and `String` and the dialog said "panic".
            let (hook_msg, hook_loc) = take_panic_note();
            let (_message, detail) = caught_panic_text(&hook_msg, &hook_loc, &*payload);
            log(&format!("ui panic: {detail}"));
            show_panic_dialog(&detail);
            fallback
        }
    }
}

pub(crate) fn catch_wnd(f: impl FnOnce() -> LRESULT + std::panic::UnwindSafe) -> LRESULT {
    WND_DEPTH.with(|d| d.set(d.get().saturating_add(1)));
    let _depth = WndDepth;
    catch_ui(LRESULT(0), f)
}

pub(crate) fn catch_bool(f: impl FnOnce() -> BOOL + std::panic::UnwindSafe) -> BOOL {
    catch_ui(BOOL::from(false), f)
}

pub(crate) fn catch_i32(f: impl FnOnce() -> i32 + std::panic::UnwindSafe) -> i32 {
    catch_ui(0, f)
}

fn panic_payload(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Box<dyn Any>".into()
    }
}

/// Dialog body: the panic's first line, then `file:line` when the hook recorded one.
pub(crate) fn panic_dialog_detail(message: &str, location: &str) -> String {
    let short = message.lines().next().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Box<dyn Any>");
    if location.is_empty() { short.to_string() } else { format!("{short}\n{location}") }
}

/// Message and dialog text for a panic `catch_ui` caught.
/// An empty hook message means the hook did not run, so the payload is used.
fn caught_panic_text(hook_msg: &str, hook_loc: &str, payload: &(dyn std::any::Any + Send)) -> (String, String) {
    let message = if hook_msg.is_empty() { panic_payload(payload) } else { hook_msg.to_string() };
    let detail = panic_dialog_detail(&message, hook_loc);
    (message, detail)
}

fn panic_location(info: &std::panic::PanicHookInfo) -> String {
    info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default()
}

fn remember_panic(message: &str, location: &str) {
    PANIC_NOTE.with(|n| {
        let mut n = n.borrow_mut();
        n.message = message.to_string();
        n.location = location.to_string();
    });
}

fn take_panic_note() -> (String, String) {
    PANIC_NOTE.with(|n| {
        let mut n = n.borrow_mut();
        (std::mem::take(&mut n.message), std::mem::take(&mut n.location))
    })
}

fn show_panic_dialog(detail: &str) {
    if cfg!(test) || SHOWING_PANIC.swap(true, Ordering::SeqCst) {
        return;
    }
    let text = format!(
        "{}\n\n{detail}",
        crate::tr(crate::Key::MaguroHitErrorAndKeptThisWindow)
    );
    unsafe {
        let _ = MessageBoxW(None, &HSTRING::from(text), w!("Maguro"), MB_OK | MB_ICONWARNING);
    }
    SHOWING_PANIC.store(false, Ordering::SeqCst);
}

/// `--crash-watch` and `--maguro-image` never install hooks or spawn another watcher.
pub(crate) fn helper(args: &[String]) -> bool {
    match args.first().map(String::as_str) {
        Some("--crash-watch") => {
            watch_main(args);
            true
        }
        Some("--maguro-image") => {
            let code = crate::pane::image_main(args);
            std::process::exit(code);
        }
        _ => false,
    }
}

pub(crate) fn install() {
    INSTALLED.call_once(|| {
        let dir = crate::store::config_dir();
        let _ = std::fs::create_dir_all(dir.join("crashes"));
        let _ = DIR.set(dir.clone());
        std::panic::set_hook(Box::new(|info| {
            // Release re-entry from a window proc is logged by `skip_reenter` and caught by `catch_ui`.
            if info.payload().is::<QuietReentry>() {
                return;
            }
            // `catch_ui` sets the guard before `catch_unwind`, so a UI panic used to return
            // here and never write `crashes/crash-*.txt`. The dialog then had no file:line.
            let msg = panic_payload(info.payload());
            let loc = panic_location(info);
            remember_panic(&msg, &loc);
            write_report("panic", &msg, &loc, "", "", "unknown");
            if guarded() {
                return;
            }
            show_panic_dialog(&panic_dialog_detail(&msg, &loc));
        }));
        unsafe {
            AddVectoredExceptionHandler(1, Some(veh));
            SetUnhandledExceptionFilter(Some(filter));
        }
        write_trace();
        let _ = std::thread::Builder::new().name("maguro-trace".into()).spawn(|| {
            let names = guard(|| crate::shell::blockext::registered_names()).unwrap_or_default();
            set_shell_dlls(names);
            loop {
                write_trace();
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });
        spawn_watcher(&dir);
    });
}

pub(crate) fn note_path(path: &str) {
    match LAST.lock() {
        Ok(mut g) => *g = path.to_string(),
        Err(p) => *p.into_inner() = path.to_string(),
    }
    log(&format!("nav {path}"));
    write_trace();
}

pub(crate) fn log(line: &str) {
    let line: String = line.chars().take(500).collect();
    let mut g = match LOG.try_lock() {
        Ok(g) => g,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    if g.len() >= LOG_CAP {
        g.pop_front();
    }
    g.push_back(line);
}

pub(crate) fn set_shell_dlls(names: Vec<String>) {
    match SHELL.lock() {
        Ok(mut g) => *g = names,
        Err(p) => *p.into_inner() = names,
    }
}

fn spawn_watcher(dir: &Path) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--crash-watch").arg(std::process::id().to_string()).arg(dir);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd.spawn();
}

fn watch_main(args: &[String]) {
    let Some(pid) = args.get(1).and_then(|s| s.parse::<u32>().ok()) else { return };
    let Some(dir) = args.get(2).map(PathBuf::from) else { return };
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, false, pid) else { return };
        let _ = WaitForSingleObject(h, INFINITE);
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code).is_ok();
        let _ = CloseHandle(h);
        if !ok || !watch_is_fatal(code) {
            return;
        }
        if dir.join("crash-pending.txt").is_file() {
            return;
        }
        let trace = std::fs::read_to_string(dir.join("last-trace.txt")).unwrap_or_default();
        let mut text = format!(
            "Maguro {}\nkind: fast-fail\nexit: 0x{code:08X}\n\
             note: fail-fast skips the vectored handler, so the faulting address was not available.\n\
             Modules below are the last snapshot taken before exit.\n\n{trace}",
            env!("CARGO_PKG_VERSION")
        );
        if !text.contains("blocked_shell_extensions") {
            text.push_str("\nhint: add the DLL file name to blocked_shell_extensions in maguro.json and restart. This blocks it inside Maguro only. Explorer is unchanged. Windows system DLLs are not blocked.\n");
        }
        let _ = DIR.set(dir);
        let _ = save_report(&text);
    }
}

unsafe extern "system" fn veh(info: *mut EXCEPTION_POINTERS) -> i32 {
    capture(info, false);
    0
}

unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
    capture(info as *mut EXCEPTION_POINTERS, true);
    write_exception_report();
    EXCEPTION_EXECUTE_HANDLER
}

/// Exit codes the detached watcher records. `0xFFFFFFFF` is `Stop-Process -Force`.
/// `0xC000013A` is CTRL+C. Anything else must be on the `is_fatal` allowlist.
fn watch_is_fatal(code: u32) -> bool {
    if code == 0xFFFF_FFFF || code == 0xC000_013A {
        return false;
    }
    is_fatal(code)
}

fn is_fatal(code: u32) -> bool {
    matches!(code, 0xC000_0005 | 0xC000_0008 | 0xC000_001D | 0xC000_0094 | 0xC000_0096 | 0xC000_00FD | 0xC000_0374 | 0xC000_0409 | 0xC000_041D | 0xC000_0602)
}

unsafe fn capture(info: *mut EXCEPTION_POINTERS, force: bool) {
    if info.is_null() {
        return;
    }
    let rec = (*info).ExceptionRecord;
    if rec.is_null() {
        return;
    }
    let code = (*rec).ExceptionCode.0 as u32;
    if !force && (!is_fatal(code) || CAP.code.load(Ordering::Acquire) != 0) {
        return;
    }
    let addr = (*rec).ExceptionAddress;
    let (name, off) = module_at(addr).unwrap_or_default();
    let bytes = name.as_bytes();
    let n = bytes.len().min(1024);
    let buf = &mut *CAP.bytes.get();
    buf[..n].copy_from_slice(&bytes[..n]);
    CAP.addr.store(addr as u64, Ordering::Relaxed);
    CAP.off.store(off, Ordering::Relaxed);
    CAP.len.store(n, Ordering::Release);
    CAP.code.store(code, Ordering::Release);
}

fn read_capture() -> (u32, u64, u64, String) {
    let code = CAP.code.load(Ordering::Acquire);
    let n = CAP.len.load(Ordering::Acquire).min(1024);
    let bytes = unsafe { &*CAP.bytes.get() };
    let name = String::from_utf8_lossy(&bytes[..n]).into_owned();
    (code, CAP.addr.load(Ordering::Relaxed), CAP.off.load(Ordering::Relaxed), name)
}

fn write_exception_report() {
    let (code, addr, off, name) = read_capture();
    let module = module_line(&name, off);
    let address = if addr == 0 { String::new() } else { format!("0x{addr:X}") };
    let exception = if code == 0 { String::new() } else { format!("0x{code:08X}") };
    write_report("exception", "", "", &exception, &address, &module);
}

fn write_report(kind: &str, detail: &str, location: &str, exception: &str, address: &str, module: &str) {
    if WROTE.swap(true, Ordering::SeqCst) {
        return;
    }
    let report = Report {
        version: env!("CARGO_PKG_VERSION").into(),
        thread: thread_label(),
        kind: kind.into(),
        detail: detail.into(),
        location: location.into(),
        exception: exception.into(),
        address: address.into(),
        module: module.into(),
        last_path: locked_string(&LAST),
        shell_loaded: shell_hits(),
        third_party: third_party_modules(&loaded_modules()),
        recent: recent_lines(),
    };
    let _ = save_report(&format_report(&report));
}

fn save_report(text: &str) -> Option<PathBuf> {
    let dir = DIR.get()?;
    write_text_report(dir, text)
}

fn write_text_report(dir: &Path, text: &str) -> Option<PathBuf> {
    let crashes = dir.join("crashes");
    let _ = std::fs::create_dir_all(&crashes);
    let path = crashes.join(format!("crash-{}.txt", stamp()));
    std::fs::write(&path, text).ok()?;
    let marker = dir.join("crash-pending.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(&marker) {
        let _ = writeln!(f, "{}", path.display());
    }
    Some(path)
}

fn write_trace() {
    let Some(dir) = DIR.get() else { return };
    let loaded = loaded_modules();
    let report = Report {
        version: env!("CARGO_PKG_VERSION").into(),
        thread: thread_label(),
        kind: "trace".into(),
        detail: String::new(),
        location: String::new(),
        exception: String::new(),
        address: String::new(),
        module: String::new(),
        last_path: locked_string(&LAST),
        shell_loaded: matching_shell(&loaded, &locked_list(&SHELL)),
        third_party: third_party_modules(&loaded),
        recent: recent_lines(),
    };
    let path = dir.join("last-trace.txt");
    let tmp = dir.join("last-trace.txt.tmp");
    if std::fs::write(&tmp, format_report(&report)).is_ok() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn shell_hits() -> Vec<String> {
    let loaded = loaded_modules();
    matching_shell(&loaded, &locked_list(&SHELL))
}

fn locked_string(m: &Mutex<String>) -> String {
    match m.try_lock() {
        Ok(g) => g.clone(),
        Err(TryLockError::Poisoned(p)) => p.into_inner().clone(),
        Err(TryLockError::WouldBlock) => "(unavailable)".into(),
    }
}

fn locked_list(m: &Mutex<Vec<String>>) -> Vec<String> {
    match m.try_lock() {
        Ok(g) => g.clone(),
        Err(TryLockError::Poisoned(p)) => p.into_inner().clone(),
        Err(TryLockError::WouldBlock) => Vec::new(),
    }
}

fn recent_lines() -> Vec<String> {
    match LOG.try_lock() {
        Ok(g) => g.iter().cloned().collect(),
        Err(TryLockError::Poisoned(p)) => p.into_inner().iter().cloned().collect(),
        Err(TryLockError::WouldBlock) => Vec::new(),
    }
}

fn thread_label() -> String {
    let t = std::thread::current();
    format!("{} id={:?}", t.name().unwrap_or("unnamed"), t.id())
}

fn stamp() -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("{ms}")
}

fn module_line(name: &str, off: u64) -> String {
    if name.is_empty() {
        "unknown".into()
    } else {
        format!("{name}+0x{off:X}")
    }
}

fn module_at(addr: *mut core::ffi::c_void) -> Option<(String, u64)> {
    if addr.is_null() {
        return None;
    }
    unsafe {
        let mut hm = HMODULE::default();
        let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        GetModuleHandleExW(flags, PCWSTR(addr as *const u16), &mut hm).ok()?;
        let name = module_path(hm)?;
        let base = hm.0 as usize;
        let off = (addr as usize).wrapping_sub(base) as u64;
        Some((name, off))
    }
}

fn module_path(hm: HMODULE) -> Option<String> {
    unsafe {
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(Some(hm), &mut buf);
        if n == 0 {
            return None;
        }
        let n = (n as usize).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

fn loaded_modules() -> Vec<String> {
    unsafe {
        let proc = GetCurrentProcess();
        let mut need = 0u32;
        let _ = EnumProcessModules(proc, std::ptr::null_mut(), 0, &mut need);
        let n = (need as usize / std::mem::size_of::<HMODULE>()).clamp(1, 1024);
        let mut mods = vec![HMODULE::default(); n];
        let cb = (mods.len() * std::mem::size_of::<HMODULE>()) as u32;
        if EnumProcessModules(proc, mods.as_mut_ptr(), cb, &mut need).is_err() {
            return Vec::new();
        }
        let count = (need as usize / std::mem::size_of::<HMODULE>()).min(mods.len());
        mods.truncate(count);
        mods.into_iter().filter_map(module_path).collect()
    }
}

pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path).trim()
}

pub(crate) fn is_system_module(path: &str) -> bool {
    let lower = path.replace('/', "\\").to_ascii_lowercase();
    lower.contains(r"\windows\system32\")
        || lower.contains(r"\windows\syswow64\")
        || lower.contains(r"\windows\winsxs\")
        || lower.contains(r"\windows\systemapps\")
}

fn is_self(path: &str) -> bool {
    file_name(path).eq_ignore_ascii_case("maguro.exe")
}

pub(crate) fn third_party_modules(paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in paths {
        if is_system_module(p) || is_self(p) {
            continue;
        }
        if !out.iter().any(|e: &String| e.eq_ignore_ascii_case(p)) {
            out.push(p.clone());
        }
    }
    out
}

pub(crate) fn matching_shell(loaded: &[String], registered: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in loaded {
        if is_system_module(p) || is_self(p) {
            continue;
        }
        let file = file_name(p);
        if registered.iter().any(|r| r.eq_ignore_ascii_case(file)) && !out.iter().any(|e: &String| e.eq_ignore_ascii_case(p)) {
            out.push(p.clone());
        }
    }
    out
}

pub(crate) struct Report {
    pub version: String,
    pub thread: String,
    pub kind: String,
    pub detail: String,
    pub location: String,
    pub exception: String,
    pub address: String,
    pub module: String,
    pub last_path: String,
    pub shell_loaded: Vec<String>,
    pub third_party: Vec<String>,
    pub recent: Vec<String>,
}

fn push_line(s: &mut String, line: &str) {
    s.push_str(line);
    s.push('\n');
}

pub(crate) fn format_report(r: &Report) -> String {
    let mut s = String::new();
    push_line(&mut s, &format!("Maguro {}", r.version));
    push_line(&mut s, &format!("thread: {}", r.thread));
    push_line(&mut s, &format!("kind: {}", r.kind));
    if !r.detail.is_empty() {
        push_line(&mut s, &format!("message: {}", r.detail));
    }
    if !r.location.is_empty() {
        push_line(&mut s, &format!("location: {}", r.location));
    }
    if !r.exception.is_empty() {
        push_line(&mut s, &format!("exception: {}", r.exception));
    }
    if !r.address.is_empty() {
        push_line(&mut s, &format!("address: {}", r.address));
    }
    if !r.module.is_empty() {
        push_line(&mut s, &format!("module: {}", r.module));
    }
    push_line(&mut s, &format!("last_path: {}", r.last_path));
    push_line(&mut s, "loaded shell extensions:");
    if r.shell_loaded.is_empty() {
        push_line(&mut s, "  (none recorded)");
    } else {
        for p in &r.shell_loaded {
            push_line(&mut s, &format!("  {p}"));
        }
    }
    push_line(&mut s, "loaded third-party modules:");
    if r.third_party.is_empty() {
        push_line(&mut s, "  (none recorded)");
    } else {
        for p in &r.third_party {
            push_line(&mut s, &format!("  {p}"));
        }
    }
    push_line(&mut s, "recent:");
    if r.recent.is_empty() {
        push_line(&mut s, "  (none)");
    } else {
        for line in &r.recent {
            push_line(&mut s, &format!("  {line}"));
        }
    }
    push_line(
        &mut s,
        "hint: add the DLL file name to blocked_shell_extensions in maguro.json and restart. This blocks it inside Maguro only. Explorer is unchanged. Windows system DLLs are not blocked.",
    );
    s
}

/// Next launch. The English sentence is fixed; Korean and Japanese come from `tr`.
pub(crate) fn show_previous() {
    let Some(dir) = DIR.get() else { return };
    let marker = dir.join("crash-pending.txt");
    let Ok(text) = std::fs::read_to_string(&marker) else { return };
    let _ = std::fs::remove_file(&marker);
    let report = text.lines().next().unwrap_or("").trim().to_string();
    let folder = Path::new(&report).parent().map(Path::to_path_buf).filter(|p| p.is_dir()).unwrap_or_else(|| dir.join("crashes"));
    if !ask_open(&folder, &report) {
        return;
    }
    open_folder(&folder);
}

fn ask_open(folder: &Path, report: &str) -> bool {
    match unsafe { task_dialog(report) } {
        Some(open) => open,
        None => message_fallback(folder, report),
    }
}

unsafe fn task_dialog(report: &str) -> Option<bool> {
    use windows::Win32::UI::Controls::{
        TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOGCONFIG_1, TASKDIALOG_BUTTON, TDCBF_CLOSE_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION,
    };
    let title = HSTRING::from("Maguro");
    let head = HSTRING::from(crate::tr(crate::Key::MaguroClosedUnexpectedlyLastTime));
    let body = HSTRING::from(format!(
        "{report}\n\n{}",
        crate::tr(crate::Key::ReportIsInFolderAboveAddDll)
    ));
    let open = HSTRING::from(crate::tr(crate::Key::OpenFolder));
    let button = TASKDIALOG_BUTTON { nButtonID: 100, pszButtonText: PCWSTR(open.as_ptr()) };
    let cfg = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: HWND::default(),
        hInstance: HINSTANCE::default(),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        dwCommonButtons: TDCBF_CLOSE_BUTTON,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: PCWSTR::null() },
        pszMainInstruction: PCWSTR(head.as_ptr()),
        pszContent: PCWSTR(body.as_ptr()),
        cButtons: 1,
        pButtons: &button,
        nDefaultButton: 8, // IDCLOSE
        cRadioButtons: 0,
        pRadioButtons: std::ptr::null(),
        nDefaultRadioButton: 0,
        pszVerificationText: PCWSTR::null(),
        pszExpandedInformation: PCWSTR::null(),
        pszExpandedControlText: PCWSTR::null(),
        pszCollapsedControlText: PCWSTR::null(),
        Anonymous2: TASKDIALOGCONFIG_1 { hFooterIcon: HICON::default() },
        pszFooter: PCWSTR::null(),
        pfCallback: None,
        lpCallbackData: 0,
        cxWidth: 0,
    };
    let mut pressed = 0i32;
    if TaskDialogIndirect(&cfg, Some(&mut pressed), None, None).is_err() {
        return None;
    }
    Some(pressed == 100)
}

fn message_fallback(_folder: &Path, report: &str) -> bool {
    let text = HSTRING::from(format!(
        "{}\n\n{report}\n\n{}",
        crate::tr(crate::Key::MaguroClosedUnexpectedlyLastTime),
        crate::tr(crate::Key::OpenFolder2),
    ));
    unsafe { MessageBoxW(None, &text, w!("Maguro"), MB_YESNO | MB_ICONWARNING) == IDYES }
}

fn open_folder(folder: &Path) {
    let path = HSTRING::from(folder.to_string_lossy().as_ref());
    unsafe {
        let r = ShellExecuteW(None, w!("explore"), &path, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if (r.0 as isize) <= 32 {
            let _ = ShellExecuteW(None, w!("open"), &path, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_kill_exit_is_not_fatal_and_fail_fast_still_is() {
        assert!(!is_fatal(0xFFFF_FFFF), "Stop-Process -Force");
        assert!(is_fatal(0xC000_0409), "STATUS_STACK_BUFFER_OVERRUN");
        assert!(!watch_is_fatal(0xFFFF_FFFF), "Stop-Process -Force");
        assert!(!watch_is_fatal(0xC000_013A), "CTRL+C");
        assert!(watch_is_fatal(0xC000_0409), "fail-fast");
    }

    #[test]
    fn report_names_the_module_and_the_shell_dll() {
        let text = format_report(&Report {
            version: "0.1.25".into(),
            thread: "main".into(),
            kind: "exception".into(),
            detail: String::new(),
            location: String::new(),
            exception: "0xC0000005".into(),
            address: "0x7FF60010".into(),
            module: r"C:\Program Files\Codec\bad.dll+0x1A2B".into(),
            last_path: r"F:\CAC\サンプル".into(),
            shell_loaded: vec![r"C:\Program Files\Codec\codec.dll".into()],
            third_party: vec![r"C:\Program Files\Codec\bad.dll".into(), r"C:\Program Files\Codec\codec.dll".into()],
            recent: vec!["nav F:\\CAC\\サンプル".into()],
        });
        assert!(text.contains("Maguro 0.1.25"), "{text}");
        assert!(text.contains("exception: 0xC0000005"), "{text}");
        assert!(text.contains(r"bad.dll+0x1A2B"), "{text}");
        assert!(text.contains(r"F:\CAC\サンプル"), "{text}");
        assert!(text.contains("codec.dll"), "{text}");
        assert!(text.contains("blocked_shell_extensions"), "{text}");
        assert!(text.contains("Explorer is unchanged"), "{text}");
    }

    #[test]
    fn system_modules_are_not_third_party() {
        let paths = vec![
            r"C:\Windows\System32\shell32.dll".into(),
            r"C:\Windows\SysWOW64\propsys.dll".into(),
            r"C:\Program Files\Maguro\maguro.exe".into(),
            r"C:\Program Files\Codec\codec.dll".into(),
        ];
        assert!(is_system_module(r"C:\Windows\System32\shell32.dll"));
        assert!(is_system_module(r"C:\Windows\WinSxS\amd64_foo\x.dll"));
        assert!(!is_system_module(r"C:\Program Files\Codec\codec.dll"));
        let third = third_party_modules(&paths);
        assert_eq!(third, vec![r"C:\Program Files\Codec\codec.dll".to_string()]);
        let hit = matching_shell(&paths, &["codec.dll".into(), "shell32.dll".into()]);
        assert_eq!(hit, vec![r"C:\Program Files\Codec\codec.dll".to_string()]);
    }

    #[test]
    fn catch_wnd_marks_and_clears_the_window_proc() {
        assert!(!super::in_wndproc());
        let n = super::catch_wnd(|| {
            assert!(super::in_wndproc());
            LRESULT(3)
        });
        assert_eq!(n.0, 3);
        let _ = super::catch_wnd(|| panic!("wnd"));
        assert!(!super::in_wndproc());
    }

    #[test]
    fn quiet_reentry_returns_the_fallback() {
        let got = super::catch_ui(4u8, || super::skip_reenter("app() already borrowed"));
        assert_eq!(got, 4);
        let wnd = super::catch_wnd(|| super::skip_reenter("app() already borrowed"));
        assert_eq!(wnd.0, 0);
        assert!(!super::in_wndproc());
    }

    #[test]
    fn ui_thread_panic_is_caught_and_does_not_exit() {
        let got = super::catch_ui(0u32, || panic!("end byte index 8 is not a char boundary; inside サ"));
        assert_eq!(got, 0);
        let wnd = super::catch_wnd(|| panic!("한글폴더"));
        assert_eq!(wnd.0, 0);
        let flag = super::catch_bool(|| panic!("サンプル"));
        assert_eq!(flag.0, 0);
        let n = super::catch_i32(|| panic!("😀"));
        assert_eq!(n, 0);
    }

    #[test]
    fn payload_is_the_panic_value_not_the_box() {
        let lit: Box<dyn std::any::Any + Send> = Box::new("already borrowed: BorrowMutError");
        // v0.1.38 passed `&payload`. That coerces to the Box, so the dialog said "panic".
        assert_eq!(panic_payload(&lit), "Box<dyn Any>");
        assert_eq!(panic_payload(&*lit), "already borrowed: BorrowMutError");
        let owned: Box<dyn std::any::Any + Send> = Box::new(String::from("index out of bounds: the len is 1 but the index is 0"));
        assert_eq!(panic_payload(&*owned), "index out of bounds: the len is 1 but the index is 0");
        let other: Box<dyn std::any::Any + Send> = Box::new(7u8);
        assert_eq!(panic_payload(&*other), "Box<dyn Any>");
    }

    #[test]
    fn dialog_shows_the_message_and_the_location() {
        let payload: Box<dyn std::any::Any + Send> = Box::new("already borrowed: BorrowMutError");
        let (message, detail) = caught_panic_text("already borrowed: BorrowMutError", "src/app/state.rs:667", &*payload);
        assert_eq!(message, "already borrowed: BorrowMutError");
        assert_eq!(detail, "already borrowed: BorrowMutError\nsrc/app/state.rs:667");
        let (message, detail) = caught_panic_text("", "", &*payload);
        assert_eq!(message, "already borrowed: BorrowMutError");
        assert_eq!(detail, "already borrowed: BorrowMutError");
        assert_eq!(panic_dialog_detail("Box<dyn Any>", "src/ui/favpanel.rs:292"), "Box<dyn Any>\nsrc/ui/favpanel.rs:292");
        assert_eq!(panic_dialog_detail("", ""), "Box<dyn Any>");
        let long = "assertion `left == right` failed\n  left: 1\n right: 2";
        assert_eq!(panic_dialog_detail(long, "src/ui/favpanel.rs:10"), "assertion `left == right` failed\nsrc/ui/favpanel.rs:10");
    }

    #[test]
    fn panic_report_is_written_in_the_crashes_folder() {
        let dir = std::env::temp_dir().join(format!("maguro-crash-{}-{}", std::process::id(), stamp()));
        let _ = std::fs::remove_dir_all(&dir);
        let text = format_report(&Report {
            version: "0.1.45".into(),
            thread: "main".into(),
            kind: "panic".into(),
            detail: "already borrowed: BorrowMutError".into(),
            location: "src/app/state.rs:602".into(),
            exception: String::new(),
            address: String::new(),
            module: "unknown".into(),
            last_path: String::new(),
            shell_loaded: Vec::new(),
            third_party: Vec::new(),
            recent: vec!["ui panic: already borrowed: BorrowMutError".into()],
        });
        let path = write_text_report(&dir, &text).expect("crash file");
        assert!(path.starts_with(dir.join("crashes")), "{}", path.display());
        assert!(path.file_name().unwrap().to_string_lossy().starts_with("crash-"));
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("kind: panic"), "{body}");
        assert!(body.contains("message: already borrowed: BorrowMutError"), "{body}");
        assert!(body.contains("location: src/app/state.rs:602"), "{body}");
        let marker = std::fs::read_to_string(dir.join("crash-pending.txt")).unwrap();
        assert!(marker.contains("crashes"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
