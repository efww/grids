//! Clipboard text into a folder to open in a new tab, or a file to hand to the shell.
//! No Win32 here: environment expansion and the file-or-folder check are supplied by the caller.

/// What to open once a path is known to exist, or known to be a share we must not stat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClipPlan {
    Folder(String),
    /// Existing file. The caller opens it with the shell. It is not a Maguro tab.
    File(String),
}

/// Result of looking at the path. `Skip` is a UNC or mapped drive: do not stat it on the UI thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Probe {
    Missing,
    Dir,
    File,
    Skip,
}

/// `drop_item` wins when it normalizes, even if that path is later missing. Otherwise the text.
pub(crate) fn choose_clip(drop_item: Option<&str>, text: Option<&str>, expand: impl Fn(&str) -> String) -> Option<String> {
    if let Some(item) = drop_item.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(path) = normalize_clip_text(item, &expand) {
            return Some(path);
        }
    }
    text.and_then(|t| normalize_clip_text(t, &expand))
}

/// First non-empty line, one layer of matching quotes, trim, `file:` URL, env expand, then a Windows path.
pub(crate) fn normalize_clip_text(raw: &str, expand: impl Fn(&str) -> String) -> Option<String> {
    let line = first_line(raw)?;
    let bare = strip_quotes(line).trim();
    if bare.is_empty() {
        return None;
    }
    let decoded = if is_file_url(bare) { parse_file_url(bare)? } else { bare.to_string() };
    let expanded = expand(decoded.trim());
    let slashed = expanded.replace('/', "\\");
    let rooted = bare_drive(&slashed);
    let tidy = strip_trailing(&rooted);
    win_path(&tidy).then_some(tidy)
}

/// Folder, file-in-parent, or nothing. A missing local path is a miss, not its nearest parent.
pub(crate) fn plan(path: &str, probe: Probe) -> Option<ClipPlan> {
    if !win_path(path) {
        return None;
    }
    match probe {
        Probe::Missing => None,
        Probe::Dir | Probe::Skip => Some(ClipPlan::Folder(path.to_string())),
        Probe::File => {
            // Parent must be a real folder path. `\\srv\a.txt` is not a file on a share.
            split_file(path)?;
            Some(ClipPlan::File(path.to_string()))
        }
    }
}

fn first_line(raw: &str) -> Option<&str> {
    raw.split(['\n', '\r']).map(str::trim).find(|s| !s.is_empty())
}

fn strip_quotes(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn is_file_url(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 5 && b[..5].eq_ignore_ascii_case(b"file:")
}

fn parse_file_url(raw: &str) -> Option<String> {
    let rest = &raw[5..];
    if !rest.starts_with("//") {
        return None;
    }
    let decoded = percent_decode(rest);
    let after = decoded.trim_start_matches(['/', '\\']);
    if after.is_empty() {
        return None;
    }
    if drive_prefix(after) {
        return Some(drive_form(after));
    }
    let (host, path) = after.split_once(['/', '\\']).unwrap_or((after, ""));
    if host.eq_ignore_ascii_case("localhost") {
        let path = path.trim_start_matches(['/', '\\']);
        return drive_prefix(path).then(|| drive_form(path));
    }
    if host.is_empty() || host.contains(':') {
        return None;
    }
    let path = path.trim_start_matches(['/', '\\']).replace('/', "\\");
    let unc = if path.is_empty() { format!(r"\\{host}") } else { format!(r"\\{host}\{path}") };
    Some(unc)
}

fn drive_prefix(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && (b[1] == b':' || b[1] == b'|')
}

fn drive_form(path: &str) -> String {
    let b = path.as_bytes();
    let letter = (b[0] as char).to_ascii_uppercase();
    let rest = path[2..].trim_start_matches(['/', '\\']).replace('/', "\\");
    if rest.is_empty() {
        format!("{letter}:\\")
    } else {
        format!("{letter}:\\{rest}")
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn bare_drive(p: &str) -> String {
    let b = p.as_bytes();
    if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        format!("{}:\\", (b[0] as char).to_ascii_uppercase())
    } else if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        let mut s = String::new();
        s.push((b[0] as char).to_ascii_uppercase());
        s.push_str(&p[1..]);
        s
    } else {
        p.to_string()
    }
}

fn strip_trailing(p: &str) -> String {
    if p.len() > 3 && p.ends_with('\\') {
        let t = p.trim_end_matches('\\');
        if win_path(t) {
            return t.to_string();
        }
    }
    p.to_string()
}

fn win_path(p: &str) -> bool {
    let b = p.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        return true;
    }
    if b.len() < 5 || b[0] != b'\\' || b[1] != b'\\' {
        return false;
    }
    let mut parts = p[2..].split('\\').filter(|s| !s.is_empty());
    parts.next().is_some() && parts.next().is_some()
}

fn split_file(path: &str) -> Option<(String, String)> {
    let (folder, name) = path.rsplit_once('\\')?;
    if name.is_empty() {
        return None;
    }
    let folder = if folder.ends_with(':') { format!("{folder}\\") } else { folder.to_string() };
    if !win_path(&folder) {
        return None;
    }
    Some((folder, name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> String {
        s.to_string()
    }

    fn expand_user(s: &str) -> String {
        s.replace("%USERPROFILE%", r"C:\Users\me")
    }

    fn norm(raw: &str) -> Option<String> {
        normalize_clip_text(raw, id)
    }

    #[test]
    fn quotes_whitespace_env_and_slashes() {
        assert_eq!(norm(r#"  "C:\Work"  "#).as_deref(), Some(r"C:\Work"));
        assert_eq!(norm("  'D:\\Archive\\'  ").as_deref(), Some(r"D:\Archive"));
        assert_eq!(norm("c:/users/public").as_deref(), Some(r"C:\users\public"));
        assert_eq!(norm("C:").as_deref(), Some(r"C:\"));
        assert_eq!(normalize_clip_text("%USERPROFILE%\\Desktop", expand_user).as_deref(), Some(r"C:\Users\me\Desktop"));
        assert_eq!(normalize_clip_text("\"%USERPROFILE%/Desktop\"", expand_user).as_deref(), Some(r"C:\Users\me\Desktop"));
        assert_eq!(norm("\n\n  C:\\One\r\nC:\\Two").as_deref(), Some(r"C:\One"));
    }

    #[test]
    fn file_urls() {
        assert_eq!(norm("file:///C:/Users").as_deref(), Some(r"C:\Users"));
        assert_eq!(norm("FILE:///C:/Users/a%20b.txt").as_deref(), Some(r"C:\Users\a b.txt"));
        assert_eq!(norm("file://localhost/C:/Users").as_deref(), Some(r"C:\Users"));
        assert_eq!(norm("file://C:/Users").as_deref(), Some(r"C:\Users"));
        assert_eq!(norm("file:///C|/Users/").as_deref(), Some(r"C:\Users"));
        assert_eq!(norm("file://server/share/a").as_deref(), Some(r"\\server\share\a"));
        assert_eq!(norm("file://server/share").as_deref(), Some(r"\\server\share"));
        assert_eq!(norm(r#""file:///D:/Work""#).as_deref(), Some(r"D:\Work"));
    }

    #[test]
    fn rejects_garbage_and_a_bare_server() {
        assert_eq!(norm("hello"), None);
        assert_eq!(norm(""), None);
        assert_eq!(norm("   "), None);
        assert_eq!(norm(r"\\srv"), None);
        assert_eq!(norm("file:///not-a-path"), None);
        assert_eq!(norm("https://example.com/a"), None);
    }

    #[test]
    fn plan_folder_file_and_missing() {
        assert_eq!(plan(r"C:\Work", Probe::Dir), Some(ClipPlan::Folder(r"C:\Work".into())));
        assert_eq!(plan(r"\\srv\share", Probe::Skip), Some(ClipPlan::Folder(r"\\srv\share".into())));
        assert_eq!(plan(r"C:\Work", Probe::Missing), None);
        assert_eq!(plan(r"C:\Work\note.txt", Probe::File), Some(ClipPlan::File(r"C:\Work\note.txt".into())));
        assert_eq!(plan(r"C:\note.txt", Probe::File), Some(ClipPlan::File(r"C:\note.txt".into())));
        assert_eq!(plan(r"\\srv\share\a.txt", Probe::File), Some(ClipPlan::File(r"\\srv\share\a.txt".into())));
        // Parent would be `\\srv`, which is not a share.
        assert_eq!(plan(r"\\srv\a.txt", Probe::File), None);
        assert_eq!(plan(r"\\srv", Probe::Dir), None);
    }

    #[test]
    fn hdrop_wins_when_it_normalizes() {
        let path = choose_clip(Some(r"C:\FromDrop"), Some(r"D:\FromText"), id);
        assert_eq!(path.as_deref(), Some(r"C:\FromDrop"));
        // A drop that is not a path falls through to the text.
        let path = choose_clip(Some("not a path"), Some(r"D:\FromText"), id);
        assert_eq!(path.as_deref(), Some(r"D:\FromText"));
        let path = choose_clip(Some("   "), Some(r"D:\FromText"), id);
        assert_eq!(path.as_deref(), Some(r"D:\FromText"));
        assert_eq!(choose_clip(None, Some("hello"), id), None);
        // The drop is kept even when a later probe says it is missing.
        let kept = choose_clip(Some(r"C:\Gone"), Some(r"D:\Other"), id).unwrap();
        assert_eq!(plan(&kept, Probe::Missing), None);
    }
}
