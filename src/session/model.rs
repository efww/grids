use crate::imports::*;
use super::*;
use crate::app::*;
use crate::i18n::*;
use crate::store;

// ---------- session (settings live in maguro.json; named sessions stay .grids files in the ini shape) ----------

/// One tab: a whole pane grid (layout, folders, view, columns, filter). The active tab's grid
/// also lives on the session fields the rest of the window already reads.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tab {
    /// Custom title. Empty, and a legacy all-digit title such as `"1"`, show the active pane's folder.
    pub(crate) name: String,
    pub(crate) locked: bool,
    /// Stripe colour 0xRRGGBB. None draws the tab in the theme colours.
    pub(crate) color: Option<u32>,
    pub(crate) heights: Vec<usize>,
    pub(crate) colr: Vec<f64>,
    pub(crate) rowr: Vec<Vec<f64>>,
    pub(crate) active: usize,
    pub(crate) paths: Vec<String>,
    pub(crate) views: Vec<(i32, i32)>,
    pub(crate) cols: Vec<String>,
    pub(crate) filters: Vec<String>,
    /// Per pane: 0 none, 1..=6 a selection-colour preset of the current theme. Separate from the tab stripe.
    pub(crate) tints: Vec<u8>,
    /// Per pane: folders this pane switches between. Empty means just `paths[i]`. Not the window tabs above.
    pub(crate) pane_folders: Vec<Vec<String>>,
    /// Per pane: which pane-tab is current. The current entry in `pane_folders` is stale; `paths[i]` is the truth.
    pub(crate) pane_cur: Vec<usize>,
    /// `.grids` file this tab was saved as, or that a session was loaded into. Empty means Save asks for a name.
    /// Other tabs keep their own paths. Omitted from older files, which still load.
    pub(crate) session: String,
}

/// A title the user typed. Empty and the old numeric defaults (`"1"`, `"2"`, …) are automatic.
pub(crate) fn custom_tab_name(name: &str) -> Option<&str> {
    let name = name.trim();
    if name.is_empty() || name.chars().all(|c| c.is_ascii_digit()) {
        None
    } else {
        Some(name)
    }
}

/// What a rename stores. Empty means automatic: the strip shows the active pane's folder.
/// A legacy all-digit title is cleared, so `"1"` does not stick as a custom name.
pub(crate) fn stored_tab_name(typed: &str) -> String {
    match custom_tab_name(typed) {
        Some(name) => name.chars().take(80).collect(),
        None => String::new(),
    }
}

/// Folder name for an automatic tab.
///
/// `C:\` and `C:` are `C:`. A trailing slash is ignored, so `C:\Work\docs\` is `docs`.
/// A UNC path uses its last component (`\\server\share` is `share`, `\\server\share\docs` is `docs`).
/// This PC's shell name is `This PC`. An empty path is empty. Any other path with no usable
/// segment (a `::{…}` name that is not This PC) falls back to the path itself.
pub(crate) fn folder_display_name(path: &str) -> String {
    if path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        return "This PC".into();
    }
    let t = path.trim_end_matches(['\\', '/']);
    if let Some(name) = t.rsplit(['\\', '/']).next() {
        if !name.is_empty() && !name.starts_with("::") {
            return name.to_string();
        }
    }
    t.to_string()
}

/// Label on the tab strip: a custom title, otherwise the active pane's folder.
/// With no folder yet, a stored token such as `"1"` stays visible so the tab is not blank.
pub(crate) fn tab_label(stored: &str, path: &str) -> String {
    if let Some(custom) = custom_tab_name(stored) {
        return custom.to_string();
    }
    let folder = folder_display_name(path);
    if !folder.is_empty() {
        return folder;
    }
    let stored = stored.trim();
    if !stored.is_empty() {
        stored.to_string()
    } else {
        String::new()
    }
}

/// Tooltip body: one full path per pane, in pane order. Empty panes are left out.
pub(crate) fn tab_tip_text(paths: &[&str]) -> String {
    paths.iter().copied().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\r\n")
}

pub(crate) fn pad_filters(mut f: Vec<String>) -> Vec<String> {
    f.resize(MAXP, String::new());
    f.truncate(MAXP);
    f
}

impl Tab {
    pub(crate) fn from_live(s: &Session, name: String, locked: bool, filters: Vec<String>) -> Self {
        let pack = pack_pane_tabs(s);
        Tab {
            name,
            locked,
            color: None,
            heights: s.heights.clone(),
            colr: s.colr.clone(),
            rowr: s.rowr.clone(),
            active: s.active,
            paths: s.paths.clone(),
            views: s.views.clone(),
            cols: s.cols.clone(),
            filters: pad_filters(filters),
            tints: s.tints.clone(),
            pane_folders: pack.0,
            pane_cur: pack.1,
            session: String::new(),
        }
    }
}

fn pad_pane_folders(mut f: Vec<Vec<String>>) -> Vec<Vec<String>> {
    f.resize(MAXP, Vec::new());
    f.truncate(MAXP);
    f
}

fn pad_pane_cur(mut c: Vec<usize>) -> Vec<usize> {
    c.resize(MAXP, 0);
    c.truncate(MAXP);
    c
}

/// Resolved pane-tab lists, dropped back to empty when a pane has only its current folder.
pub(crate) fn pack_pane_tabs(s: &Session) -> (Vec<Vec<String>>, Vec<usize>) {
    let mut folders = vec![Vec::new(); MAXP];
    let mut cur = vec![0usize; MAXP];
    for i in 0..MAXP {
        let list = s.pane_tab_list(i);
        if list.len() > 1 {
            let k = s.pane_cur.get(i).copied().unwrap_or(0).min(list.len() - 1);
            folders[i] = list;
            cur[i] = k;
        }
    }
    (folders, cur)
}

/// The next automatic token: 1, then 2, skipping numbers already stored.
/// The strip does not show this while the tab has a folder; see `tab_label`.
pub(crate) fn next_tab_name(tabs: &[Tab]) -> String {
    let mut n = 1u32;
    loop {
        let s = n.to_string();
        if !tabs.iter().any(|t| t.name == s) {
            return s;
        }
        n += 1;
        if n > 10_000 {
            return s;
        }
    }
}

/// A copy of an automatic tab (empty or a legacy number) takes the next free number.
/// A custom title becomes "Name (2)".
pub(crate) fn duplicate_name(tabs: &[Tab], src: &str) -> String {
    let Some(src) = custom_tab_name(src) else {
        return next_tab_name(tabs);
    };
    let mut n = 2u32;
    loop {
        let s = format!("{src} ({n})");
        if !tabs.iter().any(|t| t.name == s) {
            return s;
        }
        n += 1;
        if n > 1000 {
            return s;
        }
    }
}

/// A locked tab, and the last remaining tab, stay open.
pub(crate) fn can_close_tab(tabs: &[Tab], i: usize) -> bool {
    tabs.len() > 1 && tabs.get(i).is_some_and(|t| !t.locked)
}

/// Indexes closed by "close other tabs": every unlocked tab except `keep`, low to high.
pub(crate) fn close_other_indexes(tabs: &[Tab], keep: usize) -> Vec<usize> {
    tabs.iter().enumerate().filter(|(i, t)| *i != keep && !t.locked).map(|(i, _)| i).collect()
}

pub(crate) fn reorder_vec<T>(v: &mut Vec<T>, from: usize, to: usize) {
    if from == to || from >= v.len() || to >= v.len() {
        return;
    }
    let item = v.remove(from);
    v.insert(to, item);
}

/// Where the active index lands after a tab moves from `from` to `to`.
pub(crate) fn moved_active(active: usize, from: usize, to: usize) -> usize {
    if active == from {
        return to;
    }
    if from < active && to >= active {
        active - 1
    } else if from > active && to <= active {
        active + 1
    } else {
        active
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Session {
    pub(crate) win: Option<[i32; 5]>, // normal rect l,t,r,b + maximized flag
    pub(crate) side: i32,             // sidebar width in px
    pub(crate) side_dpi: i32,         // DPI `side` was measured at; 0 = unknown (older files)
    pub(crate) heights: Vec<usize>,   // panes per column, left to right (1..=4 columns of 1..=4)
    pub(crate) colr: Vec<f64>,        // column width ratios
    pub(crate) rowr: Vec<Vec<f64>>,   // row height ratios, per column
    pub(crate) active: usize,
    pub(crate) paths: Vec<String>,     // parsing names per pane slot (numbered down each column, left to right)
    pub(crate) views: Vec<(i32, i32)>, // (FOLDERVIEWMODE, icon size); 0 = shell default
    pub(crate) accent: u32,            // selection colour 0xRRGGBB, light theme
    pub(crate) accent_dark: u32,       // selection colour, dark theme
    pub(crate) dark: bool,
    pub(crate) dblup: bool,            // double-click on empty space goes to the parent folder
    /// Space in a file list opens quick look. Missing from the file means on.
    pub(crate) space_preview: bool,
    /// Wheel in quick look steps to the next or previous file at the content's end. Missing means on.
    pub(crate) peek_wheel_nav: bool,
    /// Remember sort, columns, view mode, icon size and grouping per folder. Missing means on.
    pub(crate) remember_views: bool,
    /// Status line shows a selected folder's total size. Missing means on.
    pub(crate) folder_sizes: bool,
    /// Also size UNC and mapped drives. Missing means off. WSL is sized either way.
    pub(crate) folder_sizes_network: bool,
    /// Quick look size in DIP after the user resizes, per kind. Empty means fit the file.
    /// The maximized rect is not stored. `peek_doc` is text and PDF together.
    pub(crate) peek_image: Option<(i32, i32)>,
    pub(crate) peek_doc: Option<(i32, i32)>,
    /// Per pane, for the window tab on screen: its pane tabs' folders. The current entry is stale; `paths[i]` is the truth.
    /// A pane tab is one folder inside one pane, not a window tab (`tabs` / `tab`). The strip shows from the second folder on.
    pub(crate) pane_folders: Vec<Vec<String>>,
    pub(crate) pane_cur: Vec<usize>,
    pub(crate) hotkey: bool,           // global hotkey brings Maguro forward; the chord is in the keymap
    /// List Maguro folders in another app's Open or Save dialog. Missing means on.
    pub(crate) dialog_jump: bool,
    /// Open a clipboard path in a new tab. Missing means on. The chord is in the keymap.
    pub(crate) clip_hotkey: bool,
    /// Session file of the tab on screen. Mirrors `tabs[tab].session`.
    /// Older files stored only this, for the whole window; it is adopted onto the active tab.
    pub(crate) file: String,
    pub(crate) favs: Vec<String>,      // favourite leaves, depth-first (`path` or `path\tname`)
    pub(crate) tree: Vec<FavNode>,     // groups and leaves; `favs` is derived from this
    pub(crate) recent: Vec<String>,    // recently visited folders from every pane, newest first, app-wide
    pub(crate) lang: Lang,             // UI language; missing means Auto. Auto follows Windows (ko, ja, else English)
    pub(crate) single: bool,           // a second launch opens a new window in this process
    pub(crate) one_window: bool,       // one top-level window; a second launch and new-window actions open a tab
    /// Subfolder search asks Everything when it is running. Missing from the file means on.
    pub(crate) use_everything: bool,
    /// WinMergeU.exe, or a folder that contains it. Empty means auto-detect.
    pub(crate) winmerge_path: String,
    /// The one-time "install Everything?" question was already shown. Never asked again.
    pub(crate) ev_asked: bool,
    /// Everything result order. Everything's own sort code (1 name A→Z … 14 newest first).
    /// maguro.json only. A .grids file and grids.ini do not carry this.
    pub(crate) ev_sort: u32,
    /// The one-time "install rclone and WinFsp?" question was already shown. Never asked again.
    pub(crate) remote_asked: bool,
    pub(crate) cols: Vec<String>,      // per pane: columns and sort of its view (cols_text)
    pub(crate) tints: Vec<u8>,         // per pane: colour label, 0 = none, 1..=6 = selection colour preset
    /// Slot i shows what slot `order[i]` held when the session was last saved. Empty means as saved.
    pub(crate) order: Vec<u8>,
    pub(crate) theme: u8,              // 0 follow Windows, 1 light, 2 dark
    /// Chosen UI face. Empty uses the language default (Segoe UI / Malgun Gothic / Yu Gothic UI).
    pub(crate) font_face: String,
    pub(crate) bar: bool,              // tab-row icons and drive buttons
    /// Drive-letter buttons on the tab row. Off hides them and gives that width to the tabs. Missing means on.
    pub(crate) drive_buttons: bool,
    pub(crate) stat: bool,             // status line
    pub(crate) view_new: i32,          // FOLDERVIEWMODE for a pane that has none yet (see EXPLORER_VIEWS)
    pub(crate) view_icon: i32,         // icon size beside view_new; 0 = that mode's usual size (large icons are 96)
    pub(crate) start_new: u8,          // 0 This PC, 1 Desktop, 2 start_path
    pub(crate) start_path: String,
    pub(crate) ext: bool,              // show file extensions (the Windows Explorer setting)
    pub(crate) tree_on: bool,          // left panel open
    /// Right-hand preview of the active pane. Missing means hidden.
    pub(crate) preview_on: bool,
    /// Preview plate width in DIP. 0 means the default until the user drags the splitter.
    pub(crate) preview_w: i32,
    pub(crate) fav_top: bool,          // favourites above the folder tree; false puts the tree on top
    pub(crate) side_split: f64,        // folder tree's share of the left panel; favourites take the rest
    pub(crate) pins_open: bool,        // Explorer pins group expanded; the rows are not saved
    pub(crate) files_open: bool,       // Explorer favourite-files group expanded; hidden when empty; rows are not saved
    pub(crate) edge: bool,             // accent border on the active pane
    pub(crate) confirm: bool,          // ask before delete
    pub(crate) del: u8,                // 0 recycle bin, 1 permanent
    pub(crate) new_name: String,       // empty = the name for the display language
    pub(crate) dropm: u8,              // 0 Windows rules, 1 always copy, 2 always ask (the ⇄ menu)
    /// Paste and drag use robocopy when a path is on the network. Missing means off.
    /// Fast paste on the menu uses robocopy either way.
    pub(crate) fast_copy: bool,
    pub(crate) layout_new: u8,         // 0 side by side, 1 top/bottom, 2 is 2×2, 3 one pane (fresh default 3; missing parse = 0)
    pub(crate) boot: u8,               // 0 restore last layout, 1 open boot_file, 2 the default layout
    pub(crate) boot_file: String,
    pub(crate) update_check: bool,     // ask GitHub for a newer release at startup
    pub(crate) autosave: bool,         // write the named session on exit
    pub(crate) dot: bool,              // green unsaved mark on the save button
    pub(crate) tabs: Vec<Tab>,         // every tab's grid; the active one's grid is also the fields above
    pub(crate) tab: usize,             // which tab is on screen
    /// Third-party context-menu handlers turned off, by id. Values are target names
    /// (`file`, `folder`, `background`, `drive`). A missing id stays on. maguro.json only:
    /// a .grids file never carries this, and the system registry is never written.
    pub(crate) ctx_off: std::collections::BTreeMap<String, Vec<String>>,
    /// Menu item labels seen for a handler id. maguro.json only; the registry is never written.
    pub(crate) ctx_labels: std::collections::BTreeMap<String, Vec<String>>,
    /// Targets explicitly turned on for handlers that start off. WinMerge is the only one:
    /// a missing id stays hidden. maguro.json only.
    pub(crate) ctx_on: std::collections::BTreeMap<String, Vec<String>>,
    /// Handler ids that were asked and added no visible menu item. They leave the settings list.
    /// The shell menu is unchanged. maguro.json only.
    pub(crate) ctx_quiet: std::collections::BTreeSet<String>,
    /// Packaged Windows 11 commands the user turned on (`packaged:{clsid}`).
    /// Empty means every packaged command stays off. maguro.json only; a .grids file does not carry this.
    pub(crate) ctx_packaged_on: std::collections::BTreeSet<String>,
    /// One-time switch for configs that used to show every packaged command.
    /// Once true, `ctx_packaged_on` is kept and that switch does not run again.
    pub(crate) ctx_win11_pack_migrated: bool,
    /// Settings → Right-click menu is showing the Windows 11 list. maguro.json only.
    pub(crate) ctx_win11_view: bool,
    /// DLL file names that must not load inside this process. Empty means nothing extra is blocked.
    /// maguro.json only. The registry is not written, and a .grids file does not carry this.
    pub(crate) blocked_shell_extensions: Vec<String>,
    /// Paths kept in the file basket. maguro.json only. A .grids file does not carry this.
    /// Remove and Clear edit this list. They do not delete the files.
    pub(crate) file_basket: Vec<String>,
    /// Last "Tidy to A1 · 100%" checkboxes. Missing from maguro.json means A1/zoom/first on, backup off.
    /// A .grids file does not carry these.
    pub(crate) excel_tidy_a1: bool,
    pub(crate) excel_tidy_zoom: bool,
    pub(crate) excel_tidy_first: bool,
    pub(crate) excel_tidy_backup: bool,
}

/// Explorer's View menu, in its order. `(FOLDERVIEWMODE, icon size, label)`.
/// Large and medium icons share mode 1; the size tells them apart. 96 is large.
pub(crate) const EXPLORER_VIEWS: [(i32, i32, crate::Key); 8] = [
    (5, 256, crate::Key::ViewExtraLarge),
    (1, 96, crate::Key::ViewLarge),
    (1, 48, crate::Key::ViewMedium),
    (2, 16, crate::Key::ViewSmall),
    (3, 16, crate::Key::ViewList),
    (4, 16, crate::Key::ViewDetails),
    (6, 48, crate::Key::ViewTiles),
    (8, 32, crate::Key::ViewContent),
];

/// Details, list, the four icon sizes, tiles, content. Anything else is details.
pub(crate) fn known_view_mode(mode: i32) -> i32 {
    match mode {
        1 | 2 | 3 | 4 | 5 | 6 | 8 => mode,
        _ => 4,
    }
}

/// Icon size a new pane used before a size was stored next to the mode.
pub(crate) fn historical_view_icon(mode: i32) -> i32 {
    match known_view_mode(mode) {
        1 => 96,
        5 => 256,
        6 => 48,
        8 => 32,
        _ => 16,
    }
}

/// 0 when `icon` is the usual size for `mode`, so older settings files stay unchanged.
pub(crate) fn stored_view_icon(mode: i32, icon: i32) -> i32 {
    if icon > 0 && icon != historical_view_icon(mode) { icon } else { 0 }
}

/// Which Explorer view is on screen. Two presets that share a mode pick the nearer icon size.
/// A missing size (0) keeps the first preset of that mode, which is the historical one.
pub(crate) fn explorer_view_index(mode: i32, icon: i32) -> Option<usize> {
    let mut best: Option<(usize, i32)> = None;
    for (i, &(m, z, ..)) in EXPLORER_VIEWS.iter().enumerate() {
        if m != mode {
            continue;
        }
        let dist = if icon <= 0 { 0 } else { (icon - z).abs() };
        match best {
            Some((_, d)) if dist >= d => {}
            _ => best = Some((i, dist)),
        }
    }
    best.map(|(i, _)| i)
}

impl Default for Session {
    fn default() -> Self {
        let mut s = Session {
            win: None,
            side: 0,
            side_dpi: 0,
            heights: Vec::new(),
            colr: Vec::new(),
            rowr: Vec::new(),
            active: 0,
            paths: vec![String::new(); MAXP],
            views: vec![(0, 0); MAXP],
            accent: 0x0A9BD6, // cerulean brand; a chosen swatch replaces it
            accent_dark: 0x0A9BD6,
            dark: false,
            dblup: true,
            space_preview: true,
            peek_wheel_nav: false,
            remember_views: true,
            folder_sizes: true,
            folder_sizes_network: false,
            peek_image: None,
            peek_doc: None,
            pane_folders: vec![Vec::new(); MAXP],
            pane_cur: vec![0; MAXP],
            hotkey: true,
            dialog_jump: true,
            clip_hotkey: true,
            file: String::new(),
            favs: Vec::new(),
            tree: Vec::new(),
            recent: Vec::new(),
            lang: Lang::Auto,
            single: true,
            one_window: true,
            use_everything: true,
            winmerge_path: String::new(),
            ev_asked: false,
            ev_sort: 1,
            remote_asked: false,
            cols: vec![String::new(); MAXP],
            tints: vec![0; MAXP],
            order: Vec::new(),
            theme: 0,
            font_face: String::new(),
            bar: true,
            drive_buttons: true,
            stat: true,
            view_new: 4,
            view_icon: 0,
            start_new: 0,
            start_path: String::new(),
            ext: true,
            tree_on: true,
            preview_on: false,
            preview_w: 0,
            fav_top: true,
            side_split: 0.5,
            pins_open: true,
            files_open: true,
            edge: true,
            confirm: true,
            del: 0,
            new_name: String::new(),
            dropm: 0,
            fast_copy: false,
            layout_new: 3,
            boot: 0,
            boot_file: String::new(),
            autosave: false,
            dot: true,
            update_check: true,
            tabs: Vec::new(),
            tab: 0,
            ctx_off: std::collections::BTreeMap::new(),
            ctx_labels: std::collections::BTreeMap::new(),
            ctx_on: std::collections::BTreeMap::new(),
            ctx_quiet: std::collections::BTreeSet::new(),
            ctx_packaged_on: std::collections::BTreeSet::new(),
            ctx_win11_pack_migrated: true,
            ctx_win11_view: false,
            blocked_shell_extensions: Vec::new(),
            file_basket: Vec::new(),
            excel_tidy_a1: true,
            excel_tidy_zoom: true,
            excel_tidy_first: true,
            excel_tidy_backup: false,
        };
        // A first-ever start: layout_new 3 is one pane. Saved maguro.json without layout_new stays 0 (two panes).
        s.set_heights(s.layout_heights());
        s.anchor();
        s
    }
}

impl PartialEq for Session {
    fn eq(&self, o: &Self) -> bool {
        self.win == o.win
            && self.side == o.side
            && self.heights == o.heights
            && self.colr == o.colr
            && self.rowr == o.rowr
            && self.active == o.active
            && self.paths == o.paths
            && self.views == o.views
            && self.accent == o.accent
            && self.accent_dark == o.accent_dark
            && self.dark == o.dark
            && self.dblup == o.dblup
            && self.space_preview == o.space_preview
            && self.peek_wheel_nav == o.peek_wheel_nav
            && self.remember_views == o.remember_views
            && self.folder_sizes == o.folder_sizes
            && self.folder_sizes_network == o.folder_sizes_network
            && self.peek_image == o.peek_image
            && self.peek_doc == o.peek_doc
            && self.hotkey == o.hotkey
            && self.dialog_jump == o.dialog_jump
            && self.clip_hotkey == o.clip_hotkey
            && self.file == o.file
            && self.favs == o.favs
            && self.tree == o.tree
            && self.recent == o.recent
            && self.lang == o.lang
            && self.single == o.single
            && self.one_window == o.one_window
            && self.use_everything == o.use_everything
            && self.winmerge_path == o.winmerge_path
            && self.ev_asked == o.ev_asked
            && self.ev_sort == o.ev_sort
            && self.remote_asked == o.remote_asked
            && self.cols == o.cols
            && self.tints == o.tints
            && self.theme == o.theme
            && self.font_face == o.font_face
            && self.bar == o.bar
            && self.drive_buttons == o.drive_buttons
            && self.stat == o.stat
            && self.view_new == o.view_new
            && self.view_icon == o.view_icon
            && self.start_new == o.start_new
            && self.start_path == o.start_path
            && self.ext == o.ext
            && self.tree_on == o.tree_on
            && self.preview_on == o.preview_on
            && self.preview_w == o.preview_w
            && self.fav_top == o.fav_top
            && self.side_split == o.side_split
            && self.pins_open == o.pins_open
            && self.files_open == o.files_open
            && self.edge == o.edge
            && self.confirm == o.confirm
            && self.del == o.del
            && self.new_name == o.new_name
            && self.dropm == o.dropm
            && self.fast_copy == o.fast_copy
            && self.layout_new == o.layout_new
            && self.boot == o.boot
            && self.boot_file == o.boot_file
            && self.autosave == o.autosave
            && self.dot == o.dot
            && self.update_check == o.update_check
            && self.tab == o.tab
            && self.view_tabs() == o.view_tabs()
            && self.ctx_off == o.ctx_off
            && self.ctx_labels == o.ctx_labels
            && self.ctx_on == o.ctx_on
            && self.ctx_quiet == o.ctx_quiet
            && self.ctx_packaged_on == o.ctx_packaged_on
            && self.ctx_win11_pack_migrated == o.ctx_win11_pack_migrated
            && self.ctx_win11_view == o.ctx_win11_view
            && self.blocked_shell_extensions == o.blocked_shell_extensions
            && self.file_basket == o.file_basket
            && self.excel_tidy_a1 == o.excel_tidy_a1
            && self.excel_tidy_zoom == o.excel_tidy_zoom
            && self.excel_tidy_first == o.excel_tidy_first
            && self.excel_tidy_backup == o.excel_tidy_backup
    }
}

/// Folder tree's share of the side plate. Clamps to [0, 1]; NaN becomes an even split.
/// Layout uses this while dragging so a raw 0/1 still resolves through `side_min`.
pub(crate) fn sane_split(v: f64) -> f64 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.5 }
}

/// Share read from a saved config or session file.
/// A real splitter drag never persists exact 0 or 1 (each half keeps at least `side_min`),
/// so those extremes — and any out-of-range value — mean "even split".
pub(crate) fn persist_split(v: f64) -> f64 {
    let v = sane_split(v);
    if v <= 0.0 || v >= 1.0 { 0.5 } else { v }
}

/// Favourite entries are `path` or `path<tab>display name`. The path is what gets opened.
pub(crate) fn fav_path(f: &str) -> &str {
    f.split('\t').next().unwrap_or(f)
}

pub(crate) fn fav_label(f: &str) -> String {
    match f.split_once('\t') {
        Some((_, n)) if !n.is_empty() => n.to_string(),
        _ => short_name(fav_path(f)),
    }
}

pub(crate) fn even(n: usize) -> Vec<f64> {
    vec![1.0 / n as f64; n]
}

/// Panes per column for the settings default (`layout_new`).
/// A new tab, a new window, the first start, and startup's "default layout" use this with even splits.
/// 0 side by side, 1 top and bottom, 2 is 2×2, 3 one pane. Above 3 is one pane. The fresh default is 3.
pub(crate) fn layout_heights_for(layout_new: u8) -> Vec<usize> {
    match layout_new.min(3) {
        1 => vec![2],
        2 => vec![2, 2],
        3 => vec![1],
        _ => vec![1, 1],
    }
}

/// Pane counts and even splits a new window tab takes from the settings default (`layout_new`).
/// Not the tab on screen. A path handed over by another launch is still one pane (`fresh_tab_session`).
pub(crate) fn layout_for_new_tab(layout_new: u8) -> (Vec<usize>, Vec<f64>, Vec<Vec<f64>>) {
    let heights = layout_heights_for(layout_new);
    let colr = even(heights.len());
    let rowr = heights.iter().map(|&n| even(n)).collect();
    (heights, colr, rowr)
}

pub(crate) fn nums<T: std::str::FromStr>(v: &str) -> Vec<T> {
    v.split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

pub(crate) fn join<T: Display>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

// Ratios from a hand-edited ini can be anything: wrong length, zero, negative -> fall back to equal.
pub(crate) fn fix(r: Vec<f64>, n: usize) -> Vec<f64> {
    // NaN, infinity, and ratios whose sum overflows all fall back to an even split.
    let sum: f64 = r.iter().sum();
    if r.len() != n || r.iter().any(|x| !(*x > 0.0 && x.is_finite())) || !sum.is_finite() {
        return even(n);
    }
    r.iter().map(|x| x / sum).collect()
}

// "2 × 2" for an even grid, "1 | 3" when the columns differ.
pub(crate) fn layout_label(h: &[usize]) -> String {
    let Some(&first) = h.first() else { return String::new() };
    match h.iter().all(|&n| n == first) {
        true => format!("{} × {}", first, h.len()),
        false => h.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" | "),
    }
}

impl Session {
    /// Tabs as saved: the active tab's grid is the live fields, its name and lock stay.
    pub(crate) fn view_tabs(&self) -> Vec<Tab> {
        let mut tabs = if self.tabs.is_empty() {
            vec![Tab::from_live(self, "1".into(), false, vec![String::new(); MAXP])]
        } else {
            self.tabs.clone()
        };
        let i = self.tab.min(tabs.len() - 1);
        let name = tabs[i].name.clone();
        let locked = tabs[i].locked;
        let color = tabs[i].color;
        let session = tabs[i].session.clone();
        let filters = tabs[i].filters.clone();
        tabs[i] = Tab::from_live(self, name, locked, filters);
        tabs[i].color = color;
        tabs[i].session = session;
        tabs
    }

    /// Write the live grid into the active tab. An empty list becomes one tab named 1.
    pub(crate) fn anchor(&mut self) {
        if self.tabs.is_empty() {
            self.tabs.push(Tab::from_live(self, "1".into(), false, vec![String::new(); MAXP]));
            self.tab = 0;
            settle_sessions(self);
            return;
        }
        if self.tab >= self.tabs.len() {
            self.tab = 0;
        }
        let name = self.tabs[self.tab].name.clone();
        let locked = self.tabs[self.tab].locked;
        let color = self.tabs[self.tab].color;
        let session = self.tabs[self.tab].session.clone();
        let filters = pad_filters(std::mem::take(&mut self.tabs[self.tab].filters));
        self.tabs[self.tab] = Tab::from_live(self, name, locked, filters);
        self.tabs[self.tab].color = color;
        self.tabs[self.tab].session = session;
        settle_sessions(self);
    }

    /// Copy the active tab onto the live grid fields.
    pub(crate) fn publish(&mut self) {
        if self.tabs.is_empty() {
            self.anchor();
            return;
        }
        self.tab = self.tab.min(self.tabs.len() - 1);
        let t = self.tabs[self.tab].clone();
        self.heights = t.heights;
        self.colr = t.colr;
        self.rowr = t.rowr;
        let n = self.heights.iter().sum::<usize>().max(1);
        self.active = t.active.min(n - 1);
        self.paths = t.paths;
        self.views = t.views;
        self.cols = t.cols;
        self.tints = t.tints;
        self.pane_folders = pad_pane_folders(t.pane_folders);
        self.pane_cur = pad_pane_cur(t.pane_cur);
        settle_sessions(self);
    }

    pub(crate) fn accent_now(&self) -> u32 {
        if self.dark {
            self.accent_dark
        } else {
            self.accent
        }
    }

    pub(crate) fn panes(&self) -> usize {
        self.heights.iter().sum()
    }

    /// Panes per column for a new tab or a new window (the settings default).
    pub(crate) fn layout_heights(&self) -> Vec<usize> {
        layout_heights_for(self.layout_new)
    }

    /// New layout with even splits. The active pane stays inside the new grid.
    pub(crate) fn set_heights(&mut self, h: Vec<usize>) {
        // An empty or zero column would make panes() 0 and underflow below; keep at least one pane.
        let h: Vec<usize> = h.into_iter().filter(|&n| n > 0).collect();
        let h = if h.is_empty() { vec![1] } else { h };
        self.colr = even(h.len());
        self.rowr = h.iter().map(|&n| even(n)).collect();
        self.heights = h;
        self.active = self.active.min(self.panes() - 1);
    }

    /// Another language drops a chosen face so that language's default is used.
    pub(crate) fn choose_lang(&mut self, lang: Lang) {
        if self.lang != lang {
            self.font_face.clear();
        }
        self.lang = lang;
    }

    pub(crate) fn parse(text: &str) -> Session {
        let mut s = Session::default();
        // Older ini without layout= meant two panes. Fresh Session::default is one pane (3).
        s.layout_new = 0;
        s.set_heights(layout_heights_for(0));
        s.tabs.clear();
        let mut rowr: Vec<Vec<f64>> = Vec::new();
        let mut pane_b: Vec<Option<(usize, Vec<String>)>> = vec![None; MAXP];
        let (mut saw_theme, mut saw_dark) = (false, false);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let slot = |p: char| k.strip_prefix(p).and_then(|n| n.parse::<usize>().ok()).filter(|&i| i < MAXP);
            match k {
                "win" => s.win = nums(v).try_into().ok(),
                "side" => s.side = v.parse().unwrap_or(0),
                "sidedpi" => s.side_dpi = v.parse().unwrap_or(0),
                "split" => s.side_split = persist_split(v.parse().unwrap_or(0.5)),
                "cols" => {
                    let h: Vec<usize> = nums::<usize>(v).into_iter().take(4).map(|n| n.clamp(1, 4)).collect();
                    if !h.is_empty() {
                        s.heights = h;
                    }
                }
                // older ini: even rows x cols grid
                "grid" => {
                    if let [r, c] = nums::<usize>(v)[..] {
                        s.heights = vec![r.clamp(1, 4); c.clamp(1, 4)];
                    }
                }
                "colr" => s.colr = nums(v),
                "rowr" => rowr = v.split('|').map(nums).collect(),
                "active" => s.active = v.parse().unwrap_or(0),
                "order" => {
                    let o: Vec<u8> = nums(v);
                    if o.len() == MAXP && o.iter().all(|&n| (n as usize) < MAXP) {
                        s.order = o;
                    }
                }
                "accent" => s.accent = u32::from_str_radix(v.trim().trim_start_matches('#'), 16).unwrap_or(s.accent) & 0xFFFFFF,
                "accent_dark" => s.accent_dark = u32::from_str_radix(v.trim(), 16).unwrap_or(s.accent_dark) & 0xFFFFFF,
                "dark" => {
                    s.dark = v.trim() == "1";
                    saw_dark = true;
                }
                "theme" => {
                    s.theme = match v.trim() {
                        "light" => 1,
                        "dark" => 2,
                        "auto" => 0,
                        _ => s.theme,
                    };
                    saw_theme = true;
                }
                // Retired UI scale (0/1/2). An older grids.ini still loads; the key is not written back.
                "font" => {}
                "bar" => s.bar = v.trim() != "0",
                "drivebuttons" => s.drive_buttons = v.trim() != "0",
                "stat" => s.stat = v.trim() != "0",
                "viewnew" => s.view_new = match v.trim() {
                    "list" => 3,
                    "icon" => 1,
                    // The number is the same key grids.ini already had. No new key is written.
                    other => known_view_mode(other.parse().unwrap_or(4)),
                },
                "start" => s.start_new = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "startp" => s.start_path = v.trim().to_string(),
                "ext" => s.ext = v.trim() != "0",
                "tree" => s.tree_on = v.trim() != "0",
                "previewpane" => s.preview_on = v.trim() == "1",
                "previeww" => {
                    let n = v.trim().parse::<i32>().unwrap_or(0);
                    s.preview_w = if n <= 0 { 0 } else { n.clamp(160, 4000) };
                }
                // Omitted, or any value other than tree: favourites on top. The source grids.ini is never rewritten.
                "sideorder" => s.fav_top = v.trim() != "tree",
                "edge" => s.edge = v.trim() != "0",
                "confirm" => s.confirm = v.trim() != "0",
                "del" => s.del = if v.trim() == "1" { 1 } else { 0 },
                "newname" => s.new_name = v.trim().to_string(),
                "drop" => s.dropm = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "fastcopy" => s.fast_copy = v.trim() == "1",
                "layout" => s.layout_new = v.trim().parse::<u8>().unwrap_or(0).min(3),
                "boot" => s.boot = v.trim().parse::<u8>().unwrap_or(0).min(2),
                "bootf" => s.boot_file = v.trim().to_string(),
                "autosave" => s.autosave = v.trim() == "1",
                "dot" => s.dot = v.trim() != "0",
                "upcheck" => s.update_check = v.trim() != "0",
                "dblup" => s.dblup = v.trim() != "0",
                "panetabs" => {} // older files; pane tabs are always on
                "spacepreview" => s.space_preview = v.trim() != "0",
                "peekwheelnav" => s.peek_wheel_nav = v.trim() == "1",
                "folderview" => s.remember_views = v.trim() != "0",
                "foldersize" => s.folder_sizes = v.trim() != "0",
                "foldersizenet" => s.folder_sizes_network = v.trim() == "1",
                "hotkey" => s.hotkey = v.trim() != "0",
                "dialogjump" => s.dialog_jump = v.trim() != "0",
                "cliphotkey" => s.clip_hotkey = v.trim() != "0",
                "lang" => s.lang = Lang::from_ini(v),
                "single" => s.single = v.trim() != "0",
                "onewindow" => s.one_window = v.trim() != "0",
                "everything" => s.use_everything = v.trim() != "0",
                "winmerge" => s.winmerge_path = v.trim().to_string(),
                "evasked" => s.ev_asked = v.trim() == "1",
                "remoteasked" => s.remote_asked = v.trim() == "1",
                "session" => s.file = v.trim().to_string(),
                "tabs" => {
                    if let Some((tabs, active)) = tabs_from_json(v) {
                        s.tabs = tabs;
                        s.tab = active;
                    }
                }
                "fav" if !v.trim().is_empty() => s.favs.push(v.trim().to_string()),
                "recent" if !v.trim().is_empty() && s.recent.len() < RECENT => s.recent.push(v.trim().to_string()),
                _ => {
                    if let Some(i) = slot('p') {
                        s.paths[i] = v.to_string();
                    } else if let (Some(i), [m, z]) = (slot('v'), &nums::<i32>(v)[..]) {
                        s.views[i] = (*m, *z);
                    } else if let Some(i) = slot('c') {
                        s.cols[i] = v.trim().to_string();
                    } else if let Some(i) = slot('b') {
                        // b3=1|C:\a|D:\b : current pane tab, then every folder ('|' is not used in a path)
                        let mut it = v.split('|');
                        let cur = it.next().and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(0);
                        let list: Vec<String> = it.filter(|p| !p.is_empty()).map(str::to_string).collect();
                        if list.len() > 1 {
                            pane_b[i] = Some((cur.min(list.len() - 1), list));
                        }
                    } else if let Some(i) = slot('t') {
                        s.tints[i] = v.trim().parse().ok().filter(|&k| k <= 6).unwrap_or(0);
                    }
                }
            }
        }
        s.colr = fix(std::mem::take(&mut s.colr), s.heights.len());
        // one group per column; an older single group applies to every column
        let group = |c: usize| if rowr.len() == s.heights.len() { rowr.get(c) } else { rowr.first() };
        s.rowr = s.heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
        if s.active >= s.panes() {
            s.active = 0;
        }
        // An older ini has dark= and no theme=. That stays an explicit light or dark choice.
        if !saw_theme && saw_dark {
            s.theme = if s.dark { 2 } else { 1 };
        }
        if s.tabs.is_empty() {
            s.anchor();
        } else {
            s.publish();
        }
        // b-lines fill a pane whose window-tab record has no pane tabs (an older file, or a hand edit).
        let mut folded = false;
        for (i, slot) in pane_b.into_iter().enumerate() {
            let Some((cur, list)) = slot else { continue };
            if s.pane_folders[i].len() > 1 {
                continue;
            }
            s.pane_cur[i] = cur;
            s.pane_folders[i] = list;
            folded = true;
        }
        if folded {
            s.anchor();
        }
        s.tree = super::tree_from_flat(&s.favs);
        s.accent = crate::brand_accent(s.accent);
        s.accent_dark = crate::brand_accent(s.accent_dark);
        s
    }

    pub(crate) fn to_ini(&self) -> String {
        let mut o = String::new();
        if let Some(w) = self.win {
            o += &format!("win={}\n", join(&w));
        }
        let rowr = self.rowr.iter().map(|r| join(r)).collect::<Vec<_>>().join("|");
        o += &format!("sidedpi={}\n", self.side_dpi);
        o += &format!("side={}\nsplit={}\ncols={}\n", self.side, self.side_split, join(&self.heights));
        o += &format!("colr={}\nrowr={rowr}\nactive={}\n", join(&self.colr), self.active);
        o += &format!("accent={:06X}\naccent_dark={:06X}\ndark={}\n", self.accent, self.accent_dark, self.dark as u8);
        if !self.order.is_empty() {
            o += &format!("order={}\n", join(&self.order));
        }
        let theme = ["auto", "light", "dark"][self.theme.min(2) as usize];
        o += &format!("theme={theme}\nbar={}\ndrivebuttons={}\nstat={}\n", self.bar as u8, self.drive_buttons as u8, self.stat as u8);
        o += &format!("viewnew={}\nstart={}\n", self.view_new, self.start_new);
        if !self.start_path.is_empty() {
            o += &format!("startp={}\n", self.start_path);
        }
        o += &format!("ext={}\ntree={}\nedge={}\n", self.ext as u8, self.tree_on as u8, self.edge as u8);
        if self.preview_on {
            o += "previewpane=1\n";
        }
        if self.preview_w > 0 {
            o += &format!("previeww={}\n", self.preview_w);
        }
        // Default (favourites on top) adds no line, so an imported grids.ini is not given a new key.
        if !self.fav_top {
            o += "sideorder=tree\n";
        }
        o += &format!("confirm={}\ndel={}\n", self.confirm as u8, self.del);
        if !self.new_name.is_empty() {
            o += &format!("newname={}\n", self.new_name);
        }
        o += &format!("drop={}\nlayout={}\nboot={}\n", self.dropm, self.layout_new, self.boot);
        if self.fast_copy {
            o += "fastcopy=1\n";
        }
        if !self.boot_file.is_empty() {
            o += &format!("bootf={}\n", self.boot_file);
        }
        o += &format!("autosave={}\ndot={}\nupcheck={}\n", self.autosave as u8, self.dot as u8, self.update_check as u8);
        o += &format!("dblup={}\nhotkey={}\nlang={}\nsingle={}\nonewindow={}\neverything={}\nevasked={}\nremoteasked={}\nspacepreview={}\npeekwheelnav={}\nfolderview={}\nfoldersize={}\nfoldersizenet={}\ndialogjump={}\ncliphotkey={}\n", self.dblup as u8, self.hotkey as u8, self.lang.ini(), self.single as u8, self.one_window as u8, self.use_everything as u8, self.ev_asked as u8, self.remote_asked as u8, self.space_preview as u8, self.peek_wheel_nav as u8, self.remember_views as u8, self.folder_sizes as u8, self.folder_sizes_network as u8, self.dialog_jump as u8, self.clip_hotkey as u8);
        if !self.winmerge_path.trim().is_empty() {
            o += &format!("winmerge={}\n", self.winmerge_path.trim());
        }
        if !self.file.is_empty() {
            o += &format!("session={}\n", self.file);
        }
        for f in &self.favs {
            o += &format!("fav={f}\n");
        }
        for r in &self.recent {
            o += &format!("recent={r}\n");
        }
        for i in 0..MAXP {
            if !self.paths[i].is_empty() {
                o += &format!("p{i}={}\n", self.paths[i]);
            }
            if self.views[i].0 != 0 {
                o += &format!("v{i}={},{}\n", self.views[i].0, self.views[i].1);
            }
            if !self.cols[i].is_empty() {
                o += &format!("c{i}={}\n", self.cols[i]);
            }
            if self.tints[i] != 0 {
                o += &format!("t{i}={}\n", self.tints[i]);
            }
            let folders = self.pane_tab_list(i);
            if folders.len() > 1 {
                let cur = self.pane_cur[i].min(folders.len() - 1);
                o += &format!("b{i}={cur}|{}\n", folders.join("|"));
            }
        }
        let tabs = self.view_tabs();
        let active = self.tab.min(tabs.len() - 1);
        o += &format!("tabs={}\n", tabs_json(&tabs, active));
        o
    }

    pub(crate) fn is_fav(&self, path: &str) -> bool {
        !path.is_empty()
            && (super::contains_path(&self.tree, path) || self.favs.iter().any(|f| fav_path(f).eq_ignore_ascii_case(path)))
    }

    pub(crate) fn default_view(&self) -> (i32, i32) {
        let mode = known_view_mode(self.view_new);
        let icon = if self.view_icon > 0 { self.view_icon } else { historical_view_icon(mode) };
        (mode, icon)
    }

    /// What goes into a named session file: no window position, no pointer to itself, no favourites or recent folders.
    /// Per-tab session paths stay on the window. A `.grids` file is not tied to the place it was written.
    pub(crate) fn snapshot(&self) -> String {
        let mut c = self.clone();
        c.anchor();
        c.win = None;
        c.file.clear();
        for t in &mut c.tabs {
            t.session.clear();
        }
        c.favs.clear();
        c.tree.clear();
        c.recent.clear();
        c.order.clear();
        c.to_ini()
    }

    /// Takes every tab from a named session; window position and sidebar width stay as they are.
    pub(crate) fn take_layout(&mut self, o: &Session) {
        (self.heights, self.colr, self.rowr) = (o.heights.clone(), o.colr.clone(), o.rowr.clone());
        (self.active, self.paths, self.views, self.cols) = (o.active, o.paths.clone(), o.views.clone(), o.cols.clone());
        self.tints = o.tints.clone();
        self.pane_folders = pad_pane_folders(o.pane_folders.clone());
        self.pane_cur = pad_pane_cur(o.pane_cur.clone());
        self.tabs = o.view_tabs();
        self.tab = o.tab.min(self.tabs.len().saturating_sub(1));
        self.order.clear();
    }

    /// Slots i and j trade folders, views, columns, labels and the active tab's filter.
    /// The active pane goes with its slot.
    pub(crate) fn swap_slots(&mut self, i: usize, j: usize) {
        if i >= MAXP || j >= MAXP || i == j {
            return;
        }
        self.ensure_pane_tabs();
        self.paths.swap(i, j);
        self.views.swap(i, j);
        self.cols.swap(i, j);
        self.tints.swap(i, j);
        self.pane_folders.swap(i, j);
        self.pane_cur.swap(i, j);
        if self.tab < self.tabs.len() && i < self.tabs[self.tab].filters.len() && j < self.tabs[self.tab].filters.len() {
            self.tabs[self.tab].filters.swap(i, j);
        }
        if self.active == i {
            self.active = j;
        } else if self.active == j {
            self.active = i;
        }
    }

    /// Slot i takes what slot `order[i]` held. An empty or identity order leaves everything where it is.
    pub(crate) fn reorder(&mut self, order: &[u8]) {
        if order.len() != MAXP || !order_moved(order) || order.iter().any(|&k| k as usize >= MAXP) {
            return;
        }
        let old = self.clone();
        for (i, &k) in order.iter().enumerate() {
            let k = k as usize;
            self.paths[i] = old.paths[k].clone();
            self.views[i] = old.views[k];
            self.cols[i] = old.cols[k].clone();
            self.tints[i] = old.tints[k];
            self.pane_folders[i] = old.pane_folders.get(k).cloned().unwrap_or_default();
            self.pane_cur[i] = old.pane_cur.get(k).copied().unwrap_or(0);
        }
        if self.tab < self.tabs.len() {
            let old_f = old.tabs[self.tab].filters.clone();
            for (i, &k) in order.iter().enumerate() {
                let k = k as usize;
                if i < self.tabs[self.tab].filters.len() && k < old_f.len() {
                    self.tabs[self.tab].filters[i] = old_f[k].clone();
                }
            }
        }
    }

    pub(crate) fn ensure_pane_tabs(&mut self) {
        self.pane_folders = pad_pane_folders(std::mem::take(&mut self.pane_folders));
        self.pane_cur = pad_pane_cur(std::mem::take(&mut self.pane_cur));
    }

    /// Pane i's pane tabs as they are now. The current one is the folder on display. One entry when it has none.
    pub(crate) fn pane_tab_list(&self, i: usize) -> Vec<String> {
        if i >= MAXP {
            return Vec::new();
        }
        let mut t = self.pane_folders.get(i).cloned().unwrap_or_default();
        let cur = self.pane_cur.get(i).copied().unwrap_or(0);
        let path = self.paths.get(i).cloned().unwrap_or_default();
        match cur {
            k if t.len() > 1 && k < t.len() => t[k] = path,
            _ => t = vec![path],
        }
        t
    }

    /// The folder on display in `src` becomes a new pane tab on `dst`, just after the tab on display, and is made current.
    /// Only that folder is copied: `src`'s other pane tabs stay, and `dst` keeps the tabs it already had.
    /// None when `dst` is already showing that folder (nothing to browse). The caller does not clone a back/forward stack.
    pub(crate) fn copy_pane_tab(&mut self, src: usize, dst: usize) -> Option<String> {
        if src >= MAXP || dst >= MAXP {
            return None;
        }
        self.ensure_pane_tabs();
        let path = self.paths.get(src).cloned().unwrap_or_default();
        let mut t = self.pane_tab_list(dst);
        if t.is_empty() {
            t.push(self.paths.get(dst).cloned().unwrap_or_default());
        }
        let cur = if t.len() > 1 { self.pane_cur[dst].min(t.len() - 1) } else { 0 };
        let at = cur + 1;
        t.insert(at, path.clone());
        let showing = self.paths.get(dst).is_some_and(|p| p == &path);
        (self.pane_folders[dst], self.pane_cur[dst]) = (t, at);
        if showing { None } else { Some(path) }
    }

    /// A copy of the current pane tab, placed just after it and made current. Same folder, so nothing browses.
    pub(crate) fn add_pane_tab(&mut self, i: usize) {
        if i >= MAXP {
            return;
        }
        self.ensure_pane_tabs();
        let mut t = self.pane_tab_list(i);
        let k = self.pane_cur[i].min(t.len() - 1) + 1;
        t.insert(k, self.paths[i].clone());
        (self.pane_folders[i], self.pane_cur[i]) = (t, k);
    }

    /// Pane tab k becomes current. The folder to browse to, or None when it already is (or there is no such tab).
    pub(crate) fn pick_pane_tab(&mut self, i: usize, k: usize) -> Option<String> {
        if i >= MAXP {
            return None;
        }
        self.ensure_pane_tabs();
        let t = self.pane_tab_list(i);
        let cur = if t.len() > 1 { self.pane_cur[i].min(t.len() - 1) } else { 0 };
        if k >= t.len() || k == cur {
            return None;
        }
        let to = t[k].clone();
        (self.pane_folders[i], self.pane_cur[i]) = (t, k);
        Some(to)
    }

    /// Closes pane tab k. The last one stays. Closing the current one moves to its right neighbour, else its left.
    pub(crate) fn close_pane_tab(&mut self, i: usize, k: usize) -> Option<String> {
        if i >= MAXP {
            return None;
        }
        self.ensure_pane_tabs();
        let mut t = self.pane_tab_list(i);
        if t.len() < 2 || k >= t.len() {
            return None;
        }
        let cur = self.pane_cur[i].min(t.len() - 1);
        t.remove(k);
        let (next, go) = match k.cmp(&cur) {
            std::cmp::Ordering::Less => (cur - 1, None),
            std::cmp::Ordering::Greater => (cur, None),
            std::cmp::Ordering::Equal => {
                let n = k.min(t.len() - 1);
                (n, Some(t[n].clone()))
            }
        };
        if t.len() > 1 {
            (self.pane_folders[i], self.pane_cur[i]) = (t, next);
        } else {
            self.pane_folders[i].clear();
            self.pane_cur[i] = 0;
        }
        go
    }
}

fn order_moved(order: &[u8]) -> bool {
    order.iter().enumerate().any(|(k, &o)| k != o as usize)
}

fn pad_tab_slots(t: &mut Tab) {
    t.paths.resize(MAXP, String::new());
    t.views.resize(MAXP, (0, 0));
    t.cols.resize(MAXP, String::new());
    t.tints.resize(MAXP, 0);
    t.pane_folders.resize(MAXP, Vec::new());
    t.pane_cur.resize(MAXP, 0);
    t.filters.resize(MAXP, String::new());
    t.paths.truncate(MAXP);
    t.views.truncate(MAXP);
    t.cols.truncate(MAXP);
    t.tints.truncate(MAXP);
    t.pane_folders.truncate(MAXP);
    t.pane_cur.truncate(MAXP);
    t.filters.truncate(MAXP);
}

/// The active tab's clean grid follows a pane swap, the same way `swap_in` moves the old window baseline.
/// Views and columns stay lined up, so the save dot is the permutation (`order`), not a false layout change.
pub(crate) fn swap_tab_slots(t: &mut Tab, i: usize, j: usize) {
    if i >= MAXP || j >= MAXP || i == j {
        return;
    }
    pad_tab_slots(t);
    t.paths.swap(i, j);
    t.views.swap(i, j);
    t.cols.swap(i, j);
    t.tints.swap(i, j);
    t.pane_folders.swap(i, j);
    t.pane_cur.swap(i, j);
    t.filters.swap(i, j);
}

/// `order[slot]` is the slot whose pane lands there. Same rule as `Session::reorder`.
pub(crate) fn reorder_tab_slots(t: &mut Tab, order: &[u8]) {
    if order.len() != MAXP || !order_moved(order) || order.iter().any(|&k| k as usize >= MAXP) {
        return;
    }
    pad_tab_slots(t);
    let old = t.clone();
    for (i, &k) in order.iter().enumerate() {
        let k = k as usize;
        t.paths[i] = old.paths[k].clone();
        t.views[i] = old.views[k];
        t.cols[i] = old.cols[k].clone();
        t.tints[i] = old.tints[k];
        t.pane_folders[i] = old.pane_folders[k].clone();
        t.pane_cur[i] = old.pane_cur[k];
        t.filters[i] = old.filters[k].clone();
    }
}

/// Panes i and j trade places in the current state and in the save baseline, so the two still line up.
/// `order` notes the move, which the session file does not know about.
pub(crate) fn swap_in(cur: &mut Session, base: Option<&mut Session>, i: usize, j: usize) {
    cur.swap_slots(i, j);
    if let Some(b) = base {
        b.swap_slots(i, j);
    }
    if cur.order.is_empty() {
        cur.order = (0..MAXP as u8).collect();
    }
    cur.order.swap(i, j);
    if !order_moved(&cur.order) {
        cur.order.clear();
    }
}

/// Every slot takes what `perm[slot]` held, in the current state and the save baseline.
/// `order` records the move the same way a single swap does.
pub(crate) fn permute_in(cur: &mut Session, base: Option<&mut Session>, perm: &[usize; MAXP]) {
    let p = perm.map(|k| k as u8);
    for s in std::iter::once(&mut *cur).chain(base) {
        let act = s.active;
        s.reorder(&p);
        s.active = perm.iter().position(|&k| k == act).unwrap_or(0);
    }
    if cur.order.is_empty() {
        cur.order = (0..MAXP as u8).collect();
    }
    cur.order = p.iter().map(|&k| cur.order[k as usize]).collect();
    if !order_moved(&cur.order) {
        cur.order.clear();
    }
}

/// The save dot: panes moved from where the session file has them, or a layout change.
pub(crate) fn session_dirty(cur: &Session, base: Option<&Session>) -> bool {
    order_moved(&cur.order) || base.is_some_and(|b| layout_differs(cur, b))
}

/// Where Save writes for the tab on screen. `AskName` uses `ask_session_path`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SaveChoice<'a> {
    Overwrite(&'a str),
    AskName,
}

pub(crate) fn save_choice(tabs: &[Tab], active: usize) -> SaveChoice<'_> {
    match tabs.get(active).map(|t| t.session.as_str()).filter(|p| !p.is_empty()) {
        Some(p) => SaveChoice::Overwrite(p),
        None => SaveChoice::AskName,
    }
}

/// Legacy window path → the active tab only, when no tab has a path of its own yet.
/// Then `file` mirrors the tab on screen. A later save or load touches only that tab.
pub(crate) fn settle_sessions(s: &mut Session) {
    let any = s.tabs.iter().any(|t| !t.session.is_empty());
    if !any && !s.file.is_empty() && !s.tabs.is_empty() {
        let i = s.tab.min(s.tabs.len() - 1);
        s.tabs[i].session = std::mem::take(&mut s.file);
    }
    if !s.tabs.is_empty() {
        let i = s.tab.min(s.tabs.len() - 1);
        s.file = s.tabs[i].session.clone();
    }
}

/// Point tab `i` at `path`. Every other tab keeps the path it had.
pub(crate) fn bind_tab_session(s: &mut Session, i: usize, path: &str) -> bool {
    let Some(t) = s.tabs.get_mut(i) else { return false };
    t.session = path.to_string();
    settle_sessions(s);
    true
}

/// A whole-window load (startup, boot file, new window). Only the active tab is linked to `path`.
pub(crate) fn link_loaded_window(s: &mut Session, path: &str) {
    for t in &mut s.tabs {
        t.session.clear();
    }
    if !s.tabs.is_empty() && !path.is_empty() {
        let i = s.tab.min(s.tabs.len() - 1);
        s.tabs[i].session = path.to_string();
    }
    s.file = path.to_string();
    settle_sessions(s);
}

/// Put one saved grid on the live fields. The tab list, names, and session links stay put.
pub(crate) fn apply_tab_grid(s: &mut Session, t: &Tab) {
    let heights: Vec<usize> = t.heights.iter().copied().filter(|&n| n > 0).collect();
    s.heights = if heights.is_empty() { vec![1] } else { heights };
    s.colr = t.colr.clone();
    s.rowr = t.rowr.clone();
    let n = s.heights.iter().sum::<usize>().max(1);
    s.active = t.active.min(n - 1);
    let mut paths = t.paths.clone();
    let mut views = t.views.clone();
    let mut cols = t.cols.clone();
    let mut tints = t.tints.clone();
    paths.resize(MAXP, String::new());
    views.resize(MAXP, (0, 0));
    cols.resize(MAXP, String::new());
    tints.resize(MAXP, 0);
    paths.truncate(MAXP);
    views.truncate(MAXP);
    cols.truncate(MAXP);
    tints.truncate(MAXP);
    s.paths = paths;
    s.views = views;
    s.cols = cols;
    s.tints = tints;
    s.pane_folders = pad_pane_folders(t.pane_folders.clone());
    s.pane_cur = pad_pane_cur(t.pane_cur.clone());
}

/// Load `incoming` into tab `i` and link that tab to `path`. Other tabs' links are unchanged.
/// The loaded grid replaces the tab's panes. Its name, lock, and stripe stay.
pub(crate) fn load_into_tab(s: &mut Session, i: usize, incoming: &Tab, path: &str) -> Vec<String> {
    if s.tabs.is_empty() {
        s.anchor();
    }
    if i >= s.tabs.len() {
        return s.tabs.iter().map(|t| t.session.clone()).collect();
    }
    let (name, locked, color) = {
        let t = &s.tabs[i];
        (t.name.clone(), t.locked, t.color)
    };
    let others: Vec<(usize, String)> = s.tabs.iter().enumerate().filter(|(k, _)| *k != i).map(|(k, t)| (k, t.session.clone())).collect();
    if i == s.tab.min(s.tabs.len() - 1) {
        s.tabs[i].filters = pad_filters(incoming.filters.clone());
        apply_tab_grid(s, incoming);
        s.anchor();
        if let Some(t) = s.tabs.get_mut(i) {
            t.name = name;
            t.locked = locked;
            t.color = color;
            t.session = path.to_string();
        }
    } else {
        let mut t = incoming.clone();
        t.name = name;
        t.locked = locked;
        t.color = color;
        t.session = path.to_string();
        t.filters = pad_filters(incoming.filters.clone());
        s.tabs[i] = t;
    }
    for (k, sess) in others {
        if let Some(t) = s.tabs.get_mut(k) {
            t.session = sess;
        }
    }
    settle_sessions(s);
    s.tabs.iter().map(|t| t.session.clone()).collect()
}

/// A duplicate must not keep the source file, or Save on the copy would overwrite it.
pub(crate) fn fork_tab(src: &Tab, name: String) -> Tab {
    let mut t = src.clone();
    t.name = name;
    t.locked = false;
    t.session.clear();
    t
}

/// Tabs that still point at a deleted session lose that link.
/// `linked[i]` was true when the delete began. A path that changed during the dialog stays.
pub(crate) fn sessions_after_delete(before: &[String], now: &[String], linked: &[bool], deleted: bool) -> Option<Vec<String>> {
    if !deleted {
        return None;
    }
    let mut out = now.to_vec();
    let mut hit = false;
    for (i, p) in out.iter_mut().enumerate() {
        let was = before.get(i).map(String::as_str).unwrap_or("");
        if linked.get(i).copied().unwrap_or(false) && *p == was {
            p.clear();
            hit = true;
        }
    }
    hit.then_some(out)
}

/// Same rule as delete: only a tab that still has the old path follows the rename.
pub(crate) fn sessions_after_rename(before: &[String], now: &[String], linked: &[bool], new: &str) -> Option<Vec<String>> {
    let mut out = now.to_vec();
    let mut hit = false;
    for (i, p) in out.iter_mut().enumerate() {
        let was = before.get(i).map(String::as_str).unwrap_or("");
        if linked.get(i).copied().unwrap_or(false) && !was.is_empty() && *p == was {
            *p = new.to_string();
            hit = true;
        }
    }
    hit.then_some(out)
}

/// Clean grid of one tab, plus the pane permutation since that grid was saved. Parallel to `Session::tabs`.
#[derive(Clone, Debug)]
pub(crate) struct TabBase {
    pub(crate) grid: Tab,
    pub(crate) order: Vec<u8>,
}

/// Columns the view reported after the baseline was taken. An empty baseline column is not a change yet.
pub(crate) fn absorb_cols(base: &mut Tab, live: &[String], n: usize) {
    if base.cols.len() < MAXP {
        base.cols.resize(MAXP, String::new());
    }
    for i in 0..n.min(MAXP) {
        if base.cols[i].is_empty() {
            if let Some(c) = live.get(i) {
                base.cols[i] = c.clone();
            }
        }
    }
}

fn tab_layout_differs(cur: &Session, saved: &Tab) -> bool {
    let near = |x: &[f64], y: &[f64]| x.len() == y.len() && x.iter().zip(y).all(|(a, b)| (a - b).abs() < 0.005);
    let n = cur.panes().min(MAXP);
    let view_diff = (0..n).any(|i| match saved.views.get(i) {
        Some(v) if v.0 != 0 => cur.views.get(i) != Some(v),
        _ => false,
    });
    let col_diff = (0..n).any(|i| match saved.cols.get(i) {
        Some(c) if !c.is_empty() => cur.cols.get(i).map(String::as_str) != Some(c.as_str()),
        _ => false,
    });
    cur.heights != saved.heights
        || !near(&cur.colr, &saved.colr)
        || cur.rowr.len() != saved.rowr.len()
        || cur.rowr.iter().zip(&saved.rowr).any(|(a, b)| !near(a, b))
        || view_diff
        || col_diff
        || (0..n).any(|i| cur.tints.get(i) != saved.tints.get(i))
}

/// Save dot for the tab on screen. Another tab's grid, and which tab is selected, do not count.
pub(crate) fn active_tab_dirty(cur: &Session, saved: Option<&Tab>, order: &[u8]) -> bool {
    order_moved(order) || saved.is_some_and(|t| tab_layout_differs(cur, t))
}

/// Grid a tab is clean against. `file_tab` is the active tab of its `.grids` file, when that file was read.
pub(crate) fn saved_grid(tab: &Tab, file_tab: Option<&Tab>) -> Tab {
    let Some(src) = file_tab else { return tab.clone() };
    let mut grid = tab.clone();
    grid.heights = src.heights.clone();
    grid.colr = src.colr.clone();
    grid.rowr = src.rowr.clone();
    grid.active = src.active;
    grid.paths = src.paths.clone();
    grid.views = src.views.clone();
    grid.cols = src.cols.clone();
    grid.tints = src.tints.clone();
    grid.pane_folders = src.pane_folders.clone();
    grid.pane_cur = src.pane_cur.clone();
    grid.session = tab.session.clone();
    grid
}

pub(crate) fn baseline_grid(tab: &Tab) -> Tab {
    if tab.session.is_empty() {
        return tab.clone();
    }
    let Ok(text) = std::fs::read_to_string(&tab.session) else { return tab.clone() };
    let parsed = Session::parse(&text);
    let tabs = parsed.view_tabs();
    if tabs.is_empty() {
        return tab.clone();
    }
    let at = parsed.tab.min(tabs.len() - 1);
    saved_grid(tab, Some(&tabs[at]))
}

pub(crate) fn initial_tab_bases(s: &Session) -> Vec<TabBase> {
    let shown = s.view_tabs();
    if shown.is_empty() {
        return Vec::new();
    }
    let active = s.tab.min(shown.len() - 1);
    shown.into_iter().enumerate().map(|(i, t)| {
        let order = if i == active { s.order.clone() } else { Vec::new() };
        TabBase { grid: baseline_grid(&t), order }
    }).collect()
}

/// Named-session text for one tab: its grid only.
pub(crate) fn tab_session_text(s: &Session, i: usize) -> Option<String> {
    let t = s.view_tabs().get(i)?.clone();
    let mut one = s.clone();
    one.tabs = vec![t];
    one.tab = 0;
    one.publish();
    Some(one.snapshot())
}

/// Tabs a saved session contributes when it is opened. One file is one tab, linked to `path`.
/// A file that still holds several tabs opens each of them, so none of those grids are dropped.
/// Only the tab that was active in the file keeps `path`. Save then writes that one tab, not the others.
pub(crate) fn tabs_from_session_text(text: &str, path: &str) -> Vec<Tab> {
    let text = text.trim_start_matches('\u{feff}').trim();
    if !looks_like_session(text) {
        return Vec::new();
    }
    let other = Session::parse(text);
    let view = other.view_tabs();
    if view.is_empty() {
        return Vec::new();
    }
    let active = other.tab.min(view.len() - 1);
    view.into_iter().enumerate().map(|(k, mut t)| {
        t.session.clear();
        if k == active && !path.is_empty() {
            t.session = path.to_string();
        }
        t
    }).collect()
}

/// Opened sessions sit immediately after the current tab. Tabs already open stay, in the same order.
pub(crate) fn tabs_after_open(existing: &[Tab], current: usize, incoming: &[Tab]) -> (Vec<Tab>, usize) {
    let mut tabs = existing.to_vec();
    if incoming.is_empty() {
        let at = if tabs.is_empty() { 0 } else { current.min(tabs.len() - 1) };
        return (tabs, at);
    }
    if tabs.is_empty() {
        return (incoming.to_vec(), 0);
    }
    let at = (current + 1).min(tabs.len());
    for (k, t) in incoming.iter().enumerate() {
        tabs.insert(at + k, t.clone());
    }
    (tabs, at)
}

pub(crate) fn parse_tab_color(v: &str) -> Option<u32> {
    let t = v.trim().trim_start_matches('#');
    if t.len() != 6 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(t, 16).ok().map(|n| n & 0xFFFFFF)
}

pub(crate) fn pane_json(t: &Tab) -> Vec<Value> {
    let mut panes = Vec::new();
    for i in 0..MAXP {
        let filter = t.filters.get(i).cloned().unwrap_or_default();
        let tint = t.tints.get(i).copied().unwrap_or(0);
        let folders = t.pane_folders.get(i).cloned().unwrap_or_default();
        let folder = t.pane_cur.get(i).copied().unwrap_or(0);
        if t.paths[i].is_empty() && t.views[i] == (0, 0) && t.cols[i].is_empty() && filter.is_empty() && tint == 0 && folders.len() < 2 {
            continue;
        }
        let mut p = serde_json::Map::new();
        p.insert("index".into(), Value::from(i));
        if !t.paths[i].is_empty() {
            p.insert("path".into(), Value::String(t.paths[i].clone()));
        }
        if t.views[i].0 != 0 {
            p.insert("view".into(), Value::Array(vec![Value::from(t.views[i].0), Value::from(t.views[i].1)]));
        }
        if !t.cols[i].is_empty() {
            p.insert("columns".into(), Value::String(t.cols[i].clone()));
        }
        if !filter.is_empty() {
            p.insert("filter".into(), Value::String(filter));
        }
        if tint != 0 {
            p.insert("tint".into(), Value::from(tint));
        }
        if folders.len() > 1 {
            p.insert("folders".into(), Value::Array(folders.into_iter().map(Value::String).collect()));
            p.insert("folder".into(), Value::from(folder));
        }
        panes.push(Value::Object(p));
    }
    panes
}

pub(crate) fn tabs_json(tabs: &[Tab], active: usize) -> String {
    let v: Vec<Value> = tabs.iter().enumerate().map(|(i, t)| {
        let mut obj = serde_json::json!({
            "name": t.name,
            "locked": t.locked,
            "active": i == active,
            "layout": { "columns": t.heights, "colr": t.colr, "rowr": t.rowr, "active": t.active },
            "panes": pane_json(t),
        });
        if let Some(m) = obj.as_object_mut() {
            if let Some(c) = t.color {
                m.insert("color".into(), Value::String(format!("{c:06X}")));
            }
            if !t.session.is_empty() {
                m.insert("session".into(), Value::String(t.session.clone()));
            }
        }
        obj
    }).collect();
    serde_json::to_string(&v).unwrap_or_else(|_| "[]".into())
}

/// A tab object needs a name, a layout, panes, or a lock. Anything else is left for a newer build.
pub(crate) fn tab_from_value(v: &Value) -> Option<Tab> {
    let obj = v.as_object()?;
    if !obj.contains_key("name") && !obj.contains_key("layout") && !obj.contains_key("panes") && !obj.contains_key("locked") {
        return None;
    }
    let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
    let locked = obj.get("locked").and_then(|n| n.as_bool()).unwrap_or(false);
    let session = obj.get("session").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
    let color = obj.get("color").and_then(|c| c.as_str()).and_then(parse_tab_color);
    // No saved columns: one pane, the fresh default. A file that stored columns keeps those.
    let mut heights = vec![1];
    let mut colr = Vec::new();
    let mut rowr: Vec<Vec<f64>> = Vec::new();
    let mut active = 0usize;
    if let Some(l) = obj.get("layout").and_then(|l| l.as_object()) {
        if let Some(c) = l.get("columns").and_then(|c| c.as_array()) {
            let h: Vec<usize> = c.iter().filter_map(|n| n.as_u64()).map(|n| (n as usize).clamp(1, 4)).take(4).collect();
            if !h.is_empty() {
                heights = h;
            }
        }
        if let Some(c) = l.get("colr").and_then(|c| c.as_array()) {
            colr = c.iter().filter_map(|n| n.as_f64()).collect();
        }
        if let Some(rows) = l.get("rowr").and_then(|c| c.as_array()) {
            rowr = rows.iter().map(|r| r.as_array().map(|a| a.iter().filter_map(|n| n.as_f64()).collect()).unwrap_or_default()).collect();
        }
        active = l.get("active").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
    }
    colr = fix(colr, heights.len());
    let group = |c: usize| if rowr.len() == heights.len() { rowr.get(c) } else { rowr.first() };
    let rowr = heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
    let n: usize = heights.iter().sum();
    if active >= n {
        active = 0;
    }
    let mut paths = vec![String::new(); MAXP];
    let mut views = vec![(0i32, 0i32); MAXP];
    let mut cols = vec![String::new(); MAXP];
    let mut filters = vec![String::new(); MAXP];
    let mut tints = vec![0u8; MAXP];
    let mut pane_folders = vec![Vec::new(); MAXP];
    let mut pane_cur = vec![0usize; MAXP];
    if let Some(panes) = obj.get("panes").and_then(|p| p.as_array()) {
        for p in panes {
            let Some(o) = p.as_object() else { continue };
            let i = o.get("index").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
            if i >= MAXP {
                continue;
            }
            if let Some(path) = o.get("path").and_then(|x| x.as_str()) {
                paths[i] = path.to_string();
            }
            if let Some(view) = o.get("view").and_then(|x| x.as_array()) {
                let m = view.first().and_then(|n| n.as_i64()).unwrap_or(0) as i32;
                let z = view.get(1).and_then(|n| n.as_i64()).unwrap_or(0) as i32;
                if m != 0 {
                    views[i] = (m, z);
                }
            }
            if let Some(c) = o.get("columns").and_then(|x| x.as_str()) {
                cols[i] = c.to_string();
            }
            if let Some(f) = o.get("filter").and_then(|x| x.as_str()) {
                filters[i] = f.to_string();
            }
            if let Some(t) = o.get("tint").and_then(|x| x.as_u64()) {
                tints[i] = if t <= 6 { t as u8 } else { 0 };
            }
            if let Some(fs) = o.get("folders").and_then(|x| x.as_array()) {
                let list: Vec<String> = fs.iter().filter_map(|x| x.as_str()).filter(|p| !p.is_empty()).map(str::to_string).collect();
                if list.len() > 1 {
                    let cur = o.get("folder").and_then(|n| n.as_u64()).unwrap_or(0) as usize;
                    pane_cur[i] = cur.min(list.len() - 1);
                    pane_folders[i] = list;
                }
            }
        }
    }
    let name = if name.is_empty() { "1".into() } else { name };
    Some(Tab { name, locked, color, heights, colr, rowr, active, paths, views, cols, filters, tints, pane_folders, pane_cur, session })
}

pub(crate) fn tabs_from_json(v: &str) -> Option<(Vec<Tab>, usize)> {
    let values: Vec<Value> = serde_json::from_str(v).ok()?;
    let mut tabs = Vec::new();
    let mut active = None;
    for val in &values {
        let Some(t) = tab_from_value(val) else { continue };
        if val.get("active").and_then(|a| a.as_bool()) == Some(true) && active.is_none() {
            active = Some(tabs.len());
        }
        tabs.push(t);
    }
    if tabs.is_empty() {
        return None;
    }
    let active = active.unwrap_or(0).min(tabs.len() - 1);
    Some((tabs, active))
}

/// Put `values` on the session. Invalid entries are skipped; none left means one tab from the live grid.
pub(crate) fn adopt_tabs(s: &mut Session, values: &[Value]) {
    let mut tabs = Vec::new();
    let mut active = None;
    for v in values {
        let Some(t) = tab_from_value(v) else { continue };
        if v.get("active").and_then(|a| a.as_bool()) == Some(true) && active.is_none() {
            active = Some(tabs.len());
        }
        tabs.push(t);
    }
    if tabs.is_empty() {
        s.tabs.clear();
        s.tab = 0;
        s.anchor();
        return;
    }
    s.tabs = tabs;
    s.tab = active.unwrap_or(0).min(s.tabs.len() - 1);
    s.publish();
}

/// Layout changes worth a save prompt: grid, split ratios, view modes, which tab is open. Folder changes don't count.
pub(crate) fn layout_differs(cur: &Session, base: &Session) -> bool {
    let near = |x: &[f64], y: &[f64]| x.len() == y.len() && x.iter().zip(y).all(|(a, b)| (a - b).abs() < 0.005);
    let grid = |c: &Session, b: &Session| {
        c.heights != b.heights
            || !near(&c.colr, &b.colr)
            || c.rowr.len() != b.rowr.len()
            || c.rowr.iter().zip(&b.rowr).any(|(a, b)| !near(a, b))
            || (0..c.panes()).any(|i| b.views[i].0 != 0 && c.views[i] != b.views[i])
            || (0..c.panes()).any(|i| !b.cols[i].is_empty() && c.cols[i] != b.cols[i])
            || (0..c.panes()).any(|i| c.tints.get(i) != b.tints.get(i))
    };
    if grid(cur, base) || cur.tab != base.tab || cur.tabs.len() != base.tabs.len() {
        return true;
    }
    cur.tabs.iter().zip(&base.tabs).enumerate().any(|(i, (a, b))| {
        if i == cur.tab {
            return false;
        }
        let n = a.heights.iter().sum::<usize>().min(MAXP);
        a.heights != b.heights
            || !near(&a.colr, &b.colr)
            || a.rowr.len() != b.rowr.len()
            || a.rowr.iter().zip(&b.rowr).any(|(x, y)| !near(x, y))
            || (0..n).any(|k| b.views.get(k).is_some_and(|v| v.0 != 0) && a.views.get(k) != b.views.get(k))
            || (0..n).any(|k| b.cols.get(k).is_some_and(|c| !c.is_empty()) && a.cols.get(k) != b.cols.get(k))
            || (0..n).any(|k| a.tints.get(k) != b.tints.get(k))
    })
}

// Drives, folders and shares; not shell places like This PC or the Recycle Bin, whose columns are their own.
pub(crate) fn is_fs(path: &str) -> bool {
    path.get(1..3) == Some(":\\") || path.starts_with(r"\\")
}

/// `a.PDF` becomes `*.PDF` for "same type as the selected file". Folders and names without an extension do not.
pub(crate) fn same_type(name: &str, is_dir: bool) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    let bad = is_dir || stem.is_empty() || ext.is_empty() || ext.contains([';', '*', '?']);
    (!bad).then(|| format!("*.{ext}"))
}

/// A folder you can browse. ZIP, CAB and the like are FOLDER|STREAM, so a filter treats them as files.
pub(crate) fn real_folder(attr: u32) -> bool {
    attr & SFGAO_FOLDER.0 != 0 && attr & SFGAO_STREAM.0 == 0
}

/// Address crumbs of a parsing name: (shown name, folder to open). The drive or `\\server\share` is first.
/// Shell places such as This PC have none.
pub(crate) fn crumbs(parse: &str) -> Vec<(String, String)> {
    if !is_fs(parse) {
        return Vec::new();
    }
    let (root, go, rest) = match parse.strip_prefix(r"\\") {
        Some(unc) => {
            let mut it = unc.splitn(3, '\\');
            let root = format!(r"\\{}", it.by_ref().take(2).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\\"));
            (root.clone(), root, it.next().unwrap_or(""))
        }
        None => {
            let Some(root) = crate::winpath::split_prefix(parse, 2) else { return Vec::new() };
            let Some(go) = crate::winpath::split_prefix(parse, 3) else { return Vec::new() };
            (root.to_string(), go.to_string(), parse.get(3..).unwrap_or(""))
        }
    };
    let mut path = root.clone();
    let mut out = vec![(root, go)];
    for part in rest.split('\\').filter(|p| !p.is_empty()) {
        path = format!(r"{path}\{part}");
        out.push((part.to_string(), path.clone()));
    }
    out
}

/// Crumb widths `w` into `avail` px.
/// Returns `(prefix_end, suffix_start, middle_ellipsis, leaf_width)`.
///
/// Paint `0..prefix_end`, then optional `…`, then `suffix_start..n`. The **leaf** (last crumb)
/// always sits at the right and stays full unless even the leaf alone is wider than `avail`
/// (then `leaf_width` is clipped). When the path is wider than the field, keep the drive in the
/// prefix when it fits, fold the middle into `…`, and show as many crumbs adjacent to the leaf
/// as the leftover allows — never collapse to the leaf alone while a parent still fits.
pub(crate) fn crumb_fit(w: &[i32], avail: i32, sep: i32, ell: i32, min: i32) -> (usize, usize, bool, i32) {
    let n = w.len();
    let _ = min;
    if n == 0 || avail <= 0 {
        return (0, 0, false, 0);
    }
    let total = w.iter().sum::<i32>() + sep * (n as i32 - 1);
    if total <= avail {
        return (n, n, false, w[n - 1]);
    }
    let leaf = w[n - 1];
    if n == 1 || leaf >= avail {
        return (0, n - 1, false, leaf.min(avail).max(0));
    }

    // Width of crumbs[0..pre) + optional ellipsis + crumbs[suf..n).
    let layout_w = |pre: usize, suf: usize, mid: bool| -> i32 {
        let mut segs: Vec<i32> = Vec::with_capacity(n + 1);
        segs.extend_from_slice(&w[..pre]);
        if mid {
            segs.push(ell);
        }
        segs.extend_from_slice(&w[suf..]);
        if segs.is_empty() {
            return 0;
        }
        segs.iter().sum::<i32>() + sep * (segs.len() as i32 - 1)
    };

    // (vis, has_drive, suffix_len, prefix_len, pre, suf, mid)
    let mut best: Option<(i32, i32, i32, i32, usize, usize, bool)> = None;
    let consider = |best: &mut Option<(i32, i32, i32, i32, usize, usize, bool)>, pre: usize, suf: usize, mid: bool| {
        if suf > n - 1 {
            return;
        }
        if mid && pre >= suf {
            return;
        }
        if !mid && pre != 0 {
            // Without a middle ellipsis the only non-full layout is a pure right suffix.
            return;
        }
        if !mid && pre == 0 && suf == 0 {
            return; // full path
        }
        let wd = layout_w(pre, suf, mid);
        if wd > avail {
            return;
        }
        let vis = (pre + (n - suf)) as i32;
        let drive = if pre >= 1 { 1 } else { 0 };
        let suffix_len = (n - suf) as i32;
        let key = (vis, drive, suffix_len, pre as i32, pre, suf, mid);
        let better = match best {
            None => true,
            Some(b) => (key.0, key.1, key.2, key.3) > (b.0, b.1, b.2, b.3),
        };
        if better {
            *best = Some(key);
        }
    };

    for suf in (1..n).rev() {
        for pre in 0..suf {
            consider(&mut best, pre, suf, true);
        }
        // Pure suffix without ellipsis (drive omitted, no room for …).
        consider(&mut best, 0, suf, false);
    }
    consider(&mut best, 0, n - 1, false);

    let Some((_, _, _, _, pre, suf, mid)) = best else {
        return (0, n - 1, false, leaf.min(avail).max(0));
    };
    (pre, suf, mid, leaf)
}

/// `crumb_fit`, keeping a slot for a `›` after the current folder when the full path is visible
/// and the field has room. Shell places and a field too narrow for that chevron lay out as before.
/// `(prefix_end, suffix_start, middle_ellipsis, leaf_width, tail_›)`.
pub(crate) fn crumb_fit_tail(
    w: &[i32],
    avail: i32,
    sep: i32,
    ell: i32,
    min: i32,
    fs: bool,
) -> (usize, usize, bool, i32, bool) {
    let n = w.len();
    let want_tail = fs && sep >= 0 && min >= 0 && avail >= sep.saturating_add(min);
    let (pre, suf, mid, last) = crumb_fit(w, if want_tail { avail - sep } else { avail }, sep, ell, min);
    let full = n > 0 && pre == n && suf == n && !mid;
    let tail = want_tail && full;
    if want_tail && !tail {
        let (p, s, m, l) = crumb_fit(w, avail, sep, ell, min);
        return (p, s, m, l, false);
    }
    (pre, suf, mid, last, tail)
}

/// The address crumbs hide part of the path. `w` is each segment's laid-out width.
/// A middle `…`, a missing segment, or a clipped leaf means the bar is truncated.
pub(crate) fn addr_path_truncated(w: &[i32], avail: i32, sep: i32, ell: i32, min: i32, fs: bool) -> bool {
    if w.is_empty() {
        return false;
    }
    let n = w.len();
    let (pre, suf, mid, last, _) = crumb_fit_tail(w, avail, sep, ell, min, fs);
    if n > 0 && pre == 0 && suf == 0 && last == 0 {
        return true;
    }
    let full_leaf = w[n - 1];
    mid || pre + (n - suf) < n || last < full_leaf
}

/// One line of address text (the edit, or a shell name drawn as a single crumb)
/// is cut off when the text is wider than the field it is drawn in.
pub(crate) fn addr_line_truncated(text_px: i32, box_px: i32) -> bool {
    text_px > box_px.max(0)
}

/// Tooltip for the address bar.
/// A filesystem parsing name is the absolute path. A shell place has no file path:
/// its display name, or the parsing name when the display name is empty.
/// Character boundaries only; a multibyte name is never sliced.
pub(crate) fn addr_tip_text(parse: &str, disp: &str) -> String {
    if is_fs(parse) {
        return parse.to_string();
    }
    if !disp.is_empty() {
        return disp.to_string();
    }
    parse.to_string()
}

pub(crate) const RECENT: usize = 20;

// A pane moving from `old` to `new` puts both at the front of the recent list. Coming back to the folder it
// already holds (session restore, theme/language rebuild, refresh) is no visit; neither is a fresh pane or a shell place.
pub(crate) fn visit(list: &mut Vec<String>, old: &str, new: &str) {
    if old.is_empty() || old.eq_ignore_ascii_case(new) || !is_fs(new) {
        return;
    }
    for p in [old, new].into_iter().filter(|p| is_fs(p)) {
        list.retain(|x| !x.eq_ignore_ascii_case(p));
        list.insert(0, p.to_string());
    }
    list.truncate(RECENT);
}

// The recent menu of a pane: everything but the folder it shows.
pub(crate) fn recent_items(list: &[String], cur: &str) -> Vec<String> {
    list.iter().filter(|p| !p.eq_ignore_ascii_case(cur)).cloned().collect()
}

/// Address text that names no path: the recent folder (newest first), then a favourite folder,
/// whose name equals it, else starts with it, else contains it. The folder on display is skipped.
pub(crate) fn jump_pick<'a>(q: &str, recent: &'a [String], favs: &'a [String], cur: &str) -> Option<&'a str> {
    let q = q.trim().to_lowercase();
    if q.is_empty() || q.contains(['\\', '/', ':', '"']) {
        return None;
    }
    let score_name = |n: &str| {
        let n = n.to_lowercase();
        if n == q {
            Some(0u8)
        } else if n.starts_with(&q) {
            Some(1)
        } else {
            n.contains(&q).then_some(2)
        }
    };
    let score = |p: &str| {
        let path = fav_path(p);
        [short_name(path), fav_label(p)].iter().filter_map(|n| score_name(n)).min()
    };
    let all = recent.iter().chain(favs).filter(|p| is_fs(fav_path(p)) && !fav_path(p).eq_ignore_ascii_case(cur));
    all.filter_map(|p| Some((score(p)?, p))).min_by_key(|(s, _)| *s).map(|(_, p)| fav_path(p))
}

/// Enter in the address bar fell through (`done` is None): jump only when the text named no path.
/// A path that resolved but would not open just beeps.
pub(crate) fn addr_jumps(done: Option<bool>, resolved: bool) -> bool {
    done.is_none() && !resolved
}

// A view's columns: visible ones in order with their widths, then the sort columns, each property as
// "format GUID/property id": "GUID/pid=width,...;GUID/pid=direction,...".
pub(crate) fn cols_text(cols: &[(PROPERTYKEY, u32)], sort: &[(PROPERTYKEY, i32)]) -> String {
    let key = |k: &PROPERTYKEY| format!("{:?}/{}", k.fmtid, k.pid);
    let cols: Vec<String> = cols.iter().map(|(k, w)| format!("{}={w}", key(k))).collect();
    let sort: Vec<String> = sort.iter().map(|(k, d)| format!("{}={d}", key(k))).collect();
    format!("{};{}", cols.join(","), sort.join(","))
}

pub(crate) fn parse_cols(t: &str) -> (Vec<(PROPERTYKEY, u32)>, Vec<(PROPERTYKEY, i32)>) {
    fn items<T: std::str::FromStr>(s: &str) -> Vec<(PROPERTYKEY, T)> {
        let item = |e: &str| {
            let (k, v) = e.split_once('=')?;
            let (g, pid) = k.split_once('/')?;
            Some((PROPERTYKEY { fmtid: GUID::try_from(g).ok()?, pid: pid.parse().ok()? }, v.parse().ok()?))
        };
        s.split(',').filter_map(item).collect()
    }
    let (cols, sort) = t.split_once(';').unwrap_or((t, ""));
    (items(cols), items(sort))
}

// Existing relative paths become absolute; shell names like ::{GUID} pass through.
// A UNC path or a mapped drive is not probed. `exists` waits on a dead share (~45s) and this
// runs in main() before any window. The pane resolves that path off the UI thread once it is up.
// Saved sessions do not come through here: pane paths are copied as text, and `navigate` is what
// opens them. That path must not stat a share either (free space, metadata).
pub(crate) fn resolve(p: String) -> String {
    if unsafe { crate::share_path(&p) } {
        return p;
    }
    match Path::new(&p).exists() {
        true => std::path::absolute(&p).map(|x| x.to_string_lossy().into_owned()).unwrap_or(p),
        false => p,
    }
}

pub(crate) fn is_session_file(p: &Path) -> bool {
    // .qdr: sessions saved before the rename
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("grids") || x.eq_ignore_ascii_case("qdr"))
}

fn pane_slot_key(k: &str) -> bool {
    let mut cs = k.chars();
    let Some(p) = cs.next() else { return false };
    if !matches!(p, 'p' | 'v' | 'c' | 'b' | 't') {
        return false;
    }
    let rest: String = cs.collect();
    rest.parse::<usize>().ok().is_some_and(|i| i < MAXP)
}

/// A line this build knows how to load. Plain text and a foreign `.qdr` are not sessions.
pub(crate) fn looks_like_session(text: &str) -> bool {
    let text = text.trim_start_matches('\u{feff}');
    text.lines().any(|line| {
        let Some((k, _)) = line.split_once('=') else { return false };
        let k = k.trim();
        matches!(
            k,
            "win" | "side" | "sidedpi" | "split" | "cols" | "grid" | "colr" | "rowr" | "active" | "order" | "tabs" | "session" | "layout" | "theme" | "dark" | "accent"
        ) || pane_slot_key(k)
    })
}

// A lone session-file argument means "load this session".
pub(crate) fn session_arg(args: &[String]) -> Option<&str> {
    match args {
        [f] if is_session_file(Path::new(f)) => Some(f),
        _ => None,
    }
}

// `grids.exe A B C` opens the paths in pane order, growing the grid if needed.
pub(crate) fn apply_args(s: &mut Session, args: Vec<String>) {
    if args.is_empty() {
        return;
    }
    let n = args.len().min(MAXP);
    for (i, p) in args.into_iter().take(MAXP).enumerate() {
        s.paths[i] = p;
    }
    if n > s.panes() {
        let c = (n as f64).sqrt().ceil() as usize;
        s.set_heights(vec![n.div_ceil(c); c]);
    }
    s.active = 0;
    s.anchor();
}

/// %APPDATA%\\Maguro\\maguro-sessions (next to the exe in portable mode). Created when missing.
/// The first time, sessions from the old folders next to the exe are copied in; the old files stay.
pub(crate) fn sessions_dir() -> PathBuf {
    let dir = store::config_dir().join("maguro-sessions");
    if !dir.exists() && std::fs::create_dir_all(&dir).is_ok() {
        let exe = store::exe_dir();
        for old in [exe.join("maguro-sessions"), exe.join("sessions")] {
            if old != dir && old.is_dir() {
                copy_old_sessions(&old, &dir);
            }
        }
    }
    dir
}

// Copy top-level .grids and .qdr files. Sources stay put.
fn copy_old_sessions(old: &Path, new: &Path) {
    let Ok(rd) = std::fs::read_dir(old) else { return };
    for ent in rd.flatten() {
        let src = ent.path();
        if !src.is_file() || !is_session_file(&src) {
            continue;
        }
        let dst = new.join(ent.file_name());
        if !dst.exists() {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

pub(crate) fn human(b: u64) -> String {
    let (mut v, mut k) = (b as f64, 0);
    while v >= 1024.0 && k < 4 {
        v /= 1024.0;
        k += 1;
    }
    match k {
        0 => format!("{b} B"),
        _ => format!("{v:.1} {}", ["", "KB", "MB", "GB", "TB"][k]),
    }
}

/// "pdf; *.txt" -> "*pdf*;*.txt" for PathMatchSpec: plain words match anywhere in the name.
pub(crate) fn filter_spec(text: &str) -> String {
    let parts = text.split(';').map(str::trim).filter(|t| !t.is_empty());
    let parts = parts.map(|t| if t.contains(['*', '?']) { t.to_string() } else { format!("*{t}*") });
    parts.collect::<Vec<_>>().join(";")
}

#[cfg(test)]
mod crumb_tests {
    use super::{addr_jumps, addr_line_truncated, addr_path_truncated, addr_tip_text, crumb_fit, crumbs, jump_pick, layout_differs, real_folder, resolve, same_type, Session};
    use crate::imports::{SFGAO_FOLDER, SFGAO_STREAM};
    use crate::MAXP;
    use crate::THIS_PC;

    #[test]
    fn address_crumbs() {
        let c = |p: &str| crumbs(p).into_iter().map(|(n, g)| format!("{n}={g}")).collect::<Vec<_>>();
        assert_eq!(c("C:\\"), [r"C:=C:\"]);
        assert_eq!(c(r"C:\Users\Public\"), [r"C:=C:\", r"Users=C:\Users", r"Public=C:\Users\Public"]);
        assert_eq!(c(r"\\srv\share\a"), [r"\\srv\share=\\srv\share", r"a=\\srv\share\a"]);
        assert!(crumbs(THIS_PC).is_empty());
        assert!(crumbs("C:").is_empty());
        let p = r"D:\x\y y\z";
        assert!(crumbs(p).iter().all(|(_, g)| p.starts_with(g.as_str())));
        assert_eq!(c(r"F:\CAC\サンプル"), [r"F:=F:\", r"CAC=F:\CAC", "サンプル=F:\\CAC\\サンプル"]);
    }

    #[test]
    fn crumb_fits() {
        let f = |w: &[i32], avail| crumb_fit(w, avail, 4, 20, 16);
        assert_eq!(f(&[30, 40, 50], 500), (3, 3, false, 50));
        assert_eq!(f(&[14, 15, 15], 52), (3, 3, false, 15));
        // 51: parent+leaf = 34 (vis=2). Drive+ellipsis+leaf needs 57.
        assert_eq!(f(&[14, 15, 15], 51), (0, 1, false, 15));
        // Leaf alone does not fit -> clip the leaf.
        assert_eq!(f(&[10, 200], 100), (0, 1, false, 100));
        // 70: parent+leaf = 64 fits; ellipsis+parent+leaf = 88 does not.
        assert_eq!(f(&[30, 10, 50], 70), (0, 1, false, 50));
        // Room for ellipsis+parent+leaf (88) but not drive+ellipsis+leaf (108).
        assert_eq!(f(&[30, 10, 50], 88), (0, 1, true, 50));
        // Full path is only 98 wide, so 108 shows everything. Use a longer middle for drive+ellipsis+leaf.
        assert_eq!(f(&[30, 10, 50], 98), (3, 3, false, 50));
        // [30,40,50] total=128; drive+ellipsis+leaf = 108.
        assert_eq!(f(&[30, 40, 50], 108), (1, 2, true, 50));
        assert_eq!(f(&[30, 40], 15), (0, 1, false, 15));
        assert_eq!(f(&[30, 40], 0), (0, 0, false, 0));
        assert_eq!(f(&[], 100), (0, 0, false, 0));
        // Wide drive: ellipsis+parent+leaf (118) before drive+ellipsis+leaf (178).
        assert_eq!(f(&[100, 40, 50], 118), (0, 1, true, 50));
    }

    #[test]
    fn narrow_address_keeps_leaf_and_preceding() {
        let f = |avail| crumb_fit(&[30, 40, 50], avail, 4, 20, 16);
        assert_eq!(f(128), (3, 3, false, 50));
        assert_eq!(f(108), (1, 2, true, 50));
        assert_eq!(f(118), (1, 2, true, 50));
        assert_eq!(f(120), (1, 2, true, 50));
        assert_eq!(f(50), (0, 2, false, 50));
        assert_eq!(f(49), (0, 2, false, 49));
        assert_eq!(f(0), (0, 0, false, 0));
        assert_eq!(crumb_fit(&[50], 10, 4, 20, 16), (0, 0, false, 10));
        let g = |avail| crumb_fit(&[20, 30, 40, 50], avail, 4, 20, 16);
        assert_eq!(g(152), (4, 4, false, 50));
        assert_eq!(g(142), (1, 2, true, 50));
        assert_eq!(g(98), (1, 3, true, 50));
        assert_eq!(g(118), (1, 3, true, 50));
    }

    #[test]
    fn crumb_tail_room() {
        let f = |avail, fs| super::crumb_fit_tail(&[30, 40], avail, 4, 20, 16, fs);
        assert_eq!(f(16, true), (0, 1, false, 16, false));
        assert_eq!(f(19, true), (0, 1, false, 19, false));
        assert_eq!(f(40, true), (0, 1, false, 40, false));
        assert_eq!(f(500, true), (2, 2, false, 40, true));
        assert!(!f(500, false).4);
        assert!(!f(16, false).4);
        assert_eq!(f(0, true).4, false);
    }

    #[test]
    fn address_tip_only_when_the_path_is_cut() {
        let cut = |w: &[i32], avail, fs| addr_path_truncated(w, avail, 4, 20, 16, fs);
        assert!(!cut(&[30, 40, 50], 500, true), "every segment fits");
        assert!(!cut(&[14, 15, 15], 56, true), "names and the tail chevron fit");
        assert!(!cut(&[14, 15, 15], 52, true), "reclaiming the chevron lets the full path fit");
        assert!(!cut(&[14, 15, 15], 52, false), "without the chevron the same width fits");
        assert!(cut(&[14, 15, 15], 51, true), "a segment is omitted");
        assert!(cut(&[10, 200], 100, true), "the leaf is clipped");
        assert!(cut(&[30, 10, 50], 70, true), "drive is omitted");
        assert!(cut(&[30, 40], 15, true), "the leaf is clipped");
        assert!(cut(&[30, 40], 0, true));
        assert!(!cut(&[], 100, true));
        assert!(!cut(&[30], 80, false), "a shell name that fits gets no tip");
        assert!(cut(&[80], 40, false), "a shell name that does not fit does");
        assert!(cut(&[100], 40, true));
        // Full path is 74; reclaiming the tail chevron still shows it — not truncated.
        assert!(!cut(&[30, 40], 76, true));
        assert!(!cut(&[30, 40], 78, true));
        assert!(!cut(&[30, 40], 76, false), "a shell place does not reserve the tail chevron");
        assert!(!addr_line_truncated(40, 40));
        assert!(addr_line_truncated(41, 40));
        assert!(!addr_line_truncated(0, 0));
        assert!(addr_line_truncated(10, 0));
        assert!(!addr_line_truncated(0, -4));
    }


    #[test]
    fn address_tip_text_uses_the_path_or_the_shell_name() {
        let jp = r"F:\CAC\サンプル";
        assert_eq!(addr_tip_text(jp, "サンプル"), jp);
        assert_eq!(addr_tip_text(r"\\srv\share\資料", "資料"), r"\\srv\share\資料");
        assert_eq!(addr_tip_text(THIS_PC, "This PC"), "This PC");
        assert_eq!(addr_tip_text(THIS_PC, ""), THIS_PC);
        assert_eq!(addr_tip_text("", "ごみ箱"), "ごみ箱");
        assert_eq!(addr_tip_text("", ""), "");
        // A multibyte prefix is not a drive letter. `get` on a char boundary must not panic.
        assert_eq!(addr_tip_text("サンプル", ""), "サンプル");
        assert_eq!(addr_tip_text("サンプル", "表示"), "表示");
    }

    #[test]
    fn jump_picks() {
        let v = |s: &[&str]| s.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        let recent = v(&[r"C:\Users\me\Downloads", r"C:\Work\down", r"D:\Old Downloads", r"D:\문서\보고서 2026", THIS_PC]);
        let favs = v(&[r"E:\Down", r"\\srv\share", r"C:\", "C:\\Work\\proj\t보고서"]);
        let pick = |q: &str, cur: &str| jump_pick(q, &recent, &favs, cur);
        assert_eq!(pick("down", ""), Some(r"C:\Work\down"));
        assert_eq!(pick("DOWNL", ""), Some(r"C:\Users\me\Downloads"));
        assert_eq!(pick("loads", ""), Some(r"C:\Users\me\Downloads"));
        assert_eq!(pick("  down  ", r"c:\work\DOWN"), Some(r"E:\Down"));
        assert_eq!(pick("보고", ""), Some(r"D:\문서\보고서 2026"));
        assert_eq!(pick("보고서", ""), Some(r"C:\Work\proj"));
        assert_eq!(pick("share", ""), Some(r"\\srv\share"));
        assert_eq!(pick("c", ""), Some(r"C:\"));
        assert_eq!(pick("this", ""), None);
        for q in ["", "   ", r"down\x", "a/b", "c:", "\"down\"", "nothing"] {
            assert_eq!(pick(q, ""), None, "{q:?}");
        }
        assert!(addr_jumps(None, false));
        assert!(!addr_jumps(None, true));
        assert!(!addr_jumps(Some(true), false) && !addr_jumps(Some(false), false));
    }

    #[test]
    fn layout_new_round_trips_and_each_value_has_even_heights() {
        let fresh = Session::default();
        assert_eq!(fresh.layout_new, 3);
        assert_eq!(fresh.heights, vec![1]);
        assert_eq!(fresh.colr, super::even(1));
        assert_eq!(fresh.rowr, vec![super::even(1)]);
        assert_eq!(fresh.active, 0);
        // Older ini / omitted layout key still means two panes side by side.
        assert_eq!(Session::parse("").layout_new, 0);
        assert_eq!(Session::parse("drop=1\n").layout_new, 0);
        assert_eq!(Session::parse("layout=\n").layout_new, 0);
        assert_eq!(Session::parse("layout=nope\n").layout_new, 0);
        let want = [(0u8, vec![1usize, 1]), (1, vec![2]), (2, vec![2, 2]), (3, vec![1])];
        for (n, h) in want {
            assert_eq!(super::layout_heights_for(n), h);
            let mut s = Session::default();
            s.layout_new = n;
            s.active = 9;
            s.paths[0] = r"C:\old".into();
            s.set_heights(s.layout_heights());
            assert_eq!(s.layout_heights(), h);
            assert_eq!(s.heights, h);
            assert!(s.active < s.panes(), "active {} panes {}", s.active, s.panes());
            assert_eq!(s.colr, super::even(h.len()));
            for (rows, &count) in s.rowr.iter().zip(&h) {
                assert_eq!(rows, &super::even(count));
            }
            let back = Session::parse(&s.to_ini());
            assert_eq!((back.layout_new, back.layout_heights()), (n, h.clone()));
            s.boot = 2;
            crate::apply_boot(&mut s);
            assert_eq!(s.heights, h);
            assert_eq!(s.active, 0);
            assert!(s.active < s.panes());
            assert!(s.paths.iter().all(|p| p.is_empty()));
            assert_eq!(s.colr, super::even(h.len()));
            assert_eq!(s.layout_new, n);
        }
        let high = Session::parse("layout=9\n");
        assert_eq!(high.layout_new, 3);
        assert_eq!(high.layout_heights(), vec![1]);
        assert_eq!(super::layout_heights_for(4), vec![1]);
    }

    #[test]
    fn new_tab_uses_the_settings_pane_count_not_the_tab_on_screen() {
        // Dragged ratios and an uneven grid. A new tab must not copy any of this.
        let heights = vec![1usize, 3];
        let colr = vec![0.25, 0.75];
        let rowr = vec![vec![1.0], vec![0.2, 0.5, 0.3]];
        let fresh = super::layout_for_new_tab(3);
        assert_eq!(fresh.0, vec![1]);
        assert_eq!(fresh.1, super::even(1));
        assert_eq!(fresh.2, vec![super::even(1)]);
        assert_ne!((fresh.0.clone(), fresh.1.clone(), fresh.2.clone()), (heights, colr, rowr));
        // Each saved value, with even splits. 0 is two side by side, 3 is the fresh default of one.
        let want = [(0u8, vec![1usize, 1]), (1, vec![2]), (2, vec![2, 2]), (3, vec![1])];
        for (n, h) in want {
            let (got, colr, rowr) = super::layout_for_new_tab(n);
            assert_eq!(got, h);
            assert_eq!(colr, super::even(h.len()));
            for (rows, &count) in rowr.iter().zip(&h) {
                assert_eq!(rows, &super::even(count));
            }
        }
        assert_eq!(super::layout_for_new_tab(9).0, vec![1]);
        // A new window still opens the settings default, with even splits.
        let mut win = Session::default();
        assert_eq!((win.layout_new, win.heights.clone()), (3, vec![1]));
        win.layout_new = 2;
        win.set_heights(win.layout_heights());
        assert_eq!((win.heights.clone(), win.colr.clone()), (vec![2, 2], super::even(2)));
        // A path from a second instance stays one pane, which is not a copy of the screen.
        let mut handed = Session::default();
        handed.set_heights(vec![1]);
        assert_eq!(handed.heights, vec![1]);
    }

    #[test]
    fn missing_layout_is_one_pane_and_a_saved_grid_stays() {
        let bare = serde_json::json!({"name": "1", "locked": false});
        assert_eq!(super::tab_from_value(&bare).unwrap().heights, vec![1]);
        let four = serde_json::json!({
            "name": "Work",
            "layout": {"columns": [2, 2], "active": 3},
            "panes": [{"index": 0, "path": "C:\\a"}, {"index": 3, "path": "D:\\d"}]
        });
        let t = super::tab_from_value(&four).unwrap();
        assert_eq!((t.heights.clone(), t.active, t.paths[0].as_str(), t.paths[3].as_str()), (vec![2, 2], 3, r"C:\a", r"D:\d"));

        let base = Session::default();
        assert_eq!(base.panes(), 1);
        let empty = crate::session_from_window(&serde_json::json!({}), &base);
        assert_eq!(empty.heights, vec![1]);
        assert_eq!(empty.panes(), 1);
        let saved = serde_json::json!({
            "tabs": [{
                "name": "Work",
                "active": true,
                "layout": {"columns": [2, 2], "active": 1},
                "panes": [{"index": 1, "path": "D:\\keep"}]
            }]
        });
        let back = crate::session_from_window(&saved, &base);
        assert_eq!(back.heights, vec![2, 2]);
        assert_eq!(back.paths[1], r"D:\keep");
        assert_eq!(back.layout_new, base.layout_new);

        let mut stored = Session::default();
        stored.set_heights(vec![2, 2]);
        stored.paths[2] = r"E:\saved".into();
        stored.anchor();
        let round = Session::parse(&stored.to_ini());
        assert_eq!(round.heights, vec![2, 2]);
        assert_eq!(round.paths[2], r"E:\saved");
        assert_eq!(round.layout_new, stored.layout_new);
    }

    #[test]
    fn pane_tint_round_trip() {
        let mut s = Session::default();
        s.set_heights(vec![2, 2]);
        s.tints[3] = 4;
        s.anchor();
        let text = s.to_ini();
        assert!(text.contains("t3=4\n"));
        assert!(text.contains("\"tint\":4"));
        let back = Session::parse(&text);
        assert_eq!(back.tints[3], 4);
        assert_eq!(back.view_tabs()[back.tab].tints[3], 4);
        assert_eq!(Session::parse("t5=9\nt6=x\nt99=2").tints, vec![0; MAXP]);
        assert_eq!(Session::parse(&Session::default().to_ini()).tints, vec![0; MAXP]);
        let mut base = s.clone();
        base.tints[3] = 0;
        base.anchor();
        assert!(layout_differs(&s, &base));
        assert!(!s.snapshot().contains("win="));
    }

    #[test]
    fn pane_tabs_follow_the_pane_and_round_trip() {
        let mut s = Session::default();
        s.paths[0] = r"C:\a".into();
        assert_eq!(s.pane_tab_list(0), vec![r"C:\a"]);
        assert_eq!(s.close_pane_tab(0, 0), None); // the last pane tab stays
        s.add_pane_tab(0);
        assert_eq!((s.pane_tab_list(0).len(), s.pane_cur[0]), (2, 1));
        s.paths[0] = r"C:\b".into(); // the new pane tab browsed on
        assert_eq!(s.pane_tab_list(0), vec![r"C:\a".to_string(), r"C:\b".into()]);
        assert_eq!(s.pick_pane_tab(0, 0).as_deref(), Some(r"C:\a"));
        assert_eq!(s.pick_pane_tab(0, 0), None); // already current
        s.paths[0] = r"C:\a".into();
        s.add_pane_tab(0); // a, a, b: a copy right after the current one
        assert_eq!((s.pane_tab_list(0), s.pane_cur[0]), (vec![r"C:\a".to_string(), r"C:\a".into(), r"C:\b".into()], 1));
        assert_eq!(s.close_pane_tab(0, 2), None); // another pane tab: no browsing
        assert_eq!(s.pane_cur[0], 1);
        assert_eq!(s.close_pane_tab(0, 1).as_deref(), Some(r"C:\a"));
        assert!(s.pane_folders[0].is_empty() && s.pane_cur[0] == 0);

        s.pane_folders[0] = vec!["x".into(), "y".into(), "z".into()];
        s.pane_cur[0] = 1;
        s.paths[0] = "y".into();
        assert_eq!(s.close_pane_tab(0, 1).as_deref(), Some("z"));
        assert_eq!(s.pane_cur[0], 1);
        s.paths[0] = "z".into();
        assert_eq!(s.close_pane_tab(0, 1).as_deref(), Some("x"));
        assert!(s.pane_folders[0].is_empty());
        s.pane_folders[0] = vec!["x".into(), "y".into(), "z".into()];
        (s.pane_cur[0], s.paths[0]) = (2, "z".into());
        assert_eq!((s.close_pane_tab(0, 0), s.pane_cur[0]), (None, 1));

        let mut saved = Session::default();
        saved.paths[2] = r"D:\now".into();
        saved.pane_folders[2] = vec![r"C:\a".into(), "old".into(), "::{x}".into()];
        saved.pane_cur[2] = 1;
        let t = saved.to_ini();
        assert!(t.contains("b2=1|C:\\a|D:\\now|::{x}\n") && !t.contains("panetabs="), "{t}");
        let back = Session::parse(&t);
        assert_eq!((back.pane_tab_list(2), back.pane_cur[2]), (vec![r"C:\a".to_string(), r"D:\now".into(), "::{x}".into()], 1));
        assert!(Session::parse("p0=C:\\a\nb0=0|C:\\a\nb1=9|x|y\n").pane_folders[0].is_empty());
        assert_eq!(Session::parse("b1=9|x|y\n").pane_cur[1], 1);
        // an older panetabs line is ignored; the rest of the file still loads
        let old = Session::parse("panetabs=0\nside=300\n");
        assert_eq!(old.side, 300);
        assert!(!old.to_ini().contains("panetabs"));
        let mut moved = back.clone();
        moved.swap_slots(2, 0);
        assert_eq!((moved.pane_folders[0].len(), moved.pane_cur[0], moved.pane_folders[2].len()), (3, 1, 0));
        let mut lay = Session::default();
        lay.take_layout(&back);
        assert_eq!(lay.pane_folders[2].len(), 3);
        // the window-tab JSON keeps the folders too, and a second window tab does not share them
        saved.anchor();
        let (tabs, at) = super::tabs_from_json(&super::tabs_json(&saved.view_tabs(), saved.tab)).unwrap();
        assert_eq!(tabs[at].pane_folders[2].len(), 3);
        assert_eq!(tabs[at].pane_cur[2], 1);
    }

    #[test]
    fn copy_pane_tab_takes_only_the_folder_on_display() {
        let mut s = Session::default();
        s.paths[0] = r"C:\src".into();
        s.views[0] = (4, 32);
        s.cols[0] = "name:1".into();
        s.pane_folders[0] = vec![r"C:\old".into(), r"C:\src".into(), r"C:\later".into()];
        s.pane_cur[0] = 1;
        s.paths[1] = r"D:\dst".into();
        s.views[1] = (1, 16);
        s.cols[1] = "size".into();
        s.pane_folders[2] = vec!["keep".into(), "me".into()];
        s.pane_cur[2] = 1;
        assert_eq!(s.copy_pane_tab(0, 1).as_deref(), Some(r"C:\src"));
        // The list shows the folder on display in the current tab, which stays D:\dst until the browse.
        assert_eq!(s.pane_folders[1], vec![r"D:\dst".to_string(), r"C:\src".into()]);
        assert_eq!(s.pane_cur[1], 1);
        // The source's other pane tabs stay, and so does every other slot.
        assert_eq!(s.pane_tab_list(0), vec![r"C:\old".to_string(), r"C:\src".into(), r"C:\later".into()]);
        assert_eq!((s.views[1], s.cols[1].as_str()), ((1, 16), "size"));
        assert_eq!(s.pane_folders[2], vec!["keep".to_string(), "me".into()]);
        // Same folder already on display: a new tab, and nothing to browse.
        assert_eq!(s.copy_pane_tab(0, 0), None);
        assert_eq!(s.pane_cur[0], 2);
        assert_eq!(s.pane_tab_list(0), vec![r"C:\old".to_string(), r"C:\src".into(), r"C:\src".into(), r"C:\later".into()]);
        assert_eq!(s.paths[0], r"C:\src");

        crate::copy_pane_view(&mut s, 0, 3);
        assert_eq!((s.paths[3].as_str(), s.views[3], s.cols[3].as_str()), (r"C:\src", (4, 32), "name:1"));
        assert!(s.pane_folders[3].is_empty() && s.pane_cur[3] == 0);
        assert_eq!(s.pane_folders[0].len(), 4);
    }

    #[test]
    fn same_type_uses_the_extension() {
        assert_eq!(same_type("a.PDF", false).as_deref(), Some("*.PDF"));
        assert_eq!(same_type("a.b.c", false).as_deref(), Some("*.c"));
        assert_eq!(same_type("a.PDF", true), None);
        assert_eq!(same_type("noext", false), None);
        assert_eq!(same_type(".gitignore", false), None);
        assert_eq!(same_type("a.x;y", false), None);
    }

    #[test]
    fn archives_match_by_name() {
        assert!(real_folder(SFGAO_FOLDER.0));
        assert!(!real_folder(SFGAO_FOLDER.0 | SFGAO_STREAM.0));
        assert!(!real_folder(SFGAO_STREAM.0));
        assert!(!real_folder(0));
    }

    #[test]
    fn resolve_and_restore_leave_a_share_unprobed() {
        // `exists` on a dead share blocks for the redirector timeout. These stay the typed text.
        assert_eq!(resolve(r"\\10.255.255.2\x".into()), r"\\10.255.255.2\x");
        assert_eq!(resolve(r"//10.255.255.2/x".into()), r"//10.255.255.2/x");
        assert_eq!(resolve(r"\\?\UNC\server\share".into()), r"\\?\UNC\server\share");
        let s = Session::parse("p0=\\\\10.255.255.2\\x\n");
        assert_eq!(s.paths[0], r"\\10.255.255.2\x");
    }
}

#[cfg(test)]
mod swap_tests {
    use super::*;

    #[test]
    fn swap_notes_the_order_and_a_second_swap_clears_it() {
        let mut cur = Session::default();
        cur.paths[0] = "A".into();
        cur.paths[1] = "B".into();
        let mut base = cur.clone();
        swap_in(&mut cur, Some(&mut base), 0, 1);
        assert_eq!(cur.paths[0], "B");
        assert_eq!(base.paths[0], "B");
        assert!(order_moved(&cur.order));
        assert!(session_dirty(&cur, Some(&base)));
        assert!(!layout_differs(&cur, &base));
        swap_in(&mut cur, Some(&mut base), 0, 1);
        assert!(cur.order.is_empty());
        assert_eq!(cur.paths[0], "A");
        assert!(!session_dirty(&cur, Some(&base)));
        let mut saved = Session::default();
        saved.paths[0] = "A".into();
        saved.paths[1] = "B".into();
        swap_in(&mut cur, None, 0, 1);
        saved.reorder(&cur.order);
        assert_eq!(saved.paths[0], cur.paths[0]);
        assert_eq!(saved.paths[1], cur.paths[1]);
    }
}

#[cfg(test)]
mod tab_name_tests {
    use super::*;

    #[test]
    fn folder_name_drive_root_and_unc() {
        assert_eq!(folder_display_name(r"C:\Users\me\Documents"), "Documents");
        assert_eq!(folder_display_name(r"C:\Users\me\Documents\"), "Documents");
        assert_eq!(folder_display_name(r"C:\"), "C:");
        assert_eq!(folder_display_name("C:"), "C:");
        assert_eq!(folder_display_name("D:/"), "D:");
        assert_eq!(folder_display_name(r"\\server\share\docs"), "docs");
        assert_eq!(folder_display_name(r"\\server\share\"), "share");
        assert_eq!(folder_display_name(r"\\server\share"), "share");
        assert_eq!(folder_display_name(r"\\server"), "server");
        assert_eq!(folder_display_name(""), "");
        assert_eq!(folder_display_name(r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"), "This PC");
        // A shell name that is not This PC has no leaf. The path itself is the fallback.
        assert_eq!(folder_display_name(r"::{DEADBEEF}"), r"::{DEADBEEF}");
    }

    #[test]
    fn effective_label_uses_custom_or_the_folder() {
        assert_eq!(tab_label("Work", r"C:\other"), "Work");
        assert_eq!(tab_label("  Pics  ", r"D:\"), "Pics");
        assert_eq!(tab_label("1", r"C:\Users\me\Documents"), "Documents");
        assert_eq!(tab_label("12", r"D:\"), "D:");
        assert_eq!(tab_label("", r"C:\a\b"), "b");
        assert_eq!(tab_label("1", r"\\srv\share"), "share");
        // No folder yet: the stored token keeps the tab from going blank.
        assert_eq!(tab_label("1", ""), "1");
        assert_eq!(tab_label("", ""), "");
        assert_eq!(tab_label("1", r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"), "This PC");
        assert_eq!(stored_tab_name("  Work  "), "Work");
        assert_eq!(stored_tab_name("1"), "");
        assert_eq!(stored_tab_name("   "), "");
        assert_eq!(stored_tab_name(""), "");
        assert_eq!(stored_tab_name(&"가".repeat(100)).chars().count(), 80);
        assert_eq!(tab_tip_text(&[r"C:\a", "", r"D:\b"]), "C:\\a\r\nD:\\b");
        assert_eq!(tab_tip_text(&["", ""]), "");
    }

    #[test]
    fn legacy_numeric_name_stays_stored_and_displays_as_the_folder() {
        let old = serde_json::json!({
            "name": "1",
            "locked": false,
            "layout": {"columns": [1], "active": 0},
            "panes": [{"index": 0, "path": "C:\\Work\\docs"}]
        });
        let t = tab_from_value(&old).unwrap();
        assert_eq!(t.name, "1");
        assert_eq!(tab_label(&t.name, &t.paths[t.active]), "docs");

        // A missing title is still the automatic token, same as older files.
        let blank = serde_json::json!({"name": "", "locked": true, "layout": {"columns": [1]}});
        let t = tab_from_value(&blank).unwrap();
        assert_eq!((t.name.as_str(), t.locked), ("1", true));
        assert_eq!(tab_label(&t.name, r"E:\photos"), "photos");

        let custom = serde_json::json!({
            "name": "Work",
            "layout": {"columns": [2], "active": 1},
            "panes": [{"index": 0, "path": "D:\\"}, {"index": 1, "path": "E:\\pics"}]
        });
        let t = tab_from_value(&custom).unwrap();
        assert_eq!(t.name, "Work");
        assert_eq!(tab_label(&t.name, &t.paths[t.active]), "Work");
        assert_eq!(tab_tip_text(&[&t.paths[0], &t.paths[1]]), "D:\\\r\nE:\\pics");

        let auto = Tab { name: "1".into(), ..t.clone() };
        assert_eq!(duplicate_name(&[t.clone()], ""), "1");
        assert_eq!(duplicate_name(&[auto.clone()], "1"), "2");
        assert_eq!(duplicate_name(&[auto], ""), "2");
        assert_eq!(duplicate_name(&[t], "Work"), "Work (2)");
    }
}

#[cfg(test)]
mod face_tests {
    use super::*;

    #[test]
    fn language_change_clears_font_face() {
        let mut s = Session::default();
        s.lang = Lang::En;
        s.font_face = "Consolas".into();
        s.choose_lang(Lang::En);
        assert_eq!(s.font_face, "Consolas");
        assert_eq!(s.lang, Lang::En);
        s.choose_lang(Lang::Ja);
        assert!(s.font_face.is_empty());
        assert_eq!(s.lang, Lang::Ja);
        assert_eq!(crate::default_face(2), "Yu Gothic UI");
        s.font_face = "Malgun Gothic".into();
        s.choose_lang(Lang::Ko);
        assert!(s.font_face.is_empty());
        assert_eq!(crate::default_face(1), "Malgun Gothic");
        assert_eq!(crate::default_face(0), "Segoe UI");
    }
}

#[cfg(test)]
mod session_copy_tests {
    use super::copy_old_sessions;

    #[test]
    fn copy_old_sessions_leaves_the_source_files() {
        let root = std::env::temp_dir().join(format!("maguro-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = root.join("sessions");
        let new = root.join("maguro-sessions");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("work.grids"), b"one").unwrap();
        std::fs::write(old.join("old.qdr"), b"two").unwrap();
        std::fs::write(old.join("notes.txt"), b"skip").unwrap();
        let grids_mtime = std::fs::metadata(old.join("work.grids")).unwrap().modified().unwrap();
        let qdr_mtime = std::fs::metadata(old.join("old.qdr")).unwrap().modified().unwrap();
        copy_old_sessions(&old, &new);
        assert_eq!(std::fs::read(old.join("work.grids")).unwrap(), b"one");
        assert_eq!(std::fs::read(old.join("old.qdr")).unwrap(), b"two");
        assert_eq!(std::fs::metadata(old.join("work.grids")).unwrap().modified().unwrap(), grids_mtime);
        assert_eq!(std::fs::metadata(old.join("old.qdr")).unwrap().modified().unwrap(), qdr_mtime);
        assert_eq!(std::fs::read(new.join("work.grids")).unwrap(), b"one");
        assert_eq!(std::fs::read(new.join("old.qdr")).unwrap(), b"two");
        assert!(!new.join("notes.txt").exists());
        assert!(old.join("work.grids").is_file());
        assert!(old.join("old.qdr").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}


#[cfg(test)]
mod ratio_tests {
    use super::fix;

    #[test]
    fn non_finite_ratios_fall_back_to_even() {
        assert_eq!(fix(vec![f64::INFINITY, 1.0], 2), vec![0.5, 0.5]);
        assert_eq!(fix(vec![1e308, 1e308], 2), vec![0.5, 0.5]);
        assert_eq!(fix(vec![1.0, 3.0], 2), vec![0.25, 0.75]);
    }

    #[test]
    fn empty_heights_are_safe() {
        assert_eq!(crate::layout_label(&[]), "");
        let mut s = crate::Session::default();
        s.set_heights(Vec::new());
        assert_eq!(s.heights, vec![1]);
    }
}

#[cfg(test)]
mod session_assoc_tests {
    use super::*;

    fn two_tabs() -> Session {
        let mut s = Session::default();
        s.paths[0] = r"C:\a".into();
        s.anchor();
        let mut second = s.tabs[0].clone();
        second.name = "B".into();
        second.paths[0] = r"D:\b".into();
        second.session.clear();
        s.tabs.push(second);
        s.tab = 0;
        settle_sessions(&mut s);
        s
    }

    #[test]
    fn save_uses_only_the_active_tabs_file() {
        let mut s = two_tabs();
        assert!(bind_tab_session(&mut s, 0, r"C:\s\a.grids"));
        assert_eq!(save_choice(&s.tabs, 0), SaveChoice::Overwrite(r"C:\s\a.grids"));
        assert_eq!(save_choice(&s.tabs, 1), SaveChoice::AskName);
        assert_eq!(s.tabs[1].session, "");
        assert_eq!(s.file, r"C:\s\a.grids");

        s.tab = 1;
        settle_sessions(&mut s);
        assert_eq!(s.file, "");
        assert_eq!(save_choice(&s.tabs, 1), SaveChoice::AskName);
        assert!(bind_tab_session(&mut s, 1, r"D:\s\b.grids"));
        assert_eq!(s.tabs[0].session, r"C:\s\a.grids");
        assert_eq!(s.tabs[1].session, r"D:\s\b.grids");
        assert_eq!(save_choice(&s.tabs, 1), SaveChoice::Overwrite(r"D:\s\b.grids"));
        assert_eq!(save_choice(&s.tabs, 0), SaveChoice::Overwrite(r"C:\s\a.grids"));
        assert_eq!(s.file, r"D:\s\b.grids");
    }

    #[test]
    fn load_and_save_do_not_retarget_another_tab() {
        let mut s = two_tabs();
        bind_tab_session(&mut s, 0, r"C:\s\a.grids");
        bind_tab_session(&mut s, 1, r"D:\s\b.grids");
        let mut incoming = s.tabs[1].clone();
        incoming.paths[0] = r"E:\loaded".into();
        incoming.heights = vec![1];
        let links = load_into_tab(&mut s, 1, &incoming, r"E:\s\new.grids");
        assert_eq!(links, vec![r"C:\s\a.grids".to_string(), r"E:\s\new.grids".into()]);
        assert_eq!(s.tabs[0].session, r"C:\s\a.grids");
        assert_eq!(s.tabs[1].paths[0], r"E:\loaded");
        assert_eq!(s.file, r"C:\s\a.grids"); // tab 0 is still on screen

        s.tab = 1;
        s.publish();
        assert_eq!(s.paths[0], r"E:\loaded");
        assert_eq!(s.file, r"E:\s\new.grids");
        assert_eq!(s.tabs[0].session, r"C:\s\a.grids");

        let links = load_into_tab(&mut s, 1, &incoming, r"E:\s\again.grids");
        assert_eq!(links[0], r"C:\s\a.grids");
        assert_eq!(links[1], r"E:\s\again.grids");
        assert_eq!(s.tabs[0].paths[0], r"C:\a");
    }

    #[test]
    fn old_window_session_lands_on_the_active_tab_only() {
        let bare = serde_json::json!({
            "name": "1",
            "locked": false,
            "layout": {"columns": [1], "active": 0},
            "panes": [{"index": 0, "path": "C:\\Work"}]
        });
        let t = tab_from_value(&bare).unwrap();
        assert!(t.session.is_empty());

        let with = serde_json::json!({
            "name": "Work",
            "session": "C:\\s\\work.grids",
            "layout": {"columns": [1]},
            "panes": []
        });
        assert_eq!(tab_from_value(&with).unwrap().session, r"C:\s\work.grids");

        let tabs = serde_json::json!([
            {"name": "A", "layout": {"columns": [1]}, "panes": [{"index": 0, "path": "C:\\a"}]},
            {"name": "B", "active": true, "layout": {"columns": [1]}, "panes": [{"index": 0, "path": "D:\\b"}]}
        ]);
        let old = format!("session=C:\\s\\win.grids\ntabs={tabs}\n");
        let s = Session::parse(&old);
        assert_eq!(s.tab, 1);
        assert_eq!(s.tabs[1].session, r"C:\s\win.grids");
        assert_eq!(s.tabs[0].session, "");
        assert_eq!(s.file, r"C:\s\win.grids");
        assert_eq!(s.paths[0], r"D:\b");

        let text = s.to_ini();
        let back = Session::parse(&text);
        assert_eq!(back.tabs[0].session, "");
        assert_eq!(back.tabs[1].session, r"C:\s\win.grids");
        assert_eq!(back.file, r"C:\s\win.grids");
        assert_eq!(back.tab, 1);

        // Per-tab paths win over a stale window-level session= line.
        let mut kept = s.clone();
        kept.tabs[0].session = r"C:\s\a.grids".into();
        kept.file = r"C:\s\stale.grids".into();
        settle_sessions(&mut kept);
        assert_eq!(kept.tabs[0].session, r"C:\s\a.grids");
        assert_eq!(kept.tabs[1].session, r"C:\s\win.grids");
        assert_eq!(kept.file, r"C:\s\win.grids");
    }

    #[test]
    fn named_session_text_does_not_embed_the_path() {
        let mut s = two_tabs();
        bind_tab_session(&mut s, 0, r"C:\s\a.grids");
        bind_tab_session(&mut s, 1, r"D:\s\b.grids");
        let text = s.snapshot();
        assert!(!text.contains("session="), "{text}");
        assert!(!text.contains("a.grids"), "{text}");
        assert!(!text.contains("b.grids"), "{text}");
        let one = tab_session_text(&s, 1).unwrap();
        assert!(!one.contains("b.grids"), "{one}");
        assert!(one.contains(r"D:\b") || one.contains("D:\\\\b"), "{one}");
    }

    #[test]
    fn dirty_follows_the_active_tab() {
        let mut s = two_tabs();
        s.publish();
        let clean0 = s.tabs[0].clone();
        let clean1 = s.tabs[1].clone();
        assert!(!active_tab_dirty(&s, Some(&clean0), &[]));

        // Folder changes do not light the dot. A layout change on this tab does.
        s.paths[0] = r"C:\changed".into();
        assert!(!active_tab_dirty(&s, Some(&clean0), &[]));
        s.heights = vec![3];
        assert!(active_tab_dirty(&s, Some(&clean0), &[]));
        // Tab B's baseline still matches tab B, once that tab is on screen.
        s.anchor();
        s.tab = 1;
        s.publish();
        assert!(!active_tab_dirty(&s, Some(&clean1), &[]));
        assert_eq!(s.tabs[0].heights, vec![3]);
        assert_eq!(s.tabs[0].paths[0], r"C:\changed");

        s.order = vec![1, 0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        assert!(active_tab_dirty(&s, Some(&clean1), &s.order));
        assert!(!active_tab_dirty(&s, Some(&clean1), &[]));
    }

    #[test]
    fn fork_rename_and_delete_keep_other_links() {
        let mut s = two_tabs();
        bind_tab_session(&mut s, 0, r"C:\s\a.grids");
        bind_tab_session(&mut s, 1, r"D:\s\b.grids");
        let copy = fork_tab(&s.tabs[0], "A (2)".into());
        assert_eq!(s.tabs[0].session, r"C:\s\a.grids");
        assert!(copy.session.is_empty());
        assert!(!copy.locked);
        assert_eq!(copy.name, "A (2)");

        let before = vec![s.tabs[0].session.clone(), s.tabs[1].session.clone()];
        let now = before.clone();
        let linked = vec![true, false];
        let cleared = sessions_after_delete(&before, &now, &linked, true).unwrap();
        assert_eq!(cleared, vec!["".to_string(), r"D:\s\b.grids".into()]);
        assert!(sessions_after_delete(&before, &now, &linked, false).is_none());
        // Dialog retargeted tab 0 before the delete finished.
        let moved = vec![r"C:\s\other.grids".into(), before[1].clone()];
        assert!(sessions_after_delete(&before, &moved, &linked, true).is_none());

        let renamed = sessions_after_rename(&before, &now, &[false, true], r"D:\s\b2.grids").unwrap();
        assert_eq!(renamed, vec![r"C:\s\a.grids".to_string(), r"D:\s\b2.grids".into()]);
        // Tab 0 changed during the dialog and was not linked to B. Tab 1 still is, so only B follows the rename.
        let followed = sessions_after_rename(&before, &moved, &[false, true], r"D:\s\b2.grids").unwrap();
        assert_eq!(followed, vec![r"C:\s\other.grids".to_string(), r"D:\s\b2.grids".into()]);
        // Tab 1 was pointed somewhere else while the dialog was up, so the rename leaves it.
        let moved_b = vec![before[0].clone(), r"E:\s\elsewhere.grids".into()];
        assert!(sessions_after_rename(&before, &moved_b, &[false, true], r"D:\s\b2.grids").is_none());
    }

    #[test]
    fn link_loaded_window_binds_only_the_active_tab() {
        let mut s = two_tabs();
        bind_tab_session(&mut s, 0, r"C:\s\a.grids");
        s.tab = 1;
        link_loaded_window(&mut s, r"E:\s\boot.grids");
        assert_eq!(s.tabs[0].session, "");
        assert_eq!(s.tabs[1].session, r"E:\s\boot.grids");
        assert_eq!(s.file, r"E:\s\boot.grids");
        assert_eq!(save_choice(&s.tabs, 0), SaveChoice::AskName);
        assert_eq!(save_choice(&s.tabs, 1), SaveChoice::Overwrite(r"E:\s\boot.grids"));
    }

    #[test]
    fn window_snap_keeps_each_tabs_file_and_old_snaps_still_load() {
        let mut s = two_tabs();
        bind_tab_session(&mut s, 0, r"C:\s\a.grids");
        bind_tab_session(&mut s, 1, r"D:\s\b.grids");
        s.tab = 0;
        settle_sessions(&mut s);
        let back = crate::session_from_window(&crate::window_value(&s), &Session::default());
        assert_eq!(back.tabs[0].session, r"C:\s\a.grids");
        assert_eq!(back.tabs[1].session, r"D:\s\b.grids");
        assert_eq!(back.file, r"C:\s\a.grids");
        assert_eq!(back.tab, 0);

        let old = serde_json::json!({
            "session": "C:\\s\\win.grids",
            "tabs": [
                {"name": "A", "layout": {"columns": [1]}, "panes": [{"index": 0, "path": "C:\\a"}]},
                {"name": "B", "active": true, "layout": {"columns": [1]}, "panes": [{"index": 0, "path": "D:\\b"}]}
            ]
        });
        let back = crate::session_from_window(&old, &Session::default());
        assert_eq!(back.tab, 1);
        assert_eq!(back.tabs[0].session, "");
        assert_eq!(back.tabs[1].session, r"C:\s\win.grids");
        assert_eq!(back.file, r"C:\s\win.grids");
        assert_eq!(back.paths[0], r"D:\b");
    }

    #[test]
    fn baseline_comes_from_that_tabs_file() {
        let dir = std::env::temp_dir().join(format!("maguro-tab-base-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut saved = Session::default();
        saved.paths[0] = r"C:\saved".into();
        saved.heights = vec![1];
        saved.anchor();
        let path = dir.join("a.grids");
        std::fs::write(&path, saved.snapshot()).unwrap();

        let mut live = two_tabs();
        bind_tab_session(&mut live, 0, &path.to_string_lossy());
        live.tabs[0].paths[0] = r"C:\drift".into();
        live.tabs[0].heights = vec![2];
        live.tabs[1].session = dir.join("missing.grids").to_string_lossy().into_owned();
        let bases = initial_tab_bases(&live);
        assert_eq!(bases[0].grid.paths[0], r"C:\saved");
        assert_eq!(bases[0].grid.heights, vec![1]);
        assert_eq!(bases[0].grid.session, path.to_string_lossy());
        // An unreadable file leaves the tab as it is, so the dot stays off.
        assert_eq!(bases[1].grid.paths[0], r"D:\b");
        assert_eq!(bases[1].grid.session, live.tabs[1].session);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pane_swap_does_not_look_like_a_layout_change() {
        let mut cur = Session::default();
        cur.views[0] = (4, 16);
        cur.views[1] = (3, 16);
        cur.cols[0] = "a".into();
        cur.cols[1] = "b".into();
        cur.anchor();
        let mut grid = cur.tabs[0].clone();
        swap_in(&mut cur, None, 0, 1);
        swap_tab_slots(&mut grid, 0, 1);
        cur.anchor();
        assert!(!tab_layout_differs(&cur, &grid));
        assert!(active_tab_dirty(&cur, Some(&grid), &cur.order));
        swap_in(&mut cur, None, 0, 1);
        swap_tab_slots(&mut grid, 0, 1);
        cur.anchor();
        assert!(cur.order.is_empty());
        assert!(!active_tab_dirty(&cur, Some(&grid), &cur.order));
        assert_eq!(cur.views[0], (4, 16));
        assert_eq!(grid.cols[0], "a");
    }
}

#[cfg(test)]
mod view_menu_tests {
    use super::*;

    #[test]
    fn explorer_presets_match_mode_and_icon_size() {
        assert_eq!(explorer_view_index(5, 256), Some(0));
        assert_eq!(explorer_view_index(1, 96), Some(1));
        assert_eq!(explorer_view_index(1, 0), Some(1));
        assert_eq!(explorer_view_index(1, 80), Some(1));
        assert_eq!(explorer_view_index(1, 48), Some(2));
        assert_eq!(explorer_view_index(1, 64), Some(2));
        assert_eq!(explorer_view_index(2, 16), Some(3));
        assert_eq!(explorer_view_index(3, 16), Some(4));
        assert_eq!(explorer_view_index(4, 16), Some(5));
        assert_eq!(explorer_view_index(4, 0), Some(5));
        assert_eq!(explorer_view_index(6, 48), Some(6));
        assert_eq!(explorer_view_index(8, 32), Some(7));
        assert_eq!(explorer_view_index(7, 16), None);
        assert_eq!(tr(EXPLORER_VIEWS[0].2), "Extra large icons");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(tr(EXPLORER_VIEWS[2].2), "보통 아이콘");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(tr(EXPLORER_VIEWS[7].2), "コンテンツ");
        UI.store(0, Ordering::Relaxed);
    }

    #[test]
    fn new_pane_default_keeps_old_modes_and_a_medium_icon_size() {
        assert_eq!(Session::default().default_view(), (4, 16));
        let mut s = Session::default();
        s.view_new = 1;
        assert_eq!(s.default_view(), (1, 96));
        s.view_icon = 48;
        assert_eq!(s.default_view(), (1, 48));
        s.view_new = 6;
        s.view_icon = 0;
        assert_eq!(s.default_view(), (6, 48));
        s.view_new = 8;
        assert_eq!(s.default_view(), (8, 32));
        s.view_new = 5;
        assert_eq!(s.default_view(), (5, 256));
        s.view_new = 2;
        assert_eq!(s.default_view(), (2, 16));
        assert_eq!(stored_view_icon(1, 96), 0);
        assert_eq!(stored_view_icon(1, 48), 48);
        assert_eq!(stored_view_icon(4, 16), 0);
        assert_eq!(Session::parse("viewnew=list").view_new, 3);
        assert_eq!(Session::parse("viewnew=icon").view_new, 1);
        assert_eq!(Session::parse("viewnew=6").view_new, 6);
        assert_eq!(Session::parse("viewnew=8").view_new, 8);
        assert_eq!(Session::parse("viewnew=2").view_new, 2);
        assert_eq!(Session::parse("viewnew=5").view_new, 5);
        assert_eq!(Session::parse("viewnew=nope").view_new, 4);
        s.view_new = 8;
        s.view_icon = 0;
        let ini = s.to_ini();
        assert!(ini.contains("viewnew=8\n"));
        assert!(!ini.contains("viewicon"));
        assert!(!ini.contains("grids.ini"));
        assert_eq!(Session::parse(&ini).view_new, 8);
        assert_eq!(Session::parse(&ini).view_icon, 0);
    }
}
