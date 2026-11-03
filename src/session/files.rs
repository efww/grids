use crate::imports::*;
use super::*;
use crate::app::*;
use crate::ui::*;
use crate::i18n::*;
use crate::store;

// ---------- sessions ----------

thread_local! {
    pub(crate) static LAST_SAVED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Writes every tab (name, lock, order, folders, filters, columns) and the window.
/// The startup mode is stored beside that list; it does not gate the write.
pub(crate) unsafe fn save_session() {
    if window_count() == 0 {
        return;
    }
    publish_shared();
    // Snaps are read by from_session. That runs inside app() below, so it must not call app() again.
    set_snaps(snapshot_windows());
    let text = app(|a| store::sync_text(&a.s));
    let same = LAST_SAVED.with(|l| l.borrow().as_str() == text);
    if same || store::write_settings(&text).is_ok() {
        if !same {
            LAST_SAVED.with(|l| *l.borrow_mut() = text);
        }
        app(|a| {
            a.save_since = None;
            a.save_after = None;
        });
    } else {
        // The old maguro.json stays. Try again in 5 seconds, unless a drag or resize is in progress.
        app(|a| a.save_after = Some(Instant::now() + std::time::Duration::from_secs(5)));
    }
}

/// Status timer: a failed write is retried once the wait is over and the window is not being dragged.
pub(crate) unsafe fn retry_save() {
    let now = Instant::now();
    let due = app(|a| !a.drag.is_some() && !a.sizing && a.save_after.is_some_and(|t| t <= now));
    if due {
        save_session();
    }
}

/// Coalesce tab, folder and window edits into one write, and never wait longer than SAVE_LATEST_MS.
pub(crate) unsafe fn note_save() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        return;
    }
    let flush = app(|a| {
        let now = Instant::now();
        let late = a.save_since.is_some_and(|t| now.duration_since(t).as_millis() >= SAVE_LATEST_MS);
        if late {
            a.save_since = None;
            true
        } else {
            if a.save_since.is_none() {
                a.save_since = Some(now);
            }
            false
        }
    });
    if flush {
        _ = KillTimer(Some(main), TIMER_SAVE);
        save_session();
    } else {
        SetTimer(Some(main), TIMER_SAVE, SAVE_SOON_MS, None);
    }
}

// Save button / Ctrl+S: overwrite the active tab's session file. A tab with none asks for a name.
pub(crate) unsafe fn save_current() {
    remember_tabs();
    let (i, path) = app(|a| {
        let i = a.s.tab;
        let path = match save_choice(&a.s.tabs, i) {
            SaveChoice::Overwrite(p) => Some(p.to_string()),
            SaveChoice::AskName => None,
        };
        (i, path)
    });
    match path {
        Some(file) => write_tab_session(i, Path::new(&file)),
        None => _ = save_session_as(),
    }
}

/// Write tab `i` into `path` and link that tab only. Other tabs keep their files.
pub(crate) unsafe fn write_tab_session(i: usize, path: &Path) {
    remember_tabs();
    let Some(text) = app(|a| tab_session_text(&a.s, i)) else { return };
    if crate::settings::store::write_atomic(path, text.as_bytes()).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let path_s = path.to_string_lossy().into_owned();
    app(|a| {
        if bind_tab_session(&mut a.s, i, &path_s) {
            mark_tab_clean(a, i);
            let _ = pull_dirty(a);
        }
    });
    refresh_save_button();
}

pub(crate) unsafe fn write_session(path: &Path) {
    let i = app(|a| a.s.tab);
    write_tab_session(i, path);
}

/// Each tab that already has a session file. Tabs without one are left for Save to name.
pub(crate) unsafe fn autosave_associated_tabs() {
    if !app(|a| a.s.autosave) {
        return;
    }
    remember_tabs();
    let jobs = app(|a| {
        a.s.tabs.iter().enumerate().filter(|(_, t)| !t.session.is_empty()).map(|(i, t)| (i, t.session.clone())).collect::<Vec<_>>()
    });
    for (i, path) in jobs {
        let Some(text) = app(|a| tab_session_text(&a.s, i)) else { continue };
        if crate::settings::store::write_atomic(Path::new(&path), text.as_bytes()).is_err() {
            _ = MessageBeep(MB_ICONWARNING);
            continue;
        }
        app(|a| mark_tab_clean(a, i));
    }
    app(|a| { let _ = pull_dirty(a); });
    refresh_save_button();
}

pub(crate) fn pull_dirty(a: &mut crate::Desk) -> bool {
    heal_tab_base(a);
    let tab = a.s.tab;
    let dirty = a.s.dot && active_tab_dirty(&a.s, a.tab_base.get(tab).map(|b| &b.grid), &a.s.order);
    std::mem::replace(&mut a.dirty, dirty) != dirty
}

fn heal_tab_base(a: &mut crate::Desk) {
    let n = a.s.tabs.len();
    if a.tab_base.len() < n {
        for i in a.tab_base.len()..n {
            let grid = a.s.tabs.get(i).cloned().unwrap_or_else(|| Tab::from_live(&a.s, String::new(), false, vec![String::new(); crate::MAXP]));
            a.tab_base.push(TabBase { grid, order: Vec::new() });
        }
    } else if a.tab_base.len() > n {
        a.tab_base.truncate(n);
    }
}

pub(crate) fn stash_tab_order(a: &mut crate::Desk, i: usize) {
    heal_tab_base(a);
    if let Some(b) = a.tab_base.get_mut(i) {
        b.order = a.s.order.clone();
    }
}

pub(crate) fn restore_tab_order(a: &mut crate::Desk, i: usize) {
    heal_tab_base(a);
    a.s.order = a.tab_base.get(i).map(|b| b.order.clone()).unwrap_or_default();
}

pub(crate) fn mark_tab_clean(a: &mut crate::Desk, i: usize) {
    heal_tab_base(a);
    let Some(grid) = a.s.tabs.get(i).cloned() else { return };
    if let Some(b) = a.tab_base.get_mut(i) {
        b.grid = grid;
        b.order.clear();
    }
    if a.s.tab == i {
        a.s.order.clear();
        a.dirty = false;
    }
}

pub(crate) fn insert_tab_base(a: &mut crate::Desk, at: usize, tab: &Tab) {
    let at = at.min(a.tab_base.len());
    a.tab_base.insert(at, TabBase { grid: tab.clone(), order: Vec::new() });
}

pub(crate) fn remove_tab_base(a: &mut crate::Desk, i: usize) {
    if i < a.tab_base.len() {
        a.tab_base.remove(i);
    }
}

/// Why a typed session name was refused.
#[derive(Debug, PartialEq)]
pub(crate) enum NameErr {
    Invalid,
    TooLong,
}

pub(crate) const SESSION_NAME_MAX: usize = 120;

/// The `.maguro` file a typed name saves to in `dir`, and whether a session with that name is already there
/// (any letter case).
pub(crate) fn session_target(dir: &Path, input: &str) -> std::result::Result<(PathBuf, bool), NameErr> {
    let stem = session_stem_ok(input).ok_or(NameErr::Invalid)?;
    if stem.chars().count() > SESSION_NAME_MAX {
        return Err(NameErr::TooLong);
    }
    let path = dir.join(format!("{stem}.maguro"));
    let low = stem.to_lowercase();
    let taken = std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let p = e.path();
        is_session_file(&p) && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == low)
    });
    Ok((path, taken))
}

/// Name-only save prompt for the sessions folder. A bad name is explained and asked again; a taken name
/// is replaced only after a Yes (No is the default). None means cancel.
pub(crate) unsafe fn ask_session_path(title: &str) -> Option<PathBuf> {
    let main = app(|a| a.main);
    let dir = sessions_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return None;
    }
    let caption = HSTRING::from(title);
    let mut name = String::new();
    loop {
        name = crate::settings::ask(main, title, &name)?;
        match session_target(&dir, &name) {
            Err(e) => {
                let why = match e {
                    NameErr::Invalid => tr(crate::Key::ThatNameCanTBeUsedLeave),
                    NameErr::TooLong => tr(crate::Key::ThatNameIsTooLong),
                };
                MessageBoxW(Some(main), &HSTRING::from(why), &caption, MB_OK | MB_ICONWARNING);
            }
            Ok((path, true)) => {
                let q = format!(
                    "{} \"{}\"\n{}",
                    tr(crate::Key::SessionWithThisNameAlreadyExists),
                    name.trim(),
                    tr(crate::Key::ReplaceIt)
                );
                if MessageBoxW(Some(main), &HSTRING::from(q), &caption, MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) == IDYES {
                    return Some(path);
                }
            }
            Ok((path, false)) => return Some(path),
        }
    }
}

pub(crate) unsafe fn save_session_as() -> Result<()> {
    if let Some(path) = ask_session_path(tr(crate::Key::SaveThisTab)) {
        write_session(&path);
    }
    Ok(())
}

/// Writes one tab's grid to a named session and links that tab. Other tabs keep their files.
pub(crate) unsafe fn save_tab_as_session(i: usize) {
    remember_tabs();
    if app(|a| i >= a.s.tabs.len()) {
        return;
    }
    let Some(path) = ask_session_path(tr(crate::Key::SaveThisTab)) else { return };
    write_tab_session(i, &path);
}

pub(crate) unsafe fn load_session_file(path: &Path) {
    let one = [path.to_path_buf()];
    load_session_files(&one);
}

/// Each path opens as its own tab, immediately after the tab on screen. Tabs already open stay.
pub(crate) unsafe fn load_session_files(paths: &[PathBuf]) {
    let mut incoming = Vec::new();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            _ = MessageBeep(MB_ICONWARNING);
            continue;
        };
        let path_s = path.to_string_lossy().into_owned();
        let got = tabs_from_session_text(&text, &path_s);
        if got.is_empty() {
            _ = MessageBeep(MB_ICONWARNING);
            continue;
        }
        incoming.extend(got);
    }
    if incoming.is_empty() {
        return;
    }
    open_tabs_beside(incoming);
}

// Window of a running copy of this same exe. A portable.txt copy keeps its own maguro.json.
pub(crate) unsafe fn running_instance() -> Option<HWND> {
    let me = std::env::current_exe().ok()?.to_string_lossy().to_lowercase();
    let mut h = None;
    loop {
        let w = FindWindowExW(None, h, w!("MaguroMain"), PCWSTR::null()).ok()?;
        h = Some(w);
        let mut pid = 0;
        GetWindowThreadProcessId(w, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
        let (mut buf, mut n) = ([0u16; 1024], 1024u32);
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        _ = CloseHandle(p);
        if ok && String::from_utf16_lossy(&buf[..n as usize]).to_lowercase() == me {
            return Some(w);
        }
    }
}

#[repr(C)]
pub(crate) struct CopyData {
    pub(crate) dw: usize,
    pub(crate) cb: u32,
    pub(crate) lp: *const core::ffi::c_void,
}


#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn session_target_checks_names_and_taken() {
        let d = std::env::temp_dir().join(format!("pg-name-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Work.maguro"), "x").unwrap();
        std::fs::write(d.join("old.qdr"), "x").unwrap();
        assert_eq!(session_target(&d, " new "), Ok((d.join("new.maguro"), false)));
        assert_eq!(session_target(&d, "work"), Ok((d.join("work.maguro"), true)));
        assert_eq!(session_target(&d, "OLD").map(|r| r.1), Ok(false));
        for bad in ["a/b", "x?", "CON", "", "end."] {
            assert_eq!(session_target(&d, bad), Err(NameErr::Invalid), "{bad:?}");
        }
        assert_eq!(session_target(&d, &"a".repeat(SESSION_NAME_MAX + 1)), Err(NameErr::TooLong));
        _ = std::fs::remove_dir_all(&d);
    }
}
