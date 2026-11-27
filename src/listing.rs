use crate::*;
use std::collections::HashMap;
use windows::Win32::Storage::EnhancedStorage::{PKEY_DateModified, PKEY_ItemTypeText, PKEY_Size};

// ⇄ menu "Copy the file list as a table": the selected items (or, with none selected, every item the view
// shows, in its order with its filter) as tab-separated rows — name, date modified, type, size, full path —
// that Excel or a word processor's table takes as cells on Ctrl+V.
// After Total Commander's "Copy names with path, size and date", Directory Opus' Print / Export Folder and
// XYplorer's Copy Item Info.

pub(crate) const ID_LIST: usize = 1147; // ⇄ menu (1138 stays unused: the menu number test checks it as Unknown)

pub(crate) struct Row {
    pub(crate) name: String,
    pub(crate) date: Option<[u16; 6]>, // local time; None: not read
    pub(crate) kind: String,
    pub(crate) size: Option<u64>, // bytes; None: a folder, or not read
    pub(crate) path: String,      // empty: a virtual item with no file path
}

// One cell: no tab, line break or other control character can split the row or the cell.
fn cell(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

// The header matches Explorer's Details columns; sizes are whole bytes so a spreadsheet can sum them.
// A name starting with = + - @ gets a leading ' so a spreadsheet takes it as text, never as a formula.
pub(crate) fn list_table(rows: &[Row], ko: bool) -> String {
    let head = if ko { "이름\t수정한 날짜\t유형\t크기\t전체 경로" } else { "Name\tDate modified\tType\tSize\tFull path" };
    let mut out = String::with_capacity(64 + rows.len() * 96);
    out.push_str(head);
    out.push_str("\r\n");
    for r in rows {
        let mut name = cell(&r.name);
        if name.starts_with(['=', '+', '-', '@']) {
            name.insert(0, '\'');
        }
        let date = r.date.map(date_min).unwrap_or_default();
        let size = r.size.map(|s| s.to_string()).unwrap_or_default();
        out.push_str(&format!("{name}\t{date}\t{}\t{size}\t{}\r\n", cell(&r.kind), cell(&r.path)));
    }
    out
}

// (selected, all) item counts of the pane's view: the menu label and whether the item is grey.
unsafe fn counts(src: usize) -> (i32, i32) {
    let Some(fv) = view_of::<IFolderView2>(src) else { return (0, 0) };
    (fv.ItemCount(SVGIO_SELECTION).unwrap_or(0), fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0))
}

// The ⇄ menu item: "… {n} selected items" with a selection, "this folder's file list" without, grey when
// the view shows nothing.
pub(crate) unsafe fn list_menu(menu: HMENU, src: usize) {
    let (sel, all) = counts(src);
    let (flags, label) = match (sel, all) {
        (s, _) if s > 0 => (MF_STRING, tr2(M_LIST_N).replace("{n}", &s.to_string())),
        (_, a) if a > 0 => (MF_STRING, tr2(M_LIST).to_string()),
        _ => (MF_STRING | MF_GRAYED, tr2(M_LIST).to_string()),
    };
    _ = AppendMenuW(menu, flags, ID_LIST, &HSTRING::from(label));
}

unsafe fn beep() {
    _ = MessageBeep(MB_ICONWARNING);
}

fn filetime_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

// The rows of the pane's view, read again when the item is picked (the selection may have changed since).
// Size, date and type come from the properties the view already holds; the type text is asked once per
// extension (and once for folders), so a 10,000-item list doesn't ask 10,000 times.
pub(crate) unsafe fn copy_list(src: usize) {
    let Some(fv) = view_of::<IFolderView2>(src) else { return beep() };
    let what = if fv.ItemCount(SVGIO_SELECTION).unwrap_or(0) > 0 { SVGIO_SELECTION } else { SVGIO_ALLVIEW };
    let Ok(arr) = fv.Items::<IShellItemArray>(_SVGIO(what.0 | SVGIO_FLAG_VIEWORDER.0)).or_else(|_| fv.Items::<IShellItemArray>(what)) else {
        return beep();
    };
    let n = arr.GetCount().unwrap_or(0);
    if n == 0 {
        return beep();
    }
    if n > SEND_MAX {
        beep();
        return set_note(tr("Too many items to copy as a table (over 10,000)", "항목이 너무 많아 표로 복사하지 않음 (1만 개 초과)").into());
    }
    let mut types: HashMap<String, String> = HashMap::new(); // lower-case extension ("" none, "\\" folder) -> type
    let mut rows = Vec::with_capacity(n as usize);
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { continue };
        let it2 = it.cast::<IShellItem2>().ok();
        let attr = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0)).unwrap_or_default().0;
        let dir = real_folder(attr);
        let path = it.GetDisplayName(SIGDN_FILESYSPATH).map(|p| pwstr(p)).unwrap_or_default();
        // the real file name with its extension (the view may hide extensions); a drive root, a virtual item
        // or a Recycle Bin item (its file is $RXXXXXX.ext, not the original name): its display name
        let file = Path::new(&path).file_name().map(|f| f.to_string_lossy().into_owned());
        let binned = path.to_lowercase().contains("\\$recycle.bin\\");
        let name = match &file {
            Some(f) if !binned => f.clone(),
            _ => it.GetDisplayName(SIGDN_NORMALDISPLAY).map(|p| pwstr(p)).unwrap_or_default(),
        };
        let read_type = || it2.as_ref().and_then(|i| i.GetString(&PKEY_ItemTypeText).ok()).map(|p| pwstr(p)).unwrap_or_default();
        let kind = match &file {
            // virtual items, drive roots and dot names (.gitignore: no extension to key on): each its own
            None => read_type(),
            Some(f) if f.starts_with('.') => read_type(),
            Some(f) => {
                let key = if dir {
                    "\\".to_string()
                } else {
                    Path::new(f).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
                };
                types.entry(key).or_insert_with(read_type).clone()
            }
        };
        let size = if dir { None } else { it2.as_ref().and_then(|i| i.GetUInt64(&PKEY_Size).ok()) };
        let date = it2.as_ref().and_then(|i| i.GetFileTime(&PKEY_DateModified).ok()).map(|ft| local_date(filetime_u64(ft))).filter(|d| *d != [0; 6]);
        rows.push(Row { name, date, kind, size, path });
    }
    let ko = KO.load(Ordering::Relaxed);
    if rows.is_empty() {
        return beep();
    }
    if let Err(e) = put_clip(&list_table(&rows, ko), CLIP_TRIES) {
        return clip_failed(e);
    }
    let n = rows.len();
    set_note(if ko { format!("{n}개 항목을 표로 복사함 — 엑셀에 붙여 넣기") } else { format!("Copied a table of {n} items — paste it into Excel") });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, date: Option<[u16; 6]>, kind: &str, size: Option<u64>, path: &str) -> Row {
        Row { name: name.into(), date, kind: kind.into(), size, path: path.into() }
    }

    #[test]
    fn list_table_rows() {
        let rows = [
            row("report.xlsx", Some([2026, 11, 7, 9, 5, 42]), "Microsoft Excel Worksheet", Some(12345), r"C:\w\report.xlsx"),
            row("Photos", Some([2025, 1, 31, 23, 59, 0]), "File folder", None, r"C:\w\Photos"),
            row("=SUM(A1).txt", None, "Text Document", Some(0), r"C:\w\=SUM(A1).txt"),
            row("+1 -2 @x", None, "File", None, ""),
            row("a\tb\nc\rd", None, "T\tx", Some(7), "C:\\w\\a\x07b"),
        ];
        let en = list_table(&rows, false);
        let lines: Vec<&str> = en.split("\r\n").collect();
        assert_eq!(lines[0], "Name\tDate modified\tType\tSize\tFull path");
        assert_eq!(lines[1], "report.xlsx\t2026-11-07 09:05\tMicrosoft Excel Worksheet\t12345\tC:\\w\\report.xlsx");
        // a folder: no size; a size of 0 stays 0
        assert_eq!(lines[2], "Photos\t2025-01-31 23:59\tFile folder\t\tC:\\w\\Photos");
        // a name that would run as a formula gets a leading ', its path and type don't
        assert_eq!(lines[3], "'=SUM(A1).txt\t\tText Document\t0\tC:\\w\\=SUM(A1).txt");
        assert_eq!(lines[4], "'+1 -2 @x\t\tFile\t\t");
        // control characters never split a row or a cell
        assert_eq!(lines[5], "a b c d\t\tT x\t7\tC:\\w\\a b");
        assert_eq!(lines.len(), 7);
        assert_eq!(lines[6], "");
        assert!(!en.replace("\r\n", "").contains(['\r', '\n']), "only CRLF ends a row");
        assert!(lines.iter().take(6).all(|l| l.matches('\t').count() == 4));
        let ko = list_table(&rows[..1], true);
        assert_eq!(ko.split("\r\n").next(), Some("이름\t수정한 날짜\t유형\t크기\t전체 경로"));
        assert_eq!(list_table(&[], false), "Name\tDate modified\tType\tSize\tFull path\r\n");
        assert!(is_shell_pick(ID_LIST) && open_in_pick(ID_LIST) == OpenPick::Shell);
    }
}
