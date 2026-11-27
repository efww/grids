use crate::*;

// ---------- fonts ----------

pub(crate) fn logfont(face: &str, px: i32) -> LOGFONTW {
    let mut lf = LOGFONTW { lfHeight: -px, lfCharSet: DEFAULT_CHARSET, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    lf
}

const FLUENT_ICONS: &str = "Segoe Fluent Icons";
const MDL2_ICONS: &str = "Segoe MDL2 Assets";

// The icon font to keep, given the face GDI actually picked for a Segoe Fluent Icons request: GDI quietly
// substitutes a missing font (Windows 10), and that one would draw boxes, so only an exact match counts.
pub(crate) fn icon_face(found: &str) -> &'static str {
    if found.trim_end_matches('\0').eq_ignore_ascii_case(FLUENT_ICONS) {
        FLUENT_ICONS
    } else {
        MDL2_ICONS
    }
}

// The UI font as Windows' settings make it now at this DPI: the message font's size and weight (Settings >
// Accessibility > Text size moves it), the face always Segoe UI.
pub(crate) unsafe fn ui_logfont(dpi: i32) -> LOGFONTW {
    let mut ncm = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    let sys = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, ncm.cbSize, Some(&mut ncm as *mut _ as _), 0, dpi as u32);
    let mut lf = if sys.is_ok() { ncm.lfMessageFont } else { logfont("Segoe UI", 12 * dpi / 96) };
    lf.lfFaceName = logfont("Segoe UI", 0).lfFaceName; // system size, but the Korean UI font draws '\\' in paths as '₩'
    lf
}

// How many px taller than at the default text size the bars must be for this UI font (lf_height its LOGFONT height,
// tm_height the measured tmHeight). The default message font is 12 px at 96 DPI (tmHeight 16): up to that, rounded
// for the DPI, it is 0 whatever GDI's rounding measures (or ui_logfont's fallback), so no bar moves by a stray pixel;
// a bigger Text size counts what its line is taller than the default 16 px one.
pub(crate) fn text_grow(lf_height: i32, tm_height: i32, dpi: i32) -> i32 {
    if lf_height.abs() <= (12 * dpi + 48) / 96 {
        0
    } else {
        (tm_height - 16 * dpi / 96).max(0)
    }
}

// Whether the font in use (old) and the one Windows' settings now ask for (new) draw differently. Compared
// after ui_logfont's face override, so a Korean message font named Malgun Gothic doesn't count as a change.
pub(crate) fn font_changed(old: &LOGFONTW, new: &LOGFONTW) -> bool {
    let face = |lf: &LOGFONTW| lf.lfFaceName.iter().take_while(|&&c| c != 0).copied().collect::<Vec<u16>>();
    old.lfHeight != new.lfHeight || old.lfWeight != new.lfWeight || old.lfItalic != new.lfItalic || face(old) != face(new)
}

// A WM_SETTINGCHANGE that may carry a new UI font size: the window metrics (Text size, "Make text bigger")
// or the icon title font. Light/dark ("ImmersiveColorSet") and the rest are not ours to refont for.
pub(crate) fn metrics_note(wp: usize, area: Option<&str>) -> bool {
    wp == SPI_SETNONCLIENTMETRICS.0 as usize || wp == SPI_SETICONTITLELOGFONT.0 as usize || area == Some("WindowMetrics")
}

// Windows' own UI font at this DPI (no custom fonts: they only reached our chrome and cost paint time),
// plus the icon font for the buttons: Segoe Fluent Icons where Windows 11 has it, so they match Explorer,
// else Segoe MDL2 Assets (same code points). Pushed to every edit; also used on DPI change.
pub(crate) unsafe fn apply_font() {
    let dpi = app(|a| a.dpi);
    let lf = ui_logfont(dpi);
    let font = CreateFontIndirectW(&lf);
    let small = CreateFontIndirectW(&LOGFONTW { lfHeight: lf.lfHeight * 85 / 100, ..lf });
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut tm = TEXTMETRICW::default();
    _ = GetTextMetricsW(dc, &mut tm);
    let fluent = CreateFontIndirectW(&logfont(FLUENT_ICONS, 14 * dpi / 96));
    SelectObject(dc, fluent.into());
    let mut face = [0u16; 32];
    let n = GetTextFaceW(dc, Some(&mut face)).clamp(0, face.len() as i32) as usize;
    SelectObject(dc, old); // deselect before deleting, or the delete fails and leaks on every DPI change
    ReleaseDC(None, dc);
    let icon_font = if icon_face(&String::from_utf16_lossy(&face[..n])) == FLUENT_ICONS {
        fluent
    } else {
        if !fluent.is_invalid() {
            _ = DeleteObject(fluent.into());
        }
        CreateFontIndirectW(&logfont(MDL2_ICONS, 14 * dpi / 96))
    };
    let (old, old_icon, panes, load_edit) = app(|a| {
        a.fh = tm.tmHeight;
        TEXT_GROW.store(text_grow(lf.lfHeight, tm.tmHeight, dpi), Ordering::Relaxed);
        let old_small = std::mem::replace(&mut a.small_font, small);
        if !old_small.is_invalid() {
            unsafe { _ = DeleteObject(old_small.into()) };
        }
        let old = std::mem::replace(&mut a.font, font);
        let old_icon = std::mem::replace(&mut a.icon_font, icon_font);
        (old, old_icon, a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>(), a.load_edit)
    });
    if !load_edit.0.is_null() {
        SendMessageW(load_edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    }
    for &p in &panes {
        for id in [ID_EDIT, ID_FILTER] {
            if let Ok(c) = GetDlgItem(Some(p), id) {
                SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
        }
    }
    rename_refont(); // the batch rename window stays open while the main window moves: off the old font first
    peek::refont(); // and the preview: its bold / code / title fonts come from the old one
    for f in [old, old_icon].into_iter().filter(|f| !f.is_invalid()) {
        _ = DeleteObject(f.into());
    }
    relayout();
    // the pane bars grew or shrank with the text: a pane whose height came out the same gets no WM_SIZE, so every
    // pane lays out its fields and list here (as toggle_tabs does)
    for p in panes {
        layout_pane(p);
    }
    redraw_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_face_pick() {
        assert_eq!(icon_face("Segoe Fluent Icons"), "Segoe Fluent Icons");
        assert_eq!(icon_face("segoe FLUENT icons"), "Segoe Fluent Icons"); // case
        assert_eq!(icon_face("Segoe Fluent Icons\0"), "Segoe Fluent Icons"); // trailing NUL
        assert_eq!(icon_face(""), "Segoe MDL2 Assets");
        assert_eq!(icon_face("Arial"), "Segoe MDL2 Assets"); // GDI's substitute on Windows 10
        assert_eq!(icon_face("Segoe MDL2 Assets"), "Segoe MDL2 Assets");
        assert_eq!(icon_face("Segoe Fluent"), "Segoe MDL2 Assets");
    }

    #[test]
    fn text_grow_only_past_default() {
        // the default text size gives 0 at any DPI, also when GDI measures a pixel more, or on ui_logfont's fallback
        for dpi in [96, 106, 120, 144, 168, 192, 240] {
            let lf = (12 * dpi + 48) / 96;
            for tm in [16 * dpi / 96, 16 * dpi / 96 + 1, 16 * dpi / 96 + 2] {
                assert_eq!(text_grow(-lf, tm, dpi), 0, "{dpi} {tm}");
            }
            assert_eq!(text_grow(logfont("Segoe UI", 12 * dpi / 96).lfHeight, 16 * dpi / 96 + 1, dpi), 0, "{dpi}");
        }
        // Text size 125 .. 225 % at 96 DPI: what the line is taller than the default 16 px one
        assert_eq!(text_grow(-15, 20, 96), 4);
        assert_eq!(text_grow(-18, 24, 96), 8);
        assert_eq!(text_grow(-24, 32, 96), 16);
        assert_eq!(text_grow(-27, 36, 96), 20);
        assert_eq!(text_grow(27, 36, 96), 20); // a cell height (positive) counts the same
        assert_eq!(text_grow(-36, 48, 144), 24); // 200 % on a 150 % screen
        assert_eq!(text_grow(-14, 15, 96), 0); // never negative
    }

    #[test]
    fn font_change_seen() {
        let base = logfont("Segoe UI", 12);
        assert!(!font_changed(&base, &base)); // same font: another metric (scrollbar width) moved
        assert!(font_changed(&base, &logfont("Segoe UI", 15))); // Text size raised
        assert!(font_changed(&base, &logfont("Arial", 12))); // another face
        assert!(font_changed(&base, &LOGFONTW { lfWeight: 700, ..base }));
        let mut junk = base; // garbage past the terminating NUL is not a different face
        junk.lfFaceName[20] = b'x' as u16;
        assert!(!font_changed(&base, &junk));
    }

    #[test]
    fn metrics_note_pick() {
        assert!(metrics_note(SPI_SETNONCLIENTMETRICS.0 as usize, None));
        assert!(metrics_note(SPI_SETNONCLIENTMETRICS.0 as usize, Some("WindowMetrics")));
        assert!(metrics_note(SPI_SETICONTITLELOGFONT.0 as usize, None));
        assert!(metrics_note(0, Some("WindowMetrics")));
        assert!(!metrics_note(0, Some("ImmersiveColorSet")));
        assert!(!metrics_note(SPI_SETHIGHCONTRAST.0 as usize, None));
        assert!(!metrics_note(0, Some("intl")));
        assert!(!metrics_note(0, None));
    }
}
