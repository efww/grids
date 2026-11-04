//! Window ground, card, field, and section title. Same tokens as the buttons.
//!
//! [`paint_section_header`] is the hook for later batches (favourites group titles,
//! confirm headings, the name prompt's label). This batch's windows do not have that title.

use super::tokens::*;
use crate::imports::*;

pub(crate) unsafe fn paint_window(dc: HDC, r: RECT) {
    crate::fill(dc, r, ui_tokens().bg);
}

/// Card plate: token fill, hairline, radius 2. Corners blend into the window ground.
pub(crate) unsafe fn paint_card(dc: HDC, r: RECT, dpi: i32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let t = ui_tokens();
    crate::aa_rfill(dc, r, px(dpi, RADIUS).max(1), t.card, Some(t.line), t.bg);
}

/// Single-line field. Focused: accent border plus a 1px accent ring just outside `r`.
pub(crate) unsafe fn paint_field(dc: HDC, r: RECT, dpi: i32, focused: bool) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let t = ui_tokens();
    let rad = px(dpi, RADIUS).max(1);
    if focused {
        // Accent border, plus the mockup's 1px accent ring just outside the field.
        let outer = RECT { left: r.left - 1, top: r.top - 1, right: r.right + 1, bottom: r.bottom + 1 };
        crate::aa_rfill(dc, outer, rad + 1, ACCENT, None, t.bg);
        crate::aa_rfill(dc, r, rad, t.field_bg, Some(ACCENT), ACCENT);
    } else {
        crate::aa_rfill(dc, r, rad, t.field_bg, Some(t.line), t.bg);
    }
}

/// Section title, token text, single line, left aligned. The font is the caller's 12px face.
#[allow(dead_code)] // Hook for later batches. See the module note.
pub(crate) unsafe fn paint_section_header(dc: HDC, r: RECT, title: &str, font: HFONT) {
    crate::text(dc, r, title, font, ui_tokens().text, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX);
}
