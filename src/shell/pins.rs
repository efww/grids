// Explorer's pinned folders and favourite files, shown as virtual groups above favourites.
// One Quick Access read on a background thread returns both. Neither list is part of favorites_tree.
//
// A favourite file is a Windows 11 Home Favourites item. `unpinfromhome` only changes a
// Quick Access folder pin: it can return success and leave the file in Home, so the row
// must stay until a later Home read no longer lists it.

use std::time::{Duration, Instant};

use crate::app::*;
use crate::imports::*;
use crate::session::*;
use crate::ui::*;
use windows::Win32::System::Variant::{
    VT_BOOL, VT_EMPTY, VT_ERROR, VT_I1, VT_I2, VT_I8, VT_INT, VT_NULL, VT_UI1, VT_UI2, VT_UI4, VT_UI8, VT_UINT,
};

// PROPVARIANT without the StructuredStorage feature: that module is large enough
// that a release build of it is killed on a small machine. Layout is the COM one
// (8-byte header plus a 16-byte value) on x64.
#[repr(C)]
struct PropVar {
    vt: u16,
    w1: u16,
    w2: u16,
    w3: u16,
    data: PropData,
}

#[repr(C)]
union PropData {
    bytes: [u8; 16],
    _align: u64,
}

link!("ole32.dll" "system" fn PropVariantClear(pvar: *mut PropVar) -> HRESULT);

/// IUnknown (3) + IShellItem (5) + the five IShellItem2 methods before GetProperty.
const GET_PROPERTY: usize = 13;

/// Quick Access / Home shell folder. Pinned and frequent items both live here.
const QUICK_ACCESS: &str = "shell:::{679f85cb-0220-4080-b29b-5540cc05aab6}";

/// Windows 11 Home. Favourite files are children of this namespace (or of a group under it).
/// `removefromfavorites` is registered on that shell item, not on the filesystem file.
const HOME: &str = "shell:::{f874310e-b6b7-47dc-bc84-b9e6b38f5903}";

/// System.Home.IsPinned — {30C8EEF4-A832-41E2-AB32-E3C3CA28FD29}, 4 (from PSGetPropertyKeyFromName).
const PKEY_HOME_ISPINNED: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x30C8EEF4_A832_41E2_AB32_E3C3CA28FD29),
    pid: 4,
};

const GPS_HANDLERPROPERTIESONLY: u32 = 0x1;
const GPS_READWRITE: u32 = 0x2;
const GPS_VOLATILEPROPERTIES: u32 = 0x800;
const GPS_VOLATILEPROPERTIESONLY: u32 = 0x1000;
/// `CMINVOKECOMMANDINFOEX.lpVerbW`. The windows crate does not export the flag.
const CMIC_MASK_UNICODE: u32 = 0x0000_4000;
const CMIC_MASK_FLAG_NO_UI: u32 = 0x400;
/// First command id passed to QueryContextMenu. The invoke offset is `id - CMD_FIRST`.
const CMD_FIRST: u32 = 1;
/// Wait, then read Home again, then once more so a row that only blinks off is not treated as gone.
const VERIFY_WAIT: Duration = Duration::from_millis(400);
/// `favorite-remove.log` stays small: one older file is kept when this fills.
const FAV_LOG_MAX: u64 = 32 * 1024;

pub(crate) const PIN_WAIT: Duration = Duration::from_secs(2);
pub(crate) const WM_PINS: u32 = WM_APP + 44;
const PIN_CAP: usize = 256;

struct PinState {
    folders: Vec<Pinned>,
    files: Vec<Pinned>,
    /// A read has finished at least once. Paint uses this so it does not poll.
    loaded: bool,
    last: Option<Instant>,
    /// Folder paths just pinned, kept if a read has not caught up yet.
    fresh_dir: Vec<(String, Instant)>,
    /// File paths just pinned, kept if a read has not caught up yet.
    fresh_file: Vec<(String, Instant)>,
    /// Paths just unpinned, hidden if a read still lists them.
    gone: Vec<(String, Instant)>,
    /// Favourite file that was still listed after a removal, and the panel message.
    note_path: String,
    note: String,
}

const HOLD: Duration = Duration::from_secs(3);

fn fresh_enough(t: Instant, now: Instant) -> bool {
    now.saturating_duration_since(t) < HOLD
}

fn state() -> std::sync::MutexGuard<'static, PinState> {
    static STATE: std::sync::OnceLock<Mutex<PinState>> = std::sync::OnceLock::new();
    STATE
        .get_or_init(|| {
            Mutex::new(PinState {
                folders: Vec::new(),
                files: Vec::new(),
                loaded: false,
                last: None,
                fresh_dir: Vec::new(),
                fresh_file: Vec::new(),
                gone: Vec::new(),
                note_path: String::new(),
                note: String::new(),
            })
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

static GATE: Mutex<()> = Mutex::new(());
static WAKE: Condvar = Condvar::new();
static WANT: AtomicBool = AtomicBool::new(false);
static FORCE: AtomicBool = AtomicBool::new(false);
static AGAIN: AtomicBool = AtomicBool::new(false);
static INFLIGHT: AtomicBool = AtomicBool::new(false);
/// Set when a read finished before any favourites window existed, so the next one invalidates.
static DIRTY: AtomicBool = AtomicBool::new(false);
static TRIES: AtomicUsize = AtomicUsize::new(0);
static SIDES: Mutex<Vec<isize>> = Mutex::new(Vec::new());

const PIN_TRIES: usize = 6;

pub(crate) fn pin_list() -> Vec<Pinned> {
    state().folders.clone()
}

pub(crate) fn pin_files() -> Vec<Pinned> {
    state().files.clone()
}

/// Shown at the bottom of the favourites panel when a file is still in Home after removal.
pub(crate) fn pin_file_note() -> String {
    state().note.clone()
}

pub(crate) fn pins_loaded() -> bool {
    state().loaded
}

pub(crate) fn note_pins_hwnd(h: HWND) {
    let id = h.0 as isize;
    if id == 0 {
        return;
    }
    {
        let mut v = SIDES.lock().unwrap_or_else(|e| e.into_inner());
        if !v.contains(&id) {
            v.push(id);
        }
    }
    // The read can finish before this window exists. The rows are already stored;
    // invalidate so the first paint is not the last one.
    if DIRTY.swap(false, Ordering::AcqRel) {
        unsafe {
            _ = InvalidateRect(Some(h), None, false);
            if !pins_loaded() {
                _ = SetTimer(Some(h), TIMER_PINS, 700, None);
            }
        }
    }
}

pub(crate) fn drop_pins_hwnd(h: HWND) {
    let id = h.0 as isize;
    SIDES.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| *x != id);
}

fn post_pins() {
    let mut posted = 0usize;
    {
        let mut v = SIDES.lock().unwrap_or_else(|e| e.into_inner());
        v.retain(|id| {
            let h = HWND(*id as *mut core::ffi::c_void);
            unsafe {
                if IsWindow(Some(h)).as_bool() {
                    _ = PostMessageW(Some(h), WM_PINS, WPARAM(0), LPARAM(0));
                    posted += 1;
                    true
                } else {
                    false
                }
            }
        });
    }
    DIRTY.store(posted == 0, Ordering::Release);
}

/// Start a Quick Access read. `force` skips the 2s gap (a pin or unpin we just did).
/// A collapsed group is not read unless the other group is open or `force` is set.
/// Never calls the shell on this thread.
pub(crate) fn request_pins(force: bool) {
    if !force && !app(|a| a.s.pins_open || a.s.files_open) {
        return;
    }
    if !force {
        let last = state().last;
        if !pins_due(last, Instant::now(), PIN_WAIT) {
            return;
        }
    }
    let start = {
        let _gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
        if INFLIGHT.load(Ordering::Relaxed) {
            if force {
                AGAIN.store(true, Ordering::Relaxed);
            }
            false
        } else {
            if force {
                FORCE.store(true, Ordering::Relaxed);
            }
            WANT.store(true, Ordering::Relaxed);
            true
        }
    };
    if !start {
        return;
    }
    static WORKER: Once = Once::new();
    WORKER.call_once(|| _ = std::thread::Builder::new().name("explorer-pins".into()).spawn(pin_worker));
    WAKE.notify_one();
}

/// First paint while either group is expanded and no read has finished.
/// Later paints do not re-read; expand and WM_ACTIVATE do.
pub(crate) fn pins_on_paint() {
    if !app(|a| a.s.pins_open || a.s.files_open) || state().loaded {
        return;
    }
    request_pins(false);
}

/// Insert at the front of the folder or file list when it is not already there.
/// False means that list already had it. `file` chooses the favourite-files list.
pub(crate) fn remember_pinned(path: &str, name: &str, file: bool) -> bool {
    let path = tidy_path(path);
    if path.is_empty() {
        return false;
    }
    let mut s = state();
    let listed = if file { &s.files } else { &s.folders };
    if listed.iter().any(|p| p.path.eq_ignore_ascii_case(&path)) {
        return false;
    }
    let name = {
        let n = name.trim();
        if n.is_empty() { short_name(&path) } else { n.to_string() }
    };
    s.gone.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
    if file {
        s.fresh_dir.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
        s.folders.retain(|p| !p.path.eq_ignore_ascii_case(&path));
        s.fresh_file.push((path.clone(), Instant::now()));
        s.files.insert(0, Pinned { path, name });
    } else {
        s.fresh_file.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
        s.files.retain(|p| !p.path.eq_ignore_ascii_case(&path));
        s.fresh_dir.push((path.clone(), Instant::now()));
        s.folders.insert(0, Pinned { path, name });
    }
    true
}

pub(crate) fn forget_pinned(path: &str) {
    let path = tidy_path(path);
    if path.is_empty() {
        return;
    }
    let mut s = state();
    s.fresh_dir.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
    s.fresh_file.retain(|(p, _)| !p.eq_ignore_ascii_case(&path));
    s.gone.push((path.clone(), Instant::now()));
    s.folders.retain(|p| !p.path.eq_ignore_ascii_case(&path));
    s.files.retain(|p| !p.path.eq_ignore_ascii_case(&path));
}

/// True for a real directory. A file that Explorer can also open as a folder (a zip) is not one.
pub(crate) fn shell_is_folder(flags: SFGAO_FLAGS) -> bool {
    flags.contains(SFGAO_FOLDER) && !flags.contains(SFGAO_STREAM)
}

/// `pintohome` on a filesystem item, or `unpinfromhome` on its Quick Access child.
/// A favourite file is not a Quick Access pin. `shell_unfavorite_file` removes those.
pub(crate) fn shell_home_verb(path: &str, pin: bool) -> bool {
    let path = tidy_path(path);
    if path.is_empty() {
        return false;
    }
    if pin {
        return unsafe { invoke_verb(&path, "pintohome") };
    }
    // A filesystem item only offers "Pin to Quick access": `unpinfromhome` lives on the Quick Access child.
    // Invoking it on the plain path did nothing, so the row came back once the 3s hide ran out.
    unsafe { quick_access_child(&path).is_some_and(|item| item_verb(&item, "unpinfromhome")) }
}

/// Take a file out of Windows 11 Home's Favourites. The row is hidden only after a later
/// read no longer lists it.
///
/// Explorer's own item is 「お気に入りから削除」 / "Remove from Favorites" /
/// "즐겨찾기에서 제거", on the Home namespace child
/// (`shell:::{f874310e-b6b7-47dc-bc84-b9e6b38f5903}`), not `unpinfromhome` on the Quick
/// Access child. That folder verb returns success and leaves the file in place, so a
/// guessed name that returns S_OK hid the row for the 3s hold and the next read put it back.
///
/// The Home item's menu is built with `QueryContextMenu(CMF_NORMAL | CMF_EXTENDEDVERBS)`,
/// matched by `GetCommandString(GCS_VERBW)` or by that localized label, and invoked by
/// command id. If the file is still there, `System.Home.IsPinned` is set false through
/// the item's writable property store and committed. That is the write Explorer's command
/// uses to update Home. The jump list `f01b4d95cf55d32a.automaticDestinations-ms` is the
/// on-disk favorites store; it is not edited from here because Explorer keeps it mapped.
/// Verbs, ids, and HRESULTs are appended to `favorite-remove.log` next to maguro.json.
pub(crate) fn shell_unfavorite_file(path: &str) {
    let path = tidy_path(path);
    if path.is_empty() {
        return;
    }
    {
        let mut s = state();
        if s.note_path.eq_ignore_ascii_case(&path) {
            s.note.clear();
            s.note_path.clear();
        }
    }
    let worker_path = path.clone();
    let started = std::thread::Builder::new().name("fav-remove".into()).spawn(move || unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let uninit = hr.is_ok();
        if hr.is_err() {
            diag(&format!("CoInitializeEx {:#010x}", hr.0 as u32));
        }
        remove_home_favorite(&worker_path);
        if uninit {
            CoUninitialize();
        }
    });
    if started.is_err() {
        diag("result=still_present spawn-failed");
        fail_remove(&path);
    }
}

/// What a Home or Quick Access re-read said about the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Listed {
    Absent,
    Present,
    /// The namespace did not enumerate, so removal is not confirmed.
    Unread,
}

/// Hide the row only when both reads actually omitted the file.
fn removal_persisted(home: Listed, quick: Listed) -> bool {
    home == Listed::Absent && quick == Listed::Absent
}

fn canonical_verb(verb: &str) -> String {
    verb.chars().filter(|c| *c != '&' && !c.is_whitespace()).collect::<String>().to_ascii_lowercase()
}

/// Verbs that mean "take this file out of Home Favourites". `unpinfromhome` and a bare
/// `remove` are not here: both return success without editing that list.
fn is_remove_favorite_verb(verb: &str) -> bool {
    matches!(
        canonical_verb(verb).as_str(),
        "removefromfavorites"
            | "removefromfavourite"
            | "removefromfavourites"
            | "unpinfromfavorites"
            | "unpinfromfavourites"
            | "deletefromfavorites"
            | "deletefromfavourites"
    )
}

/// Menu text with accelerators removed: `&Favorites` → `Favorites`, `削除(&R)` → `削除`.
fn menu_label(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
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
    let label = out.trim();
    let open = label.rfind(['(', '（']);
    if let Some(open) = open {
        if open > 0 {
            let Some(tail_src) = label.get(open..) else { return label.to_string() };
            let inner = tail_src.trim_start_matches(['(', '（']).trim_end_matches([')', '）']).trim();
            let tail = tail_src.chars().last();
            if (tail == Some(')') || tail == Some('）')) && inner.chars().count() == 1 {
                return label.get(..open).unwrap_or(label).trim().to_string();
            }
        }
    }
    label.to_string()
}

/// Explorer's own label, in the languages this panel ships, plus the British spelling.
fn is_remove_favorite_text(raw: &str) -> bool {
    let label = menu_label(raw);
    if label.is_empty() {
        return false;
    }
    if label.contains("お気に入りから削除") || label.contains("즐겨찾기에서 제거") {
        return true;
    }
    let low = label.to_lowercase();
    let fav = low.contains("favorite") || low.contains("favourite");
    fav && (low.contains("remove from") || low.contains("unpin from"))
}

struct MenuCmd {
    verb: String,
    text: String,
    id: u32,
    offset: usize,
    gcs: u32,
}

fn cmd_log(cmd: &MenuCmd) -> String {
    format!("cmd id={} off={} verb={:?} gcs={:#010x} text={:?}", cmd.id, cmd.offset, cmd.verb, cmd.gcs, cmd.text)
}

/// Prefer the command whose label is the one Explorer shows. A canonical verb with no
/// label still counts. `delete` / `削除` does not.
fn pick_remove<'a>(cmds: &'a [MenuCmd]) -> Option<&'a MenuCmd> {
    let mut best: Option<(u8, usize)> = None;
    for (i, cmd) in cmds.iter().enumerate() {
        let text = is_remove_favorite_text(&cmd.text);
        let verb = is_remove_favorite_verb(&cmd.verb);
        if !text && !verb {
            continue;
        }
        let name = canonical_verb(&cmd.verb);
        let primary = name == "removefromfavorites" || name == "removefromfavourites";
        let rank = if text && primary {
            0
        } else if text {
            1
        } else if primary {
            2
        } else {
            3
        };
        if best.map(|(r, _)| rank < r).unwrap_or(true) {
            best = Some((rank, i));
        }
    }
    best.map(|(_, i)| &cmds[i])
}

/// `VT_BOOL` / `VARIANT_FALSE`. Explorer writes this, not an integer zero of another type.
fn unpinned_variant() -> (u16, i16) {
    (VT_BOOL.0, 0)
}

fn store_flag_sets() -> &'static [u32] {
    &[
        GPS_READWRITE | GPS_HANDLERPROPERTIESONLY | GPS_VOLATILEPROPERTIES,
        GPS_READWRITE | GPS_HANDLERPROPERTIESONLY,
        GPS_READWRITE | GPS_VOLATILEPROPERTIESONLY,
        GPS_READWRITE,
    ]
}

fn remove_failed_text() -> String {
    crate::tr(crate::Key::CouldNotRemoveThisFileFromExplorer)
    .to_string()
}

fn fail_remove(path: &str) {
    {
        let mut s = state();
        s.note_path = tidy_path(path);
        s.note = remove_failed_text();
    }
    post_pins();
    unsafe { _ = MessageBeep(MB_ICONWARNING) };
}

fn succeed_remove(path: &str) {
    diag("result=removed");
    forget_pinned(path);
    {
        let mut s = state();
        s.note.clear();
        s.note_path.clear();
    }
    post_pins();
    kick_reread();
}

fn kick_reread() {
    let v = SIDES.lock().unwrap_or_else(|e| e.into_inner());
    for id in v.iter() {
        let h = HWND(*id as *mut core::ffi::c_void);
        unsafe {
            if IsWindow(Some(h)).as_bool() {
                _ = SetTimer(Some(h), TIMER_PINS, 700, None);
            }
        }
    }
}

/// The Quick Access child whose filesystem path is `path`.
unsafe fn quick_access_child(path: &str) -> Option<IShellItem> {
    let root = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(QUICK_ACCESS), None).ok()?;
    let items = root.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems).ok()?;
    for _ in 0..PIN_CAP {
        let mut slot = [None];
        _ = items.Next(&mut slot, None);
        let child = slot[0].take()?;
        let p = child.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)).unwrap_or_default();
        if !p.is_empty() && tidy_path(&p).eq_ignore_ascii_case(path) {
            return Some(child);
        }
    }
    None
}

/// Run a canonical verb from the item's own context menu. QueryContextMenu first, as the contract requires.
unsafe fn item_verb(item: &IShellItem, verb: &str) -> bool {
    let Ok(cm) = item.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) else { return false };
    let Ok(menu) = CreatePopupMenu() else { return false };
    let queried = cm.QueryContextMenu(menu, 0, 1, 0x7FFF, CMF_NORMAL).is_ok();
    let ok = queried && invoke_named(&cm, verb);
    _ = DestroyMenu(menu);
    ok
}

unsafe fn invoke_named(cm: &IContextMenu, verb: &str) -> bool {
    let ansi = format!("{verb}\0");
    let ici = CMINVOKECOMMANDINFO {
        cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
        fMask: 0x400, // CMIC_MASK_FLAG_NO_UI
        lpVerb: PCSTR(ansi.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    cm.InvokeCommand(&ici).is_ok()
}

struct NsWalk {
    items: Vec<IShellItem>,
    /// Root or a path-less group did not enumerate, and nothing matched.
    failed: bool,
    notes: Vec<String>,
}

struct HeldMenu {
    cm: IContextMenu,
    menu: HMENU,
    _folder: Option<IShellFolder>,
    rel: *mut ITEMIDLIST,
    cmds: Vec<MenuCmd>,
}

impl Drop for HeldMenu {
    fn drop(&mut self) {
        unsafe {
            _ = DestroyMenu(self.menu);
            if !self.rel.is_null() {
                ILFree(Some(self.rel as *const ITEMIDLIST));
            }
        }
    }
}

unsafe fn remove_home_favorite(path: &str) {
    diag(&format!("remove begin path={path}"));
    diag("store=Home IContextMenu then System.Home.IsPinned; f01b4d95cf55d32a.automaticDestinations-ms is not edited");
    let owner = side_owner();
    let invoked = invoke_matching(path, owner);
    if invoked {
        notify_path(path);
        if !still_there(path) {
            succeed_remove(path);
            return;
        }
        diag("menu command did not persist");
    }
    let committed = clear_matching(path);
    if committed {
        notify_path(path);
    }
    if !still_there(path) {
        succeed_remove(path);
    } else {
        diag(&format!("result=still_present invoked={} committed={}", u8::from(invoked), u8::from(committed)));
        fail_remove(path);
    }
}

fn side_owner() -> HWND {
    let v = SIDES.lock().unwrap_or_else(|e| e.into_inner());
    for id in v.iter() {
        let h = HWND(*id as *mut core::ffi::c_void);
        unsafe {
            if IsWindow(Some(h)).as_bool() {
                return h;
            }
        }
    }
    HWND::default()
}

/// Invoke the enumerated Home command. Shell objects are dropped before the caller re-reads.
unsafe fn invoke_matching(path: &str, owner: HWND) -> bool {
    let found = {
        let walk = walk_namespace(HOME, path, true);
        for note in &walk.notes {
            diag(note);
        }
        diag(&format!("home matches={} enum_failed={}", walk.items.len(), u8::from(walk.failed)));
        let mut ok = false;
        for item in &walk.items {
            if invoke_item(item, owner) {
                ok = true;
                break;
            }
        }
        ok
    };
    if found {
        return true;
    }
    diag("file-menu");
    let Ok(file) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path), None) else {
        diag("file-menu parse failed");
        return false;
    };
    invoke_item(&file, owner)
}

unsafe fn invoke_item(item: &IShellItem, owner: HWND) -> bool {
    let Some(menu) = item_menu(item, owner) else {
        diag("menu missing");
        return false;
    };
    let Some(cmd) = pick_remove(&menu.cmds) else {
        diag("no matching command");
        return false;
    };
    invoke_id(&menu.cm, cmd, owner) == 0
}

unsafe fn clear_matching(path: &str) -> bool {
    let cleared = {
        let walk = walk_namespace(HOME, path, false);
        let mut ok = false;
        for item in &walk.items {
            if clear_ispinned(item, "home") {
                ok = true;
                break;
            }
        }
        ok
    };
    if cleared {
        return true;
    }
    let Ok(file) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path), None) else {
        diag("property file parse failed");
        return false;
    };
    clear_ispinned(&file, "file")
}

/// True when the file is still listed, or Home could not be read. Two passes catch a row that only blinks off.
unsafe fn still_there(path: &str) -> bool {
    std::thread::sleep(VERIFY_WAIT);
    let (home, quick) = read_presence(path);
    diag(&format!("verify home={home:?} quick={quick:?}"));
    if !removal_persisted(home, quick) {
        return true;
    }
    std::thread::sleep(VERIFY_WAIT);
    let (home2, quick2) = read_presence(path);
    diag(&format!("verify2 home={home2:?} quick={quick2:?}"));
    !removal_persisted(home2, quick2)
}

unsafe fn read_presence(path: &str) -> (Listed, Listed) {
    let home = listed(&walk_namespace(HOME, path, false));
    let quick = quick_file_listed(path);
    (home, quick)
}

fn listed(walk: &NsWalk) -> Listed {
    if !walk.items.is_empty() {
        Listed::Present
    } else if walk.failed {
        Listed::Unread
    } else {
        Listed::Absent
    }
}

unsafe fn notify_path(path: &str) {
    let mut wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    if let Some(parent) = super::parent_of(path) {
        wide = parent.encode_utf16().chain(std::iter::once(0)).collect();
        SHChangeNotify(SHCNE_UPDATEDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    }
}

fn fs_path(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')
}

unsafe fn shell_fs_path(item: &IShellItem) -> String {
    if let Ok(p) = item.GetDisplayName(SIGDN_FILESYSPATH) {
        let t = tidy_path(&pwstr(p));
        if fs_path(&t) {
            return t;
        }
    }
    if let Ok(p) = item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) {
        let t = tidy_path(&pwstr(p));
        if fs_path(&t) {
            return t;
        }
    }
    String::new()
}

/// Home children whose filesystem path is `path`, including two levels of path-less groups
/// (Home → Favourites → file). `failed` is set when an enumeration returns no children and errors.
unsafe fn walk_namespace(parsing: &str, path: &str, log_children: bool) -> NsWalk {
    let mut walk = NsWalk { items: Vec::new(), failed: false, notes: Vec::new() };
    let root = match SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(parsing), None) {
        Ok(root) => root,
        Err(e) => {
            walk.failed = true;
            walk.notes.push(format!("parse hr={:#010x}", e.code().0 as u32));
            return walk;
        }
    };
    walk_children(&root, path, 0, log_children, &mut walk);
    walk
}

unsafe fn walk_children(root: &IShellItem, path: &str, depth: u8, log_children: bool, walk: &mut NsWalk) {
    if walk.items.len() >= 4 {
        return;
    }
    let items = match root.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) {
        Ok(items) => items,
        Err(e) => {
            walk.failed = true;
            walk.notes.push(format!("enum depth={depth} hr={:#010x}", e.code().0 as u32));
            return;
        }
    };
    let mut groups = Vec::new();
    let mut produced = 0u32;
    for _ in 0..PIN_CAP {
        if walk.items.len() >= 4 {
            break;
        }
        let mut slot = [None];
        let hr = items.Next(&mut slot, None);
        let Some(child) = slot[0].take() else {
            if hr.is_err() && produced == 0 {
                walk.failed = true;
                let code = hr.unwrap_err().code().0 as u32;
                walk.notes.push(format!("next depth={depth} hr={code:#010x}"));
            }
            break;
        };
        produced += 1;
        let fs = shell_fs_path(&child);
        let name = child.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| pwstr(p)).unwrap_or_default();
        if !fs.is_empty() && fs.eq_ignore_ascii_case(path) {
            if log_children {
                walk.notes.push(format!("match depth={depth} fs={fs} name={name}"));
            }
            walk.items.push(child);
            continue;
        }
        if log_children && walk.notes.len() < 24 {
            walk.notes.push(format!("child depth={depth} fs={fs} name={name}"));
        }
        if depth < 2 && fs.is_empty() && groups.len() < 8 && child.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.contains(SFGAO_FOLDER)) {
            groups.push(child);
        }
    }
    drop(items);
    if depth < 2 {
        for group in groups {
            walk_children(&group, path, depth + 1, log_children, walk);
        }
    }
}

unsafe fn quick_file_listed(path: &str) -> Listed {
    let root = match SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(QUICK_ACCESS), None) {
        Ok(root) => root,
        Err(e) => {
            diag(&format!("quick parse hr={:#010x}", e.code().0 as u32));
            return Listed::Unread;
        }
    };
    let items = match root.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) {
        Ok(items) => items,
        Err(e) => {
            diag(&format!("quick enum hr={:#010x}", e.code().0 as u32));
            return Listed::Unread;
        }
    };
    let folder = root.BindToHandler::<_, IShellFolder2>(None, &BHID_SFObject).ok();
    let mut produced = 0u32;
    for _ in 0..PIN_CAP {
        let mut slot = [None];
        let hr = items.Next(&mut slot, None);
        let Some(child) = slot[0].take() else {
            if hr.is_err() && produced == 0 {
                diag(&format!("quick next hr={:#010x}", hr.unwrap_err().code().0 as u32));
                return Listed::Unread;
            }
            return Listed::Absent;
        };
        produced += 1;
        let fs = shell_fs_path(&child);
        if fs.is_empty() || !fs.eq_ignore_ascii_case(path) {
            continue;
        }
        let (pinned, _) = item_is_pinned(&child, folder.as_ref());
        let is_folder = child.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM).is_ok_and(shell_is_folder);
        if pinned && !is_folder {
            return Listed::Present;
        }
    }
    Listed::Absent
}

unsafe fn item_menu(item: &IShellItem, owner: HWND) -> Option<HeldMenu> {
    if let Some(menu) = menu_from_parent(item, owner) {
        return Some(menu);
    }
    let cm = match item.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) {
        Ok(cm) => cm,
        Err(e) => {
            diag(&format!("BindToHandler IContextMenu hr={:#010x}", e.code().0 as u32));
            return None;
        }
    };
    finish_menu(cm, None, std::ptr::null_mut(), "bind")
}

/// The menu Explorer shows for this view. Binding the child alone follows the file and
/// omits 「お気に入りから削除」.
unsafe fn menu_from_parent(item: &IShellItem, owner: HWND) -> Option<HeldMenu> {
    let pai = item.cast::<IParentAndItem>().ok()?;
    let mut folder: Option<IShellFolder> = None;
    let mut rel: *mut ITEMIDLIST = std::ptr::null_mut();
    if pai.GetParentAndItem(None, Some(&mut folder), Some(&mut rel)).is_err() || folder.is_none() || rel.is_null() {
        if !rel.is_null() {
            ILFree(Some(rel as *const ITEMIDLIST));
        }
        return None;
    }
    let folder = folder.unwrap();
    let refs = [rel as *const ITEMIDLIST];
    let cm = match folder.GetUIObjectOf::<IContextMenu>(owner, &refs, None) {
        Ok(cm) => cm,
        Err(e) => {
            diag(&format!("GetUIObjectOf hr={:#010x}", e.code().0 as u32));
            ILFree(Some(rel as *const ITEMIDLIST));
            return None;
        }
    };
    finish_menu(cm, Some(folder), rel, "parent")
}

unsafe fn finish_menu(cm: IContextMenu, folder: Option<IShellFolder>, rel: *mut ITEMIDLIST, source: &str) -> Option<HeldMenu> {
    let menu = match CreatePopupMenu() {
        Ok(menu) => menu,
        Err(e) => {
            diag(&format!("CreatePopupMenu source={source} hr={:#010x}", e.code().0 as u32));
            if !rel.is_null() {
                ILFree(Some(rel as *const ITEMIDLIST));
            }
            return None;
        }
    };
    let flags = CMF_NORMAL | CMF_EXTENDEDVERBS;
    let queried = cm.QueryContextMenu(menu, 0, CMD_FIRST, 0x7FFF, flags);
    if queried.is_err() {
        diag(&format!("QueryContextMenu source={source} flags=CMF_NORMAL|CMF_EXTENDEDVERBS hr={:#010x}", queried.0 as u32));
        _ = DestroyMenu(menu);
        if !rel.is_null() {
            ILFree(Some(rel as *const ITEMIDLIST));
        }
        return None;
    }
    let cmds = collect_cmds(menu, &cm);
    diag(&format!("menu source={source} flags=CMF_NORMAL|CMF_EXTENDEDVERBS query=ok cmds={}", cmds.len()));
    for cmd in &cmds {
        diag(&cmd_log(cmd));
    }
    Some(HeldMenu { cm, menu, _folder: folder, rel, cmds })
}

unsafe fn collect_cmds(menu: HMENU, cm: &IContextMenu) -> Vec<MenuCmd> {
    let mut out = Vec::new();
    collect_cmds_at(menu, cm, &mut out, 0);
    out
}

unsafe fn collect_cmds_at(menu: HMENU, cm: &IContextMenu, out: &mut Vec<MenuCmd>, depth: u8) {
    let n = GetMenuItemCount(Some(menu));
    if n <= 0 {
        return;
    }
    for i in 0..n as u32 {
        if out.len() >= 48 {
            return;
        }
        let id = GetMenuItemID(menu, i as i32);
        if (CMD_FIRST..=0x7FFF).contains(&id) {
            let offset = (id - CMD_FIRST) as usize;
            let (verb, gcs) = command_verb(cm, offset);
            let text = menu_item_text(menu, i);
            out.push(MenuCmd { verb, text, id, offset, gcs });
            continue;
        }
        if depth < 1 {
            let sub = GetSubMenu(menu, i as i32);
            if !sub.0.is_null() {
                collect_cmds_at(sub, cm, out, depth + 1);
            }
        }
    }
}

/// Wide verb first. An ANSI-only handler still yields a name; `gcs` stays the wide HRESULT.
unsafe fn command_verb(cm: &IContextMenu, offset: usize) -> (String, u32) {
    let mut buf = [0u16; 128];
    let hr = cm.GetCommandString(offset, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32);
    if hr.is_ok() {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        return (String::from_utf16_lossy(&buf[..end]), 0);
    }
    let wide = hr.unwrap_err().code().0 as u32;
    let mut ansi = [0u8; 128];
    let hr_a = cm.GetCommandString(offset, GCS_VERBA, None, PSTR(ansi.as_mut_ptr()), ansi.len() as u32);
    if hr_a.is_ok() {
        let end = ansi.iter().position(|&c| c == 0).unwrap_or(ansi.len());
        return (String::from_utf8_lossy(&ansi[..end]).into_owned(), wide);
    }
    (String::new(), wide)
}

unsafe fn menu_item_text(menu: HMENU, pos: u32) -> String {
    let mut buf = [0u16; 256];
    let mut info = MENUITEMINFOW {
        cbSize: size_of::<MENUITEMINFOW>() as u32,
        fMask: MIIM_STRING,
        dwTypeData: PWSTR(buf.as_mut_ptr()),
        cch: buf.len() as u32,
        ..Default::default()
    };
    if GetMenuItemInfoW(menu, pos, true, &mut info).is_err() {
        return String::new();
    }
    let n = (info.cch as usize).min(buf.len());
    String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string()
}

/// Command id, including offset 0. A null `lpVerb` is that offset (`MAKEINTRESOURCE(0)`),
/// so the first item is not skipped. The enumerated verb is tried only when the id fails.
unsafe fn invoke_id(cm: &IContextMenu, cmd: &MenuCmd, owner: HWND) -> u32 {
    let ici = CMINVOKECOMMANDINFO {
        cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
        fMask: CMIC_MASK_FLAG_NO_UI,
        hwnd: owner,
        lpVerb: PCSTR(cmd.offset as *const u8),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    let hr = match cm.InvokeCommand(&ici) {
        Ok(()) => 0,
        Err(e) => e.code().0 as u32,
    };
    diag(&format!("invoke id={} off={} verb={:?} hr={hr:#010x}", cmd.id, cmd.offset, cmd.verb));
    if hr == 0 || cmd.verb.is_empty() {
        return hr;
    }
    let mut wide: Vec<u16> = cmd.verb.encode_utf16().chain(std::iter::once(0)).collect();
    let mut ansi: Vec<u8> = cmd.verb.bytes().chain(std::iter::once(0)).collect();
    let ex = CMINVOKECOMMANDINFOEX {
        cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: CMIC_MASK_FLAG_NO_UI | CMIC_MASK_UNICODE,
        hwnd: owner,
        lpVerb: PCSTR(ansi.as_ptr()),
        lpVerbW: PCWSTR(wide.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    let hr2 = match cm.InvokeCommand(&ex as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO) {
        Ok(()) => 0,
        Err(e) => e.code().0 as u32,
    };
    diag(&format!("invoke-name verb={:?} hr={hr2:#010x}", cmd.verb));
    let _ = (&mut wide, &mut ansi);
    hr2
}

unsafe fn clear_ispinned(item: &IShellItem, target: &str) -> bool {
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::UI::Shell::PropertiesSystem::{GETPROPERTYSTOREFLAGS, IPropertyStore};
    diag(&format!("property target={target} System.Home.IsPinned=false vt=11 val=0"));
    let item2 = match item.cast::<IShellItem2>() {
        Ok(item2) => item2,
        Err(e) => {
            diag(&format!("property target={target} cast hr={:#010x}", e.code().0 as u32));
            return false;
        }
    };
    for flags in store_flag_sets() {
        let store = match item2.GetPropertyStore::<IPropertyStore>(GETPROPERTYSTOREFLAGS(*flags as i32)) {
            Ok(store) => store,
            Err(e) => {
                let hr = e.code().0 as u32;
                diag(&format!("property target={target} flags={flags:#x} open hr={hr:#010x}"));
                continue;
            }
        };
        let (vt, val) = unpinned_variant();
        let mut bytes = [0u8; 16];
        bytes[..2].copy_from_slice(&val.to_le_bytes());
        let raw = PropVar { vt, w1: 0, w2: 0, w3: 0, data: PropData { bytes } };
        let prop: PROPVARIANT = std::mem::transmute(raw);
        let set = match store.SetValue(&PKEY_HOME_ISPINNED, &prop) {
            Ok(()) => 0,
            Err(e) => e.code().0 as u32,
        };
        let commit = if set == 0 {
            match store.Commit() {
                Ok(()) => 0,
                Err(e) => e.code().0 as u32,
            }
        } else {
            0xFFFF_FFFF
        };
        diag(&format!("property target={target} flags={flags:#x} set={set:#010x} commit={commit:#010x}"));
        if set == 0 && commit == 0 {
            return true;
        }
    }
    false
}

fn pin_worker() {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            pin_log(&format!("CoInitializeEx {:#010x}", hr.0 as u32));
        }
    }
    loop {
        let force = wait_request();
        if !force {
            let last = state().last;
            if !pins_due(last, Instant::now(), PIN_WAIT) {
                end_request();
                continue;
            }
        }
        let read = unsafe { read_quick_access() };
        apply_read(&read);
        end_request();
        post_pins();
    }
}

/// Take a queued read. `INFLIGHT` is set before the gate is released so a force
/// that arrives during the read is remembered instead of lost.
fn wait_request() -> bool {
    let mut gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
    loop {
        if WANT.swap(false, Ordering::Relaxed) {
            INFLIGHT.store(true, Ordering::Relaxed);
            return FORCE.swap(false, Ordering::Relaxed);
        }
        gate = WAKE.wait(gate).unwrap_or_else(|e| e.into_inner());
    }
}

fn end_request() {
    let again = {
        let _gate = GATE.lock().unwrap_or_else(|e| e.into_inner());
        INFLIGHT.store(false, Ordering::Relaxed);
        let again = AGAIN.swap(false, Ordering::Relaxed);
        if again {
            FORCE.store(true, Ordering::Relaxed);
            WANT.store(true, Ordering::Relaxed);
        }
        again
    };
    if again {
        WAKE.notify_one();
    }
}

fn apply_read(read: &HomeRead) {
    let split = if read.ok { split_pins(&read.items) } else { PinSplit { folders: Vec::new(), files: Vec::new() } };
    let pinned = read.items.iter().filter(|i| i.pinned).count();
    let empty_path = read.items.iter().filter(|i| i.path.is_empty()).count();
    let paths = read
        .items
        .iter()
        .filter(|i| i.pinned && !i.path.is_empty())
        .take(12)
        .map(|i| i.path.as_str())
        .collect::<Vec<_>>()
        .join(" | ");
    pin_log(&format!(
        "pins ok={} err={} seen={} pinned={} folders={} files={} empty_path={} vts={} paths={paths}",
        u8::from(read.ok),
        read.err,
        read.items.len(),
        pinned,
        split.folders.len(),
        split.files.len(),
        empty_path,
        read.vts,
    ));
    let now = Instant::now();
    let mut s = state();
    if read.ok {
        s.fresh_dir.retain(|(_, t)| fresh_enough(*t, now));
        s.fresh_file.retain(|(_, t)| fresh_enough(*t, now));
        s.gone.retain(|(_, t)| fresh_enough(*t, now));
        let gone = s.gone.clone();
        let cur_dir = std::mem::take(&mut s.folders);
        let mut fresh_dir = std::mem::take(&mut s.fresh_dir);
        let folders = merge_read(split.folders, &cur_dir, &mut fresh_dir, &gone);
        let cur_file = std::mem::take(&mut s.files);
        let mut fresh_file = std::mem::take(&mut s.fresh_file);
        let files = merge_read(split.files, &cur_file, &mut fresh_file, &gone);
        s.folders = folders;
        s.fresh_dir = fresh_dir;
        s.files = files;
        s.fresh_file = fresh_file;
        if !s.note_path.is_empty() && !s.files.iter().any(|p| p.path.eq_ignore_ascii_case(&s.note_path)) {
            s.note_path.clear();
            s.note.clear();
        }
        s.loaded = true;
        TRIES.store(0, Ordering::Relaxed);
    } else if TRIES.fetch_add(1, Ordering::Relaxed) + 1 >= PIN_TRIES {
        s.loaded = true;
    }
    s.last = Some(now);
}

fn merge_read(read: Vec<Pinned>, current: &[Pinned], fresh: &mut Vec<(String, Instant)>, gone: &[(String, Instant)]) -> Vec<Pinned> {
    let mut items = read;
    items.retain(|p| !gone.iter().any(|(g, _)| g.eq_ignore_ascii_case(&p.path)));
    // Drop a grace entry once Quick Access itself lists the item.
    fresh.retain(|(f, _)| !items.iter().any(|i| i.path.eq_ignore_ascii_case(f)));
    let keep: Vec<Pinned> = current.iter().filter(|p| fresh.iter().any(|(f, _)| f.eq_ignore_ascii_case(&p.path))).cloned().collect();
    for p in keep.into_iter().rev() {
        items.insert(0, p);
    }
    items
}

struct HomeRead {
    ok: bool,
    items: Vec<PinCand>,
    vts: String,
    err: String,
}

fn home_fail(err: String) -> HomeRead {
    HomeRead { ok: false, items: Vec::new(), vts: String::new(), err }
}

/// Nonzero `VT_BOOL` / integer means pinned. Empty, null, and error are unknown so the caller can fall back.
fn flag_from_variant(vt: u16, bool_val: i16, signed: i64, unsigned: u64) -> Option<bool> {
    if vt == VT_EMPTY.0 || vt == VT_NULL.0 || vt == VT_ERROR.0 {
        return None;
    }
    if vt == VT_BOOL.0 {
        return Some(bool_val != 0);
    }
    if vt == VT_I1.0 || vt == VT_I2.0 || vt == VT_I4.0 || vt == VT_I8.0 || vt == VT_INT.0 {
        return Some(signed != 0);
    }
    if vt == VT_UI1.0 || vt == VT_UI2.0 || vt == VT_UI4.0 || vt == VT_UI8.0 || vt == VT_UINT.0 {
        return Some(unsigned != 0);
    }
    None
}

fn note_vt(vts: &mut String, vt: u16, pinned: bool) {
    if vts.len() > 180 {
        return;
    }
    if !vts.is_empty() {
        vts.push(',');
    }
    vts.push_str(&format!("{vt}:{}", u8::from(pinned)));
}

fn favorite_log_file(dir: &Path) -> PathBuf {
    dir.join("favorite-remove.log")
}

/// Keep one older file. The active log stays under `max_bytes`.
fn rotate_log(path: &Path, max_bytes: u64) {
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if max_bytes == 0 || len < max_bytes {
        return;
    }
    let bak = path.with_extension("log.1");
    _ = std::fs::remove_file(&bak);
    _ = std::fs::rename(path, &bak);
}

fn diag_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOG: Mutex<()> = Mutex::new(());
    LOG.lock().unwrap_or_else(|e| e.into_inner())
}

fn append_diag_at(dir: &Path, line: &str) {
    let _gate = diag_lock();
    _ = std::fs::create_dir_all(dir);
    let path = favorite_log_file(dir);
    rotate_log(&path, FAV_LOG_MAX);
    let new_file = !path.exists();
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
    use std::io::Write;
    if new_file {
        _ = file.write_all(&[0xEF, 0xBB, 0xBF]);
    }
    _ = writeln!(file, "{line}");
}

fn diag(line: &str) {
    let secs = std::time::SystemTime::now().duration_since(std::time::SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    append_diag_at(&crate::settings::store::config_dir(), &format!("{secs} {line}"));
}

#[cfg(debug_assertions)]
fn pin_log(msg: &str) {
    let path = std::env::temp_dir().join("maguro-pins.log");
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return };
    use std::io::Write;
    let _ = writeln!(file, "{msg}");
}

#[cfg(not(debug_assertions))]
fn pin_log(_: &str) {}

unsafe fn read_quick_access() -> HomeRead {
    let root = match SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(QUICK_ACCESS), None) {
        Ok(root) => root,
        Err(e) => return home_fail(format!("parse {:#010x}", e.code().0 as u32)),
    };
    let enumerator = match root.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) {
        Ok(enumerator) => enumerator,
        Err(e) => return home_fail(format!("enum {:#010x}", e.code().0 as u32)),
    };
    let folder = root.BindToHandler::<_, IShellFolder2>(None, &BHID_SFObject).ok();
    let mut out = Vec::new();
    let mut vts = String::new();
    while out.len() < PIN_CAP {
        let mut slot = [None];
        // celt is 1, so a null fetched-count is allowed. The item pointer is the signal.
        let hr = enumerator.Next(&mut slot, None);
        let Some(child) = slot[0].take() else {
            if hr.is_err() && out.is_empty() {
                return home_fail(format!("next {:#010x}", hr.unwrap_err().code().0 as u32));
            }
            break;
        };
        let (pinned, vt) = item_is_pinned(&child, folder.as_ref());
        // A zip is both a folder and a stream. That is a file, not a directory.
        let is_folder = child.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM).is_ok_and(shell_is_folder);
        let path = child.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)).unwrap_or_default();
        let name = child.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| pwstr(p)).unwrap_or_default();
        note_vt(&mut vts, vt, pinned);
        out.push(PinCand { path, name, pinned, folder: is_folder });
    }
    HomeRead { ok: true, items: out, vts, err: String::new() }
}

/// `System.Home.IsPinned` is `VT_BOOL` on some Windows builds and `VT_UI4` (or another integer) on others.
/// `GetBool` rejects the integer form, which made every Quick Access child look unpinned.
unsafe fn item_is_pinned(child: &IShellItem, folder: Option<&IShellFolder2>) -> (bool, u16) {
    if let Ok(item) = child.cast::<IShellItem2>() {
        let (flag, vt) = property_flag(&item);
        if let Some(flag) = flag {
            return (flag, vt);
        }
        if let Some(folder) = folder {
            if let Some((flag, vt2)) = details_flag(folder, child) {
                return (flag, vt2);
            }
        }
        return (false, vt);
    }
    if let Some(folder) = folder {
        if let Some((flag, vt)) = details_flag(folder, child) {
            return (flag, vt);
        }
    }
    (false, 0)
}

unsafe fn property_flag(item: &IShellItem2) -> (Option<bool>, u16) {
    let mut pv = PropVar { vt: 0, w1: 0, w2: 0, w3: 0, data: PropData { bytes: [0; 16] } };
    if get_property(item, &mut pv).is_err() {
        return (None, 0);
    }
    let (vt, bool_val, signed, unsigned) = prop_numbers(&pv);
    let flag = flag_from_variant(vt, bool_val, signed, unsigned);
    _ = PropVariantClear(&mut pv);
    (flag, vt)
}

unsafe fn get_property(item: &IShellItem2, pv: *mut PropVar) -> HRESULT {
    type GetProperty = unsafe extern "system" fn(*mut core::ffi::c_void, *const PROPERTYKEY, *mut PropVar) -> HRESULT;
    let this = windows::core::Interface::as_raw(item);
    let vtbl = *(this as *const *const usize);
    let get_property: GetProperty = std::mem::transmute(*vtbl.add(GET_PROPERTY));
    get_property(this, &PKEY_HOME_ISPINNED, pv)
}

unsafe fn prop_numbers(pv: &PropVar) -> (u16, i16, i64, u64) {
    let d = pv.data.bytes;
    let bool_val = i16::from_le_bytes([d[0], d[1]]);
    let i8_val = d[0] as i8 as i64;
    let u8_val = d[0] as u64;
    let i16_val = i16::from_le_bytes([d[0], d[1]]) as i64;
    let u16_val = u16::from_le_bytes([d[0], d[1]]) as u64;
    let i32_val = i32::from_le_bytes([d[0], d[1], d[2], d[3]]) as i64;
    let u32_val = u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as u64;
    let i64_val = i64::from_le_bytes(d[0..8].try_into().unwrap());
    let u64_val = u64::from_le_bytes(d[0..8].try_into().unwrap());
    let (signed, unsigned) = match pv.vt {
        v if v == VT_I1.0 => (i8_val, 0),
        v if v == VT_UI1.0 => (0, u8_val),
        v if v == VT_I2.0 => (i16_val, 0),
        v if v == VT_UI2.0 => (0, u16_val),
        v if v == VT_I4.0 || v == VT_INT.0 => (i32_val, 0),
        v if v == VT_UI4.0 || v == VT_UINT.0 => (0, u32_val),
        v if v == VT_I8.0 => (i64_val, 0),
        v if v == VT_UI8.0 => (0, u64_val),
        _ => (0, 0),
    };
    let bool_val = if pv.vt == VT_BOOL.0 { bool_val } else { 0 };
    (pv.vt, bool_val, signed, unsigned)
}

unsafe fn details_flag(folder: &IShellFolder2, child: &IShellItem) -> Option<(bool, u16)> {
    let rel = child_rel_pidl(child)?;
    let got = folder.GetDetailsEx(rel.ptr, &PKEY_HOME_ISPINNED);
    drop(rel);
    let mut v = got.ok()?;
    let (vt, bool_val, signed, unsigned) = variant_numbers(&v);
    let flag = flag_from_variant(vt, bool_val, signed, unsigned).unwrap_or(false);
    _ = VariantClear(&mut v);
    Some((flag, vt))
}

unsafe fn variant_numbers(v: &VARIANT) -> (u16, i16, i64, u64) {
    let head = &v.Anonymous.Anonymous;
    let vt = head.vt;
    let raw = &head.Anonymous;
    let (bool_val, signed, unsigned) = match vt {
        VT_BOOL => (raw.boolVal.0, 0, 0),
        VT_I1 => (0, raw.cVal as i64, 0),
        VT_UI1 => (0, 0, raw.bVal as u64),
        VT_I2 => (0, raw.iVal as i64, 0),
        VT_UI2 => (0, 0, raw.uiVal as u64),
        VT_I4 => (0, raw.lVal as i64, 0),
        VT_INT => (0, raw.intVal as i64, 0),
        VT_UI4 => (0, 0, raw.ulVal as u64),
        VT_UINT => (0, 0, raw.uintVal as u64),
        VT_I8 => (0, raw.llVal, 0),
        VT_UI8 => (0, 0, raw.ullVal),
        _ => (0, 0, 0),
    };
    (vt.0, bool_val, signed, unsigned)
}

struct RelPidl {
    owned: *mut ITEMIDLIST,
    ptr: *const ITEMIDLIST,
}

impl Drop for RelPidl {
    fn drop(&mut self) {
        if !self.owned.is_null() {
            unsafe { ILFree(Some(self.owned as *const ITEMIDLIST)) };
        }
    }
}

unsafe fn child_rel_pidl(child: &IShellItem) -> Option<RelPidl> {
    if let Ok(pai) = child.cast::<IParentAndItem>() {
        let mut rel: *mut ITEMIDLIST = std::ptr::null_mut();
        let ok = pai.GetParentAndItem(None, None, Some(std::ptr::addr_of_mut!(rel))).is_ok();
        if ok && !rel.is_null() {
            return Some(RelPidl { owned: rel, ptr: rel });
        }
        if !rel.is_null() {
            ILFree(Some(rel as *const ITEMIDLIST));
        }
    }
    let abs = SHGetIDListFromObject(child).ok()?;
    if abs.is_null() {
        return None;
    }
    let rel = ILFindLastID(abs);
    if rel.is_null() {
        ILFree(Some(abs as *const ITEMIDLIST));
        return None;
    }
    Some(RelPidl { owned: abs, ptr: rel })
}

unsafe fn invoke_verb(path: &str, verb: &str) -> bool {
    let file = HSTRING::from(path);
    let verb_w = HSTRING::from(verb);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_INVOKEIDLIST | SEE_MASK_FLAG_NO_UI,
        lpVerb: PCWSTR(verb_w.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    if ShellExecuteExW(&mut sei).is_ok() {
        return true;
    }
    let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&file, None) else { return false };
    item_verb(&item, verb)
}

#[cfg(test)]
mod tests {
    use super::{flag_from_variant, shell_is_folder};
    use crate::imports::*;
    use windows::Win32::System::Variant::{VT_BOOL, VT_EMPTY, VT_NULL, VT_UI4};

    #[test]
    fn a_zip_is_a_file_and_a_directory_is_a_folder() {
        assert!(shell_is_folder(SFGAO_FOLDER));
        assert!(!shell_is_folder(SFGAO_FOLDER | SFGAO_STREAM));
        assert!(!shell_is_folder(SFGAO_STREAM));
        assert!(!shell_is_folder(SFGAO_FLAGS::default()));
    }

    #[test]
    fn propvar_is_the_com_layout() {
        assert_eq!(std::mem::size_of::<super::PropVar>(), 24);
        assert_eq!(std::mem::align_of::<super::PropVar>(), 8);
    }

    #[test]
    fn home_ispinned_accepts_bool_and_any_nonzero_integer() {
        assert_eq!(VT_BOOL.0, 11);
        assert_eq!(VT_UI4.0, 19);
        assert_eq!(flag_from_variant(VT_BOOL.0, -1, 0, 0), Some(true));
        assert_eq!(flag_from_variant(VT_BOOL.0, 0, 0, 0), Some(false));
        assert_eq!(flag_from_variant(VT_UI4.0, 0, 0, 1), Some(true));
        assert_eq!(flag_from_variant(VT_UI4.0, 0, 0, 0), Some(false));
        assert_eq!(flag_from_variant(VT_I4.0, 0, 0, 0), Some(false));
        assert_eq!(flag_from_variant(VT_I4.0, 0, 3, 0), Some(true));
        assert_eq!(flag_from_variant(VT_I4.0, 0, -2, 0), Some(true));
        assert_eq!(flag_from_variant(VT_EMPTY.0, -1, 1, 1), None);
        assert_eq!(flag_from_variant(VT_NULL.0, -1, 1, 1), None);
    }

    fn pinned(path: &str) -> crate::session::Pinned {
        crate::session::Pinned { path: path.into(), name: path.into() }
    }

    fn cmd(verb: &str, text: &str, id: u32) -> super::MenuCmd {
        super::MenuCmd { verb: verb.into(), text: text.into(), id, offset: id.saturating_sub(1) as usize, gcs: 0 }
    }

    #[test]
    fn the_home_command_matches_the_verb_or_the_label_explorer_shows() {
        assert!(super::is_remove_favorite_verb("removefromfavorites"));
        assert!(super::is_remove_favorite_verb(" &RemoveFromFavorites "));
        assert!(super::is_remove_favorite_verb("unpinfromfavourites"));
        assert!(!super::is_remove_favorite_verb("unpinfromhome"), "that verb unpins a Quick Access folder");
        assert!(!super::is_remove_favorite_verb("remove"));
        assert!(!super::is_remove_favorite_verb("delete"));
        assert!(super::is_remove_favorite_text("Remove from Favorites"));
        assert!(super::is_remove_favorite_text("Remove from &Favorites"));
        assert!(super::is_remove_favorite_text("Remove from Favourites"));
        assert!(super::is_remove_favorite_text("Unpin from Favorites"));
        assert!(super::is_remove_favorite_text("お気に入りから削除"));
        assert!(super::is_remove_favorite_text("お気に入りから削除(&R)"));
        assert!(super::is_remove_favorite_text("즐겨찾기에서 제거"));
        assert!(!super::is_remove_favorite_text("Add to Favorites"));
        assert!(!super::is_remove_favorite_text("お気に入りに追加"));
        assert!(!super::is_remove_favorite_text("즐겨찾기에 추가"));
        assert!(!super::is_remove_favorite_text("削除"));
        assert!(!super::is_remove_favorite_text("Delete"));
        assert_eq!(super::menu_label("お気に入りから削除(&R)"), "お気に入りから削除");
    }

    #[test]
    fn the_enumerated_command_is_chosen_by_label_and_the_first_id_is_kept() {
        let delete = cmd("delete", "削除", 10);
        let unpin = cmd("unpinfromhome", "Unpin from Quick access", 11);
        let home = cmd("removefromfavorites", "お気に入りから削除(&R)", 1);
        let menu = [delete, unpin, home];
        let picked = super::pick_remove(&menu).unwrap();
        assert_eq!(picked.id, 1);
        assert_eq!(picked.offset, 0, "offset 0 is the first command and is still invoked by id");
        assert_eq!(picked.verb, "removefromfavorites");
        assert_eq!(super::pick_remove(&[cmd("remove", "削除", 5), cmd("", "Remove from Favorites", 4)]).unwrap().id, 4);
        assert!(super::pick_remove(&[cmd("delete", "Delete", 3), cmd("remove", "Remove", 8)]).is_none());
        let text_only = cmd("", "Remove from Favorites", 4);
        let line = super::cmd_log(&text_only);
        assert!(line.contains("id=4"));
        assert!(line.contains("Remove from Favorites"));
        assert!(line.contains("0x00000000"));
    }

    #[test]
    fn the_row_stays_unless_home_and_quick_access_both_omit_the_file() {
        use super::Listed::*;
        assert!(super::removal_persisted(Absent, Absent));
        for home in [Absent, Present, Unread] {
            for quick in [Absent, Present, Unread] {
                if home == Absent && quick == Absent {
                    continue;
                }
                assert!(!super::removal_persisted(home, quick), "{home:?} {quick:?}");
            }
        }
    }

    #[test]
    fn an_unconfirmed_removal_does_not_hide_the_file() {
        assert!(!super::removal_persisted(super::Listed::Present, super::Listed::Present));
        let note = r"C:\note.txt";
        let read = vec![pinned(note)];
        let mut fresh = Vec::new();
        let shown = super::merge_read(read, &[], &mut fresh, &[]);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].path, note);
    }

    #[test]
    fn clearing_ispinned_writes_variant_false_on_a_writable_store() {
        assert_eq!(super::unpinned_variant(), (VT_BOOL.0, 0));
        assert_ne!(super::unpinned_variant().1, -1);
        let flags = super::store_flag_sets();
        assert!(flags[0] & super::GPS_READWRITE != 0);
        assert!(flags[0] & super::GPS_HANDLERPROPERTIESONLY != 0);
        assert!(flags.iter().all(|f| f & super::GPS_READWRITE != 0));
    }

    #[test]
    fn the_diagnostic_log_is_beside_maguro_settings_and_rotates() {
        let settings = std::path::PathBuf::from(r"C:\Users\Jun\AppData\Roaming\Maguro");
        let log = super::favorite_log_file(&settings);
        assert_eq!(log.file_name().and_then(|s| s.to_str()), Some("favorite-remove.log"));
        let low = log.to_string_lossy().to_ascii_lowercase();
        assert!(low.contains("maguro"));
        let dir = std::env::temp_dir().join(format!("maguro-favlog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = super::favorite_log_file(&dir);
        std::fs::write(&path, vec![b'a'; 50]).unwrap();
        super::rotate_log(&path, 40);
        let bak = path.with_extension("log.1");
        assert_eq!(std::fs::read(&bak).unwrap().len(), 50);
        assert!(!path.exists());
        std::fs::write(&path, vec![b'b'; 60]).unwrap();
        super::rotate_log(&path, 40);
        assert_eq!(std::fs::read(&bak).unwrap(), vec![b'b'; 60]);
        assert!(!path.exists());
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "only the one rotated file is kept");
        super::append_diag_at(&dir, &super::cmd_log(&cmd("removefromfavorites", "お気に入りから削除", 15)));
        let body = std::fs::read(&path).unwrap();
        assert!(body.starts_with(&[0xEF, 0xBB, 0xBF]));
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains("removefromfavorites"));
        assert!(text.contains("お気に入りから削除"));
        assert!(text.contains("id=15"));
        assert!(text.contains("0x00000000"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_favourite_file_stays_gone_when_the_next_read_omits_it() {
        let note = r"C:\note.txt";
        let keep = r"C:\keep.txt";
        let read_without = vec![pinned(keep)];
        let current = vec![pinned(keep), pinned(note)];
        let mut fresh = Vec::new();
        // The 3s hide has expired, which is what a later read does. The shell no longer lists the file.
        let merged = super::merge_read(read_without, &current, &mut fresh, &[]);
        assert!(merged.iter().any(|p| p.path == keep));
        assert!(merged.iter().all(|p| p.path != note));
    }

    #[test]
    fn an_in_memory_hide_lets_the_shell_read_put_the_file_back() {
        let note = r"C:\note.txt";
        let read = vec![pinned(note)];
        let mut fresh = Vec::new();
        let hidden = super::merge_read(read.clone(), &[], &mut fresh, &[(note.into(), std::time::Instant::now())]);
        assert!(hidden.is_empty(), "while the hold lasts, a file the shell still lists stays hidden");
        let mut fresh = Vec::new();
        let back = super::merge_read(read, &[], &mut fresh, &[]);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].path, note);
    }
}
#[cfg(test)]
mod utf8_boundary_tests {
    #[test]
    fn japanese_korean_and_emoji_menu_labels_do_not_panic() {
        assert_eq!(super::menu_label("削除(&R)"), "削除");
        assert_eq!(super::menu_label("샘플(&S)"), "샘플");
        assert_eq!(super::menu_label("サンプル(あ)"), "サンプル");
        assert_eq!(super::menu_label("한글폴더"), "한글폴더");
        assert_eq!(super::menu_label("😀あ"), "😀あ");
        assert_eq!(super::menu_label("abcdサ"), "abcdサ");
    }
}
