// One background thread per watched folder. `ReadDirectoryChangesW` reports names;
// the burst logic in `model` turns a run of them into a single note for the UI thread.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_PATH_NOT_FOUND, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileAttributesW, ReadDirectoryChangesW, FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY,
    FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, INVALID_FILE_ATTRIBUTES, OPEN_EXISTING,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForMultipleObjects, INFINITE};

use super::model::{fold_burst, parse_notify, Burst, ChangeKind, Status};
use super::{emit_burst, gen_live, post_watch, set_status};
use crate::imports::*;

const BUF: usize = 16 * 1024;
/// `ERROR_NOTIFY_ENUM_DIR`: the buffer overflowed and the names were dropped.
const OVERFLOW: u32 = 1022;
/// `ERROR_OPERATION_ABORTED`: the watch was cancelled.
const ABORTED: u32 = 995;

#[repr(align(16))]
struct Aligned([u8; BUF]);

pub(crate) struct Running {
    pub event: isize,
    pub join: Option<std::thread::JoinHandle<()>>,
    pub gen: u64,
    pub done: Arc<AtomicBool>,
    pub ready: Arc<AtomicBool>,
}

pub(crate) unsafe fn directory_status(path: &str) -> Option<Status> {
    let attr = GetFileAttributesW(&HSTRING::from(path));
    if attr == INVALID_FILE_ATTRIBUTES {
        let err = GetLastError().0;
        // A share can sit in the redirector for a long time. Leave that to the watch thread.
        if path.starts_with(r"\\") {
            return None;
        }
        if err == ERROR_FILE_NOT_FOUND.0 || err == ERROR_PATH_NOT_FOUND.0 {
            return Some(Status::Missing);
        }
        return None;
    }
    if attr & FILE_ATTRIBUTE_DIRECTORY.0 == FILE_ATTRIBUTE_DIRECTORY.0 {
        None
    } else {
        Some(Status::Failed)
    }
}

/// Manual-reset stop event. The thread waits until `ready` so the slot exists before it checks it.
pub(crate) fn prepare() -> Option<(isize, Arc<AtomicBool>, Arc<AtomicBool>)> {
    let event = unsafe { CreateEventW(None, true, false, None).ok()? };
    Some((event.0 as isize, Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false))))
}

pub(crate) fn launch(
    id: u64,
    gen: u64,
    epoch: u64,
    path: String,
    subfolders: bool,
    stop_bits: isize,
    done: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        while !ready.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        let stop = HANDLE(stop_bits as *mut core::ffi::c_void);
        let outcome = unsafe { watch_loop(id, gen, epoch, &path, subfolders, stop) };
        if let Some(status) = outcome {
            set_status(id, gen, epoch, status);
        }
        done.store(true, Ordering::SeqCst);
        post_watch();
    })
}

unsafe fn watch_loop(id: u64, gen: u64, epoch: u64, path: &str, subfolders: bool, stop: HANDLE) -> Option<Status> {
    if !gen_live(id, gen, epoch) {
        return None;
    }
    let dir = match CreateFileW(
        &HSTRING::from(path),
        FILE_LIST_DIRECTORY.0,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        None,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
        None,
    ) {
        Ok(h) => h,
        Err(e) => {
            let missing = win32_eq(&e, ERROR_FILE_NOT_FOUND) || win32_eq(&e, ERROR_PATH_NOT_FOUND);
            return Some(if missing { Status::Missing } else { Status::Failed });
        }
    };
    let io = match CreateEventW(None, true, false, None) {
        Ok(h) => h,
        Err(_) => {
            let _ = CloseHandle(dir);
            return Some(Status::Failed);
        }
    };
    let mut buf = Aligned([0u8; BUF]);
    let mut ov = OVERLAPPED { hEvent: io, ..Default::default() };
    if issue(dir, &mut buf, subfolders, &mut ov).is_err() {
        let _ = CloseHandle(io);
        let _ = CloseHandle(dir);
        return Some(Status::Failed);
    }
    let base = Instant::now();
    let mono = |base: Instant| base.elapsed().as_millis() as u64;
    let mut burst: Option<Burst> = None;
    let mut failed: Option<Status> = None;
    loop {
        if signaled(stop) || !gen_live(id, gen, epoch) {
            break;
        }
        let now = mono(base);
        if let Some(done_burst) = burst.as_ref().filter(|b| b.due(now)) {
            let done_burst = *done_burst;
            burst = None;
            emit_burst(id, gen, epoch, done_burst);
        }
        let wait = burst.as_ref().and_then(|b| b.wait_ms(mono(base))).map(|ms| ms.min(u32::MAX as u64) as u32).unwrap_or(INFINITE);
        let woke = WaitForMultipleObjects(&[stop, io], false, wait);
        if woke == WAIT_OBJECT_0 {
            break;
        }
        if woke == WAIT_TIMEOUT {
            continue;
        }
        if woke.0 != WAIT_OBJECT_0.0 + 1 {
            failed = Some(Status::Failed);
            break;
        }
        let mut n = 0u32;
        let read = GetOverlappedResult(dir, &ov, &mut n, false);
        let _ = ResetEvent(io);
        let now = mono(base);
        match read {
            Ok(()) => {
                let kinds = parse_notify(&buf.0, n as usize);
                for done_burst in fold_burst(&mut burst, now, &kinds) {
                    emit_burst(id, gen, epoch, done_burst);
                }
            }
            Err(e) => {
                if win32_eq(&e, WIN32_ERROR(OVERFLOW)) {
                    for done_burst in fold_burst(&mut burst, now, &[ChangeKind::Modified]) {
                        emit_burst(id, gen, epoch, done_burst);
                    }
                } else if win32_eq(&e, WIN32_ERROR(ABORTED)) {
                    break;
                } else {
                    failed = Some(Status::Failed);
                    break;
                }
            }
        }
        ov = OVERLAPPED { hEvent: io, ..Default::default() };
        if issue(dir, &mut buf, subfolders, &mut ov).is_err() {
            failed = Some(Status::Failed);
            break;
        }
    }
    if let Some(done_burst) = burst.take() {
        if gen_live(id, gen, epoch) {
            emit_burst(id, gen, epoch, done_burst);
        }
    }
    let _ = CancelIoEx(dir, Some(&ov));
    let mut n = 0u32;
    let _ = GetOverlappedResult(dir, &ov, &mut n, true);
    let _ = CloseHandle(io);
    let _ = CloseHandle(dir);
    failed
}

fn win32_eq(e: &Error, code: WIN32_ERROR) -> bool {
    e.code() == HRESULT::from_win32(code.0)
}

unsafe fn issue(dir: HANDLE, buf: &mut Aligned, subfolders: bool, ov: &mut OVERLAPPED) -> Result<()> {
    let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE;
    match ReadDirectoryChangesW(dir, buf.0.as_mut_ptr() as *mut _, BUF as u32, subfolders, filter, None, Some(ov), None) {
        Ok(()) => Ok(()),
        Err(e) if win32_eq(&e, ERROR_IO_PENDING) => Ok(()),
        Err(e) => Err(e),
    }
}

unsafe fn signaled(stop: HANDLE) -> bool {
    WaitForMultipleObjects(&[stop], false, 0) == WAIT_OBJECT_0
}
