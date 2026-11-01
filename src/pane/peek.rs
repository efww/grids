use std::sync::atomic::AtomicBool;
use std::time::Duration;

use crate::imports::*;
use crate::app::*;
use crate::ui::*;
use crate::i18n::*;
use super::preview::*;
use super::{focus_find_list, focus_view, peek_find_neighbour, peek_find_select_path, step_find_sel};
use windows::Win32::Graphics::Dwm::{DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_NCRENDERING_POLICY};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::ILD_NORMAL;
use windows::Win32::UI::Controls::{TTF_ABSOLUTE, TTF_TRACK, TTM_POP, TTM_TRACKACTIVATE};
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};

/// Quick look was opened on a search result, so arrows walk that list and closing it returns there.
static FROM_FIND: AtomicBool = AtomicBool::new(false);
static FROM_FIND_PANE: AtomicUsize = AtomicUsize::new(usize::MAX);

pub(crate) fn peek_shows_find(i: usize) -> bool {
    FROM_FIND.load(Ordering::SeqCst) && FROM_FIND_PANE.load(Ordering::SeqCst) == i
}

fn mark_find_peek(i: Option<usize>) {
    match i {
        Some(i) => {
            FROM_FIND_PANE.store(i, Ordering::SeqCst);
            FROM_FIND.store(true, Ordering::SeqCst);
        }
        None => {
            FROM_FIND.store(false, Ordering::SeqCst);
            FROM_FIND_PANE.store(usize::MAX, Ordering::SeqCst);
        }
    }
}

pub(crate) const WM_PEEK_DONE: u32 = WM_APP + 45; // wParam = generation; the pixels stay in PEEK
pub(crate) const WM_PEEK_LOST: u32 = WM_APP + 51; // the preview went inactive: close it unless it is active again
pub(crate) const WM_PEEK_HOSTED: u32 = WM_APP + 52; // wParam = generation, lParam = 1 shown / 0 failed
const WM_PEEK_HOST_SIZE: u32 = WM_APP + 53; // thread message: the host window changed size
const WM_PEEK_ZOOM: u32 = WM_APP + 54; // wParam = generation; a PDF page was re-rendered for zoom. Does not resize.
const WM_PEEK_DELETED: u32 = WM_APP + 56; // wParam = 1 success / 0 cancel-or-fail; apply AFTER_DEL

const ICON_PREV: &str = "";
const ICON_NEXT: &str = "";
const ICON_MAX: &str = "\u{E922}";
const ICON_RESTORE: &str = "\u{E923}";

/// Readable text width, in characters of the peek font.
pub(crate) const PEEK_TEXT_COLS: i32 = 100;
/// A short note still gets a few lines; a long file stops at this and then at the window limit.
pub(crate) const PEEK_TEXT_MIN_LINES: i32 = 8;
pub(crate) const PEEK_TEXT_MAX_LINES: i32 = 10_000;

// ---------- F3 quick preview ----------

#[derive(Clone)]
pub(crate) struct PeekReq {
    pub(crate) path: String,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) hwnd: isize, // popup told by WM_PEEK_DONE
    pub(crate) plain: bool, // the preview handler failed: read the file ourselves
}

pub(crate) struct Pix {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) bgra: Vec<u8>, // premultiplied, top-down
    pub(crate) lay_w: i32,    // fit-render size; zoom swaps the buffer but layout stays on this
    pub(crate) lay_h: i32,
}

impl Pix {
    pub(crate) fn new(w: i32, h: i32, bgra: Vec<u8>) -> Self {
        Self { w, h, bgra, lay_w: w, lay_h: h }
    }

    /// Size the fit render produced. A sharper zoom buffer keeps this so layout does not double-scale.
    pub(crate) fn lay(&self) -> (i32, i32) {
        if self.lay_w > 0 && self.lay_h > 0 { (self.lay_w, self.lay_h) } else { (self.w, self.h) }
    }
}

/// PDF pages re-rendered at the zoomed size. Does not bump `gen` or clear the picture on screen.
#[derive(Clone)]
struct ZoomReq {
    gen: usize,
    seq: u64,
    stage_w: i32,
    zoom: f64,
}

// One worker, newest request only. gen moves on every want, so a result for an item already left
// (or a closed popup) is dropped instead of shown.
pub(crate) struct PeekJob {
    pub(crate) gen: usize,
    pub(crate) req: Option<PeekReq>,
    pub(crate) done: Option<Peeked>,
    zoom: Option<ZoomReq>,
}

pub(crate) struct Peeked {
    pub(crate) body: Body,
    pub(crate) icon: i32,          // system image list index for the header, -1 = none
    pub(crate) meta: Vec<String>,  // header facts: type, dimensions / pages / lines, size, date
}

impl PeekJob {
    pub(crate) fn want(&mut self, req: Option<PeekReq>) {
        self.zoom = None;
        (self.gen, self.req, self.done) = (self.gen + 1, req, None);
    }

    pub(crate) fn publish(&mut self, gen: usize, r: Peeked) -> bool {
        if self.gen != gen {
            return false;
        }
        self.done = Some(r);
        true
    }

    // One more PDF page for the item still on screen.
    pub(crate) fn add_page(&mut self, gen: usize, p: Pix) -> bool {
        match &mut self.done {
            Some(Peeked { body: Body::Pages { pages, .. }, .. }) if self.gen == gen => {
                pages.push(p);
                true
            }
            _ => false,
        }
    }

    /// Swap one PDF page for a sharper render. `lay_*` on `p` is the fit size and must already be set.
    fn put_zoom_page(&mut self, gen: usize, seq: u64, i: usize, p: Pix) -> bool {
        if self.gen != gen || self.zoom.as_ref().map(|z| z.seq) != Some(seq) {
            return false;
        }
        match &mut self.done {
            Some(Peeked { body: Body::Pages { pages, total }, .. }) if (i as u32) < *total => {
                if i < pages.len() {
                    let (lay_w, lay_h) = pages[i].lay();
                    pages[i] = Pix { lay_w, lay_h, ..p };
                    true
                } else if i == pages.len() {
                    pages.push(p);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

pub(crate) static PEEK: Mutex<PeekJob> = Mutex::new(PeekJob { gen: 0, req: None, done: None, zoom: None });
pub(crate) static PEEK_WAKE: Condvar = Condvar::new();

pub(crate) fn peek_job() -> std::sync::MutexGuard<'static, PeekJob> {
    PEEK.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn want_peek(req: Option<PeekReq>) {
    let wake = req.is_some();
    peek_job().want(req);
    if wake {
        static WORKER: Once = Once::new();
        WORKER.call_once(|| _ = std::thread::Builder::new().name("maguro-peek".into()).spawn(peek_worker));
        PEEK_WAKE.notify_one();
    }
}

// Lives until the process ends. COM is up only while one preview is made; the lock is never held then.
pub(crate) fn peek_worker() {
    let mut seen = 0usize;
    loop {
        let (gen, req) = {
            let mut j = peek_job();
            loop {
                let fresh = j.req.is_some() && j.gen != seen;
                // A zoom re-render for the file already on screen. A new file still goes first.
                let zooming = j.zoom.as_ref().is_some_and(|z| z.gen == j.gen && j.done.is_some());
                if fresh || zooming {
                    break;
                }
                j = PEEK_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
            let fresh = j.req.is_some() && j.gen != seen;
            (j.gen, if fresh { j.req.clone() } else { None })
        };
        unsafe {
            let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            if let Some(req) = req {
                seen = gen;
                render_peek(gen, &req);
            }
            render_zoom();
            if com {
                CoUninitialize();
            }
        }
    }
}

unsafe fn render_peek(gen: usize, req: &PeekReq) {
    let body = match crate::crash::guard(std::panic::AssertUnwindSafe(|| unsafe { content(req) })) {
        Ok(b) => b,
        Err(_) => Body::Card {
            icon: None,
            rows: vec![(
                tr(crate::Key::Preview).to_string(),
                tr(crate::Key::ThisFileCouldNotBePreviewed).to_string(),
            )],
        },
    };
    let (icon, meta) = meta(&req.path, &body);
    let more = match &body {
        Body::Pages { pages, total } if (pages.len() as u32) < *total => Some(*total),
        _ => None,
    };
    let tell = || _ = PostMessageW(Some(HWND(req.hwnd as _)), WM_PEEK_DONE, WPARAM(gen), LPARAM(0));
    if peek_job().publish(gen, Peeked { body, icon, meta }) {
        tell();
        // the first page is up; the rest follow while this is still the item shown
        if let (Some(total), Ok(doc)) = (more, pdf_open(&req.path)) {
            for i in 1..total {
                // Zoom wants every page at the new size; stop the fit-size pass and let render_zoom finish them.
                if peek_job().zoom.as_ref().is_some_and(|z| z.gen == gen) {
                    break;
                }
                let Some(p) = (peek_job().gen == gen).then(|| pdf_page(&doc, i, req.w, req.h)).flatten() else { break };
                if !peek_job().add_page(gen, p) {
                    break;
                }
                tell();
            }
        }
    }
}

fn zoom_current(gen: usize, seq: u64) -> bool {
    let j = peek_job();
    j.gen == gen && j.zoom.as_ref().is_some_and(|z| z.seq == seq)
}

fn clear_zoom(gen: usize, seq: u64) {
    let mut j = peek_job();
    if j.zoom.as_ref().is_some_and(|z| z.gen == gen && z.seq == seq) {
        j.zoom = None;
    }
}

// Sharper PDF pages for the zoom already on screen. The fit-size `lay_*` is kept so paint
// still scales from fit; until a page lands, paint stretches the bitmap it has (HALFTONE).
unsafe fn render_zoom() {
    let ready = {
        let j = peek_job();
        match (j.zoom.clone(), j.req.as_ref(), j.done.as_ref()) {
            (Some(z), Some(req), Some(Peeked { body: Body::Pages { .. }, .. })) if z.gen == j.gen => Some((z, req.path.clone(), req.hwnd, req.w, req.h)),
            _ => None,
        }
    };
    let Some((z, path, hwnd, fit_w, fit_h)) = ready else { return };
    let Ok(doc) = pdf_open(&path) else {
        clear_zoom(z.gen, z.seq);
        return;
    };
    let total = {
        let j = peek_job();
        match &j.done {
            Some(Peeked { body: Body::Pages { total, .. }, .. }) if j.gen == z.gen => (*total).min(PDF_MAX),
            _ => return,
        }
    };
    for i in 0..total {
        if !zoom_current(z.gen, z.seq) {
            return;
        }
        let have = {
            let j = peek_job();
            match &j.done {
                Some(Peeked { body: Body::Pages { pages, .. }, .. }) => pages.get(i as usize).map(|p| (p.lay(), p.w, p.h)),
                _ => None,
            }
        };
        let lay = if let Some((lay, _, _)) = have {
            lay
        } else {
            let Ok(page) = doc.GetPage(i) else { break };
            let Ok(size) = page.Size() else { break };
            let fit = pdf_fit_px(size.Width, size.Height, fit_w, fit_h);
            if fit.0 <= 0 || fit.1 <= 0 {
                break;
            }
            fit
        };
        let (rw, rh) = zoom_render_px(lay.0, lay.1, z.stage_w, z.zoom);
        // A bitmap already this large (the fit render is often the max stage) stays sharp when scaled down.
        if let Some((_, bw, bh)) = have {
            if bw + rw / 32 >= rw && bh + rh / 32 >= rh {
                continue;
            }
        }
        let Some(mut pix) = crate::crash::guard(std::panic::AssertUnwindSafe(|| unsafe { pdf_page(&doc, i, rw, rh) })).ok().flatten() else { break };
        (pix.lay_w, pix.lay_h) = lay;
        if !peek_job().put_zoom_page(z.gen, z.seq, i as usize, pix) {
            return;
        }
        _ = PostMessageW(Some(HWND(hwnd as _)), WM_PEEK_ZOOM, WPARAM(z.gen), LPARAM(0));
    }
    clear_zoom(z.gen, z.seq);
}

// Photos come back with every alpha byte 0: make them opaque. Icons come back with straight alpha (a colour
// brighter than its alpha says so), which AlphaBlend would draw as a dirty fringe: premultiply those.
pub(crate) fn fix_alpha(bgra: &mut [u8]) {
    if bgra.chunks_exact(4).all(|p| p[3] == 0) {
        bgra.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    } else if bgra.chunks_exact(4).any(|p| p[0].max(p[1]).max(p[2]) > p[3]) {
        for p in bgra.chunks_exact_mut(4) {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

// Accent line around the popup: 3px at 96 dpi, and never thinner than one pixel.
fn peek_edge(dpi: i32) -> i32 {
    (3 * dpi / 96).max(1)
}

// The popup is laid out like the main window: the frame colour around, content on a plate.
// Everything sits inside `peek_edge`, so the accent border neither covers it nor gets covered.
pub(crate) struct PeekGeo {
    pub(crate) icon: RECT,  // file icon in the header
    pub(crate) title: RECT, // name, then the facts
    pub(crate) max: RECT,   // maximise / restore
    pub(crate) close: RECT,
    pub(crate) stage: RECT, // pictures and pages sit here; everything else on a plate filling it
    pub(crate) inner: RECT, // the plate's content box
    pub(crate) prev: RECT,  // ‹ and › in the side margins
    pub(crate) next: RECT,
}

pub(crate) fn peek_geo(rc: RECT, dpi: i32) -> PeekGeo {
    let s = |v: i32| v * dpi / 96;
    let e = peek_edge(dpi);
    let rc = RECT { left: rc.left + e, top: rc.top + e, right: rc.right - e, bottom: rc.bottom - e };
    let (head, side, b, ic, pad) = (s(48), s(56), s(32), s(16), s(20));
    let close = RECT { left: rc.right - s(12) - b, top: rc.top + (head - b) / 2, right: rc.right - s(12), bottom: rc.top + (head + b) / 2 };
    let max = RECT { left: close.left - s(4) - b, top: close.top, right: close.left - s(4), bottom: close.bottom };
    let icon = RECT { left: rc.left + side, top: rc.top + (head - ic) / 2, right: rc.left + side + ic, bottom: rc.top + (head + ic) / 2 };
    let title = RECT { left: icon.right + s(10), top: rc.top, right: max.left - s(8), bottom: rc.top + head };
    let stage = RECT { left: rc.left + side, top: rc.top + head, right: rc.right - side, bottom: rc.bottom - s(24) };
    let inner = RECT { left: stage.left + pad, top: stage.top + pad, right: stage.right - pad, bottom: stage.bottom - pad };
    let mid = (stage.top + stage.bottom) / 2;
    let arrow = |x: i32| RECT { left: x, top: mid - b / 2, right: x + b, bottom: mid + b / 2 };
    PeekGeo { icon, title, max, close, stage, inner, prev: arrow(rc.left + (side - b) / 2), next: arrow(rc.right - side + (side - b) / 2) }
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(crate) enum Hot {
    #[default]
    None,
    Prev,
    Next,
    Max,
    Close,
}

pub(crate) fn peek_hit(g: &PeekGeo, x: i32, y: i32) -> Hot {
    match () {
        _ if inside(g.close, x, y) => Hot::Close,
        _ if inside(g.max, x, y) => Hot::Max,
        _ if inside(g.prev, x, y) => Hot::Prev,
        _ if inside(g.next, x, y) => Hot::Next,
        _ => Hot::None,
    }
}

// ---------- window size ----------
// Pure geometry. The window procedure only measures the font, the parent, and the work area.

/// Width and height in one unit: screen pixels, or DIP. Callers do not mix them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PeekSize {
    pub w: i32,
    pub h: i32,
}

/// Pixels the frame, header, and margins add around the content box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PeekChrome {
    pub w: i32,
    pub h: i32,
}

/// What the file wants the content box to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FitContent {
    /// One image or one PDF page. The box keeps this aspect and does not grow past these pixels.
    Aspect { w: i32, h: i32 },
    /// Text or code. `char_px` / `line_px` are the peek font. `lines` is the file's line count.
    /// `gutter` is extra width (the code gutter, a little padding), in the same unit.
    Text { char_px: i32, line_px: i32, lines: i32, gutter: i32 },
}

/// Pictures remember one size. Text and PDF share the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PeekKind {
    Image,
    Doc,
}

impl PeekKind {
    pub(crate) fn of(body: &Body) -> Self {
        match body {
            Body::Pic(_) => PeekKind::Image,
            _ => PeekKind::Doc,
        }
    }
}

/// Aspect, or a line count, taken from a loaded preview. Font metrics stay with the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FitSource {
    Aspect { w: i32, h: i32 },
    Lines { lines: i32, code: bool },
}

pub(crate) fn fit_source(body: &Body) -> FitSource {
    match body {
        Body::Pic(p) => FitSource::Aspect { w: p.w, h: p.h },
        Body::Pages { pages, .. } => match pages.first() {
            Some(p) if p.w > 0 && p.h > 0 => FitSource::Aspect { w: p.w, h: p.h },
            _ => FitSource::Lines { lines: 40, code: false },
        },
        Body::Text { text, kind, .. } => FitSource::Lines { lines: text.lines().count() as i32, code: *kind == TextKind::Code },
        // Same line-count fit as plain text. The preview image is added to scrollable content while painting.
        Body::Document { text, .. } => FitSource::Lines { lines: text.lines().count() as i32, code: false },
        Body::Table { rows, .. } => FitSource::Lines { lines: rows.len() as i32, code: false },
        Body::List { rows, .. } => FitSource::Lines { lines: rows.len() as i32, code: false },
        Body::Card { rows, .. } => FitSource::Lines { lines: (rows.len() as i32).max(12), code: false },
        Body::Handler(_) => FitSource::Lines { lines: 40, code: false },
    }
}

/// `None` keeps the window where it is: maximized, or the user already resized this kind.
/// A remembered size wins over fitting the file.
pub(crate) fn pick_open_size(remembered: Option<PeekSize>, user_resized: bool, maximized: bool, fitted: PeekSize) -> Option<PeekSize> {
    if maximized || user_resized {
        None
    } else if let Some(size) = remembered {
        Some(size)
    } else {
        Some(fitted)
    }
}

fn fit_aspect(src_w: i32, src_h: i32, limit: PeekSize) -> PeekSize {
    if src_w <= 0 || src_h <= 0 || limit.w <= 0 || limit.h <= 0 {
        return PeekSize { w: 0, h: 0 };
    }
    let (sw, sh) = (src_w as i64, src_h as i64);
    let (lim_w, lim_h) = (limit.w as i64, limit.h as i64);
    // Width-bound when the source is at least as wide, relative to the limit, as it is tall.
    if sw * lim_h >= lim_w * sh {
        let w = lim_w.min(sw);
        PeekSize { w: w as i32, h: ((w * sh) / sw).max(1) as i32 }
    } else {
        let h = lim_h.min(sh);
        PeekSize { w: ((h * sw) / sh).max(1) as i32, h: h as i32 }
    }
}

fn fit_text(char_px: i32, line_px: i32, lines: i32, gutter: i32, limit: PeekSize) -> PeekSize {
    let char_px = char_px.max(1);
    let line_px = line_px.max(1);
    let lines = lines.clamp(PEEK_TEXT_MIN_LINES, PEEK_TEXT_MAX_LINES);
    let want_w = char_px.saturating_mul(PEEK_TEXT_COLS).saturating_add(gutter.max(0));
    let want_h = line_px.saturating_mul(lines);
    PeekSize { w: want_w.min(limit.w.max(0)), h: want_h.min(limit.h.max(0)) }
}

/// Window size that wraps `content` in `chrome` and stays inside `limit`.
/// Images and PDF pages keep their aspect ratio. Text is about 100 characters wide.
pub(crate) fn fit_size(content: FitContent, chrome: PeekChrome, limit: PeekSize) -> PeekSize {
    let limit = PeekSize { w: limit.w.max(0), h: limit.h.max(0) };
    let chrome = PeekChrome { w: chrome.w.max(0), h: chrome.h.max(0) };
    let inner_lim = PeekSize { w: (limit.w - chrome.w).max(0), h: (limit.h - chrome.h).max(0) };
    let inner = match content {
        FitContent::Aspect { w, h } => fit_aspect(w, h, inner_lim),
        FitContent::Text { char_px, line_px, lines, gutter } => fit_text(char_px, line_px, lines, gutter, inner_lim),
    };
    PeekSize { w: inner.w.saturating_add(chrome.w).min(limit.w), h: inner.h.saturating_add(chrome.h).min(limit.h) }
}

/// Shrink `size` so it is no larger than the main window or the monitor work area.
pub(crate) fn clamp_to_parent_and_work(size: PeekSize, parent: PeekSize, work: PeekSize) -> PeekSize {
    let max_w = parent.w.min(work.w).max(0);
    let max_h = parent.h.min(work.h).max(0);
    PeekSize { w: size.w.clamp(0, max_w), h: size.h.clamp(0, max_h) }
}

/// Slide `r` so it lies fully inside `work`. A window larger than the work area pins to its origin.
fn slide_inside(r: RECT, work: RECT) -> RECT {
    let w = (r.right - r.left).max(0).min((work.right - work.left).max(0));
    let h = (r.bottom - r.top).max(0).min((work.bottom - work.top).max(0));
    let left = if work.right - work.left <= w { work.left } else { r.left.clamp(work.left, work.right - w) };
    let top = if work.bottom - work.top <= h { work.top } else { r.top.clamp(work.top, work.bottom - h) };
    RECT { left, top, right: left + w, bottom: top + h }
}

/// Center a window of `size` on `parent`, then keep it inside `work`.
pub(crate) fn center_in(size: PeekSize, parent: RECT, work: RECT) -> RECT {
    let w = size.w.max(0);
    let h = size.h.max(0);
    let left = parent.left + (parent.right - parent.left - w) / 2;
    let top = parent.top + (parent.bottom - parent.top - h) / 2;
    slide_inside(RECT { left, top, right: left + w, bottom: top + h }, work)
}

fn rect_size(r: RECT) -> PeekSize {
    PeekSize { w: (r.right - r.left).max(0), h: (r.bottom - r.top).max(0) }
}

fn px_to_dip(px: i32, dpi: i32) -> i32 {
    let dpi = dpi.max(1) as i64;
    ((px.max(0) as i64 * 96 + dpi / 2) / dpi) as i32
}

fn dip_to_px(dip: i32, dpi: i32) -> i32 {
    (dip.max(0) as i64 * dpi.max(1) as i64 / 96) as i32
}

/// Frame around the stage (`inner` false) or around the plate's content box.
pub(crate) fn peek_chrome(dpi: i32, inner: bool) -> PeekChrome {
    let s = |v: i32| v * dpi / 96;
    let e = peek_edge(dpi);
    let (head, side, bottom, pad) = (s(48), s(56), s(24), s(20));
    let mut w = e + side + side + e;
    let mut h = e + head + bottom + e;
    if inner {
        w += pad * 2;
        h += pad * 2;
    }
    PeekChrome { w, h }
}

/// Smallest window the user can drag to, at this DPI. The parent can force it smaller.
fn min_peek(dpi: i32) -> PeekSize {
    let s = |v: i32| v * dpi / 96;
    PeekSize { w: s(360), h: s(240) }
}

/// Resize grip, in px. Wider than the painted accent line so the edge is grabbable.
fn peek_grip(dpi: i32) -> i32 {
    (6 * dpi / 96).max(4)
}

/// Hit test for the sizing border. The header drag is a client click, not a caption hit,
/// so a double-click there still reaches the window procedure.
pub(crate) fn peek_nc_hit(rc: RECT, x: i32, y: i32, grip: i32) -> u32 {
    let grip = grip.max(1);
    if x < rc.left || y < rc.top || x >= rc.right || y >= rc.bottom {
        return HTNOWHERE;
    }
    let left = x < rc.left + grip;
    let right = x >= rc.right - grip;
    let top = y < rc.top + grip;
    let bottom = y >= rc.bottom - grip;
    match (left, right, top, bottom) {
        (true, _, true, _) => HTTOPLEFT,
        (_, true, true, _) => HTTOPRIGHT,
        (true, _, _, true) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (true, _, _, _) => HTLEFT,
        (_, true, _, _) => HTRIGHT,
        (_, _, true, _) => HTTOP,
        (_, _, _, true) => HTBOTTOM,
        _ => HTCLIENT,
    }
}

/// The header strip, inside the grip, and not on a button. Dragging it moves the window.
pub(crate) fn in_header(rc: RECT, g: &PeekGeo, x: i32, y: i32, grip: i32) -> bool {
    x >= rc.left + grip && x < rc.right - grip && y >= rc.top + grip && y < g.stage.top && peek_hit(g, x, y) == Hot::None
}

unsafe fn geo_of(h: HWND) -> PeekGeo {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    peek_geo(rc, app(|a| a.dpi))
}

// UI-thread state of the open preview: scroll position, laid-out text, the hosted handler.
#[derive(Default)]
pub(crate) struct PeekView {
    pub(crate) y: i32, // scrolled down, px
    pub(crate) x: i32, // scrolled right (tables, code, zoomed pictures), px
    pub(crate) max_y: i32, // how far it can scroll down, from the last paint; 0 = it all fits
    pub(crate) max_x: i32, // how far a zoomed picture can scroll right; 0 hides the horizontal bar
    pub(crate) page_h: i32, // viewport height from that paint; the thumb is sized from it
    pub(crate) page_w: i32, // viewport width from that paint; the horizontal thumb is sized from it
    pub(crate) edge_at: Option<Instant>, // the wheel reached the top / bottom then
    pub(crate) wheel: i32, // wheel delta toward the next file step (120 a notch)
    pub(crate) layout: Option<(usize, i32, Vec<Line>, i32)>, // (gen, width, lines, total height)
    pub(crate) fonts: Option<(i32, [HFONT; 4])>,              // (dpi, [mono, bold, h1, h2])
    pub(crate) hosted: Option<usize>, // generation a preview handler is showing
    pub(crate) host_win: Option<HWND>, // the picture area's child window the handler draws in
    pub(crate) hover: bool,            // the mouse is over the popup: ‹ › show
    pub(crate) hot: Hot,
    pub(crate) bar_hot: bool,         // pointer over the vertical scrollbar thumb
    pub(crate) bar_drag: Option<i32>, // mouse y minus thumb top, while that thumb is dragged
    pub(crate) hbar_hot: bool,        // pointer over the horizontal scrollbar thumb
    pub(crate) hbar_drag: Option<i32>, // mouse x minus thumb left, while that thumb is dragged
    pub(crate) bar_down: bool,        // mouse-down hit a bar, so mouse-up is not ‹ › / ×
    restore: Option<RECT>,            // normal rect, in screen px, to return to from maximise
    maximized: bool,
    sized_image: bool, // the user resized this kind while this preview is open
    sized_doc: bool,
    sized_pending: bool, // resized before the file's kind was known
    kind: Option<PeekKind>,
    fit_gen: usize, // generation that already received a window size; later PDF pages do not resize
    grip: Option<(i32, i32)>, // window size when a move or resize started
    drag_pt: Option<(i32, i32)>, // caption press, client px, before it becomes a drag
    suppress_up: bool, // the mouse-up after a double-click must not fire the button again
    tip_on: bool,
    zoom: f64, // multiple of fit; 0 and 1 both mean fit. Header percent: see the zoom section.
    zoom_seq: u64,                   // last PDF re-render asked for
    pan_pt: Option<(i32, i32)>,      // client point while a content drag may become a pan
    pan_drag: bool,
}

thread_local! {
    pub(crate) static VIEW: RefCell<PeekView> = RefCell::new(PeekView::default());
}

fn view<R>(f: impl FnOnce(&mut PeekView) -> R) -> R {
    VIEW.with(|v| f(&mut v.borrow_mut()))
}

// New file: back to the top. A drag still holding the mouse lets go too.
unsafe fn reset_scroll() {
    let captured = view(|v| {
        (v.y, v.x, v.layout, v.max_y, v.max_x, v.edge_at, v.wheel) = (0, 0, None, 0, 0, None, 0);
        v.zoom = 1.0;
        let captured = v.bar_drag.take().is_some() || v.hbar_drag.take().is_some() || v.bar_down || v.pan_pt.take().is_some() || v.pan_drag;
        v.pan_drag = false;
        (v.page_h, v.page_w, v.bar_hot, v.hbar_hot, v.bar_down) = (0, 0, false, false, false);
        captured
    });
    if captured {
        _ = ReleaseCapture();
    }
}

/// Heading faces are cloned from the UI font. Drop them when that font changes; the next paint builds new ones.
pub(crate) unsafe fn drop_peek_fonts() {
    let old = view(|v| {
        v.layout = None;
        v.fonts.take()
    });
    if let Some((_, fonts)) = old {
        for f in fonts {
            if !f.is_invalid() {
                _ = DeleteObject(f.into());
            }
        }
    }
}

fn reset_frame() {
    view(|v| {
        v.restore = None;
        v.maximized = false;
        v.sized_image = false;
        v.sized_doc = false;
        v.sized_pending = false;
        v.kind = None;
        v.fit_gen = 0;
        v.grip = None;
        v.drag_pt = None;
        v.suppress_up = false;
        v.tip_on = false;
        v.zoom = 1.0;
        v.pan_pt = None;
        v.pan_drag = false;
    });
}

/// Drop a pending PDF re-render. The next file, and fit, must not apply it.
unsafe fn drop_zoom(h: HWND) {
    _ = KillTimer(Some(h), TIMER_PEEK_ZOOM);
    peek_job().zoom = None;
}

struct Bounds {
    parent: RECT,
    work: RECT,
}

unsafe fn main_bounds() -> Bounds {
    let main = app(|a| a.main);
    let mut parent = RECT::default();
    _ = GetWindowRect(main, &mut parent);
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let ok = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool();
    let work = if ok && mi.rcWork.right > mi.rcWork.left && mi.rcWork.bottom > mi.rcWork.top { mi.rcWork } else { parent };
    Bounds { parent, work }
}

fn open_limit(b: &Bounds) -> PeekSize {
    clamp_to_parent_and_work(PeekSize { w: i32::MAX / 8, h: i32::MAX / 8 }, rect_size(b.parent), rect_size(b.work))
}

unsafe fn window_rect(h: HWND) -> RECT {
    let mut r = RECT::default();
    _ = GetWindowRect(h, &mut r);
    r
}

unsafe fn peek_font_px(font: HFONT) -> (i32, i32) {
    if font.is_invalid() {
        return (8, 16);
    }
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut tm = TEXTMETRICW::default();
    _ = GetTextMetricsW(dc, &mut tm);
    let sample: Vec<u16> = "0000000000".encode_utf16().collect();
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &sample, &mut sz);
    SelectObject(dc, old);
    _ = ReleaseDC(None, dc);
    ((sz.cx / 10).max(1), (tm.tmHeight + tm.tmExternalLeading).max(1))
}

unsafe fn fitted_px(source: FitSource, dpi: i32, limit: PeekSize) -> PeekSize {
    let (char_px, line_px) = peek_font_px(app(|a| a.font));
    let s = |v: i32| v * dpi / 96;
    let (content, inner) = match source {
        FitSource::Aspect { w, h } => (FitContent::Aspect { w, h }, false),
        FitSource::Lines { lines, code } => {
            let gutter = if code { s(44) + s(8) } else { s(8) };
            (FitContent::Text { char_px, line_px, lines, gutter }, true)
        }
    };
    let fitted = fit_size(content, peek_chrome(dpi, inner), limit);
    let min = min_peek(dpi);
    PeekSize { w: fitted.w.max(min.w), h: fitted.h.max(min.h) }
}

unsafe fn default_peek_size(dpi: i32, limit: PeekSize) -> PeekSize {
    let (char_px, line_px) = peek_font_px(app(|a| a.font));
    let s = |v: i32| v * dpi / 96;
    let fitted = fit_size(FitContent::Text { char_px, line_px, lines: 32, gutter: s(8) }, peek_chrome(dpi, true), limit);
    let min = min_peek(dpi);
    PeekSize { w: fitted.w.max(min.w).min(limit.w), h: fitted.h.max(min.h).min(limit.h) }
}

unsafe fn place_peek(h: HWND, size: PeekSize) {
    let b = main_bounds();
    let size = clamp_to_parent_and_work(size, rect_size(b.parent), rect_size(b.work));
    let cur = window_rect(h);
    let cur_size = rect_size(cur);
    // Same size: keep a window the user has dragged. A new size is centered on the parent.
    let rect = if (cur_size.w - size.w).abs() <= 1 && (cur_size.h - size.h).abs() <= 1 {
        slide_inside(RECT { left: cur.left, top: cur.top, right: cur.left + size.w, bottom: cur.top + size.h }, b.work)
    } else {
        center_in(size, b.parent, b.work)
    };
    if cur != rect {
        _ = SetWindowPos(h, None, rect.left, rect.top, (rect.right - rect.left).max(1), (rect.bottom - rect.top).max(1), SWP_NOZORDER | SWP_NOACTIVATE);
    }
    view(|v| {
        if !v.maximized {
            v.restore = Some(rect);
        }
    });
}

fn remembered_dip(kind: PeekKind) -> Option<(i32, i32)> {
    app(|a| match kind {
        PeekKind::Image => a.s.peek_image,
        PeekKind::Doc => a.s.peek_doc,
    })
}

unsafe fn remember_peek(h: HWND, kind: PeekKind) {
    if view(|v| v.maximized) {
        return;
    }
    let r = window_rect(h);
    let dpi = app(|a| a.dpi.max(1));
    let pair = (px_to_dip(r.right - r.left, dpi), px_to_dip(r.bottom - r.top, dpi));
    if pair.0 < 1 || pair.1 < 1 {
        return;
    }
    app(|a| match kind {
        PeekKind::Image => a.s.peek_image = Some(pair),
        PeekKind::Doc => a.s.peek_doc = Some(pair),
    });
    // Same path as the other maguro.json settings. The maximized rect is never written.
    crate::save_session();
}

/// First result for this file: fit, or take the remembered size. Later pages, a maximize,
/// and a size the user already set for this kind leave the window alone.
unsafe fn fit_on_result(h: HWND, gen: usize) {
    let (kind, source) = {
        let j = peek_job();
        if j.gen != gen {
            return;
        }
        let Some(done) = j.done.as_ref() else { return };
        (PeekKind::of(&done.body), fit_source(&done.body))
    };
    let first = view(|v| {
        if v.fit_gen == gen {
            return false;
        }
        v.fit_gen = gen;
        v.kind = Some(kind);
        true
    });
    if !first {
        return;
    }
    if view(|v| v.sized_pending) {
        view(|v| {
            v.sized_pending = false;
            match kind {
                PeekKind::Image => v.sized_image = true,
                PeekKind::Doc => v.sized_doc = true,
            }
        });
        remember_peek(h, kind);
        return;
    }
    let (user_resized, maximized) = view(|v| {
        let user = match kind {
            PeekKind::Image => v.sized_image,
            PeekKind::Doc => v.sized_doc,
        };
        (user, v.maximized)
    });
    let dpi = app(|a| a.dpi);
    let b = main_bounds();
    let limit = open_limit(&b);
    let remembered = remembered_dip(kind).map(|(w, h)| PeekSize { w: dip_to_px(w, dpi), h: dip_to_px(h, dpi) });
    let fitted = fitted_px(source, dpi, limit);
    let Some(size) = pick_open_size(remembered, user_resized, maximized, fitted) else { return };
    place_peek(h, size);
}

/// Stage size the worker should render into. Pictures and PDF pages are drawn at the largest
/// stage the parent and the work area allow, so fitting the window to that bitmap does not
/// have to scale it up. A maximized preview renders at the plate it is actually showing.
unsafe fn request_stage(h: HWND) -> (i32, i32) {
    if view(|v| v.maximized) {
        let st = geo_of(h).stage;
        return ((st.right - st.left).max(16), (st.bottom - st.top).max(16));
    }
    let dpi = app(|a| a.dpi);
    let limit = open_limit(&main_bounds());
    let chrome = peek_chrome(dpi, false);
    ((limit.w - chrome.w).max(16), (limit.h - chrome.h).max(16))
}

unsafe fn create_peek(i: usize) -> bool {
    let (main, dpi, open) = app(|a| (a.main, a.dpi, a.peek));
    if !open.0.is_null() || main.0.is_null() {
        return false;
    }
    reset_frame();
    // Used to be 86% × 88% of the main client, capped at 1120×860 DIP, and not resizable.
    // The placeholder is a readable text size; the first result refits before its paint.
    let b = main_bounds();
    let limit = open_limit(&b);
    let size = default_peek_size(dpi, limit);
    let rect = center_in(clamp_to_parent_and_work(size, rect_size(b.parent), rect_size(b.work)), b.parent, b.work);
    let (w, h) = ((rect.right - rect.left).max(1), (rect.bottom - rect.top).max(1));
    let style = WS_POPUP | WS_THICKFRAME | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(hw) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("MaguroPeek"), w!(""), style, rect.left, rect.top, w, h, Some(main), None, None, None) else {
        return false;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(hw, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    let policy = DWMNCRP_DISABLED;
    _ = DwmSetWindowAttribute(hw, DWMWA_NCRENDERING_POLICY, &policy as *const _ as _, 4);
    let none = DWMWA_COLOR_NONE;
    _ = DwmSetWindowAttribute(hw, DWMWA_BORDER_COLOR, &none as *const _ as _, 4);
    view(|v| v.restore = Some(rect));
    app(|a| (a.peek, a.peek_pane) = (hw, i));
    true
}

pub(crate) unsafe fn open_peek(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    if !app(|a| a.peek.0.is_null()) {
        return;
    }
    if fv.GetFocusedItem().unwrap_or(-1) < 0 {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    if !create_peek(i) {
        return;
    }
    mark_find_peek(None);
    load_peek(false);
}

/// Quick look at one search result. Arrows then walk the result list, not the folder behind it.
pub(crate) unsafe fn open_peek_path(i: usize, path: &str, name: &str) {
    if !app(|a| a.peek.0.is_null()) {
        return;
    }
    if path.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    // Set before the popup exists, so a hide timer that lands during creation keeps this pane's rows.
    mark_find_peek(Some(i));
    if !create_peek(i) {
        mark_find_peek(None);
        return;
    }
    app(|a| a.peek_name = name.to_string());
    show_path(path, false);
}

unsafe fn show_path(path: &str, plain: bool) {
    let h = app(|a| a.peek);
    if h.0.is_null() {
        return;
    }
    unload_handler();
    drop_zoom(h);
    reset_scroll();
    let (w, ht) = request_stage(h);
    want_peek(Some(PeekReq { path: path.to_string(), w, h: ht, hwnd: h.0 as isize, plain }));
    _ = InvalidateRect(Some(h), None, false);
}

/// Theme changed under an open quick look. Drawn pages follow the palette on the next paint.
/// A Windows preview handler was started with the old colours, so it is opened again.
pub(crate) unsafe fn retint_peek() {
    let h = app(|a| a.peek);
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    let dark = DARK_ON.load(Ordering::Relaxed);
    super::theme_hwnd(h, dark);
    if let Some(host) = view(|v| v.host_win) {
        super::theme_hwnd(host, dark);
    }
    _ = InvalidateRect(Some(h), None, true);
    if HOST_GEN.load(Ordering::SeqCst) != 0 {
        load_peek(false);
    }
}

// Asks the worker for the pane's focused item at the picture area's size. plain: skip the preview handler.
pub(crate) unsafe fn load_peek(plain: bool) {
    if FROM_FIND.load(Ordering::SeqCst) {
        let path = peek_job().req.as_ref().map(|r| r.path.clone());
        if let Some(path) = path {
            show_path(&path, plain);
            return;
        }
    }
    let (h, i) = app(|a| (a.peek, a.peek_pane));
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    unload_handler();
    drop_zoom(h);
    reset_scroll();
    let it = fv.GetItem::<IShellItem>(fv.GetFocusedItem().unwrap_or(-1)).ok();
    let shown = |kind| it.as_ref().and_then(|it| it.GetDisplayName(kind).ok()).map(|p| pwstr(p));
    app(|a| a.peek_name = shown(SIGDN_NORMALDISPLAY).unwrap_or_default());
    let (w, ht) = request_stage(h);
    match shown(SIGDN_FILESYSPATH).or_else(|| shown(SIGDN_DESKTOPABSOLUTEPARSING)) {
        Some(path) => want_peek(Some(PeekReq { path, w, h: ht, hwnd: h.0 as isize, plain })),
        None => {
            let gen = {
                let mut j = peek_job();
                j.want(None);
                j.done = Some(Peeked { body: Body::Card { icon: None, rows: Vec::new() }, icon: -1, meta: Vec::new() });
                j.gen
            };
            fit_on_result(h, gen);
        }
    }
    _ = InvalidateRect(Some(h), None, false);
}

// Next / previous in the view's own order. Only a real neighbour (S_OK, in range) that got selected moves it.
// A result preview walks the result list instead, and stays on this row at either end.
pub(crate) unsafe fn step_peek(prev: bool) {
    if FROM_FIND.load(Ordering::SeqCst) {
        let i = app(|a| a.peek_pane);
        if let Some(hit) = step_find_sel(i, prev) {
            app(|a| a.peek_name = hit.name);
            show_path(&hit.path, false);
        }
    } else {
        let Some(fv) = view_of::<IFolderView2>(app(|a| a.peek_pane)) else { return };
        let cur = fv.GetFocusedItem().unwrap_or(-1);
        let mut next = -1;
        let hr = (Interface::vtable(&fv).GetVisibleItem)(Interface::as_raw(&fv), cur, prev.into(), &mut next);
        let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
        let flags = SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0;
        if hr == S_OK && (0..n).contains(&next) && fv.SelectItem(next, flags as u32).is_ok() {
            load_peek(false);
        }
    }
    // selecting puts the focus in the list; the preview stays open and keeps the keys, like Quick Look
    let h = app(|a| a.peek);
    if !h.0.is_null() && GetActiveWindow() != h {
        _ = SetActiveWindow(h);
        _ = SetFocus(Some(h));
    }
}

pub(crate) unsafe fn close_peek(h: HWND) {
    let i = app(|a| a.peek_pane);
    let from_find = FROM_FIND.load(Ordering::SeqCst);
    _ = DestroyWindow(h);
    if !(from_find && focus_find_list(i)) {
        focus_view(i);
    }
}

/// Where preview should go after a successful Delete from the peek window.
enum AfterDel {
    /// Folder / find neighbour to show next (path + display name).
    Show { path: String, name: String },
    Close,
}

static AFTER_DEL: Mutex<Option<AfterDel>> = Mutex::new(None);

/// Keep peek alive across Maguro confirm / IFileOperation progress (avoids WA_INACTIVE → close).
static HOLD_PEEK: AtomicBool = AtomicBool::new(false);

fn take_after_del() -> Option<AfterDel> {
    AFTER_DEL.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn set_after_del(plan: Option<AfterDel>) {
    *AFTER_DEL.lock().unwrap_or_else(|e| e.into_inner()) = plan;
}

unsafe fn visible_item_path(fv: &IFolderView2, cur: i32, prev: bool) -> Option<(String, String)> {
    let mut next = -1;
    let hr = (Interface::vtable(fv).GetVisibleItem)(Interface::as_raw(fv), cur, prev.into(), &mut next);
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    if hr != S_OK || !(0..n).contains(&next) {
        return None;
    }
    let it = fv.GetItem::<IShellItem>(next).ok()?;
    let path = it
        .GetDisplayName(SIGDN_FILESYSPATH)
        .ok()
        .or_else(|| it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING).ok())
        .map(|p| pwstr(p))?;
    let name = it.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| pwstr(p)).unwrap_or_else(|| {
        Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone())
    });
    Some((path, name))
}

unsafe fn plan_after_peek_delete() -> AfterDel {
    if FROM_FIND.load(Ordering::SeqCst) {
        let i = app(|a| a.peek_pane);
        return match peek_find_neighbour(i) {
            Some(hit) => AfterDel::Show { path: hit.path, name: hit.name },
            None => AfterDel::Close,
        };
    }
    let Some(fv) = view_of::<IFolderView2>(app(|a| a.peek_pane)) else {
        return AfterDel::Close;
    };
    let cur = fv.GetFocusedItem().unwrap_or(-1);
    if let Some((path, name)) = visible_item_path(&fv, cur, false).or_else(|| visible_item_path(&fv, cur, true)) {
        AfterDel::Show { path, name }
    } else {
        AfterDel::Close
    }
}

unsafe fn select_peek_path(i: usize, path: &str) -> bool {
    let Some(fv) = view_of::<IFolderView2>(i) else {
        return false;
    };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let flags = (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    for k in 0..n {
        let Ok(it) = fv.GetItem::<IShellItem>(k) else {
            continue;
        };
        let shown = |kind| it.GetDisplayName(kind).ok().map(|p| pwstr(p));
        let item = shown(SIGDN_FILESYSPATH).or_else(|| shown(SIGDN_DESKTOPABSOLUTEPARSING));
        if item.as_deref() == Some(path) {
            return fv.SelectItem(k, flags).is_ok();
        }
    }
    false
}

unsafe fn apply_after_peek_delete(h: HWND) {
    HOLD_PEEK.store(false, Ordering::SeqCst);
    let plan = take_after_del();
    let i = app(|a| a.peek_pane);
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
    match plan {
        Some(AfterDel::Show { path, name }) => {
            if FROM_FIND.load(Ordering::SeqCst) {
                peek_find_select_path(i, &path);
                app(|a| a.peek_name = name);
                show_path(&path, false);
            } else if Path::new(&path).exists() && select_peek_path(i, &path) {
                load_peek(false);
            } else if Path::new(&path).exists() {
                app(|a| a.peek_name = name);
                show_path(&path, false);
            } else {
                close_peek(h);
                return;
            }
            if !h.0.is_null() && IsWindow(Some(h)).as_bool() && GetActiveWindow() != h {
                _ = SetActiveWindow(h);
                _ = SetFocus(Some(h));
            }
        }
        _ => {
            if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
                close_peek(h);
            }
        }
    }
}

/// Rename the file the preview is showing (`peek_job` path); keep peek on the new name.
pub(crate) unsafe fn rename_peek() {
    let h = app(|a| a.peek);
    if h.0.is_null() {
        return;
    }
    let Some(path) = peek_job().req.as_ref().map(|r| r.path.clone()).filter(|p| !p.is_empty()) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let cur_name = app(|a| a.peek_name.clone()).trim().to_string();
    let cur_name = if cur_name.is_empty() {
        Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone())
    } else {
        cur_name
    };
    // MaguroAsk deactivates peek; HOLD_PEEK keeps WA_INACTIVE from destroying it.
    HOLD_PEEK.store(true, Ordering::SeqCst);
    let owner = app(|a| a.main);
    let Some(typed) = crate::settings::ask_stem(owner, tr(crate::Key::Rename), &cur_name) else {
        HOLD_PEEK.store(false, Ordering::SeqCst);
        if IsWindow(Some(h)).as_bool() {
            _ = SetActiveWindow(h);
            _ = SetFocus(Some(h));
        }
        return;
    };
    let new_name = typed.trim().to_string();
    if new_name.is_empty() || new_name == cur_name || new_name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
        HOLD_PEEK.store(false, Ordering::SeqCst);
        if new_name != cur_name && !new_name.is_empty() {
            _ = MessageBeep(MB_ICONWARNING);
        }
        if IsWindow(Some(h)).as_bool() {
            _ = SetActiveWindow(h);
            _ = SetFocus(Some(h));
        }
        return;
    }
    let new_path = match Path::new(&path).parent() {
        Some(dir) => dir.join(&new_name),
        None => {
            HOLD_PEEK.store(false, Ordering::SeqCst);
            _ = MessageBeep(MB_ICONWARNING);
            return;
        }
    };
    let new_path_s = new_path.to_string_lossy().into_owned();
    let ok = match crate::run_rename_one(&path, &new_name) {
        Ok(()) => true,
        Err(e) => {
            let text = HSTRING::from(format!("{}{}", tr(crate::Key::RenameFailed), e.message()));
            _ = MessageBoxW(Some(owner), &text, w!("Maguro"), MB_ICONWARNING);
            false
        }
    };
    if !ok {
        HOLD_PEEK.store(false, Ordering::SeqCst);
        if IsWindow(Some(h)).as_bool() {
            _ = SetActiveWindow(h);
            _ = SetFocus(Some(h));
        }
        return;
    }
    let i = app(|a| a.peek_pane);
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
    app(|a| a.peek_name = new_name.clone());
    if let Some(req) = peek_job().req.as_mut() {
        req.path = new_path_s.clone();
    }
    if FROM_FIND.load(Ordering::SeqCst) {
        peek_find_select_path(i, &new_path_s);
        show_path(&new_path_s, false);
    } else if select_peek_path(i, &new_path_s) {
        load_peek(false);
    } else if Path::new(&new_path_s).exists() {
        show_path(&new_path_s, false);
    } else {
        close_peek(h);
        HOLD_PEEK.store(false, Ordering::SeqCst);
        return;
    }
    HOLD_PEEK.store(false, Ordering::SeqCst);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        _ = SetActiveWindow(h);
        _ = SetFocus(Some(h));
    }
}

/// Delete the file the preview is showing (`peek_job` path), not an arbitrary pane selection.
pub(crate) unsafe fn delete_peek(shift: bool) {
    let h = app(|a| a.peek);
    if h.0.is_null() {
        return;
    }
    let Some(path) = peek_job().req.as_ref().map(|r| r.path.clone()).filter(|p| !p.is_empty()) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let (ask, permanent) = app(|a| (a.s.confirm, shift || a.s.del == 1));
    // Confirm / shell progress deactivate peek; without HOLD_PEEK, WA_INACTIVE would DestroyWindow it.
    HOLD_PEEK.store(true, Ordering::SeqCst);
    if ask {
        let verb = if permanent {
            tr(crate::Key::PermanentlyDelete)
        } else {
            tr(crate::Key::MoveRecycleBin)
        };
        let name = app(|a| a.peek_name.clone());
        let label = if name.is_empty() {
            Path::new(&path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone())
        } else {
            name
        };
        let text = HSTRING::from(format!("{verb}\n{label}"));
        // Own on main (same as delete_selection) so the dialog is not a child of the peek surface.
        let owner = app(|a| a.main);
        if MessageBoxW(Some(owner), &text, w!("Maguro"), MB_YESNO | MB_ICONQUESTION) != IDYES {
            HOLD_PEEK.store(false, Ordering::SeqCst);
            if IsWindow(Some(h)).as_bool() {
                _ = SetActiveWindow(h);
                _ = SetFocus(Some(h));
            }
            return;
        }
    }
    let plan = plan_after_peek_delete();
    set_after_del(Some(plan));
    let owner = if permanent && !ask { app(|a| a.main.0 as isize) } else { 0 };
    if !crate::start_delete_notify(vec![path], permanent, owner, h, WM_PEEK_DELETED) {
        set_after_del(None);
        HOLD_PEEK.store(false, Ordering::SeqCst);
        _ = MessageBeep(MB_ICONWARNING);
        if IsWindow(Some(h)).as_bool() {
            _ = SetActiveWindow(h);
            _ = SetFocus(Some(h));
        }
    }
}

// ---------- Windows preview handlers (Office, Outlook, HTML, fonts, PDF readers) ----------

// The handler wanted now (its preview generation), 0 = none. A host thread whose generation is no longer
// this one unloads and quits, so Esc or an arrow never waits for a slow handler.
pub(crate) static HOST_GEN: AtomicUsize = AtomicUsize::new(0);

struct HostBox {
    gen: usize,
    w: i32,
    h: i32,
    seq: u64,
}

static HOST_BOX: Mutex<HostBox> = Mutex::new(HostBox { gen: 0, w: 0, h: 0, seq: 0 });
static HOST_TID: AtomicUsize = AtomicUsize::new(0);

fn host_lock() -> std::sync::MutexGuard<'static, HostBox> {
    HOST_BOX.lock().unwrap_or_else(|e| e.into_inner())
}

fn publish_host(gen: usize, w: i32, h: i32) {
    let mut b = host_lock();
    b.gen = gen;
    b.w = w.max(1);
    b.h = h.max(1);
    b.seq += 1;
}

fn host_area(gen: usize) -> Option<(u64, RECT)> {
    let b = host_lock();
    (b.gen == gen && b.w > 0 && b.h > 0).then_some((b.seq, RECT { left: 0, top: 0, right: b.w, bottom: b.h }))
}

unsafe fn apply_host_rect(ph: &IPreviewHandler, gen: usize, seen: &mut u64) {
    let Some((seq, area)) = host_area(gen) else { return };
    if seq == *seen {
        return;
    }
    *seen = seq;
    _ = ph.SetRect(&area);
}

/// Move the handler's child to the plate. The host thread applies the same rect via SetRect.
unsafe fn sync_host(h: HWND) {
    let Some(win) = view(|v| v.host_win) else { return };
    if !IsWindow(Some(win)).as_bool() {
        return;
    }
    let st = geo_of(h).stage;
    let line = 1;
    let w = (st.right - st.left - line * 2).max(1);
    let ht = (st.bottom - st.top - line * 2).max(1);
    _ = SetWindowPos(win, None, st.left + line, st.top + line, w, ht, SWP_NOZORDER | SWP_NOACTIVATE);
    let gen = HOST_GEN.load(Ordering::SeqCst);
    if gen == 0 {
        return;
    }
    publish_host(gen, w, ht);
    let tid = HOST_TID.load(Ordering::SeqCst) as u32;
    if tid != 0 {
        _ = PostThreadMessageW(tid, WM_PEEK_HOST_SIZE, WPARAM(gen), LPARAM(0));
    }
}

pub(crate) unsafe fn unload_handler() {
    HOST_GEN.store(0, Ordering::SeqCst);
    if let Some(w) = view(|v| {
        v.hosted = None;
        v.host_win.take()
    }) {
        _ = DestroyWindow(w); // the handler's window goes with it, even while it is still starting
    }
}

// UTF-8 HTML that doesn't name its charset: the old HTML previewer reads it as the ANSI code page, so it
// gets a BOM in front. Everything else is handed over as the file.
unsafe fn handler_stream(path: &str) -> Result<IStream> {
    let name = HSTRING::from(path);
    if matches!(ext_of(path).as_str(), "html" | "htm") {
        if let Ok(b) = std::fs::read(path) {
            let head = String::from_utf8_lossy(&b[..b.len().min(4096)]).to_ascii_lowercase();
            if !b.starts_with(&[0xEF, 0xBB, 0xBF]) && !head.contains("charset") && std::str::from_utf8(&b).is_ok() && !b.is_ascii() {
                let with_bom = [&[0xEF, 0xBB, 0xBF][..], &b].concat();
                return SHCreateMemStream(Some(&with_bom)).ok_or_else(|| Error::from(E_FAIL));
            }
        }
    }
    SHCreateStreamOnFileEx(&name, (STGM_READ | STGM_SHARE_DENY_NONE).0, 0, false, None)
}

// Made where Explorer makes them (prevhost.exe, out of process), given the file as a stream when they take one.
unsafe fn host_handler(h: HWND, clsid: &GUID, path: &str, area: RECT) -> Result<IPreviewHandler> {
    let ph: IPreviewHandler = CoCreateInstance(clsid, None, CLSCTX_LOCAL_SERVER).or_else(|_| CoCreateInstance(clsid, None, CLSCTX_INPROC_SERVER))?;
    let name = HSTRING::from(path);
    if let Ok(init) = ph.cast::<IInitializeWithStream>() {
        init.Initialize(&handler_stream(path)?, STGM_READ.0)?;
    } else if let Ok(init) = ph.cast::<IInitializeWithItem>() {
        let item: IShellItem = SHCreateItemFromParsingName(&name, None)?;
        init.Initialize(&item, STGM_READ.0)?;
    } else {
        ph.cast::<IInitializeWithFile>()?.Initialize(&name, STGM_READ.0)?;
    }
    if let Ok(vis) = ph.cast::<IPreviewHandlerVisuals>() {
        _ = vis.SetBackgroundColor(cref(pal().card));
        _ = vis.SetTextColor(cref(pal().text));
    }
    ph.SetWindow(h, &area)?;
    ph.DoPreview()?;
    Ok(ph)
}

// One thread per hosted preview: it starts the handler, tells the popup (WM_PEEK_HOSTED, lParam 1 = shown),
// keeps it while it is the one wanted, then unloads it on its own apartment.
fn host_thread(gen: usize, clsid: GUID, path: String, hwnd: isize, popup: isize, area: RECT) {
    unsafe {
        let tid = GetCurrentThreadId() as usize;
        HOST_TID.store(tid, Ordering::SeqCst);
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let h = HWND(hwnd as _);
        let (mut seen, area) = match host_area(gen) {
            Some((seq, area)) => (seq, area),
            None => (0, area),
        };
        let ph = host_handler(h, &clsid, &path, area);
        let wanted = || HOST_GEN.load(Ordering::SeqCst) == gen;
        if wanted() {
            _ = PostMessageW(Some(HWND(popup as _)), WM_PEEK_HOSTED, WPARAM(gen), LPARAM(ph.is_ok() as isize));
        }
        if let Ok(ph) = ph {
            let mut msg = MSG::default();
            while wanted() && IsWindow(Some(h)).as_bool() {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_PEEK_HOST_SIZE {
                        apply_host_rect(&ph, gen, &mut seen);
                    } else {
                        _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                apply_host_rect(&ph, gen, &mut seen);
                _ = MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
            }
            _ = ph.Unload();
        }
        if HOST_TID.load(Ordering::SeqCst) == tid {
            HOST_TID.store(0, Ordering::SeqCst);
        }
        if com {
            CoUninitialize();
        }
    }
}

// The worker said "a handler shows this": start it on its own thread, once per item.
unsafe fn start_handler(h: HWND, gen: usize) {
    let (clsid, path) = {
        let j = peek_job();
        match (&j.done, &j.req) {
            (Some(Peeked { body: Body::Handler(g), .. }), Some(r)) if j.gen == gen => (*g, r.path.clone()),
            _ => return,
        }
    };
    if HOST_GEN.swap(gen, Ordering::SeqCst) == gen {
        return;
    }
    // The handler fills the plate. `stage` is already inside the accent border (peek_geo); the extra
    // pixel keeps the child off the plate's own line, which WS_CLIPCHILDREN would otherwise clip.
    let st = geo_of(h).stage;
    let line = 1;
    let (w, ht) = ((st.right - st.left - line * 2).max(1), (st.bottom - st.top - line * 2).max(1));
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(win) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroPeekHost"), w!(""), style, st.left + line, st.top + line, w, ht, Some(h), None, None, None) else {
        return load_peek(true);
    };
    view(|v| v.host_win = Some(win));
    publish_host(gen, w, ht);
    let (hwnd, popup, area) = (win.0 as isize, h.0 as isize, RECT { left: 0, top: 0, right: w, bottom: ht });
    if std::thread::Builder::new().spawn(move || host_thread(gen, clsid, path, hwnd, popup, area)).is_err() {
        load_peek(true);
    }
}

// The handler's parent: the plate's colour (and "Opening…") until the handler draws over it.
pub(crate) extern "system" fn peek_host_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| peek_host_proc_inner(h, msg, wp, lp)))
}
fn peek_host_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        // With two windows open, act on the window this popup belongs to.
        bind_hwnd(h);
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let font = app(|a| a.font);
                buffered(h, |dc, rc| {
                    fill(dc, rc, pal().card);
                    text(dc, rc, tr(crate::Key::OpeningEscCancel), font, pal().sub, DT_CENTER);
                });
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// The host thread is done starting: shown, or read the file ourselves.
unsafe fn handler_ready(h: HWND, gen: usize, ok: bool) {
    if HOST_GEN.load(Ordering::SeqCst) != gen || peek_job().gen != gen {
        return;
    }
    if ok {
        view(|v| v.hosted = Some(gen));
        // the handler's window may have taken the focus: arrows and Esc stay ours
        if GetActiveWindow() == h {
            _ = SetFocus(Some(h));
        }
    } else {
        load_peek(true);
    }
    _ = InvalidateRect(Some(h), None, false);
}

// ---------- text layout ----------

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Style {
    Plain,
    Mono, // not wrapped
    H1,
    H2,
    H3,
    Bullet,
    Quote,
}

pub(crate) struct Line {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) y: i32,
    pub(crate) h: i32,
    pub(crate) no: usize, // source line number, for the code gutter
    table: Option<(Vec<Vec<String>>, Vec<i32>)>,
}

/// Markdown lines with their look: headings, bullets, quotes, fenced code. Inline marks are dropped.
pub(crate) fn md_lines(text: &str) -> Vec<(String, Style)> {
    let mut out = Vec::new();
    let mut code = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            code = !code;
            continue;
        }
        if code {
            out.push((l.to_string(), Style::Mono));
            continue;
        }
        let clean = |s: &str| s.replace("**", "").replace("__", "").replace('`', "");
        let h3 = t.strip_prefix("### ").or_else(|| t.strip_prefix("#### ")).or_else(|| t.strip_prefix("##### "));
        let item = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ "));
        let (text, style) = if let Some(h) = h3 {
            (clean(h), Style::H3)
        } else if let Some(h) = t.strip_prefix("## ") {
            (clean(h), Style::H2)
        } else if let Some(h) = t.strip_prefix("# ") {
            (clean(h), Style::H1)
        } else if let Some(b) = item {
            let indent = "    ".repeat((l.len() - t.len()) / 2);
            (format!("{indent}• {}", clean(b)), Style::Bullet)
        } else if let Some(q) = t.strip_prefix("> ").or_else(|| t.strip_prefix('>')) {
            (clean(q), Style::Quote)
        } else if t.len() >= 3 && t.chars().all(|c| c == '-') {
            ("────────".into(), Style::Quote)
        } else {
            (clean(l), Style::Plain)
        };
        out.push((text, style));
    }
    out
}

unsafe fn fonts(dpi: i32, base: HFONT) -> [HFONT; 4] {
    if let Some((d, f)) = view(|v| v.fonts) {
        if d == dpi {
            return f;
        }
        for x in f {
            _ = DeleteObject(x.into());
        }
    }
    let mut lf = LOGFONTW::default();
    GetObjectW(base.into(), size_of::<LOGFONTW>() as i32, Some(&mut lf as *mut _ as _));
    let mono = CreateFontIndirectW(&LOGFONTW { lfFaceName: logfont("Consolas", 0).lfFaceName, ..lf });
    let bold = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, ..lf });
    let h1 = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, lfHeight: lf.lfHeight * 16 / 10, ..lf });
    let h2 = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, lfHeight: lf.lfHeight * 13 / 10, ..lf });
    let f = [mono, bold, h1, h2];
    view(|v| v.fonts = Some((dpi, f)));
    f
}

fn style_font(st: Style, font: HFONT, f: &[HFONT; 4]) -> HFONT {
    match st {
        Style::Mono => f[0],
        Style::H1 => f[2],
        Style::H2 => f[3],
        Style::H3 => f[1],
        _ => font,
    }
}

// Heights of every line at this width, measured once per item and width; painting then draws only what shows.
unsafe fn layout(dc: HDC, gen: usize, width: i32, text: &str, kind: TextKind, font: HFONT, f: &[HFONT; 4], dpi: i32) -> i32 {
    if let Some(total) = view(|v| v.layout.as_ref().filter(|(g, w, ..)| *g == gen && *w == width).map(|l| l.3)) {
        return total;
    }
    let src: Vec<(String, Style)> = match kind {
        TextKind::Markdown => md_lines(text),
        TextKind::Code => text.lines().map(|l| (l.to_string(), Style::Mono)).collect(),
        TextKind::Plain => text.lines().map(|l| (l.to_string(), Style::Plain)).collect(),
        TextKind::Document => text.lines().map(|l| (l.to_string(), Style::Plain)).collect(),
    };
    let mut y = 0;
    let mut lines = Vec::with_capacity(src.len().min(40_000));
    let mut source = src.into_iter().take(40_000).enumerate().peekable();
    while let Some((no, (t, st))) = source.next() {
        if kind == TextKind::Document && t.starts_with("| ") {
            let mut rows = Vec::new();
            let parse = |line: &str| -> Vec<String> {
                line.strip_prefix("| ").unwrap_or(line).strip_suffix('|').unwrap_or(line).split(" | ").map(|cell| cell.trim().to_string()).collect()
            };
            rows.push(parse(&t));
            while source.peek().is_some_and(|(_, (line, _))| line.starts_with("| ")) {
                if let Some((_, (line, _))) = source.next() {
                    rows.push(parse(&line));
                }
            }
            let mut widths = Vec::<i32>::new();
            let scale = |v: i32| v * dpi / 96;
            for row in &rows {
                for (col, cell) in row.iter().enumerate() {
                    if widths.len() <= col {
                        widths.push(scale(36));
                    }
                    widths[col] = widths[col].max((text_w(dc, cell, font) + scale(14)).min(scale(260)));
                }
            }
            let rh = app(|a| a.fh) + scale(8);
            let h = rh * rows.len() as i32;
            lines.push(Line { text: String::new(), style: st, y, h, no: no + 1, table: Some((rows, widths)) });
            y += h;
            continue;
        }
        let old = SelectObject(dc, style_font(st, font, f).into());
        let mut tm = TEXTMETRICW::default();
        _ = GetTextMetricsW(dc, &mut tm);
        let lh = tm.tmHeight + tm.tmExternalLeading;
        let h = if st == Style::Mono || t.is_empty() {
            lh
        } else {
            let mut w: Vec<u16> = t.encode_utf16().collect();
            let mut r = RECT { left: 0, top: 0, right: width, bottom: 0 };
            DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_WORDBREAK | DT_EXPANDTABS | DT_NOPREFIX | DT_EDITCONTROL);
            (r.bottom - r.top).max(lh)
        };
        SelectObject(dc, old);
        y += match st {
            Style::H1 | Style::H2 => lh / 2,
            Style::H3 => lh / 3,
            _ => 0,
        };
        lines.push(Line { text: t, style: st, y, h, no: no + 1, table: None });
        y += h + if matches!(st, Style::H1 | Style::H2) { dpi / 24 } else { 0 };
    }
    view(|v| v.layout = Some((gen, width, lines, y)));
    y
}

// ---------- scrollbar ----------
// Pure geometry: thumb size, thumb position, and clamping. No window, so the tests can pin it.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BarHit {
    None,
    Thumb,
    Track,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BarLayout {
    pub max_y: i32,
    pub thumb_h: i32,
    pub thumb_top: i32,
}

#[derive(Clone, Copy)]
pub(crate) struct BarRects {
    paint: RECT, // the drawn track
    hit: RECT,   // wider grab target, ending where › starts
}

/// `y` clamped into `0..=max_y`. A negative range is empty.
pub(crate) fn clamp_y(y: i32, max_y: i32) -> i32 {
    y.clamp(0, max_y.max(0))
}

/// Move `y` by `dy`. Once the content is taller than the view (`max_y > 0`) the result stays
/// inside it. Before the first layout `max_y` is 0, and a large `dy` is left for the paint to clamp.
pub(crate) fn nudge(y: i32, max_y: i32, dy: i32) -> i32 {
    let y = y.saturating_add(dy).max(0);
    if max_y > 0 {
        y.min(max_y)
    } else {
        y
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArrowAct {
    Scroll(i32),
    Step,
    Stay, // a key repeat ran into the end: don't walk the file list
}

/// Up / Down while the page is taller than the window. `dy` is positive.
pub(crate) fn arrow_act(y: i32, max_y: i32, down: bool, repeat: bool, dy: i32) -> ArrowAct {
    if max_y > 0 {
        let next = nudge(y, max_y, if down { dy } else { -dy });
        if next != y {
            return ArrowAct::Scroll(next);
        }
        if repeat {
            return ArrowAct::Stay;
        }
    }
    ArrowAct::Step
}

/// Thumb length along a track of `span` px. Short content gets a long thumb; a huge document
/// keeps at least `min_thumb`, and never more than the track.
pub(crate) fn thumb_len(content: i32, view: i32, span: i32, min_thumb: i32) -> i32 {
    if span <= 0 {
        return 0;
    }
    let min_thumb = min_thumb.max(1).min(span);
    if content <= view || view <= 0 || content <= 0 {
        return span;
    }
    let raw = (span as i64 * view as i64 / content as i64) as i32;
    raw.clamp(min_thumb, span)
}

/// Thumb top, relative to the track, for scroll position `y`.
pub(crate) fn thumb_offset(y: i32, max_y: i32, travel: i32) -> i32 {
    if max_y <= 0 || travel <= 0 {
        0
    } else {
        let y = y.clamp(0, max_y);
        ((y as i64 * travel as i64) / max_y as i64) as i32
    }
}

/// Scroll position for a thumb whose top is `rel` px into a travel of `travel`.
pub(crate) fn thumb_scroll(rel: i32, travel: i32, max_y: i32) -> i32 {
    if max_y <= 0 || travel <= 0 {
        0
    } else {
        let rel = rel.clamp(0, travel) as i64;
        ((rel * max_y as i64) / travel as i64).clamp(0, max_y as i64) as i32
    }
}

/// `None` when everything fits. Otherwise the thumb sits inside the track for `y`.
pub(crate) fn bar_layout(content: i32, view: i32, track_top: i32, track_h: i32, min_thumb: i32, y: i32) -> Option<BarLayout> {
    if view <= 0 || track_h <= 0 || content <= view {
        return None;
    }
    let max_y = content - view;
    let thumb_h = thumb_len(content, view, track_h, min_thumb);
    let travel = track_h - thumb_h;
    let thumb_top = track_top + thumb_offset(y, max_y, travel);
    Some(BarLayout { max_y, thumb_h, thumb_top })
}

/// One page toward the click. A click on the thumb leaves `y` alone.
pub(crate) fn page_toward(y: i32, max_y: i32, thumb_top: i32, thumb_h: i32, click_y: i32, page: i32) -> i32 {
    let dir: i32 = if click_y < thumb_top {
        -1
    } else if click_y >= thumb_top.saturating_add(thumb_h) {
        1
    } else {
        0
    };
    nudge(y, max_y, dir.saturating_mul(page.max(0)))
}

fn thumb_min(dpi: i32) -> i32 {
    (24 * dpi / 96).max(8)
}

/// Track in the gutter peek_geo leaves between the stage and ›. The hit rect is wider
/// (a 6 DIP bar is hard to grab) and stops on ›'s left edge.
pub(crate) fn peek_bar_rects(stage: RECT, dpi: i32) -> BarRects {
    let s = |v: i32| v * dpi / 96;
    let gutter = (s(56) - s(32)) / 2;
    let w = s(6).max(4).min((gutter - s(2)).max(1));
    let limit = stage.right + gutter;
    let mut right = stage.right + (gutter - w).max(0) / 2 + w;
    if right > limit {
        right = limit;
    }
    let left = (right - w).max(stage.right);
    let paint = RECT { left, top: stage.top + s(4), right, bottom: stage.bottom - s(4) };
    let hit = RECT { left: paint.left - s(8), top: paint.top, right: limit, bottom: paint.bottom };
    BarRects { paint, hit }
}

fn hit_kind(hit: RECT, thumb: RECT, x: i32, y: i32) -> BarHit {
    if inside(thumb, x, y) {
        BarHit::Thumb
    } else if inside(hit, x, y) {
        BarHit::Track
    } else {
        BarHit::None
    }
}

/// Where the pointer is on the vertical bar. The thumb wins over the track, across the wider hit width.
pub(crate) fn bar_hit(hit: RECT, thumb: RECT, x: i32, y: i32) -> BarHit {
    let thumb = RECT { left: hit.left, right: hit.right, ..thumb };
    hit_kind(hit, thumb, x, y)
}

/// Where the pointer is on the horizontal bar. The thumb wins, across the taller hit height.
pub(crate) fn hbar_hit(hit: RECT, thumb: RECT, x: i32, y: i32) -> BarHit {
    let thumb = RECT { top: hit.top, bottom: hit.bottom, ..thumb };
    hit_kind(hit, thumb, x, y)
}

/// Clear square, in px, kept where the two tracks would meet.
pub(crate) fn bar_corner(dpi: i32) -> i32 {
    let s = |v: i32| v * dpi / 96;
    s(6).max(4) + s(4)
}

/// Horizontal track along the bottom of the stage. `corner` keeps the right end clear when the
/// vertical bar is showing, so the two tracks do not meet.
pub(crate) fn peek_hbar_rects(stage: RECT, dpi: i32, corner: i32) -> BarRects {
    let s = |v: i32| v * dpi / 96;
    let inset = s(4);
    let thick = s(6).max(4);
    let left = stage.left + inset;
    let right = (stage.right - inset - corner.max(0)).max(left);
    let bottom = stage.bottom - inset;
    let top = bottom - thick;
    let paint = RECT { left, top, right, bottom };
    let hit = RECT { left: paint.left, top: (paint.top - s(8)).max(stage.top), right: paint.right, bottom: paint.bottom };
    BarRects { paint, hit }
}

fn reserve_corner(mut rects: BarRects, corner: i32) -> BarRects {
    if corner > 0 {
        rects.paint.bottom = (rects.paint.bottom - corner).max(rects.paint.top);
        rects.hit.bottom = rects.paint.bottom;
    }
    rects
}

/// Vertical track, and the horizontal one when `horiz` is set. A corner is reserved only when
/// both show; a single bar keeps the full edge.
pub(crate) fn peek_scroll_rects(stage: RECT, dpi: i32, vert: bool, horiz: bool) -> (Option<BarRects>, Option<BarRects>) {
    let corner = if vert && horiz { bar_corner(dpi) } else { 0 };
    let v = vert.then(|| reserve_corner(peek_bar_rects(stage, dpi), corner));
    let h = horiz.then(|| peek_hbar_rects(stage, dpi, corner));
    (v, h)
}

/// Track and thumb colours. The track is a theme mix; the thumb carries the accent, nudged
/// toward the theme text so a dim swatch still reads on both ivory and charcoal.
pub(crate) fn bar_rgb(bg: u32, text: u32, accent: u32, hot: bool) -> (u32, u32) {
    let track = blend(bg, text, 0.16);
    let thumb = if hot { blend(accent, on_color(accent), 0.25) } else { blend(accent, text, 0.12) };
    (track, thumb)
}

// A drag or a track click moved `y`. Restart the wheel's "rested at the end" wait when we
// arrive there, so the fling doesn't also step to the next file.
fn note_scroll(v: &mut PeekView, prev: i32) {
    let moved = v.y != prev;
    let at_edge = v.max_y > 0 && (v.y <= 0 || v.y >= v.max_y);
    if !at_edge {
        v.edge_at = None;
    } else if moved {
        v.edge_at = Some(Instant::now());
    }
    v.wheel = 0;
}

// ---------- painting ----------

fn clamp_scroll(total: i32, page: i32) {
    view(|v| {
        let page = page.max(0);
        v.page_h = page;
        v.max_y = total.saturating_sub(page).max(0);
        v.y = clamp_y(v.y, v.max_y);
        // Text, tables and lists keep their own sideways scroll and do not grow this bar.
        (v.max_x, v.page_w) = (0, 0);
    });
}

#[derive(Debug, PartialEq)]
pub(crate) enum Wheel {
    Scroll,
    Step,
    Hold, // at the end, but only just: a fling to the bottom doesn't also jump to the next file
}

// A wheel turn scrolls while there is room that way. Past the end, when `nav` is on, it moves
// to the previous / next file: at once when nothing scrolls, else once the view has rested
// at that end for a moment. Off, it never steps — a fitted picture's wheel does nothing.
pub(crate) fn wheel_action(y: i32, max_y: i32, down: bool, rested: bool, nav: bool) -> Wheel {
    let room = if down { y < max_y } else { y > 0 };
    match () {
        _ if room => Wheel::Scroll,
        _ if nav && (max_y == 0 || rested) => Wheel::Step,
        _ => Wheel::Hold,
    }
}

unsafe fn wheel_peek(h: HWND, raw: i32) {
    let down = raw < 0;
    let (px, nav) = app(|a| (-raw * a.fh * 3 / 120, a.s.peek_wheel_nav));
    let act = view(|v| {
        let rested = v.edge_at.is_some_and(|t| t.elapsed() >= Duration::from_millis(350));
        let act = wheel_action(v.y, v.max_y, down, rested, nav);
        match act {
            Wheel::Scroll => {
                v.y = (v.y + px).clamp(0, v.max_y);
                let end = if down { v.y >= v.max_y } else { v.y <= 0 };
                (v.edge_at, v.wheel) = (end.then(Instant::now), 0);
            }
            Wheel::Hold => {
                v.edge_at.get_or_insert_with(Instant::now);
            }
            Wheel::Step => {}
        }
        act
    });
    match act {
        Wheel::Scroll => _ = InvalidateRect(Some(h), None, false),
        Wheel::Step => step_by_wheel(raw),
        Wheel::Hold => {}
    }
}

// One file per notch; a touchpad's small deltas add up to one.
unsafe fn step_by_wheel(raw: i32) {
    let go = view(|v| {
        if (v.wheel < 0) != (raw < 0) {
            v.wheel = 0;
        }
        v.wheel += raw;
        let go = v.wheel.abs() >= 120;
        if go {
            v.wheel = 0;
        }
        go
    });
    if go {
        step_peek(raw > 0);
    }
}

// ---------- zoom (images and PDF pages) ----------
// Stored as a multiple of the fit size: 1.0 draws what the preview drew before zoom existed
// (an image letterboxed in the stage, never enlarged; a PDF page fitted to the stage width,
// stacked from the top). The header shows that multiple — "150%" is 1.5× fit, and the wheel
// range is 10%..800%. It is not a percent of the file's pixels: the bitmap is already a fitted
// render, and a PDF re-render would make a bitmap-pixel percent jump.

pub(crate) const ZOOM_MIN: f64 = 0.10;
pub(crate) const ZOOM_MAX: f64 = 8.0;
/// One notch (wheel delta 120).
pub(crate) const ZOOM_STEP: f64 = 1.25;
/// Longest side of a zoomed PDF render, in px.
pub(crate) const ZOOM_RENDER_MAX: i32 = 8192;
const ZOOM_WAIT_MS: u32 = 150;

fn zoom_or_fit(z: f64) -> f64 {
    if z == 0.0 { 1.0 } else { z.clamp(ZOOM_MIN, ZOOM_MAX) }
}

fn is_fit(z: f64) -> bool {
    (zoom_or_fit(z) - 1.0).abs() < 1e-9
}

/// `delta` is the wheel delta, positive zooming in. 120 is one notch.
pub(crate) fn zoom_factor(z: f64, delta: i32) -> f64 {
    let z = zoom_or_fit(z);
    if delta == 0 {
        return z;
    }
    (z * ZOOM_STEP.powf(delta as f64 / 120.0)).clamp(ZOOM_MIN, ZOOM_MAX)
}

/// Header percent. 100 is fit.
pub(crate) fn zoom_pct(z: f64) -> i32 {
    (zoom_or_fit(z) * 100.0).round() as i32
}

/// `facts` plus the zoom percent. Empty facts are just the percent.
pub(crate) fn zoom_label(facts: &str, pct: i32) -> String {
    if facts.is_empty() { format!("{pct}%") } else { format!("{facts}  {pct}%") }
}

/// Where a content edge sits when that axis is smaller than the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FitAlign {
    Center,
    Start,
}

fn scaled(n: i32, zoom: f64) -> i32 {
    if n <= 0 {
        return 0;
    }
    let z = zoom_or_fit(zoom);
    if (z - 1.0).abs() < 1e-12 {
        return n;
    }
    ((n as f64) * z).round().max(1.0) as i32
}

/// Fit an image into the stage the way paint used to: letterboxed, never enlarged.
pub(crate) fn image_fit_px(nat_w: i32, nat_h: i32, stage_w: i32, stage_h: i32) -> (i32, i32) {
    if nat_w <= 0 || nat_h <= 0 || stage_w <= 0 || stage_h <= 0 {
        return (0, 0);
    }
    let k = (stage_w as f32 / nat_w as f32).min(stage_h as f32 / nat_h as f32).min(1.0);
    ((nat_w as f32 * k) as i32, (nat_h as f32 * k) as i32)
}

/// Fit one PDF page to the stage width, but never wider than its fit-render. Zoom 1 matches paint_pages.
pub(crate) fn page_fit_size(nat_w: i32, nat_h: i32, stage_w: i32) -> (i32, i32) {
    if nat_w <= 0 || nat_h <= 0 || stage_w <= 0 {
        return (0, 0);
    }
    let w = nat_w.min(stage_w);
    let h = ((nat_h as i64 * w as i64) / nat_w as i64).max(1) as i32;
    (w, h)
}

fn page_draw(nat_w: i32, nat_h: i32, stage_w: i32, zoom: f64) -> (i32, i32) {
    let (fw, fh) = page_fit_size(nat_w, nat_h, stage_w);
    (scaled(fw, zoom), scaled(fh, zoom))
}

/// Render size for one page at this zoom, capped so a deep zoom cannot allocate a huge bitmap.
pub(crate) fn cap_render(w: i32, h: i32) -> (i32, i32) {
    let w = w.max(1);
    let h = h.max(1);
    let m = w.max(h);
    if m <= ZOOM_RENDER_MAX {
        return (w, h);
    }
    let k = ZOOM_RENDER_MAX as f64 / m as f64;
    ((w as f64 * k).round().max(1.0) as i32, (h as f64 * k).round().max(1.0) as i32)
}

pub(crate) fn zoom_render_px(lay_w: i32, lay_h: i32, stage_w: i32, zoom: f64) -> (i32, i32) {
    let (fw, fh) = page_fit_size(lay_w, lay_h, stage_w.max(1));
    cap_render(scaled(fw, zoom).max(16), scaled(fh, zoom).max(16))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PdfBox {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Pages stacked in content coordinates at this zoom. `gap` and `footer` are the fit-size px (zoom 1).
pub(crate) fn pdf_layout(pages: &[(i32, i32)], stage_w: i32, gap: i32, footer: i32, zoom: f64) -> (i32, i32, Vec<PdfBox>) {
    let gap_z = scaled(gap, zoom);
    let foot = scaled(footer, zoom);
    let mut boxes = Vec::with_capacity(pages.len());
    let mut max_w = 0i32;
    let mut y = 0i32;
    for (i, &(nw, nh)) in pages.iter().enumerate() {
        let (w, h) = page_draw(nw, nh, stage_w, zoom);
        max_w = max_w.max(w);
        boxes.push(PdfBox { x: 0, y, w, h });
        y = y.saturating_add(h);
        if i + 1 != pages.len() {
            y = y.saturating_add(gap_z);
        }
    }
    y = y.saturating_add(foot);
    for b in &mut boxes {
        b.x = (max_w - b.w) / 2;
    }
    (max_w, y.max(0), boxes)
}

fn origin_axis(scroll: i32, content: i32, view: i32, align: FitAlign) -> i32 {
    if content <= view {
        match align {
            FitAlign::Center => (view - content) / 2,
            FitAlign::Start => 0,
        }
    } else {
        -scroll.clamp(0, content - view)
    }
}

fn clamp_origin(origin: f64, content: i32, view: i32, align: FitAlign) -> f64 {
    if content <= view {
        match align {
            FitAlign::Center => (view - content) as f64 / 2.0,
            FitAlign::Start => 0.0,
        }
    } else {
        origin.clamp((view - content) as f64, 0.0)
    }
}

fn origin_to_scroll(ox: f64, oy: f64, content: (i32, i32), view: (i32, i32), align: (FitAlign, FitAlign)) -> (i32, i32) {
    let ox = clamp_origin(ox, content.0, view.0, align.0);
    let oy = clamp_origin(oy, content.1, view.1, align.1);
    let sx = if content.0 <= view.0 { 0 } else { (-ox).round().clamp(0.0, (content.0 - view.0) as f64) as i32 };
    let sy = if content.1 <= view.1 { 0 } else { (-oy).round().clamp(0.0, (content.1 - view.1) as f64) as i32 };
    (sx, sy)
}

/// Scroll after scaling about `cursor` (view px) by `factor` (`new_zoom / old_zoom`).
/// Sizes are the content before and after. An axis smaller than the view goes to `align`.
pub(crate) fn zoom_scroll(
    scroll: (i32, i32),
    cursor: (i32, i32),
    old_size: (i32, i32),
    new_size: (i32, i32),
    view: (i32, i32),
    factor: f64,
    align: (FitAlign, FitAlign),
) -> (i32, i32) {
    let ox = origin_axis(scroll.0, old_size.0, view.0, align.0) as f64;
    let oy = origin_axis(scroll.1, old_size.1, view.1, align.1) as f64;
    let nx = cursor.0 as f64 - (cursor.0 as f64 - ox) * factor;
    let ny = cursor.1 as f64 - (cursor.1 as f64 - oy) * factor;
    origin_to_scroll(nx, ny, new_size, view, align)
}

/// Drag by `delta` px; the content follows the pointer. Clamped so it cannot leave a needless gap.
pub(crate) fn pan_scroll(scroll: (i32, i32), delta: (i32, i32), content: (i32, i32), view: (i32, i32), align: (FitAlign, FitAlign)) -> (i32, i32) {
    let ox = origin_axis(scroll.0, content.0, view.0, align.0) as f64 + delta.0 as f64;
    let oy = origin_axis(scroll.1, content.1, view.1, align.1) as f64 + delta.1 as f64;
    origin_to_scroll(ox, oy, content, view, align)
}

enum ZoomShape {
    Image { nat_w: i32, nat_h: i32 },
    Pages { nat: Vec<(i32, i32)>, more: bool },
}

fn shape_align(shape: &ZoomShape) -> (FitAlign, FitAlign) {
    match shape {
        // A short PDF stays top-aligned, as it does at fit. Horizontally it centers.
        ZoomShape::Pages { .. } => (FitAlign::Center, FitAlign::Start),
        ZoomShape::Image { .. } => (FitAlign::Center, FitAlign::Center),
    }
}

fn shape_gap(shape: &ZoomShape, dpi: i32) -> (i32, i32) {
    let s = |v: i32| v * dpi / 96;
    match shape {
        ZoomShape::Pages { more, .. } => (s(16), if *more { s(48) } else { 0 }),
        ZoomShape::Image { .. } => (0, 0),
    }
}

fn zoom_shape() -> Option<ZoomShape> {
    let j = peek_job();
    match j.done.as_ref().map(|p| &p.body) {
        Some(Body::Pic(p)) => {
            let (nat_w, nat_h) = p.lay();
            Some(ZoomShape::Image { nat_w, nat_h })
        }
        Some(Body::Pages { pages, total }) => Some(ZoomShape::Pages {
            nat: pages.iter().map(|p| p.lay()).collect(),
            more: (pages.len() as u32) < *total,
        }),
        _ => None,
    }
}

fn content_extent(shape: &ZoomShape, stage: (i32, i32), dpi: i32, zoom: f64) -> (i32, i32) {
    let (gap, footer) = shape_gap(shape, dpi);
    match shape {
        ZoomShape::Image { nat_w, nat_h } => {
            let (fw, fh) = image_fit_px(*nat_w, *nat_h, stage.0, stage.1);
            (scaled(fw, zoom), scaled(fh, zoom))
        }
        ZoomShape::Pages { nat, .. } => {
            let (w, h, _) = pdf_layout(nat, stage.0, gap, footer, zoom);
            (w, h)
        }
    }
}

fn stage_of(h: HWND) -> (RECT, i32, i32) {
    let st = unsafe { geo_of(h).stage };
    let (w, ht) = ((st.right - st.left).max(0), (st.bottom - st.top).max(0));
    (st, w, ht)
}

unsafe fn set_zoom(h: HWND, new_z: f64, at: Option<(i32, i32)>) {
    let Some(shape) = zoom_shape() else { return };
    let new_z = zoom_or_fit(new_z);
    let old_z = view(|v| zoom_or_fit(v.zoom));
    if (old_z - new_z).abs() < 1e-12 {
        if is_fit(new_z) {
            schedule_pdf_zoom(h);
        }
        return;
    }
    let dpi = app(|a| a.dpi);
    let (_, vw, vh) = stage_of(h);
    if vw < 2 || vh < 2 {
        return;
    }
    let align = shape_align(&shape);
    let old = content_extent(&shape, (vw, vh), dpi, old_z);
    let new = content_extent(&shape, (vw, vh), dpi, new_z);
    let (mx, my) = at.unwrap_or((vw / 2, vh / 2));
    let factor = new_z / old_z;
    view(|v| {
        let (sx, sy) = zoom_scroll((v.x, v.y), (mx.clamp(0, vw), my.clamp(0, vh)), old, new, (vw, vh), factor, align);
        v.zoom = new_z;
        v.x = sx;
        v.y = sy;
        v.page_h = vh;
        v.page_w = vw;
        v.max_y = new.1.saturating_sub(vh).max(0);
        v.max_x = new.0.saturating_sub(vw).max(0);
        v.edge_at = None;
        v.wheel = 0;
    });
    _ = InvalidateRect(Some(h), None, false);
    schedule_pdf_zoom(h);
}

unsafe fn schedule_pdf_zoom(h: HWND) {
    let pdf = matches!(zoom_shape(), Some(ZoomShape::Pages { .. }));
    if !pdf || is_fit(view(|v| v.zoom)) {
        _ = KillTimer(Some(h), TIMER_PEEK_ZOOM);
        peek_job().zoom = None;
        return;
    }
    _ = SetTimer(Some(h), TIMER_PEEK_ZOOM, ZOOM_WAIT_MS, None);
}

unsafe fn request_pdf_zoom(h: HWND) {
    let z = view(|v| zoom_or_fit(v.zoom));
    if is_fit(z) {
        peek_job().zoom = None;
        return;
    }
    let (_, stage_w, _) = stage_of(h);
    let stage_w = stage_w.max(1);
    let mut j = peek_job();
    let (incomplete, soft, gen) = match j.done.as_ref().map(|p| &p.body) {
        Some(Body::Pages { pages, total }) => {
            let incomplete = (pages.len() as u32) < *total;
            let soft = pages.iter().any(|p| {
                let (lw, lh) = p.lay();
                let (rw, rh) = zoom_render_px(lw, lh, stage_w, z);
                p.w + rw / 32 < rw || p.h + rh / 32 < rh
            });
            (incomplete, soft, j.gen)
        }
        _ => {
            j.zoom = None;
            return;
        }
    };
    if !incomplete && !soft {
        j.zoom = None;
        return;
    }
    let seq = view(|v| {
        v.zoom_seq = v.zoom_seq.wrapping_add(1);
        v.zoom_seq
    });
    j.zoom = Some(ZoomReq { gen, seq, stage_w, zoom: z });
    drop(j);
    PEEK_WAKE.notify_one();
}

unsafe fn pan_by(h: HWND, dx: i32, dy: i32) {
    let Some(shape) = zoom_shape() else { return };
    let dpi = app(|a| a.dpi);
    let (_, vw, vh) = stage_of(h);
    let zoom = view(|v| zoom_or_fit(v.zoom));
    let content = content_extent(&shape, (vw, vh), dpi, zoom);
    let align = shape_align(&shape);
    let moved = view(|v| {
        let prev = v.y;
        let next = pan_scroll((v.x, v.y), (dx, dy), content, (vw, vh), align);
        let moved = next != (v.x, v.y);
        (v.x, v.y) = next;
        if moved {
            note_scroll(v, prev);
        }
        moved
    });
    if moved {
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn pan_ready(h: HWND) -> bool {
    if is_fit(view(|v| v.zoom)) {
        return false;
    }
    let Some(shape) = zoom_shape() else { return false };
    let dpi = app(|a| a.dpi);
    let (_, vw, vh) = stage_of(h);
    let (cw, ch) = content_extent(&shape, (vw, vh), dpi, view(|v| zoom_or_fit(v.zoom)));
    cw > vw || ch > vh
}

unsafe fn peek_cursor(drag: bool) -> HCURSOR {
    let id = if drag { IDC_SIZEALL } else { IDC_HAND };
    LoadCursorW(None, id).or_else(|_| LoadCursorW(None, IDC_ARROW)).unwrap_or_default()
}

fn clamp_plane(cw: i32, ch: i32, vw: i32, vh: i32, align: (FitAlign, FitAlign)) {
    view(|v| {
        let (sx, sy) = pan_scroll((v.x, v.y), (0, 0), (cw, ch), (vw, vh), align);
        v.x = sx;
        v.y = sy;
        v.page_h = vh.max(0);
        v.page_w = vw.max(0);
        v.max_y = ch.saturating_sub(vh.max(0)).max(0);
        v.max_x = cw.saturating_sub(vw.max(0)).max(0);
    });
}

// Opaque bitmaps scale with HALFTONE. A transparent PNG keeps AlphaBlend so the fringe stays clean.
fn opaque_enough(px: &[u8]) -> bool {
    let n = px.len() / 4;
    if n == 0 {
        return true;
    }
    let step = (n / 128).max(1);
    px.chunks(4).step_by(step).take(128).all(|p| p.len() == 4 && p[3] == 255)
}

unsafe fn blit_scaled(dc: HDC, to: RECT, w: i32, h: i32, px: &[u8]) {
    let (dw, dh) = (to.right - to.left, to.bottom - to.top);
    if dw <= 0 || dh <= 0 || w <= 0 || h <= 0 || px.len() < (w as usize) * (h as usize) * 4 {
        return;
    }
    if dw == w && dh == h || !opaque_enough(px) {
        blit_bgra(dc, to, w, h, px);
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
    let prev = SetStretchBltMode(dc, HALFTONE);
    _ = SetBrushOrgEx(dc, 0, 0, None);
    _ = StretchBlt(dc, to.left, to.top, dw, dh, Some(mem), 0, 0, w, h, SRCCOPY);
    _ = SetStretchBltMode(dc, STRETCH_BLT_MODE(prev));
    SelectObject(mem, old);
    _ = DeleteDC(mem);
    _ = DeleteObject(bmp.into());
}

// DrawTextW without text()'s single-line, end-ellipsis defaults.
pub(crate) unsafe fn text_flags(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, flags: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return; // see text()
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(rgb));
    DrawTextW(dc, &mut w, &mut r, flags | DT_NOPREFIX);
    SelectObject(dc, old);
}

unsafe fn text_w(dc: HDC, s: &str, font: HFONT) -> i32 {
    let w: Vec<u16> = s.chars().take(200).collect::<String>().encode_utf16().collect();
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    SelectObject(dc, old);
    sz.cx
}

unsafe fn clip(dc: HDC, r: RECT, draw: impl FnOnce()) {
    let rgn = CreateRectRgn(r.left, r.top, r.right, r.bottom);
    SelectClipRgn(dc, Some(rgn));
    draw();
    SelectClipRgn(dc, None);
    _ = DeleteObject(rgn.into());
}

unsafe fn paint_text(dc: HDC, area: RECT, gen: usize, text: &str, kind: TextKind, cut: bool, font: HFONT, dpi: i32, pic: Option<&Pix>) {
    let s = |v: i32| v * dpi / 96;
    let f = fonts(dpi, font);
    let gutter = if kind == TextKind::Code { s(44) } else { 0 };
    let width = area.right - area.left - gutter - s(8);
    let pic_size = pic.map(|p| {
        let k = ((area.right - area.left) as f32 / p.w as f32).min((area.bottom - area.top) as f32 / p.h as f32);
        ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32)
    });
    let offset = pic_size.map_or(0, |(_, h)| h + s(20));
    let total = offset + layout(dc, gen, width, text, kind, font, &f, dpi) + if cut { s(28) } else { 0 };
    clamp_scroll(total, area.bottom - area.top);
    if kind == TextKind::Document {
        view(|v| {
            let full = v
                .layout
                .as_ref()
                .map(|(_, _, lines, _)| lines.iter().filter_map(|l| l.table.as_ref()).map(|(_, widths)| widths.iter().sum::<i32>()).max().unwrap_or(0))
                .unwrap_or(0);
            v.x = v.x.clamp(0, (full - (area.right - area.left)).max(0));
        });
    }
    let (sy, sx) = view(|v| (v.y, v.x));
    clip(dc, area, || {
        if let (Some(p), Some((w, h))) = (pic, pic_size) {
            let x = (area.left + area.right - w) / 2;
            let y = area.top - sy;
            blit_bgra(dc, RECT { left: x, top: y, right: x + w, bottom: y + h }, p.w, p.h, &p.bgra);
        }
        let sy = sy - offset;
        view(|v| {
            let Some((_, _, lines, _)) = v.layout.as_ref() else { return };
            let first = lines.partition_point(|l| l.y + l.h < sy);
            for l in lines[first..].iter().take_while(|l| l.y - sy < area.bottom - area.top) {
                let top = area.top + l.y - sy;
                if let Some((rows, widths)) = &l.table {
                    let rh = app(|a| a.fh) + s(8);
                    let first = ((area.top - top).max(0) / rh) as usize;
                    let shown = ((area.bottom - area.top) / rh + 2) as usize;
                    for (row_no, row) in rows.iter().enumerate().skip(first).take(shown) {
                        let row_top = top + row_no as i32 * rh;
                        let mut x = area.left - sx;
                        for (col, w) in widths.iter().enumerate() {
                            if x + w > area.left && x < area.right {
                                let cell = row.get(col).map(String::as_str).unwrap_or("");
                                let rect = RECT { left: x + s(7), top: row_top, right: x + w - s(7), bottom: row_top + rh };
                                crate::text(dc, rect, cell, font, pal().text, DT_LEFT);
                                fill(dc, RECT { left: x + w - 1, top: row_top, right: x + w, bottom: row_top + rh }, pal().line);
                                fill(dc, RECT { left: x, top: row_top + rh - 1, right: x + w, bottom: row_top + rh }, pal().line);
                            }
                            x += w;
                        }
                        fill(dc, RECT { left: area.left - sx, top: row_top, right: area.left - sx + 1, bottom: row_top + rh }, pal().line);
                    }
                    let table_width: i32 = widths.iter().sum();
                    fill(dc, RECT { left: area.left - sx, top, right: area.left - sx + table_width, bottom: top + 1 }, pal().line);
                    continue;
                }
                if gutter > 0 {
                    let g = RECT { left: area.left, top, right: area.left + gutter - s(10), bottom: top + l.h };
                    text_flags(dc, g, &l.no.to_string(), f[0], pal().sub, DT_RIGHT | DT_SINGLELINE);
                }
                let mono = l.style == Style::Mono;
                let (ink, fnt) = match l.style {
                    Style::Quote => (pal().sub, font),
                    st => (pal().text, style_font(st, font, &f)),
                };
                let left = area.left + gutter - if mono { sx } else { 0 };
                let r = RECT { left, top, right: if mono { left + 100_000 } else { area.right - s(8) }, bottom: top + l.h };
                let flags = if mono { DT_SINGLELINE | DT_EXPANDTABS } else { DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL };
                text_flags(dc, r, &l.text, fnt, ink, flags);
            }
        });
        if cut {
            let y = area.top + total - offset - s(24) - sy;
            let r = RECT { left: area.left, top: y, right: area.right, bottom: y + s(24) };
            text_flags(dc, r, tr(crate::Key::OnlyFirst512KbIsShown), font, pal().sub, DT_SINGLELINE);
        }
    });
}

// A grid: column widths from the first rows' text, the first row as a header, scrolled both ways.
unsafe fn paint_table(dc: HDC, area: RECT, rows: &[Vec<String>], cut: bool, font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let f = fonts(dpi, font);
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        text(dc, area, tr(crate::Key::Empty), font, pal().sub, DT_CENTER);
        return;
    }
    let mut widths = vec![s(36); cols];
    for (k, r) in rows.iter().take(200).enumerate() {
        for (c, cell) in r.iter().enumerate() {
            let w = text_w(dc, cell, if k == 0 { f[1] } else { font }) + s(14);
            widths[c] = widths[c].max(w.min(s(260)));
        }
    }
    let num = text_w(dc, &rows.len().to_string(), font) + s(14);
    let rh = app(|a| a.fh) + s(8);
    let total = rh * rows.len() as i32 + if cut { rh } else { 0 };
    clamp_scroll(total, area.bottom - area.top);
    let full: i32 = num + widths.iter().sum::<i32>();
    view(|v| v.x = v.x.min((full - (area.right - area.left)).max(0)).max(0));
    let (sy, sx) = view(|v| (v.y, v.x));
    clip(dc, area, || {
        let first = (sy / rh) as usize;
        let shown = ((area.bottom - area.top) / rh + 2) as usize;
        for (k, r) in rows.iter().enumerate().skip(first).take(shown) {
            let top = area.top + k as i32 * rh - sy;
            let row = RECT { left: area.left, top, right: area.right, bottom: top + rh };
            if k == 0 {
                fill(dc, row, pal().pill);
            }
            fill(dc, RECT { top: row.bottom - 1, ..row }, pal().line);
            text(dc, RECT { left: area.left, right: area.left + num - s(6), ..row }, &(k + 1).to_string(), font, pal().sub, DT_RIGHT);
            let mut x = area.left + num - sx;
            for (c, w) in widths.iter().enumerate() {
                if x + w > area.left + num && x < area.right {
                    let cell = r.get(c).map(String::as_str).unwrap_or("");
                    let cr = RECT { left: (x + s(7)).max(area.left + num), top, right: x + w - s(7), bottom: top + rh };
                    let right = k > 0 && !cell.is_empty() && cell.replace(',', "").parse::<f64>().is_ok();
                    text(dc, cr, cell, if k == 0 { f[1] } else { font }, pal().text, if right { DT_RIGHT } else { DT_LEFT });
                    fill(dc, RECT { left: x + w - 1, top, right: x + w, bottom: top + rh }, pal().line);
                }
                x += w;
            }
        }
        if cut {
            let top = area.top + rows.len() as i32 * rh - sy;
            let r = RECT { left: area.left + s(6), top, right: area.right, bottom: top + rh };
            text(dc, r, tr(crate::Key::MoreRowsNotShown), font, pal().sub, DT_LEFT);
        }
    });
}

unsafe fn paint_list(dc: HDC, area: RECT, rows: &[(String, String)], font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let rh = app(|a| a.fh) + s(8);
    let top0 = area.top;
    if rows.is_empty() {
        text(dc, area, tr(crate::Key::Empty), font, pal().sub, DT_CENTER);
        return;
    }
    let page = area.bottom - top0;
    clamp_scroll(rh * rows.len() as i32, page);
    let sy = view(|v| v.y);
    let sw = s(90);
    clip(dc, RECT { top: top0, ..area }, || {
        for (k, (name, size)) in rows.iter().enumerate().skip((sy / rh) as usize).take((page / rh + 2) as usize) {
            let top = top0 + k as i32 * rh - sy;
            let r = RECT { left: area.left, top, right: area.right, bottom: top + rh };
            if k % 2 == 1 {
                fill(dc, RECT { left: r.left - s(8), right: r.right + s(8), ..r }, blend(pal().card, pal().bg, 0.45));
            }
            text(dc, RECT { right: r.right - sw - s(8), ..r }, name, font, pal().text, DT_LEFT);
            text(dc, RECT { left: r.right - sw, ..r }, size, font, pal().sub, DT_RIGHT);
        }
    });
}

// Big icon (or thumbnail) on the left, the facts beside it.
unsafe fn paint_card(dc: HDC, area: RECT, icon: Option<&Pix>, rows: &[(String, String)], font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    // drawn at its own size (only shrunk if it doesn't fit): scaling an icon blurs and fringes it
    let side = icon.map_or(s(96), |p| p.w.max(p.h)).min((area.right - area.left) / 3).min(area.bottom - area.top);
    // icon and facts as one block, centred
    let lw = rows.iter().map(|(l, _)| text_w(dc, l, font)).max().unwrap_or(0) + s(16);
    let vw = rows.iter().map(|(_, v)| text_w(dc, v, font)).max().unwrap_or(0);
    let block = side + s(32) + lw + vw;
    let x0 = area.left + ((area.right - area.left - block) / 2).max(s(8));
    if let Some(p) = icon {
        let k = (side as f32 / p.w.max(p.h) as f32).min(1.0);
        let (dw, dh) = ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32);
        let (x, y) = (x0 + (side - dw) / 2, area.top + (area.bottom - area.top - dh) / 2);
        blit_bgra(dc, RECT { left: x, top: y, right: x + dw, bottom: y + dh }, p.w, p.h, &p.bgra);
    }
    let left = x0 + side + s(32);
    let rh = app(|a| a.fh) + s(8);
    let mut y = area.top + ((area.bottom - area.top) - rh * rows.len() as i32).max(0) / 2;
    for (label, value) in rows {
        text(dc, RECT { left, top: y, right: left + lw, bottom: y + rh }, label, font, pal().sub, DT_LEFT);
        let v = RECT { left: left + lw, top: y + s(4), right: area.right, bottom: y + rh * 3 };
        let mut w: Vec<u16> = value.encode_utf16().collect();
        let mut calc = v;
        let old = SelectObject(dc, font.into());
        DrawTextW(dc, &mut w, &mut calc, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL);
        SelectObject(dc, old);
        text_flags(dc, v, value, font, pal().text, DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
        y += (calc.bottom - calc.top).clamp(rh - s(8), rh * 3 - s(8)) + s(8);
    }
}

// PDF pages one under another, each its own sheet of paper on the stage.
// Zoom scales every page (and the gap) from the fit size. A short stack stays top-aligned.
unsafe fn paint_pages(dc: HDC, area: RECT, pages: &[Pix], total: u32, font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let zoom = view(|v| zoom_or_fit(v.zoom));
    let (vw, vh) = ((area.right - area.left).max(0), (area.bottom - area.top).max(0));
    let more = (pages.len() as u32) < total;
    let nat: Vec<(i32, i32)> = pages.iter().map(|p| p.lay()).collect();
    let (gap, footer) = (s(16), if more { s(48) } else { 0 });
    let (cw, ch, boxes) = pdf_layout(&nat, vw, gap, footer, zoom);
    let align = (FitAlign::Center, FitAlign::Start);
    clamp_plane(cw, ch, vw, vh, align);
    let (sx, sy) = view(|v| (v.x, v.y));
    let (ox, oy) = (origin_axis(sx, cw, vw, align.0), origin_axis(sy, ch, vh, align.1));
    clip(dc, area, || {
        for (p, b) in pages.iter().zip(boxes.iter()) {
            let r = RECT { left: area.left + ox + b.x, top: area.top + oy + b.y, right: area.left + ox + b.x + b.w, bottom: area.top + oy + b.y + b.h };
            if r.bottom > area.top && r.top < area.bottom && r.right > area.left && r.left < area.right {
                blit_scaled(dc, r, p.w, p.h, &p.bgra);
                frame(dc, RECT { left: r.left - 1, top: r.top - 1, right: r.right + 1, bottom: r.bottom + 1 }, 1, pal().line);
            }
        }
        if more {
            let top = area.top + oy + ch - scaled(footer, zoom);
            let r = RECT { left: area.left, top, right: area.right, bottom: top + s(32) };
            let t = format!("{} {} / {}", tr(crate::Key::RenderingPages), pages.len(), total);
            text(dc, r, &t, font, pal().sub, DT_CENTER);
        }
    });
}

pub(crate) unsafe fn paint_peek(h: HWND) {
    let (dpi, font, icon_font, name, ilist, accent) = app(|a| (a.dpi, a.font, a.icon_font, a.peek_name.clone(), a.ilist.clone(), a.s.accent_now()));
    let (hosted, hover, hot, maximized, zoom) = view(|v| (v.hosted, v.hover, v.hot, v.maximized, zoom_or_fit(v.zoom)));
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let g = peek_geo(rc, dpi);
        let f = fonts(dpi, font);
        fill(dc, rc, pal().bg);
        let j = peek_job();
        let gen = j.gen;
        // header: icon, name in bold, the facts after it in the secondary colour
        if let (Some(p), Some(il)) = (&j.done, &ilist) {
            if let Some(icon) = (p.icon >= 0).then(|| il.GetIcon(p.icon, ILD_NORMAL.0).ok()).flatten() {
                let r = g.icon;
                _ = DrawIconEx(dc, r.left, r.top, icon, r.right - r.left, r.bottom - r.top, 0, None, DI_NORMAL);
                _ = DestroyIcon(icon);
            }
        }
        let room = g.title.right - g.title.left;
        let name_w = text_w(dc, &name, f[1]).min(room * 6 / 10);
        text(dc, RECT { right: g.title.left + name_w, ..g.title }, &name, f[1], pal().text, DT_LEFT);
        if let Some(p) = &j.done {
            let joined = p.meta.join("   ·   ");
            let shown = match &p.body {
                Body::Pic(_) | Body::Pages { .. } => zoom_label(&joined, zoom_pct(zoom)),
                _ => joined,
            };
            let facts = RECT { left: g.title.left + name_w + s(16), ..g.title };
            text(dc, facts, &shown, font, pal().sub, DT_LEFT);
        }
        let glyph = if maximized { ICON_RESTORE } else { ICON_MAX };
        for (r, on, mark) in [(g.max, hot == Hot::Max, glyph), (g.close, hot == Hot::Close, ICON_CLOSE)] {
            if on {
                fill(dc, r, blend(pal().bg, pal().text, 0.08));
            }
            text(dc, r, mark, icon_font, if on { pal().text } else { pal().sub }, DT_CENTER);
        }
        // content: pictures and pages on the stage; everything else on a plate, as panes are
        let plate = || {
            fill(dc, g.stage, pal().card);
            frame(dc, g.stage, 1, pal().line);
        };
        match j.done.as_ref().map(|p| &p.body) {
            Some(Body::Pic(p)) => {
                // Fit letterboxes and never enlarges. Zoom is a multiple of that; at 1.0 the rect matches the old one.
                let st = g.stage;
                let (vw, vh) = ((st.right - st.left).max(0), (st.bottom - st.top).max(0));
                let (nw, nh) = p.lay();
                let (fw, fh) = image_fit_px(nw, nh, vw, vh);
                let (dw, dh) = (scaled(fw, zoom), scaled(fh, zoom));
                let align = (FitAlign::Center, FitAlign::Center);
                clamp_plane(dw, dh, vw, vh, align);
                let (sx, sy) = view(|v| (v.x, v.y));
                let (ox, oy) = (origin_axis(sx, dw, vw, align.0), origin_axis(sy, dh, vh, align.1));
                let r = RECT { left: st.left + ox, top: st.top + oy, right: st.left + ox + dw, bottom: st.top + oy + dh };
                let draw = || {
                    blit_scaled(dc, r, p.w, p.h, &p.bgra);
                    frame(dc, RECT { left: r.left - 1, top: r.top - 1, right: r.right + 1, bottom: r.bottom + 1 }, 1, pal().line);
                };
                if is_fit(zoom) {
                    draw();
                } else {
                    clip(dc, st, draw);
                }
            }
            Some(Body::Pages { pages, total }) => paint_pages(dc, g.stage, pages, *total, font, dpi),
            Some(Body::Text { text: t, kind, cut }) => {
                plate();
                paint_text(dc, g.inner, gen, t, *kind, *cut, font, dpi, None);
            }
            Some(Body::Document { pic, text: t, cut }) => {
                plate();
                paint_text(dc, g.inner, gen, t, TextKind::Document, *cut, font, dpi, pic.as_ref());
            }
            Some(Body::Table { rows, cut }) => {
                plate();
                paint_table(dc, g.inner, rows, *cut, font, dpi);
            }
            Some(Body::List { rows, .. }) => {
                plate();
                paint_list(dc, g.inner, rows, font, dpi);
            }
            Some(Body::Card { icon, rows }) => {
                plate();
                paint_card(dc, g.inner, icon.as_ref(), rows, font, dpi);
            }
            Some(Body::Handler(_)) => {
                plate();
                if hosted != Some(gen) {
                    text(dc, g.stage, tr(crate::Key::OpeningEscCancel), font, pal().sub, DT_CENTER);
                }
            }
            None => text(dc, g.stage, tr(crate::Key::Loading), font, pal().sub, DT_CENTER),
        }
        drop(j);
        paint_bars(dc, h, accent);
        // ‹ › while the mouse is over the popup, square like the app's buttons
        if hover {
            for (r, glyph, which) in [(g.prev, ICON_PREV, Hot::Prev), (g.next, ICON_NEXT, Hot::Next)] {
                let on = hot == which;
                fill(dc, r, if on { pal().card } else { pal().bg });
                frame(dc, r, 1, if on { pal().sub } else { pal().line });
                text(dc, r, glyph, icon_font, if on { pal().text } else { pal().sub }, DT_CENTER);
            }
        }
        // Last, so nothing drawn above covers it. Content rects stay inside this thickness.
        frame(dc, rc, peek_edge(dpi), accent);
    });
}

// Wheel, PgUp / PgDn, Home / End, and Up / Down scroll. Shift+wheel moves sideways (tables, code).
unsafe fn scroll_peek(h: HWND, dy: i32, dx: i32) {
    view(|v| {
        v.y = nudge(v.y, v.max_y, dy);
        v.x = v.x.saturating_add(dx).max(0);
    });
    _ = InvalidateRect(Some(h), None, false);
}

fn lp_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

struct BarNow {
    rects: BarRects,
    lay: BarLayout,
    hot: bool,
}

fn thumb_of(rects: BarRects, b: &BarLayout) -> RECT {
    RECT { left: rects.paint.left, top: b.thumb_top, right: rects.paint.right, bottom: b.thumb_top + b.thumb_h }
}

fn hthumb_of(rects: BarRects, b: &BarLayout) -> RECT {
    RECT { left: b.thumb_top, top: rects.paint.top, right: b.thumb_top + b.thumb_h, bottom: rects.paint.bottom }
}

/// Vertical bar, then the horizontal one. The horizontal bar is only for a zoomed picture or
/// PDF that is wider than the stage: hidden at fit, and hidden when the width already fits.
unsafe fn bars_now(h: HWND) -> (Option<BarNow>, Option<BarNow>) {
    let dpi = app(|a| a.dpi);
    let stage = geo_of(h).stage;
    view(|v| {
        let show_v = v.max_y > 0 && v.page_h > 0;
        let show_h = !is_fit(v.zoom) && v.max_x > 0 && v.page_w > 0;
        let (vr, hr) = peek_scroll_rects(stage, dpi, show_v, show_h);
        let min_thumb = thumb_min(dpi);
        let vert = vr.and_then(|rects| {
            let span = rects.paint.bottom - rects.paint.top;
            bar_layout(v.max_y.saturating_add(v.page_h), v.page_h, rects.paint.top, span, min_thumb, v.y).map(|lay| BarNow {
                hot: v.bar_hot || v.bar_drag.is_some(),
                rects,
                lay,
            })
        });
        let horiz = hr.and_then(|rects| {
            let span = rects.paint.right - rects.paint.left;
            bar_layout(v.max_x.saturating_add(v.page_w), v.page_w, rects.paint.left, span, min_thumb, v.x).map(|lay| BarNow {
                hot: v.hbar_hot || v.hbar_drag.is_some(),
                rects,
                lay,
            })
        });
        (vert, horiz)
    })
}

unsafe fn thumb_hot(h: HWND, x: i32, y: i32) -> bool {
    bars_now(h).0.is_some_and(|b| bar_hit(b.rects.hit, thumb_of(b.rects, &b.lay), x, y) == BarHit::Thumb)
}

unsafe fn hthumb_hot(h: HWND, x: i32, y: i32) -> bool {
    bars_now(h).1.is_some_and(|b| hbar_hit(b.rects.hit, hthumb_of(b.rects, &b.lay), x, y) == BarHit::Thumb)
}

/// Pointer is on either track or thumb, so the click is a scroll and not a pan or a zoom reset.
unsafe fn bar_under(h: HWND, x: i32, y: i32) -> bool {
    let (v, hz) = bars_now(h);
    if let Some(b) = v {
        if bar_hit(b.rects.hit, thumb_of(b.rects, &b.lay), x, y) != BarHit::None {
            return true;
        }
    }
    if let Some(b) = hz {
        if hbar_hit(b.rects.hit, hthumb_of(b.rects, &b.lay), x, y) != BarHit::None {
            return true;
        }
    }
    false
}

// Rounded like the settings window's bar. No pen: the fill is the whole mark.
unsafe fn round_fill(dc: HDC, r: RECT, rad: i32, fill: u32) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let brush = CreateSolidBrush(cref(fill));
    let pen = HPEN(GetStockObject(NULL_PEN).0);
    let op = SelectObject(dc, pen.into());
    let ob = SelectObject(dc, brush.into());
    _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, rad.max(1), rad.max(1));
    SelectObject(dc, op);
    SelectObject(dc, ob);
    _ = DeleteObject(brush.into());
}

unsafe fn paint_bars(dc: HDC, h: HWND, accent: u32) {
    let dpi = app(|a| a.dpi);
    let (vert, horiz) = bars_now(h);
    let s = |v: i32| v * dpi / 96;
    let (bg, text) = (pal().bg, pal().text);
    if let Some(b) = vert {
        let (track, thumb) = bar_rgb(bg, text, accent, b.hot);
        round_fill(dc, b.rects.paint, s(4), track);
        round_fill(dc, thumb_of(b.rects, &b.lay), s(4), thumb);
    }
    if let Some(b) = horiz {
        let (track, thumb) = bar_rgb(bg, text, accent, b.hot);
        round_fill(dc, b.rects.paint, s(4), track);
        round_fill(dc, hthumb_of(b.rects, &b.lay), s(4), thumb);
    }
}

unsafe fn press_bar(h: HWND, x: i32, y: i32) -> bool {
    let (vert, horiz) = bars_now(h);
    if let Some(b) = vert {
        match bar_hit(b.rects.hit, thumb_of(b.rects, &b.lay), x, y) {
            BarHit::None => {}
            BarHit::Thumb => {
                view(|v| v.bar_drag = Some(y - b.lay.thumb_top));
                return true;
            }
            BarHit::Track => {
                let page = view(|v| v.page_h.max(1));
                view(|v| {
                    let prev = v.y;
                    v.y = page_toward(v.y, v.max_y, b.lay.thumb_top, b.lay.thumb_h, y, page);
                    note_scroll(v, prev);
                });
                _ = InvalidateRect(Some(h), None, false);
                return true;
            }
        }
    }
    if let Some(b) = horiz {
        match hbar_hit(b.rects.hit, hthumb_of(b.rects, &b.lay), x, y) {
            BarHit::None => {}
            BarHit::Thumb => {
                view(|v| v.hbar_drag = Some(x - b.lay.thumb_top));
                return true;
            }
            BarHit::Track => {
                let page = view(|v| v.page_w.max(1));
                view(|v| {
                    let prev = v.y;
                    v.x = page_toward(v.x, v.max_x, b.lay.thumb_top, b.lay.thumb_h, x, page);
                    note_scroll(v, prev);
                });
                _ = InvalidateRect(Some(h), None, false);
                return true;
            }
        }
    }
    false
}

unsafe fn drag_bar(h: HWND, y: i32) {
    let Some(off) = view(|v| v.bar_drag) else { return };
    let Some(b) = bars_now(h).0 else { return };
    let travel = (b.rects.paint.bottom - b.rects.paint.top - b.lay.thumb_h).max(0);
    if travel <= 0 {
        return;
    }
    let top = (y - off).clamp(b.rects.paint.top, b.rects.paint.bottom - b.lay.thumb_h);
    let rel = top - b.rects.paint.top;
    view(|v| {
        let prev = v.y;
        v.y = thumb_scroll(rel, travel, b.lay.max_y);
        note_scroll(v, prev);
    });
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn drag_hbar(h: HWND, x: i32) {
    let Some(off) = view(|v| v.hbar_drag) else { return };
    let Some(b) = bars_now(h).1 else { return };
    let travel = (b.rects.paint.right - b.rects.paint.left - b.lay.thumb_h).max(0);
    if travel <= 0 {
        return;
    }
    let left = (x - off).clamp(b.rects.paint.left, b.rects.paint.right - b.lay.thumb_h);
    let rel = left - b.rects.paint.left;
    view(|v| {
        let prev = v.y;
        v.x = thumb_scroll(rel, travel, b.lay.max_y);
        note_scroll(v, prev);
    });
    _ = InvalidateRect(Some(h), None, false);
}

// Up / Down scroll a long page. At the end a fresh press steps to the previous / next file;
// a repeat stops, so holding the key doesn't run through the list. Left / Right always step.
unsafe fn arrow_scroll(h: HWND, down: bool, repeat: bool) {
    let line = app(|a| a.fh).max(1) * 3;
    let act = view(|v| {
        let dy = line.max(v.page_h / 10).max(1);
        let act = arrow_act(v.y, v.max_y, down, repeat, dy);
        if let ArrowAct::Scroll(y) = act {
            let prev = v.y;
            v.y = y;
            note_scroll(v, prev);
        }
        act
    });
    match act {
        ArrowAct::Scroll(_) => _ = InvalidateRect(Some(h), None, false),
        ArrowAct::Step => step_peek(!down),
        ArrowAct::Stay => {}
    }
}

const PEEK_TIP: usize = 61;

fn peek_tip_info(h: HWND, label: &HSTRING) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: h,
        uId: PEEK_TIP,
        lpszText: PWSTR(label.as_ptr() as *mut u16),
        ..Default::default()
    }
}

unsafe fn peek_tip(h: HWND, hot: Hot) {
    let tip = app(|a| a.tip);
    if tip.0.is_null() || h.0.is_null() {
        return;
    }
    let text = match hot {
        Hot::Max if view(|v| v.maximized) => Some(tr(crate::Key::Restore)),
        Hot::Max => Some(tr(crate::Key::Maximise)),
        Hot::Close => Some(tr(crate::Key::Close)),
        _ => None,
    };
    let installed = view(|v| v.tip_on);
    let Some(text) = text else {
        if installed {
            let _ = crate::ui::tip_leave(h, PEEK_TIP, true);
        }
        return;
    };
    let g = geo_of(h);
    let (item_client, kid) = match hot {
        Hot::Max => (g.max, 1u64),
        Hot::Close => (g.close, 2u64),
        _ => return,
    };
    let item = crate::ui::tip_screen_rect(h, item_client);
    let key = 0x6100_0000_0000_0000u64 | kid;
    let (now, _) = crate::ui::tip_enter(
        h,
        PEEK_TIP,
        key,
        text,
        item,
        crate::ui::TipSide::Below,
        installed,
    );
    view(|v| v.tip_on = now);
}

unsafe fn drop_peek_tip(h: HWND) {
    let tip = app(|a| a.tip);
    let on = view(|v| {
        let on = v.tip_on;
        v.tip_on = false;
        on
    });
    if app(|a| a.hover_tip.as_ref().is_some_and(|l| l.tool == PEEK_TIP)) {
        app(|a| a.hover_tip = None);
    }
    if !on || tip.0.is_null() || h.0.is_null() {
        return;
    }
    let empty = HSTRING::from("");
    let ti = peek_tip_info(h, &empty);
    let lp = LPARAM(&ti as *const _ as isize);
    SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(lp));
    SendMessageW(tip, TTM_POP, None, None);
    SendMessageW(tip, TTM_DELTOOLW, None, Some(lp));
}


unsafe fn maximize_peek(h: HWND) {
    let cur = window_rect(h);
    view(|v| {
        if !v.maximized {
            v.restore = Some(cur);
        }
        v.maximized = true;
    });
    let work = main_bounds().work;
    _ = SetWindowPos(h, None, work.left, work.top, (work.right - work.left).max(1), (work.bottom - work.top).max(1), SWP_NOZORDER);
}

unsafe fn restore_peek(h: HWND) {
    let saved = view(|v| {
        v.maximized = false;
        v.restore
    });
    let Some(saved) = saved else { return };
    let b = main_bounds();
    let want = rect_size(saved);
    let size = clamp_to_parent_and_work(want, rect_size(b.parent), rect_size(b.work));
    let rect = if size == want { slide_inside(saved, b.work) } else { center_in(size, b.parent, b.work) };
    _ = SetWindowPos(h, None, rect.left, rect.top, (rect.right - rect.left).max(1), (rect.bottom - rect.top).max(1), SWP_NOZORDER);
    view(|v| v.restore = Some(rect));
}

unsafe fn toggle_peek_max(h: HWND) {
    if view(|v| v.maximized) {
        restore_peek(h);
    } else {
        maximize_peek(h);
    }
    _ = InvalidateRect(Some(h), None, false);
    peek_tip(h, view(|v| v.hot));
}

/// A drag on a maximized header restores first, with the pointer kept in the title strip.
unsafe fn restore_under_cursor(h: HWND) {
    let cur = window_rect(h);
    let saved = view(|v| v.restore).unwrap_or(cur);
    let b = main_bounds();
    let size = clamp_to_parent_and_work(rect_size(saved), rect_size(b.parent), rect_size(b.work));
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() {
        view(|v| v.maximized = false);
        place_peek(h, size);
        return;
    }
    let cur_w = (cur.right - cur.left).max(1) as i64;
    let rel = (pt.x - cur.left).clamp(0, cur_w as i32) as i64;
    let left = pt.x - (rel * size.w as i64 / cur_w) as i32;
    let head = (48 * app(|a| a.dpi) / 96).max(1);
    let top = pt.y - (pt.y - cur.top).clamp(0, head);
    let rect = slide_inside(RECT { left, top, right: left + size.w, bottom: top + size.h }, b.work);
    view(|v| {
        v.maximized = false;
        v.restore = Some(rect);
    });
    _ = SetWindowPos(h, None, rect.left, rect.top, (rect.right - rect.left).max(1), (rect.bottom - rect.top).max(1), SWP_NOZORDER);
}

pub(crate) extern "system" fn peek_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| peek_proc_inner(h, msg, wp, lp)))
}
fn peek_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        // With two windows open, act on the window this popup belongs to.
        bind_hwnd(h);
        let page = || {
            let st = geo_of(h).stage;
            (st.bottom - st.top) * 9 / 10
        };
        let notch = |w: usize| ((w >> 16) as i16 as i32) * app(|a| a.fh) * 3 / 120;
        match msg {
            WM_PAINT => paint_peek(h),
            WM_ERASEBKGND => return LRESULT(1),
            // Dragged onto a monitor with another DPI: take the size Windows suggests so the popup
            // keeps its physical size. Text and chrome still follow the main window's DPI.
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                _ = InvalidateRect(Some(h), None, false);
            }
            // The thick frame is only there so the sizing hits work. The client fills the window;
            // the accent line is painted, not a system border.
            WM_NCCALCSIZE if wp.0 != 0 => return LRESULT(0),
            WM_NCHITTEST => {
                let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                _ = ScreenToClient(h, &mut pt);
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                return LRESULT(peek_nc_hit(rc, pt.x, pt.y, peek_grip(app(|a| a.dpi))) as isize);
            }
            WM_GETMINMAXINFO => {
                let mm = &mut *(lp.0 as *mut MINMAXINFO);
                let dpi = app(|a| a.dpi);
                let b = main_bounds();
                let cap = open_limit(&b);
                let min = min_peek(dpi);
                let track = if view(|v| v.maximized) { rect_size(b.work) } else { cap };
                mm.ptMinTrackSize = POINT { x: min.w.min(cap.w).max(1), y: min.h.min(cap.h).max(1) };
                mm.ptMaxTrackSize = POINT { x: track.w.max(mm.ptMinTrackSize.x), y: track.h.max(mm.ptMinTrackSize.y) };
                let work = rect_size(b.work);
                mm.ptMaxSize = POINT { x: work.w.max(1), y: work.h.max(1) };
                mm.ptMaxPosition = POINT { x: b.work.left, y: b.work.top };
                return LRESULT(0);
            }
            WM_SIZE => {
                sync_host(h);
                // Fit layout already follows the stage. Zoomed content changes size with it, so redraw
                // the whole window; a PDF asks for sharper pages once the resize settles.
                if !is_fit(view(|v| v.zoom)) {
                    _ = InvalidateRect(Some(h), None, false);
                    schedule_pdf_zoom(h);
                }
            }
            WM_TIMER if wp.0 == TIMER_PEEK_ZOOM => {
                _ = KillTimer(Some(h), TIMER_PEEK_ZOOM);
                request_pdf_zoom(h);
            }
            WM_PEEK_ZOOM => {
                // Pixels only. Fitting the window here would fight a size the user already set.
                if peek_job().gen == wp.0 {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_SETCURSOR => {
                let ht = (lp.0 & 0xffff) as u32;
                if ht == HTCLIENT {
                    let (drag, on_bar_drag) = view(|v| (v.pan_drag, v.bar_drag.is_some() || v.hbar_drag.is_some()));
                    let mut pt = POINT::default();
                    let got = GetCursorPos(&mut pt).is_ok() && ScreenToClient(h, &mut pt).as_bool();
                    let over = got && inside(geo_of(h).stage, pt.x, pt.y);
                    // The bar sits on the stage. A hand there would say the click pans.
                    let on_bar = got && bar_under(h, pt.x, pt.y);
                    if !on_bar_drag && !on_bar && (drag || (over && pan_ready(h))) {
                        SetCursor(Some(peek_cursor(drag)));
                        return LRESULT(1);
                    }
                }
                return DefWindowProcW(h, msg, wp, lp);
            }
            WM_ENTERSIZEMOVE => {
                let r = window_rect(h);
                view(|v| v.grip = Some((r.right - r.left, r.bottom - r.top)));
            }
            WM_EXITSIZEMOVE => {
                let rect = window_rect(h);
                let (w, ht) = (rect.right - rect.left, rect.bottom - rect.top);
                let changed = view(|v| v.grip.take().is_some_and(|(pw, ph)| (w - pw).abs() > 1 || (ht - ph).abs() > 1));
                if changed {
                    let kind = view(|v| {
                        v.maximized = false;
                        v.restore = Some(rect);
                        match v.kind {
                            Some(PeekKind::Image) => {
                                v.sized_image = true;
                                Some(PeekKind::Image)
                            }
                            Some(PeekKind::Doc) => {
                                v.sized_doc = true;
                                Some(PeekKind::Doc)
                            }
                            None => {
                                v.sized_pending = true;
                                None
                            }
                        }
                    });
                    if let Some(kind) = kind {
                        remember_peek(h, kind);
                    }
                } else if !view(|v| v.maximized) {
                    view(|v| v.restore = Some(rect));
                }
            }
            WM_PEEK_DONE => {
                // Size before the content paints, so the first frame is already the fitted window.
                fit_on_result(h, wp.0);
                start_handler(h, wp.0);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_PEEK_HOSTED => handler_ready(h, wp.0, lp.0 != 0),
            WM_MOUSEMOVE => {
                let (x, y) = lp_xy(lp);
                if let Some((px, py)) = view(|v| v.pan_pt) {
                    let (dx, dy) = (x - px, y - py);
                    if view(|v| v.pan_drag) || dx.abs() + dy.abs() >= 4 {
                        view(|v| {
                            v.pan_drag = true;
                            v.pan_pt = Some((x, y));
                        });
                        pan_by(h, dx, dy);
                        SetCursor(Some(peek_cursor(true)));
                    }
                } else if view(|v| v.bar_drag.is_some()) {
                    drag_bar(h, y);
                } else if view(|v| v.hbar_drag.is_some()) {
                    drag_hbar(h, x);
                } else if let Some((ox, oy)) = view(|v| v.drag_pt) {
                    if (x - ox).abs() + (y - oy).abs() >= 4 {
                        view(|v| v.drag_pt = None);
                        if view(|v| v.maximized) {
                            restore_under_cursor(h);
                        }
                        _ = ReleaseCapture();
                        _ = SendMessageW(h, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(0)));
                    }
                } else {
                    let hot = peek_hit(&geo_of(h), x, y);
                    let over = thumb_hot(h, x, y);
                    let hover_h = hthumb_hot(h, x, y);
                    let (first, changed) = view(|v| {
                        let r = (!v.hover, !v.hover || v.hot != hot || v.bar_hot != over || v.hbar_hot != hover_h);
                        (v.hover, v.hot, v.bar_hot, v.hbar_hot) = (true, hot, over, hover_h);
                        r
                    });
                    if first {
                        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                        _ = TrackMouseEvent(&mut t);
                    }
                    if changed {
                        _ = InvalidateRect(Some(h), None, false);
                    }
                    if changed || matches!(hot, Hot::Max | Hot::Close) {
                        peek_tip(h, hot);
                    }
                }
            }
            WM_MOUSELEAVE => {
                view(|v| (v.hover, v.hot, v.bar_hot, v.hbar_hot) = (false, Hot::None, false, false));
                peek_tip(h, Hot::None);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lp_xy(lp);
                if press_bar(h, x, y) {
                    view(|v| v.bar_down = true);
                    _ = SetCapture(h);
                } else {
                    let g = geo_of(h);
                    let mut rc = RECT::default();
                    _ = GetClientRect(h, &mut rc);
                    if in_header(rc, &g, x, y, peek_grip(app(|a| a.dpi))) {
                        view(|v| v.drag_pt = Some((x, y)));
                        _ = SetCapture(h);
                    } else if inside(g.stage, x, y) && pan_ready(h) {
                        // Capture like the header drag. The up before WM_LBUTTONDBLCLK releases it,
                        // so a double-click still resets zoom and does not also click ‹ › / ×.
                        view(|v| {
                            v.pan_pt = Some((x, y));
                            v.pan_drag = false;
                        });
                        _ = SetCapture(h);
                    }
                }
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = lp_xy(lp);
                let g = geo_of(h);
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                let hot = peek_hit(&g, x, y);
                if in_header(rc, &g, x, y, peek_grip(app(|a| a.dpi))) {
                    toggle_peek_max(h);
                } else if matches!(hot, Hot::Max | Hot::Close | Hot::Prev | Hot::Next) {
                    view(|v| v.suppress_up = true);
                } else if bar_under(h, x, y) {
                    // On the bar: not a content double-click, so zoom stays where it is.
                    view(|v| v.suppress_up = true);
                } else if inside(g.stage, x, y) && zoom_shape().is_some() {
                    view(|v| v.suppress_up = true);
                    set_zoom(h, 1.0, Some((x - g.stage.left, y - g.stage.top)));
                }
            }
            // ×, the maximise mark, and ‹ › act on a click; the content itself never closes the preview.
            // a press that started on the bar is a scroll, even if the pointer has slid onto ›
            WM_LBUTTONUP => {
                let (suppress, on_bar, dragged, panned) = view(|v| {
                    let suppress = std::mem::replace(&mut v.suppress_up, false);
                    let on = v.bar_down;
                    v.bar_down = false;
                    v.bar_drag = None;
                    v.hbar_drag = None;
                    let dragged = v.drag_pt.take().is_some();
                    let panned = v.pan_pt.take().is_some();
                    v.pan_drag = false;
                    (suppress, on, dragged, panned)
                });
                if on_bar || dragged || panned {
                    _ = ReleaseCapture();
                    if on_bar {
                        _ = InvalidateRect(Some(h), None, false);
                    }
                } else if !suppress {
                    match view(|v| v.hot) {
                        Hot::Close => close_peek(h),
                        Hot::Max => toggle_peek_max(h),
                        Hot::Prev => step_peek(true),
                        Hot::Next => step_peek(false),
                        Hot::None => {}
                    }
                }
            }
            WM_CAPTURECHANGED => {
                view(|v| {
                    (v.bar_drag, v.hbar_drag, v.bar_down, v.drag_pt) = (None, None, false, None);
                    v.pan_pt = None;
                    v.pan_drag = false;
                });
            }
            // Ctrl zooms a picture or PDF about the cursor. Shift still scrolls sideways. Plain wheel is unchanged.
            WM_MOUSEWHEEL if (wp.0 & 0xffff) as u32 & MK_CONTROL.0 != 0 => {
                if zoom_shape().is_some() {
                    let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                    _ = ScreenToClient(h, &mut pt);
                    let st = geo_of(h).stage;
                    let delta = (wp.0 >> 16) as i16 as i32;
                    let z = view(|v| zoom_factor(v.zoom, delta));
                    set_zoom(h, z, Some((pt.x - st.left, pt.y - st.top)));
                }
            }
            WM_MOUSEWHEEL if (wp.0 & 0xffff) as u32 & MK_SHIFT.0 != 0 => scroll_peek(h, 0, -notch(wp.0)),
            WM_MOUSEWHEEL => wheel_peek(h, (wp.0 >> 16) as i16 as i32),
            // tilting the wheel: previous / next file, like the arrows
            WM_MOUSEHWHEEL => step_by_wheel(-((wp.0 >> 16) as i16 as i32)),
            WM_KEYDOWN => {
                let k = VIRTUAL_KEY(wp.0 as u16);
                let repeat = lp.0 & (1 << 30) != 0;
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                if ctrl && matches!(k, VK_0 | VK_NUMPAD0) {
                    set_zoom(h, 1.0, None);
                } else {
                    match k {
                        // a held F3 only opens it; its repeats must not close it again
                        VK_ESCAPE | VK_SPACE | VK_F3 if !repeat => close_peek(h),
                        VK_DELETE if !repeat && !ctrl => delete_peek(shift),
                        VK_F2 if !repeat && !ctrl => rename_peek(),
                        VK_LEFT | VK_RIGHT => step_peek(k == VK_LEFT),
                        VK_UP | VK_DOWN => arrow_scroll(h, k == VK_DOWN, repeat),
                        VK_NEXT => scroll_peek(h, page(), 0),
                        VK_PRIOR => scroll_peek(h, -page(), 0),
                        VK_HOME => scroll_peek(h, i32::MIN, i32::MIN),
                        VK_END => scroll_peek(h, i32::MAX, 0),
                        _ => {}
                    }
                }
            }
            WM_PEEK_DELETED => {
                if wp.0 != 0 {
                    apply_after_peek_delete(h);
                } else {
                    set_after_del(None);
                    HOLD_PEEK.store(false, Ordering::SeqCst);
                    if GetActiveWindow() != h {
                        _ = SetActiveWindow(h);
                        _ = SetFocus(Some(h));
                    }
                }
            }
            // another app or a click behind closes it; stepping takes the activation away only for a moment
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => {
                if !HOLD_PEEK.load(Ordering::SeqCst) {
                    _ = PostMessageW(Some(h), WM_PEEK_LOST, WPARAM(0), LPARAM(0));
                }
            }
            WM_PEEK_LOST if GetActiveWindow() != h && !HOLD_PEEK.load(Ordering::SeqCst) => _ = DestroyWindow(h),
            WM_DESTROY => {
                let from_find = FROM_FIND.load(Ordering::SeqCst);
                let i = app(|a| a.peek_pane);
                drop_zoom(h);
                drop_peek_tip(h);
                mark_find_peek(None);
                unload_handler();
                want_peek(None);
                reset_scroll();
                reset_frame();
                app(|a| a.peek = HWND::default());
                // Focus has left the result list. Ask the pane to hide those rows unless
                // close puts the keyboard back on them (or the search box still has it).
                if from_find {
                    let pane = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)).unwrap_or_default();
                    if !pane.0.is_null() {
                        _ = SetTimer(Some(pane), TIMER_FIND, 120, None);
                    }
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_newest_only() {
        let req = |p: &str| Some(PeekReq { path: p.into(), w: 10, h: 10, hwnd: 0, plain: false });
        let pix = || Peeked { body: Body::Pic(Pix::new(1, 1, vec![0; 4])), icon: -1, meta: Vec::new() };
        let card = || Peeked { body: Body::Card { icon: None, rows: Vec::new() }, icon: -1, meta: Vec::new() };
        let mut j = PeekJob { gen: 0, req: None, done: None, zoom: None };
        j.want(req("a"));
        let a = j.gen;
        j.want(req("b"));
        assert!(!j.publish(a, pix()) && j.done.is_none());
        assert!(j.publish(j.gen, card()) && matches!(j.done, Some(Peeked { body: Body::Card { .. }, .. })));
        let b = j.gen;
        j.want(None);
        assert!(j.gen != b && j.req.is_none() && j.done.is_none());
        assert!(!j.publish(b, pix()) && j.done.is_none());
    }

    #[test]
    fn peek_alpha() {
        let mut photo = [10u8, 20, 30, 0, 40, 50, 60, 0];
        fix_alpha(&mut photo);
        assert_eq!(photo, [10u8, 20, 30, 255, 40, 50, 60, 255]);
        let mut icon = [0u8, 0, 0, 0, 40, 50, 60, 128];
        fix_alpha(&mut icon);
        assert_eq!(icon, [0u8, 0, 0, 0, 40, 50, 60, 128]);
    }

    #[test]
    fn icon_alpha() {
        let mut photo = vec![10, 20, 30, 0, 40, 50, 60, 0];
        fix_alpha(&mut photo);
        assert_eq!(photo, vec![10, 20, 30, 255, 40, 50, 60, 255]);
        let mut icon = vec![255, 255, 255, 128, 200, 100, 0, 255, 0, 0, 0, 0];
        fix_alpha(&mut icon);
        assert_eq!(icon, vec![128, 128, 128, 128, 200, 100, 0, 255, 0, 0, 0, 0]);
        let mut pre = vec![100, 100, 100, 128, 0, 0, 0, 0];
        fix_alpha(&mut pre);
        assert_eq!(pre, vec![100, 100, 100, 128, 0, 0, 0, 0]);
    }

    #[test]
    fn markdown_lines() {
        let l = md_lines("# T\n\nsome **bold** `x`\n- a\n  - b\n> q\n```\ncode **\n```\n---");
        let styles: Vec<Style> = l.iter().map(|x| x.1).collect();
        use Style::*;
        assert_eq!(styles, vec![H1, Plain, Plain, Bullet, Bullet, Quote, Mono, Quote]);
        assert_eq!((l[2].0.as_str(), l[3].0.as_str(), l[4].0.as_str(), l[6].0.as_str()), ("some bold x", "• a", "    • b", "code **"));
    }

    #[test]
    fn peek_wheel() {
        use Wheel::*;
        assert_eq!(wheel_action(0, 0, true, false, true), Step);
        assert_eq!(wheel_action(0, 0, false, false, true), Step);
        assert_eq!(wheel_action(0, 500, true, false, true), Scroll);
        assert_eq!(wheel_action(0, 500, false, false, true), Hold);
        assert_eq!(wheel_action(0, 500, false, true, true), Step);
        assert_eq!(wheel_action(500, 500, true, false, true), Hold);
        assert_eq!(wheel_action(500, 500, true, true, true), Step);
        assert_eq!(wheel_action(500, 500, false, true, true), Scroll);
    }

    #[test]
    fn peek_wheel_off_never_steps() {
        use Wheel::*;
        let cases = [
            (0, 0, true, false),
            (0, 0, false, false),
            (0, 0, true, true),
            (0, 0, false, true),
            (0, 500, true, false),
            (0, 500, false, false),
            (0, 500, false, true),
            (0, 500, true, true),
            (500, 500, true, false),
            (500, 500, true, true),
            (500, 500, false, true),
            (500, 500, false, false),
            (250, 500, true, true),
            (250, 500, false, false),
        ];
        for (y, max_y, down, rested) in cases {
            let act = wheel_action(y, max_y, down, rested, false);
            assert_ne!(act, Step, "y={y} max={max_y} down={down} rested={rested}");
            let room = if down { y < max_y } else { y > 0 };
            assert_eq!(act, if room { Scroll } else { Hold }, "y={y} max={max_y} down={down} rested={rested}");
        }
    }

    #[test]
    fn peek_layout() {
        let rc = RECT { left: 0, top: 0, right: 1120, bottom: 810 };
        let g = peek_geo(rc, 96);
        assert_eq!((g.stage.left, g.stage.top, g.stage.right, g.stage.bottom), (59, 51, 1061, 783));
        assert_eq!((g.inner.left, g.inner.top), (79, 71));
        assert!(g.close.right <= rc.right - 3 && g.close.top >= 3 && g.title.top >= 3 && g.title.right < g.close.left);
        assert!(g.prev.right <= g.stage.left && g.next.left >= g.stage.right);
        assert_eq!((g.prev.top + g.prev.bottom) / 2, (g.stage.top + g.stage.bottom) / 2);
        assert_eq!(peek_hit(&g, g.next.left + 4, g.next.top + 4), Hot::Next);
        assert_eq!(peek_hit(&g, g.prev.left + 4, g.prev.top + 4), Hot::Prev);
        assert_eq!(peek_hit(&g, g.close.left + 4, g.close.top + 4), Hot::Close);
        assert_eq!(peek_hit(&g, g.max.left + 4, g.max.top + 4), Hot::Max);
        assert!(g.max.right <= g.close.left && g.title.right <= g.max.left);
        assert_eq!(peek_hit(&g, 500, 400), Hot::None);
    }

    #[test]
    fn peek_border_inset() {
        let rc = RECT { left: 0, top: 0, right: 1120, bottom: 810 };
        assert_eq!(peek_edge(1), 1);
        assert_eq!((peek_edge(96), peek_edge(144)), (3, 4));
        for dpi in [96, 144] {
            let e = peek_edge(dpi);
            let s = |v: i32| v * dpi / 96;
            let g = peek_geo(rc, dpi);
            assert_eq!(g.stage.left, e + s(56));
            assert_eq!(g.stage.top, e + s(48));
            assert_eq!(g.stage.right, rc.right - e - s(56));
            assert_eq!(g.stage.bottom, rc.bottom - e - s(24));
            assert_eq!((g.inner.left, g.inner.top, g.inner.right, g.inner.bottom), (g.stage.left + s(20), g.stage.top + s(20), g.stage.right - s(20), g.stage.bottom - s(20)));
            for r in [g.icon, g.title, g.max, g.close, g.stage, g.inner, g.prev, g.next] {
                assert!(r.left >= e && r.top >= e && r.right <= rc.right - e && r.bottom <= rc.bottom - e);
            }
        }
    }

    #[test]
    fn scroll_clamps_and_reaches_the_end() {
        assert_eq!(clamp_y(-4, 100), 0);
        assert_eq!(clamp_y(40, 100), 40);
        assert_eq!(clamp_y(400, 100), 100);
        assert_eq!(clamp_y(5, 0), 0);
        assert_eq!(clamp_y(5, -3), 0);

        assert_eq!(nudge(0, 500, 100), 100);
        assert_eq!(nudge(450, 500, 100), 500);
        assert_eq!(nudge(10, 500, -40), 0);
        assert_eq!(nudge(10, 200, i32::MAX), 200);
        assert_eq!(nudge(10, 200, i32::MIN), 0);
        // not laid out yet: a jump is kept for the paint to clamp
        assert_eq!(nudge(10, 0, i32::MAX), i32::MAX);
        assert_eq!(nudge(10, 0, i32::MIN), 0);

        let max_y = 10_000;
        let mut y = 0;
        let mut n = 0;
        while y < max_y {
            let next = nudge(y, max_y, 48);
            assert!(next > y && next <= max_y);
            y = next;
            n += 1;
            assert!(n < 1000);
        }
        assert_eq!(nudge(y, max_y, 48), max_y);
        assert_eq!(nudge(0, max_y, i32::MIN), 0);
    }

    #[test]
    fn arrow_scrolls_until_the_end() {
        use ArrowAct::*;
        assert_eq!(arrow_act(0, 0, true, false, 40), Step);
        assert_eq!(arrow_act(0, 0, true, true, 40), Step);
        assert_eq!(arrow_act(0, 0, false, true, 40), Step);
        assert_eq!(arrow_act(0, 500, true, false, 40), Scroll(40));
        assert_eq!(arrow_act(480, 500, true, false, 40), Scroll(500));
        assert_eq!(arrow_act(500, 500, true, false, 40), Step);
        assert_eq!(arrow_act(500, 500, true, true, 40), Stay);
        assert_eq!(arrow_act(0, 500, false, false, 40), Step);
        assert_eq!(arrow_act(0, 500, false, true, 40), Stay);
        assert_eq!(arrow_act(20, 500, false, false, 40), Scroll(0));
        assert_eq!(arrow_act(100, 500, false, true, 40), Scroll(60));
    }

    #[test]
    fn thumb_size_position_and_drag() {
        assert!(bar_layout(100, 100, 0, 200, 24, 0).is_none());
        assert!(bar_layout(50, 100, 0, 200, 24, 0).is_none());
        assert!(bar_layout(200, 0, 0, 200, 24, 0).is_none());
        assert!(bar_layout(200, 100, 0, 0, 24, 0).is_none());

        // content twice the view: thumb is half the track, ends meet the track ends
        let top = 10;
        let span = 200;
        let b = bar_layout(400, 200, top, span, 24, 0).unwrap();
        assert_eq!(b.max_y, 200);
        assert_eq!(b.thumb_h, 100);
        assert_eq!(b.thumb_top, top);
        let end = bar_layout(400, 200, top, span, 24, 200).unwrap();
        assert_eq!(end.thumb_top, top + span - end.thumb_h);
        assert_eq!(end.thumb_top + end.thumb_h, top + span);
        let mid = bar_layout(400, 200, top, span, 24, 100).unwrap();
        assert_eq!(mid.thumb_top, top + 50);
        let travel = span - b.thumb_h;
        assert_eq!(thumb_scroll(0, travel, b.max_y), 0);
        assert_eq!(thumb_scroll(travel, travel, b.max_y), b.max_y);
        assert_eq!(thumb_scroll(50, travel, b.max_y), 100);

        // y outside the range pins the thumb to the ends
        assert_eq!(bar_layout(400, 200, top, span, 24, -50).unwrap().thumb_top, top);
        assert_eq!(bar_layout(400, 200, top, span, 24, 9_000).unwrap().thumb_top, end.thumb_top);

        // a long document keeps the minimum thumb, and the bottom still lands on the track end
        let long = bar_layout(100_000, 100, 0, span, 24, 0).unwrap();
        assert_eq!(long.thumb_h, 24);
        let long_end = bar_layout(100_000, 100, 0, span, 24, long.max_y).unwrap();
        assert_eq!(long_end.thumb_top + long_end.thumb_h, span);
        assert_eq!(thumb_scroll(span - 24, span - 24, long.max_y), long.max_y);
        assert_eq!(thumb_len(100_000, 100, span, 24), 24);
        // min thumb larger than the track cannot overflow it
        assert_eq!(thumb_len(10_000, 100, 10, 24), 10);
        let tiny = bar_layout(10_000, 100, 5, 10, 24, 50).unwrap();
        assert_eq!((tiny.thumb_h, tiny.thumb_top), (10, 5));
        assert_eq!(thumb_scroll(0, 0, tiny.max_y), 0);

        let mut prev = -1;
        for y in [0, 1, 10, 100, 1000, 5000, 9999, 10_000] {
            let b = bar_layout(20_000, 100, 0, 400, 24, y).unwrap();
            assert!(b.thumb_top >= prev);
            assert!(b.thumb_top + b.thumb_h <= 400);
            assert!(b.thumb_h >= 24);
            prev = b.thumb_top;
        }
    }

    #[test]
    fn track_click_pages_to_the_end() {
        assert_eq!(page_toward(100, 500, 40, 20, 10, 80), 20);
        assert_eq!(page_toward(100, 500, 40, 20, 70, 80), 180);
        assert_eq!(page_toward(100, 500, 40, 20, 40, 80), 100);
        assert_eq!(page_toward(100, 500, 40, 20, 59, 80), 100);
        assert_eq!(page_toward(10, 500, 40, 20, 0, 80), 0);
        assert_eq!(page_toward(490, 500, 40, 20, 100, 80), 500);

        let max_y = 5_000;
        let page = 400;
        let mut y = 0;
        let mut n = 0;
        while y < max_y {
            let next = page_toward(y, max_y, 0, 10, 50, page);
            assert!(next > y);
            y = next;
            n += 1;
            assert!(n < 100);
        }
        assert_eq!(y, max_y);
        assert_eq!(page_toward(0, max_y, 0, 30, 10, page), 0);
    }

    #[test]
    fn bar_hit_and_gutter() {
        let hit = RECT { left: 0, top: 0, right: 20, bottom: 200 };
        let thumb = RECT { left: 4, top: 30, right: 12, bottom: 60 };
        assert_eq!(bar_hit(hit, thumb, 2, 40), BarHit::Thumb);
        assert_eq!(bar_hit(hit, thumb, 18, 59), BarHit::Thumb);
        assert_eq!(bar_hit(hit, thumb, 10, 10), BarHit::Track);
        assert_eq!(bar_hit(hit, thumb, 10, 60), BarHit::Track);
        assert_eq!(bar_hit(hit, thumb, 20, 40), BarHit::None);
        assert_eq!(bar_hit(hit, thumb, 10, 200), BarHit::None);

        for dpi in [96, 120, 144, 168, 192] {
            let rc = RECT { left: 0, top: 0, right: 1400, bottom: 900 };
            let g = peek_geo(rc, dpi);
            let b = peek_bar_rects(g.stage, dpi);
            assert!(b.paint.left >= g.stage.right, "dpi {dpi}");
            assert!(b.paint.right <= g.next.left, "dpi {dpi}");
            assert!(b.hit.right <= g.next.left, "dpi {dpi}");
            assert!(b.hit.left < b.paint.left, "dpi {dpi}");
            assert!(b.paint.top >= g.stage.top && b.paint.bottom <= g.stage.bottom, "dpi {dpi}");
            assert!(b.paint.right > b.paint.left && b.paint.bottom > b.paint.top, "dpi {dpi}");
            // ›'s own pixel is not part of the bar
            assert_eq!(bar_hit(b.hit, b.paint, g.next.left, g.next.top + 1), BarHit::None);
        }
    }

    fn overlaps(a: RECT, b: RECT) -> bool {
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
    }

    #[test]
    fn horizontal_bar_tracks_the_pan_and_leaves_a_corner() {
        for dpi in [96, 120, 144, 168, 192] {
            let g = peek_geo(RECT { left: 0, top: 0, right: 1400, bottom: 900 }, dpi);
            let stage = g.stage;
            let alone = peek_hbar_rects(stage, dpi, 0);
            assert!(alone.paint.left >= stage.left && alone.paint.right <= stage.right, "{dpi}");
            assert!(alone.paint.top >= stage.top && alone.paint.bottom <= stage.bottom, "{dpi}");
            assert!(alone.paint.right - alone.paint.left > alone.paint.bottom - alone.paint.top, "{dpi}");
            assert!(alone.hit.top < alone.paint.top && alone.hit.top >= stage.top, "{dpi}");
            assert_eq!((alone.hit.left, alone.hit.right), (alone.paint.left, alone.paint.right));
            // › sits in the side margin, beside the stage, not on this track.
            assert!(alone.paint.right <= g.next.left, "{dpi}");

            let (Some(v_only), None) = peek_scroll_rects(stage, dpi, true, false) else { panic!("vertical only") };
            let full = peek_bar_rects(stage, dpi);
            assert_eq!(
                (v_only.paint.left, v_only.paint.top, v_only.paint.right, v_only.paint.bottom),
                (full.paint.left, full.paint.top, full.paint.right, full.paint.bottom)
            );
            let (None, Some(h_only)) = peek_scroll_rects(stage, dpi, false, true) else { panic!("horizontal only") };
            assert_eq!(h_only.paint.right, alone.paint.right);

            let (Some(v), Some(h)) = peek_scroll_rects(stage, dpi, true, true) else { panic!("both") };
            assert!(v.paint.bottom < full.paint.bottom, "{dpi}");
            assert!(h.paint.right < alone.paint.right, "{dpi}");
            assert!(v.paint.bottom <= h.paint.top, "{dpi}");
            assert!(h.paint.right <= v.paint.left, "{dpi}");
            assert!(!overlaps(v.paint, h.paint), "{dpi}");
            assert!(!overlaps(v.hit, h.hit), "{dpi}");
            // The corner cell itself is empty.
            let corner = RECT { left: h.paint.right, top: v.paint.bottom, right: v.paint.right, bottom: h.paint.bottom };
            assert!(corner.right > corner.left && corner.bottom > corner.top, "{dpi}");
            assert!(!overlaps(corner, v.paint) && !overlaps(corner, h.paint), "{dpi}");
        }

        // Wider than the view: the thumb is half the track and follows the horizontal offset.
        let stage = RECT { left: 10, top: 20, right: 510, bottom: 420 };
        let h = peek_hbar_rects(stage, 96, 0);
        let span = h.paint.right - h.paint.left;
        assert!(bar_layout(span, span, h.paint.left, span, 24, 0).is_none());
        let start = bar_layout(span * 2, span, h.paint.left, span, 24, 0).unwrap();
        assert_eq!(start.max_y, span);
        assert_eq!(start.thumb_h, span / 2);
        assert_eq!(start.thumb_top, h.paint.left);
        let mid = bar_layout(span * 2, span, h.paint.left, span, 24, span / 2).unwrap();
        assert_eq!(mid.thumb_top, h.paint.left + span / 4);
        let end = bar_layout(span * 2, span, h.paint.left, span, 24, span).unwrap();
        assert_eq!(end.thumb_top + end.thumb_h, h.paint.right);
        let travel = span - start.thumb_h;
        assert_eq!(thumb_scroll(travel, travel, start.max_y), start.max_y);
        assert_eq!(page_toward(span / 2, span, mid.thumb_top, mid.thumb_h, h.paint.left, span / 5), span / 2 - span / 5);
        assert_eq!(page_toward(span / 2, span, mid.thumb_top, mid.thumb_h, h.paint.right - 1, span / 5), span / 2 + span / 5);

        let thumb = hthumb_of(h, &mid);
        assert_eq!(hbar_hit(h.hit, thumb, mid.thumb_top + 2, h.hit.top), BarHit::Thumb);
        assert_eq!(hbar_hit(h.hit, thumb, h.paint.left + 1, (h.paint.top + h.paint.bottom) / 2), BarHit::Track);
        assert_eq!(hbar_hit(h.hit, thumb, h.paint.left + 1, h.hit.top - 1), BarHit::None);
        assert_eq!(hbar_hit(h.hit, thumb, h.paint.right, h.paint.top + 1), BarHit::None);
    }

    #[test]
    fn peek_window_fits_clamps_and_centers() {
        let chrome = PeekChrome { w: 100, h: 80 };
        let limit = PeekSize { w: 1000, h: 800 };
        // 2:1 picture, larger than the content box: width-bound, then the frame is added back.
        let wide = fit_size(FitContent::Aspect { w: 1800, h: 900 }, chrome, limit);
        assert_eq!(wide, PeekSize { w: 1000, h: 530 });
        // A picture that already fits is not enlarged.
        let small = fit_size(FitContent::Aspect { w: 200, h: 100 }, chrome, limit);
        assert_eq!(small, PeekSize { w: 300, h: 180 });
        // Portrait page: height-bound.
        let page = fit_size(FitContent::Aspect { w: 1000, h: 4000 }, PeekChrome { w: 0, h: 0 }, PeekSize { w: 800, h: 600 });
        assert_eq!(page, PeekSize { w: 150, h: 600 });
        // 100 characters, clamped into the limit. One line still opens at the minimum line count.
        let text = fit_size(FitContent::Text { char_px: 10, line_px: 20, lines: 1, gutter: 0 }, PeekChrome { w: 0, h: 0 }, PeekSize { w: 2000, h: 2000 });
        assert_eq!(text, PeekSize { w: PEEK_TEXT_COLS * 10, h: PEEK_TEXT_MIN_LINES * 20 });
        let clamped = fit_size(FitContent::Text { char_px: 10, line_px: 20, lines: 10_000, gutter: 40 }, PeekChrome { w: 50, h: 40 }, PeekSize { w: 800, h: 600 });
        assert_eq!(clamped, PeekSize { w: 800, h: 600 });
        assert_eq!(fit_size(FitContent::Aspect { w: 0, h: 10 }, chrome, limit), PeekSize { w: 100, h: 80 });

        assert_eq!(
            clamp_to_parent_and_work(PeekSize { w: 2000, h: 100 }, PeekSize { w: 800, h: 600 }, PeekSize { w: 1000, h: 50 }),
            PeekSize { w: 800, h: 50 }
        );
        assert_eq!(
            clamp_to_parent_and_work(PeekSize { w: -5, h: 40 }, PeekSize { w: 100, h: 100 }, PeekSize { w: 90, h: 80 }),
            PeekSize { w: 0, h: 40 }
        );

        let parent = RECT { left: 2000, top: 100, right: 2800, bottom: 800 };
        let work = RECT { left: 1920, top: 0, right: 3840, bottom: 1080 };
        let placed = center_in(PeekSize { w: 400, h: 300 }, parent, work);
        assert_eq!(placed, RECT { left: 2200, top: 300, right: 2600, bottom: 600 });
        // Centered on a parent that hangs off the work area, then slid fully inside.
        let edge = center_in(PeekSize { w: 400, h: 300 }, RECT { left: 1900, top: -100, right: 2100, bottom: 200 }, work);
        assert_eq!(edge.left, 1920);
        assert_eq!(edge.top, 0);
        assert_eq!((edge.right - edge.left, edge.bottom - edge.top), (400, 300));

        let fitted = PeekSize { w: 10, h: 10 };
        let kept = PeekSize { w: 3, h: 4 };
        assert_eq!(pick_open_size(None, false, false, fitted), Some(fitted));
        assert_eq!(pick_open_size(Some(kept), false, false, fitted), Some(kept));
        assert_eq!(pick_open_size(Some(kept), true, false, fitted), None);
        assert_eq!(pick_open_size(None, false, true, fitted), None);

        let pic = Body::Pic(Pix::new(640, 480, vec![]));
        assert_eq!(PeekKind::of(&pic), PeekKind::Image);
        assert_eq!(fit_source(&pic), FitSource::Aspect { w: 640, h: 480 });
        let pages = Body::Pages { pages: vec![Pix::new(400, 560, vec![])], total: 3 };
        assert_eq!(PeekKind::of(&pages), PeekKind::Doc);
        assert_eq!(fit_source(&pages), FitSource::Aspect { w: 400, h: 560 });
        let code = Body::Text { text: "a\nb\nc".into(), kind: TextKind::Code, cut: false };
        assert_eq!(PeekKind::of(&code), PeekKind::Doc);
        assert_eq!(fit_source(&code), FitSource::Lines { lines: 3, code: true });

        let rc = RECT { left: 0, top: 0, right: 400, bottom: 300 };
        assert_eq!(peek_nc_hit(rc, 1, 40, 6), HTLEFT);
        assert_eq!(peek_nc_hit(rc, 398, 40, 6), HTRIGHT);
        assert_eq!(peek_nc_hit(rc, 50, 1, 6), HTTOP);
        assert_eq!(peek_nc_hit(rc, 50, 298, 6), HTBOTTOM);
        assert_eq!(peek_nc_hit(rc, 1, 1, 6), HTTOPLEFT);
        assert_eq!(peek_nc_hit(rc, 398, 1, 6), HTTOPRIGHT);
        assert_eq!(peek_nc_hit(rc, 1, 298, 6), HTBOTTOMLEFT);
        assert_eq!(peek_nc_hit(rc, 398, 298, 6), HTBOTTOMRIGHT);
        assert_eq!(peek_nc_hit(rc, 20, 20, 6), HTCLIENT);
        assert_eq!(peek_nc_hit(rc, -1, 10, 6), HTNOWHERE);

        for dpi in [96, 144, 192] {
            let window = RECT { left: 0, top: 0, right: 900, bottom: 700 };
            let g = peek_geo(window, dpi);
            let stage = peek_chrome(dpi, false);
            let inner = peek_chrome(dpi, true);
            assert_eq!(stage.w, window.right - (g.stage.right - g.stage.left), "{dpi}");
            assert_eq!(stage.h, window.bottom - (g.stage.bottom - g.stage.top), "{dpi}");
            assert_eq!(inner.w, window.right - (g.inner.right - g.inner.left), "{dpi}");
            assert_eq!(inner.h, window.bottom - (g.inner.bottom - g.inner.top), "{dpi}");
            assert!(in_header(window, &g, g.title.left + 4, g.title.top + peek_grip(dpi) + 2, peek_grip(dpi)));
            assert!(!in_header(window, &g, g.max.left + 2, g.max.top + 2, peek_grip(dpi)));
            assert!(!in_header(window, &g, 2, g.title.top + 8, peek_grip(dpi)));
        }
    }

    #[test]
    fn peek_zoom_steps_around_the_cursor_and_clamps_the_pan() {
        let close = |a: f64, b: f64| assert!((a - b).abs() < 1e-9, "{a} != {b}");
        close(zoom_factor(1.0, 120), 1.25);
        close(zoom_factor(1.0, -120), 0.8);
        close(zoom_factor(1.0, 240), 1.5625);
        close(zoom_factor(1.0, 0), 1.0);
        // 0 is the unset Default and means fit, same as 1.
        close(zoom_factor(0.0, 120), 1.25);
        assert_eq!(zoom_factor(0.1, -1200), ZOOM_MIN);
        assert_eq!(zoom_factor(ZOOM_MAX, 1200), ZOOM_MAX);
        assert_eq!(zoom_factor(7.5, 120), ZOOM_MAX);
        assert!((zoom_factor(ZOOM_MIN, 120) - 0.125).abs() < 1e-9);
        let mut z = 1.0;
        for _ in 0..4 {
            z = zoom_factor(z, 120);
        }
        close(z, 1.25 * 1.25 * 1.25 * 1.25);
        for _ in 0..4 {
            z = zoom_factor(z, -120);
        }
        close(z, 1.0);
        assert_eq!(zoom_pct(1.0), 100);
        assert_eq!(zoom_pct(1.5), 150);
        assert_eq!(zoom_pct(ZOOM_MIN), 10);
        assert_eq!(zoom_pct(ZOOM_MAX), 800);
        assert_eq!(zoom_pct(0.0), 100);
        assert_eq!(zoom_label("JPEG   ·   1.2 MB", 150), "JPEG   ·   1.2 MB  150%");
        assert_eq!(zoom_label("", 100), "100%");

        // Fit never enlarges; a wide picture is width-bound.
        assert_eq!(image_fit_px(100, 50, 400, 400), (100, 50));
        assert_eq!(image_fit_px(1000, 500, 200, 200), (200, 100));
        assert_eq!(page_fit_size(800, 1000, 400), (400, 500));
        assert_eq!(page_fit_size(100, 50, 400), (100, 50));
        assert_eq!(page_fit_size(0, 10, 100), (0, 0));
        // One page, narrower than the stage: the old top-left stack, no vertical centering.
        let (w, h, boxes) = pdf_layout(&[(400, 560)], 500, 16, 0, 1.0);
        assert_eq!((w, h), (400, 560));
        assert_eq!(boxes, vec![PdfBox { x: 0, y: 0, w: 400, h: 560 }]);
        // Two pages fitted to the stage width, gap between, footer under them. Zoom 1 uses the integer fit.
        let (w, h, boxes) = pdf_layout(&[(800, 1000), (800, 1000)], 400, 16, 48, 1.0);
        assert_eq!((w, h), (400, 500 + 16 + 500 + 48));
        assert_eq!(boxes[0], PdfBox { x: 0, y: 0, w: 400, h: 500 });
        assert_eq!(boxes[1], PdfBox { x: 0, y: 516, w: 400, h: 500 });
        assert_eq!(cap_render(100, 200), (100, 200));
        assert_eq!(cap_render(8192, 100), (8192, 100));
        assert_eq!(cap_render(16384, 8192), (8192, 4096));
        assert_eq!(cap_render(0, 10), (1, 10));
        let (rw, rh) = zoom_render_px(800, 1000, 400, 2.0);
        assert_eq!((rw, rh), (800, 1000));
        let (rw, rh) = zoom_render_px(8000, 8000, 8000, 8.0);
        assert!(rw <= ZOOM_RENDER_MAX && rh <= ZOOM_RENDER_MAX);
        assert_eq!(pdf_fit_px(800.0, 1100.0, 800, 1000).0, 727);

        let center = (FitAlign::Center, FitAlign::Center);
        let doc = (FitAlign::Center, FitAlign::Start);
        // The point under the cursor stays under the cursor. Center of a fitted picture, doubled.
        let view = (200, 100);
        let scroll = zoom_scroll((0, 0), (100, 50), (200, 100), (400, 200), view, 2.0, center);
        assert_eq!(scroll, (100, 50));
        // Top-left stays put.
        assert_eq!(zoom_scroll((0, 0), (0, 0), (200, 100), (400, 200), view, 2.0, center), (0, 0));
        // A centered picture smaller than the stage: its centre stays under the cursor as it grows past the view.
        assert_eq!(zoom_scroll((0, 0), (100, 100), (100, 100), (300, 300), (200, 200), 3.0, center), (50, 50));
        // Zooming out until it fits centers it and clears the pan.
        assert_eq!(zoom_scroll((40, 10), (50, 40), (200, 80), (100, 40), (200, 100), 0.5, center), (0, 0));
        // A short PDF stays top-aligned (origin y = 0), matching fit, and still pans horizontally.
        assert_eq!(zoom_scroll((0, 30), (10, 10), (100, 40), (50, 20), (200, 100), 0.5, doc), (0, 0));

        // Drag follows the pointer and cannot open a gap on either side.
        assert_eq!(pan_scroll((0, 0), (30, 0), (300, 100), (200, 100), center), (0, 0));
        assert_eq!(pan_scroll((0, 0), (-30, 0), (300, 100), (200, 100), center), (30, 0));
        assert_eq!(pan_scroll((0, 0), (-500, -80), (300, 180), (200, 100), center), (100, 80));
        assert_eq!(pan_scroll((100, 80), (10_000, 10_000), (300, 180), (200, 100), center), (0, 0));
        // Smaller than the stage: the pan is ignored and it stays centered (scroll 0).
        assert_eq!(pan_scroll((0, 0), (40, -20), (50, 40), (200, 100), center), (0, 0));
        assert_eq!(pan_scroll((12, 9), (40, -20), (50, 40), (200, 100), center), (0, 0));
        // PDF: vertical start, so a short page does not slide down off the top.
        assert_eq!(pan_scroll((0, 0), (0, 40), (300, 40), (200, 100), doc), (0, 0));
        assert_eq!(pan_scroll((0, 0), (-40, 40), (300, 400), (200, 100), doc), (40, 0));
    }

    #[test]
    fn bar_colors_read_on_both_themes() {
        for (bg, text, accent) in [(LIGHT.bg, LIGHT.text, 0x0067C0u32), (DARK.bg, DARK.text, 0x4CC2FFu32), (DARK.bg, DARK.text, 0x4B4B4Bu32)] {
            let (track, thumb) = bar_rgb(bg, text, accent, false);
            let (track_hot, thumb_hot) = bar_rgb(bg, text, accent, true);
            assert_ne!(track, bg);
            assert_ne!(thumb, bg);
            assert_ne!(thumb, track);
            assert_ne!(thumb_hot, thumb);
            assert_ne!(thumb_hot, bg);
            assert_eq!(track_hot, track);
        }
    }
}
