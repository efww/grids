// Select, deselect, or invert by a name pattern. Num+ / Num− / Num*, and Ctrl+Shift+A.
// One shot per key hold. At most 10,000 items, drawn in one batch. The result stays on the status line for 5 s.
// The pattern box paints through the shared window, caption, field, label, and edit colours.

use crate::imports::*;
use crate::SEND_MAX;
use crate::app::*;
use crate::ui::*;
use crate::session::*;
use crate::i18n::*;
use super::*;

/// EC_LEFTMARGIN | EC_RIGHTMARGIN.
const EM_SETMARGINS: u32 = 0x00D3;

pub(crate) fn held_key(held: Option<u16>, vk: u16, repeat: bool) -> (bool, Option<u16>) {
    match held {
        Some(k) if k == vk => (repeat, held.filter(|_| repeat)),
        _ => (false, held),
    }
}

pub(crate) struct MatchGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) label: RECT,
    pub(crate) field: RECT,
    pub(crate) edit: RECT,
}

/// 320 DIP wide by default, wider when the label needs it. The field keeps at least 120 DIP.
pub(crate) fn match_geo(label_w: i32, fh: i32, dpi: i32) -> MatchGeo {
    let s = |v: i32| v * dpi / 96;
    let h = s(32).max(fh + s(14));
    let label = RECT { left: s(12), top: 0, right: s(12) + label_w, bottom: h };
    let w = s(320).max(label.right + s(10) + s(120) + s(6));
    let field = RECT { left: label.right + s(10), top: s(5), right: w - s(6), bottom: h - s(5) };
    // The edit window sits inside the painted border. Glyphs stay one field pad in,
    // via the edit margin set in `place_match_edit`.
    let edit = field_edit_rect(field, dpi);
    MatchGeo { w, h, label, field, edit }
}

pub(crate) fn match_label(desel: bool) -> &'static str {
    if desel {
        tr(crate::Key::Deselect)
    } else {
        tr(crate::Key::Select)
    }
}

pub(crate) unsafe fn match_geo_now(desel: bool) -> MatchGeo {
    let (fh, dpi) = app(|a| (a.fh, a.dpi));
    let font = body_font();
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &match_label(desel).encode_utf16().collect::<Vec<_>>(), &mut sz);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    match_geo(sz.cx, fh, dpi)
}

pub(crate) unsafe fn open_match(i: usize, desel: bool) {
    let (main, open, last, path) = app(|a| (a.main, a.matcher.is_some(), a.match_last.clone(), a.s.paths[i].clone()));
    if open || view_of::<IFolderView2>(i).is_none() {
        return;
    }
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let g = match_geo_now(desel);
    let mut rc = RECT::default();
    // Win10 has no SysListView32. The items view (or the shell view, if that child is not up yet)
    // is the rect the pattern box sits on. A null window leaves the box at the origin.
    let mut anchor = view_list(&fv);
    if anchor.0.is_null() {
        anchor = fv.cast::<IShellView>().and_then(|v| v.GetWindow()).unwrap_or_default();
    }
    if !anchor.0.is_null() {
        _ = GetWindowRect(anchor, &mut rc);
    }
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let work = mi.rcWork;
    let x = rc.left.min(work.right - g.w).max(work.left);
    let y = rc.top.min(work.bottom - g.h).max(work.top);
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("MaguroMatch"), w!(""), WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN, x, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    style_dialog_caption(h);
    // Square on purpose: the popup is a tool window, and the client draws its own hairline.
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    let edit = child(h, w!("EDIT"), PCWSTR(HSTRING::from(last).as_ptr()), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), 1, body_font());
    place_match_edit(edit, &g);
    app(|a| a.matcher = Some(MatchBox { hwnd: h, edit, pane: i, path, desel }));
    _ = SetFocus(Some(edit));
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
}

unsafe fn place_match_edit(edit: HWND, g: &MatchGeo) {
    let dpi = app(|a| a.dpi);
    // `g.edit` is already `FIELD_INSET` inside the frame. The rest of the shared
    // field pad is the edit margin, so the glyphs stay where the old 8 DIP inset put them.
    let m = px(dpi, FIELD_PAD_X - FIELD_INSET).max(0) as isize;
    let _ = SendMessageW(edit, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(m | (m << 16))));
    place_edit(edit, g.edit, true);
}

/// Font or DPI changed while the pattern box is open.
pub(crate) unsafe fn refit_match() {
    let Some((h, edit, desel)) = app(|a| a.matcher.as_ref().map(|m| (m.hwnd, m.edit, m.desel))) else { return };
    let g = match_geo_now(desel);
    _ = SetWindowPos(h, None, 0, 0, g.w, g.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    place_match_edit(edit, &g);
}

pub(crate) unsafe fn close_match(back: bool) {
    let Some((h, i)) = app(|a| a.matcher.as_ref().map(|m| (m.hwnd, m.pane))) else { return };
    _ = DestroyWindow(h);
    if back {
        focus_view(i);
    }
}

pub(crate) unsafe fn set_note(note: String) {
    // Expiry, not the moment it was set: a note whose time has already passed vanishes on the next redraw.
    app(|x| x.note = Some((note, Instant::now() + std::time::Duration::from_secs(5))));
    update_status();
}

pub(crate) unsafe fn bulk_select(fv: &IFolderView2, mut each: impl FnMut(i32) -> bool) -> Option<usize> {
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0).max(0);
    if n as u32 > SEND_MAX {
        set_note(tr(crate::Key::TooManyItemsMatchOver10000).into());
        return None;
    }
    _ = fv.SetRedraw(false);
    let done = (0..n).filter(|&k| each(k)).count();
    _ = fv.SetRedraw(true);
    Some(done)
}

pub(crate) unsafe fn run_match(flip: bool) {
    let Some((h, edit, i, path, desel)) = app(|a| a.matcher.as_ref().map(|m| (m.hwnd, m.edit, m.pane, m.path.clone(), m.desel))) else { return };
    let desel = desel != flip;
    let fv = view_of::<IFolderView2>(i).filter(|_| app(|a| i < a.s.panes() && a.s.paths[i] == path));
    let Some(fv) = fv else {
        _ = MessageBeep(MB_ICONWARNING);
        _ = DestroyWindow(h);
        focus_view(app(|a| a.s.active));
        return;
    };
    let text = text_of(edit).trim().to_string();
    let spec = filter_spec(&text);
    if spec.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    app(|a| a.match_last = text.clone());
    let spec = HSTRING::from(spec);
    let mut first = !desel;
    let n = bulk_select(&fv, |k| {
        let Ok(it) = fv.GetItem::<IShellItem>(k) else { return false };
        let attr = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0)).unwrap_or_default().0;
        if real_folder(attr) {
            return false;
        }
        let Ok(name) = it.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).map(|p| pwstr(p)) else { return false };
        if !PathMatchSpecW(&HSTRING::from(name), &spec).as_bool() {
            return false;
        }
        let flags = match (desel, first) {
            (true, _) => SVSI_DESELECT.0,
            (false, true) => SVSI_SELECT.0 | SVSI_FOCUSED.0 | SVSI_ENSUREVISIBLE.0,
            (false, false) => SVSI_SELECT.0,
        };
        let ok = fv.SelectItem(k, flags as u32).is_ok();
        first &= !ok;
        ok
    });
    let Some(n) = n else { return };
    if n == 0 {
        _ = MessageBeep(MB_ICONWARNING);
        set_note(tf(Key::NoMatch, &[("text", &text)]));
        return;
    }
    _ = DestroyWindow(h);
    focus_view(i);
    let n = n.to_string();
    set_note(if desel {
        tf(Key::DeselectedN, &[("n", &n), ("text", &text)])
    } else {
        tf(Key::SelectedN, &[("n", &n), ("text", &text)])
    });
}

pub(crate) unsafe fn invert_selection(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let n = bulk_select(&fv, |k| {
        let Ok(pidl) = fv.Item(k) else { return false };
        let state = fv.GetSelectionState(pidl);
        CoTaskMemFree(Some(pidl as _));
        let Ok(state) = state else { return false };
        let flag = if state & SVSI_SELECT.0 as u32 != 0 { SVSI_DESELECT } else { SVSI_SELECT };
        fv.SelectItem(k, flag.0 as u32).is_ok()
    });
    if let Some(n) = n {
        set_note(tf(Key::InvertN, &[("n", &n.to_string())]));
    }
}

pub(crate) unsafe fn paint_match(h: HWND) {
    let (desel, dpi) = app(|a| (a.matcher.as_ref().is_some_and(|m| m.desel), a.dpi));
    let g = match_geo_now(desel);
    // A theme change redraws this popup. The caption colour has to follow that paint.
    style_dialog_caption(h);
    let font = body_font();
    buffered(h, |dc, rc| {
        paint_window(dc, rc);
        frame(dc, rc, 1, ui_tokens().line);
        paint_meta(dc, g.label, match_label(desel), font);
        paint_field(dc, g.field, dpi, true);
    });
}

pub(crate) extern "system" fn match_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| match_proc_inner(h, msg, wp, lp)))
}
fn match_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        // With two windows open, act on the window this popup belongs to.
        bind_hwnd(h);
        match msg {
            WM_PAINT => paint_match(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_CTLCOLOREDIT => return color_field_edit(HDC(wp.0 as _)),
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| a.matcher = None),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{held_key, match_geo};

    #[test]
    fn held_keys() {
        let (enter, a) = (0x0D, b'A' as u16);
        assert_eq!(held_key(Some(enter), enter, true), (true, Some(enter)));
        assert_eq!(held_key(Some(enter), enter, false), (false, None));
        assert_eq!(held_key(Some(enter), a, false), (false, Some(enter)));
        assert_eq!(held_key(Some(enter), a, true), (false, Some(enter)));
        assert_eq!(held_key(None, enter, true), (false, None));
    }

    #[test]
    fn match_box_fits() {
        for dpi in [96, 144, 192] {
            let s = |v: i32| v * dpi / 96;
            for label_w in [0, s(40), s(400)] {
                let g = match_geo(label_w, s(16), dpi);
                assert!(g.w >= s(320) && g.label.right - g.label.left == label_w);
                assert!(g.field.left >= g.label.right && g.field.right - g.field.left >= s(120) && g.field.right < g.w);
                assert_eq!(g.edit, crate::field_edit_rect(g.field, dpi));
                assert!(g.edit.left > g.field.left && g.edit.right < g.field.right);
                assert!(g.edit.top >= g.field.top && g.edit.bottom <= g.field.bottom && g.field.bottom < g.h);
            }
        }
    }
}
