// ---------- batch rename: the pure part (rules -> new names, checks, move order, presets) ----------
// No Win32 here, so all of it is covered by the tests at the bottom. The window is rename.rs.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Kind {
    Replace, // f0 find, f1 replace with; x match case, y regular expression
    Insert,  // f0 text, f1 position; mode start / end / at position
    Remove,  // f0 count, f1 position; mode counted from start / from end
    Number,  // f0 start, f1 step, f2 digits (empty: as many as the count needs), f3 separator; mode before / after / instead of the name
    Date,    // f0 pattern (yyyy yy MM dd HH mm ss), f1 separator; mode as Number; x photos by their date taken
    Case,    // mode lower / UPPER / Title
    Trim,    // spaces at both ends and doubled ones
}

pub(crate) const KINDS: [Kind; 7] = [Kind::Replace, Kind::Insert, Kind::Remove, Kind::Number, Kind::Date, Kind::Case, Kind::Trim];

const WORDS: [&str; 7] = ["replace", "insert", "remove", "number", "date", "case", "trim"];

// One rule. The fields are kept as typed, so the editor shows what the user wrote; numbers are read when applied.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Rule {
    pub(crate) kind: Kind,
    pub(crate) on: bool,
    pub(crate) f: [String; 4],
    pub(crate) mode: u8,
    pub(crate) x: bool,
    pub(crate) y: bool,
}

impl Rule {
    pub(crate) fn new(kind: Kind) -> Rule {
        let f = |a: &str, b: &str, c: &str, d: &str| [a.into(), b.into(), c.into(), d.into()];
        let f = match kind {
            Kind::Number => f("1", "1", "", "_"),
            Kind::Date => f("yyyyMMdd", "_", "", ""),
            Kind::Remove => f("1", "0", "", ""),
            Kind::Insert => f("", "0", "", ""),
            _ => f("", "", "", ""),
        };
        Rule { kind, on: true, f, mode: 0, x: false, y: false }
    }
}

// What a rule needs of each file besides its name.
#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) struct Src {
    pub(crate) name: String,
    pub(crate) is_dir: bool,
    pub(crate) date: [u16; 6], // modified, local: year, month, day, hour, minute, second
    pub(crate) taken: Option<[u16; 6]>, // a photo's date taken, once read (only when a date rule asks for it)
}

fn num<T: std::str::FromStr + Default>(s: &str) -> T {
    s.trim().parse().unwrap_or_default()
}

/// "photo.JPG" -> ("photo", ".JPG"). Folders, names without a dot and ".gitignore" keep it all in the stem.
pub(crate) fn split_ext(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(k) if k > 0 && !is_dir => name.split_at(k),
        _ => (name, ""),
    }
}

// Case-insensitive plain replace, char by char (lowercasing a whole string can change its byte offsets).
fn replace_plain(s: &str, find: &str, with: &str, case: bool) -> String {
    if find.is_empty() {
        return s.to_string();
    }
    let same = |a: char, b: char| if case { a == b } else { a.to_lowercase().eq(b.to_lowercase()) };
    let (hay, pat): (Vec<char>, Vec<char>) = (s.chars().collect(), find.chars().collect());
    let mut out = String::new();
    let mut i = 0;
    while i < hay.len() {
        if i + pat.len() <= hay.len() && hay[i..i + pat.len()].iter().zip(&pat).all(|(&a, &b)| same(a, b)) {
            out.push_str(with);
            i += pat.len();
        } else {
            out.push(hay[i]);
            i += 1;
        }
    }
    out
}

fn title_case(s: &str) -> String {
    let mut out = String::new();
    let mut start = true;
    for c in s.chars() {
        if start {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
        start = !c.is_alphanumeric() && c != '\'';
    }
    out
}

/// yyyyMMdd-style pattern; anything that is not a known token is copied as it is.
pub(crate) fn format_date(pat: &str, d: [u16; 6]) -> String {
    let toks: [(&str, String); 7] = [
        ("yyyy", format!("{:04}", d[0])),
        ("yy", format!("{:02}", d[0] % 100)),
        ("MM", format!("{:02}", d[1])),
        ("dd", format!("{:02}", d[2])),
        ("HH", format!("{:02}", d[3])),
        ("mm", format!("{:02}", d[4])),
        ("ss", format!("{:02}", d[5])),
    ];
    let mut out = String::new();
    let mut rest = pat;
    'next: while !rest.is_empty() {
        for (t, v) in &toks {
            if let Some(r) = rest.strip_prefix(t) {
                out += v;
                rest = r;
                continue 'next;
            }
        }
        let c = rest.chars().next().unwrap_or_default();
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn place(name: &str, add: &str, sep: &str, mode: u8) -> String {
    match mode {
        1 => format!("{name}{sep}{add}"),
        2 => add.to_string(),
        _ => format!("{add}{sep}{name}"),
    }
}

/// k-th item's number. Digits left empty: as many as the largest number in the batch needs.
pub(crate) fn number_text(r: &Rule, k: usize, count: usize) -> String {
    let (start, step): (i64, i64) = (num(&r.f[0]), if r.f[1].trim().is_empty() { 1 } else { num(&r.f[1]) });
    let v = start.saturating_add(step.saturating_mul(k as i64));
    let last = start.saturating_add(step.saturating_mul(count.saturating_sub(1) as i64));
    let width = match r.f[2].trim() {
        "" => start.unsigned_abs().max(last.unsigned_abs()).to_string().len(),
        w => num::<usize>(w).min(20),
    };
    let digits = format!("{:0width$}", v.unsigned_abs());
    if v < 0 { format!("-{digits}") } else { digits }
}

fn chars_at(s: &str, k: usize) -> usize {
    s.char_indices().nth(k).map_or(s.len(), |(b, _)| b)
}

// One rule on one name part. Regexes are built once per batch (re: one per rule, None when not one).
fn apply_one(r: &Rule, re: Option<&regex_lite::Regex>, s: &str, src: &Src, k: usize, count: usize) -> String {
    let len = s.chars().count();
    match r.kind {
        Kind::Replace => match re {
            Some(re) => re.replace_all(s, r.f[1].as_str()).into_owned(),
            None => replace_plain(s, &r.f[0], &r.f[1], r.x),
        },
        Kind::Insert => {
            let at = match r.mode {
                0 => 0,
                1 => len,
                _ => num::<usize>(&r.f[1]).min(len),
            };
            let b = chars_at(s, at);
            format!("{}{}{}", &s[..b], r.f[0], &s[b..])
        }
        Kind::Remove => {
            let (n, pos) = (num::<usize>(&r.f[0]), num::<usize>(&r.f[1]));
            let (from, to) = match r.mode {
                0 => (pos.min(len), pos.saturating_add(n).min(len)),
                _ => (len.saturating_sub(pos).saturating_sub(n), len.saturating_sub(pos)),
            };
            format!("{}{}", &s[..chars_at(s, from)], &s[chars_at(s, to)..])
        }
        Kind::Number => place(s, &number_text(r, k, count), &r.f[3], r.mode),
        Kind::Date => {
            let d = if r.x { crate::organize::pick_date(src.taken, src.date) } else { src.date };
            place(s, &format_date(&r.f[0], d), &r.f[1], r.mode)
        }
        Kind::Case => match r.mode {
            0 => s.to_lowercase(),
            1 => s.to_uppercase(),
            _ => title_case(s),
        },
        Kind::Trim => s.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" "),
    }
}

/// New names for every item, in order. Err: a regular expression that doesn't compile (its message).
pub(crate) fn new_names(rules: &[Rule], ext_too: bool, items: &[Src]) -> Result<Vec<String>, String> {
    let mut res = Vec::new();
    for r in rules {
        res.push(match (r.on, r.kind, r.y) {
            (true, Kind::Replace, true) if !r.f[0].is_empty() => {
                Some(regex_lite::RegexBuilder::new(&r.f[0]).case_insensitive(!r.x).build().map_err(|e| e.to_string())?)
            }
            _ => None,
        });
    }
    let n = items.len();
    Ok(items
        .iter()
        .enumerate()
        .map(|(k, it)| {
            let (stem, ext) = if ext_too { (it.name.as_str(), "") } else { split_ext(&it.name, it.is_dir) };
            let mut s = stem.to_string();
            for (r, re) in rules.iter().zip(&res).filter(|(r, _)| r.on) {
                s = apply_one(r, re.as_ref(), &s, it, k, n);
            }
            s + ext
        })
        .collect())
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Status {
    Same,
    Ok,
    Empty,
    BadChar,
    Reserved,
    TrailingDot, // Windows drops a trailing space or dot, so the file would not get this name
    TooLong,
    Duplicate,
    Exists,
}

impl Status {
    pub(crate) fn blocks(self) -> bool {
        !matches!(self, Status::Same | Status::Ok)
    }
}

/// Why `name` can't be a Windows file name, or None.
pub(crate) fn invalid(name: &str) -> Option<Status> {
    if name.is_empty() {
        return Some(Status::Empty);
    }
    if name.chars().any(|c| (c as u32) < 32 || "<>:\"/\\|?*".contains(c)) {
        return Some(Status::BadChar);
    }
    if name.ends_with([' ', '.']) {
        return Some(Status::TrailingDot);
    }
    if name.encode_utf16().count() > 255 {
        return Some(Status::TooLong);
    }
    if reserved_name(name) {
        return Some(Status::Reserved);
    }
    None
}

/// A device name Windows keeps for itself (CON, PRN, AUX, NUL, CONIN$, CONOUT$, COM1-9, LPT1-9), also before a
/// '.' or with spaces before it ("con .txt"). Every place that names a file or folder asks this one.
pub(crate) fn reserved_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    // COM1-9 / LPT1-9, and the superscript ¹ ² ³ forms Windows reserves as well (by string: they're multi-byte)
    let numbered = |p: &str| base.strip_prefix(p).is_some_and(|d| matches!(d, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"));
    ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&base.as_str()) || numbered("COM") || numbered("LPT")
}

// Item in a batch: its folder (index), current name, new name.
pub(crate) type Move = (usize, String, String);

fn key(dir: usize, name: &str) -> (usize, String) {
    (dir, name.to_lowercase())
}

/// Status per item. existing(dir, lowercased name): a file of that name is in the folder now.
/// A target that is another batch item's current name is fine: that one moves away first (see plan).
pub(crate) fn check(items: &[Move], existing: &dyn Fn(usize, &str) -> bool) -> Vec<Status> {
    let sources: HashSet<(usize, String)> = items.iter().map(|(d, o, _)| key(*d, o)).collect();
    let mut seen: HashMap<(usize, String), usize> = HashMap::new();
    for (d, _, n) in items {
        *seen.entry(key(*d, n)).or_default() += 1;
    }
    items
        .iter()
        .map(|(d, o, n)| {
            if n == o {
                return if seen[&key(*d, n)] > 1 { Status::Duplicate } else { Status::Same };
            }
            if let Some(bad) = invalid(n) {
                return bad;
            }
            let k = key(*d, n);
            if seen[&k] > 1 {
                Status::Duplicate
            } else if k != key(*d, o) && !sources.contains(&k) && existing(*d, &k.1) {
                Status::Exists
            } else {
                Status::Ok
            }
        })
        .collect()
}

/// Order of single renames for a checked batch (only items whose name changes). An item whose target is
/// another item's current name goes through a temporary name, so swaps and cycles work:
/// those first move aside, then the direct ones, then the parked ones to their names.
pub(crate) fn plan(items: &[Move], tmp: &dyn Fn(usize) -> String) -> Vec<Move> {
    let sources: HashSet<(usize, String)> = items.iter().map(|(d, o, _)| key(*d, o)).collect();
    let parked = |(d, o, n): &Move| key(*d, n) != key(*d, o) && sources.contains(&key(*d, n));
    let (mut first, mut direct, mut last) = (Vec::new(), Vec::new(), Vec::new());
    for (k, m) in items.iter().enumerate().filter(|(_, (_, o, n))| o != n) {
        if parked(m) {
            first.push((m.0, m.1.clone(), tmp(k)));
            last.push((m.0, tmp(k), m.2.clone()));
        } else {
            direct.push(m.clone());
        }
    }
    first.into_iter().chain(direct).chain(last).collect()
}

/// The temporary names of one batch: `nonce` keeps them apart from any other batch's.
pub(crate) fn tmp_name(nonce: u128) -> impl Fn(usize) -> String {
    move |k| format!("~grids-rename-{nonce}-{k}.tmp")
}

/// Runs `steps` in order through `mv(step, back)`. When one fails, the ones done are moved back, newest first.
/// With a journal (`stop`) that ends at the first move back that fails too, so the disk is always "the first k
/// steps done" (what recover_plan reads); without one every move back is still tried, as nothing tidies later.
/// Err: the step that failed, why, and how many are not put back (0: all were; with `stop` the first that many).
pub(crate) fn run_steps_with(steps: &[Move], stop: bool, mv: &mut dyn FnMut(&Move, bool) -> Result<(), String>) -> Result<(), (String, String, usize)> {
    for (k, m) in steps.iter().enumerate() {
        if let Err(e) = mv(m, false) {
            let left = if stop {
                steps[..k].iter().rposition(|s| mv(s, true).is_err()).map_or(0, |j| j + 1)
            } else {
                steps[..k].iter().rev().filter(|s| mv(s, true).is_err()).count()
            };
            return Err((m.1.clone(), e, left));
        }
    }
    Ok(())
}

// ---------- a batch cut off half-way (drive pulled, power gone): its journal and how to finish it ----------
// Written next to grids.ini before a batch that uses temporary names, removed once it is over.

/// Journal text: a version line with the nonce, the folders, the steps, and an end line (a cut-off write has none).
/// Tabs and line breaks can't be in a Windows name or path, so they separate safely.
pub(crate) fn journal_text(nonce: u128, dirs: &[String], steps: &[Move]) -> String {
    let mut t = format!("grids-rename 1 {nonce}\n");
    for d in dirs {
        t += &format!("D\t{d}\n");
    }
    for (d, from, to) in steps {
        t += &format!("S\t{d}\t{from}\t{to}\n");
    }
    t + "E\n"
}

/// None for anything not whole and of this version: such a file is left alone.
pub(crate) fn parse_journal(t: &str) -> Option<(u128, Vec<String>, Vec<Move>)> {
    let mut lines = t.strip_suffix("E\n")?.lines();
    let nonce = lines.next()?.strip_prefix("grids-rename 1 ")?.parse().ok()?;
    let (mut dirs, mut steps) = (Vec::new(), Vec::new());
    for l in lines {
        match l.split('\t').collect::<Vec<_>>()[..] {
            ["D", d] if steps.is_empty() && !d.is_empty() => dirs.push(d.to_string()),
            ["S", d, from, to] if !from.is_empty() && !to.is_empty() => {
                steps.push((d.parse().ok().filter(|&d: &usize| d < dirs.len())?, from.to_string(), to.to_string()))
            }
            _ => return None,
        }
    }
    Some((nonce, dirs, steps))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mend {
    Back,    // cut off while moving aside: the temporary names go back to the old ones, nothing else was touched
    Forward, // later: the batch is finished
}

/// What to do about a journal's batch, with the disk as it is now (exists: a name in a folder, any case).
/// The temporary names there tell how far it got: only the first ones = moving aside, only the last ones = the
/// final moves, all = in between. Moving aside is put back; later the rest runs, the direct renames only if it
/// got no further (once the final moves began they are done, and doing one again could move another file).
/// A move runs only if its file is there and its target free; a temporary name whose target is taken is left
/// (`stuck`). None: not steps this module plans (left alone). Mend None: no temporary name is there.
pub(crate) fn recover_plan(steps: &[Move], nonce: u128, exists: &dyn Fn(usize, &str) -> bool) -> Option<(Option<Mend>, Vec<Move>, Vec<Move>)> {
    let pre = format!("~grids-rename-{nonce}-");
    let is_tmp = |n: &str| n.starts_with(&pre) && n.ends_with(".tmp");
    let p = steps.iter().take_while(|(_, _, to)| is_tmp(to)).count();
    if p == 0 || steps.len() < 2 * p {
        return None;
    }
    let (first, rest) = steps.split_at(p);
    let (direct, last) = rest.split_at(rest.len() - p);
    let paired = first.iter().zip(last).all(|(a, b)| a.0 == b.0 && a.2 == b.1 && !is_tmp(&a.1) && !is_tmp(&b.2));
    if !paired || direct.iter().any(|(_, f, t)| is_tmp(f) || is_tmp(t)) {
        return None;
    }
    let has: Vec<bool> = first.iter().map(|(d, _, t)| exists(*d, t)).collect();
    let (mend, todo): (Mend, Vec<Move>) = match (has[0], has[p - 1]) {
        _ if !has.contains(&true) => return Some((None, Vec::new(), Vec::new())),
        (true, false) => (Mend::Back, first.iter().rev().map(|(d, o, t)| (*d, t.clone(), o.clone())).collect()),
        (true, true) => (Mend::Forward, direct.iter().chain(last).cloned().collect()),
        _ => (Mend::Forward, last.to_vec()),
    };
    let mut disk: HashMap<(usize, String), bool> = HashMap::new();
    let (mut moves, mut stuck) = (Vec::new(), Vec::new());
    for m in todo {
        let (d, from, to) = &m;
        let mut here = |n: &str| *disk.entry(key(*d, n)).or_insert_with(|| exists(*d, n));
        if !here(from) {
            continue; // done already
        }
        // a case-only rename (k -> K) finds its own file under the target: run anyway (done already, it changes nothing)
        if key(*d, from) != key(*d, to) && here(to) {
            if is_tmp(from) {
                stuck.push(m);
            }
            continue;
        }
        disk.insert(key(*d, from), false);
        disk.insert(key(*d, to), true);
        moves.push(m);
    }
    Some((Some(mend), moves, stuck))
}

// ---------- presets: one grids.ini line each, rename_preset=name|ext|rule|rule ... ----------

/// Also writes whitespace at either end of a field as %XX (per UTF-8 byte): grids.ini values are trimmed on
/// load, so a numbering separator like " " in the last field would otherwise be lost. Inner spaces stay readable.
fn esc(s: &str) -> String {
    let lead = s.len() - s.trim_start().len();
    let tail = s.trim_end().len().max(lead);
    let mut o = String::new();
    for (i, c) in s.char_indices() {
        if matches!(c, '%' | '|' | ',' | '\n' | '\r') || (c.is_whitespace() && (i < lead || i >= tail)) {
            for b in c.encode_utf8(&mut [0; 4]).bytes() {
                o += &format!("%{b:02X}");
            }
        } else {
            o.push(c);
        }
    }
    o
}

fn unesc(s: &str) -> String {
    let mut o = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match (b[i], s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                o.push(v);
                i += 3;
            }
            (c, _) => {
                o.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&o).into_owned()
}

pub(crate) fn preset_text(name: &str, ext_too: bool, rules: &[Rule]) -> String {
    let mut o = format!("{}|{}", esc(name), ext_too as u8);
    for r in rules {
        let w = WORDS[KINDS.iter().position(|&k| k == r.kind).unwrap_or(0)];
        let f: Vec<String> = r.f.iter().map(|x| esc(x)).collect();
        o += &format!("|{w},{},{},{},{},{}", r.on as u8, r.mode, r.x as u8, r.y as u8, f.join(","));
    }
    o
}

/// (name, extension included, rules). Rules of a kind this Grids doesn't know are left out.
pub(crate) fn parse_preset(v: &str) -> Option<(String, bool, Vec<Rule>)> {
    let mut it = v.split('|');
    let name = unesc(it.next()?.trim());
    if name.trim().is_empty() {
        return None;
    }
    let ext = it.next()? == "1";
    let rules = it
        .filter_map(|r| {
            let p: Vec<&str> = r.split(',').collect();
            let w0 = *p.first()?;
            let kind = KINDS[WORDS.iter().position(|w| *w == w0)?];
            let get = |k: usize| p.get(k).copied().unwrap_or("");
            let f = [unesc(get(5)), unesc(get(6)), unesc(get(7)), unesc(get(8))];
            Some(Rule { kind, on: get(1) != "0", mode: num::<u8>(get(2)).min(2), x: get(3) == "1", y: get(4) == "1", f })
        })
        .collect();
    Some((name, ext, rules))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(n: &str) -> Src {
        Src { name: n.into(), is_dir: false, date: [2024, 3, 7, 9, 5, 2], taken: None }
    }
    fn rule(kind: Kind, f: [&str; 4], mode: u8) -> Rule {
        Rule { kind, on: true, f: f.map(String::from), mode, x: false, y: false }
    }
    fn run(rules: &[Rule], ext: bool, names: &[&str]) -> Vec<String> {
        new_names(rules, ext, &names.iter().map(|n| src(n)).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn rules() {
        // replace: case-insensitive by default, extension untouched unless included
        let r = rule(Kind::Replace, ["IMG", "Photo", "", ""], 0);
        assert_eq!(run(&[r.clone()], false, &["img_1.img", "IMG2.png"]), ["Photo_1.img", "Photo2.png"]);
        assert_eq!(run(&[Rule { x: true, ..r.clone() }], false, &["img_1.img"]), ["img_1.img"]);
        assert_eq!(run(&[r.clone()], true, &["img.img"]), ["Photo.Photo"]);
        // regex with groups; a bad one is an error, a switched-off one is ignored
        let re = Rule { y: true, ..rule(Kind::Replace, [r"(\d+)-(\d+)", "$2-$1", "", ""], 0) };
        assert_eq!(run(&[re.clone()], false, &["a 12-34.txt"]), ["a 34-12.txt"]);
        let bad = Rule { y: true, ..rule(Kind::Replace, ["(", "", "", ""], 0) };
        assert!(new_names(&[bad.clone()], false, &[src("a")]).is_err());
        assert_eq!(run(&[Rule { on: false, ..bad }], false, &["a"]), ["a"]);
        // insert / remove
        assert_eq!(run(&[rule(Kind::Insert, ["x", "", "", ""], 0)], false, &["ab.c"]), ["xab.c"]);
        assert_eq!(run(&[rule(Kind::Insert, ["x", "", "", ""], 1)], false, &["ab.c"]), ["abx.c"]);
        assert_eq!(run(&[rule(Kind::Insert, ["x", "1", "", ""], 2)], false, &["가나.c", "a.c"]), ["가x나.c", "ax.c"]);
        assert_eq!(run(&[rule(Kind::Remove, ["2", "1", "", ""], 0)], false, &["abcd.e", "a.e"]), ["ad.e", "a.e"]);
        assert_eq!(run(&[rule(Kind::Remove, ["2", "0", "", ""], 1)], false, &["abcd.e"]), ["ab.e"]);
        assert_eq!(run(&[rule(Kind::Remove, ["1", "1", "", ""], 1)], false, &["abcd.e"]), ["abd.e"]);
        assert_eq!(run(&[rule(Kind::Remove, ["9", "0", "", ""], 1)], false, &["ab.e"]), [".e"]);
        // case, trim, and the order of rules matters
        assert_eq!(run(&[rule(Kind::Case, ["", "", "", ""], 2)], false, &["hello wORLD-again.TXT"]), ["Hello World-Again.TXT"]);
        assert_eq!(run(&[rule(Kind::Case, ["", "", "", ""], 1)], true, &["a.txt"]), ["A.TXT"]);
        assert_eq!(run(&[rule(Kind::Trim, ["", "", "", ""], 0)], false, &["  a   b .txt"]), ["a b.txt"]);
        let two = [rule(Kind::Replace, ["a", "b", "", ""], 0), rule(Kind::Replace, ["b", "c", "", ""], 0)];
        assert_eq!(run(&two, false, &["a.txt"]), ["c.txt"]);
        // date
        assert_eq!(run(&[rule(Kind::Date, ["yyyy-MM-dd", " ", "", ""], 0)], false, &["x.jpg"]), ["2024-03-07 x.jpg"]);
        assert_eq!(format_date("yyMMdd_HHmmss é", [2024, 3, 7, 9, 5, 2]), "240307_090502 é");
        // photos by their date taken: with one, without one (the date modified), and the option off
        let shot = Src { taken: Some([2026, 11, 28, 14, 30, 15]), ..src("IMG_1.jpg") };
        let blank = Src { taken: Some([0; 6]), ..src("IMG_2.jpg") };
        let pat = rule(Kind::Date, ["yyyy-MM-dd HH.mm.ss", "", "", ""], 2);
        let taken = Rule { x: true, ..pat.clone() };
        let items = [shot.clone(), blank, src("a.txt")];
        assert_eq!(new_names(&[taken.clone()], false, &items).unwrap(), ["2026-11-28 14.30.15.jpg", "2024-03-07 09.05.02.jpg", "2024-03-07 09.05.02.txt"]);
        assert_eq!(new_names(&[pat], false, &[shot]).unwrap(), ["2024-03-07 09.05.02.jpg"]);
        // through a preset and back (the x place every rule has); an older line (x = 0) reads as before
        let t = preset_text("p", false, &[taken.clone()]);
        assert_eq!(t, "p|0|date,1,2,1,0,yyyy-MM-dd HH.mm.ss,,,");
        assert_eq!(parse_preset(&t), Some(("p".into(), false, vec![taken])));
        assert_eq!(parse_preset("p|0|date,1,0,0,0,yyyyMMdd,_,,"), Some(("p".into(), false, vec![Rule::new(Kind::Date)])));
        // folders and dotfiles keep their whole name as the stem
        assert_eq!(split_ext("v1.2", true), ("v1.2", ""));
        assert_eq!(split_ext(".gitignore", false), (".gitignore", ""));
        assert_eq!(split_ext("a.tar.gz", false), ("a.tar", ".gz"));
    }

    #[test]
    fn numbering() {
        let n = |f: [&str; 4], mode| rule(Kind::Number, f, mode);
        let names: Vec<&str> = (0..12).map(|_| "f.jpg").collect();
        // digits left empty: as wide as the largest number
        let out = run(&[n(["1", "1", "", "_"], 0)], false, &names);
        assert_eq!((out[0].as_str(), out[11].as_str()), ("01_f.jpg", "12_f.jpg"));
        assert_eq!(run(&[n(["10", "5", "3", " - "], 1)], false, &["a.x", "b.x"]), ["a - 010.x", "b - 015.x"]);
        assert_eq!(run(&[n(["7", "", "0", ""], 2)], false, &["a.x", "b.x"]), ["7.x", "8.x"]);
        assert_eq!(run(&[n(["1", "-1", "2", ""], 2)], false, &["a", "b", "c"]), ["01", "00", "-01"]);
    }

    #[test]
    fn checks() {
        for (n, s) in [("", Status::Empty), ("a?b", Status::BadChar), ("a\tb", Status::BadChar), ("a.", Status::TrailingDot), ("a ", Status::TrailingDot)] {
            assert_eq!(invalid(n), Some(s), "{n}");
        }
        for n in ["con", "CON.txt", "Lpt3.log", "nul .x", "com9", "conin$", "CONOUT$.txt", "com¹", "LPT³.log"] {
            assert_eq!(invalid(n), Some(Status::Reserved), "{n}");
        }
        for n in ["console", "com0", "com10.txt", "a.b", "가.txt"] {
            assert_eq!(invalid(n), None, "{n}");
        }
        assert_eq!(invalid(&"a".repeat(256)), Some(Status::TooLong));
        let m = |v: &[(&str, &str)]| v.iter().map(|(o, n)| (0, o.to_string(), n.to_string())).collect::<Vec<Move>>();
        let disk = ["a", "b", "c", "other"];
        let exists = |_: usize, n: &str| disk.contains(&n);
        use Status::*;
        // swap is fine; a name another file already has is not; two items to one name are duplicates (any case)
        assert_eq!(check(&m(&[("a", "b"), ("b", "a")]), &exists), [Ok, Ok]);
        assert_eq!(check(&m(&[("a", "Other")]), &exists), [Exists]);
        assert_eq!(check(&m(&[("a", "x"), ("b", "X")]), &exists), [Duplicate, Duplicate]);
        // an unchanged item still holds its name
        assert_eq!(check(&m(&[("a", "c"), ("c", "c")]), &exists), [Duplicate, Duplicate]);
        // case-only rename of itself
        assert_eq!(check(&m(&[("a", "A"), ("b", "b")]), &exists), [Ok, Same]);
        // same name in another folder is no clash
        let two = vec![(0, "a".to_string(), "x".to_string()), (1, "a".to_string(), "x".to_string())];
        assert_eq!(check(&two, &|_, _| false), [Ok, Ok]);
        assert!(Exists.blocks() && Reserved.blocks() && !Same.blocks() && !Ok.blocks());
    }

    // Runs a plan on a fake folder: every step must find its source and not clobber anything.
    fn simulate(start: &[&str], steps: &[Move]) -> Vec<String> {
        let mut disk: Vec<String> = start.iter().map(|s| s.to_string()).collect();
        for (_, from, to) in steps {
            let i = disk.iter().position(|d| d == from).expect("source missing");
            assert!(!disk.iter().enumerate().any(|(k, d)| k != i && d.eq_ignore_ascii_case(to)), "{to} would be overwritten");
            disk[i] = to.clone();
        }
        disk.sort();
        disk
    }

    #[test]
    fn cycles() {
        let m = |v: &[(&str, &str)]| v.iter().map(|(o, n)| (0, o.to_string(), n.to_string())).collect::<Vec<Move>>();
        let tmp = |k: usize| format!("~t{k}");
        // swap, 3-cycle, chain (b->c first, then a->b), case-only and unchanged items
        let cases: [(&[&str], &[(&str, &str)], &[&str]); 4] = [
            (&["a", "b"], &[("a", "b"), ("b", "a")], &["a", "b"]),
            (&["1", "2", "3"], &[("1", "2"), ("2", "3"), ("3", "1")], &["1", "2", "3"]),
            (&["a", "b", "z"], &[("a", "b"), ("b", "c")], &["b", "c", "z"]),
            (&["a", "k"], &[("a", "A"), ("k", "k")], &["A", "k"]),
        ];
        for (disk, moves, want) in cases {
            let p = plan(&m(moves), &tmp);
            assert_eq!(simulate(disk, &p), want.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        }
        // only what needs it is parked; nothing for unchanged items
        let p = plan(&m(&[("a", "b"), ("b", "c"), ("k", "k")]), &tmp);
        assert_eq!(p, m(&[("a", "~t0"), ("b", "c"), ("~t0", "b")]));
        // numbering a whole folder in place: 1..3 -> shifted
        let p = plan(&m(&[("01", "02"), ("02", "03"), ("03", "04")]), &tmp);
        assert_eq!(simulate(&["01", "02", "03"], &p), ["02", "03", "04"]);
    }

    // A disk of (folder, lowercased name) -> file id; a move onto a name already there is a bug.
    fn apply(disk: &mut HashMap<(usize, String), usize>, (d, from, to): &Move) {
        let f = disk.remove(&key(*d, from)).unwrap_or_else(|| panic!("{from} missing"));
        assert!(disk.insert(key(*d, to), f).is_none(), "{to} taken");
    }

    // Every point a batch can stop at (the first k steps done) comes back to the old names or the new ones.
    fn mend_all(items: &[Move], others: &[(usize, &str)]) {
        let nonce = 4242;
        let steps = plan(items, &tmp_name(nonce));
        let parked = steps.len() - items.iter().filter(|(_, o, n)| o != n).count();
        let mut start: HashMap<(usize, String), usize> = items.iter().enumerate().map(|(i, (d, o, _))| (key(*d, o), i)).collect();
        start.extend(others.iter().enumerate().map(|(i, (d, n))| (key(*d, n), 1000 + i)));
        let mut end = start.clone();
        end.retain(|_, f| *f >= 1000);
        end.extend(items.iter().enumerate().map(|(i, (d, _, n))| (key(*d, n), i)));
        if parked == 0 {
            assert_eq!(recover_plan(&steps, nonce, &|_, _| true), None);
            return;
        }
        for k in 0..=steps.len() {
            let mut disk = start.clone();
            steps[..k].iter().for_each(|s| apply(&mut disk, s));
            let snap = disk.clone();
            let (mend, moves, stuck) = recover_plan(&steps, nonce, &|d, n| snap.contains_key(&key(d, n))).unwrap();
            moves.iter().for_each(|m| apply(&mut disk, m));
            assert!(stuck.is_empty(), "{k} {items:?}");
            assert!(!disk.keys().any(|(_, n)| n.starts_with("~grids-rename-")), "{k} {items:?}");
            // nothing done yet (k = 0) stays as it was, with nothing to mend
            let want = if k < parked { (&start, if k == 0 { None } else { Some(Mend::Back) }) } else { (&end, if k == steps.len() { None } else { Some(Mend::Forward) }) };
            assert_eq!((&disk, mend), want, "{k} {items:?}");
        }
    }

    #[test]
    fn journal_mends_every_stop() {
        let m = |v: &[(usize, &str, &str)]| v.iter().map(|(d, o, n)| (*d, o.to_string(), n.to_string())).collect::<Vec<Move>>();
        // swap, 3-cycle, chain, parked and direct mixed, two folders, nothing parked
        mend_all(&m(&[(0, "a", "b"), (0, "b", "a")]), &[]);
        mend_all(&m(&[(0, "1", "2"), (0, "2", "3"), (0, "3", "1")]), &[(0, "x")]);
        mend_all(&m(&[(0, "a", "b"), (0, "b", "c"), (0, "c", "d")]), &[]);
        mend_all(&m(&[(0, "a", "b"), (0, "b", "a"), (0, "k", "K"), (0, "p", "q"), (1, "a", "b"), (1, "b", "z")]), &[(1, "y")]);
        mend_all(&m(&[(0, "a", "x"), (1, "b", "y")]), &[]);
        // random batches: names drawn from a small pool per folder, so swaps, cycles and chains come up
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) % n) as usize
        };
        for _ in 0..300 {
            let mut items = Vec::new();
            for d in 0..1 + next(2) {
                let mut pool: Vec<String> = (0..8).map(|k| format!("f{k}")).collect();
                let count = 1 + next(6);
                let mut targets = pool.clone();
                for _ in 0..count {
                    let o = pool.remove(next(pool.len() as u64));
                    let n = targets.remove(next(targets.len() as u64));
                    items.push((d, o, n));
                }
            }
            mend_all(&items, &[]);
        }
    }

    #[test]
    fn journal_mend_leaves_taken_names() {
        let steps = plan(&[(0, "a".into(), "b".into()), (0, "b".into(), "a".into())], &tmp_name(7));
        // all parked, "a" moved on, then some other file took "b": that temporary name stays, listed
        let there = ["~grids-rename-7-0.tmp", "~grids-rename-7-1.tmp", "b"];
        let (mend, moves, stuck) = recover_plan(&steps, 7, &|_, n| there.contains(&n)).unwrap();
        assert_eq!(mend, Some(Mend::Forward));
        assert_eq!(moves, [(0, "~grids-rename-7-1.tmp".to_string(), "a".to_string())]);
        assert_eq!(stuck, [(0, "~grids-rename-7-0.tmp".to_string(), "b".to_string())]);
        // cut off after moving aside: the case-only rename (k -> K) still runs, though "K" is found (it is "k")
        let steps = plan(&[(0, "a".into(), "b".into()), (0, "b".into(), "a".into()), (0, "k".into(), "K".into())], &tmp_name(7));
        let there = ["~grids-rename-7-0.tmp", "~grids-rename-7-1.tmp", "k"];
        let (_, moves, stuck) = recover_plan(&steps, 7, &|_, n| there.iter().any(|t| t.eq_ignore_ascii_case(n))).unwrap();
        assert!(moves.contains(&(0, "k".to_string(), "K".to_string())) && stuck.is_empty());
        // another batch's names, or steps not of this shape: not touched
        assert_eq!(recover_plan(&steps, 8, &|_, _| true), None);
        assert_eq!(recover_plan(&steps[1..], 7, &|_, _| true), None);
        assert_eq!(recover_plan(&[(0, "a".into(), "b".into())], 7, &|_, _| true), None);
    }

    #[test]
    fn journal_round_trip() {
        let dirs = vec![r"C:\My files\a.b".to_string(), r"\\srv\공유 폴더\사진".to_string(), r"D:\".to_string()];
        let steps: Vec<Move> = vec![(0, "x y.txt".into(), "~grids-rename-5-0.tmp".into()), (1, "사진 .jpg".into(), "..a".into()), (2, "~grids-rename-5-0.tmp".into(), "é b".into())];
        let t = journal_text(5, &dirs, &steps);
        assert_eq!(parse_journal(&t), Some((5, dirs.clone(), steps.clone())));
        // cut off, another version, a step in a folder not listed, junk: none of them read
        assert_eq!(parse_journal(&t[..t.len() - 2]), None);
        assert_eq!(parse_journal(&t.replace("grids-rename 1 ", "grids-rename 2 ")), None);
        assert_eq!(parse_journal("grids-rename 1 5\nS\t0\ta\tb\nE\n"), None);
        assert_eq!(parse_journal("grids-rename 1 5\nD\tC:\\\nX\nE\n"), None);
        assert_eq!(parse_journal(""), None);
    }

    #[test]
    fn put_back_stops_at_first_failure() {
        let steps: Vec<Move> = (0..5).map(|k| (0, format!("s{k}"), format!("t{k}"))).collect();
        let mut seen = Vec::new();
        // step 3 fails; putting back step 1 fails: steps 0 and 1 stay done, step 0 is not tried
        let r = run_steps_with(&steps, true, &mut |m, back| {
            seen.push((m.1.clone(), back));
            if (m.1 == "s3" && !back) || (m.1 == "s1" && back) { Err("no".into()) } else { Ok(()) }
        });
        assert_eq!(r, Err(("s3".to_string(), "no".to_string(), 2)));
        assert_eq!(seen.last(), Some(&("s1".to_string(), true)));
        assert!(!seen.contains(&("s0".to_string(), true)));
        // everything put back: 0 left; no failure: Ok
        // without a journal every move back is tried: steps 2..0, only step 1 not put back
        seen.clear();
        let r = run_steps_with(&steps, false, &mut |m, back| {
            seen.push((m.1.clone(), back));
            if (m.1 == "s3" && !back) || (m.1 == "s1" && back) { Err("no".into()) } else { Ok(()) }
        });
        assert_eq!(r, Err(("s3".to_string(), "no".to_string(), 1)));
        assert_eq!(seen.last(), Some(&("s0".to_string(), true)));
        // everything put back: 0 left; no failure: Ok
        for stop in [true, false] {
            assert_eq!(run_steps_with(&steps, stop, &mut |m, back| if m.1 == "s2" && !back { Err("x".into()) } else { Ok(()) }), Err(("s2".into(), "x".into(), 0)));
            assert_eq!(run_steps_with(&steps, stop, &mut |_, _| Ok(())), Ok(()));
        }
    }

    #[test]
    fn presets() {
        let mut r = rule(Kind::Replace, ["a|b,c%", "x\ny", "", ""], 0);
        r.y = true;
        let rules = vec![r, Rule { on: false, ..rule(Kind::Number, ["1", "2", "3", ","], 2) }, rule(Kind::Trim, ["", "", "", ""], 0)];
        let t = preset_text("My|set", true, &rules);
        assert!(!t.contains('\n'));
        assert_eq!(parse_preset(&t), Some(("My|set".to_string(), true, rules)));
        assert_eq!(parse_preset("x|0|nope,1|case,1,1,0,0"), Some(("x".into(), false, vec![rule(Kind::Case, ["", "", "", ""], 1)])));
        assert_eq!(parse_preset(" |1"), None);
        assert_eq!(parse_preset("solo"), None);
        // kept in grids.ini, never in a named session file
        let s = crate::Session { rn_presets: vec![t.clone()], ..Default::default() };
        assert_eq!(crate::Session::parse(&s.to_ini()).rn_presets, [t]);
        assert!(!s.snapshot().contains("rename_preset"));
    }

    // Through grids.ini and back, as on the next start: values there are trimmed on load.
    fn reload(t: &str) -> Option<(String, bool, Vec<Rule>)> {
        let s = crate::Session { rn_presets: vec![t.to_string()], ..Default::default() };
        let back = crate::Session::parse(&s.to_ini()).rn_presets;
        assert_eq!(back, [t]);
        parse_preset(&back[0])
    }

    #[test]
    fn presets_keep_edge_spaces() {
        // a space-like separator in the last field (numbering last) survives a restart
        for sep in [" ", " - ", "\u{3000}", "\t", " \t"] {
            let rules = vec![rule(Kind::Number, ["1", "1", "2", sep], 1)];
            let t = preset_text("p", false, &rules);
            assert_eq!(t, t.trim(), "{sep:?}");
            let (_, _, back) = reload(&t).unwrap();
            assert_eq!(back, rules, "{sep:?}");
            assert_eq!(run(&back, false, &["Photo"]), [format!("Photo{sep}01")]);
        }
        // leading text of an insert, a lone space to replace with, a space in the name's inner part
        let rules = vec![rule(Kind::Insert, [" (copy)", "", "", ""], 1), rule(Kind::Replace, ["_", " ", "", ""], 0)];
        assert_eq!(reload(&preset_text("a b", true, &rules)), Some(("a b".into(), true, rules)));
        // fields without edge spaces are written exactly as before, so older lines and files match
        let r = vec![rule(Kind::Replace, ["IMG_", "P_", "", ""], 0)];
        assert_eq!(preset_text("사진 정리", false, &r), "사진 정리|0|replace,1,0,0,0,IMG_,P_,,");
        assert_eq!(preset_text("x", false, &[rule(Kind::Replace, ["a b", "c d", "", ""], 0)]), "x|0|replace,1,0,0,0,a b,c d,,");
        // lines written by older versions read the same
        assert_eq!(reload("x|0|number,1,1,0,0,1,1,,_"), Some(("x".into(), false, vec![rule(Kind::Number, ["1", "1", "", "_"], 1)])));
        assert_eq!(reload("x|0|replace,1,0,0,0,a b,c d,,"), Some(("x".into(), false, vec![rule(Kind::Replace, ["a b", "c d", "", ""], 0)])));
        // multi-byte spaces go per UTF-8 byte
        assert_eq!(esc("\u{3000}a\u{3000}b\u{3000}"), "%E3%80%80a\u{3000}b%E3%80%80");
        assert_eq!(unesc(&esc(" \u{a0} ")), " \u{a0} ");
    }
}
