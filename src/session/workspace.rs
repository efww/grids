// Nine workspace slots in maguro.json. Each occupied slot keeps a `.grids` snapshot of this
// window's tabs (layout and folders), the same text a named session file would store.
// Loading replaces the tabs on screen and leaves theme, hotkey, and favourites alone.
// Ctrl+Alt+1..9 loads. Ctrl+Alt+Shift+1..9 saves. Neither chord is a global hotkey.

use std::cell::RefCell;

use serde_json::{Map, Value};

use super::{looks_like_session, Session};
use crate::app::*;
use crate::i18n::*;
use crate::imports::*;
use crate::pane::*;
use crate::ui::*;

pub(crate) const SLOTS: usize = 9;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Slot {
    name: String,
    grids: String,
}

struct Book {
    slots: [Slot; SLOTS],
    /// Ask before a load replaces the tabs. Missing in maguro.json means no.
    confirm: bool,
}

thread_local! {
    static BOOK: RefCell<Book> = RefCell::new(Book { slots: std::array::from_fn(|_| Slot::default()), confirm: false });
}

fn with_book<R>(f: impl FnOnce(&mut Book) -> R) -> R {
    BOOK.with(|b| f(&mut b.borrow_mut()))
}

pub(crate) fn reset_workspaces() {
    with_book(|b| {
        b.slots = std::array::from_fn(|_| Slot::default());
        b.confirm = false;
    });
}

pub(crate) fn workspaces_confirm() -> bool {
    with_book(|b| b.confirm)
}

pub(crate) fn set_workspaces_confirm(on: bool) {
    with_book(|b| b.confirm = on);
}

/// A typed name. Tabs and newlines are dropped so a menu label cannot swallow the shortcut column.
pub(crate) fn clean_workspace_name(raw: &str) -> String {
    let t: String = raw.trim().chars().filter(|c| !matches!(c, '\n' | '\r' | '\t')).take(80).collect();
    t.trim().to_string()
}

pub(crate) fn workspace_name(slot: usize) -> String {
    with_book(|b| b.slots.get(slot).map(|s| s.name.clone()).unwrap_or_default())
}

pub(crate) fn workspace_grids(slot: usize) -> Option<String> {
    with_book(|b| {
        let s = b.slots.get(slot)?;
        (!s.grids.is_empty()).then(|| s.grids.clone())
    })
}

pub(crate) fn workspace_blank(slot: usize) -> bool {
    with_book(|b| match b.slots.get(slot) {
        Some(s) => s.name.is_empty() && s.grids.is_empty(),
        None => true,
    })
}

/// What the status line and the confirm dialog call this slot. A custom name wins; otherwise the digit.
pub(crate) fn workspace_label(slot: usize) -> String {
    let name = workspace_name(slot);
    if !name.is_empty() {
        name
    } else {
        (slot + 1).to_string()
    }
}

fn slot_ok(slot: usize) -> bool {
    slot < SLOTS
}

pub(crate) fn store_workspace(slot: usize, grids: &str) -> bool {
    if !slot_ok(slot) || grids.len() > 1_500_000 || !looks_like_session(grids) {
        return false;
    }
    with_book(|b| b.slots[slot].grids = grids.to_string());
    true
}

pub(crate) fn rename_workspace_name(slot: usize, raw: &str) -> bool {
    if !slot_ok(slot) {
        return false;
    }
    let name = clean_workspace_name(raw);
    with_book(|b| b.slots[slot].name = name);
    true
}

pub(crate) fn clear_workspace_slot(slot: usize) {
    if slot_ok(slot) {
        with_book(|b| b.slots[slot] = Slot::default());
    }
}

/// Slots written into maguro.json. Empty ones are left out. `slot` is 1..9.
pub(crate) fn workspaces_json() -> Vec<Value> {
    with_book(|b| {
        b.slots.iter().enumerate().filter_map(|(i, s)| {
            if s.name.is_empty() && s.grids.is_empty() {
                return None;
            }
            let mut m = Map::new();
            m.insert("slot".into(), Value::from(i as u64 + 1));
            if !s.name.is_empty() {
                m.insert("name".into(), Value::String(s.name.clone()));
            }
            if !s.grids.is_empty() {
                m.insert("grids".into(), Value::String(s.grids.clone()));
            }
            Some(Value::Object(m))
        }).collect()
    })
}

/// Replace the in-memory slots from maguro.json. A bad entry is skipped, not kept.
pub(crate) fn load_workspaces(slots: &[Value], confirm: bool) {
    reset_workspaces();
    set_workspaces_confirm(confirm);
    for v in slots {
        let Some(obj) = v.as_object() else { continue };
        let n = obj.get("slot").and_then(|x| x.as_u64()).unwrap_or(0);
        if !(1..=SLOTS as u64).contains(&n) {
            continue;
        }
        let i = (n - 1) as usize;
        let grids = obj.get("grids").and_then(|g| g.as_str()).unwrap_or("");
        let grids = if !grids.is_empty() && looks_like_session(grids) { grids.to_string() } else { String::new() };
        let name = clean_workspace_name(obj.get("name").and_then(|x| x.as_str()).unwrap_or(""));
        if grids.is_empty() && name.is_empty() {
            continue;
        }
        with_book(|b| b.slots[i] = Slot { name, grids });
    }
}

/// Copy the snapshot's tabs onto `dst`. Window rect, theme, hotkey, and favourites stay on `dst`.
/// The previous session-file link is dropped so Save does not overwrite that file with this layout.
pub(crate) fn apply_window_grids(dst: &mut Session, text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || !looks_like_session(text) {
        return false;
    }
    let src = Session::parse(text);
    if src.view_tabs().is_empty() {
        return false;
    }
    dst.take_layout(&src);
    dst.file.clear();
    for t in &mut dst.tabs {
        t.session.clear();
    }
    true
}

/// `.grids` text of the window on screen. Settings are in the text and ignored on load.
pub(crate) fn capture_window(s: &Session) -> String {
    s.snapshot()
}

fn menu_title(slot: usize) -> String {
    let n = (slot + 1).to_string();
    let name = workspace_name(slot);
    if workspace_grids(slot).is_none() && name.is_empty() {
        return tf(crate::Key::WorkspaceSlotEmpty, &[("n", &n)]);
    }
    if name.is_empty() {
        return n;
    }
    format!("{n}  {}", name.replace('&', "&&"))
}

fn chord_label(text: &str, key: &str) -> String {
    if key.is_empty() {
        text.to_string()
    } else {
        format!("{text}\t{key}")
    }
}

fn digit_chord(id: &str, slot: usize) -> String {
    crate::keys::index(id).map(|row| crate::keys::range_chord(row, (slot + 1) as u8)).unwrap_or_default()
}

unsafe fn note(text: String) {
    app(|a| a.note = Some((text, std::time::Instant::now() + std::time::Duration::from_secs(3))));
    update_status();
}

pub(crate) unsafe fn workspace_menu() -> Result<HMENU> {
    let root = CreatePopupMenu()?;
    for i in 0..SLOTS {
        let sub = CreatePopupMenu()?;
        fill_slot(sub, i)?;
        AppendMenuW(root, MF_POPUP | MF_STRING, sub.0 as usize, &HSTRING::from(menu_title(i)))?;
    }
    AppendMenuW(root, MF_SEPARATOR, 0, PCWSTR::null())?;
    AppendMenuW(root, MF_STRING, ID_WS_CONFIRM, &HSTRING::from(tr(crate::Key::AskBeforeLoadingWorkspace)))?;
    Ok(root)
}

unsafe fn fill_slot(sub: HMENU, slot: usize) -> Result<()> {
    let item = |id: usize, text: &str| AppendMenuW(sub, MF_STRING, id, &HSTRING::from(text));
    item(ID_WS_SAVE + slot, &chord_label(tr(crate::Key::SaveToSlot), &digit_chord("ws.save", slot)))?;
    item(ID_WS_LOAD + slot, &chord_label(tr(crate::Key::LoadSlot), &digit_chord("ws.load", slot)))?;
    item(ID_WS_RENAME + slot, tr(crate::Key::Rename))?;
    item(ID_WS_CLEAR + slot, tr(crate::Key::ClearSlot))?;
    Ok(())
}

fn workspace_root(menu: HMENU) -> Option<HMENU> {
    let n = unsafe { GetMenuItemCount(Some(menu)) };
    if n <= 0 {
        return None;
    }
    for i in 0..n {
        let sub = unsafe { GetSubMenu(menu, i) };
        if sub.0.is_null() {
            continue;
        }
        let inner = unsafe { GetSubMenu(sub, 0) };
        if !inner.0.is_null() && unsafe { GetMenuItemID(inner, 0) } == ID_WS_SAVE as u32 {
            return Some(sub);
        }
    }
    None
}

/// Labels and the empty-slot grey state. Called when the gear menu opens, so a save shows up at once.
pub(crate) unsafe fn sync_workspace_menu(menu: HMENU) {
    let Some(root) = workspace_root(menu) else { return };
    for i in 0..SLOTS {
        let sub = GetSubMenu(root, i as i32);
        if sub.0.is_null() {
            continue;
        }
        let _ = ModifyMenuW(root, i as u32, MF_BYPOSITION | MF_POPUP | MF_STRING, sub.0 as usize, &HSTRING::from(menu_title(i)));
        let save = chord_label(tr(crate::Key::SaveToSlot), &digit_chord("ws.save", i));
        let load = chord_label(tr(crate::Key::LoadSlot), &digit_chord("ws.load", i));
        let _ = ModifyMenuW(sub, (ID_WS_SAVE + i) as u32, MF_BYCOMMAND | MF_STRING, ID_WS_SAVE + i, &HSTRING::from(save));
        let _ = ModifyMenuW(sub, (ID_WS_LOAD + i) as u32, MF_BYCOMMAND | MF_STRING, ID_WS_LOAD + i, &HSTRING::from(load));
        let _ = ModifyMenuW(sub, (ID_WS_RENAME + i) as u32, MF_BYCOMMAND | MF_STRING, ID_WS_RENAME + i, &HSTRING::from(tr(crate::Key::Rename)));
        let _ = ModifyMenuW(sub, (ID_WS_CLEAR + i) as u32, MF_BYCOMMAND | MF_STRING, ID_WS_CLEAR + i, &HSTRING::from(tr(crate::Key::ClearSlot)));
        let no_grids = workspace_grids(i).is_none();
        let gray = |id: usize, off: bool| {
            let flag = MF_BYCOMMAND | if off { MF_GRAYED } else { MF_ENABLED };
            let _ = EnableMenuItem(sub, id as u32, flag);
        };
        gray(ID_WS_LOAD + i, no_grids);
        gray(ID_WS_CLEAR + i, workspace_blank(i));
    }
    let on = workspaces_confirm();
    let _ = CheckMenuItem(root, ID_WS_CONFIRM as u32, (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0);
}

pub(crate) unsafe fn save_workspace(slot: usize) {
    if !slot_ok(slot) {
        return;
    }
    remember_tabs();
    let text = app(|a| capture_window(&a.s));
    if !store_workspace(slot, &text) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let label = workspace_label(slot);
    note(tf(crate::Key::WorkspaceSaved, &[("name", &label)]));
    crate::save_session();
}

pub(crate) unsafe fn load_workspace(slot: usize) {
    if !slot_ok(slot) {
        return;
    }
    let Some(grids) = workspace_grids(slot) else {
        note(tf(crate::Key::WorkspaceEmpty, &[("name", &workspace_label(slot))]));
        return;
    };
    if workspaces_confirm() {
        let q = tf(crate::Key::ReplaceCurrentTabs, &[("name", &workspace_label(slot))]);
        let main = app(|a| a.main);
        if MessageBoxW(Some(main), &HSTRING::from(q), &HSTRING::from(tr(crate::Key::Workspaces)), MB_YESNO | MB_ICONQUESTION) != IDYES {
            return;
        }
    }
    let mut next = app(|a| a.s.clone());
    if !apply_window_grids(&mut next, &grids) {
        note(tr(crate::Key::ThisSessionCanTBeRead).to_string());
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let label = workspace_label(slot);
    let peek = app(|a| a.peek);
    if !peek.0.is_null() {
        close_peek(peek);
    }
    end_rename(false);
    install_grids(next);
    relayout();
    sync_filter_edits();
    redraw_all();
    focus_view(active_pane());
    tabs_invalidate();
    refresh_save_button();
    note(tf(crate::Key::WorkspaceLoaded, &[("name", &label)]));
    crate::save_session();
}

pub(crate) unsafe fn rename_workspace(slot: usize) {
    if !slot_ok(slot) {
        return;
    }
    let main = app(|a| a.main);
    let title = tr(crate::Key::Rename);
    let Some(text) = crate::settings::ask(main, title, &workspace_name(slot)) else { return };
    if !rename_workspace_name(slot, &text) {
        return;
    }
    crate::save_session();
}

pub(crate) unsafe fn clear_workspace(slot: usize) {
    if !slot_ok(slot) || workspace_blank(slot) {
        return;
    }
    let label = workspace_label(slot);
    clear_workspace_slot(slot);
    note(tf(crate::Key::WorkspaceCleared, &[("name", &label)]));
    crate::save_session();
}

pub(crate) unsafe fn workspace_command(id: usize) -> bool {
    if (ID_WS_SAVE..ID_WS_SAVE + SLOTS).contains(&id) {
        save_workspace(id - ID_WS_SAVE);
    } else if (ID_WS_LOAD..ID_WS_LOAD + SLOTS).contains(&id) {
        load_workspace(id - ID_WS_LOAD);
    } else if (ID_WS_RENAME..ID_WS_RENAME + SLOTS).contains(&id) {
        rename_workspace(id - ID_WS_RENAME);
    } else if (ID_WS_CLEAR..ID_WS_CLEAR + SLOTS).contains(&id) {
        clear_workspace(id - ID_WS_CLEAR);
    } else if id == ID_WS_CONFIRM {
        set_workspaces_confirm(!workspaces_confirm());
        crate::save_session();
    } else {
        return false;
    }
    true
}

/// Which slot a 1..9 range command hit. Anything else is not a slot.
pub(crate) fn workspace_digit(vk: u16) -> Option<usize> {
    (0x31..=0x39).contains(&vk).then_some((vk - 0x31) as usize)
}

/// Load replaces tabs only when the slot has a layout. The prompt is a separate, off-by-default switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoadPlan {
    Empty,
    Replace,
    Ask,
}

pub(crate) fn load_plan(has_grids: bool, confirm: bool) -> LoadPlan {
    if !has_grids {
        LoadPlan::Empty
    } else if confirm {
        LoadPlan::Ask
    } else {
        LoadPlan::Replace
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window_with_two_tabs() -> Session {
        let mut s = Session::default();
        s.set_heights(vec![2]);
        s.paths[0] = r"C:\work".into();
        s.paths[1] = r"D:\photos".into();
        s.pane_folders[0] = vec![r"C:\work".into(), r"C:\archive".into()];
        s.pane_cur[0] = 0;
        s.active = 1;
        s.anchor();
        s.tabs[0].name = "Main".into();
        s.tabs[0].locked = true;
        s.tabs[0].color = Some(0x336699);
        let mut other = s.tabs[0].clone();
        other.name = "Side".into();
        other.locked = false;
        other.color = None;
        other.session.clear();
        other.heights = vec![1];
        other.colr = vec![1.0];
        other.rowr = vec![vec![1.0]];
        other.active = 0;
        other.paths = vec![String::new(); crate::MAXP];
        other.paths[0] = r"E:\docs\b".into();
        other.pane_folders = vec![Vec::new(); crate::MAXP];
        other.pane_folders[0] = vec![r"E:\docs".into(), r"E:\docs\a".into(), r"E:\docs\b".into()];
        other.pane_cur = vec![0; crate::MAXP];
        other.pane_cur[0] = 2;
        s.tabs.push(other);
        s.tab = 0;
        s
    }

    #[test]
    fn snapshot_restores_tabs_and_leaves_settings() {
        let src = window_with_two_tabs();
        let text = capture_window(&src);
        assert!(looks_like_session(&text));
        let mut dst = Session::default();
        dst.theme = 2;
        dst.hotkey = false;
        dst.side = 180;
        dst.accent = 0x112233;
        dst.file = r"C:\old.grids".into();
        dst.paths[0] = r"C:\now".into();
        assert!(apply_window_grids(&mut dst, &text));
        assert_eq!(dst.theme, 2);
        assert!(!dst.hotkey);
        assert_eq!(dst.side, 180);
        assert_eq!(dst.accent, 0x112233);
        assert!(dst.file.is_empty());
        let tabs = dst.view_tabs();
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0].name, "Main");
        assert!(tabs[0].locked);
        assert_eq!(tabs[0].color, Some(0x336699));
        assert!(tabs[0].session.is_empty());
        assert_eq!(dst.paths[0], r"C:\work");
        assert_eq!(dst.paths[1], r"D:\photos");
        assert_eq!(dst.active, 1);
        assert_eq!(dst.pane_tab_list(0), vec![r"C:\work".to_string(), r"C:\archive".to_string()]);
        assert_eq!(tabs[1].name, "Side");
        assert_eq!(tabs[1].paths[0], r"E:\docs\b");
        assert_eq!(tabs[1].pane_folders[0], vec![r"E:\docs".to_string(), r"E:\docs\a".to_string(), r"E:\docs\b".to_string()]);
        assert_eq!(tabs[1].pane_cur[0], 2);
        assert!(!apply_window_grids(&mut dst, "not a session"));
    }

    #[test]
    fn slots_round_trip_and_the_prompt_defaults_off() {
        reset_workspaces();
        assert!(!workspaces_confirm());
        assert!(workspace_blank(0));
        assert_eq!(load_plan(false, false), LoadPlan::Empty);
        assert_eq!(load_plan(true, false), LoadPlan::Replace);
        assert_eq!(load_plan(true, true), LoadPlan::Ask);
        assert!(workspaces_json().is_empty());

        let text = capture_window(&window_with_two_tabs());
        assert!(store_workspace(2, &text));
        assert!(rename_workspace_name(2, "  Desk\twork\n"));
        assert_eq!(workspace_name(2), "Deskwork");
        assert_eq!(workspace_label(2), "Deskwork");
        assert_eq!(workspace_label(0), "1");
        rename_workspace_name(0, "Later");
        assert!(workspace_grids(0).is_none());
        assert!(!workspace_blank(0));

        let saved = workspaces_json();
        let confirm = true;
        reset_workspaces();
        assert!(workspace_grids(2).is_none());
        load_workspaces(&saved, confirm);
        assert!(workspaces_confirm());
        assert_eq!(workspace_name(2), "Deskwork");
        assert_eq!(workspace_grids(2).as_deref(), Some(text.as_str()));
        assert_eq!(workspace_name(0), "Later");
        assert!(workspace_grids(0).is_none());

        clear_workspace_slot(2);
        assert!(workspace_blank(2));
        assert!(!store_workspace(9, &text));
        assert!(!store_workspace(0, "hello"));
        let junk = vec![serde_json::json!({"slot": 0, "grids": text}), serde_json::json!({"slot": 4, "name": "Only"})];
        load_workspaces(&junk, false);
        assert!(!workspaces_confirm());
        assert!(workspace_grids(0).is_none());
        assert_eq!(workspace_name(3), "Only");
    }
}
