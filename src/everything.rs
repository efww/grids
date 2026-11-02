use crate::*;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

// ---------- Everything search: the orange magnifier in the top toolbar, Ctrl+Shift+F ----------
// Talks to a running Everything (voidtools, 1.4.1 or later) over its WM_COPYDATA IPC: no DLL, nothing installed
// or started by Grids, and the button only shows while Everything's IPC window is there. The popup itself is the
// reply window; each query carries its own reply id, so a late answer to an older query is simply dropped.

const QUERY2: usize = 18; // EVERYTHING_IPC_COPYDATA_QUERY2W
// EVERYTHING_IPC_QUERY2_REQUEST_*: the columns asked for. A reply carries them per item in this bit order.
const REQ_NAME: u32 = 0x1;
const REQ_PATH: u32 = 0x2;
const REQ_FULL: u32 = 0x4;
const REQ_EXT: u32 = 0x8;
const REQ_SIZE: u32 = 0x10;
const REQ_CREATED: u32 = 0x20;
const REQ_MODIFIED: u32 = 0x40;
const REQ_ASK: u32 = REQ_NAME | REQ_PATH | REQ_SIZE | REQ_MODIFIED;
// EVERYTHING_IPC_SORT_*_ASCENDING for the header's columns (name, path, size, date modified); +1 is descending
const SORT_ASC: [u32; 4] = [1, 3, 5, 13];
const ITEM_FOLDER: u32 = 1;
pub(crate) const EV_MAX: u32 = 200; // rows asked for; the count line says how many there are in all
const REPLY_BASE: u32 = 0x4752_4500; // + the query number
pub(crate) const EV_ORANGE: u32 = 0xEC7A1C;
pub(crate) const EV_ROWS: usize = 16; // rows on screen, fewer on a short screen (ev_rows_fit)
const EV_RH: i32 = 26; // row height at 96 dpi
const TIMER_EV_QUERY: usize = 3201; // on the popup: a short pause in typing before asking
pub(crate) const TIMER_EV_SELECT: usize = 3202; // on the main window: select the result once its folder shows

pub(crate) static EV_ON: AtomicBool = AtomicBool::new(false); // the toolbar button is shown

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EvHit {
    pub(crate) name: String,
    pub(crate) dir: String, // folder it is in, as Everything gives it ("C:" for a drive's root)
    pub(crate) folder: bool,
    pub(crate) size: Option<u64>,     // bytes; None when Everything doesn't know (folders, mostly)
    pub(crate) modified: Option<u64>, // FILETIME (UTC)
    pub(crate) icon: Option<i32>, // system image list index, looked up when first drawn
}

#[derive(Clone)]
pub(crate) struct EvPick {
    pane: usize,
    dir: String,
    name: String,
    until: Instant,
}

#[derive(Default)]
pub(crate) struct EvState {
    h: HWND,
    edit: HWND,
    prev: HWND, // focus before the popup opened; Esc goes back there
    gen: u32,   // last query sent
    waiting: bool,
    failed: bool, // Everything went away or didn't take the query
    hits: Vec<EvHit>,
    total: u32,
    sel: Option<usize>,
    top: usize,
    hot: Option<EvSpot>,
    rows: usize, // rows on screen while open
    closed: Option<Instant>,
    last: String, // text kept for the next time it opens (runtime only)
    pick: Option<EvPick>,
}

thread_local! {
    static EV: RefCell<EvState> = RefCell::new(EvState::default());
}

// Same rule as app(): no Win32 call that can come back into ev_proc while inside.
fn ev<R>(f: impl FnOnce(&mut EvState) -> R) -> R {
    EV.with(|e| f(&mut e.borrow_mut()))
}

// ---------- pure parts (tested) ----------

/// A sort code from grids.ini or anywhere: one of the header's eight, else name ascending.
pub(crate) fn ev_sort_ok(code: u32) -> u32 {
    if SORT_ASC.iter().any(|&a| code == a || code == a + 1) { code } else { 1 }
}

/// The header column a sort code belongs to, and whether it is descending.
pub(crate) fn ev_sort_col(code: u32) -> (usize, bool) {
    let code = ev_sort_ok(code);
    let c = SORT_ASC.iter().position(|&a| code == a || code == a + 1).unwrap_or(0);
    (c, code != SORT_ASC[c])
}

/// A click on header column `col`: the same column turns around; another one starts A→Z for name and path, and
/// biggest / newest first for size and date (what one usually wants from them).
pub(crate) fn ev_sort_click(cur: u32, col: usize) -> u32 {
    let (c, desc) = ev_sort_col(cur);
    let col = col.min(3);
    let desc = if c == col { !desc } else { col >= 2 };
    SORT_ASC[col] + desc as u32
}

/// EVERYTHING_IPC_QUERY2 followed by the search text: reply window, reply id, flags, offset, max, columns, sort.
pub(crate) fn ev_query(reply: u32, id: u32, text: &str, sort: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(28 + 2 * text.len() + 2);
    for v in [reply, id, 0, 0, EV_MAX, REQ_ASK, ev_sort_ok(sort)] {
        b.extend(v.to_le_bytes());
    }
    for u in text.encode_utf16().chain([0]) {
        b.extend(u.to_le_bytes());
    }
    b
}

/// EVERYTHING_IPC_LIST2 with at least name and path (size and date modified when it sends them): (rows, total
/// matches). None for anything else.
pub(crate) fn ev_parse(b: &[u8]) -> Option<(Vec<EvHit>, u32)> {
    let d = |at: usize| b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
    let q = |at: usize| Some(d(at)? as u64 | (d(at + 4)? as u64) << 32);
    let (total, num, flags) = (d(0)?, d(4)? as usize, d(12)?);
    let known = REQ_NAME | REQ_PATH | REQ_FULL | REQ_EXT | REQ_SIZE | REQ_CREATED | REQ_MODIFIED;
    if flags & (REQ_NAME | REQ_PATH) != REQ_NAME | REQ_PATH || flags & !known != 0 {
        return None;
    }
    // -1 (or 0 for a date) is how Everything says it doesn't know
    let known_u64 = |v: u64, zero_ok: bool| (v != u64::MAX && (zero_ok || v != 0)).then_some(v);
    // a counted string: length in UTF-16 units, the units, then a NUL; the next column follows
    let s = |at: usize| -> Option<(String, usize)> {
        let n = d(at)? as usize;
        let end = at + 4 + n.checked_mul(2)?;
        let w: Vec<u16> = b.get(at + 4..end)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Some((String::from_utf16_lossy(&w), end + 2))
    };
    let mut out = Vec::new();
    for k in 0..num.min(EV_MAX as usize) {
        let (f, off) = (d(20 + k * 8)?, d(24 + k * 8)? as usize);
        let (name, next) = s(off)?;
        let (dir, mut at) = s(next)?;
        for skip in [REQ_FULL, REQ_EXT] {
            if flags & skip != 0 {
                at = s(at)?.1;
            }
        }
        let (mut size, mut modified) = (None, None);
        for (bit, slot) in [(REQ_SIZE, 1), (REQ_CREATED, 0), (REQ_MODIFIED, 2)] {
            if flags & bit != 0 {
                let v = q(at)?;
                at += 8;
                match slot {
                    1 => size = known_u64(v, true),
                    2 => modified = known_u64(v, false),
                    _ => {}
                }
            }
        }
        out.push(EvHit { name, dir, folder: f & ITEM_FOLDER != 0, size, modified, icon: None });
    }
    Some((out, total))
}

/// What Everything is asked: the typed text as it is (its own syntax works), limited to a folder and below by a
/// quoted path prefix. Shell places (This PC ...) have no path to limit to: the whole PC.
pub(crate) fn ev_text(raw: &str, under: Option<&str>) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    Some(match under.filter(|f| is_fs(f)) {
        Some(f) => format!("\"{}\\\" {t}", f.trim_end_matches('\\')),
        None => t.to_string(),
    })
}

// "C:" -> "C:\": Everything names a drive's root without the backslash, the shell wants it.
fn root(d: &str) -> String {
    if d.len() == 2 && d.ends_with(':') { format!("{d}\\") } else { d.to_string() }
}

pub(crate) fn ev_full(h: &EvHit) -> String {
    match h.dir.as_str() {
        "" => root(&h.name),
        d if d.ends_with('\\') => format!("{d}{}", h.name),
        d => format!("{d}\\{}", h.name),
    }
}

/// Where Enter goes: the folder that holds the result, with the result to select there. Ctrl+Enter on a folder
/// (and a drive, which has no parent) opens it instead.
pub(crate) fn ev_target(h: &EvHit, into: bool) -> (String, Option<String>) {
    if (h.folder && into) || h.dir.is_empty() {
        (ev_full(h), None)
    } else {
        (root(&h.dir), Some(h.name.clone()))
    }
}

pub(crate) fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

/// Arrow / page keys: the next row, kept inside the list.
pub(crate) fn ev_step(sel: Option<usize>, n: usize, d: i32) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let cur = sel.map_or(if d > 0 { -1 } else { n as i32 }, |k| k as i32);
    Some((cur + d).clamp(0, n as i32 - 1) as usize)
}

/// First row on screen so that `sel` is visible, moving as little as possible.
pub(crate) fn ev_scroll(top: usize, sel: usize, rows: usize) -> usize {
    if sel < top {
        sel
    } else if sel >= top + rows {
        sel + 1 - rows
    } else {
        top
    }
}

pub(crate) struct EvGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) field: RECT,
    pub(crate) glass: RECT,
    pub(crate) edit: RECT,
    pub(crate) scope: RECT, // the toggle: box and label, all clickable
    pub(crate) check: RECT,
    pub(crate) count: RECT,
    pub(crate) head: [RECT; 4], // column titles: name, path, size, date modified; the rows use the same x ranges
    pub(crate) list: RECT,
    pub(crate) rows: Vec<RECT>,
}

pub(crate) fn ev_geo(fh: i32, dpi: i32, n: usize) -> EvGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, w, rh) = (s(6), s(860), s(EV_RH));
    let field = RECT { left: pad, top: pad, right: w - pad, bottom: pad + s(32) };
    let glass = RECT { right: field.left + s(30), ..field };
    let et = (field.top + field.bottom - fh) / 2;
    let edit = RECT { left: glass.right, top: et, right: field.right - s(8), bottom: et + fh };
    let line = RECT { left: pad, top: field.bottom + s(4), right: w - pad, bottom: field.bottom + s(30) };
    let mid = (line.top + line.bottom) / 2;
    let check = RECT { left: line.left + s(8), top: mid - s(8), right: line.left + s(24), bottom: mid + s(8) };
    let scope = RECT { right: line.left + s(270), ..line };
    let count = RECT { left: scope.right, right: line.right - s(8), ..line };
    let ht = line.bottom + s(4);
    let (x0, xr) = (pad, w - pad - s(6));
    let x3 = xr - s(136);
    let x2 = x3 - s(88);
    let x1 = x0 + (x2 - x0) * 42 / 100;
    let col = |l: i32, r: i32| RECT { left: l, top: ht, right: r, bottom: ht + s(26) };
    let head = [col(x0, x1), col(x1, x2), col(x2, x3), col(x3, xr)];
    let top = ht + s(26) + s(2);
    let rows = (0..n as i32).map(|k| RECT { left: pad, top: top + k * rh, right: w - pad, bottom: top + (k + 1) * rh }).collect();
    let list = RECT { left: pad, top, right: w - pad, bottom: top + n as i32 * rh };
    EvGeo { w, h: list.bottom + pad, field, glass, edit, scope, check, count, head, list, rows }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum EvSpot {
    Scope,
    Head(usize), // column title: 0 name, 1 path, 2 size, 3 date modified
    Row(usize),  // index on screen, not in the list
}

/// Rows that fit in `avail` px of height: the full list, else fewer, never under 3.
pub(crate) fn ev_rows_fit(avail: i32, fh: i32, dpi: i32) -> usize {
    let head = ev_geo(fh, dpi, 0).h;
    ((avail - head) / (EV_RH * dpi / 96)).clamp(3, EV_ROWS as i32) as usize
}

pub(crate) fn ev_hit(g: &EvGeo, x: i32, y: i32) -> Option<EvSpot> {
    if inside(g.scope, x, y) {
        return Some(EvSpot::Scope);
    }
    if let Some(c) = g.head.iter().position(|r| inside(*r, x, y)) {
        return Some(EvSpot::Head(c));
    }
    g.rows.iter().position(|r| inside(*r, x, y)).map(EvSpot::Row)
}

// ---------- Everything itself ----------

/// Everything's IPC window (1.4, or a 1.5 alpha running as its own instance).
pub(crate) unsafe fn ev_window() -> Option<HWND> {
    for class in [w!("EVERYTHING_TASKBAR_NOTIFICATION"), w!("EVERYTHING_TASKBAR_NOTIFICATION_(1.5a)")] {
        if let Ok(h) = FindWindowW(class, PCWSTR::null()) {
            return Some(h);
        }
    }
    None
}

/// For relayout: is the button shown (looked up again each time, it costs a FindWindow).
pub(crate) unsafe fn ev_shown() -> bool {
    let on = ev_window().is_some();
    EV_ON.store(on, Ordering::Relaxed);
    on
}

/// Status tick: Everything started or quit since the last layout.
pub(crate) unsafe fn ev_watch() {
    if ev_window().is_some() != EV_ON.load(Ordering::Relaxed) && !app(|a| a.drag.is_some() || a.sizing) {
        relayout();
    }
}

// ---------- popup ----------

unsafe fn geo_now() -> EvGeo {
    let n = ev(|e| e.rows);
    app(|a| ev_geo(a.fh, a.dpi, n))
}

pub(crate) unsafe fn open_ev() {
    if ev_window().is_none() {
        _ = MessageBeep(MB_ICONWARNING);
        set_note(tr("Everything is not running · start it to search the whole PC", "Everything이 실행 중이 아님 · 실행하면 PC 전체에서 찾을 수 있음").into());
        return;
    }
    let (open, edit, closed, last) = ev(|e| (e.h, e.edit, e.closed, e.last.clone()));
    if !open.0.is_null() {
        _ = SetFocus(Some(edit));
        return;
    }
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let (main, font, fh, dpi) = app(|a| (a.main, a.font, a.fh, a.dpi));
    let mut rc = RECT::default();
    match GetDlgItem(Some(main), ID_TB_EV) {
        Ok(b) if IsWindowVisible(b).as_bool() => _ = GetWindowRect(b, &mut rc),
        _ => {
            _ = GetWindowRect(main, &mut rc);
            rc = RECT { left: rc.left + 24 * dpi / 96, top: rc.top, right: rc.left, bottom: rc.top + 60 * dpi / 96 };
        }
    }
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let rows = ev_rows_fit(mi.rcWork.bottom - rc.bottom - 12 * dpi / 96, fh, dpi);
    ev(|e| e.rows = rows);
    let g = ev_geo(fh, dpi, rows);
    let at = fit(RECT { left: rc.left, top: rc.bottom + g.field.top, right: rc.left + g.w, bottom: rc.bottom + g.field.top + g.h }, mi.rcWork);
    let prev = GetFocus();
    let style = WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsEv"), w!(""), style, at.left, at.top, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    // Everything may run elevated: let its replies through
    _ = ChangeWindowMessageFilterEx(h, WM_COPYDATA, MSGFLT_ALLOW, None);
    let e = g.edit;
    let edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), 1, font);
    _ = MoveWindow(edit, e.left, e.top, e.right - e.left, e.bottom - e.top, false);
    ev(|e| {
        (e.h, e.edit, e.prev, e.hits, e.total, e.sel, e.top, e.hot) = (h, edit, prev, Vec::new(), 0, None, 0, None);
        (e.waiting, e.failed) = (false, false);
    });
    // the last search comes back selected, its results with it (EN_CHANGE asks again)
    _ = SetWindowTextW(edit, &HSTRING::from(last));
    _ = SetFocus(Some(edit));
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
}

pub(crate) unsafe fn ev_close(back: bool) {
    let (h, prev) = ev(|e| (e.h, e.prev));
    if h.0.is_null() {
        return;
    }
    _ = DestroyWindow(h);
    if !back {
        return;
    }
    let (main, active) = app(|a| (a.main, a.s.active));
    let btn = GetDlgItem(Some(main), ID_TB_EV).unwrap_or_default();
    if !prev.0.is_null() && IsWindow(Some(prev)).as_bool() && IsWindowVisible(prev).as_bool() && prev != btn {
        _ = SetFocus(Some(prev));
    } else {
        focus_view(active);
    }
}

// Sends the box's text to Everything; the answer comes as WM_COPYDATA to the popup.
unsafe fn ev_send() {
    let (h, edit) = ev(|e| (e.h, e.edit));
    if h.0.is_null() {
        return;
    }
    let raw = text_of(edit);
    let under = app(|a| a.s.ev_here.then(|| a.s.paths[a.s.active].clone()));
    let text = ev_text(&raw, under.as_deref());
    let ipc = ev_window();
    let id = ev(|e| {
        e.last = raw;
        e.gen = e.gen.wrapping_add(1);
        (e.hits, e.total, e.sel, e.top, e.failed) = (Vec::new(), 0, None, 0, false);
        e.waiting = text.is_some();
        REPLY_BASE.wrapping_add(e.gen)
    });
    if let Some(text) = text {
        let sort = app(|a| a.s.ev_sort);
        let q = ev_query(h.0 as usize as u32, id, &text, sort); // a 32-bit HWND: window handles fit, Everything widens it back
        let cd = CopyData { dw: QUERY2, cb: q.len() as u32, lp: q.as_ptr() as _ };
        let sent = ipc.is_some_and(|w| {
            let lp = LPARAM(&cd as *const CopyData as isize);
            SendMessageTimeoutW(w, WM_COPYDATA, WPARAM(h.0 as usize), lp, SMTO_ABORTIFHUNG, 2000, None).0 != 0
        });
        if !sent {
            ev(|e| (e.waiting, e.failed) = (false, true));
        }
    }
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn ev_move(d: i32) {
    let h = ev(|e| {
        e.sel = ev_step(e.sel, e.hits.len(), d);
        if let Some(k) = e.sel {
            e.top = ev_scroll(e.top, k, e.rows);
        }
        e.h
    });
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn ev_toggle_scope() {
    app(|a| a.s.ev_here ^= true);
    ev_send();
}

// A column title clicked: Everything sorts all the matches (not only the rows shown) and sends the first ones again.
unsafe fn ev_sort_by(col: usize) {
    app(|a| a.s.ev_sort = ev_sort_click(a.s.ev_sort, col));
    ev_send();
}

// Keys in the search box: moves in the list, Enter, Esc, Alt+O. None: not the box's key, key() goes on.
pub(crate) unsafe fn ev_key(m: &MSG) -> Option<bool> {
    let edit = ev(|e| e.edit);
    if edit.0.is_null() || m.hwnd != edit {
        return None;
    }
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, alt) = (down(VK_CONTROL), down(VK_MENU));
    let page = ev(|e| e.rows) as i32;
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        VK_ESCAPE => ev_close(true),
        VK_RETURN => {
            app(|a| a.held = Some(VK_RETURN.0)); // its repeats must not open the file in the list after
            let k = ev(|e| e.sel);
            ev_open(k, ctrl);
        }
        VK_UP if !alt => ev_move(-1),
        VK_DOWN if !alt => ev_move(1),
        VK_PRIOR if !alt => ev_move(-page),
        VK_NEXT if !alt => ev_move(page),
        VK_O if alt && !ctrl => ev_toggle_scope(),
        _ => return Some(false),
    }
    Some(true)
}

// Enter / click: the active pane shows the result's folder with the result selected (Ctrl: a folder opens).
unsafe fn ev_open(k: Option<usize>, into: bool) {
    let Some(hit) = ev(|e| k.and_then(|k| e.hits.get(k).cloned())) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let (i, main) = app(|a| (a.s.active, a.main));
    ev_close(false);
    let (to, name) = ev_target(&hit, into);
    let here = app(|a| same_dir(&a.s.paths[i], &to));
    if !here && !navigate(i, &to) {
        _ = MessageBeep(MB_ICONWARNING);
        set_note(format!("{}: {to}", tr("Can't open", "열 수 없음")));
        focus_view(i);
        return;
    }
    focus_view(i);
    if let Some(name) = name {
        ev(|e| e.pick = Some(EvPick { pane: i, dir: to, name, until: Instant::now() + Duration::from_secs(5) }));
        SetTimer(Some(main), TIMER_EV_SELECT, 50, None);
    }
}

// TIMER_EV_SELECT: once the pane shows the folder and its list has the item, select it. Gives up after a while
// (hidden by the filter or hidden files, or gone).
pub(crate) unsafe fn ev_pick_tick(main: HWND) {
    let p = ev(|e| e.pick.clone());
    let done = match p {
        None => true,
        Some(p) => {
            let there = app(|a| p.pane < a.s.panes() && same_dir(&a.s.paths[p.pane], &p.dir));
            Instant::now() > p.until || (there && select_name(p.pane, &p.name))
        }
    };
    if done {
        ev(|e| e.pick = None);
        _ = KillTimer(Some(main), TIMER_EV_SELECT);
    }
}

unsafe fn select_name(i: usize, name: &str) -> bool {
    let (Some(fv), Some(sv)) = (view_of::<IFolderView2>(i), view_of::<IShellView>(i)) else { return false };
    let Ok(sf) = fv.GetFolder::<IShellFolder>() else { return false };
    let mut pidl = std::ptr::null_mut();
    if sf.ParseDisplayName(HWND::default(), None, &HSTRING::from(name), None, &mut pidl, std::ptr::null_mut()).is_err() {
        return false;
    }
    let flags = SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0;
    // the list may not hold it yet right after the folder opened: only a real selection counts
    let ok = sv.SelectItem(pidl, flags as u32).is_ok() && fv.GetSelectionState(pidl).is_ok_and(|s| s & SVSI_SELECT.0 as u32 != 0);
    CoTaskMemFree(Some(pidl as _));
    if ok {
        focus_view(i);
    }
    ok
}

// The shell's icon for a name, by its type only (no disk access).
unsafe fn type_icon(name: &str, folder: bool) -> i32 {
    let mut fi = SHFILEINFOW::default();
    let attr = if folder { FILE_ATTRIBUTE_DIRECTORY } else { FILE_ATTRIBUTE_NORMAL };
    let flags = SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES;
    if SHGetFileInfoW(&HSTRING::from(name), attr, Some(&mut fi), size_of::<SHFILEINFOW>() as u32, flags) == 0 {
        return -1;
    }
    fi.iIcon
}

/// A FILETIME (UTC) as local "2026-11-03 14:05", the way the status bar and the preview write dates.
fn ev_date(ft: u64) -> String {
    let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let (mut utc, mut st) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut st).is_err() {
            return String::new();
        }
    }
    format!("{:04}-{:02}-{:02} {:02}:{:02}", st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute)
}

unsafe fn paint_ev(h: HWND) {
    let g = geo_now();
    let (dpi, font, small, icon_font, accent, here, sort) =
        app(|a| (a.dpi, a.font, a.small_font, a.icon_font, a.s.accent_now(), a.s.ev_here, a.s.ev_sort));
    let (sort_col, sort_desc) = ev_sort_col(sort);
    // icons for the rows about to show, once each
    let (top, n, shown) = ev(|e| (e.top, e.hits.len(), e.rows));
    for k in top..(top + shown).min(n) {
        if let Some((name, folder)) = ev(|e| e.hits[k].icon.is_none().then(|| (e.hits[k].name.clone(), e.hits[k].folder))) {
            let icon = type_icon(&name, folder);
            ev(|e| {
                if let Some(x) = e.hits.get_mut(k) {
                    x.icon = Some(icon);
                }
            });
        }
    }
    let (rows, sel, hot, waiting, failed, total, typed) =
        ev(|e| (e.hits[e.top.min(n)..(e.top + shown).min(n)].to_vec(), e.sel, e.hot, e.waiting, e.failed, e.total, !e.last.trim().is_empty()));
    let ilist = app(|a| a.ilist.clone()).or_else(|| {
        let l = SHGetImageList::<IImageList>(SHIL_SMALL as i32).ok();
        app(|a| a.ilist = l.clone());
        l
    });
    let s = |v: i32| v * dpi / 96;
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        fill(dc, g.field, pal().pill);
        frame(dc, g.field, 1, accent);
        text(dc, g.glass, ICON_SEARCH, icon_font, EV_ORANGE, DT_CENTER);
        // the toggle: a square box, filled with the selection colour and a tick when on
        let hot_scope = hot == Some(EvSpot::Scope);
        if here {
            fill(dc, g.check, accent);
            text(dc, g.check, "\u{E73E}", icon_font, on_color(accent), DT_CENTER);
        } else {
            frame(dc, g.check, 1, if hot_scope { pal().text } else { pal().sub });
        }
        let label = RECT { left: g.check.right + s(8), ..g.scope };
        text(dc, label, tr("Only under this folder", "이 폴더 아래에서만"), font, if hot_scope || here { pal().text } else { pal().sub }, DT_LEFT);
        let count = match (failed, waiting, rows.is_empty() && total == 0) {
            (true, ..) => tr("Everything did not answer", "Everything 응답 없음").to_string(),
            (_, true, _) => tr("Searching…", "찾는 중…").to_string(),
            _ if !typed => String::new(),
            (_, _, true) => tr("No results", "결과 없음").to_string(),
            _ if (n as u32) < total => match KO.load(Ordering::Relaxed) {
                true => format!("{}개 중 {n}개", group_digits(total as u64)),
                false => format!("{n} of {}", group_digits(total as u64)),
            },
            _ => match KO.load(Ordering::Relaxed) {
                true => format!("{n}개"),
                false => format!("{n} {}", if n == 1 { "result" } else { "results" }),
            },
        };
        text(dc, g.count, &count, small, pal().sub, DT_RIGHT);
        // column titles: click to sort, again to turn around; the sorted one is in text colour with its arrow
        let titles = [tr("Name", "이름"), tr("Path", "경로"), tr("Size", "크기"), tr("Date modified", "수정한 날짜")];
        for (c, (r, t)) in g.head.iter().zip(titles).enumerate() {
            if hot == Some(EvSpot::Head(c)) {
                fill(dc, *r, pal().pill);
            }
            if c > 0 {
                fill(dc, RECT { left: r.left, right: r.left + 1, top: r.top + s(5), bottom: r.bottom - s(5) }, pal().line);
            }
            let on = c == sort_col;
            let label = if on { format!("{t} {}", if sort_desc { "\u{25BE}" } else { "\u{25B4}" }) } else { t.to_string() };
            let ink = if on || hot == Some(EvSpot::Head(c)) { pal().text } else { pal().sub };
            let tr_ = RECT { left: r.left + s(8), right: r.right - s(8), ..*r };
            text(dc, tr_, &label, small, ink, if c == 2 { DT_RIGHT } else { DT_LEFT });
        }
        fill(dc, RECT { top: g.list.top - s(2), bottom: g.list.top - s(1), ..g.list }, pal().line);
        if !typed {
            let hint = tr("Type a name · Everything's own syntax works too (ext:pdf, size:>1mb, dm:today) · click a column title to sort", "이름을 입력 · Everything 문법도 됨 (ext:pdf, size:>1mb, dm:today) · 열 제목을 누르면 정렬");
            text(dc, RECT { bottom: g.list.top + s(80), ..g.list }, hint, font, pal().sub, DT_CENTER);
        }
        for (j, (hit, row)) in rows.iter().zip(&g.rows).enumerate() {
            let k = top + j;
            let on = sel == Some(k);
            let (ink, sub) = if on { (on_color(accent), blend(on_color(accent), accent, 0.3)) } else { (pal().text, pal().sub) };
            if on {
                fill(dc, *row, accent);
            } else if hot == Some(EvSpot::Row(j)) {
                fill(dc, *row, pal().pill);
            }
            let icon = RECT { left: g.head[0].left + s(8), top: row.top + (row.bottom - row.top - s(16)) / 2, right: g.head[0].left + s(24), bottom: 0 };
            if let (Some(il), Some(ix)) = (&ilist, hit.icon.filter(|&x| x >= 0)) {
                if let Ok(hi) = il.GetIcon(ix, ILD_NORMAL.0) {
                    _ = DrawIconEx(dc, icon.left, icon.top, hi, s(16), s(16), 0, None, DI_NORMAL);
                    _ = DestroyIcon(hi);
                }
            }
            let cell = |c: usize| RECT { left: g.head[c].left + s(8), right: g.head[c].right - s(8), top: row.top, bottom: row.bottom };
            text(dc, RECT { left: icon.right + s(8), ..cell(0) }, &hit.name, font, ink, DT_LEFT);
            text(dc, cell(1), &root(&hit.dir), small, sub, DT_LEFT | DT_PATH_ELLIPSIS);
            if let Some(b) = hit.size {
                text(dc, cell(2), &human(b), small, sub, DT_RIGHT);
            }
            if let Some(t) = hit.modified {
                text(dc, cell(3), &ev_date(t), small, sub, DT_LEFT);
            }
        }
        // where the rows on screen sit in the whole list
        if n > shown {
            let (lh, x) = (g.list.bottom - g.list.top, g.list.right + s(1));
            let (t, b) = (top as i32 * lh / n as i32, (top + shown) as i32 * lh / n as i32);
            fill(dc, RECT { left: x, top: g.list.top + t, right: x + s(3), bottom: g.list.top + b }, pal().line);
        }
    });
}

pub(crate) extern "system" fn ev_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => paint_ev(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_COMMAND if (wp.0 >> 16) as u32 == EN_CHANGE => _ = SetTimer(Some(h), TIMER_EV_QUERY, 90, None),
            WM_TIMER if wp.0 == TIMER_EV_QUERY => {
                _ = KillTimer(Some(h), TIMER_EV_QUERY);
                ev_send();
            }
            // Everything's answer; only the last query's is shown. Copied out now: the buffer goes with the call.
            WM_COPYDATA => {
                let cd = &*(lp.0 as *const CopyData);
                let want = ev(|e| REPLY_BASE.wrapping_add(e.gen));
                if cd.dw == want as usize && !cd.lp.is_null() {
                    let got = ev_parse(std::slice::from_raw_parts(cd.lp as *const u8, cd.cb as usize));
                    ev(|e| {
                        e.waiting = false;
                        e.failed = got.is_none();
                        if let Some((hits, total)) = got {
                            e.sel = (!hits.is_empty()).then_some(0);
                            (e.hits, e.total, e.top) = (hits, total, 0);
                        }
                    });
                    _ = InvalidateRect(Some(h), None, false);
                }
                return LRESULT(1);
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let spot = ev_hit(&geo_now(), x, y);
                let n = ev(|e| e.hits.len().saturating_sub(e.top));
                let spot = spot.filter(|s| !matches!(s, EvSpot::Row(j) if *j >= n));
                if ev(|e| std::mem::replace(&mut e.hot, spot)) != spot {
                    _ = InvalidateRect(Some(h), None, false);
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    _ = TrackMouseEvent(&mut t);
                }
            }
            WM_MOUSELEAVE => {
                if ev(|e| e.hot.take()).is_some() {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                match ev_hit(&geo_now(), x, y) {
                    Some(EvSpot::Scope) => ev_toggle_scope(),
                    Some(EvSpot::Head(c)) => ev_sort_by(c),
                    Some(EvSpot::Row(j)) => {
                        let k = ev(|e| e.top + j);
                        if k < ev(|e| e.hits.len()) {
                            ev_open(Some(k), wp.0 & MK_CONTROL.0 as usize != 0);
                        }
                    }
                    None => {}
                }
            }
            WM_MOUSEWHEEL => {
                let d = -((wp.0 >> 16) as i16 as i32) / 120 * 3;
                ev(|e| e.top = (e.top as i32 + d).clamp(0, e.hits.len().saturating_sub(e.rows) as i32) as usize);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => ev(|e| {
                (e.h, e.edit, e.hot) = (HWND::default(), HWND::default(), None);
                e.gen = e.gen.wrapping_add(1); // a reply still on its way finds nobody waiting
                e.closed = Some(Instant::now());
            }),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counted(s: &str) -> Vec<u8> {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut v = (w.len() as u32).to_le_bytes().to_vec();
        for u in w.into_iter().chain([0]) {
            v.extend(u.to_le_bytes());
        }
        v
    }

    // LIST2 the way Everything sends it: header, item table, then each item's columns in bit order
    fn list2(total: u32, items: &[(u32, &str, &str)], flags: u32) -> Vec<u8> {
        let mut b = Vec::new();
        for v in [total, items.len() as u32, 0, flags, 1] {
            b.extend(v.to_le_bytes());
        }
        let mut data = Vec::new();
        let base = 20 + 8 * items.len();
        for (k, (f, name, dir)) in items.iter().enumerate() {
            b.extend(f.to_le_bytes());
            b.extend(((base + data.len()) as u32).to_le_bytes());
            data.extend(counted(name));
            data.extend(counted(dir));
            if flags & REQ_EXT != 0 {
                data.extend(counted("x"));
            }
            if flags & REQ_SIZE != 0 {
                data.extend((if *f & ITEM_FOLDER != 0 { u64::MAX } else { 1000 + k as u64 }).to_le_bytes());
            }
            if flags & REQ_CREATED != 0 {
                data.extend(7u64.to_le_bytes());
            }
            if flags & REQ_MODIFIED != 0 {
                data.extend((133_000_000_000_000_000u64 + k as u64).to_le_bytes());
            }
        }
        b.extend(data);
        b
    }

    #[test]
    fn everything_ipc() {
        let q = ev_query(0x1234, REPLY_BASE + 7, "보고서 ext:pdf", 14);
        let words: Vec<u32> = q[..28].chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        assert_eq!(words, [0x1234, REPLY_BASE + 7, 0, 0, EV_MAX, 0x53, 14]);
        assert_eq!(ev_query(1, 2, "a", 99)[24..28], 1u32.to_le_bytes()); // a bad sort code asks by name
        let text: Vec<u16> = q[28..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(text, "보고서 ext:pdf".encode_utf16().chain([0]).collect::<Vec<_>>());

        let b = list2(9, &[(ITEM_FOLDER, "Work", r"C:\Users\me"), (0, "회의록.hwp", r"D:\문서")], 3);
        let (hits, total) = ev_parse(&b).unwrap();
        assert_eq!(total, 9);
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].name.as_str(), hits[0].dir.as_str(), hits[0].folder), ("Work", r"C:\Users\me", true));
        assert_eq!((hits[1].name.as_str(), hits[1].dir.as_str(), hits[1].folder), ("회의록.hwp", r"D:\문서", false));
        assert_eq!((hits[0].size, hits[0].modified, hits[1].size), (None, None, None));
        // with size and date modified; unknown size (-1) stays empty; extra columns are skipped over
        for flags in [REQ_ASK, REQ_ASK | REQ_EXT | REQ_CREATED] {
            let (h, _) = ev_parse(&list2(2, &[(ITEM_FOLDER, "Work", "C:"), (0, "a.pdf", r"C:\Work")], flags)).unwrap();
            assert_eq!((h[0].size, h[0].modified), (None, Some(133_000_000_000_000_000)));
            assert_eq!((h[1].name.as_str(), h[1].size, h[1].modified), ("a.pdf", Some(1001), Some(133_000_000_000_000_001)));
        }
        assert!(ev_parse(&list2(1, &[(0, "a", "C:")], REQ_ASK | 0x100)).is_none()); // a column it can't read
        // other columns, a cut-off reply, an empty one
        assert!(ev_parse(&list2(1, &[(0, "a", "C:")], 1)).is_none());
        let full = list2(2, &[(0, "a", "C:"), (0, "b", "C:")], REQ_ASK);
        assert!(ev_parse(&full[..full.len() - 3]).is_none());
        assert!(ev_parse(&b[..b.len() - 6]).is_none());
        assert!(ev_parse(&b[..10]).is_none());
        assert_eq!(ev_parse(&list2(0, &[], 3)), Some((vec![], 0)));
    }

    #[test]
    fn everything_search_and_open() {
        assert_eq!(ev_text("  ", Some(r"C:\Work")), None);
        assert_eq!(ev_text(" report ", None).as_deref(), Some("report"));
        assert_eq!(ev_text("report", Some(r"C:\Work")).as_deref(), Some(r#""C:\Work\" report"#));
        assert_eq!(ev_text("*.pdf", Some("C:\\")).as_deref(), Some(r#""C:\" *.pdf"#));
        assert_eq!(ev_text("x", Some(r"\\nas\share\a")).as_deref(), Some(r#""\\nas\share\a\" x"#));
        assert_eq!(ev_text("x", Some(THIS_PC)).as_deref(), Some("x")); // nothing to limit to

        let hit = |name: &str, dir: &str, folder| EvHit { name: name.into(), dir: dir.into(), folder, size: None, modified: None, icon: None };
        let file = hit("a.txt", r"C:\Work", false);
        assert_eq!(ev_target(&file, false), (r"C:\Work".to_string(), Some("a.txt".to_string())));
        assert_eq!(ev_target(&file, true), (r"C:\Work".to_string(), Some("a.txt".to_string()))); // Ctrl doesn't open files
        let dir = hit("Sub", r"C:\Work", true);
        assert_eq!(ev_target(&dir, false), (r"C:\Work".to_string(), Some("Sub".to_string())));
        assert_eq!(ev_target(&dir, true), (r"C:\Work\Sub".to_string(), None));
        assert_eq!(ev_target(&hit("Windows", "C:", true), false), ("C:\\".to_string(), Some("Windows".to_string())));
        assert_eq!(ev_target(&hit("D:", "", true), false), ("D:\\".to_string(), None));
        assert_eq!(ev_full(&hit("x", "D:\\", false)), r"D:\x");
        assert!(same_dir(r"c:\work\", r"C:\Work") && !same_dir(r"C:\Work2", r"C:\Work"));

        // keys stay inside the list; the view follows
        assert_eq!((ev_step(None, 0, 1), ev_step(None, 5, 1), ev_step(None, 5, -1)), (None, Some(0), Some(4)));
        assert_eq!((ev_step(Some(4), 5, 1), ev_step(Some(0), 5, -1), ev_step(Some(2), 50, 10)), (Some(4), Some(0), Some(12)));
        assert_eq!((ev_scroll(0, 3, 10), ev_scroll(0, 12, 10), ev_scroll(5, 2, 10)), (0, 3, 2));
    }

    #[test]
    fn everything_popup_layout() {
        let g = ev_geo(16, 96, EV_ROWS);
        assert_eq!(g.rows.len(), EV_ROWS);
        // a short screen gets fewer rows, never under 3
        assert_eq!((ev_rows_fit(2000, 16, 96), ev_rows_fit(g.h, 16, 96), ev_rows_fit(g.h - 80, 16, 96), ev_rows_fit(50, 16, 96)), (16, 16, 12, 3));
        assert!(g.edit.left >= g.glass.right && g.edit.right <= g.field.right && g.edit.bottom <= g.field.bottom);
        assert!(g.scope.bottom <= g.list.top && g.count.left >= g.scope.right && g.list.bottom < g.h);
        assert_eq!(ev_hit(&g, g.check.left + 2, g.check.top + 2), Some(EvSpot::Scope));
        assert_eq!(ev_hit(&g, g.rows[3].left + 40, g.rows[3].top + 2), Some(EvSpot::Row(3)));
        assert_eq!(ev_hit(&g, g.field.left + 40, g.field.top + 4), None);
        // the column titles sit between the toggle line and the rows, side by side, as wide as the list
        assert!(g.head[0].top >= g.scope.bottom && g.head[0].bottom <= g.list.top);
        assert!(g.head.windows(2).all(|p| p[0].right == p[1].left && p[0].left < p[0].right) && g.head[3].right <= g.list.right);
        for c in 0..4 {
            assert_eq!(ev_hit(&g, g.head[c].left + 4, g.head[c].top + 4), Some(EvSpot::Head(c)));
        }
        // sorting: name A→Z at first; a new column starts A→Z (name, path) or biggest/newest first (size, date);
        // the same column again turns around; codes from the ini are checked
        assert_eq!(Session::default().ev_sort, 1);
        assert_eq!([0, 1, 2, 3].map(|c| ev_sort_click(1, c)), [2, 3, 6, 14]);
        assert_eq!((ev_sort_click(14, 3), ev_sort_click(13, 3), ev_sort_click(6, 2), ev_sort_click(14, 0)), (13, 14, 5, 1));
        assert_eq!((ev_sort_col(14), ev_sort_col(3), ev_sort_col(0), ev_sort_col(9)), ((3, true), (1, false), (0, false), (0, false)));
        assert_eq!([0, 2, 5, 7, 13, 15].map(ev_sort_ok), [1, 2, 5, 1, 13, 1]);
        assert_eq!(Session::parse("ev_sort=14\n").ev_sort, 14);
        assert_eq!(Session::parse("ev_sort=9\n").ev_sort, 1);
        assert_eq!(Session::parse(&Session { ev_sort: 6, ..Session::default() }.to_ini()).ev_sort, 6);
        let mut s = Session::parse("ev_here=1\n");
        assert!(s.ev_here && !Session::default().ev_here);
        s.ev_here = false;
        assert!(!Session::parse(&s.to_ini()).ev_here);
        assert!(Session::parse(&Session { ev_here: true, ..Session::default() }.to_ini()).ev_here);
    }
}
