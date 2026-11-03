// WM_COPYDATA client. The reply window is created on the first query, on the thread that asks.
// FindWindow is the only probe, and it runs from that query, not at startup.

use std::cell::{Cell, RefCell};
use std::sync::Once;

use crate::imports::*;
use crate::CopyData;

use super::ipc::*;

/// The worker has no message loop. Take a reply that arrived during the wait, otherwise pump
/// once so Everything is not left blocked in SendMessage (the next click would then see nothing).
const QUERY_MS: u32 = 8000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) dir: bool,
    pub(crate) size: Option<u64>,
    pub(crate) modified: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FoundList {
    pub(crate) hits: Vec<Found>,
    pub(crate) more: bool,
}

pub(crate) fn running() -> bool {
    ipc_hwnd().is_some()
}

/// major.minor.revision.build. None when Everything does not answer with a version.
pub(crate) fn version(hwnd: HWND) -> Option<super::locate::Version> {
    unsafe {
        let part = |id: usize| SendMessageW(hwnd, WM_USER, Some(WPARAM(id)), Some(LPARAM(0))).0 as u32;
        super::locate::Version::from_parts(part(0), part(1), part(2), part(3))
    }
}

pub(crate) fn ipc_hwnd() -> Option<HWND> {
    for class in super::locate::window_classes() {
        let wide: Vec<u16> = class.encode_utf16().chain(std::iter::once(0)).collect();
        if let Ok(h) = unsafe { FindWindowW(PCWSTR(wide.as_ptr()), PCWSTR::null()) } {
            if !h.0.is_null() {
                return Some(h);
            }
        }
    }
    None
}

/// Query2 first (Everything 1.4.1 and later, including 1.5). Query W when that reply is missing
/// or is not a list of real paths. A junk parse used to count as success, so Query W never ran.
/// `sort` is Everything's sort code: it orders every match, then the reply keeps the first `max`.
/// `None` is a transport failure. An empty hit list is a real answer.
pub(crate) fn search(text: &str, max: u32, sort: u32) -> Option<FoundList> {
    let ipc = ipc_hwnd()?;
    let reply = reply_window()?;
    let bits = reply.0 as usize as u32;
    let cap = max as usize;
    let q2 = query2(bits, next_reply_id(), max, text, sort);
    if let Some(bytes) = send(ipc, reply, &q2) {
        if let Some(list) = best_list(&bytes, cap) {
            return Some(found(list));
        }
    }
    let qw = query_w(bits, next_reply_id(), max, text);
    let bytes = send(ipc, reply, &qw)?;
    best_list(&bytes, cap).map(found)
}

fn found(list: IpcList) -> FoundList {
    FoundList {
        hits: list.hits.into_iter().map(|h| Found { name: h.name, path: h.path, dir: h.dir, size: h.size, modified: h.modified }).collect(),
        more: list.more,
    }
}

/// Everything 1.4 queues the query and answers later with its own WM_COPYDATA, so pump this
/// thread until that reply (tagged with this query's id) arrives. A late reply to an earlier
/// query carries another id and is dropped.
fn send(ipc: HWND, reply: HWND, q: &IpcQuery) -> Option<Vec<u8>> {
    let id = u32::from_le_bytes(q.bytes[4..8].try_into().ok()?);
    WANT.with(|w| w.set(id));
    REPLY.with(|r| *r.borrow_mut() = None);
    let cd = CopyData { dw: q.copy_data, cb: q.bytes.len() as u32, lp: q.bytes.as_ptr() as _ };
    let sent = unsafe {
        SendMessageTimeoutW(ipc, WM_COPYDATA, WPARAM(reply.0 as usize), LPARAM(&cd as *const CopyData as isize), SMTO_ABORTIFHUNG, QUERY_MS, None)
    };
    if sent.0 == 0 {
        return take_reply();
    }
    let start = unsafe { windows::Win32::System::SystemInformation::GetTickCount() };
    loop {
        if let Some(bytes) = take_reply() {
            return Some(bytes);
        }
        pump_reply();
        if unsafe { windows::Win32::System::SystemInformation::GetTickCount() }.wrapping_sub(start) > QUERY_MS {
            return take_reply();
        }
        unsafe { _ = windows::Win32::UI::WindowsAndMessaging::MsgWaitForMultipleObjects(None, false, 50, windows::Win32::UI::WindowsAndMessaging::QS_ALLINPUT); }
    }
}

fn next_reply_id() -> u32 {
    NEXT_ID.with(|n| {
        let v = n.get().wrapping_add(1);
        n.set(v);
        REPLY_ID.wrapping_add(v)
    })
}

fn take_reply() -> Option<Vec<u8>> {
    REPLY.with(|r| r.borrow_mut().take()).filter(|b| !b.is_empty())
}

fn pump_reply() {
    unsafe {
        let mut m = MSG::default();
        for _ in 0..16 {
            if !PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                break;
            }
            _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
}

thread_local! {
    static REPLY: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    static IPC_HWND: Cell<isize> = const { Cell::new(0) };
    static WANT: Cell<u32> = const { Cell::new(0) };
    static NEXT_ID: Cell<u32> = const { Cell::new(0) };
}

fn reply_window() -> Option<HWND> {
    static CLASS: Once = Once::new();
    CLASS.call_once(|| unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(ipc_proc),
            hInstance: hinst.into(),
            lpszClassName: w!("MaguroEvIpc"),
            ..Default::default()
        });
    });
    IPC_HWND.with(|slot| {
        let cur = HWND(slot.get() as _);
        if !cur.0.is_null() && unsafe { IsWindow(Some(cur)).as_bool() } {
            return Some(cur);
        }
        let created = unsafe {
            CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroEvIpc"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None).ok()?
        };
        unsafe { _ = ChangeWindowMessageFilterEx(created, WM_COPYDATA, MSGFLT_ALLOW, None); }
        slot.set(created.0 as isize);
        Some(created)
    })
}

unsafe extern "system" fn ipc_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { ipc_proc_inner(h, msg, wp, lp) }))
}
unsafe fn ipc_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COPYDATA {
        let cd = &*(lp.0 as *const CopyData);
        if cd.dw == WANT.with(|w| w.get()) as usize && !cd.lp.is_null() && cd.cb > 0 {
            let bytes = std::slice::from_raw_parts(cd.lp as *const u8, cd.cb as usize).to_vec();
            REPLY.with(|r| *r.borrow_mut() = Some(bytes));
            return LRESULT(1);
        }
    }
    DefWindowProcW(h, msg, wp, lp)
}
