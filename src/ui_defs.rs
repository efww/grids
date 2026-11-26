use crate::*;

pub(crate) const MAXP: usize = 16;
pub(crate) const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
// child ids inside a pane
pub(crate) const ID_BACK: i32 = 101;
pub(crate) const ID_FWD: i32 = 102;
pub(crate) const ID_UP: i32 = 103;
pub(crate) const ID_EDIT: i32 = 104;
pub(crate) const ID_OPEN_IN: i32 = 105;
pub(crate) const ID_FILTER: i32 = 106;
pub(crate) const ID_FAV: i32 = 107; // ☆+ / ★: add or remove the pane's folder from favourites
// top toolbar buttons (children of the main window)
pub(crate) const ID_TB_LAYOUT: i32 = 201;
pub(crate) const ID_TB_SAVE: i32 = 202;
pub(crate) const ID_TB_LOAD: i32 = 204;
pub(crate) const ID_TB_THEME: i32 = 205;
pub(crate) const ID_TB_FAV: i32 = 206;
pub(crate) const ID_TB_VIEW: i32 = 203;
pub(crate) const ID_TB_EV: i32 = 207; // orange magnifier: Everything search, shown while Everything runs
pub(crate) const ID_TB_UPDATE: i32 = 208; // blue "Update", shown when a newer release is out
pub(crate) const ID_STATUS: i32 = 209; // the status line: an owner-drawn static, so screen readers find it
pub(crate) const ID_STATUS_NOTE: i32 = 210; // the note alone, a polite live region: what screen readers read out by themselves
pub(crate) const ID_DRIVE: i32 = 1500; // + 0..25: drive buttons A: … Z:
pub(crate) const ID_BIN: i32 = 1530; // the Recycle Bin button after the drive buttons
pub(crate) const TIMER_BIN: usize = 1530; // on main: the Recycle Bin is counted again after a short pause
pub(crate) const RECYCLE_BIN: &str = "::{645FF040-5081-101B-9F08-00AA002F954E}";
// menu ids
pub(crate) const ID_HIDDEN: usize = 1100;
pub(crate) const ID_KEYS: usize = 1101;
pub(crate) const ID_GATHER: usize = 1102;
pub(crate) const ID_DBLUP: usize = 1103;
pub(crate) const ID_LANG: usize = 1104;
pub(crate) const ID_HELP: usize = 1105;
pub(crate) const ID_ABOUT: usize = 1106;
pub(crate) const ID_TREE: usize = 1107;
pub(crate) const ID_EXT: usize = 1108;
pub(crate) const ID_HOTKEY: usize = 1109;
pub(crate) const ID_FOLLOW: usize = 1110;
pub(crate) const ID_TABS: usize = 1111;
pub(crate) const ID_KEY_TIPS: usize = 1112; // view menu: shortcut tips on the status line after a mouse action
pub(crate) const ID_TREE_QUICK: usize = 1113; // view menu: Explorer's Quick access (pinned folders) atop the folder tree
pub(crate) const ID_DLGJUMP: usize = 1140;
pub(crate) const ID_CTXMENU: usize = 1145; // view menu: "Open in Grids" in Explorer's right-click menu
pub(crate) const ID_AUTOSTART: usize = 1146; // view menu: start with Windows, minimized
pub(crate) const ID_UPDATES: usize = 1150; // view menu: check for updates
pub(crate) const ID_STAR: usize = 1151; // view menu: star on GitHub
pub(crate) const ID_TOUR: usize = 1152; // view menu: the getting started tour again
pub(crate) const ID_TRAY: usize = 1153; // view menu: minimize to the notification area (tray.rs)
pub(crate) const HOME: &str = "https://github.com/efww/grids";
pub(crate) const ID_ACCENT: usize = 1400; // + preset index
pub(crate) const ID_ACCENT_PICK: usize = 1410;
pub(crate) const ID_ACCENT_SYS: usize = 1411; // selection colour: follow Windows' accent colour
pub(crate) const EM_SETSEL: u32 = 0xB1;
pub(crate) const EM_GETSEL: u32 = 0xB0;
pub(crate) const EM_SCROLLCARET: u32 = 0xB7;
pub(crate) const EN_CHANGE: u32 = 0x0300;
pub(crate) const TIMER_STATUS: usize = 1;
pub(crate) const TIMER_FILTER: usize = 2;
pub(crate) const TIMER_REVEAL: usize = 3;
pub(crate) const TIMER_SETTLE: usize = 4; // on a pane: narrow column widths going back on after a folder change
pub(crate) const TIMER_SPIN: usize = 6; // on a pane: next frame of the F5 spin
pub(crate) const TIMER_FOCUS: usize = 5; // after a click brought Grids back: that click is handled, focus still nowhere?
// tree-view messages (the sidebar's SysTreeView32)
pub(crate) const TVM_GETNEXTITEM: u32 = 0x110A;
pub(crate) const TVM_ENSUREVISIBLE: u32 = 0x1114;
pub(crate) const TVM_GETITEMRECT: u32 = 0x1104;
pub(crate) const TVGN_CARET: usize = 9;
pub(crate) const TVM_SETBKCOLOR: u32 = 0x111D;
pub(crate) const TVM_GETTOOLTIPS: u32 = 0x1119;
// list-view messages (the Explorer list inside each pane)
pub(crate) const LVM_SETBKCOLOR: u32 = 0x1001;
pub(crate) const LVM_GETHEADER: u32 = 0x101F;
pub(crate) const LVM_GETTOOLTIPS: u32 = 0x104E;
pub(crate) const LVM_GETCOLUMNORDERARRAY: u32 = 0x103B;
pub(crate) const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
pub(crate) const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = 0x1037;
pub(crate) const LVS_EX_AUTOSIZECOLUMNS: usize = 0x1000_0000;
pub(crate) const LVS_EX_COLUMNSNAPPOINTS: usize = 0x4000_0000;
pub(crate) const HDM_GETITEMCOUNT: u32 = 0x1200;
pub(crate) const LVM_SETTEXTCOLOR: u32 = 0x1024;
pub(crate) const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
pub(crate) const LVM_SETSELECTEDCOLUMN: u32 = 0x108C;
pub(crate) const LVM_GETSELECTEDCOLUMN: u32 = 0x10AE;
pub(crate) const TVM_SETTEXTCOLOR: u32 = 0x111E;
pub(crate) const COPY_ID: usize = 0x4752_4944; // "GRID": WM_COPYDATA from a second instance
// Move without copying old pixels: copied bits are what left ghost borders/buttons while resizing.
pub(crate) const MOVE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_NOCOPYBITS.0);

// Themes, 0xRRGGBB. Light: an ivory frame around white plates; dark: charcoal. Each theme has its own
// selection colours (same names, tuned to the ground): pastels on light, muted mid tones on dark.
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub(crate) bg: u32,   // frame: window, toolbar, gaps, status line
    pub(crate) card: u32, // plates and lists
    pub(crate) line: u32,
    pub(crate) pill: u32, // fields
    pub(crate) text: u32,
    pub(crate) sub: u32,
    pub(crate) warn: u32, // a drive 90% or more full: its status line bar, on line
    pub(crate) accents: [(&'static str, u32); 6],
}

pub(crate) const LIGHT: Palette = Palette {
    bg: 0xF3F1EC,
    card: 0xFFFFFF,
    line: 0xE4E0D8,
    pill: 0xF4F2EE,
    text: 0x2A2825,
    sub: 0x7C776F,
    warn: 0xC42B1C,
    accents: [("하늘", 0x8DB5E8), ("그레이", 0xC5C5C5), ("라벤더", 0xA99BE0), ("세이지", 0x8CC5A8), ("피치", 0xEFAE92), ("로즈", 0xE7A1B5)],
};

pub(crate) const DARK: Palette = Palette {
    bg: 0x1C1D20,
    card: 0x26282C,
    line: 0x36393E,
    pill: 0x303338,
    text: 0xE6E8EB,
    sub: 0x9AA0A8,
    warn: 0xFF6B6B,
    accents: [("하늘", 0x4C6FA8), ("그레이", 0x4B4B4B), ("라벤더", 0x6A5AA8), ("세이지", 0x3E8676), ("피치", 0xA8624A), ("로즈", 0xA65A78)],
};

pub(crate) static DARK_ON: AtomicBool = AtomicBool::new(false);
pub(crate) static KO: AtomicBool = AtomicBool::new(false);
pub(crate) static TABS_ON: AtomicBool = AtomicBool::new(true); // copy of Session::tabs_on for pane_geo

/// UI text: English unless Korean is picked in the view menu (the Explorer parts follow Windows).
pub(crate) fn tr(en: &'static str, ko: &'static str) -> &'static str {
    if KO.load(Ordering::Relaxed) {
        ko
    } else {
        en
    }
}

pub(crate) const ACCENT_EN: [&str; 6] = ["Sky", "Gray", "Lavender", "Sage", "Peach", "Rose"];

// The colours Grids paints with now: Windows' own under high contrast, else the light or dark theme.
pub(crate) fn pal() -> Palette {
    match hc_colors() {
        Some(c) => hc_palette(&c),
        None => *theme_pal(DARK_ON.load(Ordering::Relaxed)),
    }
}

// The saved theme's palette, whatever is on screen: selection colour presets, menu ticks, colour labels.
pub(crate) fn theme_pal(dark: bool) -> &'static Palette {
    if dark {
        &DARK
    } else {
        &LIGHT
    }
}

// Windows high contrast (Settings > Accessibility > Contrast themes): Grids' own chrome takes the theme's
// system colours, in pairs, instead of its palette. Saved settings (dark, accent) are left alone.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) struct SysColors {
    pub(crate) face: u32, // COLOR_BTNFACE and the text on it, COLOR_BTNTEXT
    pub(crate) face_text: u32,
    pub(crate) window: u32, // COLOR_WINDOW / COLOR_WINDOWTEXT
    pub(crate) window_text: u32,
    pub(crate) gray: u32, // COLOR_GRAYTEXT
    pub(crate) hl: u32,   // COLOR_HIGHLIGHT / COLOR_HIGHLIGHTTEXT
    pub(crate) hl_text: u32,
}

pub(crate) static HC_ON: AtomicBool = AtomicBool::new(false);
// The SysColors fields in order, 0xRRGGBB; read only while HC_ON. The UI thread writes them.
static HC_SYS: [AtomicU32; 7] = [const { AtomicU32::new(0) }; 7];

pub(crate) fn hc_colors() -> Option<SysColors> {
    if !HC_ON.load(Ordering::Relaxed) {
        return None;
    }
    let v = |k: usize| HC_SYS[k].load(Ordering::Relaxed);
    Some(SysColors { face: v(0), face_text: v(1), window: v(2), window_text: v(3), gray: v(4), hl: v(5), hl_text: v(6) })
}

// Reads high contrast and its colours again; true when either changed (switched on or off, or another
// contrast theme picked while on).
pub(crate) unsafe fn refresh_hc() -> bool {
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    let mut hc = HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    let got = SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, Some(&mut hc as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    let on = got.is_ok() && hc.dwFlags.0 & HCF_HIGHCONTRASTON.0 != 0;
    // GetSysColor is BGR; swapping red and blue (cref) gives 0xRRGGBB
    let sys = |i: SYS_COLOR_INDEX| crate::draw::cref(GetSysColor(i)).0;
    let now = [COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT].map(sys);
    let mut changed = on != HC_ON.load(Ordering::Relaxed);
    for (slot, c) in HC_SYS.iter().zip(now) {
        // also while off: colours can land after the switch-off notice, and the list keeps them until recoloured
        changed |= slot.swap(c, Ordering::Relaxed) != c;
    }
    HC_ON.store(on, Ordering::Relaxed);
    changed
}

// The palette for a contrast theme. One `text` is painted on frame, plates and fields alike, so the frame
// takes the button face only where its text is that same ink and reads; grey text only where it reads.
pub(crate) fn hc_palette(c: &SysColors) -> Palette {
    let (card, text) = (c.window, c.window_text);
    let bg = if c.face_text == text && contrast(text, c.face) >= 4.5 { c.face } else { card };
    let sub = if contrast(c.gray, bg).min(contrast(c.gray, card)) >= 4.5 { c.gray } else { text };
    // the bar's track is the text colour here, so a full drive shows in the highlight (bar_fill then keeps
    // the usual fill off the highlight, which is also the selection colour)
    Palette { bg, card, line: text, pill: card, text, sub, warn: c.hl, accents: LIGHT.accents.map(|(n, _)| (n, c.hl)) }
}

// WCAG 2 contrast ratio of two 0xRRGGBB colours.
pub(crate) fn contrast(a: u32, b: u32) -> f64 {
    let (x, y) = (rel_lum(a), rel_lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

// WCAG 2 relative luminance 0..1 of an 0xRRGGBB colour.
pub(crate) fn rel_lum(c: u32) -> f64 {
    let lin = |v: u32| {
        let s = v as f64 / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c >> 16 & 0xFF) + 0.7152 * lin(c >> 8 & 0xFF) + 0.0722 * lin(c & 0xFF)
}

// Windows' accent colour (Settings > Personalization > Colors) as Grids shows it, light theme then dark;
// NO_SYS until read, or where Windows gives none (black is a colour, so not 0).
pub(crate) const NO_SYS: u32 = u32::MAX;
static SYS_ACCENT: [AtomicU32; 2] = [const { AtomicU32::new(NO_SYS) }; 2];

pub(crate) fn sys_accent(dark: bool) -> Option<u32> {
    Some(SYS_ACCENT[dark as usize].load(Ordering::Relaxed)).filter(|&c| c != NO_SYS)
}

// Of Windows' shades of its accent, the one for a theme: among those whose text (on_color) reads at 4.5:1,
// the nearest in lightness to the theme's Sky preset, so it sits with the pastel / muted presets. When none
// reads, the most legible one.
pub(crate) fn sys_accent_shade(shades: &[u32], dark: bool) -> Option<u32> {
    let target = rel_lum(theme_pal(dark).accents[0].1);
    let legibility = |c: u32| contrast(plain_on_color(c), c);
    let near = shades.iter().copied().filter(|&c| legibility(c) >= 4.5).min_by(|&a, &b| (rel_lum(a) - target).abs().total_cmp(&(rel_lum(b) - target).abs()));
    near.or_else(|| shades.iter().copied().max_by(|&a, &b| legibility(a).total_cmp(&legibility(b))))
}

// HKCU\...\Explorer\Accent\AccentPalette: 8 colours of 4 bytes R, G, B, A, lightest first. The 8th is a
// colour of another hue (for alerts), so only the first 7. Anything but 32 bytes: not this format.
pub(crate) fn accent_palette(bytes: &[u8]) -> Option<Vec<u32>> {
    (bytes.len() == 32).then(|| bytes.chunks(4).take(7).map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect())
}

// One colour (DWM's colorization, where the palette is missing) spread into lighter and darker shades.
pub(crate) fn shades_of(c: u32) -> Vec<u32> {
    let mut v: Vec<u32> = [0.6, 0.4, 0.2].map(|t| blend(c, 0xFFFFFF, t)).into();
    v.push(c);
    v.extend([0.2, 0.4, 0.6].map(|t| blend(c, 0, t)));
    v
}

// Reads Windows' accent colour again; true when the shade for either theme changed (the setting broadcast
// comes in bursts, so only a real change repaints).
pub(crate) unsafe fn refresh_sys_accent() -> bool {
    use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY};
    let (mut buf, mut n) = ([0u8; 64], 64u32);
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent");
    let got = RegGetValueW(HKEY_CURRENT_USER, key, w!("AccentPalette"), RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as _), Some(&mut n)).is_ok();
    let palette = buf.get(..n as usize).filter(|_| got).and_then(accent_palette);
    let shades = palette.or_else(|| {
        let (mut c, mut opaque) = (0u32, BOOL(0));
        // 0xAARRGGBB: already red, green, blue in order (not a COLORREF)
        DwmGetColorizationColor(&mut c, &mut opaque).ok().map(|_| shades_of(c & 0xFFFFFF))
    });
    let mut changed = false;
    for (dark, slot) in [false, true].into_iter().zip(&SYS_ACCENT) {
        let c = shades.as_deref().and_then(|s| sys_accent_shade(s, dark)).unwrap_or(NO_SYS);
        changed |= slot.swap(c, Ordering::Relaxed) != c;
    }
    changed
}

// Status line note (and theme button tooltip) while a theme or colour pick only changes the saved setting.
pub(crate) fn hc_text() -> &'static str {
    tr("High contrast is on — Windows colours are used", "고대비 테마 사용 중 — Windows 색을 씁니다")
}

pub(crate) const INK: u32 = 0x2A2825; // dark text on light selection colours
pub(crate) const GREEN: u32 = 0x1FA463; // unsaved layout changes
pub(crate) fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = ((a >> sh & 0xFF) as f32, (b >> sh & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(16) | ch(8) | ch(0)
}

// Lightness 0..255 of an 0xRRGGBB colour (Rec. 709 weights on the stored values).
fn luma(c: u32) -> f32 {
    0.2126 * (c >> 16 & 0xFF) as f32 + 0.7152 * (c >> 8 & 0xFF) as f32 + 0.0722 * (c & 0xFF) as f32
}

// The selection colour while another app has the keyboard: its hue drained to a grey of the same lightness,
// then 30% toward the plate, so the active pane still shows but no longer looks typed into (also for Gray).
pub(crate) fn quiet_accent(accent: u32, card: u32) -> u32 {
    let grey = luma(accent).round() as u32 * 0x010101;
    blend(blend(accent, grey, 0.75), card, 0.3)
}

// Text/icons on a coloured fill: near-black on light colours, white on dark ones (black accent -> white text).
// Under high contrast the highlight takes the theme's own highlight text.
pub(crate) fn on_color(bg: u32) -> u32 {
    if let Some(c) = hc_colors().filter(|c| c.hl == bg) {
        return c.hl_text;
    }
    plain_on_color(bg)
}

fn plain_on_color(bg: u32) -> u32 {
    if luma(bg) > 160.0 {
        INK
    } else {
        0xFFFFFF
    }
}
// Icon code points, the same in Segoe Fluent Icons (used on Windows 11) and Segoe MDL2 Assets (fallback); see fonts.rs
pub(crate) const ICON_SEARCH: &str = "\u{E721}";
pub(crate) const ICON_FILTER: &str = "\u{E71C}";
pub(crate) const ICON_CLOSE: &str = "\u{E711}";
pub(crate) const ICON_PREV: &str = "\u{E76B}";
pub(crate) const ICON_NEXT: &str = "\u{E76C}";

pub(crate) fn glyph(id: i32) -> &'static str {
    match id {
        ID_BACK => "\u{E72B}",
        ID_FWD => "\u{E72A}",
        ID_UP => "\u{E74A}",
        ID_OPEN_IN => "\u{E8AB}",
        ID_TB_LAYOUT => "\u{E8A9}",
        ID_TB_SAVE => "\u{E74E}",
        ID_TB_LOAD => "\u{E838}",
        ID_TB_FAV => "\u{E734}",
        ID_TB_EV => ICON_SEARCH,
        ID_BIN => "\u{E74D}", // a bin, only when the shell's own icon isn't there
        ID_TB_THEME if DARK_ON.load(Ordering::Relaxed) => "\u{E706}", // sun: back to light
        ID_TB_THEME => "\u{E708}",                                     // moon: go dark
        _ => "\u{E890}",
    }
}

// Filter icon / Alt+↓ menu: (English, Korean, filter spec). No spaces, so filter_spec keeps them as they are.
pub(crate) const FILE_KINDS: [(&str, &str, &str); 6] = [
    ("Pictures", "사진", "*.jpg;*.jpeg;*.png;*.gif;*.bmp;*.webp;*.heic;*.tif;*.tiff;*.svg;*.ico"),
    ("Videos", "동영상", "*.mp4;*.mkv;*.avi;*.mov;*.wmv;*.webm;*.m4v"),
    ("Music", "음악", "*.mp3;*.flac;*.wav;*.m4a;*.aac;*.ogg;*.wma"),
    ("Documents", "문서", "*.pdf;*.doc;*.docx;*.xls;*.xlsx;*.ppt;*.pptx;*.hwp;*.hwpx;*.txt;*.md;*.csv;*.rtf;*.odt"),
    ("Archives", "압축", "*.zip;*.7z;*.rar;*.tar;*.gz;*.iso"),
    ("Programs", "프로그램", "*.exe;*.msi;*.bat;*.cmd;*.ps1"),
];

pub(crate) const KEYS_EN: &str = "Ctrl+Tab / Ctrl+Shift+Tab\tNext / previous pane\n\
Ctrl+1 … Ctrl+9\tGo to pane 1–9 (the number at the left of each pane header; hidden in very narrow panes)\n\
Alt+Shift+← / → / ↑ / ↓\tPane to the left / right / above / below\n\
Ctrl+Alt+Shift+← / → / ↑ / ↓\tSwap the active pane with the one to the left / right / above / below (folder, history and selection stay; also in the ⇄ menu)\n\
Ctrl+Alt+← / → / ↑ / ↓\tMove the active pane's splitter that way (hold to keep going; the panes follow on release)\n\
Alt+← / Alt+→\tBack / forward\n\
Alt+↑\tParent folder\n\
Alt+PgDn / Alt+PgUp\tNext / previous folder in the same parent folder (Explorer's order; the status line shows which one, e.g. Folder 4/12)\n\
Alt+↓\tRecent folders (all panes, newest first; right-click ← / → too)\n\
Ctrl+G\tGo to a Windows folder in the active pane: Home, Desktop, Documents, Downloads, Pictures, Music, Videos, This PC, Recycle Bin, Temp, AppData (then its underlined letter, e.g. L for Downloads; not Ctrl+Alt+G, the file dialog jump)\n\
Ctrl+L, Alt+D, F4\tAddress bar: also .. or a subfolder name, a file to open, or a command such as cmd, wt, powershell (runs in this pane's folder), or part of a recent or favourite folder's name\n\
Ctrl+B\tFavourites list (↑/↓ or Home/End to choose, Enter to open, 1 … 9 opens the numbered row at once, Del to remove, Alt+↑/↓ or drag to reorder)\n\
Ctrl+Shift+B\tAdd the active pane's folder to favourites, or remove it if it is one (as ☆+ in its header)\n\
Ctrl+O\tSaved sessions, the one you're in picked (arrow keys or Home/End to choose, Enter to switch, 1 … 9 opens the numbered card at once, F2 to rename, Del to delete, Esc or Ctrl+O to close)\n\
Ctrl+/\tKeyboard shortcuts (this list; Ctrl+F or Tab: find box, in English or Korean words)\n\
Ctrl+Shift+M\tMaximize / restore the active pane (or double-click its header)\n\
Ctrl+Shift+E\tHide / show the folder tree (its width is kept)\n\
Ctrl+Shift+O\tOpen the folders of Explorer windows and tabs in panes (panes already showing one stay put)\n\
Ctrl+Shift+C\tCopy the full path of the selected items (or of the current folder)\n\
Ctrl+Shift+U\tFolder usage: what takes up space in this folder, biggest first with bars, filled in as the subfolders are summed (→ into a folder, ← up, Enter selects the item in the pane, Ctrl+Enter opens the folder, Esc closes; also in the ⇄ menu)\n\
Ctrl+M\t⇄ menu from the file list: open in another pane, copy / move the selected items to one, swap places and the other ⇄ items. Then one key: a pane's number opens there, C / M then a number copies / moves, S swaps, W which program is using the files, K checksum (files only), D date folders (files only; then Y / M / D), N into a new folder, B dated backup copies, E removes empty folders, U folder usage, L copies the file list as a table, R WinMerge (when installed)\n\
Ctrl+Shift+R\tBatch rename the selected items (none selected: every file in the folder): rules such as replace, numbering, date, case; live preview, Undo, presets. In the window: Ctrl+Enter renames, Esc closes; in the rule list Space switches a rule on/off, Ins adds, Del removes, Alt+↑/↓ moves it\n\
Num+ / Ctrl+Shift+A\tSelect files by pattern (*.jpg;*.png or part of a name; Shift+Enter deselects)\n\
Num−\tDeselect files by pattern\n\
Num*\tInvert the selection\n\
Insert\tSelect or deselect the focused item and move to the next (hold to mark several in a row)\n\
Ctrl+F\tFilter (*.pdf;*.txt or part of a name; folders always show)\n\
Alt+↓ in the filter\tFile types: pictures, videos, documents ... (or click the filter's icon)\n\
Ctrl+Shift+F\tSearch the whole PC with Everything, while it runs (or the orange magnifier): ↑/↓ choose, Enter shows the item in the active pane, Ctrl+Enter opens a folder, Alt+O only under this folder\n\
Ctrl+T / Ctrl+W\tNew tab (same folder) / close tab, with tabs on (View menu)\n\
Ctrl+K\tDuplicate the current tab\n\
Ctrl+Enter / Ctrl+Shift+Enter\tIn the file list, with tabs on: open the selected folders (or the focused one) as new tabs after this one / open the focused folder in a new tab and go there\n\
Ctrl+PgDn / Ctrl+PgUp\tNext / previous tab\n\
Alt+1 … Alt+9\tGo to tab 1–8 of the active pane; Alt+9: its last tab\n\
Ctrl+Shift+PgDn / Ctrl+Shift+PgUp\tMove the current tab right / left (also on a tab's right-click menu)\n\
Ctrl+F3 / Ctrl+F5 / Ctrl+F6\tSort by name / date modified / size (again: reverse; the pane keeps it)\n\
F7\tNew folder\n\
Shift+F7\tNew text file (named, selected and ready to rename)\n\
Ctrl+F7\tMove the selected items into a new folder named after them, ready to rename (Ctrl+Z moves them back)\n\
Shift+F5\tCopy the selected items next to them with the date and time added: Report.docx → Report (2026-11-27 14.30).docx (Ctrl+Z removes the copies)\n\
F6 / Shift+F6\tMove between file list, address bar, filter and folder tree\n\
Enter / Esc\tApply / cancel the address or filter\n\
Ctrl+H\tShow hidden files\n\
F3, Space\tQuick preview of the focused file: pictures, text and code, Markdown, CSV tables, Office / PDF / HTML / mail (files up to 10 MB), HWP / HWPX, ZIP contents, folders; anything else as a card of facts (arrows or the wheel: previous / next file without closing; on long text the wheel scrolls first and moves on past the end; PgUp / PgDn: scroll; Enter on an online-only cloud file: download it and preview; Esc: close, also while it is still opening)\n\
Del / Shift+Del, F2\tIn the preview: the file shown to the Recycle Bin / deleted for good (Windows asks as in Explorer; the preview moves on to the next file), or renamed in its header (Enter / Esc)\n\
Ctrl+wheel, Ctrl+= / Ctrl+- / Ctrl+0\tIn the preview: zoom a picture or PDF in / out (the wheel zooms on the cursor) / back to fit; zoomed in, drag it or its scrollbars to move around\n\
Ctrl+V\tWith a copied picture (screenshot, browser image): save it here as a PNG file; with copied text: save it as a text file named after its first line, ready to rename\n\
Alt / F10\tView menu (Alt pressed and let go alone): settings, change layout, light / dark theme, language, help; ↑/↓ and Enter to choose, Esc to close\n\
F1\tHelp (opens the README on GitHub)\n\
Win+Alt+E\tBring Grids to front from any app (again: minimize)\n\
Ctrl+Alt+G\tIn another app's Open / Save dialog: jump to a folder open in Grids\n\
F2, Del, Ctrl+C/X/V, F5\tSame as Explorer\n\n\
Address bar: click a folder in the path to go there; click a › to list the folders inside; click the current folder or empty space, or press Ctrl+L, to type a path\n\
Layout button: click for an even grid, press and drag for rows per column (arrow keys + Enter work too)\n\
⇄ button: open the selected folder (or the current one) in another pane, with this pane's columns; or copy / move the selected files (up to 10,000) to another pane's folder, a favourite or recent folder, or any folder you choose; swap places with another pane; which program is using the selected files; SHA-256 checksum of the selected files; sort the selected files into date folders; move the selected items into a new folder (Ctrl+F7); make dated backup copies of the selected items (Shift+F5); remove empty folders in this folder; what takes up space in this folder (Ctrl+Shift+U); copy the file list (name, date modified, type, size, full path) as a table for Excel; Compare with WinMerge (2 selected items, or this folder with another pane's) when WinMerge is installed\n\
Save (Ctrl+S): overwrite the current session file; green dot = unsaved layout changes (not folder moves)\n\
Load (Ctrl+O): pick a saved session by its layout preview; point at a card for its ✕ to delete it, or right-click it to rename, delete or put a shortcut to it on the desktop\n\
Drive buttons (C, D …): open that drive in the active pane; its drive is underlined; right-click: the drive menu as in Explorer (Eject, Properties …); drop files on one to copy / move them there as in Explorer (same drive moves; Ctrl copies, Shift moves), or on the Recycle Bin button to delete them (Shift: for good)\n\
Double-click a splitter: split evenly\n\
Update button (blue, top right): a newer Grids is out; click to restart into it (settings, folders and sessions stay)\n\
Pane header: right-click for a colour label (saved with the session); double-click to maximize / restore the pane; drop a folder on it to open it there (a file: its folder)\n\
Tabs: click to switch, × or middle-click to close, + or double-click the empty strip for a new tab, right-click to duplicate, move left / right or close; drop files on another tab to copy / move them into its folder as in Explorer (Ctrl copies, Shift moves)\n\
Arguments: grids.exe C:\\a D:\\b ... or grids.exe work.grids\n\
If Grids is already running, they open in that window.";

pub(crate) const KEYS_KO: &str = "Ctrl+Tab / Ctrl+Shift+Tab\t다음 / 이전 패널\n\
Ctrl+1 … Ctrl+9\t1~9번 패널로 이동 (패널 머리줄 왼쪽의 번호, 아주 좁은 패널에서는 생략)\n\
Alt+Shift+← / → / ↑ / ↓\t왼쪽·오른쪽·위·아래 패널로 이동\n\
Ctrl+Alt+Shift+← / → / ↑ / ↓\t지금 패널을 왼쪽·오른쪽·위·아래 패널과 자리 바꾸기 (폴더·방문 기록·선택 유지, ⇄ 메뉴에도 있음)\n\
Ctrl+Alt+← / → / ↑ / ↓\t지금 패널의 경계선을 그쪽으로 옮기기 (누르고 있으면 계속, 손을 떼면 패널이 맞춰짐)\n\
Alt+← / Alt+→\t뒤로 / 앞으로\n\
Alt+↑\t상위 폴더\n\
Alt+PgDn / Alt+PgUp\t같은 상위 폴더 안의 다음 / 이전 폴더 (탐색기 순서, 상태줄에 '폴더 4/12'처럼 몇 번째인지 표시)\n\
Alt+↓\t최근 폴더 (모든 패널, 최근 순, ← / → 오른쪽 클릭도 가능)\n\
Ctrl+G\t지금 패널을 Windows 기본 폴더로 이동: 홈·바탕 화면·문서·다운로드·사진·음악·동영상·내 PC·휴지통·임시 폴더·AppData (이어서 밑줄 글자, 예: L은 다운로드, 파일 대화 상자용 Ctrl+Alt+G와는 다른 키)\n\
Ctrl+L, Alt+D, F4\t주소창: .. 이나 하위 폴더 이름, 열 파일, cmd·wt·powershell 같은 명령도 가능(이 패널 폴더에서 실행), 다녀온 폴더·즐겨찾기 이름 일부\n\
Ctrl+B\t즐겨찾기 목록 (↑/↓·Home/End 고르기, Enter 열기, 1 … 9는 번호가 붙은 줄 바로 열기, Del 빼기, Alt+↑/↓ 또는 끌어서 순서 바꾸기)\n\
Ctrl+Shift+B\t선택한 패널의 폴더를 즐겨찾기에 추가, 이미 있으면 빼기 (머리줄 ☆+와 같음)\n\
Ctrl+O\t저장한 세션 (지금 세션이 골라진 채 열림, 방향키·Home/End 고르기, Enter 바꾸기, 1 … 9는 번호가 붙은 카드 바로 열기, F2 이름 바꾸기, Del 삭제, Esc 또는 Ctrl+O 닫기)\n\
Ctrl+/\t단축키 목록 (이 창, Ctrl+F 또는 Tab: 찾기 칸, 영어·한국어 낱말 모두)\n\
Ctrl+Shift+M\t선택한 패널 크게 보기 / 복원 (머리줄 더블클릭도 가능)\n\
Ctrl+Shift+E\t폴더 트리 숨기기 / 보이기 (폭은 그대로 유지)\n\
Ctrl+Shift+O\t탐색기 창·탭의 폴더를 패널로 가져오기 (이미 그 폴더를 연 패널은 그대로)\n\
Ctrl+Shift+C\t선택한 항목(없으면 현재 폴더)의 전체 경로 복사\n\
Ctrl+Shift+U\t이 폴더 용량 분석: 공간을 차지하는 항목을 큰 순서로 막대와 함께, 하위 폴더를 계산하는 대로 채움 (→ 폴더 안으로, ← 위로, Enter는 패널에서 그 항목 선택, Ctrl+Enter는 폴더 열기, Esc 닫기, ⇄ 메뉴에도 있음)\n\
Ctrl+M\t파일 목록에서 ⇄ 메뉴 열기: 다른 패널에서 열기, 선택 항목 복사·이동, 자리 바꾸기와 그 밖의 ⇄ 항목. 이어서 키 하나로: 패널 번호는 그 패널에서 열기, C / M 다음 번호는 복사 / 이동, S는 자리 바꾸기, W는 파일을 쓰고 있는 프로그램, K는 체크섬(파일만), D는 날짜별 폴더(파일만, 이어서 Y / M / D), N은 새 폴더로 묶기, B는 날짜 붙인 사본, E는 빈 폴더 지우기, U는 용량 분석, L은 파일 목록을 표로 복사, R은 WinMerge 비교(설치된 경우)\n\
Ctrl+Shift+R\t선택한 항목(선택 없으면 폴더의 모든 파일) 일괄 이름 바꾸기: 바꾸기·번호·날짜·대소문자 같은 규칙, 실시간 미리보기, 되돌리기, 프리셋. 창 안에서 Ctrl+Enter 바꾸기, Esc 닫기, 규칙 목록에서 Space 켜기/끄기, Ins 추가, Del 빼기, Alt+↑/↓ 순서 바꾸기\n\
Num+ / Ctrl+Shift+A\t패턴으로 파일 선택 (*.jpg;*.png 또는 이름 일부, Shift+Enter는 선택 해제)\n\
Num−\t패턴으로 파일 선택 해제\n\
Num*\t선택 반전\n\
Insert\t포커스한 항목 선택·해제 후 다음 항목으로 (누르고 있으면 연달아 선택)\n\
Ctrl+F\t필터 (*.pdf;*.txt 또는 이름 일부, 폴더는 항상 표시)\n\
필터 칸에서 Alt+↓\t파일 종류: 사진·동영상·문서 … (필터 칸 아이콘 클릭도 가능)\n\
Ctrl+Shift+F\tEverything으로 PC 전체 검색, Everything이 실행 중일 때 (주황 돋보기도 가능): ↑/↓ 고르기, Enter는 지금 패널에서 그 항목 보기, Ctrl+Enter는 폴더 열기, Alt+O는 이 폴더 아래에서만\n\
Ctrl+T / Ctrl+W\t새 탭(같은 폴더) / 탭 닫기, 탭을 켰을 때 (보기 메뉴)\n\
Ctrl+K\t지금 탭 복제\n\
Ctrl+Enter / Ctrl+Shift+Enter\t파일 목록에서, 탭을 켰을 때: 선택한 폴더(또는 포커스한 폴더)를 지금 탭 뒤에 새 탭으로 열기 / 포커스한 폴더를 새 탭으로 열고 바로 그 탭으로\n\
Ctrl+PgDn / Ctrl+PgUp\t다음 / 이전 탭\n\
Alt+1 … Alt+9\t지금 패널의 1~8번째 탭으로 이동, Alt+9는 마지막 탭\n\
Ctrl+Shift+PgDn / Ctrl+Shift+PgUp\t지금 탭을 오른쪽 / 왼쪽으로 옮기기 (탭 오른쪽 클릭 메뉴에도 있음)\n\
Ctrl+F3 / Ctrl+F5 / Ctrl+F6\t이름 / 수정한 날짜 / 크기순 정렬 (다시 누르면 반대 순서, 패널이 기억)\n\
F7\t새 폴더\n\
Shift+F7\t새 텍스트 파일 (이름 짓기 바로 시작)\n\
Ctrl+F7\t선택한 항목을 공통 이름의 새 폴더로 묶고 이름 짓기 바로 시작 (Ctrl+Z로 이동 되돌리기)\n\
Shift+F5\t선택한 항목 옆에 날짜·시각을 붙인 사본 만들기: 보고서.docx → 보고서 (2026-11-27 14.30).docx (Ctrl+Z로 사본 지우기)\n\
F6 / Shift+F6\t파일 목록·주소창·필터·폴더 트리 사이 이동\n\
Enter / Esc\t주소창·필터 적용 / 취소\n\
Ctrl+H\t숨김 파일 표시\n\
F3, Space\t포커스한 파일 빠른 미리보기: 사진, 텍스트·코드, 마크다운, CSV 표, Office·PDF·HTML·메일(10 MB까지), HWP·HWPX, ZIP 목록, 폴더. 나머지는 정보 카드 (방향키·휠: 닫지 않고 이전·다음 파일, 긴 텍스트에서는 휠이 먼저 스크롤하고 끝에서 넘어감, PgUp/PgDn: 스크롤, 온라인 전용 클라우드 파일에서 Enter: 내려받아 미리보기, Esc: 닫기, 여는 중에도 바로)\n\
Del / Shift+Del, F2\t미리보기에서: 보고 있는 파일을 휴지통으로 / 완전히 삭제 (탐색기처럼 Windows가 확인, 미리보기는 다음 파일로), 또는 머리줄에서 이름 바꾸기 (Enter / Esc)\n\
Ctrl+휠, Ctrl+= / Ctrl+- / Ctrl+0\t미리보기에서: 사진·PDF 확대 / 축소 (휠은 커서 위치 기준) / 화면에 맞춤, 확대한 상태에서는 끌거나 스크롤바로 이동\n\
Ctrl+V\t복사한 그림(스크린샷, 브라우저 이미지)이 있으면 이 폴더에 PNG 파일로 저장, 복사한 글은 텍스트 파일로 저장(첫 줄이 이름, 바로 이름 바꾸기)\n\
Alt / F10\t보기 메뉴 (Alt는 혼자 눌렀다 떼기): 설정, 레이아웃 바꾸기, 라이트/다크 테마, 언어, 도움말. ↑/↓와 Enter로 고르고 Esc로 닫기\n\
F1\t도움말 (GitHub README 열기)\n\
Win+Alt+E\t어디서나 Grids 앞으로 (다시 누르면 최소화)\n\
Ctrl+Alt+G\t다른 앱의 열기·저장 대화 상자에서: Grids에 열린 폴더로 이동\n\
F2, Del, Ctrl+C/X/V, F5\t탐색기와 동일\n\n\
주소 칸: 경로의 폴더를 누르면 그리로 이동, › 를 누르면 그 안의 폴더 목록, 현재 폴더·빈 곳을 누르거나 Ctrl+L이면 경로 입력\n\
레이아웃 버튼: 클릭 = 균등 배치, 누른 채 끌기 = 열마다 칸 수 (방향키+Enter도 가능)\n\
⇄ 버튼: 선택한 폴더(없으면 현재 폴더)를 다른 패널에서 열기, 열 순서·폭·정렬도 함께. 또는 선택한 파일을 다른 패널 폴더·즐겨찾기·최근 폴더·직접 고른 폴더로 복사·이동 (최대 10,000개). 다른 패널과 자리 바꾸기. 선택한 파일을 쓰고 있는 프로그램 확인. 선택한 파일의 체크섬(SHA-256). 선택한 파일을 날짜별 폴더로 정리. 선택한 항목을 새 폴더로 묶기 (Ctrl+F7). 선택한 항목의 날짜 붙인 사본 만들기 (Shift+F5). 이 폴더 안의 빈 폴더 지우기. 이 폴더 용량 분석 (Ctrl+Shift+U). 파일 목록(이름·수정한 날짜·유형·크기·전체 경로)을 엑셀용 표로 복사. WinMerge가 있으면 WinMerge로 비교 (선택한 2개, 또는 이 폴더와 다른 패널 폴더)\n\
저장 (Ctrl+S): 지금 세션 파일에 바로 덮어쓰기, 초록 점 = 저장 안 된 배치 변경(폴더 이동은 제외)\n\
불러오기 (Ctrl+O): 저장한 세션을 배치 미리보기로 보고 한 번 클릭, 카드에 마우스를 올리면 ✕로 삭제, 오른쪽 클릭으로 이름 바꾸기·삭제·바탕 화면 바로 가기 만들기\n\
드라이브 버튼 (C, D …): 지금 패널에서 그 드라이브 열기, 지금 패널의 드라이브에 밑줄, 오른쪽 클릭: 탐색기와 같은 드라이브 메뉴(꺼내기·속성 …), 파일을 끌어다 놓으면 탐색기처럼 그 드라이브로 복사·이동(같은 드라이브는 이동, Ctrl 복사·Shift 이동), 휴지통 버튼에 놓으면 휴지통으로(Shift: 영구 삭제)\n\
스플리터 더블클릭: 균등 분할\n\
업데이트 버튼 (파란색, 오른쪽 위): 새 Grids가 나옴, 누르면 재시작해 업데이트 (설정·폴더·세션 유지)\n\
패널 머리줄: 오른쪽 클릭 = 패널 색 라벨 (세션에 저장), 더블클릭 = 크게 보기 / 복원, 폴더를 끌어다 놓으면 그 패널에서 열기 (파일은 그 파일이 든 폴더)\n\
탭: 클릭 = 전환, × 또는 가운데 클릭 = 닫기, + 또는 탭 줄 빈 곳 더블클릭 = 새 탭, 오른쪽 클릭 = 복제·왼쪽/오른쪽으로 옮기기·닫기, 파일을 다른 탭에 끌어다 놓으면 탐색기처럼 그 탭 폴더로 복사·이동(Ctrl 복사·Shift 이동)\n\
실행 인자: grids.exe C:\\a D:\\b ... 또는 grids.exe 작업.grids\n\
이미 실행 중이면 그 창에서 엽니다.";
