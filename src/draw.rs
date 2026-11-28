use crate::*;
use windows::Win32::UI::Controls::ODS_FOCUS;

// ---------- drawing (plain GDI fills; the few rounded shapes smooth their corner pixels themselves) ----------

pub(crate) fn cref(rgb: u32) -> COLORREF {
    COLORREF((rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16 & 0xFF))
}

pub(crate) unsafe fn fill(dc: HDC, r: RECT, rgb: u32) {
    SetDCBrushColor(dc, cref(rgb));
    FillRect(dc, &r, HBRUSH(GetStockObject(DC_BRUSH).0));
}

pub(crate) unsafe fn frame(dc: HDC, r: RECT, w: i32, rgb: u32) {
    let sides = [RECT { bottom: r.top + w, ..r }, RECT { top: r.bottom - w, ..r }, RECT { right: r.left + w, ..r }, RECT { left: r.right - w, ..r }];
    for side in sides {
        fill(dc, side, rgb);
    }
}

// Windows 11 (the icon font came out Segoe Fluent Icons, apply_font) rounds the fields, chips and hover shades
// as Explorer and Settings do, so they match its icons; Windows 10 and high contrast keep them square, pixel for
// pixel as before. High contrast comes and goes without apply_font, so it is asked at every paint.
pub(crate) static FLUENT_ON: AtomicBool = AtomicBool::new(false);
pub(crate) fn rounded() -> bool {
    FLUENT_ON.load(Ordering::Relaxed) && !HC_ON.load(Ordering::Relaxed)
}

/// Fraction 0..1 of pixel (x, y) inside the rectangle (l, t, w, h) with corners rounded by r (cut to fit): 4 × 4 samples.
pub(crate) fn round_cover(x: i32, y: i32, (l, t, w, h): (f32, f32, f32, f32), r: f32) -> f32 {
    const N: i32 = 4;
    if w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut n = 0;
    for sy in 0..N {
        for sx in 0..N {
            let (u, v) = (x as f32 + (sx as f32 + 0.5) / N as f32, y as f32 + (sy as f32 + 0.5) / N as f32);
            if u < l || v < t || u > l + w || v > t + h {
                continue;
            }
            let (cu, cv) = (u.clamp(l + r, l + w - r), v.clamp(t + r, t + h - r));
            if (u - cu).powi(2) + (v - cv).powi(2) <= r * r {
                n += 1;
            }
        }
    }
    n as f32 / (N * N) as f32
}

/// A w × h shape with corners rounded by `radius`, cut to half its shorter side: how far in from each corner its
/// plain cross starts (0: a plain rectangle), and the corner pixels it covers as (x, y, cover) from its top-left,
/// cover in (0, 1]. Pixels it doesn't touch are left out, so they keep what is under them.
pub(crate) fn round_corners(w: i32, h: i32, radius: i32) -> (i32, Vec<(i32, i32, f32)>) {
    let k = radius.min(w / 2).min(h / 2);
    if k <= 0 {
        return (0, Vec::new());
    }
    let shape = (0.0, 0.0, w as f32, h as f32);
    let mut out = Vec::with_capacity((4 * k * k) as usize);
    for j in 0..k {
        for i in 0..k {
            let c = round_cover(i, j, shape, k as f32);
            if c > 0.0 {
                out.extend([(i, j), (w - 1 - i, j), (i, h - 1 - j), (w - 1 - i, h - 1 - j)].map(|(x, y)| (x, y, c)));
            }
        }
    }
    (k, out)
}

/// A `width` px ring round a w × h shape with corners rounded by `radius` (inside it, radius − width): how far in
/// its straight sides start (0: square, frame's), and the corner pixels it touches as (x, y, ring, inside), each
/// the share of the pixel the ring and the space within it cover.
pub(crate) fn ring_corners(w: i32, h: i32, width: i32, radius: i32) -> (i32, Vec<(i32, i32, f32, f32)>) {
    let k = radius.min(w / 2).min(h / 2);
    if k <= 0 || width <= 0 {
        return (0, Vec::new());
    }
    let (outer, inner) = ((0.0, 0.0, w as f32, h as f32), (width as f32, width as f32, (w - 2 * width) as f32, (h - 2 * width) as f32));
    let mut out = Vec::with_capacity((4 * k * k) as usize);
    for j in 0..k {
        for i in 0..k {
            let (o, n) = (round_cover(i, j, outer, k as f32), round_cover(i, j, inner, (k - width).max(0) as f32));
            if o > n {
                out.extend([(i, j), (w - 1 - i, j), (i, h - 1 - j), (w - 1 - i, h - 1 - j)].map(|(x, y)| (x, y, o - n, n)));
            }
        }
    }
    (k, out)
}

// Colours mixed by share (shares adding up to 1).
fn mix(parts: [(u32, f32); 3]) -> u32 {
    let ch = |sh: u32| (parts.iter().map(|&(c, t)| (c >> sh & 0xFF) as f32 * t).sum::<f32>().round().clamp(0.0, 255.0) as u32) << sh;
    ch(16) | ch(8) | ch(0)
}

/// fill() with corners rounded by `radius` (Windows 11, rounded()), their edge pixels smoothed toward `under`, the
/// colour the shape lies on; elsewhere, or with no room to round, fill() itself. An empty rectangle draws nothing.
pub(crate) unsafe fn round_fill(dc: HDC, r: RECT, radius: i32, rgb: u32, under: u32) {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 {
        return;
    }
    let (k, px) = if rounded() { round_corners(w, h, radius) } else { (0, Vec::new()) };
    if k == 0 {
        return fill(dc, r, rgb);
    }
    fill(dc, RECT { left: r.left + k, right: r.right - k, ..r }, rgb);
    fill(dc, RECT { top: r.top + k, right: r.left + k, bottom: r.bottom - k, ..r }, rgb);
    fill(dc, RECT { left: r.right - k, top: r.top + k, bottom: r.bottom - k, ..r }, rgb);
    for (x, y, c) in px {
        _ = SetPixelV(dc, r.left + x, r.top + y, cref(blend(under, rgb, c)));
    }
}

/// frame() with corners rounded by `radius` where round_fill rounds: only the ring is drawn, its corner pixels
/// mixed with `outside` (what lies round it) and `inside` (what lies within it). Elsewhere frame() itself.
pub(crate) unsafe fn round_ring(dc: HDC, r: RECT, width: i32, radius: i32, ring: u32, outside: u32, inside: u32) {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 {
        return;
    }
    let (k, px) = if rounded() { ring_corners(w, h, width, radius) } else { (0, Vec::new()) };
    if k == 0 {
        return frame(dc, r, width, ring);
    }
    let (l, t) = (r.left + k, r.top + k);
    let (rr, b) = (r.right - k, r.bottom - k);
    for side in [
        RECT { left: l, right: rr, bottom: r.top + width, ..r },
        RECT { left: l, right: rr, top: r.bottom - width, ..r },
        RECT { top: t, bottom: b, right: r.left + width, ..r },
        RECT { top: t, bottom: b, left: r.right - width, ..r },
    ] {
        fill(dc, side, ring);
    }
    for (x, y, on, within) in px {
        _ = SetPixelV(dc, r.left + x, r.top + y, cref(mix([(outside, 1.0 - on - within), (ring, on), (inside, within)])));
    }
}

pub(crate) unsafe fn text(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, align: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return; // an empty Vec's dangling pointer crashes DrawTextW even with length 0
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(rgb));
    DrawTextW(dc, &mut w, &mut r, align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
    SelectObject(dc, old);
}

// Multi-line text wrapped at r's width (text() is single-line). calc only measures, with the same font and flags.
pub(crate) unsafe fn wrap_text(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, calc: bool) -> (i32, i32) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return (0, 0); // see text()
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(rgb));
    DrawTextW(dc, &mut w, &mut r, DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX | if calc { DT_CALCRECT } else { DRAW_TEXT_FORMAT(0) });
    SelectObject(dc, old);
    (r.right - r.left, r.bottom - r.top)
}

// Paints only the invalid area into an off-screen buffer and blits once (no flicker mid-drag).
pub(crate) unsafe fn buffered(h: HWND, draw: impl FnOnce(HDC, RECT)) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(h, &mut ps);
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let mut mem = HDC::default();
    let pb = BeginBufferedPaint(hdc, &ps.rcPaint, BPBF_TOPDOWNDIB, None, &mut mem);
    if pb == 0 {
        draw(hdc, rc);
    } else {
        draw(mem, rc);
        _ = EndBufferedPaint(pb, true);
    }
    _ = EndPaint(h, &ps);
}

// Pane header colours (background, text/icons): tinted with the selection colour while active.
// The address and filter fields keep their own colours either way.
pub(crate) fn header_colors(pane: HWND) -> (u32, u32) {
    app(|a| {
        let active = a.panes.get(a.s.active).and_then(|p| p.as_ref()).is_some_and(|p| p.hwnd == pane);
        if active || a.drop_hover == pane {
            let c = focus_accent(a, pane);
            (c, on_color(c))
        } else {
            (pal().card, pal().text)
        }
    })
}

// The active pane's focus colour (header, border, current tab edge): quieted while another app has the
// keyboard. High contrast keeps Windows' highlight (on_color knows only that one), and a pane something is
// being dragged onto from that other app keeps the full colour.
pub(crate) fn focus_accent(a: &App, pane: HWND) -> u32 {
    let c = a.s.accent_now();
    if !a.app_away || hc_colors().is_some() || a.drop_hover == pane {
        c
    } else {
        quiet_accent(c, pal().card)
    }
}

// A pane's ← / → / ↑ with nowhere to go: its glyph halfway from the header to its usual colour (faint_nav_reads
// checks it still reads); high contrast's grey text instead, except on the highlight (the active header).
pub(crate) const FAINT: f32 = 0.5;
pub(crate) fn faint_ink(bg: u32, fg: u32) -> u32 {
    match hc_colors() {
        Some(c) if bg != c.hl => c.gray,
        _ => blend(bg, fg, FAINT),
    }
}

// How far a hot/pressed button's fill moves toward its ink; the palette_contrast test reads it too.
pub(crate) fn press_tint(tinted: bool, pressed: bool) -> f32 {
    match (tinted, pressed) {
        (true, true) => 0.3,
        (true, false) => 0.18,
        (false, true) => 0.13,
        (false, false) => 0.07,
    }
}

pub(crate) unsafe fn draw_button(d: &DRAWITEMSTRUCT) {
    draw_button_face(d);
    // the button the getting started tour points at, or the toolbar button the keys are on (F6): a ring in the
    // selection colour (high contrast's highlight; the header ink on a tinted header), drawn once if both
    let (main, kbd) = app(|a| (a.main, a.tb_kbd));
    let parent = GetParent(d.hwndItem).unwrap_or_default();
    let keyed = kbd && parent == main && d.itemState.0 & ODS_FOCUS.0 != 0;
    if tour_target() == d.hwndItem || keyed {
        let (accent, dpi) = app(|a| (a.s.accent_now(), a.dpi));
        let ring = if parent == main { accent } else { header_colors(parent).1 };
        // rounded round the hover shade (s(2) in, s(4) round), so it hugs it; the blue Update button stays square
        let f = face_state(d);
        let inside = if f.hot || f.pressed { blend(f.bg, f.ink, press_tint(f.tinted, f.pressed)) } else { f.bg };
        let radius = if d.CtlID as i32 == ID_TB_UPDATE { 0 } else { 6 * dpi / 96 };
        round_ring(d.hDC, d.rcItem, (2 * dpi / 96).max(2), radius, ring, f.bg, inside);
    }
}

// What an owner-drawn button looks like now: its background and ink, and whether it is tinted, hot, pressed, faint.
struct Face {
    bg: u32,
    ink: u32,
    tinted: bool,
    hot: bool,
    pressed: bool,
    faint: bool,
}

unsafe fn face_state(d: &DRAWITEMSTRUCT) -> Face {
    let (hover, main) = app(|a| (a.hover, a.main));
    let parent = GetParent(d.hwndItem).unwrap_or_default();
    let on_main = parent == main;
    // files dragged over a drive / Recycle Bin button press it, as a click would
    let pressed = d.itemState.0 & ODS_SELECTED.0 != 0 || DROP_BTN.load(Ordering::Relaxed) == d.hwndItem.0 as usize;
    // a pane's ← / → / ↑ with nowhere to go (nav.rs): faint, and no hover shade; pressed still shades, as it may be wrong
    let faint = !on_main && nav_dim_of(parent, d.CtlID as i32);
    let hot = (hover == d.hwndItem && !faint) || tour_target() == d.hwndItem;
    let (bg, ink) = if on_main { (pal().bg, pal().text) } else { header_colors(parent) };
    let tinted = bg != pal().bg && bg != pal().card;
    Face { bg, ink, tinted, hot, pressed, faint }
}

// A hot or pressed button's shade: on Windows 11 a rounded tile s(2) inside the button, as Explorer's command bar;
// elsewhere the whole button, as before.
unsafe fn hot_shade(dc: HDC, rc: RECT, bg: u32, shade: u32, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    if rounded() {
        round_fill(dc, RECT { left: rc.left + s(2), top: rc.top + s(2), right: rc.right - s(2), bottom: rc.bottom - s(2) }, s(4), shade, bg);
    } else {
        fill(dc, rc, shade);
    }
}

unsafe fn draw_button_face(d: &DRAWITEMSTRUCT) {
    let (dpi, font, icon_font, main) = app(|a| (a.dpi, a.font, a.icon_font, a.main));
    let parent = GetParent(d.hwndItem).unwrap_or_default();
    let on_main = parent == main;
    let (dc, rc, id) = (d.hDC, d.rcItem, d.CtlID as i32);
    let Face { bg, ink, tinted, hot, pressed, faint } = face_state(d);
    let shade = (pressed || hot).then(|| blend(bg, ink, press_tint(tinted, pressed)));
    fill(dc, rc, bg);
    if let Some(c) = shade {
        hot_shade(dc, rc, bg, c, dpi);
    }
    let fg = if tinted || hot || pressed { ink } else { pal().sub };
    if id == ID_FAV {
        // ★ filled (gray, or the header ink when tinted) in a favourite folder, otherwise ☆ with a small +
        let fav = app(|a| a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == parent)).is_some_and(|i| a.s.is_fav(&a.s.paths[i])));
        if fav {
            return draw_star(dc, rc, if tinted { ink } else { pal().sub }, Star::Filled, dpi);
        }
        // ☆ exactly where ★ goes, with a + at its exact centre
        return draw_star(dc, rc, fg, Star::Plus, dpi);
    }
    if !on_main {
        if let Some(turn) = (id == ID_UP).then(|| spin_turn(parent)).flatten() {
            return draw_spin(dc, rc, bg, shade, fg, dpi, turn);
        }
        let fg = if faint { faint_ink(bg, if tinted { ink } else { pal().sub }) } else { fg };
        return text(dc, rc, glyph(id), icon_font, fg, DT_CENTER);
    }
    let s = |v: i32| v * dpi / 96;
    if id == ID_TB_UPDATE {
        return draw_update(dc, rc, hot, pressed, font);
    }
    if id == ID_TB_LAYOUT {
        // the one label in the chrome: the current grid
        let icon = RECT { left: rc.left + s(4), right: rc.left + s(22), ..rc };
        text(dc, icon, glyph(id), icon_font, fg, DT_CENTER);
        return text(dc, RECT { left: icon.right + s(4), ..rc }, &text_of(d.hwndItem), font, pal().text, DT_LEFT);
    }
    if (ID_DRIVE..ID_DRIVE + 26).contains(&id) {
        // the active pane's drive: full-strength letter over a selection colour underline
        let (mark, accent) = app(|a| (a.drive_mark, a.s.accent_now()));
        let marked = mark == Some(b'A' + (id - ID_DRIVE) as u8);
        if marked {
            fill(dc, RECT { left: rc.left + s(7), top: rc.bottom - s(2), right: rc.right - s(7), bottom: rc.bottom }, accent);
        }
        let (label, ink) = (text_of(d.hwndItem), if marked { pal().text } else { fg });
        // the drive kind's icon before the letter on a wide button (panes.rs narrows them all when they don't fit);
        // without the icon list or its index, or when the letter would be cut, the letter alone as before
        let kind = app(|a| a.drives.iter().find(|x| x.2 == d.hwndItem).map(|x| x.1));
        let ix = kind.map_or(-1, drive_icon);
        if rc.right - rc.left >= s(40) && ix >= 0 {
            let icon = RECT { left: rc.left + s(6), top: rc.top + (rc.bottom - rc.top - s(16)) / 2, right: rc.left + s(22), bottom: 0 };
            let after = icon.right + s(4);
            if let Some(il) = (after + text_w(dc, &label, font) <= rc.right).then(|| small_ilist()).flatten() {
                if shell_icon(dc, &il, ix, RECT { right: icon.left + s(16), bottom: icon.top + s(16), ..icon }) {
                    return text(dc, RECT { left: after, ..rc }, &label, font, ink, DT_LEFT);
                }
            }
        }
        return text(dc, rc, &label, font, ink, DT_CENTER);
    }
    if (ID_CLOUD..ID_CLOUD + MAX_CLOUD as i32).contains(&id) {
        // its provider's icon (the folder icon when it has none), centred; underlined while the active pane is in it
        let (ix, marked, accent) = app(|a| (a.clouds.iter().find(|c| c.h == d.hwndItem).map_or(-1, |c| c.ix), a.cloud_mark == d.hwndItem, a.s.accent_now()));
        if marked {
            fill(dc, RECT { left: rc.left + s(7), top: rc.bottom - s(2), right: rc.right - s(7), bottom: rc.bottom }, accent);
        }
        let ix = if ix >= 0 { ix } else { folder_icon() };
        if let Some(il) = (ix >= 0).then(|| small_ilist()).flatten() {
            let (x, y) = (rc.left + (rc.right - rc.left - s(16)) / 2, rc.top + (rc.bottom - rc.top - s(16)) / 2);
            if shell_icon(dc, &il, ix, RECT { left: x, top: y, right: x + s(16), bottom: y + s(16) }) {
                return;
            }
        }
        return text(dc, rc, glyph(ID_CLOUD), icon_font, if marked { pal().text } else { fg }, DT_CENTER);
    }
    if (ID_DEVICE..ID_DEVICE + MAX_DEVICE as i32).contains(&id) {
        // the device's own icon (a phone glyph when it has none), centred; underlined while the active pane is in it
        let (ix, marked, accent) = app(|a| (a.devices.iter().find(|v| v.h == d.hwndItem).map_or(-1, |v| v.reg.ix), a.cloud_mark == d.hwndItem, a.s.accent_now()));
        if marked {
            fill(dc, RECT { left: rc.left + s(7), top: rc.bottom - s(2), right: rc.right - s(7), bottom: rc.bottom }, accent);
        }
        if let Some(il) = (ix >= 0).then(|| small_ilist()).flatten() {
            let (x, y) = (rc.left + (rc.right - rc.left - s(16)) / 2, rc.top + (rc.bottom - rc.top - s(16)) / 2);
            if shell_icon(dc, &il, ix, RECT { left: x, top: y, right: x + s(16), bottom: y + s(16) }) {
                return;
            }
        }
        return text(dc, rc, glyph(ID_DEVICE), icon_font, if marked { pal().text } else { fg }, DT_CENTER);
    }
    if id == ID_BIN {
        // the desktop's bin, empty or full, centred; a glyph bin when the shell gave no icon
        let ix = bin_icon();
        if let Some(il) = (ix >= 0).then(|| small_ilist()).flatten() {
            if let Ok(hi) = il.GetIcon(ix, ILD_NORMAL.0) {
                let (x, y) = (rc.left + (rc.right - rc.left - s(16)) / 2, rc.top + (rc.bottom - rc.top - s(16)) / 2);
                _ = DrawIconEx(dc, x, y, hi, s(16), s(16), 0, None, DI_NORMAL);
                _ = DestroyIcon(hi);
                return;
            }
        }
        return text(dc, rc, glyph(id), icon_font, fg, DT_CENTER);
    }
    // icon only, names live in the tooltips; save turns green with a dot while the layout differs
    if id == ID_TB_FAV {
        return draw_star(dc, rc, fg, Star::Outline, dpi);
    }
    let dirty = id == ID_TB_SAVE && app(|a| a.dirty);
    let ink = if dirty { GREEN } else if id == ID_TB_EV { EV_ORANGE } else { fg };
    text(dc, rc, glyph(id), icon_font, ink, DT_CENTER);
    if dirty {
        fill(dc, RECT { left: rc.right - s(10), top: rc.top + s(4), right: rc.right - s(4), bottom: rc.top + s(10) }, GREEN);
    }
}

// ---------- F5: the pane's up arrow turns into ↻ and spins ----------
// One turn per second, steady. A press while it spins keeps it going at least half a turn more, and it always
// stops on a whole turn, where ↑ comes back: pressed at 0 s, 0.9 s and 1.5 s, it stops at 2 s.
const SPIN_TURN: Duration = Duration::from_millis(1000);

pub(crate) unsafe fn spin_refresh(i: usize) {
    let now = Instant::now();
    let h = app(|a| {
        let p = a.panes.get_mut(i)?.as_mut()?;
        let start = match p.spin {
            Some((start, end)) if now < end => start,
            _ => now,
        };
        let need = (now + SPIN_TURN / 2 - start).as_nanos();
        let turns = need.div_ceil(SPIN_TURN.as_nanos()).max(1) as u32;
        p.spin = Some((start, start + SPIN_TURN * turns));
        Some(p.hwnd)
    });
    if let Some(h) = h {
        SetTimer(Some(h), TIMER_SPIN, 15, None);
        spin_redraw(h);
    }
}

pub(crate) unsafe fn spin_tick(h: HWND) {
    let now = Instant::now();
    let going = app(|a| {
        let p = a.panes.iter_mut().flatten().find(|p| p.hwnd == h)?;
        p.spin = p.spin.filter(|&(_, end)| now < end);
        p.spin.map(|_| ())
    });
    if going.is_none() {
        _ = KillTimer(Some(h), TIMER_SPIN);
    }
    spin_redraw(h);
}

unsafe fn spin_redraw(h: HWND) {
    if let Ok(b) = GetDlgItem(Some(h), ID_UP) {
        _ = InvalidateRect(Some(b), None, false);
    }
}

// ---------- a folder slow to open: a bar flows above the pane's list ----------
// Like Explorer's address bar or a browser tab while a page loads: from OnNavigationPending until the folder shows
// (or fails), a thin bar runs along the top of the list. A move that is over in under LOAD_SHOW paints nothing;
// one that never ends gives up after LOAD_GIVE_UP. Windows' "Animation effects" off: a still bar, full width.
pub(crate) const LOAD_SHOW: Duration = Duration::from_millis(300);
const LOAD_CYCLE: Duration = Duration::from_millis(1200);
const LOAD_GIVE_UP: Duration = Duration::from_secs(60);

/// The bar's span (from, to) across `width` px, `elapsed` after the move started; None while it isn't shown.
/// A run 30% of the width slides left to right, slowing down, once per LOAD_CYCLE; always inside 0..=width.
pub(crate) fn load_bar(width: i32, elapsed: Duration, animate: bool) -> Option<(i32, i32)> {
    if width <= 0 || elapsed < LOAD_SHOW || elapsed >= LOAD_GIVE_UP {
        return None;
    }
    if !animate {
        return Some((0, width));
    }
    let t = ((elapsed - LOAD_SHOW).as_millis() % LOAD_CYCLE.as_millis()) as f32 / LOAD_CYCLE.as_millis() as f32;
    let eased = 1.0 - (1.0 - t) * (1.0 - t);
    let run = (width * 3 / 10).max(1);
    let head = (eased * (width + run) as f32) as i32;
    Some(((head - run).clamp(0, width), head.clamp(0, width)))
}

// Windows' "Animation effects" (Settings > Accessibility > Visual effects)
pub(crate) unsafe fn load_animate() -> bool {
    let mut on = BOOL(1);
    _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(&mut on as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    on.as_bool()
}

// The band the bar runs in: s(2) high, right above the list.
unsafe fn load_band(h: HWND) -> RECT {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let dpi = app(|a| a.dpi);
    let v = pane_geo(rc.right, rc.bottom, dpi).view;
    RECT { top: v.top - 2 * dpi / 96, bottom: v.top, ..v }
}

// OnNavigationPending: the clock starts (again, for a move over another one). Nothing is painted until the first
// tick, LOAD_SHOW later, so a quick move costs no redraw. Not yet a pane (its first folder): nothing.
pub(crate) unsafe fn load_start(i: usize) {
    let now = Instant::now();
    let Some((h, shown)) = app(|a| {
        let p = a.panes.get_mut(i)?.as_mut()?;
        let shown = p.loading.is_some_and(|t| now - t >= LOAD_SHOW);
        p.loading = Some(now);
        p.load_framed = false; // the timer below is the LOAD_SHOW wait again
        Some((p.hwnd, shown))
    }) else {
        return;
    };
    SetTimer(Some(h), TIMER_LOAD, LOAD_SHOW.as_millis() as u32, None);
    if shown {
        _ = InvalidateRect(Some(h), Some(&load_band(h)), false); // the earlier move's bar goes until this one's shows
    }
}

// The move ended (OnNavigationComplete / Failed, or browse() refused it): the bar goes, repainted only if it showed.
// `since`: only a move started at or after it (browse() refusing its own move, not an earlier one still going).
pub(crate) unsafe fn load_end(i: usize, since: Option<Instant>) {
    let now = Instant::now();
    let Some((h, shown)) = app(|a| {
        let p = a.panes.get_mut(i)?.as_mut()?;
        let t = p.loading.filter(|&t| since.is_none_or(|s| t >= s))?;
        p.loading = None;
        Some((p.hwnd, now - t >= LOAD_SHOW))
    }) else {
        return;
    };
    _ = KillTimer(Some(h), TIMER_LOAD);
    if shown {
        _ = InvalidateRect(Some(h), Some(&load_band(h)), false);
    }
}

// TIMER_LOAD: the first tick (LOAD_SHOW after the start) turns into the frame timer; every tick repaints the band only.
// A tick a few ms early (timer granularity, ~15ms) waits out the rest rather than a whole LOAD_SHOW more.
pub(crate) unsafe fn load_tick(h: HWND) {
    let left = app(|a| {
        let p = a.panes.iter_mut().flatten().find(|p| p.hwnd == h)?;
        p.loading = p.loading.filter(|t| t.elapsed() < LOAD_GIVE_UP);
        let e = p.loading?.elapsed();
        let first = e >= LOAD_SHOW && !p.load_framed;
        p.load_framed |= first;
        Some((e, first))
    });
    match left {
        Some((e, _)) if e < LOAD_SHOW => {
            _ = SetTimer(Some(h), TIMER_LOAD, (LOAD_SHOW - e).as_millis() as u32 + 1, None);
            return;
        }
        Some((_, true)) => _ = SetTimer(Some(h), TIMER_LOAD, if load_animate() { 30 } else { 1000 }, None),
        Some((_, false)) => {}
        None => _ = KillTimer(Some(h), TIMER_LOAD),
    }
    _ = InvalidateRect(Some(h), Some(&load_band(h)), false);
}

// how far round the pane's spin is (0..1), None when it isn't spinning
fn spin_turn(pane: HWND) -> Option<f32> {
    let now = Instant::now();
    let (start, end) = app(|a| a.panes.iter().flatten().find(|p| p.hwnd == pane)?.spin)?;
    (now < end).then(|| (now - start).as_secs_f32() / SPIN_TURN.as_secs_f32() % 1.0)
}

// ↻ drawn from geometry, not the font: a glyph turned by GDI snaps to whole pixels and is re-hinted at every
// angle, and its ink box (arrowhead included) is not the ring's centre, so it shook. This one turns exactly
// about the ring's centre, with edges anti-aliased from each pixel's distance to the stroke.
pub(crate) fn refresh_coverage(n: i32, turn: f32) -> Vec<u8> {
    let c = n as f32 / 2.0;
    let (r, half) = (n as f32 * 0.35, (n as f32 / 16.0).max(1.0) / 2.0); // ring radius, half the stroke
    let (a0, sweep) = ((-50f32).to_radians(), 290f32.to_radians()); // the gap sits at the top right at rest
    let a1 = a0 + sweep;
    let end = (r * a1.cos(), r * a1.sin());
    let tangent = (-a1.sin(), a1.cos()); // clockwise on screen (y down)
    // arrowhead: two strokes back from the arc's end, each 45° off the way it came
    let (len, fan) = (n as f32 * 0.22, 45f32.to_radians());
    let wings = [fan, -fan].map(|f| {
        let (sf, cf) = f.sin_cos();
        let (bx, by) = (-tangent.0, -tangent.1);
        (end.0 + len * (bx * cf - by * sf), end.1 + len * (bx * sf + by * cf))
    });
    let start = (r * a0.cos(), r * a0.sin());
    let seg = |p: (f32, f32), q: (f32, f32), x: f32, y: f32| {
        let (dx, dy) = (q.0 - p.0, q.1 - p.1);
        let t = (((x - p.0) * dx + (y - p.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        (x - p.0 - t * dx).hypot(y - p.1 - t * dy)
    };
    let (sn, cs) = (turn * std::f32::consts::TAU).sin_cos();
    let mut out = Vec::with_capacity((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            // this pixel's centre, turned back by `turn`, against the shape at rest
            let (u, v) = (i as f32 + 0.5 - c, j as f32 + 0.5 - c);
            let (x, y) = (u * cs + v * sn, -u * sn + v * cs);
            let ang = (y.atan2(x) - a0).rem_euclid(std::f32::consts::TAU);
            // on the arc: distance to the circle; past its ends: to the nearer end (round caps)
            let ring = if ang <= sweep { (x.hypot(y) - r).abs() } else { (x - end.0).hypot(y - end.1).min((x - start.0).hypot(y - start.1)) };
            let d = wings.iter().fold(ring, |d, &w| d.min(seg(end, w, x, y)));
            out.push(((half - d + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    out
}

unsafe fn draw_spin(dc: HDC, rc: RECT, bg: u32, shade: Option<u32>, ink: u32, dpi: i32, turn: f32) {
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let n = 16 * dpi / 96;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for a in refresh_coverage(n, turn) {
        let pre = |ch: u32| ((ch & 0xFF) * a as u32 / 255) as u8; // premultiplied BGRA
        px.extend_from_slice(&[pre(ink), pre(ink >> 8), pre(ink >> 16), a]);
    }
    // off-screen in one go, so a frame never shows the background alone
    let mem = CreateCompatibleDC(Some(dc));
    let bmp = CreateCompatibleBitmap(dc, w, h);
    let old = SelectObject(mem, bmp.into());
    // the button's face as draw_button_face paints it: its background, then the hover / pressed shade
    let all = RECT { left: 0, top: 0, right: w, bottom: h };
    fill(mem, all, bg);
    if let Some(c) = shade {
        hot_shade(mem, all, bg, c, dpi);
    }
    let (x, y) = ((w - n) / 2, (h - n) / 2);
    blit_bgra(mem, RECT { left: x, top: y, right: x + n, bottom: y + n }, n, n, &px);
    _ = BitBlt(dc, rc.left, rc.top, w, h, Some(mem), 0, 0, SRCCOPY);
    SelectObject(mem, old);
    _ = DeleteObject(bmp.into());
    _ = DeleteDC(mem);
}

// A chubby five-point star (inner radius 0.56 of the outer one) with softly rounded tips, as 0..=255
// coverage per pixel of an n×n box. Font stars are too sharp, so it's computed: 4×4 samples per pixel.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Star {
    Outline,
    Filled,
    Plus, // outline with a + in its exact centre (add to favourites)
}

pub(crate) fn star_coverage(n: i32, kind: Star) -> Vec<u8> {
    let filled = kind == Star::Filled;
    let (c, r_out) = (n as f32 / 2.0, n as f32 / 2.0 - 0.5);
    let pt = |k: i32, r: f32| {
        let a = (-90.0 + 36.0 * k as f32).to_radians();
        (c + r * a.cos(), c + r * a.sin())
    };
    let poly: Vec<(f32, f32)> = (0..10).map(|k| pt(k, if k % 2 == 0 { r_out } else { r_out * 0.56 })).collect();
    let (stroke, round) = ((n as f32 * 0.11).max(1.2), r_out * 0.17);
    // each tip is replaced by a circle tangent to both of its edges
    let tips: Vec<((f32, f32), f32)> = (0..5)
        .map(|i| {
            let (t, v1, v2) = (poly[2 * i], poly[(2 * i + 9) % 10], poly[2 * i + 1]);
            let (a, b) = ((v1.0 - t.0, v1.1 - t.1), (v2.0 - t.0, v2.1 - t.1));
            let half = ((a.0 * b.0 + a.1 * b.1) / (a.0.hypot(a.1) * b.0.hypot(b.1))).acos() / 2.0; // half the tip angle
            let d = round / half.sin();
            let (ux, uy) = ((t.0 - c) / r_out, (t.1 - c) / r_out);
            ((t.0 - ux * d, t.1 - uy * d), d)
        })
        .collect();
    let inside_dist = |x: f32, y: f32| -> Option<f32> {
        for (i, &((cx, cy), d)) in tips.iter().enumerate() {
            let (ux, uy) = ((poly[2 * i].0 - c) / r_out, (poly[2 * i].1 - c) / r_out);
            if (x - cx) * ux + (y - cy) * uy > 0.0 && (x - poly[2 * i].0).hypot(y - poly[2 * i].1) < 2.0 * d {
                let e = round - (x - cx).hypot(y - cy);
                return (e >= 0.0).then_some(e);
            }
        }
        let mut inside = false;
        let mut edge = f32::MAX;
        for k in 0..10 {
            let ((x1, y1), (x2, y2)) = (poly[k], poly[(k + 1) % 10]);
            if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
                inside = !inside;
            }
            let (dx, dy) = (x2 - x1, y2 - y1);
            let t = (((x - x1) * dx + (y - y1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            edge = edge.min((x - x1 - t * dx).hypot(y - y1 - t * dy));
        }
        inside.then_some(edge)
    };
    let mut out = vec![0u8; (n * n) as usize];
    for py in 0..n {
        for px in 0..n {
            let mut hit = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let (x, y) = (px as f32 + (sx as f32 + 0.5) / 4.0, py as f32 + (sy as f32 + 0.5) / 4.0);
                    hit += inside_dist(x, y).is_some_and(|e| filled || e < stroke) as u32;
                }
            }
            out[(py * n + px) as usize] = (hit * 255 / 16) as u8;
        }
    }
    if kind == Star::Plus {
        // whole pixels, with the same parity as n so the + is centred exactly on the star's centre,
        // and arms short enough to float in the hollow without touching the outline
        let mut t = (n / 12).max(1);
        if (n - t) % 2 != 0 {
            t += 1;
        }
        let hollow = r_out * 0.516 - stroke; // centre to the inside of the outline, at its nearest
        let len = t + 2 * ((hollow - 0.4 - t as f32 / 2.0) as i32).max(1);
        let (a, b) = ((n - t) / 2, (n - len) / 2);
        for i in b..b + len {
            for j in a..a + t {
                out[(j * n + i) as usize] = 255;
                out[(i * n + j) as usize] = 255;
            }
        }
    }
    out
}

pub(crate) unsafe fn draw_star(dc: HDC, rc: RECT, rgb: u32, kind: Star, dpi: i32) {
    let n = 15 * dpi / 96;
    let mut px = Vec::with_capacity((n * n * 4) as usize);
    for a in star_coverage(n, kind) {
        let pre = |ch: u32| ((ch & 0xFF) * a as u32 / 255) as u8; // premultiplied BGRA
        px.extend_from_slice(&[pre(rgb), pre(rgb >> 8), pre(rgb >> 16), a]);
    }
    let (x, y) = ((rc.left + rc.right - n) / 2, (rc.top + rc.bottom - n) / 2);
    blit_bgra(dc, RECT { left: x, top: y, right: x + n, bottom: y + n }, n, n, &px);
}

// Premultiplied top-down BGRA pixels (w×h) drawn into `to` through a throwaway DIB.
pub(crate) unsafe fn blit_bgra(dc: HDC, to: RECT, w: i32, h: i32, px: &[u8]) {
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else { return };
    std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize).copy_from_slice(px);
    let mem = CreateCompatibleDC(Some(dc));
    let old = SelectObject(mem, bmp.into());
    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    _ = AlphaBlend(dc, to.left, to.top, to.right - to.left, to.bottom - to.top, mem, 0, 0, w, h, blend);
    SelectObject(mem, old);
    _ = DeleteDC(mem);
    _ = DeleteObject(bmp.into());
}

// Hover highlight for owner-drawn buttons (they have no hot state of their own).
pub(crate) unsafe fn set_hover(h: HWND) {
    let old = app(|a| std::mem::replace(&mut a.hover, h));
    if old == h {
        return;
    }
    for w in [old, h].into_iter().filter(|w| !w.0.is_null()) {
        _ = InvalidateRect(Some(w), None, false);
    }
    if !h.0.is_null() {
        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
        _ = TrackMouseEvent(&mut t);
    }
}

pub(crate) unsafe fn is_icon_button(h: HWND) -> bool {
    let mut c = [0u16; 16];
    let n = GetClassNameW(h, &mut c) as usize;
    String::from_utf16_lossy(&c[..n]).eq_ignore_ascii_case("Button")
        && GetWindowLongPtrW(h, GWL_STYLE) as i32 & 0xF == BS_OWNERDRAW
}

pub(crate) unsafe fn add_tip(btn: HWND, tip: &str) {
    tip_text(btn, tip, TTM_ADDTOOLW);
}

// TTM_ADDTOOLW to add a control's tooltip, TTM_UPDATETIPTEXTW to change its text.
pub(crate) unsafe fn tip_text(btn: HWND, tip: &str, msg: u32) {
    let tip = HSTRING::from(tip);
    let ti = TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: GetParent(btn).unwrap_or_default(),
        uId: btn.0 as usize,
        lpszText: PWSTR(tip.as_ptr() as *mut u16),
        ..Default::default()
    };
    let (tips, main) = app(|a| (a.tip, a.main));
    SendMessageW(tips, msg, None, Some(LPARAM(&ti as *const _ as isize)));
    // a toolbar button's screen reader description is its tooltip, so the keys it names are read out with it
    if ti.hwnd == main {
        a11y_desc(btn, (msg != TTM_DELTOOLW && !tip.is_empty()).then(|| tip.to_string()).as_deref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A w × h shape as round_fill lays it on `under`: the cross in `rgb`, the corner pixels smoothed toward `under`.
    fn laid(w: i32, h: i32, radius: i32, rgb: u32, under: u32) -> Vec<u32> {
        let (k, corners) = round_corners(w, h, radius);
        let mut px = vec![under; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                if (x >= k && x < w - k) || (y >= k && y < h - k) {
                    px[(y * w + x) as usize] = rgb;
                }
            }
        }
        for (x, y, c) in corners {
            px[(y * w + x) as usize] = blend(under, rgb, c);
        }
        px
    }

    // Windows 11's rounded fields, chips and hover shades at 100 / 150 / 200 %: corners on the colour beneath, the
    // middle and the middle of each side solid, mirrored both ways, smoothed (part-covered pixels), and square
    // (fill's pixels exactly) at radius 0.
    #[test]
    fn rounded_shapes() {
        let (rgb, under) = (0x2060C0u32, 0xF0F0F0u32);
        for dpi in [96, 144, 192] {
            let s = |v: i32| v * dpi / 96;
            for radius in [0, s(3), s(4)] {
                for (w, h) in [(s(160), s(28)), (s(18), s(16)), (s(16), s(16)), (s(40), s(22))] {
                    let px = laid(w, h, radius, rgb, under);
                    let at = |x: i32, y: i32| px[(y * w + x) as usize];
                    let tag = format!("dpi {dpi}, radius {radius}, {w}×{h}");
                    if radius == 0 {
                        assert!(px.iter().all(|&c| c == rgb), "{tag}: not fill's pixels");
                        continue;
                    }
                    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
                        assert_eq!(at(x, y), under, "{tag}: corner ({x}, {y})");
                    }
                    for (x, y) in [(w / 2, h / 2), (w / 2, 0), (w / 2, h - 1), (0, h / 2), (w - 1, h / 2)] {
                        assert_eq!(at(x, y), rgb, "{tag}: ({x}, {y})");
                    }
                    for y in 0..h {
                        for x in 0..w {
                            assert_eq!(at(x, y), at(w - 1 - x, y), "{tag}: ({x}, {y}) not mirrored across");
                            assert_eq!(at(x, y), at(x, h - 1 - y), "{tag}: ({x}, {y}) not mirrored down");
                        }
                    }
                    let soft = px.iter().filter(|&&c| c != rgb && c != under).count();
                    assert!(soft >= 2, "{tag}: {soft} smoothed pixels");
                }
            }
        }
        // cut to fit: never past half the shorter side; nothing at all for an empty or folded shape
        assert_eq!(round_corners(40, 6, 8).0, 3);
        assert_eq!(round_corners(5, 30, 8).0, 2);
        for (w, h) in [(0, 20), (20, 0), (-4, 20), (1, 1)] {
            assert_eq!(round_corners(w, h, 6), (0, Vec::new()), "{w}×{h}");
        }
    }

    // The focus ring round a rounded field / button: only corner pixels it touches, each split between the ring,
    // the space within and (the rest) the space round it; the outermost corner left alone; square at radius 0.
    #[test]
    fn rounded_rings() {
        for dpi in [96, 144, 192] {
            let s = |v: i32| v * dpi / 96;
            for (width, radius) in [(1, s(4)), ((2 * dpi / 96).max(2), s(6))] {
                let (w, h) = (s(28), s(24));
                let (k, px) = ring_corners(w, h, width, radius);
                assert_eq!(k, radius, "dpi {dpi}");
                assert!(px.iter().all(|&(x, y, on, within)| {
                    on > 0.0 && within >= 0.0 && on + within <= 1.0 + 1e-6 && (x < k || x >= w - k) && (y < k || y >= h - k)
                }));
                assert!(!px.iter().any(|&(x, y, ..)| (x, y) == (0, 0)), "dpi {dpi}: outermost corner touched");
                assert!(px.iter().any(|&(.., on, within)| on < 1.0 && within > 0.0), "dpi {dpi}: inner edge not smoothed");
                assert!(px.iter().any(|&(.., on, within)| on < 1.0 && within == 0.0), "dpi {dpi}: outer edge not smoothed");
                for &(x, y, on, within) in &px {
                    assert!(px.contains(&(w - 1 - x, y, on, within)) && px.contains(&(x, h - 1 - y, on, within)), "dpi {dpi}: ({x}, {y})");
                }
            }
        }
        assert_eq!(ring_corners(28, 24, 1, 0), (0, Vec::new()));
        assert_eq!(ring_corners(0, 24, 1, 4), (0, Vec::new()));
        // shares mix by weight: all ring is the ring's colour, half and half lands between
        assert_eq!(mix([(0x000000, 0.0), (0x2060C0, 1.0), (0xFFFFFF, 0.0)]), 0x2060C0);
        assert_eq!(mix([(0x000000, 0.5), (0x2060C0, 0.0), (0xFFFFFF, 0.5)]), 0x808080);
    }
}
