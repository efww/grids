use std::time::{SystemTime, UNIX_EPOCH};

use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;

use crate::imports::*;
use super::*;
use crate::app::*;
use crate::i18n::*;

// ---------- session list: name and date on the left, a layout preview on the right ----------
// Drawn like Settings (same fonts, cards, field, accent buttons). Not a stock list box.

const ID_OPEN: i32 = 1;
const ID_CANCEL: i32 = 2;
const ID_SAVE: i32 = 3;
const ID_RENAME: i32 = 4;
const ID_DELETE: i32 = 5;
const ID_SEVERAL: i32 = 6;
const ID_SEARCH: i32 = 10;
const EM_SETCUEBANNER: u32 = 0x1501;

thread_local! {
    static OUT: RefCell<Option<Option<Vec<PathBuf>>>> = const { RefCell::new(None) };
    static SAVE_ASKED: Cell<bool> = const { Cell::new(false) };
    static UI: RefCell<PickUi> = const { RefCell::new(PickUi::empty()) };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionEntry {
    pub name: String,
    pub path: PathBuf,
    pub modified: SystemTime,
}

/// One tab inside a session file: panes per column, and each pane's folder in that order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabPreview {
    pub name: String,
    /// Panes in each column, left to right. `columns.len()` is the column count.
    pub columns: Vec<usize>,
    /// Folder of each pane, column by column, top to bottom. Empty means that pane has no folder.
    pub paths: Vec<String>,
}

/// What the detail pane can show. `readable` is false for a missing read, bad bytes, or a file
/// that is not a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionPreview {
    pub readable: bool,
    pub tabs: Vec<TabPreview>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DetailKind {
    Title,
    Meta,
    TabHead,
    Schematic,
    Path,
    Gap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetailLine {
    pub kind: DetailKind,
    pub text: String,
    pub columns: Vec<usize>,
    pub h: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BoxPx {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl BoxPx {
    fn w(self) -> i32 {
        self.right - self.left
    }
    fn h(self) -> i32 {
        self.bottom - self.top
    }
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
    fn rect(self) -> RECT {
        RECT { left: self.left, top: self.top, right: self.right, bottom: self.bottom }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ButtonWidths {
    pub open: i32,
    pub rename: i32,
    pub delete: i32,
    pub cancel: i32,
    pub save: i32,
    pub several: i32,
}

pub(crate) struct PickLayout {
    pub search: BoxPx,
    pub list: BoxPx,
    pub detail: BoxPx,
    pub open: BoxPx,
    pub rename: BoxPx,
    pub delete: BoxPx,
    pub cancel: BoxPx,
    pub save: Option<BoxPx>,
    pub several: Option<BoxPx>,
    pub row_h: i32,
    pub rows: Vec<BoxPx>,
    pub scroll: i32,
    pub scroll_max: i32,
    pub detail_scroll: i32,
    pub detail_max: i32,
}

/// `YYYY-MM-DD HH:MM`. The window fills the numbers from the file's local time.
pub(crate) fn format_civil(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> String {
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

fn dip(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

/// Session files in `dir`, most recently changed first. The name is the file stem.
pub(crate) fn session_list(dir: &Path) -> Vec<SessionEntry> {
    let mut v: Vec<SessionEntry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_session_file(p))
        .filter_map(|p| {
            let modified = std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
            let name = p.file_stem()?.to_string_lossy().into_owned();
            Some(SessionEntry { name, path: p, modified })
        })
        .collect();
    v.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

fn tab_preview(t: &Tab) -> TabPreview {
    let columns: Vec<usize> = t.heights.iter().copied().filter(|&n| n > 0).collect();
    let columns = if columns.is_empty() { vec![1] } else { columns };
    let n: usize = columns.iter().sum();
    let paths = (0..n).map(|i| t.paths.get(i).cloned().unwrap_or_default()).collect();
    TabPreview { name: t.name.trim().to_string(), columns, paths }
}

/// Tabs, columns, and pane folders from session text. Unreadable text has `readable: false` and no tabs.
pub(crate) fn preview_summary(text: &str) -> SessionPreview {
    let text = text.trim_start_matches('\u{feff}').trim();
    if !looks_like_session(text) {
        return SessionPreview { readable: false, tabs: Vec::new() };
    }
    let tabs = Session::parse(text).view_tabs().into_iter().map(|t| tab_preview(&t)).collect();
    SessionPreview { readable: true, tabs }
}

/// Bytes on disk. Invalid UTF-8 is unreadable, not a best-effort decode.
pub(crate) fn preview_bytes(bytes: &[u8]) -> SessionPreview {
    match std::str::from_utf8(bytes) {
        Ok(t) => preview_summary(t),
        Err(_) => SessionPreview { readable: false, tabs: Vec::new() },
    }
}

/// Search box: session name, tab name, or any pane folder. An empty query keeps every row.
pub(crate) fn preview_matches(name: &str, preview: &SessionPreview, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    if name.to_lowercase().contains(&q) {
        return true;
    }
    preview.tabs.iter().any(|t| t.name.to_lowercase().contains(&q) || t.paths.iter().any(|p| p.to_lowercase().contains(&q)))
}

fn tab_count_line(n: usize) -> String {
    if n == 1 {
        tr(crate::Key::K1Tab).to_string()
    } else {
        format!("{n}{}", tr(crate::Key::Tabs))
    }
}

fn tab_heading(name: &str, index: usize) -> String {
    match custom_tab_name(name) {
        Some(n) => n.to_string(),
        None => format!("{} {}", tr(crate::Key::Tab), index + 1),
    }
}

fn path_line(index: usize, path: &str) -> String {
    // Shell folders (::{CLSID}) read as their name, e.g. This PC, not the raw id.
    let shown = if path.is_empty() {
        tr(crate::Key::Empty2).to_string()
    } else if path.starts_with("::") {
        crate::short_name(path)
    } else {
        path.to_string()
    };
    format!("{}   {shown}", index + 1)
}

/// Lines the detail pane paints, top to bottom. Heights are DIP scaled by `dpi`.
pub(crate) fn detail_plan(name: &str, preview: &SessionPreview, dpi: i32) -> Vec<DetailLine> {
    let dpi = dpi.max(96);
    let mut lines = vec![
        DetailLine { kind: DetailKind::Title, text: name.to_string(), columns: Vec::new(), h: dip(dpi, 28) },
        DetailLine {
            kind: DetailKind::Meta,
            text: if preview.readable {
                if preview.tabs.len() <= 1 {
                    preview.tabs.first().map(|t| layout_label(&t.columns)).filter(|s| !s.is_empty()).unwrap_or_else(|| tr(crate::Key::OneTab).to_string())
                } else {
                    tab_count_line(preview.tabs.len())
                }
            } else {
                tr(crate::Key::ThisSessionCanTBeRead).to_string()
            },
            columns: Vec::new(),
            h: dip(dpi, 22),
        },
    ];
    if !preview.readable {
        return lines;
    }
    for (i, tab) in preview.tabs.iter().enumerate() {
        lines.push(DetailLine { kind: DetailKind::Gap, text: String::new(), columns: Vec::new(), h: dip(dpi, 10) });
        // A session is one tab. The window title already names it, so an automatic "Tab 1" is skipped.
        if preview.tabs.len() != 1 || custom_tab_name(&tab.name).is_some() {
            let head = format!("{}   {}", tab_heading(&tab.name, i), layout_label(&tab.columns));
            lines.push(DetailLine { kind: DetailKind::TabHead, text: head, columns: Vec::new(), h: dip(dpi, 22) });
        }
        lines.push(DetailLine { kind: DetailKind::Schematic, text: String::new(), columns: tab.columns.clone(), h: dip(dpi, 72) });
        if tab.paths.is_empty() {
            lines.push(DetailLine { kind: DetailKind::Path, text: path_line(0, ""), columns: Vec::new(), h: dip(dpi, 18) });
        } else {
            for (k, path) in tab.paths.iter().enumerate() {
                lines.push(DetailLine { kind: DetailKind::Path, text: path_line(k, path), columns: Vec::new(), h: dip(dpi, 18) });
            }
        }
    }
    lines
}

/// Where each piece sits. `scroll` and `detail_scroll` are clamped to what the content needs.
pub(crate) fn pick_layout(w: i32, h: i32, dpi: i32, row_count: usize, scroll: i32, detail_h: i32, detail_scroll: i32, with_save: bool, widths: ButtonWidths) -> PickLayout {
    let dpi = dpi.max(96);
    let pad = dip(dpi, 16);
    let gap = dip(dpi, 12);
    let search_h = dip(dpi, 32);
    let btn_h = dip(dpi, 32);
    let row_h = dip(dpi, 52);
    let row_gap = dip(dpi, 4);
    let inner = dip(dpi, 8);
    let search = BoxPx { left: pad, top: pad, right: w - pad, bottom: pad + search_h };
    let btn_top = h - pad - btn_h;
    let body_top = search.bottom + gap;
    let body_bot = (btn_top - gap).max(body_top + dip(dpi, 80));
    let list_w = dip(dpi, 280).min((w - pad * 3) / 2).max(dip(dpi, 160));
    let list = BoxPx { left: pad, top: body_top, right: pad + list_w, bottom: body_bot };
    let detail = BoxPx { left: list.right + gap, top: body_top, right: w - pad, bottom: body_bot };
    let cancel = BoxPx { left: w - pad - widths.cancel, top: btn_top, right: w - pad, bottom: btn_top + btn_h };
    let open = BoxPx { left: cancel.left - gap - widths.open, top: btn_top, right: cancel.left - gap, bottom: btn_top + btn_h };
    let several = (widths.several > 0).then(|| {
        let right = open.left - gap;
        BoxPx { left: right - widths.several, top: btn_top, right, bottom: btn_top + btn_h }
    });
    let delete = BoxPx { left: pad, top: btn_top, right: pad + widths.delete, bottom: btn_top + btn_h };
    let rename = BoxPx { left: delete.right + gap, top: btn_top, right: delete.right + gap + widths.rename, bottom: btn_top + btn_h };
    let save = with_save.then(|| {
        let left = rename.right + gap;
        BoxPx { left, top: btn_top, right: left + widths.save, bottom: btn_top + btn_h }
    });
    let stride = row_h + row_gap;
    let content = if row_count == 0 { 0 } else { inner * 2 + row_count as i32 * row_h + (row_count as i32 - 1) * row_gap };
    let scroll_max = (content - list.h()).max(0);
    let scroll = scroll.clamp(0, scroll_max);
    let mut y = list.top + inner - scroll;
    let rows = (0..row_count)
        .map(|_| {
            let row = BoxPx { left: list.left + inner, top: y, right: list.right - inner, bottom: y + row_h };
            y += stride;
            row
        })
        .collect();
    let detail_max = (detail_h - (detail.h() - inner * 2)).max(0);
    PickLayout {
        search,
        list,
        detail,
        open,
        rename,
        delete,
        cancel,
        save,
        several,
        row_h,
        rows,
        scroll,
        scroll_max,
        detail_scroll: detail_scroll.clamp(0, detail_max),
        detail_max,
    }
}

/// Keep row `row_y` (content coordinates) inside a viewport of `view_h` that is scrolled by `scroll`.
pub(crate) fn reveal_row(scroll: i32, scroll_max: i32, view_h: i32, row_y: i32, row_h: i32) -> i32 {
    let mut s = scroll;
    if row_y < s {
        s = row_y;
    } else if row_y + row_h > s + view_h {
        s = row_y + row_h - view_h;
    }
    s.clamp(0, scroll_max.max(0))
}

#[derive(Clone)]
struct Listed {
    name: String,
    path: PathBuf,
    when: String,
    preview: SessionPreview,
}

struct PickUi {
    dpi: i32,
    font: HFONT,
    /// Same 14px face as `font`, weight 700. Accent Open only.
    bold: HFONT,
    small: HFONT,
    semi: HFONT,
    search: HWND,
    with_save: bool,
    rows: Vec<Listed>,
    shown: Vec<usize>,
    sel: Option<usize>,
    /// Indexes into `shown`. Open several uses these, in list order.
    picked: Vec<usize>,
    anchor: usize,
    hot: Option<usize>,
    scroll: i32,
    detail_scroll: i32,
}

impl PickUi {
    const fn empty() -> Self {
        Self {
            dpi: 96,
            font: HFONT(std::ptr::null_mut()),
            bold: HFONT(std::ptr::null_mut()),
            small: HFONT(std::ptr::null_mut()),
            semi: HFONT(std::ptr::null_mut()),
            search: HWND(std::ptr::null_mut()),
            with_save: false,
            rows: Vec::new(),
            shown: Vec::new(),
            sel: None,
            picked: Vec::new(),
            anchor: 0,
            hot: None,
            scroll: 0,
            detail_scroll: 0,
        }
    }
}

fn format_saved(t: SystemTime) -> String {
    let Ok(dur) = t.duration_since(UNIX_EPOCH) else { return String::new() };
    let ticks = dur.as_secs().saturating_mul(10_000_000).saturating_add(dur.subsec_nanos() as u64 / 100).saturating_add(11_644_473_600 * 10_000_000);
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    unsafe {
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        if FileTimeToSystemTime(&ft, &mut utc).is_err() {
            return String::new();
        }
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            local = utc;
        }
        format_civil(local.wYear as i32, local.wMonth as u32, local.wDay as u32, local.wHour as u32, local.wMinute as u32)
    }
}

fn read_rows() -> Vec<Listed> {
    session_list(&sessions_dir())
        .into_iter()
        .map(|e| {
            let bytes = std::fs::read(&e.path).unwrap_or_default();
            Listed { name: e.name, path: e.path, when: format_saved(e.modified), preview: preview_bytes(&bytes) }
        })
        .collect()
}

fn button_widths(dc: HDC, font: HFONT, with_save: bool, dpi: i32) -> ButtonWidths {
    let pad = dip(dpi, 28);
    let min = dip(dpi, 88);
    let w = |s: &str| (text_width(dc, font, s) + pad).max(min);
    ButtonWidths {
        open: w(tr(crate::Key::Open)),
        rename: w(tr(crate::Key::Rename)),
        delete: w(tr(crate::Key::Delete)),
        cancel: w(tr(crate::Key::Cancel)),
        save: if with_save { (text_width(dc, font, tr(crate::Key::SaveThisTab3)) + pad).max(dip(dpi, 120)) } else { 0 },
        several: if with_save { (text_width(dc, font, tr(crate::Key::OpenSeveral)) + pad).max(dip(dpi, 120)) } else { 0 },
    }
}

/// Click on row `index`. Shift extends from the anchor. Ctrl toggles that row. A plain click selects only it.
pub(crate) fn selection_after_click(picked: &[usize], anchor: usize, index: usize, count: usize, ctrl: bool, shift: bool) -> (Vec<usize>, usize, usize) {
    if count == 0 {
        return (Vec::new(), 0, 0);
    }
    let index = index.min(count - 1);
    if shift {
        let anchor = anchor.min(count - 1);
        let (lo, hi) = if anchor <= index { (anchor, index) } else { (index, anchor) };
        let mut next: Vec<usize> = if ctrl { picked.iter().copied().filter(|k| *k < count && !(lo..=hi).contains(k)).collect() } else { Vec::new() };
        next.extend(lo..=hi);
        next.sort_unstable();
        next.dedup();
        return (next, index, anchor);
    }
    if ctrl {
        let mut next: Vec<usize> = picked.iter().copied().filter(|k| *k < count && *k != index).collect();
        if !picked.contains(&index) {
            next.push(index);
        }
        next.sort_unstable();
        next.dedup();
        return (next, index, index);
    }
    (vec![index], index, index)
}

/// Arrow keys. Shift extends the range from the anchor; otherwise the row becomes the only selection.
pub(crate) fn selection_after_key(anchor: usize, sel: Option<usize>, count: usize, delta: i32, extend: bool) -> (Vec<usize>, usize, usize) {
    if count == 0 {
        return (Vec::new(), 0, 0);
    }
    let cur = sel.unwrap_or(0).min(count - 1) as i32;
    let next = (cur + delta).clamp(0, count as i32 - 1) as usize;
    if extend {
        let anchor = anchor.min(count - 1);
        let (lo, hi) = if anchor <= next { (anchor, next) } else { (next, anchor) };
        ((lo..=hi).collect(), next, anchor)
    } else {
        (vec![next], next, next)
    }
}

/// Second line of a list row: the saved time, plus a mark when the file cannot be previewed.
fn row_stamp(when: &str, readable: bool) -> String {
    if readable {
        when.to_string()
    } else if when.is_empty() {
        tr(crate::Key::Unreadable).to_string()
    } else {
        format!("{when}  ·  {}", tr(crate::Key::Unreadable))
    }
}

fn text_width(dc: HDC, font: HFONT, s: &str) -> i32 {
    if s.is_empty() || dc.is_invalid() {
        return 0;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    unsafe {
        let old = SelectObject(dc, font.into());
        let _ = DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
    }
    r.right
}

fn make_font(face: &str, px: i32, weight: i32, dpi: i32) -> HFONT {
    let mut lf = LOGFONTW {
        lfHeight: -(px * dpi / 96).max(1),
        lfWeight: weight,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    unsafe { CreateFontIndirectW(&lf) }
}

fn face_pair() -> (String, i32, &'static str) {
    let custom = app(|a| a.s.font_face.clone());
    if custom.is_empty() {
        let (title, w) = crate::title_style();
        (ui_face().to_string(), w, title)
    } else {
        (custom, 700, "")
    }
}

unsafe fn rebuild_fonts(dpi: i32) {
    let (body, title_w, title_face) = face_pair();
    let title = if title_face.is_empty() { body.as_str() } else { title_face };
    let new = (
        make_font(&body, 14, 400, dpi),
        make_font(&body, 14, 700, dpi),
        make_font(&body, 12, 400, dpi),
        make_font(title, 16, title_w, dpi),
    );
    let old = UI.with(|u| {
        let mut ui = u.borrow_mut();
        let old = (ui.font, ui.bold, ui.small, ui.semi);
        (ui.font, ui.bold, ui.small, ui.semi) = new;
        ui.dpi = dpi;
        old
    });
    for f in [old.0, old.1, old.2, old.3] {
        crate::delete_font(f);
    }
}

unsafe fn style_window(h: HWND) {
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = crate::cref(crate::pal().bg);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
}

unsafe fn theme_edit(h: HWND) {
    let name = if crate::DARK_ON.load(Ordering::Relaxed) { w!("DarkMode_CFD") } else { w!("CFD") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

fn current_layout(h: HWND) -> PickLayout {
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    let (dpi, n, scroll, detail_scroll, with_save, detail_h, font) = UI.with(|u| {
        let ui = u.borrow();
        let detail_h = ui.sel.and_then(|i| ui.shown.get(i).copied()).map(|i| detail_plan(&ui.rows[i].name, &ui.rows[i].preview, ui.dpi).iter().map(|l| l.h).sum()).unwrap_or(0);
        (ui.dpi, ui.shown.len(), ui.scroll, ui.detail_scroll, ui.with_save, detail_h, ui.font)
    });
    // Screen DC: this runs from inside WM_PAINT, where GetDC(h) would fight BeginPaint.
    let dc = unsafe { GetDC(None) };
    let widths = button_widths(dc, font, with_save, dpi);
    unsafe { let _ = ReleaseDC(None, dc); }
    let lay = pick_layout(rc.right, rc.bottom, dpi, n, scroll, detail_h, detail_scroll, with_save, widths);
    UI.with(|u| {
        let mut ui = u.borrow_mut();
        ui.scroll = lay.scroll;
        ui.detail_scroll = lay.detail_scroll;
    });
    lay
}

unsafe fn place_buttons(h: HWND, lay: &PickLayout) {
    let mv = |id: i32, b: BoxPx| {
        if let Ok(c) = GetDlgItem(Some(h), id) {
            let _ = MoveWindow(c, b.left, b.top, b.w().max(1), b.h().max(1), true);
        }
    };
    mv(ID_OPEN, lay.open);
    mv(ID_RENAME, lay.rename);
    mv(ID_DELETE, lay.delete);
    mv(ID_CANCEL, lay.cancel);
    if let Some(b) = lay.save {
        mv(ID_SAVE, b);
    }
    if let Some(b) = lay.several {
        mv(ID_SEVERAL, b);
    }
    let inset = dip(UI.with(|u| u.borrow().dpi), crate::settings::FIELD_INSET_DIP);
    let edit = BoxPx { left: lay.search.left + inset, top: lay.search.top + inset, right: lay.search.right - inset, bottom: lay.search.bottom - inset };
    crate::place_edit(UI.with(|u| u.borrow().search), edit.rect(), true);
}

unsafe fn sync_buttons(h: HWND) {
    let (has, readable, several) = UI.with(|u| {
        let ui = u.borrow();
        let row = ui.sel.and_then(|i| ui.shown.get(i).copied()).map(|i| ui.rows[i].preview.readable);
        let several = ui.picked.iter().filter(|&&i| ui.shown.get(i).is_some_and(|&row| ui.rows.get(row).is_some_and(|r| r.preview.readable))).count() >= 2;
        (ui.sel.is_some(), row.unwrap_or(false), several)
    });
    let en = |id: i32, on: bool| {
        if let Ok(c) = GetDlgItem(Some(h), id) {
            let _ = EnableWindow(c, on);
            let _ = InvalidateRect(Some(c), None, false);
        }
    };
    en(ID_OPEN, readable);
    en(ID_RENAME, has);
    en(ID_DELETE, has);
    en(ID_SEVERAL, several);
}

unsafe fn layout_window(h: HWND) {
    let lay = current_layout(h);
    place_buttons(h, &lay);
    let _ = InvalidateRect(Some(h), None, false);
}

fn apply_filter(keep: Option<PathBuf>) {
    UI.with(|u| {
        let mut ui = u.borrow_mut();
        let q = ui_query(&ui);
        ui.shown = ui.rows.iter().enumerate().filter(|(_, r)| preview_matches(&r.name, &r.preview, &q)).map(|(i, _)| i).collect();
        ui.sel = keep
            .and_then(|p| ui.shown.iter().position(|&i| ui.rows[i].path == p))
            .or_else(|| if ui.shown.is_empty() { None } else { Some(0) });
        ui.picked = ui.sel.into_iter().collect();
        ui.anchor = ui.sel.unwrap_or(0);
        ui.hot = None;
        ui.scroll = 0;
        ui.detail_scroll = 0;
    });
}

fn ui_query(ui: &PickUi) -> String {
    if ui.search.0.is_null() {
        String::new()
    } else {
        unsafe { crate::text_of(ui.search) }
    }
}

fn selected_path() -> Option<PathBuf> {
    UI.with(|u| {
        let ui = u.borrow();
        ui.sel.and_then(|i| ui.shown.get(i).copied()).map(|i| ui.rows[i].path.clone())
    })
}

fn selected_readable() -> bool {
    UI.with(|u| {
        let ui = u.borrow();
        ui.sel.and_then(|i| ui.shown.get(i).copied()).is_some_and(|i| ui.rows[i].preview.readable)
    })
}

fn picked_paths() -> Vec<PathBuf> {
    UI.with(|u| {
        let ui = u.borrow();
        let mut idx = ui.picked.clone();
        idx.sort_unstable();
        idx.dedup();
        idx.into_iter().filter_map(|i| {
            let row = *ui.shown.get(i)?;
            let listed = ui.rows.get(row)?;
            listed.preview.readable.then(|| listed.path.clone())
        }).collect()
    })
}

unsafe fn finish_open(h: HWND, paths: Vec<PathBuf>) {
    if paths.is_empty() {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    OUT.with(|o| *o.borrow_mut() = Some(Some(paths)));
    let _ = DestroyWindow(h);
}

unsafe fn finish(h: HWND, pick: bool) {
    if pick && !selected_readable() {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let paths = if pick { selected_path().into_iter().collect() } else { Vec::new() };
    if pick && paths.is_empty() {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    OUT.with(|o| *o.borrow_mut() = Some((!paths.is_empty()).then_some(paths)));
    let _ = DestroyWindow(h);
}

unsafe fn reload(h: HWND, keep: Option<PathBuf>) {
    let rows = read_rows();
    UI.with(|u| u.borrow_mut().rows = rows);
    apply_filter(keep);
    sync_buttons(h);
    layout_window(h);
}

fn name_taken(dir: &Path, stem: &str, except: &Path) -> bool {
    let low = stem.to_lowercase();
    let except = std::fs::canonicalize(except).unwrap_or_else(|_| except.to_path_buf());
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let p = e.path();
        if std::fs::canonicalize(&p).unwrap_or(p.clone()) == except {
            return false;
        }
        is_session_file(&p) && p.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == low)
    })
}

unsafe fn explain_name(h: HWND, why: &str) {
    MessageBoxW(Some(h), &HSTRING::from(why), &HSTRING::from(tr(crate::Key::RenameSession)), MB_OK | MB_ICONWARNING);
}

unsafe fn do_rename(h: HWND) {
    let Some(path) = selected_path() else { return };
    let current = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(typed) = crate::settings::ask(h, tr(crate::Key::RenameSession), &current) else { return };
    let Some(stem) = session_stem_ok(&typed) else {
        explain_name(h, tr(crate::Key::ThatNameCanTBeUsedLeave));
        return;
    };
    if stem.chars().count() > SESSION_NAME_MAX {
        explain_name(h, tr(crate::Key::ThatNameIsTooLong));
        return;
    }
    if stem.eq_ignore_ascii_case(&current) && stem == current {
        return;
    }
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(sessions_dir);
    if name_taken(&dir, &stem, &path) {
        explain_name(h, tr(crate::Key::SessionWithThisNameAlreadyExists2));
        return;
    }
    let old_s = path.to_string_lossy().into_owned();
    let new_path = match rename_session(&path, &stem) {
        Ok(p) => p,
        Err(_) => {
            let _ = MessageBeep(MB_ICONWARNING);
            return;
        }
    };
    let new_s = new_path.to_string_lossy().into_owned();
    app(|a| {
        if let Some(n) = file_after_rename(&old_s, &a.s.file, true, &new_s) {
            a.s.file = n;
        }
    });
    crate::refresh_save_button();
    reload(h, Some(new_path));
    let _ = SetFocus(Some(UI.with(|u| u.borrow().search)));
}

unsafe fn do_delete(h: HWND) {
    let Some(path) = selected_path() else { return };
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let q = format!(
        "{}\n\"{name}\"\n{}",
        tr(crate::Key::DeleteThisSession),
        tr(crate::Key::ItWillGoRecycleBin)
    );
    if MessageBoxW(Some(h), &HSTRING::from(q), &HSTRING::from(tr(crate::Key::DeleteSession)), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES {
        return;
    }
    let owner = GetWindow(h, GW_OWNER).unwrap_or_default();
    if !owner.0.is_null() {
        let _ = EnableWindow(owner, true);
    }
    let deleted = delete_session(&path);
    if !owner.0.is_null() {
        let _ = EnableWindow(owner, false);
        let _ = SetForegroundWindow(h);
    }
    if !deleted {
        return;
    }
    let next = UI.with(|u| {
        let ui = u.borrow();
        ui.sel.and_then(|i| ui.shown.get(i + 1).or_else(|| i.checked_sub(1).and_then(|j| ui.shown.get(j)))).map(|&k| ui.rows[k].path.clone())
    });
    reload(h, next);
}

unsafe fn move_sel(h: HWND, d: i32, extend: bool) {
    let path = UI.with(|u| {
        let mut ui = u.borrow_mut();
        let (picked, sel, anchor) = selection_after_key(ui.anchor, ui.sel, ui.shown.len(), d, extend);
        if ui.shown.is_empty() {
            return None;
        }
        ui.picked = picked;
        ui.sel = Some(sel);
        ui.anchor = anchor;
        ui.detail_scroll = 0;
        Some(ui.rows[ui.shown[sel]].path.clone())
    });
    if path.is_none() {
        return;
    }
    // Reveal uses content y, so compute it after the layout clamps scroll.
    let lay = current_layout(h);
    if let Some(i) = UI.with(|u| u.borrow().sel) {
        let stride = lay.row_h + dip(UI.with(|u| u.borrow().dpi), 4);
        let inner = dip(UI.with(|u| u.borrow().dpi), 8);
        let y = inner + i as i32 * stride;
        let scroll = reveal_row(lay.scroll, lay.scroll_max, lay.list.h(), y, lay.row_h);
        UI.with(|u| u.borrow_mut().scroll = scroll);
    }
    sync_buttons(h);
    layout_window(h);
}

unsafe fn draw_schematic(dc: HDC, r: RECT, columns: &[usize], accent: u32, dpi: i32) {
    if columns.is_empty() || r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let gap = dip(dpi, 3).max(1);
    let cols = columns.len() as i32;
    let cw = ((r.right - r.left) - gap * (cols - 1)) / cols;
    if cw <= 1 {
        return;
    }
    let mut x = r.left;
    let mut n = 1usize;
    let font = UI.with(|u| u.borrow().small);
    for (c, &rows) in columns.iter().enumerate() {
        let rows = rows.max(1) as i32;
        let rh = ((r.bottom - r.top) - gap * (rows - 1)) / rows;
        let mut y = r.top;
        for _ in 0..rows {
            let cell = RECT { left: x, top: y, right: x + cw, bottom: y + rh.max(1) };
            crate::aa_rfill(dc, cell, dip(dpi, 3), crate::blend(crate::pal().card, accent, 0.20), Some(crate::pal().line), crate::pal().card);
            if cw >= dip(dpi, 16) && rh >= dip(dpi, 14) {
                crate::text(dc, cell, &n.to_string(), font, crate::pal().sub, DT_CENTER);
            }
            n += 1;
            y += rh.max(1) + gap;
        }
        if c + 1 != columns.len() {
            x += cw + gap;
        }
    }
}

unsafe fn paint(h: HWND) {
    let accent = crate::settings::chrome_color();
    let dpi = UI.with(|u| u.borrow().dpi.max(96));
    let lay = current_layout(h);
    let (font, small, semi, shown, sel, hot, when_bad, picked) = UI.with(|u| {
        let ui = u.borrow();
        let shown: Vec<Listed> = ui.shown.iter().filter_map(|&i| ui.rows.get(i).cloned()).collect();
        (ui.font, ui.small, ui.semi, shown, ui.sel, ui.hot, ui.rows.is_empty(), ui.picked.clone())
    });
    let query_empty = UI.with(|u| ui_query(&u.borrow()).trim().is_empty());
    crate::buffered(h, |dc, rc| {
        crate::fill(dc, rc, crate::pal().bg);
        crate::aa_rfill(dc, lay.search.rect(), dip(dpi, crate::settings::FIELD_RADIUS_DIP), crate::pal().card, Some(crate::pal().line), crate::pal().bg);
        crate::aa_rfill(dc, lay.list.rect(), dip(dpi, 6), crate::pal().card, Some(crate::pal().line), crate::pal().bg);
        crate::aa_rfill(dc, lay.detail.rect(), dip(dpi, 6), crate::pal().card, Some(crate::pal().line), crate::pal().bg);
        if shown.is_empty() {
            let msg = if when_bad && query_empty {
                tr(crate::Key::NoSavedSessionsYet)
            } else {
                tr(crate::Key::NoMatches)
            };
            crate::text(dc, lay.list.rect(), msg, small, crate::pal().sub, DT_CENTER);
        }
        unsafe {
            let _ = SaveDC(dc);
            let _ = IntersectClipRect(dc, lay.list.left + 1, lay.list.top + 1, lay.list.right - 1, lay.list.bottom - 1);
        }
        for (i, row) in shown.iter().enumerate() {
            let Some(b) = lay.rows.get(i).copied() else { continue };
            if b.bottom <= lay.list.top || b.top >= lay.list.bottom {
                continue;
            }
            let on = sel == Some(i);
            let marked = picked.contains(&i);
            let over = hot == Some(i);
            if on || marked {
                let mix = if on { 0.16 } else { 0.10 };
                crate::aa_rfill(dc, b.rect(), dip(dpi, 4), crate::blend(crate::pal().card, accent, mix), None, crate::pal().card);
                crate::fill(dc, RECT { left: b.left, top: b.top + dip(dpi, 8), right: b.left + dip(dpi, 3), bottom: b.bottom - dip(dpi, 8) }, accent);
            } else if over {
                crate::aa_rfill(dc, b.rect(), dip(dpi, 4), crate::blend(crate::pal().card, accent, 0.08), None, crate::pal().card);
            }
            let name_r = RECT { left: b.left + dip(dpi, 12), top: b.top + dip(dpi, 6), right: b.right - dip(dpi, 8), bottom: b.top + dip(dpi, 28) };
            crate::text(dc, name_r, &row.name, font, crate::pal().text, DT_LEFT);
            let date_r = RECT { left: name_r.left, top: name_r.bottom, right: name_r.right, bottom: b.bottom - dip(dpi, 4) };
            let date = row_stamp(&row.when, row.preview.readable);
            let date_ink = if row.preview.readable { crate::pal().sub } else { 0xC42B1C };
            crate::text(dc, date_r, &date, small, date_ink, DT_LEFT);
        }
        // Reset the clip to the detail card. SaveDC/RestoreDC keeps the buffer's clip simple.
        unsafe {
            let _ = RestoreDC(dc, -1);
            let _ = SaveDC(dc);
            let _ = IntersectClipRect(dc, lay.detail.left + 1, lay.detail.top + 1, lay.detail.right - 1, lay.detail.bottom - 1);
        }
        let pad = dip(dpi, 16);
        if sel.is_none() || shown.is_empty() {
            let empty_folder = when_bad && query_empty;
            let msg = if empty_folder {
                tr(crate::Key::NoSavedSessionsYet)
            } else if shown.is_empty() {
                tr(crate::Key::NoMatches)
            } else {
                tr(crate::Key::SelectSession)
            };
            crate::text(dc, lay.detail.rect(), msg, font, crate::pal().sub, DT_CENTER);
            if empty_folder {
                let sub = RECT { top: (lay.detail.top + lay.detail.bottom) / 2 + dip(dpi, 4), ..lay.detail.rect() };
                crate::text(dc, sub, &sessions_dir().display().to_string(), small, crate::pal().sub, DT_CENTER);
            }
        } else if let Some(row) = sel.and_then(|i| shown.get(i)) {
            let plan = detail_plan(&row.name, &row.preview, dpi);
            let mut y = lay.detail.top + pad - lay.detail_scroll;
            let left = lay.detail.left + pad;
            let right = lay.detail.right - pad;
            if picked.len() > 1 {
                let note = format!(
                    "{}{}",
                    picked.len(),
                    tr(crate::Key::SelectedOpenSeveralOpensEachAsIts)
                );
                let r = RECT { left, top: y, right, bottom: y + dip(dpi, 22) };
                crate::text(dc, r, &note, small, crate::pal().sub, DT_LEFT);
                y += dip(dpi, 22);
            }
            for line in &plan {
                let r = RECT { left, top: y, right, bottom: y + line.h };
                if r.bottom > lay.detail.top && r.top < lay.detail.bottom {
                    match line.kind {
                        DetailKind::Title => crate::text(dc, r, &line.text, semi, crate::pal().text, DT_LEFT),
                        DetailKind::Meta => {
                            let ink = if row.preview.readable { crate::pal().sub } else { 0xC42B1C };
                            crate::text(dc, r, &line.text, font, ink, DT_LEFT);
                        }
                        DetailKind::TabHead => crate::text(dc, r, &line.text, font, crate::pal().text, DT_LEFT),
                        DetailKind::Schematic => {
                            let box_w = dip(dpi, 220).min(right - left);
                            draw_schematic(dc, RECT { right: left + box_w, ..r }, &line.columns, accent, dpi);
                        }
                        DetailKind::Path => crate::text(dc, r, &line.text, small, crate::pal().text, DT_LEFT),
                        DetailKind::Gap => {}
                    }
                }
                y += line.h;
            }
        }
        unsafe { let _ = RestoreDC(dc, -1); }
    });
}

unsafe fn draw_btn(d: &DRAWITEMSTRUCT) {
    let id = d.CtlID as i32;
    let dis = d.itemState.0 & 4 != 0;
    let pressed = d.itemState.0 & 1 != 0;
    let hot = app(|a| a.hover) == d.hwndItem;
    let dpi = UI.with(|u| u.borrow().dpi.max(96));
    let (font, bold) = UI.with(|u| {
        let ui = u.borrow();
        (ui.font, ui.bold)
    });
    let accent = crate::settings::chrome_color();
    let ground = crate::pal().bg;
    crate::fill(d.hDC, d.rcItem, ground);
    let accent_btn = id == ID_OPEN && !dis;
    let (fill, border, ink) = if accent_btn {
        (accent, accent, crate::on_color(accent))
    } else if dis {
        (crate::blend(ground, crate::pal().sub, 0.12), crate::pal().line, crate::pal().sub)
    } else if pressed {
        (crate::blend(ground, crate::pal().text, 0.08), crate::pal().line, crate::pal().text)
    } else if hot {
        (crate::blend(ground, accent, 0.12), accent, crate::pal().text)
    } else if id == ID_DELETE {
        (ground, crate::pal().line, 0xC42B1C)
    } else {
        (ground, crate::pal().line, crate::pal().text)
    };
    crate::aa_rfill(d.hDC, d.rcItem, dip(dpi, 4), fill, Some(border), ground);
    let label_font = if accent_btn && !bold.is_invalid() { bold } else { font };
    crate::text(d.hDC, d.rcItem, &crate::text_of(d.hwndItem), label_font, ink, DT_CENTER);
}

unsafe fn on_click(h: HWND, x: i32, y: i32, dbl: bool, ctrl: bool, shift: bool) {
    let lay = current_layout(h);
    let Some(i) = lay.rows.iter().position(|r| r.contains(x, y) && lay.list.contains(x, y)) else { return };
    UI.with(|u| {
        let mut ui = u.borrow_mut();
        if dbl {
            ui.sel = Some(i);
            ui.picked = vec![i];
            ui.anchor = i;
        } else {
            let (picked, sel, anchor) = selection_after_click(&ui.picked, ui.anchor, i, ui.shown.len(), ctrl, shift);
            ui.picked = picked;
            ui.sel = Some(sel);
            ui.anchor = anchor;
        }
        ui.detail_scroll = 0;
    });
    sync_buttons(h);
    layout_window(h);
    if dbl {
        finish(h, true);
    }
}

unsafe fn open_several(h: HWND) {
    let paths = picked_paths();
    if paths.len() < 2 {
        let why = tr(crate::Key::SelectTwoOrMoreSessionsWithCtrl);
        MessageBoxW(Some(h), &HSTRING::from(why), &HSTRING::from(tr(crate::Key::OpenSeveral)), MB_OK | MB_ICONINFORMATION);
        return;
    }
    finish_open(h, paths);
}

unsafe extern "system" fn pick_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { pick_proc_inner(h, msg, wp, lp) }))
}
unsafe fn pick_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let dpi = GetDpiForWindow(h).max(96) as i32;
            UI.with(|u| u.borrow_mut().dpi = dpi);
            rebuild_fonts(dpi);
            style_window(h);
            let font = UI.with(|u| u.borrow().font);
            let tab = 0x0001_0000u32;
            let search = crate::child(h, w!("EDIT"), w!(""), 0x80 | tab, WINDOW_EX_STYLE(0), ID_SEARCH, font);
            theme_edit(search);
            UI.with(|u| u.borrow_mut().search = search);
            let cue = HSTRING::from(tr(crate::Key::SearchSessions));
            SendMessageW(search, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
            let m = dip(dpi, 8) as isize;
            SendMessageW(search, 0x00D3, Some(WPARAM(3)), Some(LPARAM(m | (m << 16))));
            let btn = |id, label: &str| {
                let t = HSTRING::from(label);
                crate::child(h, w!("BUTTON"), PCWSTR(t.as_ptr()), 0xB | tab, WINDOW_EX_STYLE(0), id, font);
            };
            btn(ID_OPEN, tr(crate::Key::Open));
            btn(ID_RENAME, tr(crate::Key::Rename));
            btn(ID_DELETE, tr(crate::Key::Delete));
            btn(ID_CANCEL, tr(crate::Key::Cancel));
            if UI.with(|u| u.borrow().with_save) {
                btn(ID_SAVE, tr(crate::Key::SaveThisTab3));
                btn(ID_SEVERAL, tr(crate::Key::OpenSeveral));
            }
            let rows = read_rows();
            UI.with(|u| u.borrow_mut().rows = rows);
            apply_filter(None);
            sync_buttons(h);
            layout_window(h);
            return LRESULT(0);
        }
        WM_SIZE | WM_DPICHANGED => {
            if msg == WM_DPICHANGED {
                let dpi = (wp.0 & 0xffff) as i32;
                rebuild_fonts(dpi.max(96));
                let font = UI.with(|u| u.borrow().font);
                let search = UI.with(|u| u.borrow().search);
                SendMessageW(search, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                for id in [ID_OPEN, ID_RENAME, ID_DELETE, ID_CANCEL, ID_SAVE, ID_SEVERAL] {
                    if let Ok(c) = GetDlgItem(Some(h), id) {
                        SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                    }
                }
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                style_window(h);
            }
            layout_window(h);
            return LRESULT(0);
        }
        WM_GETMINMAXINFO => {
            let dpi = UI.with(|u| u.borrow().dpi.max(96));
            let m = &mut *(lp.0 as *mut MINMAXINFO);
            m.ptMinTrackSize.x = dip(dpi, 860);
            m.ptMinTrackSize.y = dip(dpi, 520);
            return LRESULT(0);
        }
        WM_PAINT => {
            paint(h);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_DRAWITEM => {
            draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
            return LRESULT(1);
        }
        WM_CTLCOLOREDIT => {
            let dc = HDC(wp.0 as _);
            SetBkMode(dc, TRANSPARENT);
            SetBkColor(dc, crate::cref(crate::pal().card));
            SetTextColor(dc, crate::cref(crate::pal().text));
            SetDCBrushColor(dc, crate::cref(crate::pal().card));
            return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
        }
        WM_COMMAND => {
            let (id, code) = ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32);
            if id == ID_SEARCH && code == 0x0300 {
                let keep = selected_path();
                apply_filter(keep);
                sync_buttons(h);
                layout_window(h);
                return LRESULT(0);
            }
            match id {
                ID_OPEN => finish(h, true),
                ID_SEVERAL => open_several(h),
                ID_CANCEL => finish(h, false),
                ID_SAVE => {
                    SAVE_ASKED.with(|c| c.set(true));
                    finish(h, false);
                }
                ID_RENAME => do_rename(h),
                ID_DELETE => do_delete(h),
                _ => {}
            }
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            let ctrl = wp.0 & MK_CONTROL.0 as usize != 0;
            let shift = wp.0 & MK_SHIFT.0 as usize != 0;
            on_click(h, (lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32, false, ctrl, shift);
        }
        WM_LBUTTONDBLCLK => on_click(h, (lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32, true, false, false),
        WM_MOUSEMOVE => {
            let x = (lp.0 & 0xffff) as i16 as i32;
            let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
            let lay = current_layout(h);
            let hot = lay.rows.iter().position(|r| r.contains(x, y) && lay.list.contains(x, y));
            let changed = UI.with(|u| {
                let mut ui = u.borrow_mut();
                if ui.hot != hot {
                    ui.hot = hot;
                    true
                } else {
                    false
                }
            });
            if changed {
                let _ = InvalidateRect(Some(h), None, false);
            }
            let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
            let _ = TrackMouseEvent(&mut t);
        }
        WM_MOUSELEAVE => {
            UI.with(|u| u.borrow_mut().hot = None);
            let _ = InvalidateRect(Some(h), None, false);
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) as i16 as i32) / 120;
            if delta != 0 {
                let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                let _ = ScreenToClient(h, &mut pt);
                let lay = current_layout(h);
                let step = dip(UI.with(|u| u.borrow().dpi), 48);
                if lay.detail.contains(pt.x, pt.y) {
                    UI.with(|u| {
                        let mut ui = u.borrow_mut();
                        ui.detail_scroll = (ui.detail_scroll - delta * step).clamp(0, lay.detail_max);
                    });
                } else {
                    UI.with(|u| {
                        let mut ui = u.borrow_mut();
                        ui.scroll = (ui.scroll - delta * step).clamp(0, lay.scroll_max);
                    });
                }
                layout_window(h);
            }
        }
        WM_CLOSE => {
            finish(h, false);
            return LRESULT(0);
        }
        WM_DESTROY => {
            let old = UI.with(|u| {
                let mut ui = u.borrow_mut();
                let old = (ui.font, ui.bold, ui.small, ui.semi);
                *ui = PickUi::empty();
                old
            });
            for f in [old.0, old.1, old.2, old.3] {
                crate::delete_font(f);
            }
        }
        _ => return DefWindowProcW(h, msg, wp, lp),
    }
    LRESULT(0)
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(pick_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(hinst), PCWSTR(ICON_GROUP_LIGHT as _)).unwrap_or_default(),
            lpszClassName: w!("MaguroSessionList"),
            ..Default::default()
        });
    });
}

unsafe fn outer_size(dpi: i32) -> (i32, i32) {
    let (w, h) = (dip(dpi, 920), dip(dpi, 600));
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    let mut r = RECT { left: 0, top: 0, right: w, bottom: h };
    if AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE(0), dpi as u32).is_ok() {
        (r.right - r.left, r.bottom - r.top)
    } else {
        (w, h)
    }
}

/// Small modal window listing the sessions in the sessions folder. None means cancel.
/// One session: startup uses this. The load window can return more than one.
pub(crate) unsafe fn pick_session(owner: HWND, title: &str) -> Option<PathBuf> {
    pick_session_ex(owner, title, false).and_then(|v| v.into_iter().next())
}

/// `with_save` adds "Save this tab…" and "Open several". Pressing Save closes the list and sets SAVE_ASKED.
unsafe fn pick_session_ex(owner: HWND, title: &str, with_save: bool) -> Option<Vec<PathBuf>> {
    ensure_class();
    let dir = sessions_dir();
    _ = std::fs::create_dir_all(&dir);
    SAVE_ASKED.with(|c| c.set(false));
    OUT.with(|o| *o.borrow_mut() = None);
    UI.with(|u| {
        *u.borrow_mut() = PickUi::empty();
        u.borrow_mut().with_save = with_save;
    });
    let dpi = GetDpiForWindow(owner).max(96) as i32;
    let (ww, hh) = outer_size(dpi);
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let (x, y) = (orc.left + (orc.right - orc.left - ww) / 2, orc.top + (orc.bottom - orc.top - hh) / 2);
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    let h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroSessionList"), &HSTRING::from(title), style, x, y, ww, hh, Some(owner), None, None, None).ok()?;
    style_window(h);
    let _ = ShowWindow(h, SW_SHOW);
    crate::keys::set_modal(true);
    let _ = EnableWindow(owner, false);
    let _ = SetFocus(Some(UI.with(|u| u.borrow().search)));
    let mut msg = MSG::default();
    while OUT.with(|o| o.borrow().is_none()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            PostQuitMessage(0);
            break;
        }
        let mine = msg.hwnd == h || IsChild(h, msg.hwnd).as_bool();
        if msg.message == WM_KEYDOWN && mine {
            let key = msg.wParam.0;
            if key == VK_RETURN.0 as usize {
                finish(h, true);
                continue;
            }
            if key == VK_ESCAPE.0 as usize {
                finish(h, false);
                continue;
            }
            if key == VK_UP.0 as usize || key == VK_DOWN.0 as usize {
                let extend = GetKeyState(VK_SHIFT.0 as i32) < 0;
                move_sel(h, if key == VK_UP.0 as usize { -1 } else { 1 }, extend);
                continue;
            }
            if key == VK_DELETE.0 as usize && GetFocus() != UI.with(|u| u.borrow().search) {
                do_delete(h);
                continue;
            }
        }
        if msg.message == WM_MOUSEMOVE || msg.message == WM_MOUSELEAVE {
            let on = msg.message == WM_MOUSEMOVE && crate::is_icon_button(msg.hwnd) && GetParent(msg.hwnd).unwrap_or_default() == h;
            crate::set_hover(if on { msg.hwnd } else { HWND::default() });
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    crate::set_hover(HWND::default());
    crate::keys::set_modal(false);
    let _ = EnableWindow(owner, true);
    let _ = SetForegroundWindow(owner);
    OUT.with(|o| o.borrow_mut().take().flatten())
}

/// Toolbar Load: the list window. Each chosen session opens as a new tab beside the current one.
pub(crate) unsafe fn open_session_list() {
    let main = app(|a| a.main);
    if let Some(paths) = pick_session_ex(main, tr(crate::Key::OpenSession), true) {
        load_session_files(&paths);
    } else if SAVE_ASKED.with(|c| c.replace(false)) {
        _ = save_session_as();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_tab_text() -> String {
        let mut s = Session::default();
        s.set_heights(vec![1, 1]);
        s.paths[0] = r"C:\Work".into();
        s.paths[1] = r"D:\Photos".into();
        s.anchor();
        s.tabs[0].name = "Work".into();
        let mut other = Session::default();
        other.set_heights(vec![2]);
        other.paths[0] = r"E:\Notes".into();
        other.paths[1] = r"E:\Notes\old".into();
        s.tabs.push(Tab::from_live(&other, "Notes".into(), false, vec![String::new(); MAXP]));
        s.to_ini()
    }

    #[test]
    fn session_list_names_and_order() {
        let d = std::env::temp_dir().join(format!("pg-list-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("old.qdr"), "x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(d.join("work.maguro"), "x").unwrap();
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        std::fs::create_dir_all(d.join("dir.maguro")).unwrap();
        let names: Vec<String> = session_list(&d).into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["work".to_string()]);
        assert!(session_list(&d.join("missing")).is_empty());
        _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn preview_summary_reads_tabs_columns_and_paths() {
        let p = preview_summary(&two_tab_text());
        assert!(p.readable);
        assert_eq!(p.tabs.len(), 2);
        assert_eq!(p.tabs[0].name, "Work");
        assert_eq!(p.tabs[0].columns, vec![1, 1]);
        assert_eq!(p.tabs[0].paths, vec![r"C:\Work".to_string(), r"D:\Photos".into()]);
        assert_eq!(p.tabs[1].name, "Notes");
        assert_eq!(p.tabs[1].columns, vec![2]);
        assert_eq!(p.tabs[1].paths, vec![r"E:\Notes".to_string(), r"E:\Notes\old".into()]);

        let one = preview_summary("cols=1,3\np0=C:\\a\nactive=1\n");
        assert!(one.readable);
        assert_eq!(one.tabs.len(), 1);
        assert_eq!(one.tabs[0].columns, vec![1, 3]);
        assert_eq!(one.tabs[0].paths[0], r"C:\a");
        assert_eq!(one.tabs[0].paths.len(), 4);

        let grid = preview_summary("grid=2,2\n");
        assert!(grid.readable);
        assert_eq!(grid.tabs[0].columns, vec![2, 2]);
        assert!(grid.tabs[0].paths.iter().all(|p| p.is_empty()));
    }

    #[test]
    fn unreadable_bytes_stay_marked() {
        assert!(!preview_summary("").readable);
        assert!(!preview_summary("hello from an old tool").readable);
        assert!(!preview_summary("x").readable);
        assert!(preview_summary("x").tabs.is_empty());
        assert!(!preview_bytes(&[0xFF, 0xFE, 0x61, 0x00]).readable);
        assert!(!preview_bytes(b"\xff\xfe not utf8 \x80\x81").readable);
        let bom = preview_summary("\u{feff}p0=C:\\Work\ncols=1\n");
        assert!(bom.readable);
        assert_eq!(bom.tabs[0].paths[0], r"C:\Work");
    }

    #[test]
    fn detail_plan_lists_each_tab_schematic_and_hides_a_bad_file() {
        let p = preview_summary(&two_tab_text());
        let lines = detail_plan("work", &p, 96);
        let schematics: Vec<_> = lines.iter().filter(|l| l.kind == DetailKind::Schematic).collect();
        assert_eq!(schematics.len(), 2);
        assert_eq!(schematics[0].columns, vec![1, 1]);
        assert_eq!(schematics[1].columns, vec![2]);
        assert!(lines.iter().any(|l| l.kind == DetailKind::TabHead && l.text.contains("Work") && l.text.contains('×')));
        assert!(lines.iter().any(|l| l.kind == DetailKind::Path && l.text.contains(r"C:\Work")));
        assert!(lines.iter().any(|l| l.kind == DetailKind::Path && l.text.contains(r"E:\Notes\old")));
        assert!(lines.iter().any(|l| l.kind == DetailKind::Meta && l.text.contains('2')));

        let bad = detail_plan("old", &preview_summary("not a session"), 96);
        assert!(bad.iter().all(|l| l.kind != DetailKind::Schematic));
        assert!(bad.iter().any(|l| l.kind == DetailKind::Meta && !l.text.is_empty()));

        let lo = detail_plan("work", &p, 96).iter().map(|l| l.h).sum::<i32>();
        let hi = detail_plan("work", &p, 192).iter().map(|l| l.h).sum::<i32>();
        assert!(hi > lo);
    }

    #[test]
    fn search_matches_name_tab_or_folder() {
        let p = preview_summary(&two_tab_text());
        assert!(preview_matches("work", &p, ""));
        assert!(preview_matches("work", &p, "WORK"));
        assert!(preview_matches("work", &p, "photo"));
        assert!(preview_matches("work", &p, "notes"));
        assert!(!preview_matches("work", &p, "zzz"));
        let bad = preview_summary("garbage");
        assert!(preview_matches("old", &bad, "old"));
        assert!(!preview_matches("old", &bad, "folder"));
    }

    #[test]
    fn layout_puts_the_list_beside_the_detail_and_scales() {
        let widths = ButtonWidths { open: 88, rename: 120, delete: 88, cancel: 88, save: 160, several: 0 };
        let lay = pick_layout(920, 600, 96, 4, 0, 400, 0, true, widths);
        assert!(lay.search.bottom <= lay.list.top);
        assert!(lay.list.right <= lay.detail.left);
        assert!(lay.list.bottom <= lay.open.top);
        assert_eq!(lay.open.top, lay.cancel.top);
        assert_eq!(lay.delete.top, lay.open.top);
        assert!(lay.save.unwrap().right <= lay.open.left);
        assert_eq!(lay.rows.len(), 4);
        assert!(lay.rows[0].top >= lay.list.top && lay.rows[0].right <= lay.list.right);
        assert_eq!(lay.row_h, 52);
        assert_eq!(lay.scroll_max, 0);

        let hidpi = pick_layout(1840, 1200, 192, 4, 0, 800, 0, false, widths);
        assert_eq!(hidpi.row_h, 104);
        assert!(hidpi.save.is_none());
        assert!(hidpi.list.w() > lay.list.w());

        let many = pick_layout(920, 600, 96, 30, 10_000, 100, 0, false, widths);
        assert!(many.scroll_max > 0);
        assert_eq!(many.scroll, many.scroll_max);
        assert_eq!(reveal_row(0, many.scroll_max, 200, 500, 52), 352);
        assert_eq!(reveal_row(400, many.scroll_max, 200, 100, 52), 100);
    }

    #[test]
    fn one_tab_detail_shows_the_layout_and_skips_an_automatic_name() {
        let p = preview_summary("grid=2,2\np0=C:\\a\n");
        let lines = detail_plan("Work", &p, 96);
        let meta = lines.iter().find(|l| l.kind == DetailKind::Meta).unwrap();
        assert_eq!(meta.text, "2 × 2");
        assert!(lines.iter().all(|l| l.kind != DetailKind::TabHead));
    }

    #[test]
    fn click_and_shift_build_a_multi_selection() {
        let (picked, sel, anchor) = selection_after_click(&[], 0, 1, 5, false, false);
        assert_eq!((picked.as_slice(), sel, anchor), ([1].as_slice(), 1, 1));
        let (picked, sel, anchor) = selection_after_click(&[1], 1, 3, 5, true, false);
        assert_eq!((picked.as_slice(), sel, anchor), ([1, 3].as_slice(), 3, 3));
        let (picked, sel, anchor) = selection_after_click(&[1, 3], 1, 4, 5, false, true);
        assert_eq!((picked.as_slice(), sel, anchor), ([1, 2, 3, 4].as_slice(), 4, 1));
        let (picked, sel, anchor) = selection_after_click(&[0], 0, 3, 5, true, true);
        assert_eq!((picked.as_slice(), sel, anchor), ([0, 1, 2, 3].as_slice(), 3, 0));
        let (picked, _, _) = selection_after_click(&[1], 1, 1, 5, true, false);
        assert!(picked.is_empty());
        let (picked, sel, anchor) = selection_after_key(1, Some(1), 5, 2, true);
        assert_eq!((picked.as_slice(), sel, anchor), ([1, 2, 3].as_slice(), 3, 1));
        let (picked, sel, anchor) = selection_after_key(0, Some(2), 5, -1, false);
        assert_eq!((picked.as_slice(), sel, anchor), ([1].as_slice(), 1, 1));
    }

    #[test]
    fn open_several_sits_between_save_and_open() {
        let widths = ButtonWidths { open: 88, rename: 120, delete: 88, cancel: 88, save: 160, several: 140 };
        let lay = pick_layout(920, 600, 96, 4, 0, 400, 0, true, widths);
        let save = lay.save.unwrap();
        let several = lay.several.unwrap();
        assert!(save.right <= several.left);
        assert!(several.right <= lay.open.left);
        assert_eq!(several.top, lay.open.top);
        let startup = ButtonWidths { several: 0, ..widths };
        assert!(pick_layout(920, 600, 96, 4, 0, 400, 0, false, startup).several.is_none());
    }

    #[test]
    fn format_civil_is_a_fixed_width_stamp() {
        assert_eq!(format_civil(2026, 10, 26, 5, 4), "2026-10-26 05:04");
        assert_eq!(format_civil(2026, 1, 2, 0, 0), "2026-01-02 00:00");
    }

    #[test]
    fn row_stamp_keeps_the_time_and_marks_unreadable() {
        assert_eq!(row_stamp("2026-10-26 05:04", true), "2026-10-26 05:04");
        assert_eq!(row_stamp("2026-10-26 05:04", false), "2026-10-26 05:04  ·  Unreadable");
        assert_eq!(row_stamp("", false), "Unreadable");
    }
}
