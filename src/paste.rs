use crate::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::System::Memory::GlobalSize;
use windows::Win32::System::Shutdown::{ShutdownBlockReasonCreate, ShutdownBlockReasonDestroy};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentProcessId;
use std::sync::atomic::AtomicI32;

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
    let pic = picture_only(
        !srcs.is_empty(),
        has(w!("Shell IDList Array")),
        has(w!("FileGroupDescriptorW")) || has(w!("FileGroupDescriptor")),
        IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok(),
    )
    .then(|| clip_picture())
    .flatten();
    _ = CloseClipboard();
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
pub(crate) unsafe fn start_copy(srcs: Vec<String>, dst: String, rename: bool, mv: bool) -> bool {
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        if let Err(e) = run_copy(&srcs, &dst, rename, mv) {
            if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
                let what = if mv { tr("Moving failed: ", "이동하지 못했습니다: ") } else { tr("Copying failed: ", "복사하지 못했습니다: ") };
                let text = HSTRING::from(format!("{what}{}", e.message()));
                MessageBoxW(None, &text, w!("Grids"), MB_ICONWARNING);
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
pub(crate) unsafe fn run_copy(srcs: &[String], dst: &str, rename: bool, mv: bool) -> Result<()> {
    run_op(srcs, dst, rename, mv).map(|_| ())
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
