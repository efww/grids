// Rewrite an .xlsx or .xlsm package without Excel.
//
// Unchanged zip entries are copied as stored on disk, compressed bytes included.
// Only worksheet, chartsheet, and workbook parts whose XML actually changes are
// deflated again. A package that is already in the requested state comes back
// unchanged, byte for byte.

use std::path::Path;

/// What the tidy should change. Each flag is one checkbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TidyOptions {
    pub(crate) selection_a1: bool,
    pub(crate) zoom_100: bool,
    pub(crate) first_sheet: bool,
}

impl TidyOptions {
    pub(crate) fn changes_files(self) -> bool {
        self.selection_a1 || self.zoom_100 || self.first_sheet
    }
}

/// `.xlsx` and `.xlsm` only. `.xls` is a different file format and is left alone.
pub(crate) fn is_ooxml_workbook_path(path: &str) -> bool {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let Some((stem, ext)) = name.rsplit_once('.') else { return false };
    if stem.is_empty() {
        return false;
    }
    ext.eq_ignore_ascii_case("xlsx") || ext.eq_ignore_ascii_case("xlsm")
}

/// Every selected item is a file, and every file is `.xlsx` or `.xlsm`.
pub(crate) fn applies(items: &[(String, bool)]) -> bool {
    !items.is_empty() && items.iter().all(|(path, file)| *file && is_ooxml_workbook_path(path))
}

/// `report.xlsx` → `report.bak.xlsx`, `Book.XLSM` → `Book.bak.XLSM`, in the same folder.
pub(crate) fn backup_path(path: &Path) -> std::path::PathBuf {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let bak = match name.rfind('.') {
        Some(i) if i > 0 => {
            let (stem, ext) = name.split_at(i);
            format!("{stem}.bak{ext}")
        }
        _ => format!("{name}.bak"),
    };
    path.with_file_name(bak)
}

/// Sharing violation (32) or lock violation (33). Excel holds the workbook this way.
pub(crate) fn is_share_violation(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(32) | Some(33))
}

const MAX_PART: usize = 256 << 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Leave,
    On,
    Off,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Worksheet,
    Chartsheet,
    Workbook,
}

struct ViewEdit {
    a1: bool,
    zoom: bool,
    tab: Tab,
    book: bool,
}

struct Entry {
    name: String,
    name_bytes: Vec<u8>,
    method: u16,
    flags: u16,
    time: u16,
    date: u16,
    comp_size: u64,
    uncomp_size: u64,
    local_off: u64,
    span_end: u64,
    made: u16,
    internal: u16,
    external: u32,
    comment: Vec<u8>,
    cd_raw: Vec<u8>,
    cd_order: usize,
}

struct ZipFile {
    entries: Vec<Entry>,
    cd_off: u64,
    comment: Vec<u8>,
}

/// Apply `opt` to an OOXML package. The bytes are a zip archive.
pub(crate) fn tidy_package(input: &[u8], opt: TidyOptions) -> Result<Vec<u8>, String> {
    if !opt.changes_files() {
        return Ok(input.to_vec());
    }
    let zip = parse_zip(input)?;
    let first = if opt.first_sheet { first_sheet_part(input, &zip)? } else { None };
    let mut changed: Vec<Option<Vec<u8>>> = vec![None; zip.entries.len()];
    let mut any = false;
    for (i, entry) in zip.entries.iter().enumerate() {
        let Some(edit) = edit_for(entry, opt, first.as_deref()) else { continue };
        if entry.flags & 1 != 0 {
            return Err(format!("{} is encrypted", entry.name));
        }
        let raw = compressed(input, entry)?;
        let xml = inflate(entry.method, raw, entry.uncomp_size)?;
        let next = edit_xml(&xml, edit.kind, &edit.view);
        if next != xml {
            changed[i] = Some(next);
            any = true;
        }
    }
    if !any {
        return Ok(input.to_vec());
    }
    rebuild(input, &zip, &changed)
}

struct Planned {
    kind: Kind,
    view: ViewEdit,
}

fn edit_for(entry: &Entry, opt: TidyOptions, first: Option<&str>) -> Option<Planned> {
    let name = norm_part(&entry.name);
    if name == "xl/workbook.xml" {
        if !opt.first_sheet {
            return None;
        }
        return Some(Planned {
            kind: Kind::Workbook,
            view: ViewEdit { a1: false, zoom: false, tab: Tab::Leave, book: true },
        });
    }
    let worksheet = is_worksheet_name(&name);
    let chartsheet = is_chartsheet_name(&name);
    if !worksheet && !chartsheet {
        return None;
    }
    let tab = if opt.first_sheet {
        if first.is_some_and(|p| parts_equal(&entry.name, p)) { Tab::On } else { Tab::Off }
    } else {
        Tab::Leave
    };
    let a1 = worksheet && opt.selection_a1;
    let zoom = worksheet && opt.zoom_100;
    if !a1 && !zoom && tab == Tab::Leave {
        return None;
    }
    Some(Planned {
        kind: if worksheet { Kind::Worksheet } else { Kind::Chartsheet },
        view: ViewEdit { a1, zoom, tab, book: false },
    })
}

fn is_worksheet_name(name: &str) -> bool {
    name.starts_with("xl/worksheets/") && name.ends_with(".xml") && !name.contains("/_rels/")
}

fn is_chartsheet_name(name: &str) -> bool {
    name.starts_with("xl/chartsheets/") && name.ends_with(".xml") && !name.contains("/_rels/")
}

fn norm_part(name: &str) -> String {
    name.trim().trim_start_matches('/').replace('\\', "/").to_ascii_lowercase()
}

fn parts_equal(entry: &str, target: &str) -> bool {
    let e = norm_part(entry);
    let t = norm_part(target).trim_matches('/').to_string();
    if e == t {
        return true;
    }
    if !t.starts_with("xl/") && (e == format!("xl/{t}") || e.ends_with(&format!("/{t}"))) {
        return true;
    }
    false
}

fn first_sheet_part(file: &[u8], zip: &ZipFile) -> Result<Option<String>, String> {
    let book = zip.entries.iter().find(|e| norm_part(&e.name) == "xl/workbook.xml");
    let Some(book) = book else { return Ok(None) };
    let rels = zip.entries.iter().find(|e| norm_part(&e.name) == "xl/_rels/workbook.xml.rels");
    let book_xml = inflate(book.method, compressed(file, book)?, book.uncomp_size)?;
    let rids = sheet_rids(&book_xml);
    let Some(rid) = rids.first() else { return Ok(None) };
    let Some(rels) = rels else { return Ok(None) };
    let rel_xml = inflate(rels.method, compressed(file, rels)?, rels.uncomp_size)?;
    let target = rel_target(&rel_xml, rid);
    Ok(target.map(|t| resolve_target(&t)))
}

fn sheet_rids(xml: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < xml.len() {
        if xml[i] != b'<' {
            i += 1;
            continue;
        }
        if let Some(n) = skip_special(xml, i) {
            i = n;
            continue;
        }
        let Some((tag, n)) = parse_tag(xml, i) else {
            i += 1;
            continue;
        };
        if !tag.closing && tag.local == b"sheet" {
            if let Some(id) = attr_value(tag.raw, b"id") {
                if !id.is_empty() {
                    out.push(String::from_utf8_lossy(&id).into_owned());
                }
            }
        }
        i = n;
    }
    out
}

fn rel_target(xml: &[u8], id: &str) -> Option<String> {
    let want = id.as_bytes();
    let mut i = 0;
    while i < xml.len() {
        if xml[i] != b'<' {
            i += 1;
            continue;
        }
        if let Some(n) = skip_special(xml, i) {
            i = n;
            continue;
        }
        let Some((tag, n)) = parse_tag(xml, i) else {
            i += 1;
            continue;
        };
        if !tag.closing && (tag.local == b"Relationship" || tag.local == b"relationship") {
            if attr_value(tag.raw, b"Id").as_deref() == Some(want) || attr_value(tag.raw, b"id").as_deref() == Some(want) {
                return attr_value(tag.raw, b"Target").or_else(|| attr_value(tag.raw, b"target")).map(|v| String::from_utf8_lossy(&v).into_owned());
            }
        }
        i = n;
    }
    None
}

/// Relationship targets are relative to `xl/workbook.xml`.
fn resolve_target(target: &str) -> String {
    let t = target.trim().replace('\\', "/");
    if let Some(rest) = t.strip_prefix('/') {
        return rest.trim_start_matches('/').to_string();
    }
    let mut parts = vec!["xl"];
    for seg in t.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            parts.pop();
            continue;
        }
        parts.push(seg);
    }
    parts.join("/")
}

fn rebuild(file: &[u8], zip: &ZipFile, changed: &[Option<Vec<u8>>]) -> Result<Vec<u8>, String> {
    let mut order: Vec<usize> = (0..zip.entries.len()).collect();
    order.sort_by_key(|&i| zip.entries[i].local_off);
    let first_off = order.first().map(|&i| zip.entries[i].local_off).unwrap_or(zip.cd_off) as usize;
    if first_off > file.len() {
        return Err("zip local header is past the end".into());
    }
    let mut out = Vec::with_capacity(file.len());
    out.extend_from_slice(&file[..first_off]);
    let mut new_off = vec![0u64; zip.entries.len()];
    for i in order {
        new_off[i] = out.len() as u64;
        let entry = &zip.entries[i];
        if let Some(xml) = &changed[i] {
            out.extend(local_bytes(entry, xml)?);
        } else {
            let start = entry.local_off as usize;
            let end = entry.span_end as usize;
            let Some(span) = file.get(start..end) else { return Err(format!("zip entry {} is truncated", entry.name)) };
            out.extend_from_slice(span);
        }
    }
    let cd_pos = out.len() as u64;
    let mut cd_index: Vec<usize> = (0..zip.entries.len()).collect();
    cd_index.sort_by_key(|&i| zip.entries[i].cd_order);
    for i in cd_index {
        let entry = &zip.entries[i];
        if let Some(xml) = &changed[i] {
            out.extend(cd_bytes(entry, xml, new_off[i])?);
        } else {
            out.extend(patch_cd_offset(&entry.cd_raw, new_off[i])?);
        }
    }
    let cd_len = out.len() as u64 - cd_pos;
    if zip.entries.len() > 0xFFFF || cd_len > u32::MAX as u64 || cd_pos > u32::MAX as u64 {
        return Err("workbook is too large to rewrite".into());
    }
    out.extend(eocd(zip.entries.len() as u16, cd_len as u32, cd_pos as u32, &zip.comment));
    Ok(out)
}

fn local_bytes(entry: &Entry, xml: &[u8]) -> Result<Vec<u8>, String> {
    let (method, data) = pack(xml);
    if data.len() > u32::MAX as usize || xml.len() > u32::MAX as usize {
        return Err(format!("{} is too large", entry.name));
    }
    let flags = entry.flags & !0x0001 & !0x0008;
    let need: u16 = if method == 0 { 10 } else { 20 };
    let mut v = Vec::with_capacity(30 + entry.name_bytes.len() + data.len());
    v.extend(0x0403_4b50u32.to_le_bytes());
    v.extend(need.to_le_bytes());
    v.extend(flags.to_le_bytes());
    v.extend(method.to_le_bytes());
    v.extend(entry.time.to_le_bytes());
    v.extend(entry.date.to_le_bytes());
    v.extend(crc32(xml).to_le_bytes());
    v.extend((data.len() as u32).to_le_bytes());
    v.extend((xml.len() as u32).to_le_bytes());
    v.extend((entry.name_bytes.len() as u16).to_le_bytes());
    v.extend(0u16.to_le_bytes());
    v.extend(&entry.name_bytes);
    v.extend(&data);
    Ok(v)
}

fn cd_bytes(entry: &Entry, xml: &[u8], offset: u64) -> Result<Vec<u8>, String> {
    if offset > u32::MAX as u64 {
        return Err("workbook is too large to rewrite".into());
    }
    let (method, data) = pack(xml);
    if data.len() > u32::MAX as usize || xml.len() > u32::MAX as usize {
        return Err(format!("{} is too large", entry.name));
    }
    let flags = entry.flags & !0x0001 & !0x0008;
    let need: u16 = if method == 0 { 10 } else { 20 };
    let mut v = Vec::with_capacity(46 + entry.name_bytes.len() + entry.comment.len());
    v.extend(0x0201_4b50u32.to_le_bytes());
    v.extend(entry.made.to_le_bytes());
    v.extend(need.to_le_bytes());
    v.extend(flags.to_le_bytes());
    v.extend(method.to_le_bytes());
    v.extend(entry.time.to_le_bytes());
    v.extend(entry.date.to_le_bytes());
    v.extend(crc32(xml).to_le_bytes());
    v.extend((data.len() as u32).to_le_bytes());
    v.extend((xml.len() as u32).to_le_bytes());
    v.extend((entry.name_bytes.len() as u16).to_le_bytes());
    v.extend(0u16.to_le_bytes());
    v.extend((entry.comment.len() as u16).to_le_bytes());
    v.extend(0u16.to_le_bytes());
    v.extend(entry.internal.to_le_bytes());
    v.extend(entry.external.to_le_bytes());
    v.extend((offset as u32).to_le_bytes());
    v.extend(&entry.name_bytes);
    v.extend(&entry.comment);
    Ok(v)
}

fn pack(xml: &[u8]) -> (u16, Vec<u8>) {
    let deflated = miniz_oxide::deflate::compress_to_vec(xml, 6);
    if deflated.len() < xml.len() { (8, deflated) } else { (0, xml.to_vec()) }
}

fn eocd(count: u16, cd_len: u32, cd_off: u32, comment: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(22 + comment.len());
    v.extend(0x0605_4b50u32.to_le_bytes());
    v.extend([0u8; 4]);
    v.extend(count.to_le_bytes());
    v.extend(count.to_le_bytes());
    v.extend(cd_len.to_le_bytes());
    v.extend(cd_off.to_le_bytes());
    v.extend((comment.len() as u16).to_le_bytes());
    v.extend(comment);
    v
}

fn patch_cd_offset(raw: &[u8], offset: u64) -> Result<Vec<u8>, String> {
    if raw.len() < 46 || u32::from_le_bytes(raw[0..4].try_into().unwrap()) != 0x0201_4b50 {
        return Err("central directory entry is truncated".into());
    }
    let mut out = raw.to_vec();
    let off32 = u32::from_le_bytes(out[42..46].try_into().unwrap());
    if off32 != 0xFFFF_FFFF {
        if offset > u32::MAX as u64 {
            return Err("workbook is too large to rewrite".into());
        }
        out[42..46].copy_from_slice(&(offset as u32).to_le_bytes());
        return Ok(out);
    }
    let name_len = u16::from_le_bytes(out[28..30].try_into().unwrap()) as usize;
    let extra_len = u16::from_le_bytes(out[30..32].try_into().unwrap()) as usize;
    let extra_at = 46 + name_len;
    let extra_end = extra_at + extra_len;
    if extra_end > out.len() {
        return Err("central directory extra field is truncated".into());
    }
    let uncomp32 = u32::from_le_bytes(out[24..28].try_into().unwrap());
    let comp32 = u32::from_le_bytes(out[20..24].try_into().unwrap());
    let disk = u16::from_le_bytes(out[34..36].try_into().unwrap());
    let mut q = extra_at;
    let mut patched = false;
    while q + 4 <= extra_end {
        let id = u16::from_le_bytes(out[q..q + 2].try_into().unwrap());
        let len = u16::from_le_bytes(out[q + 2..q + 4].try_into().unwrap()) as usize;
        let body = q + 4;
        if id == 1 {
            let mut v = body;
            if uncomp32 == 0xFFFF_FFFF {
                v += 8;
            }
            if comp32 == 0xFFFF_FFFF {
                v += 8;
            }
            if v + 8 <= body + len && v + 8 <= extra_end {
                out[v..v + 8].copy_from_slice(&offset.to_le_bytes());
                patched = true;
            }
        }
        q = body + len;
        let _ = disk;
    }
    if !patched {
        return Err("zip64 offset could not be updated".into());
    }
    Ok(out)
}

fn parse_zip(file: &[u8]) -> Result<ZipFile, String> {
    let eocd_at = find_eocd(file).ok_or_else(|| "not a zip archive".to_string())?;
    let eocd = &file[eocd_at..];
    if eocd.len() < 22 {
        return Err("not a zip archive".into());
    }
    let mut count = u16::from_le_bytes(eocd[10..12].try_into().unwrap()) as u64;
    let mut cd_len = u32::from_le_bytes(eocd[12..16].try_into().unwrap()) as u64;
    let mut cd_off = u32::from_le_bytes(eocd[16..20].try_into().unwrap()) as u64;
    let comment_len = u16::from_le_bytes(eocd[20..22].try_into().unwrap()) as usize;
    let comment = eocd.get(22..22 + comment_len).unwrap_or(&[]).to_vec();
    if count == 0xFFFF || cd_off == 0xFFFF_FFFF || cd_len == 0xFFFF_FFFF {
        let loc = eocd_at.checked_sub(20).ok_or_else(|| "zip64 locator is missing".to_string())?;
        let locb = file.get(loc..loc + 20).ok_or_else(|| "zip64 locator is missing".to_string())?;
        if u32::from_le_bytes(locb[0..4].try_into().unwrap()) != 0x0706_4b50 {
            return Err("zip64 locator is missing".into());
        }
        let zoff = u64::from_le_bytes(locb[8..16].try_into().unwrap()) as usize;
        let z = file.get(zoff..).ok_or_else(|| "zip64 record is missing".to_string())?;
        if z.len() < 56 || u32::from_le_bytes(z[0..4].try_into().unwrap()) != 0x0606_4b50 {
            return Err("zip64 record is missing".into());
        }
        count = u64::from_le_bytes(z[32..40].try_into().unwrap());
        cd_len = u64::from_le_bytes(z[40..48].try_into().unwrap());
        cd_off = u64::from_le_bytes(z[48..56].try_into().unwrap());
    }
    if cd_off as usize + cd_len as usize > file.len() {
        return Err("central directory is truncated".into());
    }
    let cd = &file[cd_off as usize..cd_off as usize + cd_len as usize];
    let mut entries = Vec::new();
    let mut p = 0usize;
    while entries.len() < count as usize {
        if p + 46 > cd.len() || u32::from_le_bytes(cd[p..p + 4].try_into().unwrap()) != 0x0201_4b50 {
            return Err("central directory is truncated".into());
        }
        let made = u16::from_le_bytes(cd[p + 4..p + 6].try_into().unwrap());
        let flags = u16::from_le_bytes(cd[p + 8..p + 10].try_into().unwrap());
        let method = u16::from_le_bytes(cd[p + 10..p + 12].try_into().unwrap());
        let time = u16::from_le_bytes(cd[p + 12..p + 14].try_into().unwrap());
        let date = u16::from_le_bytes(cd[p + 14..p + 16].try_into().unwrap());
        let mut comp = u32::from_le_bytes(cd[p + 20..p + 24].try_into().unwrap()) as u64;
        let mut uncomp = u32::from_le_bytes(cd[p + 24..p + 28].try_into().unwrap()) as u64;
        let name_len = u16::from_le_bytes(cd[p + 28..p + 30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(cd[p + 30..p + 32].try_into().unwrap()) as usize;
        let comment_len = u16::from_le_bytes(cd[p + 32..p + 34].try_into().unwrap()) as usize;
        let internal = u16::from_le_bytes(cd[p + 36..p + 38].try_into().unwrap());
        let external = u32::from_le_bytes(cd[p + 38..p + 42].try_into().unwrap());
        let mut local_off = u32::from_le_bytes(cd[p + 42..p + 46].try_into().unwrap()) as u64;
        let rec_len = 46 + name_len + extra_len + comment_len;
        if p + rec_len > cd.len() {
            return Err("central directory is truncated".into());
        }
        let name_bytes = cd[p + 46..p + 46 + name_len].to_vec();
        let extra = &cd[p + 46 + name_len..p + 46 + name_len + extra_len];
        let comment = cd[p + 46 + name_len + extra_len..p + rec_len].to_vec();
        apply_zip64(extra, &mut uncomp, &mut comp, &mut local_off);
        let name = if flags & 0x800 != 0 { String::from_utf8_lossy(&name_bytes).into_owned() } else { String::from_utf8_lossy(&name_bytes).into_owned() };
        entries.push(Entry {
            name: name.replace('\\', "/"),
            name_bytes,
            method,
            flags,
            time,
            date,
            comp_size: comp,
            uncomp_size: uncomp,
            local_off,
            span_end: 0,
            made,
            internal,
            external,
            comment,
            cd_raw: cd[p..p + rec_len].to_vec(),
            cd_order: entries.len(),
        });
        p += rec_len;
    }
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| entries[i].local_off);
    for (n, &i) in order.iter().enumerate() {
        let end = order.get(n + 1).map(|&j| entries[j].local_off).unwrap_or(cd_off);
        if end < entries[i].local_off {
            return Err("zip entries overlap".into());
        }
        entries[i].span_end = end;
    }
    Ok(ZipFile { entries, cd_off, comment })
}

fn apply_zip64(extra: &[u8], uncomp: &mut u64, comp: &mut u64, offset: &mut u64) {
    let mut q = 0;
    while q + 4 <= extra.len() {
        let id = u16::from_le_bytes(extra[q..q + 2].try_into().unwrap());
        let len = u16::from_le_bytes(extra[q + 2..q + 4].try_into().unwrap()) as usize;
        let mut v = q + 4;
        let end = (v + len).min(extra.len());
        if id == 1 {
            if *uncomp == 0xFFFF_FFFF && v + 8 <= end {
                *uncomp = u64::from_le_bytes(extra[v..v + 8].try_into().unwrap());
                v += 8;
            }
            if *comp == 0xFFFF_FFFF && v + 8 <= end {
                *comp = u64::from_le_bytes(extra[v..v + 8].try_into().unwrap());
                v += 8;
            }
            if *offset == 0xFFFF_FFFF && v + 8 <= end {
                *offset = u64::from_le_bytes(extra[v..v + 8].try_into().unwrap());
            }
        }
        q += 4 + len;
    }
}

fn find_eocd(file: &[u8]) -> Option<usize> {
    if file.len() < 22 {
        return None;
    }
    let start = file.len().saturating_sub(22 + 65535);
    let mut i = file.len() - 22;
    loop {
        if file[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] {
            let comment = u16::from_le_bytes([file[i + 20], file[i + 21]]) as usize;
            if i + 22 + comment == file.len() {
                return Some(i);
            }
        }
        if i == start {
            return None;
        }
        i -= 1;
    }
}

fn compressed<'a>(file: &'a [u8], entry: &Entry) -> Result<&'a [u8], String> {
    let off = entry.local_off as usize;
    let h = file.get(off..off + 30).ok_or_else(|| format!("{} local header is truncated", entry.name))?;
    if u32::from_le_bytes(h[0..4].try_into().unwrap()) != 0x0403_4b50 {
        return Err(format!("{} is not a zip entry", entry.name));
    }
    let name_len = u16::from_le_bytes(h[26..28].try_into().unwrap()) as usize;
    let extra_len = u16::from_le_bytes(h[28..30].try_into().unwrap()) as usize;
    let start = off + 30 + name_len + extra_len;
    let end = start + entry.comp_size as usize;
    file.get(start..end).ok_or_else(|| format!("{} is truncated", entry.name))
}

fn inflate(method: u16, data: &[u8], _uncomp: u64) -> Result<Vec<u8>, String> {
    match method {
        0 => Ok(data.to_vec()),
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, MAX_PART).map_err(|_| "could not inflate a workbook part".to_string()),
        _ => Err("unsupported zip compression".into()),
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

struct Tag<'a> {
    raw: &'a [u8],
    local: &'a [u8],
    prefix: &'a [u8],
    closing: bool,
    self_closing: bool,
}

fn edit_xml(xml: &[u8], kind: Kind, edit: &ViewEdit) -> Vec<u8> {
    let root: &[u8] = match kind {
        Kind::Worksheet => b"worksheet",
        Kind::Chartsheet => b"chartsheet",
        Kind::Workbook => b"workbook",
    };
    let need_view = kind != Kind::Workbook && (edit.a1 || edit.zoom || edit.tab == Tab::On);
    let mut out = Vec::with_capacity(xml.len() + 128);
    let mut i = 0;
    let mut depth = 0i32;
    let mut root_prefix: Vec<u8> = Vec::new();
    let mut saw_view = false;
    let mut saw_book = false;
    let mut injected = false;
    let mut sheet_n = 0usize;
    while i < xml.len() {
        if xml[i] != b'<' {
            let s = i;
            while i < xml.len() && xml[i] != b'<' {
                i += 1;
            }
            out.extend_from_slice(&xml[s..i]);
            continue;
        }
        if let Some(n) = skip_special(xml, i) {
            out.extend_from_slice(&xml[i..n]);
            i = n;
            continue;
        }
        let Some((tag, n)) = parse_tag(xml, i) else {
            out.push(xml[i]);
            i += 1;
            continue;
        };
        let is_root = tag.local == root;
        if !tag.closing && is_root && depth == 0 {
            root_prefix = tag.prefix.to_vec();
            if tag.self_closing && ((need_view && kind != Kind::Workbook) || (edit.book && kind == Kind::Workbook)) {
                out.extend(unclose(tag.raw));
                if kind == Kind::Workbook {
                    out.extend(book_views_block(&root_prefix));
                    saw_book = true;
                } else {
                    out.extend(sheet_views_block(&root_prefix, edit));
                    saw_view = true;
                }
                out.extend(close_el(&root_prefix, root));
                injected = true;
                i = n;
                continue;
            }
        }
        if !tag.closing && tag.local == b"sheetView" && kind != Kind::Workbook {
            saw_view = true;
            let (bytes, next) = rewrite_sheet_view(xml, i, edit);
            out.extend(bytes);
            i = next;
            continue;
        }
        // `<sheet>` in workbook.xml, not `<sheets>`. The first one in document order is the selected tab.
        if !tag.closing && tag.local == b"sheet" && edit.book && kind == Kind::Workbook {
            let change = if sheet_n == 0 { Change::SetOn(b"tabSelected") } else { Change::Remove(b"tabSelected") };
            sheet_n += 1;
            out.extend(patch_tag(tag.raw, &[change]));
            i = n;
            if !tag.self_closing {
                depth += 1;
            }
            continue;
        }
        if !tag.closing && tag.local == b"workbookView" && edit.book {
            saw_book = true;
            out.extend(patch_tag(tag.raw, &[Change::Set(b"activeTab", b"0"), Change::Set(b"firstSheet", b"0")]));
            i = n;
            if !tag.self_closing {
                depth += 1;
            }
            continue;
        }
        if !tag.closing && tag.local == b"sheetViews" && tag.self_closing && need_view {
            out.extend(unclose(tag.raw));
            out.extend(sheet_view_element(&prefix_of(&tag), edit));
            out.extend(close_el(tag.prefix, b"sheetViews"));
            saw_view = true;
            injected = true;
            i = n;
            continue;
        }
        if !tag.closing && tag.local == b"bookViews" && tag.self_closing && edit.book {
            out.extend(unclose(tag.raw));
            out.extend(workbook_view_element(tag.prefix));
            out.extend(close_el(tag.prefix, b"bookViews"));
            saw_book = true;
            injected = true;
            i = n;
            continue;
        }
        if tag.closing && tag.local == b"sheetViews" && !saw_view && need_view {
            out.extend(sheet_view_element(&root_prefix, edit));
            saw_view = true;
        }
        if tag.closing && tag.local == b"bookViews" && !saw_book && edit.book {
            out.extend(workbook_view_element(&root_prefix));
            saw_book = true;
        }
        let child = !tag.closing && depth == 1;
        if child && !injected && !saw_view && need_view && kind != Kind::Workbook && insert_sheet_before(tag.local) {
            out.extend(sheet_views_block(&root_prefix, edit));
            injected = true;
            saw_view = true;
        }
        if child && !injected && !saw_book && edit.book && kind == Kind::Workbook && insert_book_before(tag.local) {
            out.extend(book_views_block(&root_prefix));
            injected = true;
            saw_book = true;
        }
        if tag.closing && depth == 1 && is_root && !injected {
            if need_view && !saw_view && kind != Kind::Workbook {
                out.extend(sheet_views_block(&root_prefix, edit));
                injected = true;
            }
            if edit.book && !saw_book && kind == Kind::Workbook {
                out.extend(book_views_block(&root_prefix));
                injected = true;
            }
        }
        out.extend_from_slice(tag.raw);
        if tag.closing {
            depth -= 1;
        } else if !tag.self_closing {
            depth += 1;
        }
        i = n;
    }
    out
}

fn insert_sheet_before(local: &[u8]) -> bool {
    !matches!(local, b"sheetPr" | b"dimension" | b"sheetViews" | b"worksheet" | b"chartsheet")
}

fn insert_book_before(local: &[u8]) -> bool {
    !matches!(local, b"fileVersion" | b"fileSharing" | b"workbookPr" | b"workbookProtection" | b"bookViews" | b"workbook")
}

fn prefix_of(tag: &Tag) -> Vec<u8> {
    tag.prefix.to_vec()
}

fn rewrite_sheet_view(xml: &[u8], start: usize, edit: &ViewEdit) -> (Vec<u8>, usize) {
    let Some((tag, next)) = parse_tag(xml, start) else { return (xml[start..].to_vec(), xml.len()) };
    let mut changes = Vec::new();
    if edit.a1 {
        changes.push(Change::Set(b"topLeftCell", b"A1"));
        changes.push(Change::SetIfPresent(b"activeCell", b"A1"));
    }
    if edit.zoom {
        changes.push(Change::Set(b"zoomScale", b"100"));
        changes.push(Change::Set(b"zoomScaleNormal", b"100"));
    }
    match edit.tab {
        Tab::On => changes.push(Change::SetOn(b"tabSelected")),
        Tab::Off => changes.push(Change::Remove(b"tabSelected")),
        Tab::Leave => {}
    }
    let open = patch_tag(tag.raw, &changes);
    if tag.self_closing {
        if edit.a1 {
            let mut out = unclose(&open);
            out.extend(selection_el(tag.prefix));
            out.extend(close_el(tag.prefix, b"sheetView"));
            return (out, next);
        }
        return (open, next);
    }
    let mut out = open;
    let mut i = next;
    let mut depth = 1i32;
    let mut saw_sel = false;
    while i < xml.len() {
        if xml[i] != b'<' {
            let s = i;
            while i < xml.len() && xml[i] != b'<' {
                i += 1;
            }
            out.extend_from_slice(&xml[s..i]);
            continue;
        }
        if let Some(n) = skip_special(xml, i) {
            out.extend_from_slice(&xml[i..n]);
            i = n;
            continue;
        }
        let Some((inner, n)) = parse_tag(xml, i) else {
            out.push(xml[i]);
            i += 1;
            continue;
        };
        if inner.closing && inner.local == b"sheetView" && depth == 1 {
            if edit.a1 && !saw_sel {
                out.extend(selection_el(tag.prefix));
            }
            out.extend_from_slice(inner.raw);
            return (out, n);
        }
        if !inner.closing && inner.local == b"selection" && edit.a1 {
            saw_sel = true;
            out.extend(patch_tag(inner.raw, &[Change::Set(b"activeCell", b"A1"), Change::Set(b"sqref", b"A1")]));
            i = n;
            if !inner.self_closing {
                depth += 1;
            }
            continue;
        }
        if !inner.closing && inner.local == b"pane" && edit.a1 {
            out.extend(patch_tag(inner.raw, &[Change::Set(b"topLeftCell", b"A1")]));
            i = n;
            if !inner.self_closing {
                depth += 1;
            }
            continue;
        }
        out.extend_from_slice(inner.raw);
        if inner.closing {
            depth -= 1;
        } else if !inner.self_closing {
            depth += 1;
        }
        i = n;
    }
    (out, i)
}

fn sheet_views_block(prefix: &[u8], edit: &ViewEdit) -> Vec<u8> {
    let mut v = open_el(prefix, b"sheetViews");
    v.extend(sheet_view_element(prefix, edit));
    v.extend(close_el(prefix, b"sheetViews"));
    v
}

fn sheet_view_element(prefix: &[u8], edit: &ViewEdit) -> Vec<u8> {
    let mut open = Vec::new();
    open.push(b'<');
    push_name(&mut open, prefix, b"sheetView");
    open.extend(br#" workbookViewId="0""#);
    if edit.a1 {
        open.extend(br#" topLeftCell="A1""#);
    }
    if edit.zoom {
        open.extend(br#" zoomScale="100" zoomScaleNormal="100""#);
    }
    if edit.tab == Tab::On {
        open.extend(br#" tabSelected="1""#);
    }
    if edit.a1 {
        open.push(b'>');
        open.extend(selection_el(prefix));
        open.extend(close_el(prefix, b"sheetView"));
    } else {
        open.extend(b"/>");
    }
    open
}

fn book_views_block(prefix: &[u8]) -> Vec<u8> {
    let mut v = open_el(prefix, b"bookViews");
    v.extend(workbook_view_element(prefix));
    v.extend(close_el(prefix, b"bookViews"));
    v
}

fn workbook_view_element(prefix: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.push(b'<');
    push_name(&mut v, prefix, b"workbookView");
    v.extend(br#" activeTab="0" firstSheet="0"/>"#);
    v
}

fn selection_el(prefix: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.push(b'<');
    push_name(&mut v, prefix, b"selection");
    v.extend(br#" activeCell="A1" sqref="A1"/>"#);
    v
}

fn open_el(prefix: &[u8], local: &[u8]) -> Vec<u8> {
    let mut v = vec![b'<'];
    push_name(&mut v, prefix, local);
    v.push(b'>');
    v
}

fn close_el(prefix: &[u8], local: &[u8]) -> Vec<u8> {
    let mut v = vec![b'<', b'/'];
    push_name(&mut v, prefix, local);
    v.push(b'>');
    v
}

fn push_name(out: &mut Vec<u8>, prefix: &[u8], local: &[u8]) {
    if !prefix.is_empty() {
        out.extend_from_slice(prefix);
        out.push(b':');
    }
    out.extend_from_slice(local);
}

fn unclose(tag: &[u8]) -> Vec<u8> {
    if tag.len() < 3 {
        return tag.to_vec();
    }
    let mut p = tag.len() - 1;
    while p > 0 && is_ws(tag[p - 1]) {
        p -= 1;
    }
    if p == 0 || tag[p - 1] != b'/' {
        return tag.to_vec();
    }
    let mut v = tag[..p - 1].to_vec();
    while v.last().is_some_and(|b| is_ws(*b)) {
        v.pop();
    }
    v.push(b'>');
    v
}

enum Change {
    Set(&'static [u8], &'static [u8]),
    SetIfPresent(&'static [u8], &'static [u8]),
    SetOn(&'static [u8]),
    Remove(&'static [u8]),
}

fn patch_tag(tag: &[u8], changes: &[Change]) -> Vec<u8> {
    let mut cur = tag.to_vec();
    for change in changes {
        cur = match change {
            Change::Set(name, value) => set_attr(&cur, name, value),
            Change::SetIfPresent(name, value) => {
                if attr_value(&cur, name).is_some() { set_attr(&cur, name, value) } else { cur }
            }
            Change::SetOn(name) => set_on(&cur, name),
            Change::Remove(name) => remove_attr(&cur, name),
        };
    }
    cur
}

fn set_on(tag: &[u8], name: &[u8]) -> Vec<u8> {
    if let Some(v) = attr_value(tag, name) {
        if eq_ignore_ascii(&v, b"1") || eq_ignore_ascii(&v, b"true") {
            return tag.to_vec();
        }
    }
    set_attr(tag, name, b"1")
}

fn set_attr(tag: &[u8], name: &[u8], value: &[u8]) -> Vec<u8> {
    if let Some(found) = find_attr(tag, name) {
        if found.value == value {
            return tag.to_vec();
        }
        let mut out = Vec::with_capacity(tag.len() + value.len());
        out.extend_from_slice(&tag[..found.val_span.start]);
        out.extend_from_slice(value);
        out.extend_from_slice(&tag[found.val_span.end..]);
        return out;
    }
    let at = insert_point(tag);
    let mut out = Vec::with_capacity(tag.len() + name.len() + value.len() + 4);
    out.extend_from_slice(&tag[..at]);
    out.push(b' ');
    out.extend_from_slice(name);
    out.extend_from_slice(b"=\"");
    out.extend_from_slice(value);
    out.push(b'"');
    out.extend_from_slice(&tag[at..]);
    out
}

fn remove_attr(tag: &[u8], name: &[u8]) -> Vec<u8> {
    let Some(found) = find_attr(tag, name) else { return tag.to_vec() };
    let mut out = Vec::with_capacity(tag.len());
    out.extend_from_slice(&tag[..found.lead]);
    out.extend_from_slice(&tag[found.end..]);
    out
}

struct Found<'a> {
    value: &'a [u8],
    val_span: std::ops::Range<usize>,
    lead: usize,
    end: usize,
}

fn find_attr<'a>(tag: &'a [u8], name: &[u8]) -> Option<Found<'a>> {
    let mut i = name_end(tag);
    while i < tag.len() {
        while i < tag.len() && is_ws(tag[i]) {
            i += 1;
        }
        if i >= tag.len() || tag[i] == b'>' || tag[i] == b'/' {
            break;
        }
        let lead = {
            let mut s = i;
            while s > 0 && is_ws(tag[s - 1]) {
                s -= 1;
            }
            s
        };
        let ns = i;
        while i < tag.len() && !is_ws(tag[i]) && tag[i] != b'=' && tag[i] != b'>' && tag[i] != b'/' {
            i += 1;
        }
        let attr_name = &tag[ns..i];
        while i < tag.len() && is_ws(tag[i]) {
            i += 1;
        }
        if i >= tag.len() || tag[i] != b'=' {
            continue;
        }
        i += 1;
        while i < tag.len() && is_ws(tag[i]) {
            i += 1;
        }
        if i >= tag.len() || (tag[i] != b'"' && tag[i] != b'\'') {
            break;
        }
        let quote = tag[i];
        let vs = i + 1;
        i = vs;
        while i < tag.len() && tag[i] != quote {
            i += 1;
        }
        if i >= tag.len() {
            break;
        }
        let ve = i;
        i += 1;
        if local_name(attr_name) == name {
            return Some(Found { value: &tag[vs..ve], val_span: vs..ve, lead, end: i });
        }
    }
    None
}

fn attr_value(tag: &[u8], name: &[u8]) -> Option<Vec<u8>> {
    find_attr(tag, name).map(|f| f.value.to_vec())
}

fn name_end(tag: &[u8]) -> usize {
    let mut i = 1;
    if tag.get(i) == Some(&b'/') {
        i += 1;
    }
    while i < tag.len() && !is_ws(tag[i]) && tag[i] != b'/' && tag[i] != b'>' {
        i += 1;
    }
    i
}

fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&b| b == b':') {
        Some(p) => &name[p + 1..],
        None => name,
    }
}

fn insert_point(tag: &[u8]) -> usize {
    let mut p = tag.len().saturating_sub(1);
    while p > 0 && is_ws(tag[p - 1]) {
        p -= 1;
    }
    if p > 0 && tag[p - 1] == b'/' {
        return p - 1;
    }
    tag.len().saturating_sub(1)
}

fn parse_tag(xml: &[u8], i: usize) -> Option<(Tag<'_>, usize)> {
    if xml.get(i) != Some(&b'<') {
        return None;
    }
    let mut j = i + 1;
    let mut quote = 0u8;
    while j < xml.len() {
        let b = xml[j];
        if quote != 0 {
            if b == quote {
                quote = 0;
            }
            j += 1;
            continue;
        }
        if b == b'"' || b == b'\'' {
            quote = b;
            j += 1;
            continue;
        }
        if b == b'>' {
            j += 1;
            break;
        }
        j += 1;
    }
    if j > xml.len() || xml.get(j - 1) != Some(&b'>') {
        return None;
    }
    let raw = &xml[i..j];
    let closing = raw.get(1) == Some(&b'/');
    let mut k = if closing { 2 } else { 1 };
    let name_start = k;
    while k < raw.len() && !is_ws(raw[k]) && raw[k] != b'/' && raw[k] != b'>' {
        k += 1;
    }
    if k == name_start {
        return None;
    }
    let name = &raw[name_start..k];
    let (prefix, local) = match name.iter().position(|&c| c == b':') {
        Some(p) => (&name[..p], &name[p + 1..]),
        None => (&b""[..], name),
    };
    let self_closing = !closing && {
        let mut p = raw.len() - 1;
        while p > 0 && is_ws(raw[p - 1]) {
            p -= 1;
        }
        p > 0 && raw[p - 1] == b'/'
    };
    Some((Tag { raw, local, prefix, closing, self_closing }, j))
}

fn skip_special(xml: &[u8], i: usize) -> Option<usize> {
    let rest = xml.get(i..)?;
    if rest.starts_with(b"<!--") {
        return rest[4..].windows(3).position(|w| w == b"-->").map(|p| i + 4 + p + 3).or(Some(xml.len()));
    }
    if rest.starts_with(b"<![CDATA[") {
        return rest[9..].windows(3).position(|w| w == b"]]>").map(|p| i + 9 + p + 3).or(Some(xml.len()));
    }
    if rest.starts_with(b"<?") {
        return rest[2..].windows(2).position(|w| w == b"?>").map(|p| i + 2 + p + 2).or(Some(xml.len()));
    }
    if rest.starts_with(b"<!DOCTYPE") || (rest.starts_with(b"<!") && !rest.starts_with(b"</")) {
        return parse_tag(xml, i).map(|(_, n)| n);
    }
    None
}

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\r' || b == b'\n'
}

fn eq_ignore_ascii(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(a1: bool, zoom: bool, first: bool) -> TidyOptions {
        TidyOptions { selection_a1: a1, zoom_100: zoom, first_sheet: first }
    }

    fn all() -> TidyOptions {
        opts(true, true, true)
    }

    #[test]
    fn extension_gate_rejects_xls_and_accepts_xlsx_xlsm() {
        assert!(is_ooxml_workbook_path(r"C:\work\a.xlsx"));
        assert!(is_ooxml_workbook_path(r"C:\work\a.XLSM"));
        assert!(is_ooxml_workbook_path("book.xlsm"));
        assert!(is_ooxml_workbook_path("a.b.xlsx"));
        assert!(!is_ooxml_workbook_path(r"C:\work\a.xls"));
        assert!(!is_ooxml_workbook_path("a.xlsb"));
        assert!(!is_ooxml_workbook_path("a.xltx"));
        assert!(!is_ooxml_workbook_path("a.xlsx.txt"));
        assert!(!is_ooxml_workbook_path(".xlsx"));
        assert!(!is_ooxml_workbook_path("xlsx"));
        let ok = [(r"C:\a.xlsx".into(), true), (r"D:\b.xlsm".into(), true)];
        assert!(applies(&ok));
        let mixed = [(r"C:\a.xlsx".into(), true), (r"C:\a.xls".into(), true)];
        assert!(!applies(&mixed));
        let folder = [(r"C:\a.xlsx".into(), false)];
        assert!(!applies(&folder));
        assert!(!applies(&[]));
    }

    #[test]
    fn backup_name_keeps_the_extension() {
        assert_eq!(backup_path(Path::new(r"C:\docs\report.xlsx")).file_name().unwrap(), "report.bak.xlsx");
        assert_eq!(backup_path(Path::new("book.XLSM")).file_name().unwrap(), "book.bak.XLSM");
        assert_eq!(backup_path(Path::new(r"D:\a.b.xlsx")).file_name().unwrap(), "a.b.bak.xlsx");
    }

    #[test]
    fn share_violation_is_win32_32_or_33() {
        assert!(is_share_violation(&std::io::Error::from_raw_os_error(32)));
        assert!(is_share_violation(&std::io::Error::from_raw_os_error(33)));
        assert!(!is_share_violation(&std::io::Error::from_raw_os_error(5)));
        assert!(!is_share_violation(&std::io::Error::other("no")));
    }

    fn zip_of(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data, deflate) in files {
            let name_b = name.as_bytes();
            let body = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let method: u16 = if *deflate { 8 } else { 0 };
            let off = out.len() as u32;
            let crc = crc32(data);
            let mut local = Vec::new();
            local.extend(0x0403_4b50u32.to_le_bytes());
            local.extend(20u16.to_le_bytes());
            local.extend(0x800u16.to_le_bytes());
            local.extend(method.to_le_bytes());
            local.extend([0u8; 4]);
            local.extend(crc.to_le_bytes());
            local.extend((body.len() as u32).to_le_bytes());
            local.extend((data.len() as u32).to_le_bytes());
            local.extend((name_b.len() as u16).to_le_bytes());
            local.extend(0u16.to_le_bytes());
            local.extend(name_b);
            out.extend(local);
            out.extend(&body);
            let mut c = Vec::new();
            c.extend(0x0201_4b50u32.to_le_bytes());
            c.extend(20u16.to_le_bytes());
            c.extend(20u16.to_le_bytes());
            c.extend(0x800u16.to_le_bytes());
            c.extend(method.to_le_bytes());
            c.extend([0u8; 4]);
            c.extend(crc.to_le_bytes());
            c.extend((body.len() as u32).to_le_bytes());
            c.extend((data.len() as u32).to_le_bytes());
            c.extend((name_b.len() as u16).to_le_bytes());
            c.extend([0u8; 12]);
            c.extend(off.to_le_bytes());
            c.extend(name_b);
            cd.extend(c);
        }
        let start = out.len() as u32;
        out.extend(&cd);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((cd.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0, 0]);
        out
    }

    fn part<'a>(file: &'a [u8], name: &str) -> (u16, Vec<u8>, Vec<u8>) {
        let zip = parse_zip(file).unwrap();
        let e = zip.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)).unwrap_or_else(|| panic!("missing {name}"));
        let comp = compressed(file, e).unwrap().to_vec();
        let data = inflate(e.method, &comp, e.uncomp_size).unwrap();
        (e.method, comp, data)
    }

    fn sample() -> Vec<u8> {
        let types = br#"<?xml version="1.0" encoding="UTF-8"?><Types></Types>"#;
        let book = br#"<?xml version="1.0" encoding="UTF-8"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<bookViews><workbookView xWindow="0" yWindow="0" activeTab="1" firstSheet="1"/></bookViews>
<sheets>
<sheet name="Second" sheetId="2" r:id="rId2"/>
<sheet name="First" sheetId="1" r:id="rId1" tabSelected="1"/>
</sheets>
</workbook>"#;
        let rels = br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/vbaProject" Target="vbaProject.bin"/>
</Relationships>"#;
        let sheet1 = br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<dimension ref="A1:F20"/>
<sheetViews>
<sheetView zoomScale="85" zoomScaleNormal="85" topLeftCell="C5" workbookViewId="0" tabSelected="1">
<pane xSplit="1" ySplit="2" topLeftCell="D8" activePane="bottomRight" state="frozen"/>
<selection pane="topLeft" activeCell="B2" sqref="B2:B4"/>
<selection pane="bottomRight" activeCell="F9" sqref="F9"/>
</sheetView>
</sheetViews>
<!-- zoomScale="40" activeCell="Z9" -->
<sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData>
</worksheet>"#;
        let sheet2 = br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetViews>
<sheetView tabSelected="0" zoomScale='120' workbookViewId="0">
<selection activeCell="H20" sqref="H20:J22"/>
</sheetView>
</sheetViews>
<sheetData/>
</worksheet>"#;
        let styles = b"styles-unique-payload-do-not-touch-0123456789";
        let vba = b"\x00vba-project-\xff-intact";
        zip_of(&[
            ("[Content_Types].xml", types, false),
            ("xl/workbook.xml", book, true),
            ("xl/_rels/workbook.xml.rels", rels, false),
            ("xl/worksheets/sheet1.xml", sheet1, true),
            ("xl/worksheets/sheet2.xml", sheet2, false),
            ("xl/styles.xml", styles, true),
            ("xl/vbaProject.bin", vba, false),
        ])
    }

    #[test]
    fn tidy_sets_a1_zoom_and_the_first_sheet_and_keeps_the_rest() {
        let src = sample();
        let (styles_method, styles_comp, styles_raw) = part(&src, "xl/styles.xml");
        let (vba_method, vba_comp, vba_raw) = part(&src, "xl/vbaProject.bin");
        let (_, _, types_raw) = part(&src, "[Content_Types].xml");
        let out = tidy_package(&src, all()).unwrap();
        let (_, styles_comp2, styles_raw2) = part(&out, "xl/styles.xml");
        let (vba_method2, vba_comp2, vba_raw2) = part(&out, "xl/vbaProject.bin");
        let (_, _, types_raw2) = part(&out, "[Content_Types].xml");
        assert_eq!(styles_method, 8);
        assert_eq!(styles_comp, styles_comp2);
        assert_eq!(styles_raw, styles_raw2);
        assert_eq!(vba_method, 0);
        assert_eq!(vba_method2, 0);
        assert_eq!(vba_comp, vba_comp2);
        assert_eq!(vba_raw, vba_raw2);
        assert_eq!(types_raw, types_raw2);

        let s1 = String::from_utf8(part(&out, "xl/worksheets/sheet1.xml").2).unwrap();
        assert!(s1.contains(r#"topLeftCell="A1""#), "{s1}");
        assert!(s1.contains(r#"zoomScale="100""#), "{s1}");
        assert!(s1.contains(r#"zoomScaleNormal="100""#), "{s1}");
        assert!(s1.contains(r#"<pane xSplit="1" ySplit="2" topLeftCell="A1" activePane="bottomRight" state="frozen"/>"#), "{s1}");
        assert!(s1.contains(r#"pane="topLeft" activeCell="A1" sqref="A1""#), "{s1}");
        assert!(s1.contains(r#"pane="bottomRight" activeCell="A1" sqref="A1""#), "{s1}");
        assert!(s1.contains(r#"<!-- zoomScale="40" activeCell="Z9" -->"#), "{s1}");
        assert!(s1.contains("<v>1</v>"), "{s1}");
        assert!(!s1.contains(r#"tabSelected"#), "sheet1 is not the first tab\n{s1}");

        let s2 = String::from_utf8(part(&out, "xl/worksheets/sheet2.xml").2).unwrap();
        // The original wrote zoomScale in single quotes. The new value keeps that quote.
        assert!(s2.contains("zoomScale='100'"), "{s2}");
        assert!(s2.contains("zoomScaleNormal=\"100\""), "{s2}");
        assert!(s2.contains("topLeftCell=\"A1\""), "{s2}");
        assert!(s2.contains("activeCell=\"A1\""), "{s2}");
        assert!(s2.contains("sqref=\"A1\""), "{s2}");
        assert!(s2.contains("tabSelected=\"1\""), "sheet2 is the first tab\n{s2}");
        assert!(!s2.contains("zoomScale='120'"));

        let book = String::from_utf8(part(&out, "xl/workbook.xml").2).unwrap();
        assert!(book.contains(r#"activeTab="0""#), "{book}");
        assert!(book.contains(r#"firstSheet="0""#), "{book}");
        assert!(book.contains(r#"xWindow="0" yWindow="0""#), "{book}");
        assert!(book.contains(r#"<sheet name="Second" sheetId="2" r:id="rId2" tabSelected="1"/>"#), "{book}");
        assert!(book.contains(r#"<sheet name="First" sheetId="1" r:id="rId1"/>"#), "{book}");
        assert!(!book.contains("name=\"First\" sheetId=\"1\" r:id=\"rId1\" tabSelected"), "{book}");
    }

    #[test]
    fn each_checkbox_changes_only_its_own_part() {
        let src = sample();
        let zoom_only = tidy_package(&src, opts(false, true, false)).unwrap();
        let s1 = String::from_utf8(part(&zoom_only, "xl/worksheets/sheet1.xml").2).unwrap();
        assert!(s1.contains(r#"zoomScale="100""#) && s1.contains(r#"zoomScaleNormal="100""#), "{s1}");
        assert!(s1.contains(r#"activeCell="B2""#), "{s1}");
        assert!(s1.contains(r#"topLeftCell="C5""#), "{s1}");
        assert!(s1.contains(r#"topLeftCell="D8""#), "{s1}");
        let book = String::from_utf8(part(&zoom_only, "xl/workbook.xml").2).unwrap();
        assert!(book.contains(r#"activeTab="1""#), "{book}");
        assert!(book.contains(r#"r:id="rId1" tabSelected="1""#), "{book}");
        assert!(!book.contains(r#"r:id="rId2" tabSelected"#), "{book}");

        let move_only = tidy_package(&src, opts(true, false, false)).unwrap();
        let s1 = String::from_utf8(part(&move_only, "xl/worksheets/sheet1.xml").2).unwrap();
        assert!(s1.contains(r#"zoomScale="85""#), "{s1}");
        assert!(s1.contains(r#"activeCell="A1""#) && s1.contains(r#"topLeftCell="A1""#), "{s1}");
        assert!(!s1.contains("zoomScaleNormal=\"100\""), "{s1}");

        let first_only = tidy_package(&src, opts(false, false, true)).unwrap();
        let s1 = String::from_utf8(part(&first_only, "xl/worksheets/sheet1.xml").2).unwrap();
        let s2 = String::from_utf8(part(&first_only, "xl/worksheets/sheet2.xml").2).unwrap();
        assert!(!s1.contains("tabSelected"), "{s1}");
        assert!(s2.contains(r#"tabSelected="1""#), "{s2}");
        assert!(s1.contains(r#"activeCell="B2""#), "{s1}");
        assert!(s1.contains(r#"zoomScale="85""#), "{s1}");
        let book = String::from_utf8(part(&first_only, "xl/workbook.xml").2).unwrap();
        assert!(book.contains(r#"activeTab="0""#) && book.contains(r#"firstSheet="0""#), "{book}");
        assert!(book.contains(r#"<sheet name="Second" sheetId="2" r:id="rId2" tabSelected="1"/>"#), "{book}");
        assert!(book.contains(r#"<sheet name="First" sheetId="1" r:id="rId1"/>"#), "{book}");
    }

    #[test]
    fn missing_views_are_added_and_a_tidy_file_is_unchanged() {
        let bare = br#"<?xml version="1.0" encoding="UTF-8"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1"/><sheetData/></worksheet>"#;
        let book = br#"<?xml version="1.0"?><workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Only" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
        let rels = br#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let closed = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0" zoomScale="50"/></sheetViews><sheetData/></worksheet>"#;
        let src = zip_of(&[
            ("xl/workbook.xml", book, false),
            ("xl/_rels/workbook.xml.rels", rels, false),
            ("xl/worksheets/sheet1.xml", bare, false),
            ("xl/worksheets/sheet2.xml", closed, false),
        ]);
        // sheet2 is not in the workbook, so it is not the first sheet.
        let out = tidy_package(&src, all()).unwrap();
        let s1 = String::from_utf8(part(&out, "xl/worksheets/sheet1.xml").2).unwrap();
        assert!(s1.contains(r#"<sheetViews><sheetView workbookViewId="0" topLeftCell="A1" zoomScale="100" zoomScaleNormal="100" tabSelected="1"><selection activeCell="A1" sqref="A1"/></sheetView></sheetViews>"#), "{s1}");
        assert!(s1.contains(r#"<dimension ref="A1"/>"#), "{s1}");
        let s2 = String::from_utf8(part(&out, "xl/worksheets/sheet2.xml").2).unwrap();
        assert!(s2.contains(r#"zoomScale="100""#) && s2.contains(r#"zoomScaleNormal="100""#), "{s2}");
        assert!(s2.contains(r#"<selection activeCell="A1" sqref="A1"/>"#), "{s2}");
        assert!(!s2.contains("tabSelected"), "{s2}");
        let book = String::from_utf8(part(&out, "xl/workbook.xml").2).unwrap();
        assert!(book.contains(r#"<bookViews><workbookView activeTab="0" firstSheet="0"/></bookViews><sheets>"#), "{book}");
        assert!(book.contains(r#"<sheet name="Only" sheetId="1" r:id="rId1" tabSelected="1"/>"#), "{book}");

        let again = tidy_package(&out, all()).unwrap();
        assert_eq!(again, out);
    }

    #[test]
    fn prefixed_sheet_view_and_true_tab_selected_stay_put_when_already_right() {
        let sheet = br#"<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><x:sheetViews><x:sheetView tabSelected="true" zoomScale="100" zoomScaleNormal="100" topLeftCell="A1" workbookViewId="0"><x:selection activeCell="A1" sqref="A1"/></x:sheetView></x:sheetViews><x:sheetData/></x:worksheet>"#;
        let book = br#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><bookViews><workbookView activeTab="0" firstSheet="0"/></bookViews><sheets><sheet name="S" sheetId="1" r:id="rId1" tabSelected="true"/></sheets></workbook>"#;
        let rels = br#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let src = zip_of(&[("xl/workbook.xml", book, false), ("xl/_rels/workbook.xml.rels", rels, false), ("xl/worksheets/sheet1.xml", sheet, false)]);
        let out = tidy_package(&src, all()).unwrap();
        assert_eq!(out, src);
    }

    #[test]
    fn a_chartsheet_tab_follows_workbook_order_and_is_not_zoomed() {
        let book = br#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><bookViews><workbookView activeTab="1"/></bookViews><sheets><sheet name="Chart" sheetId="1" r:id="rId1"/><sheet name="Data" sheetId="2" r:id="rId2" tabSelected="1"/></sheets></workbook>"#;
        let rels = br#"<Relationships><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chartsheet" Target="chartsheets/chart1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let chart = br#"<chartsheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView tabSelected="0" workbookViewId="0"/></sheetViews><drawing r:id="rId1"/></chartsheet>"#;
        let sheet = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView tabSelected="1" zoomScale="80" workbookViewId="0"><selection activeCell="B2" sqref="B2"/></sheetView></sheetViews><sheetData/></worksheet>"#;
        let src = zip_of(&[
            ("xl/workbook.xml", book, false),
            ("xl/_rels/workbook.xml.rels", rels, false),
            ("xl/chartsheets/chart1.xml", chart, false),
            ("xl/worksheets/sheet1.xml", sheet, false),
        ]);
        let out = tidy_package(&src, all()).unwrap();
        let chart = String::from_utf8(part(&out, "xl/chartsheets/chart1.xml").2).unwrap();
        assert!(chart.contains(r#"tabSelected="1""#), "{chart}");
        assert!(!chart.contains("zoomScale"), "{chart}");
        assert!(!chart.contains("activeCell"), "{chart}");
        assert!(chart.contains(r#"<drawing r:id="rId1"/>"#), "{chart}");
        let sheet = String::from_utf8(part(&out, "xl/worksheets/sheet1.xml").2).unwrap();
        assert!(!sheet.contains("tabSelected"), "{sheet}");
        assert!(sheet.contains(r#"zoomScale="100""#) && sheet.contains(r#"activeCell="A1""#), "{sheet}");
        let book = String::from_utf8(part(&out, "xl/workbook.xml").2).unwrap();
        assert!(book.contains(r#"activeTab="0""#) && book.contains(r#"firstSheet="0""#), "{book}");
        assert!(book.contains(r#"name="Chart" sheetId="1" r:id="rId1" tabSelected="1""#), "{book}");
        assert!(book.contains(r#"name="Data" sheetId="2" r:id="rId2"/>"#), "{book}");
    }
}
