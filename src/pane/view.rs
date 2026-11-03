use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use crate::imports::*;
use super::*;
use crate::app::*;
use crate::ui::*;
use crate::session::*;
use crate::i18n::*;
use crate::settings;
use windows::Win32::UI::Accessibility::{IAccPropServices, CLSID_AccPropServices, PROPID_ACC_NAME};
use windows::Win32::UI::Controls::{
    CDIS_FOCUS, LIST_VIEW_GROUP_STATE_FLAGS, LVCDI_GROUP, LVGA_HEADER_CENTER, LVGA_HEADER_RIGHT, LVGF_ALIGN, LVGF_EXTENDEDIMAGE, LVGF_HEADER,
    LVGF_STATE, LVGF_TITLEIMAGE, LVGGR_HEADER, LVGROUP, LVGROUP_MASK, LVGS_COLLAPSED, LVGS_COLLAPSIBLE, LVGS_FOCUSED, LVGS_SELECTED, LVGS_SUBSETED, LVM_GETGROUPINFO,
    LVM_GETGROUPRECT, LVM_GETIMAGELIST, LVSIL_GROUPHEADER,
};

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
    let h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroPane"), w!(""), style, 0, 0, 0, 0, Some(main), None, None, None)?;
    let (btn, plain) = (BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0));
    let buttons = [
        (ID_BACK, tr(crate::Key::Back), tr(crate::Key::BackAltRightClickRecentFolders)),
        (ID_FWD, tr(crate::Key::Forward), tr(crate::Key::ForwardAltRightClickRecentFolders)),
        (ID_UP, tr(crate::Key::ParentFolder), tr(crate::Key::ParentFolderAlt)),
        (ID_OPEN_IN, tr(crate::Key::OpenInAnotherPane), tr(crate::Key::OpenInCopyOrMoveAnotherPane)),
        (ID_PANE_ADD, tr(crate::Key::NewPaneTab), tr(crate::Key::NewPaneTabCtrlAltT)),
        (ID_FAV, tr(crate::Key::Favourite), tr(crate::Key::AddFavouritesFilledStarRemove)),
    ];
    for (id, name, tip) in buttons {
        add_tip(child(h, w!("BUTTON"), PCWSTR(HSTRING::from(name).as_ptr()), btn, plain, id, font), tip);
    }
    let edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_EDIT, font);
    let filt = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_FILTER, font);
    _ = SetWindowSubclass(filt, Some(filter_hint_proc), 4, 0);
    name_fields(h, i);
    // Folder suggestions as you type, like Explorer's address bar; folders only, since navigate() only browses folders
    _ = SHAutoComplete(edit, SHACF_FILESYS_DIRS | SHACF_AUTOSUGGEST_FORCE_ON | SHACF_AUTOAPPEND_FORCE_OFF);
    // After autocomplete, so this proc is the top of the chain and still sees hover.
    _ = SetWindowSubclass(edit, Some(addr_edit_proc), ADDR_SUB, 0);
    add_tip(
        filt,
        tr(crate::Key::SearchThisFolderCtrlFIconOr),
    );

    let slot = Arc::new(AtomicUsize::new(i));
    let eb: IExplorerBrowser = CoCreateInstance(&ExplorerBrowser, None, CLSCTX_INPROC_SERVER)?;
    let site: IServiceProvider = Host(slot.clone()).into();
    IUnknown_SetSite(&eb, &site)?; // before Initialize, so the view picks up our ICommDlgBrowser
    // Details is the starting mode. Columns for a folder with nothing remembered are applied
    // on navigation: Name, Date modified, Size (see folderview::default_details_columns).
    let fs = FOLDERSETTINGS { ViewMode: FVM_DETAILS.0 as u32, fFlags: FWF_NOWEBVIEW.0 as u32 };
    eb.Initialize(h, &RECT::default(), Some(&fs))?;
    eb.SetOptions(EBO_NOBORDER)?;
    let ev: IExplorerBrowserEvents = Events(slot.clone()).into();
    let cookie = eb.Advise(&ev)?;
    let helper = CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok(); // drag image over the bar
    let dt: IDropTarget = PaneDrop { slot: slot.clone(), hwnd: h, ok: Cell::new(false), helper }.into();
    _ = RegisterDragDrop(h, &dt);
    let pending = Some(app(|a| a.s.cols[i].clone())); // the session's, for the first folder
    // Born under the theme that is current now, so a later switch is the only reason to rebuild.
    let themed = THEME_EPOCH.load(Ordering::Relaxed);
    Ok(Pane { hwnd: h, eb, cookie, disp: String::new(), pending, settle: None, view_writes: 0, user_width: false, view_watch: None, recover: None, icon: -1, ticket: 0, net: None, slot, themed, theme_keep: None, theme_scroll: None, nav_busy: false, nav_pending: None })
}

const EM_GETMARGINS: u32 = 0xD4;

/// Screen readers otherwise only say "edit". Redone when the language changes or a pane changes slot.
pub(crate) unsafe fn name_fields(h: HWND, i: usize) {
    let pane = format!("{} {}", tr(crate::Key::Pane), i + 1);
    a11y_name(GetDlgItem(Some(h), ID_EDIT).unwrap_or_default(), Some(&format!("{pane} {}", tr(crate::Key::Address))));
    a11y_name(GetDlgItem(Some(h), ID_FILTER).unwrap_or_default(), Some(&format!("{pane} {}", tr(crate::Key::Search))));
}

pub(crate) unsafe fn rename_fields() {
    let panes: Vec<(HWND, usize)> = app(|a| a.panes.iter().enumerate().filter_map(|(i, p)| p.as_ref().map(|p| (p.hwnd, i))).collect());
    for (h, i) in panes {
        name_fields(h, i);
        if let Ok(f) = GetDlgItem(Some(h), ID_FILTER) {
            _ = InvalidateRect(Some(f), None, true);
        }
    }
}

unsafe fn a11y_name(h: HWND, name: Option<&str>) {
    if h.0.is_null() {
        return;
    }
    let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) else { return };
    _ = match name {
        Some(n) => acc.SetHwndPropStr(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, PROPID_ACC_NAME, &HSTRING::from(n)),
        None => acc.ClearHwndProps(h, OBJID_CLIENT.0 as u32, CHILDID_SELF, &[PROPID_ACC_NAME]),
    };
}

/// Empty, unfocused filter: "Name filter · Ctrl+F" in the field's own font. The edit itself stays.
unsafe extern "system" fn filter_hint_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { filter_hint_proc_inner(h, msg, wp, lp, _a1, _a2) }))
}
unsafe fn filter_hint_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
    let hint = msg == WM_PAINT && GetWindowTextLengthW(h) == 0 && GetFocus() != h;
    if hint {
        _ = InvalidateRect(Some(h), None, true);
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
        let mut hint: Vec<u16> = tr(crate::Key::SearchCtrlF).encode_utf16().collect();
        DrawTextW(dc, &mut hint, &mut rc, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS);
        SelectObject(dc, font);
        _ = ReleaseDC(Some(h), dc);
    }
    r
}

const ADDR_SUB: usize = 5;

/// The address edit covers the pill while it is up, so hover has to be watched here.
/// Breadcrumbs are painted on the pane; that window's mouse move covers the other state.
unsafe extern "system" fn addr_edit_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { addr_edit_proc_inner(h, msg, wp, lp, _a1, _a2) }))
}
unsafe fn addr_edit_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
    let r = DefSubclassProc(h, msg, wp, lp);
    let parent = GetParent(h).unwrap_or_default();
    if parent.0.is_null() {
        return r;
    }
    if msg == WM_MOUSEMOVE {
        // Selecting text in the edit: do not run tip work on every move. Measuring the path and
        // pumping TTM_TRACK* / TTM_UPDATETIPTEXT stalls the next WM_MOUSEMOVE, so the highlight
        // and caret lag behind the mouse (#184 tip tracking on the edit subclass).
        let buttons = (wp.0 as u32) & (MK_LBUTTON.0 | MK_RBUTTON.0);
        if buttons == 0 {
            addr_tip_at(parent, true);
            let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
            _ = TrackMouseEvent(&mut t);
        }
    } else if msg == WM_MOUSELEAVE {
        refresh_addr_tip(parent);
    }
    r
}

pub(crate) unsafe fn destroy_pane(i: usize) {
    super::recent::clear_pane(i);
    pane_left(i);
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    if let Some(sv) = view_of::<IShellView>(i) {
        if let Ok(h) = sv.GetWindow() {
            clear_filter_view(h);
        }
    }
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
        end_share(i, p.hwnd);
        drop_find(p.hwnd);
        drop_icon_request(p.hwnd);
        _ = KillTimer(Some(p.hwnd), TIMER_KIDS);
        if app(|a| a.addr_tip_pane) == p.hwnd {
            hide_addr_tip();
        }
        for id in [ID_EDIT, ID_FILTER] {
            if let Ok(c) = GetDlgItem(Some(p.hwnd), id) {
                a11y_name(c, None);
                if id == ID_FILTER {
                    _ = RemoveWindowSubclass(c, Some(filter_hint_proc), 4);
                } else if id == ID_EDIT {
                    _ = RemoveWindowSubclass(c, Some(addr_edit_proc), ADDR_SUB);
                }
            }
        }
        _ = RevokeDragDrop(p.hwnd);
        _ = p.eb.Unadvise(p.cookie);
        _ = IUnknown_SetSite(&p.eb, None);
        _ = p.eb.Destroy();
        _ = DestroyWindow(p.hwnd);
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
            if path.is_empty() || !navigate(i, &path) {
                let start = app(|a| {
                    if a.s.views[i].0 == 0 {
                        a.s.views[i] = a.s.default_view();
                    }
                    start_folder(&a.s)
                });
                navigate(i, &start);
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
    let Some((eb, dpi)) = app(|a| a.panes.iter().flatten().find(|p| p.hwnd == h).map(|p| (p.eb.clone(), a.dpi))) else {
        return;
    };
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let s = |v: i32| v * dpi / 96;
    let pg = pane_geo_of(h, rc.right, rc.bottom, dpi);
    // Borderless edit. Horizontal insets stay; the line is centred in the pill from the font.
    let in_pill = |edit: HWND, pill: RECT, l: i32, r: i32| {
        let outer = RECT { left: pill.left + l, top: pill.top, right: (pill.right - r).max(pill.left + l), bottom: pill.bottom };
        (edit, unsafe { edit_window_rect(edit, outer) })
    };
    let ctl = |id: i32| GetDlgItem(Some(h), id).unwrap_or_default();
    let mut moves: Vec<(HWND, RECT)> = [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN].iter().zip(pg.btn).map(|(&id, r)| (ctl(id), r)).collect();
    // The icon slot is reserved by width, so the edit does not move when the icon arrives.
    let inset = crumb_field(pg.addr, dpi).left - pg.addr.left;
    let add = ctl(ID_PANE_ADD);
    moves.push((add, pg.add));
    moves.push(in_pill(ctl(ID_EDIT), pg.addr, inset, s(6)));
    moves.push(in_pill(ctl(ID_FILTER), pg.filt, s(24), s(6)));
    moves.push((ctl(ID_FAV), pg.fav));
    place_find(h);
    move_all(&moves, Some((&eb, pg.view)));
    // A zero rect is not enough: a button window still takes the click. Hide it.
    // SW_SHOWNA so a resize does not move focus onto a button that is coming back.
    let show = |id: i32, r: RECT| unsafe {
        _ = ShowWindow(ctl(id), if r.right > r.left { SW_SHOWNA } else { SW_HIDE });
    };
    show(ID_BACK, pg.btn[0]);
    show(ID_FWD, pg.btn[1]);
    show(ID_UP, pg.btn[2]);
    show(ID_OPEN_IN, pg.btn[3]);
    show(ID_FAV, pg.fav);
    show(ID_PANE_ADD, pg.add);
    show(ID_FILTER, pg.filt);
    sync_addr(h);
    // After the browser is placed, so a share error stays above the file list.
    sync_net_fail(h);
}

pub(crate) unsafe fn relayout() {
    sync_panes();
    let Some((main, moves, label, shown, drive_vis, show_bar, show_side, tree_h, side_h, show_preview, preview_h)) = app(|a| {
        a.gap_hot = None; // the splitter may be gone; the window is repainted below
        if a.main.0.is_null() {
            return None;
        }
        if a.zoom.is_some_and(|z| z != a.s.active || z >= a.s.panes()) {
            a.zoom = None;
        }
        let g = geo(a);
        let s = |v: i32| v * a.dpi / 96;
        let (tree, fav) = side_panes(&g, s(2));
        let mut moves = vec![(a.tree_hwnd, tree), (a.side_hwnd, fav)];
        let tb = |id: i32| unsafe { GetDlgItem(Some(a.main), id) }.unwrap_or_default();
        let show_bar = a.s.bar;
        let strip_h = s(32);
        let strip_top = 0;
        let btn_top = strip_top + (strip_h - s(24)) / 2;
        // Drive letters, then Layout, save, load, the favourites star and the gear, at the right of the tab row.
        // The last letters hide first so a minimum tab stays clear of the group.
        // The gear stays when the toolbar is off; the other icons and the letters do not.
        let icons = icon_slots(g.w, btn_top, a.dpi, drive_slots(a.s.drive_buttons, a.drives.len()), show_bar);
        for (id, r) in icons.icons {
            moves.push((tb(id), r));
        }
        let mut drive_vis = Vec::new();
        for (i, &(_, _, h)) in a.drives.iter().enumerate() {
            let r = if i < icons.drive_n {
                icons.drives[i]
            } else {
                let x = icons.icons[0].1.left;
                RECT { left: x, top: btn_top, right: x, bottom: btn_top }
            };
            moves.push((h, r));
            drive_vis.push((h, show_bar && i < icons.drive_n));
        }
        let show_side = a.s.tree_on;
        let (tree_h, side_h) = (a.tree_hwnd, a.side_hwnd);
        let show_preview = a.s.preview_on && g.preview > 0;
        let preview_h = a.preview_hwnd;
        if !preview_h.0.is_null() && g.preview > 0 {
            moves.push((preview_h, g.preview_rect()));
        }
        moves.push((a.tabs_hwnd, RECT { left: 0, top: strip_top, right: g.w, bottom: strip_top + strip_h }));
        let full = RECT { left: g.gx, top: g.top, right: g.grid_right(), bottom: g.bot };
        let mut shown = Vec::new();
        for (i, p) in a.panes.iter().enumerate().take(a.s.panes()) {
            if let Some(p) = p {
                moves.push((p.hwnd, if a.zoom == Some(i) { full } else { cell_rect(&g, &a.s.heights, i) }));
                shown.push((p.hwnd, a.zoom.is_none_or(|z| z == i)));
            }
        }
        Some((a.main, moves, layout_label(&a.s.heights), shown, drive_vis, show_bar, show_side, tree_h, side_h, show_preview, preview_h))
    }) else {
        return;
    };
    move_all(&moves, None);
    for id in [ID_TB_LAYOUT, ID_TB_SAVE, ID_TB_LOAD, ID_TB_FAV, ID_TB_SET] {
        if let Ok(b) = GetDlgItem(Some(main), id) {
            // The gear stays: with the toolbar hidden it is the only mouse path to settings.
            let on = show_bar || id == ID_TB_SET;
            _ = ShowWindow(b, if on { SW_SHOW } else { SW_HIDE });
        }
    }
    for (h, on) in &drive_vis {
        _ = ShowWindow(*h, if *on { SW_SHOW } else { SW_HIDE });
    }
    if !tree_h.0.is_null() {
        _ = ShowWindow(tree_h, if show_side { SW_SHOW } else { SW_HIDE });
    }
    if !side_h.0.is_null() {
        _ = ShowWindow(side_h, if show_side { SW_SHOW } else { SW_HIDE });
    }
    if !preview_h.0.is_null() && !show_preview {
        _ = ShowWindow(preview_h, SW_HIDE);
        preview_clear();
    }
    let strip = app(|a| a.tabs_hwnd);
    if !strip.0.is_null() {
        _ = ShowWindow(strip, SW_SHOW);
        // The strip's rect does not change with the toolbar, so a size message never arrives.
        // Invalidate it or the tabs keep the previous width until the window is resized.
        _ = InvalidateRect(Some(strip), None, true);
    }
    // The strip is full width underneath the icons. Raise the visible ones so they take the click.
    for id in [ID_TB_LAYOUT, ID_TB_SAVE, ID_TB_LOAD, ID_TB_FAV, ID_TB_SET] {
        if let Ok(b) = GetDlgItem(Some(main), id) {
            if show_bar || id == ID_TB_SET {
                _ = SetWindowPos(b, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }
    for (h, on) in &drive_vis {
        if *on {
            _ = SetWindowPos(*h, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
    for (h, on) in &shown {
        _ = ShowWindow(*h, if *on { SW_SHOW } else { SW_HIDE });
    }
    if !preview_h.0.is_null() && show_preview {
        _ = ShowWindow(preview_h, SW_SHOW);
        // Above the file panes, so a zoomed pane stops at the splitter instead of covering the preview.
        _ = SetWindowPos(preview_h, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
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
    // A tab or a layout that needs more room grows the window. A smaller one does not shrink it.
    ensure_window_fits();
    if show_preview {
        arm_preview();
    }
}

// Splitter drag shows translucent guide bars where the boundaries will land; the panes move once, on release.
pub(crate) unsafe fn show_bars() {
    let Some((main, bars, rects)) = app(|a| {
        let d = a.drag?;
        let g = geo(a);
        let r = gap_line(&g, d, 3 * a.dpi / 96);
        let rects = if matches!(d, Hit::Row(..) | Hit::SideSplit) { [None, r] } else { [r, None] };
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
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| bar_proc_inner(h, msg, wp, lp)))
}
fn bar_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_ERASEBKGND || msg == WM_PAINT {
        let rgb = app(|a| {
            (a.pane_over == h && !h.0.is_null()).then(|| {
                if a.pane_drag.as_ref().is_some_and(|d| d.blocked) { COPY_BLOCK } else { a.s.accent_now() }
            })
        });
        if let Some(rgb) = rgb {
            return unsafe { paint_drop_overlay(h, msg, wp, rgb) };
        }
    }
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

/// The drop preview keeps the class brush as the accent. A blocked copy paints red here instead.
unsafe fn paint_drop_overlay(h: HWND, msg: u32, wp: WPARAM, rgb: u32) -> LRESULT {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    if msg == WM_ERASEBKGND {
        fill(HDC(wp.0 as _), rc, rgb);
        return LRESULT(1);
    }
    let mut ps = PAINTSTRUCT::default();
    let dc = BeginPaint(h, &mut ps);
    fill(dc, rc, rgb);
    _ = EndPaint(h, &ps);
    LRESULT(0)
}

pub(crate) unsafe fn end_drag() {
    let Some((bars, kind)) = app(|a| a.drag.take().map(|d| (a.bars, d))) else { return };
    for b in bars {
        _ = ShowWindow(b, SW_HIDE);
    }
    refresh_drives(); // one that came or went while the splitter was held
    relayout();
    redraw_all();
    if matches!(kind, Hit::SideSplit) {
        note_save();
    }
    if matches!(kind, Hit::Preview) {
        publish_shared();
        across_windows(|| {
            relayout();
            redraw_all();
        });
        note_save();
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
        tabs_invalidate();
        note_preview(i);
    }
}

pub(crate) unsafe fn repaint_header(pane: HWND) {
    _ = InvalidateRect(Some(pane), None, false);
    for id in [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN, ID_PANE_ADD, ID_FAV, ID_EDIT, ID_FILTER] {
        if let Ok(c) = GetDlgItem(Some(pane), id) {
            _ = InvalidateRect(Some(c), None, true);
        }
    }
}

// The sidebar follows the active pane: expand down to its folder and select it.
// (Programmatic selection doesn't navigate: OnSelectionChanged only follows user input.)
pub(crate) unsafe fn sync_tree(item: &IShellItem) {
    let Some((t, main, th)) = app(|a| a.tree.clone().map(|t| (t, a.main, a.tree_hwnd))) else { return };
    // the tree scrolls on its own when it selects; hide that and paint once, after the reveal
    let tv = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()).ok();
    if let Some(tv) = tv {
        SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(0)), None);
    }
    _ = t.EnsureItemVisible(item);
    _ = t.SetItemState(item, NSTCIS_SELECTED.0 as u32, NSTCIS_SELECTED.0 as u32);
    reveal_tree_selection(); // turns redraw back on
    if let Some(tv) = tv {
        SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(1)), None);
        _ = InvalidateRect(Some(tv), None, true);
    }
    start_reveal(main); // and again while the tree finishes expanding
}

// Scrolls the tree so the selected folder is on screen: vertically, and sideways when deep nesting
// pushes its name past a narrow sidebar.
pub(crate) unsafe fn reveal_tree_selection() {
    let (th, dpi) = app(|a| (a.tree_hwnd, a.dpi));
    let Ok(tv) = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()) else { return };
    let item = SendMessageW(tv, TVM_GETNEXTITEM, Some(WPARAM(TVGN_CARET)), None).0;
    if item == 0 {
        return;
    }
    let label = || {
        let mut r = RECT::default();
        *(&mut r as *mut RECT as *mut isize) = item; // TVM_GETITEMRECT takes the item in the RECT
        let ok = SendMessageW(tv, TVM_GETITEMRECT, Some(WPARAM(1)), Some(LPARAM(&mut r as *mut _ as isize))).0 != 0;
        ok.then_some(r)
    };
    let mut c = RECT::default();
    _ = GetClientRect(tv, &mut c);
    if c.right == 0 {
        return; // not laid out yet; relayout asks again
    }
    let margin = 16 * dpi / 96;
    // all the scrolling happens unseen, then one repaint if anything moved
    let pos = || (GetScrollPos(tv, SB_HORZ), GetScrollPos(tv, SB_VERT));
    let before = pos();
    SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(0)), None);
    SendMessageW(tv, TVM_ENSUREVISIBLE, None, Some(LPARAM(item)));
    // sideways, a line at a time (avoids guessing the tree's scroll units): the selected folder's name
    // wins over the roots on the left, which may get cut off. One direction only, so a name wider than
    // the sidebar can't bounce between the two rules.
    let left = label().is_some_and(|r| r.left < margin);
    for _ in 0..300 {
        match label() {
            Some(r) if left && r.left < margin && GetScrollPos(tv, SB_HORZ) > 0 => _ = SendMessageW(tv, WM_HSCROLL, Some(WPARAM(0)), None), // SB_LINELEFT
            Some(r) if !left && r.right > c.right && r.left > margin => _ = SendMessageW(tv, WM_HSCROLL, Some(WPARAM(1)), None), // SB_LINERIGHT
            _ => break,
        }
    }
    if !left && label().is_some_and(|r| r.left < margin) {
        SendMessageW(tv, WM_HSCROLL, Some(WPARAM(0)), None); // overshot: back one, so the next pass is a no-op
    }
    SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(1)), None);
    if pos() != before {
        _ = InvalidateRect(Some(tv), None, true);
    }
}

// The tree expands down to a new folder asynchronously, so the reveal runs a few times as it settles.
pub(crate) unsafe fn start_reveal(main: HWND) {
    app(|a| a.reveal_tries = 3);
    SetTimer(Some(main), TIMER_REVEAL, 120, None);
}

pub(crate) unsafe fn set_accent(c: u32) {
    let (bars, panes, drives, peek) = app(|a| {
        if a.s.dark {
            a.s.accent_dark = c;
        } else {
            a.s.accent = c;
        }
        (a.bars, a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>(), a.drives.iter().map(|d| d.2).collect::<Vec<_>>(), a.peek)
    });
    let old = SetClassLongPtrW(bars[0], GCLP_HBRBACKGROUND, CreateSolidBrush(cref(c)).0 as isize);
    if old != 0 {
        _ = DeleteObject(HGDIOBJ(old as _));
    }
    for p in panes {
        repaint_header(p); // all of them: inactive panes show the accent in their filter badge
    }
    for h in drives {
        _ = InvalidateRect(Some(h), None, false); // the active drive's underline
    }
    if !peek.0.is_null() {
        _ = InvalidateRect(Some(peek), None, false); // open quick preview reads the accent as it paints
    }
    drop_group_brushes();
    settings::redraw();
}

// The hosted Explorer list ignores the app-wide dark mode, so its (new, per folder) list view is dyed here.
// An already-open DefView also keeps the scrollbars and header it was created with, so a theme switch
// restyles these windows and then rebuilds the view (see `refresh_one_pane`).
pub(crate) unsafe fn dark_list(pane: HWND) {
    let found = shell_children(pane);
    // the view paints the name column itself (custom draw, light colours); recolour it on the way back
    for (_, v) in found.iter().filter(|(c, _)| c == "SHELLDLL_DefView") {
        _ = SetWindowSubclass(*v, Some(dark_view_proc), 1, 0);
        restyle(*v, true, w!("DarkMode_Explorer"));
    }
    for (_, v) in found.iter().filter(|(c, _)| c == "DirectUIHWND") {
        restyle(*v, true, w!("DarkMode_Explorer"));
    }
    let (card, text) = (cref(pal().card).0 as isize, cref(pal().text).0 as isize);
    for (_, lv) in found.iter().filter(|(c, _)| c == "SysListView32") {
        restyle(*lv, true, w!("DarkMode_Explorer")); // also darkens its scrollbars once the view is recreated
        // the sorted-column tint stays white in dark mode
        _ = SetWindowSubclass(*lv, Some(dark_list_proc), 2, 0);
        SendMessageW(*lv, LVM_SETSELECTEDCOLUMN, Some(WPARAM(usize::MAX)), None);
        SendMessageW(*lv, LVM_SETBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(*lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(*lv, LVM_SETTEXTCOLOR, None, Some(LPARAM(text)));
        let header = HWND(SendMessageW(*lv, LVM_GETHEADER, None, None).0 as _);
        if !header.0.is_null() {
            restyle(header, true, w!("DarkMode_ItemsView"));
        }
        // the info tip over a file, and the full name over a truncated one
        let tip = HWND(SendMessageW(*lv, LVM_GETTOOLTIPS, None, None).0 as _);
        theme_tip(tip, true);
        _ = InvalidateRect(Some(*lv), None, true);
    }
}

/// Puts a list back to the light-theme colours. Subclasses stay, and they do nothing while dark mode is off.
pub(crate) unsafe fn light_list(pane: HWND) {
    let found = shell_children(pane);
    for (_, v) in found.iter().filter(|(c, _)| c == "SHELLDLL_DefView" || c == "DirectUIHWND") {
        restyle(*v, false, w!("Explorer"));
    }
    let (bg, text) = (GetSysColor(COLOR_WINDOW) as isize, GetSysColor(COLOR_WINDOWTEXT) as isize);
    for (_, lv) in found.iter().filter(|(c, _)| c == "SysListView32") {
        restyle(*lv, false, w!("Explorer"));
        SendMessageW(*lv, LVM_SETBKCOLOR, None, Some(LPARAM(bg)));
        SendMessageW(*lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(bg)));
        SendMessageW(*lv, LVM_SETTEXTCOLOR, None, Some(LPARAM(text)));
        let header = HWND(SendMessageW(*lv, LVM_GETHEADER, None, None).0 as _);
        if !header.0.is_null() {
            restyle(header, false, w!("ItemsView"));
        }
        let tip = HWND(SendMessageW(*lv, LVM_GETTOOLTIPS, None, None).0 as _);
        theme_tip(tip, false);
        _ = InvalidateRect(Some(*lv), None, true);
    }
}

/// DefView, its list, the items-view surface, and the column header under one pane.
unsafe fn shell_children(pane: HWND) -> Vec<(String, HWND)> {
    extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        crate::crash::catch_bool(std::panic::AssertUnwindSafe(|| collect_inner(h, out)))
    }
    fn collect_inner(h: HWND, out: LPARAM) -> BOOL {
        let mut c = [0u16; 32];
        let n = unsafe { GetClassNameW(h, &mut c) } as usize;
        let class = String::from_utf16_lossy(&c[..n]);
        if matches!(class.as_str(), "SysListView32" | "SHELLDLL_DefView" | "DirectUIHWND") {
            unsafe { (*(out.0 as *mut Vec<(String, HWND)>)).push((class, h)) };
        }
        true.into()
    }
    let mut found = Vec::new();
    if !pane.0.is_null() {
        _ = EnumChildWindows(Some(pane), Some(collect), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// `AllowDarkModeForWindow` (uxtheme ordinal 133) plus a theme refresh.
/// Scrollbars read this at creation; a view that already exists is rebuilt afterwards.
unsafe fn allow_dark_window(h: HWND, allow: bool) {
    if h.0.is_null() {
        return;
    }
    let Some(f) = uxtheme(133) else { return };
    let allow_dark: unsafe extern "system" fn(HWND, i32) -> i32 = std::mem::transmute(f);
    let _ = allow_dark(h, i32::from(allow));
}

unsafe fn restyle(h: HWND, dark: bool, theme: PCWSTR) {
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    allow_dark_window(h, dark);
    _ = SetWindowTheme(h, theme, PCWSTR::null());
    _ = SendMessageW(h, WM_THEMECHANGED, None, None);
    _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
}

/// Explorer or DarkMode_Explorer on a window that is not one of our owner-drawn controls.
pub(crate) unsafe fn theme_hwnd(h: HWND, dark: bool) {
    restyle(h, dark, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") });
}

unsafe fn theme_tip(t: HWND, dark: bool) {
    if t.0.is_null() {
        return;
    }
    allow_dark_window(t, dark);
    _ = SetWindowTheme(t, if dark { w!("DarkMode_Explorer") } else { PCWSTR::null() }, PCWSTR::null());
    _ = SendMessageW(t, WM_THEMECHANGED, None, None);
}

/// Colours and window theme for the shell view in this pane. Does not rebuild it.
pub(crate) unsafe fn paint_list_theme(pane: HWND) {
    if pane.0.is_null() {
        return;
    }
    if DARK_ON.load(Ordering::Relaxed) {
        dark_list(pane);
    } else {
        light_list(pane);
    }
}

/// Windows app mode. None when the value is not there, so a missing key is not treated as light.
pub(crate) unsafe fn system_dark() -> Option<bool> {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let (mut v, mut n) = (0u32, 4u32);
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let r = RegGetValueW(HKEY_CURRENT_USER, key, w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as _), Some(&mut n));
    r.is_ok().then_some(v == 0)
}

/// The dark flag to switch to while following Windows. None when following is off, the setting is unknown, or it already matches.
pub(crate) fn theme_target(auto: bool, sys: Option<bool>, cur: bool) -> Option<bool> {
    sys.filter(|&d| auto && d != cur)
}

// The view re-sets the sorted column through an internal interface (no window message to catch),
// so it is cleared right before each paint instead. The column header is the list's child, so its
// draw notifications come through here too.
pub(crate) unsafe extern "system" fn dark_list_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { dark_list_proc_inner(h, msg, wp, lp, _a1, _a2) }))
}
unsafe fn dark_list_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
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

pub(crate) unsafe extern "system" fn dark_view_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { dark_view_proc_inner(h, msg, wp, lp, _a1, _a2) }))
}
unsafe fn dark_view_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_NOTIFY && DARK_ON.load(Ordering::Relaxed) && (*(lp.0 as *const NMHDR)).code == NM_CUSTOMDRAW {
        let cd = &mut *(lp.0 as *mut NMLVCUSTOMDRAW);
        let stage = cd.nmcd.dwDrawStage.0;
        // A plain group title is drawn here. Anything richer stays on the list's own drawing.
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

/// Dark-theme group title ("Devices and drives" on This PC): flat card, light text, a hairline.
/// Images, "show more", or a failed lookup returns false and the list draws it. Only the title is
/// read: asking a shell view's list for a subtitle, task, description or footer faults inside
/// comctl32 (6.10.26100), which ends this paint or the app; shell groups have none.
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
        let up = st & LVGS_COLLAPSED.0 == 0;
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

/// AppsUseLightTheme = 0 means Windows is in dark app mode.
pub(crate) unsafe fn windows_dark() -> bool {
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD};
    let mut key = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), None, KEY_READ, &mut key).is_err() {
        return false;
    }
    let mut data = [0u8; 4];
    let mut n = 4u32;
    let mut typ = REG_DWORD;
    let ok = RegQueryValueExW(key, w!("AppsUseLightTheme"), None, Some(&mut typ), Some(data.as_mut_ptr()), Some(&mut n));
    _ = RegCloseKey(key);
    ok.is_ok() && u32::from_le_bytes(data) == 0
}

pub(crate) fn resolve_dark(theme: u8) -> bool {
    match theme {
        2 => true,
        1 => false,
        _ => unsafe { windows_dark() },
    }
}

pub(crate) unsafe fn start_folder(s: &Session) -> String {
    match s.start_new {
        1 => {
            let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None) else { return THIS_PC.into() };
            let path = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            if path.is_empty() { THIS_PC.into() } else { path }
        }
        2 if !s.start_path.is_empty() => s.start_path.clone(),
        _ => THIS_PC.into(),
    }
}

/// Show or hide extensions for the hosted Explorer views. This is Windows' own HideFileExt value.
pub(crate) unsafe fn apply_ext(show: bool) {
    use windows::Win32::System::Registry::{RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_DWORD};
    let mut key = HKEY::default();
    if RegOpenKeyExW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced"), None, KEY_SET_VALUE, &mut key).is_err() {
        return;
    }
    let data = [(!show) as u8, 0, 0, 0];
    _ = RegSetValueExW(key, w!("HideFileExt"), None, REG_DWORD, Some(&data));
    _ = RegCloseKey(key);
    SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    across_windows(|| {
        for i in 0..MAXP {
            if let Some(sv) = view_of::<IShellView>(i) {
                _ = sv.Refresh();
            }
        }
    });
}

pub(crate) unsafe fn prime_theme(dark: bool) {
    DARK_ON.store(dark, Ordering::Relaxed);
    if let Some(f) = uxtheme(135) {
        let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        set_preferred_app_mode(if dark { 2 } else { 3 }); // ForceDark / ForceLight
    }
    // Without this, a process that first built a light shell view can keep minting light
    // DirectUIHWND after ForceDark + remake (Win10 22H2). Fresh dark starts were fine.
    if let Some(f) = uxtheme(104) {
        let refresh_immersive: unsafe extern "system" fn() = std::mem::transmute(f);
        refresh_immersive();
    }
    if let Some(f) = uxtheme(136) {
        let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(f);
        flush_menu_themes();
    }
}

/// Title-bar icon follows the theme. Light is group 3 (`app.ico`). Dark is group 2
/// (`assets/maguro-dark-bg.ico`). Group 1 is the Explorer file icon and stays off this window.
/// LoadImageW picks the real 16/32 (etc.) frames from the kit .ico instead of stretching one size.
/// ICON_BIG and ICON_SMALL are the class big icon and the small title-bar / taskbar icon.
pub(crate) unsafe fn apply_window_icon(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }
    let id = window_icon_group(DARK_ON.load(Ordering::Relaxed));
    let Ok(module) = GetModuleHandleW(None) else { return };
    let hinst: HINSTANCE = module.into();
    let dpi = GetDpiForWindow(hwnd);
    let small_px = GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16);
    let large_px = GetSystemMetricsForDpi(SM_CXICON, dpi).max(32);
    let load = |px: i32| {
        LoadImageW(Some(hinst), PCWSTR(id as _), IMAGE_ICON, px, px, IMAGE_FLAGS(0)).ok().map(|h| HICON(h.0))
    };
    let small = load(small_px).or_else(|| LoadIconW(Some(hinst), PCWSTR(id as _)).ok());
    let large = load(large_px).or_else(|| LoadIconW(Some(hinst), PCWSTR(id as _)).ok());
    if let Some(icon) = large {
        _ = SendMessageW(hwnd, WM_SETICON, Some(WPARAM(1)), Some(LPARAM(icon.0 as isize))); // ICON_BIG
    }
    if let Some(icon) = small {
        _ = SendMessageW(hwnd, WM_SETICON, Some(WPARAM(0)), Some(LPARAM(icon.0 as isize))); // ICON_SMALL
    }
}

/// One window: title bar, tree, and that window's Explorer panes.
pub(crate) unsafe fn apply_theme_desk() {
    let (main, dark, tree, accent, tip, side) = app(|a| (a.main, a.s.dark, a.tree_hwnd, a.s.accent_now(), a.tip, a.side_hwnd));
    if main.0.is_null() {
        return;
    }
    apply_window_icon(main);
    // tooltips: dark on dark, back to the default on light (these windows outlive the theme switch).
    // The address-bar path tip and the favourites location tip are tracking tools on this same window.
    theme_tip(tip, dark);
    theme_hwnd(side, dark); // favourites list scrollbar
    let on = BOOL::from(dark);
    _ = DwmSetWindowAttribute(main, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4); // immersive dark title bar
    let caption = cref(pal().bg);
    _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    theme_tree(tree, dark);
    set_accent(accent);
    // Visible panes only. A hidden tab keeps its old view until it is shown (`attach_live`).
    let panes = app(|a| a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>());
    for p in panes {
        paint_list_theme(p);
    }
    let fresh = app(|a| {
        let epoch = THEME_EPOCH.load(Ordering::Relaxed);
        let fresh = a.theme_seen != epoch;
        a.theme_seen = epoch;
        fresh
    });
    if fresh {
        retint_peek();
        retint_preview();
    }
    redraw_all();
    let popups = app(|a| {
        let match_h = a.matcher.as_ref().map(|m| m.hwnd).unwrap_or_default();
        [a.fav_popup, a.load, a.grid, a.keys_popup, a.peek, match_h]
    });
    for h in popups {
        if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
            _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
        }
    }
}

/// Folder tree: the namespace host and the SysTreeView32 inside it, including its scrollbar.
unsafe fn theme_tree(tree: HWND, dark: bool) {
    if tree.0.is_null() {
        return;
    }
    theme_hwnd(tree, dark);
    let Ok(tv) = FindWindowExW(Some(tree), None, w!("SysTreeView32"), PCWSTR::null()) else { return };
    theme_hwnd(tv, dark);
    let colour = |c: u32| LPARAM(if dark { cref(c).0 as isize } else { -1 }); // -1: system default
    SendMessageW(tv, TVM_SETBKCOLOR, None, Some(LPARAM(cref(crate::ui::side_ground()).0 as isize)));
    SendMessageW(tv, TVM_SETTEXTCOLOR, None, Some(colour(pal().text)));
    theme_tip(HWND(SendMessageW(tv, TVM_GETTOOLTIPS, None, None).0 as _), dark);
    _ = InvalidateRect(Some(tv), None, true);
}

/// Follow Windows when that mode is on and the setting differs from the panes.
/// Chrome updates immediately. DefView/DirectUI lists that were built under the old theme
/// are rebuilt one pane at a time via ThemeKeep (path, selection, scroll, view snap restored;
/// back/forward history and filter boxes kept).
pub(crate) unsafe fn theme_sync() {
    let (auto, cur) = app(|a| (a.s.theme == 0, a.s.dark));
    if let Some(d) = theme_target(auto, system_dark(), cur) {
        app(|a| a.s.dark = d);
        publish_shared();
        prime_theme(d);
        note_theme_epoch();
        across_windows(|| apply_theme_desk());
        note_save();
        // Each window rebuilds its own panes. A single post would leave the other windows stale,
        // because publish_shared already made their dark bit match and a later sync is a no-op.
        across_windows(|| schedule_theme_panes());
    }
}

/// Applies the current theme. Chrome updates immediately. DefView/DirectUI lists that were
/// built under the old theme are rebuilt one pane at a time via ThemeKeep (path, selection,
/// scroll, view snap restored; back/forward history and filter boxes kept).
pub(crate) unsafe fn apply_theme() {
    let dark = resolve_dark(app(|a| a.s.theme));
    let changed = app(|a| {
        let changed = a.s.dark != dark;
        a.s.dark = dark;
        changed
    });
    publish_shared();
    prime_theme(dark);
    if changed {
        note_theme_epoch();
    }
    across_windows(|| apply_theme_desk());
    settings::refresh();
    crate::watch::restyle();
    crate::basket::refresh();
    crate::tablist::restyle();
    crate::rename::refresh();
    refresh_fav_manager();
    if changed {
        // DirectUIHWND (Win10) locks light/dark at IExplorerBrowser::Initialize. Remaking the
        // browser under ForceDark/ForceLight still mints the first-birth colour in-process;
        // a fresh start matches chrome. SysListView32 can recolour in place via ThemeKeep.
        if any_dui_pane() {
            schedule_theme_relaunch();
        } else {
            across_windows(|| schedule_theme_panes());
        }
    }
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
    pub(crate) kind: u8,
}

thread_local! {
    static LAST_CLICK: Cell<Option<Click>> = const { Cell::new(None) };
}

// Windows 11's list (DirectUIHWND) has no CS_DBLCLKS, so a double-click is two presses in a row,
// within the system's double-click time and box. The pair is consumed: a third click doesn't repeat it.
// `hit` is 0 (an item the shell handles, or nothing), 1 (empty list), or 2 (a details cell). Only two
// presses of the same kind pair.
pub(crate) fn next_click(prev: Option<Click>, now: Click, hit: u8, ms: u32, cx: i32, cy: i32) -> (Option<Click>, bool) {
    if hit == 0 || now.kind != hit {
        return (None, false);
    }
    match prev {
        Some(p)
            if p.kind == hit
                && p.win == now.win
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
// column headers are never background.
// The column header of a details view is a list too, nested in the item list: never background, so a
// double-click on a header divider (auto-fit a column) does not go up a folder.
// Windows 11 does not report a details cell as a list item. The icon is a graphic and the name, date and
// size are text. Those belong to the row: a double-click opens the item, and only the real list background
// goes up.
pub(crate) fn bg_role(role: u32, parent: Option<u32>) -> bool {
    (role == ROLE_SYSTEM_LIST && parent != Some(ROLE_SYSTEM_LIST)) || role == ROLE_SYSTEM_LISTITEM
}

pub(crate) fn cell_role(role: u32) -> bool {
    matches!(role, ROLE_SYSTEM_GRAPHIC | ROLE_SYSTEM_STATICTEXT | ROLE_SYSTEM_TEXT)
}

const HIT_NONE: u8 = 0;
const HIT_BLANK: u8 = 1;
const HIT_CELL: u8 = 2;

unsafe fn role_of(a: &IAccessible, child: &VARIANT) -> Option<u32> {
    let mut role = a.get_accRole(child).ok()?;
    let r = &role.Anonymous.Anonymous;
    let out = (r.vt == VT_I4).then(|| r.Anonymous.lVal as u32);
    _ = VariantClear(&mut role);
    out
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

fn hit_kind(role: u32, parent: Option<u32>) -> u8 {
    if cell_role(role) {
        HIT_CELL
    } else if bg_role(role, parent) {
        HIT_BLANK
    } else {
        HIT_NONE
    }
}

// Same-thread window, so WM_GETOBJECT runs inline; call outside app().
pub(crate) unsafe fn on_background(m: &MSG) -> bool {
    point_hit(m) == HIT_BLANK
}

unsafe fn point_hit(m: &MSG) -> u8 {
    if WindowFromPoint(m.pt) != m.hwnd {
        return HIT_NONE;
    }
    let mut acc: Option<IAccessible> = None;
    let mut child = VARIANT::default();
    if AccessibleObjectFromPoint(m.pt, &mut acc, &mut child).is_err() {
        return HIT_NONE;
    }
    let hit = acc.as_ref().map(|a| {
        let Some(role) = role_of(a, &child) else { return HIT_NONE };
        let parent = (role == ROLE_SYSTEM_LIST)
            .then(|| a.accParent().ok().and_then(|p| p.cast::<IAccessible>().ok()))
            .flatten()
            .and_then(|p| role_of(&p, &VARIANT::from(0i32)));
        hit_kind(role, parent)
    }).unwrap_or(HIT_NONE);
    _ = VariantClear(&mut child);
    hit
}

#[derive(Clone, Copy)]
pub(crate) enum RowClick {
    Up(usize, usize, isize),
    Open(usize, usize),
}

// A double-click on empty space in a list goes up a folder. A double-click on a details cell
// (text or icon) opens that row. `None` until the second press of the pair.
pub(crate) unsafe fn up_candidate(m: &MSG) -> Option<RowClick> {
    if !matches!(m.message, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) || !app(|a| a.s.dblup) {
        return None;
    }
    let i = pane_of(m.hwnd)?;
    let svh = Cell::new(HWND::default());
    let hit = Cell::new(HIT_NONE);
    let (g0, armed) = bg_probe(gen, || {
        let Some(h) = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok()) else {
            return false;
        };
        svh.set(h);
        let mut c = [0u16; 32];
        let n = GetClassNameW(m.hwnd, &mut c) as usize;
        let list = matches!(String::from_utf16_lossy(&c[..n]).as_str(), "DirectUIHWND" | "SysListView32");
        if !(list && (h == m.hwnd || IsChild(h, m.hwnd).as_bool()) && m.wParam.0 & (MK_CONTROL.0 | MK_SHIFT.0) as usize == 0) {
            return false;
        }
        let kind = point_hit(m);
        hit.set(kind);
        kind != HIT_NONE
    });
    let kind = if armed { hit.get() } else { HIT_NONE };
    let now = Click { win: m.hwnd.0 as isize, gen: g0, time: m.time, x: m.pt.x, y: m.pt.y, kind };
    let (cx, cy) = (GetSystemMetrics(SM_CXDOUBLECLK), GetSystemMetrics(SM_CYDOUBLECLK));
    let (next, fire) = next_click(LAST_CLICK.get(), now, kind, GetDoubleClickTime(), cx, cy);
    LAST_CLICK.set(next);
    if !fire {
        return None;
    }
    match kind {
        HIT_BLANK => Some(RowClick::Up(i, g0, svh.get().0 as isize)),
        HIT_CELL => Some(RowClick::Open(i, g0)),
        _ => None,
    }
}

thread_local! {
    static SHELL_ACT: Cell<u32> = const { Cell::new(u32::MAX) };
}

pub(crate) fn mark_shell_activate() {
    SHELL_ACT.set(unsafe { GetMessageTime() as u32 });
}

pub(crate) fn shell_activated(time: u32) -> bool {
    SHELL_ACT.get() == time
}

fn acc_label(a: &IAccessible, child: &VARIANT) -> Option<String> {
    let b = unsafe { a.get_accName(child).ok() }?;
    let s = String::from_utf16_lossy(&b);
    (!s.is_empty()).then_some(s)
}

// The list item that owns the text or icon under the pointer. A header graphic has no list item.
unsafe fn list_item_at(pt: POINT) -> Option<(IAccessible, VARIANT)> {
    let mut acc: Option<IAccessible> = None;
    let mut child = VARIANT::default();
    if AccessibleObjectFromPoint(pt, &mut acc, &mut child).is_err() {
        return None;
    }
    let start = acc?;
    if role_of(&start, &child) == Some(ROLE_SYSTEM_LISTITEM) {
        return Some((start, child));
    }
    let mut cur = start;
    for _ in 0..6 {
        let parent = cur.accParent().ok().and_then(|p| p.cast::<IAccessible>().ok())?;
        let role = role_of(&parent, &VARIANT::from(0i32))?;
        if role == ROLE_SYSTEM_LISTITEM {
            _ = VariantClear(&mut child);
            return Some((parent, VARIANT::from(0i32)));
        }
        if role == ROLE_SYSTEM_LIST {
            let outer = parent.accParent().ok().and_then(|p| p.cast::<IAccessible>().ok());
            if outer.as_ref().and_then(|p| role_of(p, &VARIANT::from(0i32))) == Some(ROLE_SYSTEM_LIST) {
                break;
            }
        }
        cur = parent;
    }
    _ = VariantClear(&mut child);
    None
}

/// Right-click on a DirectUI items view. An already selected row keeps a multi-selection.
/// A header or empty gap has no list item, so the shell's selection is left as it is.
pub(crate) unsafe fn select_accessible_row(i: usize, pt: POINT) {
    let Some((item, mut child)) = list_item_at(pt) else { return };
    let name = acc_label(&item, &child).or_else(|| acc_label(&item, &VARIANT::from(0i32)));
    _ = VariantClear(&mut child);
    let Some(name) = name else { return };
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    // The shell usually focuses the row on the way down. That index is the hit, so a big folder
    // does not walk every name. A details cell that did not move focus still needs the walk.
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let focus = fv.GetFocusedItem().ok().filter(|&k| k >= 0 && k < n);
    let first = focus.into_iter().chain(0..n);
    for k in first {
        let Ok(it) = fv.GetItem::<IShellItem>(k) else { continue };
        let disp = it.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| pwstr(p)).unwrap_or_default();
        if !disp.eq_ignore_ascii_case(&name) {
            continue;
        }
        let Ok(pidl) = fv.Item(k) else { return };
        let selected = fv.GetSelectionState(pidl).unwrap_or(0) & SVSI_SELECT.0 as u32 != 0;
        CoTaskMemFree(Some(pidl as _));
        if !selected {
            let flags = (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0) as u32;
            _ = fv.SelectItem(k, flags);
        }
        return;
    }
}

unsafe fn select_named(i: usize, name: &str) -> bool {
    let Some(fv) = view_of::<IFolderView2>(i) else { return false };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let flags = (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0 | SVSI_ENSUREVISIBLE.0) as u32;
    for k in 0..n {
        let Ok(it) = fv.GetItem::<IShellItem>(k) else { continue };
        let disp = it.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| pwstr(p)).unwrap_or_default();
        if disp.eq_ignore_ascii_case(name) {
            return fv.SelectItem(k, flags).is_ok();
        }
    }
    false
}

// Double-click on a details cell the shell did not activate: open the row that owns the text or icon.
pub(crate) unsafe fn open_row_cell(i: usize, pt: POINT) {
    // Do not accDoDefaultAction: on DirectUI that can open a real Explorer window and skip
    // OnDefaultCommand / take_default. Select the row and go through take_default instead.
    let Some((item, mut child)) = list_item_at(pt) else { return };
    let name = acc_label(&item, &child).or_else(|| acc_label(&item, &VARIANT::from(0i32)));
    if let Some(name) = name {
        if select_named(i, &name) {
            let sv = view_of::<IShellView>(i);
            take_default(i, sv.as_ref());
        }
    }
    _ = VariantClear(&mut child);
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
    let (cells, cur) = app(|a| {
        let g = geo(a);
        let cells = (0..a.s.panes()).filter(|&i| a.panes[i].is_some()).map(|i| (i, cell_rect(&g, &a.s.heights, i))).collect::<Vec<_>>();
        (cells, a.s.active)
    });
    if let Some(j) = neighbour(&cells, cur, dx, dy) {
        set_active(j);
        focus_view(j);
    }
}

/// The live pane on screen next to the active one.
fn active_neighbour(dx: i32, dy: i32) -> Option<usize> {
    let (cells, cur) = app(|a| {
        let g = geo(a);
        let cells = (0..a.s.panes()).filter(|&i| a.panes[i].is_some()).map(|i| (i, cell_rect(&g, &a.s.heights, i))).collect::<Vec<_>>();
        (cells, a.s.active)
    });
    neighbour(&cells, cur, dx, dy)
}

/// Ctrl+Alt+Shift+arrow: the active pane trades places with the one in that direction and stays active.
pub(crate) unsafe fn swap_dir(dx: i32, dy: i32) {
    if let Some(j) = active_neighbour(dx, dy) {
        swap_panes(app(|a| a.s.active), j);
    }
}

/// Two panes trade places. Nothing is rebuilt, so each keeps its folder, history, selection, scroll, filter and columns.
pub(crate) unsafe fn swap_panes(i: usize, j: usize) {
    let Some([si, sj]) = app(|a| {
        let live = |k: usize| k < a.s.panes() && a.panes[k].is_some();
        (i != j && live(i) && live(j) && a.drag.is_none()).then(|| [i, j].map(|k| a.panes[k].as_ref().map(|p| p.slot.clone()).unwrap_or_default()))
    }) else {
        return;
    };
    super::recent::note_swap(i, j);
    {
        // Filters and slots together, so a view thread sees both before or both after.
        let mut f = FILTERS.lock().unwrap_or_else(|e| e.into_inner());
        f.swap(i, j);
        si.store(j, Ordering::SeqCst);
        sj.store(i, Ordering::SeqCst);
    }
    let panes = app(|a| {
        a.live_filters.swap(i, j);
        a.panes.swap(i, j);
        let mut base = a.base.take();
        swap_in(&mut a.s, base.as_mut(), i, j);
        a.base = base;
        let tab = a.s.tab;
        if let Some(b) = a.tab_base.get_mut(tab) {
            swap_tab_slots(&mut b.grid, i, j);
        }
        a.s.anchor();
        a.zoom = None;
        let to = |k: &mut usize| {
            if *k == i {
                *k = j;
            } else if *k == j {
                *k = i;
            }
        };
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
        let _ = pull_dirty(a);
        [i, j].map(|k| a.panes[k].as_ref().map(|p| p.hwnd))
    });
    for k in [i, j] {
        SEL_REV[k].fetch_add(1, Ordering::Relaxed);
    }
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    relayout();
    for (k, h) in panes.into_iter().enumerate() {
        if let Some(h) = h {
            name_fields(h, if k == 0 { i } else { j });
        }
    }
    redraw_all();
    refresh_save_button();
    note_save();
    set_active(j);
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

/// The left panel (folder tree and favourites) gives its room to the panes. The width is kept.
pub(crate) unsafe fn toggle_tree() {
    if app(|a| a.drag.is_some()) {
        return;
    }
    let f = GetFocus();
    let (had, i) = app(|a| {
        a.s.tree_on ^= true;
        let on = |h: HWND| !h.0.is_null() && (h == f || IsChild(h, f).as_bool());
        (on(a.tree_hwnd) || on(a.side_hwnd), a.s.active)
    });
    relayout();
    redraw_all();
    if had {
        focus_view(i);
    }
    save_session();
    settings::redraw();
}

/// The preview pane follows the active pane. Hidden until Alt+P or the view menu. The width is kept.
pub(crate) unsafe fn toggle_preview() {
    if app(|a| a.drag.is_some()) {
        return;
    }
    let f = GetFocus();
    let (had, i) = app(|a| {
        a.s.preview_on ^= true;
        let ph = a.preview_hwnd;
        let inside = !ph.0.is_null() && (ph == f || IsChild(ph, f).as_bool());
        (inside, a.s.active)
    });
    publish_shared();
    across_windows(|| {
        relayout();
        redraw_all();
        if app(|a| a.s.preview_on) {
            run_preview(true);
        }
    });
    if had {
        focus_view(i);
    }
    save_session();
    settings::redraw();
}

fn blank_pane_slot(s: &mut Session, i: usize) {
    if i >= MAXP {
        return;
    }
    if i < s.paths.len() {
        s.paths[i].clear();
    }
    if i < s.views.len() {
        s.views[i] = (0, 0);
    }
    if i < s.cols.len() {
        s.cols[i].clear();
    }
    if i < s.tints.len() {
        s.tints[i] = 0;
    }
    if i < s.pane_folders.len() {
        s.pane_folders[i].clear();
    }
    if i < s.pane_cur.len() {
        s.pane_cur[i] = 0;
    }
}

pub(crate) unsafe fn set_layout(h: Vec<usize>) {
    commit_layout(h, None, None, None, None, None);
}

/// `planned` is a header-menu split or close. None is the layout picker (`remap`).
/// `seed` is `(source pane, free old slot)` copied after views are saved and before panes move.
/// `blank_old` is a closed pane: its folder, tabs, tint and columns are dropped, and it does not return.
/// `focus` is the new slot to activate. None keeps the pane that was active, or the nearest cell if that cell left.
/// `ratios` keeps a copy-split's column and row sizes. None lets `set_heights` even them.
pub(crate) unsafe fn commit_layout(
    h: Vec<usize>,
    planned: Option<([usize; MAXP], Homes)>,
    seed: Option<(usize, usize)>,
    blank_old: Option<usize>,
    focus: Option<usize>,
    ratios: Option<(Vec<f64>, Vec<Vec<f64>>)>,
) {
    if h.is_empty() || h.iter().any(|&n| n == 0) {
        return;
    }
    let menu_edit = planned.is_some();
    app(|a| a.focus_req = HWND::default());
    for i in 0..MAXP {
        save_view(i);
    }
    let (perm, slots) = app(|a| {
        let (perm, home) = match planned {
            Some((perm, home)) => (perm, home),
            None => remap(&a.s.heights, &h, &a.home),
        };
        if let Some((src, fresh)) = seed {
            copy_pane_view(&mut a.s, src, fresh);
            if fresh < a.s.tints.len() {
                a.s.tints[fresh] = 0;
            }
            a.live_filters[fresh].clear();
        }
        a.home = home;
        (perm, perm.map(|k| a.panes.get(k).and_then(|p| p.as_ref()).map(|p| p.slot.clone())))
    });
    if let Some((_, fresh)) = seed {
        if let Ok(mut f) = FILTERS.lock() {
            f[fresh].clear();
        }
    }
    {
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
    let blank_at = blank_old.and_then(|old| perm.iter().position(|&p| p == old));
    if let Some(at) = blank_at {
        if let Ok(mut f) = FILTERS.lock() {
            f[at].clear();
        }
    }
    let gone = app(|a| {
        let (old_h, old_active) = (a.s.heights.clone(), a.s.active);
        let old_f = a.live_filters.clone();
        for (j, &k) in perm.iter().enumerate() {
            a.live_filters[j] = old_f[k].clone();
        }
        let mut taken: Vec<Option<Pane>> = a.panes.iter_mut().map(Option::take).collect();
        a.panes = perm.iter().map(|&k| taken.get_mut(k).and_then(Option::take)).collect();
        let mut base = a.base.take();
        permute_in(&mut a.s, base.as_mut(), &perm);
        a.base = base;
        let tab = a.s.tab;
        let order: [u8; MAXP] = std::array::from_fn(|i| perm[i] as u8);
        if let Some(b) = a.tab_base.get_mut(tab) {
            reorder_tab_slots(&mut b.grid, &order);
        }
        if let Some(at) = blank_at {
            blank_pane_slot(&mut a.s, at);
            a.live_filters[at].clear();
            a.home[at] = None;
        }
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
        // permute_in already moved `active` with its pane. Off the new grid means that cell is gone.
        let gone = if let Some(f) = focus {
            a.s.active = f.min(n.saturating_sub(1));
            true
        } else {
            let gone = a.s.active >= n;
            if gone {
                let (c, r) = slot(&old_h, old_active);
                let c = c.min(h.len() - 1);
                a.s.active = h[..c].iter().sum::<usize>() + r.min(h[c] - 1);
            }
            gone
        };
        if menu_edit {
            let active = a.s.active;
            let fix = |k: &mut usize| {
                if *k >= n {
                    *k = active;
                }
            };
            if let Some(d) = a.drop_open.as_mut() {
                fix(&mut d.1);
            }
            if let Some(m) = a.matcher.as_mut() {
                fix(&mut m.pane);
            }
            if let Some(k) = a.kids.as_mut() {
                fix(&mut k.i);
            }
            fix(&mut a.peek_pane);
        }
        a.s.set_heights(h);
        if let Some((colr, rowr)) = ratios {
            a.s.colr = colr;
            a.s.rowr = rowr;
        }
        a.s.anchor();
        let _ = pull_dirty(a);
        gone
    });
    NAV_GEN.fetch_add(1, Ordering::SeqCst);
    relayout();
    // Destroying the closed pane writes its view back onto the parked slot. Drop that too.
    if let Some(at) = blank_at {
        app(|a| {
            blank_pane_slot(&mut a.s, at);
            a.live_filters[at].clear();
            a.home[at] = None;
            a.s.anchor();
            let _ = pull_dirty(a);
        });
        if let Ok(mut f) = FILTERS.lock() {
            f[at].clear();
        }
    }
    let named = app(|a| a.panes.iter().enumerate().filter_map(|(i, p)| p.as_ref().map(|p| (p.hwnd, i))).collect::<Vec<_>>());
    for (hwnd, i) in named {
        name_fields(hwnd, i);
    }
    redraw_all();
    refresh_save_button();
    note_save();
    if gone {
        let i = app(|a| a.s.active);
        if view_of::<IShellView>(i).is_some() {
            if let Some(item) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellItem>().ok()) {
                sync_tree(&item);
            }
            focus_view(i);
        } else {
            app(|a| a.focus_req = a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd).unwrap_or_default());
        }
        mark_drive();
    }
}

// Bumped only when the effective dark bit changes. Panes record the value they were built under.
pub(crate) static THEME_EPOCH: AtomicU64 = AtomicU64::new(0);

fn note_theme_epoch() {
    THEME_EPOCH.fetch_add(1, Ordering::Relaxed);
}

/// Same folder again, without a history entry and without selecting the first row.
/// `SBSP_ABSOLUTE` is 0. The other bits are `SBSP_ACTIVATE_NOFOCUS`, `SBSP_CREATENOHISTORY`,
/// `SBSP_NOAUTOSELECT`, and `SBSP_WRITENOHISTORY`.
pub(crate) const THEME_BROWSE_FLAGS: u32 = 0x0008_0000 | 0x0010_0000 | 0x0400_0000 | 0x0800_0000;

/// A live pane whose shell view was built under a different theme, and is not already rebuilding.
pub(crate) fn pane_theme_due(built: u64, epoch: u64, busy: bool) -> bool {
    epoch != built && !busy
}

/// How many rows to scroll so `want_top` becomes the top row. None when it already is, or the list is empty.
pub(crate) fn scroll_rows(cur_top: i32, want_top: i32, count: i32) -> Option<i32> {
    if count <= 0 {
        return None;
    }
    let want = want_top.clamp(0, count - 1);
    let cur = cur_top.clamp(0, count - 1);
    (want != cur).then_some(want - cur)
}

/// Details and list scroll by row. Icon, tile and thumbnail scroll by pixel.
pub(crate) fn theme_scroll_rows(mode: i32) -> bool {
    matches!(mode, 0 | 3 | 4)
}

pub(crate) fn theme_keep_path(i: usize) -> Option<String> {
    app(|a| a.panes.get(i).and_then(|p| p.as_ref()).and_then(|p| p.theme_keep.as_ref()).map(|k| k.path.clone()))
}

pub(crate) fn clear_theme_keep(i: usize) {
    app(|a| {
        if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
            p.theme_keep.take();
            p.theme_scroll.take();
        }
    });
}

/// One posted step. Each step rebuilds a single visible pane, so a slow folder cannot stall the rest of the UI.
pub(crate) unsafe fn schedule_theme_panes() {
    let main = app(|a| a.main);
    if !main.0.is_null() {
        _ = PostMessageW(Some(main), WM_THEME_PANE, WPARAM(0), LPARAM(0));
    }
}

/// Recolour shell views that are about to be rebuilt. Called when a hidden tab becomes visible.
pub(crate) unsafe fn recolor_due_panes() {
    let epoch = THEME_EPOCH.load(Ordering::Relaxed);
    let hwnds = app(|a| {
        a.panes
            .iter()
            .flatten()
            .filter(|p| pane_theme_due(p.themed, epoch, p.theme_keep.is_some()))
            .map(|p| p.hwnd)
            .collect::<Vec<_>>()
    });
    for h in hwnds {
        paint_list_theme(h);
    }
}

pub(crate) unsafe fn theme_pane_step() {
    let epoch = THEME_EPOCH.load(Ordering::Relaxed);
    let i = app(|a| {
        a.panes.iter().enumerate().find_map(|(i, p)| {
            let p = p.as_ref()?;
            pane_theme_due(p.themed, epoch, p.theme_keep.is_some()).then_some(i)
        })
    });
    let Some(i) = i else { return };
    refresh_one_pane(i, epoch);
    let more = app(|a| a.panes.iter().flatten().any(|p| pane_theme_due(p.themed, epoch, p.theme_keep.is_some())));
    if more {
        schedule_theme_panes();
    }
}

/// True when pane `i`'s shell view hosts a DirectUIHWND items surface (typical on Windows 10).
/// That surface takes light/dark only when it is created; mid-process remake under a new
/// PreferredAppMode still keeps the first-birth colour on Win10 22H2.
pub(crate) unsafe fn dui_list(i: usize) -> bool {
    let view = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok());
    view.is_some_and(|v| FindWindowExW(Some(v), None, w!("DirectUIHWND"), PCWSTR::null()).is_ok())
}

unsafe fn any_dui_pane() -> bool {
    let n = app(|a| a.panes.len());
    (0..n).any(|i| app(|a| a.panes.get(i).and_then(|p| p.as_ref()).is_some_and(|p| IsWindow(Some(p.hwnd)).as_bool())) && dui_list(i))
}

/// After the current message returns (and the caller has saved the session), relaunch so new
/// DirectUIHWND surfaces are born under ForceLight/ForceDark. Paths, filters, and layout come
/// back from the session; the ExplorerBrowser travel log resets (same as an in-place remake).
pub(crate) unsafe fn schedule_theme_relaunch() {
    let main = app(|a| a.main);
    if !main.0.is_null() {
        _ = PostMessageW(Some(main), WM_THEME_RELAUNCH, WPARAM(0), LPARAM(0));
    }
}

pub(crate) unsafe fn theme_relaunch() {
    save_session();
    let _ = crate::relaunch_same_session();
}

unsafe fn mark_themed(i: usize, epoch: u64) {
    app(|a| {
        if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
            p.themed = epoch;
        }
    });
}

unsafe fn refresh_one_pane(i: usize, epoch: u64) {
    let hwnd = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd));
    let Some(hwnd) = hwnd else { return };
    paint_list_theme(hwnd);
    // SysListView32 (and any non-DUI surface): in-place recolour is enough. DirectUI theme
    // flips go through schedule_theme_relaunch instead of this path.
    let pending_net = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).is_some_and(|p| p.net.as_ref().is_some_and(|n| !n.bad)));
    if pending_net || !dui_list(i) {
        mark_themed(i, epoch);
        return;
    }
    // DUI pane still due: remake cannot fix birth theme — ask for relaunch once.
    mark_themed(i, epoch);
    schedule_theme_relaunch();
}

unsafe fn capture_theme(i: usize) -> Option<ThemeKeep> {
    let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    if path.is_empty() {
        return None;
    }
    let fv = view_of::<IFolderView2>(i)?;
    let snap = read_snap(i);
    let watch = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).and_then(|p| p.view_watch.clone()));
    let mode = snap.as_ref().map(|s| s.mode).unwrap_or(0);
    let lv = view_list(&fv);
    let top = if is_sys_list(lv) { SendMessageW(lv, LVM_GETTOPINDEX, None, None).0 as i32 } else { 0 };
    let (sx, sy) = if is_sys_list(lv) { list_scroll(lv) } else { (0, 0) };
    let total = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let nsel = fv.ItemCount(SVGIO_SELECTION).unwrap_or(0);
    let select_all = total > 0 && nsel >= total;
    let sel = if select_all {
        Vec::new()
    } else {
        crate::selection_pidls(i).into_iter().map(|p| p as isize).filter(|&p| p != 0).collect()
    };
    let focus = fv.GetFocusedItem().ok().filter(|&idx| idx >= 0).and_then(|idx| fv.Item(idx).ok()).map(|p| p as isize).filter(|&p| p != 0).unwrap_or(0);
    let had_focus = shell_focused(i);
    Some(ThemeKeep { path, snap, watch, sel, focus, select_all, top, sx, sy, mode, had_focus })
}

unsafe fn shell_focused(i: usize) -> bool {
    let focus = GetFocus();
    view_of::<IShellView>(i).and_then(|sv| sv.GetWindow().ok()).is_some_and(|w| w == focus || IsChild(w, focus).as_bool())
}

unsafe fn list_scroll(lv: HWND) -> (i32, i32) {
    if lv.0.is_null() {
        return (0, 0);
    }
    let mut si = SCROLLINFO { cbSize: size_of::<SCROLLINFO>() as u32, fMask: SIF_POS, ..Default::default() };
    let x = if GetScrollInfo(lv, SB_HORZ, &mut si).is_ok() { si.nPos } else { 0 };
    let y = if GetScrollInfo(lv, SB_VERT, &mut si).is_ok() { si.nPos } else { 0 };
    (x, y)
}

const LVM_GETITEMCOUNT: u32 = 0x1004;
const LVM_GETTOPINDEX: u32 = 0x1027;
const LVM_SCROLL: u32 = 0x1014;
const LVM_GETITEMRECT: u32 = 0x100E;
const LVM_SETITEMSTATE: u32 = 0x102B;

pub(crate) unsafe fn apply_theme_scroll(h: HWND) {
    let Some(i) = pane_of(h) else { return };
    let Some(mut job) = app(|a| a.panes.get_mut(i).and_then(|p| p.as_mut()).and_then(|p| p.theme_scroll.take())) else { return };
    // The shell may publish the new list a moment after navigation completes.
    if job.tries == 8 {
        paint_list_theme(h);
    }
    let Some(fv) = view_of::<IFolderView2>(i) else {
        repost_scroll(h, i, job);
        return;
    };
    let lv = view_list(&fv);
    let sys = is_sys_list(lv);
    // DirectUIHWND does not answer LVM_GETITEMCOUNT. The folder view's own count is the row count.
    let count = if sys { SendMessageW(lv, LVM_GETITEMCOUNT, None, None).0 as i32 } else { fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0) };
    let wants = if theme_scroll_rows(job.mode) { job.top > 0 } else { job.x != 0 || job.y != 0 };
    let waiting_sel = job.select_all || !job.sel.is_empty() || job.focus != 0;
    // The list is filled after navigation completes. Keep the PIDLs until it has rows, or the tries run out.
    if count == 0 && (wants || waiting_sel) && job.tries > 1 {
        repost_scroll(h, i, job);
        return;
    }
    if job.select_all && count > 0 {
        if sys {
            select_all_list(lv);
        } else {
            let flags = SVSI_SELECT.0 as u32;
            for k in 0..count {
                _ = fv.SelectItem(k, flags);
            }
        }
        job.select_all = false;
    }
    restore_theme_selection(i, &mut job);
    if sys && count > 0 {
        if theme_scroll_rows(job.mode) {
            let cur = SendMessageW(lv, LVM_GETTOPINDEX, None, None).0 as i32;
            if let Some(rows) = scroll_rows(cur, job.top, count) {
                let px = list_row_px(lv).max(1);
                SendMessageW(lv, LVM_SCROLL, Some(WPARAM(0)), Some(LPARAM(rows.saturating_mul(px) as isize)));
            }
        } else if !job.y_done && job.y != 0 {
            SendMessageW(lv, LVM_SCROLL, Some(WPARAM(0)), Some(LPARAM(job.y as isize)));
            job.y_done = true;
        }
        if !job.x_done && job.x != 0 {
            SendMessageW(lv, LVM_SCROLL, Some(WPARAM(job.x as usize)), Some(LPARAM(0)));
            job.x_done = true;
        }
    }
    if theme_scroll_rows(job.mode) && job.top > 0 && sys && count > 0 {
        let cur = SendMessageW(lv, LVM_GETTOPINDEX, None, None).0 as i32;
        let count = SendMessageW(lv, LVM_GETITEMCOUNT, None, None).0 as i32;
        if scroll_rows(cur, job.top, count).is_some() {
            repost_scroll(h, i, job);
        }
    }
}

fn repost_scroll(h: HWND, i: usize, mut job: ThemeScroll) {
    if job.tries <= 1 {
        return;
    }
    job.tries -= 1;
    app(|a| {
        if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
            p.theme_scroll = Some(job);
        }
    });
    unsafe { _ = PostMessageW(Some(h), WM_THEME_SCROLL, WPARAM(0), LPARAM(0)) };
}

unsafe fn list_row_px(lv: HWND) -> i32 {
    let mut rc = RECT::default();
    if SendMessageW(lv, LVM_GETITEMRECT, Some(WPARAM(0)), Some(LPARAM(&mut rc as *mut _ as isize))).0 == 0 {
        return 1;
    }
    (rc.bottom - rc.top).max(1)
}

/// Put the saved child PIDLs back. They are freed here, so a later scroll retry does not select twice.
unsafe fn restore_theme_selection(i: usize, job: &mut ThemeScroll) {
    if job.sel.is_empty() && job.focus == 0 {
        return;
    }
    let Some(sv) = view_of::<IShellView>(i) else { return };
    let focus = std::mem::replace(&mut job.focus, 0);
    if !job.select_all {
        for p in job.sel.drain(..) {
            if p != 0 && p != focus {
                _ = sv.SelectItem(p as *const ITEMIDLIST, SVSI_SELECT.0 as u32);
                CoTaskMemFree(Some(p as *const std::ffi::c_void));
            }
        }
    }
    if focus != 0 {
        let flags = (SVSI_FOCUSED.0 | SVSI_SELECT.0) as u32;
        _ = sv.SelectItem(focus as *const ITEMIDLIST, flags);
        CoTaskMemFree(Some(focus as *const std::ffi::c_void));
    }
}

unsafe fn select_all_list(lv: HWND) {
    use windows::Win32::UI::Controls::{LVITEMW, LIST_VIEW_ITEM_STATE_FLAGS};
    let mut item = LVITEMW {
        state: LIST_VIEW_ITEM_STATE_FLAGS(2), // LVIS_SELECTED
        stateMask: LIST_VIEW_ITEM_STATE_FLAGS(2),
        ..Default::default()
    };
    SendMessageW(lv, LVM_SETITEMSTATE, Some(WPARAM(usize::MAX)), Some(LPARAM(&mut item as *mut _ as isize)));
}

// Same switch as Explorer's "Hidden items" (global shell setting).
#[cfg(test)]
mod tests {
    use super::{pane_theme_due, scroll_rows, theme_scroll_rows, THEME_BROWSE_FLAGS};
    use crate::imports::*;

    // Setting the path on a narrow edit leaves the drive on screen. The control clips the tail.
    #[test]
    fn narrow_address_edit_shows_the_start_of_the_path() {
        const EM_POSFROMCHAR: u32 = 0x00D6;
        let path = r"C:\Users\Jun\Documents\VeryLongFolderName\Projects\Maguro\src\pane";
        let x_of = |edit: HWND, index: usize| {
            let r = unsafe { SendMessageW(edit, EM_POSFROMCHAR, Some(WPARAM(index)), Some(LPARAM(0))) };
            (r.0 as u16) as i16 as i32
        };
        unsafe {
            let parent = CreateWindowExW(WINDOW_EX_STYLE(0), w!("Static"), w!(""), WS_POPUP, 0, 0, 400, 80, None, None, None, None).unwrap_or_default();
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("Edit"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                0,
                0,
                80,
                24,
                Some(parent),
                None,
                None,
                None,
            )
            .unwrap_or_default();
            assert!(!parent.0.is_null() && !edit.0.is_null(), "the address edit could not be created");
            _ = SetWindowTextW(edit, &HSTRING::from(path));
            let (start, tail) = (x_of(edit, 0), x_of(edit, path.len() - 1));
            _ = DestroyWindow(edit);
            _ = DestroyWindow(parent);
            assert!(start >= 0 && start < 80, "the drive stays inside the narrow edit (x={start})");
            assert!(tail >= 80, "the last folder is past the right edge (x={tail})");
        }
    }

    #[test]
    fn theme_rebuild_does_not_write_history_or_travel() {
        let f = THEME_BROWSE_FLAGS;
        assert_eq!(f & 0x0000_2000, 0); // parent
        assert_eq!(f & 0x0000_4000, 0); // back
        assert_eq!(f & 0x0000_8000, 0); // forward
        assert_ne!(f & 0x0008_0000, 0); // activate without taking focus
        assert_ne!(f & 0x0010_0000, 0); // create no history
        assert_ne!(f & 0x0400_0000, 0); // do not select the first row
        assert_ne!(f & 0x0800_0000, 0); // write no history
    }

    #[test]
    fn only_a_stale_idle_pane_is_rebuilt() {
        assert!(!pane_theme_due(0, 0, false));
        assert!(pane_theme_due(0, 1, false));
        assert!(!pane_theme_due(1, 1, false));
        assert!(!pane_theme_due(0, 1, true)); // already rebuilding
        // Hidden tabs are not in the live list the scheduler walks.
        let live = [(0u64, false), (1, false), (0, true)];
        let due = live.iter().filter(|&&(built, busy)| pane_theme_due(built, 1, busy)).count();
        assert_eq!(due, 1);
    }

    #[test]
    fn scroll_restore_moves_by_the_row_gap() {
        assert_eq!(scroll_rows(0, 0, 10), None);
        assert_eq!(scroll_rows(0, 12, 40), Some(12));
        assert_eq!(scroll_rows(12, 4, 40), Some(-8));
        assert_eq!(scroll_rows(0, 100, 10), Some(9)); // clamped to the last row
        assert_eq!(scroll_rows(3, 3, 0), None);
        assert!(theme_scroll_rows(4) && theme_scroll_rows(3) && theme_scroll_rows(0));
        assert!(!theme_scroll_rows(1) && !theme_scroll_rows(5) && !theme_scroll_rows(6));
    }
}

pub(crate) unsafe fn hidden() -> bool {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, false);
    ss._bitfield1 & 1 != 0
}

pub(crate) unsafe fn toggle_hidden() {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, false);
    ss._bitfield1 ^= 1;
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, true);
    for i in 0..MAXP {
        if let Some(sv) = view_of::<IShellView>(i) {
            _ = sv.Refresh();
        }
    }
    settings::redraw();
}

