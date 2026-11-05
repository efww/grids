use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;

use crate::imports::*;
use crate::app::*;
use crate::pane::*;
use crate::i18n::*;

// ---------- fonts ----------

pub(crate) fn logfont(face: &str, px: i32) -> LOGFONTW {
    let mut lf = LOGFONTW { lfHeight: -px, lfCharSet: DEFAULT_CHARSET, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    lf
}

fn utf16_face(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len()).min(31);
    String::from_utf16_lossy(&buf[..n])
}

/// LOGFONT stores 31 UTF-16 units plus a NUL. Longer names cannot be selected.
fn fits_face(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && !name.starts_with('@') && name.encode_utf16().nth(31).is_none()
}

pub(crate) fn same_face(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    a.eq_ignore_ascii_case(b) || a.chars().flat_map(|c| c.to_lowercase()).eq(b.chars().flat_map(|c| c.to_lowercase()))
}

/// `stored` when it is in `installed`, otherwise the language default. Empty stored is the default.
/// `installed` is the caller's list. This does not ask Windows.
pub(crate) fn resolve_face<'a>(stored: &str, lang: u8, installed: &'a [&'a str]) -> &'a str {
    let stored = stored.trim();
    if fits_face(stored) {
        if let Some(hit) = installed.iter().copied().find(|n| same_face(n, stored)) {
            return hit;
        }
    }
    default_face(lang)
}

struct FaceHit {
    name: String,
    proportional: bool,
    raster: bool,
}

fn face_rank(h: &FaceHit) -> u8 {
    // Prefer a proportional outline over a fixed-pitch or raster duplicate of the same name.
    u8::from(h.proportional) << 1 | u8::from(!h.raster)
}

fn face_key(name: &str) -> Vec<char> {
    name.trim().chars().flat_map(|c| c.to_lowercase()).collect()
}

/// Families for the Settings list: no '@' names, one row per name, proportional outline wins a duplicate.
fn family_names(hits: &[FaceHit]) -> Vec<String> {
    let mut best: Vec<FaceHit> = Vec::new();
    for h in hits {
        if !fits_face(&h.name) {
            continue;
        }
        let key = face_key(&h.name);
        if let Some(i) = best.iter().position(|e| face_key(&e.name) == key) {
            if face_rank(h) > face_rank(&best[i]) {
                best[i] = FaceHit { name: h.name.trim().to_string(), proportional: h.proportional, raster: h.raster };
            }
            continue;
        }
        best.push(FaceHit { name: h.name.trim().to_string(), proportional: h.proportional, raster: h.raster });
    }
    best.sort_by(|a, b| face_key(&a.name).cmp(&face_key(&b.name)));
    best.into_iter().map(|h| h.name).collect()
}

struct Probe<'a> {
    want: &'a str,
    found: bool,
}

unsafe extern "system" fn probe_face(lf: *const LOGFONTW, _tm: *const TEXTMETRICW, _ty: u32, lp: LPARAM) -> i32 {
    crate::crash::catch_i32(std::panic::AssertUnwindSafe(|| unsafe { probe_face_inner(lf, _tm, _ty, lp) }))
}
unsafe fn probe_face_inner(lf: *const LOGFONTW, _tm: *const TEXTMETRICW, _ty: u32, lp: LPARAM) -> i32 {
    if lf.is_null() || lp.0 == 0 {
        return 1;
    }
    let probe = &mut *(lp.0 as *mut Probe);
    let got = utf16_face(&(*lf).lfFaceName);
    if same_face(&got, probe.want) {
        probe.found = true;
        return 0;
    }
    1
}

/// One family lookup. Does not build the Settings list.
pub(crate) unsafe fn face_installed(name: &str) -> bool {
    let name = name.trim();
    if !fits_face(name) {
        return false;
    }
    let mut lf = logfont(name, 0);
    lf.lfCharSet = DEFAULT_CHARSET;
    let mut probe = Probe { want: name, found: false };
    let dc = GetDC(None);
    if dc.is_invalid() {
        return false;
    }
    let _ = EnumFontFamiliesExW(dc, &lf, Some(probe_face), LPARAM(&mut probe as *mut Probe as isize), 0);
    ReleaseDC(None, dc);
    probe.found
}

unsafe extern "system" fn collect_face(lf: *const LOGFONTW, tm: *const TEXTMETRICW, ty: u32, lp: LPARAM) -> i32 {
    crate::crash::catch_i32(std::panic::AssertUnwindSafe(|| unsafe { collect_face_inner(lf, tm, ty, lp) }))
}
unsafe fn collect_face_inner(lf: *const LOGFONTW, tm: *const TEXTMETRICW, ty: u32, lp: LPARAM) -> i32 {
    if lf.is_null() || lp.0 == 0 {
        return 1;
    }
    let name = utf16_face(&(*lf).lfFaceName);
    if name.is_empty() {
        return 1;
    }
    let proportional = !tm.is_null() && (*tm).tmPitchAndFamily.0 & TMPF_FIXED_PITCH.0 != 0;
    let hits = &mut *(lp.0 as *mut Vec<FaceHit>);
    hits.push(FaceHit { name, proportional, raster: ty & RASTER_FONTTYPE != 0 });
    1
}

/// Installed families. Call only while Settings is open; the caller drops the `Vec` when it closes.
pub(crate) unsafe fn installed_families() -> Vec<String> {
    let lf = LOGFONTW { lfCharSet: DEFAULT_CHARSET, ..Default::default() };
    let mut hits = Vec::new();
    let dc = GetDC(None);
    if !dc.is_invalid() {
        let _ = EnumFontFamiliesExW(dc, &lf, Some(collect_face), LPARAM(&mut hits as *mut Vec<FaceHit> as isize), 0);
        ReleaseDC(None, dc);
    }
    family_names(&hits)
}

/// Ascent, descent, and internal leading for one `HFONT`. `draw` caches these and
/// centers the visible glyph box from them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LineMetrics {
    pub ascent: i32,
    pub descent: i32,
    pub internal_leading: i32,
}

impl LineMetrics {
    pub(crate) fn height(self) -> i32 {
        self.ascent.saturating_add(self.descent).max(0)
    }
}

struct MetricSlot {
    font: usize,
    metrics: LineMetrics,
}

/// Keyed by the `HFONT` value. `delete_font` drops the slot, because GDI recycles handle values.
static LINE_METRICS: Mutex<Vec<MetricSlot>> = Mutex::new(Vec::new());

fn font_key(font: HFONT) -> usize {
    font.0 as usize
}

pub(crate) fn forget_line_metrics(font: HFONT) {
    let key = font_key(font);
    if key == 0 {
        return;
    }
    if let Ok(mut slots) = LINE_METRICS.lock() {
        slots.retain(|s| s.font != key);
    }
}

/// Delete a UI font and drop its cached line metrics. Stock and null handles are left alone.
pub(crate) unsafe fn delete_font(font: HFONT) {
    if font.0.is_null() || font.is_invalid() {
        return;
    }
    forget_line_metrics(font);
    _ = DeleteObject(font.into());
}

/// `GetTextMetricsW` for `font`, remembered per `HFONT` until `delete_font`.
/// The font is selected only on a cache miss, then the DC's previous object is restored.
pub(crate) unsafe fn line_metrics(dc: HDC, font: HFONT) -> LineMetrics {
    let key = font_key(font);
    if key != 0 {
        if let Ok(slots) = LINE_METRICS.lock() {
            if let Some(slot) = slots.iter().find(|s| s.font == key) {
                return slot.metrics;
            }
        }
    }
    let old = SelectObject(dc, font.into());
    let mut tm = TEXTMETRICW::default();
    let ok = GetTextMetricsW(dc, &mut tm).as_bool() && tm.tmAscent > 0;
    if !old.is_invalid() {
        SelectObject(dc, old);
    }
    let metrics = if ok {
        LineMetrics { ascent: tm.tmAscent, descent: tm.tmDescent.max(0), internal_leading: tm.tmInternalLeading.max(0) }
    } else {
        LineMetrics { ascent: 0, descent: 0, internal_leading: 0 }
    };
    if ok && key != 0 {
        if let Ok(mut slots) = LINE_METRICS.lock() {
            if let Some(slot) = slots.iter_mut().find(|s| s.font == key) {
                slot.metrics = metrics;
            } else {
                // A font deleted without delete_font would leak a slot. Cap the list.
                if slots.len() >= 64 {
                    slots.remove(0);
                }
                slots.push(MetricSlot { font: key, metrics });
            }
        }
    }
    metrics
}

// Windows' UI font metrics at this DPI, with the language default or the face chosen in Settings.
// The MDL2 icon font stays on the buttons. Pushed to every edit; also used on DPI change.
pub(crate) unsafe fn apply_font_all() {
    across_windows(|| apply_font());
}

pub(crate) unsafe fn apply_font() {
    let dpi = app(|a| a.dpi);
    // Empty stays on the language default and does not look up a face. A missing name falls back
    // to the default for drawing but stays saved, so it comes back once the font is installed again.
    let stored = app(|a| if a.s.font_face.is_empty() { None } else { Some(a.s.font_face.clone()) });
    let custom = stored.as_deref().is_some_and(|name| face_installed(name));
    let slot = stored.as_deref().unwrap_or("");
    let list = [slot];
    let face = if custom { resolve_face(slot, ui(), &list) } else { ui_face() };
    let mut ncm = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    let sys = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, ncm.cbSize, Some(&mut ncm as *mut _ as _), 0, dpi as u32);
    let mut lf = if sys.is_ok() { ncm.lfMessageFont } else { logfont(ui_face(), 12 * dpi / 96) };
    lf.lfFaceName = logfont(face, 0).lfFaceName;
    let font = CreateFontIndirectW(&lf);
    let small = CreateFontIndirectW(&LOGFONTW { lfHeight: lf.lfHeight * 85 / 100, ..lf });
    let icon_font = CreateFontIndirectW(&logfont("Segoe MDL2 Assets", 14 * dpi / 96));
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut tm = TEXTMETRICW::default();
    _ = GetTextMetricsW(dc, &mut tm);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    let (old, old_icon, old_bold, panes, edit, load_edit) = app(|a| {
        a.fh = tm.tmHeight;
        let old_small = std::mem::replace(&mut a.small_font, small);
        if !old_small.is_invalid() {
            unsafe { delete_font(old_small) };
        }
        let old = std::mem::replace(&mut a.font, font);
        let old_icon = std::mem::replace(&mut a.icon_font, icon_font);
        // Group labels clone this face on the next paint. Drop the previous one now so a DPI
        // change cannot keep a bold font at the old size.
        let old_bold = std::mem::replace(&mut a.bold_font, HFONT::default());
        let mut panes: Vec<HWND> = a.panes.iter().flatten().map(|p| p.hwnd).collect();
        for set in &a.park {
            panes.extend(set.iter().flatten().map(|p| p.hwnd));
        }
        (old, old_icon, old_bold, panes, a.tab_edit, a.load_edit)
    });
    let match_edit = app(|a| a.matcher.as_ref().map(|m| m.edit).unwrap_or_default());
    for e in [edit, load_edit, match_edit] {
        if !e.0.is_null() {
            SendMessageW(e, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        }
    }
    for p in panes {
        for id in [ID_EDIT, ID_FILTER] {
            if let Ok(c) = GetDlgItem(Some(p), id) {
                SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
        }
    }
    for f in [old, old_icon, old_bold].into_iter().filter(|f| !f.is_invalid()) {
        delete_font(f);
    }
    // The folder tree takes this font, so its item height (and the favourites rows) follow DPI.
    // Preview headings were cloned from the previous face; the next paint builds them again.
    drop_peek_fonts();
    super::sync_fav_metrics();
    relayout();
    // A font or language change does not resize the panes, so WM_SIZE never recentres the edits.
    refit_text_edits();
    redraw_all();
}

/// `tmHeight` of the font the control is using, measured in its own DC.
/// Includes internal leading. The font was created for this window's DPI.
unsafe fn edit_line_height(h: HWND) -> Option<i32> {
    if h.0.is_null() {
        return None;
    }
    let font = HFONT(SendMessageW(h, WM_GETFONT, None, None).0 as _);
    let dc = GetDC(Some(h));
    if dc.is_invalid() {
        return None;
    }
    let chosen = if font.0.is_null() { GetStockObject(SYSTEM_FONT) } else { HGDIOBJ(font.0) };
    let old = SelectObject(dc, chosen);
    let mut tm = TEXTMETRICW::default();
    let ok = GetTextMetricsW(dc, &mut tm).as_bool();
    if !old.is_invalid() {
        SelectObject(dc, old);
    }
    let _ = ReleaseDC(Some(h), dc);
    (ok && tm.tmHeight > 0).then_some(tm.tmHeight)
}

/// Client rectangle of a single-line EDIT centred in `outer`.
///
/// `line_height` is `TEXTMETRICW.tmHeight` (the line cell, internal leading included).
/// A single-line EDIT draws that cell from the top of its client, so the client is one
/// line tall and its top is `outer.top + (outer_height - line_height) / 2`. The painted
/// frame stays `outer`. A line taller than the frame is clipped to it. A non-positive
/// line height leaves `outer` unchanged.
pub(crate) fn centered_edit_rect(outer: RECT, line_height: i32) -> RECT {
    let box_h = outer.bottom - outer.top;
    if outer.right <= outer.left || box_h <= 0 {
        return outer;
    }
    let line = if line_height > 0 { line_height.min(box_h) } else { box_h };
    let top = outer.top + (box_h - line) / 2;
    RECT { left: outer.left, top, right: outer.right, bottom: top + line }
}

/// Window rect whose client area is `centered_edit_rect`. Borderless edits are unchanged.
/// A `WS_BORDER` edit grows by the DPI-scaled non-client frame so the client, not the
/// border, is the line cell. The name prompt is borderless and paints its own frame.
pub(crate) unsafe fn edit_window_rect(h: HWND, outer: RECT) -> RECT {
    if h.0.is_null() {
        return outer;
    }
    let client = centered_edit_rect(outer, edit_line_height(h).unwrap_or(0));
    let w = client.right - client.left;
    let ht = client.bottom - client.top;
    if w <= 0 || ht <= 0 {
        return client;
    }
    let style = WINDOW_STYLE(GetWindowLongPtrW(h, GWL_STYLE) as u32);
    let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(h, GWL_EXSTYLE) as u32);
    let dpi = GetDpiForWindow(h).max(96);
    let mut adj = RECT { left: 0, top: 0, right: w, bottom: ht };
    if AdjustWindowRectExForDpi(&mut adj, style, false, ex, dpi).is_err() {
        return client;
    }
    RECT { left: client.left + adj.left, top: client.top + adj.top, right: client.left + adj.right, bottom: client.top + adj.bottom }
}

/// Place a single-line EDIT in `outer` (parent client coordinates). `show` false hides it.
pub(crate) unsafe fn place_edit(h: HWND, outer: RECT, show: bool) {
    if h.0.is_null() {
        return;
    }
    if !show {
        let _ = ShowWindow(h, SW_HIDE);
        return;
    }
    let r = edit_window_rect(h, outer);
    let _ = SetWindowPos(h, None, r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1), SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

/// Edits already on screen, after the UI font changes. Panes also move on WM_SIZE.
unsafe fn refit_text_edits() {
    let panes: Vec<HWND> = app(|a| a.panes.iter().flatten().map(|p| p.hwnd).collect());
    for h in panes {
        layout_pane(h);
    }
    super::place_tab_edit();
    refit_match();
    crate::fit_load();
}

static FALLBACK_BODY: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static FALLBACK_ICON: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

unsafe fn fallback_font(slot: &std::sync::atomic::AtomicIsize, face: &str, px: i32, weight: i32) -> HFONT {
    let cur = HFONT(slot.load(Ordering::Relaxed) as *mut _);
    if !cur.0.is_null() && !cur.is_invalid() {
        return cur;
    }
    let mut lf = logfont(face, px);
    lf.lfWeight = weight;
    let font = CreateFontIndirectW(&lf);
    if !font.is_invalid() {
        slot.store(font.0 as isize, Ordering::Relaxed);
    }
    font
}

/// The app UI face. Dialogs use this instead of creating their own fonts.
pub(crate) unsafe fn body_font() -> HFONT {
    if let Some(font) = crate::app_try(|a| a.font) {
        if !font.0.is_null() && !font.is_invalid() {
            return font;
        }
    }
    fallback_font(&FALLBACK_BODY, crate::ui_face(), super::FONT_PX, 400)
}

/// Segoe MDL2 Assets, for checks and the other chrome glyphs.
pub(crate) unsafe fn icon_font() -> HFONT {
    if let Some(font) = crate::app_try(|a| a.icon_font) {
        if !font.0.is_null() && !font.is_invalid() {
            return font;
        }
    }
    fallback_font(&FALLBACK_ICON, "Segoe MDL2 Assets", 14, 400)
}

/// Bold copy of the window's UI font. Built once, the first time a group row is painted,
/// and thrown away by `apply_font` (a DPI or face change) or when the window closes.
pub(crate) unsafe fn bold_ui_font() -> HFONT {
    let cached = app(|a| a.bold_font);
    if !cached.is_invalid() {
        return cached;
    }
    let base = app(|a| a.font);
    if base.is_invalid() {
        return base;
    }
    let mut lf = LOGFONTW::default();
    let n = GetObjectW(base.into(), size_of::<LOGFONTW>() as i32, Some(&mut lf as *mut LOGFONTW as *mut _));
    if n < size_of::<LOGFONTW>() as i32 {
        return base;
    }
    lf.lfWeight = FW_BOLD.0 as i32;
    let font = CreateFontIndirectW(&lf);
    if font.is_invalid() {
        return base;
    }
    let stored = app(|a| {
        if a.bold_font.is_invalid() {
            a.bold_font = font;
            font
        } else {
            a.bold_font
        }
    });
    if stored != font {
        delete_font(font);
    }
    stored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_face_falls_back_to_the_language_default() {
        let installed = ["Segoe UI", "Malgun Gothic", "Yu Gothic UI", "Consolas"];
        assert_eq!(resolve_face("", 0, &installed), "Segoe UI");
        assert_eq!(resolve_face("   ", 1, &installed), "Malgun Gothic");
        assert_eq!(resolve_face("Nope", 2, &installed), "Yu Gothic UI");
        assert_eq!(resolve_face("consolas", 0, &installed), "Consolas");
        assert_eq!(resolve_face("Yu Gothic UI", 1, &installed), "Yu Gothic UI");
        assert_eq!(resolve_face("@Yu Gothic UI", 2, &installed), "Yu Gothic UI");
        assert_eq!(crate::default_face(2), "Yu Gothic UI");
    }

    #[test]
    fn family_list_skips_vertical_and_nonproportional_duplicates() {
        let hits = [
            FaceHit { name: "@Arial".into(), proportional: true, raster: false },
            FaceHit { name: "Arial".into(), proportional: false, raster: true },
            FaceHit { name: "Arial".into(), proportional: true, raster: false },
            FaceHit { name: "consolas".into(), proportional: false, raster: false },
            FaceHit { name: "Consolas".into(), proportional: false, raster: false },
            FaceHit { name: "Yu Gothic UI".into(), proportional: true, raster: false },
            FaceHit { name: String::new(), proportional: true, raster: false },
        ];
        assert_eq!(family_names(&hits), ["Arial".to_string(), "consolas".into(), "Yu Gothic UI".into()]);
    }

    #[test]
    fn edit_line_is_centred_in_the_frame() {
        // 32px settings-search pill. Line cells stand in for Segoe UI, Malgun Gothic and Yu Gothic UI.
        let frame = RECT { left: 8, top: 12, right: 212, bottom: 44 };
        for line in [15, 16, 19, 21] {
            let r = centered_edit_rect(frame, line);
            assert_eq!(r.bottom - r.top, line);
            assert_eq!(r.top, frame.top + (32 - line) / 2);
            assert_eq!((r.left, r.right), (frame.left, frame.right));
            assert!(r.top >= frame.top && r.bottom <= frame.bottom);
        }
    }

    #[test]
    fn edit_centring_scales_with_dpi_and_keeps_the_odd_pixel_below() {
        for dpi in [96, 120, 144, 192] {
            let s = |v: i32| v * dpi / 96;
            let frame = RECT { left: s(8), top: s(12), right: s(212), bottom: s(44) };
            let box_h = frame.bottom - frame.top;
            for px in [15, 19] {
                let line = (px * dpi / 96).max(1);
                let r = centered_edit_rect(frame, line);
                let used = line.min(box_h);
                assert_eq!(r.bottom - r.top, used);
                assert_eq!(r.top, frame.top + (box_h - used) / 2);
                assert!(r.bottom <= frame.bottom);
            }
        }
        let frame = RECT { left: 0, top: 0, right: 10, bottom: 21 };
        let r = centered_edit_rect(frame, 16);
        assert_eq!((r.top, r.bottom), (2, 18));
    }

    #[test]
    fn edit_taller_than_the_frame_or_an_empty_box_stays_put() {
        let frame = RECT { left: 4, top: 8, right: 40, bottom: 20 };
        assert_eq!(centered_edit_rect(frame, 100), frame);
        assert_eq!(centered_edit_rect(frame, 0), frame);
        assert_eq!(centered_edit_rect(frame, -3), frame);
        let empty = RECT { left: 3, top: 1, right: 3, bottom: 9 };
        assert_eq!(centered_edit_rect(empty, 4), empty);
        let flat = RECT { left: 1, top: 5, right: 9, bottom: 4 };
        assert_eq!(centered_edit_rect(flat, 8), flat);
    }

    #[test]
    fn installed_families_are_unique_sorted_and_not_vertical() {
        let names = unsafe { installed_families() };
        let mut sorted = names.clone();
        sorted.sort_by(|a, b| face_key(a).cmp(&face_key(b)));
        assert_eq!(names, sorted);
        let mut seen = Vec::new();
        for n in &names {
            assert!(fits_face(n), "{n}");
            let key = face_key(n);
            assert!(!seen.contains(&key), "duplicate {n}");
            seen.push(key);
        }
    }
}

