// Everything 1.4 IPC over WM_COPYDATA. No window is created here.
// Query2 (EVERYTHING_IPC_COPYDATA_QUERY2W, dwData 18) is the 1.4.1 wide query.
// Query W (EVERYTHING_IPC_COPYDATAQUERYW, dwData 2) is the older wide query, used when Query2 is refused.
// Both put a 32-bit HWND in the struct. Windows sign-extends it, including on x64.

pub(crate) const COPYDATA_QUERY_W: usize = 2;
pub(crate) const COPYDATA_QUERY2_W: usize = 18;
pub(crate) const REPLY_ID: u32 = 0x4752_4945;
pub(crate) const REQUEST_NAME: u32 = 0x0000_0001;
pub(crate) const REQUEST_PATH: u32 = 0x0000_0002;
pub(crate) const SORT_NAME_ASCENDING: u32 = 1;
pub(crate) const FOLDER: u32 = 0x0000_0001;
/// Name, path, size, date modified. Everything returns those columns in this bit order.
pub(crate) const REQUEST_RESULT: u32 = REQUEST_NAME | REQUEST_PATH | REQUEST_SIZE | REQUEST_DATE_MODIFIED;
/// EVERYTHING_IPC_SORT_*_ASCENDING for name, path, size, date modified. The next code is descending.
const SORT_ASC: [u32; 4] = [1, 3, 5, 13];
/// Matches kept from one reply. The total can be larger, which sets `more`.
pub(crate) const MAX_RESULTS: u32 = 5000;

const REQUEST_FULL: u32 = 0x0000_0004;
const REQUEST_EXT: u32 = 0x0000_0008;
const REQUEST_SIZE: u32 = 0x0000_0010;
const REQUEST_DATE_CREATED: u32 = 0x0000_0020;
const REQUEST_DATE_MODIFIED: u32 = 0x0000_0040;
const REQUEST_DATE_ACCESSED: u32 = 0x0000_0080;
const REQUEST_ATTR: u32 = 0x0000_0100;
const REQUEST_FILELIST: u32 = 0x0000_0200;
const REQUEST_RUN: u32 = 0x0000_0400;
const REQUEST_DATE_RUN: u32 = 0x0000_0800;
const REQUEST_DATE_RECENT: u32 = 0x0000_1000;
const REQUEST_HL_NAME: u32 = 0x0000_2000;
const REQUEST_HL_PATH: u32 = 0x0000_4000;
const REQUEST_HL_FULL: u32 = 0x0000_8000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IpcQuery {
    pub(crate) copy_data: usize,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IpcHit {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) dir: bool,
    /// Bytes. None when Everything does not know (folders, mostly).
    pub(crate) size: Option<u64>,
    /// FILETIME, UTC. None when Everything does not know.
    pub(crate) modified: Option<u64>,
}

/// A sort code from settings, or anywhere else: one of the eight header orders, else name A→Z.
pub(crate) fn ev_sort_ok(code: u32) -> u32 {
    if SORT_ASC.iter().any(|&a| code == a || code == a + 1) { code } else { SORT_NAME_ASCENDING }
}

/// Column of a sort code (0 name, 1 path, 2 size, 3 date modified) and whether it is descending.
pub(crate) fn ev_sort_col(code: u32) -> (usize, bool) {
    let code = ev_sort_ok(code);
    let c = SORT_ASC.iter().position(|&a| code == a || code == a + 1).unwrap_or(0);
    (c, code != SORT_ASC[c])
}

/// A click on column `col`. The same column turns around. A new column starts A→Z for name and path,
/// and biggest / newest first for size and date.
pub(crate) fn ev_sort_click(cur: u32, col: usize) -> u32 {
    let (c, desc) = ev_sort_col(cur);
    let col = col.min(3);
    let desc = if c == col { !desc } else { col >= 2 };
    SORT_ASC[col] + u32::from(desc)
}

/// Three column borders as fractions of the row width, strictly inside (0, 1) and increasing.
/// Anything else (a bad file, an empty value) means the default widths.
pub(crate) fn ev_cols_ok(c: [f64; 3]) -> Option<[f64; 3]> {
    (c.iter().copied().all(f64::is_finite) && c[0] > 0.0 && c[0] < c[1] && c[1] < c[2] && c[2] < 1.0).then_some(c)
}

/// A drag stores four decimal places. A fraction that no longer increases is refused.
pub(crate) fn ev_cols_quant(c: [f64; 3]) -> Option<[f64; 3]> {
    ev_cols_ok(c.map(|x| (x * 10_000.0).round() / 10_000.0))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IpcList {
    pub(crate) hits: Vec<IpcHit>,
    pub(crate) total: usize,
    pub(crate) more: bool,
}

/// Query2 asks for name, path, size and date modified, sorted by `sort`.
/// Everything orders every match, then this reply keeps the first `max_results`.
pub(crate) fn query2(reply_hwnd: u32, reply_msg: u32, max_results: u32, text: &str, sort: u32) -> IpcQuery {
    let mut bytes = Vec::with_capacity(28 + text.len() * 2 + 2);
    push_u32(&mut bytes, reply_hwnd);
    push_u32(&mut bytes, reply_msg);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, max_results);
    push_u32(&mut bytes, REQUEST_RESULT);
    push_u32(&mut bytes, ev_sort_ok(sort));
    push_utf16z(&mut bytes, text);
    IpcQuery { copy_data: COPYDATA_QUERY2_W, bytes }
}

pub(crate) fn query_w(reply_hwnd: u32, reply_msg: u32, max_results: u32, text: &str) -> IpcQuery {
    let mut bytes = Vec::with_capacity(20 + text.len() * 2 + 2);
    push_u32(&mut bytes, reply_hwnd);
    push_u32(&mut bytes, reply_msg);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, max_results);
    push_utf16z(&mut bytes, text);
    IpcQuery { copy_data: COPYDATA_QUERY_W, bytes }
}

/// EVERYTHING_IPC_LIST2. `request_flags` is the reply's own flags, so extra columns are skipped.
pub(crate) fn parse_list2(bytes: &[u8], cap: usize) -> Option<IpcList> {
    if bytes.len() < 20 || cap == 0 {
        return None;
    }
    let total = u32_at(bytes, 0)? as usize;
    let num = u32_at(bytes, 4)? as usize;
    let flags = u32_at(bytes, 12)?;
    let n = num.min(cap);
    let mut hits = Vec::with_capacity(n);
    for i in 0..n {
        let base = 20 + i * 8;
        let Some(item_flags) = u32_at(bytes, base) else { break };
        let Some(data_off) = u32_at(bytes, base + 4) else { break };
        let Some((name, path, size, modified)) = read_named(bytes, data_off as usize, flags) else { continue };
        if name.is_empty() {
            continue;
        }
        hits.push(hit(name, path, item_flags, size, modified));
    }
    let more = total > hits.len();
    Some(IpcList { hits, total, more })
}

/// EVERYTHING_IPC_LISTW. Offsets are from the start of the list and point at a NUL-terminated UTF-16 string.
pub(crate) fn parse_list_w(bytes: &[u8], cap: usize) -> Option<IpcList> {
    if bytes.len() < 28 || cap == 0 {
        return None;
    }
    let total = u32_at(bytes, 8)? as usize;
    let num = u32_at(bytes, 20)? as usize;
    let n = num.min(cap);
    let mut hits = Vec::with_capacity(n);
    for i in 0..n {
        let base = 28 + i * 12;
        let Some(flags) = u32_at(bytes, base) else { break };
        let Some(name_off) = u32_at(bytes, base + 4) else { break };
        let Some(path_off) = u32_at(bytes, base + 8) else { break };
        let Some(name) = utf16z(bytes, name_off as usize) else { continue };
        if name.is_empty() {
            continue;
        }
        let path = utf16z(bytes, path_off as usize).unwrap_or_default();
        hits.push(hit(name, path, flags, None, None));
    }
    let more = total > hits.len();
    Some(IpcList { hits, total, more })
}

fn hit(name: String, path: String, flags: u32, size: Option<u64>, modified: Option<u64>) -> IpcHit {
    let full = join_path(&path, &name);
    IpcHit { name, path: full, dir: flags & FOLDER != 0, size, modified }
}

fn join_path(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else if path.ends_with('\\') || path.ends_with('/') {
        format!("{path}{name}")
    } else {
        format!("{path}\\{name}")
    }
}

fn read_named(bytes: &[u8], mut off: usize, flags: u32) -> Option<(String, String, Option<u64>, Option<u64>)> {
    let mut name = None;
    let mut path = None;
    let mut size = None;
    let mut modified = None;
    for bit in [REQUEST_NAME, REQUEST_PATH, REQUEST_FULL, REQUEST_EXT, REQUEST_SIZE, REQUEST_DATE_CREATED, REQUEST_DATE_MODIFIED, REQUEST_DATE_ACCESSED, REQUEST_ATTR, REQUEST_FILELIST, REQUEST_RUN, REQUEST_DATE_RUN, REQUEST_DATE_RECENT, REQUEST_HL_NAME, REQUEST_HL_PATH, REQUEST_HL_FULL] {
        if flags & bit == 0 {
            continue;
        }
        if matches!(bit, REQUEST_SIZE | REQUEST_DATE_CREATED | REQUEST_DATE_MODIFIED | REQUEST_DATE_ACCESSED | REQUEST_DATE_RUN | REQUEST_DATE_RECENT) {
            let v = u64_at(bytes, off)?;
            off = off.checked_add(8)?;
            // -1 (and 0 for a date) is how Everything says it does not know.
            if bit == REQUEST_SIZE && v != u64::MAX {
                size = Some(v);
            } else if bit == REQUEST_DATE_MODIFIED && v != 0 && v != u64::MAX {
                modified = Some(v);
            }
            continue;
        }
        if matches!(bit, REQUEST_ATTR | REQUEST_RUN) {
            off = off.checked_add(4)?;
            continue;
        }
        let (text, next) = counted_utf16(bytes, off)?;
        off = next;
        if bit == REQUEST_NAME {
            name = Some(text);
        } else if bit == REQUEST_PATH {
            path = Some(text);
        }
    }
    Some((name?, path.unwrap_or_default(), size, modified))
}

fn counted_utf16(bytes: &[u8], off: usize) -> Option<(String, usize)> {
    let len = u32_at(bytes, off)? as usize;
    if len > 32_768 {
        return None;
    }
    let start = off + 4;
    let end = start.checked_add(len.checked_mul(2)?)?;
    if end > bytes.len() {
        return None;
    }
    let mut u = Vec::with_capacity(len);
    for i in 0..len {
        let o = start + i * 2;
        u.push(u16::from_le_bytes([bytes[o], bytes[o + 1]]));
    }
    let next = end.saturating_add(2);
    trim_nuls(&mut u);
    Some((String::from_utf16_lossy(&u), next))
}

fn utf16z(bytes: &[u8], off: usize) -> Option<String> {
    if off > bytes.len() || off % 2 != 0 {
        return None;
    }
    let mut u = Vec::new();
    let mut i = off;
    while i + 1 < bytes.len() {
        let w = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if w == 0 {
            break;
        }
        u.push(w);
        i += 2;
        if u.len() > 32_768 {
            break;
        }
    }
    trim_nuls(&mut u);
    Some(String::from_utf16_lossy(&u))
}

fn trim_nuls(u: &mut Vec<u16>) {
    while u.last() == Some(&0) {
        u.pop();
    }
}

/// A drive or UNC path. One code unit and no directory is a misread of the other list shape.
fn path_ok(p: &str) -> bool {
    let p = p.trim_end_matches('\0');
    let b = p.as_bytes();
    (b.len() >= 3 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || p.starts_with(r"\\") || p.starts_with("//")
}

fn hit_ok(h: &IpcHit) -> bool {
    let name_ok = !h.name.is_empty() && h.name.chars().all(|c| !c.is_control());
    name_ok && (path_ok(&h.path) || (h.name.chars().count() >= 2 && h.path.is_empty()))
}

/// Empty is a real answer only when Everything reported no hits.
pub(crate) fn list_ok(list: &IpcList) -> bool {
    if list.hits.is_empty() {
        return list.total == 0;
    }
    list.hits.iter().all(hit_ok) && list.hits.iter().any(|h| path_ok(&h.path) || h.name.chars().count() >= 2)
}

fn list_score(list: &IpcList) -> usize {
    list.hits.iter().filter(|h| path_ok(&h.path)).count()
}

/// Query2 is answered with LIST2 and Query W with LISTW. The wrong shape still parses as a
/// short list of junk names. Keep the list that actually contains paths.
pub(crate) fn best_list(bytes: &[u8], cap: usize) -> Option<IpcList> {
    let raw2 = parse_list2(bytes, cap);
    let raww = parse_list_w(bytes, cap);
    let junk = |list: &Option<IpcList>| list.as_ref().is_some_and(|l| !l.hits.is_empty() && !list_ok(l));
    let junk = junk(&raw2) || junk(&raww);
    let a = raw2.filter(list_ok);
    let b = raww.filter(list_ok);
    let good = match (a, b) {
        (Some(a), Some(b)) => if list_score(&a) >= list_score(&b) { a } else { b },
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => return None,
    };
    if good.hits.is_empty() && junk {
        return None;
    }
    Some(good)
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_utf16z(buf: &mut Vec<u8>, text: &str) {
    for u in text.encode_utf16().chain(std::iter::once(0)) {
        buf.extend_from_slice(&u.to_le_bytes());
    }
}

fn u32_at(bytes: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(off..off + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], off: usize) -> Option<u64> {
    let lo = u32_at(bytes, off)? as u64;
    let hi = u32_at(bytes, off + 4)? as u64;
    Some(lo | hi << 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32s(bytes: &[u8], n: usize) -> Vec<u32> {
        (0..n).map(|i| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())).collect()
    }

    #[test]
    fn query2_is_the_wide_v2_message_and_keeps_the_search_text() {
        let q = query2(0x1111_2222, REPLY_ID, MAX_RESULTS, r#""C:\Work\" ext:pdf"#, 14);
        assert_eq!(q.copy_data, COPYDATA_QUERY2_W);
        assert_eq!(u32s(&q.bytes, 7), vec![0x1111_2222, REPLY_ID, 0, 0, MAX_RESULTS, REQUEST_RESULT, 14]);
        assert_eq!(query2(1, 2, 10, "a", 99).bytes[24..28], 1u32.to_le_bytes());
        let text: Vec<u16> = r#""C:\Work\" ext:pdf"#.encode_utf16().chain(std::iter::once(0)).collect();
        assert_eq!(q.bytes.len(), 28 + text.len() * 2);
        for (i, u) in text.iter().enumerate() {
            let o = 28 + i * 2;
            assert_eq!(u16::from_le_bytes([q.bytes[o], q.bytes[o + 1]]), *u);
        }
        let ko = query2(1, 2, 10, "보고서", 1);
        let chars: Vec<u16> = "보고서".encode_utf16().chain(std::iter::once(0)).collect();
        assert_eq!(ko.bytes.len(), 28 + chars.len() * 2);
    }

    #[test]
    fn query_w_is_copydata_2_with_a_20_byte_header() {
        let q = query_w(7, 9, 3, "size:>1mb");
        assert_eq!(q.copy_data, COPYDATA_QUERY_W);
        assert_eq!(u32s(&q.bytes, 5), vec![7, 9, 0, 0, 3]);
        assert_eq!(q.bytes.len(), 20 + ("size:>1mb".encode_utf16().count() + 1) * 2);
    }

    #[test]
    fn list2_reads_name_and_path_and_marks_more() {
        let name = counted("Sub");
        let path = counted(r"C:\d");
        let data_off = 20 + 8;
        let mut bytes = Vec::new();
        for n in [3u32, 1, 0, REQUEST_NAME | REQUEST_PATH, SORT_NAME_ASCENDING] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        bytes.extend_from_slice(&FOLDER.to_le_bytes());
        bytes.extend_from_slice(&(data_off as u32).to_le_bytes());
        bytes.extend_from_slice(&name);
        bytes.extend_from_slice(&path);
        let list = parse_list2(&bytes, 10).unwrap();
        assert!(list_ok(&list));
        assert_eq!(best_list(&bytes, 10).unwrap().hits[0].path, r"C:\d\Sub");
        assert!(list.more);
        assert_eq!(list.total, 3);
        assert_eq!(list.hits.len(), 1);
        assert!(list.hits[0].dir);
        assert_eq!(list.hits[0].name, "Sub");
        assert_eq!(list.hits[0].path, r"C:\d\Sub");
        assert_eq!((list.hits[0].size, list.hits[0].modified), (None, None));
        assert!(parse_list2(&bytes[..10], 10).is_none());
        assert!(parse_list2(&bytes, 0).is_none());
        let capped = parse_list2(&bytes, 1).unwrap();
        assert_eq!(capped.hits.len(), 1);
        assert!(capped.more);
    }

    #[test]
    fn list_w_reads_offsets_from_the_start_of_the_buffer() {
        let mut bytes = Vec::new();
        for n in [0u32, 1, 2, 0, 1, 1, 0] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        let name: Vec<u16> = "a.txt".encode_utf16().chain(std::iter::once(0)).collect();
        let path: Vec<u16> = r"C:\d".encode_utf16().chain(std::iter::once(0)).collect();
        let name_off = 28 + 12;
        let path_off = name_off + name.len() * 2;
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(name_off as u32).to_le_bytes());
        bytes.extend_from_slice(&(path_off as u32).to_le_bytes());
        for u in name.iter().chain(&path) {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let list = parse_list_w(&bytes, 10).unwrap();
        assert!(list.more);
        assert_eq!(list.hits.len(), 1);
        assert_eq!(list.hits[0].name, "a.txt");
        assert_eq!(list.hits[0].path, r"C:\d\a.txt");
        assert!(!list.hits[0].dir);
        assert!(parse_list_w(&[], 10).is_none());
    }

    fn counted(s: &str) -> Vec<u8> {
        let mut v = Vec::new();
        let chars: Vec<u16> = s.encode_utf16().collect();
        v.extend_from_slice(&(chars.len() as u32).to_le_bytes());
        for u in chars.into_iter().chain(std::iter::once(0)) {
            v.extend_from_slice(&u.to_le_bytes());
        }
        v
    }

    fn list2_one_glyph() -> Vec<u8> {
        let name = counted("\u{3131}");
        let path = counted("");
        let data_off = 20 + 8;
        let mut bytes = Vec::new();
        for n in [1u32, 1, 0, REQUEST_NAME | REQUEST_PATH, SORT_NAME_ASCENDING] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(data_off as u32).to_le_bytes());
        bytes.extend_from_slice(&name);
        bytes.extend_from_slice(&path);
        bytes
    }

    fn list_w_findme() -> Vec<u8> {
        let rows = [
            (FOLDER, "findme_dir", r"C:\Users\Jun\dev\maguro-test\search"),
            (0, "findme_deep.txt", r"C:\Users\Jun\dev\maguro-test\search\findme_dir"),
            (0, "findme_note.txt", r"C:\Users\Jun\dev\maguro-test\search"),
        ];
        let mut strings = Vec::new();
        let mut items = Vec::new();
        let base = 28 + rows.len() * 12;
        for (flags, name, path) in rows {
            let name_off = base + strings.len();
            for u in name.encode_utf16().chain(std::iter::once(0)) {
                strings.extend_from_slice(&u.to_le_bytes());
            }
            let path_off = base + strings.len();
            for u in path.encode_utf16().chain(std::iter::once(0)) {
                strings.extend_from_slice(&u.to_le_bytes());
            }
            items.extend_from_slice(&flags.to_le_bytes());
            items.extend_from_slice(&(name_off as u32).to_le_bytes());
            items.extend_from_slice(&(path_off as u32).to_le_bytes());
        }
        let mut bytes = Vec::new();
        for n in [1u32, 2, 3, 1, 2, 3, 0] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        bytes.extend_from_slice(&items);
        bytes.extend_from_slice(&strings);
        bytes
    }

    #[test]
    fn a_single_glyph_is_not_an_everything_result_and_listw_paths_win() {
        let odd = list2_one_glyph();
        let parsed = parse_list2(&odd, 10).unwrap();
        assert_eq!(parsed.hits.len(), 1);
        assert_eq!(parsed.hits[0].name, "\u{3131}");
        assert!(!list_ok(&parsed));
        assert!(best_list(&odd, 10).is_none());
        let mut empty = vec![0u8; 20];
        empty[12..16].copy_from_slice(&REQUEST_NAME.to_le_bytes());
        let none = best_list(&empty, 10).unwrap();
        assert!(none.hits.is_empty() && none.total == 0);
        let wide = list_w_findme();
        if let Some(as_list2) = parse_list2(&wide, 10) {
            assert!(!list_ok(&as_list2), "{as_list2:?}");
        }
        let best = best_list(&wide, 10).unwrap();
        let names: Vec<_> = best.hits.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, ["findme_dir", "findme_deep.txt", "findme_note.txt"]);
        assert_eq!(best.hits[2].path, r"C:\Users\Jun\dev\maguro-test\search\findme_note.txt");
    }

    #[test]
    fn sort_codes_and_sized_rows() {
        assert_eq!([0, 1, 2, 3].map(|c| ev_sort_click(1, c)), [2, 3, 6, 14]);
        assert_eq!((ev_sort_click(14, 3), ev_sort_click(13, 3), ev_sort_click(6, 2), ev_sort_click(14, 0)), (13, 14, 5, 1));
        assert_eq!((ev_sort_col(14), ev_sort_col(3), ev_sort_col(0), ev_sort_col(9)), ((3, true), (1, false), (0, false), (0, false)));
        assert_eq!([0, 2, 5, 7, 13, 15].map(ev_sort_ok), [1, 2, 5, 1, 13, 1]);
        assert_eq!(ev_cols_ok([0.42, 0.7, 0.84]), Some([0.42, 0.7, 0.84]));
        assert!(ev_cols_ok([0.5, 0.4, 0.8]).is_none());
        assert!(ev_cols_ok([0.0, 0.5, 0.8]).is_none());
        assert!(ev_cols_ok([0.1, 0.5, 1.0]).is_none());
        assert!(ev_cols_ok([f64::NAN, 0.5, 0.8]).is_none());
        assert_eq!(ev_cols_quant([500.0 / 842.0, 624.0 / 842.0, 708.0 / 842.0]).unwrap()[0], 0.5938);

        let flags = REQUEST_RESULT | REQUEST_EXT | REQUEST_DATE_CREATED;
        let mut bytes = Vec::new();
        for n in [2u32, 2, 0, flags, 14] {
            bytes.extend_from_slice(&n.to_le_bytes());
        }
        let mut data = Vec::new();
        let base = 20 + 16;
        for (item, name, path, size, modified) in [
            (FOLDER, "Work", r"C:\Users\me", u64::MAX, 133_000_000_000_000_000u64),
            (0, "a.pdf", r"C:\Work", 1001u64, 133_000_000_000_000_001u64),
        ] {
            bytes.extend_from_slice(&item.to_le_bytes());
            bytes.extend_from_slice(&((base + data.len()) as u32).to_le_bytes());
            data.extend(counted(name));
            data.extend(counted(path));
            data.extend(counted("pdf"));
            data.extend(size.to_le_bytes());
            data.extend(7u64.to_le_bytes());
            data.extend(modified.to_le_bytes());
        }
        bytes.extend(data);
        let list = parse_list2(&bytes, 10).unwrap();
        assert_eq!((list.hits[0].name.as_str(), list.hits[0].size, list.hits[0].modified), ("Work", None, Some(133_000_000_000_000_000)));
        assert_eq!(
            (list.hits[1].name.as_str(), list.hits[1].size, list.hits[1].modified, list.hits[1].path.as_str()),
            ("a.pdf", Some(1001), Some(133_000_000_000_000_001), r"C:\Work\a.pdf"),
        );
        assert!(parse_list2(&bytes[..bytes.len() - 3], 10).unwrap().hits.len() < 2);
    }
}
