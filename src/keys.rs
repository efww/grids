// One table for every app shortcut. The settings page rebinds rows in this table,
// and keydown reads it, so a change applies on the next key without a restart.
// Chords are "Ctrl+Shift+T". A digit-range command stores "Ctrl+1 … Ctrl+9"
// and matches 1 through 9 with those modifiers.

use std::cell::RefCell;

use serde_json::{Map, Value};

use crate::tr;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Chord {
    pub vk: u16,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    /// The Windows key. A global hotkey uses this; an in-app chord can too.
    pub win: bool,
    /// Any main-row digit 1–9. `vk` is VK_1 (0x31).
    pub digits: bool,
}

struct Cmd {
    id: &'static str,
    cat: crate::Key,
    act: crate::Key,
    vk: u16,
    ctrl: bool,
    shift: bool,
    alt: bool,
    win: bool,
    /// Pressing a digit rebinds the whole 1–9 range (Go to pane).
    range: bool,
}

impl Cmd {
    fn chord(&self) -> Chord {
        Chord { vk: self.vk, ctrl: self.ctrl, shift: self.shift, alt: self.alt, win: self.win, digits: self.range }
    }
}

const fn cmd(id: &'static str, cat: crate::Key, act: crate::Key, vk: u16, ctrl: bool, shift: bool, alt: bool, win: bool, range: bool) -> Cmd {
    Cmd { id, cat, act, vk, ctrl, shift, alt, win, range }
}

const TAB: crate::Key = crate::Key::Tab;
const PANTAB: crate::Key = crate::Key::PaneTab;
const PANE: crate::Key = crate::Key::Pane;
const MOVE: crate::Key = crate::Key::Move;
const VIEW: crate::Key = crate::Key::View;
const FILE: crate::Key = crate::Key::File;
const SESS: crate::Key = crate::Key::Session;
const WS: crate::Key = crate::Key::Workspaces;
const FAV: crate::Key = crate::Key::Favourites;
const HELP: crate::Key = crate::Key::Help;
const WIN: crate::Key = crate::Key::Window;

const CMDS: &[Cmd] = &[
    cmd("tab.new", TAB, crate::Key::NewTab, 0x54, true, false, false, false, false),
    cmd("tab.close", TAB, crate::Key::CloseTab, 0x57, true, false, false, false, false),
    cmd("tab.next", TAB, crate::Key::NextTab, 0x09, true, false, false, false, false),
    cmd("tab.prev", TAB, crate::Key::PreviousTab, 0x09, true, true, false, false, false),
    cmd("tab.reopen", TAB, crate::Key::ReopenClosedTab, 0x54, true, true, false, false, false),
    // Ctrl+K stays free. Duplicate tab is Ctrl+Alt+K; the tab right-click menu does the same.
    cmd("tab.dup", TAB, crate::Key::DuplicateTab, 0x4B, true, false, true, false, false),
    cmd("tab.list", TAB, crate::Key::TabList, 0x4C, true, true, false, false, false),
    cmd("pane.next", PANE, crate::Key::NextPane, 0x22, true, false, false, false, false),
    cmd("pane.prev", PANE, crate::Key::PreviousPane, 0x21, true, false, false, false, false),
    cmd("pane.goto", PANE, crate::Key::GoPane19, 0x31, true, false, false, false, true),
    // Distinct from window tabs (Ctrl+T / Ctrl+W) and from moving between panes (Ctrl+PageUp / PageDown).
    // They run only while the focus is in a pane.
    cmd("pane.tab.new", PANTAB, crate::Key::NewPaneTab, 0x54, true, false, true, false, false),
    cmd("pane.tab.close", PANTAB, crate::Key::ClosePaneTab, 0x57, true, false, true, false, false),
    cmd("pane.tab.next", PANTAB, crate::Key::NextPaneTab, 0x22, true, false, true, false, false),
    cmd("pane.tab.prev", PANTAB, crate::Key::PreviousPaneTab, 0x21, true, false, true, false, false),
    cmd("pane.left", MOVE, crate::Key::PaneLeft, 0x25, false, true, true, false, false),
    cmd("pane.right", MOVE, crate::Key::PaneRight, 0x27, false, true, true, false, false),
    cmd("pane.up", MOVE, crate::Key::PaneAbove, 0x26, false, true, true, false, false),
    cmd("pane.down", MOVE, crate::Key::PaneBelow, 0x28, false, true, true, false, false),
    cmd("pane.swap.left", MOVE, crate::Key::SwapWithPaneLeft, 0x25, true, true, true, false, false),
    cmd("pane.swap.right", MOVE, crate::Key::SwapWithPaneRight, 0x27, true, true, true, false, false),
    cmd("pane.swap.up", MOVE, crate::Key::SwapWithPaneAbove, 0x26, true, true, true, false, false),
    cmd("pane.swap.down", MOVE, crate::Key::SwapWithPaneBelow, 0x28, true, true, true, false, false),
    cmd("nav.back", MOVE, crate::Key::Back, 0x25, false, false, true, false, false),
    // Plain Backspace, same action as Alt+Left. Address, filter, and rename keep the key for editing.
    cmd("nav.back2", MOVE, crate::Key::Back, 0x08, false, false, false, false, false),
    cmd("nav.fwd", MOVE, crate::Key::Forward, 0x27, false, false, true, false, false),
    cmd("nav.up", MOVE, crate::Key::ParentFolder, 0x26, false, false, true, false, false),
    cmd("nav.recent", MOVE, crate::Key::RecentFolders, 0x28, false, false, true, false, false),
    cmd("nav.addr", MOVE, crate::Key::AddressBar, 0x4C, true, false, false, false, false),
    cmd("nav.addr2", MOVE, crate::Key::AddressBar, 0x44, false, false, true, false, false),
    cmd("nav.addr3", MOVE, crate::Key::AddressBar, 0x73, false, false, false, false, false),
    cmd("view.filter", VIEW, crate::Key::SearchThisFolder, 0x46, true, false, false, false, false),
    cmd("view.peek", VIEW, crate::Key::QuickLook, 0x72, false, false, false, false, false),
    // Alt+P is free: no other default chord uses P, with or without modifiers.
    cmd("view.preview", VIEW, crate::Key::PreviewPane, 0x50, false, false, true, false, false),
    cmd("focus.next", VIEW, crate::Key::NextFocus, 0x75, false, false, false, false, false),
    cmd("focus.prev", VIEW, crate::Key::PreviousFocus, 0x75, false, true, false, false, false),
    cmd("view.hidden", VIEW, crate::Key::ShowHiddenFiles, 0x48, true, false, false, false, false),
    cmd("view.panel", VIEW, crate::Key::ShowLeftPanel, 0x45, true, true, false, false, false),
    cmd("view.gather", VIEW, crate::Key::OpenExplorerFolders, 0x4F, true, true, false, false, false),
    // Ctrl+Shift+R is batch rename. Recent files is Ctrl+Alt+R.
    cmd("view.recentfiles", VIEW, crate::Key::RecentFiles, 0x52, true, false, true, false, false),
    cmd("view.zoom", VIEW, crate::Key::MaximiseRestorePane, 0x4D, true, true, false, false, false),
    cmd("file.path", FILE, crate::Key::CopyFullPath, 0x43, true, true, false, false, false),
    cmd("file.rename", FILE, crate::Key::BatchRename, 0x52, true, true, false, false, false),
    cmd("file.sel", FILE, crate::Key::SelectByPattern, 0x6B, false, false, false, false, false),
    cmd("file.sel2", FILE, crate::Key::SelectByPattern, 0x41, true, true, false, false, false),
    cmd("file.desel", FILE, crate::Key::DeselectByPattern, 0x6D, false, false, false, false, false),
    cmd("file.invert", FILE, crate::Key::InvertSelection, 0x6A, false, false, false, false, false),
    cmd("file.new", FILE, crate::Key::NewTextFile, 0x4E, true, false, true, false, false),
    cmd("file.dir", FILE, crate::Key::NewFolder, 0x4E, true, true, false, false, false),
    cmd("file.mkdir", FILE, crate::Key::NewFolderInThisPane, 0x76, false, false, false, false, false),
    // Ctrl+Alt+C is free (copy path is Ctrl+Shift+C, batch rename is Ctrl+Shift+R).
    cmd("file.winmerge", FILE, crate::Key::CompareFoldersInWinmerge, 0x43, true, false, true, false, false),
    // Ctrl+B is Favourites. Ctrl+Shift+B toggles the basket; Ctrl+Alt+B adds the selection.
    cmd("file.basket", FILE, crate::Key::FileBasket, 0x42, true, true, false, false, false),
    cmd("file.basket.add", FILE, crate::Key::AddSelectionToBasket, 0x42, true, false, true, false, false),
    // Show in folder, only while the pane is showing Recent files. Elsewhere the key is forwarded.
    cmd("file.reveal", FILE, crate::Key::ShowInFolder, 0x0D, true, false, false, false, false),
    cmd("sess.save", SESS, crate::Key::SaveThisTab, 0x53, true, false, false, false, false),
    // Ctrl+1..9 stays Go to pane. These add Alt, and Alt+Shift. Not registered as global hotkeys
    // (those are Win+Shift+E, and Ctrl+Alt+G while a file dialog is in front).
    cmd("ws.load", WS, crate::Key::LoadWorkspace, 0x31, true, false, true, false, true),
    cmd("ws.save", WS, crate::Key::SaveWorkspace, 0x31, true, true, true, false, true),
    cmd("fav.list", FAV, crate::Key::Favourites, 0x42, true, false, false, false, false),
    cmd("app.settings", HELP, crate::Key::Settings2, 0xBC, true, false, false, false, false),
    cmd("app.help", HELP, crate::Key::ShortcutHelp, 0x70, false, false, false, false, false),
    cmd("app.keys", HELP, crate::Key::ShortcutList, 0xBF, true, false, false, false, false),
    cmd("app.front", WIN, crate::Key::BringMaguroFront, 0x45, false, true, false, true, false),
    // Registered only while another app's Open or Save dialog is in front. See dialogjump.
    cmd("dialog.jump", WIN, crate::Key::JumpToAFolderInAFileDialog, 0x47, true, false, true, false, false),
    // Registered for the whole time Maguro is running, same as app.front. See cliphot.
    // Win+Alt+V does not overlap app.front (Win+Shift+E).
    cmd("clip.open", WIN, crate::Key::OpenClipboardPath, 0x56, false, false, true, true, false),
    cmd("win.new", WIN, crate::Key::NewWindow, 0x4E, true, false, false, false, false),
    cmd("watch.open", WIN, crate::Key::FolderWatch, 0x57, true, true, false, false, false),
];

struct KeyState {
    binds: Vec<Chord>,
    extra: Map<String, Value>,
    capture: Option<usize>,
    swallow: u16,
    modal: bool,
}

impl KeyState {
    fn fresh() -> Self {
        KeyState { binds: CMDS.iter().map(Cmd::chord).collect(), extra: Map::new(), capture: None, swallow: 0, modal: false }
    }
}

thread_local! {
    static STATE: RefCell<KeyState> = RefCell::new(KeyState::fresh());
}

fn with<R>(f: impl FnOnce(&mut KeyState) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

pub(crate) fn len() -> usize {
    CMDS.len()
}

pub(crate) fn index(id: &str) -> Option<usize> {
    CMDS.iter().position(|c| c.id == id)
}

pub(crate) fn id_of(i: usize) -> &'static str {
    CMDS.get(i).map(|c| c.id).unwrap_or("")
}

pub(crate) fn is_command(id: &str) -> bool {
    index(id).is_some()
}

fn label(t: crate::Key) -> String {
    tr(t).to_string()
}

pub(crate) fn action(i: usize) -> String {
    CMDS.get(i).map(|c| label(c.act)).unwrap_or_default()
}

pub(crate) fn chord_text(i: usize) -> String {
    with(|s| s.binds.get(i).copied().map(|c| format_chord(&c)).unwrap_or_default())
}

pub(crate) fn row(i: usize) -> (String, String, String) {
    let Some(c) = CMDS.get(i) else { return (String::new(), String::new(), String::new()) };
    (label(c.cat), label(c.act), chord_text(i))
}

pub(crate) fn is_custom(i: usize) -> bool {
    with(|s| CMDS.get(i).is_some_and(|c| s.binds.get(i).copied() != Some(c.chord())))
}

pub(crate) fn default_chord(i: usize) -> Option<Chord> {
    CMDS.get(i).map(Cmd::chord)
}

pub(crate) fn same(i: usize, c: &Chord) -> bool {
    with(|s| s.binds.get(i).copied() == Some(*c))
}

pub(crate) fn begin(i: usize) {
    if i < CMDS.len() {
        with(|s| s.capture = Some(i));
    }
}

pub(crate) fn cancel() {
    with(|s| s.capture = None);
}

pub(crate) fn capturing() -> bool {
    with(|s| s.capture.is_some())
}

pub(crate) fn capture_row() -> Option<usize> {
    with(|s| s.capture)
}

pub(crate) fn set_modal(on: bool) {
    with(|s| s.modal = on);
}

pub(crate) fn modal() -> bool {
    with(|s| s.modal)
}

pub(crate) fn swallow(vk: u16) {
    with(|s| s.swallow = vk);
}

pub(crate) fn swallows(vk: u16) -> bool {
    vk != 0 && with(|s| s.swallow == vk)
}

pub(crate) fn clear_swallow(vk: u16) {
    with(|s| {
        if s.swallow == vk {
            s.swallow = 0;
        }
    });
}

fn overlaps(a: Chord, b: Chord) -> bool {
    if a.ctrl != b.ctrl || a.shift != b.shift || a.alt != b.alt || a.win != b.win {
        return false;
    }
    match (a.digits, b.digits) {
        (true, true) => true,
        (true, false) => (0x31..=0x39).contains(&b.vk),
        (false, true) => (0x31..=0x39).contains(&a.vk),
        (false, false) => a.vk == b.vk,
    }
}

pub(crate) fn conflict(row: usize, chord: Chord) -> Option<usize> {
    with(|s| s.binds.iter().enumerate().find(|(i, c)| *i != row && overlaps(**c, chord)).map(|(i, _)| i))
}

/// `chord` becomes row's binding. The previous owner of that chord, if any, receives row's old one.
pub(crate) fn rebind(row: usize, chord: Chord) {
    with(|s| {
        if row >= s.binds.len() || s.binds[row] == chord {
            return;
        }
        let old = s.binds[row];
        if let Some(other) = s.binds.iter().enumerate().find(|(i, c)| *i != row && overlaps(**c, chord)).map(|(i, _)| i) {
            s.binds[row] = chord;
            s.binds[other] = old;
        } else {
            s.binds[row] = chord;
        }
    });
}

pub(crate) fn reset_all() {
    with(|s| {
        s.binds = CMDS.iter().map(Cmd::chord).collect();
        s.capture = None;
    });
}

pub(crate) enum Offer {
    Ignore,
    Cancel,
    Same,
    Bind { row: usize, chord: Chord, other: Option<usize> },
}

fn modifier(vk: u16) -> bool {
    matches!(vk, 0x10 | 0x11 | 0x12 | 0xA0 | 0xA1 | 0xA2 | 0xA3 | 0xA4 | 0xA5 | 0x5B | 0x5C)
}

pub(crate) fn offer(vk: u16, ctrl: bool, shift: bool, alt: bool, win: bool) -> Offer {
    let Some(row) = with(|s| s.capture) else { return Offer::Ignore };
    if modifier(vk) {
        return Offer::Ignore;
    }
    if vk == 0x1B && !ctrl && !shift && !alt && !win {
        with(|s| s.capture = None);
        return Offer::Cancel;
    }
    let mut chord = Chord { vk, ctrl, shift, alt, win, digits: false };
    if CMDS.get(row).is_some_and(|c| c.range) && (0x31..=0x39).contains(&vk) {
        chord.vk = 0x31;
        chord.digits = true;
    }
    with(|s| s.capture = None);
    if with(|s| s.binds.get(row).copied() == Some(chord)) {
        return Offer::Same;
    }
    let other = conflict(row, chord);
    Offer::Bind { row, chord, other }
}

pub(crate) fn lookup(vk: u16, ctrl: bool, shift: bool, alt: bool, win: bool) -> Option<usize> {
    with(|s| {
        let exact = s.binds.iter().enumerate().find(|(_, c)| !c.digits && c.vk == vk && c.ctrl == ctrl && c.shift == shift && c.alt == alt && c.win == win);
        if let Some((i, _)) = exact {
            return Some(i);
        }
        if !(0x31..=0x39).contains(&vk) {
            return None;
        }
        s.binds.iter().enumerate().find(|(_, c)| c.digits && c.ctrl == ctrl && c.shift == shift && c.alt == alt && c.win == win).map(|(i, _)| i)
    })
}

/// The chord registered as the global hotkey, if that command is in the table.
pub(crate) fn hot_chord() -> Option<Chord> {
    index("app.front").and_then(|i| with(|s| s.binds.get(i).copied()))
}

/// The chord that lists Maguro folders in another app's file dialog.
pub(crate) fn dialog_chord() -> Option<Chord> {
    index("dialog.jump").and_then(|i| with(|s| s.binds.get(i).copied()))
}

/// The chord that opens a clipboard folder in a new tab, or a clipboard file with the shell.
pub(crate) fn clip_chord() -> Option<Chord> {
    index("clip.open").and_then(|i| with(|s| s.binds.get(i).copied()))
}

/// What the menu click asks for. `reg(true)` registers and reports whether it is really on.
pub(crate) fn hotkey_flip(on: bool, reg: impl FnOnce(bool) -> bool) -> bool {
    reg(!on)
}

fn untangle(binds: &mut [Chord]) {
    for i in 0..binds.len() {
        let clash = binds.iter().take(i).any(|c| overlaps(*c, binds[i]));
        if !clash {
            continue;
        }
        let def = CMDS[i].chord();
        if binds.iter().take(i).any(|c| overlaps(*c, def)) {
            continue;
        }
        binds[i] = def;
    }
}

/// Previous built-in `clip.open` (Win+Shift+V). It was not written while it was the default.
/// A file that still names it is that old default, so the new built-in (Win+Alt+V) replaces it.
fn legacy_clip_open(c: Chord) -> bool {
    c.vk == 0x56 && c.win && c.shift && !c.ctrl && !c.alt && !c.digits
}

pub(crate) fn load_bindings(map: &Map<String, Value>) {
    let mut binds: Vec<Chord> = CMDS.iter().map(Cmd::chord).collect();
    let mut extra = Map::new();
    for (k, v) in map {
        match index(k) {
            Some(i) => {
                if let Some(c) = v.as_str().and_then(parse_chord) {
                    if !(k == "clip.open" && legacy_clip_open(c)) {
                        binds[i] = c;
                    }
                }
            }
            None => {
                extra.insert(k.clone(), v.clone());
            }
        }
    }
    untangle(&mut binds);
    with(|s| {
        s.binds = binds;
        s.extra = extra;
        s.capture = None;
    });
}

pub(crate) fn bindings_json() -> Map<String, Value> {
    with(|s| {
        let mut m = s.extra.clone();
        for (i, c) in CMDS.iter().enumerate() {
            m.remove(c.id);
            if s.binds.get(i).copied().is_some_and(|b| b != c.chord()) {
                if let Some(b) = s.binds.get(i) {
                    m.insert(c.id.to_string(), Value::String(format_chord(b)));
                }
            }
        }
        m
    })
}

/// One key of a 1–9 range, as it should appear on a menu. A chord that is not a range is shown as-is.
pub(crate) fn range_chord(i: usize, digit: u8) -> String {
    with(|s| {
        let Some(c) = s.binds.get(i).copied() else { return String::new() };
        if !c.digits || !(1..=9).contains(&digit) {
            return format_chord(&c);
        }
        let mut one = c;
        one.digits = false;
        one.vk = 0x30 + digit as u16;
        format_one(&one)
    })
}

pub(crate) fn format_chord(c: &Chord) -> String {
    if c.digits {
        let mut one = *c;
        one.digits = false;
        one.vk = 0x31;
        let mut nine = one;
        nine.vk = 0x39;
        return format!("{} … {}", format_one(&one), format_one(&nine));
    }
    format_one(c)
}

fn format_one(c: &Chord) -> String {
    let mut p = Vec::new();
    if c.win {
        p.push("Win".to_string());
    }
    if c.ctrl {
        p.push("Ctrl".to_string());
    }
    if c.alt {
        p.push("Alt".to_string());
    }
    if c.shift {
        p.push("Shift".to_string());
    }
    p.push(key_name(c.vk));
    p.join("+")
}

fn key_name(vk: u16) -> String {
    match vk {
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x1B => "Esc".into(),
        0x20 => "Space".into(),
        0x21 => "PageUp".into(),
        0x22 => "PageDown".into(),
        0x25 => "←".into(),
        0x26 => "↑".into(),
        0x27 => "→".into(),
        0x28 => "↓".into(),
        0x08 => "Back".into(),
        0x2E => "Del".into(),
        0x30..=0x39 => char::from(vk as u8).to_string(),
        0x41..=0x5A => char::from(vk as u8).to_string(),
        0x60..=0x69 => format!("Num{}", vk - 0x60),
        0x6A => "Num*".into(),
        0x6B => "Num+".into(),
        0x6D => "Num-".into(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        0xBC => ",".into(),
        0xBF => "/".into(),
        _ => format!("VK_{vk:02X}"),
    }
}

pub(crate) fn parse_chord(s: &str) -> Option<Chord> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (head, digits) = if let Some((l, _)) = s.split_once('…') {
        (l.trim(), true)
    } else if let Some((l, _)) = s.split_once("...") {
        (l.trim(), true)
    } else {
        (s, false)
    };
    let mut parts: Vec<&str> = head.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    // "Num+" keeps its trailing plus; every other '+' separates a modifier.
    let plus = head.trim_end().ends_with('+') && parts.last().is_some_and(|p| p.eq_ignore_ascii_case("num"));
    let key = if plus { format!("{}+", parts.pop()?) } else { parts.pop()?.to_string() };
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut win = false;
    for p in parts {
        match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" => alt = true,
            "win" | "windows" => win = true,
            _ => return None,
        }
    }
    let vk = parse_key(&key)?;
    let mut c = Chord { vk, ctrl, shift, alt, win, digits: false };
    if digits {
        if !(0x31..=0x39).contains(&vk) {
            return None;
        }
        c.vk = 0x31;
        c.digits = true;
    }
    Some(c)
}

fn parse_key(k: &str) -> Option<u16> {
    match k {
        "←" => return Some(0x25),
        "→" => return Some(0x27),
        "↑" => return Some(0x26),
        "↓" => return Some(0x28),
        "," => return Some(0xBC),
        "/" => return Some(0xBF),
        _ => {}
    }
    let low = k.to_ascii_lowercase();
    match low.as_str() {
        "tab" => Some(0x09),
        "enter" | "return" => Some(0x0D),
        "esc" | "escape" => Some(0x1B),
        "space" => Some(0x20),
        "pageup" | "pgup" | "prior" => Some(0x21),
        "pagedown" | "pgdn" | "next" => Some(0x22),
        "left" => Some(0x25),
        "up" => Some(0x26),
        "right" => Some(0x27),
        "down" => Some(0x28),
        "back" | "backspace" => Some(0x08),
        "del" | "delete" => Some(0x2E),
        "comma" => Some(0xBC),
        "slash" | "/" => Some(0xBF),
        _ => parse_key_more(&low, k),
    }
}

fn parse_key_more(low: &str, raw: &str) -> Option<u16> {
    if let Some(rest) = low.strip_prefix("vk_") {
        return u16::from_str_radix(rest, 16).ok();
    }
    match low {
        "num*" => return Some(0x6A),
        "num+" => return Some(0x6B),
        "num-" => return Some(0x6D),
        _ => {}
    }
    if let Some(rest) = low.strip_prefix("num") {
        let n: u16 = rest.parse().ok()?;
        return (n <= 9).then_some(0x60 + n);
    }
    if let Some(rest) = low.strip_prefix('f') {
        if rest.chars().all(|c| c.is_ascii_digit()) {
            let n: u16 = rest.parse().ok()?;
            return (1..=24).contains(&n).then_some(0x70 + n - 1);
        }
    }
    let mut chars = raw.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if c.is_ascii_alphabetic() {
        return Some(c.to_ascii_uppercase() as u16);
    }
    if c.is_ascii_digit() {
        return Some(c as u16);
    }
    None
}

pub(crate) fn help_text() -> String {
    let mut s = String::new();
    for i in 0..CMDS.len() {
        s.push_str(&chord_text(i));
        s.push('\t');
        s.push_str(&action(i));
        s.push('\n');
    }
    s.push_str(tr(crate::Key::HelpExplorer));
    s.push_str(tr(crate::Key::HelpNotes));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() {
        load_bindings(&Map::new());
    }

    #[test]
    fn defaults_round_trip_and_match_the_old_chords() {
        fresh();
        let want = [
            ("tab.new", "Ctrl+T"),
            ("tab.close", "Ctrl+W"),
            ("tab.next", "Ctrl+Tab"),
            ("tab.prev", "Ctrl+Shift+Tab"),
            ("tab.reopen", "Ctrl+Shift+T"),
            ("tab.dup", "Ctrl+Alt+K"),
            ("pane.next", "Ctrl+PageDown"),
            ("pane.prev", "Ctrl+PageUp"),
            ("pane.goto", "Ctrl+1 … Ctrl+9"),
            ("pane.left", "Alt+Shift+←"),
            ("pane.swap.left", "Ctrl+Alt+Shift+←"),
            ("pane.swap.right", "Ctrl+Alt+Shift+→"),
            ("pane.swap.up", "Ctrl+Alt+Shift+↑"),
            ("pane.swap.down", "Ctrl+Alt+Shift+↓"),
            ("pane.right", "Alt+Shift+→"),
            ("nav.back", "Alt+←"),
            ("nav.back2", "Back"),
            ("nav.up", "Alt+↑"),
            ("nav.addr", "Ctrl+L"),
            ("nav.addr2", "Alt+D"),
            ("nav.addr3", "F4"),
            ("file.sel", "Num+"),
            ("file.sel2", "Ctrl+Shift+A"),
            ("file.desel", "Num-"),
            ("file.invert", "Num*"),
            ("file.rename", "Ctrl+Shift+R"),
            ("file.new", "Ctrl+Alt+N"),
            ("file.dir", "Ctrl+Shift+N"),
            ("file.mkdir", "F7"),
            ("file.winmerge", "Ctrl+Alt+C"),
            ("file.basket", "Ctrl+Shift+B"),
            ("file.basket.add", "Ctrl+Alt+B"),
            ("pane.tab.new", "Ctrl+Alt+T"),
            ("pane.tab.close", "Ctrl+Alt+W"),
            ("pane.tab.next", "Ctrl+Alt+PageDown"),
            ("pane.tab.prev", "Ctrl+Alt+PageUp"),
            ("app.settings", "Ctrl+,"),
            ("app.help", "F1"),
            ("app.keys", "Ctrl+/"),
            ("view.zoom", "Ctrl+Shift+M"),
            ("view.panel", "Ctrl+Shift+E"),
            ("view.gather", "Ctrl+Shift+O"),
            ("focus.next", "F6"),
            ("focus.prev", "Shift+F6"),
            ("view.peek", "F3"),
            ("view.preview", "Alt+P"),
            ("app.front", "Win+Shift+E"),
            ("win.new", "Ctrl+N"),
        ];
        for (id, text) in want {
            let i = index(id).unwrap();
            assert_eq!(chord_text(i), text, "{id}");
            assert_eq!(parse_chord(text).unwrap(), default_chord(i).unwrap());
            assert_eq!(format_chord(&parse_chord(&text.to_ascii_lowercase()).unwrap()), text);
        }
        assert!(parse_chord("").is_none());
        assert!(parse_chord("Ctrl+Nope").is_none());
        assert_eq!(lookup(b'T' as u16, true, false, false, false), index("tab.new"));
        assert_eq!(lookup(b'T' as u16, true, true, false, false), index("tab.reopen"));
        assert_eq!(lookup(b'5' as u16, true, false, false, false), index("pane.goto"));
        assert_eq!(lookup(b'0' as u16, true, false, false, false), None);
        assert_eq!(lookup(b'5' as u16, true, true, false, false), None);
        assert_eq!(lookup(0x25, false, true, true, false), index("pane.left"));
        assert_eq!(lookup(0x25, false, false, true, false), index("nav.back"));
        assert_eq!(lookup(0x08, false, false, false, false), index("nav.back2"));
        assert_eq!(parse_chord("Back").unwrap().vk, 0x08);
        assert_eq!(parse_chord("Backspace").unwrap().vk, 0x08);
        assert_eq!(format_chord(&parse_chord("Back").unwrap()), "Back");
        assert_eq!(format_chord(&parse_chord("Backspace").unwrap()), "Back");
        assert_eq!(lookup(b'N' as u16, true, false, false, false), index("win.new"));
        assert_eq!(lookup(b'N' as u16, true, false, true, false), index("file.new"));
        assert_eq!(lookup(b'N' as u16, true, true, false, false), index("file.dir"));
        assert_eq!(lookup(b'E' as u16, false, true, false, true), index("app.front"));
        assert_eq!(lookup(b'G' as u16, true, false, true, false), index("dialog.jump"));
        assert_eq!(chord_text(index("dialog.jump").unwrap()), "Ctrl+Alt+G");
        assert!(help_text().lines().any(|l| l == "Ctrl+Alt+G\tJump to a folder in a file dialog"));
        assert_eq!(lookup(b'P' as u16, false, false, true, false), index("view.preview"));
        assert_eq!(chord_text(index("view.preview").unwrap()), "Alt+P");
        assert!(help_text().lines().any(|l| l == "Alt+P\tPreview pane"));
        assert_ne!(lookup(b'P' as u16, true, false, false, false), index("view.preview"));
        let clip = index("clip.open").unwrap();
        assert_eq!(chord_text(clip), "Win+Alt+V");
        assert_eq!(lookup(b'V' as u16, false, false, true, true), Some(clip));
        assert_eq!(lookup(b'V' as u16, false, true, false, true), None);
        assert!(help_text().lines().any(|l| l == "Win+Alt+V\tOpen clipboard path"));
        let clip_chord = default_chord(clip).unwrap();
        let front = default_chord(index("app.front").unwrap()).unwrap();
        assert_eq!(format_chord(&front), "Win+Shift+E");
        assert!(!overlaps(front, clip_chord));
        for (i, c) in CMDS.iter().enumerate() {
            if i != clip {
                assert!(!overlaps(c.chord(), clip_chord), "{} collides with clip.open", c.id);
            }
        }
        assert_eq!(lookup(b'E' as u16, false, false, true, true), None);
        assert_eq!(lookup(b'3' as u16, true, false, true, false), index("ws.load"));
        assert_eq!(lookup(b'3' as u16, true, true, true, false), index("ws.save"));
        assert_eq!(lookup(b'E' as u16, false, false, true, false), None);
        assert_eq!(lookup(b'C' as u16, true, false, true, false), index("file.winmerge"));
        assert_eq!(lookup(b'C' as u16, true, true, false, false), index("file.path"));
        assert_eq!(lookup(b'B' as u16, true, false, false, false), index("fav.list"));
        assert_eq!(lookup(b'B' as u16, true, true, false, false), index("file.basket"));
        assert_eq!(lookup(b'B' as u16, true, false, true, false), index("file.basket.add"));
        assert_eq!(chord_text(index("file.basket").unwrap()), "Ctrl+Shift+B");
        assert_eq!(chord_text(index("file.basket.add").unwrap()), "Ctrl+Alt+B");
        for id in ["file.basket", "file.basket.add"] {
            let chord = default_chord(index(id).unwrap()).unwrap();
            for c in CMDS.iter() {
                if c.id != id {
                    assert!(!overlaps(c.chord(), chord), "{} collides with {id}", c.id);
                }
            }
        }
        assert_eq!(lookup(b'R' as u16, true, true, false, false), index("file.rename"));
        assert_eq!(chord_text(index("view.recentfiles").unwrap()), "Ctrl+Alt+R");
        assert_eq!(lookup(b'R' as u16, true, false, true, false), index("view.recentfiles"));
        assert_eq!(chord_text(index("file.reveal").unwrap()), "Ctrl+Enter");
        assert_eq!(lookup(0x0D, true, false, false, false), index("file.reveal"));
        assert!(help_text().lines().any(|l| l == "Ctrl+Alt+R\tRecent files"));
        assert!(help_text().lines().any(|l| l == "Ctrl+Enter\tShow in folder"));
        assert_eq!(lookup(0x74, false, false, false, false), None); // F5 stays a pane refresh, not a binding
        let dup = index("tab.dup").unwrap();
        assert_eq!(CMDS[dup].cat, TAB);
        assert_eq!(CMDS[dup].act, crate::Key::DuplicateTab);
        assert_eq!(lookup(b'K' as u16, true, false, true, false), Some(dup));
        assert!(help_text().lines().any(|l| l == "Ctrl+Alt+K\tDuplicate tab"));
        assert_eq!(lookup(b'K' as u16, true, false, false, false), None); // Ctrl+K stays free
        assert_eq!(lookup(b'K' as u16, true, true, true, false), None);
        let list = index("tab.list").unwrap();
        assert_eq!(CMDS[list].cat, TAB);
        assert_eq!(CMDS[list].act, crate::Key::TabList);
        assert_eq!(chord_text(list), "Ctrl+Shift+L");
        assert_eq!(lookup(b'L' as u16, true, true, false, false), Some(list));
        assert_eq!(lookup(b'L' as u16, true, false, false, false), index("nav.addr"));
        assert!(help_text().lines().any(|l| l == "Ctrl+Shift+L\tTab list"));
        let list_chord = default_chord(list).unwrap();
        for (i, c) in CMDS.iter().enumerate() {
            if i != list {
                assert!(!overlaps(c.chord(), list_chord), "{} collides with tab.list", c.id);
            }
        }
        let chord = default_chord(dup).unwrap();
        for (i, c) in CMDS.iter().enumerate() {
            if i != dup {
                assert!(!overlaps(c.chord(), chord), "{} collides with tab.dup", c.id);
            }
        }
    }

    #[test]
    fn workspace_chords_do_not_clash_with_defaults_or_global_hotkeys() {
        fresh();
        let load = index("ws.load").unwrap();
        let save = index("ws.save").unwrap();
        assert_eq!(chord_text(load), "Ctrl+Alt+1 … Ctrl+Alt+9");
        assert_eq!(chord_text(save), "Ctrl+Alt+Shift+1 … Ctrl+Alt+Shift+9");
        let load_c = default_chord(load).unwrap();
        let save_c = default_chord(save).unwrap();
        for (i, c) in CMDS.iter().enumerate() {
            if i != load {
                assert!(!overlaps(c.chord(), load_c), "{} overlaps load workspace", c.id);
            }
            if i != save {
                assert!(!overlaps(c.chord(), save_c), "{} overlaps save workspace", c.id);
            }
        }
        let hot = hot_chord().unwrap();
        let dialog = dialog_chord().unwrap();
        assert!(!overlaps(hot, load_c) && !overlaps(hot, save_c));
        assert!(!overlaps(dialog, load_c) && !overlaps(dialog, save_c));
        assert_eq!(format_chord(&hot), "Win+Shift+E");
        assert_eq!(format_chord(&dialog), "Ctrl+Alt+G");
        assert_eq!(range_chord(load, 4), "Ctrl+Alt+4");
        assert_eq!(range_chord(save, 4), "Ctrl+Alt+Shift+4");
        // Go to pane keeps the chord without Alt.
        assert_eq!(lookup(b'4' as u16, true, false, false, false), index("pane.goto"));
    }

    #[test]
    fn rebind_conflict_swap_reset_and_unknown_keys() {
        fresh();
        let tab = index("tab.new").unwrap();
        let help = index("app.help").unwrap();
        let k = parse_chord("Ctrl+K").unwrap();
        assert_eq!(conflict(tab, k), None);
        rebind(tab, k);
        assert_eq!(lookup(b'K' as u16, true, false, false, false), Some(tab));
        assert_eq!(lookup(b'T' as u16, true, false, false, false), None);
        assert!(is_custom(tab));
        // Help takes Ctrl+T, which is free, then we point New tab back at Ctrl+T and Help receives Ctrl+K.
        rebind(help, parse_chord("Ctrl+T").unwrap());
        rebind(tab, parse_chord("Ctrl+T").unwrap());
        assert_eq!(lookup(b'T' as u16, true, false, false, false), Some(tab));
        assert_eq!(lookup(b'K' as u16, true, false, false, false), Some(help));
        assert_eq!(lookup(0x70, false, false, false, false), None);
        assert_eq!(chord_text(help), "Ctrl+K");

        let mut map = Map::new();
        map.insert("tab.new".into(), serde_json::json!("Ctrl+K"));
        map.insert("tab.close".into(), serde_json::json!("Ctrl+K"));
        map.insert("copy".into(), serde_json::json!("Ctrl+C"));
        map.insert("later".into(), serde_json::json!({"keep": true}));
        load_bindings(&map);
        assert_eq!(chord_text(tab), "Ctrl+K");
        assert_eq!(chord_text(index("tab.close").unwrap()), "Ctrl+W");
        let saved = bindings_json();
        assert_eq!(saved["tab.new"], "Ctrl+K");
        assert!(saved.get("tab.close").is_none());
        assert_eq!(saved["copy"], "Ctrl+C");
        assert_eq!(saved["later"]["keep"], true);
        reset_all();
        let cleared = bindings_json();
        assert!(cleared.get("tab.new").is_none());
        assert_eq!(cleared["copy"], "Ctrl+C");
        assert_eq!(lookup(b'T' as u16, true, false, false, false), Some(tab));

        begin(tab);
        assert!(capturing());
        assert!(matches!(offer(0x11, true, false, false, false), Offer::Ignore));
        assert!(matches!(offer(0x1B, false, false, false, false), Offer::Cancel));
        assert!(!capturing());
        begin(index("pane.goto").unwrap());
        match offer(b'5' as u16, true, true, false, false) {
            Offer::Bind { chord, other, .. } => {
                assert!(chord.digits);
                assert_eq!(format_chord(&chord), "Ctrl+Shift+1 … Ctrl+Shift+9");
                assert_eq!(other, None);
            }
            _ => panic!("digit rebind of go-to-pane should stay a range"),
        }
        fresh();
    }

    #[test]
    fn stored_win_shift_v_for_clip_open_becomes_win_alt_v() {
        fresh();
        let clip = index("clip.open").unwrap();
        let mut old = Map::new();
        old.insert("clip.open".into(), serde_json::json!("Win+Shift+V"));
        load_bindings(&old);
        assert_eq!(chord_text(clip), "Win+Alt+V");
        assert!(bindings_json().get("clip.open").is_none());

        let mut lower = Map::new();
        lower.insert("clip.open".into(), serde_json::json!("win+shift+v"));
        load_bindings(&lower);
        assert_eq!(chord_text(clip), "Win+Alt+V");

        let mut custom = Map::new();
        custom.insert("clip.open".into(), serde_json::json!("Ctrl+Alt+V"));
        load_bindings(&custom);
        assert_eq!(chord_text(clip), "Ctrl+Alt+V");
        assert_eq!(bindings_json()["clip.open"], "Ctrl+Alt+V");
        fresh();
    }

    #[test]
    fn win_chord_parses_and_a_failed_register_can_be_retried() {
        fresh();
        let i = index("app.front").unwrap();
        assert_eq!(chord_text(i), "Win+Shift+E");
        assert_eq!(parse_chord("win+shift+e").unwrap(), default_chord(i).unwrap());
        assert!(hotkey_flip(false, |w| w)); // off, the click registers and it sticks
        assert!(!hotkey_flip(false, |_| false)); // the retry fails again
        assert!(!hotkey_flip(true, |w| {
            assert!(!w);
            false
        })); // turning it off
    }
}
