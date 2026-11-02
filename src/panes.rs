use crate::*;

// ---------- panes ----------

pub(crate) unsafe fn child(parent: HWND, class: PCWSTR, text: PCWSTR, style: u32, ex: WINDOW_EX_STYLE, id: i32, font: HFONT) -> HWND {
    let style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style);
    let menu = HMENU(id as isize as _);
    let h = CreateWindowExW(ex, class, text, style, 0, 0, 0, 0, Some(parent), Some(menu), None, None).unwrap_or_default();
    SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    h
}

pub(crate) unsafe fn create_pane(i: usize) -> Result<Pane> {
    let (main, font) = app(|a| (a.main, a.font));
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN;
    let h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsPane"), w!(""), style, 0, 0, 0, 0, Some(main), None, None, None)?;
    let (btn, plain) = (BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0));
    let buttons = [
        (ID_BACK, tr("Back", "뒤로"), tr("Back  (Alt+←) · right-click: recent folders", "뒤로  (Alt+←) · 오른쪽 클릭: 최근 폴더")),
        (ID_FWD, tr("Forward", "앞으로"), tr("Forward  (Alt+→) · right-click: recent folders", "앞으로  (Alt+→) · 오른쪽 클릭: 최근 폴더")),
        (ID_UP, tr("Parent folder", "상위 폴더"), tr("Parent folder  (Alt+↑)", "상위 폴더  (Alt+↑)")),
        (ID_OPEN_IN, tr("Open in another pane", "다른 패널에서 열기"), tr("Open in / copy or move to / swap with another pane", "다른 패널에서 열기 / 복사·이동 / 자리 바꾸기")),
        (ID_FAV, tr("Favourite", "즐겨찾기"), tr("Add to favourites (filled star: remove)", "즐겨찾기에 추가 (채워진 별: 빼기)")),
    ];
    for (id, name, tip) in buttons {
        add_tip(child(h, w!("BUTTON"), PCWSTR(HSTRING::from(name).as_ptr()), btn, plain, id, font), tip);
    }
    let edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_EDIT, font);
    // Folder suggestions as you type, like Explorer's address bar; folders only, since navigate() only browses folders
    _ = SHAutoComplete(edit, SHACF_FILESYS_DIRS | SHACF_AUTOSUGGEST_FORCE_ON | SHACF_AUTOAPPEND_FORCE_OFF);
    let filt = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_FILTER, font);
    add_tip(filt, tr("Name filter (Ctrl+F) · icon or Alt+↓: file types", "이름 필터 (Ctrl+F) · 아이콘 또는 Alt+↓: 파일 종류"));
    _ = SetWindowSubclass(filt, Some(filter_hint_proc), 4, 0);
    name_fields(h, i);

    let eb: IExplorerBrowser = CoCreateInstance(&ExplorerBrowser, None, CLSCTX_INPROC_SERVER)?;
    let slot = Arc::new(AtomicUsize::new(i));
    let site: IServiceProvider = Host(slot.clone()).into();
    IUnknown_SetSite(&eb, &site)?; // before Initialize, so the view picks up our ICommDlgBrowser
    let fs = FOLDERSETTINGS { ViewMode: FVM_DETAILS.0 as u32, fFlags: FWF_NOWEBVIEW.0 as u32 };
    eb.Initialize(h, &RECT::default(), Some(&fs))?;
    eb.SetOptions(EBO_NOBORDER)?;
    let ev: IExplorerBrowserEvents = Events(slot.clone()).into();
    let cookie = eb.Advise(&ev)?;
    let helper = CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok(); // drag image over the bar
    let dt: IDropTarget = PaneDrop { slot: slot.clone(), hwnd: h, ok: Cell::new(false), helper }.into();
    _ = RegisterDragDrop(h, &dt);
    let pending = Some(app(|a| a.s.cols[i].clone())); // the session's, for the first folder
    Ok(Pane { hwnd: h, eb, cookie, disp: String::new(), pending, settle: None, recover: None, icon: -1, ticket: 0, slot })
}

// Screen readers otherwise just say "edit", with no word on which of up to 16 panes; redone when a pane changes slot.
unsafe fn name_fields(h: HWND, i: usize) {
    let pane = format!("{} {}", tr("Pane", "패널"), i + 1);
    a11y_name(GetDlgItem(Some(h), ID_EDIT).unwrap_or_default(), Some(&format!("{pane} {}", tr("address", "주소"))));
    a11y_name(GetDlgItem(Some(h), ID_FILTER).unwrap_or_default(), Some(&format!("{pane} {}", tr("name filter (Ctrl+F)", "이름 필터 (Ctrl+F)"))));
}

// The name a screen reader reads for a control (Dynamic Annotation); None takes it off before the window goes.
// Failing only loses the name.
unsafe fn a11y_name(h: HWND, name: Option<&str>) {
    let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) else { return };
    _ = match name {
        Some(n) => acc.SetHwndPropStr(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, PROPID_ACC_NAME, &HSTRING::from(n)),
        None => acc.ClearHwndProps(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, &[PROPID_ACC_NAME]),
    };
}

// While the name filter is empty and not being typed in, it says what it is and how to get there,
// in the field's own font and a muted colour that still reads at 4.5:1 or more on the pill in both themes.
unsafe extern "system" fn filter_hint_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let hint = msg == WM_PAINT && GetWindowTextLengthW(h) == 0 && GetFocus() != h;
    if hint {
        _ = InvalidateRect(Some(h), None, true); // a partial repaint would draw the hint twice over itself
    }
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_SETFOCUS || msg == WM_KILLFOCUS {
        _ = InvalidateRect(Some(h), None, true);
    } else if hint {
        let dc = GetDC(Some(h));
        let mut rc = RECT::default();
        _ = GetClientRect(h, &mut rc);
        rc.left += (SendMessageW(h, EM_GETMARGINS, None, None).0 & 0xffff) as i32;
        let font = SelectObject(dc, HGDIOBJ(SendMessageW(h, WM_GETFONT, None, None).0 as _));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, cref(blend(pal().sub, pal().text, 0.2)));
        let mut hint: Vec<u16> = tr("Filter names  Ctrl+F", "이름 필터  Ctrl+F").encode_utf16().collect();
        DrawTextW(dc, &mut hint, &mut rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS);
        SelectObject(dc, font);
        ReleaseDC(Some(h), dc);
    }
    r
}

pub(crate) unsafe fn destroy_pane(i: usize) {
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    save_view(i);
    let (p, queued) = app(|a| {
        let p = a.panes[i].take();
        if p.as_ref().is_some_and(|p| p.hwnd == a.drop_hover) {
            a.drop_hover = HWND::default();
        }
        if p.as_ref().is_some_and(|p| p.hwnd == a.focus_req) {
            a.focus_req = HWND::default();
        }
        let queued = if a.drop_open.as_ref().is_some_and(|d| d.1 == i) { a.drop_open.take() } else { None };
        (p, queued)
    });
    if let Some(p) = p {
        _ = RevokeDragDrop(p.hwnd);
        _ = p.eb.Unadvise(p.cookie);
        _ = IUnknown_SetSite(&p.eb, None);
        _ = p.eb.Destroy();
        for id in [ID_EDIT, ID_FILTER] {
            a11y_name(GetDlgItem(Some(p.hwnd), id).unwrap_or_default(), None);
        }
        _ = DestroyWindow(p.hwnd);
        let waiting = unqueue(&mut icon_q(), p.hwnd.0 as isize);
        if let Some(pidl) = waiting {
            ILFree(Some(pidl as *const ITEMIDLIST));
        }
    }
    drop(queued);
    if let Ok(mut f) = FILTERS.lock() {
        f[i].clear();
    }
}

// Only the panes visible in the current grid exist; hidden slots keep just their path.
pub(crate) unsafe fn sync_panes() {
    let (main, n) = app(|a| (a.main, a.s.panes()));
    if main.0.is_null() {
        return;
    }
    for i in 0..MAXP {
        let has = app(|a| a.panes[i].is_some());
        if i < n && !has {
            let Ok(p) = create_pane(i) else { continue };
            app(|a| a.panes[i] = Some(p));
            let path = app(|a| a.s.paths[i].clone());
            if path.is_empty() || !go_near(i, &path) {
                navigate(i, THIS_PC);
            }
        } else if i >= n && has {
            destroy_pane(i);
        }
    }
}

// Moves a batch of windows at once (one repaint, no copied bits).
pub(crate) unsafe fn move_all(moves: &[(HWND, RECT)], eb: Option<(&IExplorerBrowser, RECT)>) {
    let Ok(mut dwp) = BeginDeferWindowPos(moves.len() as i32 + 1) else { return };
    for (h, r) in moves.iter().filter(|(h, _)| !h.0.is_null()) {
        match DeferWindowPos(dwp, *h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, MOVE) {
            Ok(d) => dwp = d,
            Err(_) => return,
        }
    }
    if let Some((eb, r)) = eb {
        _ = eb.SetRect(Some(&mut dwp as *mut HDWP), r);
    }
    _ = EndDeferWindowPos(dwp);
}

pub(crate) unsafe fn layout_pane(h: HWND) {
    let Some((eb, dpi, disp)) = app(|a| a.panes.iter().flatten().find(|p| p.hwnd == h).map(|p| (p.eb.clone(), a.dpi, p.disp.clone()))) else {
        return;
    };
    let fh = app(|a| a.fh) + 2;
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let s = |v: i32| v * dpi / 96;
    let pg = pane_geo(rc.right, rc.bottom, dpi);
    // borderless edit, vertically centred inside its pill
    let inside = |p: RECT, l: i32, r: i32| {
        let top = p.top + (p.bottom - p.top - fh) / 2;
        RECT { left: p.left + l, top, right: (p.right - r).max(p.left + l), bottom: top + fh }
    };
    let ctl = |id: i32| GetDlgItem(Some(h), id).unwrap_or_default();
    let mut moves: Vec<(HWND, RECT)> = [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN].iter().zip(pg.btn).map(|(&id, r)| (ctl(id), r)).collect();
    moves.push((ctl(ID_EDIT), inside(pg.addr, crumb_field(pg.addr, dpi).left - pg.addr.left, s(6)))); // where the crumbs start
    moves.push((ctl(ID_FILTER), inside(pg.filt, s(24), s(6))));
    moves.push((ctl(ID_FAV), pg.fav));
    move_all(&moves, Some((&eb, pg.view)));
    show_path_end(ctl(ID_EDIT), &disp);
    sync_addr(h);
}

pub(crate) unsafe fn relayout() {
    sync_panes();
    let Some((main, moves, label, shown)) = app(|a| {
        a.gap_hot = None; // its splitter may be gone; the whole window is repainted below
        if a.main.0.is_null() {
            return None;
        }
        if a.zoom.is_some_and(|z| z != a.s.active || z >= a.s.panes()) {
            a.zoom = None;
        }
        let g = geo(a);
        let s = |v: i32| v * a.dpi / 96;
        let sr = side_rect(&g);
        let tree = RECT { left: sr.left + s(2), top: sr.top + s(4), right: sr.right - s(2), bottom: sr.bottom - s(2) };
        let mut moves = if g.tree { vec![(a.tree_hwnd, tree)] } else { Vec::new() };
        let mut shown = vec![(a.tree_hwnd, g.tree)];
        let mut x = g.pad;
        let tb = |id: i32| unsafe { GetDlgItem(Some(a.main), id) }.unwrap_or_default();
        for (id, w) in [(ID_TB_LAYOUT, s(70)), (ID_TB_SAVE, s(32)), (ID_TB_LOAD, s(32)), (ID_TB_VIEW, s(32)), (ID_TB_FAV, s(32))] {
            moves.push((tb(id), RECT { left: x, top: s(5), right: x + w, bottom: s(5) + s(24) }));
            x += w + s(4);
        }
        let right = g.w - g.pad; // the theme toggle sits at the far right
        moves.push((tb(ID_TB_THEME), RECT { left: right - s(32), top: s(5), right, bottom: s(5) + s(24) }));
        // drive buttons after them; in a narrow window the last letters hide rather than run into the theme toggle
        x += s(6);
        for &(_, _, h) in &a.drives {
            moves.push((h, RECT { left: x, top: s(5), right: x + s(32), bottom: s(5) + s(24) }));
            shown.push((h, x + s(32) <= right - s(32) - s(12)));
            x += s(32) + s(2);
        }
        let full = RECT { left: g.gx, top: g.top, right: g.w - g.pad, bottom: g.bot };
        for (i, p) in a.panes.iter().enumerate().take(a.s.panes()) {
            if let Some(p) = p {
                moves.push((p.hwnd, if a.zoom == Some(i) { full } else { cell_rect(&g, &a.s.heights, i) }));
                shown.push((p.hwnd, a.zoom.is_none_or(|z| z == i)));
            }
        }
        Some((a.main, moves, layout_label(&a.s.heights), shown))
    }) else {
        return;
    };
    move_all(&moves, None);
    for (h, on) in shown {
        _ = ShowWindow(h, if on { SW_SHOW } else { SW_HIDE });
    }
    start_reveal(main); // sidebar size may have changed: keep its selection in view
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        if text_of(b) != label {
            _ = SetWindowTextW(b, &HSTRING::from(label));
        }
    }
    _ = InvalidateRect(Some(main), None, true);
    // Paint now: WM_PAINT is only generated once the message queue is empty, which never happens
    // while the mouse keeps moving, so mid-drag the views would keep showing the previous frame.
    _ = RedrawWindow(Some(main), None, None, RDW_UPDATENOW | RDW_ALLCHILDREN);
}

// Splitter drag shows translucent guide bars where the boundaries will land; the panes move once, on release.
pub(crate) unsafe fn show_bars() {
    let Some((main, bars, rects)) = app(|a| {
        let d = a.drag?;
        let g = geo(a);
        let r = gap_line(&g, d, 3 * a.dpi / 96);
        let rects = if matches!(d, Hit::Row(..)) { [None, r] } else { [r, None] };
        Some((a.main, a.bars, rects))
    }) else {
        return;
    };
    for (b, r) in bars.into_iter().zip(rects) {
        match r {
            Some(r) => {
                let mut p = POINT { x: r.left, y: r.top };
                _ = ClientToScreen(main, &mut p);
                _ = SetWindowPos(b, None, p.x, p.y, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
            None => _ = ShowWindow(b, SW_HIDE),
        }
    }
}

pub(crate) extern "system" fn bar_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

pub(crate) unsafe fn end_drag() {
    let Some(bars) = app(|a| a.drag.take().map(|_| a.bars)) else { return };
    for b in bars {
        _ = ShowWindow(b, SW_HIDE);
    }
    refresh_drives(); // one that came or went mid-drag
    relayout();
    redraw_all();
}

// Full repaint of everything once a drag/resize/layout change is over, so nothing stale survives.
pub(crate) unsafe fn redraw_all() {
    let main = app(|a| a.main);
    if !main.0.is_null() {
        _ = RedrawWindow(Some(main), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
    }
}

pub(crate) unsafe fn set_active(i: usize) {
    let (old, hs) = app(|a| {
        let old = std::mem::replace(&mut a.s.active, i);
        if old != i {
            a.focus_req = HWND::default();
        }
        (old, [old, i].map(|k| a.panes[k].as_ref().map(|p| p.hwnd)))
    });
    if old != i {
        if app(|a| a.zoom.take()).is_some() {
            relayout();
            redraw_all();
        }
        for h in hs.into_iter().flatten() {
            repaint_header(h);
        }
        if let Some(item) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellItem>().ok()) {
            sync_tree(&item);
        }
        mark_drive();
    }
}

pub(crate) unsafe fn repaint_header(pane: HWND) {
    _ = InvalidateRect(Some(pane), None, false);
    for id in [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN, ID_FAV, ID_EDIT, ID_FILTER] {
        if let Ok(c) = GetDlgItem(Some(pane), id) {
            _ = InvalidateRect(Some(c), None, true);
        }
    }
}

pub(crate) unsafe fn set_accent(c: u32) {
    let (bars, panes) = app(|a| {
        if a.s.dark {
            a.s.accent_dark = c;
        } else {
            a.s.accent = c;
        }
        (a.bars, a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>())
    });
    let old = SetClassLongPtrW(bars[0], GCLP_HBRBACKGROUND, CreateSolidBrush(cref(c)).0 as isize);
    if old != 0 {
        _ = DeleteObject(HGDIOBJ(old as _));
    }
    for p in panes {
        repaint_header(p); // all of them: inactive panes show the accent in their filter badge
    }
    for (_, _, h) in app(|a| a.drives.clone()) {
        _ = InvalidateRect(Some(h), None, false); // the active drive's underline
    }
}

// The hosted Explorer list ignores the app-wide dark mode, so its (new, per folder) list view is dyed here.
pub(crate) unsafe fn dark_list(pane: HWND) {
    extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        let mut c = [0u16; 32];
        let n = unsafe { GetClassNameW(h, &mut c) } as usize;
        let class = String::from_utf16_lossy(&c[..n]);
        if class == "SysListView32" || class == "SHELLDLL_DefView" {
            unsafe { (*(out.0 as *mut Vec<(String, HWND)>)).push((class, h)) };
        }
        true.into()
    }
    let mut found: Vec<(String, HWND)> = Vec::new();
    _ = EnumChildWindows(Some(pane), Some(collect), LPARAM(&mut found as *mut _ as isize));
    // the view paints the name column itself (custom draw, light colours); recolour it on the way back
    for (_, v) in found.iter().filter(|(c, _)| c == "SHELLDLL_DefView") {
        _ = SetWindowSubclass(*v, Some(dark_view_proc), 1, 0);
    }
    let lists: Vec<HWND> = found.into_iter().filter(|(c, _)| c == "SysListView32").map(|(_, h)| h).collect();
    let (card, text) = (cref(pal().card).0 as isize, cref(pal().text).0 as isize);
    for lv in lists {
        _ = SetWindowTheme(lv, w!("DarkMode_Explorer"), PCWSTR::null()); // also darkens its scrollbars
        // the sorted-column tint stays white in dark mode
        _ = SetWindowSubclass(lv, Some(dark_list_proc), 2, 0);
        SendMessageW(lv, LVM_SETSELECTEDCOLUMN, Some(WPARAM(usize::MAX)), None);
        SendMessageW(lv, LVM_SETBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(lv, LVM_SETTEXTCOLOR, None, Some(LPARAM(text)));
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        if !header.0.is_null() {
            _ = SetWindowTheme(header, w!("DarkMode_ItemsView"), PCWSTR::null());
        }
        // the info tip over a file, and the full name over a truncated one
        let tip = HWND(SendMessageW(lv, LVM_GETTOOLTIPS, None, None).0 as _);
        if !tip.0.is_null() {
            _ = SetWindowTheme(tip, w!("DarkMode_Explorer"), PCWSTR::null());
        }
        _ = InvalidateRect(Some(lv), None, true);
    }
}

// The view re-sets the sorted column through an internal interface (no window message to catch),
// so it is cleared right before each paint instead. The column header is the list's child, so its
// draw notifications come through here too.
pub(crate) unsafe extern "system" fn dark_list_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let dark = DARK_ON.load(Ordering::Relaxed);
    if msg == WM_PAINT && dark && DefSubclassProc(h, LVM_GETSELECTEDCOLUMN, WPARAM(0), LPARAM(0)).0 as i32 != -1 {
        DefSubclassProc(h, LVM_SETSELECTEDCOLUMN, WPARAM(usize::MAX), LPARAM(0));
    }
    if msg == WM_NOTIFY && dark && (*(lp.0 as *const NMHDR)).code == NM_CUSTOMDRAW {
        let cd = &*(lp.0 as *const NMCUSTOMDRAW);
        if cd.hdr.hwndFrom.0 as isize == DefSubclassProc(h, LVM_GETHEADER, WPARAM(0), LPARAM(0)).0 {
            if cd.dwDrawStage == CDDS_PREPAINT {
                return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
            }
            if cd.dwDrawStage == CDDS_ITEMPREPAINT {
                draw_header_item(cd);
                return LRESULT(CDRF_SKIPDEFAULT as isize);
            }
        }
    }
    DefSubclassProc(h, msg, wp, lp)
}

// The column header's theme draws its labels in the light theme's dark grey whatever colour it is
// given, so in dark mode each column is drawn here: flat background, label, divider, sort chevron.
pub(crate) unsafe fn draw_header_item(cd: &NMCUSTOMDRAW) {
    let (dc, rc, hdr) = (cd.hdc, cd.rc, cd.hdr.hwndFrom);
    let s = |v: i32| v * GetDpiForWindow(hdr) as i32 / 96;
    let hot = cd.uItemState.0 & (CDIS_HOT.0 | CDIS_SELECTED.0) != 0;
    fill(dc, rc, if hot { blend(pal().card, pal().text, 0.08) } else { pal().card });
    fill(dc, RECT { left: rc.right - 1, ..rc }, pal().line);
    let mut text = [0u16; 260];
    let mut item = HDITEMW { mask: HDI_TEXT | HDI_FORMAT, pszText: PWSTR(text.as_mut_ptr()), cchTextMax: text.len() as i32, ..Default::default() };
    SendMessageW(hdr, HDM_GETITEMW, Some(WPARAM(cd.dwItemSpec)), Some(LPARAM(&mut item as *mut _ as isize)));
    let n = text.iter().position(|&c| c == 0).unwrap_or(0);
    if n > 0 {
        let align = if item.fmt.0 & HDF_RIGHT.0 != 0 {
            DT_RIGHT
        } else if item.fmt.0 & HDF_CENTER.0 != 0 {
            DT_CENTER
        } else {
            DT_LEFT
        };
        let mut r = RECT { left: rc.left + s(7), right: rc.right - s(7), ..rc };
        let font = SelectObject(dc, HGDIOBJ(SendMessageW(hdr, WM_GETFONT, None, None).0 as _));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, cref(pal().sub));
        DrawTextW(dc, &mut text[..n], &mut r, align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
        SelectObject(dc, font);
    }
    // sort chevron along the top edge, centred, as Windows 11 draws it
    let up = item.fmt.0 & HDF_SORTUP.0 != 0;
    if up || item.fmt.0 & HDF_SORTDOWN.0 != 0 {
        let (cx, t, arm) = ((rc.left + rc.right) / 2, s(1).max(1), s(4));
        for k in 0..=arm {
            let y = rc.top + s(1) + if up { k } else { arm - k };
            fill(dc, RECT { left: cx - k, top: y, right: cx - k + t, bottom: y + t }, pal().sub);
            fill(dc, RECT { left: cx + k, top: y, right: cx + k + t, bottom: y + t }, pal().sub);
        }
    }
}

pub(crate) unsafe extern "system" fn dark_view_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_NOTIFY && DARK_ON.load(Ordering::Relaxed) && (*(lp.0 as *const NMHDR)).code == NM_CUSTOMDRAW {
        let cd = &mut *(lp.0 as *mut NMLVCUSTOMDRAW);
        let stage = cd.nmcd.dwDrawStage.0;
        // a group title comes as its own (pre)paint stage; what isn't drawn here keeps the default drawing
        if cd.dwItemType == LVCDI_GROUP && (stage == CDDS_PREPAINT.0 || stage == CDDS_ITEMPREPAINT.0) && draw_group_header(cd) {
            return LRESULT(CDRF_SKIPDEFAULT as isize);
        }
        if stage == CDDS_PREPAINT.0 {
            return LRESULT(r.0 | CDRF_NOTIFYITEMDRAW as isize);
        }
        if stage & CDDS_ITEMPREPAINT.0 == CDDS_ITEMPREPAINT.0 {
            (cd.clrText, cd.clrTextBk) = (cref(pal().text), cref(pal().card));
            if stage & CDDS_SUBITEM.0 == 0 {
                return LRESULT(r.0 | CDRF_NOTIFYSUBITEMDRAW as isize);
            }
        }
    }
    r
}

// The themed list draws group titles ("Devices and drives" in This PC) in the light theme's navy
// whatever colour it is given, so in dark mode a plain titled group is drawn here like the column
// header: flat background, light title, a thin rule after it, the collapse chevron. Images, "show more"
// or a failed lookup leave the group to the default drawing (false). Clicks, folding and keys stay with
// the list. Only the title is read: asking a shell view's list for a subtitle, task, description or
// footer faults inside comctl32 (6.10.26100), which ends this paint or the app; shell groups have none.
pub(crate) unsafe fn draw_group_header(cd: &NMLVCUSTOMDRAW) -> bool {
    let (dc, lv, id) = (cd.nmcd.hdc, cd.nmcd.hdr.hwndFrom, cd.nmcd.dwItemSpec);
    let mut head = [0u16; 260];
    let mask = [LVGF_HEADER, LVGF_ALIGN, LVGF_STATE, LVGF_TITLEIMAGE, LVGF_EXTENDEDIMAGE];
    let states = LVGS_COLLAPSIBLE.0 | LVGS_COLLAPSED.0 | LVGS_SELECTED.0 | LVGS_FOCUSED.0 | LVGS_SUBSETED.0;
    let mut g = LVGROUP {
        cbSize: size_of::<LVGROUP>() as u32,
        mask: LVGROUP_MASK(mask.iter().fold(0, |m, f| m | f.0)),
        stateMask: LIST_VIEW_GROUP_STATE_FLAGS(states),
        pszHeader: PWSTR(head.as_mut_ptr()),
        cchHeader: head.len() as i32,
        ..Default::default()
    };
    if SendMessageW(lv, LVM_GETGROUPINFO, Some(WPARAM(id)), Some(LPARAM(&mut g as *mut _ as isize))).0 == -1 {
        return false;
    }
    let st = g.state.0;
    let n = head.iter().position(|&c| c == 0).unwrap_or(0);
    if n == 0 || st & LVGS_SUBSETED.0 != 0 {
        return false;
    }
    let images = SendMessageW(lv, LVM_GETIMAGELIST, Some(WPARAM(LVSIL_GROUPHEADER as usize)), None).0 != 0;
    if images && (g.iTitleImage >= 0 || g.iExtendedImage >= 0) {
        return false;
    }
    let mut rc = RECT { top: LVGGR_HEADER as i32, ..Default::default() };
    if SendMessageW(lv, LVM_GETGROUPRECT, Some(WPARAM(id)), Some(LPARAM(&mut rc as *mut _ as isize))).0 == 0 || rc.right <= rc.left || rc.bottom <= rc.top {
        return false;
    }
    let s = |v: i32| v * GetDpiForWindow(lv) as i32 / 96;
    let bg = if st & LVGS_SELECTED.0 != 0 {
        blend(pal().card, pal().text, 0.16)
    } else if cd.nmcd.uItemState.0 & CDIS_HOT.0 != 0 {
        blend(pal().card, pal().text, 0.08)
    } else {
        pal().card
    };
    fill(dc, rc, bg);
    // the list keeps a square collapse button at the right end and hit-tests it there
    let collapsible = st & LVGS_COLLAPSIBLE.0 != 0;
    let bx = if collapsible { rc.bottom - rc.top } else { 0 };
    let r = RECT { left: rc.left + s(8), right: rc.right - bx - s(8), ..rc };
    let align = if g.uAlign.0 & LVGA_HEADER_RIGHT.0 != 0 {
        DT_RIGHT
    } else if g.uAlign.0 & LVGA_HEADER_CENTER.0 != 0 {
        DT_CENTER
    } else {
        DT_LEFT
    };
    // the item count "(3)" is part of the shell's title string, so it is drawn as given
    let font = HFONT(SendMessageW(lv, WM_GETFONT, None, None).0 as _);
    text(dc, r, &String::from_utf16_lossy(&head[..n]), font, pal().text, align);
    let cy = (rc.top + rc.bottom) / 2;
    if align == DT_LEFT {
        let old = SelectObject(dc, font.into());
        let mut sz = SIZE::default();
        _ = GetTextExtentPoint32W(dc, &head[..n], &mut sz);
        SelectObject(dc, old);
        let x = r.left + sz.cx + s(8);
        if x < r.right {
            fill(dc, RECT { left: x, top: cy, right: r.right, bottom: cy + 1 }, pal().line);
        }
    }
    if collapsible {
        let up = st & LVGS_COLLAPSED.0 == 0; // ∧ while open, ∨ when folded
        let (cx, t, arm) = (rc.right - bx / 2, s(1).max(1), s(4));
        for k in 0..=arm {
            let y = cy - arm / 2 + if up { k } else { arm - k };
            fill(dc, RECT { left: cx - k, top: y, right: cx - k + t, bottom: y + t }, pal().sub);
            fill(dc, RECT { left: cx + k, top: y, right: cx + k + t, bottom: y + t }, pal().sub);
        }
    }
    if cd.nmcd.uItemState.0 & CDIS_FOCUS.0 != 0 || st & LVGS_FOCUSED.0 != 0 {
        frame(dc, rc, 1, pal().sub);
    }
    true
}

// Undocumented but long-stable uxtheme exports (Windows 10 1903+): app-wide dark mode, which is what
// makes menus, scrollbars and the hosted Explorer views follow the theme.
pub(crate) unsafe fn uxtheme(ordinal: usize) -> FARPROC {
    let m = LoadLibraryW(w!("uxtheme.dll")).ok()?;
    GetProcAddress(m, PCSTR(ordinal as *const u8))
}

// Undoes dark_list on a pane's current list: what a list made in the light theme has. Its subclasses
// stay on but check DARK_ON, so they do nothing now.
pub(crate) unsafe fn light_list(pane: HWND) {
    extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        let mut c = [0u16; 32];
        let n = unsafe { GetClassNameW(h, &mut c) } as usize;
        if String::from_utf16_lossy(&c[..n]) == "SysListView32" {
            unsafe { (*(out.0 as *mut Vec<HWND>)).push(h) };
        }
        true.into()
    }
    let mut lists: Vec<HWND> = Vec::new();
    _ = EnumChildWindows(Some(pane), Some(collect), LPARAM(&mut lists as *mut _ as isize));
    let (bg, text) = (GetSysColor(COLOR_WINDOW) as isize, GetSysColor(COLOR_WINDOWTEXT) as isize);
    for lv in lists {
        _ = SetWindowTheme(lv, w!("Explorer"), PCWSTR::null());
        SendMessageW(lv, LVM_SETBKCOLOR, None, Some(LPARAM(bg)));
        SendMessageW(lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(bg)));
        SendMessageW(lv, LVM_SETTEXTCOLOR, None, Some(LPARAM(text)));
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        if !header.0.is_null() {
            _ = SetWindowTheme(header, w!("ItemsView"), PCWSTR::null());
        }
        let tip = HWND(SendMessageW(lv, LVM_GETTOOLTIPS, None, None).0 as _);
        if !tip.0.is_null() {
            _ = SetWindowTheme(tip, PCWSTR::null(), PCWSTR::null());
        }
        _ = InvalidateRect(Some(lv), None, true);
    }
}

// Windows' "app mode" light/dark (Settings > Personalization > Colors); None where it isn't set.
pub(crate) unsafe fn system_dark() -> Option<bool> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let (mut v, mut n) = (0u32, 4u32);
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let r = RegGetValueW(HKEY_CURRENT_USER, key, w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as _), Some(&mut n));
    r.is_ok().then_some(v == 0)
}

// Switches the theme in place: panes keep their views, so history, filter, selection, scroll and a
// half-typed address all stay. A view still navigating gets the theme from on_nav when it lands.
// Windows 11's newer list (DirectUIHWND, see in_view_list) takes light or dark only when it is made and
// can't be recoloured in place, so there the panes are made again on their folders, as at start-up.
pub(crate) unsafe fn retheme() {
    let (panes, popups) = app(|a| (a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>(), [a.fav_popup, a.load, a.grid, a.keys_popup, a.peek, a.matcher.as_ref().map_or(HWND::default(), |m| m.h)]));
    if (0..MAXP).any(|i| unsafe { dui_list(i) }) {
        apply_theme();
    } else {
        theme_chrome();
        for p in panes {
            if DARK_ON.load(Ordering::Relaxed) {
                dark_list(p);
            } else {
                light_list(p);
            }
        }
        redraw_all();
    }
    // open popups are owned, not children: repainted as they are
    for h in popups.into_iter().filter(|h| IsWindow(Some(*h)).as_bool()) {
        _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
    }
}

// Following Windows: switch if its setting differs from Grids' (the setting broadcast comes in bursts).
pub(crate) unsafe fn theme_sync() {
    let (follow, cur) = app(|a| (a.s.follow, a.s.dark));
    if let Some(d) = theme_target(follow, system_dark(), cur) {
        app(|a| a.s.dark = d);
        retheme();
    }
}

// Applies the current theme everywhere; panes are rebuilt so their Explorer views pick it up.
pub(crate) unsafe fn apply_theme() {
    theme_chrome();
    for i in 0..MAXP {
        destroy_pane(i);
    }
    relayout();
    redraw_all();
}

// Everything but the panes' Explorer lists: tooltips, app mode (menus, scrollbars), title bar, tree, accent.
pub(crate) unsafe fn theme_chrome() {
    let (main, dark, tree, accent, tip) = app(|a| (a.main, a.s.dark, a.tree_hwnd, a.s.accent_now(), a.tip));
    // tooltips: dark on dark, back to the default on light (these windows outlive the theme switch)
    let tip_theme = |t: HWND| {
        if !t.0.is_null() {
            _ = SetWindowTheme(t, if dark { w!("DarkMode_Explorer") } else { PCWSTR::null() }, PCWSTR::null());
        }
    };
    tip_theme(tip);
    DARK_ON.store(dark, Ordering::Relaxed);
    if let Some(f) = uxtheme(135) {
        let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        set_preferred_app_mode(if dark { 2 } else { 3 }); // ForceDark / ForceLight
    }
    if let Some(f) = uxtheme(136) {
        let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(f);
        flush_menu_themes();
    }
    let on = BOOL::from(dark);
    _ = DwmSetWindowAttribute(main, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4); // immersive dark title bar
    let caption = cref(pal().bg);
    _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    if let Ok(tv) = FindWindowExW(Some(tree), None, w!("SysTreeView32"), PCWSTR::null()) {
        _ = SetWindowTheme(tv, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, PCWSTR::null());
        let colour = |c: u32| LPARAM(if dark { cref(c).0 as isize } else { -1 }); // -1: system default
        SendMessageW(tv, TVM_SETBKCOLOR, None, Some(colour(pal().card)));
        SendMessageW(tv, TVM_SETTEXTCOLOR, None, Some(colour(pal().text)));
        tip_theme(HWND(SendMessageW(tv, TVM_GETTOOLTIPS, None, None).0 as _));
    }
    set_accent(accent);
    rename_theme();
}

pub(crate) unsafe fn pick_accent() {
    let (main, cur) = app(|a| (a.main, a.s.accent_now()));
    let mut custom = [COLORREF(0xFFFFFF); 16];
    let mut cc = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: main,
        rgbResult: cref(cur),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    if ChooseColorW(&mut cc).as_bool() {
        set_accent(cref(cc.rgbResult.0).0); // COLORREF is BGR; cref swaps back
    }
}

// Bumped on every navigation event and pane teardown, so a click pair spanning a folder change is dropped.
pub(crate) static NAV_GEN: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn gen() -> usize {
    NAV_GEN.load(Ordering::SeqCst)
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Click {
    pub(crate) win: isize,
    pub(crate) gen: usize,
    pub(crate) time: u32,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

thread_local! {
    pub(crate) static LAST_CLICK: Cell<Option<Click>> = const { Cell::new(None) };
}

// Windows 11's list (DirectUIHWND) has no CS_DBLCLKS, so a double-click is two background presses in a row,
// within the system's double-click time and box. The pair is consumed: a third click doesn't go up again.
pub(crate) fn next_click(prev: Option<Click>, now: Click, bg: bool, ms: u32, cx: i32, cy: i32) -> (Option<Click>, bool) {
    if !bg {
        return (None, false);
    }
    match prev {
        Some(p)
            if p.win == now.win
                && p.gen == now.gen
                && now.time.wrapping_sub(p.time) <= ms
                && (now.x - p.x).abs() <= cx / 2
                && (now.y - p.y).abs() <= cy / 2 =>
        {
            (None, true)
        }
        _ => (Some(now), false),
    }
}

// The list itself is background, and so may a row be: a row reports ROLE_SYSTEM_LISTITEM across all its columns,
// yet without full-row select only the name is the item, so its date and size cells are empty space. confirm()
// tells them apart after the click: a press on the name selects or opens it. Group headers, collapse buttons and
// column headers are never background. The column header bar is a list too, a strip along the top: its gaps and
// the dividers between columns (double-click: fit the width) report it, so a list counts only when it is most of
// the window (`h` its height, `list_h` the window's).
pub(crate) fn bg_role(role: u32, h: i32, list_h: i32) -> bool {
    role == ROLE_SYSTEM_LISTITEM || role == ROLE_SYSTEM_LIST && h * 2 > list_h
}

// The generation is read before the probe and again after it: a navigation during the probe voids it.
pub(crate) fn bg_probe(gen: impl Fn() -> usize, probe: impl FnOnce() -> bool) -> (usize, bool) {
    let g0 = gen();
    let b = probe();
    (g0, b && gen() == g0)
}

// After dispatch: same view window, nothing selected, and no navigation since the click. The generation is
// read last, after the queries that could run navigation callbacks.
pub(crate) fn confirm(cand: (usize, isize), query: impl FnOnce() -> (Option<isize>, Option<u32>), gen: impl Fn() -> usize) -> bool {
    let (view, sel) = query();
    view == Some(cand.1) && sel == Some(0) && gen() == cand.0
}

// Same-thread window, so WM_GETOBJECT runs inline; call outside app().
pub(crate) unsafe fn on_background(m: &MSG) -> bool {
    if WindowFromPoint(m.pt) != m.hwnd {
        return false;
    }
    let mut acc: Option<IAccessible> = None;
    let mut child = VARIANT::default();
    if AccessibleObjectFromPoint(m.pt, &mut acc, &mut child).is_err() {
        return false;
    }
    let mut win = RECT::default();
    _ = GetWindowRect(m.hwnd, &mut win);
    let bg = acc.and_then(|a| {
        let (mut l, mut t, mut w, mut h) = (0, 0, 0, 0);
        _ = a.accLocation(&mut l, &mut t, &mut w, &mut h, &child);
        Some((a.get_accRole(&child).ok()?, h))
    });
    let bg = bg.is_some_and(|(mut role, h)| {
        let r = &role.Anonymous.Anonymous;
        let bg = r.vt == VT_I4 && bg_role(r.Anonymous.lVal as u32, h, win.bottom - win.top);
        _ = VariantClear(&mut role);
        bg
    });
    _ = VariantClear(&mut child);
    bg
}

// A double-click on empty space in a list goes up a folder: (pane, generation, view window) when this press
// completes a background pair.
pub(crate) unsafe fn up_candidate(m: &MSG) -> Option<(usize, usize, isize)> {
    if !matches!(m.message, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) || !app(|a| a.s.dblup) {
        return None;
    }
    let i = pane_of(m.hwnd)?;
    let svh = Cell::new(HWND::default());
    let (g0, bg) = bg_probe(gen, || {
        let Some(h) = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok()) else {
            return false;
        };
        svh.set(h);
        in_view_list(h, m.hwnd)
            && m.wParam.0 & (MK_CONTROL.0 | MK_SHIFT.0) as usize == 0
            && on_background(m)
    });
    let now = Click { win: m.hwnd.0 as isize, gen: g0, time: m.time, x: m.pt.x, y: m.pt.y };
    let (cx, cy) = (GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK));
    let (next, up) = next_click(LAST_CLICK.get(), now, bg, GetDoubleClickTime(), cx, cy);
    LAST_CLICK.set(next);
    up.then(|| (i, g0, svh.get().0 as isize))
}

pub(crate) unsafe fn cycle(d: i32) {
    let (a, n) = app(|a| (a.s.active as i32, a.s.panes() as i32));
    let i = (a + d).rem_euclid(n) as usize;
    set_active(i);
    focus_view(i);
}

/// The slot of the nearest cell wholly to one side of `cur` (dx/dy = -1, 0 or 1) that overlaps it across that axis.
pub(crate) fn neighbour(cells: &[(usize, RECT)], cur: usize, dx: i32, dy: i32) -> Option<usize> {
    let &(_, c) = cells.iter().find(|(s, _)| *s == cur)?;
    let mid = |r: &RECT| if dx != 0 { r.top + r.bottom } else { r.left + r.right };
    cells
        .iter()
        .filter(|(s, _)| *s != cur)
        .filter_map(|&(s, r)| {
            let gap = match (dx, dy) {
                (-1, _) => c.left - r.right,
                (1, _) => r.left - c.right,
                (_, -1) => c.top - r.bottom,
                _ => r.top - c.bottom,
            };
            let overlap = if dx != 0 { r.top.max(c.top) < r.bottom.min(c.bottom) } else { r.left.max(c.left) < r.right.min(c.right) };
            (gap >= 0 && overlap).then_some((gap, (mid(&r) - mid(&c)).abs(), s))
        })
        .min()
        .map(|(_, _, s)| s)
}

// Like Windows Terminal's Alt+arrow or tmux's prefix+arrow: move to the pane on screen in that direction.
pub(crate) unsafe fn move_pane(dx: i32, dy: i32) {
    if let Some(j) = active_neighbour(dx, dy) {
        set_active(j);
        focus_view(j);
    }
}

// The live pane on screen next to the active one in that direction.
fn active_neighbour(dx: i32, dy: i32) -> Option<usize> {
    let (cells, cur) = app(|a| {
        let g = geo(a);
        let cells = (0..a.s.panes()).filter(|&i| a.panes[i].is_some()).map(|i| (i, cell_rect(&g, &a.s.heights, i))).collect::<Vec<_>>();
        (cells, a.s.active)
    });
    neighbour(&cells, cur, dx, dy)
}

// Ctrl+Alt+Shift+arrow: the active pane trades places with the one on screen in that direction, and stays active.
pub(crate) unsafe fn swap_dir(dx: i32, dy: i32) {
    if let Some(j) = active_neighbour(dx, dy) {
        swap_panes(app(|a| a.s.active), j);
    }
}

// Like Windows Terminal's swapPane or tmux's swap-pane: two panes trade places on screen. Nothing is navigated or
// rebuilt, so each keeps its folder, history, selection, scroll, filter and columns; only what is kept by slot moves.
pub(crate) unsafe fn swap_panes(i: usize, j: usize) {
    let Some([si, sj]) = app(|a| {
        let live = |k: usize| k < a.s.panes() && a.panes[k].is_some();
        (i != j && live(i) && live(j) && a.drag.is_none()).then(|| [i, j].map(|k| a.panes[k].as_ref().map(|p| p.slot.clone()).unwrap_or_default()))
    }) else {
        return;
    };
    {
        // filters and slots together: IncludeObject on a view thread sees both before or both after
        let mut f = FILTERS.lock().unwrap_or_else(|e| e.into_inner());
        f.swap(i, j);
        si.store(j, Ordering::SeqCst);
        sj.store(i, Ordering::SeqCst);
    }
    app(|a| {
        a.panes.swap(i, j);
        swap_in(&mut a.s, a.base.as_mut(), i, j);
        a.zoom = None;
        let to = |k: &mut usize| *k = if *k == i { j } else if *k == j { i } else { *k };
        if let Some(d) = a.drop_open.as_mut() {
            to(&mut d.1);
        }
        if let Some(m) = a.matcher.as_mut() {
            to(&mut m.pane);
        }
        if let Some(k) = a.kids.as_mut() {
            to(&mut k.i);
        }
        to(&mut a.peek_pane);
        a.status_key.clear();
        a.dirty = session_dirty(&a.s, a.base.as_ref());
    });
    for k in [i, j] {
        SEL_REV[k].fetch_add(1, Ordering::Relaxed);
        if let Some(h) = app(|a| a.panes[k].as_ref().map(|p| p.hwnd)) {
            name_fields(h, k);
        }
    }
    NAV_GEN.fetch_add(1, Ordering::SeqCst); // a double-click pair that spans the move is dropped
    relayout();
    redraw_all();
    refresh_save_button();
    set_active(j); // the pane that was at i
    focus_view(j);
}

// Like tmux's pane zoom: the active pane takes the whole grid until toggled back or another pane is picked.
pub(crate) unsafe fn toggle_zoom() {
    if app(|a| a.drag.is_some() || a.s.panes() <= 1) {
        return;
    }
    let i = app(|a| {
        a.zoom = if a.zoom.is_some() { None } else { Some(a.s.active) };
        a.s.active
    });
    relayout();
    redraw_all();
    focus_view(i);
}

// Like Explorer's navigation pane toggle: the grid takes the sidebar's room; its width is kept for when it comes back.
pub(crate) unsafe fn toggle_tree() {
    if app(|a| a.drag.is_some()) {
        return;
    }
    let f = GetFocus();
    let (had_focus, i) = app(|a| {
        a.s.tree ^= true;
        (a.tree_hwnd == f || IsChild(a.tree_hwnd, f).as_bool(), a.s.active)
    });
    relayout();
    redraw_all();
    if had_focus {
        focus_view(i);
    }
}

/// Splitter release stores the clamped sidebar width; while the tree is hidden the kept width stays.
pub(crate) fn kept_side(tree: bool, stored: i32, clamped: i32) -> i32 {
    if tree { clamped } else { stored }
}

// Panes keep their place on screen (remap); like swap_panes, everything kept by slot moves with them.
pub(crate) unsafe fn set_layout(h: Vec<usize>) {
    app(|a| a.focus_req = HWND::default());
    for i in 0..MAXP {
        save_view(i);
    }
    let (perm, slots) = app(|a| {
        let (perm, home) = remap(&a.s.heights, &h, &a.home);
        a.home = home;
        (perm, perm.map(|k| a.panes[k].as_ref().map(|p| p.slot.clone())))
    });
    {
        // filters and slots together, as in swap_panes
        let mut f = FILTERS.lock().unwrap_or_else(|e| e.into_inner());
        let old = f.clone();
        for (j, &k) in perm.iter().enumerate() {
            f[j] = old[k].clone();
            if let Some(s) = &slots[j] {
                s.store(j, Ordering::SeqCst);
            }
        }
    }
    let revs = SEL_REV.each_ref().map(|r| r.load(Ordering::Relaxed));
    for (j, &k) in perm.iter().enumerate() {
        SEL_REV[j].store(revs[k], Ordering::Relaxed);
    }
    let gone = app(|a| {
        let (old_h, old_active) = (a.s.heights.clone(), a.s.active);
        let mut taken: Vec<Option<Pane>> = a.panes.iter_mut().map(Option::take).collect();
        a.panes = perm.iter().map(|&k| taken[k].take()).collect();
        permute_in(&mut a.s, a.base.as_mut(), &perm);
        let to = |k: &mut usize| *k = perm.iter().position(|&p| p == *k).unwrap_or(*k);
        if let Some(d) = a.drop_open.as_mut() {
            to(&mut d.1);
        }
        if let Some(m) = a.matcher.as_mut() {
            to(&mut m.pane);
        }
        if let Some(k) = a.kids.as_mut() {
            to(&mut k.i);
        }
        to(&mut a.peek_pane);
        a.zoom = None;
        a.status_key.clear();
        let n: usize = h.iter().sum();
        let gone = a.s.active >= n;
        if gone {
            // the active pane's cell went: the nearest cell left takes over
            let (c, r) = slot(&old_h, old_active);
            let c = c.min(h.len() - 1);
            a.s.active = h[..c].iter().sum::<usize>() + r.min(h[c] - 1);
        }
        a.s.set_heights(h);
        gone
    });
    for (j, &k) in perm.iter().enumerate() {
        // a pane that changed slot is read out under its new number, as in swap_panes
        if let Some(pw) = app(|a| a.panes[j].as_ref().map(|p| p.hwnd)).filter(|_| j != k) {
            name_fields(pw, j);
        }
    }
    NAV_GEN.fetch_add(1, Ordering::SeqCst); // a double-click pair that spans the move is dropped
    relayout();
    redraw_all();
    if gone {
        let i = app(|a| a.s.active);
        if view_of::<IShellView>(i).is_some() {
            if let Some(item) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellItem>().ok()) {
                sync_tree(&item);
            }
            focus_view(i);
        } else {
            // a pane made just now for that cell: on_nav focuses it once its folder shows
            app(|a| a.focus_req = a.panes[i].as_ref().map(|p| p.hwnd).unwrap_or_default());
        }
        mark_drive();
    }
}

// Same switches as Explorer's View > Show "Hidden items" and "File name extensions"
// (global shell settings): fShowAllObjects is bit 0, fShowExtensions bit 1.
unsafe fn shell_flag(ssf: SSF_MASK, bit: i32) -> bool {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), ssf, false);
    ss._bitfield1 & bit != 0
}

unsafe fn toggle_shell_flag(ssf: SSF_MASK, bit: i32) {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), ssf, false);
    ss._bitfield1 ^= bit;
    SHGetSetSettings(Some(&mut ss), ssf, true);
    for i in 0..MAXP {
        if let Some(sv) = view_of::<IShellView>(i) {
            _ = sv.Refresh();
        }
    }
}

pub(crate) unsafe fn hidden() -> bool {
    shell_flag(SSF_SHOWALLOBJECTS, 1)
}

pub(crate) unsafe fn toggle_hidden() {
    toggle_shell_flag(SSF_SHOWALLOBJECTS, 1)
}

pub(crate) unsafe fn extensions() -> bool {
    shell_flag(SSF_SHOWEXTENSIONS, 2)
}

pub(crate) unsafe fn toggle_extensions() {
    toggle_shell_flag(SSF_SHOWEXTENSIONS, 2)
}
