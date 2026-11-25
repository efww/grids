use crate::*;
use std::cell::{Cell, RefCell};
use windows::Win32::UI::Controls::MEASUREITEMSTRUCT;
use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;

// ---------- keyboard shortcuts window (Ctrl+/ or the view menu) ----------

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
pub(crate) fn keys_layout(secs: &[Vec<KeyRow>; 2], width: i32, dpi: i32, stacked: bool, measure: &dyn Fn(&str, i32, bool) -> (i32, i32)) -> KeysGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, gap, cx, cy, cgap) = (s(KEYS_PAD), s(16), s(7), s(3), s(5));
    let inner = (width - 2 * pad).max(s(80));
    let lh = measure("Ag", inner, true).1;
    let cap_w = |t: &str| measure(t, inner, true).0 + 2 * cx;
    let (mut one, mut group) = (0, 0);
    for r in &secs[0] {
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
        if sec > 0 {
            y += s(14);
        }
        titles[sec] = RECT { left: pad, top: y, right: pad + inner, bottom: y + lh };
        y += lh + s(6);
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

pub(crate) unsafe fn keys_geo(width: i32, dpi: i32, stacked: bool) -> KeysGeo {
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
    let g = keys_layout(&key_rows(tr(KEYS_EN, KEYS_KO)), width, dpi, stacked, &measure);
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
    let mut out = Vec::new();
    for (name, list) in names.iter().zip([kb, ms]) {
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

const ID_KEYS_LIST: i32 = 1;

struct KeysList {
    g: KeysGeo,
    rows: Vec<KeyRow>,
    bands: Vec<(i32, i32)>,
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
    let tallest = |g: &KeysGeo| keys_bands(g, dpi).iter().map(|(t, b)| b - t).max().unwrap_or(0);
    let mut g = keys_geo(keys_list_w(w, dpi), dpi, false);
    // on a very high DPI the longest rows outgrow a list item: wider, they wrap into fewer lines
    while w < max_w && tallest(&g) > KEYS_ITEM_MAX {
        w = (w + w / 5).min(max_w);
        g = keys_geo(keys_list_w(w, dpi), dpi, false);
    }
    // still too tall at the widest: put the keys above the description so it wraps across the whole width
    if tallest(&g) > KEYS_ITEM_MAX {
        let st = keys_geo(keys_list_w(w, dpi), dpi, true);
        if tallest(&st) < tallest(&g) {
            g = st;
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
    KEYS_LIST.set(Some(KeysList { g, rows: key_rows(tr(KEYS_EN, KEYS_KO)).into_iter().flatten().collect(), bands }));
    let title = HSTRING::from(tr("Keyboard shortcuts", "단축키"));
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
    _ = ShowWindow(p, SW_SHOW);
    _ = SetFocus(Some(lb));
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
        let (Some(gr), Some(r), Some(&(top, bottom))) = (kl.g.rows.get(i), kl.rows.get(i), kl.bands.get(i)) else { return };
        let dy = rc.top - top;
        let mv = |r: RECT| RECT { top: r.top + dy, bottom: r.bottom + dy, ..r };
        let sel = d.itemState.0 & ODS_SELECTED.0 != 0;
        if sel {
            let hl = RECT { left: rc.left, top: (gr.row.top - s(3)).max(top) + dy, right: rc.right, bottom: (gr.row.bottom + s(3)).min(bottom) + dy };
            fill(dc, hl, pal().pill);
            fill(dc, RECT { right: hl.left + s(2).max(2), ..hl }, accent);
        }
        if i == 0 || kl.g.rows[i - 1].sec != gr.sec {
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
                let band = KEYS_LIST.with_borrow(|kl| kl.as_ref().and_then(|kl| kl.bands.get(mi.itemID as usize).copied()));
                mi.itemHeight = band.map_or(1, |(t, b)| (b - t).clamp(1, KEYS_ITEM_MAX)) as u32;
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
                if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                    close_keys();
                }
            }
            // Esc and Ctrl+/ on the list are taken in key() (keyboard.rs) and close it
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => {
                a11y_name(keys_list(h), None);
                KEYS_LIST.set(None);
                app(|a| a.keys_popup = HWND::default());
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}
