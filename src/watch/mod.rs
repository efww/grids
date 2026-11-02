// Folder Watch. The list lives for the whole process and is written into maguro.json.
// Threads run only while Maguro is open. There is no tray icon and no service.

mod engine;
mod model;
mod notify;
mod win;

use std::collections::HashMap;
use std::sync::atomic::AtomicIsize;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::SetEvent;

pub(crate) use model::WatchJson;
use model::{AddFail, Book, Burst, ChangeKind, Status, MAX_WATCHES};

use crate::imports::*;

pub(crate) static MAIN: AtomicIsize = AtomicIsize::new(0);

struct Note {
    path: String,
    kind: ChangeKind,
}

struct Live {
    book: Book,
    stops: HashMap<u64, engine::Running>,
    epoch: u64,
    next_gen: u64,
    opens: Vec<String>,
    notes: Vec<Note>,
}

impl Live {
    fn new() -> Self {
        Live { book: Book::new(), stops: HashMap::new(), epoch: 1, next_gen: 1, opens: Vec::new(), notes: Vec::new() }
    }
}

fn live() -> &'static Mutex<Live> {
    static LIVE: OnceLock<Mutex<Live>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(Live::new()))
}

fn lock() -> std::sync::MutexGuard<'static, Live> {
    live().lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn export_saved() -> Vec<WatchJson> {
    lock().book.to_json()
}

/// Replace the saved list. Running threads are signalled; `resync` joins them and starts the new set.
pub(crate) fn import_saved(rows: &[WatchJson]) {
    let mut g = lock();
    g.epoch = g.epoch.wrapping_add(1);
    for slot in g.stops.values() {
        unsafe { let _ = SetEvent(HANDLE(slot.event as *mut core::ffi::c_void)); }
    }
    g.book = Book::from_json(rows);
}

pub(crate) unsafe fn start() {
    let h = crate::app(|a| a.main);
    MAIN.store(h.0 as isize, Ordering::SeqCst);
    resync();
}

pub(crate) fn retarget(h: HWND) {
    MAIN.store(h.0 as isize, Ordering::SeqCst);
}

pub(crate) unsafe fn shutdown() {
    let ids: Vec<u64> = lock().stops.keys().copied().collect();
    for id in ids {
        halt(id);
    }
    MAIN.store(0, Ordering::SeqCst);
    notify::dismiss();
    win::close();
}

pub(crate) unsafe fn resync() {
    let ids: Vec<u64> = lock().stops.keys().copied().collect();
    for id in ids {
        halt(id);
    }
    let ids = lock().book.ids();
    for id in ids {
        arm(id);
    }
    win::redraw();
}

pub(crate) unsafe fn open(owner: HWND) {
    win::open(owner);
}

pub(crate) unsafe fn restyle() {
    win::restyle();
}

pub(crate) unsafe fn on_ui_event() {
    reap();
    let notes = std::mem::take(&mut lock().notes);
    for note in &notes {
        let name = leaf(&note.path);
        let body = crate::tf(crate::Key::WatchNotify, &[("kind", crate::tr(note.kind.key())), ("name", &name)]);
        notify::show(&note.path, crate::tr(crate::Key::FolderWatch), &body);
    }
    if !notes.is_empty() {
        crate::note_save();
    }
    win::redraw();
}

pub(crate) unsafe fn open_queued() {
    let paths = std::mem::take(&mut lock().opens);
    if paths.is_empty() {
        return;
    }
    let main = crate::app(|a| a.main);
    if !main.0.is_null() {
        crate::reveal_window(main);
    }
    for path in paths {
        crate::open_args_as_tabs(vec![path]);
    }
}

pub(crate) unsafe fn show_fallback() {
    notify::present_fallback();
}

pub(crate) fn queue_open(path: String) {
    if path.is_empty() {
        return;
    }
    lock().opens.push(path);
    post(crate::WM_WATCH_OPEN);
}

pub(crate) fn snapshot() -> Vec<model::Row> {
    lock().book.rows().to_vec()
}

pub(crate) unsafe fn add_folder(path: &str) -> Option<&'static crate::Key> {
    match engine::directory_status(path) {
        Some(Status::Failed) => return Some(&crate::Key::WatchNotFolder),
        Some(Status::Missing) => return Some(&crate::Key::WatchMissing),
        _ => {}
    }
    let id = match lock().book.add(path) {
        Ok(id) => id,
        Err(AddFail::Dup) => return Some(&crate::Key::WatchAlready),
        Err(AddFail::Limit) => return Some(&crate::Key::WatchLimit),
        Err(AddFail::Bad) => return Some(&crate::Key::WatchNotFolder),
    };
    arm(id);
    crate::note_save();
    win::redraw();
    None
}

pub(crate) unsafe fn add_active() -> Option<&'static crate::Key> {
    let path = crate::app(|a| a.s.paths.get(a.s.active).cloned().unwrap_or_default());
    add_folder(&path)
}

pub(crate) unsafe fn toggle_run(id: u64) {
    let pausing = {
        let mut g = lock();
        let Some(row) = g.book.get_mut(id) else { return };
        row.paused = !row.paused;
        if row.paused {
            row.status = Status::Paused;
        }
        row.paused
    };
    if pausing {
        halt(id);
    } else {
        arm(id);
    }
    crate::note_save();
    win::redraw();
}

pub(crate) unsafe fn toggle_subfolders(id: u64) {
    let restart = {
        let mut g = lock();
        let Some(row) = g.book.get_mut(id) else { return };
        row.subfolders = !row.subfolders;
        !row.paused
    };
    if restart {
        arm(id);
    }
    crate::note_save();
    win::redraw();
}

pub(crate) unsafe fn delete(id: u64) {
    halt(id);
    lock().book.remove(id);
    crate::note_save();
    win::redraw();
}

pub(crate) fn emit_burst(id: u64, gen: u64, epoch: u64, burst: Burst) {
    let when = unix_now();
    let note = {
        let mut g = lock();
        if g.epoch != epoch || g.stops.get(&id).map(|s| s.gen) != Some(gen) {
            return;
        }
        let Some(row) = g.book.get_mut(id) else { return };
        if row.paused {
            return;
        }
        row.last_ms = Some(when);
        row.last_kind = Some(burst.kind);
        row.status = Status::Watching;
        Note { path: row.path.clone(), kind: burst.kind }
    };
    lock().notes.push(note);
    post_watch();
}

pub(crate) fn set_status(id: u64, gen: u64, epoch: u64, status: Status) {
    let mut g = lock();
    if g.epoch != epoch || g.stops.get(&id).map(|s| s.gen) != Some(gen) {
        return;
    }
    if let Some(row) = g.book.get_mut(id) {
        if !row.paused {
            row.status = status;
        }
    }
}

pub(crate) fn gen_live(id: u64, gen: u64, epoch: u64) -> bool {
    let g = lock();
    g.epoch == epoch && g.stops.get(&id).is_some_and(|s| s.gen == gen)
}

pub(crate) fn post_watch() {
    post(crate::WM_WATCH);
}

fn post(msg: u32) {
    let bits = MAIN.load(Ordering::SeqCst);
    if bits == 0 {
        return;
    }
    let h = HWND(bits as *mut core::ffi::c_void);
    unsafe { let _ = PostMessageW(Some(h), msg, WPARAM(0), LPARAM(0)); }
}

unsafe fn arm(id: u64) {
    halt(id);
    let (path, sub, epoch, gen) = {
        let mut g = lock();
        let paused = g.book.rows().iter().find(|r| r.id == id).map(|r| r.paused);
        let Some(paused) = paused else { return };
        if paused {
            if let Some(row) = g.book.get_mut(id) {
                row.status = Status::Paused;
            }
            return;
        }
        g.next_gen = g.next_gen.wrapping_add(1);
        let gen = g.next_gen;
        let epoch = g.epoch;
        let Some(row) = g.book.rows().iter().find(|r| r.id == id) else { return };
        (row.path.clone(), row.subfolders, epoch, gen)
    };
    if let Some(status) = engine::directory_status(&path) {
        if let Some(row) = lock().book.get_mut(id) {
            row.status = status;
        }
        return;
    }
    let Some((event, done, ready)) = engine::prepare() else {
        if let Some(row) = lock().book.get_mut(id) {
            row.status = Status::Failed;
        }
        return;
    };
    let join = engine::launch(id, gen, epoch, path, sub, event, done.clone(), ready.clone());
    // Insert before the thread leaves its ready wait, so `gen_live` sees this generation.
    lock().stops.insert(id, engine::Running { event, join: Some(join), gen, done, ready: ready.clone() });
    ready.store(true, Ordering::SeqCst);
    if let Some(row) = lock().book.get_mut(id) {
        if !row.paused {
            row.status = Status::Watching;
        }
    }
}

fn halt(id: u64) {
    let running = { lock().stops.remove(&id) };
    let Some(mut slot) = running else { return };
    slot.ready.store(true, Ordering::SeqCst);
    unsafe { let _ = SetEvent(HANDLE(slot.event as *mut core::ffi::c_void)); }
    if let Some(join) = slot.join.take() {
        let _ = join.join();
    }
    unsafe { let _ = CloseHandle(HANDLE(slot.event as *mut core::ffi::c_void)); }
}

fn reap() {
    let ids: Vec<u64> = lock().stops.iter().filter(|(_, slot)| slot.done.load(Ordering::SeqCst)).map(|(id, _)| *id).collect();
    for id in ids {
        halt(id);
        if let Some(row) = lock().book.get_mut(id) {
            if !row.paused && row.status == Status::Watching {
                row.status = Status::Failed;
            }
        }
    }
}

fn leaf(path: &str) -> String {
    let trimmed = path.trim_end_matches('\\');
    trimmed.rsplit('\\').next().filter(|s| !s.is_empty()).unwrap_or(path).to_string()
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub(crate) fn limit() -> usize {
    MAX_WATCHES
}
