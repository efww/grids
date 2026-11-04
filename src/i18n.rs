use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use windows::Win32::Globalization::GetLocaleInfoW;

use super::*;
pub(crate) use crate::langkey::Key;
use crate::langkey::{EN, KEY_COUNT, KEY_ID};

// Resolved UI language for the built-in font slot: 0 English, 1 Korean, 2 Japanese.
// The preference (including Auto and any other language file) lives on the session.
pub(crate) static UI: AtomicU8 = AtomicU8::new(0);

const KO_INI: &str = include_str!("../lang/lang.ko.ini");
const JA_INI: &str = include_str!("../lang/lang.ja.ini");
const FR_INI: &str = include_str!("../lang/lang.fr.ini");
const DE_INI: &str = include_str!("../lang/lang.de.ini");

static TABLE: AtomicPtr<&'static str> = AtomicPtr::new(std::ptr::null_mut());
static FACE_PTR: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static FACE_LEN: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LangId {
    buf: [u8; 16],
    len: u8,
}

impl LangId {
    fn new(code: &str) -> Option<Self> {
        let b = code.as_bytes();
        if b.is_empty() || b.len() > 16 {
            return None;
        }
        let mut buf = [0u8; 16];
        buf[..b.len()].copy_from_slice(b);
        Some(LangId { buf, len: b.len() as u8 })
    }

    pub(crate) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("en")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Lang {
    Auto,
    En,
    Ko,
    Ja,
    Other(LangId),
}

impl Lang {
    pub(crate) fn from_ini(v: &str) -> Self {
        let v = v.trim().to_ascii_lowercase();
        match v.as_str() {
            "en" => Lang::En,
            "ko" => Lang::Ko, // also the old on/off value
            "ja" => Lang::Ja,
            "auto" | "" => Lang::Auto,
            other if iso_code(other) => Lang::Other(LangId::new(other).unwrap_or(LangId { buf: [0; 16], len: 0 })),
            _ => Lang::Auto,
        }
    }

    /// Value stored in maguro.json. `auto`, `en`, `ko`, `ja`, or another ISO code.
    pub(crate) fn ini(self) -> Cow<'static, str> {
        match self {
            Lang::Auto => Cow::Borrowed("auto"),
            Lang::En => Cow::Borrowed("en"),
            Lang::Ko => Cow::Borrowed("ko"),
            Lang::Ja => Cow::Borrowed("ja"),
            Lang::Other(id) => Cow::Owned(id.as_str().to_string()),
        }
    }

    /// Font slot. Korean and Japanese keep their faces. Everything else uses the Segoe UI slot
    /// unless the language file names a face.
    pub(crate) fn resolve(self) -> u8 {
        match self {
            Lang::Ko => 1,
            Lang::Ja => 2,
            Lang::En | Lang::Other(_) => 0,
            Lang::Auto => {
                let prim = unsafe { GetUserDefaultUILanguage() } & 0x3FF;
                if prim == 0x12 {
                    1
                } else if prim == 0x11 {
                    2
                } else {
                    0
                }
            }
        }
    }

    /// ISO code of the language file to load. Auto matches the Windows UI language, then English.
    pub(crate) fn resolved_code(self) -> String {
        match self {
            Lang::En => "en".into(),
            Lang::Ko => "ko".into(),
            Lang::Ja => "ja".into(),
            Lang::Other(id) => id.as_str().to_string(),
            Lang::Auto => auto_code(),
        }
    }
}

pub(crate) fn ui() -> u8 {
    UI.load(Ordering::Relaxed)
}

/// UI text. The active table is built at startup and when the language changes.
/// A missing translation is the English string compiled into the binary.
pub(crate) fn tr(k: Key) -> &'static str {
    #[cfg(test)]
    {
        return match ui() {
            1 => bundled_slot("ko", k),
            2 => bundled_slot("ja", k),
            _ => EN[k as usize],
        };
    }
    #[cfg(not(test))]
    {
        live_slot(k)
    }
}

/// Replace `{name}` placeholders. Used for sentences whose Korean and Japanese wording
/// is not the English word order.
pub(crate) fn tf(k: Key, pairs: &[(&str, &str)]) -> String {
    let mut s = tr(k).to_string();
    for (name, value) in pairs {
        let needle = format!("{{{name}}}");
        s = s.replace(&needle, value);
    }
    s
}

/// Face used when no `font_face` is chosen and the language file does not name one.
pub(crate) fn default_face(lang: u8) -> &'static str {
    match lang {
        1 => "Malgun Gothic",
        2 => "Yu Gothic UI",
        _ => "Segoe UI",
    }
}

pub(crate) fn ui_face() -> &'static str {
    #[cfg(test)]
    {
        return default_face(ui());
    }
    #[cfg(not(test))]
    {
        active_face()
    }
}

/// Title face and weight. Malgun Gothic and Yu Gothic UI have no semibold, so they use bold.
/// Segoe UI uses Semibold. Any other face named in a language file is drawn bold.
pub(crate) fn title_style() -> (&'static str, i32) {
    match ui_face() {
        "Malgun Gothic" => ("Malgun Gothic", 700),
        "Yu Gothic UI" => ("Yu Gothic UI", 700),
        "Segoe UI" => ("Segoe UI Semibold", 600),
        other => (other, 700),
    }
}

struct Meta {
    name: String,
    font: String,
}

fn embedded(code: &str) -> Option<&'static str> {
    match code {
        "ko" => Some(KO_INI),
        "ja" => Some(JA_INI),
        "fr" => Some(FR_INI),
        "de" => Some(DE_INI),
        _ => None,
    }
}

fn lang_dir() -> PathBuf {
    crate::store::config_dir().join("lang")
}

fn iso_code(s: &str) -> bool {
    let mut parts = s.split('-');
    let lang = parts.next().unwrap_or("");
    let region = parts.next();
    if parts.next().is_some() {
        return false;
    }
    let lang_ok = (2..=3).contains(&lang.len()) && lang.bytes().all(|b| b.is_ascii_lowercase());
    let region_ok = match region {
        None => true,
        Some(r) => (2..=8).contains(&r.len()) && r.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
    };
    lang_ok && region_ok
}

/// `lang.ko.ini` → `ko`. `lang.ko.user.ini` and `bundled.ver` are not language files.
fn code_from_filename(name: &str) -> Option<String> {
    let rest = name.strip_prefix("lang.")?.strip_suffix(".ini")?;
    if rest.ends_with(".user") {
        return None;
    }
    let low = rest.to_ascii_lowercase();
    iso_code(&low).then_some(low)
}

/// Bundled codes are always offered, even before the files are written out.
pub(crate) fn available_codes() -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for code in ["de", "fr", "ja", "ko"] {
        set.insert((*code).to_string());
    }
    if let Ok(rd) = std::fs::read_dir(lang_dir()) {
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            if let Some(code) = code_from_filename(&name) {
                set.insert(code);
            }
        }
    }
    set.into_iter().collect()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// `key = text`. A leading BOM is ignored. `#` and `;` lines are comments.
/// Inside `[meta]`, only `name` and `font` are kept. Every other key is a string.
fn parse_ini(text: &str) -> (Meta, HashMap<String, String>) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut meta = Meta { name: String::new(), font: String::new() };
    let mut map = HashMap::new();
    let mut section = String::new();
    for raw in text.split('\n') {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        if let Some(body) = trimmed.strip_prefix('[') {
            if let Some(name) = body.strip_suffix(']') {
                section = name.trim().to_ascii_lowercase();
                continue;
            }
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let key = k.trim();
        if key.is_empty() {
            continue;
        }
        let val = unescape(v.strip_prefix(' ').unwrap_or(v));
        if section == "meta" {
            match key {
                "name" => meta.name = val,
                "font" => meta.font = val,
                _ => {}
            }
        } else {
            map.insert(key.to_string(), val);
        }
    }
    (meta, map)
}

fn overlay(meta: &mut Meta, map: &mut HashMap<String, String>, text: &str) {
    let (m, extra) = parse_ini(text);
    if !m.name.is_empty() {
        meta.name = m.name;
    }
    if !m.font.is_empty() {
        meta.font = m.font;
    }
    map.extend(extra);
}

fn read_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Disk `lang.xx.ini` replaces the embedded file when it exists. `lang.xx.user.ini` wins either way.
/// English starts from the strings in code, then `lang.en.ini`, then `lang.en.user.ini`.
fn merged(code: &str, dir: &Path, embedded_only: bool) -> (Meta, HashMap<String, String>) {
    let mut meta = Meta { name: String::new(), font: String::new() };
    let mut map = HashMap::new();
    if code == "en" {
        if !embedded_only {
            if let Some(text) = read_file(&dir.join("lang.en.ini")) {
                overlay(&mut meta, &mut map, &text);
            }
            if let Some(text) = read_file(&dir.join("lang.en.user.ini")) {
                overlay(&mut meta, &mut map, &text);
            }
        }
        return (meta, map);
    }
    if embedded_only {
        if let Some(text) = embedded(code) {
            overlay(&mut meta, &mut map, text);
        }
        return (meta, map);
    }
    let disk = read_file(&dir.join(format!("lang.{code}.ini")));
    if let Some(text) = disk.as_deref() {
        overlay(&mut meta, &mut map, text);
    } else if let Some(text) = embedded(code) {
        overlay(&mut meta, &mut map, text);
    }
    if let Some(text) = read_file(&dir.join(format!("lang.{code}.user.ini"))) {
        overlay(&mut meta, &mut map, &text);
    }
    (meta, map)
}

fn slots_of(map: &HashMap<String, String>) -> Vec<&'static str> {
    let mut out = Vec::with_capacity(KEY_COUNT);
    for i in 0..KEY_COUNT {
        if let Some(v) = map.get(KEY_ID[i]) {
            out.push(Box::leak(v.clone().into_boxed_str()) as &'static str);
        } else {
            out.push(EN[i]);
        }
    }
    out
}

#[cfg(test)]
fn bundled_slot(code: &str, k: Key) -> &'static str {
    use std::sync::OnceLock;
    static KO: OnceLock<Vec<&'static str>> = OnceLock::new();
    static JA: OnceLock<Vec<&'static str>> = OnceLock::new();
    let table = match code {
        "ko" => KO.get_or_init(|| slots_of(&merged("ko", Path::new(""), true).1)),
        "ja" => JA.get_or_init(|| slots_of(&merged("ja", Path::new(""), true).1)),
        _ => return EN[k as usize],
    };
    table.get(k as usize).copied().unwrap_or(EN[k as usize])
}

fn install_table(slots: Vec<&'static str>) {
    let ptr = Box::leak(slots.into_boxed_slice()).as_mut_ptr();
    TABLE.store(ptr, Ordering::Release);
}

#[cfg(not(test))]
fn live_slot(k: Key) -> &'static str {
    let p = TABLE.load(Ordering::Acquire);
    if p.is_null() {
        return EN[k as usize];
    }
    unsafe { *p.add(k as usize) }
}

fn set_face(face: &'static str) {
    FACE_PTR.store(face.as_ptr() as *mut u8, Ordering::Release);
    FACE_LEN.store(face.len(), Ordering::Release);
}

#[cfg(not(test))]
fn active_face() -> &'static str {
    let p = FACE_PTR.load(Ordering::Acquire);
    let n = FACE_LEN.load(Ordering::Acquire);
    if p.is_null() || n == 0 {
        return "Segoe UI";
    }
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, n)) }
}

fn choose_face(code: &str, meta_font: &str) -> &'static str {
    let meta = meta_font.trim();
    if !meta.is_empty() {
        return Box::leak(meta.to_string().into_boxed_str());
    }
    match code {
        "ko" => "Malgun Gothic",
        "ja" => "Yu Gothic UI",
        _ => "Segoe UI",
    }
}

/// Write the four bundled files when `bundled.ver` is missing or not this version.
/// `*.user.ini` and any other `lang.*.ini` are left as they are.
pub(crate) fn install_languages() {
    install_into(&crate::store::config_dir());
}

pub(crate) fn install_into(cfg: &Path) {
    let dir = cfg.join("lang");
    let _ = std::fs::create_dir_all(&dir);
    let stamp = dir.join("bundled.ver");
    let ver = env!("CARGO_PKG_VERSION");
    if std::fs::read_to_string(&stamp).ok().as_deref().map(str::trim) == Some(ver) {
        return;
    }
    let files = [("lang.ko.ini", KO_INI), ("lang.ja.ini", JA_INI), ("lang.fr.ini", FR_INI), ("lang.de.ini", DE_INI)];
    let mut ok = true;
    for (name, text) in files {
        if std::fs::write(dir.join(name), text).is_err() {
            ok = false;
        }
    }
    if ok {
        let _ = std::fs::write(&stamp, ver);
    }
}

/// Build the in-memory table and the UI font for `lang`. No file is read again until the next call.
pub(crate) fn apply(lang: Lang) {
    let code = lang.resolved_code();
    let (meta, map) = merged(&code, &lang_dir(), false);
    install_table(slots_of(&map));
    set_face(choose_face(&code, &meta.font));
    UI.store(lang.resolve(), Ordering::Relaxed);
}

fn windows_iso() -> (String, Option<String>) {
    unsafe {
        let lcid = GetUserDefaultUILanguage() as u32;
        let full = locale_info(lcid, 0x5c);
        let primary = locale_info(lcid, 0x59).or_else(|| full.as_deref().and_then(|s| s.split('-').next()).map(|s| s.to_string()));
        (primary.unwrap_or_else(|| "en".into()), full)
    }
}

unsafe fn locale_info(lcid: u32, kind: u32) -> Option<String> {
    let mut buf = [0u16; 85];
    let n = GetLocaleInfoW(lcid, kind, Some(&mut buf));
    if n <= 1 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(n as usize) - 1]).trim().to_ascii_lowercase();
    (!s.is_empty()).then_some(s)
}

fn auto_code() -> String {
    let codes = available_codes();
    let (primary, full) = windows_iso();
    if let Some(full) = full {
        if let Some(hit) = codes.iter().find(|c| c.eq_ignore_ascii_case(&full)) {
            return hit.clone();
        }
    }
    if let Some(hit) = codes.iter().find(|c| c.as_str() == primary) {
        return hit.clone();
    }
    "en".into()
}

pub(crate) fn language_name(code: &str) -> String {
    if code == "en" {
        return "English".into();
    }
    let (meta, _) = merged(code, &lang_dir(), false);
    if meta.name.is_empty() { code.to_string() } else { meta.name }
}

/// English first, then every other language file, sorted by ISO code.
pub(crate) fn language_choices() -> Vec<(Lang, String)> {
    let mut out = vec![(Lang::En, "English".to_string())];
    for code in available_codes() {
        if code == "en" {
            continue;
        }
        let name = language_name(&code);
        out.push((Lang::from_ini(&code), name));
    }
    out
}

const ACCENT_KEYS: [Key; 6] = [
    Key::AccentSky,
    Key::AccentGray,
    Key::AccentLavender,
    Key::AccentSage,
    Key::AccentPeach,
    Key::AccentRose,
];

pub(crate) fn accent_name(i: usize) -> &'static str {
    tr(ACCENT_KEYS[i.min(ACCENT_KEYS.len() - 1)])
}

/// Settings combo: Auto, then [`language_choices`].
pub(crate) fn settings_languages() -> Vec<(Lang, String)> {
    let resolved = language_name(&Lang::Auto.resolved_code());
    let mut out = vec![(Lang::Auto, format!("{} ({resolved})", tr(Key::FollowWindows)))];
    out.extend(language_choices());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_face_follows_the_language() {
        assert_eq!(default_face(0), "Segoe UI");
        assert_eq!(default_face(1), "Malgun Gothic");
        assert_eq!(default_face(2), "Yu Gothic UI");
        assert_eq!(default_face(9), "Segoe UI");
        let prev = UI.load(Ordering::Relaxed);
        UI.store(2, Ordering::Relaxed);
        assert_eq!(ui_face(), "Yu Gothic UI");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(ui_face(), "Malgun Gothic");
        UI.store(0, Ordering::Relaxed);
        assert_eq!(ui_face(), "Segoe UI");
        UI.store(prev, Ordering::Relaxed);
    }

    #[test]
    fn saved_codes_round_trip_and_unknown_text_stays_auto() {
        assert_eq!(Lang::from_ini("en"), Lang::En);
        assert_eq!(Lang::from_ini("ko"), Lang::Ko);
        assert_eq!(Lang::from_ini("ja"), Lang::Ja);
        assert_eq!(Lang::from_ini("auto"), Lang::Auto);
        assert_eq!(Lang::from_ini("nope"), Lang::Auto);
        assert_eq!(Lang::from_ini(""), Lang::Auto);
        assert!(matches!(Lang::from_ini("fr"), Lang::Other(_)));
        assert!(matches!(Lang::from_ini("pt-br"), Lang::Other(_)));
        assert_eq!(Lang::from_ini("fr").ini(), "fr");
        assert_eq!(Lang::En.ini(), "en");
        assert_eq!(Lang::from_ini("fr").resolve(), 0);
        assert_eq!(Lang::Ko.resolve(), 1);
        assert_eq!(Lang::Ja.resolve(), 2);
    }

    #[test]
    fn filenames_and_ini_text() {
        assert_eq!(code_from_filename("lang.ko.ini").as_deref(), Some("ko"));
        assert_eq!(code_from_filename("lang.es.ini").as_deref(), Some("es"));
        assert_eq!(code_from_filename("lang.pt-br.ini").as_deref(), Some("pt-br"));
        assert_eq!(code_from_filename("lang.ko.user.ini"), None);
        assert_eq!(code_from_filename("bundled.ver"), None);
        assert_eq!(code_from_filename("lang.not a code.ini"), None);

        let (meta, map) = parse_ini("\u{feff}# comment\n; also\n\n[meta]\nname = Français\nfont = Candara\n\n[text]\nclose = Fermer\nblank = \nnote = line\\nnext\npath = C:\\\\a\n lead =  (x)\nfont = Police\n");
        assert_eq!(meta.name, "Français");
        assert_eq!(meta.font, "Candara");
        assert_eq!(map.get("close").map(String::as_str), Some("Fermer"));
        assert_eq!(map.get("blank").map(String::as_str), Some(""));
        assert_eq!(map.get("note").map(String::as_str), Some("line\nnext"));
        assert_eq!(map.get("path").map(String::as_str), Some("C:\\a"));
        assert_eq!(map.get("lead").map(String::as_str), Some(" (x)"));
        assert_eq!(map.get("font").map(String::as_str), Some("Police"));
        assert!(!map.contains_key("name"));
    }

    #[test]
    fn bundled_files_cover_every_key_and_no_unknown_keys() {
        for (code, text) in [("ko", KO_INI), ("ja", JA_INI), ("fr", FR_INI), ("de", DE_INI)] {
            let (meta, map) = parse_ini(text);
            assert!(!meta.name.is_empty(), "{code}");
            if code == "ko" {
                assert_eq!(meta.font, "Malgun Gothic");
            }
            if code == "ja" {
                assert_eq!(meta.font, "Yu Gothic UI");
            }
            let mut seen = vec![false; KEY_COUNT];
            for key in map.keys() {
                let pos = KEY_ID.iter().position(|id| *id == key).unwrap_or_else(|| panic!("unknown key {key} in {code}"));
                assert!(!seen[pos], "duplicate {key} in {code}");
                seen[pos] = true;
            }
            for (i, ok) in seen.iter().enumerate() {
                assert!(*ok, "missing {} in {code}", KEY_ID[i]);
            }
            assert!(map["help_explorer"].contains('\n'), "{code}");
            assert_eq!(map["close"], match code {
                "ko" => "닫기",
                "ja" => "閉じる",
                "fr" => "Fermer",
                "de" => "Schließen",
                _ => "",
            });
        }
    }

    #[test]
    fn missing_keys_use_english_and_user_files_win() {
        let dir = std::env::temp_dir().join(format!("maguro-lang-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lang.ko.ini"), "[meta]\nname = 테스트\n\n[text]\nclose = 디스크\n").unwrap();
        std::fs::write(dir.join("lang.ko.user.ini"), "close = 사용자\n").unwrap();
        let (meta, map) = merged("ko", &dir, false);
        assert_eq!(meta.name, "테스트");
        let slots = slots_of(&map);
        assert_eq!(slots[Key::Close as usize], "사용자");
        assert_eq!(slots[Key::Help as usize], EN[Key::Help as usize]);

        let only_user = dir.join("user-only");
        std::fs::create_dir_all(&only_user).unwrap();
        std::fs::write(only_user.join("lang.fr.user.ini"), "[meta]\nfont = Candara\n\n[text]\nclose = Perso\n").unwrap();
        let (meta, map) = merged("fr", &only_user, false);
        assert_eq!(meta.font, "Candara");
        assert_eq!(meta.name, "Français");
        let slots = slots_of(&map);
        assert_eq!(slots[Key::Close as usize], "Perso");
        assert_eq!(slots[Key::Help as usize], "Aide");

        std::fs::write(dir.join("lang.en.user.ini"), "close = Shut\n").unwrap();
        let (meta, map) = merged("en", &dir, false);
        assert_eq!(slots_of(&map)[Key::Close as usize], "Shut");
        assert!(meta.name.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_overwrites_bundled_files_only_when_the_version_changes() {
        let cfg = std::env::temp_dir().join(format!("maguro-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cfg);
        let lang = cfg.join("lang");
        std::fs::create_dir_all(&lang).unwrap();
        std::fs::write(lang.join("lang.ko.ini"), "close = edited\n").unwrap();
        std::fs::write(lang.join("lang.ko.user.ini"), "close = keep-me\n").unwrap();
        std::fs::write(lang.join("lang.es.ini"), "name = Español\nclose = Hola\n").unwrap();
        std::fs::write(lang.join("bundled.ver"), "0.0.1\n").unwrap();
        install_into(&cfg);
        let ko = std::fs::read_to_string(lang.join("lang.ko.ini")).unwrap();
        assert!(ko.contains("name = 한국어"), "{ko}");
        assert_ne!(ko, "close = edited\n");
        assert_eq!(std::fs::read_to_string(lang.join("lang.ko.user.ini")).unwrap(), "close = keep-me\n");
        assert_eq!(std::fs::read_to_string(lang.join("lang.es.ini")).unwrap(), "name = Español\nclose = Hola\n");
        assert!(std::fs::read_to_string(lang.join("lang.fr.ini")).unwrap().contains("name = Français"));
        assert!(std::fs::read_to_string(lang.join("lang.de.ini")).unwrap().contains("name = Deutsch"));
        assert!(std::fs::read_to_string(lang.join("lang.ja.ini")).unwrap().contains("font = Yu Gothic UI"));
        assert_eq!(std::fs::read_to_string(lang.join("bundled.ver")).unwrap().trim(), env!("CARGO_PKG_VERSION"));

        std::fs::write(lang.join("lang.ko.ini"), "close = edited-again\n").unwrap();
        install_into(&cfg);
        assert_eq!(std::fs::read_to_string(lang.join("lang.ko.ini")).unwrap(), "close = edited-again\n");
        assert_eq!(std::fs::read_to_string(lang.join("lang.ko.user.ini")).unwrap(), "close = keep-me\n");
        assert_eq!(std::fs::read_to_string(lang.join("lang.es.ini")).unwrap(), "name = Español\nclose = Hola\n");
        let _ = std::fs::remove_dir_all(&cfg);
    }

    #[test]
    fn korean_and_japanese_text_matches_the_old_strings() {
        let prev = UI.load(Ordering::Relaxed);
        UI.store(0, Ordering::Relaxed);
        assert_eq!(tr(Key::Close), "Close");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(tr(Key::Close), "닫기");
        assert_eq!(tr(Key::FollowWindows), "Windows 설정 따르기");
        assert_eq!(tr(Key::View), "보기");
        assert_eq!(tr(Key::Appearance), "모양");
        assert_eq!(tr(Key::Behavior), "동작");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(tr(Key::Close), "閉じる");
        assert_eq!(tr(Key::FollowWindows), "Windows の設定に従う");
        assert_eq!(tr(Key::View), "表示");
        assert_eq!(tr(Key::Appearance), "外観");
        assert_eq!(tr(Key::Behavior), "動作");
        UI.store(prev, Ordering::Relaxed);
    }

    #[test]
    fn update_and_help_copy_names_no_site() {
        let ids = [
            "asks_github_for_newer_maguro_release_and",
            "maguro_new_is_available_you_have_cur",
            "maguro_new_is_available_you_have_cur_2",
            "maguro_new_is_available_you_have_cur_3",
            "help_online_f1",
            "fast_multi_pane_file_manager_for_windows",
            "mit_license_2026_efww",
        ];
        let clean = |s: &str| {
            let l = s.to_ascii_lowercase();
            !l.contains("github") && !l.contains("http://") && !l.contains("https://") && !s.contains("{url}")
        };
        for id in ids {
            let i = KEY_ID.iter().position(|k| *k == id).unwrap();
            assert!(clean(EN[i]), "en {id}: {}", EN[i]);
            for (code, text) in [("ko", KO_INI), ("ja", JA_INI), ("fr", FR_INI), ("de", DE_INI)] {
                let (_, map) = parse_ini(text);
                let v = &map[id];
                assert!(clean(v), "{code} {id}: {v}");
            }
        }
        let help_i = KEY_ID.iter().position(|k| *k == "help_online_f1").unwrap();
        assert!(EN[help_i].contains("F1"));
        assert!(!EN[help_i].to_ascii_lowercase().contains("online"), "{}", EN[help_i]);
        for (code, text) in [("ko", KO_INI), ("ja", JA_INI), ("fr", FR_INI), ("de", DE_INI)] {
            let (_, map) = parse_ini(text);
            let v = &map["help_online_f1"];
            let l = v.to_ascii_lowercase();
            assert!(v.contains("F1"), "{code}");
            assert!(!l.contains("online") && !v.contains("オンライン") && !v.contains("온라인") && !l.contains("en ligne"), "{code} {v}");
        }
    }

    #[test]
    fn help_topics_explain_the_app_in_every_language() {
        let tabs = [
            ("help_tab_getting_started", "Getting started"),
            ("help_tab_panes_tabs", "Panes & tabs"),
            ("help_tab_favorites", "Favorites"),
            ("help_tab_file_operations", "File operations"),
            ("help_tab_server_connections", "Server connections"),
            ("help_tab_shortcuts", "Shortcuts"),
            ("help_tab_settings", "Settings"),
            ("help_tab_troubleshooting", "Troubleshooting"),
        ];
        let bodies = [
            "help_body_getting_started",
            "help_body_panes_tabs",
            "help_body_favorites",
            "help_body_file_operations",
            "help_body_server_connections",
            "help_body_shortcuts",
            "help_body_settings",
            "help_body_troubleshooting",
        ];
        let clean = |s: &str| {
            let l = s.to_ascii_lowercase();
            !l.contains("github") && !s.contains("http://") && !s.contains("https://") && !l.contains("samek86") && !s.contains("{url}")
        };
        for (id, title) in tabs {
            let i = KEY_ID.iter().position(|k| *k == id).unwrap();
            assert_eq!(EN[i], title);
            assert!(clean(EN[i]), "{title}");
            for (code, text) in [("ko", KO_INI), ("ja", JA_INI), ("fr", FR_INI), ("de", DE_INI)] {
                let (_, map) = parse_ini(text);
                let v = &map[id];
                assert!(!v.is_empty() && !v.contains('\n'), "{code} {id}: {v}");
                assert!(clean(v), "{code} {id}: {v}");
            }
        }
        let mut all = String::new();
        for id in bodies {
            let i = KEY_ID.iter().position(|k| *k == id).unwrap();
            assert!(EN[i].contains('\n'), "{id}");
            assert!(clean(EN[i]), "{id}");
            all.push_str(&EN[i].to_ascii_lowercase());
            all.push('\n');
            for (code, text) in [("ko", KO_INI), ("ja", JA_INI), ("fr", FR_INI), ("de", DE_INI)] {
                let (_, map) = parse_ini(text);
                let v = &map[id];
                assert!(v.contains('\n'), "{code} {id}");
                assert!(v.chars().count() > 280, "{code} {id} {}", v.chars().count());
                assert!(clean(v), "{code} {id}");
            }
        }
        for needle in [
            "pane", "ctrl+t", "session", "favourites", "robocopy", "smb", "webdav", "sftp", "ftp", "everything", "preview", "portable.txt", "crash", "ctrl+,", "theme", "winmerge", "fast copy",
        ] {
            assert!(all.contains(needle), "{needle}");
        }
        let shortcuts = EN[KEY_ID.iter().position(|k| *k == "help_body_shortcuts").unwrap()].to_ascii_lowercase();
        assert!(shortcuts.contains("ctrl+/") && shortcuts.contains("f1"));
    }
}
