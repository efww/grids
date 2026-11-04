use windows::Win32::UI::Controls::{IImageList, ILD_NORMAL};

use crate::imports::*;
use super::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;

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
    draw_line(dc, &mut r, s, font, rgb, align | DT_END_ELLIPSIS);
}

/// A path, shortened in the middle so the last part stays visible.
pub(crate) unsafe fn text_path(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, align: DRAW_TEXT_FORMAT) {
    draw_line(dc, &mut r, s, font, rgb, align | DT_PATH_ELLIPSIS);
}

/// Which ink box `ink_line_top` centers. Chosen from the string, because one face
/// (and one cached `HFONT`) draws Hangul, kana, and Latin with different ink.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InkKind {
    /// Caps sit under the internal leading.
    Latin,
    /// Hangul, kana, and ideographs fill the top of the em and leave descent unused.
    Cjk,
    /// Icons, punctuation, and other marks that already fill the em square.
    Em,
}

/// Square CJK ink: Hangul, kana, ideographs, and the halfwidth katakana block.
fn is_cjk_ink(c: char) -> bool {
    matches!(
        c,
        '\u{1100}'..='\u{11FF}'
            | '\u{2E80}'..='\u{2FDF}'
            | '\u{3040}'..='\u{30FF}'
            | '\u{3130}'..='\u{318F}'
            | '\u{31F0}'..='\u{31FF}'
            | '\u{3200}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{A960}'..='\u{A97F}'
            | '\u{AC00}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF66}'..='\u{FF9D}'
            | '\u{20000}'..='\u{3FFFD}'
    )
}

/// CJK wins when a run mixes scripts: the square glyphs set the top of the ink.
pub(crate) fn ink_kind(s: &str) -> InkKind {
    let mut cjk = false;
    let mut cap = false;
    for c in s.chars() {
        if is_cjk_ink(c) {
            cjk = true;
        } else if c.is_alphabetic() || c.is_ascii_digit() {
            cap = true;
        }
    }
    if cjk {
        InkKind::Cjk
    } else if cap {
        InkKind::Latin
    } else {
        InkKind::Em
    }
}

/// Top of a `DT_TOP | DT_SINGLELINE` rect that centers the visible glyph box in the cell.
///
/// `DT_VCENTER` centers `tmAscent + tmDescent` and gives the extra pixel to the bottom,
/// so an odd leftover leaves the ink half a pixel high. Latin caps leave the internal
/// leading empty, so that center sits about `internalLeading / 2` too low (Segoe UI
/// at 11px). CJK ink on Malgun Gothic at 100% is only that leftover pixel high in a
/// 16 DIP chip (top gap 1, bottom gap 2). Shifting by the whole `descent / 2` (1px
/// for these faces) flips the chip to top 2 / bottom 1 and drops a centered button
/// from 4/4 to 5/3. The CJK step is about half of that, truncated toward zero so it
/// does not cross the cell center, and it is kept only when the gaps get strictly
/// closer. An even split stays even. Icons stay on the `tmHeight` center.
pub(crate) fn ink_line_top(cell_top: i32, cell_h: i32, m: LineMetrics, kind: InkKind) -> i32 {
    let tm_h = m.height();
    if cell_h <= 0 || tm_h <= 0 {
        return cell_top;
    }
    let vtop = cell_top + (cell_h - tm_h) / 2;
    let leading = m.internal_leading.clamp(0, m.ascent);
    let descent = m.descent.max(0);
    let nudge = match kind {
        InkKind::Latin => -(leading / 2),
        // Half of the old `descent / 2` step, truncated toward the unshifted
        // center. That half is 0 for descent 2 or 3 (100% and 12px faces). A
        // taller face would offer 1px or more, but `DT_VCENTER` is already
        // within half a pixel, so the step is not taken: it would only swap
        // the leftover pixel on a chip or open an even split on a button.
        InkKind::Cjk => cjk_nudge(descent, cell_h, tm_h),
        InkKind::Em => 0,
    };
    vtop + nudge
}

/// Half of `descent / 2`, kept only when the tm box is off by more than that.
/// The centered gap error is 0 or 1, so this is 0 whenever the line fits.
fn cjk_nudge(descent: i32, cell_h: i32, tm_h: i32) -> i32 {
    let half = (descent.max(0) / 2) / 2;
    let slack = cell_h - tm_h;
    let top = slack / 2;
    let bottom = slack - top;
    let error = (bottom - top).abs();
    if error > half { half } else { 0 }
}

/// How far `ink_line_top` moves a line whose cell is exactly the font height.
/// Positive moves the glyphs down. Used when a block is already centered on `tmHeight`.
pub(crate) unsafe fn optical_nudge(dc: HDC, font: HFONT, s: &str) -> i32 {
    let m = line_metrics(dc, font);
    let h = m.height();
    if h <= 0 {
        return 0;
    }
    ink_line_top(0, h, m, ink_kind(s))
}

fn without_vertical(extra: DRAW_TEXT_FORMAT) -> DRAW_TEXT_FORMAT {
    DRAW_TEXT_FORMAT(extra.0 & !(DT_TOP.0 | DT_VCENTER.0 | DT_BOTTOM.0))
}

unsafe fn draw_line(dc: HDC, r: &mut RECT, s: &str, font: HFONT, rgb: u32, extra: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return; // an empty Vec's dangling pointer crashes DrawTextW even with length 0
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(rgb));
    paint_ink_words(dc, *r, &mut w, font, extra, s);
    SelectObject(dc, old);
}

/// Single line, font already selected, colours already set.
unsafe fn paint_ink_words(dc: HDC, r: RECT, words: &mut [u16], font: HFONT, extra: DRAW_TEXT_FORMAT, source: &str) {
    let m = line_metrics(dc, font);
    let flags = without_vertical(extra) | DT_SINGLELINE | DT_NOPREFIX;
    if m.ascent <= 0 {
        let mut fallback = r;
        DrawTextW(dc, words, &mut fallback, flags | DT_VCENTER);
        return;
    }
    let top = ink_line_top(r.top, r.bottom - r.top, m, ink_kind(source));
    let mut placed = RECT { top, bottom: top.saturating_add(m.height().max(1)), ..r };
    let noclip = extra.0 & DT_NOCLIP.0 != 0;
    let saved = if noclip { 0 } else { SaveDC(dc) };
    if saved != 0 {
        IntersectClipRect(dc, r.left, r.top, r.right, r.bottom);
    }
    DrawTextW(dc, words, &mut placed, flags | DT_TOP | if noclip { DT_NOCLIP } else { DRAW_TEXT_FORMAT(0) });
    if saved != 0 {
        let _ = RestoreDC(dc, saved);
    }
}

/// One line with the caller's flags. `DT_VCENTER` uses the same glyph box as `text`.
pub(crate) unsafe fn draw_ink_line(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, extra: DRAW_TEXT_FORMAT) {
    draw_line(dc, &mut r, s, font, rgb, extra);
}

/// Settings checkbox. 16 DIP is the caller's box; the corner radius is 3 DIP.
/// `hot` uses the same accent wash as a hovered settings button. Light and dark come from `pal()`.
pub(crate) unsafe fn draw_tick(dc: HDC, r: RECT, tick: crate::shell::ctxmenu::Tick, accent: u32, dpi: i32, icon: HFONT, ground: u32, hot: bool) {
    let s = |v: i32| v * dpi.max(96) / 96;
    let rad = s(3).max(1);
    let card = pal().card;
    match tick {
        crate::shell::ctxmenu::Tick::On => {
            let wash = if hot { blend(accent, 0xFFFFFF, 0.12) } else { accent };
            aa_rfill(dc, r, rad, wash, Some(wash), ground);
            text(dc, r, "\u{E73E}", icon, on_color(wash), DT_CENTER);
        }
        crate::shell::ctxmenu::Tick::Mixed => {
            let wash = if hot { blend(card, accent, 0.12) } else { card };
            aa_rfill(dc, r, rad, wash, Some(accent), ground);
            let mid = (r.top + r.bottom) / 2;
            let bar = s(1).max(1);
            fill(dc, RECT { left: r.left + s(3), top: mid - bar, right: r.right - s(3), bottom: mid + bar }, accent);
        }
        crate::shell::ctxmenu::Tick::Off => {
            let wash = if hot { blend(card, accent, 0.12) } else { card };
            let edge = if hot { accent } else { pal().line };
            aa_rfill(dc, r, rad, wash, Some(edge), ground);
        }
    }
}

/// Multi-line text wrapped at `r`'s width. `calc` only measures, with the same font and flags.
pub(crate) unsafe fn wrap_text(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, calc: bool) -> (i32, i32) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return (0, 0);
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

// Pane header colours (background, text/icons). A colour label fills the bar the way the accent
// does: solid while this pane is active, the same colour softened into the card while it is not.
// No label keeps the accent on the active pane only. Address and filter fields keep their own colours.
fn header_chrome(pane: HWND) -> PaneChrome {
    app(|a| {
        let active = a.panes.get(a.s.active).and_then(|p| p.as_ref()).is_some_and(|p| p.hwnd == pane) || a.drop_hover == pane;
        let label = a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == pane)).and_then(|i| tint_color(a.s.tints.get(i).copied().unwrap_or(0)));
        pane_chrome(a.s.accent_now(), label, active, a.s.edge, pal())
    })
}

pub(crate) fn header_colors(pane: HWND) -> (u32, u32) {
    let c = header_chrome(pane);
    (c.head, c.ink)
}

pub(crate) unsafe fn draw_button(d: &DRAWITEMSTRUCT) {
    let (hover, dpi, font, icon_font, main) = app(|a| (a.hover, a.dpi, a.font, a.icon_font, a.main));
    let parent = GetParent(d.hwndItem).unwrap_or_default();
    let on_main = parent == main;
    let (dc, rc, id) = (d.hDC, d.rcItem, d.CtlID as i32);
    let disabled = d.itemState.0 & ODS_DISABLED.0 != 0;
    let pressed = !disabled && d.itemState.0 & ODS_SELECTED.0 != 0;
    let hot = !disabled && hover == d.hwndItem;
    let (bg, ink) = if on_main { (pal().bg, pal().text) } else { header_colors(parent) };
    let tinted = bg != pal().bg && bg != pal().card;
    fill(dc, rc, bg);
    if pressed || hot {
        let t = match (tinted, pressed) {
            (true, true) => 0.3,
            (true, false) => 0.18,
            (false, true) => 0.13,
            (false, false) => 0.07,
        };
        fill(dc, rc, blend(bg, ink, t));
    }
    // Disabled sits below the resting sub colour. Hover and press use the full ink, same as the glyphs.
    let fg = if disabled {
        blend(bg, pal().sub, 0.45)
    } else if tinted || hot || pressed {
        ink
    } else {
        pal().sub
    };
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
        return text(dc, rc, glyph(id), icon_font, fg, DT_CENTER);
    }
    let s = |v: i32| v * dpi / 96;
    if (ID_DRIVE..ID_DRIVE + 26).contains(&id) {
        // The active pane's drive: full-strength letter over a selection-colour underline.
        let (mark, accent) = app(|a| (a.drive_mark, a.s.accent_now()));
        let marked = mark == Some(b'A' + (id - ID_DRIVE) as u8);
        if marked {
            fill(dc, RECT { left: rc.left + s(7), top: rc.bottom - s(2), right: rc.right - s(7), bottom: rc.bottom }, accent);
        }
        return text(dc, rc, &text_of(d.hwndItem), font, if marked { pal().text } else { fg }, DT_CENTER);
    }
    if id == ID_TB_LAYOUT {
        // the one label in the chrome: the current grid
        let icon = RECT { left: rc.left + s(4), right: rc.left + s(22), ..rc };
        text(dc, icon, glyph(id), icon_font, fg, DT_CENTER);
        return text(dc, RECT { left: icon.right + s(4), ..rc }, &text_of(d.hwndItem), font, pal().text, DT_LEFT);
    }
    // icon only, names live in the tooltips
    if id == ID_TB_FAV {
        return draw_star(dc, rc, fg, Star::Outline, dpi);
    }
    // Stacked windows, drawn (not a Segoe glyph). A dirty layout turns the whole save icon green.
    if id == ID_TB_SAVE || id == ID_TB_LOAD {
        let dirty = id == ID_TB_SAVE && app(|a| a.dirty);
        let kind = if id == ID_TB_SAVE { SessionIcon::Save } else { SessionIcon::Load };
        draw_session_icon(dc, rc, kind, if dirty { GREEN } else { fg }, dpi);
        return;
    }
    text(dc, rc, glyph(id), icon_font, fg, DT_CENTER);
}

unsafe fn draw_session_icon(dc: HDC, rc: RECT, kind: SessionIcon, rgb: u32, dpi: i32) {
    let n = session_icon_px(dpi);
    let px = session_icon_bgra(n, kind, rgb);
    let x = (rc.left + rc.right - n) / 2;
    let y = (rc.top + rc.bottom - n) / 2;
    blit_bgra(dc, RECT { left: x, top: y, right: x + n, bottom: y + n }, n, n, &px);
}

// A chubby five-point star (inner radius 0.56 of the outer one) with softly rounded tips, as 0..=255
// coverage per pixel of an n×n box. The mask is sampled at AA_SCALE, then box-filtered, so the tips
// match the toggle corners. Font stars are too sharp, so the shape stays computed.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Star {
    Outline, // tab-row favourites button; Plus is this shape with a +
    Filled,
    Plus, // outline with a + in its exact centre (add to favourites)
}

/// `n`×`n` coverage. Sampled on the shared 3× grid and box-filtered down to `n`.
pub(crate) fn star_coverage(n: i32, kind: Star) -> Vec<u8> {
    let n = n.max(0);
    if n == 0 {
        return Vec::new();
    }
    let scale = super::aa::AA_SCALE.max(1);
    if scale <= 1 || n > 160 {
        return star_mask(n, kind);
    }
    let hi = n * scale;
    let src = star_mask(hi, kind);
    let mut dst = vec![0u8; (n as usize) * (n as usize)];
    super::aa::box_down_cov(&src, hi as usize, &mut dst, n as usize, n as usize, scale as usize);
    dst
}

fn star_mask(n: i32, kind: Star) -> Vec<u8> {
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
    let cov = star_coverage(n, kind);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: n,
            biHeight: -n,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else { return };
    let px = std::slice::from_raw_parts_mut(bits as *mut u8, (n * n * 4) as usize);
    for (i, &a) in cov.iter().enumerate() {
        let pre = |ch: u32| ((ch & 0xFF) * a as u32 / 255) as u8; // premultiplied BGRA
        px[i * 4..i * 4 + 4].copy_from_slice(&[pre(rgb), pre(rgb >> 8), pre(rgb >> 16), a]);
    }
    let mem = CreateCompatibleDC(Some(dc));
    let old = SelectObject(mem, bmp.into());
    let (x, y) = ((rc.left + rc.right - n) / 2, (rc.top + rc.bottom - n) / 2);
    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    _ = AlphaBlend(dc, x, y, n, n, mem, 0, 0, n, n, blend);
    SelectObject(mem, old);
    _ = DeleteDC(mem);
    _ = DeleteObject(bmp.into());
}

#[cfg(test)]
mod star_aa_tests {
    use super::*;

    fn sixteen_level(a: u8) -> bool {
        (0..=16).any(|h| a == (h * 255 / 16) as u8)
    }

    #[test]
    fn outline_edge_is_finer_than_a_4_by_4_sample_grid() {
        let ring = star_coverage(15, Star::Outline);
        assert!(
            ring.iter().any(|&a| a > 0 && a < 255 && !sixteen_level(a)),
            "expected box-filtered coverage between the 16 sample steps"
        );
    }
}

/// Premultiplied top-down BGRA (`w`×`h`) drawn into `to` through a throwaway DIB.
pub(crate) unsafe fn blit_bgra(dc: HDC, to: RECT, w: i32, h: i32, px: &[u8]) {
    if w <= 0 || h <= 0 || px.len() < (w as usize) * (h as usize) * 4 {
        return;
    }
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
    std::slice::from_raw_parts_mut(bits as *mut u8, (w as usize) * (h as usize) * 4).copy_from_slice(&px[..(w as usize) * (h as usize) * 4]);
    let mem = CreateCompatibleDC(Some(dc));
    let old = SelectObject(mem, bmp.into());
    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    _ = AlphaBlend(dc, to.left, to.top, to.right - to.left, to.bottom - to.top, mem, 0, 0, w, h, blend);
    SelectObject(mem, old);
    _ = DeleteDC(mem);
    _ = DeleteObject(bmp.into());
}

// Repaints the save icon and puts the session name / unsaved state into its tooltip.
pub(crate) unsafe fn refresh_save_button() {
    let (main, tip, file, dirty) = app(|a| (a.main, a.tip, a.s.file.clone(), a.dirty));
    let Ok(b) = GetDlgItem(Some(main), ID_TB_SAVE) else { return };
    _ = InvalidateRect(Some(b), None, false);
    let mut hint = match Path::new(&file).file_stem() {
        None => tr(crate::Key::SaveThisTabCtrlS).into(),
        Some(n) => tf(crate::Key::SaveTabTo, &[("name", &n.to_string_lossy())]),
    };
    if dirty {
        hint += tr(crate::Key::UnsavedLayoutChanges);
    }
    let hint = HSTRING::from(hint);
    let ti = TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND,
        hwnd: main,
        uId: b.0 as usize,
        lpszText: PWSTR(hint.as_ptr() as *mut u16),
        ..Default::default()
    };
    SendMessageW(tip, TTM_UPDATETIPTEXTW, None, Some(LPARAM(&ti as *const _ as isize)));
}

// Polled with the status timer: view modes are only observable by asking the views.
pub(crate) unsafe fn check_dirty() {
    for i in 0..app(|a| a.s.panes()) {
        save_view(i);
    }
    let changed = app(|a| {
        let (n, cols, tab) = (a.s.panes(), a.s.cols.clone(), a.s.tab);
        if let Some(b) = a.tab_base.get_mut(tab) {
            absorb_cols(&mut b.grid, &cols, n);
        }
        pull_dirty(a)
    });
    if changed {
        refresh_save_button();
    }
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

unsafe fn send_tip(ti: &TTTOOLINFOW, msg: u32, wp: Option<WPARAM>) {
    SendMessageW(app(|a| a.tip), msg, wp, Some(LPARAM(ti as *const _ as isize)));
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
    send_tip(&ti, msg, None);
}

pub(crate) unsafe fn pwstr(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as _));
    s
}

pub(crate) unsafe fn name(pidl: *const ITEMIDLIST, kind: SIGDN) -> String {
    SHGetNameFromIDList(pidl, kind).map(|p| pwstr(p)).unwrap_or_default()
}

// ---------- painting ----------

// Ground, the sidebar plate and the status line. Pane plates are child windows (paint_pane).
pub(crate) unsafe fn paint_main(h: HWND) {
    let (g, status, font, dpi, hot, accent) = app(|a| {
        let g = geo(a);
        let hot = a.gap_hot.filter(|_| a.drag.is_none()).and_then(|d| gap_line(&g, d, 2 * a.dpi / 96));
        (g, a.status.clone(), a.font, a.dpi, hot.map(|r| (r, a.s.accent_now())), a.s.accent_now())
    });
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().bg);
        if g.side > 0 {
            let side = side_rect(&g);
            fill(dc, side, side_ground());
            if let Some(gap) = side_gap_rect(&g) {
                let (gap_c, line_c) = side_divider_now(accent);
                fill(dc, gap, gap_c);
                // The tree/favourites divider always shows its accent line, not only under the pointer.
                let t = (2 * dpi / 96).max(1);
                let y = gap.top + (gap.bottom - gap.top) / 2;
                fill(dc, RECT { left: gap.left, top: y - t / 2, right: gap.right, bottom: y - t / 2 + t }, line_c);
            }
            frame(dc, side, 1, pal().line);
        }
        if let Some((r, c)) = hot {
            fill(dc, r, c);
        }
        if g.bot < g.h {
            let st = status_rect(&g);
            text(dc, RECT { left: g.pad + 4 * dpi / 96, ..st }, &status, font, pal().sub, DT_LEFT);
        }
    });
}

#[derive(Clone, PartialEq)]
pub(crate) enum Crumb {
    Here,
    Go(String),
    More(Vec<(String, String)>), // omitted middle (or leading) segments
    /// `›`: the folder on its left, and the crumb shown after it (bold in the menu).
    Kids(String, Option<String>),
}

/// The folder icon's square at the front of the address pill. None when the pill is too narrow.
/// The slot depends only on the pill's size, so crumbs and the edit do not shift when an icon arrives.
pub(crate) fn addr_icon(addr: RECT, dpi: i32) -> Option<RECT> {
    let s = |v: i32| v * dpi / 96;
    if addr.right - addr.left < s(90) {
        return None;
    }
    let (left, top) = (addr.left + s(6), addr.top + (addr.bottom - addr.top - s(16)) / 2);
    Some(RECT { left, top, right: left + s(16), bottom: top + s(16) })
}

/// Connecting / unreachable line. The whole address pill: this is not a path, so it does not
/// keep the icon gutter or a breadcrumb segment.
pub(crate) fn net_note_rect(addr: RECT, dpi: i32) -> RECT {
    let pad = 8 * dpi / 96;
    RECT { left: addr.left + pad, right: (addr.right - pad).max(addr.left + pad), ..addr }
}

/// Where crumbs sit in the address pill: the edit's horizontal span, a little inside the pill vertically.
pub(crate) fn crumb_field(addr: RECT, dpi: i32) -> RECT {
    let s = |v: i32| v * dpi / 96;
    let left = addr.left + if addr_icon(addr, dpi).is_some() { s(26) } else { s(8) };
    RECT { left, top: addr.top + s(3), right: (addr.right - s(6)).max(left), bottom: addr.bottom - s(3) }
}

pub(crate) fn addr_rect(h: HWND, dpi: i32) -> RECT {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(h, &mut rc) };
    pane_geo_of(h, rc.right, rc.bottom, dpi).addr
}

struct CrumbMetrics {
    items: Vec<(String, String)>,
    w: Vec<i32>,
    sep: i32,
    ell: i32,
    min: i32,
    fs: bool,
}

/// Segment widths the breadcrumb row lays out with. Same measure `crumb_layout` paints from.
unsafe fn crumb_metrics(dc: HDC, font: HFONT, dpi: i32, parse: &str, disp: &str) -> CrumbMetrics {
    let s = |v: i32| v * dpi / 96;
    let mut items = crumbs(parse);
    let fs = !items.is_empty();
    if items.is_empty() {
        let name = if disp.is_empty() { tr(crate::Key::ThisPc).to_string() } else { disp.to_string() };
        items.push((name, String::new()));
    }
    let old = SelectObject(dc, font.into());
    let width = |t: &str| {
        let w: Vec<u16> = t.encode_utf16().collect();
        let mut sz = SIZE::default();
        _ = GetTextExtentPoint32W(dc, &w, &mut sz);
        sz.cx
    };
    let w: Vec<i32> = items.iter().map(|(n, _)| width(n) + 2 * s(4)).collect();
    let (sep, ell) = (width("›") + 2 * s(2), width("…") + 2 * s(4));
    SelectObject(dc, old);
    CrumbMetrics { items, w, sep, ell, min: s(16), fs }
}

/// Pane `i`'s crumbs in `field`: (rect, label, what a click does).
/// The **leaf** stays at the right (full unless it alone is too wide). Leftover width keeps the
/// drive when possible, folds the middle into `…`, and shows crumbs next to the leaf.
pub(crate) unsafe fn crumb_layout(dc: HDC, font: HFONT, i: usize, field: RECT) -> Vec<(RECT, String, Crumb)> {
    let Some((parse, disp, dpi)) = app(|a| Some((a.s.paths[i].clone(), a.panes[i].as_ref()?.disp.clone(), a.dpi))) else {
        return Vec::new();
    };
    let m = crumb_metrics(dc, font, dpi, &parse, &disp);
    let (sep, ell) = (m.sep, m.ell);
    let avail = field.right - field.left;
    let (items, w) = (m.items, m.w);
    let (pre, suf, mid, last, tail) = crumb_fit_tail(&w, avail, sep, ell, m.min, m.fs);
    let mut out = Vec::new();
    let mut x = field.left;
    let mut put = |wd: i32, label: &str, c: Crumb| {
        out.push((RECT { left: x, right: x + wd, ..field }, label.to_string(), c));
        x += wd;
    };
    let kids = |k: usize| Crumb::Kids(items[k].1.clone(), items.get(k + 1).map(|n| n.0.clone()));
    let n = items.len();

    for k in 0..pre {
        put(w[k], &items[k].0, Crumb::Go(items[k].1.clone()));
        put(sep, "›", kids(k));
    }
    if mid {
        put(ell, "…", Crumb::More(items[pre..suf].iter().cloned().collect()));
        if suf < n {
            let anchor = if pre > 0 { pre - 1 } else if suf > 0 { suf - 1 } else { 0 };
            put(sep, "›", kids(anchor.min(n.saturating_sub(1))));
        }
    }
    for k in suf..n {
        let wd = if k + 1 == n { last } else { w[k] };
        let crumb = if k + 1 == n {
            Crumb::Here
        } else {
            Crumb::Go(items[k].1.clone())
        };
        put(wd, &items[k].0, crumb);
        if k + 1 == n {
            if tail {
                put(sep, "›", kids(k));
            }
        } else {
            put(sep, "›", kids(k));
        }
    }
    out.retain_mut(|(r, ..)| {
        (r.left, r.right) = (r.left.max(field.left), r.right.min(field.right));
        r.right > r.left
    });
    out
}


/// None when the edit is showing or the point misses the pill. Some(None) is empty space on the pill.
pub(crate) unsafe fn crumb_at(h: HWND, i: usize, x: i32, y: i32) -> Option<Option<(usize, RECT, Crumb)>> {
    if GetDlgItem(Some(h), ID_EDIT).is_ok_and(|e| IsWindowVisible(e).as_bool()) {
        return None;
    }
    let (dpi, font) = app(|a| (a.dpi, a.font));
    let inr = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    let addr = addr_rect(h, dpi);
    if !inr(&addr) {
        return None;
    }
    let dc = GetDC(Some(h));
    let items = crumb_layout(dc, font, i, crumb_field(addr, dpi));
    ReleaseDC(Some(h), dc);
    Some(items.into_iter().enumerate().find(|(_, (r, ..))| inr(r)).map(|(k, (r, _, c))| (k, r, c)))
}

/// The address edit shows only while it is focused or its text is not the folder on display.
/// After a share failure it stays up with the path that failed, so the bar does not go empty.
pub(crate) unsafe fn sync_addr(h: HWND) {
    let Ok(e) = GetDlgItem(Some(h), ID_EDIT) else { return };
    let failed = app(|a| {
        if a.crumb_hot.is_some_and(|(w, _)| w == h) {
            a.crumb_hot = None;
        }
        a.panes.iter().flatten().find(|p| p.hwnd == h).and_then(|p| p.net.as_ref()).filter(|n| n.bad).map(|n| n.path.clone())
    });
    if let Some(path) = failed {
        if let Some(keep) = addr_keep(&path, &text_of(e), GetFocus() == e) {
            _ = SetWindowTextW(e, &HSTRING::from(keep));
        }
        if !IsWindowVisible(e).as_bool() {
            _ = ShowWindow(e, SW_SHOWNA);
        }
        _ = InvalidateRect(Some(h), None, false);
        refresh_addr_tip(h);
        return;
    }
    let disp = app(|a| a.panes.iter().flatten().find(|p| p.hwnd == h).map(|p| p.disp.clone()));
    let show = GetFocus() == e || text_of(e) != disp.unwrap_or_default();
    _ = ShowWindow(e, if show { SW_SHOWNA } else { SW_HIDE });
    _ = InvalidateRect(Some(h), None, false);
    refresh_addr_tip(h);
}

pub(crate) unsafe fn crumb_click(h: HWND, i: usize, x: i32, y: i32) {
    let target = match crumb_at(h, i, x, y) {
        None => return,
        Some(Some((_, _, Crumb::Go(p)))) => Some(p),
        Some(Some((_, r, Crumb::More(list)))) => {
            let Ok(menu) = CreatePopupMenu() else { return };
            for (k, (n, _)) in list.iter().enumerate() {
                _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(n.replace('&', "&&")));
            }
            let mut p = POINT { x: r.left, y: r.bottom };
            _ = ClientToScreen(h, &mut p);
            let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, p.x, p.y, None, h, None).0;
            _ = DestroyMenu(menu);
            (pick > 0).then(|| list.get(pick as usize - 1).map(|(_, path)| path.clone())).flatten()
        }
        // The worker lists the folder. The menu opens on WM_KIDS. Focus moves first, so a key
        // pressed meanwhile (which cancels the list) belongs to this pane, and the request
        // records the pane as it is after that.
        Some(Some((_, _, Crumb::Kids(dir, next)))) => {
            kids_cancel();
            focus_view(i);
            let Some((ph, path, dpi)) = app(|a| Some((a.panes[i].as_ref()?.hwnd, a.s.paths[i].clone(), a.dpi))) else { return };
            let id = want_kids(ph, &dir, hidden());
            app(|a| a.kids = Some(KidsAsk { id, hwnd: ph, i, path, gen: gen(), dir, next, at: Instant::now() }));
            if kids_job().net_wait(id) {
                SetTimer(Some(ph), TIMER_KIDS, NET_WAIT_MS, None);
            }
            _ = InvalidateRect(Some(ph), Some(&addr_rect(ph, dpi)), false);
            return;
        }
        Some(_) => {
            _ = focus_ctl(i, ID_EDIT);
            return;
        }
    };
    if target.as_deref().is_some_and(|p| !p.is_empty() && !navigate(i, p)) {
        _ = MessageBeep(MB_ICONWARNING);
    }
    focus_view(i);
}

/// Posted when a subfolder list is ready. wParam is the request id. The names stay in `KIDS`.
pub(crate) const WM_KIDS: u32 = WM_APP + 49;

/// A `›` click waiting for its list. Checked again before the menu opens.
pub(crate) struct KidsAsk {
    pub(crate) id: usize,
    pub(crate) hwnd: HWND,
    pub(crate) i: usize,
    pub(crate) path: String, // the pane's folder when the click happened
    pub(crate) gen: usize,
    pub(crate) dir: String, // folder being listed
    pub(crate) next: Option<String>,
    pub(crate) at: Instant, // when the click was accepted; a share list is given up after NET_WAIT_MS
}

impl KidsAsk {
    /// A `WM_KIDS` from an older request, or another pane, leaves this one alone.
    pub(crate) fn owns(&self, id: usize, h: HWND) -> bool {
        self.id == id && self.hwnd == h
    }
}

pub(crate) type KidsList = std::result::Result<(Vec<String>, bool), ()>; // (names, cut short), Err = unreadable

/// One worker, newest request only. `id` moves on every ask or cancel. A result is kept
/// only for the current id, and handed out only to that id.
/// A UNC or mapped drive is not read here. `read_dir` on a dead share would block this
/// thread, and the next local `›` would wait behind it. Those lists run beside the worker
/// and are given up after `NET_WAIT_MS`.
pub(crate) struct KidsJob {
    pub(crate) id: usize,
    pub(crate) hwnd: isize,
    pub(crate) dir: String,
    pub(crate) hidden: bool,
    pub(crate) net: bool,
    pub(crate) result: Option<(usize, KidsList)>,
}

impl KidsJob {
    pub(crate) fn ask(&mut self, hwnd: isize, dir: &str, hidden: bool, net: bool) -> usize {
        (self.id, self.hwnd, self.dir, self.hidden, self.net, self.result) = (self.id + 1, hwnd, dir.to_string(), hidden, net, None);
        self.id
    }

    pub(crate) fn publish(&mut self, id: usize, r: KidsList) -> bool {
        if self.id != id {
            return false;
        }
        self.result = Some((id, r));
        true
    }

    pub(crate) fn take(&mut self, id: usize) -> Option<KidsList> {
        self.result.take_if(|(k, _)| *k == id).map(|(_, r)| r)
    }

    /// This id is still the share list the worker handed off, and nothing has arrived.
    pub(crate) fn net_wait(&self, id: usize) -> bool {
        self.id == id && self.net && self.result.is_none()
    }

    /// The wait is over. A result that already arrived is left for `take`. Later reads are dropped.
    pub(crate) fn fail(&mut self, id: usize) -> bool {
        if self.id != id || self.result.is_some() {
            return false;
        }
        self.id += 1;
        self.dir.clear();
        self.net = false;
        self.result = Some((id, Err(())));
        true
    }

    pub(crate) fn cancel(&mut self) {
        (self.id, self.result) = (self.id + 1, None);
        self.dir.clear();
        self.net = false;
    }
}

pub(crate) static KIDS: Mutex<KidsJob> = Mutex::new(KidsJob { id: 0, hwnd: 0, dir: String::new(), hidden: false, net: false, result: None });
pub(crate) static KIDS_WAKE: Condvar = Condvar::new();
pub(crate) const KIDS_MAX: usize = 500;
const KIDS_NET_CAP: usize = 2;
static KIDS_NET: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn kids_job() -> std::sync::MutexGuard<'static, KidsJob> {
    KIDS.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn want_kids(h: HWND, dir: &str, hidden: bool) -> usize {
    let net = unsafe { share_path(dir) };
    let id = kids_job().ask(h.0 as isize, dir, hidden, net);
    static WORKER: Once = Once::new();
    WORKER.call_once(|| _ = std::thread::Builder::new().name("maguro-kids".into()).spawn(kids_worker));
    KIDS_WAKE.notify_one();
    id
}

fn read_kids(dir: &str, hidden: bool) -> KidsList {
    use std::os::windows::fs::MetadataExt;
    std::fs::read_dir(dir)
        .map(|rd| {
            let named = rd.flatten().filter_map(|e| Some((e.file_name().to_string_lossy().into_owned(), e.metadata().ok()?.file_attributes())));
            kid_names(named, hidden)
        })
        .map_err(|_| ())
}

/// A share list does not occupy the worker. At most `KIDS_NET_CAP` of them run at once;
/// past that the click fails immediately instead of queueing behind a dead share.
fn spawn_kids_net(id: usize, hwnd: isize, dir: String, hidden: bool) -> bool {
    if KIDS_NET.fetch_add(1, Ordering::Relaxed) >= KIDS_NET_CAP {
        KIDS_NET.fetch_sub(1, Ordering::Relaxed);
        return false;
    }
    let spawned = std::thread::Builder::new().name("maguro-kids-net".into()).spawn(move || {
        let current = kids_job().id == id;
        let r = if current { read_kids(&dir, hidden) } else { Err(()) };
        if current && kids_job().publish(id, r) {
            unsafe { _ = PostMessageW(Some(HWND(hwnd as _)), WM_KIDS, WPARAM(id), LPARAM(0)) };
        }
        KIDS_NET.fetch_sub(1, Ordering::Relaxed);
    });
    if spawned.is_err() {
        KIDS_NET.fetch_sub(1, Ordering::Relaxed);
        return false;
    }
    true
}

/// Lives until the process ends. The lock is never held across the directory read.
pub(crate) fn kids_worker() {
    let mut seen = 0usize;
    loop {
        let (id, hwnd, dir, hidden, net) = {
            let mut j = kids_job();
            while j.id == seen || j.dir.is_empty() {
                j = KIDS_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
            (j.id, j.hwnd, j.dir.clone(), j.hidden, j.net)
        };
        seen = id;
        if net {
            if !spawn_kids_net(id, hwnd, dir, hidden) && kids_job().publish(id, Err(())) {
                unsafe { _ = PostMessageW(Some(HWND(hwnd as _)), WM_KIDS, WPARAM(id), LPARAM(0)) };
            }
            continue;
        }
        let r = read_kids(&dir, hidden);
        if kids_job().publish(id, r) {
            unsafe { _ = PostMessageW(Some(HWND(hwnd as _)), WM_KIDS, WPARAM(id), LPARAM(0)) };
        }
    }
}

/// The share list did not come back. A newer click, or a result already in, is left alone.
pub(crate) unsafe fn kids_timeout(h: HWND) {
    _ = KillTimer(Some(h), TIMER_KIDS);
    let Some((id, at)) = app(|a| a.kids.as_ref().filter(|k| k.hwnd == h).map(|k| (k.id, k.at))) else { return };
    if !kids_job().net_wait(id) {
        return;
    }
    let age = at.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    if age < NET_WAIT_MS {
        SetTimer(Some(h), TIMER_KIDS, (NET_WAIT_MS - age).max(1), None);
        return;
    }
    if kids_job().fail(id) {
        kids_show(h, id);
    }
}

/// Subfolders from `(name, attributes)`. Hidden ones only when the shell shows them.
/// Hidden and system together are never listed, the same as Explorer. Order is Explorer's
/// (`2` before `10`). At most `KIDS_MAX`. Returns `(names, cut short)`.
pub(crate) fn kid_names(all: impl Iterator<Item = (String, u32)>, show_hidden: bool) -> (Vec<String>, bool) {
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
    let (dir, hid, hs) = (FILE_ATTRIBUTE_DIRECTORY.0, FILE_ATTRIBUTE_HIDDEN.0, FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0);
    let mut v: Vec<HSTRING> = all
        .filter(|&(_, a)| a & dir != 0 && a & hs != hs && (show_hidden || a & hid == 0))
        .map(|(n, _)| HSTRING::from(n))
        .collect();
    v.sort_by(|a, b| unsafe { StrCmpLogicalW(a, b) }.cmp(&0));
    let cut = v.len() > KIDS_MAX;
    (v.iter().take(KIDS_MAX).map(|n| n.to_string_lossy()).collect(), cut)
}

/// Where the `›` of `dir` sits in a fresh layout. None once that chevron has folded away.
pub(crate) fn kids_anchor(items: &[(RECT, String, Crumb)], dir: &str) -> Option<RECT> {
    items.iter().find(|(_, _, c)| matches!(c, Crumb::Kids(d, _) if d == dir)).map(|(r, ..)| *r)
}

/// A click, a key, a resize, or a DPI change drops a waiting `›` list.
pub(crate) unsafe fn kids_cancel() {
    let Some((h, dpi)) = app(|a| a.kids.take().map(|k| (k.hwnd, a.dpi))) else { return };
    _ = KillTimer(Some(h), TIMER_KIDS);
    kids_job().cancel();
    _ = InvalidateRect(Some(h), Some(&addr_rect(h, dpi)), false);
}

/// `WM_KIDS`: the list is in. Shown only while it is still this request, the pane still shows
/// the same folder, nothing is being resized, Maguro is in front, and the `›` is still laid out
/// where it is now.
pub(crate) unsafe fn kids_show(h: HWND, id: usize) {
    _ = KillTimer(Some(h), TIMER_KIDS);
    if !app(|a| a.kids.as_ref().is_some_and(|k| k.owns(id, h))) {
        return;
    }
    let Some(list) = kids_job().take(id) else { return };
    let Some(k) = app(|a| a.kids.take()) else { return };
    let (dpi, font, main) = app(|a| (a.dpi, a.font, a.main));
    let addr = addr_rect(h, dpi);
    _ = InvalidateRect(Some(h), Some(&addr), false);
    let same = app(|a| a.panes[k.i].as_ref().is_some_and(|p| p.hwnd == h) && a.s.paths[k.i] == k.path && !a.sizing);
    if !same || gen() != k.gen || GetForegroundWindow() != main || GetDlgItem(Some(h), ID_EDIT).is_ok_and(|e| IsWindowVisible(e).as_bool()) {
        return;
    }
    let dc = GetDC(Some(h));
    let items = crumb_layout(dc, font, k.i, crumb_field(addr, dpi));
    ReleaseDC(Some(h), dc);
    let Some(r) = kids_anchor(&items, &k.dir) else { return };
    let Ok(menu) = CreatePopupMenu() else { return };
    let gray = |t: &str| _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(t));
    let names = match &list {
        Ok((names, cut)) => {
            for (n, name) in names.iter().enumerate() {
                _ = AppendMenuW(menu, MF_STRING, n + 1, &HSTRING::from(name.replace('&', "&&")));
                if k.next.as_ref().is_some_and(|x| x.eq_ignore_ascii_case(name)) {
                    _ = SetMenuDefaultItem(menu, n as u32 + 1, 0); // the folder this pane is in: bold
                }
            }
            if *cut {
                gray("…");
            } else if names.is_empty() {
                gray(tr(crate::Key::NoSubfolders));
            }
            names.as_slice()
        }
        Err(()) => {
            gray(tr(crate::Key::CanTOpenThisFolder));
            &[]
        }
    };
    let mut p = POINT { x: r.left, y: r.bottom };
    _ = ClientToScreen(h, &mut p);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, p.x, p.y, None, h, None).0;
    _ = DestroyMenu(menu);
    if let Some(name) = names.get((pick as usize).wrapping_sub(1)) {
        if !navigate(k.i, &format!("{}\\{name}", k.dir.trim_end_matches('\\'))) {
            _ = MessageBeep(MB_ICONWARNING);
        }
    }
    focus_view(k.i);
}

/// Tracking tooltip for one pane's address. Same id space as the tab tips (lock 7, paths 8).
const ADDR_TIP_ID: usize = 9;
const EM_GETRECT: u32 = 0xB2;

unsafe fn cursor_on_addr(h: HWND) -> bool {
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() {
        return false;
    }
    _ = ScreenToClient(h, &mut pt);
    let dpi = app(|a| a.dpi);
    let r = addr_rect(h, dpi);
    pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
}

unsafe fn text_px(dc: HDC, font: HFONT, text: &str) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let old = SelectObject(dc, font.into());
    let w: Vec<u16> = text.encode_utf16().collect();
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    SelectObject(dc, old);
    sz.cx
}

/// The edit is showing this pane's place, not a path the user is still typing.
fn edit_shows_place(shown: &str, parse: &str, disp: &str, failed: Option<&str>) -> bool {
    if shown.is_empty() {
        return false;
    }
    shown == parse || (!disp.is_empty() && shown == disp) || failed.is_some_and(|p| !p.is_empty() && shown == p)
}

/// Full address tooltip, or None when the bar already shows the whole path.
/// Connecting is a status line, not a path. The edit, while it is up, is the path display.
unsafe fn addr_tip_body(h: HWND) -> Option<String> {
    let i = pane_of(h)?;
    let (parse, disp, failed, connecting) = app(|a| {
        let p = a.panes.get(i)?.as_ref()?;
        if p.hwnd != h {
            return None;
        }
        let failed = p.net.as_ref().filter(|n| n.bad).map(|n| n.path.clone());
        let connecting = p.net.as_ref().is_some_and(|n| !n.bad);
        Some((a.s.paths.get(i).cloned().unwrap_or_default(), p.disp.clone(), failed, connecting))
    })?;
    if connecting {
        return None;
    }
    let tip = match failed.as_deref() {
        Some(path) => addr_tip_text(path, ""),
        None => addr_tip_text(&parse, &disp),
    };
    if tip.is_empty() {
        return None;
    }
    let (dpi, font) = app(|a| (a.dpi, a.font));
    let edit = GetDlgItem(Some(h), ID_EDIT).unwrap_or_default();
    if !edit.0.is_null() && IsWindowVisible(edit).as_bool() {
        let shown = text_of(edit);
        if !edit_shows_place(&shown, &parse, &disp, failed.as_deref()) {
            return None;
        }
        let dc = GetDC(Some(edit));
        let face = HFONT(SendMessageW(edit, WM_GETFONT, None, None).0 as _);
        let px = text_px(dc, if face.0.is_null() { font } else { face }, &shown);
        _ = ReleaseDC(Some(edit), dc);
        let mut box_r = RECT::default();
        SendMessageW(edit, EM_GETRECT, None, Some(LPARAM(&mut box_r as *mut RECT as isize)));
        return addr_line_truncated(px, box_r.right - box_r.left).then_some(tip);
    }
    let field = crumb_field(addr_rect(h, dpi), dpi);
    let dc = GetDC(Some(h));
    let m = crumb_metrics(dc, font, dpi, &parse, &disp);
    _ = ReleaseDC(Some(h), dc);
    addr_path_truncated(&m.w, field.right - field.left, m.sep, m.ell, m.min, m.fs).then_some(tip)
}

/// A visible address tip hides when the pointer is no longer on that pane's address pill.
pub(crate) fn addr_tip_hides(shown: bool, on_addr: bool) -> bool {
    shown && !on_addr
}

/// Hide the address tooltip. The tool stays installed for the next hover.
/// Hiding sends `TTM_POP` as well as deactivating — same as #175 / #189.
pub(crate) unsafe fn hide_addr_tip() {
    let (main, added, shown) = app(|a| {
        let shown = std::mem::replace(&mut a.addr_tip_shown, false);
        a.addr_tip_pane = HWND::default();
        (a.main, a.addr_tip, shown)
    });
    if !shown && !added {
        return;
    }
    if shown {
        let (now, _) = crate::ui::tip_leave(main, ADDR_TIP_ID, added);
        app(|a| a.addr_tip = now);
    }
}

/// `over` is the pointer on pane `h`'s address pill. A drag hides it. The text is the full
/// path, and only while that path does not fit. Anchored just below the address rect + 4px.
pub(crate) unsafe fn addr_tip_at(h: HWND, over: bool) {
    let (tip, main) = app(|a| (a.tip, a.main));
    if tip.0.is_null() || main.0.is_null() || h.0.is_null() {
        return;
    }
    // Text selection / drag in the address edit: never measure or re-track the tip.
    if GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0 || GetAsyncKeyState(VK_RBUTTON.0 as i32) < 0 {
        return;
    }
    let over = over && app(|a| a.pane_drag.is_none() && a.pane_press.is_none());
    let text = if over { addr_tip_body(h) } else { None };
    let Some(text) = text else {
        let (pane, shown) = app(|a| (a.addr_tip_pane, a.addr_tip_shown));
        if shown || pane == h || (over && !pane.0.is_null()) {
            hide_addr_tip();
        }
        return;
    };
    let dpi = app(|a| a.dpi);
    let item = crate::ui::tip_screen_rect(h, addr_rect(h, dpi));
    let key = 0x9000_0000_0000_0000u64 ^ (h.0 as usize as u64);
    let added = app(|a| a.addr_tip);
    let (now, shown) = crate::ui::tip_enter(
        main,
        ADDR_TIP_ID,
        key,
        &text,
        item,
        crate::ui::TipSide::Below,
        added,
    );
    app(|a| {
        a.addr_tip = now;
        a.addr_tip_pane = h;
        a.addr_tip_shown = shown;
    });
    // The folder list is a child; leaving the address into it may not deliver WM_MOUSELEAVE.
    let mut t = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: h,
        dwHoverTime: 0,
    };
    _ = TrackMouseEvent(&mut t);
}

/// Navigation and resize: keep the tip in step with the path now on screen.
pub(crate) unsafe fn refresh_addr_tip(h: HWND) {
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        if app(|a| a.addr_tip_pane == h || a.addr_tip_shown) {
            hide_addr_tip();
        }
        return;
    }
    addr_tip_at(h, cursor_on_addr(h));
}

/// Mouse moved over `hwnd`. The address tip hides once the pointer is off that pane's
/// address pill, including when it lands on the tip window or the folder list (#175 pattern).
pub(crate) unsafe fn hide_addr_tip_if_pointer_left(hwnd: HWND) {
    let (shown, tip, pane) = app(|a| (a.addr_tip_shown, a.tip, a.addr_tip_pane));
    if !shown {
        return;
    }
    let on_addr = !pane.0.is_null() && hwnd != tip && cursor_on_addr(pane);
    if addr_tip_hides(shown, on_addr) {
        hide_addr_tip();
    }
}

pub(crate) unsafe fn crumb_hover(h: HWND, at: Option<usize>) {
    let (changed, became) = app(|a| {
        let next = at.map(|k| (h, k));
        let old = a.crumb_hot;
        if old == next {
            return (false, false);
        }
        a.crumb_hot = next;
        (true, next.is_some() && old.is_none_or(|(w, _)| w != h))
    });
    if !changed {
        return;
    }
    let dpi = app(|a| a.dpi);
    _ = InvalidateRect(Some(h), Some(&addr_rect(h, dpi)), false);
    if became {
        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
        _ = TrackMouseEvent(&mut t);
    }
}

// One white plate per pane: a single accent border when active, flat fields in the top bar.
pub(crate) unsafe fn paint_pane(h: HWND) {
    let (active, i, dpi, icon_font, small_font, font, accent, hot, tint, folder, waiting, link) = app(|a| {
        let i = a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h));
        // a drop target-to-be looks active too
        let active = a.panes[a.s.active].as_ref().is_some_and(|p| p.hwnd == h) || a.drop_hover == h;
        let hot = a.crumb_hot.filter(|(w, _)| *w == h).map(|(_, k)| k);
        let tint = i.and_then(|i| tint_color(a.s.tints.get(i).copied().unwrap_or(0)));
        let link = i.and_then(|i| a.panes[i].as_ref()).and_then(|p| p.net.clone());
        // While a share is unresolved the previous folder's icon stays out of the pill.
        let folder = if link.is_some() { -1 } else { i.and_then(|i| a.panes[i].as_ref()).map_or(-1, |p| p.icon) };
        let waiting = a.kids.as_ref().filter(|k| k.hwnd == h).map(|k| k.dir.clone());
        (active, i, a.dpi, a.icon_font, a.small_font, a.font, a.s.accent_now(), hot, tint, folder, waiting, link)
    });
    let ilist = match app(|a| a.ilist.clone()) {
        None if folder >= 0 => {
            let l = SHGetImageList::<IImageList>(SHIL_SMALL as i32).ok();
            app(|a| a.ilist = l.clone());
            l
        }
        l => l,
    };
    let filtered = i.is_some_and(|i| FILTERS.lock().is_ok_and(|f| !f[i].is_empty()));
    let chrome = header_chrome(h);
    let (head, ink) = (chrome.head, chrome.ink);
    let focus = GetFocus();
    let focused = |id: i32| GetDlgItem(Some(h), id).is_ok_and(|e| e == focus);
    let (addr_on, filt_on) = (focused(ID_EDIT), focused(ID_FILTER));
    let crumbs_on = GetDlgItem(Some(h), ID_EDIT).is_ok_and(|e| !IsWindowVisible(e).as_bool());
    let file_hot = i.and_then(|pane| crate::pane_file_hot(pane, h));
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let pg = pane_geo_of(h, rc.right, rc.bottom, dpi);
        fill(dc, rc, pal().card);
        // Header and border. A colour label fills both, the same areas the accent uses on an active pane.
        fill(dc, RECT { bottom: pg.view.top, ..rc }, head);
        let edge = app(|a| a.s.edge);
        if active && edge {
            frame(dc, rc, s(2), chrome.border);
        } else {
            frame(dc, rc, 1, chrome.border);
            fill(dc, RECT { left: pg.view.left, top: pg.view.top - 1, right: pg.view.right, bottom: pg.view.top }, pal().line);
        }
        if let Some(i) = i.filter(|_| pg.num.right > pg.num.left) {
            // On the header fill. A label colours the whole bar, so the number has no chip of its own.
            let num_ink = if tint.is_some() || active { ink } else { pal().sub };
            text(dc, pg.num, &(i + 1).to_string(), small_font, num_ink, DT_CENTER);
        }
        for (p, on) in [(pg.addr, addr_on), (pg.filt, filt_on)] {
            fill(dc, p, pal().pill);
            if on {
                frame(dc, p, 1, if active { ink } else { accent });
            }
        }
        if link.is_none() {
            if let (Some(il), Some(r), true) = (&ilist, addr_icon(pg.addr, dpi), folder >= 0) {
                if let Ok(icon) = il.GetIcon(folder, ILD_NORMAL.0) {
                    _ = DrawIconEx(dc, r.left, r.top, icon, r.right - r.left, r.bottom - r.top, 0, None, DI_NORMAL);
                    _ = DestroyIcon(icon);
                }
            }
        }
        // Connecting replaces the crumbs. A failure keeps the path in the edit, so it is not painted here.
        if link.as_ref().is_some_and(|n| !n.bad) && crumbs_on {
            text(dc, net_note_rect(pg.addr, dpi), net_bar_text(false), font, pal().text, DT_LEFT);
        } else if let Some(i) = i.filter(|_| crumbs_on) {
            for (k, (r, label, c)) in crumb_layout(dc, font, i, crumb_field(pg.addr, dpi)).into_iter().enumerate() {
                let wait = matches!(&c, Crumb::Kids(d, _) if waiting.as_ref() == Some(d));
                if wait {
                    fill(dc, r, blend(pal().pill, accent, 0.25));
                } else if hot == Some(k) {
                    fill(dc, r, blend(pal().pill, pal().text, 0.08));
                }
                if matches!(c, Crumb::Kids(..)) {
                    if wait {
                        // `…` is a little wider than the `›` slot, so it reaches into the neighbours' padding.
                        text(dc, r, "…", font, pal().sub, DT_CENTER | DT_NOCLIP);
                    } else {
                        text(dc, r, &label, font, pal().sub, DT_CENTER);
                    }
                    continue;
                }
                if matches!(c, Crumb::More(_)) {
                    text(dc, r, &label, font, pal().sub, DT_CENTER);
                    continue;
                }
                let ink = if c == Crumb::Here { pal().text } else { pal().sub };
                text(dc, RECT { left: r.left + s(4), right: r.right - s(4), ..r }, &label, font, ink, DT_LEFT);
            }
        }
        if pg.tabs.bottom > pg.tabs.top {
            if let Some(i) = i {
                let (list, cur) = app(|a| {
                    let list = a.s.pane_tab_list(i);
                    let cur = if list.len() > 1 { a.s.pane_cur.get(i).copied().unwrap_or(0).min(list.len() - 1) } else { 0 };
                    (list, cur)
                });
                paint_pane_tabs(dc, &pg, &list, cur, dpi, font, accent, file_hot);
            }
        }
        if pg.filt.right - pg.filt.left < s(24) {
            // Too narrow for the icon slot. Drawing it anyway would cover the pane number.
        } else if filtered {
            // filter applied: accent chip left of the edit (which starts at filt.left + s(24))
            let chip =
                RECT { left: pg.filt.left + s(3), top: pg.filt.top + s(3), right: pg.filt.left + s(21), bottom: pg.filt.bottom - s(3) };
            fill(dc, chip, accent);
            text(dc, chip, ICON_FILTER, icon_font, on_color(accent), DT_CENTER);
        } else {
            let icon = RECT { left: pg.filt.left + s(4), right: pg.filt.left + s(22), ..pg.filt };
            text(dc, icon, ICON_SEARCH, icon_font, pal().sub, DT_CENTER);
        }
    });
}

#[cfg(test)]
mod kids_tests {
    use super::*;
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};

    #[test]
    fn kid_name_rules() {
        let (d, h, sy) = (FILE_ATTRIBUTE_DIRECTORY.0, FILE_ATTRIBUTE_HIDDEN.0, FILE_ATTRIBUTE_SYSTEM.0);
        let all = || [("10", d), ("2", d), ("file.txt", FILE_ATTRIBUTE_ARCHIVE.0), ("Secret", d | h), ("Config.Msi", d | h | sy), ("a", d | sy)];
        let names = |show| kid_names(all().into_iter().map(|(n, a)| (n.to_string(), a)), show);
        assert_eq!(names(false), (vec!["2".to_string(), "10".into(), "a".into()], false));
        assert_eq!(names(true), (vec!["2".to_string(), "10".into(), "a".into(), "Secret".into()], false));
        let (v, cut) = kid_names((0..501).map(|k| (format!("f{k}"), d)), false);
        assert_eq!((v.len(), cut, v[0].as_str(), v[499].as_str()), (500, true, "f0", "f499"));
        assert!(!kid_names((0..500).map(|k| (format!("f{k}"), d)), false).1);
    }

    #[test]
    fn kids_late_results_dropped() {
        let mut j = KidsJob { id: 0, hwnd: 0, dir: String::new(), hidden: false, net: false, result: None };
        let a = j.ask(1, r"C:\Users", false, false);
        assert!(j.publish(a, Ok((vec!["A".into()], false))));
        let b = j.ask(1, r"C:\", false, false);
        assert!(!j.publish(a, Ok((vec!["A".into()], false))));
        assert!(j.take(a).is_none() && j.take(b).is_none());
        assert!(j.publish(b, Err(())));
        assert!(j.take(b).is_some() && j.take(b).is_none());
        let w = HWND(1 as _);
        let ask = KidsAsk { id: b, hwnd: w, i: 0, path: r"C:\Users\Public".into(), gen: 0, dir: r"C:\".into(), next: None, at: Instant::now() };
        assert!(!ask.owns(a, w) && ask.owns(b, w) && !ask.owns(b, HWND(2 as _)));
        j.cancel();
        assert!(!j.publish(b, Err(())) && j.dir.is_empty());
        let net = j.ask(1, r"\\dead\share", false, true);
        assert!(j.net_wait(net) && j.fail(net) && !j.net_wait(net));
        assert!(!j.publish(net, Ok((vec!["A".into()], false))));
        assert!(matches!(j.take(net), Some(Err(()))));
        let kept = j.ask(1, r"C:\", false, false);
        assert!(j.publish(kept, Ok((vec!["A".into()], false))));
        assert!(!j.fail(kept));
        assert!(matches!(j.take(kept), Some(Ok(_))));
    }

    #[test]
    fn address_tip_hides_once_the_pointer_leaves_the_pill() {
        assert!(addr_tip_hides(true, false));
        assert!(!addr_tip_hides(true, true));
        assert!(!addr_tip_hides(false, false));
        assert!(!addr_tip_hides(false, true));
    }

    #[test]
    fn kids_anchor_follows_layout() {
        let r = |l| RECT { left: l, top: 0, right: l + 10, bottom: 20 };
        let kids = |d: &str| Crumb::Kids(d.into(), None);
        let wide = vec![(r(0), "C:".into(), Crumb::Go(r"C:\".into())), (r(10), "›".into(), kids(r"C:\")), (r(20), "Users".into(), Crumb::Here)];
        assert_eq!(kids_anchor(&wide, r"C:\"), Some(r(10)));
        let folded = vec![(r(0), "…".into(), Crumb::More(Vec::new())), (r(30), "›".into(), kids(r"C:\Users")), (r(40), "Users".into(), Crumb::Here)];
        assert_eq!(kids_anchor(&folded, r"C:\"), None);
    }
}

#[cfg(test)]
mod ink_tests {
    use super::{ink_kind, ink_line_top, InkKind};
    use crate::LineMetrics;

    fn m(ascent: i32, descent: i32, internal_leading: i32) -> LineMetrics {
        LineMetrics { ascent, descent, internal_leading }
    }

    /// 96 DPI text metrics from the chip/tab investigation.
    fn malgun11() -> LineMetrics {
        m(10, 3, 2)
    }
    fn yu11() -> LineMetrics {
        m(11, 2, 2)
    }
    fn segoe11() -> LineMetrics {
        m(11, 2, 2)
    }
    fn face12() -> LineMetrics {
        m(12, 3, 3)
    }

    #[test]
    fn script_picks_the_glyph_box() {
        assert_eq!(ink_kind("2026 보고서"), InkKind::Cjk);
        assert_eq!(ink_kind("보고서"), InkKind::Cjk);
        assert_eq!(ink_kind("レポート"), InkKind::Cjk);
        assert_eq!(ink_kind("報告"), InkKind::Cjk);
        assert_eq!(ink_kind("File 파일"), InkKind::Cjk);
        assert_eq!(ink_kind("Reports"), InkKind::Latin);
        assert_eq!(ink_kind("Maguro"), InkKind::Latin);
        assert_eq!(ink_kind("2026"), InkKind::Latin);
        assert_eq!(ink_kind("…"), InkKind::Em);
        assert_eq!(ink_kind("+"), InkKind::Em);
        assert_eq!(ink_kind("×"), InkKind::Em);
        assert_eq!(ink_kind("\u{E73E}"), InkKind::Em);
        assert_eq!(ink_kind(""), InkKind::Em);
    }

    /// Gaps above and below a tmHeight ink block placed by `ink_line_top`.
    fn gaps(cell: i32, metrics: LineMetrics, kind: InkKind) -> (i32, i32) {
        let top = ink_line_top(0, cell, metrics, kind);
        let bottom = cell - metrics.height() - top;
        (top, bottom)
    }

    #[test]
    fn chip_shift_follows_the_measured_faces() {
        // 16px chip at 96 DPI. Positive would move the glyphs down from DT_VCENTER.
        // Malgun/Yu CJK ink is only the odd leftover pixel high there, so the CJK
        // step rounds to 0. Segoe caps are still about 1px low and move up.
        let shift = |metrics: LineMetrics, kind| ink_line_top(0, 16, metrics, kind) - (16 - metrics.height()) / 2;
        assert_eq!(shift(malgun11(), InkKind::Cjk), 0);
        assert_eq!(shift(yu11(), InkKind::Cjk), 0);
        assert_eq!(shift(segoe11(), InkKind::Latin), -1);
        assert_eq!(shift(malgun11(), InkKind::Latin), -1);
        assert_eq!(shift(segoe11(), InkKind::Em), 0);
        // Korean on the Segoe metrics no longer takes the full descent step.
        assert_eq!(shift(segoe11(), InkKind::Cjk), 0);
        assert_ne!(shift(segoe11(), InkKind::Cjk), shift(segoe11(), InkKind::Latin));
    }

    #[test]
    fn malgun_chip_stays_within_half_a_pixel() {
        // PC measurement, 100% Malgun Gothic, 16 DIP chip. Before the descent/2
        // shift the ink was top 1 / bottom 2 (0.5px high). After, top 2 / bottom 1
        // (0.5px low). Half of that 1px step rounds back toward the center.
        let (top, bottom) = gaps(16, malgun11(), InkKind::Cjk);
        assert_eq!((top, bottom), (1, 2));
        assert!(bottom.abs_diff(top) <= 1);
        assert_ne!((top, bottom), (2, 1));
        let (yu_top, yu_bottom) = gaps(16, yu11(), InkKind::Cjk);
        assert_eq!((yu_top, yu_bottom), (1, 2));
    }

    #[test]
    fn centered_button_keeps_equal_gaps() {
        // PC measurement, 100% Malgun Gothic. A 19px ink box with 11px ink was
        // top 4 / bottom 4, then the 1px descent shift made it top 5 / bottom 3.
        let button = m(8, 3, 2);
        assert_eq!(button.height(), 11);
        assert_eq!(gaps(19, button, InkKind::Cjk), (4, 4));
        // The same face, already centered: Malgun 11 (tmHeight 13) in slack 8.
        assert_eq!(gaps(21, malgun11(), InkKind::Cjk), (4, 4));
        // 12px row (tmHeight 15) with an even split.
        assert_eq!(gaps(23, face12(), InkKind::Cjk), (4, 4));
    }

    #[test]
    fn twelve_px_rows_keep_cjk_on_the_metric_center() {
        let shift = |kind| ink_line_top(0, 24, face12(), kind) - (24 - face12().height()) / 2;
        assert_eq!(shift(InkKind::Cjk), 0);
        assert_eq!(shift(InkKind::Latin), -1);
        assert_eq!(shift(InkKind::Em), 0);
        // 40 + (16 - 13) / 2, with no CJK step.
        assert_eq!(ink_line_top(40, 16, malgun11(), InkKind::Cjk), 41);
        assert_eq!(ink_line_top(40, 16, segoe11(), InkKind::Latin), 40);
    }

    #[test]
    fn offset_scales_with_the_metrics_at_higher_dpi() {
        // 16 DIP chip. The font is created at lfHeight = -(px * dpi / 96), so the
        // cached TEXTMETRIC grows with DPI. Latin still moves by internalLeading/2.
        // The old CJK shift was descent/2 (1, 2, 2, 3 at these rows). Half of that
        // is 0 or 1, and 1px cannot beat a gap error of at most 1, so CJK stays on
        // the DT_VCENTER center: within 0.5px when the slack is odd, equal when even.
        let at_100 = m(10, 3, 2);
        let at_125 = m(13, 4, 3);
        let at_150 = m(15, 5, 3);
        let at_200 = m(20, 6, 4);
        let cjk = |cell: i32, metrics: LineMetrics| ink_line_top(0, cell, metrics, InkKind::Cjk) - (cell - metrics.height()) / 2;
        let latin = |cell: i32, metrics: LineMetrics| ink_line_top(0, cell, metrics, InkKind::Latin) - (cell - metrics.height()) / 2;
        assert_eq!((cjk(16, at_100), latin(16, at_100)), (0, -1));
        assert_eq!((cjk(20, at_125), latin(20, at_125)), (0, -1));
        assert_eq!((cjk(24, at_150), latin(24, at_150)), (0, -1));
        assert_eq!((cjk(32, at_200), latin(32, at_200)), (0, -2));
        assert_eq!(gaps(16, at_100, InkKind::Cjk), (1, 2));
        assert_eq!(gaps(20, at_125, InkKind::Cjk), (1, 2));
        assert_eq!(gaps(24, at_150, InkKind::Cjk), (2, 2));
        assert_eq!(gaps(32, at_200, InkKind::Cjk), (3, 3));
        assert_ne!(latin(16, at_100), latin(32, at_200));
    }
}

