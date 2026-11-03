use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::model::*;

// Named sessions used to store every window tab in one file. A session is now one tab.
// Files that still hold two or more tabs are split once into `Name (1)`, `Name (2)`, …
// The original file is renamed to a backup beside those files. maguro.json (the open
// window) is not rewritten here: startup restore keeps every tab it already stored.
// A path map records old file → the tab that was active, so a later launch can retarget
// a session link. The settings file is left alone.

const MAP_FILE: &str = "maguro-split-map.txt";

/// One renamed session file and the single-tab file its active tab moved to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PathMap {
    pub from: String,
    pub to: String,
}

/// What one multi-tab file became. `files` is tab order. `active` indexes that list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SplitDone {
    pub old: PathBuf,
    pub backup: PathBuf,
    pub files: Vec<PathBuf>,
    pub active: usize,
}

/// Compare session paths the way Windows stores them: case and slash do not matter.
pub(crate) fn session_path_key(path: &str) -> String {
    path.trim().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

pub(crate) fn retarget_path(path: &str, maps: &[PathMap]) -> Option<String> {
    if path.is_empty() || maps.is_empty() {
        return None;
    }
    let key = session_path_key(path);
    maps.iter().find(|m| session_path_key(&m.from) == key).map(|m| m.to.clone())
}

/// Point session links at the split file. Folder paths, layout, and the tab list stay.
pub(crate) fn retarget_session_links(s: &mut Session, maps: &[PathMap]) {
    if maps.is_empty() {
        return;
    }
    if let Some(n) = retarget_path(&s.boot_file, maps) {
        s.boot_file = n;
    }
    for t in &mut s.tabs {
        if let Some(n) = retarget_path(&t.session, maps) {
            t.session = n;
        }
    }
    if let Some(n) = retarget_path(&s.file, maps) {
        s.file = n;
    }
    settle_sessions(s);
}

/// Same retarget for a saved window in maguro.json. Pane folders are left as they are.
pub(crate) fn retarget_window_value(v: &mut Value, maps: &[PathMap]) {
    if maps.is_empty() {
        return;
    }
    let Some(obj) = v.as_object_mut() else { return };
    if let Some(s) = obj.get("session").and_then(|x| x.as_str()).map(str::to_string) {
        if let Some(n) = retarget_path(&s, maps) {
            obj.insert("session".into(), Value::String(n));
        }
    }
    if let Some(tabs) = obj.get_mut("tabs").and_then(|t| t.as_array_mut()) {
        for t in tabs {
            let Some(o) = t.as_object_mut() else { continue };
            let Some(s) = o.get("session").and_then(|x| x.as_str()).map(str::to_string) else { continue };
            if let Some(n) = retarget_path(&s, maps) {
                o.insert("session".into(), Value::String(n));
            }
        }
    }
}

fn map_path(dir: &Path) -> PathBuf {
    dir.join(MAP_FILE)
}

pub(crate) fn read_split_map(dir: &Path) -> Vec<PathMap> {
    let Ok(text) = std::fs::read_to_string(map_path(dir)) else { return Vec::new() };
    let mut out: Vec<PathMap> = Vec::new();
    for line in text.lines() {
        let Some((from, to)) = line.split_once('\t') else { continue };
        let (from, to) = (from.trim(), to.trim());
        if from.is_empty() || to.is_empty() {
            continue;
        }
        if let Some(m) = out.iter_mut().find(|m| session_path_key(&m.from) == session_path_key(from)) {
            m.to = to.to_string();
        } else {
            out.push(PathMap { from: from.to_string(), to: to.to_string() });
        }
    }
    out
}

fn append_split_map(dir: &Path, map: &PathMap) -> bool {
    let path = map_path(dir);
    let mut file = match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    writeln!(file, "{}\t{}", map.from, map.to).is_ok()
}

fn stem_taken(dir: &Path, stem: &str, reserved: &HashSet<String>) -> bool {
    if reserved.contains(&stem.to_lowercase()) {
        return true;
    }
    let low = stem.to_lowercase();
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let p = e.path();
        is_session_file(&p) && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == low)
    })
}

fn free_stem(dir: &Path, want: &str, reserved: &mut HashSet<String>) -> String {
    let mut cand = want.to_string();
    let mut n = 2u32;
    while stem_taken(dir, &cand, reserved) {
        cand = format!("{want} ({n})");
        n += 1;
        if n > 5000 {
            break;
        }
    }
    reserved.insert(cand.to_lowercase());
    cand
}

fn backup_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "session.maguro".into());
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut cand = dir.join(format!("{name}.bak"));
    let mut n = 2u32;
    while cand.exists() {
        cand = dir.join(format!("{name}.bak{n}"));
        n += 1;
        if n > 5000 {
            break;
        }
    }
    cand
}

fn land(tmp: &Path, dest: &Path) -> bool {
    if std::fs::rename(tmp, dest).is_ok() {
        return true;
    }
    if std::fs::copy(tmp, dest).is_ok() {
        let _ = std::fs::remove_file(tmp);
        return dest.is_file();
    }
    false
}

/// Split `path` when it stores more than one tab. A single-tab file, and anything that is not a session, stay put.
pub(crate) fn migrate_session_file(path: &Path) -> Option<SplitDone> {
    if !is_session_file(path) || !path.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    if !looks_like_session(&text) {
        return None;
    }
    let session = Session::parse(&text);
    let tabs = session.view_tabs();
    if tabs.len() < 2 {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let dir = path.parent()?.to_path_buf();
    let mut reserved = HashSet::new();
    // The file we are about to move aside must not count as taking its own name, and the new
    // names are `Name (1)` rather than the original stem.
    let names: Vec<String> = (0..tabs.len()).map(|i| free_stem(&dir, &format!("{stem} ({})", i + 1), &mut reserved)).collect();
    let mut temps = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let body = tab_session_text(&session, i)?;
        let tmp = dir.join(format!(".{name}.{}.split.tmp", std::process::id()));
        if std::fs::write(&tmp, body.as_bytes()).is_err() {
            for t in &temps {
                let _ = std::fs::remove_file(t);
            }
            return None;
        }
        temps.push(tmp);
    }
    let backup = backup_path(path);
    if std::fs::rename(path, &backup).is_err() {
        for t in &temps {
            let _ = std::fs::remove_file(t);
        }
        return None;
    }
    let mut files = Vec::new();
    let mut failed = false;
    for (tmp, name) in temps.iter().zip(&names) {
        let dest = dir.join(format!("{name}.maguro"));
        if land(tmp, &dest) {
            files.push(dest);
        } else {
            failed = true;
            let _ = std::fs::remove_file(tmp);
        }
    }
    if failed || files.len() != names.len() {
        for f in &files {
            let _ = std::fs::remove_file(f);
        }
        if !path.exists() {
            let _ = std::fs::rename(&backup, path);
        }
        return None;
    }
    let active = session.tab.min(files.len() - 1);
    let done = SplitDone { old: path.to_path_buf(), backup, files, active };
    let map = PathMap { from: done.old.to_string_lossy().into_owned(), to: done.files[active].to_string_lossy().into_owned() };
    let _ = append_split_map(&dir, &map);
    Some(done)
}

/// Every multi-tab session directly in `dir`. Already-split files are left alone, so a second run changes nothing.
pub(crate) fn migrate_sessions_in(dir: &Path) -> Vec<SplitDone> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_session_file(p))
        .collect();
    files.sort();
    files.into_iter().filter_map(|p| migrate_session_file(&p)).collect()
}

/// Split saved sessions, then the full old→new map (this run and earlier ones).
/// The map is what startup uses to retarget links. The window file itself is not written here.
pub(crate) fn migrate_named_sessions() -> Vec<PathMap> {
    let dir = sessions_dir();
    let _ = migrate_sessions_in(&dir);
    read_split_map(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pg-split-{}-{}-{name}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn two_tab_session() -> Session {
        let mut s = Session::default();
        s.set_heights(vec![1, 1]);
        s.paths[0] = r"C:\Work".into();
        s.paths[1] = r"D:\Photos".into();
        s.views[0] = (4, 16);
        s.cols[0] = "name;".into();
        s.tints[1] = 3;
        s.anchor();
        s.tabs[0].name = "Work".into();
        s.tabs[0].color = Some(0xC4_4B_2A);
        s.tabs[0].filters[0] = "*.pdf".into();
        s.tabs[0].locked = true;
        let mut other = Session::default();
        other.set_heights(vec![2]);
        other.paths[0] = r"E:\Notes".into();
        other.paths[1] = r"E:\Notes\old".into();
        other.tints[0] = 5;
        let mut second = Tab::from_live(&other, "Notes".into(), false, vec![String::new(); crate::MAXP]);
        second.color = Some(0x2A_6A_C4);
        s.tabs.push(second);
        s.tab = 1;
        s.publish();
        s
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.retain(|n| n != MAP_FILE && !n.ends_with(".split.tmp"));
        v.sort();
        v
    }

    #[test]
    fn multi_tab_session_splits_once_and_keeps_a_backup() {
        let d = scratch("basic");
        let src = two_tab_session();
        let original = src.to_ini();
        std::fs::write(d.join("Work.maguro"), &original).unwrap();
        std::fs::write(d.join("Solo.maguro"), "cols=1\np0=C:\\only\n").unwrap();
        let solo_before = std::fs::read(d.join("Solo.maguro")).unwrap();

        let done = migrate_sessions_in(&d);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].files.len(), 2);
        assert_eq!(done[0].active, 1);
        assert!(done[0].backup.ends_with("Work.maguro.bak"));
        assert_eq!(std::fs::read_to_string(&done[0].backup).unwrap(), original);
        assert!(!d.join("Work.maguro").exists());

        let stems: Vec<String> = done[0].files.iter().map(|p| p.file_stem().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(stems, vec!["Work (1)".to_string(), "Work (2)".into()]);

        let first = Session::parse(&std::fs::read_to_string(&done[0].files[0]).unwrap());
        let first_tabs = first.view_tabs();
        assert_eq!(first_tabs.len(), 1);
        assert_eq!(first_tabs[0].name, "Work");
        assert!(first_tabs[0].locked);
        assert_eq!(first_tabs[0].color, Some(0xC4_4B_2A));
        assert_eq!(first_tabs[0].paths[0], r"C:\Work");
        assert_eq!(first_tabs[0].paths[1], r"D:\Photos");
        assert_eq!(first_tabs[0].tints[1], 3);
        assert_eq!(first_tabs[0].filters[0], "*.pdf");
        assert_eq!(first_tabs[0].views[0], (4, 16));
        assert_eq!(first_tabs[0].heights, vec![1, 1]);

        let second = Session::parse(&std::fs::read_to_string(&done[0].files[1]).unwrap());
        let second_tabs = second.view_tabs();
        assert_eq!(second_tabs.len(), 1);
        assert_eq!(second_tabs[0].name, "Notes");
        assert_eq!(second_tabs[0].color, Some(0x2A_6A_C4));
        assert_eq!(second_tabs[0].heights, vec![2]);
        assert_eq!(second_tabs[0].paths[0], r"E:\Notes");
        assert_eq!(second_tabs[0].paths[1], r"E:\Notes\old");
        assert_eq!(second_tabs[0].tints[0], 5);

        assert_eq!(std::fs::read(d.join("Solo.maguro")).unwrap(), solo_before);
        assert!(!d.join("Solo.maguro.bak").exists());

        let again = names(&d);
        assert!(migrate_sessions_in(&d).is_empty());
        assert_eq!(names(&d), again);
        assert_eq!(std::fs::read_to_string(&done[0].backup).unwrap(), original);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn split_does_not_overwrite_an_existing_name() {
        let d = scratch("clash");
        std::fs::write(d.join("Work (1).maguro"), b"cols=1\np0=C:\\keep\n").unwrap();
        let keep = std::fs::read(d.join("Work (1).maguro")).unwrap();
        std::fs::write(d.join("old (1).maguro"), b"cols=1\np0=C:\\taken\n").unwrap();
        let taken = std::fs::read(d.join("old (1).maguro")).unwrap();
        let src = two_tab_session();
        std::fs::write(d.join("old.maguro"), src.to_ini()).unwrap();
        let done = migrate_session_file(&d.join("old.maguro")).unwrap();
        assert_eq!(std::fs::read(d.join("Work (1).maguro")).unwrap(), keep);
        assert_eq!(std::fs::read(d.join("old (1).maguro")).unwrap(), taken);
        assert!(done.backup.ends_with("old.maguro.bak"));
        assert_eq!(std::fs::read_to_string(&done.backup).unwrap(), src.to_ini());
        assert!(!d.join("old.maguro").exists());
        let stems: Vec<String> = done.files.iter().map(|p| p.file_stem().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(stems[0], "old (1) (2)");
        assert_eq!(stems[1], "old (2)");
        assert_eq!(Session::parse(&std::fs::read_to_string(&done.files[1]).unwrap()).view_tabs()[0].name, "Notes");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unreadable_and_foreign_files_stay() {
        let d = scratch("skip");
        std::fs::write(d.join("bad.maguro"), [0xFFu8, 0xFE, 0x80, 0x81]).unwrap();
        std::fs::write(d.join("note.maguro"), b"hello from somewhere else").unwrap();
        std::fs::write(d.join("keep.txt"), b"lang=en\n").unwrap();
        assert!(migrate_sessions_in(&d).is_empty());
        assert_eq!(std::fs::read(d.join("bad.maguro")).unwrap(), vec![0xFF, 0xFE, 0x80, 0x81]);
        assert_eq!(std::fs::read(d.join("note.maguro")).unwrap(), b"hello from somewhere else");
        assert_eq!(std::fs::read(d.join("keep.txt")).unwrap(), b"lang=en\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn split_leaves_a_sibling_folder_and_a_text_file_alone() {
        let root = scratch("away");
        let sessions = root.join("maguro-sessions");
        let other = root.join("other");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("theme.txt"), b"theme=dark\n").unwrap();
        std::fs::write(root.join("keep.txt"), b"lang=ko\n").unwrap();
        std::fs::write(sessions.join("Work.maguro"), two_tab_session().to_ini()).unwrap();
        assert_eq!(migrate_sessions_in(&sessions).len(), 1);
        assert_eq!(std::fs::read(other.join("theme.txt")).unwrap(), b"theme=dark\n");
        assert_eq!(std::fs::read(root.join("keep.txt")).unwrap(), b"lang=ko\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn startup_window_tabs_keep_their_folders_when_a_link_moves() {
        let mut s = two_tab_session();
        s.file = r"C:\s\Work.maguro".into();
        s.tabs[1].session = r"C:\s\Work.maguro".into();
        s.boot_file = r"c:/s/Work.maguro".into();
        let before = s.view_tabs();
        let maps = vec![PathMap { from: r"C:\s\Work.maguro".into(), to: r"C:\s\Work (2).maguro".into() }];
        retarget_session_links(&mut s, &maps);
        let after = s.view_tabs();
        assert_eq!(after.len(), before.len());
        assert_eq!(after[0].paths, before[0].paths);
        assert_eq!(after[1].paths, before[1].paths);
        assert_eq!(after[0].heights, before[0].heights);
        assert_eq!(after[1].name, "Notes");
        assert_eq!(after[1].color, before[1].color);
        assert_eq!(s.tabs[1].session, r"C:\s\Work (2).maguro");
        assert_eq!(s.tabs[0].session, "");
        assert_eq!(s.file, r"C:\s\Work (2).maguro");
        assert_eq!(s.boot_file, r"C:\s\Work (2).maguro");
        assert_eq!(s.tab, 1);

        let mut win = serde_json::json!({
            "session": r"C:\s\Work.maguro",
            "tabs": [
                {"name": "A", "layout": {"columns": [1, 1]}, "panes": [{"index": 0, "path": "C:\\Keep"}]},
                {"name": "B", "active": true, "session": r"C:\s\Work.maguro", "layout": {"columns": [2]}, "panes": [{"index": 0, "path": "D:\\Other"}, {"index": 1, "path": "E:\\More"}]}
            ]
        });
        retarget_window_value(&mut win, &maps);
        assert_eq!(win["session"], r"C:\s\Work (2).maguro");
        assert_eq!(win["tabs"][1]["session"], r"C:\s\Work (2).maguro");
        assert!(win["tabs"][0].get("session").is_none());
        assert_eq!(win["tabs"][0]["panes"][0]["path"], r"C:\Keep");
        assert_eq!(win["tabs"][1]["panes"][0]["path"], r"D:\Other");
        assert_eq!(win["tabs"][1]["panes"][1]["path"], r"E:\More");
        assert_eq!(win["tabs"][1]["layout"]["columns"][0], 2);
    }

    #[test]
    fn opened_sessions_land_beside_the_current_tab() {
        let saved = two_tab_session();
        let text = tab_session_text(&saved, 1).unwrap();
        let incoming = tabs_from_session_text(&text, r"C:\s\Notes.maguro");
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].session, r"C:\s\Notes.maguro");
        assert_eq!(incoming[0].name, "Notes");
        assert_eq!(incoming[0].paths[0], r"E:\Notes");
        assert_eq!(incoming[0].tints[0], 5);
        assert_eq!(incoming[0].color, Some(0x2A_6A_C4));

        let mut open = saved.clone();
        open.tabs[0].session = r"C:\s\a.maguro".into();
        open.tab = 0;
        let (tabs, at) = tabs_after_open(&open.tabs, 0, &incoming);
        assert_eq!(at, 1);
        assert_eq!(tabs.len(), 3);
        assert_eq!(tabs[0].session, r"C:\s\a.maguro");
        assert_eq!(tabs[0].paths[0], r"C:\Work");
        assert_eq!(tabs[0].tints[1], 3);
        assert_eq!(tabs[1].name, "Notes");
        assert_eq!(tabs[1].session, r"C:\s\Notes.maguro");
        assert_eq!(tabs[2].paths[0], r"E:\Notes");
        assert_eq!(tabs[2].session, "");

        let both = tabs_from_session_text(&saved.to_ini(), r"C:\s\Work.maguro");
        assert_eq!(both.len(), 2);
        assert!(both[0].session.is_empty());
        assert_eq!(both[1].session, r"C:\s\Work.maguro");
        assert_eq!(both[0].paths[0], r"C:\Work");
        assert_eq!(both[1].paths[0], r"E:\Notes");
        let (tabs, at) = tabs_after_open(&open.tabs, 0, &both);
        assert_eq!((at, tabs.len()), (1, 4));
        assert_eq!(tabs[0].paths[0], r"C:\Work");
        assert_eq!(tabs[0].session, r"C:\s\a.maguro");
    }
}
