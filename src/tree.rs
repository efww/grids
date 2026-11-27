use crate::*;

// ---------- shell tree (sidebar) ----------

#[implement(INameSpaceTreeControlEvents)]
pub(crate) struct TreeEvents;

impl INameSpaceTreeControlEvents_Impl for TreeEvents_Impl {
    fn OnItemClick(&self, psi: Ref<IShellItem>, hit: u32, click: u32) -> Result<()> {
        // Re-clicking the already selected folder fires no selection change; navigate here instead.
        let on_item = hit & NSTCEHT_ONITEM.0 as u32 != 0 && click & NSTCECT_LBUTTON.0 as u32 != 0;
        if let (true, Some(si), Some(t)) = (on_item, psi.as_ref(), app(|a| a.tree.clone())) {
            if unsafe { t.GetItemState(si, NSTCIS_SELECTED.0 as u32) }.unwrap_or(0) != 0 {
                unsafe { browse_item(app(|a| a.s.active), &real_item(si)) };
            }
        }
        Err(S_FALSE.into()) // let the tree do its default handling
    }
    fn OnSelectionChanged(&self, sel: Ref<IShellItemArray>) -> Result<()> {
        // The tree selects its root on its own at startup; only follow selections right after user input.
        // (Focus is no good here: on a click the selection changes before the tree takes focus.)
        let user = app(|a| std::mem::take(&mut a.tree_input));
        if let Some(item) = sel.as_ref().filter(|_| user).and_then(|s| unsafe { s.GetItemAt(0) }.ok()) {
            unsafe { browse_item(app(|a| a.s.active), &real_item(&item)) };
        }
        Ok(())
    }
    fn OnPropertyItemCommit(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemStateChanging(&self, _: Ref<IShellItem>, _: u32, _: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemStateChanged(&self, _: Ref<IShellItem>, _: u32, _: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnKeyboardInput(&self, _: u32, _: WPARAM, _: LPARAM) -> Result<()> {
        Err(S_FALSE.into())
    }
    fn OnBeforeExpand(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnAfterExpand(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeginLabelEdit(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnEndLabelEdit(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnGetToolTip(&self, _: Ref<IShellItem>, _: PWSTR, _: i32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeItemDelete(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemAdded(&self, _: Ref<IShellItem>, _: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemDeleted(&self, _: Ref<IShellItem>, _: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeContextMenu(&self, _: Ref<IShellItem>, _: *const GUID, _: *mut *mut core::ffi::c_void) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnAfterContextMenu(&self, _: Ref<IShellItem>, _: Ref<IContextMenu>, _: *const GUID, _: *mut *mut core::ffi::c_void) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeStateImageChange(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnGetDefaultIconIndex(&self, _: Ref<IShellItem>, _: *mut i32, _: *mut i32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

pub(crate) unsafe fn create_tree(main: HWND) -> Result<(INameSpaceTreeControl, HWND)> {
    let t: INameSpaceTreeControl = CoCreateInstance(&NamespaceTreeControl, None, CLSCTX_INPROC_SERVER)?;
    let style = NSTCS_HASEXPANDOS.0
        | NSTCS_ROOTHASEXPANDO.0
        | NSTCS_FADEINOUTEXPANDOS.0
        | NSTCS_NOINFOTIP.0
        | NSTCS_ALLOWJUNCTIONS.0
        | NSTCS_SHOWSELECTIONALWAYS.0
        | NSTCS_FULLROWSELECT.0
        | NSTCS_HORIZONTALSCROLL.0; // a real scrollbar, so a deep folder can be scrolled into view (AUTOHSCROLL has none)
    t.Initialize(main, &RECT::default(), style as u32)?;
    let root: IShellItem = SHGetKnownFolderItem(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None)?;
    t.AppendRoot(&root, SHCONTF_FOLDERS.0 as u32, NSTCRS_EXPANDED.0 as u32, None)?;
    let quick = if app(|a| a.s.tree_quick) { add_quick(&t) } else { None };
    app(|a| a.quick_root = quick);
    let ev: INameSpaceTreeControlEvents = TreeEvents.into();
    t.TreeAdvise(&ev)?;
    let h = t.cast::<IOleWindow>()?.GetWindow()?;
    Ok((t, h))
}

// ---------- Explorer's Quick access atop the tree: the folders pinned there (and, as Explorer is set, frequent ones) ----------

const QUICK_ACCESS: PCWSTR = w!("shell:::{679f85cb-0220-4080-b29b-5540cc05aab6}");

// Quick access as the tree's first root, expanded. None when it can't be had (switched off by policy, say):
// the tree keeps just the Desktop, as before.
unsafe fn add_quick(t: &INameSpaceTreeControl) -> Option<IShellItem> {
    let q: IShellItem = SHCreateItemFromParsingName(QUICK_ACCESS, None).ok()?;
    let flags = (SHCONTF_FOLDERS.0 | SHCONTF_NAVIGATION_ENUM.0) as u32;
    t.InsertRoot(0, &q, flags, NSTCRS_EXPANDED.0 as u32, None).ok()?;
    Some(q)
}

// View > Show Explorer Quick access in the folder tree: only that root goes in or out, the rest of the tree
// (what is expanded, the selection) stays. With no tree only the setting changes.
pub(crate) unsafe fn toggle_tree_quick() {
    let on = app(|a| {
        a.s.tree_quick ^= true;
        a.s.tree_quick
    });
    let Some(t) = app(|a| a.tree.clone()) else { return };
    if on {
        if app(|a| a.quick_root.is_none()) {
            let q = add_quick(&t);
            app(|a| a.quick_root = q);
        }
    } else if let Some(q) = app(|a| a.quick_root.take()) {
        // the selection may sit under the root going away: the tree then drops it or moves it to a
        // neighbour, which must not navigate; put it back on the active pane's folder instead
        app(|a| a.tree_input = false);
        _ = t.RemoveRoot(&q);
        let i = app(|a| a.s.active);
        if let Some(item) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellItem>().ok()) {
            cancel_tree_sync(); // done right here
            sync_tree(&item);
            return;
        }
    }
    reveal_tree_selection();
}

/// "C:\..." : a folder on a drive letter. Only these are reopened by path from Quick access; a pinned share
/// that is offline would hold the window up while its path is parsed again.
pub(crate) fn drive_path(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

// The real path of an item somewhere below the Quick access root (not the root itself), when it is a drive
// path. None for anything else: the Desktop side, Quick access itself, shares, shell places.
unsafe fn quick_path(si: &IShellItem) -> Option<String> {
    let root = app(|a| a.quick_root.clone())?;
    let r = SHGetIDListFromObject(&root).ok()?;
    let path = SHGetIDListFromObject(si).ok().map(|p| {
        let below = ILIsParent(r, p, false).as_bool() && !ILIsEqual(r, p).as_bool();
        let path = if below { name(p, SIGDN_DESKTOPABSOLUTEPARSING) } else { String::new() };
        CoTaskMemFree(Some(p as _));
        path
    });
    CoTaskMemFree(Some(r as _));
    path.filter(|p| drive_path(p))
}

// What a tree pick opens: a folder under Quick access by its real path, so the pane's address, tab and
// Recent hold that path rather than a Quick access item. Anything else (or a path that won't parse) as it is.
unsafe fn real_item(si: &IShellItem) -> IShellItem {
    quick_path(si).and_then(|p| SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(p), None).ok()).unwrap_or_else(|| si.clone())
}


// The sidebar follows the active pane: expand down to its folder and select it.
// (Programmatic selection doesn't navigate: OnSelectionChanged only follows user input.)
pub(crate) unsafe fn sync_tree(item: &IShellItem) {
    let Some((t, main, th)) = app(|a| a.tree.clone().map(|t| (t, a.main, a.tree_hwnd))) else { return };
    // Grids picks this selection itself: a mark left by a tree click that changed nothing must not navigate
    app(|a| a.tree_input = false);
    // a folder picked under Quick access opened by its real path: that row stays selected (and scrolled to),
    // rather than the selection jumping to the same folder on the Desktop side
    if let Ok(sel) = t.GetSelectedItems().and_then(|s| s.GetItemAt(0)) {
        if let Some(q) = quick_path(&sel) {
            let here = item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING).map(|p| pwstr(p)).unwrap_or_default();
            if q.eq_ignore_ascii_case(&here) {
                return;
            }
        }
    }
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

/// TIMER_TREE_SYNC: the tree follows only while shown, only when asked since, and never over a click or key
/// that just reached the tree (its own pick goes on to navigate).
pub(crate) fn tree_sync_due(shown: bool, wanted: bool, tree_input: bool) -> bool {
    shown && wanted && !tree_input
}

// A pane moved or another pane became active: the tree follows a moment later (TIMER_TREE_SYNC), once,
// to whatever the active pane shows then. A hidden tree isn't expanded at all; showing it again syncs.
pub(crate) unsafe fn request_tree_sync() {
    let (main, shown) = app(|a| (a.main, a.s.tree && a.tree.is_some()));
    app(|a| a.tree_want = shown && !main.0.is_null());
    if main.0.is_null() {
        return;
    }
    if shown {
        SetTimer(Some(main), TIMER_TREE_SYNC, 100, None);
    } else {
        _ = KillTimer(Some(main), TIMER_TREE_SYNC);
    }
}

// Drops a waiting follow: the tree was hidden, or the user is picking in it.
pub(crate) unsafe fn cancel_tree_sync() {
    let main = app(|a| {
        a.tree_want = false;
        a.main
    });
    if !main.0.is_null() {
        _ = KillTimer(Some(main), TIMER_TREE_SYNC);
    }
}

// The tree selects the active pane's folder now.
pub(crate) unsafe fn sync_tree_active() {
    let i = app(|a| a.s.active);
    if let Some(item) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellItem>().ok()) {
        sync_tree(&item);
    }
}

pub(crate) unsafe fn tree_sync_tick(h: HWND) {
    _ = KillTimer(Some(h), TIMER_TREE_SYNC);
    if app(|a| tree_sync_due(a.s.tree, std::mem::take(&mut a.tree_want), a.tree_input)) {
        sync_tree_active();
    }
}

// F6 into the tree: a waiting follow runs first, so the arrows start from the folder shown now.
pub(crate) unsafe fn flush_tree_sync() {
    if app(|a| a.tree_want) {
        cancel_tree_sync();
        if app(|a| a.s.tree && !a.tree_input) {
            sync_tree_active();
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_follows_only_when_shown_asked_and_untouched() {
        assert!(tree_sync_due(true, true, false));
        assert!(!tree_sync_due(false, true, false)); // hidden: nothing expanded
        assert!(!tree_sync_due(true, false, false)); // nothing asked (already done, or cancelled)
        assert!(!tree_sync_due(true, true, true)); // a click/key in the tree: its own pick wins
    }

    #[test]
    fn quick_access_reopens_drive_paths_only() {
        for p in [r"C:\", r"C:\Users\홍길동\Downloads", r"z:\a b"] {
            assert!(drive_path(p), "{p}");
        }
        // shares, shell places, Quick access itself and odd text keep the Quick access item as it is
        for p in [r"\\nas\share", r"::{679F85CB-0220-4080-B29B-5540CC05AAB6}", r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}\::{x}", "C:", "C:relative", "", "1:\\x", "가:\\"] {
            assert!(!drive_path(p), "{p}");
        }
    }
}
