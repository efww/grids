//! Segment chips (SMB / WebDAV / SFTP / FTP, and any later radio row).
//! Selected, hover, and pressed follow the same cerulean and secondary rules as buttons.

use super::button::{ButtonState, ButtonVisual};
use super::tokens::*;
use crate::imports::*;

pub(crate) struct SegmentColors {
    pub fill: u32,
    pub border: Option<u32>,
    pub ink: u32,
    pub bold: bool,
}

pub(crate) fn segment_colors(selected: bool, state: ButtonState, dark: bool) -> SegmentColors {
    let t = tokens_for(dark);
    let (fill, border, ink, bold) = if selected {
        let c = match state {
            ButtonState::Hover => t.accent_hover,
            ButtonState::Pressed => ACCENT_PRESSED,
            _ => ACCENT,
        };
        (c, Some(c), WHITE, true)
    } else {
        let (fill, border) = match state {
            ButtonState::Hover => (crate::blend(t.card, t.text, SECONDARY_HOVER), t.line_hover),
            ButtonState::Pressed => (crate::blend(t.card, t.text, SECONDARY_PRESSED), t.line_hover),
            _ => (t.card, t.line),
        };
        (fill, Some(border), t.text, false)
    };
    if state == ButtonState::Disabled {
        return SegmentColors {
            fill: crate::blend(t.bg, fill, DISABLED_ALPHA),
            border: border.map(|c| crate::blend(t.bg, c, DISABLED_ALPHA)),
            ink: crate::blend(t.bg, ink, DISABLED_ALPHA),
            bold: false,
        };
    }
    SegmentColors { fill, border, ink, bold }
}

pub(crate) unsafe fn paint_segment(dc: HDC, r: RECT, label: &str, selected: bool, visual: ButtonVisual, dpi: i32, ground: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let dark = ui_tokens().dark;
    let colors = segment_colors(selected, visual.state, dark);
    let rad = px(dpi, RADIUS).max(1);
    crate::aa_rfill(dc, r, rad, colors.fill, colors.border.filter(|c| *c != colors.fill), ground);
    if label.is_empty() {
        return;
    }
    let font = if colors.bold { crate::bold_ui_font() } else { crate::body_font() };
    crate::text(dc, r, label, font, colors.ink, DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
}

#[cfg(test)]
mod tests {
    use super::{segment_colors, ButtonState};
    use crate::{ACCENT, ACCENT_HOVER_DARK, ACCENT_HOVER_LIGHT, ACCENT_PRESSED, CARD_LIGHT, LINE_HOVER_LIGHT, LINE_LIGHT, SECONDARY_HOVER, SECONDARY_PRESSED, TEXT_LIGHT, WHITE};

    #[test]
    fn selected_chip_is_cerulean_and_follows_button_hover_and_press() {
        let mint = 0x2EC4A6u32;
        let rest = segment_colors(true, ButtonState::Normal, false);
        let hover = segment_colors(true, ButtonState::Hover, false);
        let pressed = segment_colors(true, ButtonState::Pressed, false);
        let dark_hover = segment_colors(true, ButtonState::Hover, true);
        assert_eq!(rest.fill, ACCENT);
        assert_eq!(rest.ink, WHITE);
        assert!(rest.bold);
        assert_eq!(hover.fill, ACCENT_HOVER_LIGHT);
        assert_eq!(pressed.fill, ACCENT_PRESSED);
        assert_eq!(dark_hover.fill, ACCENT_HOVER_DARK);
        for c in [rest.fill, hover.fill, pressed.fill, dark_hover.fill] {
            assert_ne!(c, mint);
        }
    }

    #[test]
    fn resting_chip_is_a_card_and_hover_matches_secondary() {
        let rest = segment_colors(false, ButtonState::Normal, false);
        let hover = segment_colors(false, ButtonState::Hover, false);
        let pressed = segment_colors(false, ButtonState::Pressed, false);
        assert_eq!(rest.fill, CARD_LIGHT);
        assert_eq!(rest.border, Some(LINE_LIGHT));
        assert_eq!(rest.ink, TEXT_LIGHT);
        assert!(!rest.bold);
        assert_eq!(hover.fill, crate::blend(CARD_LIGHT, TEXT_LIGHT, SECONDARY_HOVER));
        assert_eq!(hover.border, Some(LINE_HOVER_LIGHT));
        assert_eq!(pressed.fill, crate::blend(CARD_LIGHT, TEXT_LIGHT, SECONDARY_PRESSED));
        assert_ne!(hover.fill, rest.fill);
        assert_ne!(pressed.fill, hover.fill);
    }

    #[test]
    fn disabled_chip_fades_toward_the_ground_and_drops_bold() {
        let rest = segment_colors(true, ButtonState::Normal, false);
        let off = segment_colors(true, ButtonState::Disabled, false);
        let dark_rest = segment_colors(false, ButtonState::Normal, true);
        let idle = segment_colors(false, ButtonState::Disabled, true);
        assert!(rest.bold);
        assert!(!off.bold);
        assert!(!idle.bold);
        assert_ne!(off.fill, rest.fill);
        assert_ne!(off.ink, WHITE);
        assert_ne!(idle.fill, dark_rest.fill);
        assert_ne!(idle.ink, dark_rest.ink);
    }
}
