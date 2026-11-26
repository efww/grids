use crate::*;
use std::cell::{Cell, RefCell};
use windows::Win32::UI::Controls::MEASUREITEMSTRUCT;
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;

// ---------- keyboard shortcuts window (Ctrl+/ or the view menu) ----------

#[derive(Clone)]
pub(crate) struct KeyRow {
    pub(crate) keys: Vec<String>, // keycaps; in the mouse section, one plain name
    pub(crate) desc: String,
}

// KEYS_EN / KEYS_KO split into [keyboard, mouse & buttons]. Keys break only at ", " and " / ",
// so "Ctrl+C/X/V" and "Ctrl+/" stay one cap; a mouse line without ": " continues the one above.
pub(crate) fn key_rows(text: &str) -> [Vec<KeyRow>; 2] {
    let (keys, mouse) = text.split_once("\n\n").unwrap_or((text, ""));
    let kb = keys
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, d)| KeyRow { keys: k.split(", ").flat_map(|p| p.split(" / ")).map(String::from).collect(), desc: d.into() })
        .collect();
    let mut ms: Vec<KeyRow> = Vec::new();
    for l in mouse.lines() {
        match (l.split_once(": "), ms.last_mut()) {
            (Some((k, d)), _) => ms.push(KeyRow { keys: vec![k.into()], desc: d.into() }),
            (None, Some(last)) => {
                last.desc.push('\n');
                last.desc.push_str(l);
            }
            (None, None) => {}
        }
    }
    [kb, ms]
}

pub(crate) struct KeyGeoRow {
    pub(crate) sec: usize,
    pub(crate) row: RECT,
    pub(crate) keys: Vec<RECT>, // keycaps, or the text of a mouse-section name
    pub(crate) desc: RECT,      // description text
}

pub(crate) struct KeysGeo {
    pub(crate) h: i32,    // content height (below the fixed title bar)
    pub(crate) titles: [RECT; 2],
    pub(crate) rows: Vec<KeyGeoRow>,
}

pub(crate) const KEYS_PAD: i32 = 20;

// Content layout in px. measure(text, max_w, cap): a cap's one-line size, or text wrapped at max_w.
// Painting and the window size both use this, so what was measured is what gets drawn.
// stacked: keys on their own line(s) above a full-width description, for when side by side a row is too tall.
#[cfg(test)]
pub(crate) fn keys_layout(secs: &[Vec<KeyRow>; 2], width: i32, dpi: i32, stacked: bool, measure: &dyn Fn(&str, i32, bool) -> (i32, i32)) -> KeysGeo {
    keys_layout_in(secs, &secs[0], width, dpi, stacked, measure)
}

// The same, with the key column sized from key_src (the whole keyboard section), so a filtered list keeps its
// columns where they were. A section with no rows takes no title and no gap.
#[cfg(test)]
pub(crate) fn keys_layout_in(secs: &[Vec<KeyRow>; 2], key_src: &[KeyRow], width: i32, dpi: i32, stacked: bool, measure: &dyn Fn(&str, i32, bool) -> (i32, i32)) -> KeysGeo {
    keys_layout_at(secs, key_src, width, dpi, stacked, true, measure)
}

// titled false: no section titles (their rects stay empty), for a found row that with its title outgrows an item.
pub(crate) fn keys_layout_at(secs: &[Vec<KeyRow>; 2], key_src: &[KeyRow], width: i32, dpi: i32, stacked: bool, titled: bool, measure: &dyn Fn(&str, i32, bool) -> (i32, i32)) -> KeysGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, gap, cx, cy, cgap) = (s(KEYS_PAD), s(16), s(7), s(3), s(5));
    let inner = (width - 2 * pad).max(s(80));
    let lh = measure("Ag", inner, true).1;
    let cap_w = |t: &str| measure(t, inner, true).0 + 2 * cx;
    let (mut one, mut group) = (0, 0);
    for r in key_src {
        let caps: Vec<i32> = r.keys.iter().map(|k| cap_w(k)).collect();
        one = one.max(caps.iter().copied().max().unwrap_or(0));
        group = group.max(caps.iter().sum::<i32>() + cgap * (caps.len() as i32 - 1).max(0));
    }
    // widest single cap, or a whole row of caps up to 40%; never over half (a wider cap gets an ellipsis)
    let key_w = if stacked { inner } else { one.max(group.min(inner * 2 / 5)).min(inner / 2) };
    let (dx, dw, ch) = if stacked { (pad, inner, lh + 2 * cy) } else { (pad + key_w + gap, inner - key_w - gap, lh + 2 * cy) };
    let mut y = pad;
    let mut titles = [RECT::default(); 2];
    let mut rows = Vec::new();
    for (sec, list) in secs.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        if !rows.is_empty() {
            y += s(14);
        }
        if titled {
            titles[sec] = RECT { left: pad, top: y, right: pad + inner, bottom: y + lh };
            y += lh + s(6);
        }
        for r in list {
            let mut keys = Vec::new();
            if sec == 0 {
                let (mut x, mut line) = (0, 0);
                for k in &r.keys {
                    let w = cap_w(k).min(key_w);
                    if x > 0 && x + w > key_w {
                        (x, line) = (0, line + 1);
                    }
                    let top = y + line * (ch + cgap);
                    keys.push(RECT { left: pad + x, top, right: pad + x + w, bottom: top + ch });
                    x += w + cgap;
                }
            } else {
                let h = measure(&r.keys[0], key_w, false).1;
                keys.push(RECT { left: pad, top: y + cy, right: pad + key_w, bottom: y + cy + h });
            }
            let dh = measure(&r.desc, dw, false).1;
            let dtop = if stacked { keys.iter().map(|k| k.bottom).max().unwrap_or(y) + cgap } else { y + cy };
            let desc = RECT { left: dx, top: dtop, right: dx + dw, bottom: dtop + dh };
            let bottom = keys.iter().map(|k| k.bottom + if sec == 0 { 0 } else { cy }).chain([desc.bottom + cy]).max().unwrap_or(y);
            rows.push(KeyGeoRow { sec, row: RECT { left: pad, top: y, right: pad + inner, bottom }, keys, desc });
            y = bottom + s(6);
        }
    }
    KeysGeo { h: y - s(6) + pad, titles, rows }
}

pub(crate) unsafe fn keys_geo(secs: &[Vec<KeyRow>; 2], key_src: &[KeyRow], width: i32, dpi: i32, stacked: bool) -> KeysGeo {
    keys_geo_at(secs, key_src, width, dpi, stacked, true)
}

pub(crate) unsafe fn keys_geo_at(secs: &[Vec<KeyRow>; 2], key_src: &[KeyRow], width: i32, dpi: i32, stacked: bool, titled: bool) -> KeysGeo {
    let font = app(|a| a.font);
    let dc = GetDC(None);
    let measure = |t: &str, max_w: i32, cap: bool| {
        if !cap {
            return wrap_text(dc, RECT { right: max_w, ..Default::default() }, t, font, 0, true);
        }
        let old = SelectObject(dc, font.into());
        let mut sz = SIZE::default();
        _ = GetTextExtentPoint32W(dc, &t.encode_utf16().collect::<Vec<_>>(), &mut sz);
        SelectObject(dc, old);
        (sz.cx, sz.cy)
    };
    let g = keys_layout_at(secs, key_src, width, dpi, stacked, titled, &measure);
    ReleaseDC(None, dc);
    g
}

pub(crate) fn keys_head(dpi: i32) -> i32 {
    44 * dpi / 96
}

pub(crate) fn keys_close(width: i32, dpi: i32) -> RECT {
    let s = |v: i32| v * dpi / 96;
    RECT { left: width - s(44), top: s(6), right: width - s(12), bottom: s(38) }
}

// One list item per row, as a screen reader reads it: "keys: description". The first item of a section
// starts with the section's name; a mouse line's continuation joins it with a space. Same rows as key_rows.
pub(crate) fn key_items(text: &str, names: [&str; 2]) -> Vec<String> {
    named_items(key_texts(text), names)
}

// key_items before the section names: [keyboard, mouse & buttons], one string per row.
pub(crate) fn key_texts(text: &str) -> [Vec<String>; 2] {
    let (keys, mouse) = text.split_once("\n\n").unwrap_or((text, ""));
    let kb: Vec<String> = keys.lines().filter_map(|l| l.split_once('\t')).map(|(k, d)| format!("{k}: {d}")).collect();
    let mut ms: Vec<String> = Vec::new();
    for l in mouse.lines() {
        match (l.contains(": "), ms.last_mut()) {
            (true, _) => ms.push(l.into()),
            (false, Some(last)) => {
                last.push(' ');
                last.push_str(l);
            }
            (false, None) => {}
        }
    }
    [kb, ms]
}

// The first item of each section that has any rows starts with the section's name.
pub(crate) fn named_items(secs: [Vec<String>; 2], names: [&str; 2]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, list) in names.iter().zip(secs) {
        for (i, t) in list.into_iter().enumerate() {
            out.push(if i == 0 { format!("{name}. {t}") } else { t });
        }
    }
    out
}

// Each row's slice of the content as a list item (top, bottom): from halfway into the gap above it to halfway
// into the gap below. The first item of a section also holds its title; the first and last reach the padding.
pub(crate) fn keys_bands(g: &KeysGeo, dpi: i32) -> Vec<(i32, i32)> {
    let half = 3 * dpi / 96;
    let n = g.rows.len();
    (0..n).map(|i| (if i == 0 { 0 } else { g.rows[i - 1].row.bottom + half }, if i + 1 == n { g.h } else { g.rows[i].row.bottom + half })).collect()
}

// An owner-drawn list box keeps an item's height in a byte
pub(crate) const KEYS_ITEM_MAX: i32 = 255;

// The find box: every word typed (any case) is somewhere in the row's keys or description, in English or
// Korean, so "zip" finds the compress row on a Korean screen too. An empty box keeps every row.
pub(crate) fn keys_match(query: &str, en: &KeyRow, ko: &KeyRow) -> bool {
    let hay = [en, ko].iter().flat_map(|r| r.keys.iter().chain([&r.desc])).map(|t| t.to_lowercase()).collect::<Vec<_>>().join("\n");
    query.split_whitespace().all(|w| hay.contains(&w.to_lowercase()))
}

// The rows of each section that match, by number. shown and other are the same rows in the two languages
// (shortcut_rows keeps KEYS_EN and KEYS_KO row for row).
pub(crate) fn keys_filter(query: &str, shown: &[Vec<KeyRow>; 2], other: &[Vec<KeyRow>; 2]) -> [Vec<usize>; 2] {
    let sec = |s: usize| (0..shown[s].len()).filter(|&j| keys_match(query, &shown[s][j], other[s].get(j).unwrap_or(&shown[s][j]))).collect();
    [sec(0), sec(1)]
}

// The find box in the title bar, between the title (title_w wide) and ✕: its pill, and the edit inside it a line (lh) high.
pub(crate) fn keys_find_rects(width: i32, dpi: i32, title_w: i32, lh: i32) -> (RECT, RECT) {
    let s = |v: i32| v * dpi / 96;
    let right = keys_close(width, dpi).left - s(8);
    let left = (right - s(280)).max(s(KEYS_PAD) + title_w + s(16)).min(right);
    let field = RECT { left, top: s(8), right, bottom: s(36) };
    let top = (field.top + field.bottom - lh) / 2;
    let edit = RECT { left: (left + s(10)).min(right), top, right: (right - s(8)).max(left), bottom: top + lh };
    (field, edit)
}

const ID_KEYS_LIST: i32 = 1;
const ID_KEYS_FIND: i32 = 2;
const ID_KEYS_LIVE: i32 = 3;

struct KeysList {
    g: KeysGeo,
    rows: Vec<KeyRow>,
    bands: Vec<(i32, i32)>,
    all: [Vec<KeyRow>; 2],    // the shown language's rows, unfiltered
    other: [Vec<KeyRow>; 2],  // the other language's, row for row, so either one finds
    texts: [Vec<String>; 2],  // each row as a screen reader reads it
    width: i32,               // the list's layout width and arrangement, kept to lay out a filtered list again
    stacked: bool,
    field: RECT,              // the find box's pill in the title bar
    none: Option<(String, i32)>, // nothing matches: the one item's words and height
    found: usize,             // the count last read out
}

thread_local! {
    // the open window's layout, measured once: its items are measured and drawn from it
    static KEYS_LIST: RefCell<Option<KeysList>> = const { RefCell::new(None) };
    // set while a wheel over the title bar is being handed to the list
    static KEYS_WHEEL: Cell<bool> = const { Cell::new(false) };
}

// The list's width less its scrollbar, so items measure and draw the same with or without one.
unsafe fn keys_list_w(win_w: i32, dpi: i32) -> i32 {
    win_w - 2 - GetSystemMetricsForDpi(SM_CXVSCROLL, dpi as u32)
}

unsafe fn keys_list(h: HWND) -> HWND {
    GetDlgItem(Some(h), ID_KEYS_LIST).unwrap_or_default()
}

pub(crate) unsafe fn open_keys() {
    let (main, dpi, open, font) = app(|a| (a.main, a.dpi, a.keys_popup, a.font));
    if !open.0.is_null() {
        _ = SetFocus(Some(keys_list(open)));
        return;
    }
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let work = mi.rcWork;
    let max_w = (work.right - work.left) * 9 / 10;
    let mut w = (760 * dpi / 96).min(max_w);
    let all = key_rows(tr(KEYS_EN, KEYS_KO));
    let mut g = keys_geo(&all, &all[0], keys_list_w(w, dpi), dpi, false);
    // on a very high DPI the longest rows outgrow a list item: wider, they wrap into fewer lines
    while w < max_w && keys_tallest(&g, dpi) > KEYS_ITEM_MAX {
        w = (w + w / 5).min(max_w);
        g = keys_geo(&all, &all[0], keys_list_w(w, dpi), dpi, false);
    }
    // still too tall at the widest: put the keys above the description so it wraps across the whole width
    let mut stacked = false;
    if keys_tallest(&g, dpi) > KEYS_ITEM_MAX {
        let st = keys_geo(&all, &all[0], keys_list_w(w, dpi), dpi, true);
        if keys_tallest(&st, dpi) < keys_tallest(&g, dpi) {
            (g, stacked) = (st, true);
        }
    }
    let head = keys_head(dpi);
    let h = (head + g.h + 1).min((work.bottom - work.top) * 9 / 10);
    let mut rc = RECT::default();
    _ = GetWindowRect(main, &mut rc);
    let x = ((rc.left + rc.right - w) / 2).clamp(work.left, work.right - w);
    let y = ((rc.top + rc.bottom - h) / 2).clamp(work.top, work.bottom - h);
    let prev = GetFocus();
    app(|a| a.keys_prev = prev);
    let bands = keys_bands(&g, dpi);
    let title = tr("Keyboard shortcuts", "단축키");
    let (field, edit) = keys_find_rects(w, dpi, ui_text_size(title).cx, ui_text_size("Ag").cy);
    let rows: Vec<KeyRow> = all.iter().flatten().cloned().collect();
    let (other, texts) = (key_rows(tr(KEYS_KO, KEYS_EN)), key_texts(tr(KEYS_EN, KEYS_KO)));
    let found = rows.len();
    let width = keys_list_w(w, dpi);
    KEYS_LIST.set(Some(KeysList { g, rows, bands, all, other, texts, width, stacked, field, none: None, found }));
    let title = HSTRING::from(title);
    let Ok(p) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsKeys"), &title, WS_POPUP | WS_CLIPCHILDREN, x, y, w, h, Some(main), None, None, None) else {
        KEYS_LIST.set(None);
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(p, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.keys_popup = p);
    // a real list: ↑/↓, PgUp/PgDn, Home/End and the wheel move through it a row at a time, and screen readers read each row
    let style = (LBS_OWNERDRAWVARIABLE | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT | LBS_NOTIFY) as u32 | WS_VSCROLL.0;
    let lb = child(p, w!("LISTBOX"), w!(""), style, WINDOW_EX_STYLE(0), ID_KEYS_LIST, font);
    _ = MoveWindow(lb, 1, head, w - 2, h - head - 1, false);
    if DARK_ON.load(Ordering::Relaxed) {
        _ = SetWindowTheme(lb, w!("DarkMode_Explorer"), PCWSTR::null()); // dark scrollbar
    }
    a11y_name(lb, Some(tr("Shortcut list", "단축키 목록")));
    for t in key_items(tr(KEYS_EN, KEYS_KO), [tr("Keyboard", "키보드"), tr("Mouse & buttons", "마우스·버튼")]) {
        let t = HSTRING::from(t);
        SendMessageW(lb, LB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
    }
    SendMessageW(lb, LB_SETCURSEL, Some(WPARAM(0)), None);
    // the find box in the title bar: typing keeps only the rows with every word (keys_refilter on EN_CHANGE)
    let find = child(p, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), ID_KEYS_FIND, font);
    _ = SetWindowSubclass(find, Some(filter_hint_proc), 4, HINT_KEYS);
    _ = MoveWindow(find, edit.left, edit.top, edit.right - edit.left, edit.bottom - edit.top, false);
    a11y_name(find, Some(tr("Find shortcuts", "단축키 찾기")));
    // zero size, a polite live region: how many rows a search left is read out by itself
    let live = child(p, w!("STATIC"), w!(""), 0, WINDOW_EX_STYLE(0), ID_KEYS_LIVE, HFONT::default());
    keys_live(live, true);
    _ = ShowWindow(p, SW_SHOW);
    _ = SetFocus(Some(lb));
}

fn keys_tallest(g: &KeysGeo, dpi: i32) -> i32 {
    keys_bands(g, dpi).iter().map(|(t, b)| b - t).max().unwrap_or(0)
}

unsafe fn ui_text_size(t: &str) -> SIZE {
    let font = app(|a| a.font);
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &t.encode_utf16().collect::<Vec<_>>(), &mut sz);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    sz
}

// The live region's annotation, on when made and off before it goes. Failing only loses the reading.
unsafe fn keys_live(h: HWND, on: bool) {
    if h.0.is_null() {
        return;
    }
    let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) else { return };
    let (obj, me) = (OBJID_CLIENT.0 as u32, CHILDID_SELF);
    _ = if on { acc.SetHwndProp(h, obj, me, LiveSetting_Property_GUID, &VARIANT::from(Polite.0)) } else { acc.ClearHwndProps(h, obj, me, &[LiveSetting_Property_GUID]) };
}

unsafe fn keys_find(h: HWND) -> HWND {
    GetDlgItem(Some(h), ID_KEYS_FIND).unwrap_or_default()
}

// The find box's words changed: lay out only the matching rows (columns as for the whole list, sections with
// none left out) and fill the list with them again. KEYS_LIST changes first and its borrow ends before the list
// is refilled: each LB_ADDSTRING measures its item from it on the spot.
unsafe fn keys_refilter(h: HWND) {
    let find = keys_find(h);
    let mut buf = vec![0u16; GetWindowTextLengthW(find).max(0) as usize + 1];
    let n = GetWindowTextW(find, &mut buf).max(0) as usize;
    let q = String::from_utf16_lossy(&buf[..n]);
    let dpi = app(|a| a.dpi);
    let picked = KEYS_LIST.with_borrow(|kl| {
        let kl = kl.as_ref()?;
        let hit = keys_filter(&q, &kl.all, &kl.other);
        let rows = |s: usize| hit[s].iter().map(|&j| kl.all[s][j].clone()).collect::<Vec<_>>();
        let texts = |s: usize| hit[s].iter().map(|&j| kl.texts[s][j].clone()).collect::<Vec<_>>();
        Some(([rows(0), rows(1)], [texts(0), texts(1)], kl.all[0].clone(), kl.width, kl.stacked))
    });
    let Some((secs, texts, key_src, width, stacked)) = picked else { return };
    let mut g = keys_geo(&secs, &key_src, width, dpi, stacked);
    // a long row first in its section takes the title above it too: if that outgrows an item, stack as when opened
    let mut st_now = stacked;
    if !stacked && keys_tallest(&g, dpi) > KEYS_ITEM_MAX {
        let st = keys_geo(&secs, &key_src, width, dpi, true);
        if keys_tallest(&st, dpi) < keys_tallest(&g, dpi) {
            (g, st_now) = (st, true);
        }
    }
    // still too tall: leave the section titles out while finding (the read-out items keep the section names)
    if keys_tallest(&g, dpi) > KEYS_ITEM_MAX {
        g = keys_geo_at(&secs, &key_src, width, dpi, st_now, false);
    }
    let count = secs[0].len() + secs[1].len();
    let q = q.trim();
    let none = (count == 0).then(|| {
        let t = if KO.load(Ordering::Relaxed) { format!("'{q}'에 맞는 단축키가 없습니다") } else { format!("No shortcuts match '{q}'") };
        (t, ui_text_size("Ag").cy + 2 * (KEYS_PAD * dpi / 96))
    });
    let items = match &none {
        Some((t, _)) => vec![t.clone()],
        None => named_items(texts, [tr("Keyboard", "키보드"), tr("Mouse & buttons", "마우스·버튼")]),
    };
    let lb = keys_list(h);
    SendMessageW(lb, WM_SETREDRAW, Some(WPARAM(0)), None);
    let said = KEYS_LIST.with_borrow_mut(|kl| {
        let kl = kl.as_mut()?;
        kl.bands = keys_bands(&g, dpi);
        kl.rows = secs.into_iter().flatten().collect();
        kl.g = g;
        kl.none = none.clone();
        // read out only when the count changes, not at every keystroke (an IME sends one per syllable too)
        let said = (kl.found != count && !q.is_empty()).then_some(count);
        kl.found = count;
        said
    });
    SendMessageW(lb, LB_RESETCONTENT, None, None);
    for t in items {
        let t = HSTRING::from(t);
        SendMessageW(lb, LB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
    }
    SendMessageW(lb, LB_SETCURSEL, Some(WPARAM(if count > 0 { 0 } else { usize::MAX })), None);
    SendMessageW(lb, WM_SETREDRAW, Some(WPARAM(1)), None);
    // the frame too: a scrollbar that came or went is not left drawn as it was
    _ = RedrawWindow(Some(lb), None, None, RDW_FRAME | RDW_INVALIDATE | RDW_ERASE);
    let live = GetDlgItem(Some(h), ID_KEYS_LIVE).unwrap_or_default();
    if let (Some(n), false) = (said, live.0.is_null()) {
        let t = match &none {
            Some((t, _)) => t.clone(),
            None if KO.load(Ordering::Relaxed) => format!("{n}개 찾음"),
            None => format!("{n} found"),
        };
        _ = SetWindowTextW(live, &HSTRING::from(t));
        NotifyWinEvent(EVENT_OBJECT_LIVEREGIONCHANGED, live, OBJID_CLIENT.0, CHILDID_SELF as i32);
    }
}

// From the find box to the list, on its first row.
unsafe fn keys_to_list(h: HWND) {
    let lb = keys_list(h);
    if KEYS_LIST.with_borrow(|kl| kl.as_ref().is_some_and(|kl| !kl.rows.is_empty())) {
        SendMessageW(lb, LB_SETCURSEL, Some(WPARAM(0)), None);
    }
    _ = SetFocus(Some(lb));
}

// To the find box, its words selected so typing replaces them.
unsafe fn keys_to_find(h: HWND) {
    let find = keys_find(h);
    _ = SetFocus(Some(find));
    SendMessageW(find, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
}

// The shortcut window's keys, from key() (keyboard.rs): what isn't taken here is the list's or the find box's
// (typing, IME included), never a pane shortcut behind it. true: taken.
// Ctrl+/ closes from anywhere. Esc first empties the find box, then closes. The box's Enter / ↓ / Tab go to the list's
// first row; the list's Tab and Ctrl+F (anywhere) go to the box.
pub(crate) unsafe fn keys_key(m: &MSG, keys: HWND, ctrl: bool, shift: bool, alt: bool) -> bool {
    let find = keys_find(keys);
    let in_find = !find.0.is_null() && m.hwnd == find;
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        // auto-repeat (bit 30) is swallowed: holding Ctrl+/ toggles once
        VK_OEM_2 if ctrl && !shift && !alt => {
            if m.lParam.0 & (1 << 30) == 0 {
                close_keys();
            }
        }
        VK_ESCAPE if GetWindowTextLengthW(find) > 0 => {
            app(|a| a.held = Some(VK_ESCAPE.0)); // held down, it doesn't go on to close the window
            _ = SetWindowTextW(find, w!("")); // EN_CHANGE: every row again
        }
        VK_ESCAPE => close_keys(),
        VK_RETURN | VK_DOWN if in_find && !ctrl && !shift && !alt => keys_to_list(keys),
        VK_TAB if !ctrl && !alt => {
            if in_find {
                keys_to_list(keys)
            } else {
                keys_to_find(keys)
            }
        }
        VK_F if ctrl && !shift && !alt => keys_to_find(keys),
        _ => return false,
    }
    true
}

// Esc, ✕ or Ctrl+/ again: back to where the user was. (Losing activation just closes: focus is already elsewhere.)
pub(crate) unsafe fn close_keys() {
    let (h, prev, active) = app(|a| (a.keys_popup, a.keys_prev, a.s.active));
    _ = DestroyWindow(h);
    if !prev.0.is_null() && IsWindow(Some(prev)).as_bool() {
        _ = SetFocus(Some(prev));
    } else {
        focus_view(active);
    }
}

pub(crate) unsafe fn toggle_keys() {
    if app(|a| a.keys_popup.0.is_null()) {
        open_keys();
    } else {
        close_keys();
    }
}

// The fixed title bar; the list below it draws the rows.
pub(crate) unsafe fn paint_keys(h: HWND) {
    let (dpi, font, icon_font) = app(|a| (a.dpi, a.font, a.icon_font));
    let s = |v: i32| v * dpi / 96;
    let head = keys_head(dpi);
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        let bar = RECT { bottom: head, ..rc };
        fill(dc, RECT { top: head - 1, ..bar }, pal().line);
        text(dc, RECT { left: s(KEYS_PAD), ..bar }, tr("Keyboard shortcuts", "단축키"), font, pal().text, DT_LEFT);
        // the find box's field (its borderless edit sits on the same colour)
        if let Some(field) = KEYS_LIST.with_borrow(|kl| kl.as_ref().map(|kl| kl.field)) {
            fill(dc, field, pal().pill);
            frame(dc, field, 1, pal().line);
        }
        text(dc, keys_close(rc.right, dpi), "\u{E711}", icon_font, pal().sub, DT_CENTER);
        frame(dc, rc, 1, pal().line);
    });
}

// One row, laid out as before but moved into its item; the chosen row gets a band and a selection-colour edge.
unsafe fn draw_key_item(d: &DRAWITEMSTRUCT) {
    let (dc, rc, i) = (d.hDC, d.rcItem, d.itemID as usize);
    let (dpi, font, small, accent) = app(|a| (a.dpi, a.font, a.small_font, a.s.accent_now()));
    let s = |v: i32| v * dpi / 96;
    fill(dc, rc, pal().card);
    KEYS_LIST.with_borrow(|kl| {
        let Some(kl) = kl else { return };
        // nothing matches: one muted line in place of the rows, never highlighted
        if let (Some((t, _)), 0) = (&kl.none, i) {
            let pad = s(KEYS_PAD);
            text(dc, RECT { left: rc.left + pad, right: rc.right - pad, ..rc }, t, font, pal().sub, DT_LEFT);
            return;
        }
        let (Some(gr), Some(r), Some(&(top, bottom))) = (kl.g.rows.get(i), kl.rows.get(i), kl.bands.get(i)) else { return };
        let dy = rc.top - top;
        let mv = |r: RECT| RECT { top: r.top + dy, bottom: r.bottom + dy, ..r };
        let sel = d.itemState.0 & ODS_SELECTED.0 != 0;
        if sel {
            let hl = RECT { left: rc.left, top: (gr.row.top - s(3)).max(top) + dy, right: rc.right, bottom: (gr.row.bottom + s(3)).min(bottom) + dy };
            fill(dc, hl, pal().pill);
            fill(dc, RECT { right: hl.left + s(2).max(2), ..hl }, accent);
        }
        let title_rc = kl.g.titles[gr.sec]; // empty when laid out without titles
        if (i == 0 || kl.g.rows[i - 1].sec != gr.sec) && title_rc.bottom > title_rc.top {
            let names = [tr("Keyboard", "키보드"), tr("Mouse & buttons", "마우스·버튼")];
            text(dc, mv(kl.g.titles[gr.sec]), names[gr.sec], small, pal().sub, DT_LEFT);
        }
        for (k, kr) in r.keys.iter().zip(&gr.keys) {
            let kr = mv(*kr);
            if gr.sec == 0 {
                fill(dc, kr, if sel { pal().card } else { pal().pill }); // caps stay caps on the band
                frame(dc, kr, 1, pal().line);
                text(dc, RECT { left: kr.left + s(4), right: kr.right - s(4), ..kr }, k, font, pal().text, DT_CENTER);
            } else {
                wrap_text(dc, kr, k, font, pal().text, false);
            }
        }
        wrap_text(dc, mv(gr.desc), &r.desc, font, pal().text, false);
    });
}

pub(crate) extern "system" fn keys_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => paint_keys(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_SETFOCUS => _ = SetFocus(Some(keys_list(h))), // a click on the title bar leaves the keys with the list
            // over the title bar the list scrolls too; what the list passes up itself (Ctrl / Shift + wheel) stops here,
            // including the wheel this forwarded: the list sends that back up within the same call, so it isn't sent again
            WM_MOUSEWHEEL => {
                let (x, y) = xy(lp);
                let mut pt = POINT { x, y };
                _ = ScreenToClient(h, &mut pt);
                if pt.y < keys_head(app(|a| a.dpi)) && !KEYS_WHEEL.replace(true) {
                    SendMessageW(keys_list(h), msg, Some(wp), Some(lp));
                    KEYS_WHEEL.set(false);
                }
            }
            WM_MEASUREITEM => {
                let mi = &mut *(lp.0 as *mut MEASUREITEMSTRUCT);
                let i = mi.itemID as usize;
                let h = KEYS_LIST.with_borrow(|kl| kl.as_ref().and_then(|kl| kl.bands.get(i).map(|(t, b)| b - t).or(kl.none.as_ref().map(|n| n.1))));
                mi.itemHeight = h.map_or(1, |h| h.clamp(1, KEYS_ITEM_MAX)) as u32;
                return LRESULT(1);
            }
            WM_DRAWITEM => {
                draw_key_item(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_CTLCOLORLISTBOX => {
                let dc = HDC(wp.0 as _);
                SetDCBrushColor(dc, cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                let r = keys_close(rc.right, app(|a| a.dpi));
                let field = KEYS_LIST.with_borrow(|kl| kl.as_ref().map(|kl| kl.field)).unwrap_or_default();
                let inside = |r: RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
                if inside(r) {
                    close_keys();
                } else if inside(field) {
                    keys_to_find(h); // the field's edge around the edit
                }
            }
            WM_COMMAND if (wp.0 & 0xffff) as i32 == ID_KEYS_FIND && (wp.0 >> 16) as u32 == EN_CHANGE => keys_refilter(h),
            WM_CTLCOLOREDIT => return edit_colors(wp),
            // Esc, Ctrl+/, Tab, Ctrl+F ... are taken in keys_key (from key() in keyboard.rs)
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => {
                a11y_name(keys_list(h), None);
                a11y_name(keys_find(h), None);
                keys_live(GetDlgItem(Some(h), ID_KEYS_LIVE).unwrap_or_default(), false);
                KEYS_LIST.set(None);
                app(|a| a.keys_popup = HWND::default());
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}
