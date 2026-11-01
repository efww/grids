// Third-party context menu entries, and the Maguro-only filter.
//
// The registry is read, never written. Enablement lives in maguro.json (`ctx_menu`),
// not in HKCR and not in a .grids file. Explorer is unchanged.
//
// The visible menu is always the shell view's own `IContextMenu` (the same object as
// when nothing is disabled). `SHCreateDefaultContextMenu` with a filtered `aKeys` list
// cannot match that file menu: dropping one class key (`*`, a ProgID, SystemFileAssociations)
// also drops every other verb on it, and putting those verbs back by hand changes order
// and adds items the shell would have hidden. Folder clicks happened to look closer
// because their menu is mostly Directory/Folder keys. `compose` is kept below and is
// not used for the popup.
//
// When a handler is off for this click, `popup_ctx` builds that normal menu and then
// deletes only the disabled handler's commands. A static verb matches its registry
// verb and the menu text this Windows language resolved, including MuiVerb.
// A COM handler is CoCreated into a throwaway menu to learn the strings it inserts;
// those strings are deleted from the real menu. With a selection, the handler is
// asked with the data object alone and again with the parent folder pidl. WinMerge
// counts that pidl as a second selected path, so the two calls insert different
// items, and every inserted string is eligible for deletion. The shell has already
// loaded every handler on the real menu, including ones that are off — that is the
// cost of an identical item set. Disabled handlers are never given their own visible menu.
//
// Set `MAGURO_CTX_TIMING=1` to log each phase with OutputDebugString (DebugView).
// `query` is the shell's own `QueryContextMenu`. The first call loads each handler DLL
// and runs its class factory; COM later unloads unused ones (`CoFreeUnusedLibraries`,
// about ten minutes idle) and the cost comes back. `warmup` does that load off the UI
// thread. `learn` asks unnamed handlers for their labels after the popup has closed.
//
// Settings shows a product name (VERSIONINFO, then a known-vendor name) and, under it,
// the menu strings that handler adds. Static verbs contribute their registry text.
// A real menu records the strings the user actually saw (`ctx_labels` in maguro.json).
// Handlers that share a product title are one settings row; a toggle writes every
// handler in that row. Recording never inserts, deletes, or reorders the menu.
//
// A handler that was asked and added no visible item is remembered (`ctx_quiet`) and
// left off the settings list. The shell menu still loads it. Handlers whose product
// is the generic Windows operating system, and Screen saver, stay off that list
// before any probe: they do not insert a row in this menu. A label recorded from a
// real menu lists them again. WinMerge starts off: its own commands and its shell
// handler share one row, and an explicit on is `ctx_on`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
    RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_MULTI_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER};

use crate::imports::*;
use crate::app::*;
use crate::i18n::tr;

/// `CMINVOKECOMMANDINFOEX.lpVerbW` is a Unicode verb. The windows crate no longer exports the flag.
const CMIC_MASK_UNICODE: u32 = 0x0000_4000;

pub(crate) const FILE: u8 = 1;
pub(crate) const FOLDER: u8 = 2;
pub(crate) const BACKGROUND: u8 = 4;
pub(crate) const DRIVE: u8 = 8;
const TARGET_ALL: u8 = FILE | FOLDER | BACKGROUND | DRIVE;
/// Maguro's own WinMerge commands. The shell extension, when installed, shares this switch.
pub(crate) const APP_WINMERGE: &str = "app:winmerge";
/// WinMerge's Explorer context-menu handler. `GetCommandString` returns no verb; both items are the text it inserts.
const WINMERGE_CLSID: &str = "clsid:4e716236-aa30-4c65-b225-d68bba81e9c2";

const FIXED: &[(&str, u8)] = &[
    ("*", FILE),
    ("AllFilesystemObjects", FILE | FOLDER | DRIVE),
    ("Directory", FOLDER | DRIVE),
    ("Directory\\Background", BACKGROUND),
    ("Folder", FOLDER | DRIVE),
    ("Folder\\Background", BACKGROUND),
    ("Drive", DRIVE),
];

const BASE_END: u32 = 0x3000;
const SLOT: u32 = 0x100;
const VERB_BASE: u32 = 0x7000;
const VERB_END: u32 = 0x7F00;
const KEY_CAP: usize = 16;

link!("shell32.dll" "system" fn SHCreateDefaultContextMenu(pdcm: *const DefCtx, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> HRESULT);
link!("kernel32.dll" "system" fn ExpandEnvironmentStringsW(src: PCWSTR, dst: PWSTR, n: u32) -> u32);
link!("shlwapi.dll" "system" fn SHLoadIndirectString(src: PCWSTR, dst: PWSTR, cch: u32, reserved: *mut core::ffi::c_void) -> HRESULT);
link!("version.dll" "system" fn GetFileVersionInfoSizeW(file: PCWSTR, handle: *mut u32) -> u32);
link!("version.dll" "system" fn GetFileVersionInfoW(file: PCWSTR, handle: u32, len: u32, data: *mut core::ffi::c_void) -> BOOL);
link!("version.dll" "system" fn VerQueryValueW(block: *const core::ffi::c_void, sub: PCWSTR, buf: *mut *mut core::ffi::c_void, len: *mut u32) -> BOOL);

#[repr(C)]
struct DefCtx {
    hwnd: HWND,
    pcmcb: *mut core::ffi::c_void,
    pidl_folder: *const ITEMIDLIST,
    psf: *mut core::ffi::c_void,
    cidl: u32,
    _pad0: u32,
    apidl: *const *const ITEMIDLIST,
    punk: *mut core::ffi::c_void,
    ckeys: u32,
    _pad1: u32,
    akeys: *const HKEY,
}

#[derive(Clone)]
struct Row {
    id: String,
    name: String,
    targets: u8,
    builtin: bool,
    clsid: Option<GUID>,
    verb: String,
    command: String,
    rank: u8,
    extended: bool,
    single: bool,
    position: i8,
    classes: Vec<String>,
    /// Class passed to IShellExtInit. The most specific registration.
    anchor: String,
    /// Friendly product title when it is better than `name`. Empty for a verb whose menu text is the title.
    product: String,
    /// Registry key or class name, shown dimmed when it is not the title.
    raw: String,
    /// Menu strings read from the registry. Observed strings live in `ctx_labels`.
    static_labels: Vec<String>,
}

struct Part {
    first: u32,
    cm: IContextMenu,
}

struct VerbSlot {
    id: u32,
    command: String,
}

#[implement(IContextMenu, IContextMenu2, IContextMenu3)]
struct CtxHost {
    folder: IShellFolder,
    folder_pidl: *mut ITEMIDLIST,
    children: Vec<*mut ITEMIDLIST>,
    data: Option<IDataObject>,
    owner: HWND,
    targets: u8,
    paths: Vec<String>,
    classes: Vec<String>,
    base: RefCell<Option<IContextMenu>>,
    parts: RefCell<Vec<Part>>,
    verbs: RefCell<Vec<VerbSlot>>,
    keys: RefCell<Vec<HKEY>>,
}

impl Drop for CtxHost {
    fn drop(&mut self) {
        unsafe {
            if !self.folder_pidl.is_null() {
                CoTaskMemFree(Some(self.folder_pidl as _));
            }
            for p in &self.children {
                if !p.is_null() {
                    CoTaskMemFree(Some(*p as _));
                }
            }
            for k in self.keys.get_mut().iter().copied() {
                _ = RegCloseKey(k);
            }
        }
    }
}

pub(crate) struct UiRow {
    /// Expand key. The handler id, or `group:` plus every id when several share a title.
    pub(crate) id: String,
    /// Every handler a click on this row changes.
    pub(crate) ids: Vec<String>,
    /// Target mask of each id, in the same order as `ids`.
    pub(crate) scopes: Vec<u8>,
    /// Friendly product name. The large title on the settings row.
    pub(crate) name: String,
    /// Component name, empty when it would repeat the title or a menu label.
    pub(crate) raw: String,
    /// Menu item labels, joined for the line under the title. Empty when unknown or identical to the title.
    pub(crate) detail: String,
    pub(crate) targets: u8,
    pub(crate) tick: Tick,
    /// File, folder, empty space, drive.
    pub(crate) subs: [Tick; 4],
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[derive(Debug)]
pub(crate) enum Tick {
    Off,
    On,
    Mixed,
}

pub(crate) fn target_label(bit: u8) -> &'static str {
    match bit {
        FILE => tr(crate::Key::File),
        FOLDER => tr(crate::Key::Folder3),
        BACKGROUND => tr(crate::Key::EmptySpace),
        DRIVE => tr(crate::Key::Drive),
        _ => "",
    }
}

pub(crate) fn target_bits() -> [u8; 4] {
    [FILE, FOLDER, BACKGROUND, DRIVE]
}

/// A drive root (`C:` or `C:\`) is a drive. Anything else is a file or a folder.
pub(crate) fn path_kind(path: &str, is_file: bool) -> u8 {
    let t = path.trim().trim_end_matches(['\\', '/']);
    let b = t.as_bytes();
    if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        DRIVE
    } else if is_file {
        FILE
    } else {
        FOLDER
    }
}

pub(crate) fn click_targets(background: bool, items: &[(String, bool)]) -> u8 {
    if background {
        return BACKGROUND;
    }
    items.iter().fold(0, |a, (p, file)| a | path_kind(p, *file))
}

pub(crate) fn needs_filter(targets: u8) -> bool {
    if targets == 0 {
        return false;
    }
    app(|a| {
        a.s.ctx_off.values().any(|v| bits_of(v) & targets != 0) || !winmerge_covers(&a.s.ctx_on, targets)
    })
}

/// WinMerge's right-click commands are shown only when this click's targets were explicitly enabled.
pub(crate) fn winmerge_menu_on(targets: u8) -> bool {
    if targets == 0 {
        return false;
    }
    app(|a| winmerge_covers(&a.s.ctx_on, targets))
}

pub(crate) fn tick_of(off: u8, attached: u8) -> Tick {
    let off = off & attached;
    if attached == 0 || off == 0 {
        Tick::On
    } else if off == attached {
        Tick::Off
    } else {
        Tick::Mixed
    }
}

#[cfg(test)]
pub(crate) fn target_enabled(off: u8, bit: u8) -> bool {
    off & bit == 0
}

/// Master checkbox. On means every attached target is enabled; a click then turns them all off.
/// Anything else (off or mixed) turns them all on.
#[cfg(test)]
pub(crate) fn toggle_master(map: &mut BTreeMap<String, Vec<String>>, id: &str, attached: u8) {
    let cur = bits_of(map.get(id).map(Vec::as_slice).unwrap_or(&[]));
    let next = if cur & attached == 0 { attached } else { cur & !attached };
    write_bits(map, id, next & TARGET_ALL);
}

#[cfg(test)]
pub(crate) fn toggle_target(map: &mut BTreeMap<String, Vec<String>>, id: &str, bit: u8, attached: u8) {
    let cur = bits_of(map.get(id).map(Vec::as_slice).unwrap_or(&[]));
    let next = (cur ^ bit) & attached & TARGET_ALL;
    write_bits(map, id, next);
}

/// One decision for the whole product group, then the same direction is written to every handler.
/// A mixed group turns every member on. Each handler keeps only its own target bits.
/// WinMerge is the exception: the row starts off, and an on is written to `ctx_on`.
pub(crate) fn toggle_ids_master(off: &mut BTreeMap<String, Vec<String>>, on: &mut BTreeMap<String, Vec<String>>, ids: &[String], scopes: &[u8]) {
    if ids.is_empty() || ids.len() != scopes.len() {
        return;
    }
    if let Some(i) = ids.iter().position(|id| id == APP_WINMERGE) {
        let scope = scopes[i];
        let cur = winmerge_bits(on);
        let fully_on = scope != 0 && cur & scope == scope;
        let next = if fully_on { cur & !scope } else { cur | scope };
        write_bits(on, APP_WINMERGE, next & TARGET_ALL);
        return;
    }
    let fully_on = ids.iter().zip(scopes).all(|(id, attached)| {
        let cur = bits_of(off.get(id).map(Vec::as_slice).unwrap_or(&[]));
        cur & attached == 0
    });
    for (id, attached) in ids.iter().zip(scopes) {
        let cur = bits_of(off.get(id).map(Vec::as_slice).unwrap_or(&[]));
        let next = if fully_on { *attached } else { cur & !attached };
        write_bits(off, id, next & TARGET_ALL);
    }
}

pub(crate) fn toggle_ids_target(off: &mut BTreeMap<String, Vec<String>>, on: &mut BTreeMap<String, Vec<String>>, ids: &[String], scopes: &[u8], bit: u8) {
    if ids.len() != scopes.len() || bit == 0 {
        return;
    }
    if ids.iter().any(|id| id == APP_WINMERGE) {
        let cur = winmerge_bits(on);
        let next = if cur & bit != 0 { cur & !bit } else { cur | bit };
        write_bits(on, APP_WINMERGE, next & TARGET_ALL);
        return;
    }
    let relevant: Vec<usize> = scopes.iter().enumerate().filter(|(_, t)| *t & bit != 0).map(|(i, _)| i).collect();
    if relevant.is_empty() {
        return;
    }
    let fully_on = relevant.iter().all(|&i| {
        let cur = bits_of(off.get(&ids[i]).map(Vec::as_slice).unwrap_or(&[]));
        cur & bit == 0
    });
    for i in relevant {
        let cur = bits_of(off.get(&ids[i]).map(Vec::as_slice).unwrap_or(&[]));
        let attached = scopes[i];
        let next = if fully_on { (cur | bit) & attached } else { (cur & !bit) & attached };
        write_bits(off, &ids[i], next & TARGET_ALL);
    }
}

pub(crate) fn clear_off(map: &mut BTreeMap<String, Vec<String>>) {
    map.clear();
}

/// Settings → Right-click menu → Reset. Every handler on, including WinMerge, and that on is stored.
pub(crate) fn reset_menu(off: &mut BTreeMap<String, Vec<String>>, on: &mut BTreeMap<String, Vec<String>>) {
    clear_off(off);
    write_bits(on, APP_WINMERGE, TARGET_ALL);
}

pub(crate) fn ui_rows(off: &BTreeMap<String, Vec<String>>, on: &BTreeMap<String, Vec<String>>, seen: &BTreeMap<String, Vec<String>>, quiet: &BTreeSet<String>, q: &str) -> Vec<UiRow> {
    let q = q.trim().to_lowercase();
    let mut rows: Vec<Row> = catalogue().into_iter().filter(|r| listed(r, seen, quiet)).collect();
    let wm = app_winmerge_row();
    let sort_key = title_of(&wm).to_lowercase();
    let at = rows.iter().position(|r| title_of(r).to_lowercase() > sort_key).unwrap_or(rows.len());
    rows.insert(at, wm);
    assemble(&rows, off, on, seen, &q)
}

/// Search hit on a handler, its component name, or a menu label. A one-character query does not walk the registry.
pub(crate) fn name_hit(q: &str) -> bool {
    let q = q.trim();
    if q.chars().count() < 2 {
        return false;
    }
    let q = q.to_lowercase();
    let seen = app(|a| a.s.ctx_labels.clone());
    if "winmerge".contains(&q) || crate::winmerge::context_labels().iter().any(|l| l.to_lowercase().contains(&q)) {
        return true;
    }
    catalogue().iter().any(|r| !r.builtin && row_matches(r, &seen, &q))
}

struct Presented {
    name: String,
    raw: String,
    labels: Vec<String>,
    #[allow(dead_code)] // learning state; the settings row no longer prints a hint
    pending: bool,
}

fn present_row(r: &Row, seen: &BTreeMap<String, Vec<String>>) -> Presented {
    let name = title_of(r);
    let mut labels = Vec::new();
    push_unique(&mut labels, &r.static_labels);
    if !r.verb.is_empty() {
        push_unique(&mut labels, std::slice::from_ref(&r.name));
    }
    if let Some(extra) = seen.get(&r.id) {
        push_unique(&mut labels, extra);
    }
    let known = !r.static_labels.is_empty() || (!r.verb.is_empty() && !r.name.is_empty()) || seen.get(&r.id).is_some_and(|v| !v.is_empty());
    let shown: Vec<String> = labels.into_iter().filter(|l| name.is_empty() || fold_menu(l) != fold_menu(&name)).collect();
    let raw = display_raw(&name, &r.raw, &shown);
    Presented { name, raw, labels: shown, pending: !known }
}

fn row_matches(r: &Row, seen: &BTreeMap<String, Vec<String>>, q: &str) -> bool {
    let title = title_of(r).to_lowercase();
    if title.contains(q) || r.name.to_lowercase().contains(q) || r.raw.to_lowercase().contains(q) || r.product.to_lowercase().contains(q) || target_blob(r.targets).contains(q) {
        return true;
    }
    if r.static_labels.iter().any(|l| l.to_lowercase().contains(q)) {
        return true;
    }
    seen.get(&r.id).is_some_and(|v| v.iter().any(|l| l.to_lowercase().contains(q)))
}

fn title_of(r: &Row) -> String {
    if let Some(t) = usable_title(&r.product) {
        return t;
    }
    // A stored readable fallback may be the menu text ("Configure"). A placeholder still is not.
    if kept_title(&r.product) {
        return r.product.clone();
    }
    // A static verb's `name` is the registry menu text used to delete the item. It is not a product.
    if r.verb.is_empty() {
        if let Some(t) = usable_title(&r.name) {
            return t;
        }
    }
    if let Some(t) = usable_title(&r.raw) {
        return t;
    }
    let raw = r.raw.trim();
    if kept_title(raw) && !menu_sentence(raw) {
        raw.to_string()
    } else {
        String::new()
    }
}

fn usable_title(s: &str) -> Option<String> {
    let t = s.trim();
    if !kept_title(t) || menu_sentence(t) {
        None
    } else {
        Some(t.to_string())
    }
}

fn kept_title(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && !title_junk(t) && !identifier_like(t) && parse_guid(t).is_none()
}

/// Empty titles stay separate. Two "Configure" verbs must not become one product.
fn group_key(title: &str) -> Option<String> {
    let t = title.trim();
    if t.is_empty() {
        None
    } else {
        Some(fold_menu(t))
    }
}

fn display_raw(title: &str, raw: &str, labels: &[String]) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let folded = fold_menu(raw);
    if folded == fold_menu(title) || labels.iter().any(|l| fold_menu(l) == folded) {
        String::new()
    } else {
        raw.to_string()
    }
}

fn push_unique(slot: &mut Vec<String>, incoming: &[String]) {
    for label in incoming {
        let label = menu_label(label);
        if label.is_empty() {
            continue;
        }
        if slot.iter().any(|e| fold_menu(e) == fold_menu(&label)) {
            continue;
        }
        slot.push(label);
        if slot.len() >= 8 {
            break;
        }
    }
}

struct Acc {
    title: String,
    key: String,
    ids: Vec<String>,
    scopes: Vec<u8>,
    targets: u8,
    labels: Vec<String>,
    raws: Vec<String>,
}

fn assemble(rows: &[Row], off: &BTreeMap<String, Vec<String>>, on: &BTreeMap<String, Vec<String>>, seen: &BTreeMap<String, Vec<String>>, q: &str) -> Vec<UiRow> {
    let mut accs: Vec<Acc> = Vec::new();
    for r in rows {
        let shown = present_row(r, seen);
        let winmerge = row_defaults_off(r);
        let key = if winmerge { Some("winmerge".to_string()) } else { group_key(&shown.name) };
        let title = if winmerge { "WinMerge".to_string() } else { shown.name };
        if let Some(k) = key.as_ref() {
            if let Some(acc) = accs.iter_mut().find(|a| a.key == *k) {
                acc.ids.push(r.id.clone());
                acc.scopes.push(r.targets);
                acc.targets |= r.targets;
                push_unique(&mut acc.labels, &shown.labels);
                if !shown.raw.is_empty() {
                    acc.raws.push(shown.raw);
                }
                continue;
            }
        }
        accs.push(Acc {
            key: key.unwrap_or_else(|| format!("id:{}", r.id)),
            title,
            ids: vec![r.id.clone()],
            scopes: vec![r.targets],
            targets: r.targets,
            labels: shown.labels,
            raws: if shown.raw.is_empty() { Vec::new() } else { vec![shown.raw] },
        });
    }
    let mut out = Vec::new();
    for acc in accs {
        let hit = q.is_empty()
            || acc.title.to_lowercase().contains(q)
            || acc.labels.iter().any(|l| l.to_lowercase().contains(q))
            || acc.raws.iter().any(|raw| raw.to_lowercase().contains(q))
            || rows.iter().any(|r| acc.ids.iter().any(|id| id == &r.id) && row_matches(r, seen, q));
        if !hit {
            continue;
        }
        let labels: Vec<String> = acc.labels.into_iter().filter(|l| fold_menu(l) != fold_menu(&acc.title)).collect();
        let raw = shared_raw(&acc.title, &acc.raws, &labels);
        let (tick, subs) = group_ticks(off, on, &acc.ids, &acc.scopes);
        let id = if acc.ids.len() == 1 {
            acc.ids[0].clone()
        } else {
            let mut sorted = acc.ids.clone();
            sorted.sort();
            format!("group:{}", sorted.join("|"))
        };
        out.push(UiRow {
            id,
            ids: acc.ids,
            scopes: acc.scopes,
            name: acc.title,
            raw,
            detail: labels.join(" · "),
            targets: acc.targets,
            tick,
            subs,
        });
    }
    out
}

fn shared_raw(title: &str, raws: &[String], labels: &[String]) -> String {
    let Some(first) = raws.first() else { return String::new() };
    let folded = fold_menu(first);
    if raws.iter().any(|raw| fold_menu(raw) != folded) {
        return String::new();
    }
    display_raw(title, first, labels)
}

fn group_ticks(off: &BTreeMap<String, Vec<String>>, on: &BTreeMap<String, Vec<String>>, ids: &[String], scopes: &[u8]) -> (Tick, [Tick; 4]) {
    if ids.iter().any(|id| id == APP_WINMERGE) {
        let enabled = winmerge_bits(on);
        let scope = ids.iter().zip(scopes).fold(0u8, |a, (_, s)| a | s);
        let bits = [FILE, FOLDER, BACKGROUND, DRIVE];
        let mut subs = [Tick::On; 4];
        for (i, bit) in bits.into_iter().enumerate() {
            subs[i] = if scope & bit == 0 || enabled & bit != 0 { Tick::On } else { Tick::Off };
        }
        return (tick_of(scope & !enabled, scope), subs);
    }
    let bits = [FILE, FOLDER, BACKGROUND, DRIVE];
    let mut subs = [Tick::On; 4];
    for (i, bit) in bits.into_iter().enumerate() {
        let mut relevant = 0usize;
        let mut offs = 0usize;
        for (id, scope) in ids.iter().zip(scopes) {
            if scope & bit == 0 {
                continue;
            }
            relevant += 1;
            let cur = bits_of(off.get(id).map(Vec::as_slice).unwrap_or(&[]));
            if cur & bit != 0 {
                offs += 1;
            }
        }
        subs[i] = if relevant == 0 || offs == 0 {
            Tick::On
        } else if offs == relevant {
            Tick::Off
        } else {
            Tick::Mixed
        };
    }
    let mut states = Vec::new();
    for (id, scope) in ids.iter().zip(scopes) {
        if *scope == 0 {
            continue;
        }
        let cur = bits_of(off.get(id).map(Vec::as_slice).unwrap_or(&[]));
        states.push(tick_of(cur, *scope));
    }
    let tick = if states.is_empty() || states.iter().all(|t| *t == Tick::On) {
        Tick::On
    } else if states.iter().all(|t| *t == Tick::Off) {
        Tick::Off
    } else {
        Tick::Mixed
    };
    (tick, subs)
}

pub(crate) fn invalidate() {
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
    // Refresh and the right-click page both call this. The packaged catalogue
    // is a separate cache and must drop with it, or the Windows 11 view stays stale.
    super::ctxpack::invalidate();
}

thread_local! {
    static STRIP: Cell<u8> = const { Cell::new(0) };
    static PROBE: RefCell<Option<ProbeCtx>> = const { RefCell::new(None) };
}

struct ProbeCtx {
    folder_pidl: *mut ITEMIDLIST,
    data: Option<IDataObject>,
}

pub(crate) struct ProbeGuard;

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        clear_probe();
    }
}

pub(crate) fn arm_strip(targets: u8) {
    STRIP.with(|s| s.set(targets));
}

pub(crate) fn take_strip() -> u8 {
    STRIP.with(|s| s.replace(0))
}

/// Remember the folder of this click so a disabled COM handler can be asked, off to the side,
/// which menu strings it would add. The visible menu is still the shell's.
pub(crate) unsafe fn arm_probe(owner: HWND, folder: &IShellFolder, children: &[*const ITEMIDLIST]) -> ProbeGuard {
    clear_probe();
    let folder_pidl = SHGetIDListFromObject(folder).unwrap_or(std::ptr::null_mut());
    let refs: Vec<*const ITEMIDLIST> = children.iter().copied().filter(|p| !p.is_null()).collect();
    let data = if refs.is_empty() { None } else { folder.GetUIObjectOf::<IDataObject>(owner, &refs, None).ok() };
    PROBE.with(|p| *p.borrow_mut() = Some(ProbeCtx { folder_pidl, data }));
    ProbeGuard
}

pub(crate) unsafe fn arm_probe_paths(owner: HWND, items: &[(String, bool)]) -> Option<ProbeGuard> {
    let parent = crate::parent_of(&items.first()?.0)?;
    if !items.iter().all(|(p, _)| crate::parent_of(p).is_some_and(|q| q.eq_ignore_ascii_case(&parent))) {
        return None;
    }
    let mut folder_pidl = std::ptr::null_mut();
    SHParseDisplayName(&HSTRING::from(&parent), None, &mut folder_pidl, 0, None).ok()?;
    if folder_pidl.is_null() {
        return None;
    }
    let desktop = SHGetDesktopFolder().ok();
    let folder: Option<IShellFolder> = desktop.as_ref().and_then(|d| d.BindToObject(folder_pidl, None).ok());
    CoTaskMemFree(Some(folder_pidl as _));
    let folder = folder?;
    let mut kids: Vec<*mut ITEMIDLIST> = Vec::new();
    for (path, _) in items {
        let name = path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(path);
        if name.is_empty() {
            continue;
        }
        let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
        let wide = HSTRING::from(name);
        if folder.ParseDisplayName(HWND::default(), None, &wide, None, &mut child, std::ptr::null_mut()).is_ok() && !child.is_null() {
            kids.push(child);
        }
    }
    if kids.len() != items.len() {
        for p in &kids {
            CoTaskMemFree(Some(*p as _));
        }
        return None;
    }
    let refs: Vec<*const ITEMIDLIST> = kids.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let guard = arm_probe(owner, &folder, &refs);
    for p in kids {
        CoTaskMemFree(Some(p as _));
    }
    Some(guard)
}

fn clear_probe() {
    let old = PROBE.with(|p| p.borrow_mut().take());
    if let Some(ctx) = old {
        unsafe {
            if !ctx.folder_pidl.is_null() {
                CoTaskMemFree(Some(ctx.folder_pidl as _));
            }
        }
    }
}

pub(crate) fn log_phase(phase: &str, t0: Instant) {
    if !timing() {
        return;
    }
    let ms = t0.elapsed().as_millis();
    log_line(&format!("[maguro] ctx {phase} {ms} ms"));
}

pub(crate) fn log_line(msg: &str) {
    if !timing() {
        return;
    }
    let line = msg.trim_end_matches('\0');
    let dbg = format!("{line}\0");
    let w: Vec<u16> = dbg.encode_utf16().collect();
    unsafe { OutputDebugStringW(PCWSTR(w.as_ptr())) };
    if let Some(path) = timing_log_path() {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

fn timing_log_path() -> Option<std::path::PathBuf> {
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        if let Ok(p) = std::env::var("MAGURO_CTX_TIMING_LOG") {
            let p = p.trim();
            if !p.is_empty() {
                return Some(std::path::PathBuf::from(p));
            }
        }
        std::env::current_exe().ok().and_then(|exe| exe.parent().map(|d| d.join("maguro-ctx-timing.log")))
    })
    .clone()
}

fn timing() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("MAGURO_CTX_TIMING") {
        Ok(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        Err(_) => false,
    })
}

fn dump_menu() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("MAGURO_CTX_DUMP") {
        Ok(v) => v == "1" || v.eq_ignore_ascii_case("true"),
        Err(_) => false,
    })
}

/// One line per visible top-level item for before/after item-set checks.
pub(crate) unsafe fn dump_menu_items(tag: &str, menu: HMENU) {
    if !dump_menu() && !timing() {
        return;
    }
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    log_line(&format!("[maguro] ctx menu-dump {tag} count={n}"));
    for i in 0..n {
        if super::menu_sep(menu, i) {
            log_line(&format!("[maguro] ctx menu-dump {tag} [{i}] ---"));
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        let text = item_text(menu, i).replace('\n', " ").replace('\r', "");
        log_line(&format!("[maguro] ctx menu-dump {tag} [{i}] id={id} {text}"));
    }
}

#[allow(dead_code)] // Kept so the filtered-key builder still compiles. The popup does not call it.
pub(crate) unsafe fn compose(owner: HWND, folder: &IShellFolder, children: &[*const ITEMIDLIST], targets: u8, paths: &[String]) -> Option<IContextMenu> {
    let t0 = Instant::now();
    let folder_pidl = SHGetIDListFromObject(folder).unwrap_or(std::ptr::null_mut());
    let mut kids = Vec::new();
    for c in children {
        if c.is_null() {
            continue;
        }
        let d = ILClone(*c as *mut ITEMIDLIST);
        if !d.is_null() {
            kids.push(d);
        }
    }
    let refs: Vec<*const ITEMIDLIST> = kids.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let data = if refs.is_empty() { None } else { folder.GetUIObjectOf::<IDataObject>(owner, &refs, None).ok() };
    let classes = classes_for(targets, paths);
    let host = CtxHost {
        folder: folder.clone(),
        folder_pidl,
        children: kids,
        data,
        owner,
        targets,
        paths: paths.to_vec(),
        classes,
        base: RefCell::new(None),
        parts: RefCell::new(Vec::new()),
        verbs: RefCell::new(Vec::new()),
        keys: RefCell::new(Vec::new()),
    };
    log_phase("compose", t0);
    Some(host.into())
}

/// Same parent folder: one filtered menu. Mixed parents fall back to the shell menu.
#[allow(dead_code)] // See `compose`.
pub(crate) unsafe fn compose_paths(owner: HWND, items: &[(String, bool)], targets: u8) -> Option<IContextMenu> {
    let parent = crate::parent_of(&items.first()?.0)?;
    if !items.iter().all(|(p, _)| crate::parent_of(p).is_some_and(|q| q.eq_ignore_ascii_case(&parent))) {
        return None;
    }
    let mut folder_pidl = std::ptr::null_mut();
    SHParseDisplayName(&HSTRING::from(&parent), None, &mut folder_pidl, 0, None).ok()?;
    if folder_pidl.is_null() {
        return None;
    }
    let desktop = SHGetDesktopFolder().ok();
    let folder: Option<IShellFolder> = desktop.as_ref().and_then(|d| d.BindToObject(folder_pidl, None).ok());
    CoTaskMemFree(Some(folder_pidl as _));
    let folder = folder?;
    let mut kids: Vec<*mut ITEMIDLIST> = Vec::new();
    for (path, _) in items {
        let name = path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(path);
        if name.is_empty() {
            continue;
        }
        let mut child: *mut ITEMIDLIST = std::ptr::null_mut();
        let wide = HSTRING::from(name);
        if folder.ParseDisplayName(HWND::default(), None, &wide, None, &mut child, std::ptr::null_mut()).is_ok() && !child.is_null() {
            kids.push(child);
        }
    }
    if kids.len() != items.len() {
        for p in &kids {
            CoTaskMemFree(Some(*p as _));
        }
        return None;
    }
    let refs: Vec<*const ITEMIDLIST> = kids.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let paths: Vec<String> = items.iter().map(|(p, _)| p.clone()).collect();
    let menu = compose(owner, &folder, &refs, targets, &paths);
    for p in kids {
        CoTaskMemFree(Some(p as _));
    }
    menu
}

pub(crate) unsafe fn strip_disabled(menu: HMENU, cm: &IContextMenu, targets: u8) {
    let (off, on) = app(|a| (a.s.ctx_off.clone(), a.s.ctx_on.clone()));
    let cat = catalogue();
    let mut verbs = Vec::new();
    let mut names = Vec::new();
    let mut marks: Vec<HandlerMarks> = Vec::new();
    for r in &cat {
        if !blocked(r, targets, &off, &on) {
            continue;
        }
        let mut mv = Vec::new();
        let mut mn = Vec::new();
        // The registry verb is language-independent. MuiVerb is the text this UI language shows.
        if !r.verb.is_empty() {
            mv.push(r.verb.to_ascii_lowercase());
            push_mark(&mut mn, &r.name);
            for label in &r.static_labels {
                push_mark(&mut mn, label);
            }
        }
        // A COM handler's catalogue name is a product or class name, not the menu text.
        // Ask it what it inserts, then delete those strings. DelegateExecute verbs already
        // matched on the registry verb above.
        if r.clsid.is_some() && r.verb.is_empty() {
            if let Some(items) = probe_marks(r, targets) {
                for item in items {
                    if !item.verb.is_empty() && !mv.iter().any(|v| v == &item.verb) {
                        mv.push(item.verb);
                    }
                    if !item.folded.is_empty() && !mn.iter().any(|n| n == &item.folded) {
                        mn.push(item.folded);
                    }
                }
            }
        }
        if mv.is_empty() && mn.is_empty() {
            continue;
        }
        verbs.extend(mv.iter().cloned());
        names.extend(mn.iter().cloned());
        marks.push(HandlerMarks { id: r.id.clone(), verbs: mv, names: mn });
    }
    let n = GetMenuItemCount(Some(menu));
    if n <= 0 {
        return;
    }
    let mut learned: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for i in (0..n as u32).rev() {
        if super::menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        // Maguro's own commands (batch rename, favourites, WinMerge) use ids at 0x8000 and above.
        if maguro_menu_id(id) {
            continue;
        }
        let raw = item_text(menu, i);
        let verb = if id != u32::MAX && id >= 1 && id < 0x8000 { super::cmd_verb(cm, (id - 1) as usize) } else { String::new() };
        if !item_dropped(&verb, &raw, &verbs, &names) {
            continue;
        }
        let verb_l = verb.trim().to_ascii_lowercase();
        let key = menu_match_key(&raw);
        let label = menu_label(&raw);
        if !label.is_empty() {
            for m in &marks {
                let hit_verb = !verb_l.is_empty() && m.verbs.iter().any(|v| v == &verb_l);
                let hit_name = !key.is_empty() && m.names.iter().any(|n| n == &key);
                if hit_verb || hit_name {
                    learned.entry(m.id.clone()).or_default().push(label.clone());
                }
            }
        }
        _ = DeleteMenu(menu, i, MF_BYPOSITION);
    }
    // Deleting a handler's rows leaves the separators that used to sit around them.
    normalize_menu(menu);
    for (id, labels) in learned {
        note_labels(&id, &labels);
    }
}

struct HandlerMarks {
    id: String,
    verbs: Vec<String>,
    names: Vec<String>,
}

fn push_mark(names: &mut Vec<String>, text: &str) {
    let key = menu_match_key(text);
    if key.is_empty() || names.iter().any(|n| n == &key) {
        return;
    }
    names.push(key);
}

/// Maguro appends its own commands at 0x8000 and above. The shell's range stops at 0x7FFF.
fn maguro_menu_id(id: u32) -> bool {
    id != u32::MAX && id >= 0x8000
}

/// A shell row belongs to a disabled handler. Builtin verbs stay. Comparison uses the verb
/// or the strings the handler inserted, after accelerators and ellipsis are folded.
fn item_dropped(verb: &str, text: &str, verbs: &[String], names: &[String]) -> bool {
    let verb_l = verb.trim().to_ascii_lowercase();
    if !verb_l.is_empty() && (builtin_verb(&verb_l) || shell_owned(&verb_l)) {
        return false;
    }
    if !verb_l.is_empty() && verbs.iter().any(|v| v == &verb_l) {
        return true;
    }
    let key = menu_match_key(text);
    !key.is_empty() && names.iter().any(|n| n == &key)
}

#[derive(Clone)]
struct ProbeItem {
    verb: String,
    /// `menu_match_key` of the raw item text. Deletion compares this, not the cleaned label.
    folded: String,
    /// What the settings page shows: accelerators and a trailing shortcut suffix removed.
    label: String,
}

/// Strings a disabled COM handler inserts. An empty menu is remembered as quiet.
/// Labels for the settings row are taken from the real menu, once a row is actually removed.
/// Process-lifetime cache: disabled COM handlers are CoCreated once per (id, targets).
static PROBE_MARK_CACHE: Mutex<Option<HashMap<(String, u8), Vec<ProbeItem>>>> = Mutex::new(None);

unsafe fn probe_marks(row: &Row, targets: u8) -> Option<Vec<ProbeItem>> {
    let key = (row.id.clone(), targets);
    if let Ok(cache) = PROBE_MARK_CACHE.lock() {
        if let Some(map) = cache.as_ref() {
            if let Some(hit) = map.get(&key) {
                log_line(&format!("[maguro] ctx probe-cache hit {}", row.id));
                return Some(hit.clone());
            }
        }
    }
    let items = probe_items(row, targets)?;
    if items.iter().all(|i| i.verb.is_empty() && i.folded.is_empty()) {
        note_quiet(&row.id);
    }
    if let Ok(mut cache) = PROBE_MARK_CACHE.lock() {
        cache.get_or_insert_with(HashMap::new).insert(key, items.clone());
    }
    Some(items)
}

/// How the shell might have called `IShellExtInit::Initialize` for this click.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ProbeContract {
    /// Selected items only. The parent folder is not also a selected path.
    Selection,
    /// Parent folder and the selection together.
    FolderAndData,
    /// Empty space. The folder itself, and no data object.
    Folder,
}

/// A selection is asked without the parent folder, then with it. WinMerge appends a
/// resolvable folder pidl to the dropped paths, so the folder-and-data call inserts
/// one "Compare" item where the real one-item menu inserted "Select Left" and "Compare...".
fn probe_contracts(has_folder: bool, has_data: bool) -> Vec<ProbeContract> {
    let mut out = Vec::new();
    if has_data {
        out.push(ProbeContract::Selection);
    }
    if has_folder && has_data {
        out.push(ProbeContract::FolderAndData);
    } else if has_folder {
        out.push(ProbeContract::Folder);
    }
    out
}

/// Top-level strings a COM handler adds. Each init contract is a fresh object. Nothing is shown.
unsafe fn probe_items(row: &Row, targets: u8) -> Option<Vec<ProbeItem>> {
    let guid = row.clsid?;
    let (folder_pidl, data) = probe_context()?;
    let contracts = probe_contracts(!folder_pidl.is_null(), data.is_some());
    if contracts.is_empty() {
        return None;
    }
    let mut items = Vec::new();
    let mut any_ok = false;
    for contract in contracts {
        let folder = match contract {
            ProbeContract::Selection => std::ptr::null(),
            ProbeContract::Folder | ProbeContract::FolderAndData => folder_pidl,
        };
        let use_data = matches!(contract, ProbeContract::Selection | ProbeContract::FolderAndData);
        let Some(found) = probe_contract(row, &guid, folder, if use_data { data.as_ref() } else { None }, targets) else {
            continue;
        };
        any_ok = true;
        for item in found {
            if items.iter().any(|e: &ProbeItem| e.verb == item.verb && e.folded == item.folded) {
                continue;
            }
            items.push(item);
        }
    }
    if any_ok { Some(items) } else { None }
}

unsafe fn probe_contract(row: &Row, guid: &GUID, folder_pidl: *const ITEMIDLIST, data: Option<&IDataObject>, targets: u8) -> Option<Vec<ProbeItem>> {
    let t0 = Instant::now();
    let cm = cocreate(guid, &row.anchor, folder_pidl, data)?;
    let menu = CreatePopupMenu().ok()?;
    let mut flags = CMF_EXPLORE | CMF_CANRENAME;
    if targets & BACKGROUND == 0 {
        flags |= CMF_ITEMMENU;
    }
    if GetKeyState(VK_SHIFT.0 as i32) < 0 {
        flags |= CMF_EXTENDEDVERBS;
    }
    let hr = cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags);
    let dll = clsid_server_dll(guid);
    let label = if dll.is_empty() { row.id.clone() } else { dll };
    log_line(&format!("[maguro] ctx probe {} {} ms", label, t0.elapsed().as_millis()));
    if hr.is_err() {
        _ = DestroyMenu(menu);
        return None;
    }
    let mut items = Vec::new();
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    // Top level only. Deleting that row on the real menu removes the handler's submenu with it.
    // A child named like a shell command ("Open") must not be collected.
    for i in 0..n {
        if super::menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        let verb = if id != u32::MAX && id >= 1 && id < 0x8000 { super::cmd_verb(&cm, (id - 1) as usize) } else { String::new() };
        let verb_l = verb.trim().to_ascii_lowercase();
        if !verb_l.is_empty() && (builtin_verb(&verb_l) || shell_owned(&verb_l)) {
            continue;
        }
        let raw = item_text(menu, i);
        let folded = menu_match_key(&raw);
        let label = menu_label(&raw);
        if verb_l.is_empty() && folded.is_empty() && label.is_empty() {
            continue;
        }
        items.push(ProbeItem { verb: verb_l, folded, label });
    }
    _ = DestroyMenu(menu);
    Some(items)
}

/// Read menu strings off the shell menu Maguro just built.
/// The menu itself is left as the shell built it. Handlers we have not named yet are
/// asked by `learn_pending` after the popup closes, so that probe is not part of the wait.
pub(crate) unsafe fn observe_menu(menu: HMENU, cm: &IContextMenu, targets: u8) {
    if targets == 0 {
        return;
    }
    let t0 = Instant::now();
    let cat = catalogue();
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    for i in 0..n {
        if super::menu_sep(menu, i) {
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        if id == u32::MAX || id < 1 || id >= 0x8000 {
            continue;
        }
        let verb = super::cmd_verb(cm, (id - 1) as usize);
        let verb_l = verb.trim().to_ascii_lowercase();
        if verb_l.is_empty() || builtin_verb(&verb_l) || shell_owned(&verb_l) {
            continue;
        }
        let text = menu_label(&item_text(menu, i));
        if text.is_empty() {
            continue;
        }
        if let Some(row) = cat.iter().find(|r| !r.builtin && !r.verb.is_empty() && r.verb.eq_ignore_ascii_case(&verb_l)) {
            note_labels(&row.id, std::slice::from_ref(&text));
        }
    }
    log_phase("labels", t0);
}

/// Enabled COM handlers whose menu text is not cached yet. Called after the popup closes.
/// A panic in a handler is contained; the menu the user already saw is not involved.
pub(crate) unsafe fn learn_pending(targets: u8) {
    if targets == 0 {
        return;
    }
    let _ = crate::crash::guard(std::panic::AssertUnwindSafe(|| unsafe { learn_pending_inner(targets) }));
}

unsafe fn learn_pending_inner(targets: u8) {
    let t0 = Instant::now();
    let cat = catalogue();
    learn_unknown_com(&cat, targets);
    log_phase("learn", t0);
}

/// Enabled COM handlers whose menu text is not cached yet. Disabled ones are asked by `probe_marks`.
unsafe fn learn_unknown_com(cat: &[Row], targets: u8) {
    if probe_context().is_none() {
        return;
    }
    let (off, on, seen, quiet) = app(|a| {
        let seen: HashSet<String> = a.s.ctx_labels.keys().cloned().collect();
        (a.s.ctx_off.clone(), a.s.ctx_on.clone(), seen, a.s.ctx_quiet.clone())
    });
    let mut order: Vec<&Row> = Vec::new();
    let mut later: Vec<&Row> = Vec::new();
    for quiet_pass in [false, true] {
        for row in cat {
            if row.builtin || row.clsid.is_none() || !row.verb.is_empty() || row.targets & targets == 0 {
                continue;
            }
            if seen.contains(&row.id) || missed_label(&row.id, targets) || blocked(row, targets, &off, &on) {
                continue;
            }
            if quiet.contains(&row.id) != quiet_pass {
                continue;
            }
            // Generic Windows and screen-saver handlers rarely insert a row. Ask them after
            // products the settings list is still waiting to name, so they do not use the budget first.
            if shell_dormant(row) {
                later.push(row);
            } else {
                order.push(row);
            }
        }
    }
    order.extend(later);
    let mut tried = 0u32;
    for row in order {
        if tried >= 8 {
            break;
        }
        tried += 1;
        match probe_items(row, targets) {
            Some(items) => {
                let labels: Vec<String> = items.into_iter().map(|i| i.label).filter(|s| !s.is_empty()).collect();
                if labels.is_empty() {
                    mark_missed(&row.id, targets);
                    note_quiet(&row.id);
                } else {
                    note_labels(&row.id, &labels);
                }
            }
            None => {
                mark_missed(&row.id, targets);
                if missed_label(&row.id, row.targets) {
                    note_quiet(&row.id);
                }
            }
        }
    }
}

/// Empty or failed probes, per handler and click kind. A few tries cover a different selection; then we stop asking.
static MISSED: Mutex<Vec<(String, u8, u8)>> = Mutex::new(Vec::new());

/// Every bit of `targets` has been asked often enough and never added an item.
fn missed_label(id: &str, targets: u8) -> bool {
    if targets == 0 {
        return false;
    }
    let g = MISSED.lock().unwrap_or_else(|e| e.into_inner());
    [FILE, FOLDER, BACKGROUND, DRIVE].into_iter().filter(|bit| targets & bit != 0).all(|bit| g.iter().any(|(i, t, n)| i == id && *t == bit && *n >= 3))
}

fn mark_missed(id: &str, targets: u8) {
    let mut g = MISSED.lock().unwrap_or_else(|e| e.into_inner());
    for bit in [FILE, FOLDER, BACKGROUND, DRIVE] {
        if targets & bit == 0 {
            continue;
        }
        if let Some((_, _, n)) = g.iter_mut().find(|(i, t, _)| i == id && *t == bit) {
            *n = n.saturating_add(1);
        } else {
            g.push((id.to_string(), bit, 1));
        }
    }
}

/// The handler ran (or was given up on) and added no visible item. The shell menu is not changed.
fn note_quiet(id: &str) {
    if id.is_empty() || id == APP_WINMERGE {
        return;
    }
    let changed = app(|a| {
        if a.s.ctx_labels.get(id).is_some_and(|v| !v.is_empty()) {
            return false;
        }
        a.s.ctx_quiet.insert(id.to_string())
    });
    if changed {
        unsafe {
            crate::note_save();
            crate::settings::refresh_ctx_labels();
        }
    }
}

/// Remember cleaned menu strings for a handler. Empty input does not count as "seen".
fn note_labels(id: &str, incoming: &[String]) {
    if id.is_empty() || incoming.is_empty() {
        return;
    }
    let changed = app(|a| {
        let slot = a.s.ctx_labels.entry(id.to_string()).or_default();
        let added = merge_labels(slot, incoming);
        let revealed = if slot.is_empty() { false } else { a.s.ctx_quiet.remove(id) };
        added || revealed
    });
    if changed {
        unsafe {
            crate::note_save();
            crate::settings::refresh_ctx_labels();
        }
    }
}

/// Cleaned menu strings for `ctx_labels`. Already-saved accelerator tails and repeated labels collapse here.
pub(crate) fn cleaned_labels(incoming: &[String]) -> Vec<String> {
    let mut slot = Vec::new();
    merge_labels(&mut slot, incoming);
    slot
}

fn merge_labels(slot: &mut Vec<String>, incoming: &[String]) -> bool {
    let old = std::mem::take(slot);
    let mut cleaned: Vec<String> = Vec::new();
    for raw in old.iter().chain(incoming.iter()) {
        let label: String = menu_label(raw);
        if label.is_empty() || label.chars().count() > 80 {
            continue;
        }
        if cleaned.iter().any(|e| fold_menu(e) == fold_menu(&label)) {
            continue;
        }
        if cleaned.len() >= 8 {
            break;
        }
        cleaned.push(label);
    }
    let changed = cleaned != old;
    *slot = cleaned;
    changed
}

fn probe_context() -> Option<(*mut ITEMIDLIST, Option<IDataObject>)> {
    PROBE.with(|p| {
        let borrowed = p.borrow();
        let ctx = borrowed.as_ref()?;
        Some((ctx.folder_pidl, ctx.data.clone()))
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SepMark {
    Item,
    Separator,
}

/// A menu with no separator at either end, and no two separators in a row.
fn normalize_separators(items: &[SepMark]) -> Vec<SepMark> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if *item == SepMark::Separator && (out.is_empty() || *out.last().unwrap() == SepMark::Separator) {
            continue;
        }
        out.push(*item);
    }
    if out.last() == Some(&SepMark::Separator) {
        out.pop();
    }
    out
}

/// Which original rows `normalize_separators` keeps. The first separator of a run stays.
fn separator_keep(is_sep: &[bool]) -> Vec<bool> {
    let marks: Vec<SepMark> = is_sep.iter().map(|sep| if *sep { SepMark::Separator } else { SepMark::Item }).collect();
    let want = normalize_separators(&marks);
    let mut keep = vec![false; is_sep.len()];
    let mut j = 0;
    for (i, mark) in marks.iter().enumerate() {
        if j < want.len() && *mark == want[j] {
            keep[i] = true;
            j += 1;
        }
    }
    debug_assert_eq!(j, want.len());
    keep
}

unsafe fn item_submenu(menu: HMENU, pos: u32) -> Option<HMENU> {
    let mut info = MENUITEMINFOW {
        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_SUBMENU,
        ..Default::default()
    };
    if GetMenuItemInfoW(menu, pos, true, &mut info).is_err() {
        return None;
    }
    let sub = info.hSubMenu;
    if sub.0.is_null() || sub.0 == menu.0 { None } else { Some(sub) }
}

/// Drop leading, trailing, and repeated separators. Submenus are cleaned the same way.
pub(crate) unsafe fn normalize_menu(menu: HMENU) {
    let mut seen = Vec::new();
    normalize_menu_into(menu, &mut seen, 0);
}

unsafe fn normalize_menu_into(menu: HMENU, seen: &mut Vec<isize>, depth: u8) {
    if menu.0.is_null() || depth > 8 {
        return;
    }
    let key = menu.0 as isize;
    if seen.contains(&key) {
        return;
    }
    seen.push(key);
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    for i in 0..n {
        if let Some(sub) = item_submenu(menu, i) {
            normalize_menu_into(sub, seen, depth + 1);
        }
    }
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    let sep: Vec<bool> = (0..n).map(|i| super::menu_sep(menu, i)).collect();
    let keep = separator_keep(&sep);
    for i in (0..n).rev() {
        if !keep[i as usize] {
            let _ = DeleteMenu(menu, i, MF_BYPOSITION);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NewFolderSlot {
    Other,
    /// Top-level "New folder". `maguro` is a command id at `0x8000` or above.
    NewFolder { maguro: bool },
}

/// Explorer's row wins. A second Explorer row, and Maguro's own row beside one, go.
fn keep_new_folder(rows: &[NewFolderSlot]) -> Vec<bool> {
    let explorer = rows.iter().any(|row| matches!(row, NewFolderSlot::NewFolder { maguro: false }));
    let mut seen_explorer = false;
    let mut seen_own = false;
    rows.iter()
        .map(|row| match row {
            NewFolderSlot::Other => true,
            NewFolderSlot::NewFolder { maguro: false } => {
                if seen_explorer {
                    false
                } else {
                    seen_explorer = true;
                    true
                }
            }
            NewFolderSlot::NewFolder { maguro: true } => {
                if explorer || seen_own {
                    false
                } else {
                    seen_own = true;
                    true
                }
            }
        })
        .collect()
}

fn is_new_folder_verb(verb: &str) -> bool {
    let v = verb.trim().trim_matches('&').to_ascii_lowercase();
    v == "newfolder" || v == "windows.newfolder"
}

/// Shell text (`새 폴더(&N)`, `新しいフォルダー(&F)`) and Maguro's `tr()` strings.
fn is_new_folder_label(text: &str) -> bool {
    matches!(
        menu_match_key(text).as_str(),
        "new folder" | "새 폴더" | "新しいフォルダー" | "新しいフォルダ" | "新規フォルダー" | "新規フォルダ"
    )
}

fn is_new_folder_item(verb: &str, text: &str) -> bool {
    is_new_folder_verb(verb) || is_new_folder_label(text)
}

/// One top-level New folder on a background menu.
///
/// Explorer can insert that command twice. `Position=Top` puts it first, and the
/// same verb is merged again under Paste from `Directory\Background` and
/// `Folder\Background`. Both rows share the shell string (`새 폴더(&N)` on a
/// Korean UI, `新しいフォルダー(&F)` on a Japanese one). Maguro does not add
/// a row of its own. A command id at `0x8000` or above with that label is removed
/// when Explorer already has the verb, so the two are never stacked.
///
/// Rows Maguro does add (favourites, batch rename, WinMerge, the Properties
/// fallback) keep `tr()`, the app UI language. Explorer's rows stay in the
/// Windows display language. This pass does not relabel either kind.
pub(crate) unsafe fn dedupe_new_folder(menu: HMENU, cm: Option<&IContextMenu>) {
    let n = GetMenuItemCount(Some(menu)).max(0) as u32;
    let mut rows = Vec::with_capacity(n as usize);
    for i in 0..n {
        if super::menu_sep(menu, i) || item_submenu(menu, i).is_some() {
            rows.push(NewFolderSlot::Other);
            continue;
        }
        let id = GetMenuItemID(menu, i as i32);
        let maguro = maguro_menu_id(id);
        let verb = match cm {
            Some(cm) if id != u32::MAX && (1..0x8000).contains(&id) => super::cmd_verb(cm, (id - 1) as usize),
            _ => String::new(),
        };
        let text = item_text(menu, i);
        if is_new_folder_item(&verb, &text) {
            rows.push(NewFolderSlot::NewFolder { maguro });
        } else {
            rows.push(NewFolderSlot::Other);
        }
    }
    let keep = keep_new_folder(&rows);
    for i in (0..n).rev() {
        if !keep[i as usize] {
            let _ = DeleteMenu(menu, i, MF_BYPOSITION);
        }
    }
}

fn item_text(menu: HMENU, pos: u32) -> String {
    let mut buf = [0u16; 256];
    let mut info = MENUITEMINFOW {
        cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        dwTypeData: PWSTR(buf.as_mut_ptr()),
        cch: buf.len() as u32,
        ..Default::default()
    };
    unsafe {
        if GetMenuItemInfoW(menu, pos, true, &mut info).is_err() {
            return String::new();
        }
    }
    let n = info.cch as usize;
    let n = n.min(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string()
}

/// Menu text with accelerators removed: `&Browse` → `Browse`, `Fish && Chips` → `Fish & Chips`.
fn strip_accel(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                out.push('&');
            }
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

fn clean_label(s: &str) -> String {
    let t = if s.starts_with('@') { indirect(s) } else { s.to_string() };
    strip_accel(&t)
}

/// `AABdzCtx Class`, `BdzShell`, `HVShell Module`: a COM identifier, not a product name.
fn identifier_like(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || parse_guid(t).is_some() {
        return true;
    }
    let low = t.to_ascii_lowercase();
    if low.ends_with(".dll") || low.ends_with(".exe") {
        return true;
    }
    if low.ends_with(" class") || low.ends_with(" context menu") || low.ends_with(" module") {
        return true;
    }
    if t.contains([' ', '\t']) {
        return false;
    }
    let shellish = low.contains("shell") || low.contains("ctx") || low.contains("context") || low.ends_with("ext") || low.ends_with("ex");
    let mixed = t.chars().any(|c| c.is_ascii_uppercase()) && t.chars().any(|c| c.is_ascii_lowercase());
    shellish && mixed
}

/// First candidate that reads as a product name. Class identifiers are kept only when nothing else exists.
fn pick_label(candidates: &[&str]) -> String {
    let mut cleaned = Vec::new();
    for s in candidates {
        let t = clean_label(s);
        if t.is_empty() || parse_guid(&t).is_some() || title_junk(&t) {
            continue;
        }
        cleaned.push(t);
    }
    if let Some(s) = cleaned.iter().find(|s| !identifier_like(s)) {
        return s.clone();
    }
    cleaned.into_iter().next().unwrap_or_default()
}

/// ProductName, then FileDescription, then a known vendor. A menu sentence is never the title.
fn friendly_name(product: &str, desc: &str, fallbacks: &[&str]) -> String {
    for s in [product, desc] {
        if let Some(name) = mapped_title(s) {
            return name;
        }
    }
    for s in fallbacks {
        if let Some(name) = known_product(s) {
            return name;
        }
    }
    let picked = polish_name(&pick_label(fallbacks));
    if let Some(name) = known_product(&picked) {
        return name;
    }
    if product_like(&picked) && !menu_sentence(&picked) && !title_junk(&picked) { picked } else { String::new() }
}

fn mapped_title(s: &str) -> Option<String> {
    if title_junk(s) {
        return None;
    }
    let t = polish_name(s);
    if title_junk(&t) {
        return None;
    }
    if let Some(name) = known_product(&t).or_else(|| known_product(s)) {
        return Some(name);
    }
    if product_like(&t) && !menu_sentence(&t) { Some(t) } else { None }
}

/// `%1`, `%*`, `%V`, `%L`, a path, or a command line. None of these is a product name.
fn title_junk(s: &str) -> bool {
    let t = s.trim().trim_matches('"').trim();
    if t.is_empty() {
        return false;
    }
    contains_placeholder(t) || looks_like_command(t)
}

fn contains_placeholder(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'%' {
            let n = b[i + 1].to_ascii_lowercase();
            if n == b'*' || b[i + 1].is_ascii_digit() || matches!(n, b'd' | b'i' | b'l' | b'u' | b'v' | b'w') {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn looks_like_command(s: &str) -> bool {
    if s.contains(['\\', '/']) {
        return true;
    }
    let b = s.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return true;
    }
    s.split_whitespace().any(|w| {
        let low = w.trim_matches('"').to_ascii_lowercase();
        [".exe", ".dll", ".ocx", ".cpl", ".bat", ".cmd", ".com"].iter().any(|ext| low.ends_with(ext))
    })
}

fn product_like(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.chars().count() >= 2 && !identifier_like(t)
}

/// Drop a generic suffix, a possessive, and a trailing `(V)` / `(x64)` tag.
fn polish_name(s: &str) -> String {
    let mut t = normalize_marks(&clean_label(s));
    loop {
        let next = strip_name_suffix(&t);
        let next = strip_possessive(&next);
        let next = strip_name_tag(&next);
        let next = normalize_marks(&next);
        if next == t {
            break;
        }
        t = next;
    }
    t
}

fn strip_name_suffix(s: &str) -> String {
    let t = s.trim();
    let low = t.to_ascii_lowercase();
    const SUFFIXES: &[&str] = &[
        "shell menu dll",
        "explorer extension library",
        "shell integration library",
        "shell extension",
        "context menu",
        "shell ext",
    ];
    for suf in SUFFIXES {
        if low == *suf {
            return String::new();
        }
        let spaced = format!(" {suf}");
        if let Some(stem) = low.strip_suffix(&spaced) {
            return t.get(..stem.len()).unwrap_or(t).trim().to_string();
        }
    }
    t.to_string()
}

fn strip_possessive(s: &str) -> String {
    let t = s.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() >= 3 {
        let n = chars.len();
        let mark = chars[n - 2];
        let last = chars[n - 1];
        if (last == 's' || last == 'S') && matches!(mark, '\'' | '\u{2019}' | '\u{FF07}') {
            let stem: String = chars[..n - 2].iter().collect();
            let stem = stem.trim();
            if stem.chars().count() >= 2 {
                return stem.to_string();
            }
        }
    }
    t.to_string()
}

fn is_mark(c: char) -> bool {
    matches!(c, '\u{00AE}' | '\u{2122}' | '\u{00A9}')
}

fn normalize_marks(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut raw = String::new();
    for (i, c) in chars.iter().enumerate() {
        if is_mark(*c) {
            let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
            let next_alnum = chars.get(i + 1).is_some_and(|d| d.is_alphanumeric());
            if prev_alnum && next_alnum {
                raw.push(' ');
            }
            continue;
        }
        raw.push(*c);
    }
    collapse_ws(&raw)
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            if !out.is_empty() {
                gap = true;
            }
            continue;
        }
        if gap {
            out.push(' ');
            gap = false;
        }
        out.push(c);
    }
    out
}

/// `Bandizip (V)` or `Bandizip(V)` → `Bandizip`. A short parenthetical tag, not a phrase.
fn strip_name_tag(s: &str) -> String {
    let t = s.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() < 3 || !matches!(chars.last().copied(), Some(')') | Some('\u{FF09}')) {
        return t.to_string();
    }
    let Some(open) = chars[..chars.len() - 1].iter().rposition(|c| matches!(c, '(' | '\u{FF08}')) else {
        return t.to_string();
    };
    if open == 0 {
        return t.to_string();
    }
    let inner: String = chars[open + 1..chars.len() - 1].iter().collect();
    let folded: String = inner.trim().chars().map(fold_width_char).collect();
    let low = folded.to_ascii_lowercase();
    let tag = (folded.chars().count() == 1 && folded.chars().all(|c| c.is_ascii_alphabetic())) || matches!(low.as_str(), "x86" | "x64" | "32" | "64");
    if !tag {
        return t.to_string();
    }
    let before: String = chars[..open].iter().collect();
    let before = before.trim();
    if before.is_empty() { t.to_string() } else { before.to_string() }
}

/// Menu text as the user reads it: no accelerator, no file name, no trailing `(X)` or `(Ctrl+C)`.
fn menu_label(s: &str) -> String {
    let t = clean_label(s);
    let t = t.split('\t').next().unwrap_or(&t).trim();
    let t = generalize_filenames(t);
    let t = strip_shortcut_suffix(&t);
    if label_is_placeholder(&t) { String::new() } else { t }
}

/// Identity of a menu row for deletion. Accelerators, a trailing `(L)`, and `...` / `…` fold together.
/// `比較` and `比較...` stay distinct: they are different commands.
fn menu_match_key(s: &str) -> String {
    let t = menu_label(s);
    if t.is_empty() {
        return String::new();
    }
    fold_menu(&t.replace("...", "\u{2026}"))
}

fn strip_shortcut_suffix(s: &str) -> String {
    let mut t = s.trim().to_string();
    while let Some(next) = strip_one_shortcut(&t) {
        t = next;
    }
    t
}

/// A trailing `(B)` / `（Ｖ）` / `(Ctrl+C)`, including the one sitting just before `...` or `…`.
fn strip_one_shortcut(s: &str) -> Option<String> {
    let t = s.trim_end();
    let (core, ellipsis) = split_ellipsis(t);
    let core = core.trim_end();
    let mut chars: Vec<char> = core.chars().collect();
    let close = chars.last().copied()?;
    if close != ')' && close != '\u{FF09}' {
        return None;
    }
    chars.pop();
    let open = chars.iter().rposition(|c| *c == '(' || *c == '\u{FF08}')?;
    if open == 0 {
        return None;
    }
    let inner: String = chars[open + 1..].iter().collect();
    if !shortcut_token(inner.trim()) {
        return None;
    }
    let before: String = chars[..open].iter().collect();
    let before = before.trim_end();
    if before.is_empty() {
        return None;
    }
    let mut out = before.to_string();
    out.push_str(ellipsis);
    if out == t { None } else { Some(out) }
}

fn split_ellipsis(s: &str) -> (&str, &str) {
    if let Some(rest) = s.strip_suffix('\u{2026}') {
        (rest, "\u{2026}")
    } else if let Some(rest) = s.strip_suffix("...") {
        (rest, "...")
    } else {
        (s, "")
    }
}

fn shortcut_token(s: &str) -> bool {
    let t: String = s.trim().chars().map(fold_width_char).collect();
    if t.chars().count() == 1 && t.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) {
        return true;
    }
    let low = t.to_ascii_lowercase().replace(' ', "");
    let modified = ["ctrl", "control", "shift", "alt", "win"].iter().any(|m| low.starts_with(m)) && low.contains('+');
    modified && low.chars().all(|c| c.is_ascii_alphanumeric() || c == '+') && low.len() <= 24
}

fn fold_width_char(c: char) -> char {
    match c as u32 {
        0xFF10..=0xFF19 => char::from_u32(c as u32 - 0xFF10 + u32::from(b'0')).unwrap_or(c),
        0xFF21..=0xFF3A => char::from_u32(c as u32 - 0xFF21 + u32::from(b'A')).unwrap_or(c),
        0xFF41..=0xFF5A => char::from_u32(c as u32 - 0xFF41 + u32::from(b'a')).unwrap_or(c),
        _ => c,
    }
}

/// `"a.zip" に圧縮` and `"b.zip" に圧縮` become the same `"…" に圧縮`.
fn generalize_filenames(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = file_quote_end(&chars, i) {
            out.push(chars[i]);
            out.push('\u{2026}');
            out.push(chars[end]);
            i = end + 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn file_quote_end(chars: &[char], i: usize) -> Option<usize> {
    let close = match chars.get(i).copied()? {
        '"' => '"',
        '\u{FF02}' => '\u{FF02}',
        '「' => '」',
        '『' => '』',
        '\u{201C}' => '\u{201D}',
        '\'' => '\'',
        _ => return None,
    };
    let end = chars[i + 1..].iter().position(|c| *c == close)? + i + 1;
    let inner: String = chars[i + 1..end].iter().collect();
    let n = inner.chars().count();
    if (1..=120).contains(&n) && inner.contains(['.', '\\', '/']) { Some(end) } else { None }
}

fn label_is_placeholder(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t.chars().all(|c| c.is_whitespace() || matches!(c, '"' | '\u{FF02}' | '「' | '」' | '『' | '』' | '\u{201C}' | '\u{201D}' | '\'' | '\u{2019}' | '\u{2026}' | '.'))
}

/// Menu text and bare verbs. These stay off the product title.
fn menu_sentence(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    if strip_one_shortcut(t).is_some() || t.ends_with("...") || t.ends_with('\u{2026}') {
        return true;
    }
    let low = fold_menu(t);
    const EXACT: &[&str] = &[
        "configure", "open", "print", "edit", "explore", "find", "run", "delete", "rename", "properties", "share", "copy", "paste",
    ];
    if EXACT.contains(&low.as_str()) {
        return true;
    }
    if low.starts_with("open in ") || low.starts_with("open with ") || low.starts_with("print ") || low.starts_with("add to ") || low.starts_with("configure ") {
        return true;
    }
    t.contains("で開く") || t.contains("を開く") || t.contains("인쇄")
}

fn norm_component(s: &str) -> String {
    let mut t = strip_name_tag(&clean_label(s)).to_ascii_lowercase();
    for suf in [" context menu", " contextmenu", " class", " module", " shell extension", " shellextension"] {
        if let Some(rest) = t.strip_suffix(suf) {
            t = rest.trim().to_string();
            break;
        }
    }
    t.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Raw component names that VERSIONINFO does not already explain.
struct Tokens {
    /// Non-alphanumeric splits, plus camel-case pieces (`OpenInExcel` → excel).
    all: Vec<String>,
    /// Splits on punctuation and spaces only. `WordPad` stays one word.
    plain: Vec<String>,
}

fn tokens_of(s: &str) -> Tokens {
    let mut all = Vec::new();
    let mut plain = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, all: &mut Vec<String>, plain: &mut Vec<String>| {
        if cur.is_empty() {
            return;
        }
        let word: String = cur.chars().map(fold_width_char).collect();
        cur.clear();
        let low = word.to_ascii_lowercase();
        push_token(plain, &low);
        push_token(all, &low);
        let chars: Vec<char> = low.chars().collect();
        let mut part = String::new();
        for (i, c) in word.chars().enumerate() {
            let prev = i.checked_sub(1).and_then(|j| word.chars().nth(j));
            if prev.is_some_and(|p| p.is_ascii_lowercase() && c.is_ascii_uppercase()) && !part.is_empty() {
                push_token(all, &part);
                part.clear();
            }
            part.push(chars.get(i).copied().unwrap_or(c).to_ascii_lowercase());
        }
        if !part.is_empty() {
            push_token(all, &part);
        }
    };
    for c in s.chars() {
        if c.is_alphanumeric() {
            cur.push(c);
        } else {
            flush(&mut cur, &mut all, &mut plain);
        }
    }
    flush(&mut cur, &mut all, &mut plain);
    Tokens { all, plain }
}

fn push_token(out: &mut Vec<String>, w: &str) {
    if !w.is_empty() && !out.iter().any(|e| e == w) {
        out.push(w.to_string());
    }
}

fn has_word(words: &[String], key: &str) -> bool {
    words.iter().any(|w| w == key)
}

fn known_product(s: &str) -> Option<String> {
    let tokens = tokens_of(s);
    if tokens.all.is_empty() && !s.contains("한컴") {
        return None;
    }
    if has_word(&tokens.all, "hwp") || s.contains("한컴") {
        return Some("한글(Hwp)".to_string());
    }
    const OFFICE: &[&str] = &["excel", "winword", "powerpnt", "powerpoint", "onenote", "msaccess", "mspub", "outlook", "msword"];
    let office_word = tokens.all.iter().any(|w| OFFICE.contains(&w.as_str())) || has_word(&tokens.plain, "word");
    let microsoft_office = has_word(&tokens.plain, "microsoft") && (has_word(&tokens.plain, "office") || office_word);
    if office_word || microsoft_office {
        return Some("Microsoft Office".to_string());
    }
    let script_host = has_word(&tokens.all, "wscript")
        || has_word(&tokens.all, "cscript")
        || (has_word(&tokens.plain, "windows") && has_word(&tokens.plain, "script") && has_word(&tokens.plain, "host"));
    if script_host {
        return Some(tr(crate::Key::WindowsScriptHost).to_string());
    }
    let n = norm_component(s);
    const VENDORS: &[(&[&str], crate::Key)] = &[
        (&["bdzshell", "aabdzctx", "bandizip"], crate::Key::VendorBandizip),
        (&["lhshellext", "lockhunter"], crate::Key::VendorLockhunter),
        (&["synofoldercontextmenu", "synologydrive"], crate::Key::VendorSynology),
        (&["hvshell", "hvcontext", "honeyview"], crate::Key::VendorHoneyview),
        (&["tortoisesvn"], crate::Key::VendorTsvn),
        (&["tortoisegit"], crate::Key::VendorTgit),
        (&["7zip", "sevenzip"], crate::Key::Vendor7zip),
        (&["winrar"], crate::Key::VendorWinrar),
        (&["winmerge"], crate::Key::VendorWinmerge),
    ];
    for (keys, label) in VENDORS {
        let word = tokens.all.iter().any(|w| keys.iter().any(|k| w == *k));
        let compact = n.len() >= 4 && keys.iter().any(|k| n == *k || (n.starts_with(k) && n.len() - k.len() <= 3));
        if word || compact {
            return Some(tr(*label).to_string());
        }
    }
    None
}

fn verb_raw(verb_key: &str, stem: &str) -> String {
    let key = component_raw(verb_key, "");
    let stem_raw = component_raw(stem, "");
    let stem_ok = kept_title(&stem_raw) && !menu_sentence(&stem_raw);
    let key_ok = kept_title(&key) && !menu_sentence(&key);
    if !key_ok && stem_ok { stem_raw } else if key_ok { key } else { String::new() }
}

/// Product title for one static verb. Version info, then the ProgID's application, then a readable name.
fn verb_product(product: &str, desc: &str, stem: &str, menu: &str, verb: &str, owner: &str) -> String {
    let stem = if title_junk(stem) { "" } else { stem };
    let mut title = friendly_name(product, desc, &[stem]);
    if title_junk(&title) || menu_sentence(&title) {
        title.clear();
    }
    if title.is_empty() {
        if let Some(mapped) = known_product(menu).or_else(|| known_product(verb)) {
            title = mapped;
        }
    }
    if title.is_empty() {
        let owner = polish_name(owner);
        if kept_title(&owner) && !menu_sentence(&owner) {
            title = known_product(&owner).unwrap_or(owner);
        }
    }
    if title.is_empty() {
        title = readable_fallback(menu, verb);
    }
    if title_junk(&title) { String::new() } else { title }
}

fn readable_fallback(menu: &str, verb: &str) -> String {
    let menu_t = polish_name(menu);
    if kept_title(&menu_t) {
        return known_product(&menu_t).unwrap_or(menu_t);
    }
    let verb_t = polish_name(verb);
    if kept_title(&verb_t) && !menu_sentence(&verb_t) {
        known_product(&verb_t).unwrap_or(verb_t)
    } else {
        String::new()
    }
}

fn component_raw(key_name: &str, fallback: &str) -> String {
    for s in [key_name, fallback] {
        if parse_guid(s).is_some() {
            continue;
        }
        let t = strip_name_tag(&clean_label(s));
        if t.is_empty() || parse_guid(&t).is_some() {
            continue;
        }
        return t;
    }
    String::new()
}

fn file_stem(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let expanded = if path.contains('%') { expand_env(path) } else { path.to_string() };
    let file = expanded.trim().trim_matches('"').rsplit(['\\', '/']).next().unwrap_or("").trim().to_string();
    let low = file.to_ascii_lowercase();
    if let Some(stem) = low.strip_suffix(".dll").or_else(|| low.strip_suffix(".exe")) {
        // `stem` is measured on the lowercased copy. Slice `file` only on a char boundary.
        file.get(..stem.len()).unwrap_or(&file).to_string()
    } else {
        file
    }
}

/// `rundll32.exe dll,Entry` is the DLL. A bare `%1` / `%*` is not a file.
fn version_target(command: &str) -> String {
    if command.trim().is_empty() {
        return String::new();
    }
    let expanded = if command.contains('%') { expand_env(command) } else { command.to_string() };
    let (exe, args) = split_cmd(&expanded);
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(&exe).to_ascii_lowercase();
    if file == "rundll32.exe" || file == "rundll32" {
        let dll = args.trim().trim_matches('"').split(',').next().unwrap_or("").trim().trim_matches('"');
        return if real_module(dll) { dll.to_string() } else { String::new() };
    }
    if real_module(&exe) { exe.trim().trim_matches('"').to_string() } else { String::new() }
}

fn real_module(s: &str) -> bool {
    let t = s.trim().trim_matches('"');
    if t.is_empty() || contains_placeholder(t) {
        return false;
    }
    let low = t.to_ascii_lowercase();
    [".exe", ".dll", ".ocx", ".cpl"].iter().any(|ext| low.ends_with(ext))
}

fn fold_menu(s: &str) -> String {
    strip_accel(s).to_ascii_lowercase()
}

impl IContextMenu_Impl for CtxHost_Impl {
    fn QueryContextMenu(&self, hmenu: HMENU, index_menu: u32, id_first: u32, id_last: u32, flags: u32) -> HRESULT {
        unsafe { self.fill_menu(hmenu, index_menu, id_first, id_last, flags) }
    }
    fn InvokeCommand(&self, pici: *const CMINVOKECOMMANDINFO) -> Result<()> {
        unsafe { self.invoke(pici) }
    }
    fn GetCommandString(&self, idcmd: usize, utype: u32, preserved: *const u32, pszname: PSTR, cchmax: u32) -> Result<()> {
        unsafe { self.command_string(idcmd, utype, preserved, pszname, cchmax) }
    }
}

impl IContextMenu2_Impl for CtxHost_Impl {
    fn HandleMenuMsg(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> Result<()> {
        unsafe { self.forward_msg(umsg, wparam, lparam, std::ptr::null_mut()) }
    }
}

impl IContextMenu3_Impl for CtxHost_Impl {
    fn HandleMenuMsg2(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM, plresult: *mut LRESULT) -> Result<()> {
        unsafe { self.forward_msg(umsg, wparam, lparam, plresult) }
    }
}

impl CtxHost {
    unsafe fn fill_menu(&self, menu: HMENU, index_menu: u32, id_first: u32, id_last: u32, flags: u32) -> HRESULT {
        self.reset_parts();
        let (off, on) = app(|a| (a.s.ctx_off.clone(), a.s.ctx_on.clone()));
        let cat = catalogue();
        let (clean, dirty) = split_classes(&self.classes, self.targets, &cat, &off, &on);
        let mut keys = Vec::new();
        let mut opened = Vec::new();
        for class in clean.iter().take(KEY_CAP) {
            if let Some(k) = open_hkcr(class) {
                keys.push(k);
                opened.push(class.clone());
            }
        }
        let refs: Vec<*const ITEMIDLIST> = self.children.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
        let dcm = DefCtx {
            hwnd: self.owner,
            pcmcb: std::ptr::null_mut(),
            pidl_folder: self.folder_pidl,
            psf: Interface::as_raw(&self.folder),
            cidl: refs.len() as u32,
            _pad0: 0,
            apidl: if refs.is_empty() { std::ptr::null() } else { refs.as_ptr() },
            punk: std::ptr::null_mut(),
            ckeys: keys.len() as u32,
            _pad1: 0,
            akeys: if keys.is_empty() { std::ptr::null() } else { keys.as_ptr() },
        };
        let t_base = Instant::now();
        let mut ppv = std::ptr::null_mut();
        let hr = SHCreateDefaultContextMenu(&dcm, &IContextMenu::IID, &mut ppv);
        if hr.is_err() || ppv.is_null() {
            for k in keys {
                _ = RegCloseKey(k);
            }
            log_phase("base-fail", t_base);
            return E_FAIL;
        }
        let base: IContextMenu = IContextMenu::from_raw(ppv);
        let base_last = id_first.saturating_add(BASE_END).min(id_last);
        let qhr = base.QueryContextMenu(menu, index_menu, id_first, base_last, flags);
        log_phase("base", t_base);
        if qhr.is_err() {
            for k in keys {
                _ = RegCloseKey(k);
            }
            return qhr;
        }
        *self.base.borrow_mut() = Some(base);
        *self.keys.borrow_mut() = keys;

        let shift = flags & CMF_EXTENDEDVERBS != 0;
        let manual = manual_rows(&dirty, &opened, self.targets, &cat, &off, &on);
        let t_ext = Instant::now();
        let mut next = id_first.saturating_add(BASE_END);
        let mut at = GetMenuItemCount(Some(menu)).max(0) as u32;
        for row in &manual {
            if row.verb.is_empty() {
                // COM handler
            } else if row.command.is_empty() && row.clsid.is_some() {
                // DelegateExecute with no command string: load it like a handler.
            } else {
                continue;
            }
            let Some(guid) = row.clsid else { continue };
            if next.saturating_add(SLOT) > id_last.min(VERB_BASE) {
                break;
            }
            let Some(cm) = cocreate(&guid, &row.anchor, self.folder_pidl, self.data.as_ref()) else { continue };
            let hr = cm.QueryContextMenu(menu, at, next, next + SLOT, flags);
            if hr.is_ok() {
                self.parts.borrow_mut().push(Part { first: next, cm });
            }
            next = next.saturating_add(SLOT);
            at = GetMenuItemCount(Some(menu)).max(0) as u32;
        }
        log_phase("handlers", t_ext);

        let t_verb = Instant::now();
        let mut verb_id = VERB_BASE.max(next);
        let mut top_at = index_menu;
        for row in &manual {
            if row.command.is_empty() || row.verb.is_empty() {
                continue;
            }
            if shell_owned(&row.verb) {
                continue;
            }
            if row.extended && !shift {
                continue;
            }
            if row.single && self.paths.len() != 1 {
                continue;
            }
            if verb_id >= VERB_END || verb_id >= id_last {
                break;
            }
            let label = HSTRING::from(&row.name);
            let pos = if row.position < 0 { top_at } else { at };
            if InsertMenuW(menu, pos, MF_STRING | MF_BYPOSITION, verb_id as usize, &label).is_ok() {
                self.verbs.borrow_mut().push(VerbSlot { id: verb_id, command: row.command.clone() });
                if row.position < 0 {
                    top_at = top_at.saturating_add(1);
                }
                at = GetMenuItemCount(Some(menu)).max(0) as u32;
            }
            verb_id += 1;
        }
        log_phase("verbs", t_verb);
        HRESULT(0x200)
    }

    unsafe fn invoke(&self, pici: *const CMINVOKECOMMANDINFO) -> Result<()> {
        if pici.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let Some(offset) = crate::verb_offset(pici) else {
            if let Some(base) = self.base.borrow().as_ref() {
                return base.InvokeCommand(pici);
            }
            return Err(E_NOTIMPL.into());
        };
        // popup_ctx queries with id 1, then passes menu_id - 1.
        let menu_id = offset.saturating_add(1);
        if let Some(slot) = self.verbs.borrow().iter().find(|v| v.id == menu_id) {
            run_command((*pici).hwnd, &slot.command, &self.paths);
            return Ok(());
        }
        if let Some(part) = self.parts.borrow().iter().find(|p| menu_id >= p.first && menu_id < p.first + SLOT) {
            let local = menu_id - part.first;
            return invoke_part(&part.cm, local, &*pici);
        }
        if menu_id < BASE_END {
            if let Some(base) = self.base.borrow().as_ref() {
                return base.InvokeCommand(pici);
            }
        }
        Err(E_NOTIMPL.into())
    }

    unsafe fn command_string(&self, idcmd: usize, utype: u32, preserved: *const u32, pszname: PSTR, cchmax: u32) -> Result<()> {
        let menu_id = idcmd.saturating_add(1) as u32;
        if let Some(slot) = self.verbs.borrow().iter().find(|v| v.id == menu_id) {
            write_verb(pszname, cchmax, utype, "");
            let _ = slot;
            return Ok(());
        }
        if let Some(part) = self.parts.borrow().iter().find(|p| menu_id >= p.first && menu_id < p.first + SLOT) {
            let local = (menu_id - part.first) as usize;
            return part.cm.GetCommandString(local, utype, Some(preserved), pszname, cchmax);
        }
        if menu_id < BASE_END {
            if let Some(base) = self.base.borrow().as_ref() {
                return base.GetCommandString(idcmd, utype, Some(preserved), pszname, cchmax);
            }
        }
        Err(E_NOTIMPL.into())
    }

    unsafe fn forward_msg(&self, umsg: u32, wparam: WPARAM, lparam: LPARAM, plresult: *mut LRESULT) -> Result<()> {
        let mut result = Ok(());
        if let Some(base) = self.base.borrow().as_ref() {
            result = forward_one(base, umsg, wparam, lparam, plresult);
        }
        for part in self.parts.borrow().iter() {
            let r = forward_one(&part.cm, umsg, wparam, lparam, plresult);
            if r.is_ok() {
                result = r;
            }
        }
        result
    }

    fn reset_parts(&self) {
        self.base.borrow_mut().take();
        self.parts.borrow_mut().clear();
        self.verbs.borrow_mut().clear();
        let old = std::mem::take(&mut *self.keys.borrow_mut());
        unsafe {
            for k in old {
                _ = RegCloseKey(k);
            }
        }
    }
}

unsafe fn forward_one(cm: &IContextMenu, umsg: u32, wparam: WPARAM, lparam: LPARAM, plresult: *mut LRESULT) -> Result<()> {
    if let Ok(m) = cm.cast::<IContextMenu3>() {
        return m.HandleMenuMsg2(umsg, wparam, lparam, if plresult.is_null() { None } else { Some(plresult) });
    }
    if let Ok(m) = cm.cast::<IContextMenu2>() {
        return m.HandleMenuMsg(umsg, wparam, lparam);
    }
    Ok(())
}

unsafe fn invoke_part(cm: &IContextMenu, local: u32, ici: &CMINVOKECOMMANDINFO) -> Result<()> {
    let verb = part_verb(cm, local as usize);
    if !verb.is_empty() {
        let mut wide: Vec<u16> = verb.encode_utf16().chain(std::iter::once(0)).collect();
        let mut ansi: Vec<u8> = verb.bytes().chain(std::iter::once(0)).collect();
        let mut ex = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: ici.fMask | CMIC_MASK_UNICODE,
            hwnd: ici.hwnd,
            lpVerb: PCSTR(ansi.as_mut_ptr()),
            lpParameters: ici.lpParameters,
            lpDirectory: ici.lpDirectory,
            nShow: ici.nShow,
            dwHotKey: ici.dwHotKey,
            lpVerbW: PCWSTR(wide.as_mut_ptr()),
            ..Default::default()
        };
        if cm.InvokeCommand(&ex as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO).is_ok() {
            return Ok(());
        }
        let _ = (&mut ex, &mut wide, &mut ansi);
    }
    let mut c = *ici;
    c.lpVerb = PCSTR(local as usize as *const u8);
    cm.InvokeCommand(&c)
}

unsafe fn part_verb(cm: &IContextMenu, offset: usize) -> String {
    let mut buf = [0u16; 80];
    if cm.GetCommandString(offset, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32).is_err() {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

unsafe fn write_verb(pszname: PSTR, cchmax: u32, utype: u32, text: &str) {
    if pszname.is_null() || cchmax == 0 {
        return;
    }
    if utype & GCS_UNICODE != 0 {
        let w = pszname.0 as *mut u16;
        let mut n = 0usize;
        for c in text.encode_utf16().take(cchmax as usize - 1) {
            *w.add(n) = c;
            n += 1;
        }
        *w.add(n) = 0;
    } else {
        let mut n = 0usize;
        for b in text.bytes().take(cchmax as usize - 1) {
            *pszname.0.add(n) = b;
            n += 1;
        }
        *pszname.0.add(n) = 0;
    }
}

fn run_command(hwnd: HWND, command: &str, paths: &[String]) {
    for group in invocations(command, paths) {
        let line = expand_command(command, &group);
        let (exe, args) = split_cmd(&line);
        if exe.is_empty() {
            continue;
        }
        unsafe {
            let file = HSTRING::from(&exe);
            let params = HSTRING::from(&args);
            let _ = ShellExecuteW(Some(hwnd), w!("open"), &file, &params, PCWSTR::null(), SW_SHOWNORMAL);
        }
    }
}

fn invocations(command: &str, paths: &[String]) -> Vec<Vec<String>> {
    let low = command.to_ascii_lowercase();
    let has_star = command.contains("%*");
    let has_one = command.contains("%1") || low.contains("%l") || low.contains("%v") || low.contains("%w");
    if has_star || !has_one || paths.len() <= 1 {
        vec![if paths.is_empty() { Vec::new() } else { paths.to_vec() }]
    } else {
        paths.iter().take(64).map(|p| vec![p.clone()]).collect()
    }
}

fn expand_command(template: &str, paths: &[String]) -> String {
    substitute(&expand_env(template), paths)
}

fn substitute(template: &str, paths: &[String]) -> String {
    let one = paths.first().map(String::as_str).unwrap_or("");
    let all = paths.iter().map(|p| quote_path(p)).collect::<Vec<_>>().join(" ");
    let mut s = template.replace("%*", &all);
    for token in ["%1", "%L", "%l", "%V", "%v", "%W", "%w"] {
        s = s.replace(token, one);
    }
    s
}

fn quote_path(p: &str) -> String {
    if p.contains(' ') || p.contains('\t') { format!("\"{p}\"") } else { p.to_string() }
}

fn split_cmd(line: &str) -> (String, String) {
    let c = line.trim();
    if let Some(rest) = c.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            let exe = rest.get(..end).unwrap_or("");
            let args = rest.get(end.saturating_add(1)..).unwrap_or("").trim();
            return (exe.to_string(), args.to_string());
        }
    }
    match c.split_once(char::is_whitespace) {
        Some((exe, args)) => (exe.to_string(), args.trim().to_string()),
        None => (c.to_string(), String::new()),
    }
}

fn expand_env(s: &str) -> String {
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = vec![0u16; 4096];
    let n = unsafe { ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), PWSTR(buf.as_mut_ptr()), buf.len() as u32) };
    if n == 0 || n as usize > buf.len() {
        return s.to_string();
    }
    String::from_utf16_lossy(&buf[..n as usize - 1])
}

unsafe fn cocreate(guid: &GUID, class: &str, folder_pidl: *const ITEMIDLIST, data: Option<&IDataObject>) -> Option<IContextMenu> {
    let cm: IContextMenu = CoCreateInstance(guid, None, CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER).ok()?;
    if let Ok(init) = cm.cast::<IShellExtInit>() {
        let key = open_hkcr(class);
        let hk = key.unwrap_or_default();
        // `Option<*const _>` uses a null pointer as None. Some(null) is not a value we can pass.
        let pidl = if folder_pidl.is_null() { None } else { Some(folder_pidl) };
        let _ = init.Initialize(pidl, data, Some(hk));
        if let Some(k) = key {
            _ = RegCloseKey(k);
        }
    }
    Some(cm)
}

fn bits_of(names: &[String]) -> u8 {
    names.iter().fold(0, |a, n| {
        a | match n.as_str() {
            "file" => FILE,
            "folder" => FOLDER,
            "background" => BACKGROUND,
            "drive" => DRIVE,
            _ => 0,
        }
    })
}

fn winmerge_bits(on: &BTreeMap<String, Vec<String>>) -> u8 {
    bits_of(on.get(APP_WINMERGE).map(Vec::as_slice).unwrap_or(&[]))
}

fn winmerge_covers(on: &BTreeMap<String, Vec<String>>, targets: u8) -> bool {
    targets != 0 && winmerge_bits(on) & targets == targets
}

fn mentions_winmerge(s: &str) -> bool {
    s.to_ascii_lowercase().contains("winmerge")
}

/// Shell handler and Maguro's own commands. New handlers start on; this one starts off.
fn row_defaults_off(r: &Row) -> bool {
    r.id == APP_WINMERGE || r.id.eq_ignore_ascii_case(WINMERGE_CLSID) || mentions_winmerge(&r.product) || mentions_winmerge(&r.raw) || mentions_winmerge(&r.name)
}

fn is_legacy_winmerge_id(id: &str) -> bool {
    let n = id.trim().to_ascii_lowercase();
    n != APP_WINMERGE && (n == WINMERGE_CLSID || n.contains("winmerge"))
}

/// An older file stored WinMerge only when it was turned off. Targets missing from that list
/// were left on, so they become an explicit on. No entry at all stays off, the new default.
pub(crate) fn reconcile_winmerge(off: &mut BTreeMap<String, Vec<String>>, on: &mut BTreeMap<String, Vec<String>>) {
    if on.contains_key(APP_WINMERGE) {
        off.retain(|id, _| !is_legacy_winmerge_id(id));
        return;
    }
    let keys: Vec<String> = off.keys().filter(|id| is_legacy_winmerge_id(id)).cloned().collect();
    if keys.is_empty() {
        return;
    }
    let mut enabled = 0u8;
    for id in &keys {
        enabled |= TARGET_ALL & !bits_of(&off[id]);
    }
    write_bits(on, APP_WINMERGE, enabled);
    for id in keys {
        off.remove(&id);
    }
}

fn listed(r: &Row, seen: &BTreeMap<String, Vec<String>>, quiet: &BTreeSet<String>) -> bool {
    if r.builtin || r.targets == 0 {
        return false;
    }
    if seen.get(&r.id).is_some_and(|v| !v.is_empty()) {
        return true;
    }
    // Registered, but this menu never shows a row. Hidden before a probe marks them quiet.
    if shell_dormant(r) {
        return false;
    }
    if quiet.contains(&r.id) && r.static_labels.is_empty() {
        return false;
    }
    true
}

/// The generic Windows product name, or a screen saver. Neither inserts a row in a folder menu.
fn shell_dormant(r: &Row) -> bool {
    generic_windows_product(&title_of(r))
        || generic_windows_product(&r.product)
        || generic_windows_product(&r.name)
        || screen_saver_title(&title_of(r))
        || screen_saver_title(&r.product)
        || screen_saver_title(&r.name)
        || screen_saver_title(&r.raw)
        || screen_saver_title(&r.verb)
}

fn generic_windows_product(s: &str) -> bool {
    let n = fold_menu(&polish_name(s));
    n.contains("windows") && n.contains("operating system")
}

fn screen_saver_title(s: &str) -> bool {
    let n: String = fold_menu(&polish_name(s)).chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    n == "screensaver" || n.contains("スクリーンセーバー") || n.contains("화면보호기")
}

fn app_winmerge_row() -> Row {
    let mut static_labels = Vec::new();
    push_unique(&mut static_labels, &crate::winmerge::context_labels());
    Row {
        id: APP_WINMERGE.into(),
        name: "WinMerge".into(),
        targets: TARGET_ALL,
        builtin: false,
        clsid: None,
        verb: String::new(),
        command: String::new(),
        rank: 0,
        extended: false,
        single: false,
        position: 0,
        classes: Vec::new(),
        anchor: String::new(),
        product: "WinMerge".into(),
        raw: String::new(),
        static_labels,
    }
}

fn write_bits(map: &mut BTreeMap<String, Vec<String>>, id: &str, bits: u8) {
    if id.is_empty() || bits == 0 {
        map.remove(id);
        return;
    }
    let mut v = Vec::new();
    for (bit, name) in [(FILE, "file"), (FOLDER, "folder"), (BACKGROUND, "background"), (DRIVE, "drive")] {
        if bits & bit != 0 {
            v.push(name.to_string());
        }
    }
    map.insert(id.to_string(), v);
}

fn blocked(row: &Row, click: u8, off: &BTreeMap<String, Vec<String>>, on: &BTreeMap<String, Vec<String>>) -> bool {
    if row.builtin {
        return false;
    }
    let relevant = row.targets & click;
    if relevant == 0 {
        return false;
    }
    if row_defaults_off(row) {
        return !winmerge_covers(on, relevant);
    }
    bits_of(off.get(&row.id).map(Vec::as_slice).unwrap_or(&[])) & relevant != 0
}

fn split_classes(classes: &[String], click: u8, cat: &[Row], off: &BTreeMap<String, Vec<String>>, on: &BTreeMap<String, Vec<String>>) -> (Vec<String>, Vec<String>) {
    let mut clean = Vec::new();
    let mut dirty = Vec::new();
    for c in classes {
        if class_dirty(c, click, cat, off, on) {
            dirty.push(c.clone());
        } else {
            clean.push(c.clone());
        }
    }
    (clean, dirty)
}

fn class_dirty(class: &str, click: u8, cat: &[Row], off: &BTreeMap<String, Vec<String>>, on: &BTreeMap<String, Vec<String>>) -> bool {
    cat.iter().any(|r| r.has_class(class) && blocked(r, click, off, on))
}

fn manual_rows<'a>(dirty: &[String], clean: &[String], click: u8, cat: &'a [Row], off: &BTreeMap<String, Vec<String>>, on: &BTreeMap<String, Vec<String>>) -> Vec<&'a Row> {
    let mut out = Vec::new();
    for row in cat {
        if row.targets & click == 0 || blocked(row, click, off, on) {
            continue;
        }
        let on_dirty = dirty.iter().any(|c| row.has_class(c));
        let on_clean = clean.iter().any(|c| row.has_class(c));
        if on_dirty && !on_clean && !out.iter().any(|r: &&Row| r.id == row.id) {
            out.push(row);
        }
    }
    out
}

impl Row {
    fn has_class(&self, class: &str) -> bool {
        self.classes.iter().any(|c| c.eq_ignore_ascii_case(class))
    }
}

fn target_blob(bits: u8) -> String {
    target_bits().into_iter().filter(|b| bits & b != 0).map(target_label).collect::<Vec<_>>().join(" ").to_lowercase()
}

struct Cache {
    rows: Vec<Row>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

pub(crate) fn ensure_catalogue() {
    let _ = catalogue();
}

fn catalogue() -> Vec<Row> {
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = guard.as_ref() {
        return c.rows.clone();
    }
    let t0 = Instant::now();
    let rows = unsafe { scan_all() };
    log_phase("scan", t0);
    guard.replace(Cache { rows: rows.clone() });
    rows
}

unsafe fn scan_all() -> Vec<Row> {
    let mut rows = Vec::new();
    let mut dlls: HashMap<String, (String, String, String)> = HashMap::new();
    let mut seen_clsid: HashMap<String, (String, String, String)> = HashMap::new();
    let approved = read_approved();
    for (root, prefix) in roots() {
        for &(rel, bits) in FIXED {
            scan_class(root, prefix, rel, bits, &mut rows, &mut dlls, &mut seen_clsid, &approved);
        }
        scan_extensions(root, prefix, &mut rows, &mut dlls, &mut seen_clsid, &approved);
    }
    rows.sort_by(|a, b| title_of(a).to_lowercase().cmp(&title_of(b).to_lowercase()).then(a.id.cmp(&b.id)));
    rows
}

fn roots() -> [(HKEY, &'static str); 5] {
    [
        (HKEY_CLASSES_ROOT, ""),
        (HKEY_CURRENT_USER, r"Software\Classes\"),
        (HKEY_LOCAL_MACHINE, r"Software\Classes\"),
        (HKEY_LOCAL_MACHINE, r"Software\Wow6432Node\Classes\"),
        (HKEY_CLASSES_ROOT, r"Wow6432Node\"),
    ]
}

unsafe fn scan_extensions(root: HKEY, prefix: &str, rows: &mut Vec<Row>, dlls: &mut HashMap<String, (String, String, String)>, clsids: &mut HashMap<String, (String, String, String)>, approved: &HashMap<String, String>) {
    let base = prefix.trim_end_matches('\\');
    let Some(key) = open_at(root, base) else { return };
    let mut i = 0u32;
    while i < 8000 {
        let Some(name) = enum_key(key, i) else { break };
        i += 1;
        if name.starts_with('.') {
            scan_class(root, prefix, &name, FILE, rows, dlls, clsids, approved);
            let full = join_reg(prefix, &name);
            if let Some(k) = open_at(root, &full) {
                let prog = key_sz(k, PCWSTR::null());
                let perceived = key_sz_named(k, "PerceivedType");
                _ = RegCloseKey(k);
                if progid_like(&prog) {
                    scan_class(root, prefix, &prog, FILE, rows, dlls, clsids, approved);
                }
                if !perceived.is_empty() && !perceived.contains(['\\', '/']) {
                    scan_class(root, prefix, &format!(r"SystemFileAssociations\{perceived}"), FILE, rows, dlls, clsids, approved);
                }
            }
            scan_class(root, prefix, &format!(r"SystemFileAssociations\{name}"), FILE, rows, dlls, clsids, approved);
        } else if name.eq_ignore_ascii_case("SystemFileAssociations") {
            let assoc = join_reg(prefix, &name);
            let Some(ak) = open_at(root, &assoc) else { continue };
            let mut j = 0u32;
            while j < 4000 {
                let Some(child) = enum_key(ak, j) else { break };
                j += 1;
                scan_class(root, prefix, &format!(r"SystemFileAssociations\{child}"), FILE, rows, dlls, clsids, approved);
            }
            _ = RegCloseKey(ak);
        }
    }
    _ = RegCloseKey(key);
}

fn progid_like(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && t.len() < 128 && !t.starts_with('.') && !t.contains(['\\', '/']) && parse_guid(t).is_none()
}

unsafe fn scan_class(root: HKEY, prefix: &str, rel: &str, targets: u8, rows: &mut Vec<Row>, dlls: &mut HashMap<String, (String, String, String)>, clsids: &mut HashMap<String, (String, String, String)>, approved: &HashMap<String, String>) {
    let full = join_reg(prefix, rel);
    scan_handlers(root, &full, rel, targets, rows, dlls, clsids, approved);
    scan_verbs(root, &full, rel, targets, rows, dlls, clsids);
}

unsafe fn scan_handlers(root: HKEY, full: &str, rel: &str, targets: u8, rows: &mut Vec<Row>, dlls: &mut HashMap<String, (String, String, String)>, clsids: &mut HashMap<String, (String, String, String)>, approved: &HashMap<String, String>) {
    let path = format!(r"{full}\shellex\ContextMenuHandlers");
    let Some(key) = open_at(root, &path) else { return };
    let mut i = 0u32;
    while i < 512 {
        let Some(name) = enum_key(key, i) else { break };
        i += 1;
        let sub = format!(r"{path}\{name}");
        let value = open_at(root, &sub).map(|k| {
            let v = key_sz(k, PCWSTR::null());
            unsafe { _ = RegCloseKey(k) };
            v
        }).unwrap_or_default();
        let guid = parse_guid(&value).or_else(|| parse_guid(&name));
        let Some(guid) = guid else { continue };
        let (legacy, dll, company) = clsid_meta(&guid, approved, dlls, clsids);
        let (desc, product, _) = if dll.is_empty() {
            (String::new(), String::new(), String::new())
        } else {
            file_version(&dll, dlls)
        };
        let stem = file_stem(&dll);
        let shown = friendly_name(&product, &desc, &[&legacy, &name, &stem]);
        let mut display = if shown.is_empty() { legacy.clone() } else { shown };
        if (display.is_empty() || identifier_like(&display) || menu_sentence(&display)) && parse_guid(&name).is_none() && !identifier_like(&name) && !menu_sentence(&name) {
            display = clean_label(&name);
        }
        if display.is_empty() {
            display = if parse_guid(&name).is_none() { clean_label(&name) } else { guid_text(&guid) };
        }
        // Builtin detection stays on the pre-vendor strings, so a friendlier title does not hide or reveal a handler.
        let builtin = com_builtin(&name, &guid, &dll, &company) || builtin_component_name(&legacy) || builtin_component_name(&name);
        let product_title = if menu_sentence(&display) {
            String::new()
        } else if product_like(&display) && (identifier_like(&legacy) || !legacy.eq_ignore_ascii_case(&display)) {
            display.clone()
        } else {
            String::new()
        };
        upsert(rows, Row {
            id: format!("clsid:{}", guid_text(&guid)),
            name: display.clone(),
            targets,
            builtin,
            clsid: Some(guid),
            verb: String::new(),
            command: String::new(),
            rank: specificity(rel),
            extended: false,
            single: false,
            position: 0,
            classes: vec![rel.to_string()],
            anchor: rel.to_string(),
            product: product_title,
            raw: component_raw(&name, &legacy),
            static_labels: Vec::new(),
        });
    }
    _ = RegCloseKey(key);
}

fn generic_class(rel: &str) -> bool {
    let n = rel.trim().trim_matches('\\').to_ascii_lowercase();
    matches!(n.as_str(), "*" | "directory" | "folder" | "drive" | "allfilesystemobjects" | "unknown")
        || n.starts_with("systemfileassociations\\")
        || n.ends_with("\\background")
}

/// The application that owns this ProgID: its open command's version info, or the ProgID's own name.
unsafe fn owner_name(root: HKEY, full: &str, rel: &str, dlls: &mut HashMap<String, (String, String, String)>) -> String {
    if generic_class(rel) {
        return String::new();
    }
    if let Some((head, tail)) = rel.split_once('\\') {
        if head.eq_ignore_ascii_case("Applications") {
            let stem = file_stem(tail);
            if kept_title(&stem) && !menu_sentence(&stem) {
                return known_product(&stem).unwrap_or(stem);
            }
        }
    }
    let open = open_at(root, &format!(r"{full}\shell\open\command")).map(|ck| {
        let command = key_sz(ck, PCWSTR::null());
        _ = RegCloseKey(ck);
        command
    }).unwrap_or_default();
    let target = version_target(&open);
    if !target.is_empty() {
        let (desc, product, _) = file_version(&target, dlls);
        let stem_owned = file_stem(&target);
        let stem = if title_junk(&stem_owned) { "" } else { stem_owned.as_str() };
        let title = friendly_name(&product, &desc, &[stem]);
        if kept_title(&title) && !menu_sentence(&title) {
            return title;
        }
    }
    if progid_like(rel) && !rel.contains('\\') {
        if let Some(k) = open_at(root, full) {
            let name = polish_name(&key_sz(k, PCWSTR::null()));
            _ = RegCloseKey(k);
            if kept_title(&name) && !menu_sentence(&name) {
                return known_product(&name).unwrap_or(name);
            }
        }
    }
    String::new()
}

unsafe fn scan_verbs(root: HKEY, full: &str, rel: &str, targets: u8, rows: &mut Vec<Row>, dlls: &mut HashMap<String, (String, String, String)>, clsids: &mut HashMap<String, (String, String, String)>) {
    let path = format!(r"{full}\shell");
    let Some(key) = open_at(root, &path) else { return };
    let mut i = 0u32;
    while i < 512 {
        let Some(name) = enum_key(key, i) else { break };
        i += 1;
        if matches!(name.to_ascii_lowercase().as_str(), "command" | "ddeexec" | "droptarget" | "shellex") {
            continue;
        }
        let sub = format!(r"{path}\{name}");
        let Some(vk) = open_at(root, &sub) else { continue };
        if value_exists(vk, "LegacyDisable") || value_exists(vk, "ProgrammaticAccessOnly") {
            _ = RegCloseKey(vk);
            continue;
        }
        let display = key_sz(vk, PCWSTR::null());
        let mui = key_sz_named(vk, "MuiVerb");
        let extended = value_exists(vk, "Extended");
        let position = match key_sz_named(vk, "Position").to_ascii_lowercase().as_str() {
            "top" => -1,
            "bottom" => 1,
            _ => 0,
        };
        let single = key_sz_named(vk, "MultiSelectModel").eq_ignore_ascii_case("single");
        let command = open_at(root, &format!(r"{sub}\command")).map(|ck| {
            let c = key_sz(ck, PCWSTR::null());
            unsafe { _ = RegCloseKey(ck) };
            c
        }).unwrap_or_default();
        let delegate = key_sz_named(vk, "DelegateExecute");
        _ = RegCloseKey(vk);
        let delegate_guid = parse_guid(&delegate);
        let delegate_dll = delegate_guid.as_ref().map(|g| clsid_meta(g, &HashMap::new(), dlls, clsids).1).unwrap_or_default();
        if command.is_empty() && delegate_guid.is_none() {
            continue;
        }
        // `name` stays the registry menu text. Deletion also matches the resolved MuiVerb. The product title is separate.
        let friendly = {
            let raw = if display.is_empty() { name.as_str() } else { display.as_str() };
            let shown = clean_label(raw);
            if shown.is_empty() { clean_label(&name) } else { shown }
        };
        let builtin = verb_builtin(&name, &command, &delegate_dll) || builtin_component_name(&friendly);
        let target = if command.is_empty() { delegate_dll.clone() } else { version_target(&command) };
        let (desc, product, _) = if target.is_empty() {
            (String::new(), String::new(), String::new())
        } else {
            file_version(&target, dlls)
        };
        let stem = file_stem(&target);
        let owner = owner_name(root, full, rel, dlls);
        let product_title = verb_product(&product, &desc, &stem, &friendly, &name, &owner);
        let mut static_labels = Vec::new();
        push_unique(&mut static_labels, &[menu_label(&mui), friendly.clone()]);
        push_unique(&mut static_labels, &cascade_labels(root, &sub));
        upsert(rows, Row {
            id: format!("verb:{}", name.to_ascii_lowercase()),
            name: friendly,
            targets,
            builtin,
            clsid: delegate_guid,
            verb: name.clone(),
            command: command.clone(),
            rank: specificity(rel),
            extended,
            single,
            position,
            classes: vec![rel.to_string()],
            anchor: rel.to_string(),
            product: product_title,
            raw: verb_raw(&name, &stem),
            static_labels,
        });
    }
    _ = RegCloseKey(key);
}

/// One level of `shell\<verb>\shell\<child>`. The parent row keeps those menu strings.
unsafe fn cascade_labels(root: HKEY, verb_key: &str) -> Vec<String> {
    let path = format!(r"{verb_key}\shell");
    let Some(key) = open_at(root, &path) else { return Vec::new() };
    let mut out = Vec::new();
    let mut i = 0u32;
    while i < 64 {
        let Some(name) = enum_key(key, i) else { break };
        i += 1;
        if name.eq_ignore_ascii_case("command") {
            continue;
        }
        let Some(vk) = open_at(root, &format!(r"{path}\{name}")) else { continue };
        if value_exists(vk, "LegacyDisable") || value_exists(vk, "ProgrammaticAccessOnly") {
            _ = RegCloseKey(vk);
            continue;
        }
        let mui = key_sz_named(vk, "MuiVerb");
        let display = key_sz(vk, PCWSTR::null());
        _ = RegCloseKey(vk);
        let raw = if !mui.is_empty() {
            mui
        } else if !display.is_empty() {
            display
        } else {
            name
        };
        let label = menu_label(&raw);
        if label.is_empty() || out.iter().any(|e: &String| fold_menu(e) == fold_menu(&label)) {
            continue;
        }
        out.push(label);
        if out.len() >= 8 {
            break;
        }
    }
    _ = RegCloseKey(key);
    out
}

fn upsert(rows: &mut Vec<Row>, incoming: Row) {
    if let Some(have) = rows.iter_mut().find(|r| r.id == incoming.id) {
        have.targets |= incoming.targets;
        have.builtin |= incoming.builtin;
        have.extended = have.extended && incoming.extended;
        have.single = have.single && incoming.single;
        if incoming.position < 0 || have.position < 0 {
            have.position = -1;
        } else if incoming.position > 0 || have.position > 0 {
            have.position = 1;
        }
        if incoming.rank >= have.rank && !incoming.command.is_empty() {
            have.command = incoming.command.clone();
            have.rank = incoming.rank;
            have.anchor = incoming.anchor.clone();
        }
        if have.clsid.is_none() {
            have.clsid = incoming.clsid;
        }
        let incoming_better = !incoming.name.is_empty()
            && !title_junk(&incoming.name)
            && (have.name.is_empty() || title_junk(&have.name) || parse_guid(&have.name).is_some() || (identifier_like(&have.name) && !identifier_like(&incoming.name)));
        if incoming_better {
            have.name = incoming.name;
        }
        if have.verb.is_empty() {
            have.verb = incoming.verb;
        }
        if better_title(&have.product, &incoming.product) {
            have.product = incoming.product;
        }
        if have.raw.is_empty() {
            have.raw = incoming.raw;
        }
        push_unique(&mut have.static_labels, &incoming.static_labels);
        for c in incoming.classes {
            if !have.classes.iter().any(|e| e.eq_ignore_ascii_case(&c)) {
                have.classes.push(c);
            }
        }
        return;
    }
    rows.push(incoming);
}

fn better_title(have: &str, incoming: &str) -> bool {
    if !kept_title(incoming) {
        return false;
    }
    if !kept_title(have) {
        return true;
    }
    let have_menu = menu_sentence(have);
    let incoming_menu = menu_sentence(incoming);
    if have_menu && !incoming_menu {
        return true;
    }
    if !have_menu && incoming_menu {
        return false;
    }
    parse_guid(have).is_some() || (identifier_like(have) && !identifier_like(incoming))
}

fn specificity(class: &str) -> u8 {
    let c = class.to_ascii_lowercase();
    if c.starts_with('.') || c.starts_with("systemfileassociations\\") {
        5
    } else if c == "drive" || c.ends_with("\\background") {
        4
    } else if c == "directory" || c == "folder" {
        3
    } else if c == "allfilesystemobjects" {
        2
    } else if c == "*" {
        1
    } else {
        4
    }
}

fn com_builtin(key_name: &str, guid: &GUID, dll: &str, company: &str) -> bool {
    if builtin_clsid(guid) || builtin_handler_key(key_name) {
        return true;
    }
    inbox_dll(dll, company)
}

fn verb_builtin(key: &str, command: &str, delegate_dll: &str) -> bool {
    if builtin_verb(key) {
        return true;
    }
    if inbox_dll(delegate_dll, "") {
        return true;
    }
    !command.is_empty() && command_inbox(command)
}

fn inbox_dll(dll: &str, company: &str) -> bool {
    if dll.is_empty() {
        return false;
    }
    let path = if dll.contains('%') { expand_env(dll) } else { dll.to_string() };
    let microsoft = company.to_ascii_lowercase().contains("microsoft");
    if under_windows(&path) {
        // A third-party DLL that happens to sit in System32 stays listed.
        return company.is_empty() || microsoft;
    }
    // Photo Viewer and OLE DB ship outside System32 (Common Files\System\Ole DB, PhotoViewer.dll).
    if microsoft {
        let p = path.replace('/', "\\").to_ascii_lowercase();
        if p.contains("\\ole db\\") || p.contains("photoviewer") || p.contains("msdasc") || p.contains("shimgvw") {
            return true;
        }
    }
    false
}

fn command_inbox(cmd: &str) -> bool {
    if foreign_path(&cmd.to_ascii_lowercase()) {
        return false;
    }
    let exe = split_cmd(cmd).0.to_ascii_lowercase();
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(&exe);
    under_windows(&exe)
        || matches!(file, "cmd.exe" | "powershell.exe" | "pwsh.exe" | "wt.exe" | "wscript.exe" | "cscript.exe" | "rundll32.exe" | "explorer.exe" | "cmd" | "powershell" | "wt")
}

fn foreign_path(low: &str) -> bool {
    let b = low.as_bytes();
    let mut i = 0;
    while i + 2 < b.len() {
        if b[i].is_ascii_alphabetic() && b[i + 1] == b':' && (b[i + 2] == b'\\' || b[i + 2] == b'/') {
            let Some(rest) = low.get(i..) else {
                i += 1;
                continue;
            };
            let end = rest.find(['"', ' ', '\t']).unwrap_or(rest.len());
            let Some(span) = rest.get(..end) else {
                i += 1;
                continue;
            };
            if !under_windows(span) {
                return true;
            }
        }
        i += 1;
    }
    false
}

pub(crate) fn under_windows(path: &str) -> bool {
    let expanded = if path.contains('%') { expand_env(path) } else { path.to_string() };
    let p = expanded.trim().trim_matches('"').replace('/', "\\").to_ascii_lowercase();
    let p = p.trim_start_matches("\\\\?\\");
    p.contains("\\windows\\")
}

/// Windows-shipped shell components that are not in the known-CLSID list.
fn builtin_component_name(name: &str) -> bool {
    let n = fold_menu(name);
    if n.contains("windows photo viewer") || (n.contains("photo") && n.contains("image verbs")) {
        return true;
    }
    if n.contains("ole db") && (n.contains("microsoft") || n.contains("data link")) {
        return true;
    }
    false
}

pub(crate) fn builtin_verb(name: &str) -> bool {
    let n = name.trim().trim_matches('&').to_ascii_lowercase();
    if n.starts_with("windows.") {
        return true;
    }
    matches!(
        n.as_str(),
        "open"
            | "opennew"
            | "openas"
            | "openwith"
            | "openwithprompt"
            | "opencontaining"
            | "opennewprocess"
            | "opennewwindow"
            | "explore"
            | "find"
            | "edit"
            | "print"
            | "printto"
            | "preview"
            | "runas"
            | "runasuser"
            | "cut"
            | "copy"
            | "paste"
            | "delete"
            | "link"
            | "rename"
            | "properties"
            | "pintohome"
            | "unpinfromhome"
            | "pintostartscreen"
            | "pintotaskbar"
            | "unpinfromtaskbar"
            | "cmd"
            | "powershell"
            | "pwsh"
            | "wsl"
            | "bash"
            | "sharing"
            | "share"
            | "previousversions"
            | "copyaspath"
            | "format"
            | "eject"
            | "disconnect"
            | "connect"
            | "manage"
            | "modernshare"
            | "updateencryptionsettings"
            | "encryptionmenu"
            | "windowsdefender"
            | "playto"
            | "casttodevice"
            | "giveaccessto"
            | "includeinlibrary"
            | "new"
    )
}

fn shell_owned(verb: &str) -> bool {
    matches!(verb.to_ascii_lowercase().as_str(), "open" | "opennew" | "explore" | "find" | "delete" | "properties" | "link" | "rename" | "cut" | "copy" | "paste")
}

fn builtin_handler_key(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    if parse_guid(&n).is_some_and(|g| builtin_clsid(&g)) {
        return true;
    }
    matches!(
        n.as_str(),
        "open with"
            | "openwith"
            | "send to"
            | "sendto"
            | "sharing"
            | "share"
            | "previous versions"
            | "previousversions"
            | "offline files"
            | "offlinefiles"
            | "compatibility"
            | "encryption"
            | "encrypt"
            | "new"
            | "work folders"
            | "workfolders"
            | "windows defender"
            | "windowsdefender"
            | "include in library"
            | "library location"
            | "play to"
            | "playto"
            | "cast to device"
            | "pin to start"
            | "pintostart"
            | "pin to taskbar"
            | "pintotaskbar"
            | "unpin from taskbar"
            | "give access to"
            | "restore previous versions"
            | "extract all"
            | "compressedfolder"
            | "briefcasemenu"
            | "modern sharing"
            | "modernsharing"
            | "efs"
            | "offline files menu"
    ) || builtin_verb(&n.replace(' ', ""))
}

fn builtin_clsid(g: &GUID) -> bool {
    const IDS: &[&str] = &[
        "7ba4c740-9e81-11cf-99d3-00aa004ae837", // Send to
        "09799afb-ad67-11d1-abcd-00c04fc30936", // Open with
        "d969a300-e7ff-11d0-a93b-00a0c90f2719", // New
        "f81e9010-6ea4-11ce-a7ff-00aa003ca9f6", // Sharing
        "596ab062-b4d2-4215-9f74-e9109b0a8153", // Previous versions
        "e88dcce0-b7b3-11d1-a9f0-00aa0060fa31", // Compressed folder
        "b8cdcb65-b1bf-4b42-9428-1dfdb7ee92af", // Extract all
        "a2a9545d-a0c2-42b4-9708-a0b2badd77c8", // Pin to Start
        "90aa3a4e-1cba-4233-b8bb-535773d48449", // Pin to taskbar
        "470c0ebd-5d73-4d58-9ced-e91e22e23282", // Pin to Start screen
        "3dad6c5d-2167-4cae-9914-f99e41c12cfa", // Include in library
        "09a47860-11b0-4da5-afa5-26d86198a780", // Windows Defender
        "4a7ded0a-ad25-11d0-98a8-0800361b1103", // Briefcase
        "e2bf9676-5f8f-435c-9bda-ad7b2c0c3c9a", // Modern share
        "474c98ee-cf3d-41f5-80e3-4aab0ab04301", // Offline files
        "a470f8cf-a1e8-4f65-8335-227475aa5c46", // Encryption
        "7ad84985-87b4-4a16-be58-8b72a5b390f7", // Play to / Cast
        "b41db860-64e4-11d2-9906-00004b3083ff", // Send to (older)
        "888dca60-fc0a-11cf-8f0f-00c04fd7d062", // Send mail
    ];
    let text = guid_text(g);
    IDS.contains(&text.as_str())
}

fn clsid_meta(guid: &GUID, approved: &HashMap<String, String>, dlls: &mut HashMap<String, (String, String, String)>, clsids: &mut HashMap<String, (String, String, String)>) -> (String, String, String) {
    let key = guid_text(guid);
    if let Some((name, dll, company)) = clsids.get(&key) {
        return (name.clone(), dll.clone(), company.clone());
    }
    let (default_name, localized, dll) = unsafe { read_clsid(&key) };
    let approved_name = approved.get(&key).cloned().unwrap_or_default();
    let (desc, product, company) = if dll.is_empty() {
        (String::new(), String::new(), String::new())
    } else {
        file_version(&dll, dlls)
    };
    // LocalizedString, then the DLL's product and description, then the Approved list, then the CLSID default.
    let mut name = pick_label(&[&localized, &product, &desc, &approved_name, &default_name]);
    if name.is_empty() {
        name = dll.rsplit(['\\', '/']).next().unwrap_or("").to_string();
    }
    clsids.insert(key, (name.clone(), dll.clone(), company.clone()));
    (name, dll, company)
}

fn clsid_server_dll(guid: &GUID) -> String {
    let (_, _, dll) = unsafe { read_clsid(&guid_text(guid)) };
    dll
}

/// CLSID default, LocalizedString, and the server DLL. LocalizedString is often `@dll,-id`.
unsafe fn read_clsid(guid: &str) -> (String, String, String) {
    let braced = format!("{{{guid}}}");
    let paths = [
        format!(r"CLSID\{braced}"),
        format!(r"Software\Classes\CLSID\{braced}"),
        format!(r"Software\Classes\Wow6432Node\CLSID\{braced}"),
        format!(r"Software\Wow6432Node\Classes\CLSID\{braced}"),
        format!(r"Wow6432Node\CLSID\{braced}"),
    ];
    let roots = [HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_LOCAL_MACHINE, HKEY_CLASSES_ROOT];
    let mut name = String::new();
    let mut localized = String::new();
    let mut dll = String::new();
    for (root, path) in roots.iter().copied().zip(paths.iter()) {
        let Some(k) = open_at(root, path) else { continue };
        if name.is_empty() {
            let v = key_sz(k, PCWSTR::null());
            if parse_guid(&v).is_none() {
                name = v;
            }
        }
        if localized.is_empty() {
            let v = key_sz_named(k, "LocalizedString");
            if !v.is_empty() {
                localized = v;
            }
        }
        if dll.is_empty() {
            if let Some(s) = open_at(root, &format!(r"{path}\InprocServer32")) {
                dll = key_sz(s, PCWSTR::null());
                _ = RegCloseKey(s);
            }
            if dll.is_empty() {
                if let Some(s) = open_at(root, &format!(r"{path}\LocalServer32")) {
                    dll = key_sz(s, PCWSTR::null());
                    _ = RegCloseKey(s);
                }
            }
        }
        _ = RegCloseKey(k);
        if !dll.is_empty() && !localized.is_empty() {
            break;
        }
    }
    (name, localized, dll)
}

unsafe fn read_approved() -> HashMap<String, String> {
    let mut out = HashMap::new();
    let paths = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved"),
    ];
    for (root, path) in paths {
        let Some(k) = open_at(root, path) else { continue };
        let mut i = 0u32;
        while i < 4000 {
            let mut name = [0u16; 128];
            let mut nlen = name.len() as u32;
            let mut data = [0u16; 256];
            let mut dlen = (data.len() * 2) as u32;
            let mut kind = 0u32;
            let err = RegEnumValueW(k, i, Some(PWSTR(name.as_mut_ptr())), &mut nlen, None, Some(&mut kind), Some(data.as_mut_ptr() as _), Some(&mut dlen));
            if err != ERROR_SUCCESS {
                break;
            }
            i += 1;
            let id = String::from_utf16_lossy(&name[..nlen as usize]);
            let Some(g) = parse_guid(&id) else { continue };
            let chars = (dlen as usize / 2).saturating_sub(1).min(data.len());
            let label = String::from_utf16_lossy(&data[..chars]).trim_end_matches('\0').trim().to_string();
            if !label.is_empty() {
                out.entry(guid_text(&g)).or_insert(label);
            }
        }
        _ = RegCloseKey(k);
    }
    out
}

fn file_version(path: &str, cache: &mut HashMap<String, (String, String, String)>) -> (String, String, String) {
    let expanded = if path.contains('%') { expand_env(path) } else { path.to_string() };
    let key = expanded.to_ascii_lowercase();
    if let Some(v) = cache.get(&key) {
        return v.clone();
    }
    let got = unsafe { read_version(&expanded) };
    cache.insert(key, got.clone());
    got
}

/// FileDescription, ProductName, CompanyName.
unsafe fn read_version(path: &str) -> (String, String, String) {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut handle = 0u32;
    let size = GetFileVersionInfoSizeW(PCWSTR(wide.as_ptr()), &mut handle);
    if size == 0 || size > 8 * 1024 * 1024 {
        return (String::new(), String::new(), String::new());
    }
    let mut block = vec![0u8; size as usize];
    if !GetFileVersionInfoW(PCWSTR(wide.as_ptr()), 0, size, block.as_mut_ptr() as _).as_bool() {
        return (String::new(), String::new(), String::new());
    }
    let mut trans = std::ptr::null_mut();
    let mut tlen = 0u32;
    let query = |sub: &str| -> String {
        let w: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
        let mut p = std::ptr::null_mut();
        let mut n = 0u32;
        if !VerQueryValueW(block.as_ptr() as _, PCWSTR(w.as_ptr()), &mut p, &mut n).as_bool() || p.is_null() || n < 2 {
            return String::new();
        }
        let words = (n as usize).min(512);
        let slice = std::slice::from_raw_parts(p as *const u16, words);
        let end = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
        String::from_utf16_lossy(&slice[..end]).trim().to_string()
    };
    if !VerQueryValueW(block.as_ptr() as _, w!("\\VarFileInfo\\Translation"), &mut trans, &mut tlen).as_bool() || trans.is_null() || tlen < 4 {
        return (String::new(), String::new(), String::new());
    }
    let b = std::slice::from_raw_parts(trans as *const u8, 4);
    let lang = u16::from_le_bytes([b[0], b[1]]);
    let cp = u16::from_le_bytes([b[2], b[3]]);
    let stem = format!("\\StringFileInfo\\{lang:04x}{cp:04x}");
    (query(&format!("{stem}\\FileDescription")), query(&format!("{stem}\\ProductName")), query(&format!("{stem}\\CompanyName")))
}

fn indirect(s: &str) -> String {
    if !s.starts_with('@') {
        return s.to_string();
    }
    let src: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 512];
    let hr = unsafe { SHLoadIndirectString(PCWSTR(src.as_ptr()), PWSTR(buf.as_mut_ptr()), buf.len() as u32, std::ptr::null_mut()) };
    if hr.is_err() {
        return s.trim_start_matches('@').to_string();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let t = String::from_utf16_lossy(&buf[..n]);
    if t.is_empty() { s.to_string() } else { t }
}

#[allow(dead_code)] // Only the unused filtered-key builder asks for this class list.
pub(crate) fn classes_for(targets: u8, paths: &[String]) -> Vec<String> {
    let mut v = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !v.iter().any(|e: &String| e.eq_ignore_ascii_case(&s)) {
            v.push(s);
        }
    };
    if targets & FILE != 0 {
        for p in paths {
            if path_kind(p, true) != FILE {
                continue;
            }
            let Some(ext) = ext_of(p) else { continue };
            push(format!(".{ext}"));
            push(format!(r"SystemFileAssociations\.{ext}"));
            if let Some(prog) = unsafe { prog_of(&ext) } {
                push(prog);
            }
            if let Some(per) = unsafe { perceived_of(&ext) } {
                push(format!(r"SystemFileAssociations\{per}"));
            }
        }
        push("*".into());
        push("AllFilesystemObjects".into());
    }
    if targets & BACKGROUND != 0 {
        push(r"Directory\Background".into());
        push(r"Folder\Background".into());
    }
    if targets & DRIVE != 0 {
        push("Drive".into());
        push("Directory".into());
        push("Folder".into());
        push("AllFilesystemObjects".into());
    }
    if targets & FOLDER != 0 {
        push("Directory".into());
        push("Folder".into());
        push("AllFilesystemObjects".into());
    }
    v
}

#[allow(dead_code)]
fn ext_of(path: &str) -> Option<String> {
    let name = path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() || ext.len() > 40 || ext.contains([' ', '\\', '/']) {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

#[allow(dead_code)]
unsafe fn prog_of(ext: &str) -> Option<String> {
    let k = open_hkcr(&format!(".{ext}"))?;
    let v = key_sz(k, PCWSTR::null());
    _ = RegCloseKey(k);
    progid_like(&v).then_some(v)
}

#[allow(dead_code)]
unsafe fn perceived_of(ext: &str) -> Option<String> {
    let k = open_hkcr(&format!(".{ext}"))?;
    let v = key_sz_named(k, "PerceivedType");
    _ = RegCloseKey(k);
    (!v.is_empty() && !v.contains(['\\', '/'])).then_some(v)
}

fn open_hkcr(rel: &str) -> Option<HKEY> {
    unsafe { open_at(HKEY_CLASSES_ROOT, rel) }
}

unsafe fn open_at(root: HKEY, path: &str) -> Option<HKEY> {
    if path.is_empty() {
        return None;
    }
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut key = HKEY::default();
    let err = RegOpenKeyExW(root, PCWSTR(wide.as_ptr()), None, KEY_READ, &mut key);
    (err == ERROR_SUCCESS && !key.is_invalid()).then_some(key)
}

fn join_reg(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else if prefix.ends_with('\\') {
        format!("{prefix}{rel}")
    } else {
        format!(r"{prefix}\{rel}")
    }
}

unsafe fn enum_key(key: HKEY, index: u32) -> Option<String> {
    let mut name = [0u16; 256];
    let mut n = name.len() as u32;
    let err = RegEnumKeyExW(key, index, Some(PWSTR(name.as_mut_ptr())), &mut n, None, None, None, None);
    if err != ERROR_SUCCESS || n == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&name[..n as usize]))
}

unsafe fn key_sz(key: HKEY, value: PCWSTR) -> String {
    let mut buf = [0u16; 1024];
    let mut n = (buf.len() * 2) as u32;
    let err = RegGetValueW(key, PCWSTR::null(), value, RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n));
    if err != ERROR_SUCCESS || n < 2 {
        return String::new();
    }
    let chars = (n as usize / 2).saturating_sub(1).min(buf.len());
    String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').trim().to_string()
}

unsafe fn key_sz_named(key: HKEY, name: &str) -> String {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    key_sz(key, PCWSTR(wide.as_ptr()))
}

unsafe fn value_exists(key: HKEY, name: &str) -> bool {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut n = 0u32;
    RegGetValueW(key, PCWSTR::null(), PCWSTR(wide.as_ptr()), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_RT_REG_MULTI_SZ, None, None, Some(&mut n)) == ERROR_SUCCESS
}

fn parse_guid(s: &str) -> Option<GUID> {
    let t = s.trim().trim_start_matches('{').trim_end_matches('}');
    let hex: String = t.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u128::from_str_radix(&hex, 16).ok()?;
    let text = format!(
        "{}-{}-{}-{}-{}",
        hex.get(0..8)?,
        hex.get(8..12)?,
        hex.get(12..16)?,
        hex.get(16..20)?,
        hex.get(20..32)?
    );
    // from_u128 matches the textual GUID, so the dashed form must round-trip.
    let g = GUID::from_u128(n);
    (guid_text(&g) == text.to_ascii_lowercase()).then_some(g)
}

fn guid_text(g: &GUID) -> String {
    format!("{g:?}").trim_start_matches('{').trim_end_matches('}').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(bits: u8) -> Vec<String> {
        let mut m = BTreeMap::new();
        write_bits(&mut m, "x", bits);
        m.remove("x").unwrap_or_default()
    }

    #[test]
    fn kinds_follow_the_path() {
        assert_eq!(path_kind(r"C:\", false), DRIVE);
        assert_eq!(path_kind(r"C:", false), DRIVE);
        assert_eq!(path_kind(r"D:\Work", false), FOLDER);
        assert_eq!(path_kind(r"D:\Work\a.txt", true), FILE);
        assert_eq!(path_kind(r"\\server\share", false), FOLDER);
        assert_eq!(click_targets(true, &[(r"C:\Windows".into(), false)]), BACKGROUND);
        assert_eq!(click_targets(false, &[(r"C:\a.txt".into(), true), (r"C:\b".into(), false)]), FILE | FOLDER);
        assert_eq!(click_targets(false, &[(r"E:\".into(), false)]), DRIVE);
    }

    #[test]
    fn new_handlers_start_enabled_and_reset_clears_them() {
        let mut m = BTreeMap::new();
        assert_eq!(bits_of(m.get("clsid:abc").map(Vec::as_slice).unwrap_or(&[])), 0);
        assert_eq!(tick_of(0, FILE | FOLDER), Tick::On);
        toggle_master(&mut m, "clsid:abc", FILE | FOLDER);
        assert_eq!(tick_of(bits_of(&m["clsid:abc"]), FILE | FOLDER), Tick::Off);
        toggle_target(&mut m, "clsid:abc", FILE, FILE | FOLDER);
        assert_eq!(tick_of(bits_of(&m["clsid:abc"]), FILE | FOLDER), Tick::Mixed);
        assert!(target_enabled(bits_of(&m["clsid:abc"]), FILE));
        assert!(!target_enabled(bits_of(&m["clsid:abc"]), FOLDER));
        toggle_master(&mut m, "clsid:abc", FILE | FOLDER);
        assert!(!m.contains_key("clsid:abc"));
        toggle_master(&mut m, "verb:7-zip", BACKGROUND);
        clear_off(&mut m);
        assert!(m.is_empty());
        // write_bits lists targets in bit order: file, folder, background, drive.
        assert_eq!(names(FILE | DRIVE), vec!["file".to_string(), "drive".to_string()]);
    }

    #[test]
    fn builtins_cannot_be_blocked_and_a_clean_class_stays_with_the_shell() {
        let mut off = BTreeMap::new();
        off.insert("clsid:third".into(), vec!["file".into()]);
        off.insert("clsid:sendto".into(), vec!["file".into(), "folder".into()]);
        let third = Row {
            id: "clsid:third".into(),
            name: "Third".into(),
            targets: FILE | FOLDER,
            builtin: false,
            clsid: None,
            verb: String::new(),
            command: String::new(),
            rank: 2,
            extended: false,
            single: false,
            position: 0,
            classes: vec!["AllFilesystemObjects".into()],
            anchor: "AllFilesystemObjects".into(),
            product: String::new(),
            raw: String::new(),
            static_labels: Vec::new(),
        };
        let send = Row {
            id: "clsid:sendto".into(),
            name: "Send to".into(),
            targets: FILE | FOLDER,
            builtin: true,
            clsid: None,
            verb: String::new(),
            command: String::new(),
            rank: 2,
            extended: false,
            single: false,
            position: 0,
            classes: vec!["AllFilesystemObjects".into()],
            anchor: "AllFilesystemObjects".into(),
            product: String::new(),
            raw: String::new(),
            static_labels: Vec::new(),
        };
        let cat = vec![third, send];
        let none = BTreeMap::new();
        assert!(blocked(&cat[0], FILE, &off, &none));
        assert!(!blocked(&cat[0], FOLDER, &off, &none));
        assert!(!blocked(&cat[1], FILE, &off, &none), "a built-in handler stays even if the map names it");
        assert!(class_dirty("AllFilesystemObjects", FILE, &cat, &off, &none));
        assert!(!class_dirty("AllFilesystemObjects", FOLDER, &cat, &off, &none));
        assert!(!class_dirty("*", FILE, &cat, &off, &none));
        let (clean, dirty) = split_classes(&["*".into(), "AllFilesystemObjects".into()], FILE, &cat, &off, &none);
        assert_eq!(clean, vec!["*".to_string()]);
        assert_eq!(dirty, vec!["AllFilesystemObjects".to_string()]);
        let manual = manual_rows(&dirty, &clean, FILE, &cat, &off, &none);
        assert!(manual.iter().any(|r| r.id == "clsid:sendto"));
        assert!(manual.iter().all(|r| r.id != "clsid:third"));
    }

    #[test]
    fn inbox_verbs_and_windows_binaries_are_not_third_party() {
        assert!(builtin_verb("Open"));
        assert!(builtin_verb("windows.copyaspath"));
        assert!(builtin_verb("cmd"));
        assert!(!builtin_verb("7-Zip"));
        assert!(!builtin_verb("OpenWithNotepad"));
        assert!(under_windows(r"C:\Windows\System32\shell32.dll"));
        assert!(under_windows(r"\\?\C:\Windows\SysWOW64\zipfldr.dll"));
        assert!(!under_windows(r"C:\Program Files\7-Zip\7-zip.dll"));
        assert!(command_inbox(r#"cmd.exe /s /k pushd "%V""#));
        assert!(!command_inbox(r#""C:\Program Files\7-Zip\7zFM.exe" "%1""#));
        assert!(shell_owned("properties"));
        assert!(!shell_owned("SevenZip"));
    }

    #[test]
    fn command_tokens_expand_without_touching_the_registry() {
        assert_eq!(substitute(r#""C:\App\a.exe" "%1""#, &[r"C:\a b.txt".into()]), r#""C:\App\a.exe" "C:\a b.txt""#);
        assert_eq!(substitute(r"tool.exe %*", &[r"C:\a.txt".into(), r"C:\b c.txt".into()]), r#"tool.exe C:\a.txt "C:\b c.txt""#);
        assert_eq!(split_cmd(r#""C:\Program Files\7-Zip\7zFM.exe" "%1""#), (r"C:\Program Files\7-Zip\7zFM.exe".into(), r#""%1""#.into()));
        let multi = invocations(r#""C:\App\a.exe" "%1""#, &[r"C:\a.txt".into(), r"C:\b.txt".into()]);
        assert_eq!(multi.len(), 2);
        let once = invocations(r"tool.exe %*", &[r"C:\a.txt".into(), r"C:\b.txt".into()]);
        assert_eq!(once.len(), 1);
    }

    #[test]
    fn file_classes_include_the_extension_and_the_shared_keys() {
        let c = classes_for(FILE, &[r"C:\work\note.txt".into()]);
        assert!(c.iter().any(|s| s == ".txt"));
        assert!(c.iter().any(|s| s == "*"));
        assert!(c.iter().any(|s| s.eq_ignore_ascii_case("AllFilesystemObjects")));
        let d = classes_for(DRIVE | BACKGROUND, &[r"C:\".into()]);
        assert!(d.iter().any(|s| s == "Drive"));
        assert!(d.iter().any(|s| s == r"Directory\Background"));
        assert!(d.iter().all(|s| s != "*"));
    }

    #[test]
    fn guid_text_round_trips_the_send_to_clsid() {
        let g = parse_guid("{7BA4C740-9E81-11CF-99D3-00AA004AE837}").unwrap();
        assert_eq!(guid_text(&g), "7ba4c740-9e81-11cf-99d3-00aa004ae837");
        assert!(builtin_clsid(&g));
        assert!(parse_guid("not-a-guid").is_none());
    }

    #[test]
    fn menu_labels_drop_accelerators_and_class_identifiers() {
        assert_eq!(strip_accel("&Browse"), "Browse");
        assert_eq!(strip_accel("C&onfigure..."), "Configure...");
        assert_eq!(strip_accel("Fish && Chips"), "Fish & Chips");
        assert_eq!(fold_menu("C&onfigure..."), "configure...");
        // `&&` is a literal ampersand. A single `&` is an accelerator and is dropped.
        assert_eq!(fold_menu("Fish && Chips"), "fish & chips");
        assert_eq!(fold_menu("Fish & Chips"), "fish  chips");
        assert!(identifier_like("AABdzCtx Class"));
        assert!(identifier_like("SynoFolderContextMenu Class"));
        assert!(identifier_like("FileSyncEx"));
        assert!(identifier_like("HVContext Class"));
        assert!(!identifier_like("7-Zip"));
        assert!(!identifier_like("Synology Drive"));
        assert_eq!(pick_label(&["AABdzCtx Class", "Synology Drive"]), "Synology Drive");
        assert_eq!(pick_label(&["FileSyncEx"]), "FileSyncEx");
        assert!(builtin_component_name("Windows Photo Viewer Image Verbs"));
        assert!(builtin_component_name("Microsoft OLE DB Service Component Data Links"));
        assert!(!builtin_component_name("7-Zip"));
        assert!(!builtin_component_name("Synology Drive"));
        assert!(under_windows(r"%SystemRoot%\System32\msdasc.dll"));
        assert!(under_windows(r"%windir%\System32\PhotoViewer.dll"));
        assert!(under_windows(r"C:\Windows\System32\shimgvw.dll"));
        assert!(!under_windows(r"C:\Program Files\7-Zip\7-zip.dll"));
        assert!(identifier_like("BdzShell"));
        assert!(identifier_like("LHShellExt"));
        assert!(identifier_like("HVShell Module"));
        assert_eq!(strip_name_tag("TortoiseSVN (V)"), "TortoiseSVN");
        assert_eq!(strip_name_tag("Bandizip (x64)"), "Bandizip");
        assert_eq!(strip_name_tag("Bandizip(V)"), "Bandizip");
        assert_eq!(strip_name_tag("Synology Drive"), "Synology Drive");
        assert_eq!(menu_label("&반디집으로 압축하기"), "반디집으로 압축하기");
        assert_eq!(menu_label("SVN 업데이트 (U)"), "SVN 업데이트");
        assert_eq!(menu_label("Add to archive...\tCtrl+A"), "Add to archive...");
        assert_eq!(menu_label("Fish && Chips (Ctrl+C)"), "Fish & Chips");
        assert_eq!(menu_label("Open with (Notepad++)"), "Open with (Notepad++)");
        assert_eq!(menu_label("Visual Studio で開く(V)"), "Visual Studio で開く");
        assert_eq!(menu_label("Visual Studio で開く（V）"), "Visual Studio で開く");
        assert_eq!(menu_label("Open(V)..."), "Open...");
        assert_eq!(menu_label("Open（Ｖ）…"), "Open…");
        assert_eq!(menu_label("Pick (B)"), "Pick");
        assert_eq!(menu_label("Pick (F)"), "Pick");
        assert_eq!(menu_label("한글 문서(Hwp) 인쇄(P)"), "한글 문서(Hwp) 인쇄");
        assert_eq!(menu_label("\"a.zip\" に圧縮"), "\"…\" に圧縮");
        assert_eq!(menu_label("\"b.zip\" に圧縮"), "\"…\" に圧縮");
        assert_eq!(menu_label("「a.zip」に圧縮"), "「…」に圧縮");
        assert_eq!(menu_label("\"a.zip\""), "");
    }

    #[test]
    fn product_titles_prefer_version_info_then_known_vendors() {
        let prev = crate::UI.load(std::sync::atomic::Ordering::Relaxed);
        crate::UI.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(friendly_name("Bandizip", "BdzShell", &["BdzShell"]), "Bandizip");
        assert_eq!(friendly_name("Bandizip (V)", "", &["BdzShell"]), "Bandizip");
        assert_eq!(friendly_name("Bandizip Shell Extension", "", &["BdzShell"]), "Bandizip");
        assert_eq!(known_product("BdzShell").as_deref(), Some("Bandizip"));
        assert_eq!(known_product("LHShellExt").as_deref(), Some("LockHunter"));
        assert_eq!(known_product("SynoFolderContextMenu Class").as_deref(), Some("Synology Drive"));
        assert_eq!(known_product("HVShell Module").as_deref(), Some("Honeyview"));
        assert_eq!(friendly_name("", "", &["HVShell Module"]), "Honeyview");
        crate::UI.store(1, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(known_product("BdzShell").as_deref(), Some("반디집"));
        assert_eq!(known_product("AABdzCtx Class").as_deref(), Some("반디집"));
        assert_eq!(friendly_name("BdzShell", "", &["BdzShell"]), "반디집");
        assert_eq!(known_product("HVShell Module").as_deref(), Some("허니뷰"));
        // A known product follows the UI language. An unknown ProductName stays as written.
        assert_eq!(friendly_name("Bandizip", "", &["BdzShell"]), "반디집");
        assert_eq!(friendly_name("Bandizip shell menu dll", "", &["BdzShell"]), "반디집");
        assert_eq!(friendly_name("Something Else", "", &["BdzShell"]), "Something Else");
        crate::UI.store(prev, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn product_titles_drop_menu_sentences_and_library_suffixes() {
        let prev = crate::UI.load(std::sync::atomic::Ordering::Relaxed);
        crate::UI.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(polish_name("Bandizip shell menu dll"), "Bandizip");
        assert_eq!(polish_name("WinMerge Explorer Extension Library"), "WinMerge");
        assert_eq!(polish_name("Foo Shell Integration library"), "Foo");
        assert_eq!(polish_name("Explorer Extension Library"), "");
        assert_eq!(polish_name("Bandizip's"), "Bandizip");
        assert_eq!(polish_name("Bandizip’s"), "Bandizip");
        assert_eq!(polish_name("Microsoft®Office"), "Microsoft Office");
        assert_eq!(polish_name("Bandizip®"), "Bandizip");
        assert_eq!(polish_name("Foo ® bar"), "Foo bar");
        assert_eq!(known_product("Open in Excel").as_deref(), Some("Microsoft Office"));
        assert_eq!(known_product("EXCEL").as_deref(), Some("Microsoft Office"));
        assert_eq!(known_product("Microsoft Word").as_deref(), Some("Microsoft Office"));
        assert_eq!(known_product("WinMerge").as_deref(), Some("WinMerge"));
        assert_eq!(known_product("WinMerge Explorer Extension Library").as_deref(), Some("WinMerge"));
        assert_eq!(known_product("wscript").as_deref(), Some("Windows Script Host"));
        assert_eq!(known_product("Windows Script Host").as_deref(), Some("Windows Script Host"));
        assert_eq!(known_product("한글 문서(Hwp) 인쇄(P)").as_deref(), Some("한글(Hwp)"));
        assert_eq!(known_product("Hwp").as_deref(), Some("한글(Hwp)"));
        assert!(known_product("Excellent Viewer").is_none());
        assert!(known_product("WordPad").is_none());
        assert!(known_product("Configure").is_none());
        assert!(menu_sentence("Configure"));
        assert!(menu_sentence("Open in Excel"));
        assert!(menu_sentence("한글 문서(Hwp) 인쇄(P)"));
        assert!(!menu_sentence("한글(Hwp)"));
        assert!(!menu_sentence("Microsoft Office"));
        assert_eq!(friendly_name("", "", &["Configure"]), "");
        assert_eq!(friendly_name("", "", &["Open in Excel"]), "Microsoft Office");
        assert_eq!(friendly_name("", "", &["한글 문서(Hwp) 인쇄(P)"]), "한글(Hwp)");
        assert_eq!(friendly_name("Microsoft Excel", "", &["excel"]), "Microsoft Office");
        assert_eq!(friendly_name("LockHunter", "", &["LHShellExt"]), "LockHunter");
        let configure = Row {
            id: "verb:configure".into(),
            name: "Configure".into(),
            verb: "configure".into(),
            product: String::new(),
            raw: "Configure".into(),
            targets: FILE,
            builtin: false,
            clsid: None,
            command: String::new(),
            rank: 1,
            extended: false,
            single: false,
            position: 0,
            classes: Vec::new(),
            anchor: String::new(),
            static_labels: vec!["Configure".into()],
        };
        assert_eq!(title_of(&configure), "");
        assert!(title_junk("%1"));
        assert!(title_junk("%*"));
        assert!(title_junk("%V"));
        assert!(title_junk("%L"));
        assert!(title_junk(r#""C:\App\a.exe" "%1""#));
        assert!(title_junk(r"C:\Windows\notepad.exe"));
        assert!(!title_junk("Configure"));
        assert!(!title_junk("구성"));
        assert!(!title_junk("Microsoft Office"));
        assert!(!title_junk("한글(Hwp)"));
        assert_eq!(version_target("%1"), "");
        assert_eq!(version_target("%*"), "");
        assert_eq!(version_target(r#""C:\App\a.exe" "%1""#), r"C:\App\a.exe");
        assert_eq!(version_target(r"rundll32.exe %1"), "");
        assert_eq!(version_target(r#"C:\Windows\System32\rundll32.exe "C:\App\a.dll",Entry %1"#), r"C:\App\a.dll");
        assert_eq!(friendly_name("", "", &["%1", "Configure"]), "");
        assert_eq!(verb_product("", "", "%1", "Configure", "config", ""), "Configure");
        assert_eq!(verb_product("", "", "%1", "구성", "config", ""), "구성");
        assert_eq!(verb_product("", "", "%1", "Open in Excel", "config", ""), "Microsoft Office");
        assert_eq!(verb_product("", "", "%1", "Configure", "config", "LockHunter"), "LockHunter");
        assert_eq!(verb_product("Bandizip", "", "%1", "Configure", "config", ""), "Bandizip");
        assert!(better_title("%1", "Bandizip"));
        assert!(better_title("Configure", "Bandizip"));
        assert!(!better_title("Bandizip", "Configure"));
        assert!(!better_title("Bandizip", "%1"));
        let placeholder = Row { product: "%1".into(), raw: "%1".into(), ..configure.clone() };
        assert_eq!(title_of(&placeholder), "");
        let named = Row { product: "Configure".into(), ..configure };
        assert_eq!(title_of(&named), "Configure");
        crate::UI.store(1, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(known_product("wscript").as_deref(), Some("Windows 스크립트 호스트"));
        assert_eq!(known_product("Hwp").as_deref(), Some("한글(Hwp)"));
        crate::UI.store(prev, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn repeated_labels_collapse_and_the_same_product_is_one_toggle() {
        let mut slot = Vec::new();
        assert!(merge_labels(&mut slot, &["Visual Studio で開く(V)".into(), "Visual Studio で開く".into()]));
        assert_eq!(slot, vec!["Visual Studio で開く".to_string()]);
        assert!(merge_labels(&mut slot, &["\"a.zip\" に圧縮".into(), "\"b.zip\" に圧縮".into()]));
        assert_eq!(slot, vec!["Visual Studio で開く".to_string(), "\"…\" に圧縮".to_string()]);
        let mut saved = Vec::new();
        assert!(!merge_labels(&mut saved, &["\"a.zip\"".into(), "\"b.zip\"".into()]));
        assert!(saved.is_empty());

        let excel = sample_row("verb:excel", "Open in Excel", "Microsoft Office", "excel", FILE);
        let word = sample_row("verb:word", "Open in Word", "Microsoft Office", "word", FILE | FOLDER);
        let configure = sample_row("verb:configure", "Configure", "", "configure", FILE);
        let rows = vec![excel, word, configure];
        let ui = assemble(&rows, &BTreeMap::new(), &BTreeMap::new(), &BTreeMap::new(), "");
        assert_eq!(ui.len(), 2);
        assert_eq!(ui[0].name, "Microsoft Office");
        assert_eq!(ui[0].ids, vec!["verb:excel".to_string(), "verb:word".to_string()]);
        assert!(ui[0].detail.contains("Open in Excel"));
        assert!(ui[0].detail.contains("Open in Word"));
        assert_eq!(ui[0].tick, Tick::On);
        assert_eq!(ui[1].name, "");
        assert_eq!(ui[1].ids, vec!["verb:configure".to_string()]);
        assert_eq!(ui[1].detail, "Configure");

        let mut off = BTreeMap::new();
        let mut on = BTreeMap::new();
        toggle_ids_master(&mut off, &mut on, &ui[0].ids, &ui[0].scopes);
        assert_eq!(bits_of(&off["verb:excel"]), FILE);
        assert_eq!(bits_of(&off["verb:word"]), FILE | FOLDER);
        assert_eq!(assemble(&rows, &off, &on, &BTreeMap::new(), "")[0].tick, Tick::Off);
        toggle_ids_target(&mut off, &mut on, &ui[0].ids, &ui[0].scopes, FILE);
        assert!(!off.contains_key("verb:excel"));
        assert_eq!(bits_of(&off["verb:word"]), FOLDER);
        write_bits(&mut off, "verb:excel", FILE);
        let mixed = assemble(&rows, &off, &on, &BTreeMap::new(), "");
        assert_eq!(mixed[0].tick, Tick::Mixed);
        assert_eq!(mixed[0].subs[0], Tick::Mixed);
        toggle_ids_master(&mut off, &mut on, &mixed[0].ids, &mixed[0].scopes);
        assert_eq!(bits_of(off.get("verb:excel").map(Vec::as_slice).unwrap_or(&[])) & (FILE | FOLDER), 0);
        assert_eq!(bits_of(off.get("verb:word").map(Vec::as_slice).unwrap_or(&[])) & (FILE | FOLDER), 0);
    }

    fn sample_row(id: &str, name: &str, product: &str, verb: &str, targets: u8) -> Row {
        Row {
            id: id.into(),
            name: name.into(),
            targets,
            builtin: false,
            clsid: None,
            verb: verb.into(),
            command: String::new(),
            rank: 1,
            extended: false,
            single: false,
            position: 0,
            classes: Vec::new(),
            anchor: String::new(),
            product: product.into(),
            raw: name.into(),
            static_labels: if verb.is_empty() { Vec::new() } else { vec![name.into()] },
        }
    }

    #[test]
    fn menu_labels_merge_once_and_drop_a_repeated_title() {
        let mut slot = Vec::new();
        assert!(merge_labels(&mut slot, &["&반디집으로 압축하기 (A)".into(), "SVN 업데이트".into()]));
        assert!(!merge_labels(&mut slot, &["반디집으로 압축하기".into()]));
        assert_eq!(slot, vec!["반디집으로 압축하기".to_string(), "SVN 업데이트".to_string()]);
        let row = Row {
            id: "clsid:x".into(),
            name: "반디집".into(),
            targets: FILE,
            builtin: false,
            clsid: None,
            verb: String::new(),
            command: String::new(),
            rank: 1,
            extended: false,
            single: false,
            position: 0,
            classes: Vec::new(),
            anchor: String::new(),
            product: "반디집".into(),
            raw: "BdzShell".into(),
            static_labels: Vec::new(),
        };
        let mut seen = BTreeMap::new();
        let pending = present_row(&row, &seen);
        assert!(pending.pending);
        assert_eq!(pending.name, "반디집");
        assert_eq!(pending.raw, "BdzShell");
        assert!(pending.labels.is_empty());
        seen.insert("clsid:x".into(), slot);
        let shown = present_row(&row, &seen);
        assert!(!shown.pending);
        assert_eq!(shown.labels.join(" · "), "반디집으로 압축하기 · SVN 업데이트");
        assert_eq!(shown.raw, "BdzShell");
        let verb = Row {
            name: "7-Zip".into(),
            product: "7-Zip".into(),
            raw: "7-Zip".into(),
            verb: "7-Zip".into(),
            static_labels: vec!["7-Zip".into()],
            ..row
        };
        let quiet = present_row(&verb, &BTreeMap::new());
        assert!(!quiet.pending);
        assert!(quiet.labels.is_empty(), "a label that repeats the title is not shown again");
        assert!(quiet.raw.is_empty());
    }

    fn com_row(id: &str, product: &str, raw: &str) -> Row {
        Row {
            id: id.into(),
            name: product.into(),
            targets: FILE | FOLDER,
            builtin: false,
            clsid: parse_guid("{11111111-2222-3333-4444-555555555555}"),
            verb: String::new(),
            command: String::new(),
            rank: 1,
            extended: false,
            single: false,
            position: 0,
            classes: Vec::new(),
            anchor: String::new(),
            product: product.into(),
            raw: raw.into(),
            static_labels: Vec::new(),
        }
    }

    fn shown(rows: &[Row], off: &BTreeMap<String, Vec<String>>, on: &BTreeMap<String, Vec<String>>, seen: &BTreeMap<String, Vec<String>>, quiet: &BTreeSet<String>) -> Vec<UiRow> {
        let kept: Vec<Row> = rows.iter().filter(|r| listed(r, seen, quiet)).cloned().collect();
        assemble(&kept, off, on, seen, "")
    }

    #[test]
    fn handlers_that_add_no_menu_item_leave_the_list() {
        let os = com_row("clsid:os", "Microsoft Windows Operating System", "ShellExt");
        let saver = com_row("clsid:saver", "Screen saver", "ScrMenu");
        let zip = sample_row("verb:7zip", "7-Zip", "7-Zip", "7-Zip", FILE);
        let band = Row {
            raw: "BdzShell".into(),
            static_labels: vec!["반디집으로 압축하기".into()],
            ..com_row("clsid:bdz", "반디집", "BdzShell")
        };
        let mut quiet = BTreeSet::new();
        quiet.insert(os.id.clone());
        quiet.insert(saver.id.clone());
        let seen = BTreeMap::new();
        let ui = shown(&[os.clone(), saver.clone(), zip.clone(), band.clone()], &BTreeMap::new(), &BTreeMap::new(), &seen, &quiet);
        assert!(ui.iter().all(|r| r.name != "Microsoft Windows Operating System"));
        assert!(ui.iter().all(|r| r.name != "Screen saver"));
        assert!(ui.iter().any(|r| r.name == "7-Zip"));
        let kept = ui.iter().find(|r| r.name == "반디집").unwrap();
        assert_eq!(kept.detail, "반디집으로 압축하기");
        assert_eq!(kept.raw, "BdzShell");

        // Hidden before any probe. A third-party handler with no learned label stays listed.
        let pending = shown(&[os.clone(), saver.clone()], &BTreeMap::new(), &BTreeMap::new(), &seen, &BTreeSet::new());
        assert!(pending.is_empty());
        let syno = com_row("clsid:syno", "Synology Drive", "SynoFolderContextMenu Class");
        let waiting = shown(&[syno], &BTreeMap::new(), &BTreeMap::new(), &seen, &BTreeSet::new());
        assert_eq!(waiting.len(), 1);
        assert!(waiting[0].detail.is_empty());
        assert_eq!(waiting[0].raw, "SynoFolderContextMenu Class");

        // A screen-saver verb still has registry text. It stays hidden until a real menu shows it.
        let saver_verb = sample_row("verb:screensaver", "Screen saver", "Microsoft Windows Operating System", "screensaver", BACKGROUND);
        assert!(shell_dormant(&saver_verb));
        assert!(!listed(&saver_verb, &BTreeMap::new(), &BTreeSet::new()));

        // A later menu that does add an item brings the handler back.
        let mut seen = BTreeMap::new();
        seen.insert(os.id.clone(), vec!["Something visible".into()]);
        let back = shown(&[os, saver], &BTreeMap::new(), &BTreeMap::new(), &seen, &quiet);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].name, "Microsoft Windows Operating System");
        assert_eq!(back[0].detail, "Something visible");
        assert_eq!(back[0].raw, "ShellExt");
        assert!(listed(&zip, &BTreeMap::new(), &{
            let mut q = BTreeSet::new();
            q.insert(zip.id.clone());
            q
        }));
    }

    #[test]
    fn winmerge_is_one_row_and_starts_off() {
        let shell = Row {
            id: WINMERGE_CLSID.into(),
            name: "WinMerge".into(),
            product: "WinMerge".into(),
            raw: "WinMerge".into(),
            targets: FILE | FOLDER | BACKGROUND | DRIVE,
            ..com_row(WINMERGE_CLSID, "WinMerge", "WinMerge")
        };
        let app = app_winmerge_row();
        let off = BTreeMap::new();
        let on = BTreeMap::new();
        let rows = [shell.clone(), app];
        let ui = assemble(&rows, &off, &on, &BTreeMap::new(), "");
        assert_eq!(ui.len(), 1);
        assert_eq!(ui[0].name, "WinMerge");
        assert!(ui[0].ids.iter().any(|id| id == APP_WINMERGE));
        assert!(ui[0].ids.iter().any(|id| id.eq_ignore_ascii_case(WINMERGE_CLSID)));
        assert_eq!(ui[0].tick, Tick::Off);
        let labels = crate::winmerge::context_labels();
        assert!(ui[0].detail.contains(&labels[0]));
        assert!(ui[0].detail.contains(&labels[1]));
        assert!(ui[0].raw.is_empty(), "the internal name repeats the title, so it stays hidden");
        assert!(blocked(&shell, FILE, &off, &on));
        assert!(!blocked(&sample_row("verb:7zip", "7-Zip", "7-Zip", "7-Zip", FILE), FILE, &off, &on));

        let mut on = BTreeMap::new();
        toggle_ids_master(&mut BTreeMap::new(), &mut on, &ui[0].ids, &ui[0].scopes);
        assert_eq!(bits_of(&on[APP_WINMERGE]), TARGET_ALL);
        assert!(on.keys().all(|id| id == APP_WINMERGE));
        let enabled = assemble(&rows, &BTreeMap::new(), &on, &BTreeMap::new(), "");
        assert_eq!(enabled[0].tick, Tick::On);
        assert!(!blocked(&shell, FILE | FOLDER, &BTreeMap::new(), &on));

        toggle_ids_target(&mut BTreeMap::new(), &mut on, &ui[0].ids, &ui[0].scopes, FILE);
        assert_eq!(bits_of(&on[APP_WINMERGE]), TARGET_ALL & !FILE);
        assert_eq!(assemble(&rows, &BTreeMap::new(), &on, &BTreeMap::new(), "")[0].tick, Tick::Mixed);

        let mut reset_off = BTreeMap::new();
        let mut reset_on = BTreeMap::new();
        reset_menu(&mut reset_off, &mut reset_on);
        assert!(reset_off.is_empty());
        assert_eq!(bits_of(&reset_on[APP_WINMERGE]), TARGET_ALL);
    }

    #[test]
    fn an_older_winmerge_off_list_becomes_an_explicit_on() {
        let mut off = BTreeMap::new();
        let mut on = BTreeMap::new();
        off.insert(WINMERGE_CLSID.into(), vec!["file".into()]);
        off.insert("clsid:other".into(), vec!["folder".into()]);
        reconcile_winmerge(&mut off, &mut on);
        assert!(!off.contains_key(WINMERGE_CLSID));
        assert_eq!(bits_of(&off["clsid:other"]), FOLDER);
        assert_eq!(bits_of(&on[APP_WINMERGE]), TARGET_ALL & !FILE);

        let mut off = BTreeMap::new();
        let mut on = BTreeMap::new();
        off.insert("verb:winmerge".into(), names(TARGET_ALL));
        reconcile_winmerge(&mut off, &mut on);
        assert!(off.is_empty());
        assert!(on.is_empty(), "fully off stays the new default");

        let mut off = BTreeMap::new();
        let mut on = BTreeMap::new();
        write_bits(&mut on, APP_WINMERGE, FILE | FOLDER);
        off.insert(WINMERGE_CLSID.into(), vec!["drive".into()]);
        reconcile_winmerge(&mut off, &mut on);
        assert!(off.is_empty());
        assert_eq!(bits_of(&on[APP_WINMERGE]), FILE | FOLDER, "an explicit on wins over the old off list");

        let mut off = BTreeMap::new();
        let mut on = BTreeMap::new();
        reconcile_winmerge(&mut off, &mut on);
        assert!(on.is_empty());
    }

    #[test]
    fn the_filtered_key_builder_stays_compiled() {
        let _compose: unsafe fn(HWND, &IShellFolder, &[*const ITEMIDLIST], u8, &[String]) -> Option<IContextMenu> = compose;
        let _paths: unsafe fn(HWND, &[(String, bool)], u8) -> Option<IContextMenu> = compose_paths;
    }

    #[test]
    fn menu_keys_fold_accelerators_without_merging_distinct_commands() {
        assert_eq!(menu_match_key("左側として選択(&L)"), menu_match_key("左側として選択(L)"));
        assert_eq!(menu_match_key("比較..."), menu_match_key("比較…"));
        assert_eq!(menu_match_key("Visual Studio で開く(&V)"), menu_match_key("Visual Studio で開く(V)"));
        assert_ne!(menu_match_key("比較"), menu_match_key("比較..."));
        assert_eq!(menu_match_key("Select Left"), "select left");
        assert!(generic_windows_product("Microsoft® Windows® Operating System"));
        assert!(!generic_windows_product("Microsoft Office"));
        assert!(!generic_windows_product("WinMerge"));
        assert!(screen_saver_title("Screen saver"));
        assert!(screen_saver_title("Screensaver"));
        assert!(screen_saver_title("スクリーンセーバー"));
        assert!(!screen_saver_title("Screen"));
        assert!(!shell_dormant(&com_row(WINMERGE_CLSID, "WinMerge", "WinMerge")));
    }

    #[test]
    fn a_selection_is_probed_without_counting_the_parent_folder() {
        assert_eq!(probe_contracts(true, true), vec![ProbeContract::Selection, ProbeContract::FolderAndData]);
        assert_eq!(probe_contracts(true, false), vec![ProbeContract::Folder]);
        assert_eq!(probe_contracts(false, true), vec![ProbeContract::Selection]);
        assert!(probe_contracts(false, false).is_empty());
    }

    #[test]
    fn disabled_handlers_drop_inserted_text_in_the_ui_language() {
        // What the handler inserted on a one-item menu, in Japanese.
        let inserted = vec![menu_match_key("左側として選択(&L)"), menu_match_key("比較...")];
        assert!(item_dropped("", "左側として選択(L)", &[], &inserted));
        assert!(item_dropped("", "比較…", &[], &inserted));
        // The two-path probe inserts a different command. That string does not remove the ellipsis item.
        let two_paths = vec![menu_match_key("比較")];
        assert!(!item_dropped("", "比較...", &[], &two_paths));
        assert!(!item_dropped("", "左側として選択(L)", &[], &two_paths));
        // English catalogue text is not a stand-in for the inserted Japanese rows.
        let english = vec![menu_match_key("WinMerge"), menu_match_key("Select Left"), menu_match_key("Compare...")];
        assert!(!item_dropped("", "左側として選択(L)", &[], &english));
        assert!(!item_dropped("", "比較...", &[], &english));
        // A resolved MuiVerb does remove the localized row, and the registry verb does too.
        let mui = vec![menu_match_key("比較...")];
        assert!(item_dropped("", "比較...", &[], &mui));
        assert!(item_dropped("WinMerge", "比較...", &["winmerge".into()], &[]));
        // Neighbouring commands and the shell's own verbs stay.
        assert!(!item_dropped("", "SVN Checkout...", &[], &inserted));
        assert!(!item_dropped("properties", "プロパティ", &[], &inserted));
        assert!(!item_dropped("open", "開く(O)", &["open".into()], &inserted));
        // Maguro's own WinMerge commands sit at 0x8020 and are not shell rows.
        assert!(maguro_menu_id(0x8020));
        assert!(maguro_menu_id(0x8021));
        assert!(!maguro_menu_id(1));
        assert!(!maguro_menu_id(u32::MAX));
    }

    fn kept_separators(items: &[SepMark]) -> Vec<SepMark> {
        let sep: Vec<bool> = items.iter().map(|m| *m == SepMark::Separator).collect();
        let keep = separator_keep(&sep);
        items.iter().zip(keep).filter(|(_, k)| *k).map(|(m, _)| *m).collect()
    }

    #[test]
    fn separators_drop_a_leading_trailing_or_repeated_bar() {
        use SepMark::*;
        assert_eq!(normalize_separators(&[]), Vec::<SepMark>::new());
        assert_eq!(normalize_separators(&[Separator]), Vec::<SepMark>::new());
        assert_eq!(normalize_separators(&[Separator, Separator]), Vec::<SepMark>::new());
        assert_eq!(normalize_separators(&[Item]), vec![Item]);
        assert_eq!(normalize_separators(&[Item, Item]), vec![Item, Item]);
        assert_eq!(normalize_separators(&[Separator, Item]), vec![Item]);
        assert_eq!(normalize_separators(&[Item, Separator]), vec![Item]);
        assert_eq!(normalize_separators(&[Separator, Item, Separator]), vec![Item]);
        assert_eq!(normalize_separators(&[Item, Separator, Separator, Item]), vec![Item, Separator, Item]);
        assert_eq!(
            normalize_separators(&[Separator, Separator, Item, Separator, Separator, Item, Separator]),
            vec![Item, Separator, Item]
        );
        // The index map keeps the same rows the pure pass keeps, including the first bar of a run.
        let messy = [Separator, Item, Separator, Separator, Item, Separator];
        assert_eq!(kept_separators(&messy), normalize_separators(&messy));
        assert_eq!(kept_separators(&messy), vec![Item, Separator, Item]);
    }

    #[test]
    fn one_new_folder_row_and_explorer_wins_when_both_exist() {
        use NewFolderSlot::*;
        let shell = NewFolder { maguro: false };
        let own = NewFolder { maguro: true };
        let other = Other;
        assert_eq!(keep_new_folder(&[other, shell, other, shell]), vec![true, true, true, false]);
        assert_eq!(keep_new_folder(&[own, other, shell]), vec![false, true, true]);
        assert_eq!(keep_new_folder(&[own, other, own]), vec![true, true, false]);
        assert_eq!(keep_new_folder(&[shell]), vec![true]);
        assert_eq!(keep_new_folder(&[other, other]), vec![true, true]);
        assert!(is_new_folder_item("NewFolder", ""));
        assert!(is_new_folder_item("windows.newfolder", ""));
        assert!(is_new_folder_item("", "새 폴더(&N)"));
        assert!(is_new_folder_item("", "새 폴더(N)"));
        assert!(is_new_folder_item("", "新しいフォルダー(&F)"));
        assert!(is_new_folder_item("", "New &folder"));
        assert!(!is_new_folder_item("paste", "貼り付け(&P)"));
        assert!(!is_new_folder_item("properties", "プロパティ(&R)"));
        assert!(!is_new_folder_item("new", "新規作成(&W)"));
        assert!(!is_new_folder_item("", "フォルダー(&F)"));
    }

    /// Background menu from the report: New folder at the top and again under Paste,
    /// a repeated bar above Paste, and a bar after Properties.
    #[test]
    fn background_menu_keeps_one_new_folder_and_drops_dangling_bars() {
        use NewFolderSlot::*;
        use SepMark::*;
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum Slot {
            Sep,
            Item,
            New,
            Own,
        }
        let slots = [
            Slot::New,  // top 새 폴더
            Slot::Sep,
            Slot::Sep,  // empty bar above 貼り付け
            Slot::Item, // 貼り付け
            Slot::Sep,
            Slot::New,  // 새 폴더 again
            Slot::Sep,
            Slot::Item, // プロパティ
            Slot::Sep,  // bar after the last item
        ];
        let folder: Vec<NewFolderSlot> = slots
            .iter()
            .map(|s| match s {
                Slot::New => NewFolder { maguro: false },
                Slot::Own => NewFolder { maguro: true },
                _ => Other,
            })
            .collect();
        let left: Vec<Slot> = slots.iter().zip(keep_new_folder(&folder)).filter(|(_, k)| *k).map(|(s, _)| *s).collect();
        let marks: Vec<SepMark> = left.iter().map(|s| if *s == Slot::Sep { Separator } else { Item }).collect();
        let keep = separator_keep(&marks.iter().map(|m| *m == Separator).collect::<Vec<_>>());
        let tidy: Vec<Slot> = left.into_iter().zip(keep).filter(|(_, k)| *k).map(|(s, _)| s).collect();
        assert_eq!(tidy, vec![Slot::New, Slot::Sep, Slot::Item, Slot::Sep, Slot::Item]);

        // Explorer already has the command, so Maguro's own row is the one that goes.
        let both = [Slot::Own, Slot::Sep, Slot::New, Slot::Sep];
        let folder: Vec<NewFolderSlot> = both
            .iter()
            .map(|s| match s {
                Slot::New => NewFolder { maguro: false },
                Slot::Own => NewFolder { maguro: true },
                _ => Other,
            })
            .collect();
        let left: Vec<Slot> = both.iter().zip(keep_new_folder(&folder)).filter(|(_, k)| *k).map(|(s, _)| *s).collect();
        let marks: Vec<SepMark> = left.iter().map(|s| if *s == Slot::Sep { Separator } else { Item }).collect();
        let keep = separator_keep(&marks.iter().map(|m| *m == Separator).collect::<Vec<_>>());
        let tidy: Vec<Slot> = left.into_iter().zip(keep).filter(|(_, k)| *k).map(|(s, _)| s).collect();
        assert_eq!(tidy, vec![Slot::New]);
    }
}
#[cfg(test)]
mod utf8_boundary_tests {
    #[test]
    fn japanese_korean_and_emoji_names_do_not_panic() {
        assert_eq!(super::file_stem(r"샘플\한글폴더.dll"), "한글폴더");
        assert_eq!(super::file_stem(r"F:\CAC\サンプル.exe"), "サンプル");
        assert_eq!(super::file_stem("😀.dll"), "😀");
        assert_eq!(super::file_stem("abcdサ.txt"), "abcdサ.txt");
        let (exe, args) = super::split_cmd(r#""D:\한글폴더\app.exe" /x"#);
        assert_eq!((exe.as_str(), args.as_str()), (r"D:\한글폴더\app.exe", "/x"));
        let (exe, args) = super::split_cmd(r#""E:\📁\샘플.exe""#);
        assert_eq!(exe, r"E:\📁\샘플.exe");
        assert!(args.is_empty());
        assert!(super::foreign_path(r"c:\한글폴더\app.exe"));
        assert!(super::foreign_path(r"d:\サンプル\😀.exe"));
        assert!(!super::foreign_path(r"c:\windows\system32\サンプル.exe"));
        assert!(super::parse_guid("{7BA4C740-9E81-11CF-99D3-00AA004AE837}").is_some());
        assert!(super::parse_guid("サンプル").is_none());
        assert!(super::parse_guid("한글폴더").is_none());
        assert!(super::parse_guid("😀あ").is_none());
        assert!(super::parse_guid("abcdサ").is_none());
    }
}
