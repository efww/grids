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

/// Fold a `ReadDirectoryChangesW` buffer into change kinds. A rename pair counts once.
pub(crate) fn parse_notify(buf: &[u8], got: usize) -> Vec<ChangeKind> {
    let got = got.min(buf.len());
    let mut out = Vec::new();
    let mut off = 0usize;
    let mut skip_new_name = false;
    while off + 12 <= got {
        let next = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
        let action = u32::from_le_bytes(buf[off + 4..off + 8].try_into().unwrap());
        let name_bytes = u32::from_le_bytes(buf[off + 8..off + 12].try_into().unwrap()) as usize;
        if off + 12 + name_bytes > got {
            break;
        }
        if skip_new_name && action == 5 {
            skip_new_name = false;
        } else {
            let kind = ChangeKind::from_action(action);
            skip_new_name = action == 4;
            out.push(kind);
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
            book.rows.push(Row {
                id,
                path,
                subfolders: row.subfolders,
                paused: row.paused,
                status: if row.paused { Status::Paused } else { Status::Watching },
                last_ms: row.last_ms.filter(|ms| *ms > 0),
                last_kind: kind,
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
        }]);
        let text = serde_json::to_string(&bare.to_json()).unwrap();
        assert!(!text.contains("subfolders"), "{text}");
        assert!(!text.contains("paused"), "{text}");
        assert_eq!(bare.rows()[0].path, r"D:\Pics");
        assert!(bare.rows()[0].subfolders);
        assert_eq!(bare.rows()[0].status, Status::Watching);
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
