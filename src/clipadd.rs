use crate::*;
use std::collections::HashSet;
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;

// ⇄ menu "Add the selected items to the clipboard": the pane's selection joins the files already copied, so
// files from several panes go to a mail, a chat or one folder with a single Ctrl+V.
// After Directory Opus' "Clipboard ADD". The clipboard gets a plain file list (CF_HDROP) marked as a copy,
// the same form Total Commander puts there; Grids' own Ctrl+V and Explorer both paste it.

pub(crate) const ID_CLIPADD: usize = 1149; // ⇄ menu (1148 is the view menu's ID_DEFOPEN)

// The clipboard after an add: `list` in order (what was there first, then the new items), `added` of the new
// ones kept, `same_name` items whose file name another item in the list already has (from another folder).
#[derive(Debug, PartialEq)]
pub(crate) struct Merge {
    pub(crate) list: Vec<String>,
    pub(crate) added: usize,
    pub(crate) same_name: usize,
}

fn name_key(p: &str) -> String {
    let p = p.trim_end_matches('\\');
    p.rsplit('\\').next().unwrap_or(p).to_lowercase()
}

// Strings only, no disk call (10,000 paths or an unreachable share would stall the window). Paths count as the
// same without regard to case or a trailing '\'; an item inside a folder the list also holds goes with that
// folder (whichever came first), so a paste never copies it twice. None: more than `cap` items in all.
pub(crate) fn merge_clip(existing: &[String], add: &[String], cap: usize) -> Option<Merge> {
    let mut keys: HashSet<String> = HashSet::new();
    let mut all: Vec<(&str, bool)> = Vec::new(); // (path, new)
    for (p, new) in existing.iter().map(|p| (p, false)).chain(add.iter().map(|p| (p, true))) {
        let p = p.trim();
        if !p.is_empty() && keys.insert(dir_key(p)) {
            all.push((p, new));
        }
    }
    let inside = |p: &str| {
        let mut k = dir_key(p);
        while let Some((up, _)) = k.rsplit_once('\\') {
            if up.is_empty() {
                break;
            }
            if keys.contains(up) {
                return true;
            }
            k = up.to_string();
        }
        false
    };
    let kept: Vec<(&str, bool)> = all.into_iter().filter(|(p, _)| !inside(p)).collect();
    if kept.len() > cap {
        return None;
    }
    let mut names = HashSet::new();
    Some(Merge {
        added: kept.iter().filter(|(_, new)| *new).count(),
        same_name: kept.iter().filter(|(p, _)| !names.insert(name_key(p))).count(),
        list: kept.into_iter().map(|(p, _)| p.to_string()).collect(),
    })
}

// How many folders the items come from (a drive root counts as its own).
pub(crate) fn folder_count(list: &[String]) -> usize {
    let parent = |p: &String| {
        let k = dir_key(p);
        k.rsplit_once('\\').map_or(k.clone(), |(up, _)| up.to_string())
    };
    list.iter().map(parent).collect::<HashSet<_>>().len()
}

// The status line after an add.
pub(crate) fn clip_note(ko: bool, added: usize, total: usize, folders: usize, same: usize) -> String {
    let mut s = if ko {
        format!("클립보드에 {added}개 더함 — 모두 {total}개 · 폴더 {folders}곳 · Ctrl+V로 한 번에 붙여 넣기")
    } else {
        let items = if total == 1 { "item" } else { "items" };
        let dirs = if folders == 1 { "folder" } else { "folders" };
        format!("Added {added} to the clipboard — {total} {items} from {folders} {dirs} · Ctrl+V pastes them all")
    };
    if same > 0 {
        s.push_str(&if ko {
            format!(" · 같은 이름 {same}개: 한 폴더에 붙이면 덮어쓸지 물음")
        } else {
            let verb = if same == 1 { "shares" } else { "share" };
            format!(" · {same} {verb} a name: pasting into one folder asks before replacing")
        });
    }
    s
}

// A DROPFILES block: the 20-byte header (file list at byte 20, no point, wide characters), then each path in
// UTF-16 ending in a 0, then one more 0.
pub(crate) fn dropfiles_bytes(list: &[String]) -> Vec<u8> {
    let mut out = Vec::with_capacity(22 + list.iter().map(|p| p.len() * 2 + 2).sum::<usize>());
    for v in [20u32, 0, 0, 0, 1] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for p in list {
        for u in p.encode_utf16().chain([0]) {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
    out.extend_from_slice(&[0, 0]);
    out
}

// The ⇄ menu item: grey unless the whole selection has file paths (the same picked the copy / move items use).
pub(crate) unsafe fn clip_menu(menu: HMENU, picked: &std::result::Result<Vec<String>, Pick>) {
    let (flags, label) = match picked {
        Ok(p) => (MF_STRING, tr2(M_CLIPADD_N).replace("{n}", &p.len().to_string())),
        Err(_) => (MF_STRING | MF_GRAYED, tr2(M_CLIPADD).to_string()),
    };
    _ = AppendMenuW(menu, flags, ID_CLIPADD, &HSTRING::from(label));
}

enum Added {
    Done { added: usize, total: usize, folders: usize, same: usize },
    AllThere(usize),
    Cut,
    Virtual,
    TooMany,
    Failed,
}

unsafe fn global_bytes(b: &[u8]) -> Option<HGLOBAL> {
    let h = GlobalAlloc(GMEM_MOVEABLE, b.len()).ok()?;
    let p = GlobalLock(h) as *mut u8;
    if p.is_null() {
        _ = GlobalFree(Some(h));
        return None;
    }
    std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
    _ = GlobalUnlock(h);
    Some(h)
}

// With the clipboard open: read, merge and write in one go, so no other program's copy lands in between.
unsafe fn add_open(add: &[String]) -> Added {
    let mut existing = Vec::new();
    if let Ok(h) = GetClipboardData(CF_HDROP.0 as u32) {
        let drop = HDROP(h.0);
        for k in 0..DragQueryFileW(drop, u32::MAX, None) {
            let mut buf = vec![0u16; DragQueryFileW(drop, k, None) as usize + 1];
            let n = DragQueryFileW(drop, k, Some(&mut buf)) as usize;
            existing.push(String::from_utf16_lossy(&buf[..n.min(buf.len())]));
        }
    }
    let effect_fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
    let mut effect = None;
    if let Ok(h) = GetClipboardData(effect_fmt) {
        let p = GlobalLock(HGLOBAL(h.0)) as *const u32;
        if !p.is_null() {
            effect = Some(*p);
            _ = GlobalUnlock(HGLOBAL(h.0));
        }
    }
    if existing.is_empty() {
        // items with no file path (a phone, a ZIP's inside, an Outlook attachment): writing would lose them
        let has = |f: PCWSTR| IsClipboardFormatAvailable(RegisterClipboardFormatW(f)).is_ok();
        if has(w!("Shell IDList Array")) || has(w!("FileGroupDescriptorW")) || has(w!("FileGroupDescriptor")) {
            return Added::Virtual;
        }
    } else if effect.is_some_and(|e| e & DROPEFFECT_MOVE.0 != 0) {
        // cut items: added ones would be moved too, and the list we write isn't one the shell clears after a move
        return Added::Cut;
    }
    let Some(m) = merge_clip(&existing, add, SEND_MAX as usize) else { return Added::TooMany };
    if m.added == 0 {
        return Added::AllThere(m.list.len());
    }
    // both blocks ready before the clipboard is emptied
    let Some(hd) = global_bytes(&dropfiles_bytes(&m.list)) else { return Added::Failed };
    let Some(he) = global_bytes(&DROPEFFECT_COPY.0.to_le_bytes()) else {
        _ = GlobalFree(Some(hd));
        return Added::Failed;
    };
    if EmptyClipboard().is_err() || SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(hd.0))).is_err() {
        _ = GlobalFree(Some(hd));
        _ = GlobalFree(Some(he));
        return Added::Failed;
    }
    // after a successful SetClipboardData the system owns the block; a list without the effect pastes as a copy
    if SetClipboardData(effect_fmt, Some(HANDLE(he.0))).is_err() {
        _ = GlobalFree(Some(he));
    }
    Added::Done { added: m.added, total: m.list.len(), folders: folder_count(&m.list), same: m.same_name }
}

pub(crate) unsafe fn add_to_clip(picked: std::result::Result<Vec<String>, Pick>) {
    let Ok(add) = picked else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let ko = KO.load(Ordering::Relaxed);
    let r = clip_open(app(|x| x.main)).map(|_open| add_open(&add));
    let note = match r {
        Ok(Added::Done { added, total, folders, same }) => return set_note(clip_note(ko, added, total, folders, same)),
        Ok(Added::AllThere(n)) => {
            if ko {
                format!("이미 클립보드에 있음 — 모두 {n}개")
            } else {
                let items = if n == 1 { "item" } else { "items" };
                format!("Already on the clipboard — {n} {items} in all")
            }
        }
        Ok(Added::Cut) => tr(
            "Not added: the clipboard holds cut items — paste them first, or start again with Ctrl+C",
            "더하지 않음: 클립보드에 잘라낸 항목이 있음 — 먼저 붙여 넣거나 Ctrl+C로 새로 시작하세요",
        )
        .into(),
        Ok(Added::Virtual) => tr(
            "Not added: the clipboard holds items with no file path (a phone, a ZIP …) — paste them first",
            "더하지 않음: 클립보드에 파일 경로가 없는 항목(휴대폰·ZIP 안 등)이 있음 — 먼저 붙여 넣으세요",
        )
        .into(),
        Ok(Added::TooMany) => tr("Not added: the clipboard would hold over 10,000 items", "더하지 않음: 클립보드 항목이 1만 개를 넘게 됨").into(),
        Ok(Added::Failed) => tr("Could not write to the clipboard", "클립보드에 쓰지 못함").into(),
        Err(ClipBusy(who)) => clip_busy_note(ko, &who),
    };
    _ = MessageBeep(MB_ICONWARNING);
    set_note(note);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn merge_clip_lists() {
        // new items go after the old ones; case and a trailing '\' don't make a second copy
        let m = merge_clip(&v(&[r"C:\a\x.txt", r"D:\Photos"]), &v(&[r"c:\A\X.TXT", r"D:\photos\", r"E:\y.doc"]), 100).unwrap();
        assert_eq!(m.list, v(&[r"C:\a\x.txt", r"D:\Photos", r"E:\y.doc"]));
        assert_eq!((m.added, m.same_name), (1, 0));
        // everything there already: nothing added
        let m = merge_clip(&v(&[r"C:\a"]), &v(&[r"C:\a\b.txt", r"C:\A\"]), 100).unwrap();
        assert_eq!((m.list.clone(), m.added), (v(&[r"C:\a"]), 0));
        // a new folder takes the old items inside it; a name that only starts the same (C:\ab) stays
        let m = merge_clip(&v(&[r"C:\w\a.txt", r"C:\w\sub\b.txt", r"C:\wx.txt"]), &v(&[r"C:\W"]), 100).unwrap();
        assert_eq!(m.list, v(&[r"C:\wx.txt", r"C:\W"]));
        assert_eq!(m.added, 1);
        // a drive root holds everything on it; shares walk up to \\server\share only
        let m = merge_clip(&v(&[r"D:\"]), &v(&[r"D:\x\y", r"\\srv\share\a", r"\\srv\share"]), 100).unwrap();
        assert_eq!(m.list, v(&[r"D:\", r"\\srv\share"]));
        // the same file name from two folders is counted, not dropped
        let m = merge_clip(&v(&[r"C:\a\Report.docx"]), &v(&[r"C:\b\report.DOCX", r"C:\b\other.txt"]), 100).unwrap();
        assert_eq!((m.list.len(), m.added, m.same_name), (3, 2, 1));
        // blank paths are skipped; over the cap nothing is returned
        assert_eq!(merge_clip(&[], &v(&["", "  "]), 100).unwrap().list.len(), 0);
        assert!(merge_clip(&v(&[r"C:\1", r"C:\2"]), &v(&[r"C:\3"]), 2).is_none());
        assert!(merge_clip(&v(&[r"C:\1", r"C:\2"]), &v(&[r"C:\2"]), 2).is_some());
        assert_eq!(folder_count(&v(&[r"C:\a\x", r"c:\A\y", r"D:\z", r"E:\"])), 3);
    }

    #[test]
    fn clip_note_both_languages() {
        let en = clip_note(false, 3, 7, 3, 0);
        let ko = clip_note(true, 3, 7, 3, 0);
        assert_eq!(en, "Added 3 to the clipboard — 7 items from 3 folders · Ctrl+V pastes them all");
        assert_eq!(ko, "클립보드에 3개 더함 — 모두 7개 · 폴더 3곳 · Ctrl+V로 한 번에 붙여 넣기");
        assert_eq!(clip_note(false, 1, 1, 1, 0), "Added 1 to the clipboard — 1 item from 1 folder · Ctrl+V pastes them all");
        for ko in [false, true] {
            let s = clip_note(ko, 4, 12, 5, 2);
            for n in ["4", "12", "5", "2"] {
                assert!(s.contains(n), "{s}");
            }
            let plain = clip_note(ko, 4, 12, 5, 0);
            assert!(s.starts_with(&plain) && s.len() > plain.len() && !s.contains('{'), "{s}");
        }
    }

    #[test]
    fn dropfiles_layout() {
        let b = dropfiles_bytes(&v(&["C:\\a", "D:\\é"]));
        assert_eq!(&b[..20], &[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
        let w: Vec<u16> = b[20..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let want: Vec<u16> = "C:\\a\0D:\\é\0\0".encode_utf16().collect();
        assert_eq!(w, want);
        assert_eq!(dropfiles_bytes(&[]).len(), 22);
        assert!(is_shell_pick(ID_CLIPADD) && open_in_pick(ID_CLIPADD) == OpenPick::Shell);
    }
}
