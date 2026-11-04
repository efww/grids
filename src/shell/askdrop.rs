// "Always ask" (dropm == 2) shows Copy / Move / Cancel for a file drop inside a pane,
// on the folder tree, and on the middle of a favourite folder. Other drop settings stay
// with the shell (or, for favourites, with the link they already insert).
//
// The copy or move itself is `start_copy` / `start_drop_move` on a worker thread.
// A deferred MoveItem must not report DROPEFFECT_MOVE from Drop. Some shell drag
// sources delete the originals as soon as they see MOVE, before our worker has
// settled and called MoveItem — the file then vanishes. Report COPY instead so the
// source keeps the files; MoveItem removes them after Drop returns. Copy does not
// need that wait. `mark_move_performed` stays for callers that finish the move
// before Drop returns.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL};
use windows::Win32::System::Ole::ReleaseStgMedium;
use windows::Win32::UI::Controls::{TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOGCONFIG_1, TASKDIALOG_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION};

use crate::imports::*;
use crate::pwstr;
use crate::app::*;
use crate::i18n::*;
use crate::pane::*;
use crate::session::*;
use super::fileop::{paste_mode, start_copy, start_drop_move};

/// What Drop returns to the source. A move is reported as a copy when the source was not
/// told that the move already happened.
pub(crate) fn reported_effect(mv: bool, performed: bool) -> u32 {
    if mv && performed { DROPEFFECT_MOVE.0 } else { DROPEFFECT_COPY.0 }
}

/// Where a pane drop lands. `child` is set only when the row under the cursor is a local
/// directory (a share is not stat'd, so it stays `None` and the current folder is used).
pub(crate) fn drop_folder(current: &str, child: Option<&str>) -> Option<String> {
    if let Some(path) = child.filter(|p| is_fs(p)) {
        return Some((*path).to_string());
    }
    is_fs(current).then(|| current.to_string())
}

/// Middle of a favourite folder row, and only while the drop action is Always ask.
/// Edges stay an insert. Groups, files, and shell places are not copy targets.
pub(crate) fn fav_into_folder(dropm: u8, group: bool, file: bool, path: &str, y: i32, rh: i32) -> bool {
    dropm == 2 && !group && !file && is_fs(path) && rh > 0 && y >= rh / 4 && y < rh * 3 / 4
}

/// Some(true) is Move, Some(false) is Copy, None is Cancel.
pub(crate) unsafe fn ask_copy_or_move(owner: HWND) -> Option<bool> {
    let title = HSTRING::from("Maguro");
    let head = HSTRING::from(tr(crate::Key::CopyOrMoveTheseItems));
    let copy = HSTRING::from(tr(crate::Key::Copy));
    let mv = HSTRING::from(tr(crate::Key::Move));
    let cancel = HSTRING::from(tr(crate::Key::Cancel));
    let buttons = [
        TASKDIALOG_BUTTON { nButtonID: 100, pszButtonText: PCWSTR(copy.as_ptr()) },
        TASKDIALOG_BUTTON { nButtonID: 101, pszButtonText: PCWSTR(mv.as_ptr()) },
        TASKDIALOG_BUTTON { nButtonID: 2, pszButtonText: PCWSTR(cancel.as_ptr()) },
    ];
    let cfg = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        hInstance: HINSTANCE::default(),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        dwCommonButtons: Default::default(),
        pszWindowTitle: PCWSTR(title.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: PCWSTR::null() },
        pszMainInstruction: PCWSTR(head.as_ptr()),
        pszContent: PCWSTR::null(),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: 100,
        cRadioButtons: 0,
        pRadioButtons: std::ptr::null(),
        nDefaultRadioButton: 0,
        pszVerificationText: PCWSTR::null(),
        pszExpandedInformation: PCWSTR::null(),
        pszExpandedControlText: PCWSTR::null(),
        pszCollapsedControlText: PCWSTR::null(),
        Anonymous2: TASKDIALOGCONFIG_1 { hFooterIcon: HICON::default() },
        pszFooter: PCWSTR::null(),
        pfCallback: None,
        lpCallbackData: 0,
        cxWidth: 0,
    };
    let mut pressed = 0i32;
    if TaskDialogIndirect(&cfg, Some(&mut pressed), None, None).is_err() {
        return None;
    }
    match pressed {
        100 => Some(false),
        101 => Some(true),
        _ => None,
    }
}

/// Tell the source the move already happened, so it does not delete the originals.
#[allow(dead_code)] // kept for a sync move that finishes before Drop returns
pub(crate) unsafe fn mark_move_performed(obj: &IDataObject) -> bool {
    let fmt = RegisterClipboardFormatW(w!("Performed DropEffect"));
    if fmt == 0 {
        return false;
    }
    let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, 4) else { return false };
    let p = GlobalLock(mem) as *mut u32;
    if p.is_null() {
        let _ = GlobalFree(Some(mem));
        return false;
    }
    *p = DROPEFFECT_MOVE.0;
    let _ = GlobalUnlock(mem);
    let etc = FORMATETC {
        cfFormat: fmt as u16,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: mem },
        pUnkForRelease: std::mem::ManuallyDrop::new(None),
    };
    if obj.SetData(&etc, &medium, true).is_ok() {
        true
    } else {
        let _ = GlobalFree(Some(mem));
        false
    }
}

/// Filesystem paths in the drop. Empty when the drop is missing, mixed, or not files:
/// the caller then leaves the drop with the shell.
pub(crate) unsafe fn drop_fs_paths(d: &Ref<IDataObject>) -> Vec<String> {
    let paths = hdrop_paths(d);
    let paths = if !paths.is_empty() { paths } else { shell_paths(d) };
    if paths.is_empty() || paths.iter().any(|p| !is_fs(p)) {
        Vec::new()
    } else {
        paths
    }
}

unsafe fn hdrop_paths(d: &Ref<IDataObject>) -> Vec<String> {
    let Some(obj) = d.as_ref() else { return Vec::new() };
    let fmt = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = obj.GetData(&fmt) else { return Vec::new() };
    let drop = HDROP(medium.u.hGlobal.0);
    let mut paths = Vec::new();
    for k in 0..DragQueryFileW(drop, u32::MAX, None) {
        let need = DragQueryFileW(drop, k, None) as usize;
        let mut buf = vec![0u16; need + 1];
        let got = DragQueryFileW(drop, k, Some(&mut buf)) as usize;
        let path = String::from_utf16_lossy(buf.get(..got).unwrap_or(&[]));
        if !path.is_empty() {
            paths.push(path);
        }
    }
    ReleaseStgMedium(&mut medium);
    paths
}

unsafe fn shell_paths(d: &Ref<IDataObject>) -> Vec<String> {
    let Some(arr) = drop_items(d) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0).min(10_000);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { return Vec::new() };
        let Ok(name) = it.GetDisplayName(SIGDN_FILESYSPATH) else { return Vec::new() };
        let path = pwstr(name);
        if path.is_empty() {
            return Vec::new();
        }
        out.push(path);
    }
    out
}

/// Start the file operation and the effect the source should see.
/// Copy and move both go through `start_transfer`, which calls `decide_fast`.
pub(crate) unsafe fn run_asked(obj: &IDataObject, srcs: Vec<String>, dst: String, mv: bool) -> u32 {
    let _ = obj; // deferred move must not stamp Performed DropEffect from Drop
    let started = if mv {
        // MoveItem inside Drop cannot see the item the drag source still holds.
        start_drop_move(srcs, dst)
    } else {
        let rename = paste_mode(&srcs, &dst) == Some(true);
        start_copy(srcs, dst, rename, false)
    };
    if !started {
        return DROPEFFECT_NONE.0;
    }
    // Deferred MoveItem: always COPY back to the source. Reporting MOVE here made
    // the shell delete before settle+MoveItem, so the file disappeared.
    reported_effect(false, false)
}

/// Windows the shell registers a drop target on. OLE delivers the drop to the deepest one,
/// which is the list (or the items-view surface), not the DefView window `GetWindow` returns.
pub(crate) fn shell_drop_class(class: &str) -> bool {
    matches!(class, "SHELLDLL_DefView" | "SysListView32" | "DirectUIHWND" | "SysTreeView32")
}

enum AskWhere {
    Pane(Arc<AtomicUsize>),
    Tree,
}

impl Clone for AskWhere {
    fn clone(&self) -> Self {
        match self {
            AskWhere::Pane(slot) => AskWhere::Pane(slot.clone()),
            AskWhere::Tree => AskWhere::Tree,
        }
    }
}

const DROP_SUB: usize = 8;

unsafe fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = GetClassNameW(hwnd, &mut buf) as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// The drop target OLE currently has on `hwnd` is ours when the mark matches that pointer.
/// The pointer is only compared, never called.
unsafe fn already_ours(hwnd: HWND) -> bool {
    let cur = GetPropW(hwnd, w!("OleDropTargetInterface")).0;
    !cur.is_null() && cur == GetPropW(hwnd, w!("MaguroAskDrop")).0
}

unsafe fn wrap_window(hwnd: HWND, fallback: &IDropTarget, where_: AskWhere) {
    if hwnd.0.is_null() || already_ours(hwnd) {
        return;
    }
    // Drop the shell's registration. Highlight and non-ask drops go to the view's own
    // target, which is `fallback` (the shell view), not whatever pointer the property held.
    let _ = RevokeDragDrop(hwnd);
    let dt: IDropTarget = AskDrop { hwnd, inner: fallback.clone(), where_, srcs: RefCell::new(Vec::new()), basket: Cell::new(false) }.into();
    if RegisterDragDrop(hwnd, &dt).is_err() {
        let _ = RegisterDragDrop(hwnd, fallback);
        return;
    }
    let now = GetPropW(hwnd, w!("OleDropTargetInterface"));
    if !now.0.is_null() {
        let _ = SetPropW(hwnd, w!("MaguroAskDrop"), Some(now));
    }
}

unsafe fn wrap_family(root: HWND, fallback: &IDropTarget, where_: &AskWhere) {
    if root.0.is_null() {
        return;
    }
    let mut wins = vec![root];
    extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        crate::crash::catch_bool(std::panic::AssertUnwindSafe(|| unsafe {
            let want = shell_drop_class(&class_name(h)) || !GetPropW(h, w!("OleDropTargetInterface")).0.is_null();
            if want {
                (*(out.0 as *mut Vec<HWND>)).push(h);
            }
            true.into()
        }))
    }
    let _ = EnumChildWindows(Some(root), Some(collect), LPARAM(&mut wins as *mut Vec<HWND> as isize));
    for hwnd in wins {
        wrap_window(hwnd, fallback, where_.clone());
    }
}

unsafe fn pane_from(mut hwnd: HWND) -> Option<usize> {
    for _ in 0..8 {
        if let Some(i) = pane_of(hwnd) {
            return Some(i);
        }
        hwnd = GetParent(hwnd).unwrap_or_default();
        if hwnd.0.is_null() {
            return None;
        }
    }
    None
}

unsafe extern "system" fn drop_watch_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, a1: usize, a2: usize) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { drop_watch_inner(h, msg, wp, lp, a1, a2) }))
}

/// A new list child means Explorer is about to register its own target on it. Wrap again
/// once that create has returned.
unsafe fn drop_watch_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_PARENTNOTIFY && wp.0 & 0xFFFF == WM_CREATE as usize {
        if let Some(i) = pane_from(h) {
            schedule_pane_drop(i);
        }
    }
    r
}

#[implement(IDropTarget)]
pub(crate) struct AskDrop {
    hwnd: HWND,
    inner: IDropTarget,
    where_: AskWhere,
    /// Filesystem paths seen on DragEnter. Kept so Drop does not have to read the data
    /// object a second time, and so DragOver can allow the drop.
    srcs: RefCell<Vec<String>>,
    /// This drag started in the file basket. Copy by default, on our worker, not the shell's.
    basket: Cell<bool>,
}

impl AskDrop {
    unsafe fn forward_enter(&self, d: &Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) {
        if let Some(obj) = d.as_ref() {
            let _ = self.inner.DragEnter(obj, key, *pt, effect);
        }
    }

    /// The tree's own target often reports no effect for a file. Without one, OLE never calls
    /// Drop, so Always ask would not run. Offer copy or move while the pointer is on a folder.
    unsafe fn keep_tree_drop(&self, pt: &POINTL, effect: *mut DROPEFFECT) {
        if effect.is_null() || !matches!(self.where_, AskWhere::Tree) || self.srcs.borrow().is_empty() {
            return;
        }
        bind_hwnd(self.hwnd);
        if tree_destination(pt).is_none() {
            return;
        }
        if (*effect).0 & (DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0) == 0 {
            *effect = DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0);
        }
    }

    unsafe fn destination(&self, pt: &POINTL) -> Option<String> {
        bind_hwnd(self.hwnd);
        match &self.where_ {
            AskWhere::Pane(slot) => pane_destination(slot),
            AskWhere::Tree => tree_destination(pt),
        }
    }

    /// Enough to show a cursor. The precise folder, including a row under the pointer,
    /// is resolved on Drop so DragOver does not walk accessibility on every move.
    unsafe fn can_land(&self, pt: &POINTL) -> bool {
        match &self.where_ {
            AskWhere::Pane(slot) => {
                let i = slot.load(Ordering::SeqCst);
                app(|a| a.s.paths.get(i).is_some_and(|p| is_fs(p)))
            }
            AskWhere::Tree => tree_destination(pt).is_some(),
        }
    }

    unsafe fn offer_basket(&self, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) {
        if effect.is_null() {
            return;
        }
        let (ctrl, shift, alt) = crate::basket::drag_modifiers(key);
        let want = crate::basket::drag_effect(ctrl, shift, alt);
        let ok = want != 0 && !self.srcs.borrow().is_empty() && self.can_land(pt);
        *effect = if ok { DROPEFFECT(want) } else { DROPEFFECT_NONE };
    }
}

unsafe fn pane_destination(slot: &AtomicUsize) -> Option<String> {
    let i = slot.load(Ordering::SeqCst);
    let cur = app(|a| a.s.paths.get(i).cloned()).unwrap_or_default();
    let child = acc_name_at_cursor().and_then(|name| join_child(&cur, &name)).filter(|p| local_directory(p));
    drop_folder(&cur, child.as_deref())
}

/// Folder under a drop on the left tree. `pt` is the screen point from `IDropTarget`.
/// `HitTest` wants that point in the control's client area; a screen point misses the node,
/// and the drop then ends with no prompt.
unsafe fn tree_destination(pt: &POINTL) -> Option<String> {
    let (tree, hwnd) = app(|a| a.tree.clone().map(|t| (t, a.tree_hwnd)))?;
    if hwnd.0.is_null() {
        return None;
    }
    let mut at = POINT { x: pt.x, y: pt.y };
    if !ScreenToClient(hwnd, &mut at).as_bool() {
        return None;
    }
    let item = tree.HitTest(&at).ok()?;
    shell_fs_path(&item)
}

unsafe fn shell_fs_path(item: &IShellItem) -> Option<String> {
    for kind in [SIGDN_FILESYSPATH, SIGDN_DESKTOPABSOLUTEPARSING] {
        let Ok(name) = item.GetDisplayName(kind) else { continue };
        let path = pwstr(name);
        if is_fs(&path) {
            return Some(path);
        }
    }
    None
}

/// Replace the shell view's drop target so Always ask can run first. The shell target
/// still handles the highlight and every drop that is not Always ask. The wrapper goes on
/// the list (and any other child that already has a target): a wrapper on the parent
/// never sees a drop the child window has registered for.
pub(crate) unsafe fn install_pane_drop(i: usize) {
    let Some(sv) = view_of::<IShellView>(i) else { return };
    let Ok(hwnd) = sv.GetWindow() else { return };
    let Ok(inner) = sv.cast::<IDropTarget>() else { return };
    let Some(slot) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.slot.clone())) else { return };
    let _ = SetWindowSubclass(hwnd, Some(drop_watch_proc), DROP_SUB, 0);
    wrap_family(hwnd, &inner, &AskWhere::Pane(slot));
}

/// Install now, then a few times more. The view often registers its target after
/// `OnViewCreated` returns, or again when the list is rebuilt.
pub(crate) unsafe fn schedule_pane_drop(i: usize) {
    install_pane_drop(i);
    let Some(h) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)).filter(|h| !h.0.is_null()) else { return };
    let _ = SetPropW(h, w!("MaguroDropTries"), Some(HANDLE(4 as *mut core::ffi::c_void)));
    let _ = SetTimer(Some(h), TIMER_DROP, 50, None);
}

pub(crate) unsafe fn on_pane_drop_timer(h: HWND) {
    let left = GetPropW(h, w!("MaguroDropTries")).0 as usize;
    if let Some(i) = pane_of(h) {
        install_pane_drop(i);
    }
    let next = left.saturating_sub(1);
    if next == 0 {
        let _ = KillTimer(Some(h), TIMER_DROP);
        let _ = RemovePropW(h, w!("MaguroDropTries"));
    } else {
        let _ = SetPropW(h, w!("MaguroDropTries"), Some(HANDLE(next as *mut core::ffi::c_void)));
        let _ = SetTimer(Some(h), TIMER_DROP, 200, None);
    }
}

/// `h` is a pane or a window inside one. Re-wrap without stretching the retry timer.
pub(crate) unsafe fn install_drop_at(h: HWND) {
    if let Some(i) = pane_from(h) {
        install_pane_drop(i);
    }
}

/// `h` is a pane or a window inside one. Re-wrap, and keep checking while the view settles.
pub(crate) unsafe fn schedule_drop_at(h: HWND) {
    if let Some(i) = pane_from(h) {
        schedule_pane_drop(i);
    }
}

pub(crate) unsafe fn install_tree_drop(tree: &INameSpaceTreeControl, hwnd: HWND) {
    let Ok(inner) = tree.cast::<IDropTarget>() else { return };
    let _ = SetWindowSubclass(hwnd, Some(tree_drop_watch), DROP_SUB, 0);
    wrap_family(hwnd, &inner, &AskWhere::Tree);
}

unsafe extern "system" fn tree_drop_watch(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, a1: usize, a2: usize) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { tree_drop_watch_inner(h, msg, wp, lp, a1, a2) }))
}

unsafe fn tree_drop_watch_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, _a2: usize) -> LRESULT {
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_PARENTNOTIFY && wp.0 & 0xFFFF == WM_CREATE as usize {
        let _ = h;
        schedule_tree_drop();
    }
    r
}

pub(crate) unsafe fn schedule_tree_drop() {
    let Some((tree, hwnd, main)) = app(|a| Some((a.tree.clone()?, a.tree_hwnd, a.main))) else { return };
    if hwnd.0.is_null() || main.0.is_null() {
        return;
    }
    install_tree_drop(&tree, hwnd);
    let _ = SetPropW(main, w!("MaguroTreeDrop"), Some(HANDLE(4 as *mut core::ffi::c_void)));
    let _ = SetTimer(Some(main), TIMER_DROP, 50, None);
}

pub(crate) unsafe fn on_tree_drop_timer(h: HWND) {
    let left = GetPropW(h, w!("MaguroTreeDrop")).0 as usize;
    if let Some((tree, hwnd)) = app(|a| Some((a.tree.clone()?, a.tree_hwnd))) {
        if !hwnd.0.is_null() {
            install_tree_drop(&tree, hwnd);
        }
    }
    let next = left.saturating_sub(1);
    if next == 0 {
        let _ = KillTimer(Some(h), TIMER_DROP);
        let _ = RemovePropW(h, w!("MaguroTreeDrop"));
    } else {
        let _ = SetPropW(h, w!("MaguroTreeDrop"), Some(HANDLE(next as *mut core::ffi::c_void)));
        let _ = SetTimer(Some(h), TIMER_DROP, 200, None);
    }
}

impl IDropTarget_Impl for AskDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            let basket = d.as_ref().is_some_and(|obj| crate::basket::is_basket_data(obj));
            self.basket.set(basket);
            self.forward_enter(&d, key, pt, effect);
            // Remember the files now. A second read on Drop fails for some objects, and
            // DragOver has to know this is a file drop before it can offer an effect.
            if basket {
                *self.srcs.borrow_mut() = drop_fs_paths(&d);
                self.offer_basket(key, pt, effect);
            } else if matches!(self.where_, AskWhere::Tree) && app(|a| a.s.dropm) == 2 {
                *self.srcs.borrow_mut() = drop_fs_paths(&d);
                self.keep_tree_drop(pt, effect);
            }
        }
        Ok(())
    }
    fn DragOver(&self, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            let _ = self.inner.DragOver(key, *pt, effect);
            if self.basket.get() {
                self.offer_basket(key, pt, effect);
            } else {
                self.keep_tree_drop(pt, effect);
            }
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe {
            self.basket.set(false);
            self.srcs.borrow_mut().clear();
            let _ = self.inner.DragLeave();
        }
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            let basket = self.basket.get();
            self.basket.set(false);
            let stored = std::mem::take(&mut *self.srcs.borrow_mut());
            // A basket drop copies unless Shift is held. The shell target would move on
            // the same drive, and it would do the work on this thread. Leave it out.
            if basket {
                let _ = self.inner.DragLeave();
                let srcs = if stored.is_empty() { drop_fs_paths(&d) } else { stored };
                let (ctrl, shift, alt) = crate::basket::drag_modifiers(key);
                let eff = match self.destination(pt) {
                    Some(dst) => crate::basket::drop_transfer(srcs, dst, ctrl, shift, alt),
                    None => DROPEFFECT_NONE.0,
                };
                if !effect.is_null() {
                    *effect = DROPEFFECT(eff);
                }
                return Ok(());
            }
            // Always ask owns every filesystem drop, same drive included. Falling through
            // to the shell here is what moved a same-drive file with no prompt.
            if app(|a| a.s.dropm) == 2 {
                let srcs = if stored.is_empty() { drop_fs_paths(&d) } else { stored };
                if !srcs.is_empty() {
                    let _ = self.inner.DragLeave();
                    let dst = self.destination(pt);
                    if let (Some(obj), Some(dst)) = (d.as_ref(), dst) {
                        let owner = app(|a| a.main);
                        if let Some(mv) = ask_copy_or_move(owner) {
                            if !effect.is_null() {
                                *effect = DROPEFFECT(run_asked(obj, srcs, dst, mv));
                            }
                        } else if !effect.is_null() {
                            *effect = DROPEFFECT_NONE;
                        }
                    } else if !effect.is_null() {
                        *effect = DROPEFFECT_NONE;
                    }
                    return Ok(());
                }
            }
            if let Some(obj) = d.as_ref() {
                let _ = self.inner.Drop(obj, key, *pt, effect);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{drop_folder, fav_into_folder, reported_effect, shell_drop_class};
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE};

    #[test]
    fn asked_move_is_a_copy_until_the_source_is_told() {
        assert_eq!(reported_effect(false, false), DROPEFFECT_COPY.0);
        assert_eq!(reported_effect(false, true), DROPEFFECT_COPY.0);
        assert_eq!(reported_effect(true, true), DROPEFFECT_MOVE.0);
        assert_eq!(reported_effect(true, false), DROPEFFECT_COPY.0);
    }


    #[test]
    fn deferred_ask_move_reports_copy_so_the_source_keeps_the_file() {
        // Same effect run_asked returns after start_drop_move: never MOVE from Drop.
        assert_eq!(reported_effect(false, false), DROPEFFECT_COPY.0);
    }

    #[test]
    fn pane_drop_lands_in_the_folder_under_the_cursor() {
        assert_eq!(drop_folder(r"C:\pics", Some(r"C:\pics\holiday")), Some(r"C:\pics\holiday".into()));
        assert_eq!(drop_folder(r"C:\pics", None), Some(r"C:\pics".into()));
        assert_eq!(drop_folder("::{thispc}", None), None);
        assert_eq!(drop_folder("::{thispc}", Some(r"D:\work")), Some(r"D:\work".into()));
    }

    #[test]
    fn shell_drop_goes_to_the_list_not_the_header() {
        assert!(shell_drop_class("SysListView32"));
        assert!(shell_drop_class("DirectUIHWND"));
        assert!(shell_drop_class("SHELLDLL_DefView"));
        assert!(shell_drop_class("SysTreeView32"));
        assert!(!shell_drop_class("SysHeader32"));
        assert!(!shell_drop_class("MaguroPane"));
    }

    #[test]
    fn favourite_folder_middle_is_a_copy_target_only_when_asking() {
        assert!(fav_into_folder(2, false, false, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(2, false, false, r"C:\pics", 1, 20));
        assert!(!fav_into_folder(2, false, false, r"C:\pics", 16, 20));
        assert!(!fav_into_folder(0, false, false, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(1, false, false, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(2, true, false, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(2, false, true, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(2, false, false, "::{thispc}", 10, 20));
    }
}
