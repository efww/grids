use crate::*;

// ---------- select by pattern (Num+ / Num− / Num*, like Total Commander's Select group) ----------

// borderless edits sit on the field colour
pub(crate) unsafe fn edit_colors(wp: WPARAM) -> LRESULT {
    let dc = HDC(wp.0 as _);
    SetBkColor(dc, cref(pal().pill));
    SetTextColor(dc, cref(pal().text));
    SetDCBrushColor(dc, cref(pal().pill));
    LRESULT(GetStockObject(DC_BRUSH).0 as isize)
}

// A key that opened a popup or ran something swallows its own auto-repeats until released, so holding it
// neither types into the popup nor reaches the shell. A fresh press of it clears the hold (keyup missed,
// e.g. after Alt+Tab) and goes through. (swallow, next hold)
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

// 320 DIP by default, wider when the label needs it: the field always keeps at least 120 DIP.
pub(crate) fn match_geo(label_w: i32, fh: i32, dpi: i32) -> MatchGeo {
    let s = |v: i32| v * dpi / 96;
    let h = s(32).max(fh + s(14));
    let label = RECT { left: s(12), top: 0, right: s(12) + label_w, bottom: h };
    let w = s(320).max(label.right + s(10) + s(120) + s(6));
    let field = RECT { left: label.right + s(10), top: s(5), right: w - s(6), bottom: h - s(5) };
    let top = (h - fh) / 2;
    let edit = RECT { left: field.left + s(8), top, right: field.right - s(8), bottom: top + fh };
    MatchGeo { w, h, label, field, edit }
}

pub(crate) fn match_label(desel: bool) -> &'static str {
    if desel {
        tr("Deselect", "선택 해제")
    } else {
        tr("Select", "선택")
    }
}

pub(crate) unsafe fn match_geo_now(desel: bool) -> MatchGeo {
    let (font, fh, dpi) = app(|a| (a.font, a.fh, a.dpi));
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &match_label(desel).encode_utf16().collect::<Vec<_>>(), &mut sz);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    match_geo(sz.cx, fh, dpi)
}

// Opens over the top-left of pane i's file list, kept inside the monitor's work area.
pub(crate) unsafe fn open_match(i: usize, desel: bool) {
    let (main, font, open, last, path) = app(|a| (a.main, a.font, a.matcher.is_some(), a.match_last.clone(), a.s.paths[i].clone()));
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    if open {
        return;
    }
    let g = match_geo_now(desel);
    let mut rc = RECT::default();
    _ = GetWindowRect(view_list(&fv), &mut rc);
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let work = mi.rcWork;
    let x = rc.left.min(work.right - g.w).max(work.left);
    let y = rc.top.min(work.bottom - g.h).max(work.top);
    let style = WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsMatch"), w!(""), style, x, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    let edit = child(h, w!("EDIT"), PCWSTR(HSTRING::from(last).as_ptr()),ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), 1, font);
    let e = g.edit;
    _ = MoveWindow(edit, e.left, e.top, e.right - e.left, e.bottom - e.top, false);
    app(|a| a.matcher = Some(Matcher { h, edit, pane: i, path, desel }));
    _ = SetFocus(Some(edit));
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
}

pub(crate) unsafe fn close_match(back: bool) {
    let Some((h, i)) = app(|a| a.matcher.as_ref().map(|m| (m.h, m.pane))) else { return };
    _ = DestroyWindow(h);
    if back {
        focus_view(i);
    }
}

pub(crate) unsafe fn set_note(note: String) {
    // the status line keeps a note until this time; a past time would drop it on the next redraw
    app(|x| x.note = Some((note, Instant::now() + Duration::from_secs(5))));
    update_status();
}

// A note that keeps updating in place (checksum progress): shown like set_note, but not read out at every step.
pub(crate) unsafe fn set_progress_note(note: String) {
    app(|x| {
        let until = Instant::now() + Duration::from_secs(5);
        x.note = Some((note, until));
        x.said = Some(until);
    });
    update_status();
}

// Calls `each` for every item in the view with redraw off; true = that item changed. None: over SEND_MAX,
// nothing touched (and the status line says so).
pub(crate) unsafe fn bulk_select(fv: &IFolderView2, mut each: impl FnMut(i32) -> bool) -> Option<usize> {
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0).max(0);
    if n as u32 > SEND_MAX {
        set_note(tr("Too many items to match (over 10,000)", "항목이 너무 많아 선택하지 않음 (1만 개 초과)").into());
        return None;
    }
    _ = fv.SetRedraw(false);
    let done = (0..n).filter(|&k| each(k)).count();
    _ = fv.SetRedraw(true);
    Some(done)
}

// Enter in the box (Shift+Enter: the other mode). Files and ZIPs whose name matches; folders are left alone.
pub(crate) unsafe fn run_match(flip: bool) {
    let Some((h, edit, i, path, desel)) = app(|a| a.matcher.as_ref().map(|m| (m.h, m.edit, m.pane, m.path.clone(), m.desel))) else { return };
    let desel = desel != flip;
    let fv = view_of::<IFolderView2>(i).filter(|_| app(|a| i < a.s.panes() && a.s.paths[i] == path));
    let Some(fv) = fv else {
        _ = MessageBeep(MB_ICONWARNING); // pane gone or moved on since the box opened
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
    let ko = KO.load(Ordering::Relaxed);
    if n == 0 {
        _ = MessageBeep(MB_ICONWARNING);
        set_note(if ko { format!("맞는 파일 없음: {text}") } else { format!("No match: {text}") });
        return;
    }
    _ = DestroyWindow(h);
    focus_view(i);
    set_note(match (ko, desel) {
        (true, false) => format!("{text} {n}개 선택"),
        (true, true) => format!("{text} {n}개 선택 해제"),
        (false, false) => format!("Selected {n} · {text}"),
        (false, true) => format!("Deselected {n} · {text}"),
    });
}

// Num*: every visible item, folders too, flips its selection.
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
        set_note(if KO.load(Ordering::Relaxed) { format!("선택 반전 ({n}개)") } else { format!("Selection inverted ({n})") });
    }
}

// Insert, as in Total Commander: (the focused item's flags, the next item's). The focused item flips and keeps
// the focus (so the last item stays focused); the next one takes the focus without losing its own selection,
// since SelectItem without SVSI_SELECT deselects (SVSI_DESELECT is 0).
pub(crate) fn ins_flags(cur_sel: bool, next_sel: bool) -> (u32, u32) {
    let flip = if cur_sel { SVSI_DESELECT.0 } else { SVSI_SELECT.0 };
    let keep = if next_sel { SVSI_SELECT.0 } else { SVSI_DESELECT.0 };
    ((flip | SVSI_FOCUSED.0) as u32, (keep | SVSI_FOCUSED.0 | SVSI_ENSUREVISIBLE.0) as u32)
}

unsafe fn is_selected(fv: &IFolderView2, k: i32) -> Option<bool> {
    let pidl = fv.Item(k).ok()?;
    let state = fv.GetSelectionState(pidl);
    CoTaskMemFree(Some(pidl as _));
    state.ok().map(|s| s & SVSI_SELECT.0 as u32 != 0)
}

// Insert: the focused item's selection flips and the focus moves down to the next visible item (in view order).
// No focused item yet: the first one takes the focus, nothing is selected.
pub(crate) unsafe fn mark_and_next(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let visible_after = |k: i32| {
        let mut next = -1;
        let hr = (Interface::vtable(&fv).GetVisibleItem)(Interface::as_raw(&fv), k, false.into(), &mut next);
        (hr == S_OK && (0..n).contains(&next)).then_some(next)
    };
    let cur = fv.GetFocusedItem().unwrap_or(-1);
    if !(0..n).contains(&cur) {
        let Some(first) = visible_after(-1).or((n > 0).then_some(0)) else { return };
        let Some(sel) = is_selected(&fv, first) else { return };
        _ = fv.SelectItem(first, ins_flags(false, sel).1);
        return;
    }
    let Some(cur_sel) = is_selected(&fv, cur) else { return };
    let next = visible_after(cur).and_then(|k| is_selected(&fv, k).map(|s| (k, s)));
    let (cur_flags, next_flags) = ins_flags(cur_sel, next.is_some_and(|(_, s)| s));
    if fv.SelectItem(cur, cur_flags).is_err() {
        return;
    }
    if let Some((k, _)) = next {
        _ = fv.SelectItem(k, next_flags);
    }
}

pub(crate) unsafe fn paint_match(h: HWND) {
    let desel = app(|a| a.matcher.as_ref().is_some_and(|m| m.desel));
    let g = match_geo_now(desel);
    let font = app(|a| a.font);
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        text(dc, g.label, match_label(desel), font, pal().sub, DT_LEFT);
        fill(dc, g.field, pal().pill);
    });
}

pub(crate) extern "system" fn match_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => paint_match(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            // the hold stays: a held Enter must not open files once the list has focus again
            WM_DESTROY => app(|a| a.matcher = None),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}
