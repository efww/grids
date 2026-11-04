use crate::*;

// ---------- drawing (plain GDI: square shapes need no anti-aliasing, and fills are cheap) ----------

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
            (a.s.accent_now(), on_color(a.s.accent_now()))
        } else {
            (pal().card, pal().text)
        }
    })
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
    let (hover, dpi, font, icon_font, main) = app(|a| (a.hover, a.dpi, a.font, a.icon_font, a.main));
    let parent = GetParent(d.hwndItem).unwrap_or_default();
    let on_main = parent == main;
    let (dc, rc, id) = (d.hDC, d.rcItem, d.CtlID as i32);
    let pressed = d.itemState.0 & ODS_SELECTED.0 != 0;
    let hot = hover == d.hwndItem;
    let (bg, ink) = if on_main { (pal().bg, pal().text) } else { header_colors(parent) };
    let tinted = bg != pal().bg && bg != pal().card;
    fill(dc, rc, bg);
    if pressed || hot {
        fill(dc, rc, blend(bg, ink, press_tint(tinted, pressed)));
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
            let back = if pressed || hot { blend(bg, ink, press_tint(tinted, pressed)) } else { bg };
            return draw_spin(dc, rc, back, fg, icon_font, turn);
        }
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
        return text(dc, rc, &text_of(d.hwndItem), font, if marked { pal().text } else { fg }, DT_CENTER);
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

// how far round the pane's spin is (0..1), None when it isn't spinning
fn spin_turn(pane: HWND) -> Option<f32> {
    let now = Instant::now();
    let (start, end) = app(|a| a.panes.iter().flatten().find(|p| p.hwnd == pane)?.spin)?;
    (now < end).then(|| (now - start).as_secs_f32() / SPIN_TURN.as_secs_f32() % 1.0)
}

// ↻ turned clockwise about its own ink centre (not its cell's, or it wobbles), drawn off-screen in one go
unsafe fn draw_spin(dc: HDC, rc: RECT, back: u32, ink: u32, font: HFONT, turn: f32) {
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let mem = CreateCompatibleDC(Some(dc));
    let bmp = CreateCompatibleBitmap(dc, w, h);
    let old_bmp = SelectObject(mem, bmp.into());
    fill(mem, RECT { left: 0, top: 0, right: w, bottom: h }, back);
    let old_font = SelectObject(mem, font.into());
    let g: Vec<u16> = ICON_REFRESH.encode_utf16().collect();
    let one = FIXED { fract: 0, value: 1 };
    let unit = MAT2 { eM11: one, eM22: one, ..Default::default() };
    let mut gm = GLYPHMETRICS::default();
    GetGlyphOutlineW(mem, g[0] as u32, GGO_METRICS, &mut gm, 0, None, &unit);
    // ink centre from the pen at the baseline (y down)
    let cx = gm.gmptGlyphOrigin.x as f32 + gm.gmBlackBoxX as f32 / 2.0;
    let cy = -(gm.gmptGlyphOrigin.y as f32) + gm.gmBlackBoxY as f32 / 2.0;
    let (sn, cs) = (turn * std::f32::consts::TAU).sin_cos();
    let (ox, oy) = (w as f32 / 2.0, h as f32 / 2.0);
    SetGraphicsMode(mem, GM_ADVANCED);
    let xf = XFORM { eM11: cs, eM12: sn, eM21: -sn, eM22: cs, eDx: ox - cx * cs + cy * sn, eDy: oy - cx * sn - cy * cs };
    _ = SetWorldTransform(mem, &xf);
    SetTextAlign(mem, TA_BASELINE | TA_LEFT);
    SetBkMode(mem, TRANSPARENT);
    SetTextColor(mem, cref(ink));
    _ = TextOutW(mem, 0, 0, &g);
    _ = ModifyWorldTransform(mem, None, MWT_IDENTITY);
    _ = BitBlt(dc, rc.left, rc.top, w, h, Some(mem), 0, 0, SRCCOPY);
    SelectObject(mem, old_font);
    SelectObject(mem, old_bmp);
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
    SendMessageW(app(|a| a.tip), msg, None, Some(LPARAM(&ti as *const _ as isize)));
}
