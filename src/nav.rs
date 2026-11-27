use crate::*;
use windows::Win32::System::SystemServices::SFGAO_FILESYSTEM;

// ---------- navigation ----------

pub(crate) unsafe fn navigate(i: usize, path: &str) -> bool {
    try_nav(i, path).is_ok()
}

pub(crate) unsafe fn try_nav(i: usize, path: &str) -> Result<()> {
    let eb = eb_of(i).ok_or_else(|| Error::from(E_FAIL))?;
    let mut pidl = std::ptr::null_mut();
    SHParseDisplayName(&HSTRING::from(path), None, &mut pidl, 0, None)?;
    let r = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE);
    CoTaskMemFree(Some(pidl as _));
    r
}

// The shell's answer for a path that isn't there; denied, offline or not ready are something else.
pub(crate) fn gone(code: HRESULT) -> bool {
    code == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) || code == HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)
}

// Nearest folder above `path` that still exists. Stops at anything but "not found" (denied, a file).
pub(crate) fn near_dir(path: &str, probe: impl Fn(&Path) -> std::io::Result<bool>) -> Option<String> {
    for p in Path::new(path).ancestors().skip(1) {
        match probe(p) {
            Ok(true) => return Some(p.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return None,
        }
    }
    None
}

#[derive(Clone, Copy)]
pub(crate) enum Nav {
    Pending,
    Complete,
    Failed,
}

// A pane's recovery (missing folder, parent being opened) through its navigation events. Returns the missing
// folder once the parent has opened; another navigation starting, or the parent failing, drops it quietly.
pub(crate) fn rec_step(rec: &mut Option<(String, String)>, ev: Nav, path: &str) -> Option<String> {
    let hit = rec.as_ref().is_some_and(|(_, t)| t.eq_ignore_ascii_case(path));
    match ev {
        Nav::Pending if !hit => *rec = None,
        Nav::Failed if hit => *rec = None,
        Nav::Complete => return rec.take().filter(|_| hit).map(|(m, _)| m),
        _ => {}
    }
    None
}

// Like Total Commander / XYplorer: a local folder that no longer exists opens its nearest existing parent,
// and the status line says so once that has opened (on_nav). Anything else fails as navigate() does.
// ponytail: local drives only, so the parent probe never blocks; shares would need a worker thread.
pub(crate) unsafe fn go_near(i: usize, path: &str) -> bool {
    let Err(e) = try_nav(i, path) else { return true };
    if !gone(e.code()) {
        return false;
    }
    // a confirmed missing folder leaves the recent list, and the pane's move away doesn't put it back (visit)
    app(|a| {
        a.s.recent.retain(|r| !r.eq_ignore_ascii_case(path));
        if a.s.paths[i].eq_ignore_ascii_case(path) {
            a.s.paths[i].clear();
        }
    });
    let local = path.get(1..3) == Some(":\\") && matches!(GetDriveTypeW(&HSTRING::from(&path[..3])), 2 | 3); // DRIVE_REMOVABLE, DRIVE_FIXED
    if !local {
        return false;
    }
    let Some(up) = near_dir(path, |p| std::fs::metadata(p).map(|m| m.is_dir())) else { return false };
    browse_marked(i, &up, path, false)
}

// Opens `to` in pane i with a mark (the folder it stands in for, `to`'s parsing name) on the pane: its `recover`,
// or its `follow` for a renamed folder. on_nav acts on it once `to` has opened; another navigation drops it.
pub(crate) unsafe fn browse_marked(i: usize, to: &str, from: &str, follow: bool) -> bool {
    let Some(eb) = eb_of(i) else { return false };
    let mut pidl = std::ptr::null_mut();
    if SHParseDisplayName(&HSTRING::from(to), None, &mut pidl, 0, None).is_err() {
        return false;
    }
    let target = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
    // set before browsing: a pane's first navigation completes inside BrowseToIDList
    let rec = |set: bool| {
        app(|a| {
            if let Some(p) = a.panes[i].as_mut() {
                let m = if follow { &mut p.follow } else { &mut p.recover };
                if set {
                    *m = Some((from.to_string(), target.clone()));
                } else if m.as_ref().is_some_and(|(_, t)| *t == target) {
                    *m = None;
                }
            }
        })
    };
    rec(true);
    let ok = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE).is_ok();
    if !ok {
        rec(false);
    }
    CoTaskMemFree(Some(pidl as _));
    ok
}

// "Not found: … · opened …" for 8 s; more recoveries while it shows merge into one count.
pub(crate) fn gone_note(a: &mut App, missing: &str, opened: &str) {
    let showing = a.note.as_ref().is_some_and(|(_, t)| Instant::now() < *t);
    a.lost = lost_next(a.lost, showing);
    let text = gone_text(KO.load(Ordering::Relaxed), a.lost, missing, opened);
    a.note = Some((text, Instant::now() + Duration::from_secs(8)));
}

// The count after one more missing folder: added to the last one while its note still shows, else a fresh 1.
pub(crate) fn lost_next(lost: usize, showing: bool) -> usize {
    if lost > 0 && showing { lost + 1 } else { 1 }
}

pub(crate) fn gone_text(ko: bool, n: usize, missing: &str, opened: &str) -> String {
    match (ko, n) {
        (false, 1) => format!("Not found: {missing} · opened {opened}"),
        (true, 1) => format!("없는 폴더: {missing} · {opened} 엶"),
        (false, n) => format!("{n} saved folders were missing · opened the nearest parent folders"),
        (true, n) => format!("저장된 폴더 {n}개가 없어 가장 가까운 상위 폴더를 엶"),
    }
}

// ---------- folders renamed or removed elsewhere ----------
// Like an Explorer window (and Total Commander / XYplorer for a removal): a pane inside a folder another pane,
// Explorer or any shell app renamed opens its new name; one inside a removed folder opens the nearest parent.
// The notification only queues; TIMER_SHELL looks at the disk once, after a short pause, for the whole batch.
// ponytail: local fixed drives only (no blocking probe); renames that bypass the shell aren't seen. A drive letter
// that goes away (USB pulled) is drives_changed's.

pub(crate) const WM_SHELL_CHANGE: u32 = WM_APP + 51; // wParam = notification handle, lParam = process id (SHCNRF_NewDelivery)
pub(crate) const TIMER_SHELL: usize = 1160; // on main: the queued changes are looked at
static SHELL_REG: AtomicU32 = AtomicU32::new(0); // SHChangeNotifyRegister id, 0 = none

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Change {
    Renamed(String, String), // old, new
    Removed(String),         // also a move into the Recycle Bin
}

#[derive(Debug, PartialEq)]
pub(crate) enum Follow {
    To(String),
    Gone,
}

// What follows `dir` in `path` when `path` is `dir` or inside it: whole names only, any case.
pub(crate) fn under<'a>(path: &'a str, dir: &str) -> Option<&'a str> {
    let rest = path.get(dir.len()..)?;
    let whole = rest.is_empty() || rest.starts_with('\\') || dir.ends_with('\\');
    (!dir.is_empty() && whole && path[..dir.len()].eq_ignore_ascii_case(dir)).then_some(rest)
}

// `path` with its leading `old` swapped for `new`; None when it isn't `old` or inside it (C:\a\bc is not in C:\a\b).
pub(crate) fn remap_dir(path: &str, old: &str, new: &str) -> Option<String> {
    under(path, old).map(|rest| format!("{new}{rest}"))
}

pub(crate) fn in_bin(path: &str) -> bool {
    let l = path.get(2..).unwrap_or("").to_ascii_lowercase();
    l == r"\$recycle.bin" || l.starts_with(r"\$recycle.bin\")
}

// Where `path` ends up after `changes` in order (a rename of a rename is followed through). None: untouched, or
// unclear: another folder took its place (path reused), or a removed one came back under a new name.
pub(crate) fn follow(path: &str, changes: &[Change]) -> Option<Follow> {
    let (mut cur, mut gone) = (path.to_string(), false);
    for c in changes {
        match c {
            Change::Renamed(old, new) => {
                if under(&cur, new).is_some() && under(&cur, old).is_none() {
                    return None;
                }
                if let Some(p) = remap_dir(&cur, old, new) {
                    if gone {
                        return None;
                    }
                    cur = p;
                }
            }
            Change::Removed(old) => gone |= under(&cur, old).is_some(),
        }
    }
    if gone {
        Some(Follow::Gone)
    } else {
        (cur != path).then_some(Follow::To(cur))
    }
}

// `list` (recent folders, favourites) after `changes`: a folder renamed on disk (`on_disk`), or one inside it, takes
// its new name in place; a removed one stays (it may come back from the Recycle Bin). Names that end up the same,
// any case, keep only the first. The flag: at least one entry took a new name.
pub(crate) fn follow_list(list: &[String], changes: &[Change], on_disk: impl Fn(&str, &str) -> bool) -> (Vec<String>, bool) {
    let (mut out, mut moved) = (Vec::<String>::with_capacity(list.len()), false);
    for p in list {
        let p = match follow(p, changes) {
            Some(Follow::To(t)) if on_disk(p, &t) => {
                moved = true;
                t
            }
            _ => p.clone(),
        };
        if !out.iter().any(|s| s.eq_ignore_ascii_case(&p)) {
            out.push(p);
        }
    }
    (out, moved)
}

fn fixed_drive(path: &str) -> bool {
    path.get(1..3) == Some(":\\") && unsafe { GetDriveTypeW(&HSTRING::from(&path[..3])) } == 3 // DRIVE_FIXED
}

fn missing(path: &str) -> bool {
    std::fs::metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

// The rename really happened: `from` is gone and `to` is a folder; a change of case only shows in the name on disk.
fn moved_on_disk(from: &str, to: &str) -> bool {
    if !fixed_drive(from) {
        return false;
    }
    if from.eq_ignore_ascii_case(to) {
        return std::fs::canonicalize(to).is_ok_and(|c| c.to_str().and_then(|c| c.strip_prefix(r"\\?\")) == Some(to));
    }
    missing(from) && std::fs::metadata(to).is_ok_and(|m| m.is_dir())
}

pub(crate) unsafe fn watch_folders(h: HWND) {
    static DESKTOP: [u16; 2] = [0; 2]; // an empty ID list: the desktop, with every folder under it
    let e = SHChangeNotifyEntry { pidl: DESKTOP.as_ptr() as *mut ITEMIDLIST, fRecursive: true.into() };
    let ev = (SHCNE_RENAMEFOLDER.0 | SHCNE_RMDIR.0) as i32;
    SHELL_REG.store(SHChangeNotifyRegister(h, SHCNRF_ShellLevel | SHCNRF_NewDelivery, ev, WM_SHELL_CHANGE, 1, &e), Ordering::Relaxed);
}

pub(crate) unsafe fn unwatch_folders() {
    for reg in [&SHELL_REG, &BIN_REG] {
        let id = reg.swap(0, Ordering::Relaxed);
        if id != 0 {
            _ = SHChangeNotifyDeregister(id);
        }
    }
}

// The Recycle Bin's own changes (not its subfolders): the toolbar's bin button counts again 500 ms after the last
// one (bin_soon), so a big delete is counted once. Only a hint: activation and WM_DRIVES count it too.
pub(crate) const WM_BIN_CHANGE: u32 = WM_APP + 60; // wParam = notification handle, lParam = process id (SHCNRF_NewDelivery)
static BIN_REG: AtomicU32 = AtomicU32::new(0);

pub(crate) unsafe fn watch_bin(h: HWND) {
    let Ok(pidl) = SHGetKnownFolderIDList(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT.0 as u32, None) else { return };
    let e = SHChangeNotifyEntry { pidl, fRecursive: false.into() };
    let ev = (SHCNE_CREATE.0 | SHCNE_DELETE.0 | SHCNE_UPDATEDIR.0 | SHCNE_UPDATEITEM.0 | SHCNE_UPDATEIMAGE.0) as i32;
    BIN_REG.store(SHChangeNotifyRegister(h, SHCNRF_ShellLevel | SHCNRF_NewDelivery, ev, WM_BIN_CHANGE, 1, &e), Ordering::Relaxed);
    // the ID list is kept for the life of the process (one, a few bytes): nothing to free while registered
}

// WM_BIN_CHANGE: the notification's shared memory is let go; what changed doesn't matter, the bin is counted again.
pub(crate) unsafe fn bin_change(wp: WPARAM, lp: LPARAM) {
    let mut pp: *mut *mut ITEMIDLIST = std::ptr::null_mut();
    let mut ev = 0i32;
    let lock = SHChangeNotification_Lock(HANDLE(wp.0 as _), lp.0 as u32, Some(&mut pp), Some(&mut ev));
    if !lock.is_invalid() {
        _ = SHChangeNotification_Unlock(lock);
    }
    bin_soon();
}

// WM_SHELL_CHANGE: the two ID lists become paths (copied while locked), and a change that concerns a pane, a tab,
// the recent list, a favourite or an earlier queued change waits for TIMER_SHELL. No disk calls here.
pub(crate) unsafe fn shell_change(h: HWND, wp: WPARAM, lp: LPARAM) {
    let mut pp: *mut *mut ITEMIDLIST = std::ptr::null_mut();
    let mut ev = 0i32;
    let lock = SHChangeNotification_Lock(HANDLE(wp.0 as _), lp.0 as u32, Some(&mut pp), Some(&mut ev));
    if lock.is_invalid() {
        return;
    }
    let path = |k: usize| {
        let p = if pp.is_null() { std::ptr::null_mut() } else { *pp.add(k) };
        let mut buf = vec![0u16; 32768];
        if p.is_null() || !SHGetPathFromIDListEx(p, &mut buf, GPFIDL_DEFAULT).as_bool() {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)])
    };
    let change = if ev as u32 & SHCNE_RENAMEFOLDER.0 != 0 {
        let (old, new) = (path(0), path(1));
        // a new name that cannot be read (or one inside the Recycle Bin) counts as a removal: shell_tick still checks
        // that the pane's folder is really gone before it moves.
        if new.is_empty() || in_bin(&new) { Change::Removed(old) } else { Change::Renamed(old, new) }
    } else if ev as u32 & SHCNE_RMDIR.0 != 0 {
        Change::Removed(path(0))
    } else {
        Change::Removed(String::new())
    };
    _ = SHChangeNotification_Unlock(lock);
    let old = match &change {
        Change::Renamed(o, _) | Change::Removed(o) => o.clone(),
    };
    if old.len() < 4 || !is_fs(&old) {
        return;
    }
    let start = app(|a| {
        let s = &a.s;
        let ours = s.paths.iter().chain(s.tabs.iter().flatten()).chain(&s.recent).chain(&s.favs).any(|p| under(p, &old).is_some());
        let chained = a.shell_q.iter().any(|c| matches!(c, Change::Renamed(_, n) if under(n, &old).is_some() || under(&old, n).is_some()));
        if !(ours || chained) || a.shell_q.len() >= 256 {
            return false;
        }
        a.shell_q.push(change);
        a.shell_q.len() == 1
    });
    if start {
        SetTimer(Some(h), TIMER_SHELL, 300, None);
    }
}

// TIMER_SHELL: renames that really happened move tabs, the recent list and favourites along, and panes open the new
// name (on_nav confirms it and says so); a pane inside a removed folder opens its nearest parent (go_near).
pub(crate) unsafe fn shell_tick(h: HWND) {
    _ = KillTimer(Some(h), TIMER_SHELL);
    let (changes, paths, tabs, recent, favs, closed) = app(|a| {
        (std::mem::take(&mut a.shell_q), a.s.paths.clone(), a.s.tabs.clone(), a.s.recent.clone(), a.s.favs.clone(), a.closed_tabs.clone())
    });
    if changes.is_empty() {
        return;
    }
    let moved = |p: &String| match follow(p, &changes) {
        Some(Follow::To(t)) if moved_on_disk(p, &t) => t,
        _ => p.clone(),
    };
    let tabs: Vec<Vec<String>> = tabs.iter().map(|t| t.iter().map(moved).collect()).collect();
    let closed: Vec<ClosedTab> = closed.into_iter().map(|c| ClosedTab { path: moved(&c.path), ..c }).collect();
    let (recent, _) = follow_list(&recent, &changes, moved_on_disk);
    // favourites change only when one took a new name: a case-only duplicate typed into grids.ini stays otherwise
    let favs = Some(follow_list(&favs, &changes, moved_on_disk)).filter(|f| f.1).map(|f| f.0);
    // written back from the same snapshot with no message in between, so an added or removed favourite isn't lost
    let fav_moved = favs.is_some();
    let (live, fewer, popup) = app(|a| {
        a.s.recent = recent;
        a.closed_tabs = closed;
        for (i, t) in tabs.into_iter().enumerate().take(a.s.tabs.len()) {
            a.s.tabs[i] = t;
        }
        let fewer = favs.as_ref().is_some_and(|f| f.len() < a.s.favs.len());
        if let Some(f) = favs {
            a.s.favs = f;
        }
        ((0..MAXP).filter(|&i| a.panes[i].is_some()).collect::<Vec<_>>(), fewer, a.fav_popup)
    });
    if fav_moved {
        // the ★ list, if open: fewer rows leave its row indexes and height stale, so it closes; else it repaints
        if !popup.0.is_null() {
            if fewer {
                app(|a| (a.fav_hot, a.fav_press) = (None, None));
                _ = SendMessageW(popup, WM_CLOSE, None, None);
            } else {
                _ = InvalidateRect(Some(popup), None, false);
            }
        }
        repaint_fav_buttons();
    }
    for i in live {
        let p = &paths[i];
        if is_fs(p) {
            match follow(p, &changes) {
                Some(Follow::To(t)) if moved_on_disk(p, &t) => _ = browse_marked(i, &t, p, true),
                Some(Follow::Gone) if fixed_drive(p) && missing(p) => {
                    if !go_near(i, p) {
                        app(|a| {
                            if a.s.paths[i].is_empty() {
                                a.s.paths[i] = p.clone(); // no parent opened: the pane keeps its folder
                            }
                        });
                    }
                }
                _ => {}
            }
        }
        if tabs_on() {
            repaint_tabs(i);
        }
    }
}

// "Followed renamed folder: name" for 5 s.
pub(crate) fn follow_note(a: &mut App, disp: &str) {
    a.note = Some((follow_text(KO.load(Ordering::Relaxed), disp), Instant::now() + Duration::from_secs(5)));
    a.lost = 0;
}

pub(crate) fn follow_text(ko: bool, disp: &str) -> String {
    let leaf = disp.trim_end_matches('\\').rsplit('\\').next().unwrap_or(disp);
    if ko { format!("폴더 이름이 바뀌어 따라감: {leaf}") } else { format!("Followed renamed folder: {leaf}") }
}

// Pending / Failed events of a pane with a recovery, a rename follow or a pulled drive's mark; Complete goes through on_nav.
pub(crate) unsafe fn rec_event(i: usize, ev: Nav, pidl: *const ITEMIDLIST) {
    if app(|a| a.panes[i].as_ref().is_some_and(|p| p.recover.is_some() || p.follow.is_some() || p.unplugged.is_some())) {
        let path = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
        app(|a| {
            a.panes[i].as_mut().map(|p| {
                unplug_step(&mut p.unplugged, ev, &path);
                rec_step(&mut p.recover, ev, &path);
                rec_step(&mut p.follow, ev, &path)
            })
        });
    }
}

// ---------- drives that went or came back ----------
// Like Explorer closing its window on a pulled USB stick (Total Commander / Directory Opus move the panel away): a
// pane in a folder on a drive letter that went opens This PC, marked with that folder; when the letter comes back
// (removable or fixed only, the kinds go_near looks at) a pane still on This PC reopens it. Letters only
// (GetLogicalDrives): a letter that went is never looked at, so a cut network drive or subst can't block.
// ponytail: a card reader or CD drive keeps its letter with nothing in it, so it isn't seen here.

// A drive letter's bit (A = bit 0) for a path on it, any case: "E:\x", "e:". None for \\server, ::{GUID} and the like.
pub(crate) fn drive_bit(path: &str) -> Option<u32> {
    let b = path.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b.get(2), None | Some(b'\\'))).then(|| 1 << (b[0].to_ascii_uppercase() - b'A'))
}

// The panes a drive change moves: (to This PC: in a folder on a letter that `gone`, not marked yet; back home:
// marked with a folder on a letter now `back` and still on This PC, so a place the user picked meanwhile stays).
pub(crate) fn drive_moves(gone: u32, back: u32, paths: &[String], marks: &[Option<&str>]) -> (Vec<usize>, Vec<usize>) {
    let on = |p: &str, m: u32| drive_bit(p).is_some_and(|b| b & m != 0);
    let away = (0..paths.len().min(marks.len())).filter(|&i| marks[i].is_none() && on(&paths[i], gone)).collect();
    let home = (0..paths.len().min(marks.len())).filter(|&i| marks[i].is_some_and(|m| on(m, back)) && paths[i].eq_ignore_ascii_case(THIS_PC)).collect();
    (away, home)
}

// A pulled drive's mark through the pane's navigation events: kept while its own This PC is opening (and once it
// has), dropped by anything else, including a later This PC (a session, the user's pick), which a drive coming
// back must not replace.
pub(crate) fn unplug_step(m: &mut Option<(String, bool)>, ev: Nav, path: &str) {
    let Some((_, reached)) = m.as_mut() else { return };
    let ours = !*reached && path.eq_ignore_ascii_case(THIS_PC);
    match ev {
        Nav::Complete if ours => *reached = true,
        Nav::Pending if ours => {}
        _ => *m = None,
    }
}

// "Drive E: was removed · moved to This PC" / "Drive E: is back · reopened its folder" (5 s); several panes, one count.
pub(crate) fn unplug_text(ko: bool, back: bool, drives: &str, n: usize) -> String {
    let (en, k) = match (back, n) {
        (false, 1) => ("Drive {drives} was removed · moved to This PC", "{drives} 드라이브가 빠져 내 PC로 옮김"),
        (false, _) => ("Drive {drives} was removed · moved {n} panes to This PC", "{drives} 드라이브가 빠져 패널 {n}개를 내 PC로 옮김"),
        (true, 1) => ("Drive {drives} is back · reopened its folder", "{drives} 드라이브가 돌아와 원래 폴더를 다시 엶"),
        (true, _) => ("Drive {drives} is back · reopened the folders of {n} panes", "{drives} 드라이브가 돌아와 패널 {n}개의 원래 폴더를 다시 엶"),
    };
    (if ko { k } else { en }).replace("{drives}", drives).replace("{n}", &n.to_string())
}

// "E:" or "E:, F:" for the letters of these paths, in order, each once.
fn drive_names<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    let mask = paths.filter_map(drive_bit).fold(0, |m, b| m | b);
    mask_letters(mask).iter().map(|&c| format!("{}:", c as char)).collect::<Vec<_>>().join(", ")
}

// From refresh_drives, after the buttons came and went (also at the end of a drag or resize): `gone` letters, and
// `back` ones that are removable or fixed drives.
pub(crate) unsafe fn drives_changed(gone: u32, back: u32) {
    let (away, home, paths) = app(|a| {
        let marks: Vec<Option<&str>> = a.panes.iter().map(|p| p.as_ref().and_then(|p| p.unplugged.as_ref()).map(|m| m.0.as_str())).collect();
        let paths: Vec<String> = (0..a.panes.len()).map(|i| if a.panes[i].is_some() { a.s.paths.get(i).cloned().unwrap_or_default() } else { String::new() }).collect();
        let (away, home) = drive_moves(gone, back, &paths, &marks);
        (away, home, paths)
    });
    let mut moved = Vec::new();
    for i in away {
        let from = paths[i].clone();
        // set before browsing: the mark must be there for this navigation's own events
        app(|a| a.panes[i].as_mut().map(|p| p.unplugged = Some((from.clone(), false))));
        if navigate(i, THIS_PC) {
            moved.push(from);
        } else {
            app(|a| a.panes[i].as_mut().map(|p| p.unplugged = None));
        }
    }
    if !moved.is_empty() {
        let text = unplug_text(KO.load(Ordering::Relaxed), false, &drive_names(moved.iter().map(String::as_str)), moved.len());
        app(|a| (a.note, a.lost) = (Some((text, Instant::now() + Duration::from_secs(5))), 0));
    }
    let homes: Vec<(usize, String)> = app(|a| home.iter().filter_map(|&i| Some((i, a.panes[i].as_mut()?.unplugged.take()?.0))).collect());
    if homes.is_empty() {
        return;
    }
    // said before opening: a folder deleted meanwhile opens its parent, and on_nav's "Not found" then replaces this
    let ko = KO.load(Ordering::Relaxed);
    let text = unplug_text(ko, true, &drive_names(homes.iter().map(|h| h.1.as_str())), homes.len());
    app(|a| (a.note, a.lost) = (Some((text.clone(), Instant::now() + Duration::from_secs(5))), 0));
    let back: Vec<&str> = homes.iter().filter(|(i, p)| go_near(*i, p)).map(|h| h.1.as_str()).collect();
    // a drive not ready yet opens nothing: the pane stays on This PC quietly, and the note counts only what opened
    let now = (!back.is_empty()).then(|| unplug_text(ko, true, &drive_names(back.iter().copied()), back.len()));
    app(|a| {
        if a.note.as_ref().is_some_and(|n| n.0 == text) {
            a.note = now.map(|t| (t, Instant::now() + Duration::from_secs(5)));
        }
    });
}

/// true when the browser accepted the move (false: no pane, or nothing to go back/forward/up to)
pub(crate) unsafe fn browse(i: usize, flags: u32) -> bool {
    let Some(eb) = eb_of(i) else { return false };
    let go = match flags {
        SBSP_NAVIGATEBACK => Go::Back,
        SBSP_NAVIGATEFORWARD => Go::Fwd,
        _ => Go::New,
    };
    // OnNavigationPending (raised inside BrowseToIDList) takes the kind from here; cleared once the call returns,
    // accepted or not, so a Pending that never came cannot hand a stale Back/Fwd to the next, unrelated move
    app(|a| a.panes.get_mut(i).and_then(|p| p.as_mut()).map(|p| p.going = Some(go)));
    let ok = eb.BrowseToIDList(std::ptr::null(), flags).is_ok();
    let fix = app(|a| {
        let p = a.panes.get_mut(i)?.as_mut()?;
        p.going = None;
        // refused: nothing that way after all, whatever was counted (the buttons follow)
        let was = p.dim();
        p.hist = p.hist.map(|(b, f)| hist_refused(b, f, go, ok));
        Some((p.hwnd, was, p.dim()))
    });
    if let Some((h, was, now)) = fix {
        redraw_nav(h, was, now);
    }
    ok
}

// ---------- faint ← / → / ↑ ----------
// Like Explorer, browsers and Finder, a pane's back / forward / up button with nowhere to go is drawn faint (draw.rs);
// clicking it still just does nothing. Grids counts the steps itself from its own events: every move goes through
// OnNavigationPending (kind from browse(), else a new folder) and OnNavigationComplete (on_nav).

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Go {
    New,
    Back,
    Fwd,
}

// Steps back and forward after a finished move; None = the pane's first folder.
pub(crate) fn hist_step(h: Option<(u32, u32)>, go: Go) -> (u32, u32) {
    match (h, go) {
        (None, _) => (0, 0),
        (Some((b, _)), Go::New) => (b + 1, 0),
        (Some((b, f)), Go::Back) => (b.saturating_sub(1), f + 1),
        (Some((b, f)), Go::Fwd) => (b + 1, f.saturating_sub(1)),
    }
}

// The counts after browse() asked to go back or forward: refused means none that way.
pub(crate) fn hist_refused(b: u32, f: u32, go: Go, ok: bool) -> (u32, u32) {
    match (go, ok) {
        (Go::Back, false) => (0, f),
        (Go::Fwd, false) => (b, 0),
        _ => (b, f),
    }
}

// Which of ←, →, ↑ to draw faint.
pub(crate) fn nav_dim(h: Option<(u32, u32)>, root: bool) -> [bool; 3] {
    let (b, f) = h.unwrap_or((0, 0));
    [b == 0, f == 0, root]
}

impl Pane {
    pub(crate) fn dim(&self) -> [bool; 3] {
        nav_dim(self.hist, self.root)
    }
}

pub(crate) fn nav_dim_of(pane: HWND, id: i32) -> bool {
    let k = [ID_BACK, ID_FWD, ID_UP].iter().position(|&x| x == id);
    k.is_some_and(|k| app(|a| a.panes.iter().flatten().find(|p| p.hwnd == pane).is_some_and(|p| p.dim()[k])))
}

unsafe fn redraw_nav(h: HWND, was: [bool; 3], now: [bool; 3]) {
    for (k, id) in [ID_BACK, ID_FWD, ID_UP].into_iter().enumerate() {
        if was[k] != now[k] {
            if let Ok(b) = GetDlgItem(Some(h), id) {
                _ = InvalidateRect(Some(b), None, false);
            }
        }
    }
}

// A finished move: its kind (taken at OnNavigationPending) applied to the counts; ↑ faint at the desktop (empty ID list).
unsafe fn nav_counted(i: usize, pidl: *const ITEMIDLIST) {
    let root = pidl.is_null() || (*pidl).mkid.cb == 0;
    let fix = app(|a| {
        let p = a.panes.get_mut(i)?.as_mut()?;
        let was = p.dim();
        p.hist = Some(hist_step(p.hist, p.moving.take().unwrap_or(Go::New)));
        p.root = root;
        Some((p.hwnd, was, p.dim()))
    });
    if let Some((h, was, now)) = fix {
        redraw_nav(h, was, now);
    }
}

pub(crate) unsafe fn browse_item(i: usize, si: &IShellItem) {
    let Some(eb) = eb_of(i) else { return };
    if let Ok(pidl) = SHGetIDListFromObject(si) {
        _ = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE);
        CoTaskMemFree(Some(pidl as _));
    }
}

pub(crate) unsafe fn focus_view(i: usize) {
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32);
        if let Ok(h) = sv.GetWindow() {
            _ = SetFocus(Some(h));
        }
    }
}

// Grids going to the back (another app, a popup, minimize): remember which of its controls had focus.
pub(crate) unsafe fn keep_focus(main: HWND) {
    let f = GetFocus();
    if IsChild(main, f).as_bool() {
        app(|a| a.focus_keep = f);
    }
}

// Coming back: focus where keep_focus left it if that is still the active pane's address / filter / list or the
// sidebar, else the active list. The edits keep their text, caret and selection (no EM_SETSEL).
pub(crate) unsafe fn restore_focus() {
    let (main, i, k, tree, th) = app(|a| (a.main, a.s.active, a.focus_keep, a.s.tree, a.tree_hwnd));
    let shown = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd)).is_some_and(|p| IsWindowVisible(p).as_bool());
    let within = |root: HWND| root == k || IsChild(root, k).as_bool();
    let ok = IsWindow(Some(k)).as_bool()
        && ((shown && (pane_ctl(i, ID_EDIT) == Some(k) || pane_ctl(i, ID_FILTER) == Some(k)))
            || view_of::<IShellView>(i).and_then(|sv| sv.GetWindow().ok()).is_some_and(within)
            || (tree && IsWindowVisible(th).as_bool() && within(th))
            // a toolbar button the keys were on (F6), still shown: the layout / load popup closing comes back there
            || (app(|a| a.tb_kbd) && IsWindowVisible(k).as_bool() && is_toolbar_button(k)));
    if ok {
        if pane_ctl(i, ID_EDIT) == Some(k) {
            _ = ShowWindow(k, SW_SHOWNA); // an idle address edit hides behind its crumbs; EN_SETFOCUS syncs it
        }
        _ = SetFocus(Some(k));
    } else {
        focus_view(i);
    }
    if GetFocus().is_invalid() {
        _ = SetFocus(Some(main)); // as before: keys reach Grids, not WM_SYSKEYDOWN
    }
}

pub(crate) unsafe fn focus_ctl(i: usize, id: i32) -> bool {
    if let Some(e) = pane_ctl(i, id) {
        _ = ShowWindow(e, SW_SHOWNA); // an idle address edit hides behind its crumbs
        _ = SetFocus(Some(e));
        SendMessageW(e, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
    true
}

// Address bar input as (a path, a program and its arguments); either may be missing. Unpaired quotes: nothing.
pub(crate) fn addr_parts(raw: &str) -> Option<(Option<&str>, Option<(&str, &str)>)> {
    let quotes = raw.matches('"').count();
    if quotes % 2 == 1 {
        return None;
    }
    let path = match quotes {
        0 => Some(raw),
        2 if raw.len() > 1 && raw.starts_with('"') && raw.ends_with('"') => Some(&raw[1..raw.len() - 1]),
        _ => None,
    };
    let cmd = if let Some(q) = raw.strip_prefix('"') {
        let (prog, rest) = q.split_once('"')?;
        (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| (prog, rest.trim()))
    } else {
        let (prog, rest) = raw.split_once(char::is_whitespace).unwrap_or((raw, ""));
        (!prog.contains(['\\', '/', ':', '"'])).then(|| (prog, rest.trim()))
    };
    Some((path, cmd.filter(|(prog, _)| !prog.is_empty())))
}

// The file names a bare command may be, as cmd.exe tries them: as typed when it has an extension, then with each of
// PATHEXT's.
fn prog_names(prog: &str, exts: &[String]) -> Vec<String> {
    let typed = prog.rsplit_once('.').is_some_and(|(s, e)| !s.is_empty() && !e.is_empty());
    typed.then(|| prog.to_string()).into_iter().chain(exts.iter().map(|e| format!("{prog}{e}"))).collect()
}

/// The first of `dirs` holding one of the names `prog` may be: its full path.
pub(crate) fn find_prog(prog: &str, dirs: &[String], exts: &[String], is_file: impl Fn(&Path) -> bool) -> Option<String> {
    let names = prog_names(prog, exts);
    dirs.iter().find_map(|d| names.iter().map(|n| Path::new(d).join(n)).find(|p| is_file(p))).map(|p| p.to_string_lossy().into_owned())
}

/// Where a bare command typed in the address bar is looked for: System32, Windows, then PATH's absolute folders.
/// Not the pane's folder, which ShellExecuteEx would otherwise search first (a PATH folder stays: it was put there).
pub(crate) fn prog_dirs(sys_root: &str, path_var: &str) -> Vec<String> {
    let root = sys_root.trim_end_matches('\\');
    let path = path_var.split(';').map(|p| p.trim().trim_matches('"').trim_end_matches('\\').to_string());
    let mut out: Vec<String> = Vec::new();
    for d in [format!(r"{root}\System32"), root.to_string()].into_iter().chain(path) {
        if !d.is_empty() && !is_rel(&d) && !out.iter().any(|o| o.eq_ignore_ascii_case(&d)) {
            out.push(d);
        }
    }
    out
}

pub(crate) fn path_exts(pathext: &str) -> Vec<String> {
    let v: Vec<String> = pathext.split(';').map(str::trim).filter(|e| e.len() > 1 && e.starts_with('.')).map(String::from).collect();
    if v.is_empty() { [".COM", ".EXE", ".BAT", ".CMD"].map(String::from).to_vec() } else { v }
}

/// What the address bar hands ShellExecuteEx for a typed command (cmd, wt, powershell, notepad ...): the full path
/// of the program found outside the pane's folder, so a same-named file planted there (cmd.bat, powershell.exe ...)
/// never runs instead. Found nowhere (an App Paths program such as chrome): the bare name as before, unless the
/// pane's folder holds a file it would match; then None, nothing runs. Opening a file there on purpose takes its
/// full name (cmd.exe), which the address bar opens as a file.
pub(crate) fn addr_program(prog: &str, cur: Option<&str>) -> Option<String> {
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let root = Some(env("SystemRoot")).filter(|r| !r.is_empty()).unwrap_or_else(|| r"C:\Windows".into());
    let exts = path_exts(&env("PATHEXT"));
    // symlink_metadata: the Store's app aliases (wt.exe in WindowsApps) are reparse points metadata can't follow
    let is_file = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_dir());
    if let Some(full) = find_prog(prog, &prog_dirs(&root, &env("PATH")), &exts, is_file) {
        return Some(full);
    }
    let planted = cur.is_some_and(|c| find_prog(prog, &[c.to_string()], &exts, is_file).is_some());
    (!planted).then(|| prog.to_string())
}

pub(crate) fn is_rel(p: &str) -> bool {
    !p.contains(':') && !p.starts_with(['\\', '/'])
}

pub(crate) unsafe fn addr_reset(i: usize, e: HWND) {
    let disp = app(|x| x.panes[i].as_ref().map(|p| p.disp.clone())).unwrap_or_default();
    _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
    focus_view(i);
    show_path_end(e, &disp);
}

// Enter in the address bar: a folder (relative to this pane's) is browsed to, a file opened, else a command
// (cmd, wt, powershell ...) runs in this pane's folder.
pub(crate) unsafe fn go_edit(i: usize) {
    let Some(e) = pane_ctl(i, ID_EDIT) else { return };
    let raw = text_of(e);
    let Some((path, cmd)) = addr_parts(raw.trim()) else {
        fail(Fail::AddrQuotes, "");
        return;
    };
    // none inside a zip or in a shell place; metadata (not GetFileAttributes) also takes a bare \\server\share
    let cur = app(|a| a.s.paths[i].clone());
    let cur = (is_fs(&cur) && std::fs::metadata(&cur).is_ok_and(|m| m.is_dir())).then_some(cur);
    let dir = cur.as_deref().map(HSTRING::from);
    let run = |mut sei: SHELLEXECUTEINFOW| {
        sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.lpDirectory = dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr()));
        sei.nShow = SW_SHOWNORMAL.0;
        ShellExecuteExW(&mut sei)
    };
    // Some(true) = browsed, Some(false) = opened or ran
    let mut resolved = false;
    // the command was there but wouldn't run (elevation declined, blocked…): not a spelling slip
    let mut ran_bad = false;
    let done = 'go: {
        if let Some(p) = path {
            // std::path::absolute is GetFullPathNameW: folds `..` and `.`
            let full = cur.as_ref().filter(|_| is_rel(p)).and_then(|c| std::path::absolute(Path::new(c).join(p)).ok());
            let target = full.filter(|f| std::fs::metadata(f).is_ok()).map_or(p.to_string(), |f| f.to_string_lossy().into_owned());
            let (mut pidl, mut sfgao) = (std::ptr::null_mut(), 0u32);
            if SHParseDisplayName(&HSTRING::from(target), None, &mut pidl, SFGAO_FOLDER.0, Some(&mut sfgao)).is_ok() {
                let r = if sfgao & SFGAO_FOLDER.0 != 0 {
                    eb_of(i).is_some_and(|eb| eb.BrowseToIDList(pidl, SBSP_ABSOLUTE).is_ok()).then_some(true)
                } else {
                    run(SHELLEXECUTEINFOW { fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI, lpIDList: pidl as _, ..Default::default() }).is_ok().then_some(false)
                };
                CoTaskMemFree(Some(pidl as _));
                resolved = true;
                break 'go r;
            }
        }
        let Some((prog, args)) = cmd else { break 'go None };
        // wt opens in its own default folder unless told; lpDirectory makes `.` this pane's
        let wt = prog.eq_ignore_ascii_case("wt") || prog.eq_ignore_ascii_case("wt.exe");
        let args = if wt && args.is_empty() && cur.is_some() { "-d ." } else { args };
        let Some(prog) = addr_program(prog, cur.as_deref()) else { break 'go None };
        let (prog, args) = (HSTRING::from(prog), HSTRING::from(args));
        let r = run(SHELLEXECUTEINFOW { fMask: SEE_MASK_FLAG_NO_UI, lpFile: PCWSTR(prog.as_ptr()), lpParameters: PCWSTR(args.as_ptr()), ..Default::default() });
        ran_bad = r.as_ref().is_err_and(|e| !gone(e.code()));
        r.is_ok().then_some(false)
    };
    // as typed, less the quotes around a whole path (the note puts its own around it)
    let name = path.unwrap_or(raw.trim());
    match done {
        Some(true) => focus_view(i),
        Some(false) => addr_reset(i, e),
        None if addr_jumps(done, resolved) => match app(|a| jump_pick(&raw, &a.s.recent, &a.s.favs, &a.s.paths[i]).map(str::to_string)) {
            // as the recent menu and favourites open a folder
            Some(p) => {
                if !go_near(i, &p) {
                    fail(Fail::FolderCantOpen, &p);
                }
                focus_view(i);
            }
            None => fail(if ran_bad { Fail::AddrCantOpen } else { Fail::AddrNotFound }, name),
        },
        // a path that resolved but wouldn't browse or open
        None => fail(Fail::AddrCantOpen, name),
    }
}

// The selected folder (or, with none selected, the folder on display) of `src` opens in `dst`.
pub(crate) unsafe fn open_in(src: usize, dst: usize) {
    let Some(fv) = view_of::<IFolderView2>(src) else { return };
    let is_dir = |it: &IShellItem| it.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
    let sel = fv.GetSelection(false).and_then(|a| a.GetItemAt(0)).ok().filter(is_dir);
    if let Some(item) = sel.or_else(|| fv.GetFolder::<IShellItem>().ok()) {
        // the target takes this pane's columns too (order, widths, sort): set one up, copy it around
        save_view(src);
        app(|a| {
            let cols = a.s.cols[src].clone();
            if let Some(p) = a.panes[dst].as_mut() {
                p.pending = Some(cols);
            }
        });
        browse_item(dst, &item);
        set_active(dst);
        focus_view(dst);
    }
}

// ⇄ menu items (English, Korean), each with its own '&' letter so Ctrl+M then one key picks it; panes 1–9 take
// the digits. Alternatives that never show together (checksum / stop, pair / folder compare) share a letter.
// R, not P, for compare: "Pane 10" … carry no '&', and menus match an unmarked item by its first letter.
pub(crate) const M_SWAP: (&str, &str) = ("&Swap places with  (Ctrl+Alt+Shift+arrows)", "자리 바꾸기(&S)  (Ctrl+Alt+Shift+방향키)");
pub(crate) const M_COPY: (&str, &str) = ("&Copy {n} selected items to", "선택한 {n}개 복사(&C)");
pub(crate) const M_MOVE: (&str, &str) = ("&Move {n} selected items to", "선택한 {n}개 이동(&M)");
// last in the copy / move submenus, under the pane digits and folder names
pub(crate) const M_CHOOSE: (&str, &str) = ("Choose an&other folder…", "다른 폴더 고르기…(&O)");
pub(crate) const M_INUSE: (&str, &str) = ("&Which program is using this?", "어떤 프로그램이 쓰고 있나요?(&W)");
pub(crate) const M_HASH: (&str, &str) = ("Chec&ksum (SHA-256) of the selected files", "선택한 파일의 체크섬(SHA-256)(&K)");
pub(crate) const M_HASH_STOP: (&str, &str) = ("Stop the chec&ksum ({p}%)", "체크섬 계산 중지(&K) ({p}%)");
pub(crate) const M_ORG: (&str, &str) = ("Sort the selected files into &date folders", "선택한 파일을 날짜별 폴더로 정리(&D)");
pub(crate) const M_PHOTO: (&str, &str) = ("Make smaller copies of the selected p&hotos", "선택한 사진의 크기 줄인 사본 만들기(&H)");
pub(crate) const M_GROUP_N: (&str, &str) = ("Move {n} selected items into a &new folder  (Ctrl+F7)", "선택한 {n}개를 새 폴더로 묶기(&N)  (Ctrl+F7)");
pub(crate) const M_GROUP: (&str, &str) = ("Move the selected items into a &new folder  (Ctrl+F7)", "선택한 항목을 새 폴더로 묶기(&N)  (Ctrl+F7)");
pub(crate) const M_STAMP_N: (&str, &str) = ("Make a dated &backup copy of {n} selected items  (Shift+F5)", "선택한 {n}개의 날짜 붙인 사본 만들기(&B)  (Shift+F5)");
pub(crate) const M_STAMP: (&str, &str) = ("Make a dated &backup copy of the selected items  (Shift+F5)", "선택한 항목의 날짜 붙인 사본 만들기(&B)  (Shift+F5)");
pub(crate) const M_EMPTY: (&str, &str) = ("Remove &empty folders in this folder…", "이 폴더 안의 빈 폴더 지우기…(&E)");
pub(crate) const M_LIST_N: (&str, &str) = ("Copy the &list of {n} selected items as a table", "선택한 {n}개 목록을 표로 복사(&L)");
pub(crate) const M_LIST: (&str, &str) = ("Copy this folder's file &list as a table (for Excel)", "이 폴더 파일 목록을 표로 복사(&L) (엑셀용)");
pub(crate) const M_CLIPADD_N: (&str, &str) = ("&Add {n} selected items to the clipboard", "선택한 {n}개를 클립보드에 더하기(&A)");
pub(crate) const M_CLIPADD: (&str, &str) = ("&Add the selected items to the clipboard", "선택한 항목을 클립보드에 더하기(&A)");
// "(with the selection)" only when it goes along: something selected and folders not set to open in Grids
pub(crate) const M_EXPLORER_SEL: (&str, &str) = ("Open in E&xplorer (with the selection)", "탐색기에서 열기(&X) (선택 그대로)");
pub(crate) const M_EXPLORER: (&str, &str) = ("Open in E&xplorer", "탐색기에서 열기(&X)");
pub(crate) const M_USAGE: (&str, &str) = ("Folder &usage: what takes up space here  (Ctrl+Shift+U)", "이 폴더 용량 분석(&U)  (Ctrl+Shift+U)");
pub(crate) const M_WM_PAIR: (&str, &str) = ("Compa&re the 2 selected items with WinMerge", "선택한 2개를 WinMerge로 비교(&R)");
pub(crate) const M_WM_PANE: (&str, &str) = ("Compa&re this folder with WinMerge", "이 폴더를 WinMerge로 비교(&R)");

pub(crate) fn tr2(t: (&'static str, &'static str)) -> &'static str {
    tr(t.0, t.1)
}

// "Pane &3 (Sage)    R&&D": panes 1–9 underline their number (10–16 stay plain); '&' in a folder name stays a letter.
pub(crate) fn pane_label(word: &str, k: usize, tint: &str, disp: &str) -> String {
    let num = if k < 9 { format!("&{}", k + 1) } else { (k + 1).to_string() };
    let tag = if tint.is_empty() { String::new() } else { format!(" ({tint})") };
    format!("{word} {num}{tag}    {}", disp.replace('&', "&&"))
}

// ⇄ menu numbers for the pane rows: + pane index. The shell helpers (1130…, 3100…) and the favourite / recent
// folders (401… / 501…, paste.rs) share the menu; open_in_pick tells them apart.
pub(crate) const ID_OPEN_TO: usize = 1;
pub(crate) const ID_COPY_TO: usize = 101;
pub(crate) const ID_MOVE_TO: usize = 201;
pub(crate) const ID_SWAP_TO: usize = 301;

#[derive(Debug, PartialEq)]
pub(crate) enum OpenPick {
    Nothing,                     // menu closed
    Open(usize),                 // open in pane k
    Swap(usize),                 // swap places with pane k
    Shell,                       // shell_pick's: which program, checksum, date folders, new folder, WinMerge
    Elsewhere,                   // send_pick's: a favourite / recent folder or "Choose another folder…"
    Send { mv: bool, k: usize }, // copy / move the selection to pane k's folder
    Unknown,                     // nothing the menu puts up: a beep, never a copy / move
}

// Same order as before: pane rows, then the shell helpers, then the other folders, then copy / move. A number
// outside every range is Unknown instead of being read as a copy / move to some pane.
pub(crate) fn open_in_pick(p: usize) -> OpenPick {
    let row = |base: usize| (base..base + MAXP).contains(&p).then(|| p - base);
    if p == 0 {
        OpenPick::Nothing
    } else if let Some(k) = row(ID_OPEN_TO) {
        OpenPick::Open(k)
    } else if let Some(k) = row(ID_SWAP_TO) {
        OpenPick::Swap(k)
    } else if is_shell_pick(p) {
        OpenPick::Shell
    } else if send_pick(p).is_some() {
        OpenPick::Elsewhere
    } else if let Some(k) = row(ID_COPY_TO) {
        OpenPick::Send { mv: false, k }
    } else if let Some(k) = row(ID_MOVE_TO) {
        OpenPick::Send { mv: true, k }
    } else {
        OpenPick::Unknown
    }
}

pub(crate) unsafe fn open_in_menu(h: HWND, src: usize) {
    // (pane, shown name, folder it held when the menu opened, colour label)
    let items = app(|a| {
        let live = a.panes.iter().enumerate().take(a.s.panes());
        let live = live.filter(|(k, _)| *k != src);
        live.filter_map(|(k, p)| Some((k, p.as_ref()?.disp.clone(), a.s.paths[k].clone(), a.s.tints[k]))).collect::<Vec<_>>()
    });
    // favourite and recent folders the copy / move submenus offer besides the panes (menu numbers 401… / 501…)
    let (here, targets) = app(|a| {
        let mut shown: Vec<String> = items.iter().map(|(_, _, path, _)| path.clone()).collect();
        shown.push(a.s.paths[src].clone());
        (a.s.paths[src].clone(), send_targets(&shown, &a.s.favs, &a.s.recent))
    });
    // "Pane &3 (Sage)    C:\work"
    let label = |k: usize, t: u8, disp: &str| HSTRING::from(pane_label(tr("Pane", "패널"), k, if t == 0 { "" } else { tint_name(t) }, disp));
    let arr = view_of::<IFolderView2>(src).and_then(|fv| fv.GetSelection(false).ok());
    let count = arr.as_ref().and_then(|a| a.GetCount().ok()).unwrap_or(0);
    let picked = collect_all(count, |k| unsafe { arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)) });
    // checksum (up to HASH_MAX, checked in shell_menu) and date folders: only a selection of files (ZIPs count
    // as files), judged from the attributes the view already holds
    let no_dirs = picked.is_ok()
        && (0..count).all(|k| unsafe {
            let attr = arr.as_ref().and_then(|a| a.GetItemAt(k).ok()).and_then(|it| it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0)).ok());
            attr.is_some_and(|f| !real_folder(f.0))
        });
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Open the selected folder (or this one) in:", "선택한 폴더(없으면 현재 폴더)를 열 패널:")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for (k, disp, _, t) in &items {
        _ = AppendMenuW(menu, MF_STRING, ID_OPEN_TO + k, &label(*k, *t, disp));
    }
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    if let Ok(sub) = CreatePopupMenu() {
        for (k, disp, _, t) in &items {
            _ = AppendMenuW(sub, MF_STRING, ID_SWAP_TO + k, &label(*k, *t, disp));
        }
        let flags = if items.is_empty() { MF_POPUP | MF_GRAYED } else { MF_POPUP };
        _ = AppendMenuW(menu, flags, sub.0 as usize, &HSTRING::from(tr2(M_SWAP)));
    }
    match &picked {
        Ok(srcs) => {
            let n = srcs.len();
            let keys = src_keys(srcs);
            let titles = [
                (false, ID_COPY_TO, tr2(M_COPY).replace("{n}", &n.to_string())),
                (true, ID_MOVE_TO, tr2(M_MOVE).replace("{n}", &n.to_string())),
            ];
            for (mv, base, title) in titles {
                let Ok(sub) = CreatePopupMenu() else { continue };
                for (k, disp, path, t) in &items {
                    let ok = is_fs(path) && send_mode(srcs, path, mv).is_some();
                    let flags = if ok { MF_STRING } else { MF_STRING | MF_GRAYED };
                    _ = AppendMenuW(sub, flags, base + k, &label(*k, *t, disp));
                }
                // then folders no pane shows: favourites, recent folders, any other
                if !items.is_empty() {
                    _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
                }
                let first = if mv { ID_SEND_MOVE } else { ID_SEND_COPY };
                for (n, (fav, path)) in targets.iter().enumerate() {
                    if n == 0 || (!fav && targets[n - 1].0) {
                        let head = if *fav { tr("Favourites", "즐겨찾기") } else { tr("Recent folders", "최근 폴더") };
                        _ = AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, &HSTRING::from(head));
                    }
                    let flags = if send_check(srcs, &keys, path, mv).is_some() { MF_STRING } else { MF_STRING | MF_GRAYED };
                    _ = AppendMenuW(sub, flags, first + n, &HSTRING::from(folder_label(path)));
                }
                if !targets.is_empty() {
                    _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
                }
                let choose = if mv { ID_CHOOSE_MOVE } else { ID_CHOOSE_COPY };
                _ = AppendMenuW(sub, MF_STRING, choose, &HSTRING::from(tr2(M_CHOOSE)));
                _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, &HSTRING::from(title));
            }
        }
        Err(why) => {
            let text = match why {
                Pick::TooMany => tr("Copy / move: more than 10,000 selected items — use Ctrl+C / Ctrl+V", "복사·이동: 선택이 10,000개를 넘습니다 — Ctrl+C / Ctrl+V를 쓰세요"),
                Pick::Unreadable => tr("Copy / move: some selected items aren't regular files", "복사·이동: 일반 파일이 아닌 항목이 섞여 있습니다"),
                Pick::Empty => tr("Copy / move: select files first", "복사·이동: 먼저 파일을 고르세요"),
            };
            _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(text));
        }
    }
    let others: Vec<(usize, HSTRING, String)> = items.iter().map(|(k, disp, path, t)| (*k, label(*k, *t, disp), path.clone())).collect();
    let wm = shell_menu(menu, src, &picked, no_dirs, &others);
    // a labelled pane's rows (open, swap, copy, move, compare) start with its colour; found in the submenus by
    // their numbers, which the whole menu never repeats
    let mut chips = MenuSwatches::new(app(|a| a.dpi)); // dropped after DestroyMenu
    for (k, _, _, t) in &items {
        if let Some(c) = tint_color(*t) {
            for base in [ID_OPEN_TO, ID_SWAP_TO, ID_COPY_TO, ID_MOVE_TO, ID_WM_PANE] {
                chips.put(menu, base + k, Some(c));
            }
        }
    }
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(src, ID_OPEN_IN) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    // picked with Enter: its auto-repeats would open a file in the list that gets the focus next
    if GetKeyState(VK_RETURN.0 as i32) < 0 {
        app(|a| a.held = Some(VK_RETURN.0));
    }
    _ = DestroyMenu(menu);
    let p = pick as usize;
    match (open_in_pick(p), picked) {
        (OpenPick::Nothing, _) => {}
        (OpenPick::Open(k), _) => open_in(src, k),
        (OpenPick::Swap(k), _) => swap_panes(src, k),
        (OpenPick::Shell, picked) => shell_pick(h, src, p, picked, wm, &others),
        (OpenPick::Elsewhere, picked) => {
            let sent = match (send_pick(p), picked) {
                (Some(SendPick::To(mv, n)), Ok(srcs)) => targets.get(n).is_some_and(|(_, dst)| send_elsewhere(srcs, dst, mv)),
                (Some(SendPick::Choose(mv)), Ok(srcs)) => {
                    let owner = app(|a| a.main);
                    let sent = match choose_folder(owner, &here, mv) {
                        Ok(None) => true, // cancelled
                        // the picker opens at this folder: confirming it at once (or an Enter repeat) would
                        // copy a "- Copy" beside the originals
                        Ok(Some(dst)) => !same_dir(&dst, &here) && send_elsewhere(srcs, &dst, mv),
                        Err(_) => false,
                    };
                    focus_view(src);
                    sent
                }
                _ => false,
            };
            if !sent {
                fail(Fail::SendCant, "");
            }
        }
        (OpenPick::Send { mv, k }, Ok(srcs)) => {
            // only to the folder the menu showed, and only the whole selection it counted
            let shown = items.iter().find(|(i, ..)| *i == k).map(|(_, _, path, _)| path.clone());
            let Some(dst) = shown.filter(|s| *s == app(|a| a.s.paths[k].clone())) else {
                fail(Fail::SendChanged, "");
                return;
            };
            let sent = send_mode(&srcs, &dst, mv).is_some_and(|rename| start_copy(srcs, dst, rename, mv));
            if !sent {
                fail(Fail::SendCant, "");
            }
        }
        (OpenPick::Send { .. }, Err(_)) => {}
        (OpenPick::Unknown, _) => _ = MessageBeep(MB_ICONWARNING),
    }
}

// Right-click on a pane header: colour label from the selection colour presets, saved with the session.
pub(crate) unsafe fn tint_menu(h: HWND, i: usize, at: POINT) {
    let (cur, dpi) = app(|a| (a.s.tints[i], a.dpi));
    let mut chips = MenuSwatches::new(dpi); // dropped after DestroyMenu
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Pane colour", "패널 색")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for k in 0..=6u8 {
        let flags = if k == cur { MF_STRING | MF_CHECKED } else { MF_STRING };
        let name = if k == 0 { tr("None", "없음") } else { tint_name(k) };
        _ = AppendMenuW(menu, flags, k as usize + 1, &HSTRING::from(name));
        chips.put(menu, k as usize + 1, tint_color(k)); // None: an empty chip
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, at.x, at.y, None, h, None).0;
    _ = DestroyMenu(menu);
    if pick > 0 {
        app(|a| a.s.tints[i] = pick as u8 - 1);
        repaint_header(h);
        check_dirty();
    }
}

// Alt+↓ or right-click on ← / →: folders recently visited in any pane, opened in this one.
pub(crate) unsafe fn recent_menu(i: usize) {
    let Some(h) = app(|a| a.panes[i].as_ref().map(|p| p.hwnd)) else { return };
    let (items, ix, dpi) = app(|a| {
        let items = recent_items(&a.s.recent, &a.s.paths[i]);
        let ix: Vec<i32> = items.iter().map(|p| menu_icon_of(a, p, -1)).collect();
        (items, ix, a.dpi)
    });
    let mut icons = MenuIcons::new(dpi); // dropped after DestroyMenu
    let Ok(menu) = CreatePopupMenu() else { return };
    if items.is_empty() {
        _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("No recent folders yet", "최근 폴더가 아직 없습니다")));
    }
    for (k, p) in items.iter().enumerate() {
        // "name<tab>parent"; drive roots and share roots show whole
        _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(folder_label(p)));
        icons.put(menu, k + 1, ix[k]);
    }
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(i, ID_BACK) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    _ = DestroyMenu(menu);
    if pick > 0 && !go_near(i, &items[pick as usize - 1]) {
        fail(Fail::FolderCantOpen, &items[pick as usize - 1]); // gone or unreachable
    }
    focus_view(i);
}

// Ctrl+G: Windows' own folders, as Finder's Go menu or Directory Opus' Go. None in a row is a separator.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum GoTarget {
    Known(&'static GUID),
    Place(&'static str), // a parsing name, as on_nav records it (a.s.paths)
    Temp,
    RecentFiles, // not a folder: a second menu of the files other apps opened lately
}

pub(crate) fn go_targets(ko: bool) -> Vec<Option<(&'static str, GoTarget)>> {
    use GoTarget::*;
    let rows: [Option<(&str, &str, GoTarget)>; 15] = [
        Some(("&Home", "홈(&H)", Known(&FOLDERID_Profile))),
        Some(("&Desktop", "바탕 화면(&D)", Known(&FOLDERID_Desktop))),
        Some(("D&ocuments", "문서(&O)", Known(&FOLDERID_Documents))),
        Some(("Down&loads", "다운로드(&L)", Known(&FOLDERID_Downloads))),
        Some(("&Pictures", "사진(&P)", Known(&FOLDERID_Pictures))),
        Some(("&Music", "음악(&M)", Known(&FOLDERID_Music))),
        Some(("&Videos", "동영상(&V)", Known(&FOLDERID_Videos))),
        None,
        Some(("This P&C", "내 PC(&C)", Place(THIS_PC))),
        Some(("&Recycle Bin", "휴지통(&R)", Place(RECYCLE_BIN))),
        None,
        Some(("&Temporary folder", "임시 폴더(&T)", Temp)),
        Some(("&AppData", "AppData(&A)", Known(&FOLDERID_RoamingAppData))),
        None,
        Some(("Recent &files", "최근 파일(&F)", RecentFiles)),
    ];
    rows.into_iter().map(|r| r.map(|(en, k, t)| (if ko { k } else { en }, t))).collect()
}

// A label's underlined letter, upper case ("Down&loads" → L).
pub(crate) fn go_letter(label: &str) -> Option<char> {
    label.split_once('&').and_then(|(_, r)| r.chars().next()).map(|c| c.to_ascii_uppercase())
}

// The menu position a typed character picks: the letter itself or, with Ctrl held, its control code (Ctrl+L = 0x0C).
// Without Ctrl a control code is a plain key (Backspace = 0x08, Tab = 0x09) and picks nothing.
pub(crate) fn go_pick_char(rows: &[Option<(&str, GoTarget)>], ch: u16, ctrl: bool) -> Option<usize> {
    let c = match ch {
        0x01..=0x1A if ctrl => char::from(b'A' + (ch - 1) as u8),
        0x00..=0x1F => return None,
        _ => char::from_u32(ch as u32)?.to_ascii_uppercase(),
    };
    rows.iter().position(|r| r.is_some_and(|(l, _)| go_letter(l) == Some(c)))
}

// A folder path without its trailing backslash, a drive root kept whole (C:\).
fn go_trim(p: &str) -> String {
    let t = p.trim_end_matches('\\');
    if t.ends_with(':') {
        format!("{t}\\")
    } else {
        t.to_string()
    }
}

unsafe fn go_path(t: GoTarget) -> Option<String> {
    match t {
        GoTarget::Place(p) => Some(p.to_string()),
        GoTarget::Temp => Some(go_trim(&std::env::temp_dir().to_string_lossy())).filter(|p| !p.is_empty()),
        GoTarget::RecentFiles => None,
        // DONT_VERIFY: a folder moved to a share or OneDrive is not probed (no wait); a missing one beeps when picked
        GoTarget::Known(id) => {
            let p = SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None).ok()?;
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as _));
            s.map(|s| go_trim(&s)).filter(|s| !s.is_empty())
        }
    }
}

// A known folder's own icon from its registered definition ("%SystemRoot%\system32\imageres.dll,-184" for Downloads),
// as a system image list index; -1 when it has none (Home, AppData) or the shell fails. Only the definition is read,
// never the folder, so one moved to a share is no wait. Asked once per folder; outside app(..).
unsafe fn known_icon(id: &GUID) -> i32 {
    thread_local! {
        static KNOWN: RefCell<Vec<(GUID, i32)>> = const { RefCell::new(Vec::new()) };
    }
    if let Some(n) = KNOWN.with(|k| k.borrow().iter().find(|e| e.0 == *id).map(|e| e.1)) {
        return n;
    }
    let n = known_icon_ask(id).unwrap_or(-1);
    KNOWN.with(|k| k.borrow_mut().push((*id, n)));
    n
}

unsafe fn known_icon_ask(id: &GUID) -> Option<i32> {
    let kfm: IKnownFolderManager = CoCreateInstance(&KnownFolderManager, None, CLSCTX_INPROC_SERVER).ok()?;
    let kf = kfm.GetFolder(id).ok()?;
    let mut d = KNOWNFOLDER_DEFINITION::default();
    let got = kf.GetFolderDefinition(&mut d).is_ok();
    let icon = (got && !d.pszIcon.is_null()).then(|| d.pszIcon.to_string().ok()).flatten();
    // FreeKnownFolderDefinitionFields is a header inline: each string the definition holds, freed here
    for s in [d.pszName, d.pszDescription, d.pszRelativePath, d.pszParsingName, d.pszTooltip, d.pszLocalizedName, d.pszIcon, d.pszSecurity] {
        if !s.is_null() {
            CoTaskMemFree(Some(s.0 as _));
        }
    }
    let (file, at) = icon_location(&icon?, |v| std::env::var(v).ok())?;
    let n = Shell_GetCachedImageIndexW(&HSTRING::from(file), at, 0);
    (n >= 0).then_some(n)
}

// A registered icon location ("%SystemRoot%\system32\imageres.dll,-184", maybe quoted, maybe without the index)
// → (its file, %NAME% expanded by `var` and left as it is when unknown, as ExpandEnvironmentStrings; the index).
pub(crate) fn icon_location(s: &str, var: impl Fn(&str) -> Option<String>) -> Option<(String, i32)> {
    let s = s.trim();
    let (file, at) = match s.rsplit_once(',').map(|(f, n)| (f, n.trim().parse::<i32>())) {
        Some((f, Ok(n))) => (f, n),
        _ => (s, 0),
    };
    let file = file.trim().trim_matches('"').trim();
    if file.is_empty() {
        return None;
    }
    Some((expand_vars(file, var), at))
}

// %NAME% expanded by `var`, left as it is when unknown (as ExpandEnvironmentStrings does).
pub(crate) fn expand_vars(file: &str, var: impl Fn(&str) -> Option<String>) -> String {
    let (mut out, mut rest) = (String::new(), file);
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%').filter(|&b| b > 0).and_then(|b| Some((var(&after[..b])?, b))) {
            Some((v, b)) => {
                out.push_str(&v);
                rest = &after[b + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// Set while the Ctrl+G menu is open, for its WM_MENUCHAR (Ctrl still held while the letter is typed).
static GO_OPEN: AtomicBool = AtomicBool::new(false);
// Set while its recent files menu (Ctrl+G, F) is open: Ctrl+F still held repeats there.
static RECENT_OPEN: AtomicBool = AtomicBool::new(false);

// The pane window's WM_MENUCHAR while the Ctrl+G menu is open: Ctrl+letter picks as the letter alone would.
// None: not this menu's, DefWindowProcW as before.
pub(crate) fn go_menu_char(wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let recent = RECENT_OPEN.load(Ordering::Relaxed);
    if !recent && !GO_OPEN.load(Ordering::Relaxed) {
        return None;
    }
    const MNC_EXECUTE: isize = 2;
    const MNC_SELECT: isize = 3;
    let ch = (wp.0 & 0xffff) as u16;
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
    if !recent {
        if let Some(k) = go_pick_char(&go_targets(KO.load(Ordering::Relaxed)), ch, ctrl) {
            return Some(LRESULT(MNC_EXECUTE << 16 | k as isize));
        }
    }
    if ch != if recent { 0x06 } else { 0x07 } {
        return None; // anything else beeps as in any menu
    }
    // Ctrl+G held (Ctrl+F in the recent files menu): its repeats keep the highlight where it is, without a beep
    let menu = HMENU(lp.0 as _);
    let n = unsafe { GetMenuItemCount(Some(menu)) }.max(0) as u32;
    let lit = (0..n).find(|&k| unsafe { GetMenuState(menu, k, MF_BYPOSITION) } & MF_HILITE.0 != 0).unwrap_or(0);
    Some(LRESULT(MNC_SELECT << 16 | lit as isize))
}

// Ctrl+G: Windows' own folders (home, desktop, documents, downloads …), opened in pane i. Each underlined
// letter picks at once, with or without Ctrl still held; the pane's own folder is checked.
pub(crate) unsafe fn go_menu(i: usize) {
    let Some(h) = app(|a| a.panes[i].as_ref().map(|p| p.hwnd)) else { return };
    let rows = go_targets(KO.load(Ordering::Relaxed));
    let (here, dpi) = app(|a| (a.s.paths[i].clone(), a.dpi));
    let paths: Vec<Option<String>> = rows.iter().map(|r| r.and_then(|(_, t)| go_path(t))).collect();
    // each place's icon: known folders' own from their definitions (asked here, outside app), the rest from the cache
    let known: Vec<i32> = rows
        .iter()
        .map(|r| match r {
            Some((_, GoTarget::Known(id))) => known_icon(id),
            Some((_, GoTarget::RecentFiles)) => known_icon(&FOLDERID_Recent), // Explorer's Recent Items clock
            _ => -1,
        })
        .collect();
    let ix: Vec<i32> = app(|a| paths.iter().zip(&known).map(|(p, &kn)| p.as_ref().map_or(-1, |p| menu_icon_of(a, p, kn))).collect());
    let mut icons = MenuIcons::new(dpi); // dropped after DestroyMenu
    let Ok(menu) = CreatePopupMenu() else { return };
    for (k, r) in rows.iter().enumerate() {
        let Some((label, _)) = r else {
            _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            continue;
        };
        let here_too = paths[k].as_ref().is_some_and(|p| same_dir(p, &here));
        let recent = matches!(r, Some((_, GoTarget::RecentFiles)));
        let flags = match &paths[k] {
            None if recent => MF_STRING,
            None => MF_STRING | MF_GRAYED,
            Some(_) if here_too => MF_STRING | MF_CHECKED,
            _ => MF_STRING,
        };
        _ = AppendMenuW(menu, flags, k + 1, &HSTRING::from(*label));
        if paths[k].is_some() {
            icons.put(menu, k + 1, ix[k]);
        } else if recent {
            icons.put(menu, k + 1, known[k]);
        }
        if here_too {
            // with an icon the theme may show the check only as a frame behind it: bold too, as the '›' menu
            _ = SetMenuDefaultItem(menu, k as u32 + 1, 0);
        }
    }
    let mut rc = RECT::default();
    if let Some(e) = pane_ctl(i, ID_EDIT) {
        _ = GetWindowRect(e, &mut rc);
    }
    GO_OPEN.store(true, Ordering::Relaxed);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    GO_OPEN.store(false, Ordering::Relaxed);
    _ = DestroyMenu(menu);
    if pick > 0 && matches!(rows.get(pick as usize - 1), Some(Some((_, GoTarget::RecentFiles)))) {
        recent_files_menu(i, h, rc.left, rc.bottom); // in the same place, as a submenu would open
        return;
    }
    if pick > 0 {
        let to = paths.get(pick as usize - 1).cloned().flatten();
        if !to.is_some_and(|p| go_near(i, &p)) {
            _ = MessageBeep(MB_ICONWARNING); // missing, or unreachable
        }
    }
    focus_view(i);
}

// ---------- Ctrl+G, F: recent files (Windows' Recent Items, as Finder's Recents or Explorer's Home) ----------
const RECENT_LOOK: usize = 60; // shortcuts read, newest first
const RECENT_KEEP: usize = 15; // rows shown
const RECENT_ID: usize = 100; // first row's command id

// The newest `n` of (item, modified) pairs, newest first.
pub(crate) fn newest_first<T>(mut v: Vec<(T, u64)>, n: usize) -> Vec<T> {
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.into_iter().take(n).map(|e| e.0).collect()
}

// A file path → (its folder, its name); None without a folder part or a name. A drive root keeps its backslash.
pub(crate) fn split_file(p: &str) -> Option<(String, String)> {
    let (dir, name) = p.rsplit_once('\\')?;
    if dir.is_empty() || name.is_empty() {
        return None;
    }
    Some((if dir.ends_with(':') { format!("{dir}\\") } else { dir.to_string() }, name.to_string()))
}

// Shortcut targets (path, saved as a folder), newest first → the files to list: folders, paths without a folder
// and repeats (in any case) left out, at most `n`.
pub(crate) fn recent_keep(targets: Vec<(String, bool)>, n: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (p, dir) in targets {
        if out.len() == n {
            break;
        }
        if dir || split_file(&p).is_none() || out.iter().any(|o| o.to_lowercase() == p.to_lowercase()) {
            continue;
        }
        out.push(p);
    }
    out
}

// `s` cut in the middle to at most `max` characters ("a very lo…name.docx"), so a menu stays on the screen.
pub(crate) fn mid_cut(s: &str, max: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= max || max < 3 {
        return s.to_string();
    }
    let tail = (max - 1) / 2;
    let head = max - 1 - tail;
    format!("{}…{}", c[..head].iter().collect::<String>(), c[c.len() - tail..].iter().collect::<String>())
}

// A recent file's menu row, "name<tab>folder": the folder by its own name (a drive root whole), & doubled so it
// shows as itself, not an underline.
pub(crate) fn recent_label(path: &str) -> Option<String> {
    let (dir, name) = split_file(path)?;
    let folder = if dir.ends_with('\\') { dir.as_str() } else { dir.rsplit('\\').next().unwrap_or(&dir) };
    Some(format!("{}\t{}", mid_cut(&name, 60), mid_cut(folder, 30)).replace('&', "&&"))
}

// Windows keeps recent files unless "Show recently opened items" is off (Start_TrackDocs = 0).
unsafe fn recent_tracked() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let (mut v, mut n) = (1u32, 4u32);
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced");
    let r = RegGetValueW(HKEY_CURRENT_USER, key, w!("Start_TrackDocs"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as _), Some(&mut n));
    !r.is_ok() || v != 0
}

// The files in Recent Items, newest first. Only the shortcuts are read (their saved target, never Resolve), so a
// file on a share or a drive that is gone is no wait.
unsafe fn recent_files() -> Vec<String> {
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, WIN32_FIND_DATAW};
    let Some(dir) = go_path(GoTarget::Known(&FOLDERID_Recent)) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let links: Vec<(PathBuf, u64)> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")))
        .filter_map(|e| {
            let t = e.metadata().ok()?.modified().ok()?;
            Some((e.path(), t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)))
        })
        .collect();
    let targets = newest_first(links, RECENT_LOOK)
        .into_iter()
        .filter_map(|p| {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
            link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(p.as_os_str()), STGM_READ).ok()?;
            let (mut buf, mut fd) = ([0u16; 1024], WIN32_FIND_DATAW::default());
            link.GetPath(&mut buf, &mut fd, 0).ok()?;
            let s = String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())]);
            Some((s, fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0))
        })
        .collect();
    recent_keep(targets, RECENT_KEEP)
}

// Ctrl+G, F: files other apps opened lately, with their type's icon; the picked one is shown selected in its
// folder in pane i (a folder that is gone says so in the status line).
unsafe fn recent_files_menu(i: usize, h: HWND, x: i32, y: i32) {
    let on = recent_tracked();
    let files = if on { recent_files() } else { Vec::new() };
    let mut icons = MenuIcons::new(app(|a| a.dpi)); // dropped after DestroyMenu
    let Ok(menu) = CreatePopupMenu() else { return };
    if files.is_empty() {
        let note = if on {
            tr("(No recent files)", "(최근 파일 없음)")
        } else {
            tr("Windows isn't keeping recent files", "Windows에서 최근 파일 기록이 꺼져 있음")
        };
        _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(note));
    }
    for (k, p) in files.iter().enumerate() {
        let Some(label) = recent_label(p) else { continue };
        _ = AppendMenuW(menu, MF_STRING, RECENT_ID + k, &HSTRING::from(label));
        let name = split_file(p).map_or(String::new(), |(_, n)| n);
        icons.put(menu, RECENT_ID + k, type_icon(&name, false)); // by its type only: no disk access
    }
    RECENT_OPEN.store(true, Ordering::Relaxed);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, x, y, None, h, None).0 as usize;
    RECENT_OPEN.store(false, Ordering::Relaxed);
    _ = DestroyMenu(menu);
    match pick.checked_sub(RECENT_ID).and_then(|k| files.get(k)).and_then(|p| split_file(p)) {
        Some((dir, name)) => show_in_pane(i, dir, Some(name)),
        None => focus_view(i),
    }
}

pub(crate) unsafe fn apply_filter(i: usize) {
    let Some(f) = pane_ctl(i, ID_FILTER) else { return };
    let spec = filter_spec(&text_of(f));
    let changed = FILTERS.lock().map(|mut fl| std::mem::replace(&mut fl[i], spec.clone()) != spec).unwrap_or(false);
    if let (true, Some(fv)) = (changed, view_of::<IFolderView2>(i)) {
        set_empty_text(&fv, &spec); // before the refresh, so an emptied list shows the new words
    }
    if let (true, Some(sv)) = (changed, view_of::<IShellView>(i)) {
        _ = sv.Refresh(); // re-enumerates, asking IncludeObject again
    }
    if let (true, Some(pane)) = (changed, app(|a| a.panes[i].as_ref().map(|p| p.hwnd))) {
        _ = InvalidateRect(Some(pane), None, false); // filter badge on/off
    }
}

// The words in the middle of an empty list: the filter's, or (no filter) Windows' own again via NULL.
pub(crate) unsafe fn set_empty_text(fv: &IFolderView2, spec: &str) {
    if spec.is_empty() {
        _ = fv.SetText(FVST_EMPTYTEXT, PCWSTR::null());
    } else {
        _ = fv.SetText(FVST_EMPTYTEXT, &HSTRING::from(empty_text(spec, KO.load(Ordering::Relaxed))));
    }
}

// Filter icon or Alt+↓ in the filter: show only one kind of file (folders stay), like Explorer's search "Kind".
pub(crate) unsafe fn kind_menu(i: usize) {
    let (Some(h), Some(f)) = (app(|a| a.panes[i].as_ref().map(|p| p.hwnd)), pane_ctl(i, ID_FILTER)) else { return };
    let cur = filter_of(i);
    // only an actual single selection: the focused item alone doesn't count
    let arr = view_of::<IFolderView2>(i).and_then(|fv| fv.GetSelection(false).ok()).filter(|a| a.GetCount().is_ok_and(|n| n == 1));
    let same = arr.and_then(|a| {
        let it = a.GetItemAt(0).ok()?;
        let dir = it.GetAttributes(SFGAO_FOLDER).ok()?.0 & SFGAO_FOLDER.0 != 0;
        same_type(&pwstr(it.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).ok()?), dir)
    });
    let Ok(menu) = CreatePopupMenu() else { return };
    for (k, (en, ko, spec)) in FILE_KINDS.iter().enumerate() {
        let flags = if cur == *spec { MF_STRING | MF_CHECKED } else { MF_STRING };
        let shown = spec.split(';').take(2).collect::<Vec<_>>().join(", ");
        _ = AppendMenuW(menu, flags, k + 1, &HSTRING::from(format!("{}\t{shown}, …", tr(en, ko))));
    }
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let same_label = tr("Same type as the selected file", "선택한 파일과 같은 형식");
    match &same {
        Some(s) => _ = AppendMenuW(menu, MF_STRING, 100, &HSTRING::from(format!("{same_label} ({})", s.replace('&', "&&")))),
        None => _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 100, &HSTRING::from(same_label)),
    }
    let clear = if cur.is_empty() && text_of(f).is_empty() { MF_STRING | MF_GRAYED } else { MF_STRING };
    _ = AppendMenuW(menu, clear, 101, &HSTRING::from(tr("Clear filter", "필터 지우기")));
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let pg = pane_geo(rc.right, rc.bottom, app(|a| a.dpi));
    let mut at = POINT { x: pg.filt.left, y: pg.filt.bottom };
    _ = ClientToScreen(h, &mut at);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, at.x, at.y, None, h, None).0 as usize;
    _ = DestroyMenu(menu);
    let spec = match pick {
        1..=6 => Some(FILE_KINDS[pick - 1].2.to_string()),
        100 => same,
        101 => Some(String::new()),
        _ => None,
    };
    if let Some(s) = spec {
        _ = SetWindowTextW(f, &HSTRING::from(s));
        apply_filter(i);
    }
    focus_view(i);
}

// Pane-sticky view mode and columns (order, widths, sort): remember them before leaving a folder,
// re-apply after arriving. Columns seen in shell places (This PC ...) are theirs, not the pane's.
pub(crate) unsafe fn save_view(i: usize) {
    if app(|a| a.panes[i].as_ref().is_none_or(|p| p.pending.is_some() || p.settle.is_some())) {
        return; // columns are about to be put on, or still settling: don't overwrite them
    }
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    free_min_widths(view_list(&fv)); // a list made after the folder change gets it here
    let (mut m, mut z) = (FOLDERVIEWMODE(0), 0);
    if fv.GetViewModeAndIconSize(&mut m, &mut z).is_ok() {
        app(|a| a.s.views[i] = (m.0, z));
    }
    if let Some(c) = read_cols(&fv, view_list(&fv)).filter(|_| app(|a| is_fs(&a.s.paths[i]))) {
        app(|a| a.s.cols[i] = c);
    }
}

// Column names in on-screen order, from the pane's list header: dragging a column to a new place
// changes only the header's order, which the view's own column list doesn't follow.
pub(crate) unsafe fn header_names(lv: HWND) -> Vec<String> {
    let n = list_titles(lv).len();
    let mut order = vec![0i32; n];
    SendMessageW(lv, LVM_GETCOLUMNORDERARRAY, Some(WPARAM(n)), Some(LPARAM(order.as_mut_ptr() as isize)));
    let titles = list_titles(lv);
    order.iter().filter_map(|&k| titles.get(k as usize).cloned()).collect()
}

// Column titles of a list, by column index.
pub(crate) unsafe fn list_titles(lv: HWND) -> Vec<String> {
    let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
    let n = SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize;
    (0..n)
        .map(|k| {
            let mut text = [0u16; 128];
            let mut item = HDITEMW { mask: HDI_TEXT, pszText: PWSTR(text.as_mut_ptr()), cchTextMax: text.len() as i32, ..Default::default() };
            SendMessageW(header, HDM_GETITEMW, Some(WPARAM(k)), Some(LPARAM(&mut item as *mut _ as isize)));
            String::from_utf16_lossy(&text[..text.iter().position(|&c| c == 0).unwrap_or(0)])
        })
        .collect()
}

// The list inside a view. During a folder change the old view's list is still around, so it is
// found from the view itself, not by looking through the pane.
pub(crate) unsafe fn view_list(fv: &IFolderView2) -> HWND {
    let view = fv.cast::<IShellView>().and_then(|v| v.GetWindow()).unwrap_or_default();
    FindWindowExW(Some(view), None, w!("SysListView32"), PCWSTR::null()).unwrap_or_default()
}

// Whether pane i's list is a DirectUIHWND: no list message reaches it, so it can't be recoloured.
pub(crate) unsafe fn dui_list(i: usize) -> bool {
    let view = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok());
    view.is_some_and(|v| FindWindowExW(Some(v), None, w!("DirectUIHWND"), PCWSTR::null()).is_ok())
}

// Whether h is the file list of the view window. The list is a SysListView32 on some systems and a
// DirectUIHWND on others, where view_list finds nothing; both are the list when they sit inside the view.
pub(crate) unsafe fn in_view_list(view: HWND, h: HWND) -> bool {
    let mut c = [0u16; 32];
    let n = GetClassNameW(h, &mut c) as usize;
    !view.0.is_null()
        && matches!(String::from_utf16_lossy(&c[..n]).as_str(), "DirectUIHWND" | "SysListView32")
        && (view == h || IsChild(view, h).as_bool())
}

// Explorer puts a floor under some columns (Size stops at 90 px) so numbers never get cut, and puts it
// back whenever a column changes (through an interface, not messages we could catch). In a pane the
// user decides: the floor is lowered to a sliver right before each resize, a drag on the header or a
// width set on the list.
pub(crate) unsafe fn free_min_widths(lv: HWND) {
    if lv.0.is_null() {
        return;
    }
    let mut was = 0usize;
    let first = !GetWindowSubclass(lv, Some(list_floor_proc), 3, Some(&mut was as *mut _)).as_bool();
    _ = SetWindowSubclass(lv, Some(list_floor_proc), 3, 0);
    if first {
        // the view puts its floor back on the first width change a list sees, and only then: spend that
        // change here (a pixel wider and back) so the user's first drag already goes below it
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        for c in 0..SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize {
            let w = SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0;
            SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w + 1)));
            SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w)));
        }
    }
    // and no snapping to the floor or ideal width while dragging, no automatic sizing: the user's width
    let snap = LVS_EX_COLUMNSNAPPOINTS | LVS_EX_AUTOSIZECOLUMNS;
    if SendMessageW(lv, LVM_GETEXTENDEDLISTVIEWSTYLE, None, None).0 as usize & snap != 0 {
        SendMessageW(lv, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(snap)), Some(LPARAM(0)));
    }
}

pub(crate) unsafe fn lower_floors(lv: HWND) {
    let floor = 20 * GetDpiForWindow(lv) as i32 / 96;
    let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
    for c in 0..SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize {
        let mut col = LVCOLUMNW { mask: LVCF_MINWIDTH, ..Default::default() };
        if SendMessageW(lv, LVM_GETCOLUMNW, Some(WPARAM(c)), Some(LPARAM(&mut col as *mut _ as isize))).0 != 0 && col.cxMin > floor {
            let lower = LVCOLUMNW { mask: LVCF_MINWIDTH, cxMin: floor, ..Default::default() };
            SendMessageW(lv, LVM_SETCOLUMNW, Some(WPARAM(c)), Some(LPARAM(&lower as *const _ as isize)));
        }
    }
}

pub(crate) unsafe extern "system" fn list_floor_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let resize = match msg {
        LVM_SETCOLUMNWIDTH => true,
        WM_NOTIFY => matches!((*(lp.0 as *const NMHDR)).code, HDN_BEGINTRACKW | HDN_TRACKW | HDN_ITEMCHANGINGW),
        _ => false,
    };
    if resize {
        lower_floors(h);
    }
    // the view stops a column coming down from above its floor at the floor (through its own channel,
    // not a message): the narrowing asked for goes on again right after, from the floor, where it isn't stopped
    if msg == WM_NOTIFY && (*(lp.0 as *const NMHDR)).code == HDN_ITEMCHANGINGW {
        let nm = &*(lp.0 as *const NMHEADERW);
        if !nm.pitem.is_null() && (*nm.pitem).mask.0 & HDI_WIDTH.0 != 0 {
            let floor = 20 * GetDpiForWindow(h) as i32 / 96; // never all the way to nothing
            let (col, want) = (nm.iItem, (*nm.pitem).cxy.max(floor));
            let now = SendMessageW(h, LVM_GETCOLUMNWIDTH, Some(WPARAM(col as usize)), None).0 as i32;
            let ask = if want < now { (col as i64) << 32 | want as i64 } else { -1 };
            NARROWING.store(ask, Ordering::Relaxed); // a newer change (or a widening) cancels an older one
            if ask >= 0 {
                _ = PostMessageW(Some(h), WM_NARROW_AGAIN, WPARAM(col as usize), LPARAM(want as isize));
            }
        }
    }
    if msg == WM_NARROW_AGAIN {
        let (col, want) = (wp.0, lp.0 as i32);
        if NARROWING.load(Ordering::Relaxed) == ((col as i64) << 32 | want as i64)
            && SendMessageW(h, LVM_GETCOLUMNWIDTH, Some(WPARAM(col)), None).0 as i32 != want // stopped at the floor, or let through to nothing
        {
            NARROWING.store(-1, Ordering::Relaxed);
            SendMessageW(h, LVM_SETCOLUMNWIDTH, Some(WPARAM(col)), Some(LPARAM(want as isize)));
        }
        return LRESULT(0);
    }
    DefSubclassProc(h, msg, wp, lp)
}

pub(crate) const WM_NARROW_AGAIN: u32 = WM_APP + 41;
pub(crate) const WM_DROP_OPEN: u32 = WM_APP + 42;
pub(crate) const WM_PEEK_DONE: u32 = WM_APP + 43; // wParam = gen; the picture itself stays in PEEK
pub(crate) const WM_PANE_ICON: u32 = WM_APP + 44; // wParam = ticket, lParam = system image list index (-1: none)
pub(crate) const WM_DRIVES: u32 = WM_APP + 45; // drives may have come or gone: refresh_drives
pub(crate) const WM_KIDS: u32 = WM_APP + 46; // wParam = request id; the list itself stays in KIDS
pub(crate) const WM_THEME_SYNC: u32 = WM_APP + 47; // Windows light/dark may have changed: theme_sync
pub(crate) const WM_PEEK_HOSTED: u32 = WM_APP + 49; // wParam = preview generation, lParam = 1 shown / 0 failed
pub(crate) const WM_PEEK_LOST: u32 = WM_APP + 48; // the preview went inactive: close it unless it is active again
pub(crate) const WM_GROUPED: u32 = WM_APP + 52; // Ctrl+F7 moved everything in; the details stay in GROUPED
pub(crate) const WM_STAMPED: u32 = WM_APP + 61; // Shift+F5's dated copies are done; the details stay in STAMPED
pub(crate) const WM_EDITED: u32 = WM_APP + 64; // Ctrl+E's files were handed to their editors; lParam = Box<Edited>
pub(crate) const WM_FONT_SYNC: u32 = WM_APP + 58; // Windows' text size may have changed: refont if the UI font did
pub(crate) static NARROWING: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1); // column << 32 | width


// The view's visible columns with widths, put in on-screen order by name, then its sort columns.
// Widths come from the list: one narrowed below the view's floor is only there (the view still says 90).
pub(crate) unsafe fn read_cols(fv: &IFolderView2, lv: HWND) -> Option<String> {
    let cm: IColumnManager = fv.cast().ok()?;
    let (titles, shown) = (list_titles(lv), header_names(lv));
    let mut keys = vec![PROPERTYKEY::default(); cm.GetColumnCount(CM_ENUM_VISIBLE).ok()? as usize];
    cm.GetColumns(CM_ENUM_VISIBLE, &mut keys).ok()?;
    let info = |k: &PROPERTYKEY| {
        let mask = (CM_MASK_WIDTH.0 | CM_MASK_NAME.0) as u32;
        let mut ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: mask, ..Default::default() };
        _ = cm.GetColumnInfo(k, &mut ci);
        let name = String::from_utf16_lossy(&ci.wszName[..ci.wszName.iter().position(|&c| c == 0).unwrap_or(0)]);
        let on_list = titles.iter().position(|t| *t == name).map(|c| SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0 as u32);
        (*k, on_list.filter(|&w| w > 0).unwrap_or(ci.uWidth), name)
    };
    let mut left: Vec<_> = keys.iter().map(info).collect();
    let mut cols = Vec::new();
    for name in &shown {
        if let Some(at) = left.iter().position(|c| c.2 == *name) {
            cols.push(left.remove(at));
        }
    }
    cols.extend(left); // any the header didn't name keep the view's order
    let cols: Vec<_> = cols.into_iter().map(|(k, w, _)| (k, w)).collect();
    let mut sort = vec![SORTCOLUMN::default(); fv.GetSortColumnCount().ok()?.max(0) as usize];
    fv.GetSortColumns(&mut sort).ok()?;
    Some(cols_text(&cols, &sort.iter().map(|c| (c.propkey, c.direction.0)).collect::<Vec<_>>()))
}

// The view keeps its own floor (Size: 90 px) under widths set through it, also when it builds a new
// folder's columns: narrower ones go straight onto the list, which lets them through (list_floor_proc).
pub(crate) unsafe fn put_widths(fv: &IFolderView2, text: &str, lv: HWND) {
    let Ok(cm) = fv.cast::<IColumnManager>() else { return };
    let titles = list_titles(lv);
    for (k, w) in parse_cols(text).0 {
        let mut ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: CM_MASK_NAME.0 as u32, ..Default::default() };
        _ = cm.GetColumnInfo(&k, &mut ci);
        let name = String::from_utf16_lossy(&ci.wszName[..ci.wszName.iter().position(|&c| c == 0).unwrap_or(0)]);
        if let Some(c) = titles.iter().position(|t| *t == name) {
            if SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0 != w as isize {
                SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w as isize)));
            }
        }
    }
}

pub(crate) unsafe fn apply_cols(fv: &IFolderView2, text: &str, lv: HWND) {
    let (cols, sort) = parse_cols(text);
    if let (false, Ok(cm)) = (cols.is_empty(), fv.cast::<IColumnManager>()) {
        if cm.SetColumns(&cols.iter().map(|c| c.0).collect::<Vec<_>>()).is_ok() {
            for (k, w) in &cols {
                let ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: CM_MASK_WIDTH.0 as u32, uWidth: *w, ..Default::default() };
                _ = cm.SetColumnInfo(k, &ci);
            }
            put_widths(fv, text, lv);
        }
    }
    if !sort.is_empty() {
        _ = fv.SetSortColumns(&sort.iter().map(|&(propkey, d)| SORTCOLUMN { propkey, direction: SORTDIRECTION(d) }).collect::<Vec<_>>());
    }
}

// Ctrl+F3 / F5 / F6 (Total Commander's keys): column, first direction (size and date: largest / newest
// first, as in the Everything window), status note for the first direction, for the other one.
pub(crate) const SORT_KEYS: [(PROPERTYKEY, i32, [&str; 2], [&str; 2]); 3] = {
    use windows::Win32::Storage::EnhancedStorage::{PKEY_DateModified, PKEY_ItemNameDisplay, PKEY_Size};
    [
        (PKEY_ItemNameDisplay, 1, ["Sorted by name · A→Z", "이름순 · A→Z"], ["Sorted by name · Z→A", "이름순 · Z→A"]),
        (PKEY_DateModified, -1, ["Sorted by date modified · newest first", "수정한 날짜순 · 최신부터"], ["Sorted by date modified · oldest first", "수정한 날짜순 · 오래된 것부터"]),
        (PKEY_Size, -1, ["Sorted by size · largest first", "크기순 · 큰 것부터"], ["Sorted by size · smallest first", "크기순 · 작은 것부터"]),
    ]
};

// Sorts pane i's list by SORT_KEYS[kind]; the pane keeps it in its columns (also while they settle, when
// save_view doesn't read them). A place without that column (or no view) only beeps.
pub(crate) unsafe fn sort_by(i: usize, kind: usize) {
    let (key, first, note_first, note_other) = SORT_KEYS[kind];
    // Ok(None) is a list with no sort column; Err is a failed query (not the same thing: beep, change nothing).
    let top = |fv: &IFolderView2| -> windows::core::Result<Option<(PROPERTYKEY, i32)>> {
        let mut s = vec![SORTCOLUMN::default(); fv.GetSortColumnCount()?.max(0) as usize];
        if !s.is_empty() {
            fv.GetSortColumns(&mut s)?;
        }
        Ok(s.first().map(|c| (c.propkey, c.direction.0)))
    };
    let Some((fv, cur)) = view_of::<IFolderView2>(i).and_then(|fv| top(&fv).ok().map(|c| (fv, c))) else {
        fail(Fail::SortHere, "");
        return;
    };
    let dir = sort_next(cur, key, first);
    if fv.SetSortColumns(&[SORTCOLUMN { propkey: key, direction: SORTDIRECTION(dir) }]).is_err() || top(&fv).ok() != Some(Some((key, dir))) {
        fail(Fail::SortHere, "");
        return;
    }
    app(|a| {
        if is_fs(&a.s.paths[i]) {
            a.s.cols[i] = with_sort(&a.s.cols[i], key, dir);
        }
        if let Some(c) = a.panes[i].as_mut().and_then(|p| p.pending.as_mut()) {
            *c = with_sort(c, key, dir);
        }
    });
    let n = if dir == first { note_first } else { note_other };
    set_note(tr(n[0], n[1]).into());
}

// A few passes after a folder change (50, 150, 400, 1000 ms) put narrow widths back as the view builds
// its columns at its own floor; then the pane's columns are read from the view again.
pub(crate) unsafe fn settle_cols(h: HWND) {
    _ = KillTimer(Some(h), TIMER_SETTLE);
    let Some(i) = pane_of(h) else { return };
    let Some((cols, left)) = app(|a| a.panes[i].as_ref().and_then(|p| p.settle.clone())) else { return };
    if let Some(fv) = view_of::<IFolderView2>(i) {
        let lv = view_list(&fv); // the new folder's list exists by now
        free_min_widths(lv);
        put_widths(&fv, &cols, lv);
    }
    let left = left - 1;
    app(|a| {
        if let Some(p) = a.panes[i].as_mut() {
            p.settle = (left > 0).then(|| (cols, left));
        }
    });
    if left > 0 {
        SetTimer(Some(h), TIMER_SETTLE, [1000, 400, 150][left as usize - 1], None);
    }
}

pub(crate) fn tabs_on() -> bool {
    TABS_ON.load(Ordering::Relaxed)
}

pub(crate) unsafe fn on_nav(i: usize, pidl: *const ITEMIDLIST) {
    let parse = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
    let disp = name(pidl, SIGDN_DESKTOPABSOLUTEEDITING);
    let Some((h, eb, view, cols, ticket)) = app(|a| {
        let fs = is_fs(&parse);
        let old = std::mem::replace(&mut a.s.paths[i], parse.clone());
        if let Some(p) = a.panes[i].as_mut() {
            unplug_step(&mut p.unplugged, Nav::Complete, &parse);
        }
        // a followed rename is no visit: its old name no longer exists (shell_tick fixed the recent list)
        let followed = a.panes[i].as_mut().and_then(|p| rec_step(&mut p.follow, Nav::Complete, &parse)).is_some();
        if followed {
            follow_note(a, &disp);
        } else {
            visit(&mut a.s.recent, &old, &a.s.paths[i]);
        }
        if let Some(m) = a.panes[i].as_mut().and_then(|p| rec_step(&mut p.recover, Nav::Complete, &parse)) {
            gone_note(a, &m, &disp); // update_status shows it
        }
        let (v, own) = (a.s.views[i], a.s.cols[i].clone());
        a.panes[i].as_mut().map(|p| {
            p.disp = disp.clone();
            // the old folder's icon goes with its address (sync_addr below repaints the pill); a new lookup starts
            (p.icon, p.ticket) = (-1, next_ticket(&ICON_TICKET));
            // waiting columns (session, open in) first, else the pane's own; none in shell places
            let cols = p.pending.take().unwrap_or(own);
            (p.hwnd, p.eb.clone(), v, Some(cols).filter(|c| fs && !c.is_empty()), p.ticket)
        })
    }) else {
        return;
    };
    if let Ok(e) = GetDlgItem(Some(h), ID_EDIT) {
        _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
        sync_addr(h);
        show_path_end(e, &disp);
    }
    request_icon(h, ticket, pidl);
    if tabs_on() {
        repaint_tabs(i); // the current tab's name is this folder's
    }
    if let Ok(b) = GetDlgItem(Some(h), ID_FAV) {
        _ = InvalidateRect(Some(b), None, false);
    }
    nav_counted(i, pidl);
    if let Ok(fv) = eb.GetCurrentView::<IFolderView2>() {
        if view.0 != 0 {
            _ = fv.SetViewModeAndIconSize(FOLDERVIEWMODE(view.0), view.1);
        }
        let lv = view_list(&fv);
        free_min_widths(lv);
        if let Some(cols) = cols {
            apply_cols(&fv, &cols, lv);
            app(|a| {
                if let Some(p) = a.panes[i].as_mut() {
                    p.settle = Some((cols, 4));
                }
            });
            SetTimer(Some(h), TIMER_SETTLE, 50, None);
        }
        // No full-row select: a drag that starts in the size/date cells draws a selection box
        // (and selects the names it covers) instead of grabbing that row's file.
        _ = fv.SetCurrentFolderFlags(FWF_FULLROWSELECT.0 as u32, 0);
    }
    if DARK_ON.load(Ordering::Relaxed) {
        dark_list(h);
    }
    if app(|a| a.s.active == i) {
        request_tree_sync(); // the list first; the tree follows the last of quick moves only
    }
    // set_layout's focus for a pane made in place of the active one: once, and only if focus went nowhere else since
    if app(|a| a.focus_req == h && a.s.active == i && std::mem::take(&mut a.focus_req) == h) {
        let f = GetFocus();
        if f.0.is_null() || f == app(|a| a.main) {
            focus_view(i);
        }
    }
    // a tab key that browsed (keyboard.rs tab_said): read out now that the new view has the focus, so the
    // focus change doesn't cut it off. Only while that pane is still the active one; otherwise just drop it.
    if app(|a| a.say_nav == Some(i) && a.say_nav.take().is_some() && a.s.active == i) {
        say_pane(i);
    }
}

// ---------- address bar folder icon ----------
// Looked up off the UI thread (a slow share can take seconds). Tickets never repeat (0 = none asked), so a late
// result can't land on a newer folder, nor on a new pane that got the same HWND value.
pub(crate) static ICON_TICKET: AtomicU32 = AtomicU32::new(1);

pub(crate) fn next_ticket(t: &AtomicU32) -> u32 {
    loop {
        let n = t.fetch_add(1, Ordering::Relaxed);
        if n != 0 {
            return n;
        }
    }
}

// A result is taken only for the pane's current lookup.
pub(crate) fn take_icon(cur: u32, msg: u32, idx: i32) -> Option<i32> {
    (cur != 0 && cur == msg).then_some(idx)
}

// Lookups not started yet: (pane HWND, ticket, PIDL copy), at most one per pane, so at most MAXP.
// The PIDL belongs to the queue from push until the worker takes it; one replaced or removed comes back to be freed.
pub(crate) type IconReq = (isize, u32, usize);
pub(crate) static ICON_Q: Mutex<Vec<IconReq>> = Mutex::new(Vec::new());
pub(crate) static ICON_WORKER: OnceLock<Option<isize>> = OnceLock::new(); // its wake event; None = couldn't start

pub(crate) fn icon_q() -> std::sync::MutexGuard<'static, Vec<IconReq>> {
    ICON_Q.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn enqueue(q: &mut Vec<IconReq>, req: IconReq) -> Option<usize> {
    let old = unqueue(q, req.0);
    q.push(req);
    old
}

pub(crate) fn unqueue(q: &mut Vec<IconReq>, h: isize) -> Option<usize> {
    let k = q.iter().position(|r| r.0 == h)?;
    Some(q.remove(k).2)
}

pub(crate) unsafe fn request_icon(h: HWND, ticket: u32, pidl: *const ITEMIDLIST) {
    let c = ILClone(pidl);
    if c.is_null() {
        return;
    }
    let ev = *ICON_WORKER.get_or_init(|| {
        let ev = CreateEventW(None, false, false, PCWSTR::null()).ok()?;
        let raw = ev.0 as isize;
        match std::thread::Builder::new().spawn(move || icon_worker(raw)) {
            Ok(_) => Some(raw),
            Err(_) => {
                _ = CloseHandle(ev);
                None
            }
        }
    });
    let Some(ev) = ev else {
        ILFree(Some(c));
        return;
    };
    let old = enqueue(&mut icon_q(), (h.0 as isize, ticket, c as usize));
    if let Some(old) = old {
        ILFree(Some(old as *const ITEMIDLIST));
    }
    _ = SetEvent(HANDLE(ev as _));
}

// Lives until the process ends (never joined: a dead share mustn't hold up closing). One lookup at a time, oldest
// first; it sleeps only on an empty queue, and pumps messages so COM calls into this apartment keep moving.
pub(crate) fn icon_worker(ev: isize) {
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let mut m = MSG::default();
        loop {
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            let next = {
                let mut q = icon_q();
                (!q.is_empty()).then(|| q.remove(0))
            };
            let Some((h, ticket, pidl)) = next else {
                MsgWaitForMultipleObjectsEx(Some(&[HANDLE(ev as _)]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                continue;
            };
            let pidl = pidl as *const ITEMIDLIST;
            let mut sfi = SHFILEINFOW::default();
            let flags = SHGFI_PIDL | SHGFI_SYSICONINDEX | SHGFI_SMALLICON; // an index only: no HICON to free
            let found = com && SHGetFileInfoW(PCWSTR(pidl as _), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut sfi), size_of::<SHFILEINFOW>() as u32, flags) != 0;
            ILFree(Some(pidl));
            let idx = if found { sfi.iIcon.max(-1) } else { -1 };
            _ = PostMessageW(Some(HWND(h as _)), WM_PANE_ICON, WPARAM(ticket as usize), LPARAM(idx as isize)); // fails if the pane is gone
        }
    }
}

#[implement(IExplorerBrowserEvents)]
pub(crate) struct Events(pub(crate) Arc<AtomicUsize>); // the pane's slot, read at each event (panes can swap places)

impl IExplorerBrowserEvents_Impl for Events_Impl {
    fn OnNavigationPending(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        let i = self.0.load(Ordering::SeqCst);
        // this move's kind for the faint ← / → (nav_counted): browse()'s back or forward, else a new folder
        app(|a| a.panes.get_mut(i).and_then(|p| p.as_mut()).map(|p| p.moving = Some(p.going.take().unwrap_or(Go::New))));
        unsafe {
            rec_event(i, Nav::Pending, pidl);
            save_view(i)
        };
        Ok(())
    }
    fn OnViewCreated(&self, sv: Ref<IShellView>) -> Result<()> {
        // each browse makes a new list: a filtered one says the filter, not "This folder is empty", when nothing shows
        let spec = filter_of(self.0.load(Ordering::SeqCst)); // a copy: the lock is let go before the shell is called
        if !spec.is_empty() {
            if let Some(fv) = sv.ok().ok().and_then(|v| v.cast::<IFolderView2>().ok()) {
                unsafe { set_empty_text(&fv, &spec) };
            }
        }
        Ok(())
    }
    fn OnNavigationComplete(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        unsafe { on_nav(self.0.load(Ordering::SeqCst), pidl) };
        Ok(())
    }
    fn OnNavigationFailed(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        let i = self.0.load(Ordering::SeqCst);
        unsafe { rec_event(i, Nav::Failed, pidl) };
        app(|a| a.say_nav = a.say_nav.filter(|&p| p != i)); // the tab key's words go with its browse
        app(|a| a.panes.get_mut(i).and_then(|p| p.as_mut()).map(|p| p.moving = None)); // no step taken
        Ok(())
    }
}

// The selection when it is exactly one folder on disk (no ZIP, no shell place such as a library or This PC).
unsafe fn lone_folder(sv: &IShellView) -> Option<IShellItem> {
    let arr = sv.cast::<IFolderView2>().ok()?.GetSelection(false).ok()?;
    if arr.GetCount().ok()? != 1 {
        return None;
    }
    let it = arr.GetItemAt(0).ok()?;
    let attr = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0 | SFGAO_FILESYSTEM.0)).ok()?.0;
    (real_folder(attr) && attr & SFGAO_FILESYSTEM.0 != 0).then_some(it)
}

// Site of each Explorer control. It asks the site for ICommDlgBrowser, whose IncludeObject is the name filter.
#[implement(IServiceProvider, ICommDlgBrowser)]
pub(crate) struct Host(pub(crate) Arc<AtomicUsize>); // the pane's slot, as in Events

impl IServiceProvider_Impl for Host_Impl {
    fn QueryService(&self, service: *const GUID, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        unsafe {
            *ppv = std::ptr::null_mut();
            // SID_SExplorerBrowserFrame is defined as IID_ICommDlgBrowser.
            if *service == ICommDlgBrowser::IID {
                return self.to_interface::<ICommDlgBrowser>().query(riid, ppv).ok();
            }
        }
        Err(E_NOINTERFACE.into())
    }
}

impl ICommDlgBrowser_Impl for Host_Impl {
    fn OnDefaultCommand(&self, sv: Ref<IShellView>) -> Result<()> {
        // Folders set to open in Grids: the default verb would start grids.exe to hand the folder back to this
        // window, so one selected folder opens right here, as before. Anything else: the view's default action.
        // Ctrl/Shift held: the view's own variant (new window etc.) as before, not this pane.
        unsafe {
            let modded = GetKeyState(VK_CONTROL.0 as i32) < 0 || GetKeyState(VK_SHIFT.0 as i32) < 0;
            if !modded && default_open_on() {
                if let Some(it) = sv.ok().ok().and_then(|v| lone_folder(v)) {
                    browse_item(self.0.load(Ordering::SeqCst), &it);
                    return Ok(());
                }
            }
        }
        Err(S_FALSE.into()) // let the view do its default action (open)
    }
    fn OnStateChange(&self, _: Ref<IShellView>, change: u32) -> Result<()> {
        if change == CDBOSC_SELCHANGE {
            SEL_REV[self.0.load(Ordering::SeqCst)].fetch_add(1, Ordering::Relaxed); // same count, other items: the status line still updates
        }
        Ok(())
    }
    fn IncludeObject(&self, sv: Ref<IShellView>, pidl: *const ITEMIDLIST) -> Result<()> {
        // may run on the view's own thread: the slot is read under the lock swap_panes moves it under
        let spec = FILTERS.lock().map(|f| f[self.0.load(Ordering::SeqCst)].clone()).unwrap_or_default();
        if spec.is_empty() {
            return Ok(());
        }
        unsafe {
            let sf: IShellFolder = sv.ok()?.cast::<IFolderView>()?.GetFolder()?;
            let mut attr = SFGAO_FOLDER.0 | SFGAO_STREAM.0;
            if filter_keeps_unnamed(sf.GetAttributesOf(&[pidl], &mut attr).ok().map(|_| attr)) {
                return Ok(()); // folders stay, so a filtered pane can still be navigated
            }
            let mut sr = STRRET::default();
            let mut name = PWSTR::null();
            sf.GetDisplayNameOf(pidl, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0), &mut sr)?;
            StrRetToStrW(&mut sr, Some(pidl), &mut name)?;
            let hit = PathMatchSpecW(PCWSTR(name.0), &HSTRING::from(spec)).as_bool();
            CoTaskMemFree(Some(name.0 as _));
            if hit {
                Ok(())
            } else {
                Err(S_FALSE.into())
            }
        }
    }
}

// A folder dropped on a pane's top bar (its fields and buttons too: OLE walks up to the pane) opens there.
// Never MOVE: a shell source deletes what it moved. LINK or COPY leaves the source alone.
pub(crate) fn drop_pick(allowed: DROPEFFECT) -> DROPEFFECT {
    [DROPEFFECT_LINK, DROPEFFECT_COPY].into_iter().find(|e| allowed.0 & e.0 != 0).unwrap_or(DROPEFFECT_NONE)
}

pub(crate) unsafe fn drop_items(d: &Ref<IDataObject>) -> Option<IShellItemArray> {
    let arr: IShellItemArray = SHCreateShellItemArrayFromDataObject(d.as_ref()?).ok()?;
    (arr.GetCount().unwrap_or(0) > 0).then_some(arr)
}

// A folder's own shell drop target (what Explorer's folder icon takes drops with): copy / move / link by
// Explorer's rules (same drive moves, Ctrl copies, Shift moves, Alt links, a right-button drag asks), with the
// shell's conflict dialog, progress and Ctrl+Z; the Recycle Bin's deletes. None: the place takes no drops.
pub(crate) unsafe fn folder_drop_target(place: &str) -> Option<IDropTarget> {
    place_item(place)?.BindToHandler(None, &BHID_SFUIObject).ok()
}

// A pane place as a shell item: a path or ::{GUID} parses; the Recycle Bin is the known folder.
unsafe fn place_item(place: &str) -> Option<IShellItem> {
    if place == RECYCLE_BIN {
        SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None).ok()
    } else {
        SHCreateItemFromParsingName(&HSTRING::from(place), None).ok()
    }
}

// The folder a pane shows, dragged out from its address bar icon as from Explorer's: the shell's own data
// object, so Explorer, the desktop, a browser's upload box, an editor, a terminal or another pane's header take
// it as that folder. Copy or link only, never move (what the pane shows stays put); link is offered as the
// preferred effect, as Explorer's address icon makes a shortcut. The drag runs its own message loop: never
// inside app(), and nothing about the pane is used again after it (the pane may have changed meanwhile).
pub(crate) unsafe fn drag_folder_out(h: HWND, place: &str) {
    let Some(data) = place_item(place).and_then(|it| it.BindToHandler::<_, IDataObject>(None, &BHID_DataObject).ok()) else {
        fail(Fail::DragCant, "");
        return;
    };
    prefer_link(&data);
    _ = SHDoDragDrop(Some(h), &data, None, DROPEFFECT_COPY | DROPEFFECT_LINK);
}

// "Preferred DropEffect" = link on a drag's data: targets that follow it (Explorer, the desktop) default to
// a shortcut; others pick for themselves. Best effort: without it the drag still works.
unsafe fn prefer_link(data: &IDataObject) {
    let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, 4) else { return };
    let p = GlobalLock(g) as *mut u32;
    if p.is_null() {
        _ = GlobalFree(Some(g));
        return;
    }
    *p = DROPEFFECT_LINK.0;
    _ = GlobalUnlock(g);
    let fmt = FORMATETC {
        cfFormat: RegisterClipboardFormatW(w!("Preferred DropEffect")) as u16,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let med = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: g }, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
    if data.SetData(&fmt, &med, true).is_err() {
        _ = GlobalFree(Some(g)); // taken only when it succeeds
    }
}

// A drag passed on to one folder's shell drop target at a time. Its target is made once on entering that folder
// (a lost share can take seconds), left before another is entered, and let go on leave or drop.
// Never called inside app(): the shell's dialogs run messages that come back into Grids.
#[derive(Default)]
pub(crate) struct Relay {
    data: RefCell<Option<IDataObject>>,                // DragOver brings none: kept from DragEnter for the next folder
    allowed: Cell<DROPEFFECT>,                         // what the source allows, as it came (not drop_pick's)
    to: RefCell<Option<(String, Option<IDropTarget>)>>, // the folder over, and its target (None: takes no drops)
    keys: Cell<MODIFIERKEYS_FLAGS>, // keys at the last DragEnter / DragOver: the drop's own have lost the mouse button
    eff: Cell<DROPEFFECT>,          // what the folder's target said at the last DragEnter / DragOver
}

impl Relay {
    pub(crate) fn begin(&self, d: &Ref<IDataObject>, allowed: DROPEFFECT) {
        *self.data.borrow_mut() = d.cloned();
        self.allowed.set(allowed);
    }
    // Over `place` now: the effect its target gives (NONE when it has none or says no).
    pub(crate) unsafe fn over(&self, place: &str, keys: MODIFIERKEYS_FLAGS, pt: &POINTL) -> DROPEFFECT {
        self.keys.set(keys);
        let mut eff = self.allowed.get();
        let same = self.to.borrow().as_ref().is_some_and(|(p, _)| p == place);
        if same {
            let t = self.to.borrow().as_ref().and_then(|(_, t)| t.clone());
            if !t.is_some_and(|t| t.DragOver(keys, *pt, &mut eff).is_ok()) {
                eff = DROPEFFECT_NONE;
            }
            self.eff.set(eff);
            return eff;
        }
        self.leave();
        let t = folder_drop_target(place);
        let data = self.data.borrow().clone();
        let entered = match (&t, &data) {
            (Some(t), Some(d)) => t.DragEnter(d, keys, *pt, &mut eff).is_ok(),
            _ => false,
        };
        *self.to.borrow_mut() = Some((place.to_string(), t.filter(|_| entered)));
        let eff = if entered { eff } else { DROPEFFECT_NONE };
        self.eff.set(eff);
        eff
    }
    // About to drop on `place`: the same folder keeps what its target said last (no DragOver with the drop's keys,
    // which have lost the mouse button and would turn a right-drag's menu off); another is entered with the last keys.
    pub(crate) unsafe fn settle(&self, place: &str, pt: &POINTL) -> DROPEFFECT {
        if self.to.borrow().as_ref().is_some_and(|(p, _)| p == place) {
            return self.eff.get();
        }
        self.over(place, self.keys.get(), pt)
    }
    // Off the folder (to another, or out of the drag): its target hears so.
    pub(crate) unsafe fn leave(&self) {
        if let Some((_, Some(t))) = self.to.take() {
            _ = t.DragLeave();
        }
    }
    // Dropped: the folder's target does the copy / move / delete itself (its dialogs may run here).
    pub(crate) unsafe fn drop_on(&self, d: &Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL) -> DROPEFFECT {
        let to = self.to.take();
        self.data.take();
        let mut eff = self.allowed.get();
        match (to, d.as_ref()) {
            (Some((_, Some(t))), Some(d)) if t.Drop(d, keys, *pt, &mut eff).is_ok() => eff,
            _ => DROPEFFECT_NONE,
        }
    }
    pub(crate) unsafe fn end(&self) {
        self.leave();
        self.data.take();
    }
}

#[implement(IDropTarget)]
pub(crate) struct PaneDrop {
    pub(crate) slot: Arc<AtomicUsize>,
    pub(crate) hwnd: HWND,
    pub(crate) ok: Cell<bool>, // the data being dragged holds shell items
    pub(crate) helper: Option<IDropTargetHelper>,
    pub(crate) relay: Relay,    // over another tab: passed on to that tab's folder
    pub(crate) lit: Cell<bool>, // a tab lightened for the drop (head_hover)
}

impl PaneDrop {
    // Where screen point pt is in the header: another tab's folder, a tab that takes no drops, or (#36) open.
    unsafe fn at(&self, pt: &POINTL) -> DropAt {
        let mut p = POINT { x: pt.x, y: pt.y };
        let mut rc = RECT::default();
        if !ScreenToClient(self.hwnd, &mut p).as_bool() || GetClientRect(self.hwnd, &mut rc).is_err() {
            return DropAt::Open;
        }
        let i = self.slot.load(Ordering::SeqCst);
        let (list, cur, dpi) = app(|a| (a.s.tab_list(i), a.s.tab[i], a.dpi));
        let hit = tab_hit(pane_geo(rc.right, rc.bottom, dpi).tabs, list.len(), cur, p.x, p.y, dpi);
        drop_at(hit, cur, &list)
    }
    // Shades tab k as the mouse does (None: off again).
    unsafe fn light(&self, k: Option<usize>) {
        if k.is_some() || self.lit.get() {
            self.lit.set(k.is_some());
            head_hover(self.hwnd, k.map(HeadHot::Tab));
        }
    }
    // The effect over pt: into another tab's folder by the shell's rules, else the header's own open (#36).
    unsafe fn route(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) {
        self.relay.keys.set(keys); // also off the tabs: a drop that lands on one enters it with these
        match self.at(pt) {
            DropAt::Into(k, path) if self.ok.get() => {
                self.hover(false);
                *effect = self.relay.over(&path, keys, pt);
                self.light((*effect != DROPEFFECT_NONE).then_some(k)); // a folder that says no isn't lit
            }
            DropAt::Into(..) | DropAt::Refuse => {
                self.relay.leave();
                self.hover(false);
                self.light(None);
                *effect = DROPEFFECT_NONE;
            }
            DropAt::Open => {
                self.relay.leave();
                self.light(None);
                self.pick(effect);
            }
        }
    }
    // sets the effect this drop would get, and tints the header only while it isn't NONE
    pub(crate) unsafe fn pick(&self, effect: *mut DROPEFFECT) {
        *effect = if self.ok.get() { drop_pick(*effect) } else { DROPEFFECT_NONE };
        self.hover(*effect != DROPEFFECT_NONE);
    }
    pub(crate) unsafe fn hover(&self, on: bool) {
        let h = self.hwnd;
        let changed = app(|a| {
            let was = a.drop_hover == h;
            if on {
                a.drop_hover = h;
            } else if was {
                a.drop_hover = HWND::default();
            }
            was != on
        });
        if changed {
            repaint_header(h);
        }
    }
    // the folder (or a file's folder) is queued, not opened: a drag from this pane's own list is still running
    pub(crate) unsafe fn queue(&self, d: &Ref<IDataObject>) -> bool {
        let Some(item) = drop_items(d).and_then(|a| a.GetItemAt(0).ok()) else { return false };
        let is_dir = item.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
        let Some(folder) = (if is_dir { Some(item) } else { item.GetParent().ok() }) else { return false };
        _ = app(|a| a.drop_open.replace((self.hwnd, self.slot.load(Ordering::SeqCst), folder)));
        if PostMessageW(Some(self.hwnd), WM_DROP_OPEN, WPARAM(0), LPARAM(0)).is_ok() {
            return true;
        }
        _ = app(|a| a.drop_open.take());
        false
    }
}

impl IDropTarget_Impl for PaneDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.ok.set(drop_items(&d).is_some());
            self.relay.begin(&d, *effect);
            self.route(keys, pt, effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragEnter(self.hwnd, d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.route(keys, pt, effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragOver(&POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe {
            self.relay.end();
            self.light(None);
            self.hover(false);
            if let Some(hp) = &self.helper {
                _ = hp.DragLeave();
            }
        }
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            let allowed = *effect;
            *effect = DROPEFFECT_NONE;
            self.hover(false);
            self.light(None); // off before the shell's dialogs, if any
            let image_off = |eff: DROPEFFECT| {
                if let Some(hp) = &self.helper {
                    _ = hp.Drop(d.as_ref(), &POINT { x: pt.x, y: pt.y }, eff);
                }
            };
            match self.at(pt) {
                // into another tab's folder: the shell copies / moves; this pane stays where it is (no queue)
                DropAt::Into(_, path) if self.ok.get() => {
                    let eff = self.relay.settle(&path, pt);
                    image_off(eff); // the drag image goes before a conflict or confirm dialog can come up
                    if eff != DROPEFFECT_NONE {
                        *effect = self.relay.drop_on(&d, keys, pt);
                    }
                    self.relay.end();
                    return Ok(());
                }
                DropAt::Into(..) | DropAt::Refuse => {}
                DropAt::Open => {
                    self.relay.end();
                    let eff = drop_pick(allowed);
                    if eff != DROPEFFECT_NONE && self.queue(&d) {
                        *effect = eff;
                    }
                }
            }
            self.relay.end();
            image_off(*effect);
        }
        Ok(())
    }
}

// Pressed look of the toolbar button something is being dragged over (drive or Recycle Bin), 0: none.
pub(crate) static DROP_BTN: AtomicUsize = AtomicUsize::new(0);

unsafe fn press_drop_btn(h: HWND, on: bool) {
    let v = h.0 as usize;
    let was = if on { DROP_BTN.swap(v, Ordering::Relaxed) } else { DROP_BTN.compare_exchange(v, 0, Ordering::Relaxed, Ordering::Relaxed).unwrap_or(0) };
    if (was == v) != on {
        _ = InvalidateRect(Some(h), None, false);
    }
}

// A drive button ("X:\") or the Recycle Bin button (RECYCLE_BIN): files dropped on it go there as onto that
// folder in Explorer. Registered when the button is made, revoked before it is destroyed.
#[implement(IDropTarget)]
pub(crate) struct ButtonDrop {
    pub(crate) hwnd: HWND,
    pub(crate) place: String,
    pub(crate) relay: Relay,
    pub(crate) helper: Option<IDropTargetHelper>,
}

pub(crate) unsafe fn button_drop(h: HWND, place: String) {
    let helper = CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok(); // drag image over the button
    let dt: IDropTarget = ButtonDrop { hwnd: h, place, relay: Relay::default(), helper }.into();
    _ = RegisterDragDrop(h, &dt);
}

impl ButtonDrop {
    unsafe fn over(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) {
        *effect = self.relay.over(&self.place, keys, pt);
        press_drop_btn(self.hwnd, *effect != DROPEFFECT_NONE);
    }
}

impl IDropTarget_Impl for ButtonDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.relay.begin(&d, *effect);
            self.over(keys, pt, effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragEnter(self.hwnd, d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.over(keys, pt, effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragOver(&POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe {
            self.relay.end();
            press_drop_btn(self.hwnd, false);
            if let Some(hp) = &self.helper {
                _ = hp.DragLeave();
            }
        }
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            let eff = self.relay.settle(&self.place, pt);
            press_drop_btn(self.hwnd, false);
            if let Some(hp) = &self.helper {
                _ = hp.Drop(d.as_ref(), &POINT { x: pt.x, y: pt.y }, eff); // before the shell's dialogs, if any
            }
            *effect = if eff != DROPEFFECT_NONE { self.relay.drop_on(&d, keys, pt) } else { DROPEFFECT_NONE };
            self.relay.end();
        }
        Ok(())
    }
}


pub(crate) unsafe fn pwstr(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as _));
    s
}

pub(crate) unsafe fn name(pidl: *const ITEMIDLIST, kind: SIGDN) -> String {
    SHGetNameFromIDList(pidl, kind).map(|p| pwstr(p)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_list_moves_renamed_folders() {
        let r = |o: &str, n: &str| Change::Renamed(o.into(), n.into());
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let yes = |_: &str, _: &str| true;
        let list = v(&[r"C:\a\b", r"C:\a\bc", r"D:\z", r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", r"\\nas\share"]);
        // the folder itself; a look-alike name, a shell folder and a share stay
        let out = v(&[r"C:\a\x", r"C:\a\bc", r"D:\z", r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", r"\\nas\share"]);
        assert_eq!(follow_list(&list, &[r(r"C:\a\b", r"C:\a\x")], yes), (out, true));
        // a parent renamed: C:\a\b under C:\a → C:\x
        assert_eq!(follow_list(&v(&[r"C:\a\b"]), &[r(r"C:\a", r"C:\x")], yes), (v(&[r"C:\x\b"]), true));
        // two renames in a row
        assert_eq!(follow_list(&v(&[r"C:\a\b"]), &[r(r"C:\a\b", r"C:\a\c"), r(r"C:\a", r"C:\x")], yes), (v(&[r"C:\x\c"]), true));
        // not confirmed on disk, or removed (it may come back from the Recycle Bin): unchanged
        assert_eq!(follow_list(&v(&[r"C:\a\b"]), &[r(r"C:\a\b", r"C:\a\x")], |_, _| false), (v(&[r"C:\a\b"]), false));
        assert_eq!(follow_list(&v(&[r"C:\a\b"]), &[Change::Removed(r"C:\a".into())], yes), (v(&[r"C:\a\b"]), false));
        // the new name was already listed: the first stays, order kept
        let list = v(&[r"C:\p", r"C:\a\x", r"C:\q", r"C:\a\b"]);
        assert_eq!(follow_list(&list, &[r(r"C:\a\b", r"C:\a\x")], yes), (v(&[r"C:\p", r"C:\a\x", r"C:\q"]), true));
        let list = v(&[r"C:\p", r"C:\a\b", r"C:\q", r"C:\A\X"]);
        assert_eq!(follow_list(&list, &[r(r"C:\a\b", r"C:\a\x")], yes), (v(&[r"C:\p", r"C:\a\x", r"C:\q"]), true));
        // nothing moved: duplicates still drop (the recent list as before), and the flag says nothing moved,
        // so favourites keep a case-only duplicate
        let list = v(&[r"C:\p", r"C:\P", r"C:\q"]);
        assert_eq!(follow_list(&list, &[r(r"C:\a", r"C:\b")], yes), (v(&[r"C:\p", r"C:\q"]), false));
    }
}
