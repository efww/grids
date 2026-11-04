// Shortcut list. Rows come from the keymap, so a rebind shows up here without a restart.
// Ctrl+/ toggles it; holding the keys does that once. Esc, the close mark, or Ctrl+/ again
// put focus back. A click outside just closes, and focus stays where that click put it.

use crate::imports::*;
use crate::app::*;
use crate::i18n::*;
use crate::pane::focus_view;
use super::*;

pub(crate) struct KeyRow {
    pub(crate) keys: Vec<String>,
    pub(crate) desc: String,
}

/// `text` is the keymap help: chord lines, a blank line, then "name: what it does".
/// Caps break on ", " and " / ", so "Ctrl+C/X/V" and "Ctrl+/" stay one cap.
/// A mouse line without ": " continues the one above.
pub(crate) fn key_rows(text: &str) -> [Vec<KeyRow>; 2] {
    let (keys, mouse) = text.split_once("\n\n").unwrap_or((text, ""));
    let kb = keys
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .map(|(k, d)| KeyRow { keys: k.split(", ").flat_map(|p| p.split(" / ")).map(String::from).collect(), desc: d.into() })
        .collect();
    let mut ms: Vec<KeyRow> = Vec::new();
    for l in mouse.lines().filter(|l| !l.is_empty()) {
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
    pub(crate) keys: Vec<RECT>,
    pub(crate) desc: RECT,
}

pub(crate) struct KeysGeo {
    pub(crate) h: i32,
    pub(crate) titles: [RECT; 2],
    pub(crate) rows: Vec<KeyGeoRow>,
}

pub(crate) const KEYS_PAD: i32 = 20;

/// Content layout in px. `measure(text, max_w, cap)` is a cap's one-line size, or text wrapped at `max_w`.
pub(crate) fn keys_layout(secs: &[Vec<KeyRow>; 2], width: i32, dpi: i32, measure: &dyn Fn(&str, i32, bool) -> (i32, i32)) -> KeysGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, gap, cx, cy, cgap) = (s(KEYS_PAD), s(16), s(7), s(3), s(5));
    let inner = (width - 2 * pad).max(s(80));
    let lh = measure("Ag", inner, true).1.max(1);
    let cap_w = |t: &str| measure(t, inner, true).0 + 2 * cx;
    let (mut one, mut group) = (0, 0);
    for r in &secs[0] {
        let caps: Vec<i32> = r.keys.iter().map(|k| cap_w(k)).collect();
        one = one.max(caps.iter().copied().max().unwrap_or(0));
        group = group.max(caps.iter().sum::<i32>() + cgap * (caps.len() as i32 - 1).max(0));
    }
    let key_w = one.max(group.min(inner * 2 / 5)).min(inner / 2).max(s(40));
    let (dx, dw, ch) = (pad + key_w + gap, (inner - key_w - gap).max(s(40)), lh + 2 * cy);
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
            } else if let Some(name) = r.keys.first() {
                let h = measure(name, key_w, false).1.max(1);
                keys.push(RECT { left: pad, top: y + cy, right: pad + key_w, bottom: y + cy + h });
            }
            let dh = measure(&r.desc, dw, false).1.max(1);
            let desc = RECT { left: dx, top: y + cy, right: dx + dw, bottom: y + cy + dh };
            let bottom = keys.iter().map(|k| k.bottom + if sec == 0 { 0 } else { cy }).chain([desc.bottom + cy]).max().unwrap_or(y);
            rows.push(KeyGeoRow { sec, row: RECT { left: pad, top: y, right: pad + inner, bottom }, keys, desc });
            y = bottom + s(6);
        }
    }
    KeysGeo { h: y - s(6) + pad, titles, rows }
}

pub(crate) unsafe fn keys_geo(width: i32, dpi: i32) -> KeysGeo {
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
    let g = keys_layout(&key_rows(&crate::keys::help_text()), width, dpi, &measure);
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

/// Shortcut list stays within this percent of the owner client, not the monitor.
const KEYS_OWNER_PCT: i32 = 85;

/// Popup size in px. `content_h(width)` is the header plus the list at that width.
/// The 760-dip design width and the content height win when they sit under the cap.
pub(crate) fn keys_fit_size(owner_w: i32, owner_h: i32, dpi: i32, content_h: impl FnOnce(i32) -> i32) -> (i32, i32) {
    let px = |dip: i32| dip * dpi / 96;
    let cap = |span: i32| span.max(0) * KEYS_OWNER_PCT / 100;
    let w = px(760).min(cap(owner_w)).max(px(320));
    let h = content_h(w).min(cap(owner_h)).max(px(200));
    (w, h)
}

/// Top-left that centers `w`×`h` on `owner`. A real work rect then keeps it on that monitor.
pub(crate) fn keys_fit_origin(owner: RECT, w: i32, h: i32, work: RECT) -> (i32, i32) {
    let x = (owner.left + owner.right - w) / 2;
    let y = (owner.top + owner.bottom - h) / 2;
    if work.right <= work.left || work.bottom <= work.top {
        return (x, y);
    }
    (x.clamp(work.left, (work.right - w).max(work.left)), y.clamp(work.top, (work.bottom - h).max(work.top)))
}

pub(crate) unsafe fn open_keys() {
    let (main, dpi, open) = app(|a| (a.main, a.dpi, a.keys_popup));
    if !open.0.is_null() {
        _ = SetFocus(Some(open));
        return;
    }
    let mut client = RECT::default();
    _ = GetClientRect(main, &mut client);
    let mut tl = POINT { x: client.left, y: client.top };
    let mut br = POINT { x: client.right, y: client.bottom };
    _ = ClientToScreen(main, &mut tl);
    _ = ClientToScreen(main, &mut br);
    let mut frame = RECT::default();
    _ = GetWindowRect(main, &mut frame);
    let (w, h) = keys_fit_size(br.x - tl.x, br.y - tl.y, dpi, |w| keys_head(dpi) + keys_geo(w, dpi).h);
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let work = if GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() { mi.rcWork } else { RECT::default() };
    let (x, y) = keys_fit_origin(frame, w, h, work);
    let prev = GetFocus();
    app(|a| {
        a.keys_prev = prev;
        a.keys_scroll = 0;
        a.keys_btn = Tracker::new();
    });
    let title = HSTRING::from(tr(crate::Key::KeyboardShortcuts2));
    let Ok(p) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("MaguroKeys"), &title, WS_POPUP | WS_VISIBLE, x, y, w, h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(p, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.keys_popup = p);
    _ = SetFocus(Some(p));
}

/// Esc, the close mark, or Ctrl+/ again. Focus goes back. Losing activation does not call this.
pub(crate) unsafe fn close_keys() {
    let (h, prev, active) = app(|a| (a.keys_popup, a.keys_prev, a.s.active));
    if h.0.is_null() {
        return;
    }
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

pub(crate) unsafe fn keys_scroll_to(h: HWND, to: impl FnOnce(i32, i32) -> i32) {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let dpi = app(|a| a.dpi);
    let page = rc.bottom - keys_head(dpi);
    let max = (keys_geo(rc.right, dpi).h - page).max(0);
    if app(|a| {
        let y = to(a.keys_scroll, page).clamp(0, max);
        std::mem::replace(&mut a.keys_scroll, y) != y
    }) {
        _ = InvalidateRect(Some(h), None, false);
    }
}

const KEYS_CLOSE_ID: u8 = 1;

fn keys_over(h: HWND, x: i32, y: i32, dpi: i32) -> Option<u8> {
    unsafe {
        let mut rc = RECT::default();
        let _ = GetClientRect(h, &mut rc);
        let r = keys_close(rc.right, dpi);
        (x >= r.left && x < r.right && y >= r.top && y < r.bottom).then_some(KEYS_CLOSE_ID)
    }
}

fn arm_leave(h: HWND) {
    unsafe {
        let mut t = TRACKMOUSEEVENT {
            cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: h,
            dwHoverTime: 0,
        };
        let _ = TrackMouseEvent(&mut t);
    }
}

pub(crate) unsafe fn paint_keys(h: HWND) {
    let (dpi, font, small, icon_font, scroll, close_vis) = app(|a| (a.dpi, a.font, a.small_font, a.icon_font, a.keys_scroll, a.keys_btn.visual(KEYS_CLOSE_ID, false)));
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let rows = key_rows(&crate::keys::help_text());
    let g = keys_layout(&rows, rc.right, dpi, &measure_font(font));
    let s = |v: i32| v * dpi / 96;
    let head = keys_head(dpi);
    let down = |r: RECT| RECT { top: r.top + head - scroll, bottom: r.bottom + head - scroll, ..r };
    let ground = ui_tokens().bg;
    buffered(h, |dc, rc| {
        paint_window(dc, rc);
        let names = [tr(crate::Key::Keyboard), tr(crate::Key::MouseAndButtons)];
        for (t, r) in names.iter().zip(g.titles) {
            text(dc, down(r), t, small, pal().sub, DT_LEFT);
        }
        for (gr, r) in g.rows.iter().zip(rows.iter().flatten()) {
            let row = down(gr.row);
            if row.bottom < head || row.top > rc.bottom {
                continue;
            }
            for (k, kr) in r.keys.iter().zip(&gr.keys) {
                let kr = down(*kr);
                if gr.sec == 0 {
                    let t = ui_tokens();
                    aa_rfill(dc, kr, px(dpi, RADIUS).max(1), t.card, Some(t.line), t.bg);
                    text(dc, RECT { left: kr.left + s(4), right: kr.right - s(4), ..kr }, k, font, pal().text, DT_CENTER);
                } else {
                    wrap_text(dc, kr, k, font, pal().text, false);
                }
            }
            wrap_text(dc, down(gr.desc), &r.desc, font, pal().text, false);
        }
        let bar = RECT { bottom: head, ..rc };
        fill(dc, bar, ground);
        fill(dc, RECT { top: head - 1, ..bar }, ui_tokens().line);
        text(dc, RECT { left: s(KEYS_PAD), ..bar }, tr(crate::Key::KeyboardShortcuts2), font, pal().text, DT_LEFT);
        paint_button(dc, keys_close(rc.right, dpi), ButtonKind::Icon, close_vis, "\u{E711}", icon_font, dpi, ground, FocusRing::Outside);
        let page = rc.bottom - head;
        if g.h > page && g.h > 0 {
            let th = (page * page / g.h).max(s(24));
            let travel = (page - th).max(1);
            let top = head + scroll * travel / (g.h - page).max(1);
            fill(dc, RECT { left: rc.right - s(6), top, right: rc.right - s(3), bottom: top + th }, pal().sub);
        }
        frame(dc, rc, 1, ui_tokens().line);
    });
}

fn measure_font(font: HFONT) -> impl Fn(&str, i32, bool) -> (i32, i32) {
    move |t: &str, max_w: i32, cap: bool| unsafe {
        let dc = GetDC(None);
        let size = if cap {
            let old = SelectObject(dc, font.into());
            let mut sz = SIZE::default();
            _ = GetTextExtentPoint32W(dc, &t.encode_utf16().collect::<Vec<_>>(), &mut sz);
            SelectObject(dc, old);
            (sz.cx, sz.cy)
        } else {
            wrap_text(dc, RECT { right: max_w, ..Default::default() }, t, font, 0, true)
        };
        ReleaseDC(None, dc);
        size
    }
}

pub(crate) extern "system" fn keys_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| keys_proc_inner(h, msg, wp, lp)))
}
fn keys_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        // With two windows open, act on the window this popup belongs to.
        bind_hwnd(h);
        let dpi = app(|a| a.dpi);
        let s = |v: i32| v * dpi / 96;
        match msg {
            WM_PAINT => paint_keys(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEWHEEL => keys_scroll_to(h, |y, _| y - s((wp.0 >> 16) as i16 as i32)),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let over = keys_over(h, x, y, dpi);
                let effect = app(|a| a.keys_btn.pointer_move(over));
                arm_leave(h);
                if matches!(effect, TrackEffect::Repaint) {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_MOUSELEAVE => {
                let effect = app(|a| a.keys_btn.pointer_leave());
                if matches!(effect, TrackEffect::Repaint) {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if keys_over(h, x, y, dpi).is_some() {
                    app(|a| { a.keys_btn.pointer_down(KEYS_CLOSE_ID); });
                    _ = SetCapture(h);
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let over = keys_over(h, x, y, dpi);
                let effect = app(|a| a.keys_btn.pointer_up(over));
                if GetCapture() == h {
                    _ = ReleaseCapture();
                }
                if matches!(effect, TrackEffect::Click(_)) {
                    close_keys();
                }
            }
            WM_CAPTURECHANGED => {
                let effect = app(|a| a.keys_btn.capture_lost());
                if matches!(effect, TrackEffect::Cancel) {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_KEYDOWN => match VIRTUAL_KEY(wp.0 as u16) {
                VK_ESCAPE => close_keys(),
                VK_UP => keys_scroll_to(h, |y, _| y - s(40)),
                VK_DOWN => keys_scroll_to(h, |y, _| y + s(40)),
                VK_PRIOR => keys_scroll_to(h, |y, p| y - p * 9 / 10),
                VK_NEXT => keys_scroll_to(h, |y, p| y + p * 9 / 10),
                VK_HOME => keys_scroll_to(h, |_, _| 0),
                VK_END => keys_scroll_to(h, |_, _| i32::MAX),
                _ => {}
            },
            // A click in another window already moved focus. Close without pulling it back.
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = DestroyWindow(h),
            WM_DESTROY => app(|a| {
                a.keys_popup = HWND::default();
                a.keys_btn = Tracker::new();
            }),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        "\
Ctrl+L, Alt+D / F4\tAddress bar\n\
F2, Del, Ctrl+C/X/V, F5\tSame as Explorer\n\
Ctrl+/\tShortcut list\n\
\n\
Layout button: click for an even grid\n\
Double-click a splitter: split evenly\n\
already running, they open here.\n"
            .into()
    }

    #[test]
    fn shortcut_rows_keep_chords_together() {
        let rows = key_rows(&sample());
        assert_eq!(rows[0][0].keys, ["Ctrl+L", "Alt+D", "F4"]);
        assert_eq!(rows[0][1].keys, ["F2", "Del", "Ctrl+C/X/V", "F5"]);
        assert_eq!(rows[0][2].keys, ["Ctrl+/"]);
        assert_eq!(rows[1].len(), 2);
        assert!(rows[1][1].desc.contains("already running"));
        let live = key_rows(&crate::keys::help_text());
        assert!(live[0].len() >= 10 && live[1].len() >= 4);
        assert!(live.iter().flatten().all(|r| !r.desc.is_empty() && r.keys.iter().all(|k| !k.is_empty())));
        assert!(live[0].iter().any(|r| r.keys.iter().any(|k| k == "Ctrl+/")));
    }

    #[test]
    fn shortcut_layout_does_not_overlap() {
        let rows = key_rows(&sample());
        for dpi in [96, 192] {
            for dip in [300, 760] {
                let (cw, lh) = (7 * dpi / 96, 16 * dpi / 96);
                let measure = |t: &str, max_w: i32, cap: bool| {
                    let w = t.chars().count() as i32 * cw;
                    if cap {
                        (w, lh)
                    } else {
                        let lines = t.split('\n').map(|l| ((l.chars().count() as i32 * cw + max_w - 1) / max_w.max(1)).max(1)).sum::<i32>();
                        (w.min(max_w), lines * lh)
                    }
                };
                let g = keys_layout(&rows, dip * dpi / 96, dpi, &measure);
                let (pad, gap) = (KEYS_PAD * dpi / 96, 16 * dpi / 96);
                let mut prev = 0;
                for r in &g.rows {
                    assert!(r.row.top >= prev);
                    for k in &r.keys {
                        assert!(k.left >= pad && k.right + gap <= r.desc.left && k.bottom <= r.row.bottom);
                    }
                    assert!(r.desc.bottom <= r.row.bottom && r.desc.right <= dip * dpi / 96 - pad);
                    prev = r.row.bottom;
                }
                assert!(prev <= g.h && g.titles[1].top > g.titles[0].bottom);
            }
        }
    }

    #[test]
    fn shortcut_list_fits_owner_client() {
        let pct = |v: i32| v * KEYS_OWNER_PCT / 100;
        // Taller than the owner: height stops at the cap, so the body still scrolls.
        let (w, h) = keys_fit_size(1200, 780, 96, |_| 1254);
        assert_eq!((w, h), (760, pct(780)));
        assert!(h < 1254);
        // Content under the cap stays at content size. Width stays at the 760 dip design.
        assert_eq!(keys_fit_size(2000, 1600, 96, |_| 400), (760, 400));
        // A narrow owner pulls both sides down to the cap, above the floor.
        assert_eq!(keys_fit_size(400, 300, 96, |_| 900), (pct(400), pct(300)));
        // Floor is 320×200 dip, scaled with dpi.
        assert_eq!(keys_fit_size(100, 80, 96, |_| 50), (320, 200));
        assert_eq!(keys_fit_size(100, 80, 192, |_| 50), (640, 400));
        let owner = RECT { left: 100, top: 80, right: 1300, bottom: 860 };
        assert_eq!(keys_fit_origin(owner, 760, 663, RECT::default()), (320, 138));
        let work = RECT { left: 0, top: 0, right: 800, bottom: 600 };
        assert_eq!(keys_fit_origin(owner, 760, 663, work), (40, 0));
    }
}
