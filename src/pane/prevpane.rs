//! Right-hand preview of the active pane's focused item.
//!
//! A registered Windows preview handler draws Office, PDF, and HTML, with the
//! attribute header kept above its child window. When no handler is registered,
//! or the handler fails, the pane paints the same body as Quick Look (text, code,
//! a CSV grid, a picture, a folder list, a card). The handler lives on its own
//! STA thread. Arrow keys only arm a short timer, so the file list never waits.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, LazyLock, Mutex, Once};

use super::fsize::{cached_stat, classify_ui, is_volume_root, preview_walk, remember_stat};
use super::peek::{bar_hit, bar_layout, bar_rgb, host_handler, nudge, page_toward, paint_card, thumb_scroll, BarHit, PeekReq, Pix};
use super::preview::*;
use super::pview::{self, line_no_at, list_row_at, row_h, selected_text, BodyScroll};
use super::status::{dir_totals, DirTotals, DIR_STACK_CAP};
use super::{focus_view, navigate};
use crate::app::*;
use crate::i18n::*;
use crate::imports::*;
use crate::ui::*;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::ILD_NORMAL;
use windows::Win32::UI::Shell::IPreviewHandler;

const PREVIEW_DELAY_MS: u32 = 200;
const WM_PREVIEW_READY: u32 = WM_APP + 70; // wParam = generation; the result sits in READY
const WM_PREVIEW_SIZE: u32 = WM_APP + 71; // thread message: the host window changed size
const WM_PREVIEW_HOSTED: u32 = WM_APP + 72; // wParam = generation, lParam = 1 shown / 0 failed
const WM_PREVIEW_TOTAL: u32 = WM_APP + 73; // wParam = generation; a folder walk finished
const CMD_COPY: usize = 1;
const CMD_EDIT: usize = 2;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Idle,
    Opening,
    Handler,
    Shown,
    Empty,
}

#[derive(Clone, Copy, PartialEq)]
enum Total {
    None,
    Pending,
    Ready { bytes: u64, partial: bool },
    Skipped,
}

#[derive(Clone, Copy, PartialEq)]
enum Btn {
    None,
    Copy,
    Edit,
}

#[derive(Clone, Copy, Default)]
struct Hit {
    copy: RECT,
    edit: RECT,
    body: RECT,
    bar: RECT,
    thumb: RECT,
}

struct Slot {
    gen: u64,
    path: String,
    title: String,
    kind: Kind,
    facts: FileFacts,
    body: Option<Body>,
    big: Option<Pix>,
    folder: bool,
    total: Total,
    scroll: BodyScroll,
    sel: Option<(usize, usize)>,
    anchor: usize,
    hot: Btn,
    hit: Hit,
    head_h: i32,
    bar_drag: Option<i32>,
    host: isize, // HWND is not Send; the UI thread is the only one that creates or destroys it
    tid: u32,
    area_w: i32,
    area_h: i32,
    seq: u64,
    /// Focus was outside this pane when the load started. A handler that takes it is given back.
    /// A click in the pane clears the flag so the handler can keep the keyboard.
    return_focus: bool,
    cancel: Option<Arc<AtomicBool>>,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            gen: 0,
            path: String::new(),
            title: String::new(),
            kind: Kind::Idle,
            facts: FileFacts::default(),
            body: None,
            big: None,
            folder: false,
            total: Total::None,
            scroll: BodyScroll::default(),
            sel: None,
            anchor: 0,
            hot: Btn::None,
            hit: Hit::default(),
            head_h: 0,
            bar_drag: None,
            host: 0,
            tid: 0,
            area_w: 0,
            area_h: 0,
            seq: 0,
            return_focus: false,
            cancel: None,
        }
    }
}

enum Ready {
    Handler { clsid: GUID, facts: FileFacts },
    Shown { body: Body, facts: FileFacts, big: Option<Pix>, folder: bool },
    Miss,
}

struct Work {
    hwnd: isize,
    gen: u64,
    path: String,
    w: i32,
    h: i32,
    skip_handler: bool,
}

fn slots() -> std::sync::MutexGuard<'static, HashMap<isize, Slot>> {
    SLOTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn jobs() -> std::sync::MutexGuard<'static, HashMap<isize, Work>> {
    JOBS.lock().unwrap_or_else(|e| e.into_inner())
}

fn ready_box() -> std::sync::MutexGuard<'static, HashMap<isize, (u64, Ready)>> {
    READY.lock().unwrap_or_else(|e| e.into_inner())
}

fn totals() -> std::sync::MutexGuard<'static, HashMap<isize, (u64, DirTotals)>> {
    TOTALS.lock().unwrap_or_else(|e| e.into_inner())
}

static SLOTS: LazyLock<Mutex<HashMap<isize, Slot>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static JOBS: LazyLock<Mutex<HashMap<isize, Work>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static READY: LazyLock<Mutex<HashMap<isize, (u64, Ready)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static TOTALS: LazyLock<Mutex<HashMap<isize, (u64, DirTotals)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static JOB_CV: Condvar = Condvar::new();
static WORKER: Once = Once::new();

thread_local! {
    static PANE_FACES: RefCell<super::pview::FaceCache> = RefCell::new(None);
}

fn key_of(h: HWND) -> isize {
    h.0 as isize
}

fn idle(slot: &Slot) -> bool {
    slot.kind == Kind::Idle && slot.host == 0 && slot.path.is_empty() && slot.body.is_none() && slot.tid == 0
}

fn cancel_walk(slot: &mut Slot) {
    if let Some(flag) = slot.cancel.take() {
        flag.store(true, Ordering::Relaxed);
    }
}

unsafe fn reset_scroll(scroll: &mut BodyScroll) {
    PANE_FACES.with(|c| pview::clear_faces(&mut c.borrow_mut()));
    *scroll = BodyScroll::default();
}

fn client_size(h: HWND) -> (i32, i32) {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(h, &mut rc) };
    ((rc.right - rc.left).max(0), (rc.bottom - rc.top).max(0))
}

/// Keep the first `max_n` header lines. Anything past that is joined onto the last line and ellipsized.
pub(crate) fn cap_lines(mut lines: Vec<String>, max_n: usize, budget: usize) -> Vec<String> {
    if max_n == 0 {
        return Vec::new();
    }
    if lines.len() <= max_n {
        return lines;
    }
    let extra = lines.split_off(max_n - 1);
    let joined = extra.join(" · ");
    lines.push(ellipsize(&joined, budget));
    lines
}

fn band(name: &str, facts: &FileFacts, width: i32, height: i32, dpi: i32, fh: i32) -> (Vec<String>, Vec<String>, i32, i32, i32, i32) {
    let s = |v: i32| v * dpi / 96;
    let pad = s(8).max(4);
    let icon = s(16).max(12);
    let line = (fh + s(2)).max(icon);
    let text_w = (width - pad * 2 - icon - s(6)).max(s(40));
    let budget = header_char_budget(text_w, fh.max(1)).max(8);
    let room = (height - s(32)).max(line);
    let max_rows = (room / line).clamp(1, 6) as usize;
    let rows = cap_lines(header_lines(name, &facts.fields(), budget), max_rows, budget);
    let exif = if facts.exif.is_empty() {
        Vec::new()
    } else {
        let parts: Vec<String> = facts.exif.iter().map(|(k, v)| format!("{k} {v}")).collect();
        let left = max_rows.saturating_sub(rows.len()).max(1).min(3);
        cap_lines(wrap_fields(&parts, budget), left, budget)
    };
    let mut h = pad + line * rows.len() as i32 + pad / 2;
    if !exif.is_empty() {
        h += s(2) + line * exif.len() as i32;
    }
    h = h.min((height - s(28)).max(line + pad));
    (rows, exif, h, line, pad, icon)
}

fn shown_name(slot: &Slot) -> String {
    if !slot.title.is_empty() { slot.title.clone() } else { slot.facts.name.clone() }
}

fn has_text(slot: &Slot) -> bool {
    matches!(slot.body, Some(Body::Text { .. } | Body::Document { .. }))
}

fn total_line(total: Total) -> String {
    match total {
        Total::None => String::new(),
        Total::Pending => tf(crate::Key::FolderTotal, &[("size", "…")]),
        Total::Ready { bytes, partial: false } => tf(crate::Key::FolderTotal, &[("size", &crate::human(bytes))]),
        Total::Ready { bytes, partial: true } => tf(crate::Key::FolderTotalPartial, &[("size", &crate::human(bytes))]),
        Total::Skipped => tr(crate::Key::FolderTotalSkipped).to_string(),
    }
}

fn join_child(root: &str, name: &str) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['\\', '/']) {
        return None;
    }
    Some(if root.ends_with(['\\', '/']) { format!("{root}{name}") } else { format!("{root}\\{name}") })
}

fn in_rect(r: RECT, x: i32, y: i32) -> bool {
    r.right > r.left && r.bottom > r.top && x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

unsafe fn focused_file(i: usize) -> (String, String) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return (String::new(), String::new()) };
    let idx = fv.GetFocusedItem().unwrap_or(-1);
    if idx < 0 {
        return (String::new(), String::new());
    }
    let Ok(it) = fv.GetItem::<IShellItem>(idx) else { return (String::new(), String::new()) };
    let name = it.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| pwstr(p)).unwrap_or_default();
    let path = it.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)).unwrap_or_default();
    let title = if name.is_empty() { path.rsplit(['\\', '/']).next().unwrap_or("").to_string() } else { name };
    (path, title)
}

/// Selection moved, or the active pane changed. The timer is reset, so a held arrow
/// does not start a handler until the highlight rests.
pub(crate) unsafe fn note_preview(i: usize) {
    let (on, active, main) = app(|a| (a.s.preview_on, a.s.active, a.main));
    if !on || i != active || main.0.is_null() {
        return;
    }
    SetTimer(Some(main), TIMER_PREVIEW, PREVIEW_DELAY_MS, None);
}

/// The pane just became visible and has nothing loaded. Arm the same short delay.
pub(crate) unsafe fn arm_preview() {
    let (on, i, h) = app(|a| (a.s.preview_on, a.s.active, a.preview_hwnd));
    if !on || h.0.is_null() {
        return;
    }
    let idle_now = slots().get(&key_of(h)).is_none_or(idle);
    if idle_now {
        note_preview(i);
    }
}

/// Status-timer backup for a selection change the shell did not report, and a late
/// focus grab by a handler the user has not clicked.
pub(crate) unsafe fn poll_preview() {
    let (on, i, h) = app(|a| (a.s.preview_on, a.s.active, a.preview_hwnd));
    if !on || h.0.is_null() {
        return;
    }
    give_back(h);
    let (path, _) = focused_file(i);
    let same = slots().get(&key_of(h)).is_some_and(|s| s.path == path);
    if !same {
        note_preview(i);
    }
}

pub(crate) unsafe fn run_preview(force: bool) {
    let (on, i, h, main) = app(|a| (a.s.preview_on, a.s.active, a.preview_hwnd, a.main));
    if !on || h.0.is_null() {
        return;
    }
    _ = KillTimer(Some(main), TIMER_PREVIEW);
    let (bw, bh) = client_size(h);
    // A zero-size plate must not be recorded as Opening: the next poll would then skip it.
    if bw < 8 || bh < 8 {
        return;
    }
    let (path, title) = focused_file(i);
    let key = key_of(h);
    if !force {
        let skip = slots().get(&key).is_some_and(|s| {
            s.path == path && matches!(s.kind, Kind::Opening | Kind::Handler | Kind::Shown | Kind::Empty)
        });
        if skip {
            return;
        }
    }
    let focus = GetFocus();
    let inside = focus == h || IsChild(h, focus).as_bool();
    let gen = {
        let mut map = slots();
        let slot = map.entry(key).or_default();
        cancel_walk(slot);
        reset_scroll(&mut slot.scroll);
        slot.gen = slot.gen.wrapping_add(1).max(1);
        slot.path = path.clone();
        slot.title = title;
        slot.facts = FileFacts::default();
        slot.body = None;
        slot.big = None;
        slot.folder = false;
        slot.total = Total::None;
        slot.sel = None;
        slot.anchor = 0;
        slot.hot = Btn::None;
        slot.hit = Hit::default();
        slot.bar_drag = None;
        slot.return_focus = !inside;
        slot.kind = if path.is_empty() { Kind::Empty } else { Kind::Opening };
        slot.gen
    };
    drop_host(key);
    if path.is_empty() {
        _ = InvalidateRect(Some(h), None, false);
        return;
    }
    enqueue(Work { hwnd: key, gen, path, w: bw, h: bh, skip_handler: false });
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) unsafe fn preview_clear() {
    let (h, main) = app(|a| (a.preview_hwnd, a.main));
    if !main.0.is_null() {
        _ = KillTimer(Some(main), TIMER_PREVIEW);
    }
    if h.0.is_null() {
        return;
    }
    let key = key_of(h);
    let stop = {
        let mut map = slots();
        let Some(slot) = map.get_mut(&key) else { return };
        if idle(slot) {
            return;
        }
        cancel_walk(slot);
        reset_scroll(&mut slot.scroll);
        slot.gen = slot.gen.wrapping_add(1).max(1);
        slot.kind = Kind::Idle;
        slot.path.clear();
        slot.title.clear();
        slot.facts = FileFacts::default();
        slot.body = None;
        slot.big = None;
        slot.folder = false;
        slot.total = Total::None;
        slot.sel = None;
        slot.anchor = 0;
        slot.return_focus = false;
        slot.tid = 0;
        true
    };
    if stop {
        jobs().remove(&key);
        drop_host(key);
        _ = InvalidateRect(Some(h), None, false);
    }
}

/// Theme changed under a live handler. Drawn bodies pick the palette up on the next paint.
/// A handler was started with the old colours, so it is opened again.
pub(crate) unsafe fn retint_preview() {
    let h = app(|a| a.preview_hwnd);
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    let dark = DARK_ON.load(Ordering::Relaxed);
    super::theme_hwnd(h, dark);
    let host = slots().get(&key_of(h)).map(|s| s.host).unwrap_or(0);
    if host != 0 {
        super::theme_hwnd(HWND(host as _), dark);
    }
    let handler = slots().get(&key_of(h)).is_some_and(|s| s.kind == Kind::Handler);
    if handler {
        run_preview(true);
    } else {
        _ = InvalidateRect(Some(h), None, true);
    }
}

fn enqueue(job: Work) {
    WORKER.call_once(|| {
        let _ = std::thread::Builder::new().name("maguro-preview".into()).spawn(|| worker_loop());
    });
    jobs().insert(job.hwnd, job);
    JOB_CV.notify_one();
}

fn worker_loop() {
    loop {
        let job = {
            let mut q = jobs();
            while q.is_empty() {
                q = JOB_CV.wait(q).unwrap_or_else(|e| e.into_inner());
            }
            let key = *q.keys().next().unwrap();
            q.remove(&key).unwrap()
        };
        if !current(job.hwnd, job.gen) {
            continue;
        }
        let ready = std::panic::catch_unwind(AssertUnwindSafe(|| produce(&job))).unwrap_or(Ready::Miss);
        if !current(job.hwnd, job.gen) {
            continue;
        }
        deliver(job.hwnd, job.gen, ready);
    }
}

fn current(hwnd: isize, gen: u64) -> bool {
    slots().get(&hwnd).is_some_and(|s| s.gen == gen)
}

fn deliver(hwnd: isize, gen: u64, ready: Ready) {
    ready_box().insert(hwnd, (gen, ready));
    unsafe { _ = PostMessageW(Some(HWND(hwnd as _)), WM_PREVIEW_READY, WPARAM(gen as usize), LPARAM(0)) };
}

fn produce(job: &Work) -> Ready {
    let meta_fs = match std::fs::metadata(&job.path) {
        Ok(m) => m,
        Err(_) => return Ready::Miss,
    };
    let is_dir = meta_fs.is_dir();
    let len = meta_fs.len();
    let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let started = com.is_ok();
    let ready = std::panic::catch_unwind(AssertUnwindSafe(|| unsafe { produce_com(job, is_dir, len) })).unwrap_or(Ready::Miss);
    if started {
        unsafe { CoUninitialize() };
    }
    ready
}

unsafe fn produce_com(job: &Work, is_dir: bool, len: u64) -> Ready {
    if !is_dir && !job.skip_handler && len > 0 && len <= HEAVY_MAX {
        let ext = ext_of(&job.path);
        if let Some(g) = handler_for(&ext) {
            let facts = meta(&job.path, None);
            return Ready::Handler { clsid: g, facts };
        }
    }
    let req = PeekReq { path: job.path.clone(), w: job.w.max(1), h: job.h.max(1), hwnd: 0, plain: true };
    let body = content(&req);
    let facts = meta(&job.path, Some(&body));
    // A UNC icon factory can stall on a dead share. The header still has the small system icon.
    let big = if is_dir && !job.path.starts_with(r"\\") { shell_pic(&job.path, 96, 96, SIIGBF_ICONONLY) } else { None };
    Ready::Shown { body, facts, big, folder: is_dir }
}

fn take_ready(hwnd: isize, gen: u64) -> Option<Ready> {
    let mut box_ = ready_box();
    match box_.get(&hwnd) {
        Some((g, _)) if *g == gen => box_.remove(&hwnd).map(|(_, r)| r),
        _ => None,
    }
}

fn publish_total(hwnd: isize, gen: u64, stat: DirTotals) {
    let mut box_ = totals();
    match box_.get(&hwnd) {
        Some((g, _)) if *g > gen => {}
        _ => {
            box_.insert(hwnd, (gen, stat));
        }
    }
}

unsafe fn arm_total(h: HWND, gen: u64) {
    let key = key_of(h);
    let path = {
        let map = slots();
        let Some(slot) = map.get(&key) else { return };
        if !slot.folder || slot.gen != gen {
            return;
        }
        slot.path.clone()
    };
    if path.is_empty() {
        return;
    }
    let root = is_volume_root(&path);
    let place = classify_ui(&path);
    let network = app(|a| a.s.folder_sizes_network);
    if !preview_walk(root, place, network) {
        if let Some(slot) = slots().get_mut(&key) {
            if slot.gen == gen {
                slot.total = Total::Skipped;
            }
        }
        return;
    }
    if let Some(st) = cached_stat(&path) {
        if let Some(slot) = slots().get_mut(&key) {
            if slot.gen == gen {
                slot.total = Total::Ready { bytes: st.bytes, partial: st.partial };
            }
        }
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut map = slots();
        let Some(slot) = map.get_mut(&key) else { return };
        if slot.gen != gen {
            return;
        }
        if let Some(old) = slot.cancel.replace(cancel.clone()) {
            old.store(true, Ordering::Relaxed);
        }
        slot.total = Total::Pending;
    }
    let hwnd = h.0 as isize;
    let path2 = path;
    let _ = std::thread::Builder::new().name("maguro-preview-size".into()).spawn(move || {
        let stop = || cancel.load(Ordering::Relaxed);
        let paused = || crate::COPYING.load(Ordering::Relaxed) > 0;
        let mut tick = |_: u64, _: bool| {};
        let got = dir_totals(Path::new(&path2), &stop, &paused, &mut tick, DIR_STACK_CAP);
        if stop() {
            return;
        }
        let Some(t) = got else { return };
        if stop() {
            return;
        }
        remember_stat(&path2, t.bytes, t.files, t.folders, t.partial);
        publish_total(key, gen, t);
        _ = PostMessageW(Some(HWND(hwnd as _)), WM_PREVIEW_TOTAL, WPARAM(gen as usize), LPARAM(0));
    });
}

unsafe fn on_total(h: HWND, gen: u64) {
    let key = key_of(h);
    let stat = {
        let box_ = totals();
        box_.get(&key).and_then(|(g, t)| (*g == gen).then_some(*t))
    };
    let Some(stat) = stat else { return };
    if let Some(slot) = slots().get_mut(&key) {
        if slot.gen == gen && slot.folder {
            slot.total = Total::Ready { bytes: stat.bytes, partial: stat.partial };
        }
    }
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn drop_host(key: isize) {
    let host = {
        let mut map = slots();
        let Some(slot) = map.get_mut(&key) else { return };
        slot.tid = 0;
        let host = slot.host;
        slot.host = 0;
        host
    };
    let host = HWND(host as _);
    if !host.0.is_null() && IsWindow(Some(host)).as_bool() {
        _ = DestroyWindow(host);
    }
}

unsafe fn give_back(h: HWND) {
    let want = slots().get(&key_of(h)).is_some_and(|s| s.return_focus);
    if !want {
        return;
    }
    let f = GetFocus();
    if f == h || IsChild(h, f).as_bool() {
        let i = app(|a| a.s.active);
        focus_view(i);
    }
}

unsafe fn place_head(h: HWND) {
    let (dpi, fh) = app(|a| (a.dpi, a.fh));
    let (w, ht) = client_size(h);
    let key = key_of(h);
    if let Some(slot) = slots().get_mut(&key) {
        let name = shown_name(slot);
        let (_, _, head, ..) = band(&name, &slot.facts, w, ht, dpi, fh);
        slot.head_h = head;
    }
}

unsafe fn body_rect(h: HWND) -> RECT {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let head = slots().get(&key_of(h)).map(|s| s.head_h).unwrap_or(0).min((rc.bottom - rc.top).max(0));
    rc.top += head;
    rc
}

unsafe fn on_ready(h: HWND, gen: u64) {
    let key = key_of(h);
    if !current(key, gen) {
        return;
    }
    let Some(ready) = take_ready(key, gen) else { return };
    match ready {
        Ready::Handler { clsid, facts } => {
            let path = {
                let mut map = slots();
                let Some(slot) = map.get_mut(&key) else { return };
                if slot.gen != gen {
                    return;
                }
                slot.facts = facts;
                slot.body = None;
                slot.folder = false;
                slot.path.clone()
            };
            if path.is_empty() || !current(key, gen) {
                return;
            }
            place_head(h);
            start_host(h, gen, clsid, path);
        }
        Ready::Shown { body, facts, big, folder } => {
            {
                let mut map = slots();
                if let Some(slot) = map.get_mut(&key) {
                    if slot.gen == gen {
                        cancel_walk(slot);
                        reset_scroll(&mut slot.scroll);
                        slot.kind = Kind::Shown;
                        slot.body = Some(body);
                        slot.facts = facts;
                        slot.big = big;
                        slot.folder = folder;
                        slot.total = Total::None;
                        slot.sel = None;
                        slot.anchor = 0;
                    }
                }
            }
            drop_host(key);
            place_head(h);
            if folder {
                arm_total(h, gen);
            }
            give_back(h);
            _ = InvalidateRect(Some(h), None, false);
        }
        Ready::Miss => on_ready_miss(h, gen),
    }
}

unsafe fn start_host(h: HWND, gen: u64, clsid: GUID, path: String) {
    let key = key_of(h);
    if !current(key, gen) {
        return;
    }
    drop_host(key);
    let body = body_rect(h);
    let w = (body.right - body.left).max(1);
    let ht = (body.bottom - body.top).max(1);
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(win) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroPreviewHost"), w!(""), style, body.left, body.top, w, ht, Some(h), None, None, None) else {
        enqueue(Work { hwnd: key, gen, path, w, h: ht, skip_handler: true });
        return;
    };
    {
        let mut map = slots();
        let Some(slot) = map.get_mut(&key) else {
            drop(map);
            _ = DestroyWindow(win);
            return;
        };
        if slot.gen != gen {
            drop(map);
            _ = DestroyWindow(win);
            return;
        }
        slot.host = win.0 as isize;
        slot.area_w = w;
        slot.area_h = ht;
        slot.seq = slot.seq.wrapping_add(1);
    }
    let (hwnd, popup, area) = (win.0 as isize, h.0 as isize, RECT { left: 0, top: 0, right: w, bottom: ht });
    let again = path.clone();
    if std::thread::Builder::new().name("maguro-preview-host".into()).spawn(move || host_thread(gen, clsid, path, hwnd, popup, area)).is_err() {
        drop_host(key);
        enqueue(Work { hwnd: key, gen, path: again, w, h: ht, skip_handler: true });
    }
}

unsafe fn host_thread(gen: u64, clsid: GUID, path: String, hwnd: isize, popup: isize, area: RECT) {
    let tid = GetCurrentThreadId();
    let key = popup;
    {
        let mut map = slots();
        if let Some(slot) = map.get_mut(&key) {
            if slot.gen == gen {
                slot.tid = tid;
            }
        }
    }
    let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
    let h = HWND(hwnd as _);
    let mut prime = MSG::default();
    // The queue has to exist before DoPreview, or a size posted during startup is lost.
    _ = PeekMessageW(&mut prime, None, WM_PREVIEW_SIZE, WM_PREVIEW_SIZE, PM_NOREMOVE);
    let ph = host_handler(h, &clsid, &path, area);
    if current(key, gen) {
        _ = PostMessageW(Some(HWND(popup as _)), WM_PREVIEW_HOSTED, WPARAM(gen as usize), LPARAM(ph.is_ok() as isize));
    }
    if let Ok(ph) = ph {
        let mut seen = 0u64;
        let mut msg = MSG::default();
        while current(key, gen) && IsWindow(Some(h)).as_bool() {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_PREVIEW_SIZE {
                    apply_host_rect(&ph, key, gen, &mut seen);
                } else {
                    _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            apply_host_rect(&ph, key, gen, &mut seen);
            _ = MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
        }
        _ = ph.Unload();
    }
    {
        let mut map = slots();
        if let Some(slot) = map.get_mut(&key) {
            if slot.tid == tid {
                slot.tid = 0;
            }
        }
    }
    if com {
        CoUninitialize();
    }
}

unsafe fn apply_host_rect(ph: &IPreviewHandler, key: isize, gen: u64, seen: &mut u64) {
    let Some((seq, w, ht)) = ({
        let map = slots();
        map.get(&key).and_then(|s| (s.gen == gen && s.area_w > 0 && s.area_h > 0).then_some((s.seq, s.area_w, s.area_h)))
    }) else {
        return;
    };
    if seq == *seen {
        return;
    }
    *seen = seq;
    let area = RECT { left: 0, top: 0, right: w, bottom: ht };
    _ = ph.SetRect(&area);
}

unsafe fn sync_host(h: HWND) {
    let key = key_of(h);
    let (host, gen) = {
        let map = slots();
        let Some(slot) = map.get(&key) else { return };
        (slot.host, slot.gen)
    };
    let host = HWND(host as _);
    if host.0.is_null() || !IsWindow(Some(host)).as_bool() {
        return;
    }
    place_head(h);
    let body = body_rect(h);
    let w = (body.right - body.left).max(1);
    let ht = (body.bottom - body.top).max(1);
    _ = SetWindowPos(host, None, body.left, body.top, w, ht, SWP_NOZORDER | SWP_NOACTIVATE);
    let tid = {
        let mut map = slots();
        let Some(slot) = map.get_mut(&key) else { return };
        if slot.gen != gen {
            return;
        }
        slot.area_w = w;
        slot.area_h = ht;
        slot.seq = slot.seq.wrapping_add(1);
        slot.tid
    };
    if tid != 0 {
        _ = PostThreadMessageW(tid, WM_PREVIEW_SIZE, WPARAM(gen as usize), LPARAM(0));
    }
}

unsafe fn on_hosted(h: HWND, gen: u64, ok: bool) {
    let key = key_of(h);
    if !current(key, gen) {
        return;
    }
    if ok {
        if let Some(slot) = slots().get_mut(&key) {
            if slot.gen == gen {
                slot.kind = Kind::Handler;
            }
        }
        give_back(h);
        _ = InvalidateRect(Some(h), None, false);
        return;
    }
    drop_host(key);
    let (path, w, ht) = {
        let map = slots();
        let Some(slot) = map.get(&key) else { return };
        (slot.path.clone(), slot.area_w.max(1), slot.area_h.max(1))
    };
    if path.is_empty() {
        on_ready_miss(h, gen);
        return;
    }
    if let Some(slot) = slots().get_mut(&key) {
        if slot.gen == gen {
            slot.kind = Kind::Opening;
        }
    }
    enqueue(Work { hwnd: key, gen, path, w, h: ht, skip_handler: true });
}

unsafe fn on_ready_miss(h: HWND, gen: u64) {
    let key = key_of(h);
    if let Some(slot) = slots().get_mut(&key) {
        if slot.gen == gen {
            slot.kind = Kind::Empty;
            slot.body = None;
            slot.big = None;
        }
    }
    drop_host(key);
    give_back(h);
    _ = InvalidateRect(Some(h), None, false);
}

fn copy_source(slot: &Slot) -> Option<String> {
    match &slot.body {
        Some(Body::Text { text, .. }) => {
            let lines = slot.scroll.layout.as_ref().map(|l| l.2.as_slice()).unwrap_or(&[]);
            Some(selected_text(text, lines, slot.sel))
        }
        Some(Body::Document { text, .. }) => Some(text.clone()),
        _ => None,
    }
}

unsafe fn copy_preview(h: HWND) {
    let text = slots().get(&key_of(h)).and_then(copy_source);
    let Some(text) = text else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if text.is_empty() || !crate::copy_plain(&text) {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

unsafe fn open_editor(path: &str) {
    if path.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let file = HSTRING::from(path);
    let edit = ShellExecuteW(None, w!("edit"), &file, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    if edit.0 as isize <= 32 {
        let open = ShellExecuteW(None, w!("open"), &file, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if open.0 as isize <= 32 {
            _ = MessageBeep(MB_ICONWARNING);
        }
    }
}

unsafe fn edit_preview(h: HWND) {
    let path = slots().get(&key_of(h)).map(|s| s.path.clone()).unwrap_or_default();
    open_editor(&path);
}

fn menu_point(h: HWND, lp: LPARAM) -> POINT {
    if lp.0 != -1 {
        return POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
    }
    let mut rc = RECT::default();
    if unsafe { GetWindowRect(h, &mut rc) }.is_ok() {
        POINT { x: (rc.left + rc.right) / 2, y: (rc.top + rc.bottom) / 2 }
    } else {
        let mut pt = POINT::default();
        _ = unsafe { GetCursorPos(&mut pt) };
        pt
    }
}

unsafe fn preview_menu(h: HWND, lp: LPARAM) {
    let (path, text) = {
        let map = slots();
        let Some(slot) = map.get(&key_of(h)) else { return };
        if !has_text(slot) {
            return;
        }
        (slot.path.clone(), copy_source(slot))
    };
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING, CMD_COPY, &HSTRING::from(tr(crate::Key::CopyText)));
    _ = AppendMenuW(menu, MF_STRING, CMD_EDIT, &HSTRING::from(tr(crate::Key::OpenInEditor)));
    let pt = menu_point(h, lp);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, h, None).0 as usize;
    _ = PostMessageW(Some(h), WM_NULL, WPARAM(0), LPARAM(0));
    _ = DestroyMenu(menu);
    match pick {
        CMD_COPY => {
            if text.as_ref().is_none_or(|t| t.is_empty()) || !crate::copy_plain(text.as_deref().unwrap_or("")) {
                _ = MessageBeep(MB_ICONWARNING);
            }
        }
        CMD_EDIT => open_editor(&path),
        _ => {}
    }
}

unsafe fn open_row(h: HWND, x: i32, y: i32) {
    let (dpi, fh) = app(|a| (a.dpi, a.fh));
    let jump = {
        let map = slots();
        let Some(slot) = map.get(&key_of(h)) else { return };
        if !slot.folder || in_rect(slot.hit.copy, x, y) || in_rect(slot.hit.edit, x, y) {
            return;
        }
        let Some(Body::List { rows, .. }) = slot.body.as_ref() else { return };
        let rh = row_h(fh, dpi);
        let Some(i) = list_row_at(slot.scroll.y, slot.hit.body.top, y, rh, rows.len()) else { return };
        let name = rows[i].0.clone();
        join_child(&slot.path, &name)
    };
    let Some(path) = jump else { return };
    let i = app(|a| a.s.active);
    if navigate(i, &path) {
        focus_view(i);
    } else {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

fn line_at(slot: &Slot, y: i32) -> Option<usize> {
    if !matches!(slot.body, Some(Body::Text { .. })) {
        return None;
    }
    if y < slot.hit.body.top || y >= slot.hit.body.bottom {
        return None;
    }
    let lines = &slot.scroll.layout.as_ref()?.2;
    line_no_at(lines, slot.scroll.y, slot.hit.body.top, y)
}

unsafe fn scroll_by(h: HWND, dy: i32, dx: i32) {
    if let Some(slot) = slots().get_mut(&key_of(h)) {
        slot.scroll.y = nudge(slot.scroll.y, slot.scroll.max_y, dy);
        slot.scroll.x = slot.scroll.x.saturating_add(dx).max(0);
    }
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn press_bar(h: HWND, x: i32, y: i32) -> bool {
    let key = key_of(h);
    let mut map = slots();
    let Some(slot) = map.get_mut(&key) else { return false };
    if !in_rect(slot.hit.bar, x, y) && !in_rect(slot.hit.thumb, x, y) {
        return false;
    }
    match bar_hit(slot.hit.bar, slot.hit.thumb, x, y) {
        BarHit::Thumb => {
            slot.bar_drag = Some(y - slot.hit.thumb.top);
            drop(map);
            _ = SetCapture(h);
            true
        }
        BarHit::Track => {
            let page = slot.scroll.page_h.max(1);
            slot.scroll.y = page_toward(slot.scroll.y, slot.scroll.max_y, slot.hit.thumb.top, slot.hit.thumb.bottom - slot.hit.thumb.top, y, page);
            drop(map);
            _ = InvalidateRect(Some(h), None, false);
            true
        }
        BarHit::None => false,
    }
}

unsafe fn drag_bar(h: HWND, y: i32) {
    let key = key_of(h);
    let mut map = slots();
    let Some(slot) = map.get_mut(&key) else { return };
    let Some(off) = slot.bar_drag else { return };
    let travel = (slot.hit.bar.bottom - slot.hit.bar.top - (slot.hit.thumb.bottom - slot.hit.thumb.top)).max(0);
    if travel <= 0 {
        return;
    }
    let top = (y - off).clamp(slot.hit.bar.top, slot.hit.bar.bottom - (slot.hit.thumb.bottom - slot.hit.thumb.top));
    slot.scroll.y = thumb_scroll(top - slot.hit.bar.top, travel, slot.scroll.max_y);
    drop(map);
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) extern "system" fn preview_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(AssertUnwindSafe(|| preview_proc_inner(h, msg, wp, lp)))
}

fn preview_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => paint_preview(h),
            WM_SIZE => sync_host(h),
            WM_MOUSEACTIVATE => {
                if let Some(slot) = slots().get_mut(&key_of(h)) {
                    slot.return_focus = false;
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lp_xy(lp);
                if press_at(h, x, y) {
                    return LRESULT(0);
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = lp_xy(lp);
                if slots().get(&key_of(h)).is_some_and(|s| s.bar_drag.is_some()) {
                    drag_bar(h, y);
                } else {
                    drag_sel(h, y);
                    hover_btn(h, x, y);
                }
            }
            WM_LBUTTONUP | WM_CAPTURECHANGED => {
                if let Some(slot) = slots().get_mut(&key_of(h)) {
                    slot.bar_drag = None;
                }
                if msg == WM_LBUTTONUP && GetCapture() == h {
                    _ = ReleaseCapture();
                }
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = lp_xy(lp);
                open_row(h, x, y);
            }
            WM_MOUSEWHEEL => {
                let raw = (wp.0 >> 16) as i16 as i32;
                let px = app(|a| -raw * a.fh * 3 / 120);
                let shift = (wp.0 & 0xffff) as u32 & MK_SHIFT.0 != 0;
                if shift { scroll_by(h, 0, -px) } else { scroll_by(h, px, 0) };
            }
            WM_KEYDOWN => {
                let k = VIRTUAL_KEY(wp.0 as u16);
                let ctrl = GetAsyncKeyState(VK_CONTROL.0 as i32) < 0;
                let page = slots().get(&key_of(h)).map(|s| s.scroll.page_h.max(1)).unwrap_or(1);
                let line = app(|a| a.fh.max(1) * 3);
                if ctrl && k == VIRTUAL_KEY(b'C' as u16) {
                    copy_preview(h);
                } else {
                    match k {
                        VK_NEXT => scroll_by(h, page, 0),
                        VK_PRIOR => scroll_by(h, -page, 0),
                        VK_HOME => scroll_by(h, i32::MIN, 0),
                        VK_END => scroll_by(h, i32::MAX, 0),
                        VK_DOWN => scroll_by(h, line, 0),
                        VK_UP => scroll_by(h, -line, 0),
                        VK_RIGHT => scroll_by(h, 0, line),
                        VK_LEFT => scroll_by(h, 0, -line),
                        _ => return DefWindowProcW(h, msg, wp, lp),
                    }
                }
            }
            WM_CONTEXTMENU => preview_menu(h, lp),
            WM_PREVIEW_READY => on_ready(h, wp.0 as u64),
            WM_PREVIEW_HOSTED => on_hosted(h, wp.0 as u64, lp.0 != 0),
            WM_PREVIEW_TOTAL => on_total(h, wp.0 as u64),
            WM_DESTROY => preview_clear(),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

fn lp_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

unsafe fn press_at(h: HWND, x: i32, y: i32) -> bool {
    if press_bar(h, x, y) {
        return true;
    }
    let act = {
        let map = slots();
        let Some(slot) = map.get(&key_of(h)) else { return false };
        if in_rect(slot.hit.copy, x, y) {
            Btn::Copy
        } else if in_rect(slot.hit.edit, x, y) {
            Btn::Edit
        } else if line_at(slot, y).is_some() {
            Btn::None
        } else {
            return false;
        }
    };
    match act {
        Btn::Copy => {
            copy_preview(h);
            true
        }
        Btn::Edit => {
            edit_preview(h);
            true
        }
        Btn::None => {
            if let Some(slot) = slots().get_mut(&key_of(h)) {
                if let Some(no) = line_at(slot, y) {
                    slot.anchor = no;
                    slot.sel = None;
                    drop(slot);
                    _ = SetCapture(h);
                    return true;
                }
            }
            false
        }
    }
}

unsafe fn drag_sel(h: HWND, y: i32) {
    if GetCapture() != h {
        return;
    }
    let changed = {
        let mut map = slots();
        let Some(slot) = map.get_mut(&key_of(h)) else { return };
        if slot.anchor == 0 || slot.bar_drag.is_some() {
            return;
        }
        let Some(no) = line_at(slot, y) else { return };
        let next = (no != slot.anchor).then_some((slot.anchor, no));
        let changed = next != slot.sel;
        slot.sel = next;
        changed
    };
    if changed {
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn hover_btn(h: HWND, x: i32, y: i32) {
    let next = {
        let map = slots();
        let Some(slot) = map.get(&key_of(h)) else { return };
        if in_rect(slot.hit.copy, x, y) {
            Btn::Copy
        } else if in_rect(slot.hit.edit, x, y) {
            Btn::Edit
        } else {
            Btn::None
        }
    };
    let changed = {
        let mut map = slots();
        let Some(slot) = map.get_mut(&key_of(h)) else { return };
        let changed = slot.hot != next;
        slot.hot = next;
        changed
    };
    if changed {
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn paint_preview(h: HWND) {
    let key = key_of(h);
    let (font, dpi, fh, accent, ilist) = app(|a| (a.font, a.dpi, a.fh, a.s.accent_now(), a.ilist.clone()));
    let mut sync = false;
    buffered(h, |dc, rc| {
        let mut map = slots();
        let Some(slot) = map.get_mut(&key) else {
            fill(dc, rc, pal().bg);
            text(dc, rc, tr(crate::Key::NoPreview), font, pal().sub, DT_CENTER);
            return;
        };
        let s = |v: i32| v * dpi / 96;
        let name = shown_name(slot);
        let (rows, exif, head, line, pad, icon) = band(&name, &slot.facts, rc.right - rc.left, rc.bottom - rc.top, dpi, fh);
        if head != slot.head_h {
            slot.head_h = head;
            sync = slot.kind == Kind::Handler && slot.host != 0;
        }
        fill(dc, rc, pal().bg);
        let bar = RECT { left: rc.left, top: rc.top, right: rc.right, bottom: (rc.top + head).min(rc.bottom) };
        fill(dc, bar, pal().card);
        if let Some(il) = &ilist {
            if slot.facts.icon >= 0 {
                if let Ok(ico) = il.GetIcon(slot.facts.icon, ILD_NORMAL.0) {
                    let y = bar.top + pad + (line - icon) / 2;
                    _ = DrawIconEx(dc, bar.left + pad, y, ico, icon, icon, 0, None, DI_NORMAL);
                    _ = DestroyIcon(ico);
                }
            }
        }
        let faces = PANE_FACES.with(|c| pview::fonts(&mut c.borrow_mut(), dpi, font));
        let text_left = bar.left + pad + icon + s(6);
        for (i, row) in rows.iter().enumerate() {
            let top = bar.top + pad + i as i32 * line;
            let r = RECT { left: text_left, top, right: bar.right - pad, bottom: top + line };
            let face = if i == 0 { faces[1] } else { font };
            let ink = if i == 0 { pal().text } else { pal().sub };
            text(dc, r, row, face, ink, DT_LEFT | DT_VCENTER);
        }
        if !exif.is_empty() {
            let top0 = bar.top + pad + rows.len() as i32 * line + s(2);
            for (i, row) in exif.iter().enumerate() {
                let top = top0 + i as i32 * line;
                let r = RECT { left: text_left, top, right: bar.right - pad, bottom: top + line };
                text(dc, r, row, font, accent, DT_LEFT | DT_VCENTER);
            }
        }
        fill(dc, RECT { top: bar.bottom.saturating_sub(1), bottom: bar.bottom, ..bar }, pal().line);

        let mut y = bar.bottom;
        slot.hit.copy = RECT::default();
        slot.hit.edit = RECT::default();
        if slot.kind == Kind::Shown && has_text(slot) {
            let bh = s(22).max(fh);
            let gap = s(6);
            let copy_lbl = tr(crate::Key::CopyText);
            let edit_lbl = tr(crate::Key::OpenInEditor);
            let copy_w = pview::text_w(dc, copy_lbl, font) + s(16);
            let edit_w = pview::text_w(dc, edit_lbl, font) + s(16);
            let row = RECT { left: rc.left + pad, top: y + s(4), right: rc.right - pad, bottom: y + s(4) + bh };
            let stack = copy_w + gap + edit_w > row.right - row.left;
            let (c, e) = if stack {
                let c = RECT { right: (row.left + copy_w).min(row.right), bottom: row.top + bh, ..row };
                let e = RECT { top: c.bottom + s(4), bottom: c.bottom + s(4) + bh, right: (row.left + edit_w).min(row.right), ..row };
                (c, e)
            } else {
                let c = RECT { right: row.left + copy_w, ..row };
                let e = RECT { left: c.right + gap, right: c.right + gap + edit_w, ..row };
                (c, e)
            };
            paint_btn(dc, c, copy_lbl, font, slot.hot == Btn::Copy, accent);
            paint_btn(dc, e, edit_lbl, font, slot.hot == Btn::Edit, accent);
            slot.hit.copy = c;
            slot.hit.edit = e;
            y = e.bottom + s(4);
        }
        if slot.kind == Kind::Shown && slot.folder {
            y = paint_hero(dc, rc, y, slot, font, faces[1], dpi, fh);
        }
        let gutter = s(12);
        let body = RECT { left: rc.left + pad, top: y, right: (rc.right - gutter).max(rc.left + pad), bottom: rc.bottom };
        slot.hit.body = body;
        match slot.kind {
            Kind::Handler => fill(dc, RECT { top: bar.bottom, ..rc }, pal().card),
            Kind::Opening => text(dc, body, tr(crate::Key::OpeningPreview), font, pal().sub, DT_CENTER),
            Kind::Empty | Kind::Idle => text(dc, body, tr(crate::Key::NoPreview), font, pal().sub, DT_CENTER),
            Kind::Shown => {
                let gen = slot.gen as usize;
                let sel = slot.sel;
                PANE_FACES.with(|c| {
                    let faces = &mut *c.borrow_mut();
                    match slot.body.as_ref() {
                    Some(Body::Text { text: t, kind, cut }) => {
                        pview::paint_text(&mut slot.scroll, dc, body, gen, t, *kind, *cut, font, dpi, None, sel, accent, faces);
                    }
                    Some(Body::Document { pic, text: t, cut }) => {
                        pview::paint_text(&mut slot.scroll, dc, body, gen, t, TextKind::Document, *cut, font, dpi, pic.as_ref(), None, 0, faces);
                    }
                    Some(Body::Table { rows, cut }) => pview::paint_table(&mut slot.scroll, dc, body, rows, *cut, font, dpi, faces),
                    Some(Body::List { rows, .. }) => pview::paint_list(&mut slot.scroll, dc, body, rows, font, dpi),
                    Some(Body::Pic(p)) => paint_fit(dc, body, p),
                    Some(Body::Pages { pages, total }) => paint_pages(&mut slot.scroll, dc, body, pages, *total, font, dpi),
                    Some(Body::Card { icon, rows }) => paint_card(dc, body, icon.as_ref(), rows, font, dpi),
                    Some(Body::Handler(_)) | None => text(dc, body, tr(crate::Key::NoPreview), font, pal().sub, DT_CENTER),
                    }
                });
                paint_bar(dc, rc, body, slot, dpi, accent);
            }
        }
    });
    if sync {
        sync_host(h);
    }
}

unsafe fn paint_btn(dc: HDC, r: RECT, label: &str, font: HFONT, hot: bool, accent: u32) {
    fill(dc, r, if hot { blend(pal().card, accent, 0.22) } else { pal().card });
    frame(dc, r, 1, if hot { accent } else { pal().line });
    text(dc, r, label, font, pal().text, DT_CENTER | DT_VCENTER);
}

unsafe fn paint_hero(dc: HDC, rc: RECT, top: i32, slot: &Slot, font: HFONT, bold: HFONT, dpi: i32, fh: i32) -> i32 {
    let s = |v: i32| v * dpi / 96;
    let pad = s(8);
    let side = s(48).max(32);
    let line = (fh + s(2)).max(s(16));
    let bottom = (top + side + pad).min(rc.bottom);
    let band = RECT { left: rc.left, top, right: rc.right, bottom };
    fill(dc, band, blend(pal().card, pal().bg, 0.35));
    if let Some(p) = &slot.big {
        let k = (side as f32 / p.w.max(1) as f32).min(side as f32 / p.h.max(1) as f32).min(1.0);
        let (w, h) = ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32);
        let x = rc.left + pad + (side - w) / 2;
        let y = top + (side - h) / 2;
        blit_bgra(dc, RECT { left: x, top: y, right: x + w, bottom: y + h }, p.w, p.h, &p.bgra);
    }
    let left = rc.left + pad + side + s(8);
    let name = shown_name(slot);
    text(dc, RECT { left, top, right: rc.right - pad, bottom: top + line }, &name, bold, pal().text, DT_LEFT | DT_VCENTER);
    let head = match &slot.body {
        Some(Body::List { head, .. }) => head.as_str(),
        _ => "",
    };
    text(dc, RECT { left, top: top + line, right: rc.right - pad, bottom: top + line * 2 }, head, font, pal().sub, DT_LEFT | DT_VCENTER);
    let total = total_line(slot.total);
    text(dc, RECT { left, top: top + line * 2, right: rc.right - pad, bottom: top + line * 3 }, &total, font, pal().text, DT_LEFT | DT_VCENTER);
    bottom.max(top + line * 3)
}

unsafe fn paint_fit(dc: HDC, area: RECT, p: &Pix) {
    let bw = (area.right - area.left).max(1);
    let bh = (area.bottom - area.top).max(1);
    let k = (bw as f32 / p.w.max(1) as f32).min(bh as f32 / p.h.max(1) as f32).min(1.0);
    let (w, h) = ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32);
    let x = area.left + (bw - w) / 2;
    let y = area.top + (bh - h) / 2;
    blit_bgra(dc, RECT { left: x, top: y, right: x + w, bottom: y + h }, p.w, p.h, &p.bgra);
}

unsafe fn paint_pages(scroll: &mut BodyScroll, dc: HDC, area: RECT, pages: &[Pix], total: u32, font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let gap = s(12);
    let width = (area.right - area.left).max(1);
    let mut boxes = Vec::with_capacity(pages.len());
    let mut y = s(8);
    for p in pages {
        let k = (width as f32 * 0.92 / p.w.max(1) as f32).min(1.0);
        let (w, h) = ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32);
        boxes.push((y, w.max(1), h.max(1)));
        y += h.max(1) + gap;
    }
    let more = (pages.len() as u32) < total;
    if more {
        y += s(28);
    }
    pview::clamp_scroll(scroll, y, (area.bottom - area.top).max(0));
    let sy = scroll.y;
    for (p, (top, w, h)) in pages.iter().zip(boxes.iter().copied()) {
        let top = area.top + top - sy;
        if top + h < area.top || top > area.bottom {
            continue;
        }
        let x = area.left + (width - w) / 2;
        blit_bgra(dc, RECT { left: x, top, right: x + w, bottom: top + h }, p.w, p.h, &p.bgra);
        frame(dc, RECT { left: x - 1, top: top - 1, right: x + w + 1, bottom: top + h + 1 }, 1, pal().line);
    }
    if more {
        let top = area.top + y - s(28) - sy;
        let label = format!("{} {} / {}", tr(crate::Key::RenderingPages), pages.len(), total);
        text(dc, RECT { left: area.left, top, right: area.right, bottom: top + s(24) }, &label, font, pal().sub, DT_CENTER);
    }
}

unsafe fn paint_bar(dc: HDC, rc: RECT, body: RECT, slot: &mut Slot, dpi: i32, accent: u32) {
    let s = |v: i32| v * dpi / 96;
    slot.hit.bar = RECT::default();
    slot.hit.thumb = RECT::default();
    let content = slot.scroll.max_y.saturating_add(slot.scroll.page_h);
    let view = slot.scroll.page_h;
    let track = RECT { left: rc.right - s(10), top: body.top + s(2), right: rc.right - s(3), bottom: body.bottom - s(2) };
    let span = track.bottom - track.top;
    let Some(lay) = bar_layout(content, view, track.top, span, s(24).max(8), slot.scroll.y) else { return };
    let thumb = RECT { left: track.left, top: lay.thumb_top, right: track.right, bottom: lay.thumb_top + lay.thumb_h };
    let (track_c, thumb_c) = bar_rgb(pal().bg, pal().text, accent, slot.bar_drag.is_some());
    fill(dc, track, track_c);
    fill(dc, thumb, thumb_c);
    slot.hit.bar = track;
    slot.hit.thumb = thumb;
}

pub(crate) extern "system" fn preview_host_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(AssertUnwindSafe(|| preview_host_proc_inner(h, msg, wp, lp)))
}

fn preview_host_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let font = app(|a| a.font);
                buffered(h, |dc, rc| {
                    fill(dc, rc, pal().card);
                    text(dc, rc, tr(crate::Key::OpeningPreview), font, pal().sub, DT_CENTER);
                });
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::cap_lines;

    #[test]
    fn header_caps_extra_lines() {
        let lines = vec!["name".into(), "type".into(), "size".into(), "date".into()];
        let out = cap_lines(lines, 2, 40);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], "name");
        assert!(out[1].contains("type") && out[1].contains("date"));
    }

    #[test]
    fn child_paths_reject_separators() {
        assert_eq!(super::join_child(r"C:\Work", "notes"), Some(r"C:\Work\notes".into()));
        assert_eq!(super::join_child(r"C:\Work\", "notes"), Some(r"C:\Work\notes".into()));
        assert_eq!(super::join_child(r"C:\Work", r"a\b"), None);
        assert_eq!(super::join_child(r"C:\Work", ".."), None);
    }
}
