use crate::*;
use windows::Win32::System::Threading::GetCurrentThreadId;

// ---------- sessions ----------

// grids.ini text for the current state. Views must be saved first (save_view).
unsafe fn ini_now() -> String {
    let main = app(|a| a.main);
    if !IsIconic(main).as_bool() && !app(|a| a.was_max) {
        note_arranged(main); // also after a move that sent no WM_SIZE (Win+Shift+arrow to a same-size screen)
    }
    let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    _ = GetWindowPlacement(main, &mut wp);
    let sig = monitor_sig();
    app(|a| {
        // the restore rectangle, also while minimized; snapped to a half or quarter, the rect it showed (as Chrome does)
        let r = if a.was_max { wp.rcNormalPosition } else { a.arranged.unwrap_or(wp.rcNormalPosition) };
        let pos = [r.left, r.top, r.right, r.bottom, a.was_max as i32];
        a.s.win = Some(pos);
        // only the setup connected now is noted; no monitor list (a failed call): win_at= stays as it is
        if let Some(sig) = sig {
            let now = Instant::now();
            if a.place_sig != sig {
                // a new setup while running: where the window is now is Windows' doing
                // (place_sig starts as the setup Grids opened on, so the first saves after start are not held)
                (a.place_sig, a.place_settle) = (sig.clone(), Some(now + Duration::from_secs(5)));
            }
            if a.place_settle.is_some_and(|t| now < t) {
                a.place_hold = Some(pos);
            }
            a.place_hold = remember_place(&mut a.s.places, &sig, pos, a.place_hold);
        }
        // not shown since a sign-in start: folders still to retry; and panes moved to This PC off a pulled drive
        // keep its folder, so starting with the drive plugged in opens it again
        let mut keep = a.late.clone();
        keep.extend(a.panes.iter().enumerate().filter_map(|(i, p)| Some((i, p.as_ref()?.unplugged.as_ref()?.0.clone()))));
        if keep.is_empty() {
            a.s.to_ini()
        } else {
            keep_late(&a.s, &keep).to_ini()
        }
    })
}

/// The monitor setup connected now (mon_sig); None when Windows lists no monitor.
pub(crate) unsafe fn monitor_sig() -> Option<String> {
    unsafe extern "system" fn each(_: HMONITOR, _: HDC, r: *mut RECT, lp: LPARAM) -> BOOL {
        if let Some(r) = r.as_ref() {
            (*(lp.0 as *mut Vec<[i32; 4]>)).push([r.left, r.top, r.right, r.bottom]);
        }
        TRUE
    }
    let mut rects: Vec<[i32; 4]> = Vec::new();
    let ok = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut rects as *mut _ as isize)).as_bool();
    (ok && !rects.is_empty()).then(|| mon_sig(&rects))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner()) // a panicked writer must not stop the last save
}

// grids.ini written off the window's thread (as VS Code and browsers keep settings): next to the exe it may be
// on a USB stick, a network share or a disk a virus scanner holds, and a write there froze the window on every
// folder change. Autosave hands in the newest text and goes on; one worker writes, skipping texts already
// overtaken. Closing and Windows ending still write right there (write_now).
pub(crate) struct IniWriter {
    path: PathBuf,
    q: Mutex<IniQ>,
    cv: Condvar,
    io: Mutex<()>, // held for every write (worker and last save): one grids.ini.tmp at a time, no older text last
}

#[derive(Default)]
struct IniQ {
    seq: u64,                    // number of the newest text handed in (or written by write_now)
    next: Option<(u64, String)>, // waiting for the worker; a newer text replaces it unwritten
    written: u64,                // newest number written or given up on: older ones are skipped
    done: Option<(u64, bool)>,   // the worker's last finished write (number, worked), for autosave
    on_disk: Option<String>,     // text of the last write that worked
    started: bool,               // the worker thread runs
    closed: bool,                // Grids closes / Windows ends: nothing more is handed in
}

impl IniWriter {
    pub(crate) fn new(path: PathBuf) -> Self {
        IniWriter { path, q: Mutex::new(IniQ::default()), cv: Condvar::new(), io: Mutex::new(()) }
    }

    // Hand in `text` (replacing one still waiting); its number, or None once closed.
    pub(crate) fn push(&self, text: String) -> Option<u64> {
        let mut q = lock(&self.q);
        if q.closed {
            return None;
        }
        q.seq += 1;
        q.next = Some((q.seq, text));
        self.cv.notify_one();
        Some(q.seq)
    }

    // The worker's last finished write, once.
    pub(crate) fn take_done(&self) -> Option<(u64, bool)> {
        lock(&self.q).done.take()
    }

    fn wait_job(&self) -> (u64, String) {
        let mut q = lock(&self.q);
        loop {
            if let Some(job) = q.next.take() {
                return job;
            }
            q = self.cv.wait(q).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn write_job(&self, n: u64, text: String) {
        let _io = lock(&self.io);
        if lock(&self.q).written >= n {
            return; // the last save wrote a newer text meanwhile
        }
        let ok = write_ini(&self.path, &text).is_ok();
        let mut q = lock(&self.q);
        (q.written, q.done) = (n, Some((n, ok)));
        if ok {
            q.on_disk = Some(text);
        }
    }

    // The worker thread, started with the first text; false when Windows gives no thread.
    fn start(&'static self) -> bool {
        let mut q = lock(&self.q);
        if !q.started {
            let spawned = std::thread::Builder::new().name("grids-ini".into()).spawn(move || loop {
                let (n, text) = self.wait_job();
                self.write_job(n, text);
            });
            q.started = spawned.is_ok();
        }
        q.started
    }

    // The last save: waits for a write under way, drops what waits and writes `text` here. Unless the worker
    // write it waited for just put this very text there: a stuck share is not waited for twice (otherwise it
    // always writes, as before: grids.ini may have been removed or changed outside meanwhile).
    // `close`: no more pushes once it is written; after a failure autosave still tries again in 5 seconds.
    pub(crate) fn write_now(&self, text: &str, close: bool) -> std::io::Result<()> {
        let before = lock(&self.q).written;
        let _io = lock(&self.io);
        let mut q = lock(&self.q);
        let just = q.written > before && q.done == Some((q.written, true)) && q.on_disk.as_deref() == Some(text);
        q.seq += 1;
        (q.next, q.written, q.done) = (None, q.seq, None);
        if just {
            q.closed |= close;
            return Ok(());
        }
        drop(q);
        let r = write_ini(&self.path, text);
        if r.is_ok() {
            let mut q = lock(&self.q);
            q.on_disk = Some(text.to_owned());
            q.closed |= close;
        }
        r
    }
}

fn ini_writer() -> &'static IniWriter {
    static W: OnceLock<IniWriter> = OnceLock::new();
    W.get_or_init(|| IniWriter::new(ini_path()))
}

// What a write did: kept, or on failure the old grids.ini stays and autosave tries again in 5 seconds.
unsafe fn ini_result(ini: Option<String>, ok: bool) {
    app(|a| {
        if let Some(ini) = ini {
            a.queued_ini = ini;
        }
        note_write(&mut a.queued_ini, &mut a.saved_ini, ok);
        a.save_after = (!ok).then(|| Instant::now() + Duration::from_secs(5));
    });
}

// Written: that text is saved. Failed: forgotten as handed in, so autosave hands it in again after its 5 seconds.
fn note_write(queued: &mut String, saved: &mut String, ok: bool) {
    if ok {
        saved.clone_from(queued);
    } else {
        queued.clear();
    }
}

// Closing / Windows ending: written here, before the process goes, also while the worker is busy or stuck.
// Autosave hands in nothing more after it (after WM_ENDSESSION the window and its timer still live).
pub(crate) unsafe fn save_session() {
    for i in 0..app(|a| a.s.panes()) {
        save_view(i);
    }
    let ini = ini_now();
    let ok = ini_writer().write_now(&ini, true).is_ok();
    ini_result(Some(ini), ok);
}

// Runs right after check_dirty (which saved the views): writes grids.ini whenever its text changes, so a restart,
// sign-out or crash loses nothing. Not mid-drag or mid-resize.
pub(crate) unsafe fn autosave() {
    let w = ini_writer();
    if let Some((n, ok)) = w.take_done() {
        if app(|a| a.ini_seq == n) {
            ini_result(None, ok); // an older text's result: the newer one reports itself
        }
    }
    let now = Instant::now();
    if app(|a| a.drag.is_some() || a.sizing || a.save_after.is_some_and(|t| t > now)) {
        return;
    }
    let ini = ini_now();
    if app(|a| a.queued_ini != ini) {
        if w.start() {
            if let Some(n) = w.push(ini.clone()) {
                app(|a| (a.queued_ini, a.ini_seq) = (ini, n));
            }
        } else {
            let ok = w.write_now(&ini, false).is_ok(); // no thread: as before, here
            ini_result(Some(ini), ok);
        }
    }
    refresh_jump_list(false); // favourites, recent folders or the language changed: the jump list follows
}

// Save button / Ctrl+S: overwrite the current session file; the first time, ask for a name.
pub(crate) unsafe fn save_current() {
    let file = app(|a| a.s.file.clone());
    if file.is_empty() {
        _ = save_session_as();
    } else {
        write_session(Path::new(&file));
    }
}

pub(crate) unsafe fn write_session(path: &Path) {
    for i in 0..app(|a| a.s.panes()) {
        save_view(i);
    }
    if std::fs::write(path, app(|a| a.s.snapshot())).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    app(|a| {
        a.s.file = path.to_string_lossy().into_owned();
        a.s.order.clear(); // the file now has the panes where they are
        (a.base, a.dirty) = (Some(a.s.clone()), false);
    });
    refresh_save_button();
    refresh_jump_list(true);
}

// "New name": the save dialog does naming, overwrite prompts, rename and delete for us.
pub(crate) unsafe fn save_session_as() -> Result<()> {
    let main = app(|a| a.main);
    let dir = sessions_dir();
    _ = std::fs::create_dir_all(&dir);
    let dlg: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
    let kind = HSTRING::from(tr("Grids session (*.grids)", "Grids 세션 (*.grids)"));
    dlg.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: PCWSTR(kind.as_ptr()), pszSpec: w!("*.grids") }])?;
    dlg.SetDefaultExtension(w!("grids"))?;
    dlg.SetTitle(&HSTRING::from(tr("Save session", "세션 저장")))?;
    if let Ok(f) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(dir.to_string_lossy().as_ref()), None) {
        dlg.SetFolder(&f)?;
    }
    dlg.Show(Some(main))?; // Err on cancel
    let path = pwstr(dlg.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?);
    write_session(Path::new(&path));
    Ok(())
}

/// The status-line note for a session file that can't be read (a desktop shortcut, the jump list or the command
/// line naming it). `missing`: it isn't there at all (renamed or deleted since the link was made), else locked,
/// no access and the like.
pub(crate) fn session_unread_note(ko: bool, path: &Path, missing: bool) -> String {
    let (name, (en, kr)) = if missing {
        (path.file_stem(), ("Session \"{name}\" not found — it was renamed or deleted", "찾을 수 없는 세션: \"{name}\" — 이름이 바뀌었거나 지워짐"))
    } else {
        (path.file_name(), ("Couldn't open session file: {name}", "세션 파일을 열지 못했습니다: {name}"))
    };
    let name = name.map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy());
    (if ko { kr } else { en }).replace("{name}", &name)
}

pub(crate) fn not_found(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::NotFound
}

pub(crate) unsafe fn load_session_file(path: &Path) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            _ = MessageBeep(MB_ICONWARNING);
            let note = session_unread_note(KO.load(Ordering::Relaxed), path, not_found(&e));
            app(|a| a.note = Some((note, Instant::now() + Duration::from_secs(5))));
            update_status();
            return;
        }
    };
    for i in 0..MAXP {
        destroy_pane(i);
    }
    let other = Session::parse(&text);
    app(|a| {
        a.s.take_layout(&other);
        (a.zoom, a.home, a.focus_req) = (None, [None; MAXP], HWND::default());
        a.closed_tabs.clear(); // the last session's closed tabs don't come back into this one
        a.s.file = path.to_string_lossy().into_owned();
        (a.base, a.dirty) = (Some(a.s.clone()), false);
    });
    refresh_save_button();
    relayout();
    redraw_all();
    focus_view(app(|a| a.s.active));
}

// Posted to main by WM_COPYDATA: open what a second instance handed over (open_queued), unless something is up.
pub(crate) const WM_OPEN_ARGS: u32 = WM_APP + 65;

/// A second instance's hand-over waits (stays in App::open_queue) while main is disabled under a modal dialog
/// (save, delete confirm, colour, About), a menu's loop runs, or something is being dragged or sized: opening a
/// session then destroys every pane, the one that put the dialog, menu or drag up included.
pub(crate) fn defer_args(main_enabled: bool, menu_mode: bool, busy: bool) -> bool {
    !main_enabled || menu_mode || busy
}

/// WM_COPYDATA's text as one queue entry: its arguments in order, never merged with an earlier hand-over.
pub(crate) fn queue_args(q: &mut Vec<Vec<String>>, text: &str) {
    q.push(text.split('\n').map(String::from).collect());
}

// Open the waiting hand-overs one at a time, checking again before each. `posted`: from WM_OPEN_ARGS, which
// may hold them back (the dialog comes forward, the status line says why); the status tick only retries.
pub(crate) unsafe fn open_queued(posted: bool) {
    let main = app(|a| a.main);
    let mut front = app(|a| std::mem::take(&mut a.open_front));
    let wait = tr("Opens once the dialog or menu is closed", "대화 상자나 메뉴를 닫으면 엽니다");
    while app(|a| !a.open_queue.is_empty()) {
        let mut gti = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        let got = GetGUIThreadInfo(GetCurrentThreadId(), &mut gti).is_ok();
        let menu = got && (gti.flags & (GUI_INMENUMODE | GUI_POPUPMENUMODE | GUI_SYSTEMMENUMODE)).0 != 0;
        let held = got && (!gti.hwndCapture.is_invalid() || (gti.flags & GUI_INMOVESIZE).0 != 0);
        let busy = held || app(|a| a.drag.is_some() || a.nudge.is_some() || a.sizing);
        let enabled = IsWindowEnabled(main).as_bool();
        if defer_args(enabled, menu, busy) {
            if posted {
                // the sender puts main in front after its answer, which may undo this: the next tick tries again
                front = !enabled;
                // a drag or resize ends on its own: no note then, only for a dialog or a menu
                if !enabled || menu {
                    app(|a| a.note = Some((wait.to_string(), Instant::now() + Duration::from_secs(5))));
                    update_status();
                }
            }
            if front && !enabled {
                bring_front(main);
            }
            app(|a| a.open_front = posted && front);
            return;
        }
        let Some(args) = app(|a| (!a.open_queue.is_empty()).then(|| a.open_queue.remove(0))) else { break };
        app(|a| {
            if a.note.as_ref().is_some_and(|n| n.0 == wait) {
                a.note = None;
            }
        });
        open_args(args);
    }
}

// Paths from the command line or a second instance: a session file loads, folders fill panes from the active one on.
pub(crate) unsafe fn open_args(mut args: Vec<String>) {
    let folders = take_folder_flag(&mut args); // a jump-list folder link or Explorer's "Open in Grids"
    if let Some(f) = session_arg(&args).filter(|_| !folders) {
        app(|x| (x.late.clear(), x.menu_burst = None)); // another session's folders now
        return load_session_file(Path::new(f));
    }
    if app(|x| x.zoom.take()).is_some() {
        relayout();
        redraw_all();
    }
    let (a, n) = app(|x| (x.s.active, x.s.panes()));
    if !folders {
        app(|x| x.menu_burst = None);
        // panes given a folder here no longer wait for a sign-in retry
        app(|x| x.late.retain(|(i, _)| (0..args.len().min(n)).all(|k| (a + k) % n != *i)));
        for (k, p) in args.iter().take(n).enumerate() {
            go_near((a + k) % n, p);
        }
        return focus_view(a);
    }
    // Explorer starts Grids once per selected folder: a start right after the last one goes on in the next pane
    let prev = app(|x| x.menu_burst);
    let (start, take, next) = burst_plan(prev, Instant::now(), a, n, args.len());
    let placed: Vec<usize> = (0..take).map(|k| (start + k) % n).collect();
    // claimed before go_near: another start handed over while it pumps messages must see these panes as taken
    app(|x| (x.late.retain(|(i, _)| !placed.contains(i)), x.menu_burst = Some(next)));
    for (&i, p) in placed.iter().zip(&args) {
        go_near(i, p);
    }
    // after the folders are in: a slow one must not split the burst (only the time; the counts may have moved on)
    app(|x| {
        if let Some(b) = x.menu_burst.as_mut() {
            b.at = Instant::now();
        }
    });
    focus_view(next.first);
    if next.missed > 0 {
        if next.missed == args.len() - take {
            _ = MessageBeep(MB_ICONWARNING); // the burst's first miss only: one start per folder, one beep
        }
        let m = next.missed;
        set_note(if KO.load(Ordering::Relaxed) { format!("패널이 모자라 폴더 {m}개를 열지 못함") } else { format!("Not enough panes: {m} folder(s) not opened") });
    }
}

/// Folders handed over one start at a time by a multi-folder "Open in Grids" (runtime only).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Burst {
    pub(crate) at: Instant,     // when the last of them was placed
    pub(crate) first: usize,    // the pane the burst began in (it keeps the focus)
    pub(crate) placed: usize,   // panes filled so far, from `first` on
    pub(crate) missed: usize,   // folders left out: every pane already had one of the burst
}

pub(crate) const BURST_GAP: Duration = Duration::from_millis(1500);

// Where `count` folder-mode folders go: (first pane, how many, the burst after them; its `at` is `now`).
// Within BURST_GAP of the last, the burst goes on after its panes; otherwise a new one starts at the active pane.
// A burst never wraps onto its own panes: the rest count as missed. One that no longer fits the layout starts over.
pub(crate) fn burst_plan(prev: Option<Burst>, now: Instant, active: usize, n: usize, count: usize) -> (usize, usize, Burst) {
    let n = n.max(1);
    let go_on = prev.filter(|p| now.saturating_duration_since(p.at) <= BURST_GAP && p.first < n && p.placed <= n);
    let b = go_on.unwrap_or(Burst { at: now, first: active.min(n - 1), placed: 0, missed: 0 });
    let take = count.min(n - b.placed);
    let start = (b.first + b.placed) % n;
    (start, take, Burst { at: now, placed: b.placed + take, missed: b.missed + count - take, ..b })
}

// Explorer folders not shown yet, paired with the panes that don't hold one of them, from the active pane on.
// Each folder already open reserves one live pane showing it; open = (slot, folder) of the live panes.
// Returns (slot, folder) to open and how many new folders found no pane.
pub(crate) fn gather_plan(found: &[String], open: &[(usize, String)], active: usize, n: usize) -> (Vec<(usize, String)>, usize) {
    let key = |p: &str| p.trim_end_matches('\\').to_lowercase();
    let (mut seen, mut new, mut kept) = (Vec::new(), Vec::new(), vec![false; open.len()]);
    for f in found {
        let k = key(f);
        if seen.contains(&k) {
            continue;
        }
        match (0..open.len()).find(|&j| !kept[j] && key(&open[j].1) == k) {
            Some(j) => kept[j] = true,
            None => new.push(f.clone()),
        }
        seen.push(k);
    }
    let free: Vec<usize> = (0..n).map(|k| (active + k) % n).filter(|&s| (0..open.len()).any(|j| open[j].0 == s && !kept[j])).collect();
    let short = new.len().saturating_sub(free.len());
    (free.into_iter().zip(new).collect(), short)
}

// File system folders of the open Explorer windows; Windows 11 tabs are listed one by one.
// Home, This PC, Control Panel and windows that don't answer are skipped.
unsafe fn explorer_folders() -> Vec<String> {
    let Ok(wins) = CoCreateInstance::<_, IShellWindows>(&ShellWindows, None, CLSCTX_ALL) else { return Vec::new() };
    let folder = |d: IDispatch| -> Result<String> {
        let sb: IShellBrowser = d.cast::<IServiceProvider>()?.QueryService(&SID_STopLevelBrowser)?;
        let it = sb.QueryActiveShellView()?.cast::<IFolderView>()?.GetFolder::<IShellItem>()?;
        Ok(pwstr(it.GetDisplayName(SIGDN_FILESYSPATH)?))
    };
    (0..wins.Count().unwrap_or(0)).filter_map(|i| folder(wins.Item(&VARIANT::from(i)).ok()?).ok()).collect()
}

static GATHERING: AtomicBool = AtomicBool::new(false);

// Ctrl+Shift+O: folders open in Explorer come into the panes that don't already show one of them.
pub(crate) unsafe fn gather_explorer() {
    if GATHERING.swap(true, Ordering::Relaxed) {
        return;
    }
    let found = explorer_folders(); // calls into explorer.exe pump messages, so this stays outside app()
    let (active, n, open) = app(|a| {
        let n = a.s.panes();
        (a.s.active, n, (0..n).filter(|&i| a.panes[i].is_some()).map(|i| (i, a.s.paths[i].clone())).collect::<Vec<_>>())
    });
    let (plan, short) = gather_plan(&found, &open, active, n);
    let ko = KO.load(Ordering::Relaxed);
    let mut parts = Vec::new();
    let mut failed = 0;
    if !plan.is_empty() {
        if app(|x| x.zoom.take()).is_some() {
            relayout();
            redraw_all();
        }
        let ok: Vec<String> = plan.iter().filter(|(i, p)| navigate(*i, p)).map(|(i, _)| (i + 1).to_string()).collect();
        failed = plan.len() - ok.len();
        focus_view(active);
        if !ok.is_empty() {
            let (k, s) = (ok.len(), if ok.len() == 1 { "" } else { "s" });
            parts.push(match ko {
                true => format!("탐색기 폴더 {k}개 열기 요청 → 패널 {}", ok.join(", ")),
                false => format!("Opening {k} Explorer folder{s} in pane{s} {}", ok.join(", ")),
            });
        }
        if failed > 0 {
            parts.push(if ko { format!("{failed}개 실패") } else { format!("{failed} failed") });
        }
    }
    if short > 0 {
        parts.push(if ko { format!("패널이 모자라 {short}개 안 엶") } else { format!("{short} not opened (not enough panes)") });
    }
    if parts.is_empty() {
        parts.push(tr(
            if found.is_empty() { "No Explorer folders to bring in" } else { "Already open in panes" },
            if found.is_empty() { "가져올 탐색기 폴더가 없습니다" } else { "이미 모두 패널에 열려 있습니다" },
        ).into());
    }
    if plan.is_empty() || failed + short > 0 {
        _ = MessageBeep(MB_ICONWARNING);
    }
    app(|x| {
        x.note = Some((parts.join(" · "), Instant::now() + Duration::from_secs(3)));
        x.lost = 0;
    });
    update_status();
    GATHERING.store(false, Ordering::Relaxed);
}

// Window of a running copy of this same exe. Portable copies elsewhere have their own ini and stay separate.
pub(crate) unsafe fn running_instance() -> Option<HWND> {
    let me = std::env::current_exe().ok()?.to_string_lossy().to_lowercase();
    let mut h = None;
    loop {
        let w = FindWindowExW(None, h, w!("GridsMain"), PCWSTR::null()).ok()?;
        h = Some(w);
        let mut pid = 0;
        GetWindowThreadProcessId(w, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
        let (mut buf, mut n) = ([0u16; 1024], 1024u32);
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        _ = CloseHandle(p);
        if ok && String::from_utf16_lossy(&buf[..n as usize]).to_lowercase() == me {
            return Some(w);
        }
    }
}

// Start lock: two starts at once (several .grids files opened together, a double click on the taskbar)
// would both find no window and open two Grids that overwrite each other's grids.ini. One start at a time:
// the next one checks for a window only after this one has its panes and its message loop.
// Per exe path, like running_instance; FNV-1a of the lowercase path, as a name may hold no '\'.
pub(crate) fn start_lock_name(exe: &str) -> String {
    let h = exe.to_lowercase().bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    format!("Local\\Grids.Start.{h:016x}")
}

#[derive(Debug, PartialEq)]
pub(crate) enum StartTurn {
    Ours,   // got it (an owner that died without releasing it counts too)
    Busy,   // another start still holds it after the wait
    Failed, // the wait itself failed
}

// The wait's result as a turn.
pub(crate) fn start_turn(wait: WAIT_EVENT) -> StartTurn {
    match wait {
        WAIT_OBJECT_0 | WAIT_ABANDONED => StartTurn::Ours,
        WAIT_TIMEOUT => StartTurn::Busy,
        _ => StartTurn::Failed,
    }
}

pub(crate) struct StartLock(Option<HANDLE>);

impl StartLock {
    // Waits up to 10 s. None = another start still holds it (slow start, a dialog): the caller tells and quits,
    // as going on would open a second window. A lock that cannot be opened or waited on (a name an elevated
    // Grids owns) starts as before, unlocked.
    pub(crate) unsafe fn take() -> Option<StartLock> {
        let Ok(exe) = std::env::current_exe() else { return Some(StartLock(None)) };
        let name = HSTRING::from(start_lock_name(&exe.to_string_lossy()));
        let Ok(h) = CreateMutexW(None, false, &name) else { return Some(StartLock(None)) };
        let turn = start_turn(WaitForSingleObject(h, 10_000));
        if turn == StartTurn::Ours {
            return Some(StartLock(Some(h)));
        }
        _ = CloseHandle(h);
        (turn == StartTurn::Failed).then_some(StartLock(None))
    }

    // Lets the next start in; also on every early return, by drop. Same thread that took it.
    pub(crate) unsafe fn release(&mut self) {
        if let Some(h) = self.0.take() {
            _ = ReleaseMutex(h);
            _ = CloseHandle(h);
        }
    }
}

impl Drop for StartLock {
    fn drop(&mut self) {
        unsafe { self.release() }
    }
}

#[repr(C)]
pub(crate) struct CopyData {
    pub(crate) dw: usize,
    pub(crate) cb: u32,
    pub(crate) lp: *const core::ffi::c_void,
}


// Repaints the save icon and puts the session name / unsaved state into its tooltip.
pub(crate) unsafe fn refresh_save_button() {
    let (main, tip, file, dirty) = app(|a| (a.main, a.tip, a.s.file.clone(), a.dirty));
    let Ok(b) = GetDlgItem(Some(main), ID_TB_SAVE) else { return };
    _ = InvalidateRect(Some(b), None, false);
    let mut hint = match Path::new(&file).file_stem() {
        Some(n) if KO.load(Ordering::Relaxed) => format!("'{}'에 저장 (Ctrl+S)", n.to_string_lossy()),
        Some(n) => format!("Save to '{}' (Ctrl+S)", n.to_string_lossy()),
        None => tr("Save session (Ctrl+S)", "세션 저장 (Ctrl+S)").into(),
    };
    if dirty {
        hint += tr(" · unsaved layout changes", " · 저장 안 된 배치 변경");
    }
    a11y_desc(b, Some(&hint)); // read out after its name, as the other toolbar buttons' tooltips (tip_text)
    let hint = HSTRING::from(hint);
    let ti = TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND,
        hwnd: main,
        uId: b.0 as usize,
        lpszText: PWSTR(hint.as_ptr() as *mut u16),
        ..Default::default()
    };
    SendMessageW(tip, TTM_UPDATETIPTEXTW, None, Some(LPARAM(&ti as *const _ as isize)));
}

// Polled with the status timer: view modes are only observable by asking the views.
pub(crate) unsafe fn check_dirty() {
    for i in 0..app(|a| a.s.panes()) {
        save_view(i);
    }
    let changed = app(|a| {
        if let Some(b) = a.base.as_mut() {
            for i in 0..a.s.panes() {
                if b.cols[i].is_empty() {
                    b.cols[i] = a.s.cols[i].clone();
                }
            }
        }
        let d = session_dirty(&a.s, a.base.as_ref());
        std::mem::replace(&mut a.dirty, d) != d
    });
    if changed {
        refresh_save_button();
    }
}

pub(crate) const TICK_FAST: u32 = 300; // ms, TIMER_STATUS while the window is shown
pub(crate) const TICK_SLOW: u32 = 5000; // ms, while minimized (or hidden in the notification area)
const IDLE_EVERY: u32 = 7; // behind another app: the views and grids.ini every 7th tick (about 2 seconds)

/// What one TIMER_STATUS tick runs. `status`: ev_watch + update_status, `views`: check_dirty (every pane's
/// columns and view mode), `save`: autosave (grids.ini, jump list), `ms`: the interval from here on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TickPlan {
    pub(crate) status: bool,
    pub(crate) views: bool,
    pub(crate) save: bool,
    pub(crate) ms: u32,
}

/// Like a browser slowing a background tab's timers. `iconic`: minimized (wins over `active`, which a taskbar
/// restore reports while still minimized); `active`: one of Grids' own windows is in front; `slow`: the timer runs
/// at TICK_SLOW now. In front: everything, every tick. Shown behind another app: the status line every tick, the
/// views and grids.ini every IDLE_EVERY-th (a Ctrl+wheel over the inactive window still counts). Minimized: nothing
/// on screen can change, so only autosave (a folder renamed elsewhere, a failed write's retry), every 5 seconds;
/// the first minimized tick still reads the views, for a column changed just before minimizing.
pub(crate) fn tick_plan(iconic: bool, active: bool, tick_no: u32, slow: bool) -> TickPlan {
    if iconic {
        return TickPlan { status: false, views: !slow, save: true, ms: TICK_SLOW };
    }
    let all = active || tick_no % IDLE_EVERY == 0;
    TickPlan { status: true, views: all, save: all, ms: TICK_FAST }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_bursts_fill_panes_in_turn() {
        let t0 = Instant::now();
        let ms = |m| t0 + Duration::from_millis(m);
        // three folders 0.4 s apart from pane 2 of 4: 2, 3, then 0
        let (s, k, b) = burst_plan(None, t0, 2, 4, 1);
        assert_eq!((s, k, b.first, b.placed, b.missed), (2, 1, 2, 1, 0));
        let (s, k, b) = burst_plan(Some(b), ms(400), 1, 4, 1);
        assert_eq!((s, k, b.placed), (3, 1, 2));
        let (s, k, b) = burst_plan(Some(b), ms(800), 3, 4, 1);
        assert_eq!((s, k, b.first, b.placed), (0, 1, 2, 3));
        // more than 1.5 s after the last: a new burst at the active pane
        let (s, k, b2) = burst_plan(Some(b), ms(800 + 1501), 1, 4, 1);
        assert_eq!((s, k, b2.first, b2.placed, b2.missed), (1, 1, 1, 1, 0));
        let (s, _, b3) = burst_plan(Some(b), ms(800 + 1500), 0, 4, 1);
        assert_eq!((s, b3.first, b3.placed), (1, 2, 4)); // still the burst: pane 2 + 3 placed, wrapped
    }

    #[test]
    fn menu_burst_never_covers_its_own_panes() {
        let t0 = Instant::now();
        let mut b = None;
        let mut got = Vec::new();
        for i in 0..6 {
            let (s, k, nb) = burst_plan(b, t0 + Duration::from_millis(100 * i), 0, 4, 1);
            got.push((s, k));
            b = Some(nb);
        }
        assert_eq!(got, [(0, 1), (1, 1), (2, 1), (3, 1), (0, 0), (0, 0)]);
        assert_eq!((b.unwrap().placed, b.unwrap().missed), (4, 2));
        // several at once (a jump list link with more folders) count the same way
        let (s, k, b) = burst_plan(None, t0, 1, 3, 5);
        assert_eq!((s, k, b.placed, b.missed), (1, 3, 3, 2));
    }

    #[test]
    fn menu_burst_from_the_first_start_and_edges() {
        let t0 = Instant::now();
        // the first start put its folder in pane 1 (index 0); the next goes to pane 2
        let first = Burst { at: t0, first: 0, placed: 1, missed: 0 };
        let (s, k, _) = burst_plan(Some(first), t0 + Duration::from_millis(300), 0, 4, 1);
        assert_eq!((s, k), (1, 1));
        // one pane: the first opens, the rest are missed
        let (s, k, b) = burst_plan(None, t0, 0, 1, 1);
        assert_eq!((s, k), (0, 1));
        let (s, k, b) = burst_plan(Some(b), t0, 0, 1, 1);
        assert_eq!((s, k, b.missed), (0, 0, 1));
        // the layout shrank under the burst: start over at the active pane
        let wide = Burst { at: t0, first: 5, placed: 2, missed: 0 };
        let (s, k, b) = burst_plan(Some(wide), t0, 1, 4, 1);
        assert_eq!((s, k, b.first, b.placed), (1, 1, 1, 1));
        let full = Burst { at: t0, first: 0, placed: 6, missed: 0 };
        assert_eq!(burst_plan(Some(full), t0, 2, 4, 1).0, 2);
    }

    fn ini_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("grids-iniw-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn unread_session_notes() {
        let p = Path::new(r"C:\Grids\sessions\업무.grids");
        assert_eq!(session_unread_note(false, p, true), "Session \"업무\" not found — it was renamed or deleted");
        assert_eq!(session_unread_note(true, p, true), "찾을 수 없는 세션: \"업무\" — 이름이 바뀌었거나 지워짐");
        assert_eq!(session_unread_note(false, p, false), "Couldn't open session file: 업무.grids");
        assert_eq!(session_unread_note(true, p, false), "세션 파일을 열지 못했습니다: 업무.grids");
        let gone = std::fs::read_to_string(std::env::temp_dir().join("grids-no-such-session.grids")).unwrap_err();
        assert!(not_found(&gone));
    }

    #[test]
    fn only_the_newest_waiting_text_is_written() {
        let dir = ini_dir("newest");
        let w = IniWriter::new(dir.join("grids.ini"));
        let ns: Vec<u64> = ["a=1\n", "a=2\n", "a=3\n"].iter().map(|t| w.push(t.to_string()).unwrap()).collect();
        assert!(ns.windows(2).all(|p| p[0] < p[1])); // numbers only go up
        let (n, text) = w.wait_job();
        assert_eq!((n, text.as_str()), (ns[2], "a=3\n"));
        assert!(lock(&w.q).next.is_none()); // the two older ones are gone, never written
        w.write_job(n, text);
        assert_eq!(w.take_done(), Some((n, true)));
        assert_eq!(w.take_done(), None);
        assert_eq!(std::fs::read_to_string(dir.join("grids.ini")).unwrap(), "a=3\n");
        assert!(w.push("a=4\n".into()).unwrap() > n);
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_save_drops_waiting_and_beats_an_older_write() {
        let dir = ini_dir("last");
        let ini = dir.join("grids.ini");
        let w = IniWriter::new(ini.clone());
        let n = w.push("old=1\n".into()).unwrap();
        let job = w.wait_job(); // the worker took it but has not written yet
        w.push("mid=1\n".into()).unwrap(); // still waiting when Grids closes
        w.write_now("last=1\n", true).unwrap();
        let (gone, seq) = {
            let q = lock(&w.q);
            (q.next.is_none(), q.seq)
        };
        assert!(gone && seq > n);
        w.write_job(job.0, job.1); // finishes late: skipped, the closing text stays
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "last=1\n");
        assert_eq!(w.take_done(), None);
        assert_eq!(w.push("after=1\n".into()), None); // closed: nothing more handed in
        // the same text again with no worker write waited for: written anyway (grids.ini was removed outside)
        std::fs::remove_file(&ini).unwrap();
        w.write_now("last=1\n", true).unwrap();
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "last=1\n");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_is_handed_in_again() {
        let dir = ini_dir("fail");
        let ini = dir.join("grids.ini");
        std::fs::create_dir(&ini).unwrap(); // cannot be replaced
        let w = IniWriter::new(ini.clone());
        let n = w.push("a=1\n".into()).unwrap();
        let (m, text) = w.wait_job();
        w.write_job(m, text);
        assert_eq!(w.take_done(), Some((n, false)));
        assert!(w.write_now("a=1\n", true).is_err());
        assert!(w.push("a=1\n".into()).is_some()); // a failed last save does not close: autosave tries again
        let (mut queued, mut saved) = ("a=1\n".to_string(), "a=0\n".to_string());
        note_write(&mut queued, &mut saved, false);
        assert!(queued != "a=1\n" && saved == "a=0\n"); // autosave's `queued_ini != ini` hands it in again
        queued = "a=1\n".into();
        note_write(&mut queued, &mut saved, true);
        assert_eq!((queued.as_str(), saved.as_str()), ("a=1\n", "a=1\n"));
        assert!(!dir.join("grids.ini.tmp").exists());
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hundred_quick_texts_then_close_leave_the_last() {
        let dir = ini_dir("hundred");
        let ini = dir.join("grids.ini");
        let w: &'static IniWriter = Box::leak(Box::new(IniWriter::new(ini.clone())));
        assert!(w.start() && w.start()); // one worker, started once
        for k in 0..100 {
            w.push(format!("k={k}\n")).unwrap();
        }
        w.write_now("k=final\n", true).unwrap();
        std::thread::sleep(Duration::from_millis(50)); // a late worker write would land here
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "k=final\n");
        assert!(!dir.join("grids.ini.tmp").exists());
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn in_front_every_tick_does_everything() {
        for n in 0..20 {
            assert_eq!(tick_plan(false, true, n, false), TickPlan { status: true, views: true, save: true, ms: TICK_FAST });
        }
    }

    #[test]
    fn behind_another_app_views_every_seventh_tick() {
        let full: Vec<u32> = (1..=21).filter(|&n| tick_plan(false, false, n, false).views).collect();
        assert_eq!(full, [7, 14, 21]);
        for n in 1..=21 {
            let p = tick_plan(false, false, n, false);
            assert!(p.status && p.save == p.views && p.ms == TICK_FAST);
        }
    }

    #[test]
    fn minimized_saves_only_and_slows_down() {
        // first tick after minimizing (timer still fast): the views once more, then grids.ini only
        assert_eq!(tick_plan(true, false, 3, false), TickPlan { status: false, views: true, save: true, ms: TICK_SLOW });
        assert_eq!(tick_plan(true, false, 4, true), TickPlan { status: false, views: false, save: true, ms: TICK_SLOW });
        // a taskbar restore activates Grids before the window leaves the minimized state
        assert_eq!(tick_plan(true, true, 5, true).ms, TICK_SLOW);
    }

    #[test]
    fn shown_again_is_fast() {
        assert_eq!(tick_plan(false, true, 9, true), TickPlan { status: true, views: true, save: true, ms: TICK_FAST });
        assert_eq!(tick_plan(false, false, 9, true).ms, TICK_FAST);
    }
}
