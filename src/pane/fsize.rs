// Folder totals for the status line. Selecting a folder queues a background walk; the result is
// drawn on the status bar. A change inside that folder drops the cached total so the next status
// tick can ask again. This module never touches the file list: no Refresh, no SelectItem, no
// column or view-mode write, no list tooltip.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicIsize, AtomicU64};
use std::sync::LazyLock;
use std::time::SystemTime;

use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadDirectoryChangesW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OVERLAPPED, FILE_LIST_DIRECTORY, FILE_NOTIFY_CHANGE_DIR_NAME,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE, FILE_NOTIFY_CHANGE_SIZE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentThread, ResetEvent, SetEvent, SetThreadPriority, WaitForMultipleObjects, WaitForSingleObject, THREAD_MODE_BACKGROUND_BEGIN,
    THREAD_PRIORITY_LOWEST,
};
use super::dir_totals;
use crate::i18n::*;
use crate::imports::*;
use crate::session::*;
use crate::ui::*;
use crate::MAXP;

pub(crate) const RANK_SELECTED: u8 = 0;
pub(crate) const RANK_HOVER: u8 = 1;
pub(crate) const RANK_VISIBLE: u8 = 2;

const QUEUE_CAP: usize = 48;
const CACHE_ENTRIES: usize = 2_048;
const CACHE_BYTES: usize = 1_500_000;
const EPOCH_CAP: usize = 8_192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    Local,
    Wsl,
    Network,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FolderStat {
    pub bytes: u64,
    pub files: u64,
    pub folders: u64,
    pub partial: bool,
    pub at: SystemTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SizeView {
    Hidden,
    Pending,
    Ready { bytes: u64, files: u64, partial: bool },
}

/// `parts` is one slot per folder that should be summed. `None` is still walking.
/// An empty list means there is nothing to say (no allowed folders).
pub(crate) fn view_selection(parts: &[Option<FolderStat>], file_bytes: u64, file_count: u64, extra_partial: bool) -> SizeView {
    if parts.is_empty() {
        return SizeView::Hidden;
    }
    if parts.iter().any(|p| p.is_none()) {
        return SizeView::Pending;
    }
    let mut bytes = file_bytes;
    let mut files = file_count;
    let mut partial = extra_partial;
    for s in parts.iter().flatten() {
        bytes += s.bytes;
        files += s.files;
        partial |= s.partial;
    }
    SizeView::Ready { bytes, files, partial }
}

pub(crate) fn folder_size_text(pending: bool, bytes: u64, files: u64, partial: bool) -> String {
    let label = tr(crate::Key::FolderSize);
    if pending {
        return format!("{label}: {}", tr(crate::Key::Calculating));
    }
    let n = files_phrase(files);
    if partial {
        format!("{label}: {} ({n}, {})", human(bytes), tr(crate::Key::Partial))
    } else {
        format!("{label}: {} ({n})", human(bytes))
    }
}

pub(crate) fn view_text(v: SizeView) -> Option<String> {
    match v {
        SizeView::Hidden => None,
        SizeView::Pending => Some(folder_size_text(true, 0, 0, false)),
        SizeView::Ready { bytes, files, partial } => Some(folder_size_text(false, bytes, files, partial)),
    }
}

fn files_phrase(n: u64) -> String {
    let g = grouped(n);
    let key = if n == 1 { Key::OneFile } else { Key::NFiles };
    tf(key, &[("g", &g)])
}

pub(crate) fn grouped(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

pub(crate) fn norm_key(path: &str) -> String {
    let mut s = path.trim().replace('/', "\\");
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if let Some(unc) = rest.strip_prefix("UNC\\").or_else(|| rest.strip_prefix("unc\\")) {
            s = format!(r"\\{unc}");
        } else {
            s = rest.to_string();
        }
    }
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    if s.len() == 2 && s.as_bytes().get(1) == Some(&b':') {
        s.push('\\');
    }
    s.to_ascii_lowercase()
}

pub(crate) fn parent_key(key: &str) -> Option<String> {
    let s = key.trim_end_matches('\\');
    if s.len() <= 2 {
        return None;
    }
    if let Some(rest) = s.strip_prefix(r"\\") {
        let parts: Vec<&str> = rest.split('\\').filter(|p| !p.is_empty()).collect();
        if parts.len() <= 2 {
            return None;
        }
        return Some(norm_key(&format!(r"\\{}", parts[..parts.len() - 1].join("\\"))));
    }
    match s.rfind('\\') {
        Some(i) => {
            let Some(parent) = s.get(..i) else { return None };
            if parent.len() == 2 && parent.as_bytes()[1] == b':' {
                Some(format!("{parent}\\"))
            } else if parent.is_empty() {
                None
            } else {
                Some(parent.to_string())
            }
        }
        None => None,
    }
}

fn under(key: &str, root: &str) -> bool {
    if key == root {
        return true;
    }
    let prefix = if root.ends_with('\\') { root.to_string() } else { format!("{root}\\") };
    key.starts_with(&prefix)
}

pub(crate) fn is_volume_root(path: &str) -> bool {
    let k = norm_key(path);
    if parent_key(&k).is_some() {
        return false;
    }
    (k.len() == 3 && k.as_bytes().get(1) == Some(&b':')) || k.starts_with(r"\\")
}

fn is_wsl_key(key: &str) -> bool {
    key.starts_with(r"\\wsl$\") || key.starts_with(r"\\wsl.localhost\") || key == r"\\wsl$" || key == r"\\wsl.localhost"
}

pub(crate) fn place(path: &str, mapped_remote: bool) -> Place {
    let k = norm_key(path);
    if is_wsl_key(&k) {
        Place::Wsl
    } else if k.starts_with(r"\\") || mapped_remote {
        Place::Network
    } else {
        Place::Local
    }
}

pub(crate) fn allowed(place: Place, network: bool) -> bool {
    match place {
        Place::Network => network,
        Place::Local | Place::Wsl => true,
    }
}

/// Whether the preview pane should walk a folder for its recursive size.
/// Volume roots are skipped. Network places run only when that option is on.
pub(crate) fn preview_walk(root: bool, place: Place, network: bool) -> bool {
    !root && allowed(place, network)
}

pub(crate) fn cached_stat(path: &str) -> Option<FolderStat> {
    lock().cache.get(&norm_key(path))
}

pub(crate) fn remember_stat(path: &str, bytes: u64, files: u64, folders: u64, partial: bool) {
    lock().cache.insert(
        norm_key(path),
        FolderStat { bytes, files, folders, partial, at: SystemTime::now() },
    );
}

/// Drive-letter paths ask `GetDriveTypeW` (the same call the free-space line already makes).
/// UNC and WSL are classified from the text, so a dead share is not queried here.
pub(crate) fn classify_ui(path: &str) -> Place {
    let k = norm_key(path);
    if is_wsl_key(&k) || k.starts_with(r"\\") {
        return place(path, false);
    }
    let mapped = unsafe { super::share_path(path) };
    place(path, mapped)
}

fn is_fs_key(key: &str) -> bool {
    (key.len() >= 3 && key.as_bytes().get(1) == Some(&b':')) || key.starts_with(r"\\")
}

#[derive(Clone)]
struct Job {
    key: String,
    path: String,
    rank: u8,
    seq: u64,
    holders: Vec<(u8, u64)>,
    place: Place,
}

struct Cache {
    map: HashMap<String, FolderStat>,
    order: VecDeque<String>,
    bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl Cache {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Cache { map: HashMap::new(), order: VecDeque::new(), bytes: 0, max_entries, max_bytes }
    }

    fn get(&mut self, key: &str) -> Option<FolderStat> {
        let stat = self.map.get(key)?.clone();
        if let Some(i) = self.order.iter().position(|k| k == key) {
            if let Some(k) = self.order.remove(i) {
                self.order.push_back(k);
            }
        }
        Some(stat)
    }

    fn peek(&self, key: &str) -> Option<&FolderStat> {
        self.map.get(key)
    }

    fn insert(&mut self, key: String, stat: FolderStat) {
        if let Some(old) = self.map.remove(&key) {
            self.bytes = self.bytes.saturating_sub(key.len());
            let _ = old;
            if let Some(i) = self.order.iter().position(|k| k == &key) {
                self.order.remove(i);
            }
        }
        self.bytes += key.len();
        self.order.push_back(key.clone());
        self.map.insert(key, stat);
        while self.map.len() > self.max_entries || (self.bytes > self.max_bytes && self.map.len() > 1) {
            let Some(old) = self.order.pop_front() else { break };
            self.bytes = self.bytes.saturating_sub(old.len());
            self.map.remove(&old);
        }
    }

    fn remove(&mut self, key: &str) {
        if self.map.remove(key).is_some() {
            self.bytes = self.bytes.saturating_sub(key.len());
            if let Some(i) = self.order.iter().position(|k| k == key) {
                self.order.remove(i);
            }
        }
    }
}

struct Store {
    cache: Cache,
    epochs: HashMap<String, u64>,
    epoch_base: u64,
    queue: Vec<Job>,
    seq: u64,
    gens: [u64; MAXP],
    desired: [String; MAXP],
    missed: HashSet<String>,
    failed: HashSet<String>,
}

impl Store {
    fn new() -> Self {
        Store {
            cache: Cache::new(CACHE_ENTRIES, CACHE_BYTES),
            epochs: HashMap::new(),
            epoch_base: 1,
            queue: Vec::new(),
            seq: 1,
            gens: [0; MAXP],
            desired: std::array::from_fn(|_| String::new()),
            missed: HashSet::new(),
            failed: HashSet::new(),
        }
    }

    fn alive(&self, job: &Job) -> bool {
        job.holders.iter().any(|&(p, g)| (p as usize) < MAXP && self.gens[p as usize] == g)
    }

    fn push(&mut self, pane: usize, gen: u64, path: &str, place: Place, rank: u8) {
        if pane >= MAXP || self.gens[pane] != gen {
            return;
        }
        let key = norm_key(path);
        if key.is_empty() || is_volume_root(path) || !is_fs_key(&key) {
            return;
        }
        if self.cache.peek(&key).is_some() {
            return;
        }
        if let Some(job) = self.queue.iter_mut().find(|j| j.key == key) {
            job.rank = job.rank.min(rank);
            if rank == RANK_SELECTED {
                job.seq = self.seq;
                self.seq = self.seq.wrapping_add(1);
            }
            if !job.holders.iter().any(|&(p, g)| p as usize == pane && g == gen) {
                job.holders.push((pane as u8, gen));
            }
            job.place = place;
            return;
        }
        if self.queue.len() >= QUEUE_CAP {
            let worst = self.queue.iter().enumerate().max_by_key(|(_, j)| (j.rank, !j.seq));
            let Some((idx, job)) = worst else { return };
            let better = rank < job.rank || (rank == job.rank && rank == RANK_SELECTED);
            if !better {
                return;
            }
            self.queue.remove(idx);
        }
        self.queue.push(Job {
            key,
            path: path.to_string(),
            rank,
            seq: self.seq,
            holders: vec![(pane as u8, gen)],
            place,
        });
        self.seq = self.seq.wrapping_add(1);
    }

    fn pop(&mut self) -> Option<Job> {
        let gens = self.gens;
        self.queue.retain(|j| j.holders.iter().any(|&(p, g)| (p as usize) < MAXP && gens[p as usize] == g));
        let best = self.queue.iter().enumerate().min_by(|(_, a), (_, b)| {
            a.rank.cmp(&b.rank).then_with(|| if a.rank == RANK_SELECTED { b.seq.cmp(&a.seq) } else { a.seq.cmp(&b.seq) })
        })?;
        Some(self.queue.remove(best.0))
    }

    fn better_queued(&self, rank: u8) -> bool {
        self.queue.iter().any(|j| j.rank < rank && self.alive(j))
    }

    fn epoch_of(&self, key: &str) -> (u64, u64) {
        (self.epochs.get(key).copied().unwrap_or(0), self.epoch_base)
    }

    fn bump_epoch(&mut self, key: &str) {
        *self.epochs.entry(key.to_string()).or_insert(0) += 1;
        if self.epochs.len() > EPOCH_CAP {
            self.epochs.clear();
            self.epoch_base = self.epoch_base.wrapping_add(1);
        }
    }

    fn drop_key(&mut self, key: &str) {
        self.cache.remove(key);
        self.bump_epoch(key);
    }

    /// The changed folder and every ancestor. Descendants stay, unless `subtree` (a removed
    /// directory, a return to a folder we stopped watching, or F5 on an unwatched folder).
    fn invalidate(&mut self, key: &str, subtree: bool) {
        if key.is_empty() {
            return;
        }
        if subtree {
            let under_cache: Vec<String> = self.cache.map.keys().filter(|k| under(k, key)).cloned().collect();
            let under_epoch: Vec<String> = self.epochs.keys().filter(|k| under(k, key)).cloned().collect();
            for k in under_cache.into_iter().chain(under_epoch) {
                self.drop_key(&k);
            }
        }
        self.drop_key(key);
        let mut p = parent_key(key);
        while let Some(a) = p {
            self.drop_key(&a);
            p = parent_key(&a);
        }
    }
}

static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::new()));

fn lock() -> std::sync::MutexGuard<'static, Store> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

static WAKE: Condvar = Condvar::new();
static SHOW: AtomicBool = AtomicBool::new(true);
static NET: AtomicBool = AtomicBool::new(false);
static POLICY: AtomicU64 = AtomicU64::new(0);
static PANE_GEN: [AtomicU64; MAXP] = [const { AtomicU64::new(0) }; MAXP];
static WATCH_EVT: AtomicIsize = AtomicIsize::new(0);

pub(crate) fn set_opts(show: bool, network: bool) {
    let changed = SHOW.swap(show, Ordering::Relaxed) != show || NET.swap(network, Ordering::Relaxed) != network;
    if !show {
        lock().queue.clear();
    }
    if changed {
        POLICY.fetch_add(1, Ordering::Relaxed);
        WAKE.notify_one();
    }
}

pub(crate) fn clear_pending() {
    lock().queue.clear();
    POLICY.fetch_add(1, Ordering::Relaxed);
    WAKE.notify_one();
}

/// The pane is leaving its folder. Queued work for it is dropped; an in-flight walk stops
/// at the next directory. A local folder we were watching is marked missed so coming back
/// recomputes instead of trusting a cache that changed while we were away.
pub(crate) fn pane_left(i: usize) {
    if i >= MAXP {
        return;
    }
    let gen = PANE_GEN[i].fetch_add(1, Ordering::AcqRel) + 1;
    let mut st = lock();
    st.gens[i] = gen;
    let old = std::mem::take(&mut st.desired[i]);
    if !old.is_empty() {
        let k = norm_key(&old);
        if !st.failed.contains(&k) {
            st.missed.insert(k);
        }
    }
    let gens = st.gens;
    st.queue.retain(|j| j.holders.iter().any(|&(p, g)| (p as usize) < MAXP && gens[p as usize] == g));
    drop(st);
    WAKE.notify_one();
    signal_watch();
}

pub(crate) fn pane_here(i: usize, path: &str, place: Place) {
    if i >= MAXP {
        return;
    }
    let key = norm_key(path);
    let next = if path.is_empty() || !is_fs_key(&key) || !matches!(place, Place::Local) { String::new() } else { path.to_string() };
    let mut st = lock();
    if st.desired[i] == next {
        return;
    }
    let old = std::mem::take(&mut st.desired[i]);
    if !old.is_empty() {
        let k = norm_key(&old);
        if !st.failed.contains(&k) {
            st.missed.insert(k);
        }
    }
    if !next.is_empty() {
        let k = norm_key(&next);
        st.failed.remove(&k);
        if st.missed.remove(&k) {
            st.invalidate(&k, true);
        }
        st.desired[i] = next;
    }
    if st.missed.len() > 1024 {
        let keys: Vec<String> = st.missed.drain().collect();
        for k in keys {
            st.invalidate(&k, true);
        }
    }
    drop(st);
    ensure_threads();
    signal_watch();
}

/// F5: WSL, network, and any folder whose watch failed are dropped (with ancestors and
/// descendants) so the next paint computes them again. A live local watch is left alone.
pub(crate) fn refresh_paths(paths: &[String]) {
    let tagged: Vec<(String, Place)> = paths.iter().filter(|p| !p.is_empty()).map(|p| (norm_key(p), classify_ui(p))).collect();
    let mut st = lock();
    for (k, place) in tagged {
        if !matches!(place, Place::Local) || st.failed.contains(&k) {
            st.invalidate(&k, true);
        }
    }
}

pub(crate) fn request_paths(pane: usize, paths: &[(String, Place)], rank: u8) {
    if pane >= MAXP || !SHOW.load(Ordering::Relaxed) {
        return;
    }
    let gen = PANE_GEN[pane].load(Ordering::Relaxed);
    let net = NET.load(Ordering::Relaxed);
    let mut st = lock();
    if st.gens[pane] != gen {
        st.gens[pane] = gen;
    }
    for (path, place) in paths {
        if !allowed(*place, net) {
            continue;
        }
        st.push(pane, gen, path, *place, rank);
    }
    let queued = !st.queue.is_empty();
    drop(st);
    if queued {
        ensure_threads();
        WAKE.notify_one();
    }
}

pub(crate) fn selection_text(paths: &[String], file_bytes: u64, file_count: u64, extra_partial: bool) -> Option<String> {
    if paths.is_empty() || !SHOW.load(Ordering::Relaxed) {
        return None;
    }
    let mut st = lock();
    let parts: Vec<Option<FolderStat>> = paths.iter().map(|p| st.cache.get(&norm_key(p))).collect();
    drop(st);
    view_text(view_selection(&parts, file_bytes, file_count, extra_partial))
}

/// Selection split for the folder-size line. The UI thread reads shell attributes and, for
/// local files only, file length. Folders are not walked here.
pub(crate) struct FolderPick {
    pub folders: Vec<String>,
    pub request: Vec<(String, Place)>,
    pub file_bytes: u64,
    pub file_count: u64,
    pub extra_partial: bool,
}

pub(crate) unsafe fn pick_selection(fv: &IFolderView2, network: bool) -> FolderPick {
    let mut pick = FolderPick { folders: Vec::new(), request: Vec::new(), file_bytes: 0, file_count: 0, extra_partial: false };
    let Ok(arr) = fv.GetSelection(false) else {
        pick.extra_partial = true;
        return pick;
    };
    let Ok(n) = arr.GetCount() else {
        pick.extra_partial = true;
        return pick;
    };
    if n > 10_000 {
        pick.extra_partial = true;
    }
    let (mut skipped_net, mut roots) = (false, false);
    for k in 0..n.min(10_000) {
        let Ok(item) = arr.GetItemAt(k) else {
            pick.extra_partial = true;
            continue;
        };
        let attr = item.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM).map(|a| a.0).unwrap_or(0);
        let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)).unwrap_or_default();
        if path.is_empty() || !is_fs(&path) {
            pick.extra_partial = true;
            continue;
        }
        if real_folder(attr) {
            if is_volume_root(&path) {
                roots = true;
                continue;
            }
            let place = classify_ui(&path);
            if !allowed(place, network) {
                skipped_net = true;
                continue;
            }
            pick.folders.push(path.clone());
            pick.request.push((path, place));
        } else {
            let place = classify_ui(&path);
            if !matches!(place, Place::Local) {
                pick.extra_partial = true;
                continue;
            }
            match std::fs::metadata(&path) {
                Ok(m) if m.is_file() => {
                    pick.file_bytes += m.len();
                    pick.file_count += 1;
                }
                _ => pick.extra_partial = true,
            }
        }
    }
    if !pick.folders.is_empty() && (skipped_net || roots) {
        pick.extra_partial = true;
    }
    pick
}

fn ensure_threads() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = std::thread::Builder::new().name("folder-size".into()).spawn(worker);
        let _ = std::thread::Builder::new().name("folder-watch".into()).spawn(watch_loop);
    });
}

fn signal_watch() {
    let h = WATCH_EVT.load(Ordering::Relaxed);
    if h != 0 {
        unsafe { _ = SetEvent(HANDLE(h as *mut _)) };
    }
}

fn low_priority() {
    unsafe {
        let t = GetCurrentThread();
        if SetThreadPriority(t, THREAD_MODE_BACKGROUND_BEGIN).is_err() {
            _ = SetThreadPriority(t, THREAD_PRIORITY_LOWEST);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Halt {
    Run,
    Yield,
    Drop,
}

fn worker() {
    low_priority();
    loop {
        let job = {
            let mut st = lock();
            loop {
                if !SHOW.load(Ordering::Relaxed) {
                    st.queue.clear();
                }
                if let Some(job) = st.pop() {
                    break job;
                }
                st = WAKE.wait(st).unwrap_or_else(|e| e.into_inner());
            }
        };
        run_job(job);
    }
}

fn run_job(job: Job) {
    let (epoch, base) = lock().epoch_of(&job.key);
    let policy = POLICY.load(Ordering::Relaxed);
    let halt = std::cell::Cell::new(Halt::Run);
    let seen = std::cell::Cell::new(0usize);
    let holders = job.holders.clone();
    let rank = job.rank;
    let place = job.place;
    let stop = || {
        let n = seen.get() + 1;
        seen.set(n);
        if policy != POLICY.load(Ordering::Relaxed) || !SHOW.load(Ordering::Relaxed) {
            halt.set(Halt::Drop);
            return true;
        }
        if place == Place::Network && !NET.load(Ordering::Relaxed) {
            halt.set(Halt::Drop);
            return true;
        }
        let alive = holders.iter().any(|&(p, g)| (p as usize) < MAXP && PANE_GEN[p as usize].load(Ordering::Relaxed) == g);
        if !alive {
            halt.set(Halt::Drop);
            return true;
        }
        if n % 48 == 0 && lock().better_queued(rank) {
            halt.set(Halt::Yield);
            return true;
        }
        false
    };
    let paused = || crate::COPYING.load(Ordering::Relaxed) > 0;
    let mut quiet = |_, _| {};
    let result = match crate::crash::guard(std::panic::AssertUnwindSafe(|| dir_totals(Path::new(&job.path), &stop, &paused, &mut quiet, super::DIR_STACK_CAP))) {
        Ok(v) => v,
        Err(_) => {
            halt.set(Halt::Drop);
            None
        }
    };
    let (now_epoch, now_base) = lock().epoch_of(&job.key);
    let fresh = now_base == base && now_epoch == epoch;
    if let Some(t) = result {
        if halt.get() == Halt::Run && fresh {
            let stat = FolderStat { bytes: t.bytes, files: t.files, folders: t.folders, partial: t.partial, at: SystemTime::now() };
            let mut st = lock();
            if st.epoch_base == base && st.epochs.get(&job.key).copied().unwrap_or(0) == epoch {
                st.cache.insert(job.key.clone(), stat);
            }
            return;
        }
    }
    if halt.get() == Halt::Drop || !SHOW.load(Ordering::Relaxed) {
        return;
    }
    if !fresh {
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    let mut st = lock();
    if st.gens.iter().enumerate().any(|(i, g)| job.holders.iter().any(|&(p, gen)| p as usize == i && gen == *g)) && st.cache.peek(&job.key).is_none() {
        st.queue.push(job);
    }
    drop(st);
    WAKE.notify_one();
}

// ---------- change watches (one thread, overlapped ReadDirectoryChangesW, never the UI thread) ----------

struct Open {
    path: String,
    handle: HANDLE,
    event: HANDLE,
    ov: Box<OVERLAPPED>,
    buf: Box<[u8; 16_384]>,
}

fn watch_loop() {
    low_priority();
    let wake = unsafe { CreateEventW(None, false, false, None) }.unwrap_or_default();
    if wake.is_invalid() {
        return;
    }
    WATCH_EVT.store(wake.0 as isize, Ordering::Relaxed);
    let mut slots: [Option<Open>; MAXP] = std::array::from_fn(|_| None);
    loop {
        sync_slots(&mut slots);
        let mut handles = vec![wake];
        let mut who = vec![usize::MAX];
        for i in 0..MAXP {
            if let Some(o) = &slots[i] {
                handles.push(o.event);
                who.push(i);
            }
        }
        let w = unsafe { WaitForMultipleObjects(&handles, false, 1000) };
        let code = w.0;
        if code == WAIT_OBJECT_0.0 || code == WAIT_TIMEOUT.0 {
            continue;
        }
        let idx = code.wrapping_sub(WAIT_OBJECT_0.0) as usize;
        if idx < who.len() {
            let pane = who[idx];
            if pane < MAXP {
                if let Some(o) = slots[pane].as_mut() {
                    if !on_changes(o) {
                        if let Some(mut old) = slots[pane].take() {
                            shutdown(&mut old);
                            note_failed(&old.path, true);
                        }
                    }
                }
            }
        }
    }
}

fn sync_slots(slots: &mut [Option<Open>; MAXP]) {
    let desired = lock().desired.clone();
    for i in 0..MAXP {
        let want = desired[i].clone();
        let same = slots[i].as_ref().is_some_and(|o| o.path == want) && !want.is_empty();
        if same {
            continue;
        }
        if let Some(mut old) = slots[i].take() {
            shutdown(&mut old);
        }
        if want.is_empty() {
            continue;
        }
        match open_watch(&want) {
            Some(o) => {
                note_failed(&want, false);
                slots[i] = Some(o);
            }
            None => note_failed(&want, true),
        }
    }
}

fn note_failed(path: &str, failed: bool) {
    let k = norm_key(path);
    let mut st = lock();
    if failed {
        if st.failed.len() > 256 {
            st.failed.clear();
        }
        st.failed.insert(k);
    } else {
        st.failed.remove(&k);
    }
}

fn open_watch(path: &str) -> Option<Open> {
    unsafe {
        let opened = crate::winpath::win_long_path(path);
        let handle = CreateFileW(
            &HSTRING::from(&opened),
            FILE_LIST_DIRECTORY.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            None,
        )
        .ok()?;
        let event = CreateEventW(None, true, false, None).ok()?;
        let mut ov = Box::new(OVERLAPPED::default());
        ov.hEvent = event;
        let buf = Box::new([0u8; 16_384]);
        let mut open = Open { path: path.to_string(), handle, event, ov, buf };
        if !issue(&mut open) {
            shutdown(&mut open);
            return None;
        }
        Some(open)
    }
}

fn issue(o: &mut Open) -> bool {
    unsafe {
        o.ov.Internal = 0;
        o.ov.InternalHigh = 0;
        o.ov.hEvent = o.event;
        let filter = FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_DIR_NAME | FILE_NOTIFY_CHANGE_SIZE | FILE_NOTIFY_CHANGE_LAST_WRITE;
        let r = ReadDirectoryChangesW(o.handle, o.buf.as_mut_ptr() as *mut _, o.buf.len() as u32, true, filter, None, Some(o.ov.as_mut()), None);
        match r {
            Ok(()) => true,
            Err(e) => win32_code(&e) == 997, // ERROR_IO_PENDING
        }
    }
}

fn on_changes(o: &mut Open) -> bool {
    let mut n = 0u32;
    let ok = unsafe { GetOverlappedResult(o.handle, o.ov.as_ref(), &mut n, false) };
    unsafe { _ = ResetEvent(o.event) };
    match ok {
        Ok(()) => {
            let events = parse_notify(&o.buf[..n as usize], &o.path);
            apply_events(&events);
        }
        Err(e) => {
            let c = win32_code(&e);
            if c == 1022 {
                // ERROR_NOTIFY_ENUM_DIR: the buffer overflowed, so the whole tree is stale.
                lock().invalidate(&norm_key(&o.path), true);
            } else if c != 995 && c != 0 {
                // 995 is the cancel we issued while switching folders.
                return false;
            }
        }
    }
    issue(o)
}

fn shutdown(o: &mut Open) {
    unsafe {
        _ = CancelIoEx(o.handle, Some(o.ov.as_ref()));
        _ = WaitForSingleObject(o.event, 1000);
        let mut n = 0u32;
        _ = GetOverlappedResult(o.handle, o.ov.as_ref(), &mut n, false);
        _ = CloseHandle(o.handle);
        _ = CloseHandle(o.event);
        o.handle = HANDLE::default();
        o.event = HANDLE::default();
    }
}

fn win32_code(e: &windows::core::Error) -> u32 {
    let c = e.code().0 as u32;
    if c & 0xFFFF_0000 == 0x8007_0000 { c & 0xFFFF } else { c }
}

const FILE_ACTION_REMOVED: u32 = 2;
const FILE_ACTION_RENAMED_OLD: u32 = 4;

fn le_u32(buf: &[u8], off: usize) -> Option<u32> {
    let b = buf.get(off..off + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

pub(crate) fn parse_notify(buf: &[u8], root: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 12 <= buf.len() {
        let Some(next) = le_u32(buf, off) else { break };
        let Some(action) = le_u32(buf, off + 4) else { break };
        let Some(len) = le_u32(buf, off + 8) else { break };
        let len = len as usize;
        let end = off.saturating_add(12).saturating_add(len);
        if end > buf.len() || len % 2 != 0 {
            break;
        }
        let name_bytes = &buf[off + 12..end];
        let units: Vec<u16> = name_bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let name = String::from_utf16_lossy(&units);
        let full = if name.is_empty() { norm_key(root) } else { norm_key(&format!("{root}\\{name}")) };
        let removed = action == FILE_ACTION_REMOVED || action == FILE_ACTION_RENAMED_OLD;
        out.push((full, removed));
        let next = next as usize;
        if next == 0 || off.saturating_add(next) <= off {
            break;
        }
        off += next;
    }
    out
}

fn apply_events(events: &[(String, bool)]) {
    let mut st = lock();
    for (path, removed) in events {
        st.invalidate(path, *removed);
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::UI;

    fn stat(bytes: u64, files: u64, partial: bool) -> FolderStat {
        FolderStat { bytes, files, folders: 1, partial, at: SystemTime::UNIX_EPOCH }
    }

    #[test]
    fn preview_skips_roots_and_unwanted_network() {
        assert!(preview_walk(false, Place::Local, false));
        assert!(preview_walk(false, Place::Wsl, false));
        assert!(!preview_walk(true, Place::Local, true));
        assert!(!preview_walk(false, Place::Network, false));
        assert!(preview_walk(false, Place::Network, true));
    }

    #[test]
    fn keys_parents_and_roots() {
        assert_eq!(norm_key(r"C:\Work\Docs\"), r"c:\work\docs");
        assert_eq!(norm_key(r"C:\"), r"c:\");
        assert_eq!(norm_key(r"\\?\C:\A"), r"c:\a");
        assert_eq!(norm_key(r"\\?\UNC\Server\Share\A"), r"\\server\share\a");
        assert_eq!(parent_key(r"c:\work\docs"), Some(r"c:\work".into()));
        assert_eq!(parent_key(r"c:\work"), Some(r"c:\".into()));
        assert_eq!(parent_key(r"c:\"), None);
        assert_eq!(parent_key(r"\\server\share\a"), Some(r"\\server\share".into()));
        assert_eq!(parent_key(r"\\server\share"), None);
        assert!(is_volume_root(r"C:\"));
        assert!(is_volume_root(r"\\server\share"));
        assert!(!is_volume_root(r"C:\Work"));
        assert!(under(r"c:\a\b", r"c:\a"));
        assert!(!under(r"c:\ab", r"c:\a"));
        assert!(under(r"c:\a\b", r"c:\"));
    }

    #[test]
    fn notify_copies_japanese_names_and_stops_on_a_short_buffer() {
        let name: Vec<u16> = "サンプル.jpg".encode_utf16().collect();
        let mut buf = Vec::new();
        buf.extend(0u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(((name.len() * 2) as u32).to_le_bytes());
        for u in &name {
            buf.extend(u.to_le_bytes());
        }
        let ev = parse_notify(&buf, r"F:\CAC");
        assert_eq!(ev.len(), 1);
        assert!(ev[0].0.contains("サンプル.jpg"), "{}", ev[0].0);
        assert!(parse_notify(&buf[..10], r"F:\CAC").is_empty());
        let mut bad = buf.clone();
        bad[8..12].copy_from_slice(&3u32.to_le_bytes());
        assert!(parse_notify(&bad, r"F:\CAC").is_empty());
    }

    #[test]
    fn place_and_network_gate() {
        assert_eq!(place(r"C:\Work", false), Place::Local);
        assert_eq!(place(r"Z:\Share", true), Place::Network);
        assert_eq!(place(r"\\server\share\a", false), Place::Network);
        assert_eq!(place(r"\\wsl$\Ubuntu\home", false), Place::Wsl);
        assert_eq!(place(r"\\wsl.localhost\Ubuntu\home", true), Place::Wsl);
        assert!(allowed(Place::Local, false));
        assert!(allowed(Place::Wsl, false));
        assert!(!allowed(Place::Network, false));
        assert!(allowed(Place::Network, true));
    }

    #[test]
    fn invalidate_touches_only_the_folder_and_ancestors() {
        let mut st = Store::new();
        let put = |st: &mut Store, k: &str, n: u64| {
            st.cache.insert(k.into(), stat(n, 1, false));
        };
        put(&mut st, r"c:\a", 1);
        put(&mut st, r"c:\a\b", 2);
        put(&mut st, r"c:\a\b\c", 3);
        put(&mut st, r"c:\a\d", 4);
        st.invalidate(r"c:\a\b\c\file.txt", false);
        assert!(st.cache.peek(r"c:\a\b\c").is_none());
        assert!(st.cache.peek(r"c:\a\b").is_none());
        assert!(st.cache.peek(r"c:\a").is_none());
        assert!(st.cache.peek(r"c:\a\d").is_some(), "a sibling stays");
        put(&mut st, r"c:\a\b\c\nested", 5);
        st.invalidate(r"c:\a\b", true);
        assert!(st.cache.peek(r"c:\a\b\c\nested").is_none());
        assert!(st.cache.peek(r"c:\a\d").is_some());
    }

    #[test]
    fn queue_prefers_the_selection_and_drops_a_pane_that_left() {
        assert!(RANK_SELECTED < RANK_HOVER && RANK_HOVER < RANK_VISIBLE);
        let mut st = Store::new();
        st.push(0, 0, r"C:\visible", Place::Local, RANK_VISIBLE);
        st.push(0, 0, r"C:\selected", Place::Local, RANK_SELECTED);
        st.push(1, 0, r"C:\other", Place::Local, RANK_VISIBLE);
        let first = st.pop().unwrap();
        assert_eq!(first.key, r"c:\selected");
        st.gens[1] = 1; // pane 1 navigated
        let next = st.pop().unwrap();
        assert_eq!(next.key, r"c:\visible");
        assert!(st.pop().is_none());
    }

    #[test]
    fn queue_is_bounded_and_keeps_the_better_job() {
        let mut st = Store::new();
        for n in 0..QUEUE_CAP {
            st.push(0, 0, &format!(r"C:\v{n}"), Place::Local, RANK_VISIBLE);
        }
        assert_eq!(st.queue.len(), QUEUE_CAP);
        st.push(0, 0, r"C:\extra", Place::Local, RANK_VISIBLE);
        assert_eq!(st.queue.len(), QUEUE_CAP);
        assert!(st.queue.iter().all(|j| j.key != r"c:\extra"));
        st.push(0, 0, r"C:\now", Place::Local, RANK_SELECTED);
        assert!(st.queue.iter().any(|j| j.key == r"c:\now"));
        assert_eq!(st.pop().unwrap().key, r"c:\now");
    }

    #[test]
    fn cache_caps_entries() {
        let mut c = Cache::new(3, 10_000);
        for n in 0..4 {
            c.insert(format!("c:\\{n}"), stat(n, 1, false));
        }
        assert!(c.peek("c:\\0").is_none());
        assert!(c.peek("c:\\3").is_some());
        assert_eq!(c.map.len(), 3);
    }

    #[test]
    fn selection_line_waits_until_every_folder_is_known() {
        UI.store(0, Ordering::Relaxed);
        assert_eq!(view_selection(&[], 0, 0, false), SizeView::Hidden);
        assert_eq!(view_selection(&[None], 0, 0, false), SizeView::Pending);
        assert_eq!(view_selection(&[Some(stat(1, 1, false)), None], 0, 0, false), SizeView::Pending);
        let ready = view_selection(&[Some(stat(100, 2, false)), Some(stat(50, 3, true))], 7, 1, false);
        assert_eq!(ready, SizeView::Ready { bytes: 157, files: 6, partial: true });
        assert_eq!(view_text(SizeView::Pending).as_deref(), Some("Folder size: calculating…"));
        assert_eq!(view_text(ready).as_deref(), Some("Folder size: 157 B (6 files, partial)"));
        let one = view_selection(&[Some(stat(3_435_973_837, 1204, false))], 0, 0, false);
        let text = view_text(one).unwrap();
        assert!(text.starts_with("Folder size: "), "{text}");
        assert!(text.contains("1,204 files"), "{text}");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(view_text(SizeView::Pending).as_deref(), Some("폴더 크기: 계산 중…"));
        assert!(view_text(one).unwrap().contains("1,204개 파일"));
        UI.store(2, Ordering::Relaxed);
        assert_eq!(view_text(SizeView::Pending).as_deref(), Some("フォルダーのサイズ: 計算中…"));
        assert!(view_text(one).unwrap().contains("1,204 ファイル"));
        UI.store(0, Ordering::Relaxed);
    }

    #[test]
    fn notify_buffer_names_the_changed_path() {
        fn entry(action: u32, name: &str, more: bool) -> Vec<u8> {
            let wide: Vec<u16> = name.encode_utf16().collect();
            let nbytes = wide.len() * 2;
            let mut body = Vec::new();
            body.extend(0u32.to_le_bytes());
            body.extend(action.to_le_bytes());
            body.extend((nbytes as u32).to_le_bytes());
            for u in wide {
                body.extend(u.to_le_bytes());
            }
            while body.len() % 4 != 0 {
                body.push(0);
            }
            if more {
                let n = body.len() as u32;
                body[..4].copy_from_slice(&n.to_le_bytes());
            }
            body
        }
        let mut buf = entry(3, r"sub\a.txt", true);
        buf.extend(entry(FILE_ACTION_REMOVED, "sub", false));
        let ev = parse_notify(&buf, r"C:\Work");
        assert_eq!(ev, vec![(r"c:\work\sub\a.txt".into(), false), (r"c:\work\sub".into(), true)]);
    }

    #[test]
    fn a_content_change_drops_the_cached_size_so_the_status_line_can_ask_again() {
        let mut st = Store::new();
        st.push(0, 0, r"C:\Work", Place::Local, RANK_SELECTED);
        let job = st.pop().unwrap();
        st.cache.insert(job.key.clone(), stat(10, 1, false));
        st.push(0, 0, r"C:\Work", Place::Local, RANK_SELECTED);
        assert!(st.queue.is_empty(), "a cached folder is not walked again");
        st.invalidate(r"c:\work\a.txt", false);
        assert!(st.cache.peek(r"c:\work").is_none());
        st.push(0, 0, r"C:\Work", Place::Local, RANK_SELECTED);
        assert_eq!(st.queue.len(), 1);
        assert_eq!(st.queue[0].rank, RANK_SELECTED);
    }
}
#[cfg(test)]
mod utf8_boundary_tests {
    #[test]
    fn japanese_korean_and_emoji_parent_keys_do_not_panic() {
        assert_eq!(super::parent_key(r"f:\한글폴더\샘플.txt").as_deref(), Some(r"f:\한글폴더"));
        assert_eq!(super::parent_key(r"d:\サンプル\📄.txt").as_deref(), Some(r"d:\サンプル"));
        assert_eq!(super::parent_key(r"e:\😀\あ.txt").as_deref(), Some(r"e:\😀"));
        assert_eq!(super::parent_key(r"c:\abcdサ.txt").as_deref(), Some(r"c:\"));
        assert_eq!(
            super::parent_key(r"\\서버\공유\한글폴더\샘플").as_deref(),
            Some(r"\\서버\공유\한글폴더")
        );
    }
}
