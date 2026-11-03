// One styled combo for every list in the app. The control stays a native COMBOBOX, so
// arrows, F4, Alt+Down, type-to-select and the mouse wheel are unchanged. What changes
// is the paint: the closed face matches an edit (card fill, hairline border, same
// glyph baseline) and the dropped rows use a cerulean selection.

use std::panic::AssertUnwindSafe;

use windows::Win32::UI::Controls::{CB_SETMINVISIBLE, EM_SETMARGINS, MEASUREITEMSTRUCT, ODS_COMBOBOXEDIT, ODT_COMBOBOX};

use crate::imports::*;

use super::{centered_edit_rect, draw_ink_line, edit_window_rect, on_color, optical_nudge, place_edit, CERULEAN_BRAND};

/// `CBS_DROPDOWNLIST | CBS_HASSTRINGS | CBS_OWNERDRAWFIXED | WS_VSCROLL`.
pub(crate) const LIST: u32 = 0x3 | 0x200 | 0x10 | 0x0020_0000;
/// `CBS_DROPDOWN | CBS_HASSTRINGS | CBS_OWNERDRAWFIXED | WS_VSCROLL`. The child edit stays.
pub(crate) const DROPDOWN: u32 = 0x2 | 0x200 | 0x10 | 0x0020_0000;

const PAD_DIP: i32 = 8;
const CHEVRON_DIP: i32 = 22;
/// Filled arrow: 8×4 device pixels at 96 DPI, inset from the right by the text pad.
const TRI_W_DIP: i32 = 8;
const TRI_H_DIP: i32 = 4;
const TRI_PAD_DIP: i32 = PAD_DIP;
const RADIUS_DIP: i32 = 4;
const INSET_DIP: i32 = 1;
const VISIBLE: usize = 8;

const COMBO_SUB: usize = 0x4D43_4F4D;
const EDIT_SUB: usize = 0x4D43_4544;
const PARENT_SUB: usize = 0x4D43_5041;

/// Colours of the closed face. Focus and hover replace `edge` with cerulean.
#[derive(Clone, Copy)]
pub(crate) struct Face {
    pub fill: u32,
    pub edge: u32,
    pub ink: u32,
    pub ground: u32,
}

pub(crate) type FaceFn = unsafe fn(HWND) -> Face;

struct Slot {
    hwnd: isize,
    face: FaceFn,
    hot: bool,
    edit_hooked: bool,
    /// Closed face height in pixels. 0 until layout gives the control its slot.
    face_h: i32,
}

static SLOTS: Mutex<Vec<Slot>> = Mutex::new(Vec::new());
static PARENTS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

thread_local! {
    static FITTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Nonzero while a size message is inside `DefSubclassProc`, so the combo's
    /// own follow-up `SetWindowPos` is not mistaken for a new layout slot.
    static DEPTH: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

/// user32 stores `CB_SETITEMHEIGHT(-1, request)` as `request + 2` and sizes the
/// closed window as that stored height plus `2 * COMBO_YBORDERSIZE` (2).
/// `slot - 6` is the request whose closed window is the slot.
const SELECTION_EXTRA: i32 = 6;
/// Closed window minus the stored selection height.
const CLOSED_BORDER: i32 = 4;
/// Owner-draw selection `WM_MEASUREITEM` is stored as `returned + 6`.
const MEASURE_SELECTION_PAD: i32 = 6;

fn px(dpi: i32, dip: i32) -> i32 {
    dip * dpi.max(96) / 96
}

/// Card fill, theme line, theme text. Corners match the card the combo sits on.
pub(crate) unsafe fn card_face(_hwnd: HWND) -> Face {
    Face { fill: super::pal().card, edge: super::pal().line, ink: super::pal().text, ground: super::pal().card }
}

pub(crate) fn is_styled(hwnd: HWND) -> bool {
    SLOTS.lock().ok().is_some_and(|g| g.iter().any(|s| s.hwnd == hwnd.0 as isize))
}

pub(crate) unsafe fn clear_theme(hwnd: HWND) {
    if !hwnd.0.is_null() {
        let _ = SetWindowTheme(hwnd, w!(""), w!(""));
    }
}

/// Walk direct children. Theme changes invalidate the parent, not these.
pub(crate) unsafe fn invalidate_in(parent: HWND) {
    if parent.0.is_null() {
        return;
    }
    let mut c = FindWindowExW(Some(parent), None, w!("ComboBox"), PCWSTR::null()).unwrap_or_default();
    while !c.0.is_null() {
        if is_styled(c) {
            let _ = InvalidateRect(Some(c), None, false);
        }
        c = FindWindowExW(Some(parent), Some(c), w!("ComboBox"), PCWSTR::null()).unwrap_or_default();
    }
}

/// Subclass `hwnd` and its parent. `face` is read on each paint.
pub(crate) unsafe fn adopt(hwnd: HWND, face: FaceFn) {
    if hwnd.0.is_null() {
        return;
    }
    if is_styled(hwnd) {
        if let Ok(mut g) = SLOTS.lock() {
            if let Some(slot) = g.iter_mut().find(|s| s.hwnd == hwnd.0 as isize) {
                slot.face = face;
            }
        }
        return;
    }
    let parent = GetParent(hwnd).unwrap_or_default();
    if !parent.0.is_null() {
        ensure_parent(parent);
    }
    if let Ok(mut g) = SLOTS.lock() {
        g.push(Slot { hwnd: hwnd.0 as isize, face, hot: false, edit_hooked: false, face_h: 0 });
    }
    clear_theme(hwnd);
    let cur = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
    let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, (cur | WS_CLIPCHILDREN.0) as isize);
    let _ = SetWindowSubclass(hwnd, Some(combo_proc), COMBO_SUB, 0);
    let _ = SendMessageW(hwnd, CB_SETMINVISIBLE, Some(WPARAM(VISIBLE)), None);
    let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    hook_edit(hwnd);
}

fn ensure_parent(parent: HWND) {
    let Ok(mut g) = PARENTS.lock() else { return };
    let key = parent.0 as isize;
    if g.contains(&key) {
        return;
    }
    let ok = unsafe { SetWindowSubclass(parent, Some(parent_proc), PARENT_SUB, 0).as_bool() };
    if ok {
        g.push(key);
    }
}

fn dpi_of(hwnd: HWND) -> i32 {
    unsafe { GetDpiForWindow(hwnd).max(96) as i32 }
}

fn client_of(hwnd: HWND) -> RECT {
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut rc); }
    rc
}

fn chevron_rect(client: RECT, dpi: i32) -> RECT {
    let sys = unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, dpi as u32) };
    let w = px(dpi, CHEVRON_DIP).max(sys).max(1);
    RECT { left: (client.right - w).max(client.left), top: client.top, right: client.right, bottom: client.bottom }
}

/// Downward triangle in inclusive pixels. `None` when the face is too small.
///
/// The rightmost pixel is `TRI_PAD_DIP` inside the client, and every vertex is
/// strictly inside the client, so the fill is not cut by the clip or the border.
/// Width and height are `TRI_W_DIP` × `TRI_H_DIP` at 96 DPI.
fn triangle_points(client: RECT, dpi: i32) -> Option<[POINT; 3]> {
    let face_w = client.right - client.left;
    let face_h = client.bottom - client.top;
    if face_w < 4 || face_h < 4 {
        return None;
    }
    let w = px(dpi, TRI_W_DIP).max(2).min(face_w - 2);
    let h = px(dpi, TRI_H_DIP).max(2).min(face_h - 2);
    let pad = px(dpi, TRI_PAD_DIP).max(1).min(face_w - w - 1);
    // Last paintable pixel is `right - 1`. Leave `pad` pixels empty after the arrow.
    let rightmost = client.right - 1 - pad;
    let leftmost = rightmost - w + 1;
    if leftmost < client.left {
        return None;
    }
    let topmost = client.top + (face_h - h) / 2;
    let bottommost = topmost + h - 1;
    if topmost < client.top || bottommost >= client.bottom {
        return None;
    }
    let apex_x = leftmost + (rightmost - leftmost) / 2;
    Some([
        POINT { x: leftmost, y: topmost },
        POINT { x: rightmost, y: topmost },
        POINT { x: apex_x, y: bottommost },
    ])
}

/// Text box inside the border. `chevron` reserves the button; list rows do not.
fn text_bounds(client: RECT, dpi: i32, chevron: bool) -> RECT {
    let pad = px(dpi, PAD_DIP);
    let inset = px(dpi, INSET_DIP).max(1);
    let right = if chevron { chevron_rect(client, dpi).left } else { client.right - pad };
    let left = (client.left + pad).min(right);
    RECT { left, top: client.top + inset, right, bottom: (client.bottom - inset).max(client.top + inset) }
}

/// Shift the box by `-nudge` so `draw_line`'s optical nudge lands on the edit's cell.
fn baseline_rect(content: RECT, nudge: i32) -> RECT {
    RECT { left: content.left, top: content.top - nudge, right: content.right, bottom: content.bottom - nudge }
}

/// Top `draw_line` will use for a line of `line_h` after `baseline_rect`.
fn line_top(content: RECT, line_h: i32, nudge: i32) -> i32 {
    let placed = baseline_rect(content, nudge);
    let h = placed.bottom - placed.top;
    placed.top + (h - line_h) / 2 + nudge
}

fn face_of(hwnd: HWND) -> FaceFn {
    SLOTS.lock().ok().and_then(|g| g.iter().find(|s| s.hwnd == hwnd.0 as isize).map(|s| s.face)).unwrap_or(card_face)
}

fn is_hot(hwnd: HWND) -> bool {
    SLOTS.lock().ok().and_then(|g| g.iter().find(|s| s.hwnd == hwnd.0 as isize).map(|s| s.hot)).unwrap_or(false)
}

fn set_hot(hwnd: HWND, hot: bool) {
    let changed = SLOTS.lock().ok().and_then(|mut g| {
        let slot = g.iter_mut().find(|s| s.hwnd == hwnd.0 as isize)?;
        (slot.hot != hot).then(|| slot.hot = hot)
    }).is_some();
    if changed {
        unsafe { let _ = InvalidateRect(Some(hwnd), None, false); }
    }
}

fn editable(hwnd: HWND) -> bool {
    unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & 3 == 2 }
}

unsafe fn find_edit(hwnd: HWND) -> HWND {
    FindWindowExW(Some(hwnd), None, w!("Edit"), PCWSTR::null()).unwrap_or_default()
}

unsafe fn hook_edit(combo: HWND) {
    if !editable(combo) {
        return;
    }
    let already = SLOTS.lock().ok().and_then(|g| g.iter().find(|s| s.hwnd == combo.0 as isize).map(|s| s.edit_hooked)).unwrap_or(true);
    if already {
        return;
    }
    let edit = find_edit(combo);
    if edit.0.is_null() {
        return;
    }
    if SetWindowSubclass(edit, Some(edit_proc), EDIT_SUB, combo.0 as usize).as_bool() {
        if let Ok(mut g) = SLOTS.lock() {
            if let Some(slot) = g.iter_mut().find(|s| s.hwnd == combo.0 as isize) {
                slot.edit_hooked = true;
            }
        }
    }
}

unsafe fn focused(hwnd: HWND) -> bool {
    let f = GetFocus();
    f == hwnd || (!f.0.is_null() && IsChild(hwnd, f).as_bool())
}

unsafe fn dropped(hwnd: HWND) -> bool {
    SendMessageW(hwnd, CB_GETDROPPEDSTATE, None, None).0 != 0
}

fn slot_height(hwnd: HWND) -> i32 {
    SLOTS.lock().ok().and_then(|g| g.iter().find(|s| s.hwnd == hwnd.0 as isize).map(|s| s.face_h)).unwrap_or(0)
}

fn remember_slot(hwnd: HWND, h: i32) {
    if h <= 0 {
        return;
    }
    if let Ok(mut g) = SLOTS.lock() {
        if let Some(slot) = g.iter_mut().find(|s| s.hwnd == hwnd.0 as isize) {
            slot.face_h = h;
        }
    }
}

/// `CB_SETITEMHEIGHT(-1)` request that makes the closed window `slot` pixels tall.
fn selection_request(slot: i32) -> i32 {
    (slot - SELECTION_EXTRA).max(1)
}

/// The closed face is the layout slot. The dropped list is a separate popup.
fn face_rect(slot: RECT) -> RECT {
    slot
}

fn locking() -> bool {
    FITTING.with(|c| c.get()) || DEPTH.with(|c| c.get()) > 0
}

unsafe fn stored_selection(hwnd: HWND) -> i32 {
    SendMessageW(hwnd, CB_GETITEMHEIGHT, Some(WPARAM(usize::MAX)), None).0 as i32
}

/// Height user32 will snap the closed window to, from the stored selection field.
unsafe fn preferred_closed(hwnd: HWND) -> i32 {
    let item = stored_selection(hwnd);
    if item <= 0 { 0 } else { item + CLOSED_BORDER }
}

/// Keep the HWND at the layout slot. A taller request is the combo adding its
/// own border on top of the selection field, which is what covered the rows below.
/// The combo's own resize keeps the position (`SWP_NOMOVE`) and asks for the
/// selection height plus its border. Layout moves the window, so that size sticks.
unsafe fn apply_pos(hwnd: HWND, pos: &mut WINDOWPOS) {
    if pos.flags.0 & SWP_NOSIZE.0 != 0 || pos.cy <= 0 {
        return;
    }
    let slot = slot_height(hwnd);
    if locking() {
        if slot > 0 {
            pos.cy = slot;
        }
        return;
    }
    if slot > 0 && pos.cy != slot {
        let nomove = pos.flags.0 & SWP_NOMOVE.0 != 0;
        let preferred = preferred_closed(hwnd);
        if nomove || (preferred > 0 && pos.cy == preferred) {
            pos.cy = slot;
            return;
        }
    }
    if pos.cy != slot {
        remember_slot(hwnd, pos.cy);
    }
}

/// Client area of the closed face, clipped to the slot when the window is taller.
fn face_client(hwnd: HWND) -> RECT {
    let mut rc = client_of(hwnd);
    let h = slot_height(hwnd);
    if h > 0 && rc.bottom > rc.top + h {
        rc.bottom = rc.top + h;
    }
    rc
}

unsafe fn lock_face(hwnd: HWND) {
    if FITTING.with(|c| c.get()) {
        return;
    }
    let slot = slot_height(hwnd);
    if slot <= 0 {
        return;
    }
    FITTING.with(|c| c.set(true));
    let want_stored = (slot - CLOSED_BORDER).max(1);
    if stored_selection(hwnd) != want_stored {
        let _ = SendMessageW(hwnd, CB_SETITEMHEIGHT, Some(WPARAM(usize::MAX)), Some(LPARAM(selection_request(slot) as isize)));
    }
    let list = SendMessageW(hwnd, CB_GETITEMHEIGHT, Some(WPARAM(0)), None).0 as i32;
    if list != slot {
        let _ = SendMessageW(hwnd, CB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(slot as isize)));
    }
    let got = window_in_parent(hwnd);
    let h = got.bottom - got.top;
    let w = got.right - got.left;
    if h != slot && w > 0 {
        let _ = SetWindowPos(hwnd, None, 0, 0, w, slot, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    FITTING.with(|c| c.set(false));
    place_inner_edit(hwnd);
    hook_edit(hwnd);
}

unsafe fn in_size(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DEPTH.with(|c| c.set(c.get().saturating_add(1)));
    let r = DefSubclassProc(hwnd, msg, wp, lp);
    DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
    if DEPTH.with(|c| c.get()) == 0 && !FITTING.with(|c| c.get()) {
        lock_face(hwnd);
    }
    r
}

fn window_in_parent(hwnd: HWND) -> RECT {
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let parent = GetParent(hwnd).unwrap_or_default();
        let mut pts = [POINT { x: r.left, y: r.top }, POINT { x: r.right, y: r.bottom }];
        let _ = MapWindowPoints(None, Some(parent), &mut pts);
        RECT { left: pts[0].x, top: pts[0].y, right: pts[1].x, bottom: pts[1].y }
    }
}

fn same_rect(a: RECT, b: RECT) -> bool {
    a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom
}

unsafe fn place_inner_edit(hwnd: HWND) {
    if FITTING.with(|c| c.get()) {
        return;
    }
    let edit = find_edit(hwnd);
    if edit.0.is_null() {
        return;
    }
    let dpi = dpi_of(hwnd);
    let bounds = text_bounds(face_client(hwnd), dpi, true);
    if bounds.bottom <= bounds.top || bounds.right <= bounds.left {
        return;
    }
    let want = edit_window_rect(edit, bounds);
    if same_rect(window_in_parent(edit), want) {
        return;
    }
    FITTING.with(|c| c.set(true));
    place_edit(edit, bounds, true);
    // The box already starts at the pad. A margin would shift the line off the painted rows.
    let _ = SendMessageW(edit, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(0)));
    FITTING.with(|c| c.set(false));
}

unsafe fn current_face(hwnd: HWND) -> Face {
    let paint = face_of(hwnd);
    let mut face = paint(hwnd);
    if !IsWindowEnabled(hwnd).as_bool() {
        face.ink = super::pal().sub;
    }
    face
}

fn edge_of(hwnd: HWND, face: Face) -> u32 {
    let live = unsafe { IsWindowEnabled(hwnd).as_bool() && (is_hot(hwnd) || focused(hwnd) || dropped(hwnd)) };
    if live { CERULEAN_BRAND } else { face.edge }
}

unsafe fn font_of(hwnd: HWND) -> HFONT {
    HFONT(SendMessageW(hwnd, WM_GETFONT, None, None).0 as _)
}

unsafe fn item_text(hwnd: HWND, index: u32) -> String {
    let n = SendMessageW(hwnd, CB_GETLBTEXTLEN, Some(WPARAM(index as usize)), None).0;
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = SendMessageW(hwnd, CB_GETLBTEXT, Some(WPARAM(index as usize)), Some(LPARAM(buf.as_mut_ptr() as isize))).0;
    if got <= 0 {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(got as usize).min(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

unsafe fn selected_text(hwnd: HWND) -> String {
    let sel = SendMessageW(hwnd, CB_GETCURSEL, None, None).0;
    if sel < 0 {
        return String::new();
    }
    item_text(hwnd, sel as u32)
}

unsafe fn paint_label(dc: HDC, bounds: RECT, text: &str, font: HFONT, color: u32) {
    if text.is_empty() || font.is_invalid() || bounds.right <= bounds.left || bounds.bottom <= bounds.top {
        return;
    }
    // `draw_line` adds `nudge` inside the rect. Shifting by `-nudge` puts the glyph
    // on the same cell `place_edit` gives the edit beside this combo.
    let nudge = optical_nudge(dc, font, text);
    draw_ink_line(dc, baseline_rect(bounds, nudge), text, font, color, DT_LEFT | DT_END_ELLIPSIS);
}

unsafe fn paint_triangle(dc: HDC, client: RECT, rgb: u32, dpi: i32) {
    let Some(pts) = triangle_points(client, dpi) else { return };
    let brush = CreateSolidBrush(super::cref(rgb));
    if brush.is_invalid() {
        return;
    }
    // No pen: a stroke is centred on the vertices and was what clipped the old chevron.
    let pen = HPEN(GetStockObject(NULL_PEN).0);
    let old_pen = SelectObject(dc, pen.into());
    let old_brush = SelectObject(dc, brush.into());
    let _ = Polygon(dc, &pts);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    let _ = DeleteObject(brush.into());
}

unsafe fn paint_closed(hwnd: HWND, dc: HDC) {
    let client = face_client(hwnd);
    if client.right <= client.left || client.bottom <= client.top || dc.is_invalid() {
        return;
    }
    let dpi = dpi_of(hwnd);
    let face = current_face(hwnd);
    let edge = edge_of(hwnd, face);
    super::aa_rfill(dc, client, px(dpi, RADIUS_DIP).max(1), face.fill, Some(edge), face.ground);
    // Same ink as the label: theme text when enabled, `pal().sub` when disabled.
    paint_triangle(dc, client, face.ink, dpi);
    // An editable combo's child edit draws the text on the same cell.
    if find_edit(hwnd).0.is_null() {
        paint_label(dc, text_bounds(client, dpi, true), &selected_text(hwnd), font_of(hwnd), face.ink);
    }
}

unsafe fn paint_item(d: &DRAWITEMSTRUCT) {
    if d.itemState.0 & ODS_COMBOBOXEDIT.0 != 0 {
        paint_closed(d.hwndItem, d.hDC);
        return;
    }
    if d.itemID == u32::MAX {
        return;
    }
    let face = current_face(d.hwndItem);
    let disabled = d.itemState.0 & ODS_DISABLED.0 != 0 || !IsWindowEnabled(d.hwndItem).as_bool();
    let selected = !disabled && d.itemState.0 & ODS_SELECTED.0 != 0;
    let fill = if selected { CERULEAN_BRAND } else { face.fill };
    let ink = if disabled {
        super::pal().sub
    } else if selected {
        on_color(CERULEAN_BRAND)
    } else {
        face.ink
    };
    super::fill(d.hDC, d.rcItem, fill);
    let label = item_text(d.hwndItem, d.itemID);
    paint_label(d.hDC, text_bounds(d.rcItem, dpi_of(d.hwndItem), false), &label, font_of(d.hwndItem), ink);
}

unsafe fn ctl_color(hwnd: HWND, wp: WPARAM) -> LRESULT {
    let face = current_face(hwnd);
    let dc = HDC(wp.0 as _);
    SetBkMode(dc, TRANSPARENT);
    SetBkColor(dc, super::cref(face.fill));
    SetTextColor(dc, super::cref(face.ink));
    SetDCBrushColor(dc, super::cref(face.fill));
    LRESULT(GetStockObject(DC_BRUSH).0 as isize)
}

unsafe fn track(hwnd: HWND) {
    let mut ev = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    let _ = TrackMouseEvent(&mut ev);
}

unsafe fn still_inside(hwnd: HWND) -> bool {
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() {
        return false;
    }
    let mut rc = RECT::default();
    if GetWindowRect(hwnd, &mut rc).is_err() {
        return false;
    }
    pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom
}

unsafe fn drop_slot(hwnd: HWND) {
    let edit = find_edit(hwnd);
    if !edit.0.is_null() {
        let _ = RemoveWindowSubclass(edit, Some(edit_proc), EDIT_SUB);
    }
    if let Ok(mut g) = SLOTS.lock() {
        g.retain(|s| s.hwnd != hwnd.0 as isize);
    }
}

unsafe extern "system" fn combo_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    crate::crash::catch_wnd(AssertUnwindSafe(|| unsafe { combo_proc_inner(h, msg, wp, lp) }))
}

unsafe fn combo_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCCALCSIZE => {
            // No system border: the client is the slot, the same outer box as the edit frame.
            if wp.0 == 0 {
                let rect = &mut *(lp.0 as *mut RECT);
                let full = *rect;
                let _ = DefSubclassProc(h, msg, wp, lp);
                *rect = full;
                return LRESULT(0);
            }
            let params = &mut *(lp.0 as *mut NCCALCSIZE_PARAMS);
            let full = params.rgrc[0];
            let _ = DefSubclassProc(h, msg, wp, lp);
            params.rgrc[0] = full;
            return LRESULT(0);
        }
        WM_NCPAINT => return LRESULT(0),
        WM_ERASEBKGND => return LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = BeginPaint(h, &mut ps);
            paint_closed(h, dc);
            let _ = EndPaint(h, &ps);
            return LRESULT(0);
        }
        WM_MOUSEMOVE => {
            set_hot(h, true);
            track(h);
        }
        WM_MOUSELEAVE => {
            if still_inside(h) {
                track(h);
            } else {
                set_hot(h, false);
            }
        }
        WM_SETFOCUS | WM_KILLFOCUS | WM_ENABLE => {
            let _ = InvalidateRect(Some(h), None, false);
        }
        WM_WINDOWPOSCHANGING => {
            if lp.0 != 0 {
                apply_pos(h, &mut *(lp.0 as *mut WINDOWPOS));
            }
        }
        WM_SETFONT | WM_WINDOWPOSCHANGED => return in_size(h, msg, wp, lp),
        CB_SETITEMHEIGHT => {
            let r = DefSubclassProc(h, msg, wp, lp);
            if !FITTING.with(|c| c.get()) {
                lock_face(h);
            }
            return r;
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC => return ctl_color(h, wp),
        WM_NCDESTROY => {
            drop_slot(h);
            let r = DefSubclassProc(h, msg, wp, lp);
            let _ = RemoveWindowSubclass(h, Some(combo_proc), COMBO_SUB);
            return r;
        }
        _ => {}
    }
    DefSubclassProc(h, msg, wp, lp)
}

unsafe extern "system" fn edit_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, combo_bits: usize) -> LRESULT {
    crate::crash::catch_wnd(AssertUnwindSafe(|| unsafe { edit_proc_inner(h, msg, wp, lp, combo_bits) }))
}

unsafe fn edit_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, combo_bits: usize) -> LRESULT {
    let combo = HWND(combo_bits as *mut core::ffi::c_void);
    if msg == WM_SETFOCUS || msg == WM_KILLFOCUS || msg == WM_MOUSEMOVE {
        if msg == WM_MOUSEMOVE {
            set_hot(combo, true);
            track(combo);
        } else {
            let _ = InvalidateRect(Some(combo), None, false);
        }
    }
    if msg == WM_NCDESTROY {
        let r = DefSubclassProc(h, msg, wp, lp);
        let _ = RemoveWindowSubclass(h, Some(edit_proc), EDIT_SUB);
        return r;
    }
    DefSubclassProc(h, msg, wp, lp)
}

unsafe extern "system" fn parent_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    crate::crash::catch_wnd(AssertUnwindSafe(|| unsafe { parent_proc_inner(h, msg, wp, lp) }))
}

unsafe fn parent_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MEASUREITEM => {
            let mi = &mut *(lp.0 as *mut MEASUREITEMSTRUCT);
            if mi.CtlType == ODT_COMBOBOX {
                let combo = GetDlgItem(Some(h), mi.CtlID as i32).unwrap_or_default();
                let slot = slot_height(combo);
                // A measure of the whole window is what made the face ~46px: user32
                // adds 6 to the selection measure and then a 4px border.
                if is_styled(combo) && slot > 0 {
                    mi.itemHeight = if mi.itemID == u32::MAX {
                        (slot - CLOSED_BORDER - MEASURE_SELECTION_PAD).max(1) as u32
                    } else {
                        slot.max(1) as u32
                    };
                    return LRESULT(1);
                }
            }
        }
        WM_DRAWITEM => {
            let d = &*(lp.0 as *const DRAWITEMSTRUCT);
            if d.CtlType == ODT_COMBOBOX && is_styled(d.hwndItem) {
                paint_item(d);
                return LRESULT(1);
            }
        }
        WM_NCDESTROY => {
            if let Ok(mut g) = PARENTS.lock() {
                g.retain(|p| *p != h.0 as isize);
            }
            let r = DefSubclassProc(h, msg, wp, lp);
            let _ = RemoveWindowSubclass(h, Some(parent_proc), PARENT_SUB);
            return r;
        }
        _ => {}
    }
    DefSubclassProc(h, msg, wp, lp)
}

#[cfg(test)]
mod tests {
    use super::{
        adopt, centered_edit_rect, face_rect, find_edit, line_top, px, selection_request, text_bounds, triangle_points, window_in_parent, CHEVRON_DIP, CLOSED_BORDER, DROPDOWN, LIST, PAD_DIP, RADIUS_DIP, SELECTION_EXTRA, TRI_H_DIP, TRI_PAD_DIP, TRI_W_DIP, CERULEAN_BRAND,
    };
    use crate::imports::*;

    #[test]
    fn styles_stay_native_lists_with_strings() {
        assert_eq!(LIST & 0x3, 3, "dropdown list, not an edit");
        assert_eq!(DROPDOWN & 0x3, 2, "editable preset field");
        assert_ne!(LIST & 0x10, 0, "owner-drawn rows");
        assert_ne!(DROPDOWN & 0x10, 0, "owner-drawn rows");
        assert_ne!(LIST & 0x200, 0, "type-to-select reads the strings");
        assert_ne!(DROPDOWN & 0x200, 0, "type-to-select reads the strings");
        assert_ne!(LIST & 0x0020_0000, 0);
    }

    #[test]
    fn chrome_scales_at_100_125_and_150_percent() {
        for dpi in [96, 120, 144] {
            assert_eq!(px(dpi, PAD_DIP), PAD_DIP * dpi / 96, "pad {dpi}");
            assert_eq!(px(dpi, CHEVRON_DIP), CHEVRON_DIP * dpi / 96, "chevron {dpi}");
            assert_eq!(px(dpi, TRI_W_DIP), TRI_W_DIP * dpi / 96, "arrow width {dpi}");
            assert_eq!(px(dpi, TRI_H_DIP), TRI_H_DIP * dpi / 96, "arrow height {dpi}");
            assert_eq!(px(dpi, TRI_PAD_DIP), TRI_PAD_DIP * dpi / 96, "arrow pad {dpi}");
            assert_eq!(px(dpi, RADIUS_DIP), RADIUS_DIP * dpi / 96, "radius {dpi}");
        }
    }

    #[test]
    fn filled_triangle_stays_inside_the_face_at_100_125_and_150() {
        // Batch Rename is 26 DIP, Settings is 32. 120 and 144 are 125% and 150%.
        for dpi in [96, 120, 144] {
            let w = px(dpi, TRI_W_DIP);
            let h = px(dpi, TRI_H_DIP);
            let pad = px(dpi, TRI_PAD_DIP);
            assert_eq!((w, h, pad), (8 * dpi / 96, 4 * dpi / 96, 8 * dpi / 96), "dpi {dpi}");
            for (origin_x, origin_y, face_h) in [(0, 0, 26 * dpi / 96), (40, 12, 32 * dpi / 96)] {
                let client = RECT { left: origin_x, top: origin_y, right: origin_x + 220, bottom: origin_y + face_h };
                let pts = triangle_points(client, dpi).expect("face fits the arrow");
                let min_x = pts.iter().map(|p| p.x).min().unwrap();
                let max_x = pts.iter().map(|p| p.x).max().unwrap();
                let min_y = pts.iter().map(|p| p.y).min().unwrap();
                let max_y = pts.iter().map(|p| p.y).max().unwrap();
                assert!(min_x >= client.left && max_x < client.right, "x inside at {dpi} h {face_h}: {min_x}..{max_x}");
                assert!(min_y >= client.top && max_y < client.bottom, "y inside at {dpi} h {face_h}: {min_y}..{max_y}");
                assert_eq!(max_x - min_x + 1, w, "width {dpi}");
                assert_eq!(max_y - min_y + 1, h, "height {dpi}");
                assert_eq!(client.right - 1 - max_x, pad, "right pad {dpi}");
                // Pixel centre of the arrow matches the face. Odd heights land within one pixel.
                let slack = (min_y + max_y) - (origin_y + origin_y + face_h - 1);
                assert!(slack.abs() <= 1, "centred at {dpi} h {face_h}, slack {slack}");
                assert_eq!(pts[0].y, min_y, "flat top");
                assert_eq!(pts[1].y, min_y, "flat top");
                assert_eq!(pts[2].y, max_y, "apex down");
                assert!(pts[2].x > min_x && pts[2].x < max_x, "apex between the top corners");
                let text = text_bounds(client, dpi, true);
                assert!(text.right <= min_x, "label clears the arrow at {dpi}");
            }
        }
    }

    #[test]
    fn text_baseline_matches_the_edit_cell_at_100_125_and_150() {
        for dpi in [96, 120, 144] {
            let h = 26 * dpi / 96;
            let line = 16 * dpi / 96;
            let content = RECT { left: px(dpi, PAD_DIP), top: 40, right: 240, bottom: 40 + h };
            for nudge in [-(2 * dpi / 96), 0, 2 * dpi / 96] {
                let cell = centered_edit_rect(content, line);
                assert_eq!(line_top(content, line, nudge), cell.top, "dpi {dpi} nudge {nudge}");
            }
        }
    }

    #[test]
    fn selection_is_cerulean_not_mint() {
        assert_eq!(CERULEAN_BRAND, 0x0A9B_D6);
        assert_ne!(CERULEAN_BRAND, 0x2EC4_A6);
    }

    /// Batch Rename fields are 26 DIP and Settings combos are 32 DIP.
    /// 120 and 144 are 125% and 150% of 96.
    fn slot_heights() -> [i32; 6] {
        let mut out = [0; 6];
        let mut i = 0;
        for dip in [26, 32] {
            for dpi in [96, 120, 144] {
                out[i] = dip * dpi / 96;
                i += 1;
            }
        }
        out
    }

    #[test]
    fn face_rect_equals_the_slot() {
        for h in slot_heights() {
            let slot = RECT { left: 8, top: 12, right: 160, bottom: 12 + h };
            assert_eq!((face_rect(slot).bottom - face_rect(slot).top), h, "face {h}");
            assert_eq!(face_rect(slot).top, slot.top, "face origin {h}");
            assert_eq!(selection_request(h) + SELECTION_EXTRA, h, "request {h}");
            assert!(h > CLOSED_BORDER, "{h}");
        }
        unsafe {
            for style in [LIST, DROPDOWN] {
                let parent = CreateWindowExW(WINDOW_EX_STYLE(0), w!("Static"), w!("t"), WS_POPUP, 0, 0, 480, 320, None, None, None, None).unwrap_or_default();
                if parent.0.is_null() {
                    return;
                }
                let combo = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("ComboBox"),
                    w!(""),
                    WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style),
                    0,
                    0,
                    0,
                    0,
                    Some(parent),
                    Some(HMENU(7isize as _)),
                    None,
                    None,
                )
                .unwrap_or_default();
                if combo.0.is_null() {
                    let _ = DestroyWindow(parent);
                    return;
                }
                adopt(combo, super::card_face);
                for h in slot_heights() {
                    let slot = RECT { left: 10, top: 20, right: 180, bottom: 20 + h };
                    let _ = MoveWindow(combo, slot.left, slot.top, slot.right - slot.left, h, true);
                    let got = window_in_parent(combo);
                    assert_eq!(got.left, slot.left, "style {style:#x} x {h}");
                    assert_eq!(got.top, slot.top, "style {style:#x} y {h}");
                    assert_eq!(got.right, slot.right, "style {style:#x} right {h}");
                    assert_eq!(got.bottom - got.top, h, "style {style:#x} face height {h}");
                    // Setting the selection field to the client is what stretched the face.
                    let _ = SendMessageW(combo, CB_SETITEMHEIGHT, Some(WPARAM(usize::MAX)), Some(LPARAM(80)));
                    let after = window_in_parent(combo);
                    assert_eq!(after.bottom - after.top, h, "item height must not grow the face, style {style:#x} {h}");
                    assert_eq!(after.top, slot.top, "grown face moved, style {style:#x} {h}");
                    if style & 3 == 2 {
                        let edit = find_edit(combo);
                        assert!(!edit.0.is_null(), "preset field keeps its edit");
                        // The edit's parent is the combo, so this rect is inside the face.
                        let er = window_in_parent(edit);
                        let above = er.top;
                        let below = h - er.bottom;
                        assert!(above >= 0 && below >= 0 && er.bottom - er.top < h, "edit inside the {h}px face, above {above} below {below}");
                        assert!((above - below).abs() <= 1, "edit centred in the {h}px face, above {above} below {below}");
                    }
                }
                let _ = DestroyWindow(combo);
                let _ = DestroyWindow(parent);
            }
        }
    }

    #[test]
    fn editable_style_keeps_a_child_edit() {
        unsafe {
            let parent = CreateWindowExW(WINDOW_EX_STYLE(0), w!("Static"), w!("t"), WS_POPUP, 0, 0, 10, 10, None, None, None, None).unwrap_or_default();
            if parent.0.is_null() {
                return;
            }
            let combo = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("ComboBox"),
                w!(""),
                WS_CHILD | WINDOW_STYLE(DROPDOWN),
                0,
                0,
                180,
                220,
                Some(parent),
                None,
                None,
                None,
            )
            .unwrap_or_default();
            let edit = if combo.0.is_null() { HWND::default() } else { find_edit(combo) };
            if !combo.0.is_null() {
                let _ = DestroyWindow(combo);
            }
            let _ = DestroyWindow(parent);
            if combo.0.is_null() {
                return;
            }
            assert!(!edit.0.is_null(), "typing a preset name needs the combo's edit");
        }
    }
}
