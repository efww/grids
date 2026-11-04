// Gear menu, the file-list menu, and the commands behind them.

use super::plan::*;
use super::run;
use crate::imports::*;
use crate::tr;

/// Gear menu. Direct commands, plus the popup parents so a later open can delete them.
/// 1123 is Show in folder (`ID_REVEAL`) and 1126 is the preview pane (`ID_PREVIEW`).
/// Those rows now sit in other parts of the same menu. `MF_BYCOMMAND` searches submenus,
/// so a shared id would check, rename, or delete the wrong row.
pub(crate) const ID_WM_NEXT: usize = 1133;
pub(crate) const ID_WM_FILES: usize = 1124;
pub(crate) const ID_WM_SAME: usize = 1134;
pub(crate) const ID_WM_POP_FOLDERS: usize = 1127;
pub(crate) const ID_WM_POP_SAME: usize = 1128;

const _: () = assert!(
    ID_WM_NEXT != crate::ID_REVEAL
        && ID_WM_SAME != crate::ID_PREVIEW
        && ID_WM_NEXT != crate::ID_WATCH
        && ID_WM_SAME != crate::ID_WATCH
        && ID_WM_FILES != crate::ID_BASKET
);
pub(crate) const ID_WM_PANE: usize = 1300; // + pane index
pub(crate) const ID_WM_SAMEP: usize = 1320; // + pane index

const ID_CTX_WM_NEXT: u32 = 0x8020;
const ID_CTX_WM_FILES: u32 = 0x8021;
const ID_CTX_WM_SAME: u32 = 0x8022;
const ID_CTX_WM_PANE: u32 = 0x8030; // + pane index
const ID_CTX_WM_SAMEP: u32 = 0x8050; // + pane index

struct Live {
    panes: Vec<(usize, String)>,
    plan: MenuPlan,
    items: Vec<(String, bool)>,
}

fn live(pane: usize, items: &[(String, bool)]) -> Live {
    let (paths, n) = crate::app(|a| (a.s.paths.clone(), a.s.panes()));
    let panes = folder_panes(&paths, n);
    let active_is = panes.iter().any(|(i, _)| *i == pane);
    let others = others_after(pane, &panes);
    let files = items.iter().filter(|(_, file)| *file).count();
    let plan = menu_plan(active_is, &others, files, items.len());
    Live { panes, plan, items: items.to_vec() }
}

fn gear_live() -> Live {
    let active = crate::active_pane();
    let items = unsafe { crate::selected_items(active) };
    live(active, &items)
}

pub(crate) unsafe fn sync_gear(menu: HMENU) {
    clear_gear(menu);
    let live = gear_live();
    let at = insert_at(menu);
    let mut pos = at;
    add_gear(&mut pos, menu, &live);
}

unsafe fn clear_gear(menu: HMENU) {
    for id in [ID_WM_NEXT, ID_WM_FILES, ID_WM_SAME, ID_WM_POP_FOLDERS, ID_WM_POP_SAME] {
        let _ = DeleteMenu(menu, id as u32, MF_BYCOMMAND);
    }
    for i in 0..crate::MAXP {
        let _ = DeleteMenu(menu, (ID_WM_PANE + i) as u32, MF_BYCOMMAND);
        let _ = DeleteMenu(menu, (ID_WM_SAMEP + i) as u32, MF_BYCOMMAND);
    }
}

/// The tools group ends at Folder watch. Rows are inserted there, before the separator
/// that leads to View. A menu without that item falls back to just before Settings.
unsafe fn insert_at(menu: HMENU) -> u32 {
    if let Some(i) = crate::menu_index(menu, crate::ID_WATCH) {
        return i + 1;
    }
    crate::menu_index(menu, crate::ID_SETTINGS).unwrap_or_else(|| GetMenuItemCount(Some(menu)).max(0) as u32)
}

unsafe fn add_gear(pos: &mut u32, menu: HMENU, live: &Live) {
    let chord = folder_chord();
    match &live.plan.folders {
        Shape::Submenu(ids) => {
            if let Some(sub) = popup_of(ids, &live.panes, ID_WM_PANE) {
                put(menu, pos, ID_WM_POP_FOLDERS, &folders_label(&chord), false, Some(sub));
            }
        }
        Shape::Command | Shape::Hidden => {
            put(menu, pos, ID_WM_NEXT, &folders_label(&chord), matches!(live.plan.folders, Shape::Hidden), None);
        }
    }
    put(menu, pos, ID_WM_FILES, &files_label(), !live.plan.two_files, None);
    match &live.plan.same_name {
        Shape::Submenu(ids) => {
            if let Some(sub) = popup_of(ids, &live.panes, ID_WM_SAMEP) {
                put(menu, pos, ID_WM_POP_SAME, &same_label(), false, Some(sub));
            }
        }
        Shape::Command | Shape::Hidden => {
            put(menu, pos, ID_WM_SAME, &same_label(), matches!(live.plan.same_name, Shape::Hidden), None);
        }
    }
}

/// The WinMerge commands a right-click can add, without a shortcut tail.
/// Folder compare and two-file compare are the two named WinMerge items. Same-name compare
/// is the third command in this block and follows the same on/off switch.
pub(crate) fn context_labels() -> Vec<String> {
    vec![folders_name(), files_label(), same_label()]
}

fn folders_name() -> String {
    tr(crate::Key::CompareFoldersInWinmerge).to_string()
}

/// App items next to Favourites. Only the commands that fit this click are added.
/// Hidden unless Settings has explicitly turned WinMerge on for this click.
pub(crate) unsafe fn insert_ctx(menu: HMENU, at: u32, pane: usize, items: &[(String, bool)]) {
    // Empty items are a background click. A selection is a file, a folder, or both.
    let targets = if items.is_empty() {
        crate::shell::ctxmenu::BACKGROUND
    } else {
        crate::shell::ctxmenu::click_targets(false, items)
    };
    if !crate::shell::ctxmenu::winmerge_menu_on(targets) {
        return;
    }
    let live = live(pane, items);
    if matches!(live.plan.folders, Shape::Hidden) && !live.plan.two_files && matches!(live.plan.same_name, Shape::Hidden) {
        return;
    }
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    let mut pos = at.min(n);
    let chord = folder_chord();
    match &live.plan.folders {
        Shape::Submenu(ids) => {
            if let Some(sub) = popup_of(ids, &live.panes, ID_CTX_WM_PANE as usize) {
                let _ = InsertMenuW(menu, pos, MF_BYPOSITION | MF_POPUP | MF_STRING, sub.0 as usize, &HSTRING::from(folders_label(&chord)));
                pos += 1;
            }
        }
        Shape::Command => {
            let _ = InsertMenuW(menu, pos, MF_BYPOSITION | MF_STRING, ID_CTX_WM_NEXT as usize, &HSTRING::from(folders_label(&chord)));
            pos += 1;
        }
        Shape::Hidden => {}
    }
    if live.plan.two_files {
        let _ = InsertMenuW(menu, pos, MF_BYPOSITION | MF_STRING, ID_CTX_WM_FILES as usize, &HSTRING::from(files_label()));
        pos += 1;
    }
    match &live.plan.same_name {
        Shape::Submenu(ids) => {
            if let Some(sub) = popup_of(ids, &live.panes, ID_CTX_WM_SAMEP as usize) {
                let _ = InsertMenuW(menu, pos, MF_BYPOSITION | MF_POPUP | MF_STRING, sub.0 as usize, &HSTRING::from(same_label()));
            }
        }
        Shape::Command => {
            let _ = InsertMenuW(menu, pos, MF_BYPOSITION | MF_STRING, ID_CTX_WM_SAME as usize, &HSTRING::from(same_label()));
        }
        Shape::Hidden => {}
    }
}

unsafe fn popup_of(ids: &[usize], panes: &[(usize, String)], base: usize) -> Option<HMENU> {
    let sub = CreatePopupMenu().ok()?;
    for idx in ids {
        let path = panes.iter().find(|(i, _)| *i == *idx).map(|(_, p)| p.as_str()).unwrap_or("");
        let _ = AppendMenuW(sub, MF_STRING, base + idx, &HSTRING::from(pane_label(*idx, path)));
    }
    Some(sub)
}

unsafe fn put(menu: HMENU, pos: &mut u32, id: usize, label: &str, gray: bool, sub: Option<HMENU>) {
    let mut wide: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
    let mut mask = MIIM_ID | MIIM_FTYPE | MIIM_STRING | MIIM_STATE;
    if sub.is_some() {
        mask |= MIIM_SUBMENU;
    }
    let info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: mask,
        fType: MFT_STRING,
        fState: if gray { MFS_GRAYED } else { MENU_ITEM_STATE(0) },
        wID: id as u32,
        hSubMenu: sub.unwrap_or_default(),
        dwTypeData: PWSTR(wide.as_mut_ptr()),
        cch: wide.len().saturating_sub(1) as u32,
        ..Default::default()
    };
    if InsertMenuItemW(menu, *pos, true, &info).is_ok() {
        *pos += 1;
    }
}

pub(crate) unsafe fn gear(owner: HWND, id: usize) -> bool {
    let pane = crate::active_pane();
    if id == ID_WM_NEXT || id == ID_WM_POP_FOLDERS {
        run_folders_in(owner, pane, None);
    } else if (ID_WM_PANE..ID_WM_PANE + crate::MAXP).contains(&id) {
        run_folders_in(owner, pane, Some(id - ID_WM_PANE));
    } else if id == ID_WM_FILES {
        run_two_files(owner, &gear_live().items);
    } else if id == ID_WM_SAME || id == ID_WM_POP_SAME {
        run_same(owner, pane, None, &gear_live().items);
    } else if (ID_WM_SAMEP..ID_WM_SAMEP + crate::MAXP).contains(&id) {
        run_same(owner, pane, Some(id - ID_WM_SAMEP), &gear_live().items);
    } else {
        return false;
    }
    true
}

pub(crate) unsafe fn ctx_command(owner: HWND, cmd: i32, pane: usize, items: &[(String, bool)]) -> bool {
    let id = cmd as u32;
    if id == ID_CTX_WM_NEXT {
        run_folders_in(owner, pane, None);
    } else if (ID_CTX_WM_PANE..ID_CTX_WM_PANE + crate::MAXP as u32).contains(&id) {
        run_folders_in(owner, pane, Some((id - ID_CTX_WM_PANE) as usize));
    } else if id == ID_CTX_WM_FILES {
        run_two_files(owner, items);
    } else if id == ID_CTX_WM_SAME {
        run_same(owner, pane, None, items);
    } else if (ID_CTX_WM_SAMEP..ID_CTX_WM_SAMEP + crate::MAXP as u32).contains(&id) {
        run_same(owner, pane, Some((id - ID_CTX_WM_SAMEP) as usize), items);
    } else {
        return false;
    }
    true
}

pub(crate) unsafe fn run_folders(owner: HWND, chosen: Option<usize>) {
    run_folders_in(owner, crate::active_pane(), chosen);
}

unsafe fn run_folders_in(owner: HWND, active: usize, chosen: Option<usize>) {
    let (paths, n) = crate::app(|a| (a.s.paths.clone(), a.s.panes()));
    match folder_target(active, &folder_panes(&paths, n), chosen) {
        FolderTarget::Ready { left, right, .. } => {
            let (l, r) = sides_folders(&left, &right);
            open_compare(owner, Mode::Folders, &l, &r);
        }
        FolderTarget::NotAFolder => run::prompt_need_folder(owner),
        FolderTarget::NeedAnother => run::prompt_need_two(owner),
    }
}

unsafe fn run_two_files(owner: HWND, items: &[(String, bool)]) {
    match picked_files(items) {
        FilePick::Two(a, b) => {
            let (l, r) = sides_files(&a, &b);
            open_compare(owner, Mode::Files, &l, &r);
        }
        _ => run::prompt_need_two_files(owner),
    }
}

unsafe fn run_same(owner: HWND, active: usize, chosen: Option<usize>, items: &[(String, bool)]) {
    let FilePick::One(file) = picked_files(items) else {
        run::prompt_need_file(owner);
        return;
    };
    let (paths, n) = crate::app(|a| (a.s.paths.clone(), a.s.panes()));
    let active_path = paths.get(active).cloned().unwrap_or_default();
    match folder_target(active, &folder_panes(&paths, n), chosen) {
        FolderTarget::Ready { right, .. } => {
            let left = Side { path: file.clone(), desc: folder_label(&active_path) };
            let other = Side { path: same_named(&file, &right), desc: folder_label(&right) };
            open_compare(owner, Mode::Files, &left, &other);
        }
        _ => run::prompt_need_two(owner),
    }
}

unsafe fn open_compare(owner: HWND, mode: Mode, left: &Side, right: &Side) {
    let custom = crate::app(|a| a.s.winmerge_path.clone());
    let found = run::status_now(&custom);
    let Some(exe) = found.exe().map(str::to_string) else {
        run::prompt_missing(owner);
        return;
    };
    let params = command_params(mode, left, right);
    if !run::launch(&exe, &params) {
        if Path::new(&exe).is_file() {
            run::prompt_failed(owner);
        } else {
            run::forget_status();
            run::prompt_missing(owner);
        }
    }
}

fn folder_chord() -> String {
    crate::keys::index("file.winmerge").map(crate::keys::chord_text).unwrap_or_default()
}

fn folders_label(chord: &str) -> String {
    let name = folders_name();
    if chord.is_empty() {
        name.to_string()
    } else {
        format!("{name}\t{chord}")
    }
}

fn files_label() -> String {
    tr(crate::Key::CompareTwoFilesInWinmerge).to_string()
}

fn same_label() -> String {
    tr(crate::Key::CompareWithSameNameInOtherPane).to_string()
}

fn pane_label(index: usize, path: &str) -> String {
    format!("{} · {}", index + 1, folder_label(path))
}
