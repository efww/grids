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
}

static SLOTS: Mutex<Vec<Slot>> = Mutex::new(Vec::new());
static PARENTS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

thread_local! {
    static FITTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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
        g.push(Slot { hwnd: hwnd.0 as isize, face, hot: false, edit_hooked: false });
    }
    clear_theme(hwnd);
    let cur = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
    let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, (cur | WS_CLIPCHILDREN.0) as isize);
    let _ = SetWindowSubclass(hwnd, Some(combo_proc), COMBO_SUB, 0);
    let _ = SendMessageW(hwnd, CB_SETMINVISIBLE, Some(WPARAM(VISIBLE)), None);
    let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    hook_edit(hwnd);
    fit(hwnd);
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

unsafe fn row_height(hwnd: HWND) -> i32 {
    let rc = client_of(hwnd);
    let h = rc.bottom - rc.top;
    if h >= 8 { h } else { px(dpi_of(hwnd), 26) }
}

unsafe fn fit(hwnd: HWND) {
    if FITTING.with(|c| c.get()) {
        return;
    }
    FITTING.with(|c| c.set(true));
    let rc = client_of(hwnd);
    if rc.bottom > rc.top {
        let h = (rc.bottom - rc.top) as isize;
        // -1 is the selection field. 0 sets the owner-drawn row. Both match the edit slot.
        let _ = SendMessageW(hwnd, CB_SETITEMHEIGHT, Some(WPARAM(usize::MAX)), Some(LPARAM(h)));
        let _ = SendMessageW(hwnd, CB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(h)));
    }
    FITTING.with(|c| c.set(false));
    place_inner_edit(hwnd);
    hook_edit(hwnd);
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
    let bounds = text_bounds(client_of(hwnd), dpi, true);
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

unsafe fn paint_chevron(dc: HDC, slot: RECT, rgb: u32, dpi: i32) {
    if slot.right - slot.left < 6 || slot.bottom <= slot.top {
        return;
    }
    let pen = CreatePen(PS_SOLID, (dpi / 96).max(1), super::cref(rgb));
    if pen.is_invalid() {
        return;
    }
    let old = SelectObject(dc, pen.into());
    let cx = (slot.left + slot.right) / 2;
    let cy = (slot.top + slot.bottom) / 2;
    let dx = px(dpi, 4).max(3);
    let dy = px(dpi, 2).max(1);
    let pts = [POINT { x: cx - dx, y: cy - dy }, POINT { x: cx, y: cy + dy }, POINT { x: cx + dx, y: cy - dy }];
    let _ = Polyline(dc, &pts);
    SelectObject(dc, old);
    let _ = DeleteObject(pen.into());
}

unsafe fn paint_closed(hwnd: HWND, dc: HDC) {
    let client = client_of(hwnd);
    if client.right <= client.left || client.bottom <= client.top || dc.is_invalid() {
        return;
    }
    let dpi = dpi_of(hwnd);
    let face = current_face(hwnd);
    let edge = edge_of(hwnd, face);
    super::aa_rfill(dc, client, px(dpi, RADIUS_DIP).max(1), face.fill, Some(edge), face.ground);
    let chev = chevron_rect(client, dpi);
    paint_chevron(dc, chev, super::pal().sub, dpi);
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
        WM_SETFONT => {
            let r = DefSubclassProc(h, msg, wp, lp);
            place_inner_edit(h);
            return r;
        }
        WM_WINDOWPOSCHANGED => {
            let r = DefSubclassProc(h, msg, wp, lp);
            fit(h);
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
                if is_styled(combo) {
                    mi.itemHeight = row_height(combo).max(1) as u32;
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
    use super::{centered_edit_rect, find_edit, line_top, px, CHEVRON_DIP, DROPDOWN, LIST, PAD_DIP, RADIUS_DIP, CERULEAN_BRAND};
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
            assert_eq!(px(dpi, RADIUS_DIP), RADIUS_DIP * dpi / 96, "radius {dpi}");
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
