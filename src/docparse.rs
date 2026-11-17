use std::io::{Read, Seek, SeekFrom};
#[cfg(windows)]
use windows::Win32::Globalization::{MultiByteToWideChar, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};

// ---------- reading documents for the preview: text, CSV, ZIP, XML, mail (no Windows UI here) ----------

/// Bytes as text: a BOM decides, else UTF-8 if it is (a character cut at the end is fine), else the Korean
/// ANSI code page, which is what Windows tools write here.
pub(crate) fn decode_text(b: &[u8]) -> String {
    if let Some(r) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(r).into_owned();
    }
    if let Some(r) = b.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(r, false);
    }
    if let Some(r) = b.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(r, true);
    }
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(e) if e.error_len().is_none() => String::from_utf8_lossy(&b[..e.valid_up_to()]).into_owned(),
        Err(_) => ansi(b, 949),
    }
}

pub(crate) fn utf16(b: &[u8], be: bool) -> String {
    let u: Vec<u16> = b.chunks_exact(2).map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
    String::from_utf16_lossy(&u)
}

/// A code page's bytes (949 = Korean) as text; a lead byte cut off at the end is dropped.
#[cfg(windows)]
pub(crate) fn ansi(b: &[u8], cp: u32) -> String {
    if b.is_empty() {
        return String::new();
    }
    unsafe {
        let n = MultiByteToWideChar(cp, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), b, None);
        if n <= 0 {
            return String::from_utf8_lossy(b).into_owned();
        }
        let mut w = vec![0u16; n as usize];
        MultiByteToWideChar(cp, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), b, Some(&mut w));
        String::from_utf16_lossy(&w)
    }
}

#[cfg(not(windows))]
pub(crate) fn ansi(b: &[u8], _cp: u32) -> String { String::from_utf8_lossy(b).into_owned() }

/// Text, not a binary: no NUL in the first 8 KB (UTF-16 has a BOM, then NULs are its own).
pub(crate) fn looks_text(b: &[u8]) -> bool {
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    let head = &b[..b.len().min(8192)];
    let ctl = head.iter().filter(|&&c| c < 9 || (c > 13 && c < 32 && c != 27)).count();
    !head.contains(&0) && ctl * 50 <= head.len()
}

/// CSV / TSV rows: quoted fields may hold the separator, "" and line breaks. At most `max` rows.
pub(crate) fn csv_rows(text: &str, sep: char, max: usize) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let (mut row, mut cell, mut quoted) = (Vec::new(), String::new(), false);
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' if quoted && it.peek() == Some(&'"') => {
                cell.push('"');
                it.next();
            }
            '"' if quoted => quoted = false,
            '"' if cell.is_empty() => quoted = true,
            c if c == sep && !quoted => row.push(std::mem::take(&mut cell)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
                if rows.len() >= max {
                    return rows;
                }
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

/// ',' unless the first line has none and has ';' (European Excel).
pub(crate) fn csv_sep(text: &str) -> char {
    let first = text.lines().next().unwrap_or("");
    if !first.contains(',') && first.contains(';') { ';' } else { ',' }
}

// ---------- ZIP (central directory, stored and deflated entries) ----------

#[derive(Debug, Clone)]
pub(crate) struct ZipEntry {
    pub(crate) name: String,
    pub(crate) size: u64,
    csize: u64,
    method: u16,
    offset: u64,
}

impl ZipEntry {
    pub(crate) fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }
}

fn le16(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 2).map_or(0, |x| u16::from_le_bytes([x[0], x[1]]) as u64)
}

fn le32(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 4).map_or(0, |x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]) as u64)
}

fn le64(b: &[u8], at: usize) -> u64 {
    b.get(at..at + 8).map_or(0, |x| u64::from_le_bytes(x.try_into().unwrap()))
}

/// Every entry of a ZIP, from its central directory (ZIP64 too). None: not a ZIP.
pub(crate) fn zip_entries<R: Read + Seek>(f: &mut R) -> Option<Vec<ZipEntry>> {
    let len = f.seek(SeekFrom::End(0)).ok()?;
    let tail = len.min(65_557 + 20 + 56);
    f.seek(SeekFrom::Start(len - tail)).ok()?;
    let mut t = vec![0u8; tail as usize];
    f.read_exact(&mut t).ok()?;
    let e = (0..t.len().saturating_sub(21)).rev().find(|&i| le32(&t, i) == 0x0605_4b50)?;
    let (mut count, mut size, mut start) = (le16(&t, e + 10), le32(&t, e + 12), le32(&t, e + 16));
    // ZIP64: the locator sits right before the end record and points at the larger one
    if (count == 0xFFFF || start == 0xFFFF_FFFF) && e >= 20 && le32(&t, e - 20) == 0x0706_4b50 {
        let at = le64(&t, e - 20 + 8);
        let mut z = [0u8; 56];
        f.seek(SeekFrom::Start(at)).ok()?;
        f.read_exact(&mut z).ok()?;
        if le32(&z, 0) == 0x0606_4b50 {
            (count, size, start) = (le64(&z, 32), le64(&z, 40), le64(&z, 48));
        }
    }
    if size > 64 << 20 {
        return None;
    }
    let mut cd = vec![0u8; size as usize];
    f.seek(SeekFrom::Start(start)).ok()?;
    f.read_exact(&mut cd).ok()?;
    let mut out = Vec::new();
    let mut p = 0;
    while out.len() < count as usize && le32(&cd, p) == 0x0201_4b50 {
        let (flags, method) = (le16(&cd, p + 8), le16(&cd, p + 10) as u16);
        let (mut csize, mut usize_, mut offset) = (le32(&cd, p + 20), le32(&cd, p + 24), le32(&cd, p + 42));
        let (n, x, c) = (le16(&cd, p + 28) as usize, le16(&cd, p + 30) as usize, le16(&cd, p + 32) as usize);
        let raw = cd.get(p + 46..p + 46 + n)?;
        let name = if flags & 0x800 != 0 { String::from_utf8_lossy(raw).into_owned() } else { decode_text(raw) };
        // ZIP64 extra field: the 64-bit values, in order, of those that were 0xFFFFFFFF
        let extra = cd.get(p + 46 + n..p + 46 + n + x).unwrap_or(&[]);
        let mut q = 0;
        while q + 4 <= extra.len() {
            let (id, l) = (le16(extra, q), le16(extra, q + 2) as usize);
            if id == 1 {
                let mut v = q + 4;
                for field in [&mut usize_, &mut csize, &mut offset] {
                    if *field == 0xFFFF_FFFF {
                        *field = le64(extra, v);
                        v += 8;
                    }
                }
            }
            q += 4 + l;
        }
        out.push(ZipEntry { name: name.replace('\\', "/"), size: usize_, csize, method, offset });
        p += 46 + n + x + c;
    }
    Some(out)
}

/// One entry's bytes, at most `max` of them uncompressed. Stored and deflated only.
pub(crate) fn zip_read<R: Read + Seek>(f: &mut R, e: &ZipEntry, max: usize) -> Option<Vec<u8>> {
    let mut h = [0u8; 30];
    f.seek(SeekFrom::Start(e.offset)).ok()?;
    f.read_exact(&mut h).ok()?;
    if le32(&h, 0) != 0x0403_4b50 {
        return None;
    }
    f.seek(SeekFrom::Current((le16(&h, 26) + le16(&h, 28)) as i64)).ok()?;
    let take = if e.method == 0 { e.csize.min(max as u64) } else { e.csize.min(256 << 20) };
    let mut data = vec![0u8; take as usize];
    f.read_exact(&mut data).ok()?;
    match e.method {
        0 => Some(data),
        8 => match miniz_oxide::inflate::decompress_to_vec_with_limit(&data, max) {
            Ok(v) => Some(v),
            Err(err) if !err.output.is_empty() => Some(err.output), // over the limit: what fits
            Err(_) => None,
        },
        _ => None,
    }
}

pub(crate) fn zip_find<'a>(es: &'a [ZipEntry], name: &str) -> Option<&'a ZipEntry> {
    es.iter().find(|e| e.name.eq_ignore_ascii_case(name))
}

// ---------- XML text ----------

pub(crate) fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(j) = rest.find(';').filter(|&j| j <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..j];
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The words of an Office / HWPX / ODF XML part: character data inside any `text` element, a line break
/// after each `para` element, a tab for `tab` and a break for `br` elements. Tag names are compared whole
/// ("w:t", "a:p", "hp:t", "text:p").
pub(crate) fn xml_text(xml: &str, text: &[&str], para: &[&str], tab: &[&str], br: &[&str]) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        if depth > 0 {
            out.push_str(&unescape(&rest[..i]));
        }
        rest = &rest[i + 1..];
        let Some(j) = rest.find('>') else { break };
        let tag = &rest[..j];
        rest = &rest[j + 1..];
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        let close = tag.starts_with('/');
        let empty = tag.ends_with('/');
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        if text.contains(&name) && !empty {
            depth = if close { depth.saturating_sub(1) } else { depth + 1 };
        }
        if close && para.contains(&name) {
            out.push('\n');
        }
        if !close && tab.contains(&name) {
            out.push('\t');
        }
        if !close && br.contains(&name) {
            out.push('\n');
        }
    }
    out
}

// HWPX body in document order. Cells remain separate, including empty cells.
pub(crate) fn hwpx_body(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    let mut text_depth = 0usize;
    let mut table = 0usize;
    let mut cell = String::new();
    let mut row: Vec<String> = Vec::new();
    while let Some(i) = rest.find('<') {
        if text_depth > 0 {
            if table > 0 { cell.push_str(&unescape(&rest[..i])); } else { out.push_str(&unescape(&rest[..i])); }
        }
        rest = &rest[i + 1..];
        let Some(j) = rest.find('>') else { break };
        let tag = &rest[..j]; rest = &rest[j + 1..];
        let close = tag.starts_with('/');
        let empty = tag.trim_end().ends_with('/');
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        match (name, close) {
            ("hp:t", false) if !empty => text_depth += 1,
            ("hp:t", true) => text_depth = text_depth.saturating_sub(1),
            ("hp:tbl", false) => { if table == 0 { out.push('\n'); } table += 1; },
            ("hp:tbl", true) => { table = table.saturating_sub(1); if table == 0 { out.push('\n'); } },
            ("hp:tc", false) if table == 1 => cell.clear(),
            ("hp:tc", true) if table == 1 => row.push(cell.trim().replace(['\n', '\t'], " ").replace('|', "\\|")),
            ("hp:tr", true) if table == 1 => { out.push_str("| "); out.push_str(&row.join(" | ")); out.push_str(" |\n"); row.clear(); },
            ("hp:p", true) => { if table > 0 { if !cell.is_empty() { cell.push(' '); } } else { out.push('\n'); } },
            ("hp:tab", false) => { if table > 0 { cell.push(' '); } else { out.push('\t'); } },
            ("hp:lineBreak", false) => { if table > 0 { cell.push(' '); } else { out.push('\n'); } },
            _ => {}
        }
    }
    out
}

// HWP5 BodyText records, with cell boundaries indicated by table LIST_HEADER records.
pub(crate) fn hwp5_body(data: &[u8]) -> String {
    let mut out = String::new();
    let mut p = 0usize;
    let mut table_level: Option<usize> = None;
    let mut cell_level: Option<usize> = None;
    let mut row = None;
    while p + 4 <= data.len() {
        let h = le32(data, p) as u32; p += 4;
        let tag = h & 1023; let level = ((h >> 10) & 1023) as usize;
        let mut len = (h >> 20) as usize;
        if len == 0xfff { if p + 4 > data.len() { break; } len = le32(data, p) as usize; p += 4; }
        let Some(end) = p.checked_add(len).filter(|&end| end <= data.len()) else { break };
        let b = &data[p..end]; p = end;
        if table_level.is_some_and(|l| level < l) && tag != 77 { table_level = None; cell_level = None; row = None; out.push('\n'); }
        if tag == 77 { table_level = Some(level); cell_level = None; row = None; out.push('\n'); continue; }
        if tag == 72 && table_level.is_some_and(|l| level == l) && b.len() >= 12 {
            let r = le16(b, 10);
            if row != Some(r) { if row.is_some() { out.push_str(" |\n"); } out.push_str("| "); row = Some(r); } else { out.push_str(" | "); }
            cell_level = Some(level); continue;
        }
        if tag == 67 {
            let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0],c[1]])).collect();
            let mut v = Vec::new(); let mut i = 0;
            while i < u.len() {
                let c = u[i];
                match c {
                    9 => { v.push(9); i += 8; },
                    10 | 13 => { i += 1; },
                    0..=31 => { i += if matches!(c, 1..=8 | 11 | 12 | 14..=23) { 8 } else { 1 }; },
                    _ => { v.push(c); i += 1; }
                }
            }
            out.push_str(&String::from_utf16_lossy(&v));
            if cell_level.is_some_and(|l| level > l) { out.push(' '); } else { out.push('\n'); }
        }
    }
    out
}

#[cfg(test)]
mod hwp_tests {
    #[test]
    fn hwpx_cells_and_text() {
        let x = "<hp:p><hp:t>앞</hp:t></hp:p><hp:tbl><hp:tr><hp:tc><hp:p><hp:t>가</hp:t></hp:p></hp:tc><hp:tc><hp:p><hp:t>나</hp:t></hp:p></hp:tc></hp:tr></hp:tbl>";
        let result = super::hwpx_body(x);
        assert!(result.contains("앞") && result.contains("| 가 | 나 |"));
    }
}

/// "AB12" -> column 27 (zero-based), row 11.
pub(crate) fn cell_ref(r: &str) -> Option<(usize, usize)> {
    let letters = r.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    // a made-up reference with too many letters overflows: no cell, not a wrong one
    let col = r[..letters].chars().try_fold(0usize, |a, c| a.checked_mul(26)?.checked_add(c.to_ascii_uppercase() as usize - 'A' as usize + 1))?;
    let row: usize = r[letters..].parse().ok()?;
    (col > 0 && row > 0).then(|| (col - 1, row - 1))
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = tag[at..].find('"')?;
    Some(&tag[at..at + end])
}

/// sharedStrings.xml: each <si>'s text, runs joined.
pub(crate) fn xlsx_strings(xml: &str) -> Vec<String> {
    xml.split("<si>").skip(1).map(|si| xml_text(si.split("</si>").next().unwrap_or(""), &["t"], &[], &[], &[])).collect()
}

/// A worksheet as rows of cell text, at most `max_r` x `max_c`; shared strings looked up, inline strings read.
pub(crate) fn xlsx_rows(sheet: &str, strings: &[String], max_r: usize, max_c: usize) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for c in sheet.split("<c ").skip(1) {
        let Some(end) = c.find('>') else { continue };
        let tag = format!(" {}", &c[..end]);
        let Some((col, row)) = attr(&tag, "r").and_then(cell_ref) else { continue };
        if row >= max_r || col >= max_c {
            continue;
        }
        let body = c.split("</c>").next().unwrap_or("");
        let v = body.split("<v>").nth(1).and_then(|v| v.split("</v>").next()).map(unescape);
        let text = match attr(&tag, "t") {
            Some("s") => v.and_then(|v| v.trim().parse::<usize>().ok()).and_then(|k| strings.get(k).cloned()).unwrap_or_default(),
            Some("inlineStr") => xml_text(body, &["t"], &[], &[], &[]),
            Some("b") => match v.as_deref() {
                Some("1") => "TRUE".into(),
                _ => "FALSE".into(),
            },
            _ => v.map(|v| tidy_number(&v)).unwrap_or_default(),
        };
        if text.is_empty() {
            continue;
        }
        while rows.len() <= row {
            rows.push(Vec::new());
        }
        let r = &mut rows[row];
        while r.len() <= col {
            r.push(String::new());
        }
        r[col] = text;
    }
    rows
}

/// "0.30000000000000004" -> "0.3": Excel stores doubles; show them the way it would, roughly.
pub(crate) fn tidy_number(v: &str) -> String {
    match v.parse::<f64>() {
        Ok(x) if v.len() > 12 && x.is_finite() => {
            let s = format!("{:.10}", x);
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        _ => v.to_string(),
    }
}

// ---------- HTML and mail ----------

/// Visible text of an HTML page or mail body: scripts and styles dropped, blocks on their own lines.
pub(crate) fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let lower = |s: &str| s.to_ascii_lowercase();
    while let Some(i) = rest.find('<') {
        out.push_str(&unescape(&rest[..i]).replace(['\r', '\n'], " "));
        rest = &rest[i + 1..];
        let Some(j) = rest.find('>') else { break };
        let tag = lower(&rest[..j]);
        rest = &rest[j + 1..];
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("").to_string();
        if (name == "script" || name == "style" || name == "head") && !tag.starts_with('/') {
            let end = format!("</{name}");
            match lower(rest).find(&end) {
                Some(k) => rest = &rest[k..],
                None => break,
            }
            continue;
        }
        if matches!(name.as_str(), "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "table" | "blockquote") {
            out.push('\n');
        }
        if name == "td" || name == "th" {
            out.push('\t');
        }
        if name == "li" && !tag.starts_with('/') {
            out.push_str("• ");
        }
    }
    out.push_str(&unescape(rest));
    // trim each line, at most one empty line in a row
    let mut lines: Vec<&str> = Vec::new();
    for l in out.lines().map(str::trim) {
        if !(l.is_empty() && lines.last().is_none_or(|p| p.is_empty())) {
            lines.push(l);
        }
    }
    lines.join("\n").trim().to_string()
}

fn b64(s: &str) -> Vec<u8> {
    let v = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let bits: Vec<u8> = s.bytes().filter_map(v).collect();
    bits.chunks(4)
        .flat_map(|c| {
            let n = c.iter().fold(0u32, |a, &x| a << 6 | x as u32) << (6 * (4 - c.len()));
            let b = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
            b.into_iter().take(c.len().saturating_sub(1))
        })
        .collect()
}

fn qp(b: &[u8], header: bool) -> Vec<u8> {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'=' if b.get(i + 1) == Some(&b'\r') || b.get(i + 1) == Some(&b'\n') => {
                i += if b.get(i + 1) == Some(&b'\r') { 3 } else { 2 }; // soft line break
                continue;
            }
            b'=' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 3;
                    continue;
                }
                _ => out.push(b'='),
            },
            b'_' if header => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    out
}

fn charset_text(b: &[u8], charset: &str) -> String {
    let cs = charset.trim().trim_matches('"').to_ascii_lowercase();
    match cs.as_str() {
        "euc-kr" | "ks_c_5601-1987" | "cp949" | "x-windows-949" | "ks_c_5601" => ansi(b, 949),
        "iso-2022-jp" | "shift_jis" | "shift-jis" => ansi(b, 932),
        "gb2312" | "gbk" | "gb18030" => ansi(b, 936),
        "big5" => ansi(b, 950),
        "iso-8859-1" | "latin1" | "windows-1252" | "us-ascii" | "ascii" => ansi(b, 1252),
        _ => decode_text(b),
    }
}

/// "=?UTF-8?B?7JWI64WV?= there" -> "안녕 there".
pub(crate) fn mime_words(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    let mut last_word = false;
    while let Some(i) = rest.find("=?") {
        let gap = &rest[..i];
        if !(last_word && gap.trim().is_empty()) {
            out.push_str(gap);
        }
        let w = &rest[i + 2..];
        let parts: Vec<&str> = w.splitn(3, '?').collect();
        let Some(end) = parts.get(2).and_then(|t| t.find("?=")) else {
            out.push_str("=?");
            rest = w;
            last_word = false;
            continue;
        };
        let (cs, enc, text) = (parts[0], parts[1], &parts[2][..end]);
        let raw = if enc.eq_ignore_ascii_case("b") { b64(text) } else { qp(&latin(text), true) };
        out.push_str(&charset_text(&raw, cs.split('*').next().unwrap_or(cs)));
        rest = &parts[2][end + 2..];
        last_word = true;
    }
    out.push_str(rest);
    out
}

fn split_head(m: &str) -> (&str, &str) {
    for sep in ["\r\n\r\n", "\n\n"] {
        if let Some(i) = m.find(sep) {
            return (&m[..i], &m[i + sep.len()..]);
        }
    }
    (m, "")
}

/// Unfolded header lines as (lowercase name, value).
fn headers(h: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for l in h.lines() {
        if (l.starts_with(' ') || l.starts_with('\t')) && !out.is_empty() {
            let last = out.last_mut().unwrap();
            last.1.push(' ');
            last.1.push_str(l.trim());
        } else if let Some((k, v)) = l.split_once(':') {
            out.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    out
}

fn param(v: &str, name: &str) -> Option<String> {
    v.split(';').skip(1).find_map(|p| {
        let (k, val) = p.split_once('=')?;
        k.trim().eq_ignore_ascii_case(name).then(|| val.trim().trim_matches('"').to_string())
    })
}

// Real mail nests multiparts 2 to 4 deep. A file nested thousands deep would run the preview worker's stack
// out (that ends the whole process, nothing unwinds), so parts below this many multiparts are not read.
const MIME_DEPTH: usize = 16;

/// The readable body of a MIME part: text/plain first, else HTML as text; attachments' names listed.
/// depth: multiparts above this part (the message itself is 0).
fn mime_body(head: &[(String, String)], body: &str, files: &mut Vec<String>, depth: usize) -> Option<(bool, String)> {
    let get = |k: &str| head.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let ctype = get("content-type").unwrap_or("text/plain");
    let kind = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let disp = get("content-disposition").unwrap_or("");
    if let Some(name) = param(disp, "filename").or_else(|| param(ctype, "name")) {
        if disp.to_ascii_lowercase().starts_with("attachment") || !kind.starts_with("text/") {
            files.push(mime_words(&name));
            return None;
        }
    }
    if kind.starts_with("multipart/") {
        if depth >= MIME_DEPTH {
            return None;
        }
        let b = param(ctype, "boundary")?;
        let mark = format!("--{b}");
        let mut best: Option<(bool, String)> = None;
        for part in body.split(&mark).skip(1) {
            if part.starts_with("--") {
                break;
            }
            let (h, b) = split_head(part.trim_start_matches(['\r', '\n']));
            if let Some((plain, t)) = mime_body(&headers(h), b, files, depth + 1) {
                // plain beats html; the first of each kind wins
                if best.as_ref().is_none_or(|(p, _)| !p && plain) {
                    best = Some((plain, t));
                }
            }
        }
        return best;
    }
    if !kind.starts_with("text/") {
        return None;
    }
    let raw = match get("content-transfer-encoding").map(|e| e.to_ascii_lowercase()) {
        Some(e) if e == "base64" => b64(body),
        Some(e) if e == "quoted-printable" => qp(&latin(body), false),
        _ => latin(body),
    };
    let text = charset_text(&raw, &param(ctype, "charset").unwrap_or_default());
    match kind.as_str() {
        "text/html" => Some((false, html_text(&text))),
        _ => Some((true, text.trim_end().to_string())),
    }
}

/// An .eml as readable text: Subject, From, To, Cc, Date, attachments, then the body.
pub(crate) fn eml_text(raw: &[u8], ko: bool) -> String {
    // headers are ASCII (or should be); bodies decode by their own charset, so read as Latin-1 to keep bytes
    let m: String = raw.iter().map(|&b| b as char).collect();
    let (h, body) = split_head(&m);
    let head = headers(h);
    let mut out = String::new();
    let label = |en: &'static str, k: &'static str| if ko { k } else { en };
    for (key, en, k) in [("subject", "Subject", "제목"), ("from", "From", "보낸 사람"), ("to", "To", "받는 사람"), ("cc", "Cc", "참조"), ("date", "Date", "날짜")] {
        if let Some((_, v)) = head.iter().find(|(n, _)| n == key) {
            out += &format!("{}: {}\n", label(en, k), mime_words(&latin_bytes_as_text(v)));
        }
    }
    let mut files = Vec::new();
    let text = mime_body(&head, body, &mut files, 0).map(|(_, t)| t).unwrap_or_default();
    if !files.is_empty() {
        out += &format!("{}: {}\n", label("Attachments", "첨부"), files.join(", "));
    }
    out.push('\n');
    out.push_str(&text);
    out
}

// Back to the bytes of text read one byte per char (eml_text).
fn latin(s: &str) -> Vec<u8> {
    s.chars().map(|c| c as u32 as u8).collect()
}

// A header value read byte-per-char: raw UTF-8 (allowed by newer mailers) back to text.
fn latin_bytes_as_text(v: &str) -> String {
    decode_text(&latin(v))
}
