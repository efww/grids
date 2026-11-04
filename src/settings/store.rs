// Settings file: %APPDATA%\Maguro\maguro.json, or next to the exe when portable.txt is there.
// Named .maguro sessions stay in the ini shape. A missing maguro.json starts from the defaults.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use windows::core::HSTRING;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH};
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use crate::{fav_path, fix, Lang, Session, MAXP, RECENT};

fn v1() -> u32 {
    1
}
fn yes() -> bool {
    true
}

fn no() -> bool {
    false
}

fn half_split() -> f64 {
    0.5
}
fn side_fav() -> String {
    "fav".into()
}
fn side_is_fav(v: &String) -> bool {
    v != "tree"
}
fn pins_open() -> bool {
    true
}
fn pins_are_open(v: &bool) -> bool {
    *v
}
fn is_on(v: &bool) -> bool {
    *v
}
fn is_off(v: &bool) -> bool {
    !*v
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ExcelTidyJson {
    #[serde(default = "yes")]
    selection: bool,
    #[serde(default = "yes")]
    zoom: bool,
    #[serde(default = "yes")]
    first_sheet: bool,
    #[serde(default = "no")]
    backup: bool,
}

fn excel_tidy_default() -> ExcelTidyJson {
    ExcelTidyJson { selection: true, zoom: true, first_sheet: true, backup: false }
}

fn excel_tidy_is_default(v: &ExcelTidyJson) -> bool {
    v.selection && v.zoom && v.first_sheet && !v.backup
}
fn is_zero_i32(v: &i32) -> bool {
    *v == 0
}

fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}
fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}
fn sort_by_name() -> u32 {
    1
}
fn is_name_sort(v: &u32) -> bool {
    *v == 1
}

/// A bad `ev_cols` (wrong type, wrong length, out of order) is the default widths.
/// The rest of maguro.json still loads.
fn de_ev_cols<'de, D>(d: D) -> Result<Option<[f64; 3]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = Value::deserialize(d)?;
    Ok(ev_cols_json(&v))
}

fn ev_cols_json(v: &Value) -> Option<[f64; 3]> {
    let a = v.as_array()?;
    if a.len() != 3 {
        return None;
    }
    let mut c = [0.0; 3];
    for (i, n) in a.iter().enumerate() {
        c[i] = n.as_f64()?;
    }
    crate::everything::ev_cols_ok(c)
}
fn files_open() -> bool {
    true
}
fn files_are_open(v: &bool) -> bool {
    *v
}
fn is_half_split(v: &f64) -> bool {
    (v - 0.5).abs() < 1e-6
}
fn accent_light() -> String {
    "0A9BD6".into()
}
fn accent_dark() -> String {
    "0A9BD6".into()
}
fn theme_auto() -> String {
    "auto".into()
}
fn lang_auto() -> String {
    "auto".into()
}
fn view_details() -> i32 {
    4
}
fn left_right() -> Vec<usize> {
    // Missing layout.columns in an older maguro.json: keep two panes (layout_new omitted also meant 0).
    // A brand-new Session::default() uses layout_new 3 (one pane) instead.
    crate::layout_heights_for(0)
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct WinJson {
    #[serde(default)]
    left: i32,
    #[serde(default)]
    top: i32,
    #[serde(default)]
    right: i32,
    #[serde(default)]
    bottom: i32,
    #[serde(default)]
    maximized: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct LayoutJson {
    #[serde(default = "left_right")]
    columns: Vec<usize>,
    #[serde(default)]
    colr: Vec<f64>,
    #[serde(default)]
    rowr: Vec<Vec<f64>>,
    #[serde(default)]
    active: usize,
}

impl Default for LayoutJson {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap_or(LayoutJson { columns: left_right(), colr: Vec::new(), rowr: Vec::new(), active: 0 })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct PaneJson {
    #[serde(default)]
    index: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    view: Vec<i32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    columns: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    filter: String,
    /// Pane tabs: folders this pane switches between. The current one is also `path`. Empty means one folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    folders: Vec<String>,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    folder: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct FavJson {
    path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
}

/// Quick look sizes in DIP. `image` is pictures; `doc` is text and PDF. Absent means fit the file.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct PeekJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<[i32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    doc: Option<[i32; 2]>,
}

fn peek_pair(v: Option<[i32; 2]>) -> Option<(i32, i32)> {
    let [w, h] = v?;
    (w > 0 && h > 0 && w <= 16_000 && h <= 16_000).then_some((w, h))
}

fn peek_json(s: &Session) -> Option<PeekJson> {
    let image = s.peek_image.map(|(w, h)| [w, h]);
    let doc = s.peek_doc.map(|(w, h)| [w, h]);
    (image.is_some() || doc.is_some()).then_some(PeekJson { image, doc })
}

/// Version 1. `tabs` is the focused window. `windows` is every window, focused first.
/// `favorites_tree` is the favourites tree. Unknown keys land in `extra` and are written back.
#[derive(Serialize, Deserialize, Clone, Debug)]
struct SettingsFile {
    #[serde(default = "v1")]
    version: u32,
    #[serde(default)]
    window: Option<WinJson>,
    #[serde(default)]
    side: i32,
    /// DPI `side` was measured at. Missing in older files: no rescale.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    side_dpi: i32,
    #[serde(default)]
    layout: LayoutJson,
    /// Permutation of pane slots since the named session was saved. Empty means they match the file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    order: Vec<u8>,
    #[serde(default)]
    panes: Vec<PaneJson>,
    #[serde(default = "accent_light")]
    accent: String,
    #[serde(default = "accent_dark")]
    accent_dark: String,
    #[serde(default)]
    dark: bool,
    #[serde(default = "theme_auto")]
    theme: String,
    /// UI face. Missing or empty uses the language default.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    font_face: String,
    #[serde(default = "yes")]
    bar: bool,
    /// Drive buttons on the tab row. Missing means on. Omitted while on, so an older file stays unchanged.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    drive_buttons: bool,
    #[serde(default = "yes")]
    stat: bool,
    #[serde(default = "view_details")]
    view_new: i32,
    /// Icon size for `view_new`. Missing or 0 is that mode's usual size (large icons stay 96).
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    view_icon: i32,
    #[serde(default)]
    start: u8,
    #[serde(default)]
    start_path: String,
    #[serde(default = "yes")]
    ext: bool,
    #[serde(default = "yes")]
    tree: bool,
    /// Retired activity-bar choice ("tree" / "fav"). Read so it is not kept, never written.
    #[serde(default, skip_serializing)]
    #[allow(dead_code)]
    panel: String,
    /// Folder tree's share of the left panel. Favourites use the rest. Missing means even.
    #[serde(default = "half_split", skip_serializing_if = "is_half_split")]
    side_split: f64,
    /// Which half is on top. `"tree"` puts the folder tree above favourites.
    /// Missing, `"fav"`, or anything else is favourites on top, and the key is omitted.
    #[serde(default = "side_fav", skip_serializing_if = "side_is_fav")]
    side_order: String,
    /// Explorer pins group expanded. The pinned folders themselves are not stored.
    #[serde(default = "pins_open", skip_serializing_if = "pins_are_open")]
    explorer_pins_open: bool,
    /// Explorer favourite-files group expanded. The files themselves are not stored.
    /// Omitted when open, which is the default. The group is hidden when Explorer has no such files.
    #[serde(default = "files_open", skip_serializing_if = "files_are_open")]
    explorer_files_open: bool,
    #[serde(default = "yes")]
    edge: bool,
    #[serde(default = "yes")]
    confirm: bool,
    #[serde(default)]
    del: u8,
    #[serde(default)]
    new_name: String,
    #[serde(default)]
    drop: u8,
    /// Paste and drag use robocopy when a network folder is involved. Missing means off.
    /// Omitted while off. Fast paste on the menu does not read this.
    #[serde(default, skip_serializing_if = "is_off")]
    fast_copy: bool,
    /// New tab / new window. Omitted when 0 (two panes). Fresh Session::default uses 3 (one pane) and writes it.
    /// A missing key still loads as 0 so existing configs stay two panes.
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    layout_new: u8,
    #[serde(default)]
    boot: u8,
    #[serde(default)]
    boot_file: String,
    #[serde(default)]
    autosave: bool,
    #[serde(default = "yes")]
    dot: bool,
    /// Check GitHub for a newer release at startup. Missing means on.
    #[serde(default = "yes")]
    update_check: bool,
    #[serde(default = "yes")]
    dblup: bool,
    /// Global hotkey. Missing means on, so an older settings file keeps the hotkey on.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    hotkey: bool,
    /// Folders in another app's Open or Save dialog. Missing means on.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    dialog_jump: bool,
    /// Open a clipboard path in a new tab. Missing means on.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    clip_hotkey: bool,
    /// UI language. Missing means `"auto"`: the Windows display language (Korean, Japanese, otherwise English).
    #[serde(default = "lang_auto")]
    lang: String,
    #[serde(default = "yes")]
    single: bool,
    /// One window. Missing means on: a second launch and new-window actions open tabs in the one window.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    one_window: bool,
    /// Subfolder search uses Everything when it is running. Missing means on.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    use_everything: bool,
    /// Space in a file list opens quick look. Missing means on. Omitted while on, so an older file stays unchanged.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    space_preview: bool,
    /// Right-hand preview pane. Missing means hidden. Omitted while hidden.
    #[serde(default, skip_serializing_if = "is_off")]
    preview_pane: bool,
    /// Preview width in DIP. 0 means the default until the splitter is dragged. Omitted while 0.
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    preview_width: i32,
    /// Wheel in quick look steps to the next or previous file past the content end.
    /// Missing means off; it is on only when the user ticks it. Omitted while off.
    #[serde(default, skip_serializing_if = "is_wheel_off")]
    peek_wheel_nav: bool,
    /// Remember sort, columns, view mode, icon size and grouping for each folder.
    /// Missing means on. The views themselves live in folder-views.json.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    remember_folder_view: bool,
    /// Status line shows a selected folder's total. Missing means on. Omitted while on.
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    show_folder_sizes: bool,
    /// Size UNC and mapped drives. Missing means off. Omitted while off.
    #[serde(default, skip_serializing_if = "is_off")]
    folder_sizes_network: bool,
    /// Quick look window size in DIP, per kind. Omitted until the user resizes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    peek: Option<PeekJson>,
    /// The one-time Everything install question was shown. Missing means not yet.
    #[serde(default, skip_serializing_if = "is_off")]
    ev_asked: bool,
    /// Everything result sort code. Missing means name A→Z. A named session file does not store this.
    #[serde(default = "sort_by_name", skip_serializing_if = "is_name_sort")]
    ev_sort: u32,
    /// Everything column borders as fractions of the row (name|path, path|size, size|date).
    /// Missing, or any value that is not three increasing fractions inside (0, 1), means the default widths.
    /// maguro.json only. A .maguro file does not store this.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "de_ev_cols")]
    ev_cols: Option<[f64; 3]>,
    /// The one-time rclone and WinFsp install question was shown. Missing means not yet.
    #[serde(default, skip_serializing_if = "is_off")]
    remote_asked: bool,
    /// WinMergeU.exe, or a folder that contains it. Empty means auto-detect. Omitted while empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    winmerge_path: String,
    #[serde(default)]
    session: String,
    #[serde(default)]
    favorites: Vec<FavJson>,
    #[serde(default)]
    recent: Vec<String>,
    #[serde(default)]
    tabs: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    windows: Vec<Value>,
    #[serde(default)]
    keybindings: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    favorites_tree: Vec<Value>,
    /// Third-party context-menu targets turned off in Maguro. Omitted when every handler is on.
    /// A .maguro file does not store this.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    ctx_menu: BTreeMap<String, Vec<String>>,
    /// Menu strings observed for each handler id. Omitted until a menu has been opened.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    ctx_labels: BTreeMap<String, Vec<String>>,
    /// Targets explicitly enabled for handlers that start off (WinMerge). Omitted while empty,
    /// which keeps those commands hidden. A .maguro file does not store this.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    ctx_on: BTreeMap<String, Vec<String>>,
    /// Handler ids that added no visible menu item. Omitted until a menu has been learned.
    /// They leave the settings list only. A .maguro file does not store this.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    ctx_quiet: BTreeSet<String>,
    /// Older denylist (`packaged:{clsid}`). Read so a saved file does not keep the key
    /// in `extra`, then dropped. Empty used to mean all on. Not written back.
    #[serde(default, skip_serializing)]
    #[allow(dead_code)]
    ctx_packaged_off: BTreeSet<String>,
    /// Packaged Windows 11 commands the user turned on. Empty means all off.
    /// A named session file does not store this.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    ctx_packaged_on: BTreeSet<String>,
    /// Set after the one-time switch that turns previously-on packaged commands off.
    /// Missing on an older file means that switch still has to run.
    #[serde(default, skip_serializing_if = "is_off")]
    ctx_win11_pack_migrated: bool,
    /// Settings right-click page is on the Windows 11 list. Omitted while classic.
    #[serde(default, skip_serializing_if = "is_off")]
    ctx_win11_view: bool,
    /// DLL file names blocked inside Maguro only. Omitted when empty.
    /// Not written to the registry. A named session file does not store this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    blocked_shell_extensions: Vec<String>,
    /// Workspace slots 1–9. Each entry is `{slot, name?, session?}`. Omitted when every slot is empty.
    /// A `.maguro` file does not store this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    workspaces: Vec<Value>,
    /// Ask before loading a workspace replaces the current tabs. Missing means no. Omitted while off.
    #[serde(default, skip_serializing_if = "is_off")]
    workspace_confirm: bool,
    /// Paths in the file basket. Omitted when empty. A named session file does not store this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    file_basket: Vec<String>,
    /// Last Excel tidy checkboxes. Omitted at the portable default (A1/zoom/first on, backup off).
    /// A named session file does not store this.
    #[serde(default = "excel_tidy_default", skip_serializing_if = "excel_tidy_is_default")]
    excel_tidy: ExcelTidyJson,
    /// Folders watched while Maguro is running. Not part of a session file.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    folder_watches: Vec<crate::watch::WatchJson>,
    #[serde(default, flatten)]
    extra: Map<String, Value>,
}

impl Default for SettingsFile {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap_or_else(|_| SettingsFile::blank())
    }
}

impl SettingsFile {
    fn blank() -> Self {
        SettingsFile {
            version: 1,
            window: None,
            side: 0,
            side_dpi: 0,
            layout: LayoutJson::default(),
            order: Vec::new(),
            panes: Vec::new(),
            accent: accent_light(),
            accent_dark: accent_dark(),
            dark: false,
            theme: theme_auto(),
            font_face: String::new(),
            bar: true,
            drive_buttons: true,
            stat: true,
            view_new: 4,
            view_icon: 0,
            start: 0,
            start_path: String::new(),
            ext: true,
            tree: true,
            panel: String::new(),
            side_split: 0.5,
            side_order: side_fav(),
            explorer_pins_open: true,
            explorer_files_open: true,
            edge: true,
            confirm: true,
            del: 0,
            new_name: String::new(),
            drop: 0,
            fast_copy: false,
            layout_new: 3,
            boot: 0,
            boot_file: String::new(),
            autosave: false,
            dot: true,
            update_check: true,
            dblup: true,
            hotkey: true,
            dialog_jump: true,
            clip_hotkey: true,
            lang: lang_auto(),
            single: true,
            one_window: true,
            use_everything: true,
            space_preview: true,
            preview_pane: false,
            preview_width: 0,
            peek_wheel_nav: false,
            remember_folder_view: true,
            show_folder_sizes: true,
            folder_sizes_network: false,
            peek: None,
            ev_asked: false,
            ev_sort: 1,
            ev_cols: None,
            remote_asked: false,
            winmerge_path: String::new(),
            session: String::new(),
            favorites: Vec::new(),
            recent: Vec::new(),
            tabs: Vec::new(),
            windows: Vec::new(),
            keybindings: Map::new(),
            favorites_tree: Vec::new(),
            ctx_menu: BTreeMap::new(),
            ctx_labels: BTreeMap::new(),
            ctx_on: BTreeMap::new(),
            ctx_quiet: BTreeSet::new(),
            ctx_packaged_off: BTreeSet::new(),
            ctx_packaged_on: BTreeSet::new(),
            ctx_win11_pack_migrated: false,
            ctx_win11_view: false,
            blocked_shell_extensions: Vec::new(),
            workspaces: Vec::new(),
            workspace_confirm: false,
            file_basket: Vec::new(),
            excel_tidy: excel_tidy_default(),
            folder_watches: Vec::new(),
            extra: Map::new(),
        }
    }

    fn from_session(s: &Session) -> Self {
        let shown = s.view_tabs();
        let at = s.tab.min(shown.len().saturating_sub(1));
        let filters = shown.get(at).map(|t| t.filters.clone()).unwrap_or_default();
        let mut panes = Vec::new();
        for i in 0..MAXP {
            let filter = filters.get(i).cloned().unwrap_or_default();
            let folders = s.pane_tab_list(i);
            let folders = if folders.len() > 1 { folders } else { Vec::new() };
            if s.paths[i].is_empty() && s.views[i] == (0, 0) && s.cols[i].is_empty() && filter.is_empty() && folders.is_empty() {
                continue;
            }
            panes.push(PaneJson {
                index: i,
                path: s.paths[i].clone(),
                view: if s.views[i].0 == 0 { Vec::new() } else { vec![s.views[i].0, s.views[i].1] },
                columns: s.cols[i].clone(),
                filter,
                folders: folders.clone(),
                folder: if folders.len() > 1 { s.pane_cur[i].min(folders.len() - 1) } else { 0 },
            });
        }
        let tree = if s.tree.is_empty() { crate::tree_from_flat(&s.favs) } else { s.tree.clone() };
        let flat = if s.tree.is_empty() { s.favs.clone() } else { crate::flat_favs(&s.tree) };
        let favorites = flat.iter().map(|f| {
            let path = fav_path(f).to_string();
            let name = f.split_once('\t').map(|(_, n)| n.to_string()).unwrap_or_default();
            FavJson { path, name }
        }).collect();
        SettingsFile {
            version: 1,
            window: s.win.map(|w| WinJson { left: w[0], top: w[1], right: w[2], bottom: w[3], maximized: w[4] != 0 }),
            side: s.side,
            side_dpi: s.side_dpi,
            layout: LayoutJson { columns: s.heights.clone(), colr: s.colr.clone(), rowr: s.rowr.clone(), active: s.active },
            order: s.order.clone(),
            panes,
            accent: format!("{:06X}", s.accent & 0xFFFFFF),
            accent_dark: format!("{:06X}", s.accent_dark & 0xFFFFFF),
            dark: s.dark,
            theme: ["auto", "light", "dark"][s.theme.min(2) as usize].into(),
            font_face: s.font_face.trim().to_string(),
            bar: s.bar,
            drive_buttons: s.drive_buttons,
            stat: s.stat,
            view_new: s.view_new,
            view_icon: s.view_icon,
            start: s.start_new.min(2),
            start_path: s.start_path.clone(),
            ext: s.ext,
            tree: s.tree_on,
            panel: String::new(),
            side_split: crate::sane_split(s.side_split),
            side_order: if s.fav_top { side_fav() } else { "tree".into() },
            explorer_pins_open: s.pins_open,
            explorer_files_open: s.files_open,
            edge: s.edge,
            confirm: s.confirm,
            del: s.del,
            new_name: s.new_name.clone(),
            drop: s.dropm.min(2),
            fast_copy: s.fast_copy,
            layout_new: s.layout_new.min(3),
            boot: s.boot.min(2),
            boot_file: s.boot_file.clone(),
            autosave: s.autosave,
            dot: s.dot,
            update_check: s.update_check,
            dblup: s.dblup,
            hotkey: s.hotkey,
            dialog_jump: s.dialog_jump,
            clip_hotkey: s.clip_hotkey,
            lang: s.lang.ini().into(),
            single: s.single,
            one_window: s.one_window,
            use_everything: s.use_everything,
            space_preview: s.space_preview,
            preview_pane: s.preview_on,
            preview_width: s.preview_w.max(0),
            peek_wheel_nav: s.peek_wheel_nav,
            remember_folder_view: s.remember_views,
            show_folder_sizes: s.folder_sizes,
            folder_sizes_network: s.folder_sizes_network,
            peek: peek_json(s),
            ev_asked: s.ev_asked,
            ev_sort: crate::everything::ev_sort_ok(s.ev_sort),
            ev_cols: s.ev_cols.and_then(crate::everything::ev_cols_ok),
            remote_asked: s.remote_asked,
            winmerge_path: s.winmerge_path.trim().to_string(),
            session: s.file.clone(),
            favorites,
            recent: s.recent.clone(),
            tabs: serde_json::from_str(&crate::tabs_json(&shown, at)).unwrap_or_default(),
            windows: crate::take_snaps(),
            keybindings: crate::keys::bindings_json(),
            favorites_tree: crate::tree_to_values(&tree),
            ctx_menu: clean_ctx(&s.ctx_off),
            ctx_labels: clean_labels(&s.ctx_labels),
            ctx_on: clean_ctx(&s.ctx_on),
            ctx_quiet: clean_quiet(&s.ctx_quiet),
            ctx_packaged_off: BTreeSet::new(),
            ctx_packaged_on: clean_packaged(&s.ctx_packaged_on),
            ctx_win11_pack_migrated: s.ctx_win11_pack_migrated,
            ctx_win11_view: s.ctx_win11_view,
            blocked_shell_extensions: clean_blocked(&s.blocked_shell_extensions),
            workspaces: crate::workspaces_json(),
            workspace_confirm: crate::workspaces_confirm(),
            file_basket: crate::basket::tidy_list(&s.file_basket),
            excel_tidy: ExcelTidyJson {
                selection: s.excel_tidy_a1,
                zoom: s.excel_tidy_zoom,
                first_sheet: s.excel_tidy_first,
                backup: s.excel_tidy_backup,
            },
            folder_watches: crate::watch::export_saved(),
            extra: Map::new(),
        }
    }

    /// Keep a newer file's version and any keys this build does not edit.
    /// `tabs` is this build's: the session just wrote it.
    fn merge_forward(&mut self, previous: &SettingsFile) {
        if previous.version >= 1 {
            self.version = previous.version;
        }
        // Command ids come from the live keymap. Anything else (a future build's keys) stays.
        let mut kb = self.keybindings.clone();
        for (k, v) in &previous.keybindings {
            if crate::keys::is_command(k) {
                continue;
            }
            kb.entry(k.clone()).or_insert_with(|| v.clone());
        }
        self.keybindings = kb;
        // An empty list means this save did not see live windows (a test, or a build that
        // does not write them). A real save always has the window that is still open.
        if self.windows.is_empty() {
            self.windows = previous.windows.clone();
        }
        self.extra = previous.extra.clone();
        self.extra.remove("panel");
        // Retired UI scale. An older maguro.json still loads; the key is not written back.
        self.extra.remove("font");
        self.extra.remove("one_window");
        self.extra.remove("drive_buttons");
        self.extra.remove("use_everything");
        self.extra.remove("dialog_jump");
        self.extra.remove("clip_hotkey");
        self.extra.remove("ev_asked");
        self.extra.remove("ev_sort");
        self.extra.remove("ev_cols");
        self.extra.remove("remote_asked");
        self.extra.remove("winmerge_path");
        self.extra.remove("space_preview");
        self.extra.remove("preview_pane");
        self.extra.remove("preview_width");
        self.extra.remove("peek_wheel_nav");
        self.extra.remove("remember_folder_view");
        self.extra.remove("show_folder_sizes");
        self.extra.remove("folder_sizes_network");
        self.extra.remove("fast_copy");
        self.extra.remove("layout_new");
        self.extra.remove("ctx_menu");
        self.extra.remove("ctx_labels");
        self.extra.remove("ctx_on");
        self.extra.remove("ctx_quiet");
        self.extra.remove("ctx_packaged_off");
        self.extra.remove("ctx_packaged_on");
        self.extra.remove("ctx_win11_pack_migrated");
        self.extra.remove("ctx_win11_view");
        self.extra.remove("blocked_shell_extensions");
        self.extra.remove("workspaces");
        self.extra.remove("workspace_confirm");
        self.extra.remove("file_basket");
        self.extra.remove("excel_tidy");
        self.extra.remove("folder_watches");
        // Older builds stored a pane-tab on/off switch. Tabs are always on; the key is dropped.
        self.extra.remove("pane_tabs");
    }

    fn to_session(&self) -> Session {
        crate::watch::import_saved(&self.folder_watches);
        let mut s = Session::default();
        if let Some(w) = &self.window {
            s.win = Some([w.left, w.top, w.right, w.bottom, w.maximized as i32]);
        }
        s.side = self.side;
        s.side_dpi = self.side_dpi;
        let heights: Vec<usize> = self.layout.columns.iter().take(4).copied().map(|n| n.clamp(1, 4)).collect();
        s.heights = if heights.is_empty() { left_right() } else { heights };
        s.colr = fix(self.layout.colr.clone(), s.heights.len());
        let rowr = &self.layout.rowr;
        let group = |c: usize| if rowr.len() == s.heights.len() { rowr.get(c) } else { rowr.first() };
        s.rowr = s.heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
        s.order = if self.order.len() == MAXP && self.order.iter().all(|&n| (n as usize) < MAXP) { self.order.clone() } else { Vec::new() };
        s.active = self.layout.active;
        if s.active >= s.panes() {
            s.active = 0;
        }
        s.paths = vec![String::new(); MAXP];
        s.views = vec![(0, 0); MAXP];
        s.cols = vec![String::new(); MAXP];
        for p in &self.panes {
            if p.index >= MAXP {
                continue;
            }
            s.paths[p.index] = p.path.clone();
            if p.view.len() >= 2 {
                s.views[p.index] = (p.view[0], p.view[1]);
            } else if let Some(m) = p.view.first() {
                s.views[p.index] = (*m, 0);
            }
            s.cols[p.index] = p.columns.clone();
            if p.folders.len() > 1 {
                s.pane_cur[p.index] = p.folder.min(p.folders.len() - 1);
                s.pane_folders[p.index] = p.folders.clone();
            }
        }
        let mut filters = vec![String::new(); MAXP];
        for p in &self.panes {
            if p.index < MAXP {
                filters[p.index] = p.filter.clone();
            }
        }
        s.accent = crate::brand_accent(parse_color(&self.accent).unwrap_or(s.accent));
        s.accent_dark = crate::brand_accent(parse_color(&self.accent_dark).unwrap_or(s.accent_dark));
        s.dark = self.dark;
        s.theme = match self.theme.as_str() {
            "light" => 1,
            "dark" => 2,
            _ => 0,
        };
        s.font_face = self.font_face.trim().to_string();
        s.bar = self.bar;
        s.drive_buttons = self.drive_buttons;
        s.stat = self.stat;
        let mode = crate::known_view_mode(self.view_new);
        s.view_icon = if mode == self.view_new { self.view_icon.max(0) } else { 0 };
        s.view_new = mode;
        s.start_new = self.start.min(2);
        s.start_path = self.start_path.clone();
        s.ext = self.ext;
        s.tree_on = self.tree;
        s.side_split = crate::persist_split(self.side_split);
        s.fav_top = self.side_order != "tree";
        s.pins_open = self.explorer_pins_open;
        s.files_open = self.explorer_files_open;
        s.edge = self.edge;
        s.confirm = self.confirm;
        s.del = if self.del == 1 { 1 } else { 0 };
        s.new_name = self.new_name.clone();
        s.dropm = self.drop.min(2);
        s.fast_copy = self.fast_copy;
        s.layout_new = self.layout_new.min(3);
        s.boot = self.boot.min(2);
        s.boot_file = self.boot_file.clone();
        s.autosave = self.autosave;
        s.dot = self.dot;
        s.update_check = self.update_check;
        s.dblup = self.dblup;
        s.hotkey = self.hotkey;
        s.dialog_jump = self.dialog_jump;
        s.clip_hotkey = self.clip_hotkey;
        s.lang = Lang::from_ini(&self.lang);
        s.single = self.single;
        s.one_window = self.one_window;
        s.use_everything = self.use_everything;
        s.space_preview = self.space_preview;
        s.preview_on = self.preview_pane;
        s.preview_w = if self.preview_width <= 0 { 0 } else { self.preview_width.clamp(160, 4000) };
        s.peek_wheel_nav = self.peek_wheel_nav;
        s.remember_views = self.remember_folder_view;
        s.folder_sizes = self.show_folder_sizes;
        s.folder_sizes_network = self.folder_sizes_network;
        s.peek_image = self.peek.as_ref().and_then(|p| peek_pair(p.image));
        s.peek_doc = self.peek.as_ref().and_then(|p| peek_pair(p.doc));
        s.ev_asked = self.ev_asked;
        s.ev_sort = crate::everything::ev_sort_ok(self.ev_sort);
        s.ev_cols = self.ev_cols.and_then(crate::everything::ev_cols_ok);
        s.remote_asked = self.remote_asked;
        s.winmerge_path = self.winmerge_path.trim().to_string();
        s.ctx_off = clean_ctx(&self.ctx_menu);
        s.ctx_labels = clean_labels(&self.ctx_labels);
        s.ctx_on = clean_ctx(&self.ctx_on);
        s.ctx_quiet = clean_quiet(&self.ctx_quiet);
        // Older files listed commands to hide, and every other packaged command was on.
        // Switch those on-commands off once. A later save keeps `ctx_packaged_on` and the flag,
        // so a command the user then enables is not cleared on the next launch.
        if self.ctx_win11_pack_migrated {
            s.ctx_packaged_on = clean_packaged(&self.ctx_packaged_on);
        } else {
            s.ctx_packaged_on.clear();
        }
        s.ctx_win11_pack_migrated = true;
        s.ctx_win11_view = self.ctx_win11_view;
        s.blocked_shell_extensions = clean_blocked(&self.blocked_shell_extensions);
        s.file_basket = crate::basket::tidy_list(&self.file_basket);
        s.excel_tidy_a1 = self.excel_tidy.selection;
        s.excel_tidy_zoom = self.excel_tidy.zoom;
        s.excel_tidy_first = self.excel_tidy.first_sheet;
        s.excel_tidy_backup = self.excel_tidy.backup;
        crate::shell::ctxmenu::reconcile_winmerge(&mut s.ctx_off, &mut s.ctx_on);
        s.file = self.session.clone();
        let parsed_tree = crate::tree_from_values(&self.favorites_tree);
        if !parsed_tree.is_empty() {
            s.tree = parsed_tree;
            s.favs = crate::flat_favs(&s.tree);
        } else {
            s.favs = self.favorites.iter().filter(|f| !f.path.trim().is_empty()).map(|f| {
                if f.name.trim().is_empty() { f.path.trim().to_string() } else { format!("{}\t{}", f.path.trim(), f.name.trim()) }
            }).collect();
            s.tree = crate::tree_from_flat(&s.favs);
        }
        s.recent = self.recent.iter().filter(|r| !r.trim().is_empty()).take(RECENT).cloned().collect();
        if self.tabs.is_empty() {
            s.tabs.clear();
            s.tab = 0;
            s.anchor();
            if let Some(t) = s.tabs.get_mut(0) {
                t.filters = filters;
            }
        } else {
            crate::adopt_tabs(&mut s, &self.tabs);
        }
        if self.windows.is_empty() {
            crate::stash_windows(Vec::new());
            s
        } else {
            let first = crate::session_from_window(self.windows.first().unwrap(), &s);
            crate::stash_windows(self.windows.iter().skip(1).cloned().collect());
            first
        }
    }
}

/// Drop `font_face` when that family is not installed. The pure decision is `resolve_face`.
fn forget_missing_face(s: &mut Session) {
    if s.font_face.is_empty() {
        return;
    }
    let ok = unsafe { crate::face_installed(&s.font_face) };
    let only = [s.font_face.as_str()];
    let resolved = crate::resolve_face(&s.font_face, s.lang.resolve(), if ok { &only[..] } else { &[] });
    if ok && resolved == s.font_face {
        return;
    }
    s.font_face.clear();
    SETTINGS_KEEP.with(|k| k.borrow_mut().font_face.clear());
}

fn clean_ctx(map: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    const NAMES: [&str; 4] = ["file", "folder", "background", "drive"];
    let mut out = BTreeMap::new();
    for (id, targets) in map {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        let mut names = Vec::new();
        for t in targets {
            let t = t.trim().to_ascii_lowercase();
            if NAMES.contains(&t.as_str()) && !names.iter().any(|n: &String| n == &t) {
                names.push(t);
            }
        }
        names.sort();
        if !names.is_empty() {
            out.insert(id.to_string(), names);
        }
    }
    out
}

fn clean_blocked(names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for raw in names.iter().take(64) {
        let t = raw.trim();
        let base = t.rsplit(['\\', '/']).next().unwrap_or(t).trim();
        if base.is_empty() || base == "." || base == ".." || base.len() > 128 || base.contains(['*', '?', '"', '<', '>', '|']) {
            continue;
        }
        if out.iter().any(|e: &String| e.eq_ignore_ascii_case(base)) {
            continue;
        }
        out.push(base.to_string());
    }
    out
}

fn clean_packaged(ids: &BTreeSet<String>) -> BTreeSet<String> {
    ids.iter().filter_map(|id| {
        let id = id.trim().to_ascii_lowercase();
        let Some(rest) = id.strip_prefix("packaged:") else { return None };
        if id.len() > 80 || rest.chars().any(|c| c.is_whitespace()) {
            None
        } else {
            Some(id)
        }
    }).collect()
}

fn clean_quiet(ids: &BTreeSet<String>) -> BTreeSet<String> {
    ids.iter().filter_map(|id| {
        let id = id.trim();
        if id.is_empty() || id.len() > 96 { None } else { Some(id.to_string()) }
    }).collect()
}

fn clean_labels(map: &BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for (id, labels) in map {
        let id = id.trim();
        if id.is_empty() || id.len() > 96 {
            continue;
        }
        let names = crate::shell::ctxmenu::cleaned_labels(labels);
        if !names.is_empty() {
            out.insert(id.to_string(), names);
        }
    }
    out
}

fn parse_color(v: &str) -> Option<u32> {
    let t = v.trim().trim_start_matches('#');
    if t.is_empty() {
        return None;
    }
    u32::from_str_radix(t, 16).ok().map(|n| n & 0xFFFFFF)
}

fn pretty(file: &SettingsFile) -> String {
    let mut s = serde_json::to_string_pretty(file).unwrap_or_else(|_| "{}".into());
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

thread_local! {
    static SETTINGS_KEEP: RefCell<SettingsFile> = RefCell::new(SettingsFile::default());
}

fn remember(file: SettingsFile) {
    SETTINGS_KEEP.with(|k| *k.borrow_mut() = file);
}

fn strip_bom(text: &str) -> &str {
    let text = text.trim_start();
    text.strip_prefix('\u{FEFF}').unwrap_or(text).trim_start()
}

fn adopt(text: &str) -> Option<Session> {
    let text = strip_bom(text);
    let file: SettingsFile = serde_json::from_str(text).ok()?;
    let s = file.to_session();
    crate::keys::load_bindings(&file.keybindings);
    crate::load_workspaces(&file.workspaces, file.workspace_confirm);
    remember(file);
    Some(s)
}

/// Serialize `s`, keeping version and unknown fields from the file that was loaded.
pub(crate) fn sync_text(s: &Session) -> String {
    let mut file = SettingsFile::from_session(s);
    let prev = SETTINGS_KEEP.with(|k| k.borrow().clone());
    file.merge_forward(&prev);
    let text = pretty(&file);
    remember(file);
    text
}

pub(crate) fn current_text() -> String {
    SETTINGS_KEEP.with(|k| pretty(&k.borrow()))
}

/// A leading `{` is JSON. Anything else is a legacy ini. Invalid JSON is refused.
pub(crate) fn import_text(text: &str) -> Result<Session, ()> {
    let text = strip_bom(text);
    if text.starts_with('{') {
        adopt(text).ok_or(())
    } else {
        let s = Session::parse(text);
        remember(SettingsFile::from_session(&s));
        Ok(s)
    }
}

pub(crate) fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(|| PathBuf::from("."))
}

fn portable_mode() -> bool {
    exe_dir().join("portable.txt").is_file()
}

fn roaming_appdata() -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, None).ok()?;
        let path = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as _));
        if path.is_empty() { None } else { Some(PathBuf::from(path)) }
    }
}

pub(crate) const SETTINGS_FILE: &str = "maguro.json";

fn appdata_dir(base: &Path) -> PathBuf {
    base.join("Maguro")
}

pub(crate) fn config_dir() -> PathBuf {
    if portable_mode() {
        return exe_dir();
    }
    match roaming_appdata() {
        Some(p) => appdata_dir(&p),
        None => {
            let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(exe_dir);
            appdata_dir(&base)
        }
    }
}

pub(crate) fn settings_path() -> PathBuf {
    config_dir().join(SETTINGS_FILE)
}

/// maguro.json bytes. Missing is `Ok(None)`. A locked or unreadable file is an error, never defaults.
pub(crate) fn read_settings_file(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Load maguro.json. An existing file that cannot be read or parsed is an error,
/// so a later save does not replace it with defaults. A missing file starts from the defaults.
pub(crate) fn try_load_at(cfg: &Path) -> Result<Session, String> {
    let _ = std::fs::create_dir_all(cfg);
    let json_path = cfg.join(SETTINGS_FILE);
    match read_settings_file(&json_path)? {
        Some(raw) => {
            let text = strip_bom(&raw);
            let Some(mut s) = adopt(text) else {
                return Err("maguro.json is not valid settings".into());
            };
            // A name that is not installed falls back to the language default. Empty skips the lookup.
            forget_missing_face(&mut s);
            // Pre-allowlist files showed every packaged command. Write the flag once so the
            // next launch does not clear commands the user has since turned on.
            let pack_migrate = serde_json::from_str::<Value>(text).ok().is_some_and(|v| v.get("ctx_win11_pack_migrated").and_then(|x| x.as_bool()) != Some(true));
            // The activity-bar choice is retired. Drop it once so the next launch does not see it.
            let had_panel = serde_json::from_str::<Value>(text).ok().is_some_and(|v| v.get("panel").is_some());
            // Retired accents (pre-#222 mint, old first sky swatches) rewrite as cerulean.
            // brand_accent already applied the same map to the in-memory session above.
            let had_retired_accent = serde_json::from_str::<Value>(text).ok().is_some_and(|v| {
                ["accent", "accent_dark"].iter().any(|k| {
                    v.get(*k).and_then(|x| x.as_str()).is_some_and(|c| parse_color(c).is_some_and(|n| crate::brand_accent(n) != n))
                })
            });
            if had_panel || had_retired_accent || pack_migrate {
                let migrated = sync_text(&s);
                if migrated != text {
                    let _ = write_atomic(&json_path, migrated.as_bytes());
                }
            }
            Ok(s)
        }
        None => {
            let s = Session::default();
            remember(SettingsFile::from_session(&s));
            Ok(s)
        }
    }
}

/// Same as `try_load_at` for tests that only see a readable file.
#[cfg(test)]
pub(crate) fn load_at(cfg: &Path) -> Session {
    try_load_at(cfg).unwrap_or_else(|_| {
        remember(SettingsFile::default());
        Session::default()
    })
}

pub(crate) fn load() -> Result<Session, String> {
    let cfg = config_dir();
    let _ = std::fs::create_dir_all(&cfg);
    try_load_at(&cfg)
}

pub(crate) fn write_settings(text: &str) -> std::io::Result<()> {
    write_atomic(&settings_path(), text.as_bytes())
}

/// Write `bytes` next to `path` and swap it in. A failed swap leaves the previous file in place.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if path.is_dir() {
        return Err(std::io::Error::other("settings path is a directory"));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut tmp = path.to_path_buf();
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| SETTINGS_FILE.into());
    // One tmp (and .bak) per process: two windows with single=0 must not write the same file.
    tmp.set_file_name(format!("{name}.{}.tmp", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    let src = HSTRING::from(tmp.to_string_lossy().as_ref());
    let dst = HSTRING::from(path.to_string_lossy().as_ref());
    let moved = unsafe { MoveFileExW(&src, &dst, MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) };
    if moved.is_ok() {
        return Ok(());
    }
    // Replace by renaming aside, then back if the new file does not land.
    let bak = tmp.with_extension("bak");
    if path.exists() && std::fs::rename(path, &bak).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(std::io::Error::other("could not replace settings"));
    }
    if std::fs::rename(&tmp, path).is_ok() {
        let _ = std::fs::remove_file(&bak);
        return Ok(());
    }
    if bak.exists() {
        let _ = std::fs::rename(&bak, path);
    }
    let _ = std::fs::remove_file(&tmp);
    Err(std::io::Error::other("could not replace settings"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("maguro-json-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn sample() -> Session {
        let mut s = Session::default();
        s.win = Some([10, 20, 900, 700, 1]);
        s.side = 260;
        s.set_heights(vec![1, 3]);
        s.colr = vec![0.25, 0.75];
        s.rowr = vec![vec![1.0], vec![0.5, 0.25, 0.25]];
        s.active = 2;
        s.paths[1] = "C:\\a=b\\폴더".into();
        s.views[1] = (4, 16);
        s.cols[1] = "{guid}/10=220;{guid}/12=-1".into();
        s.accent = 0xA8624A;
        s.accent_dark = 0x4CC2FF;
        s.dark = true;
        s.dblup = false;
        s.file = "C:\\s\\work.maguro".into();
        s.favs = vec!["C:\\WORK\tWork".into(), "D:\\Photos".into(), "E:\\\tLater".into()];
        s.recent = vec!["D:\\Photos".into(), "C:\\WORK".into()];
        s.lang = Lang::Ko;
        s.single = false;
        s.theme = 0;
        s.bar = false;
        s.drive_buttons = false;
        s.stat = false;
        s.view_new = 1;
        s.start_new = 2;
        s.start_path = "C:\\Work".into();
        s.ext = false;
        s.tree_on = false;
        s.edge = false;
        s.confirm = false;
        s.del = 1;
        s.new_name = "note.txt".into();
        s.dropm = 2;
        s.layout_new = 2;
        s.boot = 1;
        s.boot_file = "C:\\s.maguro".into();
        s.autosave = true;
        s.dot = false;
        s
    }

    #[test]
    fn json_round_trip_keeps_settings_and_forward_fields() {
        let parsed = Session::parse(&sample().to_ini());
        let text = pretty(&SettingsFile::from_session(&parsed));
        let back = adopt(&text).unwrap();
        assert_eq!(back, parsed);
        // Empty JSON is an older file: two panes side by side. A fresh Session is one pane.
        // Every other field still matches once that layout difference is set aside.
        let mut empty = adopt("{}").unwrap();
        assert_eq!((empty.layout_new, empty.heights.clone()), (0, vec![1, 1]));
        empty.layout_new = Session::default().layout_new;
        empty.set_heights(Session::default().heights.clone());
        empty.anchor();
        assert_eq!(empty, Session::default());

        let mut file = SettingsFile::from_session(&parsed);
        file.version = 2;
        file.tabs = vec![serde_json::json!({"id": "later"})];
        file.keybindings.insert("copy".into(), serde_json::json!("Ctrl+C"));
        file.favorites_tree = vec![serde_json::json!("group")];
        file.extra.insert("future".into(), serde_json::json!({"keep": true}));
        let loaded = adopt(&pretty(&file)).unwrap();
        assert_eq!(loaded, parsed);
        let saved: Value = serde_json::from_str(&sync_text(&loaded)).unwrap();
        assert_eq!(saved["version"], 2);
        assert_eq!(saved["tabs"][0]["name"], "1");
        assert_eq!(saved["tabs"][0]["locked"], false);
        assert_eq!(saved["tabs"][0]["layout"]["columns"][0], 1);
        assert_eq!(saved["tabs"][0]["layout"]["columns"][1], 3);
        assert!(saved["tabs"][0]["panes"].as_array().unwrap().iter().any(|p| p["path"] == "C:\\a=b\\폴더"));
        assert_eq!(saved["keybindings"]["copy"], "Ctrl+C");
        assert_eq!(saved["favorites_tree"][0]["path"], "C:\\WORK");
        assert_eq!(saved["favorites_tree"][0]["name"], "Work");
        assert_eq!(saved["future"]["keep"], true);
        assert_eq!(saved["lang"], "ko");
        assert_eq!(saved["layout_new"], 2);
        assert_eq!(saved["favorites"][0]["name"], "Work");
        assert_eq!(saved["favorites"][1]["path"], "D:\\Photos");
        assert!(saved["panes"].as_array().unwrap().iter().any(|p| p["path"] == "C:\\a=b\\폴더" && p["columns"] == "{guid}/10=220;{guid}/12=-1"));

        let ini = import_text(&parsed.to_ini()).unwrap();
        assert_eq!(ini, parsed);
        assert!(import_text("{").is_err());
    }

    #[test]
    fn workspaces_live_in_maguro_json_and_are_omitted_when_empty() {
        crate::reset_workspaces();
        let s = Session::default();
        let text = sync_text(&s);
        assert!(!text.contains("\"workspaces\""));
        assert!(!text.contains("workspace_confirm"));
        let session = "cols=1\np0=C:\\work\n";
        assert!(crate::store_workspace(0, session));
        crate::rename_workspace_name(0, "Desk");
        crate::set_workspaces_confirm(true);
        let text = sync_text(&s);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["workspaces"][0]["slot"], 1);
        assert_eq!(v["workspaces"][0]["name"], "Desk");
        assert_eq!(v["workspaces"][0]["session"], session);
        assert_eq!(v["workspace_confirm"], true);
        crate::reset_workspaces();
        assert!(crate::workspace_session(0).is_none());
        let back = adopt(&text).unwrap();
        assert_eq!(back.theme, s.theme);
        assert!(crate::workspaces_confirm());
        assert_eq!(crate::workspace_session(0).as_deref(), Some(session));
        assert_eq!(crate::workspace_name(0), "Desk");
        crate::reset_workspaces();
    }

    #[test]
    fn ctx_menu_lives_in_json_and_not_in_a_session_file() {
        let mut s = Session::default();
        s.ctx_off.insert("clsid:abc".into(), vec!["nope".into(), "File".into(), "drive".into(), "file".into()]);
        s.ctx_labels.insert("clsid:abc".into(), vec!["  ".into(), "반디집으로 압축하기".into(), "반디집으로 압축하기".into(), "SVN 업데이트".into()]);
        let text = sync_text(&s);
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved["ctx_menu"]["clsid:abc"], serde_json::json!(["drive", "file"]));
        assert_eq!(saved["ctx_labels"]["clsid:abc"], serde_json::json!(["반디집으로 압축하기", "SVN 업데이트"]));
        assert!(!s.to_ini().contains("ctx_menu"));
        assert!(!s.to_ini().contains("ctx_labels"));
        assert!(!s.to_ini().contains("clsid:abc"));
        let back = adopt(&text).unwrap();
        assert_eq!(back.ctx_off.get("clsid:abc").map(Vec::as_slice), Some(["drive".to_string(), "file".to_string()].as_slice()));
        assert_eq!(back.ctx_labels.get("clsid:abc").map(Vec::as_slice), Some(["반디집으로 압축하기".to_string(), "SVN 업데이트".to_string()].as_slice()));
        assert_eq!(Session::parse(&s.to_ini()).ctx_off, Session::default().ctx_off);

        s.ctx_off.clear();
        let cleared = sync_text(&s);
        let saved: Value = serde_json::from_str(&cleared).unwrap();
        assert!(saved.get("ctx_menu").is_none(), "{cleared}");
        assert_eq!(saved["ctx_labels"]["clsid:abc"], serde_json::json!(["반디집으로 압축하기", "SVN 업데이트"]));
        assert_eq!(adopt("{}").unwrap().ctx_off, Session::default().ctx_off);
        assert!(adopt("{}").unwrap().ctx_labels.is_empty());

        let mut dirty = Session::default();
        dirty.ctx_labels.insert(
            "clsid:vs".into(),
            vec!["Visual Studio で開く(V)".into(), "Visual Studio で開く".into(), "\"a.zip\" に圧縮".into(), "\"b.zip\" に圧縮".into()],
        );
        let cleaned = adopt(&sync_text(&dirty)).unwrap();
        assert_eq!(
            cleaned.ctx_labels.get("clsid:vs").map(Vec::as_slice),
            Some(["Visual Studio で開く".to_string(), "\"…\" に圧縮".to_string()].as_slice())
        );

        let mut marked = Session::default();
        marked.ctx_on.insert("app:winmerge".into(), vec!["folder".into(), "file".into()]);
        marked.ctx_quiet.insert("clsid:os".into());
        marked.ctx_quiet.insert("  ".into());
        let text = sync_text(&marked);
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved["ctx_on"]["app:winmerge"], serde_json::json!(["file", "folder"]));
        assert_eq!(saved["ctx_quiet"], serde_json::json!(["clsid:os"]));
        assert!(!marked.to_ini().contains("ctx_on"));
        assert!(!marked.to_ini().contains("ctx_quiet"));
        let back = adopt(&text).unwrap();
        assert_eq!(back.ctx_on.get("app:winmerge").map(Vec::as_slice), Some(["file".to_string(), "folder".to_string()].as_slice()));
        assert!(back.ctx_quiet.contains("clsid:os"));
        let fresh = serde_json::from_str::<Value>(&sync_text(&Session::default())).unwrap();
        assert!(fresh.get("ctx_on").is_none());
        assert!(fresh.get("ctx_quiet").is_none());

        let partial = adopt(r#"{"ctx_menu":{"clsid:4e716236-aa30-4c65-b225-d68bba81e9c2":["file"]}}"#).unwrap();
        assert!(partial.ctx_off.is_empty());
        assert_eq!(
            partial.ctx_on.get("app:winmerge").map(Vec::as_slice),
            Some(["folder".to_string(), "background".to_string(), "drive".to_string()].as_slice())
        );
        let hidden = adopt(r#"{"ctx_menu":{"clsid:4e716236-aa30-4c65-b225-d68bba81e9c2":["file","folder","background","drive"]}}"#).unwrap();
        assert!(hidden.ctx_off.is_empty());
        assert!(hidden.ctx_on.is_empty());
        let kept = adopt(r#"{"ctx_on":{"app:winmerge":["file"]},"ctx_menu":{"clsid:4e716236-aa30-4c65-b225-d68bba81e9c2":["drive"]}}"#).unwrap();
        assert!(kept.ctx_off.is_empty());
        assert_eq!(kept.ctx_on.get("app:winmerge").map(Vec::as_slice), Some(["file".to_string()].as_slice()));
    }

    #[test]
    fn win11_packaged_commands_start_off_and_migrate_once() {
        let id = "packaged:{9f156763-7844-4dc4-b2b1-901f640f5155}";
        let old = r#"{"ctx_packaged_off":["packaged:{9f156763-7844-4dc4-b2b1-901f640f5155}"],"ctx_win11_view":true}"#;
        let migrated = adopt(old).unwrap();
        assert!(migrated.ctx_win11_pack_migrated);
        assert!(migrated.ctx_packaged_on.is_empty(), "commands that used to be on are switched off once");
        assert!(migrated.ctx_win11_view);
        let saved = sync_text(&migrated);
        let saved_v: Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved_v["ctx_win11_pack_migrated"], true);
        assert!(saved_v.get("ctx_packaged_off").is_none(), "{saved}");
        assert!(saved_v.get("ctx_packaged_on").is_none(), "{saved}");
        assert!(!migrated.to_ini().contains("ctx_packaged"));
        assert!(!migrated.to_ini().contains("ctx_win11_pack"));
        assert!(!migrated.to_ini().contains("packaged:"));

        let mut chosen = adopt(&saved).unwrap();
        assert!(chosen.ctx_packaged_on.is_empty());
        chosen.ctx_packaged_on.insert(id.into());
        chosen.ctx_packaged_on.insert("  ".into());
        chosen.ctx_packaged_on.insert("clsid:nope".into());
        let again = sync_text(&chosen);
        let again_v: Value = serde_json::from_str(&again).unwrap();
        assert_eq!(again_v["ctx_packaged_on"], serde_json::json!([id]));
        assert_eq!(again_v["ctx_win11_pack_migrated"], true);
        assert!(again_v.get("ctx_packaged_off").is_none());
        let back = adopt(&again).unwrap();
        assert!(back.ctx_win11_pack_migrated);
        assert!(back.ctx_packaged_on.contains(id));
        assert!(!back.ctx_packaged_on.contains("clsid:nope"));
        assert_eq!(back.ctx_packaged_on.len(), 1);

        // Flag already set: an explicit on survives, and the old denylist is not applied again.
        let kept = adopt(
            r#"{"ctx_win11_pack_migrated":true,"ctx_packaged_on":["packaged:{9f156763-7844-4dc4-b2b1-901f640f5155}"],"ctx_packaged_off":["packaged:{aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}"]}"#,
        )
        .unwrap();
        assert!(kept.ctx_win11_pack_migrated);
        assert!(kept.ctx_packaged_on.contains(id));
        assert_eq!(kept.ctx_packaged_on.len(), 1);

        // A false flag runs the switch again, so a stored on-list from a half-written file is cleared once.
        let flipped = adopt(r#"{"ctx_win11_pack_migrated":false,"ctx_packaged_on":["packaged:{9f156763-7844-4dc4-b2b1-901f640f5155}"]}"#).unwrap();
        assert!(flipped.ctx_win11_pack_migrated);
        assert!(flipped.ctx_packaged_on.is_empty());

        let fresh = serde_json::from_str::<Value>(&sync_text(&Session::default())).unwrap();
        assert_eq!(fresh["ctx_win11_pack_migrated"], true);
        assert!(fresh.get("ctx_packaged_on").is_none());
        assert!(fresh.get("ctx_packaged_off").is_none());
        assert!(fresh.get("ctx_win11_view").is_none());
        let empty = adopt("{}").unwrap();
        assert!(empty.ctx_win11_pack_migrated);
        assert!(empty.ctx_packaged_on.is_empty());
        assert!(!empty.ctx_win11_view);
    }

    #[test]
    fn blocked_shell_extensions_stay_in_json_only() {
        let mut s = Session::default();
        s.blocked_shell_extensions = vec![r"C:\Vendor\Codec.dll".into(), "codec.dll".into(), "  ".into(), "jpegxr.dll".into()];
        let text = sync_text(&s);
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved["blocked_shell_extensions"], serde_json::json!(["Codec.dll", "jpegxr.dll"]));
        assert!(!s.to_ini().contains("blocked_shell"));
        assert!(!s.to_ini().contains("Codec.dll"));
        let back = adopt(&text).unwrap();
        assert_eq!(back.blocked_shell_extensions, vec!["Codec.dll".to_string(), "jpegxr.dll".to_string()]);
        let fresh = serde_json::from_str::<Value>(&sync_text(&Session::default())).unwrap();
        assert!(fresh.get("blocked_shell_extensions").is_none());
    }

    #[test]
    fn file_basket_stays_in_json_only_and_drops_duplicates() {
        let mut s = Session::default();
        s.file_basket = vec![r"C:\Work\A.txt".into(), r"c:\work\a.txt\".into(), r"D:\Photos".into(), "  ".into()];
        let text = sync_text(&s);
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(saved["file_basket"], serde_json::json!([r"C:\Work\A.txt", r"D:\Photos"]));
        assert!(!s.to_ini().contains("file_basket"));
        assert!(!s.to_ini().contains(r"C:\Work\A.txt"));
        let back = adopt(&text).unwrap();
        assert_eq!(back.file_basket, vec![r"C:\Work\A.txt".to_string(), r"D:\Photos".to_string()]);
        let fresh = serde_json::from_str::<Value>(&sync_text(&Session::default())).unwrap();
        assert!(fresh.get("file_basket").is_none());
        assert!(adopt("{}").unwrap().file_basket.is_empty());
    }

    #[test]
    fn layout_new_defaults_to_zero_and_round_trips() {
        // Empty JSON (older file without the key) still means two panes.
        let fresh = adopt("{}").unwrap();
        assert_eq!(fresh.layout_new, 0);
        assert_eq!(fresh.heights, vec![1, 1]);
        // Brand-new in-memory default is one pane, and the key is written.
        assert_eq!(Session::default().layout_new, 3);
        assert_eq!(Session::default().heights, vec![1]);
        let omitted = sync_text(&Session::default());
        assert_eq!(serde_json::from_str::<Value>(&omitted).unwrap().get("layout_new"), Some(&serde_json::json!(3)), "{omitted}");

        let zero = adopt(r#"{"version":1,"layout_new":0}"#).unwrap();
        assert_eq!(zero.layout_new, 0);
        assert!(serde_json::from_str::<Value>(&sync_text(&zero)).unwrap().get("layout_new").is_none());

        let mut s = Session::default();
        s.layout_new = 1;
        s.active = 9;
        s.set_heights(s.layout_heights());
        assert_eq!(s.heights, vec![2]);
        assert!(s.active < s.panes());
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"layout_new\": 1"), "{text}");
        let back = adopt(&text).unwrap();
        assert_eq!((back.layout_new, back.layout_heights()), (1, vec![2]));
        assert_eq!(Session::parse(&s.to_ini()).layout_new, 1);

        let high = adopt(r#"{"layout_new":9}"#).unwrap();
        assert_eq!((high.layout_new, high.layout_heights()), (3, vec![1]));
        let saved_high: Value = serde_json::from_str(&sync_text(&high)).unwrap();
        assert_eq!(saved_high["layout_new"], 3);

        let _ = adopt(r#"{"version":1,"future":{"keep":true}}"#);
        SETTINGS_KEEP.with(|k| {
            k.borrow_mut().extra.insert("layout_new".into(), serde_json::json!(2));
        });
        let kept: Value = serde_json::from_str(&sync_text(&Session::default())).unwrap();
        assert_eq!(kept.get("layout_new"), Some(&serde_json::json!(3)), "{kept}");
        assert_eq!(kept["future"]["keep"], true);
    }

    #[test]
    fn fresh_config_is_one_pane_and_a_saved_layout_stays() {
        let root = scratch("fresh-panes");
        let cfg = root.join("cfg");
        let fresh = load_at(&cfg);
        assert_eq!((fresh.layout_new, fresh.heights.clone(), fresh.panes()), (3, vec![1], 1));
        assert!(!cfg.join(SETTINGS_FILE).exists());

        let mut saved = Session::default();
        saved.set_heights(vec![2, 2]);
        saved.paths[0] = r"C:\keep".into();
        saved.layout_new = 3;
        saved.anchor();
        std::fs::write(cfg.join(SETTINGS_FILE), sync_text(&saved)).unwrap();
        let back = load_at(&cfg);
        assert_eq!(back.heights, vec![2, 2]);
        assert_eq!(back.paths[0], r"C:\keep");
        assert_eq!(back.layout_new, 3);
        assert_eq!(back.panes(), 4);
    }

    #[test]
    fn favorites_tree_nests_files_and_migrates_a_flat_list() {
        let old = r#"{"version":1,"favorites":[{"path":"C:\\Work","name":"Work"},{"path":"D:\\Photos"}]}"#;
        let migrated = adopt(old).unwrap();
        assert_eq!(migrated.favs.len(), 2);
        assert_eq!(migrated.tree.len(), 2);
        assert!(migrated.is_fav(r"c:\work"));
        let again: Value = serde_json::from_str(&sync_text(&migrated)).unwrap();
        assert_eq!(again["favorites_tree"][0]["name"], "Work");
        assert!(again["favorites_tree"][1].get("name").is_none());

        let mut s = Session::default();
        s.tree.push(crate::FavNode::group("Jobs"));
        s.tree[0].children.push(crate::FavNode::leaf(r"C:\a\note.txt", "note", true));
        s.tree.push(crate::FavNode::leaf(r"D:\Photos", "", false));
        s.sync_favs();
        let back = adopt(&sync_text(&s)).unwrap();
        assert_eq!(back.tree, s.tree);
        assert_eq!(back.favs, vec![r"C:\a\note.txt".to_string() + "\tnote", r"D:\Photos".into()]);
        assert!(back.is_fav(r"C:\a\note.txt"));
        assert!(back.tree[0].children[0].file);
    }

    #[test]
    fn explorer_pins_open_round_trips_and_stays_out_of_the_tree() {
        let fresh = adopt("{}").unwrap();
        assert!(fresh.pins_open);
        let text = sync_text(&fresh);
        assert!(serde_json::from_str::<Value>(&text).unwrap().get("explorer_pins_open").is_none());

        let mut closed = fresh.clone();
        closed.pins_open = false;
        closed.tree.push(crate::FavNode::leaf(r"D:\Photos", "", false));
        closed.sync_favs();
        let saved: Value = serde_json::from_str(&sync_text(&closed)).unwrap();
        assert_eq!(saved["explorer_pins_open"], false);
        let tree = saved["favorites_tree"].as_array().unwrap();
        assert!(tree.iter().all(|n| n.get("name").and_then(|v| v.as_str()) != Some("Explorer pins")));
        let back = adopt(&saved.to_string()).unwrap();
        assert!(!back.pins_open);
        assert!(back.files_open);
        assert_eq!(back.tree.len(), 1);
        assert_eq!(back.tree[0].path, r"D:\Photos");
    }

    #[test]
    fn explorer_files_open_round_trips_and_stays_out_of_the_tree() {
        let fresh = adopt("{}").unwrap();
        assert!(fresh.files_open);
        let text = sync_text(&fresh);
        let saved_fresh: Value = serde_json::from_str(&text).unwrap();
        assert!(saved_fresh.get("explorer_files_open").is_none());

        let mut closed = fresh.clone();
        closed.files_open = false;
        closed.tree.push(crate::FavNode::leaf(r"C:\a\note.txt", "note", true));
        closed.sync_favs();
        let saved: Value = serde_json::from_str(&sync_text(&closed)).unwrap();
        assert_eq!(saved["explorer_files_open"], false);
        let tree = saved["favorites_tree"].as_array().unwrap();
        assert!(tree.iter().all(|n| n.get("name").and_then(|v| v.as_str()) != Some("Explorer favourite files")));
        let back = adopt(&saved.to_string()).unwrap();
        assert!(!back.files_open);
        assert!(back.pins_open);
        assert!(back.tree[0].file);
        assert_eq!(back.tree[0].path, r"C:\a\note.txt");
    }

    #[test]
    fn tabs_round_trip_in_json() {
        let mut s = Session::default();
        s.set_heights(vec![1]);
        s.paths[0] = r"C:\one".into();
        s.anchor();
        let mut other = Session::default();
        other.set_heights(vec![2]);
        other.paths[0] = r"D:\two".into();
        s.tabs.push(crate::Tab::from_live(&other, "Notes".into(), true, vec!["*.txt".into()]));
        let back = adopt(&sync_text(&s)).unwrap();
        assert_eq!(back, s);
        assert!(back.tabs[1].locked);
        assert_eq!(back.tabs[1].filters[0], "*.txt");
        assert_eq!(back.tabs[1].paths[0], r"D:\two");
        assert_eq!(back.paths[0], r"C:\one");

        let old = r#"{"version":1,"layout":{"columns":[1,3],"active":2},"panes":[{"index":1,"path":"C:\\z","filter":"*.pdf"}]}"#;
        let loaded = adopt(old).unwrap();
        assert_eq!(loaded.tabs.len(), 1);
        assert_eq!(loaded.tabs[0].name, "1");
        assert!(!loaded.tabs[0].locked);
        assert_eq!(loaded.paths[1], r"C:\z");
        assert_eq!(loaded.tabs[0].filters[1], "*.pdf");
        assert_eq!((loaded.heights.clone(), loaded.active), (vec![1, 3], 2));
        let again: Value = serde_json::from_str(&sync_text(&loaded)).unwrap();
        assert_eq!(again["tabs"][0]["name"], "1");
        assert_eq!(again["tabs"][0]["panes"][0]["filter"], "*.pdf");
    }


    #[test]
    fn missing_lang_is_auto_and_a_saved_lang_stays() {
        assert_eq!(Session::default().lang, Lang::Auto);
        assert_eq!(adopt("{}").unwrap().lang, Lang::Auto);
        assert_eq!(adopt(r#"{"version":1,"bar":true}"#).unwrap().lang, Lang::Auto);
        assert_eq!(adopt(r#"{"lang":"auto"}"#).unwrap().lang, Lang::Auto);
        assert_eq!(adopt(r#"{"lang":"ko"}"#).unwrap().lang, Lang::Ko);
        assert_eq!(adopt(r#"{"lang":"en"}"#).unwrap().lang, Lang::En);
        assert_eq!(adopt(r#"{"lang":"ja"}"#).unwrap().lang, Lang::Ja);
        assert!(pretty(&SettingsFile::from_session(&Session::default())).contains("\"lang\": \"auto\""));
        let mut auto = Session::default();
        auto.lang = Lang::Auto;
        assert_eq!(adopt(&pretty(&SettingsFile::from_session(&auto))).unwrap().lang, Lang::Auto);
        let mut en = Session::default();
        en.lang = Lang::En;
        assert_eq!(adopt(&pretty(&SettingsFile::from_session(&en))).unwrap().lang, Lang::En);

        let lang_json = |name: &str, body: &str| {
            let root = scratch(name);
            let cfg = root.join("cfg");
            std::fs::create_dir_all(&cfg).unwrap();
            std::fs::write(cfg.join(SETTINGS_FILE), body).unwrap();
            load_at(&cfg).lang
        };
        assert_eq!(lang_json("lang-json-missing", "{\"version\":1,\"bar\":true}\n"), Lang::Auto);
        assert_eq!(lang_json("lang-json-auto", "{\"lang\":\"auto\"}\n"), Lang::Auto);
        assert_eq!(lang_json("lang-json-en", "{\"lang\":\"en\"}\n"), Lang::En);
        assert_eq!(lang_json("lang-json-ko", "{\"lang\":\"ko\"}\n"), Lang::Ko);

        let root = scratch("lang-fresh");
        let cfg = root.join("cfg");
        assert_eq!(load_at(&cfg).lang, Lang::Auto);
        assert!(!cfg.join(SETTINGS_FILE).exists());
    }

    #[test]
    fn missing_settings_use_defaults_and_write_nothing() {
        let dir = scratch("fresh-missing");
        let loaded = load_at(&dir);
        assert_eq!(loaded.lang, Lang::Auto);
        assert_eq!(loaded.layout_new, Session::default().layout_new);
        assert!(!dir.join(SETTINGS_FILE).exists());
    }

    #[test]
    fn keybinding_round_trip_keeps_unknown_keys_and_drops_defaults() {
        crate::keys::load_bindings(&Map::new());
        let _ = adopt("{}");
        let i = crate::keys::index("tab.new").unwrap();
        crate::keys::rebind(i, crate::keys::parse_chord("Ctrl+K").unwrap());
        let saved: Value = serde_json::from_str(&sync_text(&Session::default())).unwrap();
        assert_eq!(saved["keybindings"]["tab.new"], "Ctrl+K");

        let mut file = SettingsFile::from_session(&Session::default());
        file.keybindings.insert("copy".into(), serde_json::json!("Ctrl+C"));
        let _ = adopt(&pretty(&file));
        assert_eq!(crate::keys::lookup(b'K' as u16, true, false, false, false), Some(i));
        let again: Value = serde_json::from_str(&sync_text(&Session::default())).unwrap();
        assert_eq!(again["keybindings"]["tab.new"], "Ctrl+K");
        assert_eq!(again["keybindings"]["copy"], "Ctrl+C");

        crate::keys::reset_all();
        let cleared: Value = serde_json::from_str(&sync_text(&Session::default())).unwrap();
        assert!(cleared["keybindings"].get("tab.new").is_none());
        assert_eq!(cleared["keybindings"]["copy"], "Ctrl+C");
        crate::keys::load_bindings(&Map::new());
        let _ = adopt("{}");
    }

    #[test]
    fn atomic_write_replaces_the_file() {
        let dir = scratch("atomic");
        let path = dir.join(SETTINGS_FILE);
        write_atomic(&path, b"{\"version\":1}\n").unwrap();
        write_atomic(&path, b"{\"version\":1,\"lang\":\"ja\"}\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"version\":1,\"lang\":\"ja\"}\n");
        assert!(!dir.join("maguro.json.tmp").exists());
        let blocked = dir.join("blocked.json");
        std::fs::create_dir(&blocked).unwrap();
        assert!(write_atomic(&blocked, b"{\"version\":1}\n").is_err());
        assert!(blocked.is_dir());
        assert!(!dir.join("blocked.json.tmp").exists());
    }

    #[test]
    fn unreadable_settings_are_not_defaults() {
        let dir = scratch("unread");
        assert!(read_settings_file(&dir.join("missing.json")).unwrap().is_none());
        std::fs::write(dir.join(SETTINGS_FILE), "{\"lang\":\"ko\"}").unwrap();
        assert!(try_load_at(&dir).is_ok());
        let locked = scratch("locked");
        std::fs::create_dir(locked.join(SETTINGS_FILE)).unwrap();
        assert!(read_settings_file(&locked.join(SETTINGS_FILE)).is_err());
        assert!(try_load_at(&locked).is_err());
        assert!(locked.join(SETTINGS_FILE).is_dir());
        let bad = scratch("badjson");
        std::fs::write(bad.join(SETTINGS_FILE), "{").unwrap();
        assert!(try_load_at(&bad).is_err());
        assert_eq!(std::fs::read_to_string(bad.join(SETTINGS_FILE)).unwrap(), "{");
    }

    #[test]
    fn windows_round_trip_keeps_each_path_and_an_empty_save_does_not_wipe_them() {
        let mut a = sample();
        a.paths[0] = "C:\\win-a".into();
        a.anchor();
        let mut b = sample();
        b.paths[0] = "D:\\win-b".into();
        b.win = Some([40, 50, 800, 600, 0]);
        b.file.clear();
        b.anchor();
        crate::set_snaps(vec![crate::window_value(&a), crate::window_value(&b)]);
        let text = sync_text(&a);
        let saved: Value = serde_json::from_str(&text).unwrap();
        assert!(saved["windows"][0]["tabs"][0]["panes"].as_array().unwrap().iter().any(|p| p["path"] == "C:\\win-a"));
        assert!(saved["windows"][1]["tabs"][0]["panes"].as_array().unwrap().iter().any(|p| p["path"] == "D:\\win-b"));
        assert!(saved["panes"].as_array().unwrap().iter().any(|p| p["path"] == "C:\\win-a"));
        let loaded = adopt(&text).unwrap();
        assert_eq!(loaded.paths[0], "C:\\win-a");
        let rest = crate::take_stashed();
        assert_eq!(rest.len(), 1);
        let back = crate::session_from_window(&rest[0], &loaded);
        assert_eq!(back.paths[0], "D:\\win-b");
        assert_eq!(back.win, Some([40, 50, 800, 600, 0]));
        assert_eq!(back.accent, loaded.accent);
        // A save that did not snapshot windows must leave the stored list in place.
        let kept: Value = serde_json::from_str(&sync_text(&loaded)).unwrap();
        assert_eq!(kept["windows"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn old_panel_key_is_dropped_and_side_split_round_trips() {
        let mut s = Session::default();
        s.side_split = 0.25;
        s.anchor();
        let saved: Value = serde_json::from_str(&sync_text(&s)).unwrap();
        assert!(saved.get("panel").is_none());
        assert!((saved["side_split"].as_f64().unwrap() - 0.25).abs() < 1e-9);
        let back = adopt(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert!((back.side_split - 0.25).abs() < 1e-9);
        let migrated = adopt(r#"{"version":1,"panel":"fav","side_split":0.25}"#).unwrap();
        assert!((migrated.side_split - 0.25).abs() < 1e-9);
        let again: Value = serde_json::from_str(&sync_text(&migrated)).unwrap();
        assert!(again.get("panel").is_none());
        assert!((again["side_split"].as_f64().unwrap() - 0.25).abs() < 1e-9);
        let even: Value = serde_json::from_str(&sync_text(&Session::default())).unwrap();
        assert!(even.get("panel").is_none());
        assert!(even.get("side_split").is_none());
        let only = adopt(r#"{"panel":"fav"}"#).unwrap();
        assert!((only.side_split - 0.5).abs() < 1e-9);
        let dropped: Value = serde_json::from_str(&sync_text(&only)).unwrap();
        assert!(dropped.get("panel").is_none());
        let wide = adopt(r#"{"side_split":9}"#).unwrap();
        assert!((wide.side_split - 0.5).abs() < 1e-9);
        let zero = adopt(r#"{"side_split":0}"#).unwrap();
        assert!((zero.side_split - 0.5).abs() < 1e-9);
        let one = adopt(r#"{"side_split":1}"#).unwrap();
        assert!((one.side_split - 0.5).abs() < 1e-9);
        assert!(even.get("side_order").is_none());
        assert!(Session::default().fav_top);
    }

    #[test]
    fn side_order_defaults_to_favourites_and_round_trips_the_tree() {
        let fresh = adopt("{}").unwrap();
        assert!(fresh.fav_top);
        let saved: Value = serde_json::from_str(&sync_text(&fresh)).unwrap();
        assert!(saved.get("side_order").is_none());
        assert!(!fresh.to_ini().contains("sideorder"));

        let mut s = Session::default();
        s.fav_top = false;
        s.side_split = 0.25;
        let text = sync_text(&s);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["side_order"], "tree");
        assert!((v["side_split"].as_f64().unwrap() - 0.25).abs() < 1e-9);
        let back = adopt(&text).unwrap();
        assert!(!back.fav_top);
        assert!((back.side_split - 0.25).abs() < 1e-9);
        assert!(back.to_ini().contains("sideorder=tree\n"));
        assert_eq!(Session::parse(&back.to_ini()).fav_top, false);

        assert!(adopt(r#"{"side_order":"fav"}"#).unwrap().fav_top);
        assert!(adopt(r#"{"side_order":"nope"}"#).unwrap().fav_top);
        let tree_again: Value = serde_json::from_str(&sync_text(&adopt(r#"{"side_order":"tree"}"#).unwrap())).unwrap();
        assert_eq!(tree_again["side_order"], "tree");
        let cleared = adopt(r#"{"side_order":"fav","side_split":0.25}"#).unwrap();
        assert!(cleared.fav_top);
        assert!(serde_json::from_str::<Value>(&sync_text(&cleared)).unwrap().get("side_order").is_none());
    }


    #[test]
    fn load_drops_a_retired_panel_key_once() {
        let root = scratch("drop-panel");
        let exe = root.join("exe");
        let cfg = root.join("cfg");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join(SETTINGS_FILE), "{\"version\":1,\"panel\":\"fav\",\"side_split\":0.25,\"lang\":\"en\"}\n").unwrap();
        let s = load_at(&cfg);
        assert!((s.side_split - 0.25).abs() < 1e-9);
        let json = std::fs::read_to_string(cfg.join(SETTINGS_FILE)).unwrap();
        assert!(!json.contains("\"panel\""));
        assert!(json.contains("0.25"));
        let again = load_at(&cfg);
        assert!((again.side_split - 0.25).abs() < 1e-9);
        assert_eq!(std::fs::read_to_string(cfg.join(SETTINGS_FILE)).unwrap(), json);
    }

    #[test]
    fn one_window_defaults_on_and_round_trips() {
        assert!(Session::default().one_window);
        assert!(adopt(r#"{"version":1,"single":true}"#).unwrap().one_window);
        let mut s = Session::default();
        s.one_window = false;
        assert!(!Session::parse(&s.to_ini()).one_window);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"one_window\": false"), "{text}");
        assert!(!adopt(&text).unwrap().one_window);
        assert!(!text_of_on());
    }

    fn text_of_on() -> bool {
        pretty(&SettingsFile::from_session(&Session::default())).contains("one_window")
    }

    #[test]
    fn everything_question_is_remembered() {
        assert!(!Session::default().ev_asked);
        let mut s = Session::default();
        s.ev_asked = true;
        assert!(Session::parse(&s.to_ini()).ev_asked);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"ev_asked\": true"), "{text}");
        assert!(adopt(&text).unwrap().ev_asked);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("ev_asked"));
    }

    #[test]
    fn remote_tools_question_is_remembered() {
        assert!(!Session::default().remote_asked);
        let mut s = Session::default();
        s.remote_asked = true;
        assert!(Session::parse(&s.to_ini()).remote_asked);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"remote_asked\": true"), "{text}");
        assert!(adopt(&text).unwrap().remote_asked);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("remote_asked"));
    }

    #[test]
    fn drive_buttons_defaults_on_and_round_trips() {
        assert!(Session::default().drive_buttons);
        assert!(adopt(r#"{"version":1}"#).unwrap().drive_buttons);
        let mut s = Session::default();
        s.drive_buttons = false;
        assert!(!Session::parse(&s.to_ini()).drive_buttons);
        assert!(Session::parse("drivebuttons=1").drive_buttons);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"drive_buttons\": false"), "{text}");
        assert!(!adopt(&text).unwrap().drive_buttons);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("drive_buttons"));
        // A key this build now owns is not written twice out of an older file's extra bag.
        let mut file = SettingsFile::from_session(&s);
        file.extra.insert("drive_buttons".into(), serde_json::json!(true));
        file.merge_forward(&file.clone());
        let saved: Value = serde_json::from_str(&pretty(&file)).unwrap();
        assert_eq!(saved["drive_buttons"], false);
        assert!(saved.get("extra").is_none());
    }

    #[test]
    fn old_pane_tabs_switch_is_ignored() {
        let off = adopt(r#"{"version":1,"pane_tabs":false,"panes":[{"index":0,"path":"C:\\a","folders":["C:\\a","D:\\b"],"folder":1}]}"#).unwrap();
        assert_eq!(off.pane_folders[0], vec![r"C:\a".to_string(), r"D:\b".into()]);
        assert_eq!(off.pane_cur[0], 1);
        let saved = sync_text(&off);
        assert!(!saved.contains("pane_tabs"), "{saved}");
        assert!(adopt(r#"{"pane_tabs":true}"#).is_some());
        assert!(adopt(r#"{"pane_tabs":"off"}"#).is_some());
        assert!(adopt(r#"{"pane_tabs":0}"#).is_some());
        let kept = adopt(r#"{"version":1,"pane_tabs":false,"future_flag":1}"#).unwrap();
        let again = sync_text(&kept);
        assert!(!again.contains("pane_tabs"), "{again}");
        assert!(again.contains("future_flag"));
    }

    #[test]
    fn everything_sort_lives_in_maguro_json_only() {
        assert_eq!(Session::default().ev_sort, 1);
        assert_eq!(adopt(r#"{"version":1}"#).unwrap().ev_sort, 1);
        assert_eq!(adopt(r#"{"ev_sort":9}"#).unwrap().ev_sort, 1);
        assert_eq!(adopt(r#"{"ev_sort":14}"#).unwrap().ev_sort, 14);
        let mut s = Session::default();
        s.ev_sort = 6;
        assert!(!s.to_ini().contains("ev_sort"), "{}", s.to_ini());
        assert_eq!(Session::parse(&s.to_ini()).ev_sort, 1);
        assert_eq!(Session::parse("ev_sort=14\n").ev_sort, 1);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"ev_sort\": 6"), "{text}");
        assert_eq!(adopt(&text).unwrap().ev_sort, 6);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("ev_sort"));
        let mut file = SettingsFile::from_session(&s);
        file.extra.insert("ev_sort".into(), serde_json::json!(14));
        file.merge_forward(&file.clone());
        let saved: Value = serde_json::from_str(&pretty(&file)).unwrap();
        assert_eq!(saved["ev_sort"], 6);
        assert!(saved.get("extra").is_none());
    }

    #[test]
    fn everything_column_widths_live_in_maguro_json_only() {
        assert!(Session::default().ev_cols.is_none());
        assert!(adopt(r#"{"version":1}"#).unwrap().ev_cols.is_none());
        let cols = [0.3, 0.65, 0.8];
        assert_eq!(adopt(r#"{"ev_cols":[0.3,0.65,0.8]}"#).unwrap().ev_cols, Some(cols));
        // A bad value is the default widths. The file still loads.
        for bad in [
            r#"{"ev_cols":[0.5,0.4,0.8]}"#,
            r#"{"ev_cols":[0,0.5,0.8]}"#,
            r#"{"ev_cols":[0.1,0.5,1]}"#,
            r#"{"ev_cols":[0.1,0.2]}"#,
            r#"{"ev_cols":[0.1,0.2,0.3,0.4]}"#,
            r#"{"ev_cols":"0.3,0.65,0.8"}"#,
            r#"{"ev_cols":null}"#,
        ] {
            let s = adopt(bad).unwrap();
            assert!(s.ev_cols.is_none(), "{bad}");
        }
        let mut s = Session::default();
        s.ev_cols = Some(cols);
        assert!(!s.to_ini().contains("ev_cols"), "{}", s.to_ini());
        assert!(Session::parse(&s.to_ini()).ev_cols.is_none());
        assert!(Session::parse("ev_cols=0.30,0.65,0.80\n").ev_cols.is_none());
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"ev_cols\""), "{text}");
        assert_eq!(adopt(&text).unwrap().ev_cols, Some(cols));
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("ev_cols"));
        let mut file = SettingsFile::from_session(&s);
        file.extra.insert("ev_cols".into(), serde_json::json!([0.1, 0.2, 0.3]));
        file.merge_forward(&file.clone());
        let saved: Value = serde_json::from_str(&pretty(&file)).unwrap();
        assert_eq!(saved["ev_cols"], serde_json::json!(cols));
        assert!(saved.get("extra").is_none());
    }

    #[test]
    fn use_everything_defaults_on_and_round_trips() {
        assert!(Session::default().use_everything);
        assert!(adopt(r#"{"version":1}"#).unwrap().use_everything);
        let mut s = Session::default();
        s.use_everything = false;
        assert!(!Session::parse(&s.to_ini()).use_everything);
        assert!(Session::parse("everything=1").use_everything);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"use_everything\": false"), "{text}");
        assert!(!adopt(&text).unwrap().use_everything);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("use_everything"));
    }

    #[test]
    fn dialog_jump_defaults_on_and_round_trips() {
        assert!(Session::default().dialog_jump);
        assert!(adopt(r#"{"version":1}"#).unwrap().dialog_jump);
        let mut s = Session::default();
        s.dialog_jump = false;
        assert!(!Session::parse(&s.to_ini()).dialog_jump);
        assert!(Session::parse("dialogjump=1").dialog_jump);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"dialog_jump\": false"), "{text}");
        assert!(!adopt(&text).unwrap().dialog_jump);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("dialog_jump"));
        let mut kept = Session::default();
        kept.take_layout(&Session { dialog_jump: false, ..Session::default() });
        assert!(kept.dialog_jump);
    }

    #[test]
    fn clip_hotkey_defaults_on_and_round_trips() {
        assert!(Session::default().clip_hotkey);
        assert!(adopt(r#"{"version":1}"#).unwrap().clip_hotkey);
        let mut s = Session::default();
        s.clip_hotkey = false;
        assert!(!Session::parse(&s.to_ini()).clip_hotkey);
        assert!(Session::parse("cliphotkey=1").clip_hotkey);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"clip_hotkey\": false"), "{text}");
        assert!(!adopt(&text).unwrap().clip_hotkey);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("clip_hotkey"));
        let mut kept = Session::default();
        kept.take_layout(&Session { clip_hotkey: false, ..Session::default() });
        assert!(kept.clip_hotkey);
    }

    #[test]
    fn winmerge_path_is_omitted_when_empty_and_round_trips() {
        assert!(Session::default().winmerge_path.is_empty());
        assert!(adopt(r#"{"version":1}"#).unwrap().winmerge_path.is_empty());
        assert!(!Session::default().to_ini().contains("winmerge="));
        let mut s = Session::default();
        s.winmerge_path = r"D:\Tools\WinMergeU.exe".into();
        assert_eq!(Session::parse(&s.to_ini()).winmerge_path, s.winmerge_path);
        assert_eq!(Session::parse("winmerge=C:\\WinMerge\\WinMergeU.exe").winmerge_path, r"C:\WinMerge\WinMergeU.exe");
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"winmerge_path\""), "{text}");
        assert_eq!(adopt(&text).unwrap().winmerge_path, s.winmerge_path);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("winmerge_path"));
        let mut previous = SettingsFile::blank();
        previous.extra.insert("winmerge_path".into(), serde_json::json!(r"C:\old.exe"));
        previous.extra.insert("future_flag".into(), serde_json::json!(1));
        let mut file = SettingsFile::from_session(&s);
        file.merge_forward(&previous);
        let saved = pretty(&file);
        assert_eq!(saved.matches("winmerge_path").count(), 1, "{saved}");
        assert!(saved.contains("future_flag"), "{saved}");
        assert!(saved.contains(r"D:\\Tools\\WinMergeU.exe"), "{saved}");
    }

    #[test]
    #[test]
    fn mint_brand_accent_migrates_to_cerulean_on_adopt() {
        let s = adopt(r#"{"version":1,"accent":"2EC4A6","accent_dark":"2EC4A6"}"#).unwrap();
        assert_eq!(s.accent, crate::CERULEAN_BRAND);
        assert_eq!(s.accent_dark, crate::CERULEAN_BRAND);
        let peach = adopt(r#"{"version":1,"accent":"EFAE92","accent_dark":"A8624A"}"#).unwrap();
        assert_eq!(peach.accent, 0xEFAE92);
        assert_eq!(peach.accent_dark, 0xA8624A);
    }

    #[test]
    fn old_first_swatches_migrate_to_cerulean_on_adopt_and_ini() {
        let both = adopt(r#"{"version":1,"accent":"8DB5E8","accent_dark":"4C6FA8"}"#).unwrap();
        assert_eq!(both.accent, 0x0A9BD6);
        assert_eq!(both.accent_dark, 0x0A9BD6);
        let mixed = adopt(r#"{"version":1,"accent":"EFAE92","accent_dark":"8DB5E8"}"#).unwrap();
        assert_eq!(mixed.accent, 0xEFAE92);
        assert_eq!(mixed.accent_dark, 0x0A9BD6);
        let custom = adopt(r#"{"version":1,"accent":"123456","accent_dark":"ABCDEF"}"#).unwrap();
        assert_eq!(custom.accent, 0x123456);
        assert_eq!(custom.accent_dark, 0xABCDEF);
        let ini = Session::parse("accent=8DB5E8\naccent_dark=4C6FA8\n");
        assert_eq!(ini.accent, 0x0A9BD6);
        assert_eq!(ini.accent_dark, 0x0A9BD6);
        let kept = Session::parse("accent=EFAE92\naccent_dark=123456\n");
        assert_eq!(kept.accent, 0xEFAE92);
        assert_eq!(kept.accent_dark, 0x123456);
    }

    #[test]
    fn retired_first_swatch_is_rewritten_on_load_and_other_colours_stay() {
        let sky = scratch("sky-rewrite");
        let sky_path = sky.join(SETTINGS_FILE);
        std::fs::write(&sky_path, r#"{"version":1,"ctx_win11_pack_migrated":true,"accent":"8DB5E8","accent_dark":"4C6FA8"}"#).unwrap();
        let s = try_load_at(&sky).unwrap();
        assert_eq!(s.accent, 0x0A9BD6);
        assert_eq!(s.accent_dark, 0x0A9BD6);
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&sky_path).unwrap()).unwrap();
        assert_eq!(saved["accent"], "0A9BD6");
        assert_eq!(saved["accent_dark"], "0A9BD6");

        let keep = scratch("custom-keep");
        let keep_path = keep.join(SETTINGS_FILE);
        let raw = r#"{"version":1,"ctx_win11_pack_migrated":true,"accent":"123456","accent_dark":"EFAE92"}"#;
        std::fs::write(&keep_path, raw).unwrap();
        let kept = try_load_at(&keep).unwrap();
        assert_eq!(kept.accent, 0x123456);
        assert_eq!(kept.accent_dark, 0xEFAE92);
        assert_eq!(std::fs::read_to_string(&keep_path).unwrap(), raw);
    }

    fn space_preview_defaults_on_and_round_trips() {
        assert!(Session::default().space_preview);
        assert!(adopt(r#"{"version":1}"#).unwrap().space_preview);
        assert!(Session::parse("side=1\n").space_preview);
        let mut s = Session::default();
        s.space_preview = false;
        assert!(!Session::parse(&s.to_ini()).space_preview);
        assert!(Session::parse("spacepreview=1").space_preview);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"space_preview\": false"), "{text}");
        assert!(!adopt(&text).unwrap().space_preview);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("space_preview"));
        let mut file = SettingsFile::from_session(&s);
        file.extra.insert("space_preview".into(), serde_json::json!(true));
        file.merge_forward(&file.clone());
        let saved: Value = serde_json::from_str(&pretty(&file)).unwrap();
        assert_eq!(saved["space_preview"], false);
        assert!(saved.get("extra").is_none());
    }

    #[test]
    fn preview_pane_defaults_off_and_round_trips() {
        assert!(!Session::default().preview_on);
        assert_eq!(Session::default().preview_w, 0);
        assert!(!adopt(r#"{"version":1}"#).unwrap().preview_on);
        assert_eq!(adopt(r#"{"version":1}"#).unwrap().preview_w, 0);
        assert!(!Session::parse("side=1\n").preview_on);
        let mut s = Session::default();
        s.preview_on = true;
        s.preview_w = 320;
        assert!(Session::parse(&s.to_ini()).preview_on);
        assert_eq!(Session::parse(&s.to_ini()).preview_w, 320);
        assert!(Session::parse("previewpane=1").preview_on);
        assert_eq!(Session::parse("previeww=50").preview_w, 160);
        assert_eq!(Session::parse("previeww=0").preview_w, 0);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"preview_pane\": true"), "{text}");
        assert!(text.contains("\"preview_width\": 320"), "{text}");
        let back = adopt(&text).unwrap();
        assert!(back.preview_on && back.preview_w == 320);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("preview_pane"));
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("preview_width"));
        let mut kept = Session::default();
        kept.preview_on = true;
        kept.preview_w = 300;
        kept.take_layout(&Session { preview_on: false, preview_w: 900, ..Session::default() });
        assert!(kept.preview_on);
        assert_eq!(kept.preview_w, 300);
        let mut file = SettingsFile::from_session(&s);
        file.extra.insert("preview_pane".into(), serde_json::json!(false));
        file.extra.insert("preview_width".into(), serde_json::json!(10));
        file.merge_forward(&file.clone());
        let saved: Value = serde_json::from_str(&pretty(&file)).unwrap();
        assert_eq!(saved["preview_pane"], true);
        assert_eq!(saved["preview_width"], 320);
        assert!(saved.get("extra").is_none());
    }

    #[test]
    fn remember_folder_view_defaults_on_and_round_trips_in_json() {
        assert!(Session::default().remember_views);
        assert!(adopt("{}").unwrap().remember_views);
        assert!(adopt(r#"{"version":1}"#).unwrap().remember_views);
        assert!(Session::parse("side=1\n").remember_views);
        assert!(!Session::parse("folderview=0").remember_views);
        assert!(Session::parse("folderview=1").remember_views);
        let mut s = Session::default();
        s.remember_views = false;
        assert!(!Session::parse(&s.to_ini()).remember_views);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"remember_folder_view\": false"), "{text}");
        assert!(!adopt(&text).unwrap().remember_views);
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("remember_folder_view"));
        let mut file = SettingsFile::from_session(&s);
        file.extra.insert("remember_folder_view".into(), serde_json::json!(true));
        file.merge_forward(&file.clone());
        let saved: Value = serde_json::from_str(&pretty(&file)).unwrap();
        assert_eq!(saved["remember_folder_view"], false);
        assert!(saved.get("extra").is_none());
    }

    #[test]
    fn folder_sizes_default_on_network_off_and_round_trip() {
        assert!(Session::default().folder_sizes);
        assert!(!Session::default().folder_sizes_network);
        assert!(adopt("{}").unwrap().folder_sizes);
        assert!(!adopt("{}").unwrap().folder_sizes_network);
        assert!(Session::parse("side=1\n").folder_sizes);
        assert!(!Session::parse("side=1\n").folder_sizes_network);
        assert!(!Session::parse("foldersize=0").folder_sizes);
        assert!(Session::parse("foldersizenet=1").folder_sizes_network);
        assert!(!Session::parse("foldersizenet=0").folder_sizes_network);
        let mut s = Session::default();
        s.folder_sizes = false;
        s.folder_sizes_network = true;
        let back = Session::parse(&s.to_ini());
        assert!(!back.folder_sizes && back.folder_sizes_network);
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"show_folder_sizes\": false"), "{text}");
        assert!(text.contains("\"folder_sizes_network\": true"), "{text}");
        let loaded = adopt(&text).unwrap();
        assert!(!loaded.folder_sizes && loaded.folder_sizes_network);
        let def = pretty(&SettingsFile::from_session(&Session::default()));
        assert!(!def.contains("show_folder_sizes") && !def.contains("folder_sizes_network"), "{def}");
        let mut file = SettingsFile::from_session(&s);
        file.extra.insert("show_folder_sizes".into(), serde_json::json!(true));
        file.extra.insert("folder_sizes_network".into(), serde_json::json!(false));
        file.extra.insert("future_flag".into(), serde_json::json!(1));
        file.merge_forward(&file.clone());
        let saved: Value = serde_json::from_str(&pretty(&file)).unwrap();
        assert_eq!(saved["show_folder_sizes"], false);
        assert_eq!(saved["folder_sizes_network"], true);
        assert_eq!(saved["future_flag"], 1);
        assert!(saved.get("extra").is_none());
    }

    #[test]
    fn fast_copy_defaults_off_and_round_trips() {
        assert!(!Session::default().fast_copy);
        assert!(!adopt("{}").unwrap().fast_copy);
        assert!(!adopt(r#"{"version":1}"#).unwrap().fast_copy);
        assert!(!Session::parse("side=1\n").fast_copy);
        assert!(Session::parse("fastcopy=1").fast_copy);
        assert!(!Session::parse("fastcopy=0").fast_copy);
        let mut s = Session::default();
        s.fast_copy = true;
        assert!(Session::parse(&s.to_ini()).fast_copy);
        assert!(s.to_ini().contains("fastcopy=1\n"));
        assert!(!Session::default().to_ini().contains("fastcopy"));
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"fast_copy\": true"), "{text}");
        assert!(adopt(&text).unwrap().fast_copy);
        let def = pretty(&SettingsFile::from_session(&Session::default()));
        assert!(!def.contains("fast_copy"), "{def}");
        let mut file = SettingsFile::from_session(&s);
        file.extra.insert("fast_copy".into(), serde_json::json!(false));
        file.extra.insert("future_flag".into(), serde_json::json!(1));
        file.merge_forward(&file.clone());
        let saved: Value = serde_json::from_str(&pretty(&file)).unwrap();
        assert_eq!(saved["fast_copy"], true);
        assert_eq!(saved["future_flag"], 1);
        assert!(saved.get("extra").is_none());
    }

    #[test]
    fn peek_wheel_nav_defaults_off_when_the_key_is_missing() {
        assert!(!Session::default().peek_wheel_nav);
        assert!(!adopt("{}").unwrap().peek_wheel_nav);
        assert!(!adopt(r#"{"version":1}"#).unwrap().peek_wheel_nav);
        assert!(Session::parse("peekwheelnav=1").peek_wheel_nav);
        assert!(!Session::parse("peekwheelnav=0").peek_wheel_nav);
        let mut s = Session::default();
        s.peek_wheel_nav = true;
        assert!(Session::parse(&s.to_ini()).peek_wheel_nav);
    }


    #[test]
    fn peek_sizes_round_trip_in_dip_and_stay_out_until_set() {
        assert!(Session::default().peek_image.is_none() && Session::default().peek_doc.is_none());
        assert!(adopt(r#"{"version":1}"#).unwrap().peek_image.is_none());
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("peek"));
        let mut s = Session::default();
        s.peek_image = Some((960, 540));
        s.peek_doc = Some((820, 720));
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"image\": [") || text.contains("\"image\":["), "{text}");
        assert!(text.contains("960") && text.contains("540") && text.contains("820"), "{text}");
        let back = adopt(&text).unwrap();
        assert_eq!((back.peek_image, back.peek_doc), (Some((960, 540)), Some((820, 720))));
        // A named session is the ini shape and does not carry the quick look size.
        assert!(Session::parse(&s.to_ini()).peek_image.is_none());
        let bad = adopt(r#"{"version":1,"peek":{"image":[0,10],"doc":[-4,20]}}"#).unwrap();
        assert!(bad.peek_image.is_none() && bad.peek_doc.is_none());
        let one = adopt(r#"{"version":1,"peek":{"doc":[400,300]}}"#).unwrap();
        assert_eq!(one.peek_doc, Some((400, 300)));
        assert!(one.peek_image.is_none());
    }

    #[test]
    fn utf8_bom_loads_the_favorites_tree() {
        let root = scratch("bom-fav");
        let exe = root.join("exe");
        let cfg = root.join("cfg");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(&cfg).unwrap();
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend(br#"{"version":1,"favorites_tree":[{"name":"Jobs","children":[{"name":"note","path":"C:\\a\\note.txt","file":true}]}]}"#);
        std::fs::write(cfg.join(SETTINGS_FILE), raw).unwrap();
        let s = load_at(&cfg);
        assert_eq!(s.tree.len(), 1);
        assert_eq!(s.tree[0].name, "Jobs");
        assert!(s.tree[0].is_group());
        assert_eq!(s.tree[0].children[0].name, "note");
        assert!(s.tree[0].children[0].file);
        assert!(s.is_fav(r"C:\a\note.txt"));
        let imported = import_text("\u{FEFF}{\"version\":1,\"favorites_tree\":[{\"name\":\"Places\",\"path\":\"D:\\\\Photos\"}]}").unwrap();
        assert_eq!(imported.tree[0].name, "Places");
        assert!(imported.is_fav(r"D:\Photos"));
    }

    #[test]
    fn font_face_round_trips_present_absent_and_unknown_keys() {
        let _ = adopt("{}");
        assert!(Session::default().font_face.is_empty());
        let bare = adopt(r#"{"version":1,"lang":"ja","future":{"keep":true}}"#).unwrap();
        assert!(bare.font_face.is_empty());
        assert_eq!(bare.lang, Lang::Ja);
        let saved_bare: Value = serde_json::from_str(&sync_text(&bare)).unwrap();
        assert!(saved_bare.get("font_face").is_none());
        assert_eq!(saved_bare["future"]["keep"], true);
        assert_eq!(saved_bare["lang"], "ja");

        let mut s = bare.clone();
        s.font_face = "Yu Gothic UI".into();
        let saved: Value = serde_json::from_str(&sync_text(&s)).unwrap();
        assert_eq!(saved["font_face"], "Yu Gothic UI");
        assert_eq!(saved["future"]["keep"], true);
        let back = adopt(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(back.font_face, "Yu Gothic UI");
        assert_eq!(back.lang, Lang::Ja);

        let mut named = Session::default();
        named.font_face = "Consolas".into();
        named.lang = Lang::En;
        assert!(!named.to_ini().contains("font_face"));
        assert!(Session::parse(&format!("font_face=Consolas\n{}", named.to_ini())).font_face.is_empty());
    }

    #[test]
    fn old_font_size_key_still_parses() {
        let s = adopt(r#"{"version":1,"font":2,"font_face":"Consolas","lang":"en","future":{"keep":true}}"#).unwrap();
        assert_eq!(s.lang, Lang::En);
        assert_eq!(s.font_face, "Consolas");
        let saved: Value = serde_json::from_str(&sync_text(&s)).unwrap();
        assert!(saved.get("font").is_none(), "{saved}");
        assert_eq!(saved["font_face"], "Consolas");
        assert_eq!(saved["lang"], "en");
        assert_eq!(saved["future"]["keep"], true);
        let odd = adopt(r#"{"font":"large","lang":"ja"}"#).unwrap();
        assert_eq!(odd.lang, Lang::Ja);
        let dropped: Value = serde_json::from_str(&sync_text(&odd)).unwrap();
        assert!(dropped.get("font").is_none(), "{dropped}");

        let ini = Session::parse("font=2\nlang=ko\nbar=0\n");
        assert_eq!(ini.lang, Lang::Ko);
        assert!(!ini.bar);
        assert!(!ini.to_ini().contains("font="));
    }

    #[test]
    fn load_drops_a_font_face_that_is_not_installed() {
        let root = scratch("missing-face");
        let exe = root.join("exe");
        let cfg = root.join("cfg");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::write(cfg.join(SETTINGS_FILE), "{\"version\":1,\"lang\":\"ja\",\"font_face\":\"Not A Real Font Zzz\",\"future\":1}\n").unwrap();
        let s = load_at(&cfg);
        assert!(s.font_face.is_empty(), "missing face should fall back");
        assert_eq!(s.lang, Lang::Ja);
        let kept: Value = serde_json::from_str(&current_text()).unwrap();
        assert!(kept.get("font_face").is_none());
        assert_eq!(kept["future"], 1);
        assert_eq!(kept["lang"], "ja");
    }

    #[test]
    fn config_paths_use_maguro_names() {
        assert_eq!(SETTINGS_FILE, "maguro.json");
        assert_eq!(settings_path().file_name().and_then(|s| s.to_str()), Some("maguro.json"));
        let base = Path::new(r"C:\Users\me\AppData\Roaming");
        assert_eq!(appdata_dir(base), base.join("Maguro"));
        assert!(appdata_dir(base).ends_with("Maguro"));
    }



    #[test]
    fn default_view_for_a_new_pane_roundtrips_in_json() {
        let mut s = Session::default();
        s.view_new = 1;
        s.view_icon = 48;
        let text = pretty(&SettingsFile::from_session(&s));
        assert!(text.contains("\"view_new\": 1"), "{text}");
        assert!(text.contains("\"view_icon\": 48"), "{text}");
        let back = adopt(&text).unwrap();
        assert_eq!((back.view_new, back.view_icon), (1, 48));
        assert_eq!(back.default_view(), (1, 48));
        s.view_new = 6;
        s.view_icon = 0;
        let tiles = pretty(&SettingsFile::from_session(&s));
        assert!(tiles.contains("\"view_new\": 6"), "{tiles}");
        assert!(!tiles.contains("view_icon"), "{tiles}");
        assert_eq!(adopt(&tiles).unwrap().default_view(), (6, 48));
        assert_eq!(adopt(r#"{"view_new":1}"#).unwrap().default_view(), (1, 96));
        assert_eq!(adopt(r#"{"view_new":8}"#).unwrap().default_view(), (8, 32));
        assert_eq!(adopt(r#"{"view_new":9,"view_icon":48}"#).unwrap().default_view(), (4, 16));
        assert!(!pretty(&SettingsFile::from_session(&Session::default())).contains("view_icon"));
    }

    #[test]
    fn excel_tidy_choices_default_backup_off_and_round_trip_in_json_only() {
        let fresh = adopt("{}").unwrap();
        assert!(fresh.excel_tidy_a1 && fresh.excel_tidy_zoom && fresh.excel_tidy_first);
        assert!(!fresh.excel_tidy_backup);
        assert!(!sync_text(&fresh).contains("excel_tidy"));
        let mut s = fresh;
        s.excel_tidy_zoom = false;
        s.excel_tidy_backup = true;
        let text = sync_text(&s);
        assert!(text.contains("\"excel_tidy\""), "{text}");
        assert!(text.contains("\"zoom\": false"), "{text}");
        assert!(text.contains("\"backup\": true"), "{text}");
        let back = adopt(&text).unwrap();
        assert!(back.excel_tidy_a1 && back.excel_tidy_first);
        assert!(!back.excel_tidy_zoom && back.excel_tidy_backup);
        let partial = adopt(r#"{"excel_tidy":{"backup":true}}"#).unwrap();
        assert!(partial.excel_tidy_a1 && partial.excel_tidy_zoom && partial.excel_tidy_first);
        assert!(partial.excel_tidy_backup);
        let partial_off = adopt(r#"{"excel_tidy":{"zoom":false}}"#).unwrap();
        assert!(partial_off.excel_tidy_a1 && partial_off.excel_tidy_first);
        assert!(!partial_off.excel_tidy_zoom && !partial_off.excel_tidy_backup);
        assert!(!s.to_ini().contains("excel_tidy"));
        assert!(!Session::parse(&s.to_ini()).excel_tidy_backup);
    }
}

fn is_wheel_off(v: &bool) -> bool {
    !*v
}
