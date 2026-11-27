use crate::*;
use std::collections::BTreeMap;
use windows::Win32::System::Threading::GetCurrentThreadId;

// ---------- folder usage: Ctrl+Shift+U, or the ⇄ menu ----------
// What takes up the space in a folder (ncdu, WinDirStat, TreeSize): its items biggest first, with a bar and a share
// of the whole, filled in while one worker sums the subfolders (dir_bytes, as the status line's folder size). → goes
// into a folder, ← back up, Enter selects the item in the pane. Nothing is changed on disk from here.

pub(crate) const ID_USAGE: usize = 1139; // ⇄ menu
pub(crate) const WM_USAGE: u32 = WM_APP + 57; // to main from the worker: wParam = gen; the rows stay in JOB
const USAGE_MAX: usize = 2000; // rows listed; the smallest of the rest go into one "n more" row
const USAGE_EVERY: Duration = Duration::from_millis(150); // the window is told at most this often while counting
const RH: i32 = 26; // row height at 96 dpi; 16 rows on screen, fewer on a short screen

// bytes: so far while a folder is being summed; partial: some part unreadable; more > 0: the n smallest items together
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct URow { name: String, folder: bool, bytes: u64, done: bool, partial: bool, more: usize }

// ---------- pure parts (tested) ----------

/// Biggest first, the same size by name (any case); past `max` rows the smallest go into one "n more" row.
pub(crate) fn usage_list(mut rows: Vec<URow>, max: usize) -> Vec<URow> {
    rows.sort_by_cached_key(|r| (std::cmp::Reverse(r.bytes), r.name.to_lowercase())); // one lower-case name per row
    if rows.len() > max {
        let rest = rows.split_off(max.saturating_sub(1));
        let (done, partial) = (rest.iter().all(|r| r.done), rest.iter().any(|r| r.partial));
        rows.push(URow { bytes: rest.iter().map(|r| r.bytes).sum(), done, partial, more: rest.len(), ..Default::default() });
    }
    rows
}

/// Where the row named `name` is now (the selection follows its item when sizes change the order).
pub(crate) fn usage_pick(rows: &[URow], name: &str) -> Option<usize> {
    rows.iter().position(|r| r.more == 0 && r.name == name)
}

/// The share of the whole: "42%", "<1%" for a sliver, "0%" for nothing (or an empty whole).
pub(crate) fn usage_pct(bytes: u64, total: u64) -> String {
    if total == 0 || bytes == 0 { return "0%".into() }
    let p = ((bytes as u128 * 100 + total as u128 / 2) / total as u128).min(100);
    if p == 0 { "<1%".into() } else { format!("{p}%") }
}

/// A bar's length out of `w` px: as long as the biggest row is, at least 1 px for anything over 0 bytes.
pub(crate) fn usage_bar(bytes: u64, max: u64, w: i32) -> i32 {
    if max == 0 || bytes == 0 || w <= 0 { return 0 }
    ((bytes.min(max) as u128 * w as u128 / max as u128) as i32).max(1)
}

/// The folder above: "C:\a" -> "C:\", "\\srv\share\a" -> "\\srv\share". None at a drive or share's root.
pub(crate) fn usage_parent(p: &str) -> Option<String> {
    let t = p.trim_end_matches('\\');
    let head = &t[..t.rfind('\\')?];
    if head.len() == 2 && head.ends_with(':') { return Some(format!("{head}\\")) }
    let share_root = t.starts_with(r"\\") && head.get(2..).is_none_or(|h| !h.contains('\\'));
    (!head.is_empty() && !share_root).then(|| head.to_string())
}

pub(crate) fn usage_child(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') { format!("{dir}{name}") } else { format!("{dir}\\{name}") }
}

/// The line over the rows: the total, how far the folders are, and whether some part couldn't be read.
pub(crate) fn usage_head(ko: bool, rows: &[URow], done: bool) -> String {
    let dirs = rows.iter().filter(|r| r.folder).count();
    let counted = rows.iter().filter(|r| r.folder && r.done).count();
    let mut t = human(rows.iter().map(|r| r.bytes).sum());
    t += &match (done, ko) {
        (true, true) => " · 다 계산함".to_string(),
        (true, false) => " · all counted".to_string(),
        (false, true) => format!(" · 하위 폴더 {counted}/{dirs} 계산됨"),
        (false, false) => format!(" · {counted} of {dirs} folders counted"),
    };
    if rows.iter().any(|r| r.partial) { t += if ko { " · 일부만" } else { " · partial" } }
    t
}

/// A folder's own items: files with their size, folders to be summed, links and junctions as 0 bytes (not
/// followed, as dir_bytes). None: the folder can't be read, or `stop`.
pub(crate) fn usage_entries(dir: &Path, stop: &dyn Fn() -> bool) -> Option<Vec<URow>> {
    let mut rows = Vec::new();
    for e in std::fs::read_dir(dir).ok()? {
        if stop() { return None }
        let Ok(e) = e else { continue };
        let (folder, bytes, done, partial) = match e.file_type() {
            Ok(t) if t.is_dir() => (true, 0, false, false),
            Ok(t) if t.is_file() => e.metadata().map_or((false, 0, true, true), |m| (false, m.len(), true, false)),
            Ok(_) => (false, 0, true, false),
            Err(_) => (false, 0, true, true),
        };
        rows.push(URow { name: e.file_name().to_string_lossy().into_owned(), folder, bytes, done, partial, more: 0 });
    }
    Some(rows)
}

/// Sums the folder rows not done yet, one after another; `update(k, bytes, done, partial)` as each goes.
/// False: `stop` turned true.
pub(crate) fn usage_fill(dir: &Path, rows: &mut [URow], stop: &dyn Fn() -> bool, paused: &dyn Fn() -> bool, update: &mut dyn FnMut(usize, u64, bool, bool)) -> bool {
    for k in 0..rows.len() {
        if !rows[k].folder || rows[k].done { continue }
        let mut progress = |b, p| update(k, b, false, p);
        let Some((b, p)) = dir_bytes(&dir.join(&rows[k].name), stop, paused, &mut progress) else { return false };
        (rows[k].bytes, rows[k].done, rows[k].partial) = (b, true, p);
        update(k, b, true, p);
    }
    true
}

// ---------- the worker ----------

// known: folders summed while the window is open, path (lower case) -> (bytes, partial); told: the last WM_USAGE sent
struct UsageJob { gen: usize, dir: String, main: usize, rows: Vec<URow>, done: bool, failed: bool, known: BTreeMap<String, (u64, bool)>, told: Option<Instant> }

static JOB: Mutex<UsageJob> = Mutex::new(UsageJob { gen: 0, dir: String::new(), main: 0, rows: Vec::new(), done: false, failed: false, known: BTreeMap::new(), told: None });
static WAKE: Condvar = Condvar::new();
static GEN: AtomicUsize = AtomicUsize::new(0); // JOB.gen, read without the lock
static POSTED: AtomicBool = AtomicBool::new(false); // a WM_USAGE is on its way: no need for another

fn job() -> std::sync::MutexGuard<'static, UsageJob> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

// A new folder for the worker (None: stop, and forget the sizes). Returns its gen.
fn usage_start(dir: Option<&str>, main: HWND) -> usize {
    let mut j = job();
    j.gen += 1;
    (j.dir, j.main, j.done, j.failed) = (dir.unwrap_or_default().to_string(), main.0 as usize, false, false);
    j.rows.clear();
    GEN.store(j.gen, Ordering::Relaxed);
    if dir.is_none() {
        j.known.clear();
    } else {
        static WORKER: Once = Once::new();
        WORKER.call_once(|| _ = std::thread::Builder::new().spawn(usage_worker));
        WAKE.notify_one();
    }
    j.gen
}

// Changes the job if it is still `gen`'s, then tells the main window: once until it has looked, and while counting
// (not `now`) at most every USAGE_EVERY, so thousands of small folders don't flood it (input and painting go first).
fn publish(gen: usize, now: bool, f: impl FnOnce(&mut UsageJob)) {
    let main = {
        let mut j = job();
        if j.gen != gen { return }
        f(&mut j);
        if !(now || j.told.is_none_or(|t| t.elapsed() >= USAGE_EVERY)) || POSTED.swap(true, Ordering::Relaxed) { return }
        j.told = Some(Instant::now());
        j.main
    };
    if unsafe { PostMessageW(Some(HWND(main as _)), WM_USAGE, WPARAM(gen), LPARAM(0)) }.is_err() {
        POSTED.store(false, Ordering::Relaxed); // not on its way after all: the next change tries again
    }
}

// Lives until the process ends. The lock is never held across file system calls.
fn usage_worker() {
    let mut seen = 0;
    loop {
        let (gen, dir) = {
            let mut j = job();
            while j.dir.is_empty() || j.gen == seen {
                j = WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
            (j.gen, j.dir.clone())
        };
        seen = gen;
        let stop = || GEN.load(Ordering::Relaxed) != gen;
        let Some(mut rows) = usage_entries(Path::new(&dir), &stop) else {
            publish(gen, true, |j| (j.done, j.failed) = (true, true));
            continue;
        };
        let key = |name: &str| usage_child(&dir, name).to_lowercase();
        let known = job().known.clone();
        for r in rows.iter_mut().filter(|r| r.folder) {
            if let Some(&(b, p)) = known.get(&key(&r.name)) {
                (r.bytes, r.done, r.partial) = (b, true, p);
            }
        }
        publish(gen, true, |j| j.rows = rows.clone());
        // a copy only pauses the walk (dir_bytes), as for the status line's folder size
        let paused = || COPYING.load(Ordering::Relaxed) > 0;
        let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
        let mut update = |k: usize, b, done, p| publish(gen, false, |j| {
            if let Some(r) = j.rows.get_mut(k) { (r.bytes, r.done, r.partial) = (b, done, p) }
            if done { j.known.insert(key(&names[k]), (b, p)); }
        });
        if usage_fill(Path::new(&dir), &mut rows, &stop, &paused, &mut update) {
            publish(gen, true, |j| j.done = true);
        }
    }
}

// ---------- the window ----------

// prev: focus before it opened (Esc goes back there); want: the row to select once the rows come (the folder ←
// came up from); list: sorted, as shown; rows: rows on screen
#[derive(Default)]
struct UsageUi { h: HWND, prev: HWND, pane: usize, gen: usize, dir: String, want: Option<String>, list: Vec<URow>, head: String, max: u64, total: u64, done: bool, failed: bool, sel: Option<usize>, top: usize, rows: usize, w: i32, wheel: i32 }

thread_local! {
    static UI: RefCell<UsageUi> = RefCell::new(UsageUi::default());
}

// Same rule as app(): no Win32 call that can come back into usage_proc while inside.
fn ui<R>(f: impl FnOnce(&mut UsageUi) -> R) -> R {
    UI.with(|u| f(&mut u.borrow_mut()))
}

struct Geo { h: i32, head: RECT, rows: Vec<RECT>, foot: RECT }

fn geo(dpi: i32, n: usize, w: i32) -> Geo {
    let s = |v: i32| v * dpi / 96;
    let (pad, rh) = (s(6), s(RH));
    let head = RECT { left: pad, top: pad, right: w - pad, bottom: pad + s(30) };
    let top = head.bottom + s(4);
    let rows = (0..n as i32).map(|k| RECT { left: pad, top: top + k * rh, right: w - pad, bottom: top + (k + 1) * rh }).collect();
    let foot = RECT { left: pad, top: top + n as i32 * rh + s(2), right: w - pad, bottom: top + n as i32 * rh + s(26) };
    Geo { h: foot.bottom + pad, head, rows, foot }
}

// A row's cells: icon, name, note (calculating / partial), size, share, bar.
fn cells(r: RECT, dpi: i32) -> [RECT; 6] {
    let s = |v: i32| v * dpi / 96;
    let cell = |l: i32, rt: i32| RECT { left: l, right: rt, ..r };
    let bar = cell(r.right - s(148), r.right - s(8));
    let pct = cell(bar.left - s(52), bar.left - s(8));
    let size = cell(pct.left - s(86), pct.left - s(6));
    let note = cell(size.left - s(78), size.left - s(6));
    let icon = RECT { left: r.left + s(8), top: r.top + (r.bottom - r.top - s(16)) / 2, right: r.left + s(24), bottom: 0 };
    [icon, cell(icon.right + s(8), note.left - s(6)), note, size, pct, bar]
}

pub(crate) unsafe fn open_usage(i: usize) {
    let path = app(|a| a.s.paths[i].clone());
    if !is_fs(&path) {
        _ = MessageBeep(MB_ICONWARNING);
        set_note(tr("Folder usage works in a folder on a disk or a share", "용량 분석은 디스크나 공유 폴더에서만 쓸 수 있음").into());
        return;
    }
    let prev = GetFocus();
    usage_close(false);
    let (main, dpi, pane) = app(|a| (a.main, a.dpi, a.panes[i].as_ref().map(|p| p.hwnd)));
    let mut rc = RECT::default();
    _ = GetWindowRect(pane.unwrap_or(main), &mut rc);
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let work = mi.rcWork;
    let w = (760 * dpi / 96).min(work.right - work.left - 12 * dpi / 96).max(560 * dpi / 96);
    let rows = ((work.bottom - work.top - geo(dpi, 0, w).h - 24 * dpi / 96) / (RH * dpi / 96)).clamp(3, 16) as usize;
    let gh = geo(dpi, rows, w).h;
    let (cx, cy) = ((rc.left + rc.right - w) / 2, (rc.top + rc.bottom - gh) / 2);
    let at = fit(RECT { left: cx, top: cy, right: cx + w, bottom: cy + gh }, work);
    let style = WS_POPUP | WS_VISIBLE;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsUsage"), w!(""), style, at.left, at.top, w, gh, Some(main), None, None, None) else { return };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    ui(|u| *u = UsageUi { h, prev, pane: i, rows, w, ..Default::default() });
    _ = SetFocus(Some(h));
    usage_go(path, None);
}

// Shows `dir` (sizes known from this window come back at once), selecting `want` when it is there.
unsafe fn usage_go(dir: String, want: Option<String>) {
    let (h, main) = (ui(|u| u.h), app(|a| a.main));
    let gen = usage_start(Some(&dir), main);
    ui(|u| *u = UsageUi { gen, dir, want, h: u.h, prev: u.prev, pane: u.pane, rows: u.rows, w: u.w, ..Default::default() });
    _ = InvalidateRect(Some(h), None, false);
}

// WM_USAGE on the main window: the worker's rows, sorted again; the selection stays on its item.
pub(crate) unsafe fn usage_msg() {
    POSTED.store(false, Ordering::Relaxed);
    let (h, gen) = ui(|u| (u.h, u.gen));
    let snap = {
        let j = job();
        (j.gen == gen && !h.0.is_null()).then(|| (j.rows.clone(), j.done, j.failed))
    };
    let Some((raw, done, failed)) = snap else { return };
    let head = if failed { tr("Can't read this folder", "이 폴더를 읽을 수 없음").to_string() } else { usage_head(KO.load(Ordering::Relaxed), &raw, done) };
    let total = raw.iter().map(|r| r.bytes).sum();
    let list = usage_list(raw, USAGE_MAX);
    ui(|u| {
        // the item selected before (the "n more" row as "")
        let keep = u.want.take().or_else(|| u.sel.and_then(|k| u.list.get(k)).map(|r| if r.more > 0 { String::new() } else { r.name.clone() }));
        let sel = match keep {
            Some(n) if n.is_empty() => list.len().checked_sub(1).filter(|&k| list[k].more > 0),
            Some(n) => usage_pick(&list, &n),
            None => None,
        };
        let was = u.sel;
        u.sel = sel.or((!list.is_empty()).then_some(0));
        u.max = list.iter().map(|r| r.bytes).max().unwrap_or(0);
        (u.list, u.head, u.total, u.done, u.failed) = (list, head, total, done, failed);
        // only when the selected row moved: a list scrolled with the wheel stays put while the counts come in
        if u.sel != was {
            u.top = u.sel.map_or(0, |k| ev_scroll(u.top, k, u.rows));
        }
    });
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) unsafe fn usage_close(back: bool) {
    let (h, prev, pane) = ui(|u| (u.h, u.prev, u.pane));
    if h.0.is_null() { return }
    _ = DestroyWindow(h);
    if back && !prev.0.is_null() && IsWindow(Some(prev)).as_bool() && IsWindowVisible(prev).as_bool() {
        _ = SetFocus(Some(prev));
    } else if back {
        focus_view(pane.min(app(|a| a.s.panes()) - 1));
    }
}

// A click anywhere else in Grids closes it (another app may take the focus meanwhile: the counts go on).
pub(crate) unsafe fn usage_click_away(at: HWND) {
    if ui(|u| !u.h.0.is_null() && at != u.h) { usage_close(false) }
}

unsafe fn usage_move(d: i32) {
    let h = ui(|u| {
        u.sel = ev_step(u.sel, u.list.len(), d);
        u.top = u.sel.map_or(u.top, |k| ev_scroll(u.top, k, u.rows));
        u.h
    });
    _ = InvalidateRect(Some(h), None, false);
}

fn picked() -> Option<URow> {
    ui(|u| u.sel.and_then(|k| u.list.get(k).cloned()).filter(|r| r.more == 0))
}

// → / double-click: into the selected folder.
unsafe fn usage_into() {
    match picked() {
        Some(r) if r.folder => usage_go(usage_child(&ui(|u| u.dir.clone()), &r.name), None),
        _ => _ = MessageBeep(MB_ICONWARNING),
    }
}

// ← / Backspace: the folder above (also above the one it opened at), with the one just left selected.
unsafe fn usage_back() {
    let dir = ui(|u| u.dir.clone());
    match usage_parent(&dir) {
        Some(up) => usage_go(up, dir.trim_end_matches('\\').rsplit('\\').next().map(str::to_string)),
        None => _ = MessageBeep(MB_ICONWARNING),
    }
}

// Enter: the pane shows the item selected (in its folder, once it shows: TIMER_EV_SELECT); Ctrl+Enter opens a folder.
unsafe fn usage_show(open: bool) {
    let Some(r) = picked() else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let (dir, pane) = ui(|u| (u.dir.clone(), u.pane));
    let i = app(|a| if pane < a.s.panes() { pane } else { a.s.active });
    usage_close(false);
    let (to, name) = if open && r.folder { (usage_child(&dir, &r.name), None) } else { (dir, Some(r.name)) };
    show_in_pane(i, to, name);
}

// Every key while it has the focus is its own (no Ctrl+W, Alt+← … on the pane behind). Alt keys go to the window
// itself, still not to the pane: Alt+F4 closes it.
pub(crate) unsafe fn usage_key(m: &MSG) -> Option<bool> {
    let (h, page) = ui(|u| (u.h, u.rows as i32));
    if h.0.is_null() || m.hwnd != h { return None }
    if m.message == WM_SYSKEYDOWN || m.message == WM_SYSKEYUP { return Some(false) }
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        VK_ESCAPE => usage_close(true),
        VK_UP => usage_move(-1),
        VK_DOWN => usage_move(1),
        VK_PRIOR => usage_move(-page),
        VK_NEXT => usage_move(page),
        VK_HOME => usage_move(-(USAGE_MAX as i32)),
        VK_END => usage_move(USAGE_MAX as i32),
        VK_RIGHT => usage_into(),
        VK_LEFT | VK_BACK => usage_back(),
        VK_RETURN => {
            app(|a| a.held = Some(VK_RETURN.0)); // its repeats must not open the item in the list after
            usage_show(GetKeyState(VK_CONTROL.0 as i32) < 0);
        }
        _ => {}
    }
    Some(true)
}

unsafe fn paint_usage(h: HWND) {
    let (dpi, font, small, accent) = app(|a| (a.dpi, a.font, a.small_font, a.s.accent_now()));
    let (n, w, top, sel, len) = ui(|u| (u.rows, u.w, u.top, u.sel, u.list.len()));
    let g = geo(dpi, n, w);
    let (dir, head, rows, max, total, done, failed) =
        ui(|u| (u.dir.clone(), u.head.clone(), u.list[top.min(len)..(top + n).min(len)].to_vec(), u.max, u.total, u.done, u.failed));
    let (ilist, p, s) = (small_ilist(), pal(), |v: i32| v * dpi / 96);
    buffered(h, |dc, rc| {
        fill(dc, rc, p.card);
        frame(dc, rc, 1, p.line);
        let sum_w = s(300);
        text(dc, RECT { left: g.head.left + s(8), right: g.head.right - sum_w, ..g.head }, &dir, font, p.text, DT_LEFT | DT_PATH_ELLIPSIS);
        text(dc, RECT { left: g.head.right - sum_w + s(8), right: g.head.right - s(8), ..g.head }, &head, small, p.sub, DT_RIGHT);
        fill(dc, RECT { top: g.head.bottom + s(1), bottom: g.head.bottom + s(2), ..g.head }, p.line);
        if rows.is_empty() {
            let msg = if failed { tr("Can't read this folder", "이 폴더를 읽을 수 없음") } else if done { tr("Empty folder", "빈 폴더") } else { tr("Reading…", "읽는 중…") };
            text(dc, RECT { top: g.head.bottom + s(4), bottom: g.head.bottom + s(84), ..g.head }, msg, font, p.sub, DT_CENTER);
        }
        for (j, (r, row)) in rows.iter().zip(&g.rows).enumerate() {
            let on = sel == Some(top + j);
            let (ink, sub) = if on { (on_color(accent), blend(on_color(accent), accent, 0.3)) } else { (p.text, p.sub) };
            if on { fill(dc, *row, accent) }
            let [icon, name, note, size, pct, bar] = cells(*row, dpi);
            let ix = if r.more > 0 { -1 } else { type_icon(&r.name, r.folder) };
            if let Some(hi) = ilist.as_ref().filter(|_| ix >= 0).and_then(|il| il.GetIcon(ix, ILD_NORMAL.0).ok()) {
                _ = DrawIconEx(dc, icon.left, icon.top, hi, s(16), s(16), 0, None, DI_NORMAL);
                _ = DestroyIcon(hi);
            }
            let label = match (r.more, KO.load(Ordering::Relaxed)) {
                (0, _) => r.name.clone(),
                (k, true) => format!("그 밖의 항목 {}개", group_digits(k as u64)),
                (k, false) => format!("{} more items", group_digits(k as u64)),
            };
            text(dc, name, &label, font, if r.more > 0 { sub } else { ink }, DT_LEFT);
            let tag = if !r.done { tr("calculating…", "계산 중…") } else if r.partial { tr("partial", "일부만") } else { "" };
            text(dc, note, tag, small, sub, DT_RIGHT);
            if r.done || r.bytes > 0 {
                text(dc, size, &human(r.bytes), small, if r.done { ink } else { sub }, DT_RIGHT);
                text(dc, pct, &usage_pct(r.bytes, total), small, sub, DT_RIGHT);
            }
            // the bar: as long as the biggest row's; a folder still being summed in a paler shade
            let mid = (bar.top + bar.bottom) / 2;
            let track = RECT { top: mid - s(4), bottom: mid + s(4), ..bar };
            let (back, ink_bar) = if on { (blend(on_color(accent), accent, 0.75), on_color(accent)) } else { (p.line, bar_fill(&p, accent, false)) };
            fill(dc, track, back);
            let len = usage_bar(r.bytes, max, bar.right - bar.left);
            if len > 0 {
                fill(dc, RECT { right: bar.left + len, ..track }, if r.done { ink_bar } else { blend(ink_bar, back, 0.5) });
            }
        }
        fill(dc, RECT { top: g.foot.top, bottom: g.foot.top + s(1), ..g.foot }, p.line);
        let hint = tr("→ into a folder · ← up · Enter: select it in the pane · Ctrl+Enter: open the folder · Esc: close", "→ 폴더 안으로 · ← 위로 · Enter: 패널에서 선택 · Ctrl+Enter: 폴더 열기 · Esc: 닫기");
        text(dc, g.foot, hint, small, p.sub, DT_CENTER);
    });
}

pub(crate) extern "system" fn usage_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => paint_usage(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = xy(lp);
                let (dpi, (n, w, top, len)) = (app(|a| a.dpi), ui(|u| (u.rows, u.w, u.top, u.list.len())));
                if let Some(j) = geo(dpi, n, w).rows.iter().position(|r| inside(*r, x, y)).filter(|j| top + j < len) {
                    ui(|u| u.sel = Some(top + j));
                    _ = InvalidateRect(Some(h), None, false);
                    if msg == WM_LBUTTONDBLCLK {
                        usage_into();
                    }
                }
            }
            WM_MOUSEWHEEL => {
                // touchpads send less than a notch at a time: the rest waits in u.wheel
                let d = ui(|u| -wheel_notches(&mut u.wheel, (wp.0 >> 16) as i16 as i32) * 3);
                if d != 0 {
                    ui(|u| u.top = (u.top as i32 + d).clamp(0, u.list.len().saturating_sub(u.rows) as i32) as usize);
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            // another Grids window took over: close; another app: stay, the counting goes on
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => {
                let to = HWND(lp.0 as _);
                if !to.0.is_null() && GetWindowThreadProcessId(to, None) == GetCurrentThreadId() {
                    _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            WM_DESTROY => {
                ui(|u| *u = UsageUi::default());
                usage_start(None, HWND::default()); // stops the worker and forgets the sizes
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, bytes: u64) -> URow {
        URow { name: name.into(), bytes, done: true, ..Default::default() }
    }

    #[test]
    fn usage_sorting_and_text() {
        let names = |v: &[URow]| v.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
        let rows = vec![row("b", 5), row("A", 5), row("big", 900), row("c", 0)];
        let l = usage_list(rows.clone(), 10);
        assert_eq!(names(&l), ["big", "A", "b", "c"]); // biggest first, the same size by name in any case
        // sizes change: the selected item is found again where it went
        let mut grown = rows.clone();
        grown[0].bytes = 1000;
        let l2 = usage_list(grown, 10);
        assert_eq!((usage_pick(&l, "b"), usage_pick(&l2, "b"), usage_pick(&l2, "gone")), (Some(2), Some(0), None));
        // past the cap: the smallest together in one last row, never picked by name
        let capped = usage_list(rows, 3);
        assert_eq!((capped.len(), capped[2].more, capped[2].bytes, capped[2].name.as_str(), usage_pick(&capped, "")), (3, 2, 5, "", None));
        // shares and bars, an empty whole and 0 bytes included
        assert_eq!([usage_pct(50, 200), usage_pct(1, 1000), usage_pct(0, 10), usage_pct(5, 0), usage_pct(10, 10)], ["25%", "<1%", "0%", "0%", "100%"]);
        assert_eq!([usage_bar(50, 100, 140), usage_bar(1, 1 << 40, 140), usage_bar(0, 100, 140), usage_bar(5, 0, 140), usage_bar(100, 100, 140)], [70, 1, 0, 0, 140]);
        let mut half = vec![row("a", 1024), URow { name: "d".into(), folder: true, bytes: 1024, ..Default::default() }];
        assert_eq!(usage_head(false, &half, false), "2.0 KB · 0 of 1 folders counted");
        assert_eq!(usage_head(true, &half, false), "2.0 KB · 하위 폴더 0/1 계산됨");
        (half[1].done, half[1].partial) = (true, true);
        assert_eq!(usage_head(false, &half, true), "2.0 KB · all counted · partial");
    }

    #[test]
    fn usage_paths() {
        let up = |p: &str| usage_parent(p);
        assert_eq!([up(r"C:\a\b"), up(r"C:\a\"), up(r"\\nas\share\x")], [Some(r"C:\a".into()), Some("C:\\".into()), Some(r"\\nas\share".into())]);
        assert_eq!([up("C:\\"), up(r"\\nas\share"), up(r"\\nas")], [None, None, None]); // a drive or share's root: as far up as it goes
        assert_eq!((usage_child("C:\\", "a"), usage_child(r"C:\a", "b")), (r"C:\a".to_string(), r"C:\a\b".to_string()));
    }

    // A small tree: the rows add up to the real bytes, folders summed whole, the files as they are.
    #[test]
    fn usage_worker_sums() {
        let root = std::env::temp_dir().join(format!("grids_usage_{}", std::process::id()));
        _ = std::fs::remove_dir_all(&root);
        for (p, n) in [("a.bin", 10), (r"sub\x.bin", 5), (r"sub\deep\y.bin", 7)] {
            let f = root.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, vec![0u8; n]).unwrap();
        }
        std::fs::create_dir_all(root.join("empty")).unwrap();
        let mut rows = usage_entries(&root, &|| false).unwrap();
        let mut finished = Vec::new();
        assert!(usage_fill(&root, &mut rows, &|| false, &|| false, &mut |k, b, done, _| finished.extend(done.then_some((k, b)))));
        let l = usage_list(rows.clone(), USAGE_MAX);
        let got: Vec<(&str, u64, bool, bool)> = l.iter().map(|r| (r.name.as_str(), r.bytes, r.folder, r.done)).collect();
        assert_eq!(got, [("sub", 12, true, true), ("a.bin", 10, false, true), ("empty", 0, true, true)]);
        assert_eq!(finished.len(), 2);
        assert!(usage_fill(&root, &mut rows, &|| true, &|| false, &mut |_, _, _, _| panic!("done already")));
        let mut fresh = usage_entries(&root, &|| false).unwrap();
        assert!(!usage_fill(&root, &mut fresh, &|| true, &|| false, &mut |_, _, _, _| {})); // stopped
        assert!(usage_entries(&root.join("gone"), &|| false).is_none());
        _ = std::fs::remove_dir_all(root);
    }
}
