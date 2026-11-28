use crate::*;

// ---------- panes ----------

pub(crate) unsafe fn child(parent: HWND, class: PCWSTR, text: PCWSTR, style: u32, ex: WINDOW_EX_STYLE, id: i32, font: HFONT) -> HWND {
    let style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style);
    let menu = HMENU(id as isize as _);
    let h = CreateWindowExW(ex, class, text, style, 0, 0, 0, 0, Some(parent), Some(menu), None, None).unwrap_or_default();
    SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    h
}

// A pane header's buttons, left to right: (id, name, tooltip), English and Korean. The Ctrl hints (keyboard.rs) take
// their keys from these tooltips, so a keycap can't name another key than its tooltip does.
pub(crate) const PANE_TIPS: [(i32, (&str, &str), (&str, &str)); 5] = [
    (ID_BACK, ("Back", "뒤로"), ("Back  (Alt+←) · right-click: recent folders", "뒤로  (Alt+←) · 오른쪽 클릭: 최근 폴더")),
    (ID_FWD, ("Forward", "앞으로"), ("Forward  (Alt+→) · right-click: recent folders", "앞으로  (Alt+→) · 오른쪽 클릭: 최근 폴더")),
    (ID_UP, ("Parent folder", "상위 폴더"), ("Parent folder  (Alt+↑)", "상위 폴더  (Alt+↑)")),
    (ID_OPEN_IN, ("Open in another pane", "다른 패널에서 열기"), ("Open in / copy or move to / swap with another pane  (Ctrl+M)", "다른 패널에서 열기 / 복사·이동 / 자리 바꾸기  (Ctrl+M)")),
    (ID_FAV, ("Favourite", "즐겨찾기"), ("Add to favourites (filled star: remove)  (Ctrl+Shift+B)", "즐겨찾기에 추가 (채워진 별: 빼기)  (Ctrl+Shift+B)")),
];
pub(crate) const FILTER_TIP: (&str, &str) = ("Name filter (Ctrl+F) · icon or Alt+↓: file types", "이름 필터 (Ctrl+F) · 아이콘 또는 Alt+↓: 파일 종류");

pub(crate) unsafe fn create_pane(i: usize) -> Result<Pane> {
    let (main, font) = app(|a| (a.main, a.font));
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN;
    let h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsPane"), w!(""), style, 0, 0, 0, 0, Some(main), None, None, None)?;
    let (btn, plain) = (BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0));
    for (id, name, tip) in PANE_TIPS {
        add_tip(child(h, w!("BUTTON"), PCWSTR(HSTRING::from(tr(name.0, name.1)).as_ptr()), btn, plain, id, font), tr(tip.0, tip.1));
    }
    let edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_EDIT, font);
    // Folder suggestions as you type, like Explorer's address bar; folders only, since navigate() only browses folders
    _ = SHAutoComplete(edit, SHACF_FILESYS_DIRS | SHACF_AUTOSUGGEST_FORCE_ON | SHACF_AUTOAPPEND_FORCE_OFF);
    let filt = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_FILTER, font);
    add_tip(filt, tr(FILTER_TIP.0, FILTER_TIP.1));
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
    let dt: IDropTarget = PaneDrop { slot: slot.clone(), hwnd: h, ok: Cell::new(false), helper, relay: Relay::default(), lit: Cell::new(false) }.into();
    _ = RegisterDragDrop(h, &dt);
    let pending = Some(app(|a| a.s.cols[i].clone())); // the session's, for the first folder
    Ok(Pane { hwnd: h, eb, cookie, disp: String::new(), pending, settle: None, recover: None, follow: None, unplugged: None, icon: -1, ticket: 0, slot, spin: None, loading: None, load_framed: false, hist: None, root: false, going: None, moving: None, wait_first: false, pace: FilterPace::default() })
}

// Screen readers otherwise just say "edit", with no word on which of up to 16 panes; redone when a pane changes slot.
unsafe fn name_fields(h: HWND, i: usize) {
    let pane = format!("{} {}", tr("Pane", "패널"), i + 1);
    a11y_name(GetDlgItem(Some(h), ID_EDIT).unwrap_or_default(), Some(&format!("{pane} {}", tr("address", "주소"))));
    a11y_name(GetDlgItem(Some(h), ID_FILTER).unwrap_or_default(), Some(&format!("{pane} {}", tr("name filter (Ctrl+F)", "이름 필터 (Ctrl+F)"))));
}

// The name a screen reader reads for a control (Dynamic Annotation); None takes it off before the window goes.
// Failing only loses the name.
pub(crate) unsafe fn a11y_name(h: HWND, name: Option<&str>) {
    let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) else { return };
    _ = match name {
        Some(n) => acc.SetHwndPropStr(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, PROPID_ACC_NAME, &HSTRING::from(n)),
        None => acc.ClearHwndProps(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, &[PROPID_ACC_NAME]),
    };
}

// The role a screen reader gives a drawn popup (the ★ list is a list).
pub(crate) unsafe fn a11y_role(h: HWND, role: u32) {
    let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) else { return };
    _ = acc.SetHwndProp(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, PROPID_ACC_ROLE, &VARIANT::from(role as i32));
}

// A new name, said out loud: the focus stays where it is, so a changed name alone is not read.
// Like a11y_name, never inside app(): the reader may call straight back in.
pub(crate) unsafe fn a11y_say(h: HWND, name: &str, say: bool) {
    a11y_name(h, Some(name));
    if say {
        NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, h, OBJID_CLIENT.0, CHILDID_SELF as i32);
    }
}

// What a reader adds after the name (the keys a popup takes); None takes it off.
pub(crate) unsafe fn a11y_desc(h: HWND, desc: Option<&str>) {
    let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) else { return };
    _ = match desc {
        Some(d) => acc.SetHwndPropStr(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, PROPID_ACC_DESCRIPTION, &HSTRING::from(d)),
        None => acc.ClearHwndProps(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, &[PROPID_ACC_DESCRIPTION]),
    };
}

// Name, role and description off before a popup goes, so a reused HWND value doesn't inherit them.
pub(crate) unsafe fn a11y_clear(h: HWND) {
    let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) else { return };
    _ = acc.ClearHwndProps(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, &[PROPID_ACC_NAME, PROPID_ACC_ROLE, PROPID_ACC_DESCRIPTION]);
}

// filter_hint_proc's ref data: the shortcut window's find box (0: a pane's name filter)
pub(crate) const HINT_KEYS: usize = 1;

// While the name filter is empty and not being typed in, it says what it is and how to get there,
// in the field's own font and a muted colour that still reads at 4.5:1 or more on the pill in both themes.
pub(crate) unsafe extern "system" fn filter_hint_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, which: usize) -> LRESULT {
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
        let hint = if which == HINT_KEYS { tr("Find  Ctrl+F", "찾기  Ctrl+F") } else { tr("Filter names  Ctrl+F", "이름 필터  Ctrl+F") };
        let mut hint: Vec<u16> = hint.encode_utf16().collect();
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
        if let Some(p) = p.as_ref() {
            a.load_q.retain(|&h| h != p.hwnd); // a reused window handle must not open in its place
        }
        if a.head_hot.is_some_and(|(w, _)| p.as_ref().is_some_and(|p| p.hwnd == w)) {
            a.head_hot = None;
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
    let (main, n, active, now_all) = app(|a| (a.main, a.s.panes(), a.s.active, a.fill_now || a.quiet != Quiet::No));
    if main.0.is_null() {
        return;
    }
    let mut fresh = Vec::new();
    for i in 0..MAXP {
        let has = app(|a| a.panes[i].is_some());
        if i < n && !has {
            let Ok(p) = create_pane(i) else { continue };
            app(|a| a.panes[i] = Some(p));
            fresh.push(i);
        } else if i >= n && has {
            destroy_pane(i);
        }
    }
    // Like a browser restoring its tabs: the active pane opens now, the others one at a time from the message loop,
    // so the window paints and takes clicks and keys in between. Not for a minimized sign-in start (nothing to
    // see, and its retry list is made here) or a theme switch.
    let (now, later) = if now_all { (fresh, Vec::new()) } else { load_order(active, &fresh) };
    if !later.is_empty() {
        app(|a| {
            for &i in &later {
                if let Some(p) = a.panes[i].as_mut() {
                    p.wait_first = true;
                    a.load_q.push(p.hwnd);
                }
            }
        });
    }
    for i in now {
        first_nav(i);
    }
    if !later.is_empty() {
        SetTimer(Some(main), TIMER_PANE_LOAD, 10, None);
    }
}

/// New panes: which open their first folder now and which wait their turn (in slot order). One new pane opens
/// now; of more, only the active one (the first new one if the active pane is not new).
pub(crate) fn load_order(active: usize, fresh: &[usize]) -> (Vec<usize>, Vec<usize>) {
    if fresh.len() < 2 {
        return (fresh.to_vec(), Vec::new());
    }
    let first = if fresh.contains(&active) { active } else { fresh[0] };
    (vec![first], fresh.iter().copied().filter(|&i| i != first).collect())
}

// A new pane's first folder: its saved one (or the nearest parent left), else This PC.
unsafe fn first_nav(i: usize) {
    let path = app(|a| a.s.paths[i].clone());
    // minimized sign-in start: a network or USB drive may just not be there yet
    let recent = app(|a| (a.quiet != Quiet::No).then(|| a.s.recent.clone()));
    if path.is_empty() || !go_near(i, &path) {
        if let Some(recent) = recent.filter(|_| !path.is_empty()) {
            // kept as it was, and the folder is tried again when the window is first shown
            app(|a| {
                a.s.recent = recent;
                a.late.push((i, path.clone()));
            });
        }
        navigate(i, THIS_PC);
    }
}

/// TIMER_PANE_LOAD: the next waiting pane opens its first folder, found by its window (it may have changed slot
/// since). One per tick, so painting and input come in between; never inside a menu, a modal dialog or a drag,
/// where it would wait the next turn. A pane that opened a folder of the user's meanwhile is skipped.
pub(crate) unsafe fn pane_load_tick(main: HWND) {
    _ = KillTimer(Some(main), TIMER_PANE_LOAD);
    let (enabled, menu, busy) = hold_state(main);
    if defer_args(enabled, menu, busy) {
        SetTimer(Some(main), TIMER_PANE_LOAD, 100, None);
        return;
    }
    let next = app(|a| {
        // the user made a waiting pane the active one (Ctrl+number, Alt+arrow, a swap): it goes first
        let act = a.panes.get(a.s.active).and_then(Option::as_ref).filter(|p| p.wait_first).map(|p| p.hwnd);
        if let Some(h) = act {
            a.load_q.retain(|&q| q != h);
            a.load_q.insert(0, h);
        }
        while !a.load_q.is_empty() {
            let h = a.load_q.remove(0);
            let slot = a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h && p.wait_first));
            if let Some(i) = slot {
                a.panes[i].as_mut().map(|p| p.wait_first = false);
                return Some((i, i == a.s.active, !a.load_q.is_empty()));
            }
        }
        None
    });
    let Some((i, active, more)) = next else { return };
    let focus = GetFocus();
    first_nav(i);
    if active {
        // the pane in use had no list to take the keys until now (focus_view found none): give it them
        if !focus.is_invalid() {
            focus_view(i);
        }
    } else if GetFocus() != focus && IsWindow(Some(focus)).as_bool() {
        // its new list must not take the keys from the pane in use (focus only: the active pane stays as it is)
        _ = SetFocus(Some(focus));
    }
    if more {
        SetTimer(Some(main), TIMER_PANE_LOAD, 10, None);
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

// Pane window h's geometry as it is on screen: a narrow bar spreads its focused address or filter edit. Painting,
// clicks, hover and layout_pane all go through here, so what is drawn is what is hit.
pub(crate) unsafe fn pane_geo_of(h: HWND, dpi: i32) -> PaneGeo {
    pane_geo_wide(h, dpi, None)
}

// pane_geo_of with the focused field given (None: read it)
unsafe fn pane_geo_wide(h: HWND, dpi: i32, wide: Option<Wide>) -> PaneGeo {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let wide = wide.unwrap_or_else(|| {
        let f = GetFocus();
        let on = |id: i32| !f.is_invalid() && GetDlgItem(Some(h), id).is_ok_and(|e| e == f);
        if on(ID_EDIT) {
            Wide::Addr
        } else if on(ID_FILTER) {
            Wide::Filt
        } else {
            Wide::No
        }
    });
    pane_geo_at(rc.right, rc.bottom, dpi, text_grow_now(), TABS_ON.load(Ordering::Relaxed), wide)
}

// Posted to a pane: its address edit, clicked while it showed typed text, took the focus; spread it now that the
// click has put the caret (moved during the click, the caret would land where the edit was)
pub(crate) const WM_FIELD_SPREAD: u32 = WM_APP + 66;

// Focus going into or out of h's address / filter edit: a folded bar spreads or folds that field again (only the
// bar's own controls move); on a bar with room the field just repaints its focus ring (and the address swaps
// crumbs and edit).
pub(crate) unsafe fn field_focus(h: HWND, id: i32) {
    let dpi = app(|a| a.dpi);
    let (no, wide) = (pane_geo_wide(h, dpi, Some(Wide::No)), if id == ID_EDIT { Wide::Addr } else { Wide::Filt });
    if pane_geo_wide(h, dpi, Some(wide)) != no {
        let e = GetDlgItem(Some(h), ID_EDIT).unwrap_or_default();
        if id == ID_EDIT && GetFocus() == e && GetKeyState(VK_LBUTTON.0 as i32) < 0 {
            _ = PostMessageW(Some(h), WM_FIELD_SPREAD, WPARAM(0), LPARAM(0));
        } else {
            place_head(h);
        }
        sync_addr(h); // repaints
    } else if id == ID_EDIT {
        sync_addr(h);
    } else {
        _ = InvalidateRect(Some(h), None, false);
    }
}

// The bar's buttons and edits where pane_geo_of puts them now (the list below stays)
pub(crate) unsafe fn place_head(h: HWND) {
    if let Some((moves, _)) = head_moves(h) {
        move_all(&moves, None);
        head_rehover(h); // the filter icon may have moved under a still pointer
    }
}

// The bar's controls with their places, and pane_geo_of itself
unsafe fn head_moves(h: HWND) -> Option<(Vec<(HWND, RECT)>, PaneGeo)> {
    let dpi = app(|a| a.panes.iter().flatten().any(|p| p.hwnd == h).then_some(a.dpi))?;
    let fh = app(|a| a.fh) + 2;
    let s = |v: i32| v * dpi / 96;
    let pg = pane_geo_of(h, dpi);
    // borderless edit, vertically centred inside its pill (none in a folded one: it is zero wide)
    let inside = |p: RECT, l: i32, r: i32| {
        let top = p.top + (p.bottom - p.top - fh) / 2;
        RECT { left: p.left + l, top, right: (p.right - r).max(p.left + l), bottom: top + fh }
    };
    let ctl = |id: i32| GetDlgItem(Some(h), id).unwrap_or_default();
    let mut moves: Vec<(HWND, RECT)> = [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN].iter().zip(pg.btn).map(|(&id, r)| (ctl(id), r)).collect();
    moves.push((ctl(ID_EDIT), inside(pg.addr, crumb_field(pg.addr, dpi).left - pg.addr.left, s(6)))); // where the crumbs start
    moves.push((ctl(ID_FILTER), inside(pg.filt, s(24), s(6))));
    moves.push((ctl(ID_FAV), pg.fav));
    Some((moves, pg))
}

pub(crate) unsafe fn layout_pane(h: HWND) {
    let Some((eb, disp)) = app(|a| a.panes.iter().flatten().find(|p| p.hwnd == h).map(|p| (p.eb.clone(), p.disp.clone()))) else {
        return;
    };
    let Some((moves, pg)) = head_moves(h) else { return };
    let ctl = |id: i32| GetDlgItem(Some(h), id).unwrap_or_default();
    move_all(&moves, Some((&eb, pg.view)));
    show_path_end(ctl(ID_EDIT), &disp);
    sync_addr(h);
    head_rehover(h); // resized, or the tab strip shown / hidden: the tabs and the filter icon moved
}

pub(crate) unsafe fn relayout() {
    ctrl_hint_off(); // Ctrl-held keycaps were placed for the old layout
    kbd_tip_hide(); // so was a toolbar button's tooltip (drive / update buttons come and go, the buttons move)
    flash_off(); // so was the ring round a pane just picked
    pane_fx_off(); // and a ring gliding to a zoomed / swapped pane (started again after the layout that moved it)
    // a pending Ctrl+Alt+arrow lands with this layout (key release, zoom, a session, DPI ...): its guide bar goes
    if let Some(bars) = app(|a| a.nudge.take().map(|_| a.bars)) {
        for b in bars {
            _ = ShowWindow(b, SW_HIDE);
        }
    }
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
        let btn_h = bars_h(a.dpi, text_grow_now()).1; // toolbar buttons grow with the Text size, as the toolbar does
        let sr = side_rect(&g);
        let tree = RECT { left: sr.left + s(2), top: sr.top + s(4), right: sr.right - s(2), bottom: sr.bottom - s(2) };
        let mut moves = if g.tree { vec![(a.tree_hwnd, tree)] } else { Vec::new() };
        let mut shown = vec![(a.tree_hwnd, g.tree)];
        let mut x = g.pad;
        let tb = |id: i32| unsafe { GetDlgItem(Some(a.main), id) }.unwrap_or_default();
        // the Everything magnifier only while Everything runs
        let ev = unsafe { ev_shown() };
        shown.push((tb(ID_TB_EV), ev));
        let left = [(ID_TB_LAYOUT, s(70)), (ID_TB_SAVE, s(32)), (ID_TB_LOAD, s(32)), (ID_TB_VIEW, s(32)), (ID_TB_FAV, s(32)), (ID_TB_EV, s(32))];
        for (id, w) in left.into_iter().filter(|&(id, _)| id != ID_TB_EV || ev) {
            moves.push((tb(id), RECT { left: x, top: s(5), right: x + w, bottom: s(5) + btn_h }));
            x += w + s(4);
        }
        let right = g.w - g.pad; // the theme toggle sits at the far right
        moves.push((tb(ID_TB_THEME), RECT { left: right - s(32), top: s(5), right, bottom: s(5) + btn_h }));
        // a newer release: the blue Update button just left of it
        let mut edge = right - s(32);
        let up = update_shown();
        if up {
            moves.push((tb(ID_TB_UPDATE), RECT { left: edge - s(8) - s(78), top: s(5), right: edge - s(8), bottom: s(5) + btn_h }));
            edge -= s(8) + s(78);
        }
        shown.push((tb(ID_TB_UPDATE), up));
        // drive buttons after them; in a narrow window the last letters hide rather than run into the theme toggle
        // wide enough for a drive-kind icon before the letter when all of them fit, else letters only, as before
        // then the cloud folder buttons (square icons), s(6) after the drives; the Recycle Bin button last, s(6)
        // after them: the first to hide, the clouds next; all counted in so wide drive buttons never push them out
        // alone. No cloud folders: everything exactly where it was without them. Plugged-in phones / cameras go
        // between the drives and the clouds, the same way: hidden after the clouds, before the drives; none, no change.
        x += s(6);
        let n = a.drives.len() as i32;
        let m = a.clouds.len() as i32;
        let k = a.devices.len() as i32;
        let cw = if m > 0 { m * (s(30) + s(2)) - s(2) + s(6) } else { 0 };
        let vw = if k > 0 { k * (s(30) + s(2)) - s(2) + s(6) } else { 0 };
        let dw = if x + n * (s(46) + s(2)) - s(2) + s(6) + vw + cw + s(30) <= edge - s(12) { s(46) } else { s(32) };
        let mut bx = x;
        for &(_, _, h) in &a.drives {
            moves.push((h, RECT { left: x, top: s(5), right: x + dw, bottom: s(5) + btn_h }));
            shown.push((h, x + dw <= edge - s(12)));
            x += dw + s(2);
            bx = x - s(2) + s(6);
        }
        for d in &a.devices {
            moves.push((d.h, RECT { left: bx, top: s(5), right: bx + s(30), bottom: s(5) + btn_h }));
            shown.push((d.h, bx + s(30) <= edge - s(12)));
            bx += s(30) + s(2);
        }
        if k > 0 {
            bx += s(6) - s(2);
        }
        for c in &a.clouds {
            moves.push((c.h, RECT { left: bx, top: s(5), right: bx + s(30), bottom: s(5) + btn_h }));
            shown.push((c.h, bx + s(30) <= edge - s(12)));
            bx += s(30) + s(2);
        }
        if m > 0 {
            bx += s(6) - s(2);
        }
        let bin = tb(ID_BIN);
        if !bin.0.is_null() {
            moves.push((bin, RECT { left: bx, top: s(5), right: bx + s(30), bottom: s(5) + btn_h }));
            shown.push((bin, bx + s(30) <= edge - s(12)));
        }
        if !a.status_hwnd.0.is_null() {
            moves.push((a.status_hwnd, status_rect(&g)));
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
        // a toolbar button the keys were on, squeezed out of a narrow window: the focus to the list, not lost
        if !on && !h.0.is_null() && GetFocus() == h && is_toolbar_button(h) {
            focus_view(app(|a| a.s.active));
        }
        _ = ShowWindow(h, if on { SW_SHOW } else { SW_HIDE });
    }
    start_reveal(main); // sidebar size may have changed: keep its selection in view
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        if text_of(b) != label {
            _ = SetWindowTextW(b, &HSTRING::from(label));
            layout_a11y(b);
        }
    }
    _ = InvalidateRect(Some(main), None, true);
    status_dirty(); // its zones depend on the width, and the main window no longer paints over it
    // Paint now: WM_PAINT is only generated once the message queue is empty, which never happens
    // while the mouse keeps moving, so mid-drag the views would keep showing the previous frame.
    _ = RedrawWindow(Some(main), None, None, RDW_UPDATENOW | RDW_ALLCHILDREN);
}

// Splitter drag shows translucent guide bars where the boundaries will land; the panes move once, on release.
pub(crate) unsafe fn show_bars() {
    let Some((main, bars, rects)) = app(|a| {
        let d = a.drag.or(a.nudge)?;
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

// ---------- the ring round a pane picked from the keyboard ----------
// Like a tiling window manager's focus flash (flashfocus in i3 / sway) or macOS's shake to find the pointer: a pane
// reached with Ctrl+1 … 9, Alt+Shift+arrow, Ctrl+Tab, Ctrl+P or moved with Ctrl+Alt+Shift+arrow gets a ring in the
// selection colour that fades within half a second, so the eye finds it among 16. The mouse brings no ring: it is
// where the eye already is. A see-through popup shaped to the ring (no clicks, no focus, as the drag guide bars).

thread_local! {
    // the ring on screen: when it came, the pane it marks, that pane's window rect then, whether it fades
    static FLASH: Cell<Option<(Instant, HWND, RECT, bool)>> = const { Cell::new(None) };
}

const FLASH_TIMER: usize = 1; // the ring window's own (and only) timer
const FLASH_TICK_MS: u32 = 30;

/// The ring's opacity `ms` after it came, None once it is gone. With animations on it holds a moment, then eases
/// out (fast first, slow at the end) to nothing by 500 ms; with them off it stays as it is for 400 ms, then goes.
pub(crate) fn flash_alpha(ms: u64, animate: bool) -> Option<u8> {
    const FULL: u8 = 230;
    const HOLD: u64 = 80;
    const FADE: u64 = 420;
    const STILL: u64 = 400;
    if !animate {
        return (ms < STILL).then_some(FULL);
    }
    if ms < HOLD {
        return Some(FULL);
    }
    let left = 1.0 - (ms - HOLD) as f64 / FADE as f64;
    (left > 0.0).then(|| (FULL as f64 * left * left).round() as u8)
}

/// The hole inside a w × h ring `t` thick (window coordinates), or None when the pane is too small to ring.
pub(crate) fn ring_hole(w: i32, h: i32, t: i32) -> Option<RECT> {
    (t > 0 && w > 2 * t && h > 2 * t).then_some(RECT { left: t, top: t, right: w - t, bottom: h - t })
}

// Called after set_active / focus_view (which may unzoom and lay out again), so the pane's rect is its final one.
pub(crate) unsafe fn flash_pane(i: usize) {
    let (w, main, pane, dpi) = app(|a| (a.flash, a.main, a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd), a.dpi));
    let Some(pane) = pane else { return };
    if w.is_invalid() || !IsWindowVisible(main).as_bool() || IsIconic(main).as_bool() || !IsWindowVisible(pane).as_bool() {
        return;
    }
    let mut r = RECT::default();
    if GetWindowRect(pane, &mut r).is_err() {
        return;
    }
    let (cw, ch) = (r.right - r.left, r.bottom - r.top);
    let Some(hole) = ring_hole(cw, ch, 4 * dpi / 96) else { return };
    // inside the pane's own rect: never over a neighbour or past the window's edge
    let rgn = CreateRectRgn(0, 0, cw, ch);
    let inner = CreateRectRgn(hole.left, hole.top, hole.right, hole.bottom);
    CombineRgn(Some(rgn), Some(rgn), Some(inner), RGN_DIFF);
    _ = DeleteObject(inner.into());
    SetWindowRgn(w, Some(rgn), false); // the system owns the region from here
    let animate = load_animate();
    // Placed and painted while still see-through (alpha 0 when hidden: flash_off, or never set at first), and only
    // then made opaque: there is no background brush, so shown first it would be a black ring, or the old colour
    // after a theme or selection colour change, for a frame before WM_PAINT.
    _ = SetWindowPos(w, None, r.left, r.top, cw, ch, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    _ = InvalidateRect(Some(w), None, false); // in the selection colour of now (a held key's next pane, a new theme)
    _ = UpdateWindow(w);
    _ = SetLayeredWindowAttributes(w, COLORREF(0), flash_alpha(0, animate).unwrap_or(0), LWA_ALPHA);
    FLASH.set(Some((Instant::now(), pane, r, animate)));
    SetTimer(Some(w), FLASH_TIMER, FLASH_TICK_MS, None); // the same number again restarts it
}

pub(crate) unsafe fn flash_off() {
    if FLASH.take().is_none() {
        return;
    }
    let w = app(|a| a.flash);
    _ = KillTimer(Some(w), FLASH_TIMER);
    _ = SetLayeredWindowAttributes(w, COLORREF(0), 0, LWA_ALPHA);
    _ = ShowWindow(w, SW_HIDE);
}

// Each tick the ring fades, and goes at once when what it marks changed: the pane moved or resized (window moved
// to another monitor, resized, a new DPI, a new layout), it or main hid (tray, minimised) or another pane is active.
unsafe fn flash_tick(w: HWND) {
    let Some((t0, pane, at, animate)) = FLASH.get() else {
        _ = KillTimer(Some(w), FLASH_TIMER);
        return;
    };
    let (main, active) = app(|a| (a.main, a.panes.get(a.s.active).and_then(|p| p.as_ref()).map(|p| p.hwnd)));
    let mut r = RECT::default();
    let same = active == Some(pane)
        && IsWindowVisible(main).as_bool()
        && !IsIconic(main).as_bool()
        && IsWindowVisible(pane).as_bool()
        && GetWindowRect(pane, &mut r).is_ok()
        && r == at;
    match flash_alpha(t0.elapsed().as_millis() as u64, animate).filter(|_| same) {
        Some(alpha) => _ = SetLayeredWindowAttributes(w, COLORREF(0), alpha, LWA_ALPHA),
        None => flash_off(),
    }
}

pub(crate) extern "system" fn flash_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER if wp.0 == FLASH_TIMER => flash_tick(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                // the whole window is the ring (its region): one fill in the selection colour (high contrast's included)
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(h, &mut ps);
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                fill(dc, rc, app(|a| a.s.accent_now()));
                _ = EndPaint(h, &ps);
            }
            // main minimised and back: an owned popup comes back with it; the ring is not to (flash_tick hides it)
            WM_SHOWWINDOW if lp.0 != 0 => {}
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// ---------- a ring gliding from a pane's old cell to its new one ----------
// Like Windows' maximize / snap animation or macOS's window zoom: zooming a pane in or out, or swapping two, changes
// the whole grid in one frame, so among 9 or 16 panes it is easy to lose which went where. A ring in the selection
// colour glides from the old cell to the new one (two crossing rings for a swap), then fades. Drawn over the finished
// layout only: focus, zoom and the session are as before, and nothing waits on it. Two more GridsFlash windows (no
// clicks, no focus); timed by the clock, not by ticks, as views repainting after a zoom can hold timer messages back.

thread_local! {
    // the glide on screen: when the layout landed, and per ring window the pane it follows, its from and to (screen)
    static FX: Cell<Option<(Instant, [Option<(HWND, RECT, RECT)>; 2])>> = const { Cell::new(None) };
}

const FX_MOVE_MS: u64 = 160;
const FX_FADE_MS: u64 = 60;
const FX_ALPHA: u8 = 230;

/// The ring's rect at `t` (0 = from, 1 = to, clamped outside), eased out: fast first, settling into place.
pub(crate) fn pane_fx_rect(from: RECT, to: RECT, t: f64) -> RECT {
    let e = 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3);
    let at = |a: i32, b: i32| a + ((b - a) as f64 * e).round() as i32;
    let (left, top) = (at(from.left, to.left), at(from.top, to.top));
    RECT { left, top, right: at(from.right, to.right).max(left), bottom: at(from.bottom, to.bottom).max(top) }
}

/// The rings' opacity `ms` after the layout landed: full while they glide, then fading out; None once gone.
pub(crate) fn pane_fx_alpha(ms: u64) -> Option<u8> {
    if ms < FX_MOVE_MS {
        return Some(FX_ALPHA);
    }
    let left = 1.0 - (ms - FX_MOVE_MS) as f64 / FX_FADE_MS as f64;
    (left > 0.0).then(|| (FX_ALPHA as f64 * left).round() as u8)
}

/// Pane window `h`'s rect on screen, if it shows.
pub(crate) unsafe fn shown_rect(h: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    (IsWindowVisible(h).as_bool() && GetWindowRect(h, &mut r).is_ok()).then_some(r)
}

/// Where the ring following pane `h` is on screen now, mid-glide: a second zoom goes on from there.
pub(crate) fn pane_fx_now(h: HWND) -> Option<RECT> {
    let (t0, rings) = FX.get()?;
    let (_, from, to) = rings.into_iter().flatten().find(|r| r.0 == h)?;
    Some(pane_fx_rect(from, to, t0.elapsed().as_millis() as f64 / FX_MOVE_MS as f64))
}

// Ring window w shaped to a `t`-thick ring round r and painted; hidden when r is too small to ring.
unsafe fn fx_place(w: HWND, r: RECT, t: i32) -> bool {
    let (cw, ch) = (r.right - r.left, r.bottom - r.top);
    let Some(hole) = ring_hole(cw, ch, t) else {
        _ = ShowWindow(w, SW_HIDE);
        return false;
    };
    let rgn = CreateRectRgn(0, 0, cw, ch);
    let inner = CreateRectRgn(hole.left, hole.top, hole.right, hole.bottom);
    CombineRgn(Some(rgn), Some(rgn), Some(inner), RGN_DIFF);
    _ = DeleteObject(inner.into());
    SetWindowRgn(w, Some(rgn), false); // the system owns the region from here
    _ = SetWindowPos(w, None, r.left, r.top, cw, ch, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    _ = InvalidateRect(Some(w), None, false); // in the selection colour of now (flash_proc paints it)
    _ = UpdateWindow(w);
    true
}

/// Ring k glides for pane `.0` from `.1` to `.2` (screen rects), the first frame at once. Called once the layout, the
/// active pane and the focus have landed. Not with Windows' animation effects off, Grids minimized or hidden or not
/// in front, mid-drag, or when nothing moved.
pub(crate) unsafe fn pane_fx_start(rings: [Option<(HWND, RECT, RECT)>; 2]) {
    pane_fx_off();
    let rings = rings.map(|r| r.filter(|r| r.1 != r.2));
    let (fx, main, dpi, drag) = app(|a| (a.fx, a.main, a.dpi, a.drag.is_some()));
    if rings.iter().all(Option::is_none)
        || drag
        || fx.iter().any(|w| w.is_invalid())
        || !IsWindowVisible(main).as_bool()
        || IsIconic(main).as_bool()
        || !app_in_front()
        || !load_animate()
    {
        return;
    }
    // placed and painted while still see-through (alpha 0 from pane_fx_off, or never set), only then shown: there is
    // no background brush, so a black or old-colour frame would show first otherwise
    for (w, r) in fx.into_iter().zip(rings) {
        if let Some((_, from, _)) = r {
            if fx_place(w, from, 3 * dpi / 96) {
                _ = SetLayeredWindowAttributes(w, COLORREF(0), FX_ALPHA, LWA_ALPHA);
            }
        }
    }
    FX.set(Some((Instant::now(), rings)));
    SetTimer(Some(main), TIMER_PANE_FX, 15, None);
}

/// The rings go at once: the glide ended, or what it showed changed (a new layout, the window moved, another app).
pub(crate) unsafe fn pane_fx_off() {
    if FX.take().is_none() {
        return;
    }
    let (main, fx) = app(|a| (a.main, a.fx));
    _ = KillTimer(Some(main), TIMER_PANE_FX);
    for w in fx {
        _ = SetLayeredWindowAttributes(w, COLORREF(0), 0, LWA_ALPHA);
        _ = ShowWindow(w, SW_HIDE); // hidden, not left see-through: a still screen is as without the glide
    }
}

/// TIMER_PANE_FX: the next frame, by the time since the layout landed.
pub(crate) unsafe fn pane_fx_tick() {
    let (main, fx, dpi) = app(|a| (a.main, a.fx, a.dpi));
    let Some((t0, rings)) = FX.get() else {
        _ = KillTimer(Some(main), TIMER_PANE_FX);
        return;
    };
    let ms = t0.elapsed().as_millis() as u64;
    let shown = IsWindowVisible(main).as_bool() && !IsIconic(main).as_bool() && app_in_front();
    let Some(alpha) = pane_fx_alpha(ms).filter(|_| shown) else {
        pane_fx_off();
        return;
    };
    for (w, r) in fx.into_iter().zip(rings) {
        let Some((_, from, to)) = r else { continue };
        let at = pane_fx_rect(from, to, ms as f64 / FX_MOVE_MS as f64);
        let mut now = RECT::default();
        // landed: only the fade from here
        if (GetWindowRect(w, &mut now).is_ok() && now == at && IsWindowVisible(w).as_bool()) || fx_place(w, at, 3 * dpi / 96) {
            _ = SetLayeredWindowAttributes(w, COLORREF(0), alpha, LWA_ALPHA);
        }
    }
}

pub(crate) unsafe fn end_drag() {
    let Some(bars) = app(|a| a.drag.take().map(|_| a.bars)) else { return };
    for b in bars {
        _ = ShowWindow(b, SW_HIDE);
    }
    refresh_drives(); // one that came or went mid-drag
    refresh_clouds();
    devices_found(); // a phone found mid-drag
    relayout();
    redraw_all();
}

// Ctrl+Alt+arrow (tmux's resize-pane): the active pane's splitter moves 32 px that way, as if dragged; held, it keeps
// going. As in a drag only the guide bar moves; the panes follow once the key is let go (end_nudge).
pub(crate) unsafe fn nudge_split(dx: i32, dy: i32, repeat: bool) {
    // Err: why not (only the 60 px minimum is PaneMin; zoomed, mid-drag or no border that way are NoBorder)
    let moved = app(|a| {
        if a.zoom.is_some() || a.drag.is_some() {
            return Err(Fail::NoBorder);
        }
        let Some((d, sign)) = nudge_target(&a.s.heights, a.s.active, dx, dy) else { return Err(Fail::NoBorder) };
        let g = geo(a);
        let (step, min) = (sign * 32 * a.dpi / 96, 60 * a.dpi / 96);
        // the splitter's spot as the mouse hands it to drag_ratio: the gap's middle less half a gap = the cell's end
        let (r, k, end, start, len) = match d {
            Hit::Col(k) => (&mut a.s.colr, k, g.xs.get(k).map(|x| x.1), g.gx, g.w - g.pad - g.gx),
            Hit::Row(c, k) => {
                let Some(r) = a.s.rowr.get_mut(c) else { return Err(Fail::NoBorder) };
                (r, k, g.ys.get(c).and_then(|y| y.get(k)).map(|y| y.1), g.top, g.bot - g.top)
            }
            Hit::Side => return Err(Fail::NoBorder),
        };
        let Some(end) = end.filter(|_| k + 1 < r.len()) else { return Err(Fail::NoBorder) };
        let old = r[k];
        drag_ratio(r, k, end + step, start, len, g.gap, min);
        if (r[k] - old).abs() < 1e-9 {
            return Err(Fail::PaneMin); // already at the 60 px minimum
        }
        a.nudge = Some(d);
        Ok(())
    });
    match moved {
        Ok(()) => show_bars(),
        Err(why) if !repeat => fail(why, ""),
        Err(_) => {}
    }
}

pub(crate) unsafe fn end_nudge() {
    if app(|a| a.nudge.is_some()) {
        relayout();
        redraw_all();
    }
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
            a.prev_active = Some(old); // Ctrl+` goes back to it
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
        request_tree_sync();
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

// A selection colour picked from the menu: saved for the current theme. Under high contrast the screen
// keeps Windows' highlight and the status line says why.
pub(crate) unsafe fn set_accent(c: u32) {
    app(|a| {
        a.s.accent_sys = false; // a colour of its own: Windows' accent no longer followed
        if a.s.dark {
            a.s.accent_dark = c;
        } else {
            a.s.accent = c;
        }
    });
    if HC_ON.load(Ordering::Relaxed) {
        set_note(hc_text().into());
    } else {
        paint_accent(c);
    }
}

// View menu > Selection colour > Windows accent colour: followed from now on, read and painted now (under
// high contrast only saved, as set_accent).
pub(crate) unsafe fn follow_sys_accent() {
    refresh_sys_accent();
    app(|a| a.s.accent_sys = true);
    if HC_ON.load(Ordering::Relaxed) {
        set_note(hc_text().into());
    } else {
        repaint_accent();
    }
}

// The selection colour on screen changed by itself (Windows' accent): every place it shows, open popups too.
pub(crate) unsafe fn repaint_accent() {
    paint_accent(app(|a| a.s.accent_now()));
    redraw_popups();
}

// Paints the selection colour everywhere it shows, without saving it.
pub(crate) unsafe fn paint_accent(c: u32) {
    let (bars, panes) = app(|a| (a.bars, a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>()));
    let old = SetClassLongPtrW(bars[0], GCLP_HBRBACKGROUND, CreateSolidBrush(cref(c)).0 as isize);
    if old != 0 {
        _ = DeleteObject(HGDIOBJ(old as _));
    }
    for p in panes {
        repaint_header(p); // all of them: inactive panes show the accent in their filter badge
    }
    for h in app(|a| place_buttons(a)) {
        _ = InvalidateRect(Some(h), None, false); // the active drive's (or cloud folder's, or device's) underline
    }
    status_dirty(); // the note chip and the drive bar
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
// High contrast shows light lists whatever the saved theme, so only a change of what is shown (dark or not)
// needs that rebuild; a contrast theme switching on or off over the light theme is recoloured in place.
pub(crate) unsafe fn retheme() {
    let (panes, dark) = app(|a| (a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>(), a.s.dark));
    let shown_changed = DARK_ON.load(Ordering::Relaxed) != shown_dark(dark);
    if shown_changed && (0..MAXP).any(|i| unsafe { dui_list(i) }) {
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
    redraw_popups();
}

// Open popups are owned, not children: repainted as they are.
pub(crate) unsafe fn redraw_popups() {
    let popups = app(|a| [a.fav_popup, a.load, a.grid, a.keys_popup, a.peek, a.matcher.as_ref().map_or(HWND::default(), |m| m.h), tour_hwnd()]);
    for h in popups.into_iter().filter(|h| IsWindow(Some(*h)).as_bool()) {
        _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
    }
}

// Dark is shown only when saved and no contrast theme is on.
pub(crate) fn shown_dark(saved: bool) -> bool {
    saved && !HC_ON.load(Ordering::Relaxed)
}

// Following Windows: switch if its setting differs from Grids' (the setting broadcast comes in bursts).
// High contrast switching on or off, or another contrast theme, recolours too; turning it off brings the
// saved theme back (and Windows light/dark, when followed).
pub(crate) unsafe fn theme_sync() {
    let hc = refresh_hc();
    let sys_accent = refresh_sys_accent(); // Windows' accent colour, followed or not (picked from the menu later)
    let (follow, cur, main, accent_sys) = app(|a| (a.s.follow, a.s.dark, a.main, a.s.accent_sys));
    let target = theme_target(follow, system_dark(), cur);
    if let Some(d) = target {
        app(|a| a.s.dark = d);
    }
    if hc {
        if let Ok(b) = GetDlgItem(Some(main), ID_TB_THEME) {
            tip_text(b, theme_tip(), TTM_UPDATETIPTEXTW);
        }
    }
    if hc || target.is_some() {
        retheme(); // the accent with it
    } else if sys_accent && accent_sys && !HC_ON.load(Ordering::Relaxed) {
        repaint_accent();
    }
}

// The theme button's tooltip; under high contrast it says the button only changes the saved theme.
pub(crate) fn theme_tip() -> &'static str {
    match HC_ON.load(Ordering::Relaxed) {
        true => tr(
            "Light / dark theme (High contrast is on — Windows colours are used)",
            "라이트 / 다크 테마 (고대비 테마 사용 중 — Windows 색을 씁니다)",
        ),
        false => tr("Light / dark theme", "라이트 / 다크 테마"),
    }
}

// Title bar colour: the ground colour, so title bar and toolbar read as one surface; under high contrast
// Windows' own (DWMWA_COLOR_DEFAULT).
pub(crate) unsafe fn set_caption(h: HWND) {
    let caption = if HC_ON.load(Ordering::Relaxed) { COLORREF(0xFFFF_FFFF) } else { cref(pal().bg) };
    _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
}

// Applies the current theme everywhere; panes are rebuilt so their Explorer views pick it up.
pub(crate) unsafe fn apply_theme() {
    theme_chrome();
    // every pane at once on a real theme switch (filling panes in turn would read as flicker); at start, when this
    // makes the first panes, the active one first like any other start
    let had = app(|a| a.panes.iter().any(Option::is_some));
    for i in 0..MAXP {
        destroy_pane(i);
    }
    app(|a| a.fill_now = had);
    relayout();
    app(|a| a.fill_now = false);
    redraw_all();
}

// Everything but the panes' Explorer lists: tooltips, app mode (menus, scrollbars), title bar, tree, accent.
pub(crate) unsafe fn theme_chrome() {
    let (main, dark, tree, accent, tip, kbd_tip) = app(|a| (a.main, shown_dark(a.s.dark), a.tree_hwnd, a.s.accent_now(), a.tip, a.kbd_tip));
    // tooltips: dark on dark, back to the default on light (these windows outlive the theme switch)
    let tip_theme = |t: HWND| {
        if !t.0.is_null() {
            _ = SetWindowTheme(t, if dark { w!("DarkMode_Explorer") } else { PCWSTR::null() }, PCWSTR::null());
        }
    };
    tip_theme(tip);
    tip_theme(kbd_tip);
    DARK_ON.store(dark, Ordering::Relaxed);
    if let Some(f) = uxtheme(135) {
        let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        // ForceDark / ForceLight; Default under high contrast, so menus and scrollbars take its colours
        set_preferred_app_mode(if HC_ON.load(Ordering::Relaxed) { 0 } else if dark { 2 } else { 3 });
    }
    if let Some(f) = uxtheme(136) {
        let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(f);
        flush_menu_themes();
    }
    let on = BOOL::from(dark);
    _ = DwmSetWindowAttribute(main, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4); // immersive dark title bar
    set_caption(main);
    if let Ok(tv) = FindWindowExW(Some(tree), None, w!("SysTreeView32"), PCWSTR::null()) {
        _ = SetWindowTheme(tv, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, PCWSTR::null());
        let colour = |c: u32| LPARAM(if dark { cref(c).0 as isize } else { -1 }); // -1: system default
        SendMessageW(tv, TVM_SETBKCOLOR, None, Some(colour(pal().card)));
        SendMessageW(tv, TVM_SETTEXTCOLOR, None, Some(colour(pal().text)));
        tip_theme(HWND(SendMessageW(tv, TVM_GETTOOLTIPS, None, None).0 as _));
    }
    paint_accent(accent);
    rename_theme();
}

pub(crate) unsafe fn pick_accent() {
    let (main, cur) = app(|a| (a.main, a.s.accent_saved()));
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

// What a screen reader hears on moving to a pane or tab by key, as Windows Terminal / VS Code read "tab 2 of 3":
// "Pane 3 of 4 · Documents · tab 2 of 3". i and tab_k count from 0. No folder yet: no folder part; tabs off or
// only one: no tab part.
pub(crate) fn pane_speech(ko: bool, i: usize, panes: usize, folder: &str, tab_k: usize, tab_n: usize, tabs_on: bool) -> String {
    let pick = |en: &str, k: &str| if ko { k } else { en }.to_string();
    let mut s = pick("Pane {n} of {all}", "패널 {n}/{all}").replace("{n}", &(i + 1).to_string()).replace("{all}", &panes.to_string());
    if !folder.is_empty() {
        s += " · ";
        s += folder;
    }
    if tabs_on && tab_n > 1 {
        s += " · ";
        s += &pick("tab {n} of {all}", "탭 {n}/{all}").replace("{n}", &(tab_k + 1).to_string()).replace("{all}", &tab_n.to_string());
    }
    s
}

// Pane i as it is now (its folder by the tab strip's name, place_label), read out by itself (say_only).
pub(crate) unsafe fn say_pane(i: usize) {
    let text = app(|a| {
        let t = a.s.tab_list(i);
        let folder = place_label(&a.s.paths[i]);
        pane_speech(KO.load(Ordering::Relaxed), i, a.s.panes(), &folder, a.s.tab[i].min(t.len() - 1), t.len(), a.s.tabs_on)
    });
    say_only(&text);
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
        let before = app(|a| a.s.active);
        swap_panes(before, j);
        // the active pane is somewhere else on screen now: ringed there (not for a swap from the mouse's menu)
        if app(|a| a.s.active) != before {
            flash_pane(j);
        }
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
    // each pane's cell before the move, for the two crossing rings; none out of a zoom (the swap unzooms too)
    let old = app(|a| a.zoom.is_none().then(|| [i, j].map(|k| a.panes[k].as_ref().map(|p| p.hwnd))))
        .and_then(|[hi, hj]| Some([(hi?, shown_rect(hi?)?), (hj?, shown_rect(hj?)?)]));
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
        if let Some(p) = a.say_nav.as_mut() {
            to(p);
        }
        if let Some(p) = a.prev_active.as_mut() {
            to(p);
        }
        for c in &mut a.closed_tabs {
            to(&mut c.pane);
        }
        a.status_key.clear();
        forget_sel(a);
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
    if let Some(old) = old {
        // ring 0 follows the pane now active (a zoom right after goes on from where it is)
        pane_fx_start(old.map(|(h, from)| shown_rect(h).map(|to| (h, from, to))));
    }
}

// Like tmux's pane zoom: the active pane takes the whole grid until toggled back or another pane is picked.
pub(crate) unsafe fn toggle_zoom() {
    if app(|a| a.drag.is_some() || a.s.panes() <= 1) {
        return;
    }
    // where its ring starts: the cell (or the whole grid) it is in now, or where a glide still going has got to
    let pane = app(|a| a.panes.get(a.s.active).and_then(|p| p.as_ref()).map(|p| p.hwnd));
    let from = pane.and_then(|h| pane_fx_now(h).or_else(|| shown_rect(h)));
    let i = app(|a| {
        a.zoom = if a.zoom.is_some() { None } else { Some(a.s.active) };
        a.s.active
    });
    relayout();
    redraw_all();
    focus_view(i);
    if let (Some(h), Some(from)) = (pane, from) {
        if let Some(to) = shown_rect(h) {
            pane_fx_start([Some((h, from, to)), None]); // into the whole grid, or back to its cell
        }
    }
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
    // hidden, the tree followed nothing: shown again (and laid out), it goes to the active pane's folder now
    cancel_tree_sync();
    if app(|a| a.s.tree) {
        sync_tree_active();
    }
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
        forget_sel(a);
        let n: usize = h.iter().sum();
        a.prev_active = prev_after_perm(a.prev_active, &perm, n);
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
            request_tree_sync();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_fx_glides_from_old_cell_to_new_one() {
        let cell = RECT { left: 410, top: 300, right: 800, bottom: 590 };
        let full = RECT { left: 10, top: 40, right: 1600, bottom: 900 };
        for (from, to) in [(cell, full), (full, cell), (cell, RECT { left: 10, top: 40, right: 400, bottom: 290 })] {
            assert_eq!(pane_fx_rect(from, to, 0.0), from);
            assert_eq!(pane_fx_rect(from, to, 1.0), to);
            assert_eq!(pane_fx_rect(from, to, -0.5), from); // clamped either side
            assert_eq!(pane_fx_rect(from, to, 7.0), to);
            let mut was = from;
            for k in 1..=40 {
                let r = pane_fx_rect(from, to, k as f64 / 40.0);
                assert!(r.right >= r.left && r.bottom >= r.top, "{k}");
                // each edge only ever heads for its target, never back nor past it
                let steady = |a: i32, b: i32, s: i32, e: i32| (b - a) * (e - s) >= 0 && (e - b) * (e - s) >= 0;
                assert!(steady(was.left, r.left, from.left, to.left), "{k}");
                assert!(steady(was.top, r.top, from.top, to.top), "{k}");
                assert!(steady(was.right, r.right, from.right, to.right), "{k}");
                assert!(steady(was.bottom, r.bottom, from.bottom, to.bottom), "{k}");
                was = r;
            }
        }
        // eased out: past halfway at half the time
        let mid = pane_fx_rect(cell, full, 0.5);
        assert!(cell.left - mid.left > (cell.left - full.left) / 2);
        // a degenerate pair never yields a negative size
        let r = pane_fx_rect(RECT { left: 100, top: 100, right: 100, bottom: 100 }, RECT { left: 50, top: 50, right: 50, bottom: 50 }, 0.3);
        assert!(r.right >= r.left && r.bottom >= r.top);
    }

    #[test]
    fn pane_fx_holds_while_gliding_then_fades_then_goes() {
        assert_eq!(pane_fx_alpha(0), Some(230));
        assert_eq!(pane_fx_alpha(159), Some(230));
        let mut was = 230;
        for ms in 160..220 {
            let a = pane_fx_alpha(ms).expect("still fading before 220 ms");
            assert!(a <= was, "{ms}");
            was = a;
        }
        assert_eq!(pane_fx_alpha(220), None);
        assert_eq!(pane_fx_alpha(10_000), None);
    }

    #[test]
    fn flash_holds_then_fades_out_then_goes() {
        assert_eq!(flash_alpha(0, true), Some(230));
        assert_eq!(flash_alpha(79, true), Some(230));
        let mut last = u8::MAX;
        for ms in (0..500).step_by(10) {
            let a = flash_alpha(ms, true).expect("still showing before 500 ms");
            assert!(a <= last, "{ms} ms: {a} after {last}");
            last = a;
        }
        assert!(last < 10, "nearly gone at the end: {last}");
        // ease-out: most of it is gone by half way through the fade
        assert!(flash_alpha(80 + 210, true).is_some_and(|a| a > 0 && a < 115));
        assert_eq!(flash_alpha(500, true), None);
        assert_eq!(flash_alpha(5000, true), None);
    }

    #[test]
    fn flash_without_animations_stays_then_goes_at_once() {
        for ms in [0, 30, 200, 399] {
            assert_eq!(flash_alpha(ms, false), Some(230));
        }
        assert_eq!(flash_alpha(400, false), None);
        assert_eq!(flash_alpha(1000, false), None);
    }

    #[test]
    fn ring_hole_sits_inside_the_pane() {
        assert_eq!(ring_hole(300, 200, 4), Some(RECT { left: 4, top: 4, right: 296, bottom: 196 }));
        assert_eq!(ring_hole(300, 200, 8), Some(RECT { left: 8, top: 8, right: 292, bottom: 192 })); // 200% scaling
        assert_eq!(ring_hole(8, 200, 4), None); // nothing left inside
        assert_eq!(ring_hole(300, 8, 4), None);
        assert_eq!(ring_hole(300, 200, 0), None);
    }
}
