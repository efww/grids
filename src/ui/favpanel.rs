// Favourites, one half of the left panel. One owner-drawn window: groups, files, and
// folders from the same tree the Ctrl+B list uses, plus two virtual Explorer groups on top.
// Pinned folders come first. Favourite files sit directly under them, and only when Explorer has any.

use crate::imports::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;
use crate::shell::*;
use super::*;
use windows::Win32::UI::Controls::{TTF_ABSOLUTE, TTF_TRACK, TTM_POP, TTM_TRACKACTIVATE, TTM_DELTOOLW};
use windows::Win32::UI::Input::Ime::{ImmAssociateContextEx, ImmGetVirtualKey, GCS_COMPSTR, IACE_DEFAULT, HIMC};

const ID_SIDE_EDIT: i32 = 209;
/// Subclass id for the favourites rename edit. Per window, so it only has to be unique here.
const SIDE_EDIT_SUB: usize = 9;
/// Tracking tool for a row in the favourites panel. Lock is 7, tab paths 8, address 9.
const FAV_TIP_ID: usize = 10;
/// Tracking tool for a row in the favourites window.
const FAV_POP_TIP_ID: usize = 11;

/// Favourites row height in px. `tree` is the folder tree's `TVM_GETITEMHEIGHT`.
/// When that control is not up yet, `font_px` (the UI font's line height, already scaled
/// for DPI) plus the padding a tree view adds.
pub(crate) fn fav_row_height(tree: i32, font_px: i32) -> i32 {
    if tree > 0 {
        tree
    } else {
        (font_px + 4).max(1)
    }
}

unsafe fn sys_tree() -> HWND {
    let host = app(|a| a.tree_hwnd);
    if host.0.is_null() {
        HWND::default()
    } else {
        FindWindowExW(Some(host), None, w!("SysTreeView32"), PCWSTR::null()).unwrap_or_default()
    }
}

/// The folder tree's current item height, or 0 when the control is not created yet.
unsafe fn tree_item_height() -> i32 {
    let tv = sys_tree();
    if tv.0.is_null() {
        return 0;
    }
    let h = SendMessageW(tv, TVM_GETITEMHEIGHT, None, None).0 as i32;
    if h > 0 { h } else { 0 }
}

fn row_h() -> i32 {
    unsafe {
        let tree = tree_item_height();
        let (fh, dpi) = app(|a| (a.fh, a.dpi));
        fav_row_height(tree, if fh > 0 { fh } else { 16 * dpi.max(1) / 96 })
    }
}

/// Give the folder tree the UI font (so its item height follows DPI)
/// and keep the favourites scroll inside the new row size.
pub(crate) unsafe fn sync_fav_metrics() {
    let (font, fh, dpi) = app(|a| (a.font, a.fh, a.dpi));
    let tv = sys_tree();
    if !tv.0.is_null() && !font.is_invalid() {
        SendMessageW(tv, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    }
    _ = (fh, dpi);
    clamp_side_scroll();
}

fn rows() -> Vec<FavRow> {
    app(|a| {
        a.s.ensure_fav_tree();
        let pins = if a.s.pins_open { pin_list() } else { Vec::new() };
        let files = pin_files();
        let pins_label = pins_section_label();
        let files_label = files_section_label();
        compose_side(&a.s.tree, a.s.pins_open, &pins, pins_label, a.s.files_open, &files, files_label)
    })
}

fn side_view_h() -> i32 {
    let h = app(|a| a.side_hwnd);
    let mut rc = RECT::default();
    if !h.0.is_null() {
        unsafe { _ = GetClientRect(h, &mut rc) };
    }
    rc.bottom
}

/// Height of the removal error, or 0 when there is nothing to say.
fn note_band() -> i32 {
    if pin_file_note().is_empty() { 0 } else { row_h().saturating_mul(3) }
}

/// The list's visible height. The error band sits under it and does not scroll away.
fn list_page() -> i32 {
    (side_view_h() - note_band()).max(0)
}

unsafe fn paint_pin_note(dc: HDC, rc: RECT, font: HFONT, dpi: i32) {
    let note = pin_file_note();
    if note.is_empty() {
        return;
    }
    let s = |v: i32| v * dpi / 96;
    let top = (rc.bottom - note_band()).max(rc.top);
    fill(dc, RECT { left: 0, top, right: rc.right, bottom: rc.bottom }, side_ground());
    let ink = if DARK_ON.load(Ordering::Relaxed) { 0xFFB4AB } else { 0x9F1C14 };
    fill(dc, RECT { left: 0, top, right: rc.right, bottom: top + s(2).max(1) }, ink);
    wrap_text(dc, RECT { left: s(8), top: top + s(4), right: rc.right - s(8), bottom: rc.bottom - s(2) }, &note, font, ink, false);
}

/// Largest vertical scroll offset: total row height minus what the panel shows.
fn max_side_scroll() -> i32 {
    let total = rows().len() as i32 * row_h();
    (total - list_page()).max(0)
}

/// Mirror the scroll state into the native vertical scrollbar (hidden while everything fits).
pub(crate) fn sync_side_scrollbar() {
    let h = app(|a| a.side_hwnd);
    if h.0.is_null() {
        return;
    }
    let total = rows().len() as i32 * row_h();
    let (page, pos) = (list_page(), app(|a| a.side_scroll));
    let si = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (total - 1).max(0),
        nPage: page.max(0) as u32,
        nPos: pos,
        nTrackPos: 0,
    };
    unsafe { windows::Win32::UI::Controls::SetScrollInfo(h, SB_VERT, &si, true) };
}

pub(crate) fn clamp_side_scroll() {
    let max = max_side_scroll();
    let changed = app(|a| {
        let next = a.side_scroll.min(max);
        let changed = next != a.side_scroll;
        a.side_scroll = next;
        changed
    });
    sync_side_scrollbar();
    if changed {
        unsafe { dismiss_side_fav_tip() };
    }
}

fn mix_str(h: u64, s: &str) -> u64 {
    let mut h = h.wrapping_mul(31).wrapping_add(s.chars().count() as u64);
    for c in s.chars() {
        h = h.wrapping_mul(31).wrapping_add(c as u64);
    }
    h
}

/// Changes when a row is added, removed, renamed, or its path changes. Character-wise, so a
/// non-ASCII path is part of the value and is never cut.
fn fav_rows_mark() -> u64 {
    mark_rows(&rows())
}

fn mark_rows(list: &[FavRow]) -> u64 {
    let mut h = 0x9E37_79B9u64.wrapping_add(list.len() as u64);
    for r in list {
        h = mix_str(h, &r.path);
        h = mix_str(h, &r.name);
        h = h.wrapping_mul(31).wrapping_add(r.id.len() as u64);
        for id in &r.id {
            h = h.wrapping_mul(31).wrapping_add(*id as u64);
        }
        h = h.wrapping_mul(31).wrapping_add(r.depth as u64);
        h = h.wrapping_mul(31).wrapping_add(u64::from(r.group));
        h = h.wrapping_mul(31).wrapping_add(u64::from(r.open));
        h = h.wrapping_mul(31).wrapping_add(u64::from(r.file));
        h = h.wrapping_mul(31).wrapping_add(match r.kind {
            RowKind::Tree => 1,
            RowKind::Pins => 2,
            RowKind::Files => 3,
        });
    }
    h
}

fn drag_of(r: &FavRow) -> SideDrag {
    SideDrag { id: r.id.clone(), kind: r.kind, group: r.group, file: r.file, path: r.path.clone(), name: r.name.clone() }
}

fn tree_pos(list: &[FavRow], id: &[usize]) -> Option<usize> {
    list.iter().position(|r| r.in_tree() && r.id == id)
}

/// What the favourites location tip should do for this pointer.
/// `inside` is the panel client area. `row` is the hit-test; `None` is no item.
/// An index past `rows` is stale after the tree changed and hides the tip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FavTipAct {
    Hide,
    Show(usize),
}

pub(crate) fn fav_tip_act(inside: bool, row: Option<usize>, rows: usize) -> FavTipAct {
    if !inside {
        return FavTipAct::Hide;
    }
    match row {
        Some(i) if i < rows => FavTipAct::Show(i),
        _ => FavTipAct::Hide,
    }
}

/// The location tip stays down while a rename edit is open, an IME composition is up,
/// or the pointer has not left the panel since an outside drop.
pub(crate) fn fav_tip_blocked(renaming: bool, composing: bool, after_drop: bool) -> bool {
    renaming || composing || after_drop
}

/// The tracking tip already shows this tool's text at this screen point.
/// Another `TTM_UPDATETIPTEXTW` / `TTM_TRACKPOSITION` / `TTM_TRACKACTIVATE` would only re-enter it.
/// An empty `text` is a hide, and a missing point is not "unchanged".
/// Same favourites tip tool and text (item-anchored; position is fixed on enter).
pub(crate) fn fav_tip_same(live_id: Option<usize>, live_text: &str, id: usize, text: &str) -> bool {
    !text.is_empty() && live_id == Some(id) && live_text == text
}

/// A visible tip hides when the pointer is no longer on the panel.
pub(crate) fn fav_tip_hides(shown: bool, on_panel: bool) -> bool {
    shown && !on_panel
}

/// Manhattan pixels before a press becomes a reorder. A shorter move is still a click.
const FAV_DRAG_SLOP: i32 = 4;

/// A left-button release that did not become a reorder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FavRelease {
    /// The pointer left the pressed row, or moved far enough to drag.
    None,
    /// Folder group, Explorer pins, or Explorer files header. Expand or collapse only.
    Toggle,
    /// Path favourite. Open it in the active pane.
    Open,
}

fn fav_drag_moved(dx: i32, dy: i32) -> bool {
    dx.abs().saturating_add(dy.abs()) > FAV_DRAG_SLOP
}

/// What a left-button release does when a reorder drag has not started.
/// A group header only toggles. A leaf opens. Past the drag slop, or a release
/// that left the pressed row, does neither: that press is a drag or a miss.
pub(crate) fn fav_release(group: bool, dx: i32, dy: i32, on_pressed_row: bool) -> FavRelease {
    if fav_drag_moved(dx, dy) || !on_pressed_row {
        FavRelease::None
    } else if group {
        FavRelease::Toggle
    } else {
        FavRelease::Open
    }
}

/// Selection after a click. Opening a favourite navigates the pane and clears the highlight.
/// A group toggle and a drag keep the row that was pressed.
pub(crate) fn fav_sel_after_release(release: FavRelease, sel: Option<usize>) -> Option<usize> {
    match release {
        FavRelease::Open => None,
        FavRelease::Toggle | FavRelease::None => sel,
    }
}

/// Hover and timer repaints of the favourites panel.
/// The panel blits its whole client over the rename edit. While that edit is open, or an IME
/// composition is up, a hover wash or a pin timer covers the composition and the candidate window.
/// `hot_changed` is false when the pointer stayed on the same row, so there is nothing to paint.
pub(crate) fn fav_hover_paints(hot_changed: bool, edit_open: bool, composing: bool) -> bool {
    hot_changed && !edit_open && !composing
}

/// What a key does to the favourites rename edit.
/// `process` is `VK_PROCESSKEY`: the IME owns that message, so it is never a commit or a cancel.
/// `composing` is an open reading or syllable: Enter and Esc stay with the IME.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SideRenameKey {
    /// The edit or the IME keeps the key.
    Pass,
    Commit,
    Cancel,
}

pub(crate) fn side_rename_key(vk: u16, composing: bool, process: bool) -> SideRenameKey {
    if process || composing || vk == VK_PROCESSKEY.0 {
        return SideRenameKey::Pass;
    }
    match VIRTUAL_KEY(vk) {
        VK_RETURN => SideRenameKey::Commit,
        VK_ESCAPE => SideRenameKey::Cancel,
        _ => SideRenameKey::Pass,
    }
}

/// Keys that belong in the rename edit when they arrive on the panel instead.
/// Pane chords (F6, Ctrl+Tab, Alt+arrows) stay with the window. `VK_PROCESSKEY` always
/// belongs to the edit: swallowing it stops Japanese composition.
/// Virtual key a favourites key should act on. A Japanese IME delivers the real key as
/// `VK_PROCESSKEY`; `ImmGetVirtualKey` is the key the user pressed. Zero means leave it.
pub(crate) fn side_aim_key(hwnd: HWND, vk: u16) -> u16 {
    if vk != VK_PROCESSKEY.0 || hwnd.0.is_null() {
        return vk;
    }
    unsafe {
        let v = ImmGetVirtualKey(hwnd);
        if v == 0 { vk } else { v as u16 }
    }
}

pub(crate) fn side_key_belongs_to_edit(vk: u16, ctrl: bool, alt: bool) -> bool {
    if vk == VK_PROCESSKEY.0 {
        return true;
    }
    if alt {
        return false;
    }
    if matches!(side_rename_key(vk, false, false), SideRenameKey::Cancel | SideRenameKey::Commit) {
        return true;
    }
    if ctrl {
        return matches!(VIRTUAL_KEY(vk), VK_A | VK_C | VK_V | VK_X | VK_Z | VK_Y | VK_BACK | VK_LEFT | VK_RIGHT);
    }
    matches!(
        VIRTUAL_KEY(vk),
        VK_BACK | VK_DELETE | VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_SPACE
    ) || types_char(VIRTUAL_KEY(vk))
}

/// `WM_GETDLGCODE` for the rename edit. A single-line edit does not ask for Esc or Enter,
/// so `IsDialogMessage` (and the shell's equivalent) drops the real key and never queues it.
/// A posted `WM_KEYDOWN` skips that filter, which is why only the posted key cancelled.
/// `WANTCHARS` keeps IME result characters. This does not strip the IME context.
pub(crate) fn side_edit_dlg_code(base: u32) -> u32 {
    base | DLGC_WANTALLKEYS | DLGC_WANTCHARS
}

/// Pointer over the favourites panel.
/// A leaf opens on click, so the hand says so. A group header only expands or
/// collapses, and empty space is not a row, so both keep the arrow.
/// A reorder or an outside drop already chose a cursor; that one stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FavCursor {
    Keep,
    Hand,
    Arrow,
}

/// `group` is the row under the pointer. `None` is empty space (or the removal note).
/// `Some(true)` is a header. `Some(false)` is an openable leaf.
pub(crate) fn fav_hover_cursor(dragging: bool, group: Option<bool>) -> FavCursor {
    if dragging {
        FavCursor::Keep
    } else if group == Some(false) {
        FavCursor::Hand
    } else {
        FavCursor::Arrow
    }
}

/// Row under `y`, or none. `row_h <= 0` is not a hit (that would divide by zero).
/// The sum is widened so a large scroll does not overflow.
pub(crate) fn fav_row_at(y: i32, scroll: i32, row_h: i32, page: i32, n: usize) -> Option<usize> {
    if row_h <= 0 || y < 0 || y >= page || n == 0 {
        return None;
    }
    let i = (y as i64 + scroll as i64) / row_h as i64;
    if i < 0 || i as usize >= n { None } else { Some(i as usize) }
}

/// Location tooltip for one favourites row.
/// A filesystem path is the absolute path. A shell place has no file path, so the tip is its
/// display name, or the parsing name when that name is empty — the address-bar rule.
/// Every leaf is included, whether or not the label on screen is cut off. A group has no location.
/// A missing leaf keeps the location and adds a short note. Nothing is cut on a byte index.
pub(crate) fn fav_tip_text(path: &str, name: &str, group: bool, missing: bool) -> String {
    if group {
        return String::new();
    }
    let loc = addr_tip_text(path, name);
    if loc.is_empty() {
        return String::new();
    }
    if !missing {
        return loc;
    }
    let note = fav_missing_note();
    let mut out = String::with_capacity(loc.len() + 2 + note.len());
    out.push_str(&loc);
    out.push('\r');
    out.push('\n');
    out.push_str(note);
    out
}

fn fav_row_tip(row: &FavRow) -> String {
    fav_tip_text(&row.path, &row.name, row.group, !row.group && known_missing(&row.path))
}

fn fav_tip_info(owner: HWND, id: usize, label: &HSTRING) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: id,
        lpszText: PWSTR(label.as_ptr() as *mut u16),
        ..Default::default()
    }
}

thread_local! {
    /// Set while a tip is being hidden. Tooltip messages are synchronous and can deliver
    /// WM_MOUSEMOVE back into the panel; that must not show the tip again.
    static TIP_GUARD: Cell<bool> = const { Cell::new(false) };
    /// TSF composition on the rename edit. `ImmGetContext` is null for the modern Japanese IME,
    /// so the IMM composition string cannot be the signal. `WM_IME_STARTCOMPOSITION` still arrives.
    static SIDE_COMPOSING: Cell<bool> = const { Cell::new(false) };
}

/// Clears `TIP_GUARD` on drop, including when hiding the tip panics.
struct TipGuard;
impl Drop for TipGuard {
    fn drop(&mut self) {
        TIP_GUARD.with(|c| c.set(false));
    }
}


/// Stable tip key for a favourites row path.
fn fav_tip_key(tool: usize, path: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64 ^ ((tool as u64) << 32);
    for b in path.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Client row rect for visible favourites row `i`, mapped to screen.
unsafe fn side_row_item(i: usize) -> Option<RECT> {
    let (h, scroll) = app(|a| (a.side_hwnd, a.side_scroll));
    if h.0.is_null() {
        return None;
    }
    let mut rc = RECT::default();
    if GetClientRect(h, &mut rc).is_err() {
        return None;
    }
    let rh = row_h();
    let top = (i as i32) * rh - scroll;
    let row = RECT {
        left: rc.left,
        top,
        right: rc.right,
        bottom: top + rh,
    };
    Some(crate::ui::tip_screen_rect(h, row))
}

/// Show or hide a favourites tracking tip anchored to `item` (screen). Empty text hides once.
unsafe fn place_fav_tip(
    owner: HWND,
    id: usize,
    installed: bool,
    key: u64,
    text: &str,
    item: Option<RECT>,
) -> (bool, bool) {
    if text.is_empty() || item.is_none() {
        return crate::ui::tip_leave(owner, id, installed);
    }
    crate::ui::tip_enter(
        owner,
        id,
        key,
        text,
        item.unwrap(),
        crate::ui::TipSide::Right,
        installed,
    )
}

/// Client point of the cursor when it is inside `h`, else None.
unsafe fn cursor_in_client(h: HWND) -> Option<(i32, i32)> {
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() || !ScreenToClient(h, &mut pt).as_bool() {
        return None;
    }
    let mut rc = RECT::default();
    if GetClientRect(h, &mut rc).is_err() {
        return None;
    }
    (pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom).then_some((pt.x, pt.y))
}

/// Hide the panel tip and let the next hover show it.
/// `place_fav_tip` / `tip_leave` borrow the app state themselves, so they run outside `app()`.
pub(crate) unsafe fn hide_side_fav_tip() {
    if TIP_GUARD.with(|c| c.replace(true)) {
        return;
    }
    let _clear = TipGuard;
    let (owner, shown, installed) = app(|a| {
        a.fav_tip_hold = None;
        let shown = std::mem::replace(&mut a.fav_tip_shown, false);
        (a.side_hwnd, shown, a.fav_tip)
    });
    // Only leave when the tip was actually shown (#189: no POP storm on quiet moves).
    if shown {
        let (now, _) = crate::ui::tip_leave(owner, FAV_TIP_ID, installed);
        app(|a| a.fav_tip = now);
    }
    let back = cursor_in_client(owner);
    if back.is_none() {
        let paint = app(|a| a.side_hot.take().is_some());
        // Same freeze as a hover wash: this blit covers the rename edit and the IME window.
        if paint && !owner.0.is_null() && !fav_repaint_frozen() {
            _ = InvalidateRect(Some(owner), None, false);
        }
    } else if !owner.0.is_null() && !app(|a| a.fav_tip_after_drop) {
        let mut t = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: owner,
            dwHoverTime: 0,
        };
        _ = TrackMouseEvent(&mut t);
    }
}

/// Hide the panel tip after a scroll or a change of the rows. The pointer has to move before it returns.
pub(crate) unsafe fn dismiss_side_fav_tip() {
    let h = app(|a| a.side_hwnd);
    hide_side_fav_tip();
    app(|a| a.fav_tip_hold = cursor_in_client(h));
}

/// Rename, an open IME composition, or the hover that follows an outside drop.
fn side_tip_quiet() -> bool {
    let (renaming, after_drop, hwnd) = app(|a| (!a.side_edit_at.is_empty(), a.fav_tip_after_drop, a.side_hwnd));
    if renaming || after_drop {
        return true;
    }
    fav_tip_blocked(false, ime_composing(hwnd), false)
}

/// The rename edit is up, or an IME composition is open on it.
/// Hover and pin-timer paints stay down so they do not cover the composition window.
fn fav_repaint_frozen() -> bool {
    let (editing, host, edit) = app(|a| (!a.side_edit_at.is_empty(), a.side_hwnd, a.side_edit));
    let composing = if edit.0.is_null() { ime_composing(host) } else { ime_composing(edit) };
    !fav_hover_paints(true, editing, composing)
}

/// `x` and `y` are client coordinates. `row_i` is the visible row index for anchoring.
pub(crate) unsafe fn show_side_fav_tip(x: i32, y: i32, row_i: Option<usize>, row: Option<&FavRow>) {
    if TIP_GUARD.with(|c| c.get()) {
        return;
    }
    if app(|a| a.side_drag.is_some() || a.side_external) || side_tip_quiet() {
        if app(|a| a.fav_tip_shown) {
            hide_side_fav_tip();
        }
        return;
    }
    if app(|a| a.fav_tip_hold) == Some((x, y)) {
        return;
    }
    app(|a| a.fav_tip_hold = None);
    let owner = app(|a| a.side_hwnd);
    let text = row.map(fav_row_tip).unwrap_or_default();
    let path = row.map(|r| r.path.as_str()).unwrap_or("");
    if text.is_empty() || row_i.is_none() {
        hide_side_fav_tip();
        return;
    }
    let item = side_row_item(row_i.unwrap());
    let installed = app(|a| a.fav_tip);
    let key = fav_tip_key(FAV_TIP_ID, path);
    let (now, shown) = place_fav_tip(owner, FAV_TIP_ID, installed, key, &text, item);
    app(|a| {
        a.fav_tip = now;
        a.fav_tip_shown = shown;
    });
}

/// Mouse moved over `hwnd`. The panel tip hides once the pointer is on anything else,
/// including the tip window itself and the folder list.
pub(crate) unsafe fn hide_fav_tip_if_pointer_left(hwnd: HWND) {
    let (side, shown, tip) = app(|a| (a.side_hwnd, a.fav_tip_shown, a.tip));
    if !shown {
        return;
    }
    let on_panel = !side.0.is_null() && hwnd != tip && (hwnd == side || IsChild(side, hwnd).as_bool());
    if fav_tip_hides(shown, on_panel) {
        hide_side_fav_tip();
    }
}

/// Favourites window tip, anchored to the hit row's screen rect.
pub(crate) unsafe fn show_pop_fav_tip(owner: HWND, row: Option<&FavRow>, item: Option<RECT>) {
    let text = row.map(fav_row_tip).unwrap_or_default();
    let path = row.map(|r| r.path.as_str()).unwrap_or("");
    let installed = app(|a| a.fav_pop_tip);
    if text.is_empty() || item.is_none() {
        let (now, _) = crate::ui::tip_leave(owner, FAV_POP_TIP_ID, installed);
        app(|a| a.fav_pop_tip = now);
        return;
    }
    let key = fav_tip_key(FAV_POP_TIP_ID, path);
    let (now, _) = place_fav_tip(owner, FAV_POP_TIP_ID, installed, key, &text, item);
    app(|a| a.fav_pop_tip = now);
}

pub(crate) unsafe fn hide_pop_fav_tip(owner: HWND) {
    let installed = app(|a| a.fav_pop_tip);
    let (now, _) = crate::ui::tip_leave(owner, FAV_POP_TIP_ID, installed);
    app(|a| a.fav_pop_tip = now);
}

/// The window is going away, so the tool has to come off the tooltip. The next window adds its own.
pub(crate) unsafe fn drop_pop_fav_tip(owner: HWND) {
    let (tip, on) = app(|a| {
        if a.hover_tip.as_ref().is_some_and(|l| l.tool == FAV_POP_TIP_ID) {
            a.hover_tip = None;
        }
        let on = a.fav_pop_tip;
        a.fav_pop_tip = false;
        (a.tip, on)
    });
    if !on || tip.0.is_null() || owner.0.is_null() {
        return;
    }
    let label = HSTRING::from("");
    let ti = fav_tip_info(owner, FAV_POP_TIP_ID, &label);
    let lp = LPARAM(&ti as *const _ as isize);
    SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(lp));
    SendMessageW(tip, TTM_POP, None, None);
    SendMessageW(tip, TTM_DELTOOLW, None, Some(lp));
}

fn row_at(y: i32) -> Option<usize> {
    let n = rows().len();
    let scroll = app(|a| a.side_scroll);
    fav_row_at(y, scroll, row_h(), list_page(), n)
}

fn chevron_hit(x: i32, row: &FavRow, dpi: i32) -> bool {
    if !row.group {
        return false;
    }
    let s = |v: i32| v * dpi / 96;
    x < s(4) + s(14) * row.depth.min(8) as i32 + s(18)
}

/// Throw away every window's group wash. The next paint builds a brush for the accent in effect then.
pub(crate) unsafe fn drop_group_brushes() {
    let old = app(|a| {
        a.desks.iter_mut().map(|d| {
            d.group_brush_rgb = 0;
            std::mem::replace(&mut d.group_brush, HBRUSH::default())
        }).collect::<Vec<_>>()
    });
    for b in old {
        if !b.is_invalid() {
            _ = DeleteObject(b.into());
        }
    }
}

unsafe fn group_wash_brush(rgb: u32) -> HBRUSH {
    let cached = app(|a| (!a.group_brush.is_invalid() && a.group_brush_rgb == rgb).then_some(a.group_brush));
    if let Some(b) = cached {
        return b;
    }
    let brush = CreateSolidBrush(cref(rgb));
    if brush.is_invalid() {
        return brush;
    }
    let old = app(|a| {
        a.group_brush_rgb = rgb;
        std::mem::replace(&mut a.group_brush, brush)
    });
    if !old.is_invalid() && old != brush {
        _ = DeleteObject(old.into());
    }
    brush
}

pub(crate) unsafe fn paint_side(h: HWND) {
    pins_on_paint();
    let list = rows();
    app(|a| a.side_rows_mark = Some(mark_rows(&list)));
    let (dpi, font, icon, hot, sel, scroll, drop, accent) = app(|a| {
        (a.dpi, a.font, a.icon_font, a.side_hot, a.side_sel, a.side_scroll, a.side_drop, a.s.accent_now())
    });
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let rh = row_h();
        let list_bottom = rc.bottom - note_band();
        fill(dc, rc, side_ground());
        if list.is_empty() {
            text(dc, RECT { left: s(12), top: s(8), right: rc.right - s(8), bottom: s(48) }, tr(crate::Key::NoFavouritesYet), font, pal().sub, DT_LEFT);
            text(dc, RECT { left: s(12), top: s(28), right: rc.right - s(8), bottom: s(72) }, tr(crate::Key::RightClickAddGroup), font, pal().sub, DT_LEFT);
            paint_pin_note(dc, rc, font, dpi);
            return;
        }
        let any_group = list.iter().any(|f| f.group);
        let bold = if any_group { bold_ui_font() } else { font };
        let wash = if any_group {
            group_wash_brush(group_tint(accent, side_ground(), DARK_ON.load(Ordering::Relaxed)))
        } else {
            HBRUSH::default()
        };
        for (i, f) in list.iter().enumerate() {
            let y = i as i32 * rh - scroll;
            if y + rh < 0 || y >= list_bottom {
                continue;
            }
            let r = RECT { left: 0, top: y, right: rc.right, bottom: y + rh };
            let into = drop.is_some_and(|(row, zone)| row == i && zone == 1);
            if f.group && !wash.is_invalid() {
                FillRect(dc, &r, wash);
            }
            // Selection and hover replace the wash. The selected fill is mostly accent, so it wins.
            if sel == Some(i) {
                fill(dc, r, blend(accent, side_ground(), 0.16));
            } else if hot == Some(i) {
                fill(dc, r, blend(pal().bg, pal().card, if DARK_ON.load(Ordering::Relaxed) { 0.5 } else { 0.7 }));
            }
            if into {
                fill(dc, r, blend(accent, side_ground(), 0.28));
            }
            let indent = s(14) * f.depth.min(8) as i32;
            let mut x = s(4) + indent;
            let label_left;
            if f.group {
                let chev = RECT { left: x, top: y, right: x + s(16), bottom: y + rh };
                text(dc, chev, if f.open { "\u{E70D}" } else { "\u{E76C}" }, icon, pal().sub, DT_CENTER);
                x += s(16);
                let ir = RECT { left: x, top: y, right: x + s(18), bottom: y + rh };
                let glyph_ink = if sel == Some(i) || into { on_color(accent) } else { accent };
                text(dc, ir, "\u{E8B7}", icon, glyph_ink, DT_CENTER);
                label_left = ir.right + s(4);
            } else {
                let slot = (16 * dpi / 96).max(1);
                let iy = y + (rh - slot).max(0) / 2;
                if let Some((list, index, dim)) = shell_icon(&f.path, f.file) {
                    draw_fav_icon(dc, list, index, x, iy, slot, dim);
                }
                label_left = x + slot + s(4);
            }
            let label = if f.name.is_empty() { short_name(&f.path) } else { f.name.clone() };
            let label_font = if f.group { bold } else { font };
            // A path the background check (or a click) has found gone is drawn in the secondary colour.
            let ink = if !f.group && known_missing(&f.path) { pal().sub } else { pal().text };
            text(dc, RECT { left: label_left, right: rc.right - s(6), ..r }, &label, label_font, ink, DT_LEFT);
        }
        if !list.iter().any(|f| f.in_tree()) {
            let y = list.len() as i32 * rh - scroll + s(8);
            text(dc, RECT { left: s(12), top: y, right: rc.right - s(8), bottom: y + s(22) }, tr(crate::Key::NoFavouritesYet), font, pal().sub, DT_LEFT);
            text(dc, RECT { left: s(12), top: y + s(20), right: rc.right - s(8), bottom: y + s(44) }, tr(crate::Key::RightClickAddGroup), font, pal().sub, DT_LEFT);
        }
        if let Some((row, zone)) = drop {
            if zone != 1 {
                if let Some(f) = list.get(row) {
                    let at = if zone == 0 { row } else { after_row(&list, row) };
                    let line_y = if zone == 0 { row as i32 * rh - scroll } else { at as i32 * rh - scroll + rh - s(2) };
                    // A line under the Explorer groups is a new root favourite, not a child of that row.
                    let indent = if f.in_tree() { s(14) * f.depth.min(8) as i32 } else { 0 };
                    if line_y + s(2) >= 0 && line_y < list_bottom {
                        fill(dc, RECT { left: s(8) + indent, top: line_y, right: rc.right - s(8), bottom: line_y + s(2) }, accent);
                    }
                }
            }
        }
        paint_pin_note(dc, rc, font, dpi);
    });
    // The buffer covers the rename edit. Paint the edit again so the composition string comes back.
    let edit = app(|a| a.side_edit);
    if !edit.0.is_null() && IsWindowVisible(edit).as_bool() {
        _ = InvalidateRect(Some(edit), None, false);
        _ = UpdateWindow(edit);
    }
    if list.iter().any(|f| !f.group) {
        arm_icon_prewarm(h);
    }
}

/// The rename box over its row. Hidden unless a rename is in progress.
pub(crate) unsafe fn place_side_edit() {
    let (edit, id, host, scroll, dpi) = app(|a| (a.side_edit, a.side_edit_at.clone(), a.side_hwnd, a.side_scroll, a.dpi));
    if edit.0.is_null() || id.is_empty() || host.0.is_null() {
        return;
    }
    let list = rows();
    let rh = row_h();
    let y = tree_pos(&list, &id).map(|i| i as i32 * rh - scroll).unwrap_or(0);
    let s = |v: i32| v * dpi / 96;
    let outer = RECT { left: s(8), top: y, right: s(8) + s(220), bottom: y + rh };
    place_edit(edit, outer, true);
}

/// The edit and its panel while a favourites rename is open.
pub(crate) fn side_rename_windows() -> Option<(HWND, HWND)> {
    let (edit, host, on) = app(|a| (a.side_edit, a.side_hwnd, !a.side_edit_at.is_empty()));
    if !on || edit.0.is_null() || host.0.is_null() {
        None
    } else {
        Some((edit, host))
    }
}

/// Open TSF/IMM composition on the rename edit. IMM is null for the modern Japanese IME.
pub(crate) fn side_ime_busy() -> bool {
    if SIDE_COMPOSING.with(|c| c.get()) {
        return true;
    }
    let Some((edit, _)) = side_rename_windows() else { return false };
    ime_composing(edit)
}

/// The edit has to own the keyboard. A second `SetFocus` covers a parent that kept it.
pub(crate) unsafe fn focus_side_edit(edit: HWND) {
    if edit.0.is_null() {
        return;
    }
    _ = SetFocus(Some(edit));
    if GetFocus() != edit {
        _ = SetFocus(Some(edit));
    }
}

/// Attach the thread's default IME context. A null context would strip Japanese composition.
unsafe fn keep_side_ime(edit: HWND) {
    if edit.0.is_null() {
        return;
    }
    _ = ImmAssociateContextEx(edit, HIMC(std::ptr::null_mut()), IACE_DEFAULT);
}

unsafe fn ensure_side_edit(host: HWND, font: HFONT) -> HWND {
    let edit = app(|a| a.side_edit);
    if !edit.0.is_null() && IsWindow(Some(edit)).as_bool() {
        return edit;
    }
    let e = child(host, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), ID_SIDE_EDIT, font);
    if e.0.is_null() {
        return e;
    }
    app(|a| a.side_edit = e);
    _ = SetWindowSubclass(e, Some(side_edit_proc), SIDE_EDIT_SUB, 0);
    keep_side_ime(e);
    e
}

unsafe extern "system" fn side_edit_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, id: usize, data: usize) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| side_edit_proc_inner(h, msg, wp, lp, id, data)))
}

fn side_edit_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    unsafe {
        match msg {
            WM_NCDESTROY => {
                SIDE_COMPOSING.with(|c| c.set(false));
                _ = RemoveWindowSubclass(h, Some(side_edit_proc), SIDE_EDIT_SUB);
            }
            WM_GETDLGCODE => {
                let base = DefSubclassProc(h, msg, wp, lp);
                return LRESULT(side_edit_dlg_code(base.0 as u32) as isize);
            }
            WM_IME_STARTCOMPOSITION => {
                SIDE_COMPOSING.with(|c| c.set(true));
                return DefSubclassProc(h, msg, wp, lp);
            }
            WM_IME_ENDCOMPOSITION => {
                let r = DefSubclassProc(h, msg, wp, lp);
                SIDE_COMPOSING.with(|c| c.set(false));
                return r;
            }
            WM_IME_COMPOSITION => {
                // Pass the composition through. Only a still-open comp string keeps Esc with the IME.
                let comp = lp.0 & GCS_COMPSTR.0 as isize != 0;
                if comp {
                    SIDE_COMPOSING.with(|c| c.set(true));
                }
                let r = DefSubclassProc(h, msg, wp, lp);
                if !comp {
                    SIDE_COMPOSING.with(|c| c.set(false));
                }
                return r;
            }
            WM_IME_KEYDOWN => {
                // Do not swallow the IME key. A bare Esc/Enter, with no composition, still closes the edit.
                let busy = side_ime_busy();
                let r = DefSubclassProc(h, msg, wp, lp);
                if !busy {
                    apply_side_rename_key(wp.0 as u16);
                }
                return r;
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let vk = wp.0 as u16;
                let process = vk == VK_PROCESSKEY.0;
                let busy = side_ime_busy();
                let orig = side_aim_key(h, vk);
                // VK_PROCESSKEY and every WM_IME_* go to the edit. Closing the rename on the
                // process key itself would eat Japanese composition. Alt chords are WM_SYSKEYDOWN
                // and stay with the edit (Alt+Enter must not commit the name).
                let r = DefSubclassProc(h, msg, wp, lp);
                if msg == WM_KEYDOWN && !busy && !(process && side_ime_busy()) {
                    apply_side_rename_key(orig);
                }
                return r;
            }
            WM_CHAR if wp.0 == 0x1B && !side_ime_busy() => {
                end_side_rename(false);
                return LRESULT(0);
            }
            _ => {}
        }
        DefSubclassProc(h, msg, wp, lp)
    }
}

fn apply_side_rename_key(vk: u16) {
    match side_rename_key(vk, false, false) {
        SideRenameKey::Cancel => unsafe { end_side_rename(false) },
        SideRenameKey::Commit => unsafe { end_side_rename(true) },
        SideRenameKey::Pass => {}
    }
}

pub(crate) unsafe fn begin_side_rename(id: &[usize]) {
    // Gone before the edit takes focus, so the tip cannot sit under the IME candidate.
    hide_side_fav_tip();
    SIDE_COMPOSING.with(|c| c.set(false));
    let list = rows();
    let label = list.iter().find(|r| r.in_tree() && r.id == id).map(|r| if r.name.is_empty() { short_name(&r.path) } else { r.name.clone() }).unwrap_or_default();
    let (host, font) = app(|a| (a.side_hwnd, a.font));
    if host.0.is_null() {
        return;
    }
    let edit = ensure_side_edit(host, font);
    if edit.0.is_null() {
        return;
    }
    app(|a| {
        a.side_edit_at = id.to_vec();
        a.side_edit_key = node_key(&a.s.tree, id);
    });
    _ = SetWindowTextW(edit, &HSTRING::from(label.as_str()));
    SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    place_side_edit();
    // After the edit is on screen. A hidden or zero-size window does not take a TSF context.
    keep_side_ime(edit);
    focus_side_edit(edit);
    let len = GetWindowTextLengthW(edit) as usize;
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(len as isize)));
    focus_side_edit(edit);
}

pub(crate) unsafe fn end_side_rename(save: bool) {
    SIDE_COMPOSING.with(|c| c.set(false));
    let (edit, id, key) = app(|a| (a.side_edit, std::mem::take(&mut a.side_edit_at), a.side_edit_key.take()));
    if edit.0.is_null() || id.is_empty() {
        return;
    }
    if save {
        let name = text_of(edit);
        // The tree may have changed under the edit box; rename the node it was opened on, or nothing.
        let done = app(|a| {
            let Some(id) = key.as_ref().and_then(|k| locate(&a.s.tree, &id, k)) else { return false };
            rename_at(&mut a.s.tree, &id, name.trim());
            a.s.sync_favs();
            true
        });
        if done {
            favs_changed();
        }
    }
    // Cleared above, so the EN_KILLFOCUS this hide sends cannot save over a cancel.
    _ = ShowWindow(edit, SW_HIDE);
    let host = app(|a| a.side_hwnd);
    if !host.0.is_null() {
        // Esc returns to the panel. A focus-loss save must not pull focus back off the address bar.
        if !save {
            _ = SetFocus(Some(host));
        }
        // Invalidate without painting left the edit's white rectangle on the row.
        _ = RedrawWindow(Some(host), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW);
    }
}

fn changed() {
    unsafe { favs_changed() };
}

fn new_group(parent: &[usize]) {
    let name = tr(crate::Key::Group).to_string();
    app(|a| {
        a.s.ensure_fav_tree();
        if !parent.is_empty() {
            set_open(&mut a.s.tree, parent, true);
        }
        add_group(&mut a.s.tree, parent, &name);
        a.s.sync_favs();
    });
    let id = rows().iter().rev().find(|r| r.in_tree() && r.group && r.name == name).map(|r| r.id.clone());
    changed();
    if let Some(id) = id {
        unsafe { begin_side_rename(&id) };
    }
}

fn toggle_row(row: &FavRow) {
    if row.group && !row.in_tree() {
        let open = app(|a| match row.kind {
            RowKind::Pins => {
                a.s.pins_open = !a.s.pins_open;
                a.s.pins_open
            }
            RowKind::Files => {
                a.s.files_open = !a.s.files_open;
                a.s.files_open
            }
            RowKind::Tree => false,
        });
        if open {
            request_pins(false);
        }
        changed();
        clamp_side_scroll();
        return;
    }
    if !row.in_tree() || !row.group {
        return;
    }
    app(|a| {
        toggle_open(&mut a.s.tree, &row.id);
    });
    changed();
    clamp_side_scroll();
}

unsafe fn side_menu(h: HWND, index: Option<usize>, x: i32, y: i32) {
    let row = index.and_then(|i| rows().get(i).cloned());
    if row.as_ref().is_some_and(|r| r.group && !r.in_tree()) {
        return;
    }
    let Ok(menu) = CreatePopupMenu() else { return };
    let add = |id: usize, label: &str| {
        _ = AppendMenuW(menu, MF_STRING, id, &HSTRING::from(label));
    };
    let pin_leaf = row.as_ref().is_some_and(|r| r.kind == RowKind::Pins && !r.group);
    let file_leaf = row.as_ref().is_some_and(|r| r.kind == RowKind::Files && !r.group);
    if pin_leaf {
        add(6, tr(crate::Key::UnpinFromExplorer));
        add(4, tr(crate::Key::Open));
    } else if file_leaf {
        add(6, tr(crate::Key::RemoveFromExplorerFavourites));
        add(4, tr(crate::Key::Open));
        add(5, tr(crate::Key::Reveal));
    } else {
        add(1, tr(crate::Key::NewGroup));
        if row.is_some() {
            add(2, tr(crate::Key::Rename2));
            add(3, tr(crate::Key::Delete));
        }
        if row.as_ref().is_some_and(|r| !r.group) {
            add(4, tr(crate::Key::Open));
        }
        if row.as_ref().is_some_and(|r| r.file) {
            add(5, tr(crate::Key::Reveal));
        }
        let server = row.as_ref().is_some_and(|r| r.in_tree() && !r.group && app(|a| node_ref(&a.s.tree, &r.id).is_some_and(|n| n.server.is_some())));
        match crate::remote::menu_extra(row.is_some(), row.as_ref().is_some_and(|r| r.in_tree()), row.as_ref().is_some_and(|r| r.group), server) {
            crate::remote::MenuExtra::Add => add(7, tr(crate::Key::AddServerConnection)),
            crate::remote::MenuExtra::Edit => add(8, tr(crate::Key::EditServerConnection)),
            crate::remote::MenuExtra::None => {}
        }
    }
    let mut pt = POINT { x, y };
    _ = ClientToScreen(h, &mut pt);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, h, None).0;
    _ = DestroyMenu(menu);
    let Some(row) = row else {
        if cmd == 1 {
            new_group(&[]);
        } else if cmd == 7 {
            open_server_dialog(app(|a| a.main), &[], None, None);
        }
        return;
    };
    if !row.in_tree() {
        match cmd {
            4 => open_fav_row(&row),
            5 if row.file => {
                if let Some(parent) = parent_of(&row.path) {
                    let i = active_pane();
                    navigate(i, &parent);
                    focus_view(i);
                }
            }
            6 => unpin_path(&row.path, row.kind == RowKind::Files),
            _ => {}
        }
        return;
    }
    match cmd {
        1 => {
            let parent: Vec<usize> = if row.group { row.id.clone() } else { row.id[..row.id.len() - 1].to_vec() };
            new_group(&parent);
        }
        2 => begin_side_rename(&row.id),
        3 => {
            let links = servers_at(&row.id);
            app(|a| {
                delete_at(&mut a.s.tree, &row.id);
                a.s.sync_favs();
                a.side_sel = None;
            });
            release_servers(links);
            changed();
        }
        7 => {
            let parent = if row.group { row.id.clone() } else { row.id[..row.id.len() - 1].to_vec() };
            open_server_dialog(app(|a| a.main), &parent, None, None);
        }
        8 => open_server_dialog(app(|a| a.main), &[], Some(&row.id), None),
        4 => open_fav_row(&row),
        6 => unpin_path(&row.path, row.kind == RowKind::Files),
        5 => {
            if let Some(parent) = parent_of(&row.path) {
                let i = active_pane();
                navigate(i, &parent);
                focus_view(i);
            }
        }
        _ => {}
    }
}

fn set_drop(next: Option<(usize, u8)>) -> bool {
    app(|a| {
        let paint = a.side_drop != next;
        a.side_drop = next;
        paint
    })
}

fn track_drop(y: i32) -> bool {
    let drag = app(|a| a.side_drag.clone());
    let Some(drag) = drag else { return false };
    let list = rows();
    let Some(i) = row_at(y) else { return set_drop(None) };
    let Some(target) = list.get(i) else { return set_drop(None) };
    let kind = side_act(drag.kind, drag.group, drag.file, drag.path.is_empty(), target.kind);
    let next = match kind {
        SideAct::None => None,
        SideAct::Pin => list.iter().position(|r| r.kind == RowKind::Pins && r.group).map(|row| (row, 1)),
        SideAct::PinFile => list.iter().position(|r| r.kind == RowKind::Files && r.group).map(|row| (row, 1)),
        SideAct::Copy | SideAct::Move => {
            let scroll = app(|a| a.side_scroll);
            let rh = row_h();
            let top = i as i32 * rh - scroll;
            Some((i, drop_zone(target.group, y - top, rh)))
        }
    };
    set_drop(next)
}

fn repaint_side() {
    clamp_side_scroll();
    let h = app(|a| a.side_hwnd);
    if !h.0.is_null() {
        unsafe { _ = InvalidateRect(Some(h), None, false) };
    }
}

/// The shell folder updates after the verb returns. One read shortly after, off the UI thread.
fn refresh_pins_soon() {
    let h = app(|a| a.side_hwnd);
    if h.0.is_null() {
        request_pins(true);
        return;
    }
    unsafe { _ = SetTimer(Some(h), TIMER_PINS, 700, None) };
}

fn pin_home(path: &str, name: &str, file: bool) {
    if path.is_empty() {
        return;
    }
    let added = remember_pinned(path, name, file);
    if !shell_home_verb(path, true) {
        if added {
            forget_pinned(path);
        }
        repaint_side();
        unsafe { _ = MessageBeep(MB_ICONWARNING) };
        return;
    }
    let was_open = app(|a| if file { a.s.files_open } else { a.s.pins_open });
    app(|a| {
        if file {
            a.s.files_open = true;
        } else {
            a.s.pins_open = true;
        }
    });
    if was_open {
        unsafe {
            across_windows(|| {
                let h = app(|a| a.side_hwnd);
                if !h.0.is_null() {
                    _ = InvalidateRect(Some(h), None, false);
                }
            });
        }
    } else {
        changed();
    }
    refresh_pins_soon();
}

fn unpin_path(path: &str, file: bool) {
    if file {
        // The row stays until a Home re-read omits it. A verb that returns success and
        // leaves the file in place used to hide it for 3 seconds, then the next read put it back.
        shell_unfavorite_file(path);
        repaint_side();
        return;
    }
    forget_pinned(path);
    repaint_side();
    let removed = shell_home_verb(path, false);
    refresh_pins_soon();
    if !removed {
        unsafe { _ = MessageBeep(MB_ICONWARNING) };
    }
}

fn side_inside(h: HWND, x: i32, y: i32) -> bool {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(h, &mut rc) };
    x >= rc.left && x < rc.right && y >= rc.top && y < rc.bottom
}

fn nudge_side(down: bool) {
    let list = rows();
    let Some(row) = app(|a| a.side_sel).and_then(|i| list.get(i).cloned()).filter(|r| r.in_tree()) else { return };
    let moved = app(|a| {
        let id = nudge_node(&mut a.s.tree, &row.id, down);
        if id.is_some() {
            a.s.sync_favs();
        }
        id
    });
    if let Some(id) = moved {
        let i = tree_pos(&rows(), &id);
        app(|a| a.side_sel = i);
        changed();
    }
}

fn finish_drop() {
    let list = rows();
    let (drag, onto) = app(|a| {
        let onto = a.side_drop.and_then(|(i, zone)| list.get(i).cloned().map(|r| (r, zone)));
        let drag = a.side_drag.clone();
        a.side_drag = None;
        a.side_drop = None;
        a.side_press = None;
        (drag, onto)
    });
    let Some(drag) = drag else { return };
    let Some((target, zone)) = onto else {
        repaint_side();
        return;
    };
    match side_act(drag.kind, drag.group, drag.file, drag.path.is_empty(), target.kind) {
        SideAct::None => repaint_side(),
        SideAct::Pin => pin_home(&drag.path, &drag.name, false),
        SideAct::PinFile => pin_home(&drag.path, &drag.name, true),
        SideAct::Copy => {
            let name = if drag.name.is_empty() || drag.name == short_name(&drag.path) { String::new() } else { drag.name.clone() };
            let copied = app(|a| {
                let id = copy_leaf(&mut a.s.tree, &drag.path, &name, &target.id, zone, drag.file);
                if id.is_some() {
                    if zone == 1 {
                        set_open(&mut a.s.tree, &target.id, true);
                    }
                    a.s.sync_favs();
                }
                id
            });
            if let Some(id) = copied {
                let i = tree_pos(&rows(), &id);
                app(|a| a.side_sel = i);
                changed();
            } else {
                repaint_side();
            }
        }
        SideAct::Move => {
            let moved = app(|a| {
                let id = apply_drop(&mut a.s.tree, &drag.id, &target.id, zone);
                if id.is_some() {
                    reveal_drop(&mut a.s.tree, &drag.id, &target.id, zone);
                    a.s.sync_favs();
                }
                id
            });
            if let Some(id) = moved {
                let i = tree_pos(&rows(), &id);
                app(|a| a.side_sel = i);
                changed();
            } else {
                repaint_side();
            }
        }
    }
}

fn fav_exts(nodes: &[FavNode], seen: &mut std::collections::HashSet<String>, out: &mut Vec<String>) {
    for n in nodes {
        if n.is_group() {
            fav_exts(&n.children, seen, out);
        } else if n.file {
            let ext = file_ext(&n.path);
            if !ext.is_empty() && !per_file_icon(&ext) && seen.insert(ext.clone()) {
                out.push(ext);
            }
        }
    }
}

fn prewarm_fav_icons() {
    let exts = app(|a| {
        a.s.ensure_fav_tree();
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        fav_exts(&a.s.tree, &mut seen, &mut out);
        for p in pin_files() {
            let ext = file_ext(&p.path);
            if !ext.is_empty() && !per_file_icon(&ext) && seen.insert(ext.clone()) {
                out.push(ext);
            }
        }
        out
    });
    prewarm_exts(&exts);
}

pub(crate) extern "system" fn side_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| side_proc_inner(h, msg, wp, lp)))
}
fn side_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        note_side(h);
        note_pins_hwnd(h);
        match msg {
            WM_DESTROY => {
                hide_side_fav_tip();
                drop_side(h);
                drop_pins_hwnd(h);
                _ = RevokeDragDrop(h);
                _ = KillTimer(Some(h), TIMER_ICON);
                _ = KillTimer(Some(h), TIMER_PINS);
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_FAV_ICON => {
                clear_icon_refresh();
                if !fav_repaint_frozen() {
                    _ = InvalidateRect(Some(h), None, false);
                }
                // A row may have just been marked missing. Refresh the note if the pointer is still on it.
                if app(|a| a.fav_tip_hold.is_none() && a.side_drag.is_none() && !a.side_external && !a.fav_tip_after_drop) {
                    if let Some((x, y)) = cursor_in_client(h) {
                        let hot = row_at(y);
                        let list = rows();
                        match fav_tip_act(true, hot, list.len()) {
                            FavTipAct::Hide => hide_side_fav_tip(),
                            FavTipAct::Show(i) => show_side_fav_tip(x, y, Some(i), list.get(i)),
                        }
                    } else if app(|a| a.fav_tip_shown) {
                        hide_side_fav_tip();
                    }
                }
            }
            WM_TIMER if wp.0 == TIMER_ICON => {
                _ = KillTimer(Some(h), TIMER_ICON);
                icon_prewarm_done();
                prewarm_fav_icons();
            }
            WM_TIMER if wp.0 == TIMER_PINS => {
                _ = KillTimer(Some(h), TIMER_PINS);
                request_pins(true);
            }
            WM_PINS => {
                // Row count changed under the panel. Reclamp the scroll, then paint now:
                // a later WM_PAINT may never arrive while the queue stays busy.
                // Pin reads retry until Explorer answers. Only a real change of the rows hides the tip.
                let mark = fav_rows_mark();
                let changed = app(|a| {
                    let changed = a.side_rows_mark.is_some_and(|prev| prev != mark);
                    a.side_rows_mark = Some(mark);
                    changed
                });
                if changed {
                    dismiss_side_fav_tip();
                }
                clamp_side_scroll();
                // A pin retry paints every 700ms. That blit sits on the IME candidate while renaming.
                if !fav_repaint_frozen() {
                    _ = InvalidateRect(Some(h), None, false);
                    _ = UpdateWindow(h);
                }
                if !pins_loaded() {
                    _ = SetTimer(Some(h), TIMER_PINS, 700, None);
                }
            }
            WM_PAINT => paint_side(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_SETCURSOR => {
                // The scrollbar is not a row. A drag (reorder or a file dropped from outside)
                // keeps the cursor it already shows; do not put a hand on top of it.
                if (lp.0 & 0xffff) as u32 != HTCLIENT {
                    return DefWindowProcW(h, msg, wp, lp);
                }
                let dragging = app(|a| a.side_drag.is_some() || a.side_external);
                let group = if dragging {
                    None
                } else {
                    cursor_in_client(h).and_then(|(_, y)| {
                        let i = row_at(y)?;
                        rows().get(i).map(|r| r.group)
                    })
                };
                let id = match fav_hover_cursor(dragging, group) {
                    FavCursor::Keep => return DefWindowProcW(h, msg, wp, lp),
                    FavCursor::Hand => IDC_HAND,
                    FavCursor::Arrow => IDC_ARROW,
                };
                if let Ok(c) = LoadCursorW(None, id) {
                    SetCursor(Some(c));
                    return LRESULT(1);
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_MOUSEMOVE => {
                if TIP_GUARD.with(|c| c.get()) || crate::ui::tip_pumping() {
                    return LRESULT(0);
                }
                let (x, y) = xy(lp);
                let hot = row_at(y);
                let dragging = app(|a| a.side_drag.is_some());
                if dragging {
                    // The tip sits below the pointer. Hide it before the drag moves onto that spot.
                    hide_side_fav_tip();
                    if track_drop(y) {
                        _ = InvalidateRect(Some(h), None, false);
                    }
                } else if let Some((px, py, i)) = app(|a| a.side_press) {
                    if fav_drag_moved(x - px, y - py) {
                        if let Some(row) = rows().get(i).filter(|r| !(r.group && !r.in_tree())) {
                            hide_side_fav_tip();
                            let drag = drag_of(row);
                            app(|a| a.side_drag = Some(drag));
                            if track_drop(y) {
                                _ = InvalidateRect(Some(h), None, false);
                            }
                        }
                    }
                } else if app(|a| a.side_external || a.fav_tip_after_drop) {
                    // Same as an outside drag: no tip until the pointer leaves (after_drop)
                    // or the drag ends (side_external). Arm leave only after a real post-drop
                    // hover inside, so OLE leave during Drop cannot clear the hold early.
                    // Hide once; repeating hide while the tool stayed installed was a TTM_POP storm.
                    if app(|a| a.fav_tip_shown) {
                        hide_side_fav_tip();
                    }
                    if app(|a| a.fav_tip_after_drop) && cursor_in_client(h).is_some() {
                        let arm = app(|a| {
                            if a.fav_tip_leave_armed {
                                false
                            } else {
                                a.fav_tip_leave_armed = true;
                                true
                            }
                        });
                        if arm {
                            let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                            _ = TrackMouseEvent(&mut t);
                        }
                    }
                } else {
                    let list = rows();
                    let act = fav_tip_act(true, hot, list.len());
                    let paint = app(|a| {
                        let next = match act {
                            FavTipAct::Show(i) => Some(i),
                            FavTipAct::Hide => None,
                        };
                        let paint = a.side_hot != next;
                        a.side_hot = next;
                        paint
                    });
                    if paint && !fav_repaint_frozen() {
                        _ = InvalidateRect(Some(h), None, false);
                    }
                    match act {
                        FavTipAct::Hide => hide_side_fav_tip(),
                        FavTipAct::Show(i) => show_side_fav_tip(x, y, Some(i), list.get(i)),
                    }
                    // After the tip is shown. Activating it can replace an earlier leave request.
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    _ = TrackMouseEvent(&mut t);
                }
            }
            WM_MOUSELEAVE => {
                // The post-drop hold ends only once the pointer has actually left after a
                // real post-drop hover. OLE drop and tip hide can synthesize a leave while
                // the cursor is still inside (or still owned by the drag source); ignore
                // those so the tip stays down until a real exit.
                if cursor_in_client(h).is_none() {
                    app(|a| {
                        if a.fav_tip_leave_armed {
                            a.fav_tip_after_drop = false;
                            a.fav_tip_leave_armed = false;
                        }
                    });
                }
                hide_side_fav_tip();
                app(|a| a.side_hot = None);
                if !fav_repaint_frozen() {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_MOUSEWHEEL => {
                dismiss_side_fav_tip();
                let rh = row_h();
                let delta = ((wp.0 >> 16) as i16 as i32) * rh / 120;
                let max = max_side_scroll();
                app(|a| a.side_scroll = (a.side_scroll - delta).clamp(0, max));
                sync_side_scrollbar();
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_SIZE => {
                clamp_side_scroll();
                place_side_edit();
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_VSCROLL => {
                dismiss_side_fav_tip();
                let mut si = SCROLLINFO { cbSize: size_of::<SCROLLINFO>() as u32, fMask: SIF_ALL, ..Default::default() };
                _ = GetScrollInfo(h, SB_VERT, &mut si);
                let rh = row_h();
                let cur = app(|a| a.side_scroll);
                let want = match SCROLLBAR_COMMAND((wp.0 & 0xFFFF) as i32) {
                    SB_LINEUP => cur - rh,
                    SB_LINEDOWN => cur + rh,
                    SB_PAGEUP => cur - si.nPage as i32,
                    SB_PAGEDOWN => cur + si.nPage as i32,
                    SB_THUMBTRACK | SB_THUMBPOSITION => si.nTrackPos,
                    SB_TOP => 0,
                    SB_BOTTOM => i32::MAX,
                    _ => cur,
                };
                let max = max_side_scroll();
                app(|a| a.side_scroll = want.clamp(0, max));
                sync_side_scrollbar();
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_LBUTTONDOWN => {
                // Gone before the click is handled, so the tip cannot take the click.
                hide_side_fav_tip();
                let (x, y) = xy(lp);
                _ = SetFocus(Some(h));
                if let Some(i) = row_at(y) {
                    let list = rows();
                    let dpi = app(|a| a.dpi);
                    if list.get(i).is_some_and(|r| chevron_hit(x, r, dpi)) {
                        let row = list[i].clone();
                        toggle_row(&row);
                    } else {
                        app(|a| {
                            a.side_sel = Some(i);
                            a.side_press = Some((x, y, i));
                        });
                        _ = SetCapture(h);
                        _ = InvalidateRect(Some(h), None, false);
                    }
                } else {
                    app(|a| a.side_sel = None);
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                // Take the press before opening. A navigate focuses the pane and can re-enter here.
                let (dragged, press) = app(|a| (a.side_drag.is_some(), a.side_press.take()));
                if dragged {
                    // Outside the panel the drop is cancelled. Inside, the slot is this release point.
                    if !side_inside(h, x, y) {
                        app(|a| {
                            a.side_drag = None;
                            a.side_drop = None;
                            a.side_press = None;
                        });
                        repaint_side();
                    } else {
                        _ = track_drop(y);
                        finish_drop();
                    }
                } else if let Some((px, py, i)) = press {
                    let on_row = side_inside(h, x, y) && row_at(y) == Some(i);
                    if let Some(row) = rows().get(i).cloned() {
                        match fav_release(row.group, x - px, y - py, on_row) {
                            FavRelease::Toggle => toggle_row(&row),
                            FavRelease::Open => {
                                // The accent wash is selection, not hover. Clear it before the pane navigates.
                                app(|a| a.side_sel = fav_sel_after_release(FavRelease::Open, a.side_sel));
                                _ = InvalidateRect(Some(h), None, false);
                                open_fav_row(&row);
                            }
                            FavRelease::None => {}
                        }
                    }
                }
                _ = ReleaseCapture();
            }
            WM_CAPTURECHANGED => {
                let paint = app(|a| {
                    let paint = a.side_drag.is_some();
                    a.side_drag = None;
                    a.side_drop = None;
                    a.side_press = None;
                    paint
                });
                if paint {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_SYSKEYDOWN => {
                let vk = wp.0 as u16;
                let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
                if fav_step(vk, down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT)).is_none() || app(|a| a.side_drag.is_some() || !a.side_edit_at.is_empty()) {
                    return DefWindowProcW(h, msg, wp, lp);
                }
                nudge_side(vk == VK_DOWN.0);
            }
            WM_LBUTTONDBLCLK => {
                // The first button-up already opened the leaf or toggled the group.
                // CS_DBLCLKS delivers the second press here. Acting again would open
                // twice, or expand a group and immediately collapse it.
            }
            WM_RBUTTONDOWN => {
                // Left click focuses the panel. Right click did not, so F2 after the menu
                // hit the file list. Focus before the menu opens; TrackPopupMenu restores it.
                hide_side_fav_tip();
                let (_x, y) = xy(lp);
                _ = SetFocus(Some(h));
                if let Some(i) = row_at(y) {
                    app(|a| a.side_sel = Some(i));
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_RBUTTONUP => {
                // The menu opens at the pointer. The tip would sit on top of it.
                hide_side_fav_tip();
                let (x, y) = xy(lp);
                let i = row_at(y);
                if let Some(i) = i {
                    app(|a| a.side_sel = Some(i));
                }
                _ = SetFocus(Some(h));
                side_menu(h, i, x, y);
                // The menu restores focus to whoever had it. Rename moved it to the edit;
                // anything else, including a dismissed menu, stays on this panel so F2 works.
                let (editing, edit) = app(|a| (!a.side_edit_at.is_empty(), a.side_edit));
                if editing && !edit.0.is_null() {
                    focus_side_edit(edit);
                } else {
                    _ = SetFocus(Some(h));
                }
            }
            WM_KEYDOWN => {
                // Japanese IME delivers F2 and Esc as VK_PROCESSKEY. A posted WM_KEYDOWN is the
                // raw key, which is why only the posted key opened or closed the edit.
                let renaming = side_rename_windows().is_some();
                let raw = wp.0 as u16;
                let process = raw == VK_PROCESSKEY.0;
                let vk = side_aim_key(h, raw);
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let alt = GetKeyState(VK_MENU.0 as i32) < 0;
                if process {
                    let busy = side_ime_busy();
                    let r = DefWindowProcW(h, msg, wp, lp);
                    if busy || side_ime_busy() {
                        if renaming {
                            if let Some((edit, _)) = side_rename_windows() {
                                focus_side_edit(edit);
                            }
                        }
                        return r;
                    }
                }
                if renaming && side_key_belongs_to_edit(vk, ctrl, alt) {
                    match side_rename_key(vk, false, false) {
                        SideRenameKey::Cancel => end_side_rename(false),
                        SideRenameKey::Commit => end_side_rename(true),
                        SideRenameKey::Pass if !process => {
                            if let Some((edit, _)) = side_rename_windows() {
                                focus_side_edit(edit);
                                _ = PostMessageW(Some(edit), WM_KEYDOWN, wp, lp);
                            }
                        }
                        SideRenameKey::Pass => {
                            if let Some((edit, _)) = side_rename_windows() {
                                focus_side_edit(edit);
                            }
                        }
                    }
                } else if !renaming {
                    match VIRTUAL_KEY(vk) {
                        VK_DELETE => {
                            let row = app(|a| a.side_sel).and_then(|i| rows().get(i).cloned());
                            if let Some(row) = row.filter(|r| r.in_tree()) {
                                let links = servers_at(&row.id);
                                app(|a| {
                                    delete_at(&mut a.s.tree, &row.id);
                                    a.s.sync_favs();
                                    a.side_sel = None;
                                });
                                release_servers(links);
                                changed();
                            }
                        }
                        VK_F2 => {
                            if let Some(row) = app(|a| a.side_sel).and_then(|i| rows().get(i).cloned()).filter(|r| r.in_tree()) {
                                begin_side_rename(&row.id);
                            }
                        }
                        VK_RETURN => {
                            if let Some(row) = app(|a| a.side_sel).and_then(|i| rows().get(i).cloned()) {
                                if row.group {
                                    toggle_row(&row);
                                } else {
                                    open_fav_row(&row);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().card));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => {
                if (wp.0 & 0xffff) as i32 == ID_SIDE_EDIT && ((wp.0 >> 16) & 0xffff) as u32 == EN_KILLFOCUS {
                    end_side_rename(true);
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

/// A pane selection that can land on one Explorer group. Mixed files and folders are refused.
#[derive(Clone)]
enum HomeDrop {
    Folders(Vec<String>),
    Files(Vec<String>),
}

/// Paths from the drop, plus whether each one is a file when the drag data already says so.
struct RawDrop {
    path: String,
    file: Option<bool>,
}

#[derive(Clone)]
struct Pending {
    items: Vec<Dropped>,
    home: Option<HomeDrop>,
}

fn pending_of(raw: Vec<RawDrop>) -> Pending {
    let home = home_of(&raw);
    let items = raw.into_iter().map(|r| Dropped { path: r.path, file: r.file.unwrap_or(false) }).collect();
    Pending { items, home }
}

/// Pinning needs a known file-or-folder bit for every item. An unknown bit is not guessed:
/// the same drop can still be added to favourites.
fn home_of(raw: &[RawDrop]) -> Option<HomeDrop> {
    if raw.is_empty() || raw.iter().any(|r| r.file.is_none()) {
        return None;
    }
    let pairs: Vec<(String, bool)> = raw.iter().map(|r| (r.path.clone(), r.file == Some(false))).collect();
    if let Some(paths) = pin_drop_paths(&pairs) {
        Some(HomeDrop::Folders(paths))
    } else {
        file_drop_paths(&pairs).map(HomeDrop::Files)
    }
}

fn hits_of(list: &[FavRow]) -> Vec<DropHit> {
    list.iter().map(|r| DropHit { tree: r.in_tree(), group: r.group, id: r.id.clone() }).collect()
}

/// Always ask, middle of a filesystem folder row: copy or move into that folder.
fn into_folder_at(y: i32) -> Option<String> {
    let dropm = app(|a| a.s.dropm);
    let list = rows();
    let i = row_at(y)?;
    let row = list.get(i)?;
    let scroll = app(|a| a.side_scroll);
    let rh = row_h();
    let top = i as i32 * rh - scroll;
    fav_into_folder(dropm, row.group, row.file, &row.path, y - top, rh).then(|| row.path.clone())
}

fn fav_target(y: i32) -> (DropSpot, Option<(usize, u8)>) {
    let list = rows();
    let hits = hits_of(&list);
    let index = row_at(y);
    let zone = match index.and_then(|i| list.get(i).map(|row| (i, row.group))) {
        Some((i, group)) => {
            let scroll = app(|a| a.side_scroll);
            let rh = row_h();
            let top = i as i32 * rh - scroll;
            drop_zone(group, y - top, rh)
        }
        None => 2,
    };
    let spot = drop_spot(&hits, index, zone);
    let line = drop_line(&hits, &spot);
    (spot, line)
}

fn home_accepts(drop: &HomeDrop, kind: RowKind) -> bool {
    matches!((drop, kind), (HomeDrop::Folders(_), RowKind::Pins) | (HomeDrop::Files(_), RowKind::Files))
}

#[implement(IDropTarget)]
struct SideDrop {
    hwnd: HWND,
    pending: RefCell<Option<Pending>>,
}

impl SideDrop {
    unsafe fn client_y(&self, pt: &POINTL) -> Option<i32> {
        let mut p = POINT { x: pt.x, y: pt.y };
        ScreenToClient(self.hwnd, &mut p).as_bool().then_some(p.y)
    }

    unsafe fn home_at(&self, y: i32) -> Option<RowKind> {
        let list = rows();
        let kind = row_at(y).and_then(|i| list.get(i)).map(|r| r.kind)?;
        matches!(kind, RowKind::Pins | RowKind::Files).then_some(kind)
    }

    unsafe fn paint_line(&self, line: Option<(usize, u8)>) {
        if set_drop(line) {
            _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Pins and favourite-file headers keep the old Explorer pin drop. Everywhere else the
    /// cursor is a link and the insertion line follows the pointer. Shift must not turn this
    /// into a move of the real files.
    unsafe fn hover(&self, pt: &POINTL, effect: *mut DROPEFFECT) {
        if effect.is_null() {
            return;
        }
        bind_hwnd(self.hwnd);
        let pending = self.pending.borrow().clone();
        let y = self.client_y(pt);
        let at = y.and_then(|y| self.home_at(y));
        let home = pending.as_ref().and_then(|p| p.home.as_ref());
        if home.zip(at).is_some_and(|(d, k)| home_accepts(d, k)) {
            *effect = drop_pick(*effect);
            let next = (*effect != DROPEFFECT_NONE).then(|| {
                let want = match home {
                    Some(HomeDrop::Folders(_)) => RowKind::Pins,
                    _ => RowKind::Files,
                };
                rows().iter().position(|r| r.group && r.kind == want).map(|row| (row, 1u8))
            }).flatten();
            self.paint_line(next);
            return;
        }
        let has = pending.as_ref().is_some_and(|p| !p.items.is_empty());
        if !has {
            *effect = DROPEFFECT_NONE;
            self.paint_line(None);
            return;
        }
        if y.and_then(into_folder_at).is_some() {
            let offer = (*effect).0 & (DROPEFFECT_COPY.0 | DROPEFFECT_MOVE.0);
            *effect = if offer == 0 { DROPEFFECT_COPY } else { DROPEFFECT(offer) };
            self.paint_line(None);
            return;
        }
        *effect = DROPEFFECT(fav_link_effect((*effect).0));
        if (*effect).0 == 0 {
            self.paint_line(None);
            return;
        }
        self.paint_line(y.map(|yy| fav_target(yy).1).unwrap_or(None));
    }

    unsafe fn clear(&self) {
        bind_hwnd(self.hwnd);
        app(|a| a.side_external = false);
        *self.pending.borrow_mut() = None;
        self.paint_line(None);
    }
}

impl IDropTarget_Impl for SideDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            // Hide before the drop target is live, so the tip cannot sit under the drag cursor.
            app(|a| a.side_external = true);
            hide_side_fav_tip();
            *self.pending.borrow_mut() = Some(pending_of(read_drop(&d)));
            self.hover(pt, effect);
        }
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { self.hover(pt, effect) };
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe { self.clear() };
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            // Hide while side_external is still set. Do not arm TME_LEAVE here: during an
            // OLE drop Win32 may treat the pointer as outside, so leave would fire as soon as
            // changed pumps paint and clear the hold before the pointer has really left.
            hide_side_fav_tip();
            let pending = self.pending.borrow().clone();
            let y = self.client_y(pt);
            let at = y.and_then(|y| self.home_at(y));
            let allowed = if effect.is_null() { DROPEFFECT_NONE } else { *effect };
            self.clear();
            // clear drops side_external before the tree changes. The pointer is still
            // inside, so keep the tip down until a real leave. A drag that leaves without a
            // drop does not set this: that path is DragLeave. Set the flag after clear
            // so a nested leave from the hide above cannot wipe it, and before changed
            // so hover during the tree update stays quiet. The next WM_MOUSEMOVE arms leave.
            app(|a| {
                a.fav_tip_after_drop = true;
                a.fav_tip_leave_armed = false;
            });
            hide_side_fav_tip();
            if !effect.is_null() {
                *effect = DROPEFFECT_NONE;
            }
            let Some(pending) = pending else { return Ok(()) };
            if let (Some(home), Some(at)) = (pending.home.as_ref(), at) {
                if home_accepts(home, at) {
                    let eff = drop_pick(allowed);
                    if eff == DROPEFFECT_NONE {
                        return Ok(());
                    }
                    if !effect.is_null() {
                        *effect = eff;
                    }
                    match home {
                        HomeDrop::Folders(paths) => {
                            for path in paths {
                                pin_home(path, "", false);
                            }
                        }
                        HomeDrop::Files(paths) => {
                            for path in paths {
                                pin_home(path, "", true);
                            }
                        }
                    }
                    return Ok(());
                }
            }
            if let Some(dst) = y.and_then(into_folder_at) {
                let srcs: Vec<String> = pending.items.iter().map(|it| it.path.clone()).filter(|p| is_fs(p)).collect();
                if srcs.len() == pending.items.len() && !srcs.is_empty() {
                    if let Some(obj) = d.as_ref() {
                        let owner = app(|a| a.main);
                        if let Some(mv) = ask_copy_or_move(owner) {
                            let eff = run_asked(obj, srcs, dst, mv);
                            if !effect.is_null() {
                                *effect = DROPEFFECT(eff);
                            }
                        }
                    }
                    return Ok(());
                }
            }
            let link = fav_link_effect(allowed.0);
            if link == 0 || pending.items.is_empty() {
                return Ok(());
            }
            if !effect.is_null() {
                *effect = DROPEFFECT(link);
            }
            let spot = y.map(|yy| fav_target(yy).0).unwrap_or(DropSpot::End);
            let id = app(|a| {
                a.s.ensure_fav_tree();
                let id = insert_dropped(&mut a.s.tree, &pending.items, &spot);
                if id.is_some() {
                    a.s.sync_favs();
                }
                id
            });
            if let Some(id) = id {
                let sel = tree_pos(&rows(), &id);
                app(|a| a.side_sel = sel);
                changed();
            } else {
                repaint_side();
            }
        }
        Ok(())
    }
}

/// `CF_HDROP` first: the paths are already in the drop memory, so a UNC name is not resolved.
/// Shell items are only asked for a display name when that format is missing.
unsafe fn read_drop(d: &Ref<IDataObject>) -> Vec<RawDrop> {
    let paths = hdrop_paths(d);
    if !paths.is_empty() {
        let flags = shell_flags(d, &paths);
        return paths.into_iter().zip(flags).filter(|(p, _)| is_fs(p)).map(|(path, file)| RawDrop { path, file }).collect();
    }
    shell_paths(d)
}

unsafe fn hdrop_paths(d: &Ref<IDataObject>) -> Vec<String> {
    use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
    use windows::Win32::System::Ole::ReleaseStgMedium;
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
        let path = canon_drop_path(&String::from_utf16_lossy(buf.get(..got).unwrap_or(&[])));
        if !path.is_empty() {
            paths.push(path);
        }
    }
    ReleaseStgMedium(&mut medium);
    paths
}

/// File bits aligned with `paths`. A count mismatch does not fall back to display names:
/// those can open a share. An unknown bit stays unknown.
unsafe fn shell_flags(d: &Ref<IDataObject>, paths: &[String]) -> Vec<Option<bool>> {
    let Some(arr) = drop_items(d) else {
        return paths.iter().map(|p| path_is_file(p)).collect();
    };
    let n = arr.GetCount().unwrap_or(0) as usize;
    if n != paths.len() {
        return paths.iter().map(|p| path_is_file(p)).collect();
    }
    (0..n)
        .map(|k| match arr.GetItemAt(k as u32) {
            Ok(item) => file_bit(&item, &paths[k]).or_else(|| path_is_file(&paths[k])),
            Err(_) => path_is_file(&paths[k]),
        })
        .collect()
}

unsafe fn shell_paths(d: &Ref<IDataObject>) -> Vec<RawDrop> {
    let Some(arr) = drop_items(d) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(item) = arr.GetItemAt(k) else { continue };
        let Ok(name) = item.GetDisplayName(SIGDN_FILESYSPATH) else { continue };
        let path = canon_drop_path(&pwstr(name));
        if path.is_empty() || !is_fs(&path) {
            continue;
        }
        let file = file_bit(&item, &path).or_else(|| path_is_file(&path));
        out.push(RawDrop { path, file });
    }
    out
}

/// `Some(true)` is a file. Fast cached attributes first. `GetAttributes` and
/// `GetFileAttributesW` are skipped for UNC and mapped drives: either can bind the share.
unsafe fn file_bit(item: &IShellItem, path: &str) -> Option<bool> {
    if let Some(dir) = fast_is_dir(item) {
        return Some(!dir);
    }
    if !unc_path(path) && !share_path(path) {
        if let Ok(flags) = item.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM) {
            return Some(!shell_is_folder(flags));
        }
    }
    None
}

unsafe fn path_is_file(path: &str) -> Option<bool> {
    use windows::Win32::Storage::FileSystem::{GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, INVALID_FILE_ATTRIBUTES};
    if unc_path(path) || share_path(path) {
        return None;
    }
    let attr = GetFileAttributesW(&HSTRING::from(path));
    if attr == INVALID_FILE_ATTRIBUTES {
        return None;
    }
    Some(attr & FILE_ATTRIBUTE_DIRECTORY.0 == 0)
}

const PKEY_FILE_ATTRIBUTES: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xb725f130_47ef_101a_a5f1_02608c9eebac),
    pid: 13,
};

unsafe fn fast_is_dir(item: &IShellItem) -> Option<bool> {
    use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PROPVARIANT};
    use windows::Win32::UI::Shell::PropertiesSystem::{GPS_FASTPROPERTIESONLY, IPropertyStore};
    let item2 = item.cast::<IShellItem2>().ok()?;
    let store = item2.GetPropertyStore::<IPropertyStore>(GPS_FASTPROPERTIESONLY).ok()?;
    let mut v: PROPVARIANT = store.GetValue(&PKEY_FILE_ATTRIBUTES).ok()?;
    let dir = prop_directory(&v);
    _ = PropVariantClear(&mut v);
    dir
}

fn prop_directory(v: &windows::Win32::System::Com::StructuredStorage::PROPVARIANT) -> Option<bool> {
    use windows::Win32::System::Variant::{VT_I4, VT_INT, VT_UI4, VT_UINT};
    unsafe {
        let head = &*v.Anonymous.Anonymous;
        let n = if head.vt == VT_UI4 || head.vt == VT_UINT {
            head.Anonymous.ulVal
        } else if head.vt == VT_I4 || head.vt == VT_INT {
            head.Anonymous.lVal as u32
        } else {
            return None;
        };
        Some(n & 0x10 != 0)
    }
}

pub(crate) unsafe fn register_side_drop(h: HWND) {
    if h.0.is_null() {
        return;
    }
    let dt: IDropTarget = SideDrop { hwnd: h, pending: RefCell::new(None) }.into();
    _ = RegisterDragDrop(h, &dt);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{
        fav_hover_cursor, fav_hover_paints, fav_release, fav_row_at, fav_row_height, fav_sel_after_release, fav_tip_act, fav_tip_blocked, fav_tip_hides, fav_tip_same, fav_tip_text,
        mark_rows, side_edit_dlg_code, side_key_belongs_to_edit, side_rename_key, FavCursor, FavRelease, FavRow, FavTipAct, RowKind, SideRenameKey,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_A, VK_ESCAPE, VK_F2, VK_F5, VK_F6, VK_PROCESSKEY, VK_RETURN, VK_TAB};
    use windows::Win32::UI::WindowsAndMessaging::{DLGC_WANTALLKEYS, DLGC_WANTCHARS};
    use crate::i18n::UI;

    #[test]
    fn row_height_uses_the_tree_and_font_fallback_scales() {
        assert_eq!(fav_row_height(19, 16), 19);
        assert_eq!(fav_row_height(38, 32), 38);
        assert_eq!(fav_row_height(0, 16), 20);
        assert_eq!(fav_row_height(0, 32), 36);
        assert_eq!(fav_row_height(-1, 16), 20);
    }

    #[test]
    fn favourite_tip_is_the_full_location_and_a_missing_note() {
        let prev = UI.load(Ordering::Relaxed);
        UI.store(0, Ordering::Relaxed);
        let jp = r"F:\CAC\サンプル";
        assert_eq!(fav_tip_text(jp, "表示", false, false), jp);
        assert_eq!(fav_tip_text(r"\\srv\share\資料", "資料", false, false), r"\\srv\share\資料");
        assert_eq!(fav_tip_text(r"C:\Users\me\Documents", "Docs", false, false), r"C:\Users\me\Documents");
        assert_eq!(fav_tip_text("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", "This PC", false, false), "This PC");
        assert_eq!(fav_tip_text("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", "", false, false), "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}");
        assert_eq!(fav_tip_text("", "ごみ箱", false, false), "ごみ箱");
        assert_eq!(fav_tip_text("", "", false, false), "");
        assert_eq!(fav_tip_text("", "", false, true), "");
        // A group has no location, even when a path was passed in by mistake.
        assert_eq!(fav_tip_text(r"C:\gone", "gone", true, true), "");
        // A multibyte name is not a drive letter. Slicing it by bytes would panic or drop a character.
        assert_eq!(fav_tip_text("サンプル", "", false, false), "サンプル");
        assert_eq!(fav_tip_text(r"D:\없는\폴더", "폴더", false, true), "D:\\없는\\폴더\r\nNot found");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(fav_tip_text(r"D:\없는\폴더", "폴더", false, true), "D:\\없는\\폴더\r\n찾을 수 없음");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(fav_tip_text(r"D:\없는\폴더", "폴더", false, true), "D:\\없는\\폴더\r\n見つかりません");
        UI.store(prev, Ordering::Relaxed);
    }

    #[test]
    fn row_mark_includes_the_whole_path() {
        let row = |path: &str| FavRow {
            id: vec![0],
            depth: 0,
            group: false,
            file: false,
            open: false,
            name: "n".into(),
            path: path.into(),
            kind: RowKind::Tree,
        };
        let a = mark_rows(&[row(r"D:\없는\폴더")]);
        let b = mark_rows(&[row(r"D:\없는\폴더2")]);
        let c = mark_rows(&[row("サンプル")]);
        let d = mark_rows(&[row("サンフ")]);
        assert_ne!(a, b);
        assert_ne!(c, d);
        assert_eq!(a, mark_rows(&[row(r"D:\없는\폴더")]));
    }

    #[test]
    fn row_hit_misses_empty_space_a_stale_index_and_a_zero_height() {
        assert_eq!(fav_row_at(10, 0, 20, 100, 4), Some(0));
        assert_eq!(fav_row_at(20, 0, 20, 100, 4), Some(1));
        assert_eq!(fav_row_at(10, 60, 20, 100, 4), Some(3));
        // index 4 is past the last row after the tree shrinks
        assert_eq!(fav_row_at(10, 80, 20, 100, 4), None);
        assert_eq!(fav_row_at(100, 0, 20, 100, 4), None);
        assert_eq!(fav_row_at(-1, 0, 20, 100, 4), None);
        assert_eq!(fav_row_at(10, 0, 0, 100, 4), None);
        assert_eq!(fav_row_at(10, i32::MAX, 20, 100, 4), None);
        assert_eq!(fav_row_at(0, 0, 20, 100, 0), None);
    }

    #[test]
    fn tip_hides_when_the_pointer_leaves_misses_a_row_or_the_index_is_stale() {
        assert_eq!(fav_tip_act(false, Some(1), 3), FavTipAct::Hide);
        assert_eq!(fav_tip_act(true, None, 3), FavTipAct::Hide);
        assert_eq!(fav_tip_act(true, Some(3), 3), FavTipAct::Hide);
        assert_eq!(fav_tip_act(true, Some(0), 0), FavTipAct::Hide);
        assert_eq!(fav_tip_act(true, Some(1), 3), FavTipAct::Show(1));
        assert!(fav_tip_hides(true, false));
        assert!(!fav_tip_hides(true, true));
        assert!(!fav_tip_hides(false, false));
    }

    #[test]
    fn tip_stays_down_while_renaming_composing_or_until_the_pointer_leaves_after_a_drop() {
        assert!(!fav_tip_blocked(false, false, false));
        assert!(fav_tip_blocked(true, false, false));
        assert!(fav_tip_blocked(false, true, false));
        assert!(fav_tip_blocked(false, false, true));
        assert!(fav_tip_blocked(true, true, true));
    }

    #[test]
    fn tip_pump_skips_only_the_same_tool_and_text() {
        assert!(fav_tip_same(Some(10), r"C:", 10, r"C:"));
        assert!(!fav_tip_same(Some(10), r"C:", 10, r"C:"));
        assert!(!fav_tip_same(Some(10), r"C:", 11, r"C:"));
        assert!(!fav_tip_same(Some(10), "", 10, ""));
        assert!(!fav_tip_same(None, r"C:", 10, r"C:"));
    }

    #[test]
    fn release_opens_a_leaf_toggles_a_group_and_ignores_a_drag() {
        assert_eq!(fav_release(false, 0, 0, true), FavRelease::Open);
        assert_eq!(fav_release(false, 4, 0, true), FavRelease::Open);
        assert_eq!(fav_release(false, 2, 2, true), FavRelease::Open);
        assert_eq!(fav_release(true, 0, 0, true), FavRelease::Toggle);
        assert_eq!(fav_release(true, -4, 0, true), FavRelease::Toggle);
        // One pixel past the reorder slop is a drag, for a leaf and for a header.
        assert_eq!(fav_release(false, 5, 0, true), FavRelease::None);
        assert_eq!(fav_release(true, 0, 5, true), FavRelease::None);
        assert_eq!(fav_release(true, 3, 3, true), FavRelease::None);
        // Released off the pressed row, or outside the panel: not a click.
        assert_eq!(fav_release(false, 0, 0, false), FavRelease::None);
        assert_eq!(fav_release(true, 0, 0, false), FavRelease::None);
        assert_eq!(fav_release(false, 100, 100, false), FavRelease::None);
    }

    #[test]
    fn click_open_clears_the_highlight_and_a_toggle_keeps_it() {
        assert_eq!(fav_sel_after_release(FavRelease::Open, Some(2)), None);
        assert_eq!(fav_sel_after_release(FavRelease::Open, None), None);
        assert_eq!(fav_sel_after_release(FavRelease::Toggle, Some(1)), Some(1));
        assert_eq!(fav_sel_after_release(FavRelease::None, Some(4)), Some(4));
        assert_eq!(fav_sel_after_release(FavRelease::None, None), None);
    }

    #[test]
    fn side_rename_esc_cancels_enter_commits_and_ime_keys_pass() {
        assert_eq!(side_rename_key(VK_ESCAPE.0, false, false), SideRenameKey::Cancel);
        assert_eq!(side_rename_key(VK_RETURN.0, false, false), SideRenameKey::Commit);
        // An open composition keeps Esc and Enter. The process key is never a rename command.
        assert_eq!(side_rename_key(VK_ESCAPE.0, true, false), SideRenameKey::Pass);
        assert_eq!(side_rename_key(VK_RETURN.0, true, false), SideRenameKey::Pass);
        assert_eq!(side_rename_key(VK_PROCESSKEY.0, false, false), SideRenameKey::Pass);
        assert_eq!(side_rename_key(VK_ESCAPE.0, false, true), SideRenameKey::Pass);
        assert_eq!(side_rename_key(VK_A.0, false, false), SideRenameKey::Pass);
        // Real Esc/Enter are dropped by a dialog filter unless the edit asks for every key.
        let code = side_edit_dlg_code(0);
        assert_eq!(code & DLGC_WANTALLKEYS, DLGC_WANTALLKEYS);
        assert_eq!(code & DLGC_WANTCHARS, DLGC_WANTCHARS);
        // The panel must hand the process key and Esc to the edit, and must leave F6 / F5 alone.
        assert!(side_key_belongs_to_edit(VK_PROCESSKEY.0, false, false));
        assert!(side_key_belongs_to_edit(VK_ESCAPE.0, false, false));
        assert!(side_key_belongs_to_edit(VK_RETURN.0, false, false));
        assert!(side_key_belongs_to_edit(VK_A.0, false, false));
        assert!(!side_key_belongs_to_edit(VK_F2.0, false, false));
        assert!(!side_key_belongs_to_edit(VK_F6.0, false, false));
        assert!(!side_key_belongs_to_edit(VK_F5.0, false, false));
        assert!(!side_key_belongs_to_edit(VK_TAB.0, false, false));
        assert!(!side_key_belongs_to_edit(VK_A.0, false, true)); // Alt chord stays with the window
    }

    #[test]
    fn hover_paint_stays_down_while_renaming_or_composing() {
        assert!(fav_hover_paints(true, false, false));
        assert!(!fav_hover_paints(false, false, false)); // pointer stayed on the same row
        assert!(!fav_hover_paints(true, true, false)); // rename edit is open
        assert!(!fav_hover_paints(true, false, true)); // IME composition
        assert!(!fav_hover_paints(true, true, true));
        assert!(!fav_hover_paints(false, true, true));
    }

    #[test]
    fn hover_cursor_is_hand_on_a_leaf_and_arrow_elsewhere_except_while_dragging() {
        assert_eq!(fav_hover_cursor(false, Some(false)), FavCursor::Hand);
        assert_eq!(fav_hover_cursor(false, Some(true)), FavCursor::Arrow);
        assert_eq!(fav_hover_cursor(false, None), FavCursor::Arrow);
        // Reorder and an outside drop keep the cursor that drag already shows.
        assert_eq!(fav_hover_cursor(true, Some(false)), FavCursor::Keep);
        assert_eq!(fav_hover_cursor(true, Some(true)), FavCursor::Keep);
        assert_eq!(fav_hover_cursor(true, None), FavCursor::Keep);
    }
}
