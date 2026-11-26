use crate::*;

// ---------- status bar ----------

// Bumped by each pane's selection-change notification; part of the status key.
pub(crate) static SEL_REV: [AtomicUsize; MAXP] = [const { AtomicUsize::new(0) }; MAXP];

// The selection split into summed file bytes and folders left to the size worker.
// partial: something wasn't counted (over 10k items, no file system path, unreadable, a drive root).
#[derive(Debug, PartialEq)]
pub(crate) struct Sel {
    pub(crate) file_bytes: u64,
    pub(crate) dirs: Vec<String>,
    pub(crate) partial: bool,
}

// n: the selection's count (None = it couldn't be read), path(k): item k's file system path.
pub(crate) fn collect_sel(n: Option<u32>, path: impl Fn(u32) -> Option<String>) -> Sel {
    let Some(n) = n else { return Sel { file_bytes: 0, dirs: Vec::new(), partial: true } };
    // ponytail: sums at most 10k selected items; bigger selections are marked partial
    let mut s = Sel { file_bytes: 0, dirs: Vec::new(), partial: n > 10_000 };
    for k in 0..n.min(10_000) {
        let Some(p) = path(k) else {
            s.partial = true;
            continue;
        };
        match std::fs::metadata(&p) {
            Ok(m) if m.is_dir() && Path::new(&p).parent().is_some() => s.dirs.push(p),
            Ok(m) if m.is_file() => s.file_bytes += m.len(),
            // a drive or share root (its size is the free-space part), or unreadable
            _ => s.partial = true,
        }
    }
    s
}

pub(crate) unsafe fn selected_size(fv: &IFolderView2) -> Sel {
    let arr = fv.GetSelection(false).ok();
    let n = arr.as_ref().and_then(|a| a.GetCount().ok());
    collect_sel(n, |k| Some(pwstr(arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?)))
}

// The " (…)" after "n selected". dirs: the folder total so far (bytes, done, partial), None = not started.
pub(crate) fn size_text(ko: bool, file_bytes: u64, has_dirs: bool, dirs: Option<(u64, bool, bool)>, collect_partial: bool) -> String {
    let (b, done, partial) = match dirs {
        _ if !has_dirs => (0, true, false),
        Some(d) => d,
        None => (0, false, false),
    };
    let (bytes, partial) = (file_bytes + b, partial || collect_partial);
    if !done {
        return match bytes {
            0 => (if ko { " (계산 중…)" } else { " (calculating…)" }).into(),
            b => format!(" ({}…)", human(b)),
        };
    }
    match (bytes, partial) {
        (0, false) if !has_dirs => String::new(),
        (0, true) if !has_dirs => (if ko { " (크기 모름)" } else { " (size unknown)" }).into(),
        (b, true) => format!(" ({}, {})", human(b), if ko { "일부만" } else { "partial" }),
        (b, false) => format!(" ({})", human(b)),
    }
}

// Folder sizes are summed by one worker thread. gen changes on every new target or cancel;
// the worker drops results of an older gen and stops walking once DIR_GEN moves on.
#[derive(Default)]
pub(crate) struct SizeJob {
    pub(crate) gen: usize,
    pub(crate) target: Vec<String>,
    pub(crate) bytes: u64,
    pub(crate) done: bool,
    pub(crate) partial: bool,
}

impl SizeJob {
    // True = a new walk is due. busy (a copy runs) or no folders cancel; the same target keeps
    // running, or keeps its result.
    pub(crate) fn want(&mut self, dirs: &[String], busy: bool) -> bool {
        if busy || dirs.is_empty() {
            (self.gen, self.done) = (self.gen + 1, false);
            self.target.clear();
            return false;
        }
        if self.target == dirs {
            return false;
        }
        *self = SizeJob { gen: self.gen + 1, target: dirs.to_vec(), ..Default::default() };
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

pub(crate) static DIR_SIZE: Mutex<SizeJob> = Mutex::new(SizeJob { gen: 0, target: Vec::new(), bytes: 0, done: false, partial: false });
pub(crate) static DIR_WAKE: Condvar = Condvar::new();
pub(crate) static DIR_GEN: AtomicUsize = AtomicUsize::new(0); // copy of DIR_SIZE.gen, read without the lock

pub(crate) fn size_job() -> std::sync::MutexGuard<'static, SizeJob> {
    DIR_SIZE.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn want_size(dirs: &[String], busy: bool) {
    let mut j = size_job();
    if j.want(dirs, busy) {
        static WORKER: Once = Once::new();
        WORKER.call_once(|| _ = std::thread::Builder::new().spawn(size_worker));
        DIR_WAKE.notify_one();
    }
    DIR_GEN.store(j.gen, Ordering::Relaxed);
}

// Lives until the process ends. The lock is never held across file system calls.
pub(crate) fn size_worker() {
    let mut seen = 0;
    loop {
        let (gen, target) = {
            let mut j = size_job();
            while j.target.is_empty() || j.gen == seen {
                j = DIR_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
            (j.gen, j.target.clone())
        };
        seen = gen;
        let stop = || DIR_GEN.load(Ordering::Relaxed) != gen;
        // a copy only pauses the walk; the status line cancels it when it sees the copy
        let paused = || COPYING.load(Ordering::Relaxed) > 0;
        let (mut total, mut partial) = (0, false);
        let finished = target.iter().all(|root| {
            let (base, was) = (total, partial);
            let mut progress = |b, p| _ = size_job().publish(gen, base + b, false, was || p);
            dir_bytes(Path::new(root), &stop, &paused, &mut progress).map(|(b, p)| (total, partial) = (total + b, partial || p)).is_some()
        });
        if finished {
            size_job().publish(gen, total, true, partial);
        }
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

// F5 in a file list: the shell reloads it, and selected folders are summed again.
pub(crate) fn resum_sizes() {
    want_size(&[], true);
    app(|a| a.status_key.clear());
}

// Polled by a 300ms timer; the expensive parts only run when the active pane's state changed.
// Other ticks only merge the folder size worker's progress into the stored text.
pub(crate) unsafe fn update_status() {
    mark_drive();
    let (i, path, main) = app(|a| (a.s.active, a.s.paths[a.s.active].clone(), a.main));
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let sel = fv.ItemCount(SVGIO_SELECTION).unwrap_or(0);
    let spec = filter_of(i);
    // while a background copy runs, the disk is left alone (size and free space come back after)
    let busy = COPYING.load(Ordering::Relaxed) > 0;
    let zoom = app(|a| a.zoom);
    let rev = SEL_REV[i].load(Ordering::Relaxed);
    let key = format!("{i}|{path}|{n}|{sel}|{}|{spec}|{busy}|{zoom:?}|{rev}", fv.GetFocusedItem().unwrap_or(-1));
    if app(|a| a.status_key != key) {
        status_base(i, &path, main, &fv, n, sel, &spec, busy, zoom);
        app(|a| a.status_key = key);
    }
    let job = {
        let j = size_job();
        (!j.target.is_empty()).then_some((j.bytes, j.done, j.partial))
    };
    let ko = KO.load(Ordering::Relaxed);
    let Some(rc) = app(|a| {
        let (head, size, mid, drive) = &a.status_base;
        let size = size.map(|(f, d, p)| size_text(ko, f, d, job, p)).unwrap_or_default();
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
        (a.status != line).then(|| {
            a.status = line;
            status_rect(&geo(a))
        })
    }) else {
        return;
    };
    _ = InvalidateRect(Some(main), Some(&rc), true);
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

// The status text around the selection size, the window title, and a new folder size job.
pub(crate) unsafe fn status_base(i: usize, path: &str, main: HWND, fv: &IFolderView2, n: i32, sel: i32, spec: &str, busy: bool, zoom: Option<usize>) {
    let title = window_title(&app(|a| a.panes[i].as_ref().map(|p| p.disp.clone()).unwrap_or_default()));
    if text_of(main) != title {
        _ = SetWindowTextW(main, &HSTRING::from(title));
    }
    let ko = KO.load(Ordering::Relaxed);
    let mut parts = vec![if ko { format!("항목 {n}개") } else { format!("{n} items") }];
    let mut size = None;
    let mut dirs = Vec::new();
    if sel > 0 {
        if !busy {
            let s = selected_size(fv);
            size = Some((s.file_bytes, !s.dirs.is_empty(), s.partial));
            dirs = s.dirs;
        }
        parts.push(if ko { format!("{sel}개 선택") } else { format!("{sel} selected") });
    }
    want_size(&dirs, busy);
    let head = parts.join(ZONE_DOT);
    let mut parts = Vec::new();
    let mut disk = None;
    if !spec.is_empty() {
        parts.push(format!("{} {spec}", tr("Filter", "필터")));
    }
    let drive = path.get(1..2) == Some(":");
    if busy {
        parts.push(tr("Copying / moving…", "복사·이동 중…").into());
    } else if drive || path.starts_with(r"\\") {
        // ponytail: a dead network share can stall this call once per change
        let (mut free, mut total) = (0, 0);
        if GetDiskFreeSpaceExW(&HSTRING::from(path),Some(&mut free), Some(&mut total), None).is_ok() {
            let label = if drive { &path[..2] } else { tr("Network", "네트워크") };
            disk = Some((label.to_string(), free, total));
        }
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
}
