//! One button painter and one pointer state machine.
//!
//! Kinds: primary, secondary, danger, icon, link. Every kind shares hover, press,
//! release, disabled, and focus. Mouse leave clears hover. Release over the button
//! clicks. Release outside cancels. A lost capture cancels and does not leave hover stuck.
//!
//! Later batches call [`paint_button`] from the name prompt, favourites, About, Excel,
//! popups, and notices. The custom confirm already calls it. Those other windows still
//! have their own painters.

use super::tokens::*;
use crate::imports::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    Primary,
    Secondary,
    Danger,
    Icon,
    Link,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonState {
    Normal,
    Hover,
    Pressed,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ButtonVisual {
    pub state: ButtonState,
    pub focus: bool,
}

/// Where the focus ring is drawn. Parent-painted buttons have room outside the face.
/// An owner-draw control clips to its window, so the ring is kept inside the rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FocusRing {
    Outside,
    Inside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ButtonColors {
    /// `None` leaves the ground showing (icon and link at rest).
    pub fill: Option<u32>,
    pub border: Option<u32>,
    pub ink: u32,
    pub underline: bool,
}

/// Face colours for `kind` in `state`. `ground` is the paint behind the button,
/// used for the disabled fade and for icon washes.
pub(crate) fn button_colors(kind: ButtonKind, state: ButtonState, dark: bool, ground: u32) -> ButtonColors {
    let t = tokens_for(dark);
    let resting = resting_colors(kind, &t, ground);
    if state == ButtonState::Disabled {
        return ButtonColors {
            fill: resting.fill.map(|c| crate::blend(ground, c, DISABLED_ALPHA)),
            border: resting.border.map(|c| crate::blend(ground, c, DISABLED_ALPHA)),
            ink: crate::blend(ground, resting.ink, DISABLED_ALPHA),
            underline: false,
        };
    }
    match kind {
        ButtonKind::Primary => {
            let c = match state {
                ButtonState::Hover => t.accent_hover,
                ButtonState::Pressed => ACCENT_PRESSED,
                _ => ACCENT,
            };
            ButtonColors { fill: Some(c), border: Some(c), ink: WHITE, underline: false }
        }
        ButtonKind::Secondary => {
            let (fill, border) = match state {
                ButtonState::Hover => (crate::blend(t.card, t.text, SECONDARY_HOVER), t.line_hover),
                ButtonState::Pressed => (crate::blend(t.card, t.text, SECONDARY_PRESSED), t.line_hover),
                _ => (t.card, t.line),
            };
            ButtonColors { fill: Some(fill), border: Some(border), ink: t.text, underline: false }
        }
        ButtonKind::Danger => match state {
            ButtonState::Hover => ButtonColors { fill: Some(DANGER_HOVER_FILL), border: Some(DANGER_HOVER_FILL), ink: WHITE, underline: false },
            ButtonState::Pressed => ButtonColors { fill: Some(DANGER_PRESSED), border: Some(DANGER_PRESSED), ink: WHITE, underline: false },
            _ => ButtonColors { fill: Some(t.card), border: Some(t.line), ink: t.danger, underline: false },
        },
        ButtonKind::Icon => {
            let wash = match state {
                ButtonState::Hover => Some(crate::blend(ground, t.text, t.icon_hover)),
                ButtonState::Pressed => Some(crate::blend(ground, t.text, t.icon_pressed)),
                _ => None,
            };
            ButtonColors { fill: wash, border: None, ink: t.text, underline: false }
        }
        ButtonKind::Link => {
            let (ink, underline) = match state {
                ButtonState::Hover => (ACCENT, true),
                ButtonState::Pressed => (ACCENT_PRESSED, true),
                _ => (ACCENT, false),
            };
            ButtonColors { fill: None, border: None, ink, underline }
        }
    }
}

fn resting_colors(kind: ButtonKind, t: &Tokens, _ground: u32) -> ButtonColors {
    match kind {
        ButtonKind::Primary => ButtonColors { fill: Some(ACCENT), border: Some(ACCENT), ink: WHITE, underline: false },
        ButtonKind::Secondary => ButtonColors { fill: Some(t.card), border: Some(t.line), ink: t.text, underline: false },
        ButtonKind::Danger => ButtonColors { fill: Some(t.card), border: Some(t.line), ink: t.danger, underline: false },
        ButtonKind::Icon => ButtonColors { fill: None, border: None, ink: t.text, underline: false },
        ButtonKind::Link => ButtonColors { fill: None, border: None, ink: ACCENT, underline: false },
    }
}

/// Owner-draw `DRAWITEM` flags plus the shared hover window. Pressed wins over hover.
/// Disabled draws the faded face and no focus ring.
pub(crate) fn visual_from_item(item_state: u32, hovered: bool) -> ButtonVisual {
    let disabled = item_state & ODS_DISABLED.0 != 0;
    if disabled {
        return ButtonVisual { state: ButtonState::Disabled, focus: false };
    }
    let pressed = item_state & ODS_SELECTED.0 != 0;
    let focus = item_state & ODS_FOCUS.0 != 0;
    let state = if pressed {
        ButtonState::Pressed
    } else if hovered {
        ButtonState::Hover
    } else {
        ButtonState::Normal
    };
    ButtonVisual { state, focus }
}

/// Pointer state for buttons painted by the parent (not a child `BUTTON`).
/// `T` is the caller's button id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tracker<T: Copy + Eq> {
    hot: Option<T>,
    down: Option<T>,
    focus: Option<T>,
}

impl<T: Copy + Eq> Default for Tracker<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrackEffect<T> {
    None,
    Repaint,
    Click(T),
    Cancel,
}

impl<T: Copy + Eq> Tracker<T> {
    pub(crate) fn new() -> Self {
        Self { hot: None, down: None, focus: None }
    }

    pub(crate) fn down(&self) -> Option<T> {
        self.down
    }

    pub(crate) fn visual(&self, id: T, disabled: bool) -> ButtonVisual {
        if disabled {
            return ButtonVisual { state: ButtonState::Disabled, focus: false };
        }
        let focus = self.focus == Some(id);
        let state = if self.down == Some(id) && self.hot == Some(id) {
            ButtonState::Pressed
        } else if self.down.is_none() && self.hot == Some(id) {
            ButtonState::Hover
        } else {
            ButtonState::Normal
        };
        ButtonVisual { state, focus }
    }

    /// Pointer moved. `over` is the button under it, or `None`. Other buttons do not
    /// light up while one is captured.
    pub(crate) fn pointer_move(&mut self, over: Option<T>) -> TrackEffect<T> {
        if self.hot == over {
            return TrackEffect::None;
        }
        self.hot = over;
        TrackEffect::Repaint
    }

    /// Mouse leave. Hover drops. A captured press stays captured, so release can
    /// still click or cancel from the up coordinates. The pressed look drops because
    /// the pointer is no longer over the button.
    pub(crate) fn pointer_leave(&mut self) -> TrackEffect<T> {
        if self.hot.is_none() {
            return TrackEffect::None;
        }
        self.hot = None;
        TrackEffect::Repaint
    }

    pub(crate) fn pointer_down(&mut self, id: T) -> TrackEffect<T> {
        self.hot = Some(id);
        self.down = Some(id);
        self.focus = Some(id);
        TrackEffect::Repaint
    }

    /// `over` is the button under the pointer at release. Same id clicks. Anything
    /// else cancels. No prior press returns [`TrackEffect::None`].
    pub(crate) fn pointer_up(&mut self, over: Option<T>) -> TrackEffect<T> {
        let down = self.down.take();
        self.hot = over;
        match down {
            Some(id) if over == Some(id) => TrackEffect::Click(id),
            Some(_) => TrackEffect::Cancel,
            None => TrackEffect::None,
        }
    }

    /// Capture was taken away. Cancel the press and clear hover so it cannot stick.
    /// Focus set by a completed click (press already cleared) stays.
    pub(crate) fn capture_lost(&mut self) -> TrackEffect<T> {
        if self.down.take().is_some() {
            self.hot = None;
            self.focus = None;
            TrackEffect::Cancel
        } else {
            TrackEffect::None
        }
    }
}

fn reach(dpi: i32) -> i32 {
    px(dpi, FOCUS_GAP) + 1
}

fn inflate(r: RECT, d: i32) -> RECT {
    RECT { left: r.left - d, top: r.top - d, right: r.right + d, bottom: r.bottom + d }
}

fn deflate(r: RECT, d: i32) -> RECT {
    RECT { left: r.left + d, top: r.top + d, right: r.right - d, bottom: r.bottom - d }
}

/// Paint one button. `ground` is the colour already behind `rect` (or the colour
/// the corners should blend into). `ring` is [`FocusRing::Inside`] for owner-draw
/// child windows, which clip to `rect`.
pub(crate) unsafe fn paint_button(dc: HDC, rect: RECT, kind: ButtonKind, visual: ButtonVisual, label: &str, font: HFONT, dpi: i32, ground: u32, ring: FocusRing) {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }
    let dark = ui_tokens().dark;
    let colors = button_colors(kind, visual.state, dark, ground);
    let focused = visual.focus && visual.state != ButtonState::Disabled;
    let inset = reach(dpi);
    let face = if ring == FocusRing::Inside && focused { deflate(rect, inset) } else { rect };
    if ring == FocusRing::Inside {
        crate::fill(dc, rect, ground);
    }
    if focused && ring == FocusRing::Inside {
        crate::frame(dc, rect, 1, ACCENT);
    }
    if face.right > face.left && face.bottom > face.top {
        if let Some(fill) = colors.fill {
            let border = colors.border.filter(|c| *c != fill);
            crate::aa_rfill(dc, face, px(dpi, RADIUS).max(1), fill, border, ground);
        }
        if !label.is_empty() {
            crate::text(dc, face, label, font, colors.ink, DT_CENTER);
            if colors.underline {
                underline(dc, face, label, font, colors.ink);
            }
        }
    }
    if focused && ring == FocusRing::Outside {
        crate::frame(dc, inflate(rect, inset), 1, ACCENT);
    }
}

unsafe fn underline(dc: HDC, r: RECT, label: &str, font: HFONT, ink: u32) {
    let m = crate::line_metrics(dc, font);
    let top = crate::ink_line_top(r.top, (r.bottom - r.top).max(0), m, crate::ink_kind(label));
    let y = (top + m.ascent).clamp(r.top, (r.bottom - 1).max(r.top));
    let w: Vec<u16> = label.encode_utf16().collect();
    if w.is_empty() {
        return;
    }
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    let ok = GetTextExtentPoint32W(dc, &w, &mut sz).as_bool();
    SelectObject(dc, old);
    if !ok {
        return;
    }
    let width = (r.right - r.left).max(0);
    let tw = sz.cx.max(0).min(width);
    if tw <= 0 {
        return;
    }
    let x = r.left + (width - tw) / 2;
    crate::fill(dc, RECT { left: x, top: y, right: x + tw, bottom: (y + 1).min(r.bottom) }, ink);
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINT: u32 = 0x2EC4A6;
    const GROUND: u32 = BG_LIGHT;

    fn fills(kind: ButtonKind, dark: bool) -> Vec<u32> {
        [ButtonState::Normal, ButtonState::Hover, ButtonState::Pressed, ButtonState::Disabled]
            .into_iter()
            .filter_map(|s| button_colors(kind, s, dark, if dark { BG_DARK } else { GROUND }).fill)
            .collect()
    }

    #[test]
    fn primary_changes_on_hover_and_press_and_never_uses_mint() {
        for dark in [false, true] {
            let ground = if dark { BG_DARK } else { GROUND };
            let rest = button_colors(ButtonKind::Primary, ButtonState::Normal, dark, ground);
            let hot = button_colors(ButtonKind::Primary, ButtonState::Hover, dark, ground);
            let down = button_colors(ButtonKind::Primary, ButtonState::Pressed, dark, ground);
            assert_eq!(rest.fill, Some(ACCENT));
            assert_eq!(rest.ink, WHITE);
            assert_eq!(hot.fill, Some(if dark { ACCENT_HOVER_DARK } else { ACCENT_HOVER_LIGHT }));
            assert_ne!(hot.fill, rest.fill);
            assert_eq!(down.fill, Some(ACCENT_PRESSED));
            assert_ne!(down.fill, hot.fill);
            for c in fills(ButtonKind::Primary, dark) {
                assert_ne!(c, MINT, "{c:#08X}");
            }
        }
    }

    #[test]
    fn secondary_danger_icon_and_link_follow_the_mockup() {
        let sec = button_colors(ButtonKind::Secondary, ButtonState::Hover, false, GROUND);
        assert_eq!(sec.fill, Some(crate::blend(CARD_LIGHT, TEXT_LIGHT, SECONDARY_HOVER)));
        assert_eq!(sec.border, Some(LINE_HOVER_LIGHT));
        let pressed = button_colors(ButtonKind::Secondary, ButtonState::Pressed, false, GROUND);
        assert_eq!(pressed.fill, Some(crate::blend(CARD_LIGHT, TEXT_LIGHT, SECONDARY_PRESSED)));

        let danger = button_colors(ButtonKind::Danger, ButtonState::Normal, false, GROUND);
        assert_eq!(danger.ink, DANGER_LIGHT);
        assert_eq!(danger.fill, Some(CARD_LIGHT));
        let danger_hot = button_colors(ButtonKind::Danger, ButtonState::Hover, true, BG_DARK);
        assert_eq!(danger_hot.fill, Some(DANGER_HOVER_FILL));
        assert_eq!(danger_hot.ink, WHITE);
        let danger_down = button_colors(ButtonKind::Danger, ButtonState::Pressed, true, BG_DARK);
        assert_eq!(danger_down.fill, Some(DANGER_PRESSED));
        assert_eq!(button_colors(ButtonKind::Danger, ButtonState::Normal, true, BG_DARK).ink, DANGER_DARK);

        let icon = button_colors(ButtonKind::Icon, ButtonState::Hover, false, GROUND);
        assert_eq!(icon.fill, Some(crate::blend(GROUND, TEXT_LIGHT, ICON_HOVER_LIGHT)));
        assert_eq!(icon.border, None);
        let icon_down = button_colors(ButtonKind::Icon, ButtonState::Pressed, true, BG_DARK);
        assert_eq!(icon_down.fill, Some(crate::blend(BG_DARK, TEXT_DARK, ICON_PRESSED_DARK)));

        let link = button_colors(ButtonKind::Link, ButtonState::Normal, false, GROUND);
        assert_eq!(link.fill, None);
        assert!(!link.underline);
        let link_hot = button_colors(ButtonKind::Link, ButtonState::Hover, false, GROUND);
        assert!(link_hot.underline);
        assert_eq!(link_hot.ink, ACCENT);
        let link_down = button_colors(ButtonKind::Link, ButtonState::Pressed, true, BG_DARK);
        assert_eq!(link_down.ink, ACCENT_PRESSED);
        assert!(link_down.underline);

        let faded = button_colors(ButtonKind::Primary, ButtonState::Disabled, false, GROUND);
        assert_eq!(faded.fill, Some(crate::blend(GROUND, ACCENT, DISABLED_ALPHA)));
        assert!(!faded.underline);
    }

    #[test]
    fn pointer_hover_press_release_and_leave() {
        let mut t = Tracker::new();
        assert_eq!(t.pointer_move(Some(1)), TrackEffect::Repaint);
        assert_eq!(t.visual(1, false).state, ButtonState::Hover);
        assert_eq!(t.pointer_leave(), TrackEffect::Repaint);
        assert_eq!(t.visual(1, false).state, ButtonState::Normal);
        assert_eq!(t.pointer_leave(), TrackEffect::None);

        assert_eq!(t.pointer_down(1), TrackEffect::Repaint);
        assert_eq!(t.visual(1, false).state, ButtonState::Pressed);
        assert!(t.visual(1, false).focus);
        // Drag outside: pressed look drops, capture stays, release cancels.
        assert_eq!(t.pointer_move(None), TrackEffect::Repaint);
        assert_eq!(t.visual(1, false).state, ButtonState::Normal);
        assert_eq!(t.pointer_up(None), TrackEffect::Cancel);
        assert!(t.focus_cleared_down());

        assert_eq!(t.pointer_down(2), TrackEffect::Repaint);
        assert_eq!(t.pointer_move(Some(2)), TrackEffect::None);
        assert_eq!(t.pointer_up(Some(2)), TrackEffect::Click(2));
        assert_eq!(t.visual(2, false).state, ButtonState::Hover);
        assert!(t.visual(2, false).focus);
        // Capture ends after the click was already taken. Hover and focus stay.
        assert_eq!(t.capture_lost(), TrackEffect::None);
        assert!(t.visual(2, false).focus);
    }

    #[test]
    fn leave_while_pressed_still_clicks_when_release_is_over_the_button() {
        let mut t = Tracker::new();
        t.pointer_down(7);
        assert_eq!(t.pointer_leave(), TrackEffect::Repaint);
        assert_eq!(t.visual(7, false).state, ButtonState::Normal);
        assert_eq!(t.pointer_up(Some(7)), TrackEffect::Click(7));
    }

    #[test]
    fn lost_capture_cancels_and_clears_stuck_hover() {
        let mut t = Tracker::<u8>::new();
        t.pointer_down(3);
        t.pointer_move(Some(3));
        assert_eq!(t.capture_lost(), TrackEffect::Cancel);
        assert_eq!(t.visual(3, false).state, ButtonState::Normal);
        assert!(!t.visual(3, false).focus);
        assert_eq!(t.pointer_up(Some(3)), TrackEffect::None);
    }

    #[test]
    fn a_captured_press_does_not_hover_a_different_button() {
        let mut t = Tracker::new();
        t.pointer_down(1);
        t.pointer_move(Some(2));
        assert_eq!(t.visual(1, false).state, ButtonState::Normal);
        assert_eq!(t.visual(2, false).state, ButtonState::Normal);
        assert_eq!(t.pointer_up(Some(2)), TrackEffect::Cancel);
    }

    #[test]
    fn disabled_ignores_hover_and_focus() {
        let mut t = Tracker::new();
        t.pointer_move(Some(1));
        t.pointer_down(1);
        let v = t.visual(1, true);
        assert_eq!(v.state, ButtonState::Disabled);
        assert!(!v.focus);
    }

    impl<T: Copy + Eq> Tracker<T> {
        fn focus_cleared_down(&self) -> bool {
            self.down.is_none()
        }
    }
}
