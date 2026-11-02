//! Right-hand preview of the active pane's focused item.
//!
//! Windows preview handlers draw it, the same way Explorer's preview pane does.
//! A thumbnail is the fallback, then a short note. The handler is created on its
//! own STA thread and unloaded there. Arrow keys only arm a short timer, so the
//! file list never waits on a handler or a codec.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::{Condvar, LazyLock, Mutex, Once};

use crate::app::*;
use crate::i18n::*;
use crate::imports::*;
use crate::ui::*;
use super::imgjob::isolated_pic;
use super::peek::{host_handler, Pix};
use super::preview::{ext_of, handler_for, shell_pic, HEAVY_MAX};
use super::focus_view;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Shell::IPreviewHandler;

const PREVIEW_DELAY_MS: u32 = 200;
const WM_PREVIEW_READY: u32 = WM_APP + 70; // wParam = generation; the result sits in READY
const WM_PREVIEW_SIZE: u32 = WM_APP + 71; // thread message: the host window changed size
const WM_PREVIEW_HOSTED: u32 = WM_APP + 72; // wParam = generation, lParam = 1 shown / 0 failed

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Idle,
    Opening,
    Handler,
    Thumb,
    Empty,
}

struct Slot {
    gen: u64,
    path: String,
    title: String,
    kind: Kind,
    pix: Option<Pix>,
    host: isize, // HWND is not Send; the UI thread is the only one that creates or destroys it
    tid: u32,
    area_w: i32,
    area_h: i32,
    seq: u64,
    /// Focus was outside this pane when the load started. A handler that takes it is given back.
    /// A click in the pane clears the flag so the handler can keep the keyboard.
    return_focus: bool,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            gen: 0,
            path: String::new(),
            title: String::new(),
            kind: Kind::Idle,
            pix: None,
            host: 0,
            tid: 0,
            area_w: 0,
            area_h: 0,
            seq: 0,
            return_focus: false,
        }
    }
}

enum Ready {
    Handler(GUID),
    Thumb(Pix),
    Miss,
}

struct Work {
    hwnd: isize,
    gen: u64,
    path: String,
    w: i32,
    h: i32,
    thumb_only: bool,
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

static SLOTS: LazyLock<Mutex<HashMap<isize, Slot>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static JOBS: LazyLock<Mutex<HashMap<isize, Work>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static READY: LazyLock<Mutex<HashMap<isize, (u64, Ready)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static JOB_CV: Condvar = Condvar::new();
static WORKER: Once = Once::new();

fn key_of(h: HWND) -> isize {
    h.0 as isize
}

fn idle(slot: &Slot) -> bool {
    slot.kind == Kind::Idle && slot.host == 0 && slot.path.is_empty() && slot.pix.is_none() && slot.tid == 0
}

fn header_px() -> i32 {
    app(|a| (a.fh + 10 * a.dpi / 96).max(20))
}

unsafe fn body_rect(h: HWND) -> RECT {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let head = header_px().min((rc.bottom - rc.top).max(0));
    rc.top += head;
    rc
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
    let body = body_rect(h);
    let (bw, bh) = (body.right - body.left, body.bottom - body.top);
    // A zero-size plate must not be recorded as Opening: the next poll would then skip it.
    if bw < 8 || bh < 8 {
        return;
    }
    let (path, title) = focused_file(i);
    let key = key_of(h);
    if !force {
        let skip = slots().get(&key).is_some_and(|s| {
            s.path == path && matches!(s.kind, Kind::Opening | Kind::Handler | Kind::Thumb | Kind::Empty)
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
        slot.gen = slot.gen.wrapping_add(1).max(1);
        slot.path = path.clone();
        slot.title = title;
        slot.pix = None;
        slot.return_focus = !inside;
        slot.kind = if path.is_empty() { Kind::Empty } else { Kind::Opening };
        slot.gen
    };
    drop_host(key);
    if path.is_empty() {
        _ = InvalidateRect(Some(h), None, false);
        return;
    }
    enqueue(Work { hwnd: key, gen, path, w: bw, h: bh, thumb_only: false });
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
        slot.gen = slot.gen.wrapping_add(1).max(1);
        slot.kind = Kind::Idle;
        slot.path.clear();
        slot.title.clear();
        slot.pix = None;
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

/// Theme changed under a live handler. Drawn thumbnails pick up the palette on the next paint.
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
    let meta = std::fs::metadata(&job.path).ok();
    let Some(meta) = meta else { return Ready::Miss };
    if meta.is_dir() {
        return dir_thumb(job);
    }
    if !job.thumb_only && meta.len() > 0 && meta.len() <= HEAVY_MAX {
        let ext = ext_of(&job.path);
        if let Some(g) = handler_for(&ext) {
            return Ready::Handler(g);
        }
    }
    match isolated_pic(&job.path, job.w.max(1), job.h.max(1), (SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY).0 as u32) {
        Ok(p) => Ready::Thumb(p),
        Err(_) => Ready::Miss,
    }
}

fn dir_thumb(job: &Work) -> Ready {
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let pic = shell_pic(&job.path, job.w.max(1), job.h.max(1), SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY);
        if com {
            CoUninitialize();
        }
        pic.map(Ready::Thumb).unwrap_or(Ready::Miss)
    }
}

fn take_ready(hwnd: isize, gen: u64) -> Option<Ready> {
    let mut box_ = ready_box();
    match box_.get(&hwnd) {
        Some((g, _)) if *g == gen => box_.remove(&hwnd).map(|(_, r)| r),
        _ => None,
    }
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

unsafe fn on_ready(h: HWND, gen: u64) {
    let key = key_of(h);
    if !current(key, gen) {
        return;
    }
    let Some(ready) = take_ready(key, gen) else { return };
    match ready {
        Ready::Handler(clsid) => {
            let path = slots().get(&key).map(|s| s.path.clone()).unwrap_or_default();
            if path.is_empty() || !current(key, gen) {
                return;
            }
            start_host(h, gen, clsid, path);
        }
        Ready::Thumb(pix) => {
            if let Some(slot) = slots().get_mut(&key) {
                if slot.gen == gen {
                    slot.kind = Kind::Thumb;
                    slot.pix = Some(pix);
                }
            }
            drop_host(key);
            give_back(h);
            _ = InvalidateRect(Some(h), None, false);
        }
        Ready::Miss => {
            if let Some(slot) = slots().get_mut(&key) {
                if slot.gen == gen {
                    slot.kind = Kind::Empty;
                    slot.pix = None;
                }
            }
            drop_host(key);
            give_back(h);
            _ = InvalidateRect(Some(h), None, false);
        }
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
        enqueue(Work { hwnd: key, gen, path, w, h: ht, thumb_only: true });
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
        enqueue(Work { hwnd: key, gen, path: again, w, h: ht, thumb_only: true });
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
    enqueue(Work { hwnd: key, gen, path, w, h: ht, thumb_only: true });
}

unsafe fn on_ready_miss(h: HWND, gen: u64) {
    let key = key_of(h);
    if let Some(slot) = slots().get_mut(&key) {
        if slot.gen == gen {
            slot.kind = Kind::Empty;
            slot.pix = None;
        }
    }
    give_back(h);
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
            WM_PREVIEW_READY => on_ready(h, wp.0 as u64),
            WM_PREVIEW_HOSTED => on_hosted(h, wp.0 as u64, lp.0 != 0),
            WM_DESTROY => preview_clear(),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

unsafe fn paint_preview(h: HWND) {
    let key = key_of(h);
    let (title, kind, pix) = {
        let map = slots();
        match map.get(&key) {
            Some(s) => {
                let pix = s.pix.as_ref().map(|p| Pix::new(p.w, p.h, p.bgra.clone()));
                (s.title.clone(), s.kind, pix)
            }
            None => (String::new(), Kind::Idle, None),
        }
    };
    let font = app(|a| a.font);
    let head = header_px();
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().bg);
        let bar = RECT { left: rc.left, top: rc.top, right: rc.right, bottom: (rc.top + head).min(rc.bottom) };
        fill(dc, bar, pal().card);
        fill(dc, RECT { top: bar.bottom.saturating_sub(1), bottom: bar.bottom, ..bar }, pal().line);
        let mut label = bar;
        let pad = 8 * app(|a| a.dpi) / 96;
        label.left += pad;
        label.right -= pad;
        if !title.is_empty() {
            text(dc, label, &title, font, pal().text, DT_LEFT);
        }
        let body = RECT { top: bar.bottom, ..rc };
        match kind {
            Kind::Handler => fill(dc, body, pal().card),
            Kind::Thumb => {
                fill(dc, body, pal().bg);
                if let Some(pix) = pix.as_ref() {
                    let bw = (body.right - body.left).max(0);
                    let bh = (body.bottom - body.top).max(0);
                    let x = body.left + (bw - pix.w).max(0) / 2;
                    let y = body.top + (bh - pix.h).max(0) / 2;
                    blit_bgra(dc, RECT { left: x, top: y, right: x + pix.w, bottom: y + pix.h }, pix.w, pix.h, &pix.bgra);
                }
            }
            Kind::Opening => {
                fill(dc, body, pal().bg);
                text(dc, body, tr(crate::Key::OpeningPreview), font, pal().sub, DT_CENTER);
            }
            Kind::Empty | Kind::Idle => {
                fill(dc, body, pal().bg);
                text(dc, body, tr(crate::Key::NoPreview), font, pal().sub, DT_CENTER);
            }
        }
    });
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
