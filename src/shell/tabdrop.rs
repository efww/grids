// File and folder drags can rest on a window tab or a pane tab. After about half a second
// that tab opens, the way a browser tab does. Releasing on the tab itself copies or moves
// into its folder when the drop is a plain set of files and the target is a real folder.
// Anything else only opens the tab. The effect shown to the source is copy or move; a
// move that `IFileOperation` finishes later is reported as a copy, same as a pane drop,
// so the source does not delete the files first.
//
// Tab reorder stays a mouse capture on this strip. Nothing here calls `DoDragDrop`.

use crate::app::*;
use crate::imports::*;
use crate::pane::recent::path_is_recent;
use crate::pane::*;
use crate::session::*;
use crate::ui::*;

use super::askdrop::{ask_copy_or_move, drop_fs_paths, run_asked};
use super::fileop::send_mode;

const COPY: u32 = 1;
const MOVE: u32 = 2;
const TAB_HOVER_MS: u32 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabFile {
    Copy,
    Move,
    /// Open the tab and leave the files where they are.
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Window(usize),
    Pane(usize, usize),
}

struct Hover {
    kind: Hit,
    at: Instant,
    switched: bool,
    owner: HWND,
}

struct Live {
    files: bool,
    srcs: Vec<String>,
    allowed: u32,
    obj: usize,
    hover: Option<Hover>,
    /// `switch_tab` hides panes. A drag-leave from that hide must not drop the hover.
    switching: bool,
}

impl Live {
    fn new() -> Live {
        Live { files: false, srcs: Vec::new(), allowed: 0, obj: 0, hover: None, switching: false }
    }
}

thread_local! {
    static LIVE: RefCell<Live> = RefCell::new(Live::new());
}

struct Switching;

impl Switching {
    fn arm() -> Switching {
        LIVE.with(|l| l.borrow_mut().switching = true);
        Switching
    }
}

impl Drop for Switching {
    fn drop(&mut self) {
        LIVE.with(|l| l.borrow_mut().switching = false);
    }
}

fn fs_path(path: &str) -> bool {
    is_fs(&path.replace('/', "\\"))
}

fn norm_path(path: &str) -> String {
    path.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// `src` is `dst`, or `dst` is inside `src`. `C:\a` does not contain `C:\ab`.
fn nests(src: &str, dst: &str) -> bool {
    let src = norm_path(src);
    let dst = norm_path(dst);
    if src.is_empty() || dst.is_empty() {
        return false;
    }
    src == dst || dst.starts_with(&format!("{src}\\"))
}

fn volume_key(path: &str) -> Option<String> {
    let mut s = norm_path(path);
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_prefix(r"unc\") {
        return unc_key(rest);
    }
    if let Some(rest) = s.strip_prefix(r"\\") {
        return unc_key(rest);
    }
    let b = s.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':').then(|| s[..2].to_string())
}

fn unc_key(rest: &str) -> Option<String> {
    let mut parts = rest.split('\\');
    let server = parts.next().filter(|s| !s.is_empty())?;
    let share = parts.next().filter(|s| !s.is_empty())?;
    Some(format!(r"\\{server}\{share}"))
}

fn same_volume(srcs: &[String], dst: &str) -> bool {
    let Some(key) = volume_key(dst) else { return false };
    srcs.iter().all(|s| volume_key(s).as_deref() == Some(key.as_str()))
}

fn plain(srcs: &[String], dst: &str) -> bool {
    !srcs.is_empty() && fs_path(dst) && srcs.iter().all(|s| fs_path(s)) && srcs.iter().all(|s| !nests(s, dst))
}

fn act_ok(srcs: &[String], dst: &str, mv: bool) -> bool {
    send_mode(srcs, dst, mv).is_some()
}

/// The ask dialog's choice is legal for this folder. The Recent folder is not a write target.
fn tab_file_can(srcs: &[String], dst: &str, mv: bool) -> bool {
    plain(srcs, dst) && act_ok(srcs, dst, mv)
}

/// What a release on the tab should do, and the cursor effect while the pointer is there.
/// `blocked` is an empty target or the Recent folder. Ctrl+Shift is a link in Explorer;
/// this strip does not create shortcuts, so the tab only opens.
fn tab_file_drop(srcs: &[String], dst: &str, ctrl: bool, shift: bool, allowed: u32, blocked: bool) -> (TabFile, u32) {
    if blocked || !plain(srcs, dst) || (ctrl && shift) {
        return (TabFile::Open, 0);
    }
    let want_move = if ctrl { false } else { shift || same_volume(srcs, dst) };
    if !act_ok(srcs, dst, want_move) {
        return (TabFile::Open, 0);
    }
    let bit = if want_move { MOVE } else { COPY };
    if allowed & bit != 0 {
        return (if want_move { TabFile::Move } else { TabFile::Copy }, bit);
    }
    // A held Ctrl or Shift is the user's choice. Do not quietly swap it for the other one.
    if ctrl || shift {
        return (TabFile::Open, 0);
    }
    let other = !want_move;
    let other_bit = if other { MOVE } else { COPY };
    if allowed & other_bit != 0 && act_ok(srcs, dst, other) {
        return (if other { TabFile::Move } else { TabFile::Copy }, other_bit);
    }
    (TabFile::Open, 0)
}

fn obj_ptr(d: &Ref<IDataObject>) -> usize {
    d.as_ref().map(|o| o.as_raw() as usize).unwrap_or(0)
}

fn read_effect(effect: *mut DROPEFFECT) -> u32 {
    if effect.is_null() { 0 } else { unsafe { (*effect).0 } }
}

fn set_effect(effect: *mut DROPEFFECT, bits: u32) {
    if !effect.is_null() {
        unsafe { *effect = DROPEFFECT(bits) }
    }
}

/// Keep the widest mask the source offered. A later pass often repeats the single
/// effect we wrote back, and that must not erase copy|move|link.
fn note_allowed(incoming: u32) {
    if incoming == 0 {
        return;
    }
    LIVE.with(|l| {
        let mut g = l.borrow_mut();
        if g.allowed == 0 || incoming.count_ones() >= g.allowed.count_ones() {
            g.allowed = incoming;
        }
    });
}

fn remember_enter(d: &Ref<IDataObject>) {
    let ptr = obj_ptr(d);
    let skip = LIVE.with(|l| {
        let g = l.borrow();
        ptr != 0 && g.obj == ptr && g.files
    });
    if skip {
        return;
    }
    let srcs = unsafe { drop_fs_paths(d) };
    let virtual_only = srcs.is_empty() && unsafe { drop_items(d).is_some() };
    LIVE.with(|l| {
        let mut g = l.borrow_mut();
        if g.obj != ptr {
            g.allowed = 0;
        }
        g.obj = ptr;
        if !srcs.is_empty() {
            g.srcs = srcs;
            g.files = true;
        } else if virtual_only {
            g.srcs.clear();
            g.files = true;
        } else {
            g.srcs.clear();
            g.files = false;
        }
    });
}

fn refresh_srcs(d: &Ref<IDataObject>) {
    let empty = LIVE.with(|l| l.borrow().srcs.is_empty());
    if !empty {
        return;
    }
    let srcs = unsafe { drop_fs_paths(d) };
    if srcs.is_empty() {
        return;
    }
    LIVE.with(|l| {
        let mut g = l.borrow_mut();
        g.srcs = srcs;
        g.files = true;
    });
}

fn files() -> bool {
    LIVE.with(|l| l.borrow().files)
}

fn mods(key: MODIFIERKEYS_FLAGS) -> (bool, bool) {
    (key.0 & MK_CONTROL.0 != 0, key.0 & MK_SHIFT.0 != 0)
}

fn root_of(hwnd: HWND) -> HWND {
    let parent = unsafe { GetParent(hwnd).unwrap_or_default() };
    if parent.0.is_null() { hwnd } else { parent }
}

fn client_pt(hwnd: HWND, pt: &POINTL) -> Option<(i32, i32)> {
    let mut at = POINT { x: pt.x, y: pt.y };
    unsafe { ScreenToClient(hwnd, &mut at).as_bool() }.then_some((at.x, at.y))
}

fn already(hit: &Hit) -> bool {
    app(|a| match *hit {
        Hit::Window(i) => i == a.s.tab,
        Hit::Pane(i, k) => {
            let list = a.s.pane_tab_list(i);
            let cur = if list.len() > 1 { a.s.pane_cur.get(i).copied().unwrap_or(0).min(list.len() - 1) } else { 0 };
            list.len() > 1 && k == cur
        }
    })
}

fn folder_of(hit: Hit) -> String {
    app(|a| match hit {
        Hit::Window(i) => {
            if i == a.s.tab {
                let n = a.s.panes().max(1).min(MAXP);
                let active = a.s.active.min(n - 1);
                return a.s.paths.get(active).cloned().unwrap_or_default();
            }
            let Some(t) = a.s.tabs.get(i) else { return String::new() };
            let n = t.heights.iter().sum::<usize>().max(1).min(MAXP);
            let active = t.active.min(n.saturating_sub(1));
            t.paths.get(active).cloned().unwrap_or_default()
        }
        Hit::Pane(i, k) => a.s.pane_tab_list(i).get(k).cloned().unwrap_or_default(),
    })
}

fn retint(prev: Option<Hit>, next: Option<Hit>) {
    let tabs = matches!(prev, Some(Hit::Window(_))) || matches!(next, Some(Hit::Window(_)));
    let mut panes = Vec::new();
    for hit in [prev, next].into_iter().flatten() {
        if let Hit::Pane(i, _) = hit {
            if !panes.contains(&i) {
                panes.push(i);
            }
        }
    }
    if tabs {
        tabs_invalidate();
    }
    for i in panes {
        unsafe { repaint_pane_tabs(i) };
    }
}

fn arm_hover(hwnd: HWND, hit: Hit) {
    let owner = root_of(hwnd);
    let now = already(&hit);
    let (changed, prev_owner, prev) = LIVE.with(|l| {
        let mut g = l.borrow_mut();
        if g.hover.as_ref().is_some_and(|h| h.kind == hit) {
            return (false, HWND::default(), None);
        }
        let prev = g.hover.take();
        let prev_owner = prev.as_ref().map(|h| h.owner).unwrap_or_default();
        let prev_kind = prev.map(|h| h.kind);
        g.hover = Some(Hover { kind: hit, at: Instant::now(), switched: now, owner });
        (true, prev_owner, prev_kind)
    });
    if !changed {
        return;
    }
    unsafe {
        if !prev_owner.0.is_null() {
            let _ = KillTimer(Some(prev_owner), TIMER_TAB_HOVER);
        }
        let _ = KillTimer(Some(owner), TIMER_TAB_HOVER);
        if !now {
            let _ = SetTimer(Some(owner), TIMER_TAB_HOVER, TAB_HOVER_MS, None);
        }
    }
    retint(prev, Some(hit));
}

fn clear_hover() {
    if LIVE.with(|l| l.borrow().switching) {
        return;
    }
    let prev = LIVE.with(|l| l.borrow_mut().hover.take());
    if let Some(h) = prev {
        unsafe { let _ = KillTimer(Some(h.owner), TIMER_TAB_HOVER); }
        retint(Some(h.kind), None);
    }
}

fn wipe() {
    let prev = LIVE.with(|l| {
        let mut g = l.borrow_mut();
        if g.switching {
            return None;
        }
        g.files = false;
        g.srcs.clear();
        g.allowed = 0;
        g.obj = 0;
        g.hover.take()
    });
    if let Some(h) = prev {
        unsafe { let _ = KillTimer(Some(h.owner), TIMER_TAB_HOVER); }
        retint(Some(h.kind), None);
    }
}

fn activate(hit: Hit) {
    if app(|a| a.tab_drag.is_some() || a.pane_drag.is_some()) {
        return;
    }
    if already(&hit) {
        return;
    }
    let _hold = Switching::arm();
    match hit {
        Hit::Window(i) => {
            let (n, cur) = app(|a| (a.s.tabs.len(), a.s.tab));
            if i < n && i != cur {
                unsafe { switch_tab(i) };
            }
        }
        Hit::Pane(i, k) => unsafe { pick_pane_tab(i, k) },
    }
}

enum Tick {
    Ignore,
    Stop,
    Rearm(u32),
    Fire(Hit),
}

/// The hover timer lives on the main window. `DragOver` does not repeat while the pointer rests.
pub(crate) unsafe fn on_tab_hover_timer(main: HWND) {
    let tick = LIVE.with(|l| {
        let mut g = l.borrow_mut();
        let Some(h) = g.hover.as_ref() else { return Tick::Stop };
        if h.owner != main {
            return Tick::Ignore;
        }
        if h.switched {
            return Tick::Stop;
        }
        let elapsed = h.at.elapsed().as_millis() as u32;
        if elapsed < TAB_HOVER_MS {
            return Tick::Rearm(TAB_HOVER_MS - elapsed);
        }
        let hit = h.kind;
        if let Some(h) = g.hover.as_mut() {
            h.switched = true;
        }
        Tick::Fire(hit)
    });
    match tick {
        Tick::Ignore => {}
        Tick::Stop => {
            let _ = KillTimer(Some(main), TIMER_TAB_HOVER);
        }
        Tick::Rearm(ms) => {
            let _ = SetTimer(Some(main), TIMER_TAB_HOVER, ms.max(1), None);
        }
        Tick::Fire(hit) => {
            let _ = KillTimer(Some(main), TIMER_TAB_HOVER);
            activate(hit);
        }
    }
}

fn pane_hit(hwnd: HWND, pt: &POINTL) -> Option<Hit> {
    let i = pane_of(hwnd)?;
    let (x, y) = client_pt(hwnd, pt)?;
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut rc); }
    let (dpi, n) = app(|a| (a.dpi, a.s.pane_tab_list(i).len()));
    if n < 2 {
        return None;
    }
    let pg = pane_geo_of(hwnd, rc.right, rc.bottom, dpi);
    match pane_tab_hit(&pg, n, x, y, dpi)? {
        PaneTabHit::Tab(k) | PaneTabHit::Close(k) => Some(Hit::Pane(i, k)),
        PaneTabHit::Plus | PaneTabHit::Strip => None,
    }
}

fn strip_hit(hwnd: HWND, pt: &POINTL) -> Option<Hit> {
    let (x, y) = client_pt(hwnd, pt)?;
    match tab_hit(&current_tab_lay(), x, y) {
        TabHit::Tab(i) | TabHit::Close(i) | TabHit::Lock(i) => Some(Hit::Window(i)),
        _ => None,
    }
}

fn button_up() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) >= 0 && GetAsyncKeyState(VK_RBUTTON.0 as i32) >= 0 }
}

/// `DragOver` that returns no effect never gets `Drop`. A release still arrives as `DragLeave`.
fn release_hit(hwnd: HWND, strip: bool) -> Option<Hit> {
    if !button_up() || !files() {
        return None;
    }
    let mut pt = POINT::default();
    if unsafe { GetCursorPos(&mut pt).is_err() } {
        return None;
    }
    let at = POINTL { x: pt.x, y: pt.y };
    if strip { strip_hit(hwnd, &at) } else { pane_hit(hwnd, &at) }
}

fn track(hwnd: HWND, hit: Hit, key: MODIFIERKEYS_FLAGS, effect: *mut DROPEFFECT) {
    note_allowed(read_effect(effect));
    let (ctrl, shift) = mods(key);
    let srcs = LIVE.with(|l| l.borrow().srcs.clone());
    let allowed = LIVE.with(|l| l.borrow().allowed);
    let dst = folder_of(hit);
    let blocked = dst.is_empty() || (fs_path(&dst) && path_is_recent(&dst));
    let (_act, bits) = tab_file_drop(&srcs, &dst, ctrl, shift, allowed, blocked);
    set_effect(effect, bits);
    arm_hover(hwnd, hit);
}

fn perform(d: &Ref<IDataObject>, srcs: Vec<String>, dst: String, mut mv: bool) -> u32 {
    if app(|a| a.s.dropm) == 2 {
        let owner = app(|a| a.main);
        let Some(choice) = (unsafe { ask_copy_or_move(owner) }) else { return 0 };
        if !tab_file_can(&srcs, &dst, choice) {
            return 0;
        }
        mv = choice;
    }
    let Some(obj) = d.as_ref() else { return 0 };
    unsafe { run_asked(obj, srcs, dst, mv) }
}

fn commit(d: &Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, effect: *mut DROPEFFECT, hit: Hit) {
    note_allowed(read_effect(effect));
    refresh_srcs(d);
    let (ctrl, shift) = mods(key);
    let srcs = LIVE.with(|l| l.borrow().srcs.clone());
    let allowed = LIVE.with(|l| l.borrow().allowed);
    let dst = folder_of(hit);
    let blocked = dst.is_empty() || (fs_path(&dst) && path_is_recent(&dst));
    let act = tab_file_drop(&srcs, &dst, ctrl, shift, allowed, blocked).0;
    clear_hover();
    activate(hit);
    let reported = match act {
        TabFile::Open => 0,
        TabFile::Copy => perform(d, srcs, dst, false),
        TabFile::Move => perform(d, srcs, dst, true),
    };
    set_effect(effect, reported);
    wipe();
}

pub(crate) unsafe fn pane_tab_drag_enter(hwnd: HWND, d: &Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> bool {
    bind_hwnd(hwnd);
    remember_enter(d);
    pane_tab_drag_over(hwnd, key, pt, effect)
}

pub(crate) unsafe fn pane_tab_drag_over(hwnd: HWND, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> bool {
    bind_hwnd(hwnd);
    if !files() {
        return false;
    }
    let Some(hit) = pane_hit(hwnd, pt) else {
        clear_hover();
        return false;
    };
    track(hwnd, hit, key, effect);
    true
}

pub(crate) unsafe fn pane_tab_drag_leave(hwnd: HWND) {
    bind_hwnd(hwnd);
    if LIVE.with(|l| l.borrow().switching) {
        return;
    }
    if let Some(hit) = release_hit(hwnd, false) {
        activate(hit);
        wipe();
    } else {
        clear_hover();
    }
}

pub(crate) unsafe fn pane_tab_drop(hwnd: HWND, d: &Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> bool {
    bind_hwnd(hwnd);
    if !files() {
        return false;
    }
    let Some(hit) = pane_hit(hwnd, pt) else {
        clear_hover();
        wipe();
        return false;
    };
    commit(d, key, effect, hit);
    true
}

/// Window tab under a file drag, for the warm fill. Mouse hover on the strip is left alone.
/// A second window does not light up: the hover belongs to one tab strip.
pub(crate) fn file_window_hot(tabs: HWND) -> Option<usize> {
    let root = root_of(tabs);
    LIVE.with(|l| match l.borrow().hover.as_ref() {
        Some(h) if h.owner == root => match h.kind {
            Hit::Window(i) => Some(i),
            _ => None,
        },
        _ => None,
    })
}

/// Pane tab under a file drag. `None` while the pointer is not on this pane's strip.
pub(crate) fn pane_file_hot(pane: usize, hwnd: HWND) -> Option<usize> {
    let root = root_of(hwnd);
    LIVE.with(|l| match l.borrow().hover.as_ref() {
        Some(h) if h.owner == root => match h.kind {
            Hit::Pane(i, k) if i == pane => Some(k),
            _ => None,
        },
        _ => None,
    })
}

#[implement(IDropTarget)]
struct StripDrop {
    hwnd: HWND,
    helper: Option<IDropTargetHelper>,
}

fn over_strip(hwnd: HWND, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) {
    if !files() {
        set_effect(effect, 0);
        return;
    }
    let Some(hit) = strip_hit(hwnd, pt) else {
        clear_hover();
        set_effect(effect, 0);
        return;
    };
    track(hwnd, hit, key, effect);
}

impl StripDrop {
    unsafe fn helper_enter(&self, d: &Ref<IDataObject>, pt: &POINTL, effect: *mut DROPEFFECT) {
        if effect.is_null() {
            return;
        }
        if let Some(hp) = &self.helper {
            let _ = hp.DragEnter(self.hwnd, d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
        }
    }
    unsafe fn helper_over(&self, pt: &POINTL, effect: *mut DROPEFFECT) {
        if effect.is_null() {
            return;
        }
        if let Some(hp) = &self.helper {
            let _ = hp.DragOver(&POINT { x: pt.x, y: pt.y }, *effect);
        }
    }
}

/// The tab strip accepts file drags. Reorder still starts from the left button, not from here.
pub(crate) unsafe fn register_tab_drop(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }
    let helper = CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok();
    let dt: IDropTarget = StripDrop { hwnd, helper }.into();
    let _ = RegisterDragDrop(hwnd, &dt);
}

pub(crate) unsafe fn revoke_tab_drop(hwnd: HWND) {
    wipe();
    if !hwnd.0.is_null() {
        let _ = RevokeDragDrop(hwnd);
    }
}

impl IDropTarget_Impl for StripDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            remember_enter(&d);
            note_allowed(read_effect(effect));
            over_strip(self.hwnd, key, pt, effect);
            self.helper_enter(&d, pt, effect);
        }
        Ok(())
    }
    fn DragOver(&self, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            over_strip(self.hwnd, key, pt, effect);
            self.helper_over(pt, effect);
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            if !LIVE.with(|l| l.borrow().switching) {
                if let Some(hit) = release_hit(self.hwnd, true) {
                    activate(hit);
                    wipe();
                } else {
                    clear_hover();
                }
            }
            if let Some(hp) = &self.helper {
                let _ = hp.DragLeave();
            }
        }
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            remember_enter(&d);
            if !files() {
                set_effect(effect, 0);
                wipe();
            } else if let Some(hit) = strip_hit(self.hwnd, pt) {
                commit(&d, key, effect, hit);
            } else {
                set_effect(effect, 0);
                clear_hover();
                wipe();
            }
            if let Some(hp) = &self.helper {
                if !effect.is_null() {
                    let _ = hp.Drop(d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn go(srcs: &[&str], dst: &str, ctrl: bool, shift: bool, allowed: u32) -> (TabFile, u32) {
        let srcs: Vec<String> = srcs.iter().map(|s| (*s).to_string()).collect();
        tab_file_drop(&srcs, dst, ctrl, shift, allowed, false)
    }

    #[test]
    fn ole_bits_match_the_shell() {
        assert_eq!(COPY, DROPEFFECT_COPY.0);
        assert_eq!(MOVE, DROPEFFECT_MOVE.0);
    }

    #[test]
    fn volume_is_drive_or_unc_share() {
        assert_eq!(volume_key(r"C:\a"), volume_key(r"c:\b"));
        assert_ne!(volume_key(r"C:\a"), volume_key(r"D:\b"));
        assert_eq!(volume_key(r"\\Server\Share\a"), volume_key(r"\\server\share\b"));
        assert_ne!(volume_key(r"\\server\share\a"), volume_key(r"\\server\other\b"));
        assert_eq!(volume_key(r"\\?\C:\a"), volume_key(r"C:\b"));
        assert_eq!(volume_key(r"\\?\UNC\Server\Share\a"), volume_key(r"\\server\share\b"));
        assert!(same_volume(&[r"C:\a\f.txt".into()], r"C:\b"));
        assert!(!same_volume(&[r"C:\a\f.txt".into()], r"D:\b"));
    }

    #[test]
    fn a_folder_contains_its_children_only() {
        assert!(nests(r"C:\pics", r"C:\pics"));
        assert!(nests(r"C:\pics", r"C:\pics\holiday"));
        assert!(nests(r"C:\pics\", r"c:/pics/holiday"));
        assert!(!nests(r"C:\a", r"C:\ab"));
        assert!(!nests(r"C:\pics\a.txt", r"C:\pics"));
        assert!(nests(r"C:\", r"C:\Windows"));
    }

    #[test]
    fn same_drive_moves_other_drive_copies() {
        assert_eq!(go(&[r"C:\a\f.txt"], r"C:\b", false, false, 7), (TabFile::Move, MOVE));
        assert_eq!(go(&[r"C:\a\f.txt"], r"D:\b", false, false, 7), (TabFile::Copy, COPY));
        assert_eq!(go(&[r"C:\a\f.txt"], r"C:\b", true, false, 7), (TabFile::Copy, COPY));
        assert_eq!(go(&[r"C:\a\f.txt"], r"D:\b", false, true, 7), (TabFile::Move, MOVE));
    }

    #[test]
    fn link_chord_and_illegal_targets_only_open() {
        assert_eq!(go(&[r"C:\a\f.txt"], r"D:\b", true, true, 7), (TabFile::Open, 0));
        assert_eq!(go(&[r"C:\a\f.txt"], "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", false, false, 7), (TabFile::Open, 0));
        assert_eq!(go(&[r"C:\pics"], r"C:\pics\holiday", false, false, 7), (TabFile::Open, 0));
        assert_eq!(go(&[r"C:\pics\a.txt"], r"C:\pics", false, false, 7), (TabFile::Open, 0));
        assert_eq!(go(&[r"C:\pics\a.txt"], r"C:\pics", false, true, 7), (TabFile::Open, 0));
        assert_eq!(go(&[r"C:\pics\a.txt"], r"C:\pics", true, false, 7), (TabFile::Copy, COPY));
        assert_eq!(go(&[r"C:\a\f.txt", r"C:\b\g.txt"], r"C:\a", false, true, 7), (TabFile::Open, 0));
        assert_eq!(go(&["::{not-a-file}"], r"C:\b", false, false, 7), (TabFile::Open, 0));
        assert_eq!(go(&[], r"C:\b", false, false, 7), (TabFile::Open, 0));
        let srcs = vec![r"C:\a\f.txt".to_string()];
        assert_eq!(tab_file_drop(&srcs, r"C:\b", false, false, 7, true), (TabFile::Open, 0));
    }

    #[test]
    fn the_source_mask_can_force_the_other_effect() {
        assert_eq!(go(&[r"C:\a\f.txt"], r"C:\b", false, false, COPY), (TabFile::Copy, COPY));
        assert_eq!(go(&[r"C:\a\f.txt"], r"D:\b", false, true, COPY), (TabFile::Open, 0));
        assert_eq!(go(&[r"C:\a\f.txt"], r"C:\b", false, false, 0), (TabFile::Open, 0));
    }

    #[test]
    fn hover_waits_half_a_second() {
        assert_eq!(TAB_HOVER_MS, 500);
    }
}
