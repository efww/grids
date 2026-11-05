//! Window ground, card, field, and section title. Same tokens as the buttons.
//!
//! [`paint_section_header`] is the name prompt's label, and the hook for later batches
//! (favourites group titles, confirm headings).
//!
//! A text box is one 1px border: the token line at rest, cerulean when focused.
//! There is no second ring outside the field.

use super::tokens::*;
use crate::imports::*;

struct CachedBrush {
    rgb: u32,
    handle: isize,
}

static FIELD_BRUSH: Mutex<Option<CachedBrush>> = Mutex::new(None);

pub(crate) unsafe fn paint_window(dc: HDC, r: RECT) {
    crate::fill(dc, r, ui_tokens().bg);
}

/// Card plate: token fill, hairline, radius 2. Corners blend into the window ground.
pub(crate) unsafe fn paint_card(dc: HDC, r: RECT, dpi: i32) {
    paint_card_fill(dc, r, dpi, ui_tokens().card);
}

/// Same plate as [`paint_card`] with a caller fill. Hover passes the selection mist.
/// Hairline and radius stay on the shared tokens. Corners blend into the window ground.
pub(crate) unsafe fn paint_card_fill(dc: HDC, r: RECT, dpi: i32, fill: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let t = ui_tokens();
    crate::aa_rfill(dc, r, px(dpi, RADIUS).max(1), fill, Some(t.line), t.bg);
}

/// The one border colour of a field. Focused is cerulean. Rest is the token line.
/// Callers paint this on `r` itself. Nothing is drawn outside `r`.
pub(crate) fn field_border(focused: bool, dark: bool) -> u32 {
    if focused { ACCENT } else { tokens_for(dark).line }
}

/// How a field is painted. `outset` is always 0: a focus state does not grow a second ring.
pub(crate) struct FieldChrome {
    pub border: u32,
    pub outset: i32,
}

pub(crate) fn field_chrome(focused: bool, dark: bool) -> FieldChrome {
    FieldChrome { border: field_border(focused, dark), outset: 0 }
}

/// Single-line field. One 1px border on `r`: token line at rest, cerulean when focused.
/// Corners blend into the window ground.
pub(crate) unsafe fn paint_field(dc: HDC, r: RECT, dpi: i32, focused: bool) {
    paint_field_on(dc, r, dpi, focused, ui_tokens().bg);
}

/// Same field as [`paint_field`], blending corners into `ground` (a card plate, not the window).
pub(crate) unsafe fn paint_field_on(dc: HDC, r: RECT, dpi: i32, focused: bool, ground: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let t = ui_tokens();
    let chrome = field_chrome(focused, t.dark);
    let rad = px(dpi, RADIUS).max(1);
    crate::aa_rfill(dc, r, rad, t.field_bg, Some(chrome.border), ground);
}

/// Card plate. `on` false fades the fill and the hairline into the window ground.
/// Radius is the shared token, the same as [`paint_card`].
pub(crate) unsafe fn paint_plate(dc: HDC, r: RECT, dpi: i32, on: bool) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let t = ui_tokens();
    let fill = if on { t.card } else { crate::blend(t.bg, t.card, 0.55) };
    let line = if on { t.line } else { crate::blend(t.bg, t.line, 0.55) };
    crate::aa_rfill(dc, r, px(dpi, RADIUS).max(1), fill, Some(line), t.bg);
}

/// Edit client inside a painted field. The 1px border stays in the inset.
pub(crate) fn field_edit_rect(outer: RECT, dpi: i32) -> RECT {
    let h = outer.bottom - outer.top;
    let w = outer.right - outer.left;
    if h <= 0 || w <= 0 {
        return outer;
    }
    let d = px(dpi, FIELD_INSET).min(h / 2).min(w / 2);
    RECT {
        left: outer.left + d,
        top: outer.top + d,
        right: (outer.right - d).max(outer.left + d),
        bottom: (outer.bottom - d).max(outer.top + d),
    }
}

/// Brush for a borderless edit sitting in [`paint_field`]. Kept for the life of the colour.
pub(crate) unsafe fn field_brush() -> HBRUSH {
    let rgb = ui_tokens().field_bg;
    let mut slot = FIELD_BRUSH.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = slot.as_ref() {
        if c.rgb == rgb && c.handle != 0 {
            return HBRUSH(c.handle as *mut _);
        }
        if c.handle != 0 {
            let _ = DeleteObject(HGDIOBJ(c.handle as *mut _));
        }
    }
    let brush = CreateSolidBrush(crate::cref(rgb));
    *slot = Some(CachedBrush { rgb, handle: brush.0 as isize });
    brush
}

/// `WM_CTLCOLOREDIT` for a shared field: token text on the field fill.
pub(crate) unsafe fn color_field_edit(dc: HDC) -> LRESULT {
    color_field_ink(dc, ui_tokens().text)
}

/// Same brush as [`color_field_edit`], with a caller ink. An off rule fades the letters.
pub(crate) unsafe fn color_field_ink(dc: HDC, ink: u32) -> LRESULT {
    SetTextColor(dc, crate::cref(ink));
    SetBkColor(dc, crate::cref(ui_tokens().field_bg));
    LRESULT(field_brush().0 as isize)
}

/// `WM_CTLCOLOREDIT` for a borderless edit that sits on `fill`, not the shared field token.
/// The stock `DC_BRUSH` is coloured for this message and returned, same as a pill-backed edit.
pub(crate) unsafe fn color_edit_on(dc: HDC, fill: u32, ink: u32) -> LRESULT {
    SetTextColor(dc, crate::cref(ink));
    SetBkColor(dc, crate::cref(fill));
    SetDCBrushColor(dc, crate::cref(fill));
    LRESULT(GetStockObject(DC_BRUSH).0 as isize)
}

/// Folder chip and key cap. Card fill, hairline, shared radius. Corners blend into `ground`.
pub(crate) unsafe fn paint_chip(dc: HDC, r: RECT, dpi: i32, ground: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let t = ui_tokens();
    crate::aa_rfill(dc, r, px(dpi, RADIUS).max(1), t.card, Some(t.line), ground);
}

/// Accent wash with no border. Corners blend into `ground`.
pub(crate) unsafe fn paint_wash(dc: HDC, r: RECT, dpi: i32, fill: u32, ground: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    crate::aa_rfill(dc, r, px(dpi, RADIUS).max(1), fill, None, ground);
}

/// Groove fill and a darker edge. Both stay off `bg` so a 0% bar is still visible.
pub(crate) fn meter_colors(bg: u32, sub: u32) -> (u32, u32) {
    (crate::blend(bg, sub, 0.35), crate::blend(bg, sub, 0.55))
}

/// Progress groove. `frac` is clamped to 0..=1. The fill uses `accent`.
/// Radius is 4 DIP: the groove is 8 DIP tall and stays a pill.
pub(crate) unsafe fn paint_meter(dc: HDC, bar: RECT, frac: f32, dpi: i32, accent: u32) {
    if bar.right <= bar.left || bar.bottom <= bar.top {
        return;
    }
    let t = ui_tokens();
    let (track, edge) = meter_colors(t.bg, t.sub);
    let rad = px(dpi, 4);
    crate::aa_rfill(dc, bar, rad, track, Some(edge), t.bg);
    let span = (bar.right - bar.left).max(1);
    let fill_w = ((span as f32) * frac.clamp(0.0, 1.0)).round() as i32;
    if fill_w > 0 {
        let fill = RECT { right: (bar.left + fill_w.max(px(dpi, SPACE_8))).min(bar.right), ..bar };
        crate::aa_rfill(dc, fill, rad, accent, None, track);
    }
}

/// Section title, token text, single line, left aligned. The font is the caller's 12px face.
pub(crate) unsafe fn paint_section_header(dc: HDC, r: RECT, title: &str, font: HFONT) {
    crate::text(dc, r, title, font, ui_tokens().text, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX);
}

/// Body label, token text.
pub(crate) unsafe fn paint_label(dc: HDC, r: RECT, s: &str, font: HFONT) {
    crate::text(dc, r, s, font, ui_tokens().text, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX);
}

/// Secondary label, token sub ink.
pub(crate) unsafe fn paint_meta(dc: HDC, r: RECT, s: &str, font: HFONT) {
    crate::text(dc, r, s, font, ui_tokens().sub, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX);
}

/// Secondary line centred in `r`. Status and the page footer use this.
pub(crate) unsafe fn paint_center_meta(dc: HDC, r: RECT, s: &str, font: HFONT) {
    crate::text(dc, r, s, font, ui_tokens().sub, DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX);
}

/// Caption of a small dialog: token ground, no cerulean fill, no gradient.
pub(crate) unsafe fn style_dialog_caption(h: HWND) {
    let dark = ui_tokens().dark;
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = crate::cref(ui_tokens().bg);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    let def = COLORREF(0xFFFF_FFFF);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(36), &def as *const _ as _, 4);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(34), &def as *const _ as _, 4);
}

/// Preview-handler colours. The handler paints its own window on the token card with token text.
pub(crate) unsafe fn style_preview_visuals(vis: &IPreviewHandlerVisuals) {
    let t = ui_tokens();
    _ = vis.SetBackgroundColor(crate::cref(t.card));
    _ = vis.SetTextColor(crate::cref(t.text));
}

/// Borderless popup (the server dialog): round corners and a token-coloured frame.
pub(crate) unsafe fn style_popup_chrome(h: HWND) {
    let pref: u32 = 2;
    let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as _, 4);
    let dark = ui_tokens().dark;
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let border = crate::cref(ui_tokens().bg);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(34), &border as *const _ as _, 4);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imports::RECT;
    use crate::{ACCENT, ACCENT_HOVER_LIGHT, LINE_DARK, LINE_LIGHT};

    #[test]
    fn field_is_one_border_and_never_grows_a_ring() {
        let rest = field_chrome(false, false);
        let focus = field_chrome(true, false);
        let focus_dark = field_chrome(true, true);
        assert_eq!(rest.outset, 0);
        assert_eq!(focus.outset, 0);
        assert_eq!(focus_dark.outset, 0);
        assert_eq!(rest.border, LINE_LIGHT);
        assert_eq!(field_chrome(false, true).border, LINE_DARK);
        assert_eq!(focus.border, ACCENT);
        assert_eq!(focus_dark.border, ACCENT);
        assert_ne!(focus.border, ACCENT_HOVER_LIGHT);
        assert_ne!(focus.border, 0x2EC4A6);
    }

    #[test]
    fn field_edit_sits_inside_the_border() {
        let outer = RECT { left: 10, top: 20, right: 110, bottom: 46 };
        let inner = field_edit_rect(outer, 96);
        assert_eq!(inner.left, 12);
        assert_eq!(inner.top, 22);
        assert_eq!(inner.right, 108);
        assert_eq!(inner.bottom, 44);
        assert_eq!(field_edit_rect(RECT::default(), 96), RECT::default());
    }

    #[test]
    fn meter_groove_stays_off_the_window_ground() {
        let (fill, edge) = meter_colors(BG_LIGHT, SUB_LIGHT);
        assert_ne!(fill, BG_LIGHT);
        assert_ne!(fill, edge);
        let (dark, dark_edge) = meter_colors(BG_DARK, SUB_DARK);
        assert_ne!(dark, BG_DARK);
        assert_ne!(dark, dark_edge);
    }
}
