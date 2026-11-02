// ---------- batch rename: the pure part (rules -> new names, checks, move order, presets) ----------
// No Win32 here, so all of it is covered by the tests at the bottom. The window is rename.rs.

use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Kind {
    Replace, // f0 find, f1 replace with; x match case, y regular expression
    Insert,  // f0 text, f1 position; mode start / end / at position
    Remove,  // f0 count, f1 position; mode counted from start / from end
    Number,  // f0 start, f1 step, f2 digits (empty: as many as the count needs), f3 separator; mode before / after / instead of the name
    Date,    // f0 pattern (yyyy yy MM dd HH mm ss), f1 separator; mode as Number
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
        Kind::Date => place(s, &format_date(&r.f[0], src.date), &r.f[1], r.mode),
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
    let base = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let numbered = |p: &str| base.strip_prefix(p).is_some_and(|d| d.len() == 1 && ("1"..="9").contains(&d));
    if ["CON", "PRN", "AUX", "NUL"].contains(&base.as_str()) || numbered("COM") || numbered("LPT") {
        return Some(Status::Reserved);
    }
    None
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

// ---------- presets: one grids.ini line each, rename_preset=name|ext|rule|rule ... ----------

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '%' | '|' | ',' | '\n' | '\r' => o += &format!("%{:02X}", c as u32),
            c => o.push(c),
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
        Src { name: n.into(), is_dir: false, date: [2024, 3, 7, 9, 5, 2] }
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
        for n in ["con", "CON.txt", "Lpt3.log", "nul .x", "com9"] {
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
}
