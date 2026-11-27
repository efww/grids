use crate::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::System::Memory::GlobalSize;
use windows::Win32::System::Power::{PowerClearRequest, PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest};
use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{GetCurrentProcessId, POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0};
use std::sync::atomic::{AtomicI32, AtomicIsize};

// ---------- background paste ----------

// Copies running on worker threads; the status line says so and skips its disk calls meanwhile.
pub(crate) static COPYING: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct Busy;

impl Busy {
    pub(crate) fn new() -> Busy {
        COPYING.fetch_add(1, Ordering::Relaxed);
        Busy
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        COPYING.fetch_sub(1, Ordering::Relaxed);
    }
}

// ---------- Windows' own file operations outlive the window ----------

// Cut + paste, drags between panes and big deletes run on the shell's threads in this process, not on JOBS.
// Each holds a reference on this count while it runs (SHSetInstanceExplorer, as Explorer does), and closing
// Grids waits for it. A static, so a thread still running after the 30 s give-up never writes into a dead stack.
pub(crate) static SHELL_REFS: AtomicI32 = AtomicI32::new(0);
// A shutdown block reason is up on the main window (WM_QUERYENDSESSION said no).
pub(crate) static BLOCKING: AtomicBool = AtomicBool::new(false);
const GIVE_UP: Duration = Duration::from_secs(30);

// Called once at start; the object is kept until wait_shell_ops.
pub(crate) unsafe fn hold_shell_refs() -> Option<IUnknown> {
    let mut unk = None;
    SHCreateThreadRef(SHELL_REFS.as_ptr(), &mut unk).ok()?;
    let unk = unk?;
    SHSetInstanceExplorer(&unk);
    SHELL_BASE.store(SHELL_REFS.load(Ordering::Acquire), Ordering::Relaxed);
    Some(unk)
}

// After the message loop: our two references go, then whatever the shell still holds is waited out.
// No window of this process seen for 30 s (not even a progress window) = a reference nobody gives back.
pub(crate) fn keep_waiting(refs: i32, unseen: Duration) -> bool {
    refs > 0 && unseen < GIVE_UP
}

pub(crate) unsafe fn wait_shell_ops(held: Option<IUnknown>) {
    let Some(held) = held else { return };
    SHSetInstanceExplorer(None::<&IUnknown>);
    drop(held);
    // a handle that never fires: CoWaitForMultipleHandles fails at once with none, and 200 ms is the poll
    let Ok(ev) = CreateEventW(None, true, false, None) else { return };
    let mut seen = Instant::now();
    loop {
        if own_windows().0 {
            seen = Instant::now();
        }
        if !keep_waiting(SHELL_REFS.load(Ordering::Acquire), seen.elapsed()) {
            break;
        }
        // RPC_S_CALLPENDING (the time-out) is the normal way out; COM calls and messages keep moving meanwhile
        _ = CoWaitForMultipleHandles((COWAIT_DISPATCH_CALLS | COWAIT_DISPATCH_WINDOW_MESSAGES).0 as u32, 200, &[ev]);
    }
    _ = CloseHandle(ev);
}

// The shell's copy / move / delete progress window.
pub(crate) unsafe fn is_progress_window(h: HWND) -> bool {
    let mut cls = [0u16; 32];
    let n = GetClassNameW(h, &mut cls) as usize;
    String::from_utf16_lossy(&cls[..n]) == "OperationStatusWindow"
}

// (a visible top-level window of this process, a progress window of this process). Explorer's own copies
// are not ours: they must not hold up a restart.
pub(crate) unsafe fn own_windows() -> (bool, bool) {
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let found = &mut *(lp.0 as *mut (bool, bool));
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == GetCurrentProcessId() && IsWindowVisible(h).as_bool() {
            found.0 = true;
            found.1 |= is_progress_window(h);
        }
        TRUE
    }
    let mut found = (false, false);
    _ = EnumWindows(Some(each), LPARAM(&mut found as *mut (bool, bool) as isize));
    found
}

// Restart / sign-out: say no while a copy of ours runs. Only then, so an open Properties window doesn't block it.
pub(crate) fn block_shutdown(copying: usize, progress_window: bool) -> bool {
    copying > 0 || progress_window
}

// WM_QUERYENDSESSION on the main window (the reason can only be set from the thread that made it).
// A critical end (ENDSESSION_CRITICAL) can't be refused, so it isn't. Returns the answer: TRUE = go ahead.
pub(crate) unsafe fn query_end_session(h: HWND, lp: LPARAM) -> LRESULT {
    let critical = lp.0 as u32 & ENDSESSION_CRITICAL != 0;
    if !critical && block_shutdown(COPYING.load(Ordering::Relaxed), own_windows().1) {
        let why = tr("Copying or moving files — wait for it to finish", "파일 복사·이동 중 — 끝날 때까지 기다려 주세요");
        if ShutdownBlockReasonCreate(h, &HSTRING::from(why)).is_ok() {
            BLOCKING.store(true, Ordering::Relaxed);
        }
        return LRESULT(0);
    }
    release_shutdown(h);
    LRESULT(1)
}

// The restart was cancelled, or the copy is over (TIMER_STATUS): an old reason must not block the next restart.
pub(crate) unsafe fn release_shutdown(h: HWND) {
    if BLOCKING.swap(false, Ordering::Relaxed) {
        _ = ShutdownBlockReasonDestroy(h);
    }
}

pub(crate) unsafe fn shutdown_tick(h: HWND) {
    if BLOCKING.load(Ordering::Relaxed) && !block_shutdown(COPYING.load(Ordering::Relaxed), own_windows().1) {
        release_shutdown(h);
    }
}

// ---------- no idle sleep while files move (TeraCopy's "Prevent sleep mode", FreeFileSync) ----------

// A start_copy's run_copy going on a worker thread. Counted apart from Busy: renames, date sorting and the
// like are short or sit waiting on a question, and the failure message box comes after this is let go.
pub(crate) static COPY_RUNS: AtomicUsize = AtomicUsize::new(0);
// SHELL_REFS just after hold_shell_refs: only above it can a shell copy (drag, cut + paste) be running.
// MAX = never held, so the shell's progress window isn't looked for.
static SHELL_BASE: AtomicI32 = AtomicI32::new(i32::MAX);
// The power request, made on first use and kept until the process ends (0 = not yet, -1 = could not be made).
// A HANDLE isn't Sync: kept as its value.
static POWER_REQ: AtomicIsize = AtomicIsize::new(0);
// The request is set now. PowerSetRequest counts, so Set and Clear go strictly in pairs, from the UI thread only.
pub(crate) static AWAKE: AtomicBool = AtomicBool::new(false);

pub(crate) struct CopyRun;

impl CopyRun {
    pub(crate) fn new() -> CopyRun {
        COPY_RUNS.fetch_add(1, Ordering::Relaxed);
        CopyRun
    }
}

impl Drop for CopyRun {
    fn drop(&mut self) {
        COPY_RUNS.fetch_sub(1, Ordering::Relaxed);
    }
}

// The shell may be copying for us (a reference above the one we hold): only then is EnumWindows worth a tick.
pub(crate) fn shell_op_maybe(refs: i32, base: i32) -> bool {
    refs > base
}

unsafe fn power_request() -> Option<HANDLE> {
    let mut h = POWER_REQ.load(Ordering::Relaxed);
    if h == 0 {
        // shown to an administrator by powercfg /requests; the string is copied, so it need only last the call
        let why = HSTRING::from(tr("Grids: copying or moving files", "Grids: 파일 복사·이동 중"));
        let ctx = REASON_CONTEXT {
            Version: 0, // POWER_REQUEST_CONTEXT_VERSION
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 { SimpleReasonString: PWSTR(why.as_ptr() as *mut u16) },
        };
        h = PowerCreateRequest(&ctx).map_or(-1, |r| r.0 as isize);
        POWER_REQ.store(h, Ordering::Relaxed);
    }
    (h != -1).then_some(HANDLE(h as *mut _))
}

// Keeps the system out of idle sleep (the screen may still turn off). Only calls Windows when the state changes;
// a failed Set leaves AWAKE off (the status line then doesn't claim it), a failed Clear is tried again next tick.
pub(crate) unsafe fn stay_awake(on: bool) {
    if on == AWAKE.load(Ordering::Relaxed) {
        return;
    }
    let Some(req) = power_request() else { return };
    if on {
        if PowerSetRequest(req, PowerRequestSystemRequired).is_ok() {
            AWAKE.store(true, Ordering::Relaxed);
        }
    } else if PowerClearRequest(req, PowerRequestSystemRequired).is_ok() {
        AWAKE.store(false, Ordering::Relaxed);
    }
}

// TIMER_STATUS (also the slow tick while minimized): a copy of ours, or the shell's progress window, holds sleep
// off. After WM_DESTROY the timer is gone and the last state stays until the process ends, which drops it.
pub(crate) unsafe fn awake_tick() {
    let runs = COPY_RUNS.load(Ordering::Relaxed);
    let shell = runs == 0
        && shell_op_maybe(SHELL_REFS.load(Ordering::Acquire), SHELL_BASE.load(Ordering::Relaxed))
        && own_windows().1;
    stay_awake(block_shutdown(runs, shell));
}

thread_local! {
    // joined after the message loop so closing Grids doesn't cut a copy short
    pub(crate) static JOBS: RefCell<Vec<std::thread::JoinHandle<()>>> = const { RefCell::new(Vec::new()) };
}

// Ctrl+V in a file list: a held key's repeats are swallowed; the first press starts a copy, or goes
// to the shell when `start` can't take it. True = handled, don't pass it on.
pub(crate) fn paste_key(repeat: bool, start: impl FnOnce() -> bool) -> bool {
    repeat || start()
}

// Some(true): every source sits in `dst` (copy with "- Copy" names), Some(false): none does,
// None: empty, not a file system path or mixed (left to the shell).
pub(crate) fn paste_mode(srcs: &[String], dst: &str) -> Option<bool> {
    let dst = dst.trim_end_matches('\\').to_lowercase();
    let parent = |p: &String| Some(p.trim_end_matches('\\').rsplit_once('\\')?.0.trim_end_matches('\\').to_lowercase());
    let same: Vec<bool> = srcs.iter().map(|p| if is_fs(p) { parent(p).map(|d| d == dst) } else { None }).collect::<Option<_>>()?;
    match (same.iter().all(|&s| s), same.iter().any(|&s| s)) {
        _ if same.is_empty() => None,
        (true, _) => Some(true),
        (false, false) => Some(false),
        _ => None,
    }
}

// The ⇄ menu's copy / move: like a paste, but a move into the folder the files already sit in is refused.
pub(crate) fn send_mode(srcs: &[String], dst: &str, mv: bool) -> Option<bool> {
    paste_mode(srcs, dst).filter(|&same| !(mv && same))
}

// The ⇄ menu sends a whole selection or nothing: never a partial list from a capped or unreadable read.
pub(crate) const SEND_MAX: u32 = 10_000;

#[derive(Debug, PartialEq)]
pub(crate) enum Pick {
    Empty,
    TooMany,
    Unreadable,
}

pub(crate) fn collect_all(n: u32, mut get: impl FnMut(u32) -> Option<String>) -> std::result::Result<Vec<String>, Pick> {
    match n {
        0 => Err(Pick::Empty),
        n if n > SEND_MAX => Err(Pick::TooMany),
        n => (0..n).map(|k| get(k).ok_or(Pick::Unreadable)).collect(),
    }
}

// Ctrl+Shift+C in a file list: the selected items' paths (or the folder's), quoted, one per line.
// Virtual items without a file path are skipped. The clipboard is only touched once the text is ready.
pub(crate) unsafe fn copy_paths(a: usize) {
    let Some(fv) = view_of::<IFolderView2>(a) else { return };
    let path = |it: IShellItem| it.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p));
    let mut paths: Vec<String> = Vec::new();
    if let Ok(arr) = fv.GetSelection(false) {
        paths = (0..arr.GetCount().unwrap_or(0)).filter_map(|k| path(arr.GetItemAt(k).ok()?)).collect();
    }
    if fv.ItemCount(SVGIO_SELECTION).unwrap_or(0) == 0 {
        paths = fv.GetFolder::<IShellItem>().ok().and_then(path).into_iter().collect();
    }
    if paths.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let text: Vec<u16> = paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("\r\n").encode_utf16().chain([0]).collect();
    let main = app(|x| x.main);
    let mut ok = false;
    if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2) {
        let p = GlobalLock(h) as *mut u16;
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
            _ = GlobalUnlock(h);
            if OpenClipboard(Some(main)).is_ok() {
                ok = EmptyClipboard().is_ok() && SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0))).is_ok();
                _ = CloseClipboard();
            }
        }
        if !ok {
            _ = GlobalFree(Some(h)); // after a successful SetClipboardData the system owns it
        }
    }
    if !ok {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let n = paths.len();
    let note = match (KO.load(Ordering::Relaxed), n) {
        (true, 1) => "경로 복사함".into(),
        (true, n) => format!("경로 {n}개 복사함"),
        (false, 1) => "Copied path".into(),
        (false, n) => format!("Copied {n} paths"),
    };
    app(|x| {
        x.note = Some((note, Instant::now() + Duration::from_secs(3)));
        x.lost = 0;
    });
    update_status();
}


// Plain file copies (Ctrl+C, then Ctrl+V into a folder) run on their own thread, so Grids stays usable.
// Cut, links, virtual items and anything else stay with the shell.
pub(crate) unsafe fn try_paste(a: usize) -> bool {
    let (dst, main) = app(|x| (x.s.paths[a].clone(), x.main));
    if !is_fs(&dst) || OpenClipboard(Some(main)).is_err() {
        return false;
    }
    let mut srcs = Vec::new();
    if let Ok(h) = GetClipboardData(CF_HDROP.0 as u32) {
        let drop = HDROP(h.0);
        for k in 0..DragQueryFileW(drop, u32::MAX, None) {
            let mut buf = vec![0u16; DragQueryFileW(drop, k, None) as usize + 1];
            let n = DragQueryFileW(drop, k, Some(&mut buf)) as usize;
            srcs.push(String::from_utf16_lossy(&buf[..n]));
        }
    }
    let mut effect = None;
    if let Ok(h) = GetClipboardData(RegisterClipboardFormatW(w!("Preferred DropEffect"))) {
        let p = GlobalLock(HGLOBAL(h.0)) as *const u32;
        if !p.is_null() {
            effect = Some(*p);
            _ = GlobalUnlock(HGLOBAL(h.0));
        }
    }
    let has = |f: PCWSTR| IsClipboardFormatAvailable(RegisterClipboardFormatW(f)).is_ok();
    let (idlist, fgd) = (has(w!("Shell IDList Array")), has(w!("FileGroupDescriptorW")) || has(w!("FileGroupDescriptor")));
    let has_text = IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok();
    let text = text_only(!srcs.is_empty(), idlist, fgd, has_text, clip_private()).then(|| clip_text()).flatten();
    let pic = (text.is_none() && picture_only(!srcs.is_empty(), idlist, fgd, has_text)).then(|| clip_picture()).flatten();
    _ = CloseClipboard();
    match text {
        Some(Ok(t)) if !t.trim().is_empty() => {
            save_text(a, &dst, &t);
            return true;
        }
        Some(Err(e)) => {
            text_failed(&e);
            return true;
        }
        _ => {}
    }
    if let Some(pic) = pic {
        return start_picture(pic, dst);
    }
    let copy = effect.is_none_or(|e| e & DROPEFFECT_MOVE.0 == 0 && e & DROPEFFECT_COPY.0 != 0);
    let Some(rename) = paste_mode(&srcs, &dst).filter(|_| copy) else { return false };
    start_copy(srcs, dst, rename, false)
}

// ---------- a copied picture pasted as a PNG file ----------

// Only a bare picture: files, virtual files (Outlook attachments) and text (Office copies) stay with the shell.
pub(crate) fn picture_only(has_hdrop_items: bool, has_idlist: bool, has_fgd: bool, has_text: bool) -> bool {
    !(has_hdrop_items || has_idlist || has_fgd || has_text)
}

// ---------- copied text pasted as a .txt file ----------

// Text alone (a picture beside it, as Office copies carry, doesn't count): files and virtual files stay with
// the shell, and so does text a password manager marked to be kept out of clipboard monitors and history.
pub(crate) fn text_only(has_hdrop_items: bool, has_idlist: bool, has_fgd: bool, has_text: bool, private: bool) -> bool {
    has_text && !(has_hdrop_items || has_idlist || has_fgd || private)
}

// "ExcludeClipboardContentFromMonitorProcessing" present, or "CanIncludeInClipboardHistory" set to 0. Clipboard open.
unsafe fn clip_private() -> bool {
    if IsClipboardFormatAvailable(RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"))).is_ok() {
        return true;
    }
    let Ok(h) = GetClipboardData(RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"))) else { return false };
    let h = HGLOBAL(h.0);
    let p = GlobalLock(h) as *const u32;
    if p.is_null() {
        return false;
    }
    let no = GlobalSize(h) >= 4 && p.read_unaligned() == 0;
    _ = GlobalUnlock(h);
    no
}

pub(crate) const TEXT_MAX: usize = 64 << 20;

// CF_UNICODETEXT up to its first NUL (GlobalSize rounds up, so what follows it is padding). Clipboard open.
// None = unreadable; Err = too big to save.
unsafe fn clip_text() -> Option<std::result::Result<String, String>> {
    let h = HGLOBAL(GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?.0);
    let size = GlobalSize(h);
    if size > TEXT_MAX {
        return Some(Err(format!("{} MB > {} MB", size >> 20, TEXT_MAX >> 20)));
    }
    let p = GlobalLock(h) as *const u16;
    if p.is_null() {
        return None;
    }
    let all = std::slice::from_raw_parts(p, size / 2);
    let n = all.iter().position(|&c| c == 0).unwrap_or(all.len());
    let text = String::from_utf16_lossy(&all[..n]);
    _ = GlobalUnlock(h);
    Some(Ok(text))
}

// Characters Windows refuses in a file name.
const NOT_IN_NAMES: &str = "\\/:*?\"<>|";

// The file's name: the first non-blank line, characters Windows refuses turned to spaces, runs of spaces made
// one, cut to 40 characters, no '.' or ' ' at the end. Blank or a reserved device name (CON, COM1, … also before
// a '.') gives "Clipboard text 2026-11-26 153012". Then " (k)" from the second try on, and ".txt".
pub(crate) fn text_name(ko: bool, text: &str, t: &SYSTEMTIME, k: u32) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut stem = String::new();
    let mut len = 0;
    for c in line.chars() {
        if len == 40 {
            break;
        }
        let space = c.is_whitespace() || c.is_control() || NOT_IN_NAMES.contains(c);
        if space && (stem.is_empty() || stem.ends_with(' ')) {
            continue;
        }
        stem.push(if space { ' ' } else { c });
        len += 1;
    }
    let stem = stem.trim_end_matches(['.', ' ']);
    let device = stem.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let numbered = |p: &str| device.len() == 4 && device.starts_with(p) && matches!(device.as_bytes()[3], b'1'..=b'9');
    let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&device.as_str()) || numbered("COM") || numbered("LPT");
    let stem = if stem.is_empty() || reserved {
        let head = if ko { "클립보드 텍스트" } else { "Clipboard text" };
        format!("{head} {:04}-{:02}-{:02} {:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
    } else {
        stem.to_string()
    };
    let n = if k >= 2 { format!(" ({k})") } else { String::new() };
    format!("{stem}{n}.txt")
}

// As Shift+F7: a new file (UTF-8, line breaks as copied, never over an existing one), listed, selected and in rename.
unsafe fn save_text(a: usize, dst: &str, text: &str) {
    let (ko, now) = (KO.load(Ordering::Relaxed), GetLocalTime());
    match save_unique(Path::new(dst), |k| text_name(ko, text, &now, k), text.as_bytes()) {
        Ok(name) => {
            let full = Path::new(dst).join(name).to_string_lossy().into_owned();
            let wide = HSTRING::from(full.as_str());
            SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
            rename_soon((a, full, 40, None));
        }
        Err(e) => text_failed(&e.to_string()),
    }
}

unsafe fn text_failed(why: &str) {
    _ = MessageBeep(MB_ICONWARNING);
    let what = tr("Couldn't save the clipboard text: ", "클립보드 글을 저장하지 못했습니다: ");
    MessageBoxW(Some(app(|x| x.main)), &HSTRING::from(format!("{what}{why}")), w!("Grids"), MB_ICONWARNING);
}

pub(crate) enum Pic {
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

// The clipboard's "PNG" bytes (Snipping Tool, browsers: keeps transparency), else its CF_DIB. Clipboard open.
unsafe fn clip_picture() -> Option<Pic> {
    let bytes = |fmt: u32| {
        let h = HGLOBAL(GetClipboardData(fmt).ok()?.0);
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(p, GlobalSize(h)).to_vec();
        _ = GlobalUnlock(h);
        Some(v)
    };
    if let Some(mut png) = bytes(RegisterClipboardFormatW(w!("PNG"))).filter(|b| b.starts_with(b"\x89PNG\r\n\x1a\n")) {
        // GlobalSize rounds up: drop the padding after the IEND chunk
        if let Some(i) = png.windows(4).rposition(|w| w == b"IEND").filter(|i| i + 8 <= png.len()) {
            png.truncate(i + 8);
        }
        return Some(Pic::Png(png));
    }
    bytes(CF_DIB.0 as u32).map(Pic::Dib)
}

// A CF_DIB as a .bmp file: BITMAPFILEHEADER + the DIB. None when the header or palette is cut short.
pub(crate) fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |i: usize| Some(u32::from_le_bytes(dib.get(i..i + 4)?.try_into().ok()?));
    let size = u32_at(0)? as usize;
    let bits = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
    let (compression, used) = (u32_at(16)?, u32_at(32)? as usize);
    if size < 40 {
        return None;
    }
    let colors = if used > 0 { used } else if bits <= 8 { 1 << bits } else { 0 };
    let masks = if size == 40 && compression == 3 { 12 } else { 0 }; // BI_BITFIELDS after a plain BITMAPINFOHEADER
    let off = colors.checked_mul(4)?.checked_add(size + masks)?;
    if off >= dib.len() {
        return None;
    }
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&u32::try_from(14 + dib.len()).ok()?.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&(14 + off as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

// "Clipboard image 2026-10-21 153012.png", " (k)" from the second try on.
pub(crate) fn image_name(ko: bool, t: &SYSTEMTIME, k: u32) -> String {
    let head = if ko { "클립보드 이미지" } else { "Clipboard image" };
    let n = if k >= 2 { format!(" ({k})") } else { String::new() };
    format!("{head} {:04}-{:02}-{:02} {:02}{:02}{:02}{n}.png", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

// Writes `bytes` under the first free name_of(1..=999); never replaces a file. Returns the name used.
pub(crate) fn save_unique(dir: &Path, name_of: impl Fn(u32) -> String, bytes: &[u8]) -> std::io::Result<String> {
    use std::io::{ErrorKind, Write};
    for k in 1..=999 {
        let name = name_of(k);
        let path = dir.join(&name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(bytes) {
                    drop(f);
                    _ = std::fs::remove_file(&path); // only the file just created
                    return Err(e);
                }
                return Ok(name);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(ErrorKind::AlreadyExists.into())
}

// The status line shows this on its next tick (set by the worker, taken by update_status).
pub(crate) static SAVED: Mutex<Option<String>> = Mutex::new(None);

// Encodes and saves on a worker thread; the panes, views, focus and selection are left alone
// (the view lists the new file itself). False = the thread didn't start.
pub(crate) unsafe fn start_picture(pic: Pic, dst: String) -> bool {
    let ko = KO.load(Ordering::Relaxed);
    let job = std::thread::Builder::new().spawn(move || {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let png = match pic {
            Pic::Png(b) => Ok(b),
            Pic::Dib(d) => dib_to_bmp(&d).ok_or_else(|| tr("unreadable picture", "읽을 수 없는 그림").to_string()).and_then(|b| to_png(&b).map_err(|e| e.message())),
        };
        let now = GetLocalTime();
        match png.and_then(|b| save_unique(Path::new(&dst), |k| image_name(ko, &now, k), &b).map_err(|e| e.to_string())) {
            Ok(name) => *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(if ko { format!("{name} 저장함") } else { format!("Saved {name}") }),
            Err(e) => {
                let what = tr("Couldn't save the clipboard picture: ", "클립보드 그림을 저장하지 못했습니다: ");
                MessageBoxW(None, &HSTRING::from(format!("{what}{e}")), w!("Grids"), MB_ICONWARNING);
            }
        }
        if com {
            CoUninitialize();
        }
    });
    let Ok(job) = job else { return false };
    keep_job(job);
    true
}

// A .bmp file's bytes re-encoded as PNG with WIC, all in memory.
unsafe fn to_png(bmp: &[u8]) -> Result<Vec<u8>> {
    let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
    let input = SHCreateMemStream(Some(bmp)).ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
    let src = wic.CreateDecoderFromStream(&input, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?.GetFrame(0)?;
    let out = SHCreateMemStream(None).ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
    let enc = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
    enc.Initialize(&out, WICBitmapEncoderNoCache)?;
    let (mut frame, mut props) = (None, None);
    enc.CreateNewFrame(&mut frame, &mut props)?;
    let frame = frame.ok_or_else(|| Error::from(E_FAIL))?;
    frame.Initialize(props.as_ref())?;
    frame.WriteSource(&src, std::ptr::null())?;
    frame.Commit()?;
    enc.Commit()?;
    let mut len = 0u64;
    out.Seek(0, STREAM_SEEK_END, Some(&mut len))?;
    out.Seek(0, STREAM_SEEK_SET, None)?;
    let mut png = vec![0u8; len as usize];
    let mut read = 0u32;
    out.Read(png.as_mut_ptr().cast(), len as u32, Some(&mut read)).ok()?;
    png.truncate(read as usize);
    Ok(png)
}

pub(crate) fn keep_job(job: std::thread::JoinHandle<()>) {
    JOBS.with_borrow_mut(|j| {
        j.retain(|j| !j.is_finished());
        j.push(job);
    });
}

// Copies (or, with `mv`, moves) `srcs` into `dst` on a worker thread. False = the thread didn't start.
// Once it is done (not after an error or a cancel), the main window hears of it: WM_COPY_DONE.
pub(crate) unsafe fn start_copy(srcs: Vec<String>, dst: String, rename: bool, mv: bool) -> bool {
    let busy = Busy::new();
    // the worker can't use app(): the main window and the start time are taken here, on the UI thread
    let (main, start) = (app(|x| x.main.0 as isize), Instant::now());
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let res = {
            let _run = CopyRun::new(); // no idle sleep meanwhile; let go before a failure waits on its message box
            run_copy(&srcs, &dst, rename, mv)
        };
        match res {
            Ok(aborted) => {
                let done = CopyDone { mv, n: srcs.len(), to: short_name(&dst), aborted, took: start.elapsed() };
                COPY_DONE.lock().unwrap_or_else(|e| e.into_inner()).push(done);
                // Grids already closing: the post just fails (JOBS are joined after the message loop)
                _ = PostMessageW(Some(HWND(main as _)), WM_COPY_DONE, WPARAM(0), LPARAM(0));
            }
            Err(e) => {
                if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
                    let what = if mv { tr("Moving failed: ", "이동하지 못했습니다: ") } else { tr("Copying failed: ", "복사하지 못했습니다: ") };
                    let text = HSTRING::from(format!("{what}{}", e.message()));
                    MessageBoxW(None, &text, w!("Grids"), MB_ICONWARNING);
                }
            }
        }
        if com {
            CoUninitialize();
        }
    });
    let Ok(job) = job else { return false };
    keep_job(job);
    true
}

// No owner window: the progress and conflict dialogs belong to the worker thread.
// Ok(true) = done, but something was skipped or cancelled on the way.
pub(crate) unsafe fn run_copy(srcs: &[String], dst: &str, rename: bool, mv: bool) -> Result<bool> {
    let op = run_op(srcs, dst, rename, mv)?;
    Ok(op.GetAnyOperationsAborted().map_or(true, |b| b.as_bool()))
}

// ---------- a finished copy / move says so (TeraCopy, a browser's "download complete") ----------

pub(crate) const WM_COPY_DONE: u32 = WM_APP + 56; // a start_copy is over; the details stay in COPY_DONE

pub(crate) struct CopyDone {
    pub(crate) mv: bool,
    pub(crate) n: usize,     // items picked (not the files inside folders)
    pub(crate) to: String,   // the folder's short name
    pub(crate) aborted: bool, // some skipped, or the rest cancelled: no count is claimed then
    pub(crate) took: Duration,
}

// a list, so two copies ending together both arrive (the last one is said)
static COPY_DONE: Mutex<Vec<CopyDone>> = Mutex::new(Vec::new());
// ended while another app was in front, or the main window minimized / hidden: said on the status line once
// Grids is back (WM_ACTIVATEAPP) or the main window is shown again (WM_SIZE)
static COPY_LATER: Mutex<Option<String>> = Mutex::new(None);

// Short jobs end before anyone looks away: only these get the taskbar / Windows notification.
pub(crate) const NOTICE_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq)]
pub(crate) struct Notice {
    pub(crate) now: bool,     // status line chip now (else kept until Grids is back in front)
    pub(crate) flash: bool,   // taskbar button flashes until Grids is in front
    pub(crate) balloon: bool, // a Windows notification from the notification area icon
}

// `hidden`: the window is off the taskbar (minimized to the notification area). The flash stays even with the
// icon there, so a blocked notification (Focus, Do not disturb) still leaves something; never neither.
pub(crate) fn done_notice(took: Duration, in_front: bool, hidden: bool, tray_on: bool) -> Notice {
    let loud = !in_front && took > NOTICE_AFTER;
    Notice { now: in_front, flash: loud && (!hidden || !tray_on), balloon: loud && tray_on }
}

pub(crate) fn done_text(ko: bool, d: &CopyDone) -> String {
    let (n, to) = (d.n, &d.to);
    match (ko, d.mv, d.aborted) {
        (true, false, false) => format!("복사 끝: {n}개 → {to}"),
        (true, true, false) => format!("이동 끝: {n}개 → {to}"),
        (true, false, true) => format!("복사 끝 → {to} (일부 건너뜀 또는 취소)"),
        (true, true, true) => format!("이동 끝 → {to} (일부 건너뜀 또는 취소)"),
        (false, false, false) if n == 1 => format!("Copied 1 item to {to}"),
        (false, true, false) if n == 1 => format!("Moved 1 item to {to}"),
        (false, false, false) => format!("Copied {n} items to {to}"),
        (false, true, false) => format!("Moved {n} items to {to}"),
        (false, false, true) => format!("Copy to {to} ended (some skipped or cancelled)"),
        (false, true, true) => format!("Move to {to} ended (some skipped or cancelled)"),
    }
}

// At most `cap` - 1 UTF-16 units (room for the terminating 0), never half a surrogate pair.
pub(crate) fn utf16_cut(s: &str, cap: usize) -> Vec<u16> {
    let mut w: Vec<u16> = s.encode_utf16().take(cap.saturating_sub(1)).collect();
    if w.last().is_some_and(|c| (0xD800..0xDC00).contains(c)) {
        w.pop();
    }
    w
}

// WM_COPY_DONE on the main window.
pub(crate) unsafe fn copy_done_msg(main: HWND) {
    let done: Vec<CopyDone> = std::mem::take(&mut *COPY_DONE.lock().unwrap_or_else(|e| e.into_inner()));
    let Some(d) = done.into_iter().last() else { return };
    let text = done_text(KO.load(Ordering::Relaxed), &d);
    let how = done_notice(d.took, app_in_front(), !IsWindowVisible(main).as_bool(), app(|a| a.tray_on));
    // Grids in front, but by another of its windows (a progress window, the preview) while the main one is
    // minimized or hidden: its status line is not seen, so keep the text for when it is shown again (WM_SIZE)
    if how.now && (IsIconic(main).as_bool() || !IsWindowVisible(main).as_bool()) {
        *COPY_LATER.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
        return;
    }
    if how.now {
        // the same way as a saved picture: the next status tick shows it
        *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
        update_status();
        return;
    }
    if how.balloon {
        tray_balloon(main, &text);
    }
    *COPY_LATER.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
    if how.flash {
        let f = FLASHWINFO { cbSize: size_of::<FLASHWINFO>() as u32, hwnd: main, dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG, uCount: 0, dwTimeout: 0 };
        _ = FlashWindowEx(&f);
    }
}

// Grids is in front again (WM_ACTIVATEAPP): a copy that ended meanwhile is said once on the status line.
pub(crate) unsafe fn copy_done_back() {
    if let Some(text) = COPY_LATER.lock().unwrap_or_else(|e| e.into_inner()).take() {
        *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
    }
}

// Another finished job the main window may not show now (Grids behind, minimized or hidden): said the same way,
// once Grids is back in front.
pub(crate) fn say_when_back(text: String) {
    *COPY_LATER.lock().unwrap_or_else(|e| e.into_inner()) = Some(text);
}

// run_copy, handing back the operation so the caller can ask whether anything was skipped or cancelled.
pub(crate) unsafe fn run_op(srcs: &[String], dst: &str, rename: bool, mv: bool) -> Result<IFileOperation> {
    let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
    let flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD;
    op.SetOperationFlags(if rename { flags | FOF_RENAMEONCOLLISION } else { flags })?;
    let to: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(dst), None)?;
    for s in srcs {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(s), None)?;
        if mv {
            op.MoveItem(&item, &to, PCWSTR::null(), None)?;
        } else {
            op.CopyItem(&item, &to, PCWSTR::null(), None)?;
        }
    }
    op.PerformOperations()?;
    Ok(op)
}

// ---------- ⇄ menu: copy / move to a folder no pane shows ----------

// Menu numbers under "Copy / Move N selected items to": 401… / 501… the favourite and recent folders listed
// (an index into send_targets), 450 / 550 "Choose another folder…". Clear of 1–100, 101–116, 201–216, 301–316,
// 1130+ and 3100+.
pub(crate) const ID_SEND_COPY: usize = 401;
pub(crate) const ID_SEND_MOVE: usize = 501;
pub(crate) const ID_CHOOSE_COPY: usize = 450;
pub(crate) const ID_CHOOSE_MOVE: usize = 550;
pub(crate) const SEND_SLOTS: usize = 40;
// Favourites have no cap of their own; the rest are a "Choose another folder…" away.
pub(crate) const SEND_FAVS: usize = 12;
pub(crate) const SEND_RECENT: usize = 8;

#[derive(Debug, PartialEq)]
pub(crate) enum SendPick {
    To(bool, usize), // (move, index into send_targets)
    Choose(bool),    // move
}

pub(crate) fn send_pick(p: usize) -> Option<SendPick> {
    match p {
        ID_CHOOSE_COPY => Some(SendPick::Choose(false)),
        ID_CHOOSE_MOVE => Some(SendPick::Choose(true)),
        p if (ID_SEND_COPY..ID_SEND_COPY + SEND_SLOTS).contains(&p) => Some(SendPick::To(false, p - ID_SEND_COPY)),
        p if (ID_SEND_MOVE..ID_SEND_MOVE + SEND_SLOTS).contains(&p) => Some(SendPick::To(true, p - ID_SEND_MOVE)),
        _ => None,
    }
}

fn dir_key(p: &str) -> String {
    p.trim_end_matches('\\').to_lowercase()
}

// (favourite, folder): favourites then recent folders, file system paths only, none a pane shows now
// (`shown`, the source pane's folder among them) and none twice. Strings only: no disk call, so an
// unreachable share can't stall the menu.
pub(crate) fn send_targets(shown: &[String], favs: &[String], recent: &[String]) -> Vec<(bool, String)> {
    let mut seen: Vec<String> = shown.iter().map(|p| dir_key(p)).collect();
    let mut out = Vec::new();
    for (fav, list, cap) in [(true, favs, SEND_FAVS), (false, recent, SEND_RECENT)] {
        let mut n = 0;
        for p in list.iter().map(|p| p.trim()).filter(|p| is_fs(p)) {
            if n == cap {
                break;
            }
            let key = dir_key(p);
            if !seen.contains(&key) {
                seen.push(key);
                out.push((fav, p.to_string()));
                n += 1;
            }
        }
    }
    out
}

// The selection's paths as into_itself compares them: worked out once per menu, not once per target row.
pub(crate) fn src_keys(srcs: &[String]) -> Vec<String> {
    srcs.iter().map(|s| dir_key(s)).collect()
}

// A selected folder (`keys`, from src_keys) sent into itself or below it: Windows would only say it can't.
pub(crate) fn into_itself(keys: &[String], dst: &str) -> bool {
    let dst = dir_key(dst);
    keys.iter().any(|s| dst.strip_prefix(s.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with('\\')))
}

// Some(rename) when the selection can go to `dst`: a file system folder, not into itself, not a move into
// the folder it already sits in. `keys` = src_keys(srcs).
pub(crate) fn send_check(srcs: &[String], keys: &[String], dst: &str, mv: bool) -> Option<bool> {
    if !is_fs(dst) || into_itself(keys, dst) {
        return None;
    }
    send_mode(srcs, dst, mv)
}

// "name<tab>parent", as the recent folders menu shows them; drive and share roots whole. '&' stays a letter.
pub(crate) fn folder_label(p: &str) -> String {
    let path = std::path::Path::new(p);
    let (leaf, parent) = match (path.file_name(), path.parent()) {
        (Some(n), Some(d)) => (n.to_string_lossy().into_owned(), d.display().to_string()),
        _ => (p.to_string(), String::new()),
    };
    format!("{leaf}\t{parent}").replace('&', "&&")
}

// Starts the copy / move and says where in the status line. False = not sent (the caller beeps).
pub(crate) unsafe fn send_elsewhere(srcs: Vec<String>, dst: &str, mv: bool) -> bool {
    let Some(rename) = send_check(&srcs, &src_keys(&srcs), dst, mv) else { return false };
    let n = srcs.len().to_string();
    if !start_copy(srcs, dst.to_string(), rename, mv) {
        return false;
    }
    let text = if mv { tr("Moving {n} items to {to}", "{n}개 이동 시작 → {to}") } else { tr("Copying {n} items to {to}", "{n}개 복사 시작 → {to}") };
    let text = text.replace("{n}", &n).replace("{to}", &short_name(dst));
    app(|a| a.note = Some((text, Instant::now() + Duration::from_secs(3))));
    update_status();
    true
}

// "Choose another folder…": Windows' folder picker over the main window, opening at `start`.
// Ok(None) = cancelled.
pub(crate) unsafe fn choose_folder(owner: HWND, start: &str, mv: bool) -> Result<Option<String>> {
    let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
    dlg.SetOptions(dlg.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)?;
    let title = if mv { tr("Choose the folder to move to", "이동할 폴더 고르기") } else { tr("Choose the folder to copy to", "복사할 폴더 고르기") };
    _ = dlg.SetTitle(&HSTRING::from(title));
    if is_fs(start) {
        if let Ok(at) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start), None) {
            _ = dlg.SetFolder(&at);
        }
    }
    match dlg.Show(Some(owner)) {
        Err(e) if e.code() == ERROR_CANCELLED.to_hresult() => return Ok(None),
        r => r?,
    }
    Ok(Some(pwstr(dlg.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_notice_never_silent_in_back() {
        let long = NOTICE_AFTER + Duration::from_secs(1);
        for took in [Duration::from_secs(1), NOTICE_AFTER, long] {
            for hidden in [false, true] {
                for tray in [false, true] {
                    // in front: the status line now, nothing else
                    let n = done_notice(took, true, hidden, tray);
                    assert_eq!(n, Notice { now: true, flash: false, balloon: false });
                    let n = done_notice(took, false, hidden, tray);
                    assert!(!n.now);
                    if took > NOTICE_AFTER {
                        // something always shows: the flash unless only the icon is left, the notification with an icon
                        assert!(n.flash || n.balloon, "{hidden} {tray}");
                        assert_eq!(n.balloon, tray);
                        assert_eq!(n.flash, !hidden || !tray);
                    } else {
                        assert!(!n.flash && !n.balloon); // short: only said once Grids is back
                    }
                }
            }
        }
        // on the taskbar with an icon too: both, so a blocked notification still leaves the flash
        assert_eq!(done_notice(long, false, false, true), Notice { now: false, flash: true, balloon: true });
    }

    #[test]
    fn done_texts() {
        let d = |mv, n, aborted| CopyDone { mv, n, to: "Backup".into(), aborted, took: Duration::ZERO };
        assert_eq!(done_text(false, &d(false, 12, false)), "Copied 12 items to Backup");
        assert_eq!(done_text(false, &d(false, 1, false)), "Copied 1 item to Backup");
        assert_eq!(done_text(false, &d(true, 3, false)), "Moved 3 items to Backup");
        assert_eq!(done_text(true, &d(false, 12, false)), "복사 끝: 12개 → Backup");
        assert_eq!(done_text(true, &d(true, 12, false)), "이동 끝: 12개 → Backup");
        // skipped or cancelled: no count is claimed
        for ko in [false, true] {
            for mv in [false, true] {
                let t = done_text(ko, &d(mv, 12, true));
                assert!(!t.contains("12") && t.contains("Backup"), "{t}");
            }
        }
    }

    #[test]
    fn utf16_cut_keeps_pairs() {
        assert_eq!(utf16_cut("Grids", 64), "Grids".encode_utf16().collect::<Vec<_>>());
        assert_eq!(utf16_cut("abcdef", 4), "abc".encode_utf16().collect::<Vec<_>>());
        // "a😀": a, high, low; room for 2 units would end on the high half: dropped
        assert_eq!(utf16_cut("a😀", 3), vec!['a' as u16]);
        assert_eq!(utf16_cut("a😀", 4).len(), 3);
        assert!(utf16_cut("x", 0).is_empty());
    }
}
