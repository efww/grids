//! Design tokens for every Maguro window. Colours, type, and spacing live here so a
//! button or background change is one edit. Values match the approved UI mockup.
//!
//! The retired mint brand (`#2EC4A6`) is not a token. Primary buttons use cerulean.

use crate::imports::*;

/// Window background, light `#F3F1EC`.
pub(crate) const BG_LIGHT: u32 = 0xF3F1EC;
/// Card and secondary-button fill, light `#FFFFFF`.
pub(crate) const CARD_LIGHT: u32 = 0xFFFFFF;
/// Hairline, light `#E4E0D8`.
pub(crate) const LINE_LIGHT: u32 = 0xE4E0D8;
/// Primary text, light `#2A2825`.
pub(crate) const TEXT_LIGHT: u32 = 0x2A2825;
/// Secondary text, light `#7C776F`.
pub(crate) const SUB_LIGHT: u32 = 0x7C776F;
/// Field fill, light `#FFFFFF`.
pub(crate) const FIELD_LIGHT: u32 = 0xFFFFFF;
/// Secondary border on hover, light `#C9C4BA`.
pub(crate) const LINE_HOVER_LIGHT: u32 = 0xC9C4BA;

/// Window background, dark `#1C1D20`.
pub(crate) const BG_DARK: u32 = 0x1C1D20;
/// Card and secondary-button fill, dark `#26282C`.
pub(crate) const CARD_DARK: u32 = 0x26282C;
/// Hairline, dark `#36393E`.
pub(crate) const LINE_DARK: u32 = 0x36393E;
/// Primary text, dark `#E6E8EB`.
pub(crate) const TEXT_DARK: u32 = 0xE6E8EB;
/// Secondary text, dark `#9AA0A8`.
pub(crate) const SUB_DARK: u32 = 0x9AA0A8;
/// Field fill, dark `#1A1B1E`.
pub(crate) const FIELD_DARK: u32 = 0x1A1B1E;
/// Secondary border on hover, dark `#4A4E55`.
pub(crate) const LINE_HOVER_DARK: u32 = 0x4A4E55;

/// Brand accent `#0A9BD6`. Never mint.
pub(crate) const ACCENT: u32 = 0x0A9BD6;
/// Primary hover on a light ground `#0890C7`.
pub(crate) const ACCENT_HOVER_LIGHT: u32 = 0x0890C7;
/// Primary hover on a dark ground `#22A9E0` (lighter, not deeper).
pub(crate) const ACCENT_HOVER_DARK: u32 = 0x22A9E0;
/// Primary and link pressed `#0786BC`. Same in both themes.
pub(crate) const ACCENT_PRESSED: u32 = 0x0786BC;

/// Danger label, light `#C42B1C`.
pub(crate) const DANGER_LIGHT: u32 = 0xC42B1C;
/// Danger label, dark `#F07167`.
pub(crate) const DANGER_DARK: u32 = 0xF07167;
/// Danger hover fill `#C42B1C`. White label. Same in both themes.
pub(crate) const DANGER_HOVER_FILL: u32 = 0xC42B1C;
/// Danger pressed fill `#A52417`. Same in both themes.
pub(crate) const DANGER_PRESSED: u32 = 0xA52417;

pub(crate) const WHITE: u32 = 0xFFFFFF;

/// Corner radius, DIP. Buttons, cards, and fields.
pub(crate) const RADIUS: i32 = 2;
/// Tool-row button height, DIP.
pub(crate) const BTN_H_TOOL: i32 = 24;
/// Dialog-footer button height, DIP.
pub(crate) const BTN_H_FOOTER: i32 = 28;
/// Horizontal label padding inside a text button, DIP.
pub(crate) const BTN_PAD_X: i32 = 12;
/// Minimum text-button width, DIP.
pub(crate) const BTN_MIN_W: i32 = 72;
/// Gap between footer buttons, DIP.
pub(crate) const BTN_GAP: i32 = 8;
/// Field height, DIP.
pub(crate) const FIELD_H: i32 = 26;
/// Field text inset, DIP.
pub(crate) const FIELD_PAD_X: i32 = 8;
/// Focus ring sits this many DIP outside the face. The ring itself is 1px.
pub(crate) const FOCUS_GAP: i32 = 2;

/// Body padding from the mockup, DIP.
pub(crate) const PAD_BODY_X: i32 = 16;
pub(crate) const PAD_BODY_Y: i32 = 14;
pub(crate) const SPACE_4: i32 = 4;
pub(crate) const SPACE_8: i32 = 8;
pub(crate) const SPACE_12: i32 = 12;

/// UI face. CJK falls through Segoe UI to Malgun Gothic / Yu Gothic UI on Windows.
pub(crate) const FONT_UI: &str = "Segoe UI";
pub(crate) const FONT_FALLBACKS: &[&str] = &["Malgun Gothic", "Yu Gothic UI", "Noto Sans CJK KR"];
/// Body size, px. Section titles use the same size at weight 600.
pub(crate) const FONT_PX: i32 = 12;
/// Meta, hints, and list paths, px.
pub(crate) const FONT_META_PX: i32 = 11;

/// Secondary hover is 6% text mixed into the card. Pressed is 12%.
pub(crate) const SECONDARY_HOVER: f32 = 0.06;
pub(crate) const SECONDARY_PRESSED: f32 = 0.12;
/// Disabled controls are the resting face at 40% over the ground.
pub(crate) const DISABLED_ALPHA: f32 = 0.4;

/// Icon hover / pressed: text mixed into the ground. Light 7% / 13%, dark 10% / 18%.
pub(crate) const ICON_HOVER_LIGHT: f32 = 0.07;
pub(crate) const ICON_PRESSED_LIGHT: f32 = 0.13;
pub(crate) const ICON_HOVER_DARK: f32 = 0.10;
pub(crate) const ICON_PRESSED_DARK: f32 = 0.18;

/// Selection mist: accent mixed into the card. Light 16%, dark 22%.
pub(crate) const MIST_LIGHT: f32 = 0.16;
pub(crate) const MIST_DARK: f32 = 0.22;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Tokens {
    pub bg: u32,
    pub card: u32,
    pub line: u32,
    pub text: u32,
    pub sub: u32,
    pub field_bg: u32,
    pub line_hover: u32,
    pub accent_hover: u32,
    pub danger: u32,
    pub icon_hover: f32,
    pub icon_pressed: f32,
    pub mist: f32,
    pub dark: bool,
}

pub(crate) const LIGHT_TOKENS: Tokens = Tokens {
    bg: BG_LIGHT,
    card: CARD_LIGHT,
    line: LINE_LIGHT,
    text: TEXT_LIGHT,
    sub: SUB_LIGHT,
    field_bg: FIELD_LIGHT,
    line_hover: LINE_HOVER_LIGHT,
    accent_hover: ACCENT_HOVER_LIGHT,
    danger: DANGER_LIGHT,
    icon_hover: ICON_HOVER_LIGHT,
    icon_pressed: ICON_PRESSED_LIGHT,
    mist: MIST_LIGHT,
    dark: false,
};

pub(crate) const DARK_TOKENS: Tokens = Tokens {
    bg: BG_DARK,
    card: CARD_DARK,
    line: LINE_DARK,
    text: TEXT_DARK,
    sub: SUB_DARK,
    field_bg: FIELD_DARK,
    line_hover: LINE_HOVER_DARK,
    accent_hover: ACCENT_HOVER_DARK,
    danger: DANGER_DARK,
    icon_hover: ICON_HOVER_DARK,
    icon_pressed: ICON_PRESSED_DARK,
    mist: MIST_DARK,
    dark: true,
};

/// Tokens for the theme that is on now.
pub(crate) fn ui_tokens() -> Tokens {
    if crate::DARK_ON.load(Ordering::Relaxed) {
        DARK_TOKENS
    } else {
        LIGHT_TOKENS
    }
}

pub(crate) fn tokens_for(dark: bool) -> Tokens {
    if dark { DARK_TOKENS } else { LIGHT_TOKENS }
}

/// DIP to px. 96 DPI is 1:1. Below 96 stays at 96, matching the other painters.
pub(crate) fn px(dpi: i32, dip: i32) -> i32 {
    dip * dpi.max(96) / 96
}

impl Tokens {
    /// Accent mixed into the card. Light uses white, which is the light card.
    pub(crate) fn sel_mist(self) -> u32 {
        crate::blend(self.card, ACCENT, self.mist)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: u32) -> f32 {
        0.2126 * (c >> 16 & 0xFF) as f32 + 0.7152 * (c >> 8 & 0xFF) as f32 + 0.0722 * (c & 0xFF) as f32
    }

    #[test]
    fn mockup_colours_and_metrics() {
        assert_eq!(BG_LIGHT, 0xF3F1EC);
        assert_eq!(CARD_LIGHT, 0xFFFFFF);
        assert_eq!(BG_DARK, 0x1C1D20);
        assert_eq!(CARD_DARK, 0x26282C);
        assert_eq!(TEXT_DARK, 0xE6E8EB);
        assert_eq!(SUB_DARK, 0x9AA0A8);
        assert_eq!(LINE_DARK, 0x36393E);
        assert_eq!(FIELD_DARK, 0x1A1B1E);
        assert_eq!(ACCENT, 0x0A9BD6);
        assert_eq!(ACCENT_HOVER_LIGHT, 0x0890C7);
        assert_eq!(ACCENT_HOVER_DARK, 0x22A9E0);
        assert_eq!(ACCENT_PRESSED, 0x0786BC);
        assert_eq!(DANGER_LIGHT, 0xC42B1C);
        assert_eq!(DANGER_DARK, 0xF07167);
        assert_eq!(DANGER_HOVER_FILL, 0xC42B1C);
        assert_eq!(DANGER_PRESSED, 0xA52417);
        assert_eq!(RADIUS, 2);
        assert_eq!(BTN_H_TOOL, 24);
        assert_eq!(BTN_H_FOOTER, 28);
        assert_eq!(BTN_PAD_X, 12);
        assert_eq!(BTN_MIN_W, 72);
        assert_eq!(BTN_GAP, 8);
        assert_eq!(FIELD_H, 26);
        assert_eq!(FIELD_PAD_X, 8);
        assert_eq!(FOCUS_GAP, 2);
        assert_eq!(PAD_BODY_X, 16);
        assert_eq!(PAD_BODY_Y, 14);
        assert_eq!(SPACE_4, 4);
        assert_eq!(SPACE_8, 8);
        assert_eq!(SPACE_12, 12);
        assert_eq!(FONT_PX, 12);
        assert_eq!(FONT_META_PX, 11);
        assert_eq!(FONT_UI, "Segoe UI");
        assert!(FONT_FALLBACKS.contains(&"Malgun Gothic"));
    }

    #[test]
    fn primary_hover_moves_and_mint_is_absent() {
        let mint = 0x2EC4A6u32;
        assert!(lum(ACCENT_HOVER_LIGHT) < lum(ACCENT));
        assert!(lum(ACCENT_HOVER_DARK) > lum(ACCENT));
        assert_ne!(ACCENT_HOVER_LIGHT, ACCENT_PRESSED);
        assert_ne!(ACCENT_HOVER_DARK, ACCENT);
        for c in [ACCENT, ACCENT_HOVER_LIGHT, ACCENT_HOVER_DARK, ACCENT_PRESSED, DANGER_LIGHT, DANGER_DARK, DANGER_HOVER_FILL, DANGER_PRESSED] {
            assert_ne!(c, mint, "{c:#08X}");
        }
        assert_eq!(LIGHT_TOKENS.sel_mist(), crate::blend(CARD_LIGHT, ACCENT, 0.16));
        assert_eq!(DARK_TOKENS.sel_mist(), crate::blend(CARD_DARK, ACCENT, 0.22));
        assert_eq!(px(96, BTN_H_TOOL), 24);
        assert_eq!(px(144, RADIUS), 3);
    }
}
