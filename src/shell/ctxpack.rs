// Packaged Explorer context menus (`windows.fileExplorerContextMenus` + PackagedCom).
//
// These registrations are not under `shellex\ContextMenuHandlers`. The classic
// scanner never sees them. Some still appear in the shell menu through other
// Windows 11 paths (Photos, Clipchamp, Copilot, …); `inject_menu` skips any
// title the menu already shows. Settings lists them on the Windows 11 view.
// The popup injects missing enabled commands (`inject_menu`) in Maguro's id range.
//
// The catalogue is built only when that view is laid out, Refresh runs while it
// is showing, or a popup needs to inject. It is not read at process start.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

use windows::Win32::Storage::Packaging::Appx::GetPackagePathByFullName;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_EXPAND_SZ,
    RRF_RT_REG_SZ,
};
use windows::Win32::UI::Shell::ECS_HIDDEN;

use crate::imports::*;

use super::ctxmenu::{self, BACKGROUND, DRIVE, FILE, FOLDER};

const REPO: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

/// First id handed to an injected packaged command. Above favourites, batch rename, and WinMerge.
const ID_CTX_PACK: u32 = 0x8100;
const PACK_LIMIT: u32 = 48;

static CACHE: Mutex<Option<Vec<PackRow>>> = Mutex::new(None);
/// Titles from a prior CoCreate/GetTitle. Inject reuses them and CoCreates only on invoke.
static TITLE_CACHE: Mutex<Option<std::collections::HashMap<String, String>>> = Mutex::new(None);

link!("shlwapi.dll" "system" fn SHLoadIndirectString(src: PCWSTR, dst: PWSTR, cch: u32, reserved: *mut core::ffi::c_void) -> HRESULT);

#[derive(Clone)]
pub(crate) struct PackRow {
    pub(crate) id: String,
    pub(crate) clsid: GUID,
    pub(crate) name: String,
    /// Bits this command applies to. Unknown manifest types count as [`FILE`].
    pub(crate) targets: u8,
    /// Bits that came from a known item type, not from the unknown-as-file fallback.
    known: u8,
    unknown: Vec<String>,
}

pub(crate) struct PackUi {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) detail: String,
    pub(crate) targets: u8,
    pub(crate) on: bool,
}

impl PackRow {
    pub(crate) fn detail(&self) -> String {
        let mut parts = Vec::new();
        for bit in [FILE, FOLDER, BACKGROUND, DRIVE] {
            if self.known & bit != 0 {
                let label = ctxmenu::target_label(bit);
                if !label.is_empty() {
                    parts.push(label.to_string());
                }
            }
        }
        if self.targets & FILE != 0 && self.known & FILE == 0 {
            let label = ctxmenu::target_label(FILE);
            if !label.is_empty() && !parts.iter().any(|p| p == label) {
                parts.insert(0, label.to_string());
            }
        }
        for raw in &self.unknown {
            if !parts.iter().any(|p| p.eq_ignore_ascii_case(raw)) {
                parts.push(raw.clone());
            }
        }
        parts.join(", ")
    }
}

pub(crate) fn invalidate() {
    if let Ok(mut t) = TITLE_CACHE.lock() { *t = None; }
    if let Ok(mut g) = CACHE.lock() {
        *g = None;
    }
}

fn cached() -> Option<Vec<PackRow>> {
    CACHE.lock().ok().and_then(|g| g.clone())
}

/// Stable settings id. `raw` is a CLSID with or without braces.
pub(crate) fn packaged_id(raw: &str) -> Option<String> {
    let g = guid_of(raw)?;
    Some(format!("packaged:{{{}}}", guid_text(&g)))
}

/// `(bit, known)`. Unknown types are [`FILE`] and `known` is false so the raw type can still be listed.
pub(crate) fn type_bits(ty: &str) -> (u8, bool) {
    let norm = ty.trim().replace('/', "\\");
    let low = norm.to_ascii_lowercase();
    let known = match low.as_str() {
        "*" | ".*" => FILE,
        "directory" | "folder" => FOLDER,
        "directory\\background" | "folder\\background" => BACKGROUND,
        "drive" | "drive\\background" => DRIVE,
        _ if low.starts_with('.') && !low.contains('\\') => FILE,
        _ => 0,
    };
    if known == 0 {
        (FILE, false)
    } else {
        (known, true)
    }
}

pub(crate) fn catalogue() -> Vec<PackRow> {
    #[cfg(test)]
    if let Some(rows) = TEST_ROWS.with(|t| t.borrow().clone()) {
        return rows;
    }
    if let Some(rows) = cached() {
        return rows;
    }
    let rows = scan();
    if let Ok(mut g) = CACHE.lock() {
        *g = Some(rows.clone());
    }
    rows
}

/// A one-character query, or a cold cache, does not walk packages.
pub(crate) fn name_hit(q: &str) -> bool {
    let q = q.trim();
    if q.chars().count() < 2 {
        return false;
    }
    let q = q.to_lowercase();
    let Some(rows) = cached() else { return false };
    rows.iter().any(|r| r.name.to_lowercase().contains(&q))
}

pub(crate) fn ui_rows(off: &BTreeSet<String>, q: &str) -> Vec<PackUi> {
    let q = q.trim().to_lowercase();
    let mut rows = catalogue();
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    rows.into_iter()
        .filter(|r| {
            if q.is_empty() {
                return true;
            }
            r.name.to_lowercase().contains(&q) || r.detail().to_lowercase().contains(&q)
        })
        .map(|r| PackUi {
            on: !off.contains(&r.id),
            detail: r.detail(),
            id: r.id,
            name: r.name,
            targets: r.targets,
        })
        .collect()
}

pub(crate) fn toggle(off: &mut BTreeSet<String>, id: &str) {
    if !off.remove(id) {
        off.insert(id.to_string());
    }
}

pub(crate) fn rows_from_xml(xml: &str, display: &str) -> Vec<PackRow> {
    let parsed = parse_manifest(xml);
    let mut out: BTreeMap<String, PackRow> = BTreeMap::new();
    for verb in &parsed.verbs {
        let Some(id) = packaged_id(&verb.clsid) else { continue };
        let Some(class) = parsed.classes.get(&id) else { continue };
        if !class.runnable {
            continue;
        }
        let (bit, known_bit) = type_bits(&verb.item_type);
        let name = row_name(display, &class.display);
        let row = out.entry(id.clone()).or_insert_with(|| PackRow {
            id: id.clone(),
            clsid: guid_of(&verb.clsid).unwrap_or_else(GUID::zeroed),
            name,
            targets: 0,
            known: 0,
            unknown: Vec::new(),
        });
        row.targets |= bit;
        if known_bit {
            row.known |= bit;
        } else {
            let raw = verb.item_type.trim();
            if !raw.is_empty() && row.unknown.len() < 6 && !row.unknown.iter().any(|u| u.eq_ignore_ascii_case(raw)) {
                row.unknown.push(raw.to_string());
            }
        }
    }
    let mut rows: Vec<PackRow> = out.into_values().collect();
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    rows
}

fn row_name(display: &str, server: &str) -> String {
    let shown = indirect(display);
    if human(&shown) {
        return shown;
    }
    let server = indirect(server);
    if human(&server) {
        return server;
    }
    if !display.trim().is_empty() {
        return display.trim().to_string();
    }
    "App".into()
}

struct Verb {
    clsid: String,
    item_type: String,
}

struct ClassInfo {
    display: String,
    runnable: bool,
}

struct Parsed {
    verbs: Vec<Verb>,
    classes: BTreeMap<String, ClassInfo>,
}

struct ServerFrame {
    display: String,
    executable: bool,
}

fn parse_manifest(xml: &str) -> Parsed {
    let mut verbs = Vec::new();
    let mut classes: BTreeMap<String, ClassInfo> = BTreeMap::new();
    let mut stack: Vec<String> = Vec::new();
    let mut servers: Vec<ServerFrame> = Vec::new();
    let mut item_type = String::new();
    let mut i = 0;
    while i < xml.len() {
        let Some(rel) = xml[i..].find('<') else { break };
        i += rel;
        let Some((tag, next)) = read_tag(xml, i) else { break };
        i = next;
        if tag.skip {
            continue;
        }
        if tag.close {
            if stack.last().map(String::as_str) == Some(tag.name.as_str()) {
                if tag.name == "ItemType" {
                    item_type.clear();
                }
                if tag.name == "SurrogateServer" || tag.name == "ExeServer" {
                    servers.pop();
                }
                stack.pop();
            } else if let Some(at) = stack.iter().rposition(|n| n == &tag.name) {
                stack.truncate(at);
                servers.truncate(servers.len().min(stack.iter().filter(|n| *n == "SurrogateServer" || *n == "ExeServer").count()));
                if !stack.iter().any(|n| n == "ItemType") {
                    item_type.clear();
                }
            }
            continue;
        }
        let in_menus = stack.iter().any(|n| n == "FileExplorerContextMenus");
        let in_item = stack.iter().any(|n| n == "ItemType");
        if tag.name == "ItemType" && in_menus {
            item_type = attr(&tag.attrs, "type").unwrap_or("").to_string();
        }
        if tag.name == "Verb" && in_menus && in_item {
            if let Some(clsid) = attr(&tag.attrs, "clsid") {
                if !item_type.is_empty() {
                    verbs.push(Verb { clsid: clsid.to_string(), item_type: item_type.clone() });
                }
            }
        }
        if (tag.name == "SurrogateServer" || tag.name == "ExeServer") && !tag.self_close {
            let display = attr(&tag.attrs, "displayname").unwrap_or("").to_string();
            let executable = attr(&tag.attrs, "executable").is_some_and(|s| !s.is_empty());
            servers.push(ServerFrame { display, executable });
        }
        if tag.name == "Class" && stack.iter().any(|n| n == "SurrogateServer" || n == "ExeServer") {
            if let Some(id_raw) = attr(&tag.attrs, "id") {
                if let Some(id) = packaged_id(id_raw) {
                    let path = attr(&tag.attrs, "path").unwrap_or("");
                    let own_exe = attr(&tag.attrs, "executable").is_some_and(|s| !s.is_empty());
                    let server_exe = servers.last().is_some_and(|s| s.executable);
                    let runnable = !path.is_empty() || own_exe || server_exe;
                    let display = servers.last().map(|s| s.display.clone()).unwrap_or_default();
                    let class = classes.entry(id).or_insert(ClassInfo { display: String::new(), runnable: false });
                    class.runnable |= runnable;
                    if class.display.is_empty() {
                        class.display = display;
                    }
                }
            }
        }
        if !tag.self_close {
            if stack.len() < 64 {
                stack.push(tag.name);
            }
        }
    }
    Parsed { verbs, classes }
}

struct Tag {
    skip: bool,
    close: bool,
    self_close: bool,
    name: String,
    attrs: Vec<(String, String)>,
}

fn read_tag(xml: &str, start: usize) -> Option<(Tag, usize)> {
    let rest = xml.get(start + 1..)?;
    if rest.starts_with("!--") {
        let end = xml[start..].find("-->")? + start + 3;
        return Some((Tag { skip: true, close: false, self_close: false, name: String::new(), attrs: Vec::new() }, end));
    }
    if rest.starts_with('?') {
        let end = xml[start..].find("?>")? + start + 2;
        return Some((Tag { skip: true, close: false, self_close: false, name: String::new(), attrs: Vec::new() }, end));
    }
    let body_end = find_gt(rest)?;
    let next = start + 1 + body_end + 1;
    let mut body = rest[..body_end].trim();
    if body.starts_with('!') {
        return Some((Tag { skip: true, close: false, self_close: false, name: String::new(), attrs: Vec::new() }, next));
    }
    let close = body.starts_with('/');
    if close {
        body = body[1..].trim();
    }
    let self_close = body.ends_with('/');
    if self_close {
        body = body[..body.len() - 1].trim();
    }
    let name_end = body.find(char::is_whitespace).unwrap_or(body.len());
    let name = local_name(&body[..name_end]).to_string();
    let attrs = if name_end < body.len() { parse_attrs(&body[name_end..]) } else { Vec::new() };
    Some((Tag { skip: false, close, self_close, name, attrs }, next))
}

fn find_gt(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut quote = 0u8;
    while i < b.len() {
        let c = b[i];
        if quote != 0 {
            if c == quote {
                quote = 0;
            }
        } else if c == b'"' || c == b'\'' {
            quote = c;
        } else if c == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn local_name(tag: &str) -> &str {
    tag.rsplit(':').next().unwrap_or(tag)
}

fn parse_attrs(body: &str) -> Vec<(String, String)> {
    let b = body.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let start = i;
        while i < b.len() && b[i] != b'=' && !b[i].is_ascii_whitespace() {
            i += 1;
        }
        let key = local_name(&body[start..i]).to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            break;
        }
        i += 1;
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let quote = b[i];
        if quote != b'"' && quote != b'\'' {
            break;
        }
        i += 1;
        let vstart = i;
        while i < b.len() && b[i] != quote {
            i += 1;
        }
        let val = unescape(&body[vstart..i]);
        if i < b.len() {
            i += 1;
        }
        out.push((key, val));
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn scan() -> Vec<PackRow> {
    let mut acc: BTreeMap<String, PackRow> = BTreeMap::new();
    let mut rank: BTreeMap<String, u8> = BTreeMap::new();
    for full in package_names() {
        let Some(folder) = install_path(&full) else { continue };
        let Some(xml) = read_manifest(&folder) else { continue };
        let display = resolve_display(&full, &folder);
        let rows = rows_from_xml(&xml, &display);
        let this = name_rank(&display);
        for row in rows {
            match acc.get_mut(&row.id) {
                Some(old) => {
                    old.targets |= row.targets;
                    old.known |= row.known;
                    for raw in row.unknown {
                        if old.unknown.len() < 6 && !old.unknown.iter().any(|u| u.eq_ignore_ascii_case(&raw)) {
                            old.unknown.push(raw);
                        }
                    }
                    if this > rank.get(&row.id).copied().unwrap_or(0) && human(&row.name) {
                        old.name = row.name;
                        rank.insert(row.id, this);
                    }
                }
                None => {
                    rank.insert(row.id.clone(), this);
                    acc.insert(row.id.clone(), row);
                }
            }
        }
    }
    let mut rows: Vec<PackRow> = acc.into_values().collect();
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.id.cmp(&b.id)));
    rows
}

fn name_rank(name: &str) -> u8 {
    if !human(name) {
        0
    } else if name.contains('_') {
        1
    } else {
        2
    }
}

fn package_names() -> Vec<String> {
    let mut out = BTreeSet::new();
    unsafe {
        for (root, path) in [
            (HKEY_CLASSES_ROOT, r"PackagedCom\Package"),
            (HKEY_CURRENT_USER, r"Software\Classes\PackagedCom\Package"),
            (HKEY_LOCAL_MACHINE, r"SOFTWARE\Classes\PackagedCom\Package"),
        ] {
            let Some(key) = open_at(root, path) else { continue };
            let mut i = 0u32;
            while i < 4000 {
                let Some(name) = enum_key(key, i) else { break };
                i += 1;
                if !name.is_empty() {
                    out.insert(name);
                }
            }
            _ = RegCloseKey(key);
        }
    }
    out.into_iter().collect()
}

fn install_path(full: &str) -> Option<String> {
    if let Some(p) = unsafe { package_path_api(full) } {
        if Path::new(&p).is_dir() {
            return Some(p);
        }
    }
    unsafe { repo_value(full, "PackageRootFolder") }.filter(|p| Path::new(p).is_dir())
}

unsafe fn package_path_api(full: &str) -> Option<String> {
    let wide: Vec<u16> = full.encode_utf16().chain(std::iter::once(0)).collect();
    let name = PCWSTR(wide.as_ptr());
    let mut need = 0u32;
    let _ = GetPackagePathByFullName(name, &mut need, None);
    if need < 2 || need > 2048 {
        return None;
    }
    let mut buf = vec![0u16; need as usize];
    let mut n = need;
    let err = GetPackagePathByFullName(name, &mut n, Some(PWSTR(buf.as_mut_ptr())));
    if err != ERROR_SUCCESS {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let p = String::from_utf16_lossy(&buf[..end]).trim().trim_end_matches('\\').to_string();
    (!p.is_empty()).then_some(p)
}

fn read_manifest(root: &str) -> Option<String> {
    let dir = Path::new(root);
    for rel in ["AppxManifest.xml", r"AppxMetadata\AppxBundleManifest.xml"] {
        let p = dir.join(rel);
        let Ok(meta) = std::fs::metadata(&p) else { continue };
        if meta.len() == 0 || meta.len() > 4 * 1024 * 1024 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        if text.to_ascii_lowercase().contains("fileexplorercontextmenus") {
            return Some(text);
        }
    }
    None
}

fn resolve_display(full: &str, folder: &str) -> String {
    if let Some(raw) = unsafe { repo_value(full, "DisplayName") } {
        let shown = indirect(&raw);
        if human(&shown) {
            return shown;
        }
    }
    let fam = family_name(full);
    if human(&fam) {
        return fam;
    }
    Path::new(folder).file_name().and_then(|s| s.to_str()).filter(|s| human(s)).unwrap_or(&fam).to_string()
}

unsafe fn repo_value(full: &str, value: &str) -> Option<String> {
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let path = format!(r"{REPO}\{full}");
        let Some(key) = open_at(root, &path) else { continue };
        let got = key_sz(key, value);
        _ = RegCloseKey(key);
        if !got.is_empty() {
            return Some(got);
        }
    }
    None
}

fn family_name(full: &str) -> String {
    let parts: Vec<&str> = full.split('_').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        format!("{}_{}", parts[0], parts[parts.len() - 1])
    } else {
        full.to_string()
    }
}

fn human(s: &str) -> bool {
    let t = s.trim();
    !t.is_empty() && !t.starts_with('@') && !t.to_ascii_lowercase().contains("ms-resource:")
}

fn indirect(s: &str) -> String {
    let t = s.trim();
    if !t.starts_with('@') {
        return t.to_string();
    }
    let src: Vec<u16> = t.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 512];
    let hr = unsafe { SHLoadIndirectString(PCWSTR(src.as_ptr()), PWSTR(buf.as_mut_ptr()), buf.len() as u32, std::ptr::null_mut()) };
    if hr.is_err() {
        return t.to_string();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let got = String::from_utf16_lossy(&buf[..n]).trim().to_string();
    if got.is_empty() { t.to_string() } else { got }
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

unsafe fn enum_key(key: HKEY, index: u32) -> Option<String> {
    let mut name = [0u16; 400];
    let mut n = name.len() as u32;
    let err = RegEnumKeyExW(key, index, Some(PWSTR(name.as_mut_ptr())), &mut n, None, None, None, None);
    if err != ERROR_SUCCESS || n == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&name[..n as usize]))
}

unsafe fn key_sz(key: HKEY, name: &str) -> String {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buf = [0u16; 1024];
    let mut n = (buf.len() * 2) as u32;
    let err = RegGetValueW(key, PCWSTR::null(), PCWSTR(wide.as_ptr()), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n));
    if err != ERROR_SUCCESS || n < 2 {
        return String::new();
    }
    let chars = (n as usize / 2).saturating_sub(1).min(buf.len());
    String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').trim().to_string()
}

fn guid_of(raw: &str) -> Option<GUID> {
    let hex: String = raw.chars().filter(|c| c.is_ascii_hexdigit()).map(|c| c.to_ascii_lowercase()).collect();
    if hex.len() != 32 {
        return None;
    }
    let n = u128::from_str_radix(&hex, 16).ok()?;
    let g = GUID::from_u128(n);
    (guid_text(&g) == format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])).then_some(g)
}

fn guid_text(g: &GUID) -> String {
    format!("{g:?}").trim_matches(|c| c == '{' || c == '}').to_ascii_lowercase()
}

thread_local! {
    static PACK_IDS: std::cell::RefCell<Vec<(u32, GUID)>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(crate) fn clear_ids() {
    PACK_IDS.with(|m| m.borrow_mut().clear());
}

/// Append enabled packaged commands that match this click and are not already
/// on the menu under the same title (or the catalogue display name).
///
/// Pure packaged `IExplorerCommand` registrations are often absent from the
/// classic shell menu, so Maguro inserts the title with an id at `0x8100` or
/// above and CoCreates the CLSID only when that item is built or chosen. When
/// Windows already painted the same label (Photos / Clipchamp / Copilot, …),
/// skip the inject so the popup does not grow a duplicate group after Properties.
/// A failed CoCreate, GetTitle, or Invoke skips that one item.
pub(crate) unsafe fn inject_menu(menu: HMENU, paths: &[String], items: &[(String, bool)], background: bool, folder: &str) {
    clear_ids();
    let targets = ctxmenu::click_targets(background, items);
    if targets == 0 {
        return;
    }
    let off = crate::app(|a| a.s.ctx_packaged_off.clone());
    let hits: Vec<PackRow> = catalogue().into_iter().filter(|r| r.targets & targets != 0 && !off.contains(&r.id)).take(PACK_LIMIT as usize).collect();
    if hits.is_empty() {
        return;
    }
    let use_paths: Vec<String> = if background {
        let folder = folder.trim();
        if folder.is_empty() {
            Vec::new()
        } else {
            vec![folder.to_string()]
        }
    } else {
        paths.iter().filter(|p| !p.trim().is_empty()).cloned().collect()
    };
    let arr = shell_array(&use_paths);
    let mut existing = ctxmenu::folded_menu_titles(menu);
    let mut added = 0u32;
    let mut opened = false;
    for row in hits {
        // Catalogue display name often matches the shell label before GetTitle runs.
        if title_already_present(&existing, &row.name) {
            continue;
        }
        let cached = TITLE_CACHE.lock().ok().and_then(|g| g.as_ref().and_then(|c| c.get(&row.id).cloned()));
        let title = if let Some(title) = cached {
            // Safe reuse: title was shown for this command before; CoCreate only on invoke.
            title
        } else {
            let Ok(cmd) = CoCreateInstance::<_, IExplorerCommand>(&row.clsid, None, CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER) else { continue };
            if let Some(state) = command_state(&cmd, arr.as_ref()) {
                if state & ECS_HIDDEN.0 as u32 != 0 {
                    continue;
                }
            }
            let Some(title) = command_title(&cmd, arr.as_ref()) else { continue };
            if let Ok(mut guard) = TITLE_CACHE.lock() {
                guard.get_or_insert_with(std::collections::HashMap::new).insert(row.id.clone(), title.clone());
            }
            title
        };
        if title_already_present(&existing, &title) {
            continue;
        }
        if !opened {
            _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            opened = true;
        }
        let id = ID_CTX_PACK + added;
        added += 1;
        _ = AppendMenuW(menu, MF_STRING, id as usize, &HSTRING::from(&title));
        existing.insert(ctxmenu::fold_menu_label(&title));
        PACK_IDS.with(|m| m.borrow_mut().push((id, row.clsid)));
        if added >= PACK_LIMIT {
            break;
        }
    }
}

fn title_already_present(existing: &std::collections::HashSet<String>, title: &str) -> bool {
    let folded = ctxmenu::fold_menu_label(title);
    !folded.is_empty() && existing.contains(&folded)
}

pub(crate) unsafe fn invoke_menu_command(cmd: u32, paths: &[String], background: bool, folder: &str) -> bool {
    let clsid = PACK_IDS.with(|m| m.borrow().iter().find(|(id, _)| *id == cmd).map(|(_, g)| *g));
    let Some(clsid) = clsid else { return false };
    let use_paths: Vec<String> = if background {
        let folder = folder.trim();
        if folder.is_empty() { Vec::new() } else { vec![folder.to_string()] }
    } else {
        paths.iter().filter(|p| !p.trim().is_empty()).cloned().collect()
    };
    let Some(arr) = shell_array(&use_paths) else { return true };
    if let Ok(command) = CoCreateInstance::<_, IExplorerCommand>(&clsid, None, CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER) {
        _ = command.Invoke(&arr, None::<&IBindCtx>);
    }
    true
}

unsafe fn command_state(cmd: &IExplorerCommand, arr: Option<&IShellItemArray>) -> Option<u32> {
    match arr {
        Some(a) => cmd.GetState(a, false).ok(),
        None => cmd.GetState(None::<&IShellItemArray>, false).ok(),
    }
}

unsafe fn command_title(cmd: &IExplorerCommand, arr: Option<&IShellItemArray>) -> Option<String> {
    let pw = match arr {
        Some(a) => cmd.GetTitle(a).ok()?,
        None => cmd.GetTitle(None::<&IShellItemArray>).ok()?,
    };
    let s = crate::pwstr(pw);
    let s = s.trim().replace(['\r', '\n'], " ");
    if s.is_empty() { None } else { Some(s) }
}

unsafe fn shell_array(paths: &[String]) -> Option<IShellItemArray> {
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    for p in paths.iter().take(64) {
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(p.trim()), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
            pidls.push(pidl);
        }
    }
    if pidls.is_empty() {
        return None;
    }
    let refs: Vec<*const ITEMIDLIST> = pidls.iter().copied().map(|p| p as *const ITEMIDLIST).collect();
    let arr = SHCreateShellItemArrayFromIDLists(&refs).ok();
    for p in pidls {
        CoTaskMemFree(Some(p as _));
    }
    arr
}

#[cfg(test)]
thread_local! {
    static TEST_ROWS: std::cell::RefCell<Option<Vec<PackRow>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_rows(rows: Option<Vec<PackRow>>) {
    TEST_ROWS.with(|t| *t.borrow_mut() = rows);
}

#[cfg(test)]
pub(crate) fn cache_warm() -> bool {
    CACHE.lock().ok().is_some_and(|g| g.is_some())
}

#[cfg(test)]
pub(crate) fn seed_cache(rows: Vec<PackRow>) {
    if let Ok(mut g) = CACHE.lock() {
        *g = Some(rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_already_present_folds_accelerators() {
        let mut set = std::collections::HashSet::new();
        set.insert(ctxmenu::fold_menu_label("속성(&R)"));
        set.insert(ctxmenu::fold_menu_label("Copilot에게 질문하기"));
        assert!(title_already_present(&set, "속성(&R)"));
        assert!(title_already_present(&set, "속성(R)"));
        assert!(title_already_present(&set, "Copilot에게 질문하기"));
        assert!(!title_already_present(&set, "Clipchamp로 편집"));
        assert!(!title_already_present(&set, ""));
    }

    #[test]
    fn packaged_id_is_lowercase_with_braces() {
        assert_eq!(
            packaged_id("{9F156763-7844-4DC4-B2B1-901F640F5155}").as_deref(),
            Some("packaged:{9f156763-7844-4dc4-b2b1-901f640f5155}")
        );
        assert_eq!(
            packaged_id("9f156763-7844-4dc4-b2b1-901f640f5155").as_deref(),
            Some("packaged:{9f156763-7844-4dc4-b2b1-901f640f5155}")
        );
        assert!(packaged_id("not-a-clsid").is_none());
    }

    #[test]
    fn item_types_map_onto_maguro_bits() {
        assert_eq!(type_bits("*"), (FILE, true));
        assert_eq!(type_bits(".py"), (FILE, true));
        assert_eq!(type_bits("Directory"), (FOLDER, true));
        assert_eq!(type_bits(r"Directory\Background"), (BACKGROUND, true));
        assert_eq!(type_bits("Drive"), (DRIVE, true));
        assert_eq!(type_bits(r"Drive\Background"), (DRIVE, true));
        assert_eq!(type_bits("WeirdThing"), (FILE, false));
    }

    #[test]
    fn manifest_groups_one_runnable_clsid_and_drops_the_rest() {
        let xml = r#"
            <Package>
              <com:ComServer>
                <com:SurrogateServer DisplayName="WindowsTerminalShellExt">
                  <com:Class Id="9F156763-7844-4DC4-B2B1-901F640F5155" Path="WindowsTerminalShellExt.dll" ThreadingModel="STA" />
                </com:SurrogateServer>
                <com:ExeServer Executable="WindowsTerminal.exe" DisplayName="WindowsTerminal">
                  <com:Class Id="E12CFF52-A866-4C77-9A90-F570A7AA2C6B" />
                </com:ExeServer>
              </com:ComServer>
              <desktop4:FileExplorerContextMenus>
                <desktop5:ItemType Type="Directory">
                  <desktop5:Verb Id="OpenTerminalHere" Clsid="9f156763-7844-4dc4-b2b1-901f640f5155" />
                </desktop5:ItemType>
                <desktop5:ItemType Type="Directory\Background">
                  <desktop5:Verb Id="OpenTerminalHere" Clsid="9f156763-7844-4dc4-b2b1-901f640f5155" />
                </desktop5:ItemType>
                <desktop5:ItemType Type="WeirdThing">
                  <desktop5:Verb Id="Nope" Clsid="11111111-2222-3333-4444-555555555555" />
                </desktop5:ItemType>
              </desktop4:FileExplorerContextMenus>
            </Package>
        "#;
        let rows = rows_from_xml(xml, "Windows Terminal");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "packaged:{9f156763-7844-4dc4-b2b1-901f640f5155}");
        assert_eq!(rows[0].name, "Windows Terminal");
        assert_eq!(rows[0].targets, FOLDER | BACKGROUND);
        assert!(rows[0].detail().to_lowercase().contains("folder") || !rows[0].detail().is_empty());
    }

    #[test]
    fn invalidate_clears_the_packaged_cache_and_a_cold_name_does_not_scan() {
        seed_cache(Vec::new());
        assert!(cache_warm());
        ctxmenu::invalidate();
        assert!(!cache_warm());
        assert!(!name_hit("terminal"));
        assert!(!cache_warm());
    }
}
