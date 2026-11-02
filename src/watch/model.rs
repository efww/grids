// Folder Watch entries, the burst debounce, and the notification route.
// A portable exe has no AppUserModelID. Creating one would write the registry, which Maguro does not do.

use serde::{Deserialize, Serialize};

use crate::{is_fs, Key, CERULEAN_BRAND, OLD_MINT_BRAND};

/// Quiet time before a burst becomes one notification, and the longest a burst may run.
pub(crate) const QUIET_MS: u64 = 1500;
pub(crate) const BURST_CAP_MS: u64 = 2000;
pub(crate) const MAX_WATCHES: usize = 32;

/// Cerulean brand. The retired mint value is never used for this window.
pub(crate) const CHROME: u32 = CERULEAN_BRAND;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Added,
    Removed,
    Modified,
    Renamed,
}

impl ChangeKind {
    pub(crate) fn tag(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Removed => "removed",
            ChangeKind::Modified => "modified",
            ChangeKind::Renamed => "renamed",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "added" => Some(ChangeKind::Added),
            "removed" => Some(ChangeKind::Removed),
            "modified" => Some(ChangeKind::Modified),
            "renamed" => Some(ChangeKind::Renamed),
            _ => None,
        }
    }

    pub(crate) fn key(self) -> Key {
        match self {
            ChangeKind::Added => Key::ChangeAdded,
            ChangeKind::Removed => Key::ChangeRemoved,
            ChangeKind::Modified => Key::ChangeModified,
            ChangeKind::Renamed => Key::ChangeRenamed,
        }
    }

    fn from_action(action: u32) -> Self {
        match action {
            1 => ChangeKind::Added,
            2 => ChangeKind::Removed,
            4 | 5 => ChangeKind::Renamed,
            _ => ChangeKind::Modified,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Watching,
    Paused,
    Missing,
    Failed,
}

impl Status {
    pub(crate) fn key(self) -> Key {
        match self {
            Status::Watching => Key::WatchWatching,
            Status::Paused => Key::WatchPaused,
            Status::Missing => Key::WatchMissing,
            Status::Failed => Key::WatchFailed,
        }
    }
}

/// One burst of directory events. `started_ms` and `last_ms` are a monotonic clock, not wall time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Burst {
    pub started_ms: u64,
    pub last_ms: u64,
    pub kind: ChangeKind,
    pub count: u32,
}

impl Burst {
    pub(crate) fn start(now_ms: u64, kind: ChangeKind) -> Self {
        Burst { started_ms: now_ms, last_ms: now_ms, kind, count: 1 }
    }

    pub(crate) fn push(&mut self, now_ms: u64, kind: ChangeKind) {
        // A rename is reported as the old name and then the new one. That is one change.
        let pair = self.kind == ChangeKind::Renamed && kind == ChangeKind::Renamed;
        if !pair {
            self.count = self.count.saturating_add(1);
        }
        self.kind = kind;
        self.last_ms = now_ms;
    }

    pub(crate) fn due(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_ms) >= QUIET_MS || now_ms.saturating_sub(self.started_ms) >= BURST_CAP_MS
    }

    /// Milliseconds until [`due`]. `None` when it is already due.
    pub(crate) fn wait_ms(&self, now_ms: u64) -> Option<u64> {
        if self.due(now_ms) {
            None
        } else {
            let quiet = QUIET_MS - now_ms.saturating_sub(self.last_ms);
            let cap = BURST_CAP_MS - now_ms.saturating_sub(self.started_ms);
            Some(quiet.min(cap).max(1))
        }
    }
}

/// One name from a `ReadDirectoryChangesW` buffer. A rename keeps the new name in `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NamedChange {
    pub kind: ChangeKind,
    pub name: String,
    pub to: Option<String>,
}

/// What a file watch should do with one event about that file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileHit {
    pub kind: ChangeKind,
    pub gone: bool,
    pub renamed_to: Option<String>,
}

/// Status to store when a burst is shown. A rename follows the file to its new name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Follow {
    Watching,
    Missing,
    Renamed(String),
}

/// Fold a `ReadDirectoryChangesW` buffer into named changes. A rename pair counts once.
pub(crate) fn parse_named(buf: &[u8], got: usize) -> Vec<NamedChange> {
    let got = got.min(buf.len());
    let mut out: Vec<NamedChange> = Vec::new();
    let mut off = 0usize;
    let mut skip_new_name = false;
    while off + 12 <= got {
        let next = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        let action = u32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap());
        let name_bytes = u32::from_le_bytes(buf[off + 8..off + 12].try_into().unwrap()) as usize;
        if off + 12 + name_bytes > got {
            break;
        }
        let name = utf16_name(&buf[off + 12..off + 12 + name_bytes]);
        if skip_new_name && action == 5 {
            if let Some(prev) = out.last_mut() {
                if prev.kind == ChangeKind::Renamed && prev.to.is_none() {
                    prev.to = Some(name);
                }
            }
            skip_new_name = false;
        } else {
            let kind = ChangeKind::from_action(action);
            skip_new_name = action == 4;
            out.push(NamedChange { kind, name, to: None });
        }
        if next == 0 {
            break;
        }
        let next_off = off.saturating_add(next);
        if next_off <= off || next_off > got {
            break;
        }
        off = next_off;
    }
    out
}

/// Fold a `ReadDirectoryChangesW` buffer into change kinds. A rename pair counts once.
pub(crate) fn parse_notify(buf: &[u8], got: usize) -> Vec<ChangeKind> {
    parse_named(buf, got).into_iter().map(|n| n.kind).collect()
}

fn utf16_name(bytes: &[u8]) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let u = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if u == 0 {
            break;
        }
        units.push(u);
        i += 2;
    }
    String::from_utf16_lossy(&units)
}

/// Keep modified, renamed, added, and removed events for `file_name` only.
/// Other names in the same directory are dropped. A delete sets `gone`.
pub(crate) fn filter_file(file_name: &str, events: &[NamedChange]) -> Vec<FileHit> {
    let mut out = Vec::new();
    for ev in events {
        if same_leaf(&ev.name, file_name) {
            match ev.kind {
                ChangeKind::Removed => out.push(FileHit { kind: ChangeKind::Removed, gone: true, renamed_to: None }),
                ChangeKind::Renamed => {
                    if let Some(to) = ev.to.as_deref() {
                        let leaf = leaf_name(to);
                        out.push(FileHit { kind: ChangeKind::Renamed, gone: false, renamed_to: Some(leaf) });
                    } else {
                        out.push(FileHit { kind: ChangeKind::Renamed, gone: true, renamed_to: None });
                    }
                }
                kind => out.push(FileHit { kind, gone: false, renamed_to: None }),
            }
        } else if ev.kind == ChangeKind::Renamed {
            if ev.to.as_deref().is_some_and(|to| same_leaf(to, file_name)) {
                out.push(FileHit { kind: ChangeKind::Renamed, gone: false, renamed_to: None });
            }
        }
    }
    out
}

pub(crate) fn leaf_name(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    t.rsplit(['\\', '/']).next().filter(|s| !s.is_empty()).unwrap_or(path).to_string()
}

pub(crate) fn parent_of(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    let Some(i) = t.rfind('\\') else { return t.to_string() };
    if i == 2 && t.as_bytes().get(1) == Some(&b':') {
        return format!("{}\\", &t[..2]);
    }
    if i == 0 {
        return t.to_string();
    }
    t[..i].to_string()
}

pub(crate) fn join_under(dir: &str, name: &str) -> String {
    let name = name.trim_matches(['\\', '/']);
    let dir = dir.trim_end_matches(['\\', '/']);
    if dir.len() == 2 && dir.as_bytes().get(1) == Some(&b':') {
        format!("{dir}\\{name}")
    } else {
        format!("{dir}\\{name}")
    }
}

fn same_leaf(event_name: &str, want: &str) -> bool {
    leaf_name(event_name).eq_ignore_ascii_case(&leaf_name(want))
}

/// Text colour for a watch status. Cerulean stays off this column.
pub(crate) fn status_color(status: Status, dark: bool) -> u32 {
    match status {
        Status::Watching => {
            if dark {
                0x3DDB8A
            } else {
                0x1FA463
            }
        }
        Status::Paused => {
            if dark {
                0xE0B15A
            } else {
                0xA07820
            }
        }
        Status::Missing | Status::Failed => {
            if dark {
                0xF07167
            } else {
                0x9F1C14
            }
        }
    }
}

/// Push kinds into a burst, flushing first when the open burst is already due.
pub(crate) fn fold_burst(burst: &mut Option<Burst>, now_ms: u64, kinds: &[ChangeKind]) -> Vec<Burst> {
    let mut flushed = Vec::new();
    for kind in kinds {
        if burst.as_ref().is_some_and(|b| b.due(now_ms)) {
            if let Some(done) = burst.take() {
                flushed.push(done);
            }
        }
        if let Some(b) = burst.as_mut() {
            b.push(now_ms, *kind);
        } else {
            *burst = Some(Burst::start(now_ms, *kind));
        }
    }
    flushed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotifyRoute {
    /// WinRT toast. Needs an AppUserModelID that a portable exe does not have.
    Toast,
    /// `IUserNotification2` balloon. No registry write and no lasting tray icon.
    Balloon,
}

/// `aumid_without_registry` is never true here: Maguro does not write the registry to invent one.
pub(crate) fn notify_route(aumid_without_registry: bool) -> NotifyRoute {
    if aumid_without_registry {
        NotifyRoute::Toast
    } else {
        NotifyRoute::Balloon
    }
}

/// Filesystem folder, without a trailing slash except a drive root. Shell places are refused.
pub(crate) fn canon_path(path: &str) -> Option<String> {
    let t = path.trim();
    if t.is_empty() || t.len() > 1024 {
        return None;
    }
    let mut s = t.replace('/', "\\");
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        s = format!(r"\\{rest}");
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    if s.len() >= 2 && s.as_bytes().get(1) == Some(&b':') && s.len() == 2 {
        s.push('\\');
    } else if !(s.len() == 3 && s.as_bytes().get(1) == Some(&b':') && s.ends_with('\\')) {
        while s.len() > 3 && s.ends_with('\\') {
            s.pop();
        }
    }
    if !is_fs(&s) {
        return None;
    }
    Some(s)
}

pub(crate) fn same_path(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WatchJson {
    pub path: String,
    /// Missing in an older file means include subfolders.
    #[serde(default = "default_sub", skip_serializing_if = "is_true")]
    pub subfolders: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_kind: Option<String>,
    /// Missing in an older file means this entry is a folder.
    #[serde(default, skip_serializing_if = "is_false")]
    pub file: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_name: Option<String>,
}

fn default_sub() -> bool {
    true
}
fn is_true(v: &bool) -> bool {
    *v
}
fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AddFail {
    Bad,
    Dup,
    Limit,
}

#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub id: u64,
    pub path: String,
    pub subfolders: bool,
    pub paused: bool,
    pub status: Status,
    pub last_ms: Option<i64>,
    pub last_kind: Option<ChangeKind>,
    pub file: bool,
    pub last_name: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct Book {
    rows: Vec<Row>,
    next_id: u64,
}

impl Book {
    pub(crate) fn new() -> Self {
        Book { rows: Vec::new(), next_id: 1 }
    }

    pub(crate) fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub(crate) fn ids(&self) -> Vec<u64> {
        self.rows.iter().map(|r| r.id).collect()
    }

    pub(crate) fn get_mut(&mut self, id: u64) -> Option<&mut Row> {
        self.rows.iter_mut().find(|r| r.id == id)
    }

    pub(crate) fn add(&mut self, path: &str) -> Result<u64, AddFail> {
        let path = canon_path(path).ok_or(AddFail::Bad)?;
        if self.rows.iter().any(|r| same_path(&r.path, &path)) {
            return Err(AddFail::Dup);
        }
        if self.rows.len() >= MAX_WATCHES {
            return Err(AddFail::Limit);
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.rows.push(Row {
            id,
            path,
            subfolders: true,
            paused: false,
            status: Status::Watching,
            last_ms: None,
            last_kind: None,
            file: false,
            last_name: None,
        });
        Ok(id)
    }

    /// One file. Subfolders stay off: the parent directory is watched and filtered to this name.
    pub(crate) fn add_file(&mut self, path: &str) -> Result<u64, AddFail> {
        let path = canon_path(path).ok_or(AddFail::Bad)?;
        if self.rows.iter().any(|r| same_path(&r.path, &path)) {
            return Err(AddFail::Dup);
        }
        if self.rows.len() >= MAX_WATCHES {
            return Err(AddFail::Limit);
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.rows.push(Row {
            id,
            path,
            subfolders: false,
            paused: false,
            status: Status::Watching,
            last_ms: None,
            last_kind: None,
            file: true,
            last_name: None,
        });
        Ok(id)
    }

    pub(crate) fn remove(&mut self, id: u64) -> bool {
        let n = self.rows.len();
        self.rows.retain(|r| r.id != id);
        self.rows.len() != n
    }

    pub(crate) fn to_json(&self) -> Vec<WatchJson> {
        self.rows.iter().map(|r| WatchJson {
            path: r.path.clone(),
            subfolders: r.subfolders,
            paused: r.paused,
            last_ms: r.last_ms,
            last_kind: r.last_kind.map(ChangeKind::tag).map(str::to_string),
            file: r.file,
            last_name: r.last_name.clone(),
        }).collect()
    }

    pub(crate) fn from_json(rows: &[WatchJson]) -> Self {
        let mut book = Book::new();
        for row in rows.iter().take(MAX_WATCHES) {
            let Some(path) = canon_path(&row.path) else { continue };
            if book.rows.iter().any(|r| same_path(&r.path, &path)) {
                continue;
            }
            let id = book.next_id;
            book.next_id += 1;
            let kind = row.last_kind.as_deref().and_then(ChangeKind::parse);
            let file = row.file;
            book.rows.push(Row {
                id,
                path,
                subfolders: if file { false } else { row.subfolders },
                paused: row.paused,
                status: if row.paused { Status::Paused } else { Status::Watching },
                last_ms: row.last_ms.filter(|ms| *ms > 0),
                last_kind: kind,
                file,
                last_name: row.last_name.clone().filter(|s| !s.is_empty()),
            });
        }
        book
    }
}

pub(crate) fn format_civil(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> String {
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_is_cerulean_not_mint() {
        assert_eq!(CHROME, CERULEAN_BRAND);
        assert_eq!(CHROME, 0x0A9BD6);
        assert_ne!(CHROME, OLD_MINT_BRAND);
        assert_ne!(CHROME, 0x2EC4A6);
    }

    #[test]
    fn portable_exe_uses_the_balloon_not_a_toast() {
        assert_eq!(notify_route(false), NotifyRoute::Balloon);
        assert_eq!(notify_route(true), NotifyRoute::Toast);
    }

    #[test]
    fn paths_are_canonical_and_shell_places_are_refused() {
        assert_eq!(canon_path(r"  C:/Work/ ").as_deref(), Some(r"C:\Work"));
        assert_eq!(canon_path("C:").as_deref(), Some(r"C:\"));
        assert_eq!(canon_path(r"C:\").as_deref(), Some(r"C:\"));
        assert_eq!(canon_path(r"\\?\C:\Work\").as_deref(), Some(r"C:\Work"));
        assert_eq!(canon_path(r"\\?\UNC\server\share\").as_deref(), Some(r"\\server\share"));
        assert_eq!(canon_path(r"  \\server\share\a\ ").as_deref(), Some(r"\\server\share\a"));
        assert!(canon_path("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}").is_none());
        assert!(canon_path("").is_none());
        assert!(same_path(r"C:\Work", r"c:\work"));
    }

    #[test]
    fn a_burst_waits_out_the_quiet_time_then_caps() {
        let mut b = Burst::start(0, ChangeKind::Added);
        assert!(!b.due(1499));
        assert_eq!(b.wait_ms(0), Some(QUIET_MS));
        b.push(400, ChangeKind::Modified);
        assert!(!b.due(400 + QUIET_MS - 1));
        assert!(b.due(400 + QUIET_MS));
        // A stream that never goes quiet still flushes at the cap, measured from the first event.
        let mut stream = Burst::start(0, ChangeKind::Modified);
        stream.push(1800, ChangeKind::Removed);
        assert!(stream.due(BURST_CAP_MS));
        assert!(stream.wait_ms(BURST_CAP_MS).is_none());
        assert_eq!(stream.kind, ChangeKind::Removed);
        assert_eq!(stream.count, 2);
    }

    #[test]
    fn a_rename_pair_in_one_read_is_one_change() {
        let mut buf = Vec::new();
        push_entry(&mut buf, 4, "old.txt", true);
        push_entry(&mut buf, 5, "new.txt", false);
        push_entry(&mut buf, 1, "extra.txt", false);
        // The third entry is unreachable: the second wrote next = 0. Rebuild a chain.
        buf.clear();
        push_entry(&mut buf, 4, "old.txt", true);
        push_entry(&mut buf, 5, "new.txt", true);
        push_entry(&mut buf, 3, "extra.txt", false);
        let kinds = parse_notify(&buf, buf.len());
        assert_eq!(kinds, vec![ChangeKind::Renamed, ChangeKind::Modified]);
        let mut burst = None;
        let flushed = fold_burst(&mut burst, 10, &kinds);
        assert!(flushed.is_empty());
        let b = burst.unwrap();
        assert_eq!(b.count, 2);
        assert_eq!(b.kind, ChangeKind::Modified);
    }

    #[test]
    fn the_list_rejects_duplicates_and_round_trips() {
        let mut book = Book::new();
        let id = book.add(r"C:\Work").unwrap();
        assert_eq!(book.add(r"c:\work"), Err(AddFail::Dup));
        assert_eq!(book.add("not-a-path"), Err(AddFail::Bad));
        book.get_mut(id).unwrap().subfolders = false;
        book.get_mut(id).unwrap().paused = true;
        book.get_mut(id).unwrap().last_ms = Some(1_700_000_000_000);
        book.get_mut(id).unwrap().last_kind = Some(ChangeKind::Renamed);
        let json = book.to_json();
        let text = serde_json::to_string(&json).unwrap();
        assert!(text.contains("\"subfolders\":false"), "{text}");
        assert!(text.contains("\"paused\":true"), "{text}");
        assert!(text.contains("renamed"), "{text}");
        let back = Book::from_json(&serde_json::from_str::<Vec<WatchJson>>(&text).unwrap());
        let row = &back.rows()[0];
        assert_eq!(row.path, r"C:\Work");
        assert!(!row.subfolders);
        assert!(row.paused);
        assert_eq!(row.status, Status::Paused);
        assert_eq!(row.last_kind, Some(ChangeKind::Renamed));
        // Omitted subfolders means include them. Omitted paused means running.
        let bare = Book::from_json(&[WatchJson {
            path: r"D:\Pics\".into(),
            subfolders: true,
            paused: false,
            last_ms: None,
            last_kind: None,
            file: false,
            last_name: None,
        }]);
        let text = serde_json::to_string(&bare.to_json()).unwrap();
        assert!(!text.contains("subfolders"), "{text}");
        assert!(!text.contains("paused"), "{text}");
        assert_eq!(bare.rows()[0].path, r"D:\Pics");
        assert!(bare.rows()[0].subfolders);
        assert_eq!(bare.rows()[0].status, Status::Watching);
        assert!(!bare.rows()[0].file);
    }

    #[test]
    fn an_older_folder_file_loads_and_a_file_row_round_trips() {
        let old = r#"[{"path":"C:\\Work","paused":false}]"#;
        let rows: Vec<WatchJson> = serde_json::from_str(old).unwrap();
        assert!(!rows[0].file);
        assert!(rows[0].last_name.is_none());
        let book = Book::from_json(&rows);
        assert!(!book.rows()[0].file);
        assert!(book.rows()[0].subfolders);
        let text = serde_json::to_string(&book.to_json()).unwrap();
        assert!(!text.contains("\"file\""), "{text}");

        let mut files = Book::new();
        let id = files.add_file(r"C:\Work\a.txt").unwrap();
        assert!(files.rows()[0].file);
        assert!(!files.rows()[0].subfolders);
        assert_eq!(files.add_file(r"c:\work\a.txt"), Err(AddFail::Dup));
        assert!(files.add(r"C:\Work").is_ok());
        files.get_mut(id).unwrap().last_name = Some("a.txt".into());
        let saved = serde_json::to_string(&files.to_json()).unwrap();
        assert!(saved.contains("\"file\":true"), "{saved}");
        let back = Book::from_json(&serde_json::from_str::<Vec<WatchJson>>(&saved).unwrap());
        let file = back.rows().iter().find(|r| r.file).unwrap();
        assert_eq!(file.path, r"C:\Work\a.txt");
        assert!(!file.subfolders);
        assert_eq!(file.last_name.as_deref(), Some("a.txt"));
        assert_eq!(back.rows().iter().filter(|r| !r.file).count(), 1);
    }

    #[test]
    fn a_file_watch_ignores_other_names_and_follows_a_rename() {
        let events = vec![
            NamedChange { kind: ChangeKind::Modified, name: "other.txt".into(), to: None },
            NamedChange { kind: ChangeKind::Modified, name: "a.txt".into(), to: None },
            NamedChange { kind: ChangeKind::Removed, name: "A.TXT".into(), to: None },
            NamedChange { kind: ChangeKind::Renamed, name: "a.txt".into(), to: Some("b.txt".into()) },
            NamedChange { kind: ChangeKind::Added, name: r"sub\a.txt".into(), to: None },
        ];
        let hits = filter_file("a.txt", &events);
        assert_eq!(hits.len(), 4);
        assert_eq!(hits[0], FileHit { kind: ChangeKind::Modified, gone: false, renamed_to: None });
        assert!(hits[1].gone);
        assert_eq!(hits[1].kind, ChangeKind::Removed);
        assert_eq!(hits[2].renamed_to.as_deref(), Some("b.txt"));
        assert!(!hits[2].gone);
        assert_eq!(hits[3].kind, ChangeKind::Added);
        assert!(filter_file("a.txt", &[NamedChange { kind: ChangeKind::Modified, name: "notes.txt".into(), to: None }]).is_empty());
        let onto = filter_file("b.txt", &[NamedChange { kind: ChangeKind::Renamed, name: "a.txt".into(), to: Some("b.txt".into()) }]);
        assert_eq!(onto, vec![FileHit { kind: ChangeKind::Renamed, gone: false, renamed_to: None }]);
        assert_eq!(parent_of(r"C:\Work\a.txt"), r"C:\Work");
        assert_eq!(parent_of(r"C:\a.txt"), r"C:\");
        assert_eq!(join_under(r"C:\Work", "b.txt"), r"C:\Work\b.txt");
        assert_eq!(join_under(r"C:\", "b.txt"), r"C:\b.txt");
        assert_eq!(leaf_name(r"C:\Work\a.txt"), "a.txt");
    }

    #[test]
    fn status_colours_are_not_cerulean_or_mint() {
        for dark in [false, true] {
            for st in [Status::Watching, Status::Paused, Status::Missing, Status::Failed] {
                let c = status_color(st, dark);
                assert_ne!(c, CHROME);
                assert_ne!(c, CERULEAN_BRAND);
                assert_ne!(c, OLD_MINT_BRAND);
                assert_ne!(c, 0x2EC4A6);
            }
        }
        assert_eq!(status_color(Status::Watching, false), 0x1FA463);
        assert_eq!(status_color(Status::Missing, false), 0x9F1C14);
    }

    #[test]
    fn ctrl_shift_w_is_folder_watch_and_does_not_close_a_tab() {
        let watch = crate::keys::lookup(b'W' as u16, true, true, false, false).unwrap();
        let close = crate::keys::lookup(b'W' as u16, true, false, false, false).unwrap();
        let pane = crate::keys::lookup(b'W' as u16, true, false, true, false).unwrap();
        assert_eq!(crate::keys::id_of(watch), "watch.open");
        assert_eq!(crate::keys::chord_text(watch), "Ctrl+Shift+W");
        assert_eq!(crate::keys::id_of(close), "tab.close");
        assert_eq!(crate::keys::id_of(pane), "pane.tab.close");
    }

    fn push_entry(buf: &mut Vec<u8>, action: u32, name: &str, more: bool) {
        let wide: Vec<u16> = name.encode_utf16().collect();
        let name_bytes = wide.len() * 2;
        let raw = 12 + name_bytes;
        let size = (raw + 3) & !3;
        let start = buf.len();
        let take = if more { size } else { raw };
        buf.resize(start + take, 0);
        let next = if more { size as u32 } else { 0 };
        buf[start..start + 4].copy_from_slice(&next.to_le_bytes());
        buf[start + 4..start + 8].copy_from_slice(&action.to_le_bytes());
        buf[start + 8..start + 12].copy_from_slice(&(name_bytes as u32).to_le_bytes());
        for (i, unit) in wide.iter().enumerate() {
            let at = start + 12 + i * 2;
            buf[at..at + 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
}
