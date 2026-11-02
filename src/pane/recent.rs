//! Recent files, from the Windows Recent folder (`FOLDERID_Recent`, `shell:Recent`).
//!
//! The pane's `IExplorerBrowser` opens that folder. The shell view shows each shortcut's
//! name and icon, and activating one opens the target. This module hides a link whose
//! local target is gone, sorts Date modified descending (newest first), and adds
//! Show in folder.
//!
//! Read only. No `SHAddToRecentDocs`, no `IShellLink::Resolve` (that can rewrite the
//! shortcut and show UI), no registry writes. Delete, rename, cut, paste, and new items
//! are refused so Maguro does not change the Recent folder.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::imports::*;
use crate::app::*;
use crate::is_fs;
use super::{focus_view, local_path_gone, navigate, share_path, update_status};

/// `IShellLink::GetPath` flag. The stored path, even when the target is already gone.
/// `Resolve` is never called.
const SLGP_RAWPATH: u32 = 0x4;
const RECENT_GUID: &str = "ae50c081-ebd2-438a-8655-8a092e34987a";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetKind {
    /// A real file or folder that is still there.
    File,
    /// No filesystem path (a shell item). Shown; it cannot be probed.
    Virtual,
    /// A share, or a path we will not stat on this thread. Shown.
    Unprobed,
    /// Local path confirmed missing.
    Missing,
    /// A shortcut with no target at all.
    Broken,
}

/// What to do with one Recent entry. Missing and broken links are hidden.
pub(crate) fn show_target(kind: TargetKind) -> bool {
    matches!(kind, TargetKind::File | TargetKind::Virtual | TargetKind::Unprobed)
}

/// `has_path` is a stored target path. `virtual_item` is a pidl with no path.
/// `gone` is only read for a local filesystem path.
pub(crate) fn classify(has_path: bool, virtual_item: bool, fs: bool, share: bool, gone: bool) -> TargetKind {
    if !has_path {
        return if virtual_item { TargetKind::Virtual } else { TargetKind::Broken };
    }
    if !fs || share {
        TargetKind::Unprobed
    } else if gone {
        TargetKind::Missing
    } else {
        TargetKind::File
    }
}

/// Verbs that would change the Recent folder. Matched on the last dotted piece (`delete`, `Windows.delete`).
pub(crate) fn write_verb(verb: &str) -> bool {
    let v = verb.trim().trim_matches('\0').to_ascii_lowercase();
    let tail = v.rsplit(['.', '\\']).next().unwrap_or(v.as_str());
    matches!(tail, "delete" | "cut" | "rename" | "paste" | "pastelink" | "link" | "newfolder" | "newlink" | "new")
}

/// Keys that edit the folder. Recent files does not pass them to the shell view.
pub(crate) fn blocks_key(vk: u16, ctrl: bool, alt: bool, shift: bool) -> bool {
    if alt {
        return false;
    }
    match vk {
        0x2E | 0x71 => true, // Del, F2
        0x58 | 0x56 if ctrl => true, // Ctrl+X, Ctrl+V
        0x2D if shift && !ctrl => true, // Shift+Insert
        _ => false,
    }
}

pub(crate) fn norm_dir(path: &str) -> String {
    let t = path.trim().trim_matches('"').replace('/', "\\").to_ascii_lowercase();
    let t = if let Some(rest) = t.strip_prefix(r"\\?\unc\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = t.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        t
    };
    t.trim_end_matches('\\').to_string()
}

pub(crate) fn same_dir(a: &str, b: &str) -> bool {
    let (a, b) = (norm_dir(a), norm_dir(b));
    !a.is_empty() && a == b
}

/// `%NAME%` from the process environment. An unknown name is left as written.
pub(crate) fn expand_vars(path: &str) -> String {
    if !path.contains('%') {
        return path.to_string();
    }
    let mut out = String::new();
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('%') {
            let name = &after[..end];
            if !name.is_empty() {
                if let Ok(val) = std::env::var(name) {
                    out.push_str(&val);
                    rest = &after[end + 1..];
                    continue;
                }
            }
            out.push('%');
            out.push_str(name);
            rest = &after[end..];
            continue;
        }
        out.push('%');
        out.push_str(after);
        return out;
    }
    out.push_str(rest);
    out
}

/// Parent folder and the child's name. Drive roots stay `C:\`.
pub(crate) fn parent_and_name(path: &str) -> Option<(String, String)> {
    let path = path.trim().trim_matches('"').trim_end_matches(['\\', '/']);
    if path.is_empty() {
        return None;
    }
    let slash = path.rfind(['\\', '/'])?;
    let name = path[slash + 1..].trim();
    if name.is_empty() || name == "." || name == ".." || name.contains(':') {
        return None;
    }
    let mut parent = path[..slash].to_string();
    if parent.len() == 2 && parent.as_bytes().get(1) == Some(&b':') {
        parent.push('\\');
    }
    if parent.is_empty() {
        return None;
    }
    Some((parent, name.to_string()))
}

/// A name relative to the Recent folder, or a path that is already absolute.
pub(crate) fn child_path(root: &str, rel: &str) -> String {
    let rel = rel.trim().trim_matches('"');
    if rel.is_empty() {
        return String::new();
    }
    if is_fs(rel) || rel.to_ascii_lowercase().starts_with("shell:") {
        return rel.to_string();
    }
    let root = root.trim().trim_end_matches(['\\', '/']);
    if root.is_empty() {
        return rel.to_string();
    }
    format!("{root}\\{}", rel.trim_start_matches(['\\', '/']))
}

pub(crate) fn path_is_recent_dir(path: &str, known: &str) -> bool {
    let low = path.trim().to_ascii_lowercase();
    if low == "shell:recent" || low.contains(RECENT_GUID) {
        return true;
    }
    !known.is_empty() && same_dir(path, known)
}

fn show_flags() -> &'static [AtomicBool; MAXP] {
    static SHOW: OnceLock<[AtomicBool; MAXP]> = OnceLock::new();
    SHOW.get_or_init(|| std::array::from_fn(|_| AtomicBool::new(false)))
}

#[derive(Clone, Copy)]
struct Later {
    refresh: bool,
    sort_left: u8,
}

impl Later {
    const QUIET: Later = Later { refresh: false, sort_left: 0 };
}

static LATER: Mutex<[Later; MAXP]> = Mutex::new([Later::QUIET; MAXP]);

struct Reveal {
    pane: usize,
    folder: String,
    name: String,
    /// Select attempts once the parent folder is the one showing.
    left: u8,
    /// Ticks to wait while navigation has not arrived yet.
    wait: u8,
}

static REVEAL: Mutex<Option<Reveal>> = Mutex::new(None);

fn known_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Filesystem path of `FOLDERID_Recent`. Empty when the shell cannot name it.
/// The folder is not created (`KF_FLAG_CREATE` is not set).
pub(crate) fn folder_path() -> String {
    if let Ok(g) = known_cache().lock() {
        if let Some(s) = g.as_ref() {
            if !s.is_empty() {
                return s.clone();
            }
        }
    }
    let s = unsafe { query_folder() };
    if !s.is_empty() {
        if let Ok(mut g) = known_cache().lock() {
            *g = Some(s.clone());
        }
    }
    s
}

unsafe fn query_folder() -> String {
    let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Recent, KF_FLAG_DONT_VERIFY, None) else { return String::new() };
    let path = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as _));
    path
}

pub(crate) fn path_is_recent(path: &str) -> bool {
    path_is_recent_dir(path, &folder_path())
}

pub(crate) fn showing(i: usize) -> bool {
    i < MAXP && show_flags()[i].load(Ordering::Relaxed)
}

/// The folder this pane is opening or showing. Set before the view enumerates.
pub(crate) fn note_folder(i: usize, path: &str) {
    if i < MAXP {
        show_flags()[i].store(path_is_recent(path), Ordering::Relaxed);
    }
}

pub(crate) fn locked(i: usize) -> bool {
    if showing(i) {
        return true;
    }
    let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    path_is_recent(&path)
}

pub(crate) fn clear_pane(i: usize) {
    if i >= MAXP {
        return;
    }
    show_flags()[i].store(false, Ordering::Relaxed);
    if let Ok(mut later) = LATER.lock() {
        later[i] = Later::QUIET;
    }
    if let Ok(mut rev) = REVEAL.lock() {
        if rev.as_ref().is_some_and(|r| r.pane == i) {
            *rev = None;
        }
    }
}

pub(crate) fn note_swap(i: usize, j: usize) {
    if i >= MAXP || j >= MAXP || i == j {
        return;
    }
    let flags = show_flags();
    let a = flags[i].load(Ordering::Relaxed);
    let b = flags[j].load(Ordering::Relaxed);
    flags[i].store(b, Ordering::Relaxed);
    flags[j].store(a, Ordering::Relaxed);
    if let Ok(mut later) = LATER.lock() {
        later.swap(i, j);
    }
    if let Ok(mut rev) = REVEAL.lock() {
        if let Some(r) = rev.as_mut() {
            if r.pane == i {
                r.pane = j;
            } else if r.pane == j {
                r.pane = i;
            }
        }
    }
}

pub(crate) fn open_label() -> String {
    with_chord(crate::tr(crate::Key::RecentFiles), "view.recentfiles")
}

pub(crate) fn reveal_label() -> String {
    with_chord(crate::tr(crate::Key::ShowInFolder), "file.reveal")
}

fn with_chord(name: &str, id: &str) -> String {
    let chord = crate::keys::index(id).map(crate::keys::chord_text).unwrap_or_default();
    if chord.is_empty() { name.to_string() } else { format!("{name}\t{chord}") }
}

/// Open Recent files in this pane. Back returns to the folder that was showing.
pub(crate) unsafe fn open_recent(i: usize) {
    let path = folder_path();
    if path.is_empty() || !navigate(i, &path) {
        _ = MessageBeep(MB_ICONWARNING);
        app(|a| {
            a.note = Some((crate::tr(crate::Key::CanTOpenThisFolder).to_string(), Instant::now() + Duration::from_secs(6)));
        });
        update_status();
        return;
    }
    focus_view(i);
}

pub(crate) unsafe fn can_show(i: usize) -> bool {
    locked(i) && selected_path(i).is_some()
}

pub(crate) unsafe fn show_selection(i: usize) {
    let Some(path) = selected_path(i) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    show_item(i, &path);
}

/// Navigate this pane to the target's folder and select the file.
pub(crate) unsafe fn show_item(i: usize, path: &str) {
    let target = if crate::folderapp::link_file(path) {
        match read_target(path) {
            Ok(Some(p)) => p,
            _ => {
                _ = MessageBeep(MB_ICONWARNING);
                return;
            }
        }
    } else if is_fs(path) {
        path.to_string()
    } else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let target = expand_vars(&target);
    let Some((folder, name)) = parent_and_name(&target) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if is_fs(&target) && !share_path(&target) && local_path_gone(&target) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    if let Ok(mut rev) = REVEAL.lock() {
        *rev = Some(Reveal { pane: i, folder: folder.clone(), name, left: 8, wait: 40 });
    }
    if !navigate(i, &folder) {
        clear_reveal(i);
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let here = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    if same_dir(&here, &folder) && !try_select(i, &here) && reveal_is(i) {
        // Already in the parent, but the view was not ready to select. Retry on the timer.
        arm_timer(i);
    }
    focus_view(i);
}

/// This pane's browse did not finish. Drop Show in folder so a later visit does not select it.
pub(crate) fn nav_failed(i: usize) {
    clear_reveal(i);
}

/// After a browse finishes. Sorts Recent newest-first and selects a file we just revealed.
pub(crate) unsafe fn on_arrived(i: usize, path: &str) {
    let was = showing(i);
    note_folder(i, path);
    let revealing = reveal_is(i) && reveal_folder_is(path);
    if !showing(i) {
        if let Ok(mut later) = LATER.lock() {
            later[i] = Later::QUIET;
        }
        if reveal_is(i) && !revealing {
            clear_reveal(i);
        }
        if revealing {
            let _ = try_select(i, path);
            if reveal_is(i) {
                arm_timer(i);
                return;
            }
        }
        kill_timer(i);
        return;
    }
    sort_newest(i);
    let refresh = !was;
    if let Ok(mut later) = LATER.lock() {
        later[i].refresh = refresh;
        later[i].sort_left = 4;
    }
    if !refresh {
        let _ = try_select(i, path);
    }
    arm_timer(i);
}

pub(crate) unsafe fn on_timer(h: HWND) {
    let Some(i) = pane_of(h) else {
        _ = KillTimer(Some(h), TIMER_RECENT);
        return;
    };
    let (refresh, sort) = {
        let Ok(mut later) = LATER.lock() else {
            _ = KillTimer(Some(h), TIMER_RECENT);
            return;
        };
        let t = &mut later[i];
        let refresh = std::mem::replace(&mut t.refresh, false);
        let sort = t.sort_left > 0;
        if sort {
            t.sort_left -= 1;
        }
        (refresh, sort)
    };
    if refresh {
        if let Some(sv) = view_of::<IShellView>(i) {
            _ = sv.Refresh();
        }
    }
    if sort && showing(i) {
        sort_newest(i);
    }
    let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    let alive = step_reveal(i, &path);
    let pending = LATER.lock().ok().is_some_and(|later| later[i].sort_left > 0 || later[i].refresh);
    if !alive && !pending {
        _ = KillTimer(Some(h), TIMER_RECENT);
    }
}

/// `true` when this child of the Recent folder should be left out of the view.
pub(crate) unsafe fn hide_missing(slot: usize, sf: &IShellFolder, pidl: *const ITEMIDLIST) -> bool {
    if pidl.is_null() || !showing(slot) {
        return false;
    }
    let rel = child_parsing(sf, pidl);
    if rel.is_empty() {
        return false;
    }
    let full = child_path(&folder_path(), &rel);
    if !crate::folderapp::link_file(&full) {
        return false;
    }
    let kind = match read_target(&full) {
        Ok(Some(p)) => {
            let p = expand_vars(&p);
            let fs = is_fs(&p);
            let share = fs && share_path(&p);
            let gone = fs && !share && local_path_gone(&p);
            classify(true, false, fs, share, gone)
        }
        Ok(None) => classify(false, link_has_pidl(&full), false, false, false),
        Err(_) => TargetKind::Unprobed,
    };
    !show_target(kind)
}

/// Date modified, descending. A Recent shortcut is rewritten when its target is used.
unsafe fn sort_newest(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let col = SORTCOLUMN { propkey: crate::folderview::PKEY_DateModified, direction: SORTDIRECTION(-1) };
    _ = fv.SetSortColumns(&[col]);
}

fn reveal_is(i: usize) -> bool {
    REVEAL.lock().ok().is_some_and(|g| g.as_ref().is_some_and(|r| r.pane == i))
}

fn reveal_folder_is(path: &str) -> bool {
    REVEAL.lock().ok().is_some_and(|g| g.as_ref().is_some_and(|r| same_dir(&r.folder, path)))
}

fn clear_reveal(i: usize) {
    if let Ok(mut rev) = REVEAL.lock() {
        if rev.as_ref().is_some_and(|r| r.pane == i) {
            *rev = None;
        }
    }
}

unsafe fn try_select(i: usize, path: &str) -> bool {
    let job = REVEAL.lock().ok().and_then(|g| g.as_ref().filter(|r| r.pane == i && same_dir(&r.folder, path)).map(|r| r.name.clone()));
    let Some(name) = job else { return false };
    if select_child(i, &name) {
        clear_reveal(i);
        focus_view(i);
        true
    } else {
        false
    }
}

unsafe fn step_reveal(i: usize, path: &str) -> bool {
    let job = {
        let Ok(rev) = REVEAL.lock() else { return false };
        let Some(r) = rev.as_ref() else { return false };
        if r.pane != i {
            return false;
        }
        Some((r.name.clone(), r.folder.clone(), r.left, r.wait))
    };
    let Some((name, folder, mut left, mut wait)) = job else { return false };
    let here = same_dir(&folder, path);
    if here {
        left = left.saturating_sub(1);
    } else {
        wait = wait.saturating_sub(1);
    }
    let stop = if here { left == 0 } else { wait == 0 };
    if let Ok(mut rev) = REVEAL.lock() {
        if let Some(r) = rev.as_mut() {
            if r.pane == i {
                r.left = left;
                r.wait = wait;
                if stop {
                    *rev = None;
                }
            }
        }
    }
    if stop || !here {
        return !stop && !here && wait > 0;
    }
    if select_child(i, &name) {
        clear_reveal(i);
        focus_view(i);
        return false;
    }
    left > 0
}

unsafe fn select_child(i: usize, name: &str) -> bool {
    let Some(fv) = view_of::<IFolderView2>(i) else { return false };
    let Ok(folder) = fv.GetFolder::<IShellFolder>() else { return false };
    let mut pidl = std::ptr::null_mut();
    let parsed = folder.ParseDisplayName(HWND::default(), None, &HSTRING::from(name), None, &mut pidl, std::ptr::null_mut());
    if parsed.is_err() || pidl.is_null() {
        if !pidl.is_null() {
            CoTaskMemFree(Some(pidl as _));
        }
        return false;
    }
    let Some(sv) = view_of::<IShellView>(i) else {
        CoTaskMemFree(Some(pidl as _));
        return false;
    };
    let flags = (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    let ok = sv.SelectItem(pidl, flags).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

unsafe fn selected_path(i: usize) -> Option<String> {
    let fv = view_of::<IFolderView2>(i)?;
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let focused = fv.GetFocusedItem().ok().filter(|&k| k >= 0 && (n == 0 || k < n));
    let item = focused.and_then(|k| fv.GetItem::<IShellItem>(k).ok()).or_else(|| {
        let arr = fv.GetSelection(false).ok()?;
        if arr.GetCount().ok()? != 1 {
            return None;
        }
        arr.GetItemAt(0).ok()
    })?;
    let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| crate::pwstr(p)).unwrap_or_default();
    (!path.is_empty()).then_some(path)
}

fn arm_timer(i: usize) {
    let h = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd).unwrap_or_default());
    if !h.0.is_null() {
        unsafe { SetTimer(Some(h), TIMER_RECENT, 80, None) };
    }
}

fn kill_timer(i: usize) {
    let h = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd).unwrap_or_default());
    if !h.0.is_null() {
        unsafe { _ = KillTimer(Some(h), TIMER_RECENT) };
    }
}

unsafe fn child_parsing(sf: &IShellFolder, pidl: *const ITEMIDLIST) -> String {
    let mut sr = STRRET::default();
    let flags = SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0);
    if sf.GetDisplayNameOf(pidl, flags, &mut sr).is_err() {
        return String::new();
    }
    let mut name = PWSTR::null();
    if StrRetToStrW(&mut sr, Some(pidl), &mut name).is_err() || name.0.is_null() {
        return String::new();
    }
    crate::pwstr(name)
}

/// Stored target path. `Ok(None)` is a link with no filesystem path. `Err` is unreadable: keep it.
unsafe fn read_target(path: &str) -> Result<Option<String>> {
    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
    link.cast::<IPersistFile>()?.Load(&HSTRING::from(path), STGM_READ)?;
    if let Some(p) = link_path(&link, SLGP_RAWPATH).or_else(|| link_path(&link, 0)) {
        return Ok(Some(expand_vars(&p)));
    }
    Ok(None)
}

unsafe fn link_has_pidl(path: &str) -> bool {
    let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER) else { return false };
    if link.cast::<IPersistFile>().and_then(|f| f.Load(&HSTRING::from(path), STGM_READ)).is_err() {
        return false;
    }
    match link.GetIDList() {
        Ok(pidl) if !pidl.is_null() => {
            CoTaskMemFree(Some(pidl as _));
            true
        }
        _ => false,
    }
}

unsafe fn link_path(link: &IShellLinkW, flags: u32) -> Option<String> {
    let mut buf = [0u16; 4096];
    if link.GetPath(&mut buf, std::ptr::null_mut(), flags).is_err() {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0)?;
    let s = String::from_utf16_lossy(&buf[..end]);
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_match_ignoring_case_slash_and_prefix() {
        assert!(same_dir(r"C:\Users\A\AppData\Roaming\Microsoft\Windows\Recent", r"c:\users\a\appdata\roaming\microsoft\windows\recent\"));
        assert!(same_dir(r"\\?\C:\Recent", r"C:\Recent"));
        assert!(!same_dir(r"C:\Recent", r"C:\Recent\other"));
        assert!(!same_dir("", r"C:\Recent"));
        assert!(path_is_recent_dir("shell:Recent", ""));
        assert!(path_is_recent_dir(r"::{AE50C081-EBD2-438A-8655-8A092E34987A}", ""));
        assert!(!path_is_recent_dir(r"C:\Windows", r"C:\Users\a\AppData\Roaming\Microsoft\Windows\Recent"));
        assert!(path_is_recent_dir(r"C:\Users\a\AppData\Roaming\Microsoft\Windows\Recent", r"C:\Users\a\AppData\Roaming\Microsoft\Windows\Recent"));
    }

    #[test]
    fn parent_of_a_file_and_a_drive_child() {
        assert_eq!(parent_and_name(r"C:\Work\notes.txt").as_ref().map(|(d, n)| (d.as_str(), n.as_str())), Some((r"C:\Work", "notes.txt")));
        assert_eq!(parent_and_name(r"C:\Windows").as_ref().map(|(d, n)| (d.as_str(), n.as_str())), Some((r"C:\", "Windows")));
        assert_eq!(parent_and_name(r"\\server\share\a.txt").as_ref().map(|(d, n)| (d.as_str(), n.as_str())), Some((r"\\server\share", "a.txt")));
        assert_eq!(parent_and_name(r"C:\"), None);
        assert_eq!(parent_and_name(""), None);
        assert_eq!(child_path(r"C:\Recent", "a.lnk"), r"C:\Recent\a.lnk");
        assert_eq!(child_path(r"C:\Recent\", r"D:\other\a.lnk"), r"D:\other\a.lnk");
    }

    #[test]
    fn missing_local_targets_are_hidden() {
        assert!(!show_target(classify(true, false, true, false, true)));
        assert!(!show_target(classify(false, false, false, false, false)));
        assert!(show_target(classify(true, false, true, false, false)));
        assert!(show_target(classify(true, false, true, true, true)));
        assert!(show_target(classify(false, true, false, false, false)));
        assert!(show_target(classify(true, false, false, false, true)));
    }

    #[test]
    fn edits_are_the_blocked_keys_and_verbs() {
        assert!(blocks_key(0x2E, false, false, false));
        assert!(blocks_key(0x71, false, false, false));
        assert!(blocks_key(b'X' as u16, true, false, false));
        assert!(blocks_key(b'V' as u16, true, false, true));
        assert!(!blocks_key(b'R' as u16, true, true, false));
        assert!(!blocks_key(0x0D, true, false, false));
        assert!(write_verb("delete"));
        assert!(write_verb("Windows.Cut"));
        assert!(write_verb("paste"));
        assert!(write_verb("rename"));
        assert!(!write_verb("open"));
        assert!(!write_verb("properties"));
        assert!(!write_verb(""));
    }

    #[test]
    fn environment_tokens_expand() {
        let key = "MAGURO_RECENT_TEST";
        std::env::set_var(key, r"C:\Users\a");
        assert_eq!(expand_vars(&format!("%{key}%\\notes.txt")), r"C:\Users\a\notes.txt");
        assert_eq!(expand_vars(r"C:\plain"), r"C:\plain");
        assert_eq!(expand_vars(r"%MAGURO_RECENT_MISSING%\a"), r"%MAGURO_RECENT_MISSING%\a");
        std::env::remove_var(key);
    }
}
