use crate::*;

// ---------- status bar ----------

// Bumped by each pane's selection-change notification; part of the status key.
pub(crate) static SEL_REV: [AtomicUsize; MAXP] = [const { AtomicUsize::new(0) }; MAXP];

// The selection's file system paths, for the size worker. No disk is touched here (the UI thread): a big
// selection or a slow share used to freeze the window while each item's size was read.
// partial: something can't be counted (over 10k items, no file system path).
#[derive(Debug, PartialEq)]
pub(crate) struct Sel {
    pub(crate) paths: Vec<String>,
    pub(crate) partial: bool,
}

// n: the selection's count (None = it couldn't be read), path(k): item k's file system path.
pub(crate) fn collect_sel(n: Option<u32>, path: impl Fn(u32) -> Option<String>) -> Sel {
    let Some(n) = n else { return Sel { paths: Vec::new(), partial: true } };
    // ponytail: sums at most 10k selected items; bigger selections are marked partial
    let mut s = Sel { paths: Vec::with_capacity(n.min(10_000) as usize), partial: n > 10_000 };
    for k in 0..n.min(10_000) {
        match path(k) {
            Some(p) => s.paths.push(p),
            None => s.partial = true,
        }
    }
    s
}

// What the gathered paths belong to: (pane, folder, selected count, SEL_REV, focused row).
// The same key = the same selection, so its paths are used again instead of asked from the shell.
pub(crate) type SelKey = (usize, String, i32, usize, i32);

// Above this many selected items, a selection still changing (Shift+PgDn held) is gathered once it rests.
pub(crate) const BIG_SEL: usize = 2000;
pub(crate) const SEL_REST: Duration = Duration::from_millis(250);

#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum SelStep {
    Reuse,   // the stored paths are this selection's
    Collect, // ask the shell now
    Wait,    // a big selection that just changed: look again next tick
}

// cached: the stored paths have this key, sel: the selected count (big = over BIG_SEL items),
// held: how long this key was seen unchanged (None = it wasn't seen on the last look).
pub(crate) fn sel_step(cached: bool, sel: i32, held: Option<Duration>) -> SelStep {
    if cached {
        SelStep::Reuse
    } else if sel as usize <= BIG_SEL || held.is_some_and(|d| d >= SEL_REST) {
        SelStep::Collect
    } else {
        SelStep::Wait
    }
}

// The stored selection paths are dropped (F5, panes moved: SEL_REV went with them).
pub(crate) fn forget_sel(a: &mut App) {
    a.sel_cache = None;
    a.sel_seen = None;
}

pub(crate) unsafe fn selected_size(fv: &IFolderView2) -> Sel {
    let arr = fv.GetSelection(false).ok();
    let n = arr.as_ref().and_then(|a| a.GetCount().ok());
    collect_sel(n, |k| Some(pwstr(arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?)))
}

// The " (…)" after "n selected". job: the worker's (bytes, done, partial, a folder met), None = no path to sum.
// While it runs with nothing summed and no folder met yet (files on a local disk: done in moments) nothing shows,
// so the line doesn't blink "calculating"; a folder or bytes so far show as before.
// last: the size this one folder had when it was last fully counted; while it is counted again that shows, as is.
pub(crate) fn size_text(ko: bool, job: Option<(u64, bool, bool, bool)>, collect_partial: bool, last: Option<u64>) -> String {
    let (bytes, done, partial, dirs) = job.unwrap_or((0, true, false, false));
    let partial = partial || collect_partial;
    if !done {
        if let Some(b) = last {
            return if ko { format!(" (지난번 {} · 다시 세는 중…)", human(b)) } else { format!(" (last {} · recounting…)", human(b)) };
        }
        return match bytes {
            0 if !dirs => String::new(),
            0 => (if ko { " (계산 중…)" } else { " (calculating…)" }).into(),
            b => format!(" ({}…)", human(b)),
        };
    }
    match (bytes, partial) {
        (0, false) if !dirs => String::new(),
        (0, true) if !dirs => (if ko { " (크기 모름)" } else { " (size unknown)" }).into(),
        (b, true) => format!(" ({}, {})", human(b), if ko { "일부만" } else { "partial" }),
        (b, false) => format!(" ({})", human(b)),
    }
}

// ---------- worker relay ----------
// The free space, selection size and '›' workers are one thread each, and a dead share can hang one call for
// minutes. A worker in one call over STUCK_MS on another drive or share than the request waiting is left behind
// (it ends once that call returns and posts nothing) and a fresh thread takes the requests; at most RELAY_CAP
// are left behind per kind. A request for a volume a left-behind worker hangs on waits for it to come back (a
// fresh thread would only hang there too). A fresh worker's first relay call is blocked() (the lock), so it
// never runs before the relay names it the owner.
pub(crate) const STUCK_MS: u64 = 2000;
pub(crate) const RELAY_CAP: usize = 3;

pub(crate) type Vol = Option<String>; // vol_key of what a worker is on

struct RelayState {
    busy: bool,      // the owner is in a call
    vol: Vol,        // ... for this drive or share
    stuck: Vec<Vol>, // where the left-behind workers hang, one entry each
}

pub(crate) struct WorkerRelay {
    owner: AtomicUsize,                 // the worker taking requests: 1, 2, … (0 = none started)
    beat: std::sync::atomic::AtomicU64, // the owner's last sign of life while busy, ms on relay_ms
    s: Mutex<RelayState>,
}

pub(crate) static DISK_RELAY: WorkerRelay = WorkerRelay::new();
pub(crate) static SIZE_RELAY: WorkerRelay = WorkerRelay::new();
pub(crate) static KIDS_RELAY: WorkerRelay = WorkerRelay::new();

fn relay_ms() -> u64 {
    static CLOCK: OnceLock<Instant> = OnceLock::new();
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64
}

// True = a fresh worker is due: the owner has shown no life for over STUCK_MS in one call (busy_since), on
// another volume than the request waiting, and fewer than `cap` are left behind already.
pub(crate) fn respawn(busy_since: Option<u64>, now: u64, busy_vol: Option<&str>, want_vol: Option<&str>, retired: usize, cap: usize) -> bool {
    busy_since.is_some_and(|t| now.saturating_sub(t) > STUCK_MS) && busy_vol != want_vol && retired < cap
}

impl WorkerRelay {
    pub(crate) const fn new() -> Self {
        WorkerRelay { owner: AtomicUsize::new(0), beat: std::sync::atomic::AtomicU64::new(0), s: Mutex::new(RelayState { busy: false, vol: None, stuck: Vec::new() }) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RelayState> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    // want = the waiting request's volume (None = nothing waits). Starts the first worker, or a fresh one in
    // place of an owner stuck elsewhere.
    pub(crate) fn ensure(&self, want: Option<Vol>, work: fn(usize)) {
        self.ensure_at(relay_ms(), want, work);
    }

    fn ensure_at(&self, now: u64, want: Option<Vol>, work: fn(usize)) -> bool {
        let Some(want) = want else { return false };
        let mut s = self.lock();
        let owner = self.owner.load(Ordering::Relaxed);
        if owner != 0 {
            let since = s.busy.then(|| self.beat.load(Ordering::Relaxed));
            if !respawn(since, now, s.vol.as_deref(), want.as_deref(), s.stuck.len(), RELAY_CAP) || s.stuck.contains(&want) {
                return false;
            }
        }
        let me = owner + 1;
        // the owner moves only once the thread is there: a failed spawn keeps the one there was
        if std::thread::Builder::new().spawn(move || work(me)).is_err() {
            return false;
        }
        if owner != 0 {
            let vol = s.vol.take();
            s.stuck.push(vol);
            s.busy = false;
        }
        self.owner.store(me, Ordering::Relaxed);
        true
    }

    // The owner starts a call that may hang.
    pub(crate) fn busy(&self, me: usize, vol: Vol) {
        self.busy_at(me, vol, relay_ms());
    }

    fn busy_at(&self, me: usize, vol: Vol, now: u64) {
        let mut s = self.lock();
        if self.owner.load(Ordering::Relaxed) == me {
            (s.busy, s.vol) = (true, vol);
            self.beat.store(now, Ordering::Relaxed);
        }
    }

    // A sign of life between calls (no lock: called per entry); false = left behind.
    pub(crate) fn alive(&self, me: usize) -> bool {
        let mine = self.owner.load(Ordering::Relaxed) == me;
        if mine {
            self.beat.store(relay_ms(), Ordering::Relaxed);
        }
        mine
    }

    // The call is back. false = left behind meanwhile: post nothing, wake the owner (blocked() may change)
    // and end.
    pub(crate) fn done(&self, me: usize, vol: &Vol) -> bool {
        let mut s = self.lock();
        if self.owner.load(Ordering::Relaxed) == me {
            s.busy = false;
            return true;
        }
        if let Some(k) = s.stuck.iter().position(|v| v == vol) {
            s.stuck.remove(k);
        }
        false
    }

    // A left-behind worker hangs on this volume: its requests wait.
    pub(crate) fn blocked(&self, vol: &Vol) -> bool {
        self.lock().stuck.contains(vol)
    }
}

// A left-behind worker's goodbye: the job's lock is taken first, so an owner between its blocked() look and
// its wait can't miss the wake.
pub(crate) fn wake_owner<T>(job: &Mutex<T>, wake: &Condvar) {
    drop(job.lock().unwrap_or_else(|e| e.into_inner()));
    wake.notify_one();
}

// Each status tick (300ms): a request waiting behind a worker stuck on a dead share gets a fresh worker, even
// one asked only once (a '›' click) or skipped as already pending.
pub(crate) fn relay_tick() {
    let disk = disk_job().want.as_deref().map(vol_key);
    DISK_RELAY.ensure(disk, disk_worker);
    let size = {
        let j = size_job();
        j.target.first().filter(|_| !j.done).map(|p| vol_key(p))
    };
    SIZE_RELAY.ensure(size, size_worker);
    let kids = {
        let j = kids_job();
        (!j.dir.is_empty() && j.result.is_none()).then(|| vol_key(&j.dir))
    };
    KIDS_RELAY.ensure(kids, kids_worker);
}

// The selection's size is summed by one worker thread. gen changes on every new target or cancel;
// the worker drops results of an older gen and stops once DIR_GEN moves on.
#[derive(Default)]
pub(crate) struct SizeJob {
    pub(crate) gen: usize,
    pub(crate) target: Vec<String>,
    pub(crate) bytes: u64,
    pub(crate) done: bool,
    pub(crate) partial: bool,
    pub(crate) dirs: bool, // a folder was met in the target
}

impl SizeJob {
    // True = a new sum is due. busy (a copy runs) or no paths cancel; the same target keeps
    // running, or keeps its result.
    pub(crate) fn want(&mut self, paths: &[String], busy: bool) -> bool {
        if busy || paths.is_empty() {
            (self.gen, self.done) = (self.gen + 1, false);
            self.target.clear();
            return false;
        }
        if self.target == paths {
            return false;
        }
        *self = SizeJob { gen: self.gen + 1, target: paths.to_vec(), ..Default::default() };
        true
    }

    pub(crate) fn publish(&mut self, gen: usize, bytes: u64, done: bool, partial: bool) -> bool {
        if self.gen != gen {
            return false;
        }
        (self.bytes, self.done, self.partial) = (bytes, done, partial);
        true
    }
}

// Folder sizes counted to the end (the status line's one selected folder, the usage window's subfolders), so a folder
// picked again shows its last size at once while it is counted again. Only in memory; F5 forgets them. The oldest go
// past SIZE_MEMO_MAX; one noted again moves to the back. Keys: the path in lower case (no trailing backslash on
// either side's paths). The lock is never held across file system calls.
pub(crate) const SIZE_MEMO_MAX: usize = 512;

pub(crate) struct SizeMemo {
    items: Vec<(String, u64, bool)>, // key, bytes, partial; oldest first
}

pub(crate) fn memo_key(path: &str) -> String {
    path.to_lowercase()
}

impl SizeMemo {
    pub(crate) const fn new() -> Self {
        SizeMemo { items: Vec::new() }
    }

    pub(crate) fn get(&self, path: &str) -> Option<(u64, bool)> {
        let k = memo_key(path);
        self.items.iter().find(|(p, ..)| *p == k).map(|&(_, b, q)| (b, q))
    }

    pub(crate) fn put(&mut self, path: &str, bytes: u64, partial: bool) {
        self.forget(path);
        if self.items.len() >= SIZE_MEMO_MAX {
            self.items.drain(..=self.items.len() - SIZE_MEMO_MAX);
        }
        self.items.push((memo_key(path), bytes, partial));
    }

    pub(crate) fn forget(&mut self, path: &str) {
        let k = memo_key(path);
        self.items.retain(|(p, ..)| *p != k);
    }

    pub(crate) fn clear(&mut self) {
        self.items.clear();
    }
}

static SIZE_MEMO: Mutex<SizeMemo> = Mutex::new(SizeMemo::new());

pub(crate) fn size_memo() -> std::sync::MutexGuard<'static, SizeMemo> {
    SIZE_MEMO.lock().unwrap_or_else(|e| e.into_inner())
}

// What the worker learnt about a one-item target counted to the end: a folder's size is noted; anything else
// (gone, a file of that name, a drive root) drops what was noted for that path.
pub(crate) fn memo_result(memo: &mut SizeMemo, target: &[String], bytes: u64, partial: bool, dir: bool) {
    let [p] = target else { return };
    if dir {
        memo.put(p, bytes, partial);
    } else {
        memo.forget(p);
    }
}

pub(crate) static DIR_SIZE: Mutex<SizeJob> = Mutex::new(SizeJob { gen: 0, target: Vec::new(), bytes: 0, done: false, partial: false, dirs: false });
pub(crate) static DIR_WAKE: Condvar = Condvar::new();
pub(crate) static DIR_GEN: AtomicUsize = AtomicUsize::new(0); // copy of DIR_SIZE.gen, read without the lock
// The main window, for the workers' "done" messages (0 = not known yet).
pub(crate) static STATUS_MAIN: AtomicUsize = AtomicUsize::new(0);
pub(crate) const WM_SIZE_DONE: u32 = WM_APP + 62; // the selection's size is summed: the status line shows it now
pub(crate) const WM_DISK_DONE: u32 = WM_APP + 63; // a free space answer is in DISK.got

pub(crate) fn size_job() -> std::sync::MutexGuard<'static, SizeJob> {
    DIR_SIZE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn want_size(paths: &[String], busy: bool) {
    let mut j = size_job();
    let fresh = j.want(paths, busy);
    DIR_GEN.store(j.gen, Ordering::Relaxed);
    drop(j);
    if fresh {
        SIZE_RELAY.ensure(Some(sel_vol(paths)), size_worker);
        DIR_WAKE.notify_one();
    }
}

// The volume a selection is on (its first path's; a selection comes from one folder).
fn sel_vol(target: &[String]) -> Vol {
    target.first().and_then(|p| vol_key(p))
}

fn tell_main(msg: u32) {
    let main = STATUS_MAIN.load(Ordering::Relaxed);
    if main != 0 {
        _ = unsafe { PostMessageW(Some(HWND(main as _)), msg, WPARAM(0), LPARAM(0)) };
    }
}

// Lives until the process ends, or until SIZE_RELAY leaves it behind. The lock is never held across file
// system calls. Stuck = no stop() look (one per path and entry, also while paused) for STUCK_MS, so a big
// folder still being walked or a sum paused by a copy is never taken for stuck.
pub(crate) fn size_worker(me: usize) {
    let mut seen = 0;
    loop {
        let (gen, target) = {
            let mut j = size_job();
            while j.target.is_empty() || j.gen == seen || SIZE_RELAY.blocked(&sel_vol(&j.target)) {
                j = DIR_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
            (j.gen, j.target.clone())
        };
        seen = gen;
        let vol = sel_vol(&target);
        SIZE_RELAY.busy(me, vol.clone());
        let stop = || !SIZE_RELAY.alive(me) || DIR_GEN.load(Ordering::Relaxed) != gen;
        // a copy only pauses the sum; the status line cancels it when it sees the copy
        let paused = || COPYING.load(Ordering::Relaxed) > 0;
        let tell = |b, done, p, d| {
            let mut j = size_job();
            let ok = j.publish(gen, b, done, p);
            if ok {
                j.dirs |= d;
            }
            ok
        };
        let mut progress = |b, p, d| _ = tell(b, false, p, d);
        // only the end is posted; progress waits for the status tick
        // cancelled (None): what was noted stays; a finished count is right even if the selection moved on
        let sum = sel_bytes(&target, &stop, &paused, &mut progress);
        if !SIZE_RELAY.done(me, &vol) {
            wake_owner(&DIR_SIZE, &DIR_WAKE);
            return;
        }
        if let Some((b, p, d)) = sum {
            memo_result(&mut size_memo(), &target, b, p, d);
            if tell(b, true, p, d) {
                tell_main(WM_SIZE_DONE);
            }
        }
    }
}

// What a selected path is, following links as before.
#[derive(Debug, PartialEq)]
pub(crate) enum SelKind {
    File(u64),
    Dir,
    Unknown, // a drive or share root (its size is the free-space part), gone or unreadable
}

pub(crate) fn sel_kind(p: &str) -> SelKind {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() && Path::new(p).parent().is_some() => SelKind::Dir,
        Ok(m) if m.is_file() => SelKind::File(m.len()),
        _ => SelKind::Unknown,
    }
}

// The selection's (bytes, partial, folder met): files by their size, folders walked (dir_bytes), anything else
// partial. None = `stop` turned true; checked before each path, also while `paused`. progress(bytes, partial,
// folder met) every 2000 paths or 200ms, as each folder starts and while it walks.
pub(crate) fn sel_bytes(target: &[String], stop: &dyn Fn() -> bool, paused: &dyn Fn() -> bool, progress: &mut dyn FnMut(u64, bool, bool)) -> Option<(u64, bool, bool)> {
    let go = || {
        while paused() && !stop() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        !stop()
    };
    let (mut total, mut partial, mut dirs, mut last) = (0, false, false, Instant::now());
    for (k, p) in target.iter().enumerate() {
        if !go() {
            return None;
        }
        match sel_kind(p) {
            SelKind::File(n) => total += n,
            SelKind::Dir => {
                dirs = true;
                progress(total, partial, true);
                let (base, was) = (total, partial);
                let (b, q) = dir_bytes(Path::new(p), stop, paused, &mut |b, q| progress(base + b, was || q, true))?;
                (total, partial) = (total + b, partial || q);
            }
            SelKind::Unknown => partial = true,
        }
        if (k + 1) % 2000 == 0 || last.elapsed().as_millis() >= 200 {
            progress(total, partial, dirs);
            last = Instant::now();
        }
    }
    Some((total, partial, dirs))
}

// Free space is asked by one worker too: a dead share can hang the call (DISK_RELAY then starts a fresh one
// for another drive). Only the newest request waits;
// got is the last answer, (path asked, (free, total) or None = failed).
// asking = the path the worker is on now, at = when `got` came (a path just answered is not asked again).
pub(crate) struct DiskJob {
    pub(crate) want: Option<String>,
    pub(crate) got: Option<(String, Option<(u64, u64)>)>,
    asking: Option<String>,
    at: Option<std::time::Instant>,
}

static DISK: Mutex<DiskJob> = Mutex::new(DiskJob { want: None, got: None, asking: None, at: None });
static DISK_WAKE: Condvar = Condvar::new();

fn disk_job() -> std::sync::MutexGuard<'static, DiskJob> {
    DISK.lock().unwrap_or_else(|e| e.into_inner())
}

// "C:" when `path` starts with a drive letter; a shell path like "::{…}" (This PC) is not a drive.
fn drive(path: &str) -> Option<&str> {
    drive_letter(path).map(|_| &path[..2])
}

// The drive ("c:") or share ("\\server\share") a path is on, lower case; None = neither (no free space shown).
pub(crate) fn vol_key(path: &str) -> Option<String> {
    if let Some(d) = drive(path) {
        return Some(d.to_lowercase());
    }
    let mut it = path.strip_prefix(r"\\")?.split('\\');
    let (server, share) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
    Some(format!(r"\\{server}\{share}").to_lowercase())
}

fn disk_label(path: &str) -> String {
    drive(path).map_or_else(|| tr("Network", "네트워크").to_string(), str::to_string)
}

// The last answer for the drive or share `path` is on, kept while a new one is asked (no blink on the same
// drive); another drive or share starts empty.
fn known_disk(path: &str) -> Option<(String, u64, u64)> {
    let v = vol_key(path)?;
    let j = disk_job();
    let (p, got) = j.got.as_ref()?;
    let (free, total) = got.filter(|_| vol_key(p).as_ref() == Some(&v))?;
    Some((disk_label(path), free, total))
}

// Not asked again while the same path is pending or being asked, or was answered under a second ago: a
// folder whose item count keeps changing would otherwise chain answer -> status_base -> ask on the UI thread.
fn ask_disk(path: &str) {
    {
        let mut j = disk_job();
        let fresh = j.got.as_ref().is_some_and(|(p, _)| p == path) && j.at.is_some_and(|t| t.elapsed().as_millis() < 1000);
        if j.want.as_deref() == Some(path) || j.asking.as_deref() == Some(path) || fresh {
            return;
        }
        j.want = Some(path.to_string());
    }
    DISK_RELAY.ensure(Some(vol_key(path)), disk_worker);
    DISK_WAKE.notify_one();
}

// A worker left behind (DISK_RELAY) touches neither got, asking nor at, and posts no WM_DISK_DONE.
fn disk_worker(me: usize) {
    loop {
        let path = {
            let mut j = disk_job();
            loop {
                if let Some(p) = j.want.take_if(|p| !DISK_RELAY.blocked(&vol_key(p))) {
                    j.asking = Some(p.clone());
                    break p;
                }
                j = DISK_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
        };
        let vol = vol_key(&path);
        DISK_RELAY.busy(me, vol.clone());
        let (mut free, mut total) = (0, 0);
        let ok = unsafe { GetDiskFreeSpaceExW(&HSTRING::from(path.as_str()), Some(&mut free), Some(&mut total), None) }.is_ok();
        if !DISK_RELAY.done(me, &vol) {
            wake_owner(&DISK, &DISK_WAKE);
            return;
        }
        {
            let mut j = disk_job();
            j.got = Some((path, ok.then_some((free, total))));
            j.asking = None;
            j.at = Some(std::time::Instant::now());
        }
        tell_main(WM_DISK_DONE);
    }
}

// WM_DISK_DONE on the main window: the answer counts while the active pane is on that drive or share and
// no copy runs (the line shows the copy then). The drive zone is set here: status_base only runs on a change.
pub(crate) unsafe fn disk_done() {
    let Some((p, got)) = disk_job().got.clone() else { return };
    if COPYING.load(Ordering::Relaxed) > 0 {
        return;
    }
    let fresh = app(|a| {
        let cur = &a.s.paths[a.s.active];
        let v = vol_key(cur);
        if v.is_none() || v != vol_key(&p) {
            return false;
        }
        a.status_base.3 = got.map(|(free, total)| (disk_label(cur), free, total));
        true
    });
    if fresh {
        update_status();
    }
}

// Bytes under `root`, not following links or junctions. Unreadable parts set the flag and are skipped.
// None = `stop` turned true. Checked before each directory read and each entry, also while `paused`;
// a single read_dir/metadata call that hangs (a dead share) still has to return first.
pub(crate) fn dir_bytes(root: &Path, stop: &dyn Fn() -> bool, paused: &dyn Fn() -> bool, progress: &mut dyn FnMut(u64, bool)) -> Option<(u64, bool)> {
    let go = || {
        while paused() && !stop() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        !stop()
    };
    match std::fs::symlink_metadata(root) {
        Ok(m) if m.is_dir() => {}
        Ok(m) => return Some((if m.is_file() { m.len() } else { 0 }, false)),
        Err(_) => return Some((0, true)),
    }
    let (mut sum, mut partial, mut n, mut last) = (0, false, 0usize, Instant::now());
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if !go() {
            return None;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            partial = true;
            continue;
        };
        for e in rd {
            if !go() {
                return None;
            }
            let Ok(e) = e else {
                partial = true;
                continue;
            };
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => match e.metadata() {
                    Ok(m) => sum += m.len(),
                    Err(_) => partial = true,
                },
                Ok(_) => {} // a link or junction
                Err(_) => partial = true,
            }
            n += 1;
            if n % 2000 == 0 || last.elapsed().as_millis() >= 200 {
                progress(sum, partial);
                last = Instant::now();
            }
        }
    }
    Some((sum, partial))
}

// F5 in a file list: the shell reloads it, and the selection is summed again (no last sizes meanwhile).
pub(crate) fn resum_sizes() {
    size_memo().clear();
    want_size(&[], true);
    app(|a| {
        a.status_key.clear();
        forget_sel(a);
    });
}

// Polled by a 300ms timer; the expensive parts only run when the active pane's state changed.
// Other ticks only merge the folder size worker's progress into the stored text.
pub(crate) unsafe fn update_status() {
    mark_drive();
    relay_tick();
    let (i, path, main) = app(|a| (a.s.active, a.s.paths[a.s.active].clone(), a.main));
    // no list yet (a folder still opening): the counts wait, a new note still shows and is read out
    if let Some(fv) = view_of::<IFolderView2>(i) {
        let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
        let sel = fv.ItemCount(SVGIO_SELECTION).unwrap_or(0);
        let spec = filter_of(i);
        // while a background copy runs, the disk is left alone (size and free space come back after)
        let busy = COPYING.load(Ordering::Relaxed) > 0;
        let awake = AWAKE.load(Ordering::Relaxed);
        let zoom = app(|a| a.zoom);
        let rev = SEL_REV[i].load(Ordering::Relaxed);
        let key = format!("{i}|{path}|{n}|{sel}|{}|{spec}|{busy}|{awake}|{zoom:?}|{rev}", fv.GetFocusedItem().unwrap_or(-1));
        // a big selection still changing isn't settled: no key is kept (not the last settled one either:
        // the selection may go back to it), so the next look runs this again
        if app(|a| a.status_key != key) {
            let settled = status_base(i, &path, main, &fv, n, sel, &spec, busy, zoom);
            app(|a| a.status_key = if settled { key } else { String::new() });
        }
    }
    let (job, one) = {
        let j = size_job();
        let one = match j.target.as_slice() {
            [p] if !j.done => Some(p.clone()),
            _ => None,
        };
        ((!j.target.is_empty()).then_some((j.bytes, j.done, j.partial, j.dirs)), one)
    };
    // one item still being counted: its last full count, looked up after the job's lock is let go
    let last = one.and_then(|p| size_memo().get(&p)).map(|(b, _)| b);
    let ko = KO.load(Ordering::Relaxed);
    let (changed, fresh, ctl) = app(|a| {
        let (head, size, mid, drive) = &a.status_base;
        let size = size.map(|p| size_text(ko, job, p, last)).unwrap_or_default();
        // a short confirmation (path copied, missing folder) leads for a few seconds in its own chip
        if let Some(saved) = SAVED.lock().unwrap_or_else(|e| e.into_inner()).take() {
            a.note = Some((saved, Instant::now() + Duration::from_secs(5)));
            a.lost = 0;
        }
        a.note = a.note.take().filter(|(_, t)| Instant::now() < *t);
        if a.note.is_none() {
            a.lost = 0;
        }
        // every zone's value (the note, the size progress, raw free/total bytes) takes part in the compare
        let line = StatusLine { note: a.note.as_ref().map(|(s, _)| s.clone()), head: format!("{head}{size}"), mid: mid.clone(), drive: drive.clone() };
        // each note set anew (its own end time) is read out once, the same words too; expiry, counts and size don't
        let fresh = a.note.as_ref().filter(|n| a.said != Some(n.1)).map(|(s, t)| (s.clone(), *t));
        if let Some((_, t)) = &fresh {
            a.said = Some(*t);
        }
        let changed = (a.status != line).then(|| {
            let words = status_speech(&line);
            a.status = line;
            words
        });
        (changed, fresh.map(|(s, _)| (s, a.note_hwnd)), a.status_hwnd)
    });
    // outside the borrow: setting the text and the event may call straight back in from a screen reader
    if let Some(words) = changed {
        if !ctl.0.is_null() {
            _ = SetWindowTextW(ctl, &HSTRING::from(words)); // the whole line, read on request
        }
        status_dirty();
    }
    // only the note itself is read out by itself, not the counts and free space around it
    if let Some((note, live)) = fresh.filter(|(_, h)| !h.0.is_null()) {
        _ = SetWindowTextW(live, &HSTRING::from(note));
        NotifyWinEvent(EVENT_OBJECT_LIVEREGIONCHANGED, live, OBJID_CLIENT.0, CHILDID_SELF as i32);
    }
}

// Words for screen readers only (pane / tab moved by key): the live region's text, nothing drawn. The status line,
// a.note and a.said stay as they are, so a note showing is neither hidden nor read out again. Called outside app():
// the event may call straight back in. No live region (the status line failed): nothing.
pub(crate) unsafe fn say_only(text: &str) {
    let live = app(|a| a.note_hwnd);
    if live.0.is_null() {
        return;
    }
    _ = SetWindowTextW(live, &HSTRING::from(text));
    NotifyWinEvent(EVENT_OBJECT_LIVEREGIONCHANGED, live, OBJID_CLIENT.0, CHILDID_SELF as i32);
}

// Shortcut tips (Key Promoter): the mouse just did something a key also does. Its key shows on the status line for
// 4 seconds, once per run and at most TIP_MAX times in all; never over another note (path copied, missing folder,
// Everything not running) still showing or waiting to show.
pub(crate) unsafe fn mouse_tip(action: usize) {
    if SAVED.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        return;
    }
    let ko = KO.load(Ordering::Relaxed);
    let now = Instant::now();
    let shown = app(|a| {
        // an expired note stays Some until the next update_status: its end time decides
        let busy = a.note.as_ref().is_some_and(|(_, t)| now < *t);
        if busy || a.tips_now & (1 << action) != 0 {
            return false;
        }
        let Some(tip) = key_tip(action, a.s.key_tips_seen[action], a.s.key_tips, ko) else { return false };
        a.tips_now |= 1 << action;
        a.s.key_tips_seen[action] += 1;
        a.note = Some((tip, now + Duration::from_secs(4)));
        true
    });
    if shown {
        update_status();
    }
}

// The key itself was used: its tip never shows again.
pub(crate) fn key_used(action: usize) {
    app(|a| a.s.key_tips_seen[action] = TIP_LEARNED);
}

// ---------- the status line for screen readers ----------
// An owner-drawn static on the status line, like Explorer's and Notepad's status bars: its text is the whole line
// (a screen reader's "read status bar") and its role a status bar. Next to it a zero-size static holds only the
// note and is a polite live region, so a new note is read out by itself without the counts and free space.
// Without SS_NOTIFY both hit-test transparent, so clicks still go to the main window.
// Failing leaves the main window painting the line, as before; a failed annotation only loses the role or the reading.
pub(crate) unsafe fn create_status(main: HWND) {
    const SS_OWNERDRAW: u32 = 0x0D;
    let h = child(main, w!("STATIC"), w!(""), SS_OWNERDRAW, WINDOW_EX_STYLE(0), ID_STATUS, HFONT::default());
    if h.0.is_null() {
        return;
    }
    let live = child(main, w!("STATIC"), w!(""), 0, WINDOW_EX_STYLE(0), ID_STATUS_NOTE, HFONT::default());
    if let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) {
        let (obj, me) = (OBJID_CLIENT.0 as u32, CHILDID_SELF);
        _ = acc.SetHwndProp(h, obj, me, PROPID_ACC_ROLE, &VARIANT::from(ROLE_SYSTEM_STATUSBAR as i32));
        if !live.0.is_null() {
            _ = acc.SetHwndProp(live, obj, me, LiveSetting_Property_GUID, &VARIANT::from(Polite.0));
        }
    }
    app(|a| (a.status_hwnd, a.note_hwnd) = (h, live));
}

// Before the main window goes (its children go after its WM_DESTROY): the annotations come off.
pub(crate) unsafe fn drop_status() {
    let (h, live) = app(|a| (std::mem::take(&mut a.status_hwnd), std::mem::take(&mut a.note_hwnd)));
    if h.0.is_null() && live.0.is_null() {
        return;
    }
    if let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) {
        let (obj, me) = (OBJID_CLIENT.0 as u32, CHILDID_SELF);
        if !h.0.is_null() {
            _ = acc.ClearHwndProps(h, obj, me, &[PROPID_ACC_ROLE]);
        }
        if !live.0.is_null() {
            _ = acc.ClearHwndProps(live, obj, me, &[LiveSetting_Property_GUID]);
        }
    }
}

// Repaint the status line: its control, or that strip of the main window when there is none.
pub(crate) unsafe fn status_dirty() {
    let (main, ctl, rc) = app(|a| (a.main, a.status_hwnd, status_rect(&geo(a))));
    if !ctl.0.is_null() {
        _ = InvalidateRect(Some(ctl), None, false); // draw_status covers all of it
    } else if !main.0.is_null() {
        _ = InvalidateRect(Some(main), Some(&rc), true);
    }
}

// What a screen reader hears for the line: the note first, then items, the middle zone and the drive.
pub(crate) fn status_speech(st: &StatusLine) -> String {
    let mut parts: Vec<String> = st.note.iter().cloned().collect();
    parts.push(st.head.replace(ZONE_DOT, ", "));
    parts.extend(st.mid.iter().cloned());
    if let Some((label, free, total)) = &st.drive {
        parts.push(if KO.load(Ordering::Relaxed) {
            format!("{label} 여유 {} / 전체 {}", human(*free), human(*total))
        } else {
            format!("{label} {} free of {}", human(*free), human(*total))
        });
    }
    parts.retain(|p| !p.trim().is_empty());
    parts.join(", ")
}

// What the status line shows, by zone: [note chip] [items · selected (size)] | [filter · copying · maximized] ... [drive].
#[derive(Clone, Default, PartialEq)]
pub(crate) struct StatusLine {
    pub(crate) note: Option<String>,
    pub(crate) head: String,
    pub(crate) mid: Vec<String>,
    pub(crate) drive: Option<(String, u64, u64)>, // label, free and total bytes
}

// Inside a zone items are joined by a dot; zones are split by a thin line.
pub(crate) const ZONE_DOT: &str = "  ·  ";

// The drive bar: (used share, 90% or more used). None = no total to compare with.
pub(crate) fn drive_use(free: u64, total: u64) -> Option<(f64, bool)> {
    (total > 0).then(|| {
        let used = total.saturating_sub(free);
        (used as f64 / total as f64, used as u128 * 10 >= total as u128 * 9)
    })
}

// The bar's fill: red from 90% used, else the selection colour, or the theme's grey (text, if that too
// fades) when the colour fades into the track (Gray or Peach on light, a pale custom colour). The usual fill is
// never the warning colour itself: under high contrast both would be the highlight, so the ground stands in.
pub(crate) fn bar_fill(p: &Palette, accent: u32, warn: bool) -> u32 {
    if warn {
        return p.warn;
    }
    [accent, p.sub, p.text].into_iter().find(|&c| c != p.warn && contrast(c, p.line) >= 1.5).unwrap_or(p.card)
}

// Where the zones go between left and right: x spans, None = not shown.
#[derive(Debug, PartialEq)]
pub(crate) struct StatusFit {
    pub(crate) note: Option<(i32, i32)>,
    pub(crate) head: (i32, i32),
    pub(crate) mid: Option<(i32, i32)>,
    pub(crate) drive: Option<(i32, i32, usize)>, // span, and which of its forms
}

// Widths in, spans out. The note chip leads and may take whatever the items zone and the full drive zone
// leave over, and at least half the line. The drive zone hugs the right edge and
// steps down its forms (widest first: bar + text, text, drive and free only) only once the items zone would
// get under min_head; when even the last form doesn't fit, it takes at most half of what's left.
// The middle zone gets the rest, so it is cut first.
#[allow(clippy::too_many_arguments)]
pub(crate) fn status_fit(left: i32, right: i32, gap: i32, min_head: i32, note: Option<i32>, head: i32, mid: Option<i32>, drive: &[i32]) -> StatusFit {
    let all = (right - left).max(0);
    let cap = (all / 2).max(all - gap - head - drive.first().map_or(0, |&w| gap + w));
    let note = note.map(|w| (left, left + w.min(cap)));
    let x = note.map_or(left, |(_, r)| r + gap);
    let room = (right - x).max(0);
    let need = head.min(min_head);
    let pick = match drive.iter().position(|&w| need + gap + w <= room) {
        Some(k) => Some((k, drive[k])),
        None => drive.len().checked_sub(1).map(|k| (k, drive[k].min(room / 2 - gap))),
    };
    let drive = pick.filter(|&(_, w)| w > 0).map(|(k, w)| (right - w, right, k));
    let end = drive.map_or(right, |(l, ..)| l - gap);
    let head = (x, x + head.min(end - x).max(0));
    let mx = head.1 + gap;
    let mid = mid.map(|w| (mx, (mx + w).min(end))).filter(|&(l, r)| r - l >= 3 * gap);
    StatusFit { note, head, mid, drive }
}

// The middle zone during a background job; "sleep held off" only while the power request is really set.
pub(crate) fn copy_note(ko: bool, awake: bool) -> &'static str {
    match (ko, awake) {
        (false, false) => "Copying / moving…",
        (false, true) => "Copying / moving… · sleep held off",
        (true, false) => "복사·이동 중…",
        (true, true) => "복사·이동 중… · 절전 안 함",
    }
}

// The selection's size: Some(partial while collecting) = a size to show. The paths are asked from the shell
// (up to 10k COM calls) only when the selection itself changed, not when the item count, filter or a copy did;
// a big selection that just changed waits for a look where it stayed the same for SEL_REST.
// false = waiting: the counts show now, the size once the selection rests.
unsafe fn selection_size(i: usize, path: &str, fv: &IFolderView2, sel: i32, busy: bool) -> (Option<bool>, bool) {
    if sel <= 0 || busy {
        // a copy only pauses: the stored paths are summed again once it's done
        want_size(&[], busy);
        if sel <= 0 {
            // nothing selected now: the paths go to Num/'s memory (Esc, a click on empty space)
            app(|a| {
                if let Some((k, s)) = a.sel_cache.take() {
                    remember_sel(&mut a.sel_prev, &k.1, s, &[]);
                }
                forget_sel(a);
            });
        }
        return (None, true);
    }
    let key: SelKey = (i, path.to_string(), sel, SEL_REV[i].load(Ordering::Relaxed), fv.GetFocusedItem().unwrap_or(-1));
    let (cache, seen) = app(|a| (a.sel_cache.take(), a.sel_seen.take()));
    // another pane or folder: its paths go (up to 10k strings), to Num/'s memory under the folder they were in
    let (cache, gone) = match cache {
        Some(c) if c.0 .0 == key.0 && c.0 .1 == key.1 => (Some(c), None),
        c => (None, c),
    };
    if let Some((k, s)) = gone {
        app(|a| remember_sel(&mut a.sel_prev, &k.1, s, &[]));
    }
    let since = seen.filter(|(k, _)| *k == key).map(|(_, t)| t);
    match sel_step(cache.as_ref().is_some_and(|(k, _)| *k == key), sel, since.map(|t| t.elapsed())) {
        SelStep::Wait => {
            // only the focus moved: the same items stay selected, the size shown and its sum go on
            // (unless nothing is being summed, e.g. right after a copy: then no "(size unknown)" blink)
            let summing = !size_job().target.is_empty();
            let size = cache.as_ref().filter(|(k, _)| summing && (k.2, k.3) == (key.2, key.3)).map(|(_, s)| s.partial);
            // the first sighting's time is kept, so other calls meanwhile don't push the wait back
            app(|a| (a.sel_cache, a.sel_seen) = (cache, Some((key, since.unwrap_or_else(Instant::now)))));
            (size, false)
        }
        step => {
            let (k, s) = match (step, cache) {
                (SelStep::Reuse, Some(c)) => c,
                (_, old) => {
                    let s = selected_size(fv);
                    // the selection it replaces goes to Num/'s memory, unless the new one holds all its items
                    // (a selection being grown: each step would push the real earlier choice out)
                    if let Some((ok, os)) = old.filter(|(_, os)| !sub_sel(&os.paths, &s.paths)) {
                        app(|a| remember_sel(&mut a.sel_prev, &ok.1, os, &s.paths));
                    }
                    (key, s)
                }
            };
            // reused too: after a copy the target was cleared, so the same paths are summed again
            want_size(&s.paths, false);
            let partial = s.partial;
            app(|a| a.sel_cache = Some((k, s)));
            (Some(partial), true)
        }
    }
}

// The status text around the selection size, the window title, and a new folder size job.
// false = not settled (a big selection still changing): it runs again on the next look.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn status_base(i: usize, path: &str, main: HWND, fv: &IFolderView2, n: i32, sel: i32, spec: &str, busy: bool, zoom: Option<usize>) -> bool {
    let title = window_title(&app(|a| a.panes[i].as_ref().map(|p| p.disp.clone()).unwrap_or_default()));
    if text_of(main) != title {
        _ = SetWindowTextW(main, &HSTRING::from(title));
    }
    let ko = KO.load(Ordering::Relaxed);
    let mut parts = vec![if ko { format!("항목 {n}개") } else { format!("{n} items") }];
    if sel > 0 {
        parts.push(if ko { format!("{sel}개 선택") } else { format!("{sel} selected") });
    }
    STATUS_MAIN.store(main.0 as usize, Ordering::Relaxed);
    let (size, settled) = selection_size(i, path, fv, sel, busy);
    let head = parts.join(ZONE_DOT);
    let mut parts = Vec::new();
    let mut disk = None;
    if !spec.is_empty() {
        parts.push(format!("{} {spec}", tr("Filter", "필터")));
    }
    if busy {
        parts.push(copy_note(ko, AWAKE.load(Ordering::Relaxed)).into());
    } else if vol_key(path).is_some() {
        // asked off the UI thread (a dead share would stall it); the last answer for this drive stays meanwhile
        disk = known_disk(path);
        ask_disk(path);
    }
    if let Some(z) = zoom {
        let z = z + 1;
        parts.push(if ko {
            format!("패널 {z} 크게 보는 중 · Ctrl+Shift+M 또는 머리줄 더블클릭으로 복원")
        } else {
            format!("Pane {z} maximized · Ctrl+Shift+M or double-click its header to restore")
        });
    }
    app(|a| a.status_base = (head, size, parts, disk));
    settled
}

#[cfg(test)]
mod tests {
    use super::*;

    // The status bar names the drive as the path writes it ("c:", not "C:"); vol_key lower-cases it only to compare.
    #[test]
    fn drive_keeps_its_case() {
        assert_eq!([drive(r"c:\x"), drive(r"D:\"), drive("e:"), drive("C:x")], [Some("c:"), Some("D:"), Some("e:"), Some("C:")]);
        for p in [r"\\srv\share", r"\\?\C:\x", THIS_PC, r"1:\", r"가:\", "", "C"] {
            assert_eq!(drive(p), None, "{p}");
        }
    }

    fn nop(_: usize) {}

    #[test]
    fn respawn_only_when_stuck_elsewhere() {
        let (z, c) = (Some(r"\\nas\share"), Some("c:"));
        assert!(!respawn(None, 9000, z, c, 0, 3)); // idle
        assert!(!respawn(Some(1000), 2500, z, c, 0, 3)); // under 2s
        assert!(!respawn(Some(1000), 3000, z, c, 0, 3)); // just 2s
        assert!(!respawn(Some(1000), 9000, z, z, 0, 3)); // same share: a fresh one would hang there too
        assert!(!respawn(Some(1000), 9000, z, c, 3, 3)); // cap reached
        assert!(respawn(Some(1000), 3001, z, c, 0, 3));
        assert!(respawn(Some(1000), 9000, None, c, 2, 3));
    }

    #[test]
    fn relay_hands_over_and_drops_the_late_worker() {
        let r = WorkerRelay::new();
        let (z, c) = (Some("z:".to_string()), Some("c:".to_string()));
        assert!(!r.ensure_at(0, None, nop)); // nothing asked: no thread
        assert!(r.ensure_at(0, Some(z.clone()), nop)); // the first worker (1)
        assert!(!r.ensure_at(0, Some(c.clone()), nop)); // idle: it takes the request itself
        r.busy_at(1, z.clone(), 1000);
        assert!(!r.ensure_at(2500, Some(c.clone()), nop)); // not stuck long enough
        assert!(!r.ensure_at(9000, Some(z.clone()), nop)); // same drive: waits
        assert!(r.ensure_at(3500, Some(c.clone()), nop)); // worker 2 takes over
        assert!(r.blocked(&z) && !r.blocked(&c));
        assert!(!r.ensure_at(9000, Some(z.clone()), nop)); // z: waits for the one hanging there
        r.busy_at(2, c.clone(), 3600);
        assert!(!r.alive(1) && r.alive(2));
        // worker 1 comes back: it posts nothing, z: is free again; worker 2's answer counts
        assert!(!r.done(1, &z));
        assert!(!r.blocked(&z));
        assert!(r.done(2, &c));
        assert!(!r.ensure_at(99_000, Some(z), nop)); // idle owner: no fresh thread
    }

    #[test]
    fn relay_leaves_at_most_cap_behind() {
        let r = WorkerRelay::new();
        let c =|| Some(Some("c:".to_string()));
        let share = |k: usize| Some(format!(r"\\s{k}\x"));
        assert!(r.ensure_at(0, c(), nop));
        for k in 1..=RELAY_CAP {
            r.busy_at(k, share(k), 0);
            assert!(r.ensure_at(5000, c(), nop), "{k}");
        }
        r.busy_at(RELAY_CAP + 1, Some("d:".into()), 0);
        assert!(!r.ensure_at(5000, c(), nop)); // the owner just waits now
        assert!(!r.done(1, &share(1)));
        assert!(r.ensure_at(5000, c(), nop)); // room again
    }
}
