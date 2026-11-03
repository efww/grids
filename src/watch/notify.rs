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

use super::model::{notify_route, NotifyRoute, CHROME};
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

struct ToastUi {
    hwnd: HWND,
    title: String,
    body: String,
    detail: String,
    path: String,
    file: bool,
    close: RECT,
    open: RECT,
    font: HFONT,
    bold: HFONT,
    small: HFONT,
}

thread_local! {
    static TOAST: RefCell<ToastUi> = RefCell::new(ToastUi {
        hwnd: HWND::default(),
        title: String::new(),
        body: String::new(),
        detail: String::new(),
        path: String::new(),
        file: false,
        close: RECT::default(),
        open: RECT::default(),
        font: HFONT::default(),
        bold: HFONT::default(),
        small: HFONT::default(),
    });
}

const TOAST_TIMER: usize = 1;

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
        });
        let _ = SetTimer(Some(existing), TOAST_TIMER, 6000, None);
        let _ = InvalidateRect(Some(existing), None, false);
        let _ = ShowWindow(existing, SW_SHOWNOACTIVATE);
        return;
    }
    let dpi = crate::app(|a| a.dpi.max(96));
    let (ww, hh) = (320 * dpi / 96, 118 * dpi / 96);
    let (x, y) = toast_origin(ww, hh);
    let style = WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN;
    let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE;
    let Ok(h) = CreateWindowExW(ex, w!("MaguroWatchToast"), w!("Maguro"), style, x, y, ww, hh, None, None, None, None) else { return };
    TOAST.with(|t| {
        let mut ui = t.borrow_mut();
        ui.hwnd = h;
        ui.title = title.to_string();
        ui.body = body.to_string();
        ui.detail = detail.to_string();
        ui.path = path.to_string();
        ui.file = file;
        rebuild_toast_fonts(&mut ui, dpi);
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

unsafe fn rebuild_toast_fonts(ui: &mut ToastUi, dpi: i32) {
    drop_font(&mut ui.font);
    drop_font(&mut ui.bold);
    drop_font(&mut ui.small);
    let face = toast_face();
    let (title, weight) = crate::title_style();
    ui.bold = make_font(title, 12, weight, dpi);
    ui.font = make_font(&face, 12, 400, dpi);
    ui.small = make_font(&face, 11, 400, dpi);
}

fn toast_face() -> String {
    let custom = crate::app(|a| a.s.font_face.clone());
    if custom.is_empty() { crate::ui_face().to_string() } else { custom }
}

fn drop_font(font: &mut HFONT) {
    unsafe { crate::delete_font(*font) };
    *font = HFONT::default();
}

fn make_font(face: &str, px: i32, weight: i32, dpi: i32) -> HFONT {
    let mut lf = LOGFONTW {
        lfHeight: -(px * dpi / 96).max(1),
        lfWeight: weight,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    unsafe { CreateFontIndirectW(&lf) }
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
            crate::buffered(h, |dc, rc| paint_toast(dc, rc));
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = crate::xy(lp);
            let close = TOAST.with(|t| t.borrow().close);
            if x >= close.left && x < close.right && y >= close.top && y < close.bottom {
                let _ = DestroyWindow(h);
            } else {
                let path = TOAST.with(|t| t.borrow().path.clone());
                let _ = DestroyWindow(h);
                if !path.is_empty() {
                    queue_open(path);
                }
            }
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
                    drop_font(&mut ui.font);
                    drop_font(&mut ui.bold);
                    drop_font(&mut ui.small);
                }
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

unsafe fn paint_toast(dc: HDC, rc: RECT) {
    let (title, body, detail, font, bold, small) = TOAST.with(|t| {
        let ui = t.borrow();
        (ui.title.clone(), ui.body.clone(), ui.detail.clone(), ui.font, ui.bold, ui.small)
    });
    let dpi = crate::app(|a| a.dpi.max(96));
    let s = |v: i32| v * dpi / 96;
    crate::fill(dc, rc, crate::pal().card);
    crate::fill(dc, RECT { bottom: rc.top + s(3).max(2), ..rc }, CHROME);
    crate::frame(dc, rc, 1, crate::pal().line);
    let pad = s(12);
    let icon = RECT { left: rc.left + pad, top: rc.top + s(12), right: rc.left + pad + s(28), bottom: rc.top + s(40) };
    let wash = crate::blend(crate::pal().card, CHROME, 0.14);
    crate::fill(dc, icon, wash);
    crate::frame(dc, icon, 1, crate::blend(crate::pal().line, CHROME, 0.35));
    let (path, file) = TOAST.with(|t| {
        let ui = t.borrow();
        (ui.path.clone(), ui.file)
    });
    if let Some((list, idx, dim)) = crate::shell_icon(&path, file) {
        crate::draw_fav_icon(dc, list, idx, icon.left + s(6), icon.top + s(6), s(16), dim);
    }
    let text_l = icon.right + s(10);
    let title_r = RECT { left: text_l, top: rc.top + s(10), right: rc.right - pad, bottom: rc.top + s(28) };
    crate::text(dc, title_r, &title, bold, crate::pal().text, DT_LEFT);
    let body_r = RECT { left: text_l, top: title_r.bottom, right: rc.right - pad, bottom: title_r.bottom + s(16) };
    crate::text(dc, body_r, &body, font, crate::pal().sub, DT_LEFT);
    let path_r = RECT { left: text_l, top: body_r.bottom, right: rc.right - pad, bottom: body_r.bottom + s(16) };
    crate::text(dc, path_r, &detail, small, crate::pal().text, DT_LEFT);
    let foot_top = rc.bottom - s(32);
    crate::fill(dc, RECT { top: foot_top, bottom: foot_top + 1, ..rc }, crate::pal().line);
    let hint = RECT { left: rc.left + pad, top: foot_top, right: rc.right - s(150), bottom: rc.bottom };
    crate::text(dc, hint, crate::tr(crate::Key::WatchToastHint), small, crate::pal().sub, DT_LEFT);
    let btn_h = s(20);
    let btn_top = foot_top + (s(32) - btn_h) / 2;
    let open_w = s(52);
    let close_w = s(52);
    let open = RECT { left: rc.right - pad - open_w, top: btn_top, right: rc.right - pad, bottom: btn_top + btn_h };
    let close = RECT { left: open.left - s(4) - close_w, top: btn_top, right: open.left - s(4), bottom: btn_top + btn_h };
    TOAST.with(|t| {
        let mut ui = t.borrow_mut();
        ui.close = close;
        ui.open = open;
    });
    toast_btn(dc, close, crate::tr(crate::Key::Close), small, false, dpi);
    toast_btn(dc, open, crate::tr(crate::Key::Open), small, true, dpi);
}

unsafe fn toast_btn(dc: HDC, r: RECT, label: &str, font: HFONT, primary: bool, dpi: i32) {
    let rad = (2 * dpi / 96).max(1);
    if primary {
        crate::aa_rfill(dc, r, rad, CHROME, Some(CHROME), crate::pal().card);
        crate::text(dc, r, label, font, crate::on_color(CHROME), DT_CENTER);
    } else {
        crate::aa_rfill(dc, r, rad, crate::pal().card, Some(crate::pal().line), crate::pal().card);
        crate::text(dc, r, label, font, crate::pal().text, DT_CENTER);
    }
}

pub(crate) unsafe fn dismiss() {
    let h = TOAST.with(|t| t.borrow().hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = DestroyWindow(h);
    }
}
