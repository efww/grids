use crate::imports::*;

// Themes, 0xRRGGBB. Light: an ivory frame around white plates; dark: charcoal. Each theme has its own
// selection colours (same names, tuned to the ground): pastels on light, muted mid tones on dark.
pub(crate) struct Palette {
    pub(crate) bg: u32,   // frame: window, toolbar, gaps, status line
    pub(crate) card: u32, // plates and lists
    pub(crate) line: u32,
    pub(crate) pill: u32, // fields
    pub(crate) text: u32,
    pub(crate) sub: u32,
    pub(crate) accents: [(&'static str, u32); 6],
}

pub(crate) const LIGHT: Palette = Palette {
    bg: super::tokens::BG_LIGHT,
    card: super::tokens::CARD_LIGHT,
    line: super::tokens::LINE_LIGHT,
    pill: 0xF4F2EE,
    text: super::tokens::TEXT_LIGHT,
    sub: super::tokens::SUB_LIGHT,
    accents: [("하늘", 0x8DB5E8), ("그레이", 0xC5C5C5), ("라벤더", 0xA99BE0), ("세이지", 0x8CC5A8), ("피치", 0xEFAE92), ("로즈", 0xE7A1B5)],
};

pub(crate) const DARK: Palette = Palette {
    bg: super::tokens::BG_DARK,
    card: super::tokens::CARD_DARK,
    line: super::tokens::LINE_DARK,
    pill: 0x303338,
    text: super::tokens::TEXT_DARK,
    sub: super::tokens::SUB_DARK,
    accents: [("하늘", 0x4C6FA8), ("그레이", 0x4B4B4B), ("라벤더", 0x6A5AA8), ("세이지", 0x3E8676), ("피치", 0xA8624A), ("로즈", 0xA65A78)],
};

pub(crate) static DARK_ON: AtomicBool = AtomicBool::new(false);

pub(crate) fn pal() -> &'static Palette {
    if DARK_ON.load(Ordering::Relaxed) {
        &DARK
    } else {
        &LIGHT
    }
}

pub(crate) const INK: u32 = 0x2A2825; // dark text on light selection colours
pub(crate) const GREEN: u32 = 0x1FA463; // unsaved layout changes
pub(crate) fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = ((a >> sh & 0xFF) as f32, (b >> sh & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(16) | ch(8) | ch(0)
}

/// What the left panel (folder tree and favourites) sits on: the app background in both themes.
pub(crate) fn side_ground() -> u32 {
    pal().bg
}

/// Gap fill and accent line of the left-panel divider. The same pair in either order.
pub(crate) fn side_divider_colors(dark: bool, accent: u32) -> (u32, u32) {
    ((if dark { &DARK } else { &LIGHT }).bg, accent)
}

pub(crate) fn side_divider_now(accent: u32) -> (u32, u32) {
    side_divider_colors(DARK_ON.load(Ordering::Relaxed), accent)
}

/// Favourite group row wash: accent mixed into the panel. Light keeps about 18% accent.
/// Dark uses a smaller share so a bright accent stays a dim tint rather than a glow.
pub(crate) fn group_tint(accent: u32, ground: u32, dark: bool) -> u32 {
    blend(ground, accent, if dark { 0.12 } else { 0.18 })
}

// Text/icons on a coloured fill: near-black on light colours, white on dark ones (black accent -> white text).
/// Pre-#222 mint brand (#2EC4A6). Saved settings that still use it follow the cerulean rebrand.
pub(crate) const OLD_MINT_BRAND: u32 = 0x2EC4A6;
/// Current brand accent (#0A9BD6). The button tokens own the value.
pub(crate) const CERULEAN_BRAND: u32 = super::tokens::ACCENT;

/// Map the retired mint brand to cerulean. Any other chosen swatch is left alone.
pub(crate) fn brand_accent(c: u32) -> u32 {
    if c == OLD_MINT_BRAND {
        CERULEAN_BRAND
    } else {
        c
    }
}

pub(crate) fn on_color(bg: u32) -> u32 {
    let l = 0.2126 * (bg >> 16 & 0xFF) as f32 + 0.7152 * (bg >> 8 & 0xFF) as f32 + 0.0722 * (bg & 0xFF) as f32;
    if l > 160.0 {
        INK
    } else {
        0xFFFFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: u32, b: u32) -> u32 {
        let d = |s: u32| (a >> s & 0xFF).abs_diff(b >> s & 0xFF) as u32;
        d(16) + d(8) + d(0)
    }

    fn lum(c: u32) -> f32 {
        0.2126 * (c >> 16 & 0xFF) as f32 + 0.7152 * (c >> 8 & 0xFF) as f32 + 0.0722 * (c & 0xFF) as f32
    }

    #[test]
    fn group_tint_is_a_light_wash_and_selection_is_stronger() {
        let accent = 0x0067C0;
        let tint = group_tint(accent, LIGHT.card, false);
        let sel = blend(accent, LIGHT.card, 0.16);
        let hot = blend(LIGHT.bg, LIGHT.card, 0.35);
        assert_eq!(tint, blend(LIGHT.card, accent, 0.18));
        assert_ne!(tint, hot);
        assert!(dist(sel, LIGHT.card) > dist(tint, LIGHT.card) * 2);

        let accent_d = 0x4CC2FF;
        let tint_d = group_tint(accent_d, DARK.card, true);
        let sel_d = blend(accent_d, DARK.card, 0.16);
        let hot_d = blend(DARK.bg, DARK.card, 0.35);
        assert_eq!(tint_d, blend(DARK.card, accent_d, 0.12));
        assert!(lum(tint_d) < lum(tint));
        assert_ne!(tint_d, hot_d);
        assert!(dist(sel_d, DARK.card) > dist(tint_d, DARK.card) * 2);
    }

    #[test]
    fn divider_colours_follow_the_palette() {
        assert_eq!(side_divider_colors(false, 0x112233), (LIGHT.bg, 0x112233));
        assert_eq!(side_divider_colors(true, 0xAABBCC), (DARK.bg, 0xAABBCC));
        assert_eq!(side_ground_of(false), LIGHT.bg);
        assert_eq!(side_ground_of(true), DARK.bg);
    }

    fn side_ground_of(dark: bool) -> u32 {
        side_divider_colors(dark, 0).0
    }

    #[test]
    fn cerulean_fill_text_contrast() {
        // Default accent is cerulean #0A9BD6. Mid luminance → white text on the fill
        // (mint #2EC4A6 used to keep dark ink; do not revive mint).
        assert_eq!(on_color(0x0A9BD6), 0xFFFFFF);
        assert_eq!(on_color(CERULEAN_BRAND), 0xFFFFFF);
        assert_eq!(brand_accent(OLD_MINT_BRAND), CERULEAN_BRAND);
        assert_eq!(brand_accent(0xEFAE92), 0xEFAE92); // peach swatch stays
        assert_eq!(LIGHT.accents.len(), 6);
        assert_eq!(LIGHT.accents[1].1, 0xC5C5C5);
    }
}
