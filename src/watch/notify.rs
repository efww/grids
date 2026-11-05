// A burst is told with a shell balloon (`IUserNotification2`).
// WinRT toast needs an AppUserModelID. A portable exe does not have one, and registering
// an activator would write the registry. Maguro never writes the registry, so the toast
// route stays closed. If the balloon cannot be created, a small cerulean window is the
// click target instead. Neither path adds a tray icon.

use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::{Arc, OnceLock};

use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::core::{Interface, GUID, HRESULT, PCWSTR};

use super::model::{notify_route, NotifyRoute};
use super::{queue_open, MAIN};
use crate::imports::*;

static TOAST_GEN: AtomicU64 = AtomicU64::new(0);

struct Pending {
    gen: u64,
    title: String,
    body: String,
    detail: String,
    path: String,
    file: bool,
}

fn pending() -> &'static Mutex<Option<Pending>> {
    static PENDING: OnceLock<Mutex<Option<Pending>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(None))
}

pub(crate) fn show(path: &str, title: &str, body: &str, detail: &str, file: bool) {
    debug_assert_eq!(notify_route(false), NotifyRoute::Balloon);
    let gen = TOAST_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let path = path.to_string();
    let title = title.to_string();
    let body = body.to_string();
    let detail = detail.to_string();
    let balloon = if detail.is_empty() { body.clone() } else { format!("{body}\n{detail}") };
    std::thread::spawn(move || {
        let ok = unsafe { try_balloon(gen, &title, &balloon, &path) };
        if ok || TOAST_GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        if let Ok(mut slot) = pending().lock() {
            *slot = Some(Pending { gen, title, body, detail, path, file });
        }
        let bits = MAIN.load(Ordering::SeqCst);
        if bits != 0 {
            let h = HWND(bits as *mut core::ffi::c_void);
            unsafe { let _ = PostMessageW(Some(h), crate::WM_WATCH_TOAST, WPARAM(0), LPARAM(0)); }
        }
    });
}

pub(crate) unsafe fn present_fallback() {
    let pending = pending().lock().ok().and_then(|mut g| g.take());
    let Some(p) = pending else { return };
    if TOAST_GEN.load(Ordering::SeqCst) != p.gen {
        return;
    }
    show_window(&p.title, &p.body, &p.detail, &p.path, p.file);
}

/// Shell balloon. `false` when the COM object is missing, so the caller draws its own.
unsafe fn try_balloon(gen: u64, title: &str, body: &str, path: &str) -> bool {
    let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    let started = hr.is_ok();
    let ok = balloon_show(gen, title, body, path);
    if started {
        CoUninitialize();
    }
    ok
}

unsafe fn balloon_show(gen: u64, title: &str, body: &str, path: &str) -> bool {
    let clsid = GUID::from_u128(0x0010890e_8789_413c_adbc_48f5b511b3af);
    let iid = GUID::from_u128(0x215913cc_57eb_4fab_ab5a_e5fa7bea2a6c);
    let Ok(unk) = CoCreateInstance::<_, IUnknown>(&clsid, None, CLSCTX_INPROC_SERVER) else { return false };
    let raw = Interface::as_raw(&unk);
    let base = *(raw as *mut *mut IUnkVtbl);
    let mut got = std::ptr::null_mut();
    let hr = ((*base).qi)(raw, &iid, &mut got);
    if !hr.is_ok() || got.is_null() {
        return false;
    }
    let vtbl = *(got as *mut *mut BalloonVtbl);
    let title_w = wide(title);
    let body_w = wide(body);
    let flags = 0x91u32; // NIIF_INFO | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME
    if !((*vtbl).set_info)(got, PCWSTR(title_w.as_ptr()), PCWSTR(body_w.as_ptr()), flags).is_ok() {
        release(got);
        return false;
    }
    let _ = ((*vtbl).set_retry)(got, 8000, 0, 1);
    // No custom icon: a null icon does not plant a tray button. The info glyph is the shell's.
    let _ = ((*vtbl).set_icon)(got, HICON(std::ptr::null_mut()), PCWSTR::null());
    let cancel = Arc::new(AtomicBool::new(false));
    let qc = QuerySink::new(gen, cancel.clone());
    let cb = ClickSink::new(path, cancel);
    let show_hr = ((*vtbl).show)(got, qc.as_ptr(), 200, cb.as_ptr());
    release(got);
    qc.release_ours();
    cb.release_ours();
    show_hr.is_ok()
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn release(p: *mut core::ffi::c_void) {
    if p.is_null() {
        return;
    }
    let vtbl = *(p as *mut *mut IUnkVtbl);
    ((*vtbl).release)(p);
}

#[repr(C)]
struct IUnkVtbl {
    qi: unsafe extern "system" fn(*mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT,
    addref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
}

#[repr(C)]
struct BalloonVtbl {
    qi: unsafe extern "system" fn(*mut core::ffi::c_void, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT,
    addref: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32,
    set_info: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR, PCWSTR, u32) -> HRESULT,
    set_retry: unsafe extern "system" fn(*mut core::ffi::c_void, u32, u32, u32) -> HRESULT,
    set_icon: unsafe extern "system" fn(*mut core::ffi::c_void, HICON, PCWSTR) -> HRESULT,
    show: unsafe extern "system" fn(*mut core::ffi::c_void, *mut core::ffi::c_void, u32, *mut core::ffi::c_void) -> HRESULT,
    play: unsafe extern "system" fn(*mut core::ffi::c_void, PCWSTR) -> HRESULT,
}

struct QuerySink {
    ptr: *mut QueryObj,
}

struct ClickSink {
    ptr: *mut ClickObj,
}

#[repr(C)]
struct QueryObj {
    vtbl: *const QueryVtbl,
    refc: AtomicU32,
    gen: u64,
    cancel: Arc<AtomicBool>,
}

#[repr(C)]
struct ClickObj {
    vtbl: *const ClickVtbl,
    refc: AtomicU32,
    path: String,
    cancel: Arc<AtomicBool>,
}

#[repr(C)]
struct QueryVtbl {
    qi: unsafe extern "system" fn(*mut QueryObj, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT,
    addref: unsafe extern "system" fn(*mut QueryObj) -> u32,
    release: unsafe extern "system" fn(*mut QueryObj) -> u32,
    query: unsafe extern "system" fn(*mut QueryObj) -> HRESULT,
}

#[repr(C)]
struct ClickVtbl {
    qi: unsafe extern "system" fn(*mut ClickObj, *const GUID, *mut *mut core::ffi::c_void) -> HRESULT,
    addref: unsafe extern "system" fn(*mut ClickObj) -> u32,
    release: unsafe extern "system" fn(*mut ClickObj) -> u32,
    balloon: unsafe extern "system" fn(*mut ClickObj, *const POINT) -> HRESULT,
    left: unsafe extern "system" fn(*mut ClickObj, *const POINT) -> HRESULT,
    menu: unsafe extern "system" fn(*mut ClickObj, *const POINT) -> HRESULT,
}

fn iid_unknown() -> GUID {
    GUID::from_u128(0x00000000_0000_0000_c000_000000000046)
}
fn iid_query() -> GUID {
    GUID::from_u128(0x7307055c_b24a_486b_9f25_163e597a28a9)
}
fn iid_click() -> GUID {
    GUID::from_u128(0x19108294_0441_4aff_8013_fa0a730b0bea)
}

fn same_guid(a: *const GUID, b: GUID) -> bool {
    !a.is_null() && unsafe { *a == b }
}

impl QuerySink {
    fn new(gen: u64, cancel: Arc<AtomicBool>) -> Self {
        let obj = Box::new(QueryObj { vtbl: &QUERY_VTBL, refc: AtomicU32::new(1), gen, cancel });
        QuerySink { ptr: Box::into_raw(obj) }
    }
    fn as_ptr(&self) -> *mut core::ffi::c_void {
        self.ptr as *mut _
    }
    fn release_ours(self) {
        unsafe { query_release(self.ptr) };
        std::mem::forget(self);
    }
}

impl Drop for QuerySink {
    fn drop(&mut self) {
        unsafe { query_release(self.ptr) };
    }
}

impl ClickSink {
    fn new(path: &str, cancel: Arc<AtomicBool>) -> Self {
        let obj = Box::new(ClickObj { vtbl: &CLICK_VTBL, refc: AtomicU32::new(1), path: path.to_string(), cancel });
        ClickSink { ptr: Box::into_raw(obj) }
    }
    fn as_ptr(&self) -> *mut core::ffi::c_void {
        self.ptr as *mut _
    }
    fn release_ours(self) {
        unsafe { click_release(self.ptr) };
        std::mem::forget(self);
    }
}

impl Drop for ClickSink {
    fn drop(&mut self) {
        unsafe { click_release(self.ptr) };
    }
}

static QUERY_VTBL: QueryVtbl = QueryVtbl { qi: query_qi, addref: query_add, release: query_release, query: query_poll };
static CLICK_VTBL: ClickVtbl = ClickVtbl { qi: click_qi, addref: click_add, release: click_release, balloon: click_balloon, left: click_left, menu: click_menu };

unsafe extern "system" fn query_qi(this: *mut QueryObj, iid: *const GUID, out: *mut *mut core::ffi::c_void) -> HRESULT {
    if out.is_null() {
        return HRESULT(-2147024809); // E_POINTER
    }
    *out = std::ptr::null_mut();
    if same_guid(iid, iid_unknown()) || same_guid(iid, iid_query()) {
        query_add(this);
        *out = this as *mut _;
        HRESULT(0)
    } else {
        HRESULT(-2147467262) // E_NOINTERFACE
    }
}

unsafe extern "system" fn query_add(this: *mut QueryObj) -> u32 {
    (*this).refc.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn query_release(this: *mut QueryObj) -> u32 {
    let n = (*this).refc.fetch_sub(1, Ordering::SeqCst) - 1;
    if n == 0 {
        drop(Box::from_raw(this));
    }
    n
}

unsafe extern "system" fn query_poll(this: *mut QueryObj) -> HRESULT {
    let stop = (*this).cancel.load(Ordering::SeqCst) || (*this).gen != TOAST_GEN.load(Ordering::SeqCst);
    HRESULT(if stop { 1 } else { 0 })
}

unsafe extern "system" fn click_qi(this: *mut ClickObj, iid: *const GUID, out: *mut *mut core::ffi::c_void) -> HRESULT {
    if out.is_null() {
        return HRESULT(-2147024809);
    }
    *out = std::ptr::null_mut();
    if same_guid(iid, iid_unknown()) || same_guid(iid, iid_click()) {
        click_add(this);
        *out = this as *mut _;
        HRESULT(0)
    } else {
        HRESULT(-2147467262)
    }
}

unsafe extern "system" fn click_add(this: *mut ClickObj) -> u32 {
    (*this).refc.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn click_release(this: *mut ClickObj) -> u32 {
    let n = (*this).refc.fetch_sub(1, Ordering::SeqCst) - 1;
    if n == 0 {
        drop(Box::from_raw(this));
    }
    n
}

unsafe extern "system" fn click_balloon(this: *mut ClickObj, _: *const POINT) -> HRESULT {
    (*this).cancel.store(true, Ordering::SeqCst);
    queue_open((*this).path.clone());
    HRESULT(1)
}

unsafe extern "system" fn click_left(this: *mut ClickObj, pt: *const POINT) -> HRESULT {
    click_balloon(this, pt)
}

unsafe extern "system" fn click_menu(_: *mut ClickObj, _: *const POINT) -> HRESULT {
    HRESULT(0)
}

// ---- in-app fallback, used only when the balloon object cannot be shown ----

const TOAST_CLOSE: u8 = 1;
const TOAST_OPEN: u8 = 2;

struct ToastLay {
    icon: RECT,
    title: RECT,
    body: RECT,
    detail: RECT,
    hint: RECT,
    close: RECT,
    open: RECT,
}

struct ToastUi {
    hwnd: HWND,
    title: String,
    body: String,
    detail: String,
    path: String,
    file: bool,
    dpi: i32,
    track: crate::Tracker<u8>,
    body_down: bool,
}

thread_local! {
    static TOAST: RefCell<ToastUi> = RefCell::new(ToastUi {
        hwnd: HWND::default(),
        title: String::new(),
        body: String::new(),
        detail: String::new(),
        path: String::new(),
        file: false,
        dpi: 96,
        track: crate::Tracker::new(),
        body_down: false,
    });
}

const TOAST_TIMER: usize = 1;

/// 360×121 at 96 DPI. The accent bar is paint only and is not part of this height.
fn toast_size(dpi: i32) -> (i32, i32) {
    let s = |v: i32| crate::px(dpi, v);
    let h = crate::PAD_BODY_Y + 16 + 16 + 16 + crate::SPACE_8 + 1 + crate::SPACE_8 + crate::BTN_H_FOOTER + crate::PAD_BODY_Y;
    (s(360), s(h))
}

fn toast_layout(w: i32, h: i32, dpi: i32) -> ToastLay {
    let s = |v: i32| crate::px(dpi, v);
    let pad_x = s(crate::PAD_BODY_X);
    let pad_y = s(crate::PAD_BODY_Y);
    let line = s(16);
    let icon_sz = s(28);
    let icon = RECT { left: pad_x, top: pad_y, right: pad_x + icon_sz, bottom: pad_y + icon_sz };
    let text_l = icon.right + s(crate::SPACE_8);
    let text_r = (w - pad_x).max(text_l);
    let title = RECT { left: text_l, top: pad_y, right: text_r, bottom: pad_y + line };
    let body = RECT { left: text_l, top: title.bottom, right: text_r, bottom: title.bottom + line };
    let detail = RECT { left: text_l, top: body.bottom, right: text_r, bottom: body.bottom + line };
    let btn_h = s(crate::BTN_H_FOOTER);
    let btn_w = s(crate::BTN_MIN_W);
    let gap = s(crate::BTN_GAP);
    let btn_top = (h - pad_y - btn_h).max(0);
    let open_r = (w - pad_x).max(0);
    let open = RECT { left: (open_r - btn_w).max(0), top: btn_top, right: open_r, bottom: (btn_top + btn_h).min(h.max(btn_top)) };
    let close = RECT { left: (open.left - gap - btn_w).max(0), top: btn_top, right: (open.left - gap).max(0), bottom: open.bottom };
    let hint = RECT { left: pad_x.min(close.left), top: btn_top, right: (close.left - gap).max(pad_x.min(close.left)), bottom: open.bottom };
    ToastLay { icon, title, body, detail, hint, close, open }
}

fn hit_rect(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn toast_client(h: HWND) -> (i32, i32) {
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    (rc.right.max(0), rc.bottom.max(0))
}

fn point_in_client(h: HWND, x: i32, y: i32) -> bool {
    let (w, ht) = toast_client(h);
    x >= 0 && y >= 0 && x < w && y < ht
}

fn toast_hit(h: HWND, x: i32, y: i32) -> Option<u8> {
    let (w, ht) = toast_client(h);
    let dpi = TOAST.with(|t| t.borrow().dpi.max(96));
    let lay = toast_layout(w, ht, dpi);
    if hit_rect(lay.close, x, y) {
        Some(TOAST_CLOSE)
    } else if hit_rect(lay.open, x, y) {
        Some(TOAST_OPEN)
    } else {
        None
    }
}

unsafe fn track_leave(h: HWND) {
    let mut t = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: h,
        dwHoverTime: 0,
    };
    let _ = TrackMouseEvent(&mut t);
}

unsafe fn show_window(title: &str, body: &str, detail: &str, path: &str, file: bool) {
    ensure_toast_class();
    let existing = TOAST.with(|t| t.borrow().hwnd);
    if !existing.0.is_null() && IsWindow(Some(existing)).as_bool() {
        TOAST.with(|t| {
            let mut ui = t.borrow_mut();
            ui.title = title.to_string();
            ui.body = body.to_string();
            ui.detail = detail.to_string();
            ui.path = path.to_string();
            ui.file = file;
            ui.track = crate::Tracker::new();
            ui.body_down = false;
        });
        let _ = SetTimer(Some(existing), TOAST_TIMER, 6000, None);
        let _ = InvalidateRect(Some(existing), None, false);
        let _ = ShowWindow(existing, SW_SHOWNOACTIVATE);
        return;
    }
    let dpi = crate::app(|a| a.dpi.max(96));
    let (ww, hh) = toast_size(dpi);
    let (x, y) = toast_origin(ww, hh);
    TOAST.with(|t| {
        let mut ui = t.borrow_mut();
        ui.title = title.to_string();
        ui.body = body.to_string();
        ui.detail = detail.to_string();
        ui.path = path.to_string();
        ui.file = file;
        ui.dpi = dpi;
        ui.track = crate::Tracker::new();
        ui.body_down = false;
    });
    let style = WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN;
    let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
    let Ok(h) = CreateWindowExW(ex, w!("MaguroWatchToast"), w!("Maguro"), style, x, y, ww, hh, None, None, None, None) else {
        return;
    };
    TOAST.with(|t| {
        let mut ui = t.borrow_mut();
        ui.hwnd = h;
        crate::note_side(h);
    });
    let _ = SetTimer(Some(h), TOAST_TIMER, 6000, None);
}

unsafe fn toast_origin(ww: i32, hh: i32) -> (i32, i32) {
    let mut rc = RECT::default();
    let ok = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut rc as *mut RECT as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    if ok.is_err() {
        rc.right = GetSystemMetrics(SM_CXSCREEN);
        rc.bottom = GetSystemMetrics(SM_CYSCREEN);
    }
    let margin = 16;
    ((rc.right - ww - margin).max(rc.left), (rc.bottom - hh - margin).max(rc.top))
}

fn ensure_toast_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(toast_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroWatchToast"),
            ..Default::default()
        });
    });
}

unsafe extern "system" fn toast_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            crate::buffered(h, |dc, rc| paint_toast(h, dc, rc));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = crate::xy(lp);
            let over = toast_hit(h, x, y);
            let effect = TOAST.with(|t| {
                let mut ui = t.borrow_mut();
                if ui.body_down { crate::TrackEffect::None } else { ui.track.pointer_move(over) }
            });
            track_leave(h);
            if matches!(effect, crate::TrackEffect::Repaint) {
                let _ = InvalidateRect(Some(h), None, false);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            let effect = TOAST.with(|t| t.borrow_mut().track.pointer_leave());
            if matches!(effect, crate::TrackEffect::Repaint) {
                let _ = InvalidateRect(Some(h), None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = crate::xy(lp);
            let over = toast_hit(h, x, y);
            TOAST.with(|t| {
                let mut ui = t.borrow_mut();
                if let Some(id) = over {
                    ui.body_down = false;
                    ui.track.pointer_down(id);
                } else {
                    ui.body_down = true;
                }
            });
            let _ = SetCapture(h);
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = crate::xy(lp);
            let inside = point_in_client(h, x, y);
            let over = if inside { toast_hit(h, x, y) } else { None };
            let (effect, body, path) = TOAST.with(|t| {
                let mut ui = t.borrow_mut();
                let body = std::mem::take(&mut ui.body_down);
                let effect = ui.track.pointer_up(over);
                (effect, body, ui.path.clone())
            });
            let _ = ReleaseCapture();
            match effect {
                crate::TrackEffect::Click(TOAST_CLOSE) => {
                    let _ = DestroyWindow(h);
                }
                crate::TrackEffect::Click(TOAST_OPEN) => {
                    let _ = DestroyWindow(h);
                    if !path.is_empty() {
                        queue_open(path);
                    }
                }
                crate::TrackEffect::None if body && over.is_none() && inside => {
                    let _ = DestroyWindow(h);
                    if !path.is_empty() {
                        queue_open(path);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            TOAST.with(|t| {
                let mut ui = t.borrow_mut();
                ui.body_down = false;
                ui.track.capture_lost();
            });
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TOAST_TIMER => {
            let _ = DestroyWindow(h);
            LRESULT(0)
        }
        crate::WM_FAV_ICON => {
            crate::clear_icon_refresh();
            let _ = InvalidateRect(Some(h), None, false);
            LRESULT(0)
        }
        WM_DESTROY => {
            crate::drop_side(h);
            TOAST.with(|t| {
                let mut ui = t.borrow_mut();
                if ui.hwnd == h {
                    ui.hwnd = HWND::default();
                    ui.track = crate::Tracker::new();
                    ui.body_down = false;
                }
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

/// Hint and detail. The app meta face, or the body face before it exists.
unsafe fn meta_font() -> HFONT {
    if let Some(font) = crate::app_try(|a| a.small_font) {
        if !font.0.is_null() && !font.is_invalid() {
            return font;
        }
    }
    crate::body_font()
}

unsafe fn paint_toast(h: HWND, dc: HDC, rc: RECT) {
    let (title, body, detail, path, file, dpi, close_vis, open_vis) = TOAST.with(|t| {
        let ui = t.borrow();
        (
            ui.title.clone(),
            ui.body.clone(),
            ui.detail.clone(),
            ui.path.clone(),
            ui.file,
            ui.dpi.max(96),
            ui.track.visual(TOAST_CLOSE, false),
            ui.track.visual(TOAST_OPEN, false),
        )
    });
    // A theme change redraws this popup. The caption colour has to follow that paint.
    crate::style_dialog_caption(h);
    let font = crate::body_font();
    let bold = crate::bold_ui_font();
    let small = meta_font();
    let t = crate::ui_tokens();
    let lay = toast_layout((rc.right - rc.left).max(0), (rc.bottom - rc.top).max(0), dpi);
    crate::paint_window(dc, rc);
    crate::fill(dc, RECT { bottom: rc.top + crate::px(dpi, 3).max(2), ..rc }, crate::ACCENT);
    crate::ui::paint_card(dc, lay.icon, dpi);
    if let Some((list, idx, dim)) = crate::shell_icon(&path, file) {
        let icon_px = crate::px(dpi, 16);
        let inset = ((lay.icon.right - lay.icon.left) - icon_px) / 2;
        crate::draw_fav_icon(dc, list, idx, lay.icon.left + inset, lay.icon.top + inset, icon_px, dim);
    }
    crate::paint_label(dc, lay.title, &title, bold);
    crate::paint_meta(dc, lay.body, &body, font);
    crate::paint_label(dc, lay.detail, &detail, small);
    let rule = lay.detail.bottom + crate::px(dpi, crate::SPACE_8);
    crate::fill(dc, RECT { left: rc.left, top: rule, right: rc.right, bottom: rule + 1 }, t.line);
    crate::paint_meta(dc, lay.hint, crate::tr(crate::Key::WatchToastHint), small);
    let ground = t.bg;
    crate::paint_button(dc, lay.close, crate::ButtonKind::Secondary, close_vis, crate::tr(crate::Key::Close), font, dpi, ground, crate::FocusRing::Outside);
    crate::paint_button(dc, lay.open, crate::ButtonKind::Primary, open_vis, crate::tr(crate::Key::Open), font, dpi, ground, crate::FocusRing::Outside);
    crate::frame(dc, rc, 1, t.line);
}

pub(crate) unsafe fn dismiss() {
    let h = TOAST.with(|t| t.borrow().hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = DestroyWindow(h);
    }
}

#[cfg(test)]
mod tests {
    use super::{toast_layout, toast_size};

    #[test]
    fn toast_buttons_use_footer_metrics_and_cerulean() {
        let (w, h) = toast_size(96);
        assert_eq!((w, h), (360, 121));
        let lay = toast_layout(w, h, 96);
        assert_eq!(lay.open.bottom - lay.open.top, crate::BTN_H_FOOTER);
        assert_eq!(lay.open.right - lay.open.left, crate::BTN_MIN_W);
        assert_eq!(lay.close.right - lay.close.left, crate::BTN_MIN_W);
        assert!(lay.close.right <= lay.open.left);
        assert!(lay.detail.bottom <= lay.hint.top);
        assert!(lay.hint.right <= lay.close.left);
        assert_eq!((lay.icon.left, lay.icon.top, lay.icon.right, lay.icon.bottom), (16, 14, 44, 42));
        assert_eq!(lay.open.left, 272);
        assert_eq!(lay.open.top, 79);
        assert_eq!(lay.close.left, 192);
        assert_eq!((lay.hint.left, lay.hint.top, lay.hint.right, lay.hint.bottom), (16, 79, 184, 107));
        assert_eq!(lay.detail.bottom, 62);
        let fill = crate::button_colors(crate::ButtonKind::Primary, crate::ButtonState::Normal, false, crate::BG_LIGHT).fill;
        assert_eq!(fill, Some(crate::ACCENT));
        assert_ne!(fill, Some(0x2EC4A6));
    }
}
