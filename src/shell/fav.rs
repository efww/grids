use crate::imports::*;
use crate::app::*;
use crate::ui::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;
use crate::settings;

// ---------- favourites: ☆+ in each pane header; Ctrl+B opens the same list ----------

pub(crate) unsafe fn repaint_fav_buttons() {
    for h in app(|a| a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>()) {
        if let Ok(b) = GetDlgItem(Some(h), ID_FAV) {
            _ = InvalidateRect(Some(b), None, false);
        }
    }
}

pub(crate) unsafe fn set_favs(paths: &[String], add: bool) {
    let items: Vec<(String, bool)> = paths.iter().cloned().map(|p| (p, false)).collect();
    set_entries(&items, add);
}

/// Add or remove folders and files on the favourites tree in the left panel.
/// Files are marked so a double-click opens them. The folder tree is left alone.
pub(crate) unsafe fn set_entries(items: &[(String, bool)], add: bool) {
    if items.is_empty() {
        return;
    }
    app(|a| {
        a.s.ensure_fav_tree();
        for (path, file) in items {
            if add {
                add_item(&mut a.s.tree, path, *file);
            } else {
                remove_path(&mut a.s.tree, path);
            }
        }
        a.s.sync_favs();
        a.fav_hot = None;
    });
    favs_changed();
}

/// The tree changed. Every window, the Ctrl+B list, and the settings page read it.
pub(crate) unsafe fn favs_changed() {
    publish_shared();
    let (pop, dpi, n) = app(|a| (a.fav_popup, a.dpi, visible_rows(&a.s.tree).len()));
    if !pop.0.is_null() && IsWindow(Some(pop)).as_bool() {
        let g = fav_geo(n.max(1), dpi);
        _ = SetWindowPos(pop, None, 0, 0, g.w, g.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        _ = InvalidateRect(Some(pop), None, false);
    }
    settings::refresh();
    refresh_fav_manager();
    across_windows(|| {
        // The rows under the pointer may have moved. Hide until the pointer moves.
        dismiss_side_fav_tip();
        repaint_fav_buttons();
        clamp_side_scroll();
        let h = app(|a| a.side_hwnd);
        if !h.0.is_null() {
            _ = InvalidateRect(Some(h), None, false);
        }
    });
    note_save();
}

fn server_of(row: &FavRow) -> Option<crate::session::ServerLink> {
    if row.in_tree() && !row.id.is_empty() {
        if let Some(link) = app(|a| node_ref(&a.s.tree, &row.id).and_then(|n| n.server.clone())) {
            return Some(link);
        }
    }
    app(|a| server_by_path(&a.s.tree, &row.path))
}

/// Servers on this node, including every server inside a group, before it is deleted.
pub(crate) fn servers_at(id: &[usize]) -> Vec<crate::session::ServerLink> {
    app(|a| node_ref(&a.s.tree, id).map(servers_in).unwrap_or_default())
}

/// Unmount after the node is gone, when no remaining favourite uses the same mount.
pub(crate) fn release_servers(links: Vec<crate::session::ServerLink>) {
    for link in links {
        if !app(|a| crate::remote::still_used(&a.s.tree, &link)) {
            crate::remote::unmount_link(&link);
        }
    }
}

/// Connect, mounting first when the favourite is a server. SFTP/FTP ask once for rclone and WinFsp.
pub(crate) unsafe fn launch_server(owner: HWND, link: crate::session::ServerLink, open: Option<String>) {
    if crate::remote::needs_rclone(&link.kind) {
        let rclone = crate::remote::rclone_ok();
        let winfsp = crate::remote::winfsp_ok();
        if !rclone || !winfsp {
            let asked = app(|a| a.s.remote_asked);
            if crate::remote::should_ask_tools(asked, rclone, winfsp) {
                app(|a| a.s.remote_asked = true);
                save_session();
                let q = tr(crate::Key::InstallRcloneAndWinfspToConnectToSftp);
                if MessageBoxW(Some(owner), &HSTRING::from(q), w!("Maguro"), MB_YESNO | MB_ICONQUESTION) == IDYES {
                    crate::remote::install_then_connect(owner.0 as isize, link, open);
                }
                return;
            }
            let q = tr(crate::Key::RcloneOrWinfspIsNotInstalled);
            let _ = MessageBoxW(Some(owner), &HSTRING::from(q), w!("Maguro"), MB_ICONWARNING);
            return;
        }
    }
    crate::remote::begin_connect(owner.0 as isize, link, open);
}

pub(crate) unsafe fn open_typed_remote(parsed: crate::remote::Parsed) {
    let owner = app(|a| a.main);
    let matched = app(|a| crate::remote::best_match(&a.s.tree, &parsed));
    if let Some(link) = matched {
        launch_server(owner, link, None);
        return;
    }
    open_server_dialog(owner, &[], None, Some(parsed));
}

pub(crate) unsafe fn remote_arrived() {
    let owner = app(|a| a.main);
    for done in crate::remote::take_done() {
        match done {
            crate::remote::Done::Quiet => {}
            crate::remote::Done::NeedTools => {
                let q = tr(crate::Key::RcloneOrWinfspIsNotInstalled);
                let _ = MessageBoxW(Some(owner), &HSTRING::from(q), w!("Maguro"), MB_ICONWARNING);
            }
            crate::remote::Done::Fail(detail) => {
                let base = tr(crate::Key::CouldNotConnectToTheServer);
                let text = if detail.trim().is_empty() { base.to_string() } else { format!("{base}\n\n{detail}") };
                let _ = MessageBoxW(Some(owner), &HSTRING::from(text), w!("Maguro"), MB_ICONWARNING);
            }
            crate::remote::Done::Open(path) => {
                let i = active_pane();
                if !navigate(i, &path) {
                    let q = tr(crate::Key::CouldNotConnectToTheServer);
                    let _ = MessageBoxW(Some(owner), &HSTRING::from(q), w!("Maguro"), MB_ICONWARNING);
                } else if !net_failed(i) {
                    focus_view(i);
                }
            }
        }
    }
}

/// The settings editor and the favourites window both write through here.
pub(crate) unsafe fn commit_favs() {
    favs_changed();
    save_session();
}

#[allow(dead_code)] // path-only entry; the panel passes the row through open_fav_row
pub(crate) unsafe fn open_fav_target(path: &str, file: bool) {
    open_fav_row(&FavRow {
        id: Vec::new(),
        depth: 0,
        group: false,
        file,
        open: false,
        name: String::new(),
        path: path.to_string(),
        kind: RowKind::Tree,
    });
}

/// Open a favourites row in the active pane. A local path that is already gone does not navigate:
/// the current folder stays, the row is dimmed, and a notice offers remove or a new location.
/// A UNC or mapped path is resolved in the background. If it does not answer, the pane shows
/// the network error view. The favourite is not removed.
pub(crate) unsafe fn open_fav_row(row: &FavRow) {
    if row.group || row.path.is_empty() {
        return;
    }
    if let Some(link) = server_of(row) {
        launch_server(app(|a| a.main), link, None);
        return;
    }
    let unc = unc_path(&row.path);
    // Mapped drives are remote only when the path is not already a UNC. share_path stats a drive root, never the share itself.
    let place = path_place(unc, !unc && share_path(&row.path), is_fs(&row.path));
    let hit = if place == PathPlace::Local { Some(probe_local(&row.path)) } else { None };
    match fav_act(place, hit) {
        FavAct::Missing => {
            mark_missing(&row.path);
            show_fav_missing(row);
        }
        FavAct::Share => {
            let i = active_pane();
            if !open_share(i, &row.path, row.file) {
                _ = MessageBeep(MB_ICONWARNING);
            } else if !row.file && !net_failed(i) {
                focus_view(i);
            }
        }
        FavAct::Open => open_present(&row.path, row.file),
    }
}

unsafe fn open_present(path: &str, file: bool) {
    // A shortcut to a folder would ShellExecute, and that verb is us while the setting is on.
    // The link is loaded only after the local existence check above, so a missing favourite
    // still stays on the current pane. A UNC link is a share and is resolved off this thread.
    if crate::folderapp::shown_on() && crate::folderapp::link_file(path) {
        if let Some(dir) = folder_link_target(path) {
            let i = active_pane();
            if !navigate(i, &dir) {
                _ = MessageBeep(MB_ICONWARNING);
                return;
            }
            if !net_failed(i) {
                focus_view(i);
            }
            return;
        }
    }
    if file {
        if shell_open(path) {
            return;
        }
        if let Some(parent) = parent_of(path) {
            let i = active_pane();
            if navigate(i, &parent) {
                if !net_failed(i) {
                    focus_view(i);
                }
            } else {
                _ = MessageBeep(MB_ICONWARNING);
            }
        } else {
            _ = MessageBeep(MB_ICONWARNING);
        }
        return;
    }
    let i = active_pane();
    if !navigate(i, path) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    if !net_failed(i) {
        focus_view(i);
    }
}

/// Open / Explore / our verb on one folder browses this pane while Maguro is the
/// folder handler. A file shortcut is not taken, so the verb still opens it.
/// Returns true when the command must not reach the shell (that launch is a new tab).
unsafe fn claim_folder_open(pane: usize, verb: &str, items: &[(String, bool)], default_item: bool) -> bool {
    let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
    let middle = GetKeyState(VK_MBUTTON.0 as i32) < 0;
    // New-window verbs and a multi-selection Open are meant to become tabs. Record that
    // before the verb runs, or the handoff would treat the child process as a slipped double-click.
    let opening = crate::folderapp::open_verb(verb) || (verb.trim().is_empty() && default_item);
    let new_window = matches!(verb.trim().to_ascii_lowercase().as_str(), "opennew" | "opennewwindow");
    let wants_tab = ctrl || middle || new_window || (items.len() > 1 && opening);
    crate::folderapp::note_activate(app(|a| a.main.0 as isize), pane, wants_tab);
    let file = items.len() == 1 && items[0].1;
    let link = items.len() == 1 && crate::folderapp::link_file(&items[0].0);
    if crate::folderapp::menu_open_item(crate::folderapp::shown_on(), ctrl, middle, verb, items.len(), file, link, default_item) != crate::folderapp::OpenWhere::Pane {
        return false;
    }
    let path = &items[0].0;
    let dest = if link { folder_link_target(path) } else { Some(path.clone()) };
    let Some(dest) = dest else { return false };
    if !navigate(pane, &dest) {
        _ = MessageBeep(MB_ICONWARNING);
    }
    focus_view(pane);
    true
}

fn shell_open(path: &str) -> bool {
    unsafe {
        // A value above 32 is success. Smaller values are the old error codes.
        let h = ShellExecuteW(None, w!("open"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        h.0 as isize > 32
    }
}

/// Parent folder of a file path. `C:\a\b.txt` → `C:\a`, `C:\b.txt` → `C:\`.
pub(crate) fn parent_of(path: &str) -> Option<String> {
    let t = path.trim_end_matches(['\\', '/']);
    let i = t.rfind(['\\', '/'])?;
    let parent = t.get(..i)?;
    if parent.is_empty() {
        return None;
    }
    if parent.ends_with(':') {
        Some(format!("{parent}\\"))
    } else {
        Some(parent.to_string())
    }
}

pub(crate) unsafe fn toggle_fav(i: usize) {
    let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    set_favs(std::slice::from_ref(&path), !app(|a| a.s.is_fav(&path)));
}

/// Every path already a favourite → the menu removes them; otherwise it adds the rest.
pub(crate) fn fav_removes(paths: &[&str], is_fav: impl Fn(&str) -> bool) -> bool {
    !paths.is_empty() && paths.iter().all(|p| is_fav(p))
}

pub(crate) fn fav_menu_label(paths: &[String]) -> &'static str {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    fav_action_label(fav_removes(&refs, |p| app(|a| a.s.is_fav(p))))
}

/// Maguro's own row on the shell menu. Explorer already shows its Home favourite
/// (`お気に入りに追加`), so this label names Maguro in every UI language.
pub(crate) fn fav_action_label(remove: bool) -> &'static str {
    if remove {
        tr(crate::Key::RemoveFromMaguroFavorites)
    } else {
        tr(crate::Key::AddMaguroFavorites)
    }
}

/// Menu index the favourites command sits above: the properties item, or the last item.
pub(crate) fn fav_anchor(properties: Option<u32>, count: i32) -> u32 {
    if count <= 0 {
        0
    } else {
        properties.filter(|&i| (i as i32) < count).unwrap_or(count as u32 - 1)
    }
}

pub(crate) unsafe fn cmd_verb(cm: &IContextMenu, offset: usize) -> String {
    let mut buf = [0u16; 80];
    if cm.GetCommandString(offset, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32).is_err() {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// One GetCommandString pass for observe / properties / fav_anchor / fast paste / Excel read-only / WM_INITMENUPOPUP.
#[derive(Clone, Default)]
struct VerbIndex {
    by_id: std::collections::HashMap<u32, String>,
}

thread_local! {
    static VERB_INDEX: std::cell::RefCell<Option<VerbIndex>> = const { std::cell::RefCell::new(None) };
}

impl VerbIndex {
    unsafe fn build(menu: HMENU, cm: &IContextMenu, id_first: u32, id_last: u32) -> Self {
        let mut by_id = std::collections::HashMap::new();
        let n = GetMenuItemCount(Some(menu)).max(0) as u32;
        for i in 0..n {
            if menu_sep(menu, i) {
                continue;
            }
            let id = GetMenuItemID(menu, i as i32);
            if id == u32::MAX || id < id_first || id >= id_last {
                continue;
            }
            let verb = cmd_verb(cm, (id - id_first) as usize);
            if !verb.is_empty() {
                by_id.insert(id, verb);
            }
        }
        Self { by_id }
    }

    fn verb(&self, id: u32) -> &str {
        self.by_id.get(&id).map(String::as_str).unwrap_or("")
    }

    unsafe fn has_verb(&self, menu: HMENU, want: &str) -> bool {
        let n = GetMenuItemCount(Some(menu)).max(0) as u32;
        (0..n).any(|i| {
            if menu_sep(menu, i) {
                return false;
            }
            let id = GetMenuItemID(menu, i as i32);
            self.verb(id).trim().eq_ignore_ascii_case(want)
        })
    }

    unsafe fn find_pos_after(&self, menu: HMENU, want: &str) -> Option<u32> {
        let n = GetMenuItemCount(Some(menu)).max(0) as u32;
        for i in 0..n {
            if menu_sep(menu, i) {
                continue;
            }
            let id = GetMenuItemID(menu, i as i32);
            if self.verb(id).trim().eq_ignore_ascii_case(want) {
                return Some(i + 1);
            }
        }
        None
    }

    unsafe fn fav_anchor(&self, menu: HMENU, start: u32) -> u32 {
        let n = GetMenuItemCount(Some(menu));
        if n <= 0 {
            return 0;
        }
        let mut properties = None;
        for i in start.min(n as u32)..n as u32 {
            if menu_sep(menu, i) {
                continue;
            }
            let id = GetMenuItemID(menu, i as i32);
            if self.verb(id).trim().eq_ignore_ascii_case("properties") {
                properties = Some(i);
                break;
            }
        }
        fav_anchor(properties, n)
    }
}

unsafe fn insert_fast_paste_idx(menu: HMENU, verbs: &VerbIndex) {
    if let Some(i) = menu_index(menu, ID_CTX_FAST as usize) {
        let _ = DeleteMenu(menu, i, MF_BYPOSITION);
    }
    let Some(at) = verbs.find_pos_after(menu, "paste") else { return };
    let _ = InsertMenuW(menu, at, MF_STRING | MF_BYPOSITION, ID_CTX_FAST as usize, &HSTRING::from(tr(crate::Key::FastPaste)));
}

/// Index of the "properties" verb. Shell command ids are `id_first + offset`; separators and submenus have no id.
pub(crate) unsafe fn fav_anchor_pos(menu: HMENU, cm: &IContextMenu, id_first: u32, id_last: u32, start: u32) -> u32 {
    let n = GetMenuItemCount(Some(menu));
    if n <= 0 {
        return 0;
    }
    let mut properties = None;
    for i in start.min(n as u32)..n as u32 {
        if menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        if id == u32::MAX || id < id_first || id >= id_last {
            continue;
        }
        if cmd_verb(cm, (id - id_first) as usize).trim().eq_ignore_ascii_case("properties") {
            properties = Some(i);
            break;
        }
    }
    fav_anchor(properties, n)
}

/// Puts Fast paste directly under the shell Paste row. No row when Paste is absent.
unsafe fn insert_fast_paste(menu: HMENU, cm: &IContextMenu) {
    if let Some(i) = menu_index(menu, ID_CTX_FAST as usize) {
        let _ = DeleteMenu(menu, i, MF_BYPOSITION);
    }
    let n = GetMenuItemCount(Some(menu));
    if n <= 0 {
        return;
    }
    let mut at = None;
    for i in 0..n as u32 {
        if menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        if id == u32::MAX || id < 1 || id >= ID_CTX_FAV {
            continue;
        }
        if cmd_verb(cm, (id - 1) as usize).trim().eq_ignore_ascii_case("paste") {
            at = Some(i + 1);
            break;
        }
    }
    let Some(at) = at else { return };
    let _ = InsertMenuW(menu, at, MF_STRING | MF_BYPOSITION, ID_CTX_FAST as usize, &HSTRING::from(tr(crate::Key::FastPaste)));
}

/// Directly under the top shell Open row. A csv file selected with an xlsx file
/// often has no shared Open verb, so the row goes at the top instead.
/// Hidden unless every selected file is an Excel workbook or CSV.
unsafe fn insert_excel_idx(menu: HMENU, verbs: &VerbIndex) {
    if let Some(i) = menu_index(menu, ID_CTX_EXCEL as usize) {
        let _ = DeleteMenu(menu, i, MF_BYPOSITION);
    }
    if !super::excelro::armed() {
        return;
    }
    let at = verbs.find_pos_after(menu, "open").unwrap_or(0);
    let _ = InsertMenuW(menu, at, MF_STRING | MF_BYPOSITION, ID_CTX_EXCEL as usize, &HSTRING::from(tr(crate::Key::OpenReadOnly)));
}

/// Same row when the verb index was not kept. Shell ids stop below the app range.
unsafe fn insert_excel(menu: HMENU, cm: &IContextMenu) {
    if let Some(i) = menu_index(menu, ID_CTX_EXCEL as usize) {
        let _ = DeleteMenu(menu, i, MF_BYPOSITION);
    }
    if !super::excelro::armed() {
        return;
    }
    let n = GetMenuItemCount(Some(menu));
    if n <= 0 {
        return;
    }
    let mut at = None;
    for i in 0..n as u32 {
        if menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        if id == u32::MAX || id < 1 || id >= ID_CTX_FAV {
            continue;
        }
        if cmd_verb(cm, (id - 1) as usize).trim().eq_ignore_ascii_case("open") {
            at = Some(i + 1);
            break;
        }
    }
    let at = at.unwrap_or(0);
    let _ = InsertMenuW(menu, at, MF_STRING | MF_BYPOSITION, ID_CTX_EXCEL as usize, &HSTRING::from(tr(crate::Key::OpenReadOnly)));
}

fn fast_paste_dest(pane: usize, background: bool, items: &[(String, bool)]) -> String {
    if !background && items.len() == 1 && !items[0].1 && is_fs(&items[0].0) {
        return items[0].0.clone();
    }
    app(|a| a.s.paths.get(pane).cloned().unwrap_or_default())
}

/// Separators often have a zero id, so GetMenuItemID cannot tell them from a command. MFT_SEPARATOR can.
pub(crate) unsafe fn menu_sep(menu: HMENU, pos: u32) -> bool {
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_FTYPE,
        ..Default::default()
    };
    GetMenuItemInfoW(menu, pos, true, &mut info).is_ok() && info.fType.0 & MFT_SEPARATOR.0 != 0
}

/// Verb recorded for a shell command id while this menu was built. Empty when
/// the id is Maguro's own row or the index was not built.
pub(crate) fn indexed_verb(id: u32) -> String {
    VERB_INDEX.with(|v| v.borrow().as_ref().map(|idx| idx.verb(id).to_string()).unwrap_or_default())
}

pub(crate) unsafe fn menu_index(menu: HMENU, id: usize) -> Option<u32> {
    let n = GetMenuItemCount(Some(menu)).max(0);
    (0..n).find(|&i| GetMenuItemID(menu, i) as usize == id).map(|i| i as u32)
}

/// Exactly one separator above the favourites command, and none between it and Properties.
/// Run after the shell menu is filled (and again on WM_INITMENUPOPUP, once merged items exist).
pub(crate) unsafe fn tidy_fav_item(menu: HMENU, id: usize) {
    if id == usize::MAX {
        return;
    }
    loop {
        let Some(fav) = menu_index(menu, id) else { return };
        let n = GetMenuItemCount(Some(menu)).max(0) as u32;
        if fav + 1 >= n || !menu_sep(menu, fav + 1) {
            break;
        }
        if DeleteMenu(menu, fav + 1, MF_BYPOSITION).is_err() {
            break;
        }
    }
    loop {
        let Some(fav) = menu_index(menu, id) else { return };
        let anchor = pair_top(menu, fav);
        if anchor < 2 || !menu_sep(menu, anchor - 1) || !menu_sep(menu, anchor - 2) {
            break;
        }
        if DeleteMenu(menu, anchor - 1, MF_BYPOSITION).is_err() {
            break;
        }
    }
    let Some(fav) = menu_index(menu, id) else { return };
    // Batch rename is the row directly above favourites. The separator stays above that pair.
    let anchor = pair_top(menu, fav);
    if anchor > 0 && !menu_sep(menu, anchor - 1) {
        _ = InsertMenuW(menu, anchor, MF_SEPARATOR | MF_BYPOSITION, 0, PCWSTR::null());
    }
}

/// Favourites, or the app rows sitting directly above it (batch rename, who is using).
unsafe fn pair_top(menu: HMENU, fav: u32) -> u32 {
    let mut top = fav;
    while top > 0 {
        let id = GetMenuItemID(menu, (top - 1) as i32) as u32;
        if id == ID_CTX_BATCH || id == ID_CTX_WHO {
            top -= 1;
        } else {
            break;
        }
    }
    top
}

/// Favourites command at `at` (directly above Properties, or the last item). The separator is decided afterwards.
pub(crate) unsafe fn insert_fav(menu: HMENU, at: u32, id: usize, label: &str) {
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    let label = HSTRING::from(label);
    if n == 0 {
        _ = AppendMenuW(menu, MF_STRING, id, &label);
        return;
    }
    _ = InsertMenuW(menu, at.min(n - 1), MF_STRING | MF_BYPOSITION, id, &label);
    tidy_fav_item(menu, id);
}

pub(crate) fn wnd_class(h: HWND) -> String {
    let mut c = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut c) } as usize;
    String::from_utf16_lossy(&c[..n])
}

// The real Explorer menu is built by the shell. These are the windows that show it; a header
// or an in-place rename box is left alone.
pub(crate) fn ctx_surface(h: HWND) -> bool {
    matches!(wnd_class(h).as_str(), "SHELLDLL_DefView" | "DirectUIHWND" | "SysListView32")
}

pub(crate) const CTX_SUB: usize = 4;
pub(crate) const ID_CTX_FAV: u32 = 0x8000; // outside the id range handed to IContextMenu::QueryContextMenu
pub(crate) const ID_CTX_PROPS: u32 = 0x8001; // folder Properties, added when the background menu has none (Windows 10)
pub(crate) const ID_CTX_BATCH: u32 = 0x8002; // batch rename, also outside the shell id range
pub(crate) const ID_CTX_FAST: u32 = 0x8003; // Fast paste, also outside the shell id range
pub(crate) const ID_CTX_WHO: u32 = 0x8004; // who is using this file, also outside the shell id range
pub(crate) const ID_CTX_EXCEL: u32 = 0x8005; // open Excel read-only, also outside the shell id range

pub(crate) struct MenuBridge {
    pub(crate) cm3: Option<IContextMenu3>,
    pub(crate) cm2: Option<IContextMenu2>,
}

thread_local! {
    static MENU_BRIDGE: RefCell<Option<MenuBridge>> = const { RefCell::new(None) };
    static CTX_MENU: Cell<isize> = const { Cell::new(0) }; // top HMENU; separator cleanup also walks its submenus
    static CTX_BACKGROUND: Cell<bool> = const { Cell::new(false) };
    static WHO_PATH: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub(crate) static CTX_OPEN: AtomicBool = AtomicBool::new(false);

pub(crate) struct ClearBridge;
impl Drop for ClearBridge {
    fn drop(&mut self) {
        MENU_BRIDGE.with(|b| *b.borrow_mut() = None);
    VERB_INDEX.with(|v| *v.borrow_mut() = None);
        CTX_MENU.with(|m| m.set(0));
        CTX_BACKGROUND.with(|c| c.set(false));
        CTX_OPEN.store(false, Ordering::Release);
    }
}

pub(crate) unsafe fn hook_ctx_hwnd(h: HWND, i: usize) {
    if !h.0.is_null() && ctx_surface(h) {
        _ = SetWindowSubclass(h, Some(ctx_proc), CTX_SUB, i);
    }
}

pub(crate) unsafe fn hook_ctx(i: usize) {
    let Some(sv) = view_of::<IShellView>(i) else { return };
    let Ok(view) = sv.GetWindow() else { return };
    hook_ctx_hwnd(view, i);
    let mut after = None;
    loop {
        let Ok(c) = FindWindowExW(Some(view), after, PCWSTR::null(), PCWSTR::null()) else { break };
        if c.0.is_null() {
            break;
        }
        hook_ctx_hwnd(c, i);
        let mut inner = None;
        loop {
            let Ok(g) = FindWindowExW(Some(c), inner, PCWSTR::null(), PCWSTR::null()) else { break };
            if g.0.is_null() {
                break;
            }
            hook_ctx_hwnd(g, i);
            inner = Some(g);
        }
        after = Some(c);
    }
}

#[repr(C)]
pub(crate) struct LvHit {
    pub(crate) pt: POINT,
    pub(crate) flags: u32,
    pub(crate) item: i32,
    pub(crate) sub: i32,
    pub(crate) group: i32,
}

// Right-click on a row the view hasn't selected yet: select it, unless it is already part of
// a multi-selection (Explorer keeps that selection).
pub(crate) unsafe fn select_clicked(i: usize, screen: POINT) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let lv = view_list(&fv);
    // Win10's items view has no LVM_HITTEST. The accessible row name is the shell display name.
    if !is_sys_list(lv) {
        select_accessible_row(i, screen);
        return;
    }
    let mut pt = screen;
    if !ScreenToClient(lv, &mut pt).as_bool() {
        return;
    }
    let mut hit = LvHit { pt, flags: 0, item: -1, sub: 0, group: 0 };
    let idx = SendMessageW(lv, 0x1012, None, Some(LPARAM(&mut hit as *mut _ as isize))).0 as i32; // LVM_HITTEST
    if idx < 0 {
        return;
    }
    let Ok(pidl) = fv.Item(idx) else { return };
    let selected = fv.GetSelectionState(pidl).unwrap_or(0) & SVSI_SELECT.0 as u32 != 0;
    CoTaskMemFree(Some(pidl as _));
    if !selected {
        _ = fv.SelectItem(idx, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0) as u32);
    }
}

pub(crate) unsafe fn selection_pidls(i: usize) -> Vec<*mut ITEMIDLIST> {
    let Some(fv) = view_of::<IFolderView2>(i) else { return Vec::new() };
    let Ok(en) = fv.Items::<IEnumIDList>(SVGIO_SELECTION) else { return Vec::new() };
    let mut out = Vec::new();
    loop {
        let mut slot = [std::ptr::null_mut()];
        let mut n = 0u32;
        let hr = en.Next(&mut slot, Some(&mut n));
        if !hr.is_ok() || n == 0 || slot[0].is_null() || out.len() == 2000 {
            break;
        }
        out.push(slot[0]);
    }
    out
}

/// Selected folders and files. The bool is true for a file.
pub(crate) unsafe fn selected_items(i: usize) -> Vec<(String, bool)> {
    let Some(fv) = view_of::<IFolderView2>(i) else { return Vec::new() };
    let Ok(arr) = fv.GetSelection(false) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0).min(500);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { continue };
        // A failed attribute read is not a file. Leaving it unknown keeps Open from claiming it;
        // the handoff still browses this pane if the verb launches us.
        let file = item_is_container(&it) != Some(true);
        if let Ok(p) = it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) {
            let p = pwstr(p);
            if !p.is_empty() {
                out.push((p, file));
            }
        }
    }
    out
}

pub(crate) unsafe fn forward_menu(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    MENU_BRIDGE.with(|b| {
        let b = b.borrow();
        let b = b.as_ref()?;
        unsafe {
            if let Some(cm3) = &b.cm3 {
                let mut lr = LRESULT(0);
                if cm3.HandleMenuMsg2(msg, wp, lp, Some(&mut lr)).is_ok() {
                    return Some(lr);
                }
            }
            if let Some(cm2) = &b.cm2 {
                if cm2.HandleMenuMsg(msg, wp, lp).is_ok() {
                    return Some(LRESULT(0));
                }
            }
        }
        None
    })
}

// Shell menu for the pane. Favourites sits directly above Properties. Batch rename sits
// directly above favourites. Returns false when the shell menu could not be built.
pub(crate) unsafe fn show_ctx(i: usize, pt: POINT) -> bool {
    if CTX_OPEN.swap(true, Ordering::AcqRel) {
        return true;
    }
    let t_pre = std::time::Instant::now();
    let _open = ClearBridge;
    let Some(sv) = view_of::<IShellView>(i) else { return false };
    let owner = sv.GetWindow().unwrap_or_else(|_| app(|a| a.main));
    if owner.0.is_null() {
        return false;
    }
    let t_select = std::time::Instant::now();
    select_clicked(i, pt);
    super::ctxmenu::log_phase("select", t_select);
    let items = selected_items(i);
    let paths: Vec<String> = items.iter().map(|(p, _)| p.clone()).collect();
    let pidls = selection_pidls(i);
    let refs: Vec<*const ITEMIDLIST> = pidls.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let background = refs.is_empty();
    let props = if background { app(|a| a.s.paths.get(i).cloned().unwrap_or_default()) } else { String::new() };
    let targets = super::ctxmenu::click_targets(background, &items);
    // Same shell menu as an unfiltered click. Disabled commands are removed afterwards.
    // The probe is also how a handler's own menu strings are learned. It is not shown.
    let t_arm = std::time::Instant::now();
    let _probe = view_of::<IFolderView2>(i)
        .and_then(|fv| fv.GetFolder::<IShellFolder>().ok())
        .map(|folder| super::ctxmenu::arm_probe(owner, &folder, &refs));
    if super::ctxmenu::needs_filter(targets) {
        super::ctxmenu::arm_strip(targets);
    }
    super::ctxmenu::log_phase("probe-arm", t_arm);
    let t_cm = std::time::Instant::now();
    let cm = if background {
        sv.GetItemObject::<IContextMenu>(SVGIO_BACKGROUND).ok()
    } else if let Some(folder) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellFolder>().ok()) {
        folder.GetUIObjectOf::<IContextMenu>(owner, &refs, None).ok().or_else(|| sv.GetItemObject::<IContextMenu>(SVGIO_SELECTION).ok())
    } else {
        sv.GetItemObject::<IContextMenu>(SVGIO_SELECTION).ok()
    };
    super::ctxmenu::log_phase("get-cm", t_cm);
    for p in pidls {
        CoTaskMemFree(Some(p as _));
    }
    let Some(cm) = cm else {
        let _ = super::ctxmenu::take_strip();
        return false;
    };
    popup_ctx(owner, cm, &paths, &items, background, true, &props, pt, i, t_pre)
}

/// Explorer menu for files that are not the pane's current selection (search results, by full path).
/// The favourites item is inserted the same way as a click in the file list.
pub(crate) unsafe fn show_paths_ctx(i: usize, items: &[(String, bool)], pt: POINT) -> bool {
    if items.is_empty() {
        return false;
    }
    if CTX_OPEN.swap(true, Ordering::AcqRel) {
        return true;
    }
    let _open = ClearBridge;
    let Some(sv) = view_of::<IShellView>(i) else { return false };
    let owner = sv.GetWindow().unwrap_or_else(|_| app(|a| a.main));
    if owner.0.is_null() {
        return false;
    }
    let paths: Vec<String> = items.iter().map(|(p, _)| p.clone()).collect();
    let targets = super::ctxmenu::click_targets(false, items);
    let _probe = super::ctxmenu::arm_probe_paths(owner, items);
    if super::ctxmenu::needs_filter(targets) {
        super::ctxmenu::arm_strip(targets);
    }
    let Some(cm) = menu_for_paths(&paths) else {
        let _ = super::ctxmenu::take_strip();
        return false;
    };
    popup_ctx(owner, cm, &paths, items, false, false, "", pt, i, std::time::Instant::now())
}

unsafe fn menu_for_paths(paths: &[String]) -> Option<IContextMenu> {
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    for p in paths {
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(p), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            pidls.push(pidl);
        }
    }
    if pidls.is_empty() {
        return None;
    }
    let refs: Vec<*const ITEMIDLIST> = pidls.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let menu = SHCreateShellItemArrayFromIDLists(&refs).ok().and_then(|arr| arr.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject).ok());
    for p in pidls {
        CoTaskMemFree(Some(p as _));
    }
    menu
}

fn open_cm() -> Option<IContextMenu> {
    MENU_BRIDGE.with(|b| {
        let b = b.borrow();
        let b = b.as_ref()?;
        if let Some(cm) = &b.cm3 {
            if let Ok(cm) = cm.cast() {
                return Some(cm);
            }
        }
        b.cm2.as_ref().and_then(|cm| cm.cast().ok())
    })
}

/// One New folder on a background menu, then no dangling separators.
unsafe fn finish_shell_menu(menu: HMENU, cm: Option<&IContextMenu>, background: bool) {
    if background {
        super::ctxmenu::dedupe_new_folder(menu, cm);
    }
    super::ctxmenu::normalize_menu(menu);
}

struct ClearPackIds;
impl Drop for ClearPackIds {
    fn drop(&mut self) {
        super::ctxpack::clear_ids();
    }
}

struct ClearWho;
impl Drop for ClearWho {
    fn drop(&mut self) {
        WHO_PATH.with(|p| *p.borrow_mut() = None);
    }
}

struct ClearExcel;
impl Drop for ClearExcel {
    fn drop(&mut self) {
        super::excelro::clear();
    }
}

/// One filesystem file. Folders and a background click have no single file to ask about.
fn who_path(items: &[(String, bool)], background: bool) -> Option<String> {
    if background || items.len() != 1 {
        return None;
    }
    let (path, file) = &items[0];
    if *file && is_fs(path) { Some(path.clone()) } else { None }
}

/// Directly above batch rename (or favourites, if that row is the one present).
unsafe fn insert_who(menu: HMENU) {
    if let Some(i) = menu_index(menu, ID_CTX_WHO as usize) {
        let _ = DeleteMenu(menu, i, MF_BYPOSITION);
    }
    let Some(path) = WHO_PATH.with(|p| p.borrow().clone()) else { return };
    if path.is_empty() {
        return;
    }
    let Some(at) = menu_index(menu, ID_CTX_BATCH as usize).or_else(|| menu_index(menu, ID_CTX_FAV as usize)) else { return };
    let label = HSTRING::from(tr(crate::Key::WhoIsUsingThisFile));
    _ = InsertMenuW(menu, at, MF_STRING | MF_BYPOSITION, ID_CTX_WHO as usize, &label);
}

unsafe fn popup_ctx(owner: HWND, cm: IContextMenu, paths: &[String], items: &[(String, bool)], background: bool, pane_selection: bool, props_folder: &str, pt: POINT, pane: usize, t_pre: std::time::Instant) -> bool {
    super::excelro::arm(background, items);
    let _clear_excel = ClearExcel;
    let _pack_ids = ClearPackIds;
    let _clear_who = ClearWho;
    WHO_PATH.with(|p| *p.borrow_mut() = who_path(items, background));
    CTX_BACKGROUND.with(|c| c.set(background));
    let Ok(menu) = CreatePopupMenu() else { return false };
    let mut flags = CMF_EXPLORE | CMF_CANRENAME;
    if !background {
        flags |= CMF_ITEMMENU;
    }
    if GetKeyState(VK_SHIFT.0 as i32) < 0 {
        flags |= CMF_EXTENDEDVERBS;
    }
    let t_query = std::time::Instant::now();
    let hr = cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags);
    super::ctxmenu::log_phase("query", t_query);
    if !hr.is_ok() {
        let _ = super::ctxmenu::take_strip();
        _ = DestroyMenu(menu);
        return false;
    }
    // Handlers are in memory now. A later idle period is measured from this menu.
    super::warmup::note_menu();
    let targets = super::ctxmenu::click_targets(background, items);
    // First open pays for the shell-extension catalogue scan once; strip/observe then hit the cache.
    let t_cat = std::time::Instant::now();
    super::ctxmenu::ensure_catalogue();
    super::ctxmenu::log_phase("catalogue", t_cat);
    // Labels are learned after TrackPopupMenu so settings writes are not on the open path.
    // One GetCommandString pass first; strip/app-insert reuse the index (and warm the COM cache).
    let t_verbs = std::time::Instant::now();
    let verbs = VerbIndex::build(menu, &cm, 1, ID_CTX_FAV);
    VERB_INDEX.with(|v| *v.borrow_mut() = Some(verbs.clone()));
    super::ctxmenu::log_phase("verbs-walk", t_verbs);
    let strip = super::ctxmenu::take_strip();
    if strip != 0 {
        let t_strip = std::time::Instant::now();
        super::ctxmenu::strip_disabled(menu, &cm, strip);
        super::ctxmenu::log_phase("strip", t_strip);
    }
    // The background menu of some views (Windows 10) comes without Properties. Add the folder's own.
    let t_app = std::time::Instant::now();
    let mut add_props = false;
    if background {
        let has = verbs.has_verb(menu, "properties");
        let n = GetMenuItemCount(Some(menu)).max(0) as u32;
        if !has && n > 0 {
            if !menu_sep(menu, n - 1) {
                _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            }
            _ = AppendMenuW(menu, MF_STRING, ID_CTX_PROPS as usize, &HSTRING::from(tr(crate::Key::Properties)));
            add_props = true;
        }
    }
    if !paths.is_empty() {
        let at = verbs.fav_anchor(menu, 0);
        insert_fav(menu, at, ID_CTX_FAV as usize, fav_menu_label(&paths));
    }
    // Directly above favourites, in the normal menu and after a filtered shell menu.
    // Background clicks have nothing to rename.
    if !background && !paths.is_empty() {
        if let Some(fav_at) = menu_index(menu, ID_CTX_FAV as usize) {
            let label = HSTRING::from(crate::rename::menu_label());
            _ = InsertMenuW(menu, fav_at, MF_STRING | MF_BYPOSITION, ID_CTX_BATCH as usize, &label);
        }
    }
    insert_who(menu);
    // Next to Favourites (or above Properties on a background click). Only commands that fit this click.
    if !paths.is_empty() {
        let at = menu_index(menu, ID_CTX_FAV as usize).map(|i| i + 1).unwrap_or(0);
        crate::winmerge::insert_ctx(menu, at, pane, items);
    } else if background {
        let at = verbs.fav_anchor(menu, 0);
        crate::winmerge::insert_ctx(menu, at, pane, items);
    }
    super::ctxmenu::log_phase("app-insert", t_app);
    // After strip. Enabled packaged commands are inserted in the upper group, not after Properties.
    let t_inject = std::time::Instant::now();
    super::ctxpack::inject_menu(menu, paths, items, background, props_folder);
    super::ctxmenu::log_phase("inject", t_inject);
    // After every removal and insertion. The shell can still add a row when the menu opens.
    finish_shell_menu(menu, Some(&cm), background);
    tidy_fav_item(menu, ID_CTX_FAV as usize);
    insert_fast_paste_idx(menu, &verbs);
    insert_excel_idx(menu, &verbs);
    super::ctxmenu::dump_menu_items(if background { "background" } else if items.first().is_some_and(|(_, f)| !*f) { "folder" } else { "file" }, menu);
    super::ctxmenu::log_phase("total-pre", t_pre);
    MENU_BRIDGE.with(|b| {
        *b.borrow_mut() = Some(MenuBridge { cm3: cm.cast().ok(), cm2: cm.cast().ok() });
    });
    CTX_MENU.with(|m| m.set(menu.0 as isize));
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, None, owner, None).0;
    super::ctxmenu::observe_menu(menu, &cm, targets);
    MENU_BRIDGE.with(|b| *b.borrow_mut() = None);
    if cmd == ID_CTX_WHO as i32 {
        if let Some(path) = WHO_PATH.with(|p| p.borrow().clone()) {
            super::inuse::show_who(owner, &path);
        }
    } else if cmd == ID_CTX_FAST as i32 {
        let dest = fast_paste_dest(pane, background, items);
        super::fileop::try_fast_paste(pane, Some(dest));
    } else if cmd == ID_CTX_EXCEL as i32 {
        super::excelro::open_readonly(owner);
    } else if cmd == ID_CTX_BATCH as i32 {
        // The pane's own menu can re-read the selection in view order. Search results pass their paths.
        if pane_selection {
            crate::rename::open_pane(pane);
        } else {
            crate::rename::open_paths(owner, items);
        }
    } else if cmd == ID_CTX_FAV as i32 {
        let remove = app(|a| paths.iter().all(|p| a.s.is_fav(p)));
        set_entries(items, !remove);
    } else if add_props && cmd == ID_CTX_PROPS as i32 {
        if !props_folder.is_empty() {
            _ = SHObjectProperties(Some(owner), SHOP_FILEPATH, &HSTRING::from(props_folder), None);
        }
    } else if crate::winmerge::ctx_command(owner, cmd, pane, items) {
    } else if super::ctxpack::invoke_menu_command(cmd as u32, paths, background, props_folder) {
    } else if cmd > 0 {
        let verb = cmd_verb(&cm, (cmd as u32 - 1) as usize);
        let default_item = GetMenuDefaultItem(menu, 0, GET_MENU_DEFAULT_ITEM_FLAGS(0)) as i32 == cmd;
        if !claim_folder_open(pane, &verb, items, default_item) {
            let mut ici = CMINVOKECOMMANDINFO {
                cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
                hwnd: owner,
                lpVerb: PCSTR((cmd as u32 - 1) as usize as *const u8),
                nShow: SW_SHOWNORMAL.0 as i32,
                ..Default::default()
            };
            if GetKeyState(VK_SHIFT.0 as i32) < 0 {
                ici.fMask |= CMIC_MASK_SHIFT_DOWN;
            }
            if GetKeyState(VK_CONTROL.0 as i32) < 0 {
                ici.fMask |= CMIC_MASK_CONTROL_DOWN;
            }
            _ = cm.InvokeCommand(&ici);
        }
    }
    _ = DestroyMenu(menu);
    // Label probes CoCreate handlers. The popup is already gone, so this is not the wait to open it.
    super::ctxmenu::learn_pending(targets);
    true
}

unsafe fn eat_list_activation(h: HWND, data: usize) -> bool {
    bind_hwnd(h);
    let pane = pane_of(h).unwrap_or(data);
    let sv = view_of::<IShellView>(pane);
    take_default(pane, sv.as_ref())
}

fn list_notify_item(lp: LPARAM) -> i32 {
    use windows::Win32::UI::Controls::NMLISTVIEW;
    unsafe { (*(lp.0 as *const NMLISTVIEW)).iItem }
}

/// Empty space in the list. Eating that double-click would open the focused folder
/// instead of going up a level.
unsafe fn dblclick_on_background(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> bool {
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() {
        return false;
    }
    let probe = MSG { hwnd: h, message: msg, wParam: wp, lParam: lp, pt, ..Default::default() };
    on_background(&probe)
}

pub(crate) unsafe extern "system" fn ctx_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, data: usize) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { ctx_proc_inner(h, msg, wp, lp, _a1, data) }))
}
unsafe fn ctx_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _a1: usize, data: usize) -> LRESULT {
    if msg == WM_INITMENUPOPUP {
        // The shell may insert its own separator, or a second New folder, while the menu opens.
        let lr = forward_menu(msg, wp, lp);
        let popup = HMENU(wp.0 as _);
        let top = CTX_MENU.with(|m| m.get() != 0 && m.get() == popup.0 as isize);
        let background = CTX_BACKGROUND.with(|c| c.get());
        if top && background {
            finish_shell_menu(popup, open_cm().as_ref(), true);
        } else if !popup.0.is_null() {
            super::ctxmenu::normalize_menu(popup);
        }
        // Favourites keeps its one separator after New folder and the bars are settled.
        if top {
            // The shell can rebuild the top menu as it opens. Put the file row back, then the separator.
            insert_who(popup);
            tidy_fav_item(popup, ID_CTX_FAV as usize);
            if let Some(verbs) = VERB_INDEX.with(|v| v.borrow().clone()) {
                insert_fast_paste_idx(popup, &verbs);
                insert_excel_idx(popup, &verbs);
            } else if let Some(cm) = open_cm() {
                insert_fast_paste(popup, &cm);
                insert_excel(popup, &cm);
            }
        }
        if let Some(lr) = lr {
            return lr;
        }
    } else if matches!(msg, WM_UNINITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        if let Some(lr) = forward_menu(msg, wp, lp) {
            return lr;
        }
    }
    if msg == WM_PARENTNOTIFY && (wp.0 & 0xFFFF) == WM_CREATE as usize {
        hook_ctx_hwnd(HWND(lp.0 as _), data);
    }
    if msg == WM_NOTIFY && wnd_class(h) == "SHELLDLL_DefView" {
        let hdr = &*(lp.0 as *const NMHDR);
        let from = wnd_class(hdr.hwndFrom);
        // NM_DBLCLK (-3) and LVN_ITEMACTIVATE (-114). A header click auto-sizes a column and is not an open.
        let activate = hdr.code == (-3i32 as u32) || hdr.code == (-114i32 as u32);
        let background = hdr.code == (-3i32 as u32) && from == "SysListView32" && list_notify_item(lp) < 0;
        if activate && from != "SysHeader32" && !background && eat_list_activation(h, data) {
            return LRESULT(0);
        }
    }
    if msg == WM_LBUTTONDBLCLK && wnd_class(h) != "SHELLDLL_DefView" && ctx_surface(h) && !dblclick_on_background(h, msg, wp, lp) && eat_list_activation(h, data) {
        return LRESULT(0);
    }
    if msg == WM_KEYDOWN && wp.0 as u16 == VK_RETURN.0 && ctx_surface(h) && eat_list_activation(h, data) {
        return LRESULT(1);
    }
    if matches!(msg, WM_MBUTTONDOWN | WM_MBUTTONDBLCLK) && ctx_surface(h) {
        bind_hwnd(h);
        let pane = pane_of(h).unwrap_or(data);
        crate::folderapp::note_activate(app(|a| a.main.0 as isize), pane, true);
    }
    if msg == WM_CONTEXTMENU {
        let pt = if lp.0 == -1 {
            let mut rc = RECT::default();
            let _ = GetWindowRect(h, &mut rc);
            POINT { x: (rc.left + rc.right) / 2, y: (rc.top + rc.bottom) / 2 }
        } else {
            POINT { x: (lp.0 & 0xFFFF) as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32 }
        };
        let under = if lp.0 == -1 { h } else { WindowFromPoint(pt) };
        if ctx_surface(under) && show_ctx(data, pt) {
            return LRESULT(0);
        }
    }
    DefSubclassProc(h, msg, wp, lp)
}

// Sidebar menu: the tree builds the shell menu, then asks us for a replacement. Ours forwards
// every shell command and puts the favourites item directly above Properties.
#[implement(IContextMenu, IContextMenu2, IContextMenu3)]
pub(crate) struct FavCtx {
    pub(crate) inner: IContextMenu,
    pub(crate) path: String,
    pub(crate) cmd: Cell<u32>,
    pub(crate) item: Cell<usize>, // menu id of the favourites command
    pub(crate) popup: Cell<isize>, // HMENU that command was inserted into
}

impl IContextMenu_Impl for FavCtx_Impl {
    fn QueryContextMenu(&self, hmenu: HMENU, indexmenu: u32, idcmdfirst: u32, idcmdlast: u32, uflags: u32) -> HRESULT {
        unsafe {
            let hr = self.inner.QueryContextMenu(hmenu, indexmenu, idcmdfirst, idcmdlast, uflags);
            if !hr.is_ok() || uflags & CMF_DEFAULTONLY != 0 {
                return hr;
            }
            let used = (hr.0 as u32) & 0xFFFF;
            // Offset 0 looks like a null verb, so an empty shell menu gets the next slot.
            let offset = if used == 0 { 1 } else { used };
            if idcmdfirst.saturating_add(offset) >= idcmdlast {
                return hr;
            }
            self.cmd.set(offset);
            let item = (idcmdfirst + offset) as usize;
            self.item.set(item);
            self.popup.set(hmenu.0 as isize);
            let at = fav_anchor_pos(hmenu, &self.inner, idcmdfirst, idcmdlast.saturating_add(1), indexmenu);
            insert_fav(hmenu, at, item, fav_menu_label(std::slice::from_ref(&self.path)));
            HRESULT((offset + 1) as i32)
        }
    }
    fn InvokeCommand(&self, pici: *const CMINVOKECOMMANDINFO) -> Result<()> {
        unsafe {
            if !pici.is_null() && verb_offset(pici) == Some(self.cmd.get()) {
                let remove = app(|a| a.s.is_fav(&self.path));
                set_favs(std::slice::from_ref(&self.path), !remove);
                return Ok(());
            }
            if !pici.is_null() {
                if let Some(offset) = verb_offset(pici) {
                    let verb = cmd_verb(&self.inner, offset as usize);
                    // Offset 0 with an empty verb is the menu default (Open) when the string could not be read.
                    let default_item = offset == 0 && verb.trim().is_empty();
                    let item = [(self.path.clone(), false)];
                    if claim_folder_open(active_pane(), &verb, &item, default_item) {
                        return Ok(());
                    }
                }
            }
            self.inner.InvokeCommand(pici)
        }
    }
    fn GetCommandString(&self, idcmd: usize, utype: u32, preserved: *const u32, pszname: PSTR, cchmax: u32) -> Result<()> {
        if idcmd == self.cmd.get() as usize {
            // No verb or help text for our item. Leave an empty string and say so, rather than Ok with an untouched buffer.
            if !pszname.is_null() && cchmax > 0 {
                unsafe {
                    if utype & GCS_UNICODE != 0 {
                        *(pszname.0 as *mut u16) = 0;
                    } else {
                        *pszname.0 = 0;
                    }
                }
            }
            return Err(E_NOTIMPL.into());
        }
        unsafe { self.inner.GetCommandString(idcmd, utype, Some(preserved), pszname, cchmax) }
    }
}

impl IContextMenu2_Impl for FavCtx_Impl {
    fn HandleMenuMsg(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> Result<()> {
        unsafe {
            let result = if let Ok(m) = self.inner.cast::<IContextMenu2>() { m.HandleMenuMsg(umsg, wparam, lparam) } else { Ok(()) };
            if umsg == WM_INITMENUPOPUP {
                tidy_fav_popup(self.popup.get(), self.item.get(), HMENU(wparam.0 as _));
            }
            result
        }
    }
}

impl IContextMenu3_Impl for FavCtx_Impl {
    fn HandleMenuMsg2(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM, plresult: *mut LRESULT) -> Result<()> {
        unsafe {
            let result = if let Ok(m) = self.inner.cast::<IContextMenu3>() {
                m.HandleMenuMsg2(umsg, wparam, lparam, Some(plresult))
            } else if let Ok(m) = self.inner.cast::<IContextMenu2>() {
                m.HandleMenuMsg(umsg, wparam, lparam)
            } else {
                Ok(())
            };
            if umsg == WM_INITMENUPOPUP {
                tidy_fav_popup(self.popup.get(), self.item.get(), HMENU(wparam.0 as _));
            }
            result
        }
    }
}

pub(crate) unsafe fn tidy_fav_popup(popup: isize, item: usize, menu: HMENU) {
    if popup != 0 && popup == menu.0 as isize {
        tidy_fav_item(menu, item);
    }
}

pub(crate) fn verb_offset(ici: *const CMINVOKECOMMANDINFO) -> Option<u32> {
    unsafe {
        let v = (*ici).lpVerb.0 as usize;
        if v > 0 && v < 0x10000 {
            return Some(v as u32);
        }
        if (*ici).cbSize >= size_of::<CMINVOKECOMMANDINFOEX>() as u32 {
            let w = (*(ici as *const CMINVOKECOMMANDINFOEX)).lpVerbW.0 as usize;
            if w > 0 && w < 0x10000 {
                return Some(w as u32);
            }
        }
        None
    }
}

pub(crate) fn shell_folder(it: &IShellItem) -> bool {
    unsafe { item_is_container(it) == Some(true) }
}

pub(crate) fn shell_parse(it: &IShellItem) -> String {
    unsafe { it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING).map(|p| pwstr(p)).unwrap_or_default() }
}

pub(crate) struct FavGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) rows: Vec<(RECT, RECT)>, // (row, remove ✕)
}

pub(crate) fn fav_geo(n: usize, dpi: i32) -> FavGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, rh, w) = (s(6), s(40), s(380));
    let rows = (0..n as i32)
        .map(|k| {
            let row = RECT { left: pad, top: pad + k * rh, right: w - pad, bottom: pad + (k + 1) * rh };
            (row, RECT { left: row.right - s(32), ..row })
        })
        .collect();
    FavGeo { w, h: 2 * pad + n.max(1) as i32 * rh, rows }
}

fn fav_rows() -> Vec<FavRow> {
    app(|a| {
        a.s.ensure_fav_tree();
        visible_rows(&a.s.tree)
    })
}

/// Where the favourites list opens: under the gear, or at the top of the active pane when the bar is hidden.
unsafe fn fav_popup_anchor(main: HWND) -> RECT {
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_SET) {
        if IsWindowVisible(b).as_bool() {
            _ = GetWindowRect(b, &mut rc);
            if rc.bottom > rc.top {
                return rc;
            }
        }
    }
    let pane = app(|a| a.panes.get(a.s.active).and_then(|p| p.as_ref()).map(|p| p.hwnd)).unwrap_or_default();
    if !pane.0.is_null() {
        _ = GetWindowRect(pane, &mut rc);
        rc.bottom = rc.top;
    }
    rc
}

pub(crate) unsafe fn open_favs() {
    let (main, dpi, open, closed, n) = app(|a| {
        a.s.ensure_fav_tree();
        (a.main, a.dpi, a.fav_popup, a.fav_closed, visible_rows(&a.s.tree).len())
    });
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let g = fav_geo(n, dpi);
    let rc = fav_popup_anchor(main);
    let prev = GetFocus();
    app(|a| (a.fav_hot, a.fav_prev) = (None, prev));
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("MaguroFav"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.fav_popup = h);
}

pub(crate) unsafe fn paint_favs(h: HWND) {
    let (rows, hot, dpi, font, small, icon_font) = app(|a| {
        a.s.ensure_fav_tree();
        (visible_rows(&a.s.tree), a.fav_hot, a.dpi, a.font, a.small_font, a.icon_font)
    });
    let g = fav_geo(rows.len(), dpi);
    let s = |v: i32| v * dpi / 96;
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        if rows.is_empty() {
            text(dc, rc, tr(crate::Key::NoFavouritesYetAddOneWithIn), font, pal().sub, DT_CENTER);
        }
        for (k, (f, (row, remove))) in rows.iter().zip(&g.rows).enumerate() {
            if hot.is_some_and(|(i, _)| i == k) {
                fill(dc, *row, pal().pill);
                let x_on = hot == Some((k, true));
                text(dc, *remove, "\u{E711}", icon_font, if x_on { pal().text } else { pal().sub }, DT_CENTER);
            }
            let indent = s(14) * f.depth.min(6) as i32;
            let icon = RECT { left: row.left + s(6) + indent, right: row.left + s(28) + indent, ..*row };
            let glyph = if f.file { "\u{E8A5}" } else { "\u{E8B7}" };
            text(dc, icon, glyph, icon_font, pal().sub, DT_CENTER);
            let line = |top: i32, hgt: i32| RECT { left: icon.right + s(6), right: remove.left - s(4), top, bottom: top + hgt };
            let label = if f.name.is_empty() { short_name(&f.path) } else { f.name.clone() };
            let ink = if !f.group && known_missing(&f.path) { pal().sub } else { pal().text };
            text(dc, line(row.top + s(3), s(19)), &label, font, ink, DT_LEFT);
            let sub = if f.group {
                tr(crate::Key::Group)
            } else {
                f.path.as_str()
            };
            text(dc, line(row.top + s(21), s(16)), sub, small, pal().sub, DT_LEFT | DT_PATH_ELLIPSIS);
        }
    });
}

pub(crate) fn fav_hit(g: &FavGeo, x: i32, y: i32) -> Option<(usize, bool)> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    g.rows.iter().position(|(row, _)| inside(row)).map(|k| (k, inside(&g.rows[k].1)))
}

pub(crate) extern "system" fn fav_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| fav_proc_inner(h, msg, wp, lp)))
}
fn fav_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        let g = fav_geo(fav_rows().len(), app(|a| a.dpi));
        match msg {
            WM_PAINT => paint_favs(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hot = fav_hit(&g, x, y);
                if app(|a| std::mem::replace(&mut a.fav_hot, hot)) != hot {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP | WM_KEYDOWN => {
                // ✕ / Del: drop it and shrink the list in place
                let remove = |k: usize| {
                    let id = fav_rows().get(k).map(|r| r.id.clone());
                    if let Some(id) = id {
                        let links = servers_at(&id);
                        app(|a| {
                            delete_at(&mut a.s.tree, &id);
                            a.s.sync_favs();
                            a.fav_hot = None;
                        });
                        release_servers(links);
                        favs_changed();
                    }
                };
                let open = |k: usize| {
                    let row = fav_rows().get(k).cloned();
                    let Some(row) = row else { return };
                    if row.group {
                        app(|a| {
                            toggle_open(&mut a.s.tree, &row.id);
                        });
                        favs_changed();
                        return;
                    }
                    _ = DestroyWindow(h);
                    open_fav_row(&row);
                };
                if msg == WM_LBUTTONUP {
                    let (x, y) = xy(lp);
                    match fav_hit(&g, x, y) {
                        Some((k, true)) => remove(k),
                        Some((k, false)) => open(k),
                        None => {}
                    }
                    return LRESULT(0);
                }
                let (n, cur) = (fav_rows().len(), app(|a| a.fav_hot.map(|(k, _)| k)));
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => {
                        let (main, prev, active) = app(|a| (a.main, a.fav_prev, a.s.active));
                        _ = DestroyWindow(h);
                        let gear = GetDlgItem(Some(main), ID_TB_SET).unwrap_or_default();
                        if !prev.0.is_null() && IsWindow(Some(prev)).as_bool() && prev != gear {
                            _ = SetFocus(Some(prev));
                        } else {
                            focus_view(active);
                        }
                    }
                    _ if n == 0 => {}
                    k @ (VK_DOWN | VK_UP) => {
                        let next = if k == VK_DOWN { cur.map_or(0, |k| (k + 1) % n) } else { cur.map_or(n - 1, |k| (k + n - 1) % n) };
                        app(|a| a.fav_hot = Some((next, false)));
                        _ = InvalidateRect(Some(h), None, false);
                    }
                    VK_RETURN => if let Some(k) = cur { open(k) },
                    VK_DELETE => if let Some(k) = cur {
                        remove(k);
                        app(|a| a.fav_hot = if n > 1 { Some((k.min(n - 2), false)) } else { None });
                    },
                    _ => {}
                }
            }
            WM_SYSKEYDOWN => {
                let vk = wp.0 as u16;
                let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
                if fav_step(vk, down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT)).is_none() {
                    return DefWindowProcW(h, msg, wp, lp);
                }
                let rows = fav_rows();
                let Some(row) = app(|a| a.fav_hot).and_then(|(k, _)| rows.get(k).cloned()).filter(|r| r.in_tree()) else {
                    return LRESULT(0);
                };
                let moved = app(|a| {
                    let id = nudge_node(&mut a.s.tree, &row.id, vk == VK_DOWN.0);
                    if id.is_some() {
                        a.s.sync_favs();
                    }
                    id
                });
                if let Some(id) = moved {
                    let i = fav_rows().iter().position(|r| r.id == id);
                    app(|a| a.fav_hot = i.map(|k| (k, false)));
                    favs_changed();
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.fav_popup, a.fav_closed) = (HWND::default(), Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// F7: the view's own "New folder", so the shell names it, selects it and starts rename.
pub(crate) unsafe fn explorer_new_folder(i: usize) {
    let Some(sv) = view_of::<IShellView>(i) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let Ok(cm) = sv.GetItemObject::<IContextMenu>(SVGIO_BACKGROUND) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let Ok(menu) = CreatePopupMenu() else { return };
    let verb = b"NewFolder\0";
    if cm.QueryContextMenu(menu, 0, 1, 0x7fff, CMF_NORMAL).is_ok() {
        let ci = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd: sv.GetWindow().unwrap_or_default(),
            lpVerb: PCSTR(verb.as_ptr()),
            nShow: SW_SHOWNORMAL.0 as i32,
            ..Default::default()
        };
        if cm.InvokeCommand(&ci).is_err() {
            _ = MessageBeep(MB_ICONWARNING);
        }
    } else {
        _ = MessageBeep(MB_ICONWARNING);
    }
    _ = DestroyMenu(menu);
}

// Ctrl+Shift+N: a new folder in the active pane, then the shell's own rename (F2) once it shows up.
pub(crate) unsafe fn new_folder(i: usize) {
    let dir = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    if !is_fs(&dir) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let base = tr(crate::Key::NewFolder);
    let name = fresh_folder(base, |n| Path::new(&dir).join(n).exists());
    if std::fs::create_dir(Path::new(&dir).join(&name)).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let full = Path::new(&dir).join(&name);
    let wide: Vec<u16> = full.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    SHChangeNotify(SHCNE_MKDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
    if let Ok(mut p) = PENDING_NEW.lock() {
        *p = Some(PendingNew { pane: i, name, left: 8 });
    }
    if let Some(h) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)) {
        SetTimer(Some(h), TIMER_NEWF, 40, None);
    }
    focus_view(i);
}

// The view lists the new folder a moment after Refresh. Select it and start the shell rename.
pub(crate) unsafe fn select_new_folder(i: usize, name: &str) -> bool {
    let Some(fv) = view_of::<IFolderView2>(i) else { return false };
    let Ok(folder) = fv.GetFolder::<IShellFolder>() else { return false };
    let mut pidl = std::ptr::null_mut();
    let parsed = folder.ParseDisplayName(HWND::default(), None, &HSTRING::from(name), None, &mut pidl, std::ptr::null_mut());
    if parsed.is_err() || pidl.is_null() {
        return false;
    }
    let Some(sv) = view_of::<IShellView>(i) else {
        CoTaskMemFree(Some(pidl as _));
        return false;
    };
    // SVSI_EDIT is the shell's rename. If this view won't edit, selecting is enough.
    let edit = (SVSI_EDIT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    let ok = sv.SelectItem(pidl, edit).is_ok()
        || sv.SelectItem(pidl, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

#[cfg(test)]
mod tests {
    use super::{fav_action_label, parent_of};
    use crate::i18n::UI;
    use std::sync::atomic::Ordering;

    #[test]
    fn parent_of_a_file_keeps_the_drive_root() {
        assert_eq!(parent_of(r"C:\a\note.txt").as_deref(), Some(r"C:\a"));
        assert_eq!(parent_of(r"C:\note.txt").as_deref(), Some(r"C:\"));
        assert_eq!(parent_of("note.txt"), None);
    }

    #[test]
    fn maguro_favourite_row_names_the_app_in_every_language() {
        let prev = UI.load(Ordering::Relaxed);
        UI.store(0, Ordering::Relaxed);
        assert_eq!(fav_action_label(false), "Add to Maguro favorites");
        assert_eq!(fav_action_label(true), "Remove from Maguro favorites");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(fav_action_label(false), "Maguro 즐겨찾기에 추가");
        assert_eq!(fav_action_label(true), "Maguro 즐겨찾기에서 제거");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(fav_action_label(false), "Maguro のお気に入りに追加");
        assert_eq!(fav_action_label(true), "Maguro のお気に入りから削除");
        UI.store(prev, Ordering::Relaxed);
    }
}

pub(crate) unsafe fn popup_menu(menu: HMENU, id: i32) {
    let main = app(|a| a.main);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), id) {
        _ = GetWindowRect(b, &mut rc);
    }
    _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom + 4, None, main, None);
}

#[cfg(test)]
mod utf8_boundary_tests {
    #[test]
    fn japanese_korean_and_emoji_parents_do_not_panic() {
        assert_eq!(super::parent_of(r"F:\한글폴더\샘플.txt").as_deref(), Some(r"F:\한글폴더"));
        assert_eq!(super::parent_of(r"D:\サンプル\📄.txt").as_deref(), Some(r"D:\サンプル"));
        assert_eq!(super::parent_of(r"E:\😀\あ.txt").as_deref(), Some(r"E:\😀"));
        assert_eq!(super::parent_of(r"C:\abcdサ.txt").as_deref(), Some(r"C:\"));
        assert_eq!(super::parent_of("샘플.txt"), None);
    }
}
