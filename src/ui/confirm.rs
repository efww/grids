//! Yes/no confirm on the token window ground. Cancel is a secondary button.
//! The other button is primary OK, or danger when the action deletes or resets.
//!
//! OK-only notices stay on the system message box. The crash-report fallback does too:
//! that path can run before this window class exists.

use std::cell::{Cell, RefCell};

use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;

use super::button::{paint_button, visual_from_item, ButtonKind, FocusRing};
use super::tokens::{px, ui_tokens, BTN_GAP, BTN_H_FOOTER, BTN_MIN_W, BTN_PAD_X, FONT_META_PX, FONT_PX, PAD_BODY_X, PAD_BODY_Y};
use crate::imports::*;

const ID_AFFIRM: i32 = 1;
const ID_CANCEL: i32 = 2;

/// Mockup (E) window, `.win.w-sm`.
const CONFIRM_W_DIP: i32 = 360;
/// Message to the detail line: tight body gap 10 plus `.confirm-sub` margin-top 6.
const DETAIL_GAP_DIP: i32 = 16;
/// Last line of text to the footer rule: tight body gap 10 plus `.foot` margin-top 2.
const RULE_GAP_DIP: i32 = 12;
/// `.foot` padding above the buttons.
const FOOT_PAD_DIP: i32 = 12;
/// Long prompts stay on screen. The buttons remain below this cap.
const TEXT_CAP_DIP: i32 = 320;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfirmTone {
    Primary,
    Danger,
}

/// One blocking confirm. Build it with [`Confirm::ok`] or [`Confirm::danger`].
pub(crate) struct Confirm {
    title: String,
    message: String,
    detail: String,
    affirm: String,
    tone: ConfirmTone,
    default_cancel: bool,
}

impl Confirm {
    /// Cancel plus a cerulean OK. `true` from [`ask_confirm`] means OK.
    pub(crate) fn ok(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(title, message, ConfirmTone::Primary)
    }

    /// Cancel plus a danger button. The label is Delete unless [`Confirm::affirm_label`] sets one.
    pub(crate) fn danger(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(title, message, ConfirmTone::Danger)
    }

    fn new(title: impl Into<String>, message: impl Into<String>, tone: ConfirmTone) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            detail: String::new(),
            affirm: String::new(),
            tone,
            default_cancel: false,
        }
    }

    /// Second line, in the secondary text colour. Empty hides it.
    pub(crate) fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    /// Affirmative label. Empty keeps OK or Delete for the tone.
    pub(crate) fn affirm_label(mut self, label: impl Into<String>) -> Self {
        self.affirm = label.into();
        self
    }

    /// Esc still cancels. Enter and the first focus land on Cancel.
    pub(crate) fn default_cancel(mut self) -> Self {
        self.default_cancel = true;
        self
    }
}

struct DialogFonts {
    body: HFONT,
    detail: HFONT,
}

struct Live {
    hwnd: HWND,
    message: String,
    detail: String,
    cancel: String,
    affirm: String,
    kind: ButtonKind,
    fonts: DialogFonts,
    msg_h: i32,
    detail_h: i32,
}

thread_local! {
    static DONE: Cell<Option<bool>> = const { Cell::new(None) };
    static DEFAULT_CANCEL: Cell<bool> = const { Cell::new(false) };
    static LIVE: RefCell<Option<Live>> = const { RefCell::new(None) };
}

/// Puts the owner back, drops the dialog fonts, and clears the modal flag.
struct Guard {
    owner: HWND,
    disabled: bool,
    modal_was: bool,
}

impl Guard {
    fn arm(owner: HWND) -> Self {
        let modal_was = crate::keys::modal();
        crate::keys::set_modal(true);
        let disabled = !owner.0.is_null() && unsafe { IsWindow(Some(owner)).as_bool() && IsWindowEnabled(owner).as_bool() };
        if disabled {
            unsafe { let _ = EnableWindow(owner, false); }
        }
        Self { owner, disabled, modal_was }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            let hwnd = LIVE.with(|s| s.borrow().as_ref().map(|l| l.hwnd).unwrap_or_default());
            if !hwnd.0.is_null() && IsWindow(Some(hwnd)).as_bool() {
                let _ = DestroyWindow(hwnd);
            }
            let fonts = LIVE.with(|s| s.borrow_mut().take().map(|l| l.fonts));
            if let Some(fonts) = fonts {
                crate::delete_font(fonts.body);
                crate::delete_font(fonts.detail);
            }
            DONE.with(|d| d.set(None));
            crate::set_hover(HWND::default());
            crate::keys::set_modal(self.modal_was);
            if self.disabled && !self.owner.0.is_null() && IsWindow(Some(self.owner)).as_bool() {
                let _ = EnableWindow(self.owner, true);
                let _ = SetForegroundWindow(self.owner);
            }
        }
    }
}

struct ConfirmMetrics {
    msg_h: i32,
    detail_h: i32,
    cancel_w: i32,
    affirm_w: i32,
}

struct ConfirmLay {
    message: RECT,
    detail: RECT,
    rule: RECT,
    cancel: RECT,
    affirm: RECT,
    client_h: i32,
}

fn confirm_client_w(dpi: i32, m: &ConfirmMetrics) -> i32 {
    let pad = px(dpi, PAD_BODY_X);
    let need = pad * 2 + m.cancel_w + px(dpi, BTN_GAP) + m.affirm_w;
    need.max(px(dpi, CONFIRM_W_DIP))
}

fn confirm_layout(client_w: i32, dpi: i32, m: ConfirmMetrics) -> ConfirmLay {
    let pad_x = px(dpi, PAD_BODY_X);
    let pad_y = px(dpi, PAD_BODY_Y);
    let cw = client_w.max(1);
    let left = pad_x.min(cw / 2);
    let right = (cw - pad_x).max(left + 1);
    let mut y = pad_y;
    let message = RECT { left, top: y, right, bottom: y + m.msg_h.max(0) };
    y = message.bottom;
    let detail = if m.detail_h > 0 {
        y += px(dpi, DETAIL_GAP_DIP);
        let d = RECT { left, top: y, right, bottom: y + m.detail_h };
        y = d.bottom;
        d
    } else {
        RECT { left, top: y, right: left, bottom: y }
    };
    y += px(dpi, RULE_GAP_DIP);
    let rule_h = px(dpi, 1).max(1);
    let rule = RECT { left, top: y, right, bottom: y + rule_h };
    y = rule.bottom + px(dpi, FOOT_PAD_DIP);
    let btn_h = px(dpi, BTN_H_FOOTER);
    let gap = px(dpi, BTN_GAP);
    let affirm_w = m.affirm_w.max(px(dpi, BTN_MIN_W));
    let cancel_w = m.cancel_w.max(px(dpi, BTN_MIN_W));
    let affirm = RECT { left: (right - affirm_w).max(left), top: y, right, bottom: y + btn_h };
    let cancel_right = (affirm.left - gap).max(left);
    let cancel = RECT { left: (cancel_right - cancel_w).max(left), top: y, right: cancel_right, bottom: y + btn_h };
    ConfirmLay { message, detail, rule, cancel, affirm, client_h: affirm.bottom + pad_y }
}

fn button_kind_for(id: i32, affirm: ButtonKind) -> ButtonKind {
    if id == ID_AFFIRM { affirm } else { ButtonKind::Secondary }
}

fn affirm_label(tone: ConfirmTone, custom: &str) -> String {
    let custom = custom.trim();
    if !custom.is_empty() {
        return custom.to_string();
    }
    match tone {
        ConfirmTone::Primary => crate::tr(crate::Key::Ok).to_string(),
        ConfirmTone::Danger => crate::tr(crate::Key::Delete).to_string(),
    }
}

fn cancel_label() -> String {
    crate::tr(crate::Key::Cancel).to_string()
}

fn confirm_style() -> WINDOW_STYLE {
    WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN
}

fn make_font(px_size: i32, dpi: i32) -> HFONT {
    let face = crate::ui_face();
    let mut lf = LOGFONTW {
        lfHeight: -(px_size * dpi.max(96) / 96).max(1),
        lfWeight: 400,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    unsafe { CreateFontIndirectW(&lf) }
}

fn make_fonts(dpi: i32) -> DialogFonts {
    DialogFonts { body: make_font(FONT_PX, dpi), detail: make_font(FONT_META_PX, dpi) }
}

fn swap_fonts(dpi: i32) {
    let next = make_fonts(dpi);
    let old = LIVE.with(|s| {
        let mut slot = s.borrow_mut();
        let Some(live) = slot.as_mut() else { return None };
        Some(std::mem::replace(&mut live.fonts, next))
    });
    if let Some(old) = old {
        unsafe {
            crate::delete_font(old.body);
            crate::delete_font(old.detail);
        }
    }
}

unsafe fn label_px(dc: HDC, font: HFONT, s: &str, dpi: i32) -> i32 {
    let min_w = px(dpi, BTN_MIN_W);
    if s.is_empty() {
        return min_w;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    let old = SelectObject(dc, font.into());
    let _ = DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
    (r.right + px(dpi, BTN_PAD_X) * 2).max(min_w)
}

unsafe fn block_px(dc: HDC, font: HFONT, s: &str, width: i32, dpi: i32) -> i32 {
    if s.is_empty() || width <= 0 {
        return 0;
    }
    let (_, h) = crate::wrap_text(dc, RECT { left: 0, top: 0, right: width, bottom: 0 }, s, font, 0, true);
    h.max(1).min(px(dpi, TEXT_CAP_DIP))
}

fn work_rect(owner: HWND) -> RECT {
    unsafe {
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let mon = if owner.0.is_null() {
            MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
        } else {
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST)
        };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            mi.rcWork
        } else {
            RECT { left: 0, top: 0, right: 1280, bottom: 800 }
        }
    }
}

fn center_on(owner: HWND, ww: i32, hh: i32) -> (i32, i32) {
    let work = work_rect(owner);
    let (x0, y0) = if owner.0.is_null() {
        (work.left + (work.right - work.left - ww) / 2, work.top + (work.bottom - work.top - hh) / 2)
    } else {
        let mut orc = RECT::default();
        unsafe { let _ = GetWindowRect(owner, &mut orc); }
        (orc.left + (orc.right - orc.left - ww) / 2, orc.top + (orc.bottom - orc.top - hh) / 2)
    };
    let x = x0.clamp(work.left, (work.right - ww).max(work.left));
    let y = y0.clamp(work.top, (work.bottom - hh).max(work.top));
    (x, y)
}

fn clamp_pos(owner: HWND, x: i32, y: i32, ww: i32, hh: i32) -> (i32, i32) {
    let work = work_rect(owner);
    (x.clamp(work.left, (work.right - ww).max(work.left)), y.clamp(work.top, (work.bottom - hh).max(work.top)))
}

unsafe fn outer_size(client_w: i32, client_h: i32, dpi: i32) -> (i32, i32) {
    let mut r = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
    if AdjustWindowRectExForDpi(&mut r, confirm_style(), false, WS_EX_DLGMODALFRAME, dpi.max(96) as u32).is_ok() {
        ((r.right - r.left).max(client_w), (r.bottom - r.top).max(client_h))
    } else {
        (client_w, client_h + px(dpi, 32))
    }
}

unsafe fn style_caption(h: HWND) {
    // Same caption as Settings and the name prompt: token ground, no cerulean fill.
    let dark = ui_tokens().dark;
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = crate::cref(ui_tokens().bg);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    let def = COLORREF(0xFFFF_FFFF);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(36), &def as *const _ as _, 4);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(34), &def as *const _ as _, 4);
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(confirm_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(hinst), PCWSTR(crate::window_icon_group(false) as _)).unwrap_or_default(),
            lpszClassName: w!("MaguroConfirm"),
            ..Default::default()
        });
    });
}

unsafe fn ctl(h: HWND, id: i32) -> HWND {
    GetDlgItem(Some(h), id).unwrap_or_default()
}

unsafe fn move_ctl(h: HWND, id: i32, r: RECT) {
    let _ = SetWindowPos(ctl(h, id), None, r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1), SWP_NOZORDER | SWP_NOACTIVATE);
}

enum Place {
    Center(HWND),
    At(i32, i32),
}

unsafe fn layout_now(h: HWND, place: Place) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let dc = GetDC(Some(h));
    let (cancel_w, affirm_w, msg_h, detail_h, client_w) = LIVE.with(|s| {
        let live = s.borrow();
        let Some(live) = live.as_ref() else { return (0, 0, 0, 0, 0) };
        let cancel_w = label_px(dc, live.fonts.body, &live.cancel, dpi);
        let affirm_w = label_px(dc, live.fonts.body, &live.affirm, dpi);
        let client_w = confirm_client_w(dpi, &ConfirmMetrics { msg_h: 0, detail_h: 0, cancel_w, affirm_w });
        let inner = (client_w - px(dpi, PAD_BODY_X) * 2).max(1);
        let msg_h = block_px(dc, live.fonts.body, &live.message, inner, dpi);
        let detail_h = block_px(dc, live.fonts.detail, &live.detail, inner, dpi);
        (cancel_w, affirm_w, msg_h, detail_h, client_w)
    });
    let _ = ReleaseDC(Some(h), dc);
    if client_w <= 0 {
        return;
    }
    LIVE.with(|s| {
        if let Some(live) = s.borrow_mut().as_mut() {
            live.msg_h = msg_h;
            live.detail_h = detail_h;
        }
    });
    let lay = confirm_layout(client_w, dpi, ConfirmMetrics { msg_h, detail_h, cancel_w, affirm_w });
    let (ww, hh) = outer_size(client_w, lay.client_h, dpi);
    let owner = GetWindow(h, GW_OWNER).unwrap_or_default();
    let (x, y) = match place {
        Place::Center(owner) => center_on(owner, ww, hh),
        Place::At(x, y) => clamp_pos(owner, x, y, ww, hh),
    };
    let _ = SetWindowPos(h, None, x, y, ww, hh, SWP_NOZORDER | SWP_NOACTIVATE);
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let lay = confirm_layout(rc.right - rc.left, dpi, ConfirmMetrics { msg_h, detail_h, cancel_w, affirm_w });
    move_ctl(h, ID_CANCEL, lay.cancel);
    move_ctl(h, ID_AFFIRM, lay.affirm);
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn paint(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let t = ui_tokens();
    crate::buffered(h, |dc, rc| {
        crate::fill(dc, rc, t.bg);
        let snap = LIVE.with(|s| {
            let live = s.borrow();
            let Some(live) = live.as_ref() else { return None };
            let lay = confirm_layout(
                rc.right - rc.left,
                dpi,
                ConfirmMetrics { msg_h: live.msg_h, detail_h: live.detail_h, cancel_w: 0, affirm_w: 0 },
            );
            Some((lay.message, lay.detail, lay.rule, live.message.clone(), live.detail.clone(), live.fonts.body, live.fonts.detail))
        });
        let Some((message, detail, rule, msg, det, body, small)) = snap else { return };
        if message.bottom > message.top && !msg.is_empty() {
            crate::wrap_text(dc, message, &msg, body, t.text, false);
        }
        if !det.is_empty() && detail.right > detail.left && detail.bottom > detail.top {
            crate::wrap_text(dc, detail, &det, small, t.sub, false);
        }
        if rule.right > rule.left && rule.bottom > rule.top {
            crate::fill(dc, rule, t.line);
        }
    });
}

fn finish(ok: bool) {
    if DONE.with(|d| d.get().is_some()) {
        return;
    }
    DONE.with(|d| d.set(Some(ok)));
    let hwnd = LIVE.with(|s| s.borrow().as_ref().map(|l| l.hwnd).unwrap_or_default());
    if !hwnd.0.is_null() {
        unsafe { let _ = DestroyWindow(hwnd); }
    }
}

unsafe extern "system" fn confirm_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { confirm_proc_inner(h, msg, wp, lp) }))
}

unsafe fn confirm_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(h);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_CTLCOLORBTN => {
            let dc = HDC(wp.0 as _);
            let ground = crate::cref(ui_tokens().bg);
            SetBkColor(dc, ground);
            SetDCBrushColor(dc, ground);
            return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
        }
        WM_DRAWITEM => {
            let d = &*(lp.0 as *const DRAWITEMSTRUCT);
            let id = d.CtlID as i32;
            let (kind, label, font) = LIVE.with(|s| {
                let live = s.borrow();
                let Some(live) = live.as_ref() else { return (ButtonKind::Secondary, String::new(), HFONT::default()) };
                let kind = button_kind_for(id, live.kind);
                let label = if id == ID_AFFIRM { live.affirm.clone() } else { live.cancel.clone() };
                (kind, label, live.fonts.body)
            });
            if label.is_empty() {
                return LRESULT(1);
            }
            let hovered = crate::app(|a| a.hover) == d.hwndItem;
            let dpi = GetDpiForWindow(h).max(96) as i32;
            paint_button(d.hDC, d.rcItem, kind, visual_from_item(d.itemState.0, hovered), &label, font, dpi, ui_tokens().bg, FocusRing::Inside);
            return LRESULT(1);
        }
        WM_COMMAND => {
            // BN_CLICKED is 0. Focus notifications share WM_COMMAND and must not accept.
            if (wp.0 >> 16) & 0xffff != 0 {
                return LRESULT(0);
            }
            let id = (wp.0 & 0xffff) as i32;
            if id == ID_AFFIRM {
                finish(true);
                return LRESULT(0);
            }
            if id == ID_CANCEL {
                finish(false);
                return LRESULT(0);
            }
        }
        WM_CLOSE => {
            finish(false);
            return LRESULT(0);
        }
        WM_DESTROY => {
            if DONE.with(|d| d.get().is_none()) {
                DONE.with(|d| d.set(Some(false)));
            }
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let r = &*(lp.0 as *const RECT);
            swap_fonts(GetDpiForWindow(h).max(96) as i32);
            layout_now(h, Place::At(r.left, r.top));
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

unsafe fn focused_id(h: HWND) -> i32 {
    let focus = GetFocus();
    if focus == ctl(h, ID_CANCEL) {
        ID_CANCEL
    } else if focus == ctl(h, ID_AFFIRM) {
        ID_AFFIRM
    } else if DEFAULT_CANCEL.with(|c| c.get()) {
        ID_CANCEL
    } else {
        ID_AFFIRM
    }
}

/// Blocks until the user accepts or cancels. `true` is the affirmative button.
/// A failure to open the window cancels.
pub(crate) unsafe fn ask_confirm(owner: HWND, ask: Confirm) -> bool {
    ensure_class();
    let _guard = Guard::arm(owner);
    DONE.with(|d| d.set(None));
    DEFAULT_CANCEL.with(|c| c.set(ask.default_cancel));
    let title = if ask.title.trim().is_empty() { "Maguro".to_string() } else { ask.title };
    let detail = ask.detail.trim().to_string();
    let cancel = cancel_label();
    let affirm = affirm_label(ask.tone, &ask.affirm);
    let kind = match ask.tone {
        ConfirmTone::Primary => ButtonKind::Primary,
        ConfirmTone::Danger => ButtonKind::Danger,
    };
    let dpi0 = if owner.0.is_null() { 96 } else { GetDpiForWindow(owner).max(96) as i32 };
    LIVE.with(|s| {
        *s.borrow_mut() = Some(Live {
            hwnd: HWND::default(),
            message: ask.message,
            detail,
            cancel,
            affirm,
            kind,
            fonts: make_fonts(dpi0),
            msg_h: 0,
            detail_h: 0,
        });
    });
    let parent = (!owner.0.is_null()).then_some(owner);
    // Open on the owner's monitor so the first DPI sample matches the final window.
    let (x0, y0) = if owner.0.is_null() {
        (0, 0)
    } else {
        let mut orc = RECT::default();
        let _ = GetWindowRect(owner, &mut orc);
        (orc.left, orc.top)
    };
    // Hidden until the caption colours are set, or the first frame stays light.
    let h = CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        w!("MaguroConfirm"),
        &HSTRING::from(title),
        confirm_style(),
        x0,
        y0,
        px(dpi0, CONFIRM_W_DIP),
        px(dpi0, 160),
        parent,
        None,
        None,
        None,
    )
    .unwrap_or_default();
    if h.0.is_null() {
        return false;
    }
    LIVE.with(|s| {
        if let Some(live) = s.borrow_mut().as_mut() {
            live.hwnd = h;
        }
    });
    style_caption(h);
    crate::apply_window_icon(h);
    let dpi = GetDpiForWindow(h).max(96) as i32;
    if dpi != dpi0 {
        swap_fonts(dpi);
    }
    layout_now(h, Place::Center(owner));
    let (cancel, affirm, font) = LIVE.with(|s| {
        let live = s.borrow();
        let Some(live) = live.as_ref() else { return (String::new(), String::new(), HFONT::default()) };
        (live.cancel.clone(), live.affirm.clone(), live.fonts.body)
    });
    let tab = 0x0001_0000u32;
    let cancel_h = HSTRING::from(&cancel);
    let affirm_h = HSTRING::from(&affirm);
    let _ = crate::child(h, w!("BUTTON"), PCWSTR(cancel_h.as_ptr()), BS_OWNERDRAW as u32 | tab, WINDOW_EX_STYLE(0), ID_CANCEL, font);
    let _ = crate::child(h, w!("BUTTON"), PCWSTR(affirm_h.as_ptr()), BS_OWNERDRAW as u32 | tab, WINDOW_EX_STYLE(0), ID_AFFIRM, font);
    layout_now(h, Place::Center(owner));
    let _ = ShowWindow(h, SW_SHOW);
    let _ = SetForegroundWindow(h);
    let focus_id = if ask.default_cancel { ID_CANCEL } else { ID_AFFIRM };
    let _ = SetFocus(Some(ctl(h, focus_id)));
    let mut msg = MSG::default();
    while DONE.with(|d| d.get().is_none()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            finish(false);
            PostQuitMessage(0);
            break;
        }
        let mine = msg.hwnd == h || IsChild(h, msg.hwnd).as_bool();
        if msg.message == WM_MOUSEMOVE {
            crate::set_hover(if crate::is_icon_button(msg.hwnd) { msg.hwnd } else { HWND::default() });
        } else if msg.message == WM_MOUSELEAVE && crate::app(|a| a.hover) == msg.hwnd {
            crate::set_hover(HWND::default());
        }
        if msg.message == WM_KEYDOWN && mine {
            let vk = msg.wParam.0 as u16;
            if vk == VK_ESCAPE.0 {
                finish(false);
                continue;
            }
            if vk == VK_RETURN.0 {
                let id = focused_id(h);
                let _ = SendMessageW(h, WM_COMMAND, Some(WPARAM(id as usize)), Some(LPARAM(0)));
                continue;
            }
            if vk == VK_TAB.0 {
                // Two buttons: Tab and Shift+Tab both move to the other one.
                let cur = GetFocus();
                let next = if cur == ctl(h, ID_CANCEL) { ctl(h, ID_AFFIRM) } else { ctl(h, ID_CANCEL) };
                let _ = SetFocus(Some(next));
                continue;
            }
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    DONE.with(|d| d.get().unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{button_colors, ButtonState, ACCENT, BG_DARK, BG_LIGHT, CARD_LIGHT, DANGER_HOVER_FILL, DANGER_LIGHT};

    #[test]
    fn layout_matches_the_confirm_mockup_at_96_dpi() {
        let m = ConfirmMetrics { msg_h: 18, detail_h: 15, cancel_w: 72, affirm_w: 72 };
        assert_eq!(confirm_client_w(96, &m), 360);
        let lay = confirm_layout(360, 96, m);
        assert_eq!((lay.message.left, lay.message.top, lay.message.right, lay.message.bottom), (16, 14, 344, 32));
        assert_eq!((lay.detail.left, lay.detail.top, lay.detail.bottom), (16, 48, 63));
        assert_eq!((lay.rule.top, lay.rule.bottom, lay.rule.left, lay.rule.right), (75, 76, 16, 344));
        assert_eq!((lay.affirm.left, lay.affirm.top, lay.affirm.right, lay.affirm.bottom), (272, 88, 344, 116));
        assert_eq!((lay.cancel.left, lay.cancel.right, lay.cancel.top), (192, 264, 88));
        assert_eq!(lay.affirm.left - lay.cancel.right, 8);
        assert_eq!(lay.cancel.bottom - lay.cancel.top, 28);
        assert_eq!(lay.client_h, 130);
        assert_eq!(lay.client_h - lay.affirm.bottom, 14);
    }

    #[test]
    fn a_missing_detail_drops_its_gap_and_wide_labels_widen_the_window() {
        let plain = ConfirmMetrics { msg_h: 18, detail_h: 0, cancel_w: 72, affirm_w: 72 };
        let lay = confirm_layout(360, 96, plain);
        assert_eq!(lay.detail.right, lay.detail.left);
        assert_eq!(lay.rule.top, lay.message.bottom + 12);
        assert_eq!(lay.client_h, 99);

        let wide = ConfirmMetrics { msg_h: 20, detail_h: 11, cancel_w: 200, affirm_w: 180 };
        assert_eq!(confirm_client_w(96, &wide), 16 + 200 + 8 + 180 + 16);
        let lay = confirm_layout(confirm_client_w(96, &wide), 96, wide);
        assert_eq!(lay.cancel.right, lay.affirm.left - 8);
        assert!(lay.cancel.left >= 16);
        assert_eq!(lay.affirm.right, lay.rule.right);
        assert_eq!(lay.detail.top, lay.message.bottom + 16);
    }

    #[test]
    fn footer_metrics_scale_with_dpi() {
        let m = ConfirmMetrics { msg_h: 36, detail_h: 0, cancel_w: 144, affirm_w: 144 };
        let lay = confirm_layout(720, 192, m);
        assert_eq!(lay.message.left, 32);
        assert_eq!(lay.affirm.bottom - lay.affirm.top, 56);
        assert_eq!(lay.affirm.right, 720 - 32);
        assert_eq!(lay.rule.bottom - lay.rule.top, 2);
        assert_eq!(lay.client_h - lay.affirm.bottom, 28);
    }

    #[test]
    fn buttons_use_the_shared_primary_danger_and_secondary_faces() {
        assert_eq!(button_kind_for(ID_AFFIRM, ButtonKind::Primary), ButtonKind::Primary);
        assert_eq!(button_kind_for(ID_AFFIRM, ButtonKind::Danger), ButtonKind::Danger);
        assert_eq!(button_kind_for(ID_CANCEL, ButtonKind::Danger), ButtonKind::Secondary);

        let primary = button_colors(ButtonKind::Primary, ButtonState::Normal, false, BG_LIGHT);
        assert_eq!(primary.fill, Some(ACCENT));
        assert_ne!(primary.fill, Some(0x2EC4A6));
        let danger = button_colors(ButtonKind::Danger, ButtonState::Normal, false, BG_LIGHT);
        assert_eq!(danger.ink, DANGER_LIGHT);
        assert_eq!(danger.fill, Some(CARD_LIGHT));
        let danger_hot = button_colors(ButtonKind::Danger, ButtonState::Hover, true, BG_DARK);
        assert_eq!(danger_hot.fill, Some(DANGER_HOVER_FILL));
        let cancel = button_colors(ButtonKind::Secondary, ButtonState::Normal, false, BG_LIGHT);
        assert_eq!(cancel.fill, Some(CARD_LIGHT));
        assert_eq!(ui_tokens_bg(false), BG_LIGHT);
        assert_eq!(ui_tokens_bg(true), BG_DARK);
    }

    fn ui_tokens_bg(dark: bool) -> u32 {
        super::super::tokens::tokens_for(dark).bg
    }

    #[test]
    fn default_labels_are_cancel_with_ok_or_delete() {
        assert_eq!(cancel_label(), "Cancel");
        assert_eq!(affirm_label(ConfirmTone::Primary, ""), "OK");
        assert_eq!(affirm_label(ConfirmTone::Danger, ""), "Delete");
        assert_eq!(affirm_label(ConfirmTone::Danger, " Close "), "Close");
        assert_eq!(affirm_label(ConfirmTone::Primary, "Reset"), "Reset");
    }
}
