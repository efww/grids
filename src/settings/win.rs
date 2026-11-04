use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Once;
use windows::core::{w, BOOL, HSTRING, PCWSTR};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWINDOWATTRIBUTE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{SetWindowTheme, DRAWITEMSTRUCT, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, GetFocus, IsWindowEnabled, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::*;
use crate::tf;

/// Posted to the settings window. Handled only after the menu loop has exited.
const WM_REVEAL: u32 = WM_APP + 87;
static MENU_DEPTH: AtomicI32 = AtomicI32::new(0);
static REVEAL: AtomicBool = AtomicBool::new(false);

pub(crate) fn menu_enter() {
    MENU_DEPTH.fetch_add(1, Ordering::Relaxed);
}

/// The menu loop is done. A settings window opened from that menu can show now.
pub(crate) unsafe fn menu_exit() {
    let prev = MENU_DEPTH.fetch_sub(1, Ordering::Relaxed);
    if prev <= 1 {
        MENU_DEPTH.store(0, Ordering::Relaxed);
        poke_reveal();
    }
}

fn in_menu() -> bool {
    MENU_DEPTH.load(Ordering::Relaxed) > 0
}

fn poke_reveal() {
    if !REVEAL.load(Ordering::Relaxed) {
        return;
    }
    let h = hwnd();
    if h.0.is_null() {
        return;
    }
    unsafe {
        let _ = PostMessageW(Some(h), WM_REVEAL, WPARAM(0), LPARAM(0));
    }
}

fn schedule_reveal() {
    REVEAL.store(true, Ordering::Relaxed);
    // Inside the menu command the restore of the menu bitmap still lies ahead.
    // Showing now paints, then that restore puts white back and the region is
    // already valid, so the body stays white until a hover. `reveal` waits
    // while a menu is open; `menu_exit` posts again once it has closed.
    poke_reveal();
}

unsafe fn reveal(h: HWND) {
    if in_menu() {
        return;
    }
    if !REVEAL.swap(false, Ordering::Relaxed) {
        return;
    }
    style_window(h);
    let _ = ShowWindow(h, SW_SHOWNORMAL);
    let _ = SetForegroundWindow(h);
    let _ = InvalidateRect(Some(h), None, true);
    let _ = UpdateWindow(h);
}

pub unsafe fn open(owner: HWND) {
    ensure_class();
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        crate::winmerge::forget_status();
        crate::folderapp::reload_shown();
        schedule_reveal();
        return;
    }
    let dpi = GetDpiForWindow(owner) as i32;
    let (ww, hh) = (sc(dpi.max(96), 1000), sc(dpi.max(96), 640));
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    // Shown from `reveal`, after the dark caption is set. WS_VISIBLE here is the
    // white first frame: the menu command returns and restores over the client.
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    let _ = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroSettings"), w!("Maguro"), style, x, y, ww, hh, Some(owner), None, None, None);
    schedule_reveal();
}

pub unsafe fn close() {
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = DestroyWindow(h);
    }
}

pub unsafe fn captures(h: HWND) -> bool {
    let w = hwnd();
    !w.0.is_null() && IsWindow(Some(w)).as_bool() && (h == w || IsChild(w, h).as_bool())
}

pub unsafe fn redraw() {
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = InvalidateRect(Some(h), None, false);
        crate::ui::combo::invalidate_in(h);
    }
}

pub unsafe fn refresh() {
    crate::folderapp::reload_shown();
    let h = hwnd();
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    style_window(h);
    rebuild_fonts();
    let font = ST.with(|s| s.borrow().font);
    each_child(h, |c| {
        SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        theme_ctl(c);
    });
    fill_strings(h);
    let _ = SetWindowTextW(h, &HSTRING::from(tx(TITLE)));
    cue(h, S_SEARCH, tx(FIND));
    cue(h, S_KEYQ, tx(crate::Key::SearchActionsOrKeys));
    cue(h, S_WM, tx(crate::Key::AutoDetect));
    layout(h);
    let _ = InvalidateRect(Some(h), None, true);
    crate::ui::combo::invalidate_in(h);
}

/// The right-click page is open and a menu just learned new item names.
pub(crate) unsafe fn refresh_ctx_labels() {
    let h = hwnd();
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    if ST.with(|s| s.borrow().page) != CTX_PAGE {
        return;
    }
    layout(h);
    let _ = InvalidateRect(Some(h), None, false);
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(settings_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(crate::ICON_GROUP_LIGHT as _)).unwrap_or_default(),
            lpszClassName: w!("MaguroSettings"),
            ..Default::default()
        });
    });
}

unsafe fn style_window(h: HWND) {
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = crate::cref(crate::pal().bg);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
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

fn rebuild_fonts() {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let custom = app(|a| a.s.font_face.clone());
    let fallback = crate::ui_face();
    let face = if custom.is_empty() { fallback } else { custom.as_str() };
    // A chosen face is the page title at weight 700. The language default keeps its own title face.
    let (title_face, title_w) = if custom.is_empty() {
        crate::title_style()
    } else {
        (face, 700)
    };
    let new = (
        make_font(face, 14, 400, dpi),
        make_font(face, 14, 700, dpi),
        make_font(face, 12, 400, dpi),
        make_font(face, 16, 700, dpi),
        make_font(title_face, 24, title_w, dpi),
        make_font("Consolas", 12, 400, dpi),
        make_font("Segoe MDL2 Assets", 14, 400, dpi),
    );
    let old = ST.with(|s| {
        let mut st = s.borrow_mut();
        let old = (st.font, st.btn_bold, st.small, st.bold, st.title_font, st.mono, st.icon);
        (st.font, st.btn_bold, st.small, st.bold, st.title_font, st.mono, st.icon) = new;
        old
    });
    unsafe {
        for f in [old.0, old.1, old.2, old.3, old.4, old.5, old.6] {
            crate::delete_font(f);
        }
    }
}

unsafe fn theme_ctl(h: HWND) {
    if crate::ui::combo::is_styled(h) {
        crate::ui::combo::clear_theme(h);
        return;
    }
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let name = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

fn child_at(parent: HWND, class: PCWSTR, text: &str, style: u32, id: i32, font: HFONT) -> HWND {
    let t = HSTRING::from(text);
    unsafe { crate::child(parent, class, PCWSTR(t.as_ptr()), style, WINDOW_EX_STYLE(0), id, font) }
}

unsafe fn build_children(h: HWND) {
    let font = ST.with(|s| s.borrow().font);
    let tab = 0x00010000u32;
    let dis = 0x08000000u32;
    child_at(h, w!("EDIT"), "", 0x80 | tab, S_SEARCH, font);
    child_at(h, w!("EDIT"), "", 0x80 | tab, S_KEYQ, font);
    let combo = |id, on| {
        let c = child_at(h, w!("COMBOBOX"), "", crate::ui::combo::LIST | tab | if on { 0 } else { dis }, id, font);
        crate::ui::combo::adopt(c, crate::ui::combo::card_face);
        c
    };
    combo(S_LANG, true);
    combo(S_FACE, true);
    combo(S_THEME, true);
    for id in [S_START, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_LAYOUT, S_SIDE] {
        combo(id, true);
    }
    let btn = |id, label: &str, on| child_at(h, w!("BUTTON"), label, 0xB | tab | if on { 0 } else { dis }, id, font);
    btn(S_CLOSE, tx(CLOSE_L), true);
    btn(S_RESET, tx(crate::Key::ResetAllDefaults), true);
    btn(S_SESS, tx(crate::Key::OpenFolder), true);
    btn(S_ACCENT, tx(crate::Key::Custom2), true);
    btn(S_ADV_OPEN, tx(crate::Key::OpenFolder), true);
    btn(S_ADV_EX, tx(crate::Key::Export), true);
    btn(S_ADV_IM, tx(crate::Key::Import), true);
    btn(S_ADV_RESET, tx(crate::Key::Reset), true);
    btn(S_EV, ev_button_label(), true);
    btn(S_FV_DEF, tx(crate::Key::SetFromThisPane), true);
    btn(S_FV_RESET, tx(crate::Key::Reset2), true);
    btn(S_FV_CLEAR, tx(crate::Key::ClearAll), true);
    btn(S_CTX_RESET, tx(CTX_RESET_L), true);
    btn(S_CTX_REFRESH, tx(CTX_REFRESH_L), true);
    child_at(h, w!("EDIT"), "", 0x80 | tab, S_WM, font);
    btn(S_WM_BROWSE, tx(crate::Key::Browse), true);
}

fn each_child(h: HWND, mut f: impl FnMut(HWND)) {
    let ids = [S_SEARCH, S_CLOSE, S_KEYQ, S_RESET, S_LANG, S_FACE, S_START, S_THEME, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_LAYOUT, S_SIDE, S_SESS, S_ACCENT, S_ADV_OPEN, S_ADV_EX, S_ADV_IM, S_ADV_RESET, S_EV, S_FV_DEF, S_FV_RESET, S_FV_CLEAR, S_CTX_RESET, S_CTX_REFRESH, S_WM, S_WM_BROWSE];
    for id in ids {
        let c = unsafe { GetDlgItem(Some(h), id).unwrap_or_default() };
        if !c.0.is_null() {
            f(c);
        }
    }
    let made = FAV_MADE.load(std::sync::atomic::Ordering::Relaxed);
    for base in [S_FAV0, S_REN0] {
        for i in 0..made {
            let c = unsafe { GetDlgItem(Some(h), base + i).unwrap_or_default() };
            if !c.0.is_null() {
                f(c);
            }
        }
    }
}

unsafe fn fill_strings(h: HWND) {
    let options = crate::settings_languages();
    let lang = app(|a| a.s.lang);
    let lang_i = options.iter().position(|(l, _)| *l == lang).unwrap_or(0);
    let labels: Vec<String> = options.iter().map(|(_, name)| name.clone()).collect();
    fill_combo(h, S_LANG, &labels, lang_i);
    fill_face(h);
    let theme = app(|a| a.s.theme) as usize;
    fill_combo(h, S_THEME, &[tx(crate::Key::FollowWindows).into(), tx(crate::Key::Light).into(), tx(crate::Key::Dark).into()], theme.min(2));
    let boot = app(|a| a.s.boot) as usize;
    fill_combo(h, S_START, &[tx(crate::Key::RestoreLastState).into(), tx(crate::Key::OpenSession3).into(), tx(crate::Key::DefaultLayout).into()], boot.min(2));
    let view_labels: Vec<String> = crate::EXPLORER_VIEWS.iter().map(|&(_, _, key)| tr(key).to_string()).collect();
    let view_i = app(|a| {
        let (mode, icon) = a.s.default_view();
        crate::explorer_view_index(mode, icon).unwrap_or(5)
    });
    fill_combo(h, S_VIEW, &view_labels, view_i);
    let start = app(|a| a.s.start_new) as usize;
    fill_combo(h, S_DIR, &[tx(crate::Key::ThisPc).into(), tx(crate::Key::Desktop).into(), tx(crate::Key::ChooseFolder).into()], start.min(2));
    let del = app(|a| a.s.del) as usize;
    fill_combo(h, S_DEL, &[tx(crate::Key::RecycleBin).into(), tx(crate::Key::DeletePermanently).into()], del.min(1));
    let custom = app(|a| a.s.new_name.clone());
    let new_label = if custom.is_empty() { tx(crate::Key::Custom2).to_string() } else { custom };
    let new_i = usize::from(!app(|a| a.s.new_name.is_empty()));
    fill_combo(h, S_NEW, &[tx(crate::Key::NewTextDocumentTxt).into(), new_label], new_i);
    let dropm = app(|a| a.s.dropm) as usize;
    fill_combo(h, S_DROP, &[tx(crate::Key::WindowsRulesSameDriveMove).into(), tx(crate::Key::AlwaysCopy).into(), tx(crate::Key::AlwaysAsk).into()], dropm.min(2));
    let layout_new = app(|a| a.s.layout_new) as usize;
    fill_combo(h, S_LAYOUT, &[tx(crate::Key::TwoPanesSideBySide).into(), tx(crate::Key::TwoPanesTopAndBottom).into(), tx(crate::Key::K22FourPanes).into(), tx(crate::Key::OnePane).into()], layout_new.min(3));
    let side_i = usize::from(!app(|a| a.s.fav_top));
    fill_combo(h, S_SIDE, &[tx(SIDE_FAV).into(), tx(SIDE_TREE).into()], side_i.min(1));
    let labels = [
        (S_CLOSE, tx(CLOSE_L)),
        (S_RESET, tx(crate::Key::ResetAllDefaults)),
        (S_SESS, tx(crate::Key::OpenFolder)),
        (S_ACCENT, tx(crate::Key::Custom2)),
        (S_ADV_OPEN, tx(crate::Key::OpenFolder)),
        (S_ADV_EX, tx(crate::Key::Export)),
        (S_ADV_IM, tx(crate::Key::Import)),
        (S_ADV_RESET, tx(crate::Key::Reset)),
        (S_EV, ev_button_label()),
        (S_FV_DEF, tx(crate::Key::SetFromThisPane)),
        (S_FV_RESET, tx(crate::Key::Reset2)),
        (S_FV_CLEAR, tx(crate::Key::ClearAll)),
        (S_CTX_RESET, tx(CTX_RESET_L)),
        (S_CTX_REFRESH, tx(CTX_REFRESH_L)),
        (S_WM_BROWSE, tx(crate::Key::Browse)),
    ];
    BUSY.with(|b| b.set(true));
    let _ = SetWindowTextW(ctl(h, S_WM), &HSTRING::from(app(|a| a.s.winmerge_path.clone())));
    BUSY.with(|b| b.set(false));
    for (id, label) in labels {
        let _ = SetWindowTextW(ctl(h, id), &HSTRING::from(label));
    }
}

fn ensure_faces() {
    if ST.with(|s| s.borrow().faces.is_some()) {
        return;
    }
    // The family list exists only while this window does. Startup never calls this.
    let list = unsafe { crate::installed_families() };
    ST.with(|s| s.borrow_mut().faces = Some(list));
}

unsafe fn fill_face(h: HWND) {
    ensure_faces();
    let def = format!("{} ({})", tx(crate::Key::Default), crate::ui_face());
    let stored = app(|a| a.s.font_face.clone());
    let mut items = vec![def];
    let mut sel = 0usize;
    ST.with(|s| {
        if let Some(faces) = s.borrow().faces.as_ref() {
            for (i, name) in faces.iter().enumerate() {
                if !stored.is_empty() && crate::same_face(name, &stored) {
                    sel = i + 1;
                }
                items.push(name.clone());
            }
        }
    });
    if sel == 0 && !stored.is_empty() {
        items.push(stored);
        sel = items.len() - 1;
    }
    fill_combo(h, S_FACE, &items, sel);
    let c = ctl(h, S_FACE);
    if !c.0.is_null() {
        let dpi = ST.with(|s| s.borrow().dpi.max(96));
        SendMessageW(c, CB_SETDROPPEDWIDTH, Some(WPARAM(sc(dpi, 320) as usize)), None);
    }
}

unsafe fn combo_lb_text(h: HWND, id: i32, sel: usize) -> String {
    let c = ctl(h, id);
    if c.0.is_null() {
        return String::new();
    }
    let n = SendMessageW(c, CB_GETLBTEXTLEN, Some(WPARAM(sel)), None).0;
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = SendMessageW(c, CB_GETLBTEXT, Some(WPARAM(sel)), Some(LPARAM(buf.as_mut_ptr() as isize))).0;
    if got <= 0 {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(got as usize).min(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

unsafe fn fill_combo(h: HWND, id: i32, items: &[String], sel: usize) {
    let c = ctl(h, id);
    if c.0.is_null() {
        return;
    }
    BUSY.with(|b| b.set(true));
    SendMessageW(c, CB_RESETCONTENT, None, None);
    for item in items {
        let hs = HSTRING::from(item.as_str());
        SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
    SendMessageW(c, CB_SETCURSEL, Some(WPARAM(sel)), None);
    SendMessageW(c, CB_SETMINVISIBLE, Some(WPARAM(8)), None);
    BUSY.with(|b| b.set(false));
}

unsafe fn cue(h: HWND, id: i32, text: &str) {
    let c = ctl(h, id);
    let hs = HSTRING::from(text);
    SendMessageW(c, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(hs.as_ptr() as isize)));
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let m = sc(dpi, 8) as isize;
    SendMessageW(c, 0x00D3, Some(WPARAM(3)), Some(LPARAM(m | (m << 16)))); // EM_SETMARGINS
}

fn ctl(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

fn layout(h: HWND) {
    let mut sn = snap();
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    let mut g = geo(rc, &sn);
    if sn.scroll > g.scroll_max {
        sn.scroll = g.scroll_max;
        ST.with(|s| s.borrow_mut().scroll = sn.scroll);
        g = geo(rc, &sn);
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.scroll_max = g.scroll_max;
        st.track = g.track;
        st.thumb_h = g.thumb.bottom - g.thumb.top;
    });
    unsafe { place(h, &g, &sn); }
}

unsafe fn place(h: HWND, g: &Geo, sn: &Snap) {
    let hide = |id: i32| {
        let c = ctl(h, id);
        if !c.0.is_null() {
            let _ = ShowWindow(c, SW_HIDE);
        }
    };
    for id in [S_KEYQ, S_RESET, S_LANG, S_FACE, S_START, S_THEME, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_LAYOUT, S_SIDE, S_SESS, S_ACCENT, S_ADV_OPEN, S_ADV_EX, S_ADV_IM, S_ADV_RESET, S_EV, S_FV_DEF, S_FV_RESET, S_FV_CLEAR, S_CTX_RESET, S_CTX_REFRESH, S_WM, S_WM_BROWSE] {
        hide(id);
    }
    let n = fav_tree_rows().len().min(FAV_MAX as usize) as i32;
    let made = FAV_MADE.load(std::sync::atomic::Ordering::Relaxed);
    for base in [S_FAV0, S_REN0] {
        for i in 0..made {
            let c = ctl(h, base + i);
            if c.0.is_null() {
                continue;
            }
            if i >= n {
                let _ = DestroyWindow(c);
            } else {
                let _ = ShowWindow(c, SW_HIDE);
            }
        }
    }
    crate::place_edit(ctl(h, S_SEARCH), inset(g.search, sc(sn.dpi, FIELD_INSET_DIP)), true);
    move_to(ctl(h, S_CLOSE), g.close, true);
    let content = g.content;
    for item in &g.items {
        match item {
            Item::Card(c) => {
                if let Some(id) = ctl_id(c.spec.ctl) {
                    move_to(ctl(h, id), c.ctl, intersects(c.ctl, content));
                }
                if c.spec.ctl == Ctl::Adv {
                    let ids = [S_ADV_OPEN, S_ADV_EX, S_ADV_IM];
                    for (i, id) in ids.iter().enumerate() {
                        move_to(ctl(h, *id), c.adv[i], intersects(c.adv[i], content));
                    }
                }
                if matches!(c.spec.ctl, Ctl::AdvReset | Ctl::FolderDefault | Ctl::FolderReset | Ctl::FolderClear) {
                    let id = match c.spec.ctl {
                        Ctl::FolderDefault => S_FV_DEF,
                        Ctl::FolderReset => S_FV_RESET,
                        Ctl::FolderClear => S_FV_CLEAR,
                        _ => S_ADV_RESET,
                    };
                    move_to(ctl(h, id), c.adv[0], intersects(c.adv[0], content));
                }
                if c.spec.ctl == Ctl::EvAct {
                    let b = ctl(h, S_EV);
                    let _ = SetWindowTextW(b, &HSTRING::from(ev_button_label()));
                    let _ = EnableWindow(b, !crate::everything::busy());
                    move_to(b, c.ctl, crate::everything::action().is_some() && intersects(c.ctl, content));
                }
                if c.spec.ctl == Ctl::WmPath {
                    move_to(ctl(h, S_WM_BROWSE), c.adv[0], intersects(c.adv[0], content));
                    crate::place_edit(ctl(h, S_WM), inset(c.ctl, sc(sn.dpi, FIELD_INSET_DIP)), intersects(c.ctl, content));
                }
            }
            Item::CtxBar { reset, refresh } => {
                move_to(ctl(h, S_CTX_REFRESH), *refresh, intersects(*refresh, content));
                move_to(ctl(h, S_CTX_RESET), *reset, intersects(*reset, content));
            }
            Item::KeyBar { edit, reset } => {
                crate::place_edit(ctl(h, S_KEYQ), inset(*edit, sc(sn.dpi, FIELD_INSET_DIP)), intersects(*edit, content));
                move_to(ctl(h, S_RESET), *reset, intersects(*reset, content));
            }
            Item::FavRow { i, rename, btn, .. } if *i != usize::MAX && *i < FAV_MAX as usize && btn.right > btn.left => {
                let id = S_FAV0 + *i as i32;
                FAV_MADE.fetch_max(*i as i32 + 1, std::sync::atomic::Ordering::Relaxed);
                let mut b = ctl(h, id);
                if b.0.is_null() {
                    b = child_at(h, w!("BUTTON"), tx(crate::Key::Delete), 0xB | 0x00010000, id, sn.font);
                }
                let _ = SetWindowTextW(b, &HSTRING::from(tx(crate::Key::Delete)));
                move_to(b, *btn, intersects(*btn, content));
                let rid = S_REN0 + *i as i32;
                let mut r = ctl(h, rid);
                if r.0.is_null() {
                    r = child_at(h, w!("BUTTON"), tx(crate::Key::Rename2), 0xB | 0x00010000, rid, sn.font);
                }
                let _ = SetWindowTextW(r, &HSTRING::from(tx(crate::Key::Rename2)));
                move_to(r, *rename, intersects(*rename, content));
            }
            _ => {}
        }
    }
}

fn intersects(a: RECT, b: RECT) -> bool {
    a.right > a.left && a.bottom > a.top && a.bottom > b.top && a.top < b.bottom && a.right > b.left && a.left < b.right
}

fn inset(r: RECT, d: i32) -> RECT {
    RECT { left: r.left + d, top: r.top + d, right: (r.right - d).max(r.left + d), bottom: (r.bottom - d).max(r.top + d) }
}

unsafe fn move_to(h: HWND, r: RECT, show: bool) {
    if h.0.is_null() {
        return;
    }
    if !show {
        let _ = ShowWindow(h, SW_HIDE);
        return;
    }
    let _ = SetWindowPos(h, None, r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1), SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

unsafe fn paint(h: HWND) {
    let sn = snap();
    // Layout reads app state. Do it before BeginPaint: a nested borrow there used
    // to unwind out of the paint, which validated the white client until a hover.
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let g = geo(rc, &sn);
    crate::buffered_solid(h, |dc, rc| {
        draw(dc, rc, &g, &sn);
    });
}

unsafe fn draw(dc: HDC, rc: RECT, g: &Geo, sn: &Snap) {
    let dpi = sn.dpi.max(96);
    // Ground first. A later borrow must not leave the buffer transparent.
    crate::fill(dc, rc, crate::pal().bg);
    let accent = chrome();
    crate::fill(dc, RECT { bottom: g.footer.top + 1, top: g.footer.top, ..g.footer }, crate::pal().line);
    draw_field(dc, g.search, dpi, crate::pal().bg);
    if !g.show.iter().any(|s| *s) {
        crate::text(dc, RECT { left: sc(dpi, 16), top: g.search.bottom + sc(dpi, 12), right: g.nav_w - sc(dpi, 8), bottom: g.search.bottom + sc(dpi, 40) }, tx(NONE), sn.font, crate::pal().sub, DT_LEFT);
    }
    for i in 0..PAGE_N {
        if !g.show[i] {
            continue;
        }
        let r = g.nav[i];
        let on = i == sn.page;
        if on || sn.hot == Some(i) {
            let t = if on { 0.08 } else { 0.04 };
            crate::aa_rfill(dc, r, sc(dpi, 4), crate::blend(crate::pal().bg, crate::pal().text, t), None, crate::pal().bg);
        }
        if on {
            let bar = RECT { left: r.left, top: r.top + sc(dpi, 10), right: r.left + sc(dpi, 3), bottom: r.bottom - sc(dpi, 10) };
            crate::aa_rfill(dc, bar, sc(dpi, 2), accent, None, crate::pal().bg);
        }
        crate::text(dc, RECT { left: r.left + sc(dpi, 14), ..r }, tx(PAGES[i]), sn.font, crate::pal().text, DT_LEFT);
    }
    crate::text(dc, g.hint, tx(FOOT), sn.small, crate::pal().sub, DT_LEFT);
    let _ = SaveDC(dc);
    let _ = IntersectClipRect(dc, g.content.left, g.content.top, g.content.right, g.content.bottom);
    // A miss keeps the user's page but must not paint it. Shortcuts would otherwise
    // show its title and the empty key table while the nav says nothing matched.
    if search_miss(&sn.query) {
        let r = RECT {
            left: g.content.left + sc(dpi, 28),
            top: g.content.top + sc(dpi, 24),
            right: g.content.right - sc(dpi, 28),
            bottom: g.content.top + sc(dpi, 72),
        };
        crate::text(dc, r, tx(NO_RESULTS), sn.title_font, crate::pal().text, DT_LEFT);
        let _ = RestoreDC(dc, -1);
        return;
    }
    let title = tx(PAGES[sn.page]);
    crate::text(dc, g.title, title, sn.title_font, crate::pal().text, DT_LEFT);
    let fav_rows = if sn.page == 4 { fav_tree_rows() } else { Vec::new() };
    for item in &g.items {
        match item {
            Item::Card(c) => draw_card(dc, c, sn, accent),
            Item::Head(r, t) => crate::text(dc, *r, tx(*t), sn.font, crate::pal().text, DT_LEFT),
            Item::Table(r) => crate::aa_rfill(dc, *r, sc(dpi, 6), crate::pal().card, Some(crate::pal().line), crate::pal().bg),
            Item::FavHead(r) => crate::paint_fav_head(dc, *r, dpi, sn.small),
            Item::FavRow { i, rect, .. } => {
                let empty = *i == usize::MAX;
                let row = if empty { None } else { fav_rows.get(*i) };
                draw_fav_row(dc, row, *rect, sn, empty);
            }
            Item::Note(r, t) => crate::text(dc, *r, tx(*t), sn.small, crate::pal().sub, DT_LEFT),
            Item::Alert(r) => {
                if let Some(note) = crate::cliphot::registration_notice() {
                    crate::text(dc, *r, &note, sn.small, crate::pal().text, DT_LEFT);
                }
            }
            Item::CtxNote(r) => {
                let _ = crate::wrap_text(dc, *r, tx(ctx_note_key()), sn.small, crate::pal().sub, false);
            }
            Item::CtxSeg { classic, win11 } => {
                let on_win = app(|a| a.s.ctx_win11_view);
                draw_seg(dc, *classic, tx(CTX_CLASSIC), !on_win, sn, accent);
                draw_seg(dc, *win11, tx(CTX_WIN11), on_win, sn, accent);
            }
            Item::CtxBar { .. } => {}
            Item::CtxRow { id, rect, mark, chev, name, detail, raw, tick, chips, packaged, .. } => {
                let open = !*packaged && sn.ctx_open.iter().any(|x| x == id);
                draw_ctx_row(dc, *rect, *mark, *chev, name, detail, raw, *tick, chips, open, *packaged, sn, accent);
            }
            Item::CtxSub { rect, mark, bit, tick, .. } => draw_ctx_sub(dc, *rect, *mark, *bit, *tick, sn, accent),
            Item::CtxEmpty(r, key) => crate::text(dc, *r, tx(*key), sn.small, crate::pal().sub, DT_LEFT),
            Item::KeyBar { edit, .. } => draw_field(dc, *edit, dpi, crate::pal().bg),
            Item::KeyHead(r) => {
                let (a, b) = key_cols(*r);
                crate::text(dc, a, tx(crate::Key::Category), sn.small, crate::pal().sub, DT_LEFT);
                crate::text(dc, RECT { left: a.right, right: b, ..*r }, tx(crate::Key::Action), sn.small, crate::pal().sub, DT_LEFT);
                crate::text(dc, RECT { left: b, ..*r }, tx(crate::Key::Shortcut), sn.small, crate::pal().sub, DT_LEFT);
                crate::fill(dc, RECT { top: r.bottom - 1, ..*r }, crate::pal().line);
            }
            Item::KeyRow { i, rect, change, reset } => draw_key_row(dc, *i, *rect, *change, *reset, sn),
        }
    }
    if g.has_bar {
        let track = crate::blend(crate::pal().bg, crate::pal().text, 0.08);
        crate::aa_rfill(dc, g.track, sc(dpi, 4), track, None, crate::pal().bg);
        crate::aa_rfill(dc, g.thumb, sc(dpi, 4), crate::blend(crate::pal().bg, crate::pal().text, 0.35), None, track);
    }
    let _ = RestoreDC(dc, -1);
}

fn key_cols(r: RECT) -> (RECT, i32) {
    let w = r.right - r.left;
    let a = RECT { left: r.left + (w * 2 / 100).max(8), right: r.left + w * 16 / 100, ..r };
    (a, r.left + w * 40 / 100)
}

fn chip_h(dpi: i32) -> i32 {
    sc(dpi, 24)
}

fn chord_gap(dpi: i32) -> i32 {
    sc(dpi, 6)
}

/// Change and Reset, sitting at the right of a shortcut row.
pub(crate) fn key_btn_rects(r: RECT, dpi: i32) -> (RECT, RECT) {
    let h = sc(dpi, 24).min((r.bottom - r.top - sc(dpi, 4)).max(sc(dpi, 18)));
    let top = r.top + ((r.bottom - r.top - h) / 2).max(0);
    let gap = sc(dpi, 6);
    let edge = r.right - sc(dpi, 8);
    let reset = RECT { left: edge - sc(dpi, 72), top, right: edge, bottom: top + h };
    let change = RECT { left: reset.left - gap - sc(dpi, 76), top, right: reset.left - gap, bottom: top + h };
    (change, reset)
}

/// Shortcut column: x where chips start, and the width they must stay inside.
fn chord_box(r: RECT, dpi: i32) -> (i32, i32) {
    let (_, keys_x) = key_cols(r);
    let (change, _) = key_btn_rects(r, dpi);
    (keys_x, (change.left - sc(dpi, 8) - keys_x).max(sc(dpi, 36)))
}

/// Pack chip widths onto lines that stay within `max_w`. A chip wider than the
/// cell is clamped so the row cannot paint past the table's right edge.
fn pack_chords(widths: &[i32], max_w: i32, gap: i32) -> Vec<Vec<i32>> {
    let max_w = max_w.max(1);
    let mut lines: Vec<Vec<i32>> = Vec::new();
    let mut line: Vec<i32> = Vec::new();
    let mut used = 0i32;
    for &raw in widths {
        let w = raw.clamp(1, max_w);
        if !line.is_empty() && used.saturating_add(gap).saturating_add(w) > max_w {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push(w);
        used = if line.len() == 1 { w } else { used.saturating_add(gap).saturating_add(w) };
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn key_row_h_from_lines(n: i32, dpi: i32) -> i32 {
    let n = n.max(1);
    let min_h = sc(dpi, 40);
    if n == 1 {
        return min_h;
    }
    let pad = sc(dpi, 8);
    let gap = sc(dpi, 4);
    (pad * 2 + n * chip_h(dpi) + (n - 1) * gap).max(min_h)
}

unsafe fn chord_widths(dc: HDC, font: HFONT, keys: &[&str], dpi: i32) -> Vec<i32> {
    let pad = sc(dpi, 14);
    keys.iter()
        .map(|k| {
            let tw = if font.is_invalid() || dc.is_invalid() {
                sc(dpi, 8) * k.chars().count() as i32
            } else {
                text_w(dc, font, k)
            };
            (tw + pad).max(pad)
        })
        .collect()
}

unsafe fn key_row_height(dc: HDC, keys: &[&str], row: RECT, dpi: i32, font: HFONT) -> i32 {
    let (_, max_w) = chord_box(row, dpi);
    let lines = pack_chords(&chord_widths(dc, font, keys, dpi), max_w, chord_gap(dpi));
    key_row_h_from_lines(lines.len() as i32, dpi)
}

pub(crate) unsafe fn key_row_heights(labels: &[String], row: RECT, dpi: i32, font: HFONT) -> Vec<i32> {
    if labels.is_empty() {
        return vec![sc(dpi, 40)];
    }
    let dc = GetDC(None);
    let font = if dc.is_invalid() { HFONT::default() } else { font };
    let heights = labels.iter().map(|label| key_row_height(dc, &[label.as_str()], row, dpi, font)).collect();
    if !dc.is_invalid() {
        let _ = ReleaseDC(None, dc);
    }
    heights
}

unsafe fn draw_card(dc: HDC, c: &CardItem, sn: &Snap, accent: u32) {
    let dpi = sn.dpi.max(96);
    // Disabled rows stay on a white (or dark-theme card) ground; only the label grays.
    crate::aa_rfill(dc, c.rect, sc(dpi, 6), crate::pal().card, Some(crate::pal().line), crate::pal().bg);
    if c.spec.ctl == Ctl::WmPath {
        let title_r = RECT {
            left: c.rect.left + sc(dpi, 16),
            top: c.rect.top + sc(dpi, 12),
            right: c.adv[0].left - sc(dpi, 12),
            bottom: c.adv[0].bottom,
        };
        crate::text(dc, title_r, tx(c.spec.title), sn.font, crate::pal().text, DT_LEFT);
        let desc_r = RECT {
            left: c.rect.left + sc(dpi, 16),
            top: c.adv[0].bottom + sc(dpi, 2),
            right: c.rect.right - sc(dpi, 16),
            bottom: c.ctl.top - sc(dpi, 4),
        };
        crate::text(dc, desc_r, tx(c.spec.desc), sn.small, crate::pal().sub, DT_LEFT);
        draw_field(dc, c.ctl, dpi, crate::pal().card);
        return;
    }
    if c.spec.ctl == Ctl::About {
        let copy = live_about();
        let text_r = RECT {
            left: c.rect.left + sc(dpi, 16),
            top: c.rect.top + sc(dpi, 12),
            right: c.rect.right - sc(dpi, 16),
            bottom: c.rect.bottom - sc(dpi, 12),
        };
        let line = sc(dpi, 18);
        let title_line = RECT { bottom: text_r.top + sc(dpi, 20), ..text_r };
        crate::text(dc, title_line, &format!("{} v{}", copy.title, copy.version), sn.font, crate::pal().text, DT_LEFT);
        let mut y = title_line.bottom;
        for (s, color) in [
            (copy.desc.as_str(), crate::pal().sub),
            (copy.license.as_str(), crate::pal().sub),
            (copy.repo.as_str(), crate::pal().sub),
        ] {
            let r = RECT { top: y, bottom: (y + line).min(text_r.bottom), ..text_r };
            if r.bottom > r.top {
                crate::text(dc, r, s, sn.small, color, DT_LEFT);
            }
            y = r.bottom;
        }
        return;
    }
    let color = if c.spec.live { crate::pal().text } else { crate::pal().sub };
    let title = tx(c.spec.title).to_string();
    let text_r = RECT { left: c.rect.left + sc(dpi, 16), top: c.rect.top + sc(dpi, 12), right: c.ctl.left.min(c.sw.first().map(|s| s.left).unwrap_or(i32::MAX)) - sc(dpi, 12), bottom: c.rect.bottom - sc(dpi, 12) };
    let title_line = if tx(c.spec.desc).is_empty() { text_r } else { RECT { bottom: text_r.top + sc(dpi, 20), ..text_r } };
    crate::text(dc, title_line, &title, sn.font, color, DT_LEFT);
    if c.spec.ctl == Ctl::EvStatus || c.spec.ctl == Ctl::WmStatus {
        let line = sc(dpi, 18);
        let desc_r = RECT { top: title_line.bottom, bottom: title_line.bottom + line, ..text_r };
        if c.spec.ctl == Ctl::WmStatus {
            let found = crate::winmerge::status_now(&app(|a| a.s.winmerge_path.clone()));
            let (head, detail) = crate::winmerge::status_text(&found);
            crate::text(dc, desc_r, &head, sn.small, crate::pal().text, DT_LEFT);
            let more = RECT { top: desc_r.bottom, bottom: (desc_r.bottom + line).min(text_r.bottom), ..text_r };
            crate::text(dc, more, &detail, sn.small, crate::pal().sub, DT_LEFT);
        } else {
            crate::text(dc, desc_r, &ev_status_line(), sn.small, crate::pal().text, DT_LEFT);
        }
    } else if c.spec.ctl == Ctl::EvAct {
        let line = sc(dpi, 18);
        let desc_r = RECT { top: title_line.bottom, bottom: (title_line.bottom + line).min(text_r.bottom), ..text_r };
        let act = ev_activity_line();
        let text = if !act.is_empty() {
            act
        } else if matches!(crate::everything::shown(), crate::everything::Shown::Ready(crate::everything::Presence::Running { .. })) {
            tx(crate::Key::EverythingIsAlreadyRunning).to_string()
        } else {
            tx(c.spec.desc).to_string()
        };
        crate::text(dc, desc_r, &text, sn.small, crate::pal().sub, DT_LEFT);
    } else if c.spec.ctl == Ctl::Adv {
        let line = sc(dpi, 18);
        let desc_r = RECT { top: title_line.bottom, bottom: title_line.bottom + line, ..text_r };
        crate::text(dc, desc_r, tx(c.spec.desc), sn.small, crate::pal().sub, DT_LEFT);
        let path = crate::store::settings_path().to_string_lossy().into_owned();
        let path_r = RECT { left: c.rect.left + sc(dpi, 16), top: desc_r.bottom, right: c.rect.right - sc(dpi, 16), bottom: c.rect.bottom - sc(dpi, 6) };
        crate::text(dc, path_r, &path, sn.small, crate::pal().text, DT_LEFT);
    } else if c.spec.ctl == Ctl::Toggle(TG_CLIP) {
        let line = sc(dpi, 16);
        let desc_r = RECT { top: title_line.bottom, bottom: (title_line.bottom + line).min(text_r.bottom), ..text_r };
        crate::text(dc, desc_r, tx(c.spec.desc), sn.small, crate::pal().sub, DT_LEFT);
        if let Some(note) = crate::cliphot::registration_notice() {
            let more = RECT { top: desc_r.bottom, bottom: text_r.bottom, ..text_r };
            if more.bottom > more.top {
                crate::text(dc, more, &note, sn.small, crate::pal().text, DT_LEFT);
            }
        }
    } else if !tx(c.spec.desc).is_empty() {
        crate::text(dc, RECT { top: title_line.bottom, ..text_r }, tx(c.spec.desc), sn.small, crate::pal().sub, DT_LEFT);
    }
    match c.spec.ctl {
        Ctl::Toggle(id) => crate::draw_toggle(dc, c.ctl, toggle_on(id), c.spec.live, accent, dpi),
        Ctl::CopyEng => crate::text(dc, c.ctl, "IFileOperation", sn.small, crate::pal().sub, DT_RIGHT),
        Ctl::Accent => {
            let colors = crate::pal().accents;
            let cur = app(|a| a.s.accent_now());
            for (i, r) in c.sw.iter().enumerate() {
                let col = colors.get(i).map(|(_, c)| *c).unwrap_or(0);
                let ring = (col == cur).then(|| RECT { left: r.left - sc(dpi, 2), top: r.top - sc(dpi, 2), right: r.right + sc(dpi, 2), bottom: r.bottom + sc(dpi, 2) });
                let outer = ring.unwrap_or(*r);
                let painted = crate::aa_blit(dc, outer, crate::pal().card, |mem, scale, ox, oy| unsafe {
                    if let Some(ring) = ring {
                        crate::ellipse(mem, crate::scaled_from(outer, ring, scale, ox, oy), crate::pal().text, None);
                    }
                    crate::ellipse(mem, crate::scaled_from(outer, *r, scale, ox, oy), col, None);
                });
                if !painted {
                    if let Some(ring) = ring {
                        crate::ellipse(dc, ring, crate::pal().text, None);
                    }
                    crate::ellipse(dc, *r, col, None);
                }
            }
        }
        _ => {}
    }
}

unsafe fn draw_tick(dc: HDC, r: RECT, tick: crate::shell::ctxmenu::Tick, accent: u32, dpi: i32, icon: HFONT, ground: u32) {
    crate::draw_tick(dc, r, tick, accent, dpi, icon, ground, false);
}

unsafe fn draw_seg(dc: HDC, r: RECT, label: &str, on: bool, sn: &Snap, accent: u32) {
    let h = (r.bottom - r.top).max(1);
    let ground = crate::pal().bg;
    if on {
        crate::aa_rfill(dc, r, h / 2, accent, None, ground);
        // Same size as the resting label. Weight 700 so the ink stays readable on the fill.
        let face = if sn.btn_bold.is_invalid() { sn.font } else { sn.btn_bold };
        crate::text(dc, r, label, face, on_chrome(accent), DT_CENTER);
    } else {
        crate::aa_rfill(dc, r, h / 2, crate::pal().card, Some(crate::pal().line), ground);
        crate::text(dc, r, label, sn.font, crate::pal().text, DT_CENTER);
    }
}

unsafe fn draw_ctx_row(dc: HDC, rect: RECT, mark: RECT, chev: RECT, name: &str, detail: &str, raw: &str, tick: crate::shell::ctxmenu::Tick, chips: &[(u8, RECT)], open: bool, packaged: bool, sn: &Snap, accent: u32) {
    let dpi = sn.dpi.max(96);
    crate::aa_rfill(dc, rect, sc(dpi, 6), crate::pal().card, Some(crate::pal().line), crate::pal().bg);
    draw_tick(dc, mark, tick, accent, dpi, sn.icon, crate::pal().card);
    let text_left = if packaged {
        mark.right + sc(dpi, 8)
    } else {
        let chev_box = RECT { top: mark.top, bottom: mark.bottom, ..chev };
        crate::text(dc, chev_box, if open { "\u{E70D}" } else { "\u{E76C}" }, sn.icon, crate::pal().sub, DT_CENTER);
        chev.right
    };
    let name_right = chips.first().map(|(_, r)| r.left - sc(dpi, 8)).unwrap_or(rect.right - sc(dpi, 12));
    let text_r = RECT { left: text_left, right: name_right, ..rect };
    let line = detail;
    if line.is_empty() && raw.is_empty() {
        crate::text(dc, text_r, name, sn.bold, crate::pal().text, DT_LEFT);
    } else {
        let title = RECT { top: rect.top + sc(dpi, 6), bottom: rect.top + sc(dpi, 28), ..text_r };
        crate::text(dc, title, name, sn.bold, crate::pal().text, DT_LEFT);
        let sub = RECT { top: title.bottom, bottom: rect.bottom - sc(dpi, 4), ..text_r };
        let mut left = sub.left;
        if !line.is_empty() {
            let w = line_width(dc, line, sn.small);
            let shown = RECT { right: (left + w).min(sub.right), ..sub };
            crate::text(dc, shown, line, sn.small, crate::pal().sub, DT_LEFT);
            left = shown.right + sc(dpi, 10);
        }
        if !raw.is_empty() && left + sc(dpi, 24) < sub.right {
            crate::text(dc, RECT { left, ..sub }, raw, sn.small, crate::blend(crate::pal().sub, crate::pal().card, 0.45), DT_LEFT);
        }
    }
    for (bit, r) in chips {
        crate::aa_rfill(dc, *r, sc(dpi, 4), crate::blend(crate::pal().card, crate::pal().text, 0.06), Some(crate::pal().line), crate::pal().card);
        crate::text(dc, *r, crate::shell::ctxmenu::target_label(*bit), sn.small, crate::pal().sub, DT_CENTER);
    }
}

unsafe fn line_width(dc: HDC, s: &str, font: HFONT) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    let old = SelectObject(dc, font.into());
    DrawTextW(dc, &mut w, &mut r, DT_SINGLELINE | DT_CALCRECT | DT_NOPREFIX);
    SelectObject(dc, old);
    r.right - r.left
}

unsafe fn draw_ctx_sub(dc: HDC, rect: RECT, mark: RECT, bit: u8, tick: crate::shell::ctxmenu::Tick, sn: &Snap, accent: u32) {
    let dpi = sn.dpi.max(96);
    draw_tick(dc, mark, tick, accent, dpi, sn.icon, crate::pal().bg);
    crate::text(dc, RECT { left: mark.right + sc(dpi, 8), ..rect }, crate::shell::ctxmenu::target_label(bit), sn.font, crate::pal().text, DT_LEFT);
}

unsafe fn draw_fav_row(dc: HDC, row: Option<&crate::FavRow>, r: RECT, sn: &Snap, empty: bool) {
    crate::paint_fav_row(dc, row, r, sn.dpi.max(96), sn.font, sn.small, sn.icon, empty, false, true);
}

unsafe fn draw_key_row(dc: HDC, i: usize, r: RECT, change: RECT, reset: RECT, sn: &Snap) {
    let dpi = sn.dpi.max(96);
    if i == usize::MAX || i >= crate::keys::len() {
        crate::text(dc, r, tx(NONE), sn.small, crate::pal().sub, DT_CENTER);
        return;
    }
    let row_ground = if crate::keys::capture_row() == Some(i) {
        let wash = crate::blend(crate::pal().bg, chrome(), 0.16);
        crate::fill(dc, r, wash);
        wash
    } else {
        crate::pal().card
    };
    let (cat_s, act_s, chord) = crate::keys::row(i);
    let shown = if crate::keys::capture_row() == Some(i) {
        tx(crate::Key::PressKey).to_string()
    } else {
        chord
    };
    let (cat, keys_x) = key_cols(r);
    crate::text(dc, cat, &cat_s, sn.font, crate::pal().sub, DT_LEFT);
    crate::text(dc, RECT { left: cat.right, right: keys_x - sc(dpi, 8), ..r }, &act_s, sn.font, crate::pal().text, DT_LEFT);
    let (_, max_w) = chord_box(r, dpi);
    let widths = chord_widths(dc, sn.mono, &[&shown], dpi);
    let lines = pack_chords(&widths, max_w, chord_gap(dpi));
    let n = lines.len().max(1) as i32;
    let ch = chip_h(dpi);
    let lg = sc(dpi, 4);
    let stack = n * ch + (n - 1) * lg;
    let y = r.top + (r.bottom - r.top - stack).max(0) / 2;
    let limit = keys_x + max_w;
    let border = if crate::keys::is_custom(i) { chrome() } else { crate::pal().line };
    let mut x = keys_x;
    if let Some(line) = lines.first() {
        for &w in line {
            let right = (x + w).min(limit);
            let chip = RECT { left: x, top: y, right, bottom: (y + ch).min(r.bottom - 1) };
            if chip.right > chip.left && chip.bottom > chip.top {
                crate::aa_rfill(dc, chip, sc(dpi, 6), crate::pal().bg, Some(border), row_ground);
                crate::text(dc, chip, &shown, sn.mono, crate::pal().text, DT_CENTER);
            }
            x = right + chord_gap(dpi);
        }
    }
    let mini = |rect: RECT, label: &str, on: bool| {
        if rect.right <= rect.left {
            return;
        }
        let ink = if on { crate::pal().text } else { crate::pal().sub };
        crate::aa_rfill(dc, rect, sc(dpi, 4), crate::pal().card, Some(crate::pal().line), row_ground);
        crate::text(dc, rect, label, sn.small, ink, DT_CENTER);
    };
    mini(change, tx(crate::Key::Change), true);
    mini(reset, tx(crate::Key::Reset3), crate::keys::is_custom(i));
    crate::fill(dc, RECT { top: r.bottom - 1, ..r }, crate::pal().line);
}

unsafe fn text_w(dc: HDC, font: HFONT, s: &str) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    let old = SelectObject(dc, font.into());
    let _ = DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
    r.right
}

/// Search-box frame: card fill, `line` border, 4 DIP corners.
unsafe fn draw_field(dc: HDC, r: RECT, dpi: i32, ground: u32) {
    crate::aa_rfill(dc, r, sc(dpi, FIELD_RADIUS_DIP), crate::pal().card, Some(crate::pal().line), ground);
}


/// Settings footer and in-card buttons. The name prompt, About, and Excel tidy use
/// [`crate::paint_button`] so a theme accent cannot tint them.
pub(crate) unsafe fn draw_btn(d: &DRAWITEMSTRUCT) {
    let id = d.CtlID as i32;
    let dis = d.itemState.0 & 4 != 0;
    let pressed = d.itemState.0 & 1 != 0;
    let hot = app(|a| a.hover) == d.hwndItem;
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let (font, heavy) = ST.with(|s| {
        let st = s.borrow();
        (st.font, st.btn_bold)
    });
    // The name prompt can open after Settings has dropped its fonts. The control then
    // carries the UI face; the accent label uses that face at weight 700.
    let font = if font.is_invalid() {
        HFONT(SendMessageW(d.hwndItem, WM_GETFONT, None, None).0 as _)
    } else {
        font
    };
    let accent = chrome();
    // Prompt Close is also id 2 (S_CLOSE). Only a settings child uses that id as the accent;
    // the prompt accents OK, which is id 1.
    let parent = GetParent(d.hwndItem).unwrap_or_default();
    let on_settings = parent.0.is_null() || parent == hwnd();
    let accent_btn = if on_settings { id == S_CLOSE } else { id == 1 };
    let on_card = if on_settings { id != S_CLOSE && id != S_RESET } else { true };
    let ground = if on_card { crate::pal().card } else { crate::pal().bg };
    crate::fill(dc_of(d), d.rcItem, ground);
    let (fill, border, ink) = if accent_btn && !dis {
        (accent, accent, on_chrome(accent))
    } else if dis {
        (crate::blend(ground, crate::pal().sub, 0.12), crate::pal().line, crate::pal().sub)
    } else if pressed {
        (crate::blend(ground, crate::pal().text, 0.08), crate::pal().line, crate::pal().text)
    } else if hot {
        (crate::blend(ground, accent, 0.12), accent, crate::pal().text)
    } else if id == S_ACCENT && app(|a| !crate::pal().accents.iter().any(|(_, c)| *c == a.s.accent_now())) {
        (ground, accent, crate::pal().text)
    } else if id == S_ADV_RESET {
        (ground, crate::pal().line, 0xC42B1C)
    } else {
        (ground, crate::pal().line, crate::pal().text)
    };
    crate::aa_rfill(dc_of(d), d.rcItem, sc(dpi, 4), fill, Some(border), ground);
    let label = crate::text_of(d.hwndItem);
    let label_font = if accent_btn && !dis {
        if !heavy.is_invalid() {
            heavy
        } else {
            let bold = crate::bold_ui_font();
            if bold.is_invalid() { font } else { bold }
        }
    } else {
        font
    };
    crate::text(dc_of(d), d.rcItem, &label, label_font, ink, DT_CENTER);
}

fn dc_of(d: &DRAWITEMSTRUCT) -> HDC {
    d.hDC
}

fn ev_button_label() -> &'static str {
    match crate::everything::action() {
        Some(crate::everything::EvAction::Start) => tx(crate::Key::StartEverything),
        _ => tx(crate::Key::InstallEverything),
    }
}

fn ev_status_line() -> String {
    use crate::everything::{Presence, Shown};
    match crate::everything::shown() {
        Shown::Unknown => tx(crate::Key::Checking).to_string(),
        Shown::Ready(Presence::NotInstalled) => tx(crate::Key::NotInstalled).to_string(),
        Shown::Ready(Presence::InstalledStopped) => tx(crate::Key::InstalledButNotRunning).to_string(),
        Shown::Ready(Presence::Running { version }) => {
            let base = tx(crate::Key::InstalledAndRunning);
            match version {
                Some(v) => format!("{base} · {}", v.label()),
                None => base.to_string(),
            }
        }
    }
}

fn ev_activity_line() -> String {
    use crate::everything::Activity;
    match crate::everything::activity() {
        Activity::Idle | Activity::Checking => String::new(),
        Activity::Starting => tx(crate::Key::StartingEverything).to_string(),
        Activity::Installing => tx(crate::Key::InstallingEverything).to_string(),
        Activity::Winget(line) => line,
        Activity::Started => tx(crate::Key::EverythingIsRunning).to_string(),
        Activity::Installed => tx(crate::Key::EverythingWasInstalled).to_string(),
        Activity::StartFailed => tx(crate::Key::EverythingDidNotStart).to_string(),
        Activity::InstallFailed(code) => format!("{} ({code})", tx(crate::Key::InstallFailed)),
        Activity::OpenedPage => tx(crate::Key::OpenedDownloadPage).to_string(),
    }
}

fn flip(id: u8) {
    match id {
        TG_SINGLE => app(|a| a.s.single ^= true),
        TG_ONE => app(|a| a.s.one_window ^= true),
        TG_DBLUP => app(|a| a.s.dblup ^= true),
        TG_HIDDEN => {
            unsafe { crate::toggle_hidden() };
            return;
        }
        TG_BAR => {
            app(|a| a.s.bar ^= true);
            unsafe {
                crate::save_session();
                // Every window shares the switch. Lay each one out, or a window that was not
                // current would keep the icons while its tabs already took that width.
                crate::across_windows(|| {
                    crate::relayout();
                    crate::redraw_all();
                });
                redraw();
            }
            return;
        }
        TG_DRIVES => {
            app(|a| a.s.drive_buttons ^= true);
            unsafe {
                crate::save_session();
                // Every window shares the switch. Lay each one out, or a window that was not
                // current would keep the letters while its tabs already took that width.
                crate::across_windows(|| {
                    crate::relayout();
                    crate::redraw_all();
                });
                redraw();
            }
            return;
        }
        TG_STATUS => app(|a| a.s.stat ^= true),
        TG_EXT => {
            let on = app(|a| {
                a.s.ext ^= true;
                a.s.ext
            });
            unsafe {
                crate::apply_ext(on);
                crate::save_session();
                redraw();
            }
            return;
        }
        TG_TREE => {
            unsafe { crate::toggle_tree() };
            return;
        }
        TG_PREVIEW => {
            unsafe { crate::toggle_preview() };
            return;
        }
        TG_EDGE => app(|a| a.s.edge ^= true),
        TG_CONFIRM => app(|a| a.s.confirm ^= true),
        TG_FAST => app(|a| a.s.fast_copy ^= true),
        TG_DIALOG => {
            app(|a| a.s.dialog_jump ^= true);
            unsafe {
                crate::save_session();
                crate::dialogjump::sync();
                redraw();
            }
            return;
        }
        TG_CLIP => {
            app(|a| a.s.clip_hotkey ^= true);
            unsafe {
                crate::publish_shared();
                crate::save_session();
                crate::cliphot::apply(true);
                refresh();
            }
            return;
        }
        TG_AUTOSAVE => app(|a| a.s.autosave ^= true),
        TG_UPDATE => app(|a| a.s.update_check ^= true),
        TG_EV => {
            app(|a| a.s.use_everything ^= true);
            unsafe {
                crate::save_session();
                redraw();
            }
            return;
        }
        TG_SPACE => app(|a| a.s.space_preview ^= true),
        TG_WHEEL => app(|a| a.s.peek_wheel_nav ^= true),
        TG_FSIZE => {
            app(|a| a.s.folder_sizes ^= true);
            let (show, net) = app(|a| (a.s.folder_sizes, a.s.folder_sizes_network));
            crate::set_opts(show, net);
            if !show {
                crate::clear_pending();
            }
            app(|a| a.status_key.clear());
        }
        TG_FSIZE_NET => {
            app(|a| a.s.folder_sizes_network ^= true);
            let (show, net) = app(|a| (a.s.folder_sizes, a.s.folder_sizes_network));
            crate::set_opts(show, net);
            crate::clear_pending();
            app(|a| a.status_key.clear());
        }
        TG_REMEMBER => {
            app(|a| a.s.remember_views ^= true);
            let on = app(|a| a.s.remember_views);
            unsafe {
                // Other windows read the flag from the shared session. Arm or capture before they poll.
                crate::publish_shared();
                crate::across_windows(|| {
                    if on {
                        crate::arm_folder_watches();
                    } else {
                        // Each pane keeps the view on screen as its own, now that folders no longer override it.
                        let n = app(|a| a.s.panes());
                        for i in 0..n {
                            crate::save_view(i);
                        }
                    }
                });
                crate::save_session();
                redraw();
            }
            return;
        }
        TG_DOT => {
            app(|a| a.s.dot ^= true);
            unsafe {
                crate::save_session();
                crate::check_dirty();
                redraw();
            }
            return;
        }
        TG_FOLDER => {
            let want = !crate::folderapp::shown_on();
            crate::folderapp::set_registered(want);
            unsafe {
                if crate::folderapp::shown_on() != want {
                    let q = tx(crate::Key::CouldNotChangeDefaultFolderApp);
                    let _ = MessageBoxW(Some(hwnd()), &HSTRING::from(q), w!("Maguro"), MB_ICONWARNING);
                }
                redraw();
            }
            return;
        }
        _ => return,
    }
    unsafe {
        crate::save_session();
        crate::relayout();
        crate::redraw_all();
        redraw();
    }
}

unsafe fn delete_fav(k: usize) {
    let Some(id) = fav_node(k).map(|r| r.id) else { return };
    let links = crate::servers_at(&id);
    app(|a| {
        a.s.ensure_fav_tree();
        crate::delete_at(&mut a.s.tree, &id);
        a.s.sync_favs();
        a.fav_hot = None;
    });
    crate::release_servers(links);
    crate::commit_favs();
}

unsafe fn on_command(h: HWND, id: i32, code: u32) {
    if BUSY.with(|b| b.get()) {
        return;
    }
    if code == EN_CHANGE && id == S_SEARCH {
        let q = crate::text_of(ctl(h, S_SEARCH));
        let (anchor, page) = ST.with(|s| {
            let st = s.borrow();
            (st.anchor, st.page)
        });
        let (anchor, page) = commit_search(anchor, page, &q);
        ST.with(|s| {
            let mut st = s.borrow_mut();
            st.query = q;
            st.anchor = anchor;
            st.page = page;
            st.scroll = 0;
        });
        layout(h);
        let _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if code == EN_CHANGE && id == S_WM {
        let path = crate::text_of(ctl(h, S_WM));
        app(|a| a.s.winmerge_path = path);
        crate::winmerge::forget_status();
        crate::save_session();
        let _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if code == EN_CHANGE && id == S_KEYQ {
        let q = crate::text_of(ctl(h, S_KEYQ));
        ST.with(|s| {
            let mut st = s.borrow_mut();
            st.keyq = q;
            st.scroll = 0;
        });
        layout(h);
        let _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if code == CBN_SELCHANGE && id == S_LANG {
        let sel = SendMessageW(ctl(h, S_LANG), 0x0147, None, None).0 as usize;
        let lang = crate::settings_languages().get(sel).map(|(l, _)| *l).unwrap_or(Lang::Auto);
        if app(|a| a.s.lang) != lang {
            crate::set_language(lang);
        }
        return;
    }
    if code == CBN_SELCHANGE {
        let sel = SendMessageW(ctl(h, id), 0x0147, None, None).0 as usize;
        if combo_pick(h, id, sel) {
            return;
        }
    }
    if code != 0 {
        return;
    }
    match id {
        S_CLOSE => close(),
        S_RESET => reset_keys(h),
        S_SESS => {
            let d = crate::sessions_dir();
            let _ = std::fs::create_dir_all(&d);
            let p = HSTRING::from(d.to_string_lossy().as_ref());
            ShellExecuteW(Some(h), w!("open"), &p, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
        S_ACCENT => {
            crate::pick_accent();
            crate::save_session();
            redraw();
        }
        S_ADV_OPEN => {
            let d = crate::store::config_dir();
            let _ = std::fs::create_dir_all(&d);
            let p = HSTRING::from(d.to_string_lossy().as_ref());
            ShellExecuteW(Some(h), w!("open"), &p, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
        S_ADV_EX => export_settings(h),
        S_ADV_IM => import_settings(h),
        S_ADV_RESET => reset_ini(h),
        S_FV_DEF => crate::set_default_folder_view(),
        S_FV_RESET => crate::reset_active_folder_view(),
        S_CTX_RESET => {
            app(|a| {
                let s = &mut a.s;
                crate::shell::ctxmenu::reset_menu(&mut s.ctx_off, &mut s.ctx_on);
                crate::shell::ctxpack::enable_all(&mut s.ctx_packaged_on);
            });
            crate::save_session();
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
        }
        S_CTX_REFRESH => {
            crate::shell::ctxmenu::invalidate();
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
        }
        S_FV_CLEAR => {
            let q = tx(crate::Key::ForgetSavedViewOfEveryFolderDefault);
            if confirm_box(h, q) {
                crate::clear_remembered_folder_views();
            }
        }
        S_EV => {
            if crate::everything::busy() {
                return;
            }
            match crate::everything::action() {
                Some(crate::everything::EvAction::Start) => crate::everything::start_async(),
                Some(crate::everything::EvAction::Install) => crate::everything::install_async(),
                None => {}
            }
        }
        S_WM_BROWSE => {
            let cur = app(|a| a.s.winmerge_path.clone());
            let Some(path) = crate::winmerge::pick_exe(h, &cur) else { return };
            app(|a| a.s.winmerge_path = path.clone());
            crate::winmerge::forget_status();
            BUSY.with(|b| b.set(true));
            let _ = SetWindowTextW(ctl(h, S_WM), &HSTRING::from(&path));
            BUSY.with(|b| b.set(false));
            crate::save_session();
            let _ = InvalidateRect(Some(h), None, false);
        }
        id if (S_FAV0..S_REN0).contains(&id) => delete_fav((id - S_FAV0) as usize),
        id if (S_REN0..S_REN0 + FAV_MAX).contains(&id) => rename_fav(h, (id - S_REN0) as usize),
        _ => {}
    }
}

fn combo_pick(h: HWND, id: i32, sel: usize) -> bool {
    match id {
        S_THEME => {
            app(|a| a.s.theme = sel.min(2) as u8);
            unsafe {
                crate::apply_theme();
                crate::save_session();
            }
        }
        S_FACE => {
            let name = if sel == 0 { String::new() } else { unsafe { combo_lb_text(h, S_FACE, sel) } };
            if sel != 0 && name.is_empty() {
                return true;
            }
            let changed = app(|a| {
                if a.s.font_face == name {
                    false
                } else {
                    a.s.font_face = name;
                    true
                }
            });
            if changed {
                unsafe {
                    crate::publish_shared();
                    crate::apply_font_all();
                    crate::save_session();
                    refresh();
                }
            }
        }
        S_VIEW => {
            let sel = sel.min(crate::EXPLORER_VIEWS.len() - 1);
            let (mode, icon, ..) = crate::EXPLORER_VIEWS[sel];
            app(|a| {
                a.s.view_new = mode;
                a.s.view_icon = crate::stored_view_icon(mode, icon);
            });
            unsafe { crate::save_session(); }
        }
        S_DIR => {
            if sel == 2 {
                let path = unsafe { crate::pick_folder(h) };
                match path {
                    Some(p) => app(|a| {
                        a.s.start_new = 2;
                        a.s.start_path = p;
                    }),
                    None => {
                        unsafe { refresh() };
                        return true;
                    }
                }
            } else {
                app(|a| a.s.start_new = sel.min(1) as u8);
            }
            unsafe { crate::save_session() };
        }
        S_DEL => {
            app(|a| a.s.del = if sel == 1 { 1 } else { 0 });
            unsafe { crate::save_session() };
        }
        S_NEW => {
            if sel == 0 {
                app(|a| a.s.new_name.clear());
            } else {
                let cur = app(|a| if a.s.new_name.is_empty() { crate::default_new_name() } else { a.s.new_name.clone() });
                match unsafe { ask(h, tx(crate::Key::NewFileName), &cur) } {
                    Some(name) if !name.trim().is_empty() => app(|a| a.s.new_name = name),
                    _ => {
                        unsafe { refresh() };
                        return true;
                    }
                }
            }
            unsafe {
                crate::save_session();
                refresh();
            }
        }
        S_DROP => {
            app(|a| a.s.dropm = sel.min(2) as u8);
            unsafe { crate::save_session() };
        }
        S_LAYOUT => {
            app(|a| a.s.layout_new = sel.min(3) as u8);
            unsafe { crate::save_session() };
        }
        S_SIDE => {
            let fav_top = sel != 1;
            let changed = app(|a| {
                if a.s.fav_top == fav_top {
                    false
                } else {
                    a.s.fav_top = fav_top;
                    true
                }
            });
            if changed {
                unsafe {
                    crate::publish_shared();
                    crate::save_session();
                    crate::across_windows(|| {
                        crate::relayout();
                        crate::redraw_all();
                    });
                    redraw();
                }
            }
        }
        S_START => {
            if sel == 1 {
                match unsafe { crate::pick_session(h, crate::tr(crate::Key::SessionOpenOnStartup)) } {
                    Some(p) => app(|a| {
                        a.s.boot = 1;
                        a.s.boot_file = p.to_string_lossy().into_owned();
                    }),
                    None => {
                        unsafe { refresh() };
                        return true;
                    }
                }
            } else {
                app(|a| a.s.boot = if sel >= 2 { 2 } else { 0 });
            }
            unsafe { crate::save_session() };
        }
        _ => return false,
    }
    true
}

unsafe fn export_settings(h: HWND) {
    let Some(path) = crate::pick_settings(h, true) else { return };
    crate::save_session();
    let text = crate::store::current_text();
    if crate::store::write_atomic(std::path::Path::new(&path), text.as_bytes()).is_err() {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(crate::Key::CouldNotExportSettingsFile)), w!("Maguro"), MB_ICONWARNING);
    }
}

unsafe fn import_settings(h: HWND) {
    let Some(path) = crate::pick_settings(h, false) else { return };
    let Ok(text) = std::fs::read_to_string(&path) else {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(crate::Key::CouldNotReadThatFile)), w!("Maguro"), MB_ICONWARNING);
        return;
    };
    let q = tx(crate::Key::ReplaceCurrentSettingsWithThisFile);
    if !crate::ask_confirm(h, crate::Confirm::ok("Maguro", q)) {
        return;
    }
    if !crate::import_settings(&text) {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(crate::Key::ThatFileIsNotValidSettingsJson)), w!("Maguro"), MB_ICONWARNING);
    }
}

unsafe fn reset_ini(h: HWND) {
    let q = tx(crate::Key::ResetEveryPreferenceItsDefaultLayoutFold);
    if !crate::ask_confirm(h, crate::Confirm::danger("Maguro", q).affirm_label(tx(crate::Key::Reset3))) {
        return;
    }
    crate::reset_prefs();
}

unsafe fn rename_fav(h: HWND, k: usize) {
    let Some(row) = fav_node(k) else { return };
    let Some(name) = ask(h, tx(crate::Key::Rename2), &row.name) else { return };
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let key = (row.name.clone(), row.path.clone(), row.group);
    let done = app(|a| {
        a.s.ensure_fav_tree();
        let Some(id) = crate::locate(&a.s.tree, &row.id, &key) else { return false };
        crate::rename_at(&mut a.s.tree, &id, &name);
        a.s.sync_favs();
        true
    });
    if done {
        crate::commit_favs();
    }
}

unsafe fn reset_keys(h: HWND) {
    let q = tx(crate::Key::ResetEveryShortcutItsDefault);
    if !crate::ask_confirm(h, crate::Confirm::danger("Maguro", q).affirm_label(tx(crate::Key::Reset3))) {
        return;
    }
    crate::keys::reset_all();
    crate::apply_hotkey(true);
    crate::save_session();
    refresh();
}

unsafe fn reset_one(i: usize) {
    let Some(chord) = crate::keys::default_chord(i) else { return };
    if crate::keys::same(i, &chord) {
        return;
    }
    if let Some(other) = crate::keys::conflict(i, chord) {
        if !confirm_swap(&crate::keys::format_chord(&chord), &crate::keys::action(other)) {
            return;
        }
    }
    crate::keys::rebind(i, chord);
    crate::apply_hotkey(true);
    crate::save_session();
    refresh();
}

fn key_click(g: &Geo, x: i32, y: i32) -> Option<(usize, u8)> {
    for item in &g.items {
        if let Item::KeyRow { i, rect, change, reset } = item {
            if *i == usize::MAX || !inside(*rect, x, y) {
                continue;
            }
            if inside(*change, x, y) {
                return Some((*i, 1));
            }
            if inside(*reset, x, y) {
                return Some((*i, 2));
            }
            return Some((*i, 0));
        }
    }
    None
}

pub(crate) unsafe fn confirm_swap(chord: &str, other: &str) -> bool {
    let q = tf(crate::Key::ShortcutSwap, &[("other", other), ("chord", chord)]);
    confirm_box(hwnd(), &q)
}

unsafe fn confirm_box(owner: HWND, q: &str) -> bool {
    crate::ask_confirm(owner, crate::Confirm::ok("Maguro", q))
}

unsafe fn on_click(h: HWND, x: i32, y: i32) {
    let sn = snap();
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let g = geo(rc, &sn);
    if let Some((i, kind)) = key_click(&g, x, y) {
        if kind == 1 {
            crate::keys::begin(i);
            let _ = InvalidateRect(Some(h), None, false);
            return;
        }
        if kind == 2 {
            reset_one(i);
            return;
        }
    }
    for i in 0..PAGE_N {
        if g.show[i] && inside(g.nav[i], x, y) {
            if i == CTX_PAGE {
                crate::shell::ctxmenu::invalidate();
            }
            ST.with(|s| {
                let mut st = s.borrow_mut();
                st.page = i;
                st.anchor = i;
                st.scroll = 0;
            });
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
            return;
        }
    }
    for item in &g.items {
        if let Item::FavRow { i, rect, rename, btn } = item {
            if *i != usize::MAX && inside(*rect, x, y) && !inside(*rename, x, y) && !inside(*btn, x, y) {
                if let Some(row) = fav_node(*i) {
                    ST.with(|s| s.borrow_mut().fav_drag = Some(row.id));
                    let _ = SetCapture(h);
                }
                return;
            }
        }
        if let Item::CtxSeg { classic, win11 } = item {
            let next = if inside(*classic, x, y) {
                Some(false)
            } else if inside(*win11, x, y) {
                Some(true)
            } else {
                None
            };
            if let Some(win) = next {
                let changed = app(|a| {
                    if a.s.ctx_win11_view == win {
                        false
                    } else {
                        a.s.ctx_win11_view = win;
                        true
                    }
                });
                if changed {
                    crate::save_session();
                    ST.with(|s| s.borrow_mut().scroll = 0);
                    layout(h);
                    let _ = InvalidateRect(Some(h), None, false);
                }
                return;
            }
        }
        if let Item::CtxRow { id, ids, scopes, rect, mark, packaged, .. } = item {
            if inside(*rect, x, y) {
                if *packaged {
                    let id = id.clone();
                    app(|a| crate::shell::ctxpack::toggle(&mut a.s.ctx_packaged_on, &id));
                    crate::save_session();
                    layout(h);
                    let _ = InvalidateRect(Some(h), None, false);
                    return;
                }
                if inside(*mark, x, y) {
                    let ids = ids.clone();
                    let scopes = scopes.clone();
                    app(|a| {
                        let s = &mut a.s;
                        crate::shell::ctxmenu::toggle_ids_master(&mut s.ctx_off, &mut s.ctx_on, &ids, &scopes);
                    });
                    crate::save_session();
                } else {
                    let id = id.clone();
                    ST.with(|s| {
                        let mut st = s.borrow_mut();
                        if let Some(at) = st.ctx_open.iter().position(|x| x == &id) {
                            st.ctx_open.remove(at);
                        } else {
                            st.ctx_open.push(id);
                        }
                    });
                }
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
                return;
            }
        }
        if let Item::CtxSub { ids, scopes, bit, rect, .. } = item {
            if inside(*rect, x, y) {
                let ids = ids.clone();
                let scopes = scopes.clone();
                let bit = *bit;
                app(|a| {
                    let s = &mut a.s;
                    crate::shell::ctxmenu::toggle_ids_target(&mut s.ctx_off, &mut s.ctx_on, &ids, &scopes, bit);
                });
                crate::save_session();
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
                return;
            }
        }
        if let Item::Card(c) = item {
            if let Ctl::Toggle(id) = c.spec.ctl {
                if c.spec.live && inside(c.ctl, x, y) {
                    flip(id);
                    return;
                }
            }
            if c.spec.ctl == Ctl::Accent {
                let colors = crate::pal().accents;
                for (i, r) in c.sw.iter().enumerate() {
                    if inside(*r, x, y) {
                        if let Some((_, col)) = colors.get(i) {
                            crate::set_accent(*col);
                            crate::save_session();
                        }
                        return;
                    }
                }
            }
        }
    }
    if g.has_bar && inside(g.track, x, y) {
        if inside(g.thumb, x, y) {
            ST.with(|s| s.borrow_mut().drag = Some(y - g.thumb.top));
            let _ = SetCapture(h);
        } else {
            let page = (g.content.bottom - g.content.top).max(1);
            let dir = if y < g.thumb.top { -1 } else { 1 };
            let max = g.scroll_max;
            ST.with(|s| {
                let mut st = s.borrow_mut();
                st.scroll = (st.scroll + dir * page).clamp(0, max);
            });
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
        }
    }
}

extern "system" fn settings_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| settings_proc_inner(h, msg, wp, lp)))
}
fn settings_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let dpi = GetDpiForWindow(h) as i32;
                ST.with(|s| {
                    let mut st = s.borrow_mut();
                    *st = St::default();
                    st.hwnd = h;
                    st.dpi = dpi.max(96);
                });
                rebuild_fonts();
                build_children(h);
                crate::note_side(h);
                crate::everything::bind_settings(h.0 as isize);
                crate::everything::probe_async();
                crate::winmerge::forget_status();
                refresh();
                return LRESULT(0);
            }
            WM_SIZE => {
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as i32;
                ST.with(|s| s.borrow_mut().dpi = dpi.max(96));
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                refresh();
            }
            WM_GETMINMAXINFO => {
                let dpi = ST.with(|s| s.borrow().dpi.max(96));
                let m = &mut *(lp.0 as *mut MINMAXINFO);
                m.ptMinTrackSize.x = sc(dpi, 780);
                m.ptMinTrackSize.y = sc(dpi, 520);
                return LRESULT(0);
            }
            WM_PAINT => paint(h),
            // The class brush is null. Claiming the erase without filling leaves
            // COLOR_WINDOW, which is white, under the first dark frame.
            WM_ERASEBKGND => {
                let mut rc = RECT::default();
                let _ = GetClientRect(h, &mut rc);
                crate::fill(HDC(wp.0 as _), rc, crate::pal().bg);
                return LRESULT(1);
            }
            WM_REVEAL => reveal(h),
            WM_DRAWITEM => {
                draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC => {
                let dc = HDC(wp.0 as _);
                let hw = HWND(lp.0 as _);
                let off = !IsWindowEnabled(hw).as_bool();
                SetBkMode(dc, TRANSPARENT);
                SetBkColor(dc, crate::cref(crate::pal().card));
                SetTextColor(dc, crate::cref(if off { crate::pal().sub } else { crate::pal().text }));
                SetDCBrushColor(dc, crate::cref(crate::pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => on_command(h, (wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32),
            WM_LBUTTONDOWN => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                on_click(h, x, y);
            }
            WM_LBUTTONDBLCLK => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                let sn = snap();
                let mut rc = RECT::default();
                let _ = GetClientRect(h, &mut rc);
                let g = geo(rc, &sn);
                if let Some((i, 0)) = key_click(&g, x, y) {
                    crate::keys::begin(i);
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_MOUSEMOVE => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                if let Some(from) = ST.with(|s| s.borrow().fav_drag.clone()) {
                    let sn = snap();
                    let mut rc = RECT::default();
                    let _ = GetClientRect(h, &mut rc);
                    let g = geo(rc, &sn);
                    let hit = g.items.iter().find_map(|it| match it {
                        Item::FavRow { i, rect, .. } if *i != usize::MAX && y >= rect.top && y < rect.bottom => Some((*i, *rect)),
                        _ => None,
                    });
                    if let Some((to_i, rect)) = hit {
                        let moved = app(|a| {
                            a.s.ensure_fav_tree();
                            let target = crate::tree_rows(&a.s.tree).get(to_i).map(|r| (r.id.clone(), r.group));
                            let Some((tid, group)) = target else { return None };
                            if tid == from {
                                return None;
                            }
                            let zone = crate::drop_zone(group, y - rect.top, (rect.bottom - rect.top).max(1));
                            let new_id = crate::apply_drop(&mut a.s.tree, &from, &tid, zone)?;
                            if new_id == from {
                                return None;
                            }
                            a.s.sync_favs();
                            Some(new_id)
                        });
                        if let Some(new_id) = moved {
                            ST.with(|s| s.borrow_mut().fav_drag = Some(new_id));
                            layout(h);
                            let _ = InvalidateRect(Some(h), None, false);
                        }
                    }
                    let _ = x;
                    return LRESULT(0);
                }
                if let Some(off) = ST.with(|s| s.borrow().drag) {
                    let (track, thumb_h, max) = ST.with(|s| {
                        let st = s.borrow();
                        (st.track, st.thumb_h, st.scroll_max)
                    });
                    let span = (track.bottom - track.top - thumb_h).max(1);
                    let top = (y - off).clamp(track.top, track.bottom - thumb_h);
                    let scroll = if max == 0 { 0 } else { (top - track.top) * max / span };
                    ST.with(|s| s.borrow_mut().scroll = scroll.clamp(0, max));
                    layout(h);
                    let _ = InvalidateRect(Some(h), None, false);
                } else {
                    let sn = snap();
                    let mut rc = RECT::default();
                    let _ = GetClientRect(h, &mut rc);
                    let g = geo(rc, &sn);
                    let hot = (0..PAGE_N).find(|&i| g.show[i] && inside(g.nav[i], x, y));
                    let changed = ST.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.hot != hot {
                            st.hot = hot;
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
            }
            WM_LBUTTONUP => {
                let fav = ST.with(|s| s.borrow_mut().fav_drag.take());
                if ST.with(|s| s.borrow().drag.is_some()) || fav.is_some() {
                    ST.with(|s| s.borrow_mut().drag = None);
                    let _ = ReleaseCapture();
                }
                if fav.is_some() {
                    crate::commit_favs();
                }
            }
            WM_CAPTURECHANGED => {
                // Capture taken away mid-drag (Alt+Tab, a popup): end the drag as a button-up would.
                // A normal button-up has already cleared both, so this does nothing then.
                let fav = ST.with(|s| s.borrow_mut().fav_drag.take());
                ST.with(|s| s.borrow_mut().drag = None);
                if fav.is_some() {
                    crate::commit_favs();
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) as i16 as i32) / 120;
                if delta != 0 {
                    let dpi = ST.with(|s| s.borrow().dpi.max(96));
                    let (scroll, max) = ST.with(|s| {
                        let st = s.borrow();
                        (st.scroll, st.scroll_max)
                    });
                    let next = (scroll - delta * sc(dpi, 48)).clamp(0, max);
                    if next != scroll {
                        ST.with(|s| s.borrow_mut().scroll = next);
                        layout(h);
                        let _ = InvalidateRect(Some(h), None, false);
                    }
                }
            }
            WM_MOUSELEAVE => {
                ST.with(|s| s.borrow_mut().hot = None);
                let _ = InvalidateRect(Some(h), None, false);
            }
            crate::WM_EV => {
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
            }
            crate::WM_FAV_ICON => {
                crate::clear_icon_refresh();
                if ST.with(|s| s.borrow().page) == 4 {
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_CLOSE => {
                let _ = DestroyWindow(h);
            }
            WM_DESTROY => {
                crate::drop_side(h);
                crate::everything::bind_settings(0);
                crate::keys::cancel();
                let old = ST.with(|s| {
                    let mut st = s.borrow_mut();
                    let old = (st.font, st.btn_bold, st.small, st.bold, st.title_font, st.mono, st.icon);
                    *st = St::default(); // drops the installed-family list
                    old
                });
                for f in [old.0, old.1, old.2, old.3, old.4, old.5, old.6] {
                    crate::delete_font(f);
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

thread_local! {
    static ASK_EDIT: Cell<HWND> = Cell::new(HWND(std::ptr::null_mut()));
    static ASK_FONT: Cell<HFONT> = Cell::new(HFONT::default());
    static ASK_OUT: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
    static ABOUT_DONE: Cell<bool> = const { Cell::new(false) };
}

pub(crate) struct AboutCopy {
    pub title: String,
    pub version: String,
    pub desc: String,
    pub license: String,
    /// Plain-text repo address only. Not a link; never opened in a browser.
    pub repo: String,
}

/// Body of the About window: product name, version, description, license, and plain-text repo address.
pub(crate) fn about_copy(version: &str, desc: &str, license: &str) -> AboutCopy {
    AboutCopy {
        title: "Maguro".into(),
        version: version.into(),
        desc: desc.into(),
        license: license.into(),
        repo: "github.com/Samek86/maguro".into(),
    }
}

pub(crate) fn about_body(c: &AboutCopy) -> String {
    format!("{}\n{}\n{}\n{}\n{}", c.title, c.version, c.desc, c.license, c.repo)
}

/// About fallback icon. Light is group 3 and dark is group 2. Both embed `assets/Maguro.ico`.
/// The painted logo is the theme PNG; this group is the DrawIconEx fallback and matches the title bar.
pub(crate) fn about_icon_group(dark: bool) -> usize {
    crate::window_icon_group(dark)
}

pub(crate) struct AboutLay {
    pub icon: RECT,
    pub title: RECT,
    pub version: RECT,
    pub desc: RECT,
    pub license: RECT,
    pub repo: RECT,
    pub ok: RECT,
}

const ABOUT_W_DIP: i32 = 440;
const ABOUT_ICON_DIP: i32 = 48;
const ABOUT_BTN_W_DIP: i32 = 88;
const ABOUT_BTN_H_DIP: i32 = 28;

/// Client height for the stacked About body. Heights are already in pixels.
pub(crate) fn about_client_h(dpi: i32, desc_h: i32, lic_h: i32, repo_h: i32) -> i32 {
    let dpi = dpi.max(96);
    let m = sc(dpi, 16);
    sc(dpi, ABOUT_ICON_DIP)
        + desc_h
        + lic_h
        + repo_h
        + sc(dpi, ABOUT_BTN_H_DIP)
        + m * 2
        + sc(dpi, 16)
        + sc(dpi, 8)
        + sc(dpi, 4)
        + sc(dpi, 16)
}

pub(crate) fn about_lay(client_w: i32, client_h: i32, dpi: i32, desc_h: i32, lic_h: i32, repo_h: i32) -> AboutLay {
    let dpi = dpi.max(96);
    let s = |v: i32| sc(dpi, v);
    let m = s(16);
    let cw = client_w.max(1);
    let ch = client_h.max(1);
    let icon_s = s(ABOUT_ICON_DIP);
    let icon = RECT { left: m, top: m, right: m + icon_s, bottom: m + icon_s };
    let text_l = icon.right + s(12);
    let text_r = (cw - m).max(text_l + 1);
    let title_h = s(22);
    let ver_h = s(18);
    let title = RECT { left: text_l, top: m, right: text_r, bottom: m + title_h };
    let version = RECT { left: text_l, top: title.bottom + s(4), right: text_r, bottom: title.bottom + s(4) + ver_h };
    let desc_top = icon.bottom + s(16);
    let desc = RECT { left: m, top: desc_top, right: text_r, bottom: desc_top + desc_h.max(1) };
    let lic_top = desc.bottom + s(8);
    let license = RECT { left: m, top: lic_top, right: text_r, bottom: lic_top + lic_h.max(1) };
    let repo_top = license.bottom + s(4);
    let repo = RECT { left: m, top: repo_top, right: text_r, bottom: repo_top + repo_h.max(1) };
    let btn_h = s(ABOUT_BTN_H_DIP);
    let btn_w = s(ABOUT_BTN_W_DIP).min((cw - 2 * m).max(1));
    let ok_top = repo.bottom + s(16);
    let ok = RECT { left: (cw - m - btn_w).max(0), top: ok_top, right: (cw - m).max(1), bottom: ok_top + btn_h };
    let _ = ch;
    AboutLay { icon, title, version, desc, license, repo, ok }
}

fn about_version_label() -> String {
    let base = env!("CARGO_PKG_VERSION");
    let hash = env!("MAGURO_GIT_HASH");
    // Debug + portable builds only: makes a stale test exe obvious in About.
    let show = cfg!(debug_assertions) || crate::settings::store::exe_dir().join("portable.txt").is_file();
    if show && !hash.is_empty() {
        format!("{base} ({hash})")
    } else {
        base.into()
    }
}

fn live_about() -> AboutCopy {
    about_copy(
        &about_version_label(),
        tx(crate::Key::FastMultiPaneFileManagerForWindows),
        tx(crate::Key::MitLicense2026Efww),
    )
}

unsafe fn about_line_h(text: &str, width: i32, font: HFONT, dpi: i32) -> i32 {
    let min = sc(dpi.max(96), 18);
    if text.is_empty() || font.is_invalid() || width < 1 {
        return min;
    }
    let dc = GetDC(None);
    if dc.is_invalid() {
        return min;
    }
    let (_, h) = crate::wrap_text(dc, RECT { right: width, ..Default::default() }, text, font, 0, true);
    let _ = ReleaseDC(None, dc);
    h.max(min)
}

unsafe fn about_metrics(cw: i32, ch: i32, dpi: i32) -> AboutLay {
    let dpi = dpi.max(96);
    let font = ask_font();
    let inner = (cw - 2 * sc(dpi, 16)).max(1);
    let copy = live_about();
    let desc_h = about_line_h(&copy.desc, inner, font, dpi);
    let lic_h = about_line_h(&copy.license, inner, font, dpi);
    let repo_h = about_line_h(&copy.repo, inner, font, dpi);
    about_lay(cw, ch, dpi, desc_h, lic_h, repo_h)
}

unsafe fn draw_about_btn(d: &DRAWITEMSTRUCT) {
    let parent = GetParent(d.hwndItem).unwrap_or(d.hwndItem);
    let dpi = GetDpiForWindow(parent).max(96) as i32;
    let font = ask_font();
    let visual = crate::visual_from_item(d.itemState.0, app(|a| a.hover) == d.hwndItem);
    crate::paint_button(d.hDC, d.rcItem, crate::ButtonKind::Primary, visual, &crate::text_of(d.hwndItem), font, dpi, crate::ui_tokens().bg, crate::FocusRing::Inside);
}

unsafe fn paint_about(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let copy = live_about();
    crate::buffered(h, |dc, rc| {
        let cw = rc.right - rc.left;
        let ch = rc.bottom - rc.top;
        crate::paint_window(dc, rc);
        let lay = about_metrics(cw, ch, dpi);
        let font = ask_font();
        let small = app(|a| a.small_font);
        let small = if small.is_invalid() { font } else { small };
        let bold = crate::bold_ui_font();
        let bold = if bold.is_invalid() { font } else { bold };
        {
            let dark = crate::DARK_ON.load(Ordering::Relaxed);
            let png: &[u8] = if dark {
                include_bytes!("../../assets/maguro-lockup-dark.png")
            } else {
                include_bytes!("../../assets/maguro-lockup-light.png")
            };
            let side = (lay.icon.right - lay.icon.left).max(1);
            if let Some(pix) = crate::decode_pic(png, side, side) {
                crate::blit_bgra(dc, lay.icon, pix.w, pix.h, &pix.bgra);
            } else if let Ok(module) = GetModuleHandleW(None) {
                let hinst: HINSTANCE = module.into();
                let id = about_icon_group(dark);
                if let Ok(icon) = LoadIconW(Some(hinst), PCWSTR(id as _)) {
                    let _ = DrawIconEx(dc, lay.icon.left, lay.icon.top, icon, side, side, 0, None, DI_NORMAL);
                }
            }
        }
        crate::text(dc, lay.title, &copy.title, bold, crate::pal().text, DT_LEFT);
        crate::text(dc, lay.version, &copy.version, small, crate::pal().sub, DT_LEFT);
        crate::wrap_text(dc, lay.desc, &copy.desc, font, crate::pal().text, false);
        crate::wrap_text(dc, lay.license, &copy.license, small, crate::pal().sub, false);
        // Plain text only — no SysLink, no ShellExecute, no click handler.
        crate::wrap_text(dc, lay.repo, &copy.repo, small, crate::pal().sub, false);
    });
}

unsafe fn layout_about(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let lay = about_metrics(rc.right - rc.left, rc.bottom - rc.top, dpi);
    move_to(ctl(h, 1), lay.ok, true);
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe extern "system" fn about_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { about_proc_inner(h, msg, wp, lp) }))
}

unsafe fn about_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_about(h);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_DRAWITEM => {
            draw_about_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
            return LRESULT(1);
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xffff) as i32;
            if id == 1 {
                let _ = DestroyWindow(h);
                return LRESULT(0);
            }
        }
        WM_CLOSE => {
            let _ = DestroyWindow(h);
            return LRESULT(0);
        }
        WM_DESTROY => {
            ABOUT_DONE.with(|d| d.set(true));
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            let font = ask_font();
            if !font.is_invalid() {
                SendMessageW(ctl(h, 1), WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
            layout_about(h);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

fn ensure_about_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(about_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(hinst), PCWSTR(crate::ICON_GROUP_LIGHT as _)).unwrap_or_default(),
            lpszClassName: w!("MaguroAbout"),
            ..Default::default()
        });
    });
}

/// About Maguro. One OK button. Body: name, version, description, license, plain-text repo.
pub(crate) unsafe fn show_about(owner: HWND) {
    ensure_about_class();
    ABOUT_DONE.with(|d| d.set(false));
    let dpi = GetDpiForWindow(owner).max(96) as i32;
    let client_w = sc(dpi, ABOUT_W_DIP);
    let font = ask_font();
    let copy = live_about();
    let inner = (client_w - 2 * sc(dpi, 16)).max(1);
    let desc_h = about_line_h(&copy.desc, inner, font, dpi);
    let lic_h = about_line_h(&copy.license, inner, font, dpi);
    let repo_h = about_line_h(&copy.repo, inner, font, dpi);
    let client_h = about_client_h(dpi, desc_h, lic_h, repo_h);
    let mut r = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
    let (ww, hh) = if AdjustWindowRectExForDpi(&mut r, ask_style(), false, WS_EX_DLGMODALFRAME, dpi as u32).is_ok() {
        ((r.right - r.left).max(client_w), (r.bottom - r.top).max(client_h))
    } else {
        (client_w, client_h + sc(dpi, 48))
    };
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    let title = {
        let base = tx(crate::Key::AboutMaguro);
        let hash = env!("MAGURO_GIT_HASH");
        let show = cfg!(debug_assertions) || crate::settings::store::exe_dir().join("portable.txt").is_file();
        if show && !hash.is_empty() {
            HSTRING::from(format!("{base} · {hash}"))
        } else {
            HSTRING::from(base)
        }
    };
    let h = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("MaguroAbout"), &title, ask_style(), x, y, ww, hh, Some(owner), None, None, None).unwrap_or_default();
    if h.0.is_null() {
        return;
    }
    style_window(h);
    let _ = child_at(h, w!("BUTTON"), tx(crate::Key::Ok), 0xB | 0x00010000, 1, font);
    layout_about(h);
    let _ = ShowWindow(h, SW_SHOW);
    crate::keys::set_modal(true);
    let _ = EnableWindow(owner, false);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(ctl(h, 1)));
    let mut msg = MSG::default();
    while !ABOUT_DONE.with(|d| d.get()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            ABOUT_DONE.with(|d| d.set(true));
            PostQuitMessage(0);
            break;
        }
        let mine = msg.hwnd == h || IsChild(h, msg.hwnd).as_bool();
        if msg.message == WM_MOUSEMOVE {
            crate::set_hover(if crate::is_icon_button(msg.hwnd) { msg.hwnd } else { HWND::default() });
        } else if msg.message == WM_MOUSELEAVE && app(|a| a.hover) == msg.hwnd {
            crate::set_hover(HWND::default());
        }
        if msg.message == WM_KEYDOWN && mine {
            let vk = msg.wParam.0 as u16;
            if vk == VK_RETURN.0 || vk == VK_ESCAPE.0 {
                let _ = SendMessageW(h, WM_COMMAND, Some(WPARAM(1)), Some(LPARAM(0)));
                continue;
            }
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    crate::set_hover(HWND::default());
    crate::keys::set_modal(false);
    let _ = EnableWindow(owner, true);
    let _ = SetForegroundWindow(owner);
}

unsafe extern "system" fn ask_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { ask_proc_inner(h, msg, wp, lp) }))
}
unsafe fn ask_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_ask(h);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let dc = HDC(wp.0 as _);
            let t = crate::ui_tokens();
            // The edit sits on the field fill. A static, if one appears, sits on the window.
            let bg = if msg == WM_CTLCOLOREDIT { t.field_bg } else { t.bg };
            // Transparent text so a selection highlight stays visible. The brush still fills the control.
            SetBkMode(dc, TRANSPARENT);
            SetBkColor(dc, crate::cref(bg));
            SetTextColor(dc, crate::cref(t.text));
            SetDCBrushColor(dc, crate::cref(bg));
            return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
        }
        WM_DRAWITEM => {
            draw_ask_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
            return LRESULT(1);
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xffff) as i32;
            let code = ((wp.0 >> 16) & 0xffff) as u32;
            if id == ASK_EDIT_ID && (code == EN_SETFOCUS || code == EN_KILLFOCUS) {
                let _ = InvalidateRect(Some(h), None, false);
                return LRESULT(0);
            }
            if id == ASK_OK {
                let text = crate::text_of(ASK_EDIT.with(|e| e.get()));
                ASK_OUT.with(|o| *o.borrow_mut() = Some(Some(text)));
                let _ = DestroyWindow(h);
                return LRESULT(0);
            }
            if id == ASK_CANCEL {
                ASK_OUT.with(|o| *o.borrow_mut() = Some(None));
                let _ = DestroyWindow(h);
                return LRESULT(0);
            }
        }
        WM_CLOSE => {
            ASK_OUT.with(|o| *o.borrow_mut() = Some(None));
            let _ = DestroyWindow(h);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let dpi = ((wp.0 & 0xffff) as i32).max(96);
            let (ww, hh) = ask_outer(dpi);
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(h, None, r.left, r.top, ww, hh, SWP_NOZORDER | SWP_NOACTIVATE);
            let _ = apply_ask_font(h, dpi);
            layout_ask(h);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

fn ask_font() -> HFONT {
    let font = ST.with(|s| s.borrow().font);
    if font.is_invalid() { app(|a| a.font) } else { font }
}

fn ask_style() -> WINDOW_STYLE {
    WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN
}

/// Mockup window (B) is 360px wide. The caption is the non-client strip of that same width.
const ASK_W_DIP: i32 = 360;
/// 12px label at the body line-height.
const ASK_LABEL_H_DIP: i32 = 16;
/// Space under the label. The mockup's label margin is 6px, between the 4 and 8 tokens.
const ASK_LABEL_GAP_DIP: i32 = 6;
const ASK_OK: i32 = 1;
const ASK_CANCEL: i32 = 2;
const ASK_EDIT_ID: i32 = 10;

/// 12px UI face. The settings page font is 14px and may already have been dropped.
fn ask_ui_font(dpi: i32) -> HFONT {
    let custom = app(|a| a.s.font_face.clone());
    let face = if custom.is_empty() { crate::ui_face().to_string() } else { custom };
    make_font(&face, crate::FONT_PX, 400, dpi.max(96))
}

/// Swap in a font for this DPI and point the live children at it. The previous face is deleted
/// only after `WM_SETFONT`, so a control never paints a freed handle.
unsafe fn apply_ask_font(h: HWND, dpi: i32) -> HFONT {
    let font = ask_ui_font(dpi);
    let old = ASK_FONT.with(|f| f.replace(font));
    if !font.is_invalid() {
        for id in [ASK_OK, ASK_CANCEL, ASK_EDIT_ID] {
            let c = ctl(h, id);
            if !c.0.is_null() {
                SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
        }
    }
    if !old.is_invalid() {
        crate::delete_font(old);
    }
    font
}

fn drop_ask_font() {
    let old = ASK_FONT.with(|f| f.replace(HFONT::default()));
    if !old.is_invalid() {
        unsafe { crate::delete_font(old) };
    }
}

fn ask_button_kind(id: i32) -> crate::ButtonKind {
    if id == ASK_OK { crate::ButtonKind::Primary } else { crate::ButtonKind::Secondary }
}

/// Label width plus the token pad, and at least the token minimum.
fn ask_btn_w(text_px: i32, dpi: i32) -> i32 {
    let dpi = dpi.max(96);
    (text_px.max(0) + 2 * sc(dpi, crate::BTN_PAD_X)).max(sc(dpi, crate::BTN_MIN_W))
}

/// Client height: body pad, label, field, footer rule, footer buttons, body pad.
fn ask_client_h(dpi: i32) -> i32 {
    let dpi = dpi.max(96);
    let s = |v: i32| sc(dpi, v);
    s(crate::PAD_BODY_Y) + s(ASK_LABEL_H_DIP) + s(ASK_LABEL_GAP_DIP) + s(crate::FIELD_H)
        + s(crate::SPACE_12) + 1 + s(crate::SPACE_12) + s(crate::BTN_H_FOOTER) + s(crate::PAD_BODY_Y)
}

/// Outer size. Both axes include the dialog frame so the client stays `ASK_W_DIP` by `ask_client_h`.
fn ask_outer(dpi: i32) -> (i32, i32) {
    let dpi = dpi.max(96);
    let client_w = sc(dpi, ASK_W_DIP);
    let client_h = ask_client_h(dpi);
    let mut r = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
    unsafe {
        if AdjustWindowRectExForDpi(&mut r, ask_style(), false, WS_EX_DLGMODALFRAME, dpi as u32).is_ok() {
            ((r.right - r.left).max(client_w), (r.bottom - r.top).max(client_h))
        } else {
            (client_w, client_h + sc(dpi, 48))
        }
    }
}

struct AskLay {
    label: RECT,
    field: RECT,
    edit: RECT,
    rule: RECT,
    ok: RECT,
    cancel: RECT,
}

/// Label, token field, footer rule, and right-aligned buttons. Content is top-aligned;
/// the caller sizes the client with [`ask_client_h`].
/// Cancel sits left of Save. `ok_w` and `cancel_w` are already in px; a pair that
/// cannot fit the content row is shrunk so the two still share `BTN_GAP`.
/// The edit slot is the field inset by 2 DIP. `place_edit` centres the line in that slot.
fn ask_lay(client_w: i32, dpi: i32, ok_w: i32, cancel_w: i32) -> AskLay {
    let dpi = dpi.max(96);
    let s = |v: i32| sc(dpi, v);
    let cw = client_w.max(1);
    let margin_x = s(crate::PAD_BODY_X);
    let left = margin_x.min(cw / 2);
    let right = (cw - margin_x).max(left + 1);
    let mut y = s(crate::PAD_BODY_Y);
    let label = RECT { left, top: y, right, bottom: y + s(ASK_LABEL_H_DIP) };
    y = label.bottom + s(ASK_LABEL_GAP_DIP);
    let field_h = s(crate::FIELD_H).max(1);
    let field = RECT { left, top: y, right, bottom: y + field_h };
    y = field.bottom + s(crate::SPACE_12);
    let rule = RECT { left, top: y, right, bottom: y + 1 };
    y = rule.bottom + s(crate::SPACE_12);
    let btn_h = s(crate::BTN_H_FOOTER);
    let gap = s(crate::BTN_GAP);
    let mut ok_w = ok_w.max(1);
    let mut cancel_w = cancel_w.max(1);
    let content = (right - left).max(1);
    if ok_w + cancel_w + gap > content {
        let room = (content - gap).max(2);
        let sum = ok_w + cancel_w;
        ok_w = (ok_w * room / sum).clamp(1, room - 1);
        cancel_w = room - ok_w;
    }
    let ok = RECT { left: right - ok_w, top: y, right, bottom: y + btn_h };
    let cancel_right = ok.left - gap;
    let cancel = RECT { left: cancel_right - cancel_w, top: y, right: cancel_right, bottom: y + btn_h };
    let edit = inset(field, s(FIELD_INSET_DIP).min(field_h / 2));
    AskLay { label, field, edit, rule, ok, cancel }
}

unsafe fn ask_button_widths(font: HFONT, dpi: i32) -> (i32, i32) {
    let dc = GetDC(None);
    if dc.is_invalid() {
        let w = sc(dpi, crate::BTN_MIN_W);
        return (w, w);
    }
    let ok = ask_btn_w(text_w(dc, font, tx(crate::Key::Save)), dpi);
    let cancel = ask_btn_w(text_w(dc, font, tx(crate::Key::Cancel)), dpi);
    let _ = ReleaseDC(None, dc);
    (ok, cancel)
}

unsafe fn paint_ask(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let font = ASK_FONT.with(|f| f.get());
    let font = if font.is_invalid() { ask_font() } else { font };
    let focused = GetFocus() == ASK_EDIT.with(|e| e.get());
    crate::buffered(h, |dc, rc| {
        crate::paint_window(dc, rc);
        let (ok_w, cancel_w) = ask_button_widths(font, dpi);
        let lay = ask_lay(rc.right - rc.left, dpi, ok_w, cancel_w);
        crate::paint_section_header(dc, lay.label, tx(crate::Key::Name), font);
        crate::paint_field(dc, lay.field, dpi, focused);
        crate::fill(dc, lay.rule, crate::ui_tokens().line);
    });
}

/// Place the borderless edit in the token field, and the footer buttons under the rule.
unsafe fn layout_ask(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let font = ASK_FONT.with(|f| f.get());
    let font = if font.is_invalid() { ask_font() } else { font };
    let (ok_w, cancel_w) = ask_button_widths(font, dpi);
    let lay = ask_lay(rc.right - rc.left, dpi, ok_w, cancel_w);
    let edit = ASK_EDIT.with(|e| e.get());
    let m = sc(dpi, crate::FIELD_PAD_X) as isize;
    SendMessageW(edit, 0x00D3, Some(WPARAM(3)), Some(LPARAM(m | (m << 16)))); // EM_SETMARGINS
    crate::place_edit(edit, lay.edit, true);
    move_to(ctl(h, ASK_OK), lay.ok, true);
    move_to(ctl(h, ASK_CANCEL), lay.cancel, true);
    let _ = InvalidateRect(Some(h), None, false);
}

/// Owner-draw footer button. Hover comes from the shared hover window, the same way the
/// session list does. Pressed wins, and the face is cerulean or secondary — never the theme accent.
unsafe fn draw_ask_btn(d: &DRAWITEMSTRUCT) {
    let dpi = GetDpiForWindow(d.hwndItem).max(96) as i32;
    let font = ASK_FONT.with(|f| f.get());
    let font = if font.is_invalid() {
        HFONT(SendMessageW(d.hwndItem, WM_GETFONT, None, None).0 as _)
    } else {
        font
    };
    let visual = crate::visual_from_item(d.itemState.0, app(|a| a.hover) == d.hwndItem);
    let kind = ask_button_kind(d.CtlID as i32);
    crate::paint_button(d.hDC, d.rcItem, kind, visual, &crate::text_of(d.hwndItem), font, dpi, crate::ui_tokens().bg, crate::FocusRing::Inside);
}

/// One-line prompt. None means cancel.
pub(crate) unsafe fn ask(owner: HWND, title: &str, initial: &str) -> Option<String> {
    ask_with(owner, title, initial, false)
}

/// Like [`ask`], but selects the stem (before the last extension) like Explorer F2.
pub(crate) unsafe fn ask_stem(owner: HWND, title: &str, initial: &str) -> Option<String> {
    ask_with(owner, title, initial, true)
}

fn stem_utf16_len(name: &str) -> isize {
    let p = std::path::Path::new(name);
    match (p.file_stem(), p.extension()) {
        (Some(stem), Some(_)) => stem.to_string_lossy().encode_utf16().count() as isize,
        _ => -1,
    }
}

/// One-line prompt. None means cancel. When `select_stem`, highlights the name stem.
unsafe fn ask_with(owner: HWND, title: &str, initial: &str, select_stem: bool) -> Option<String> {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(ask_proc),
            hInstance: m.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("MaguroAsk"),
            ..Default::default()
        });
    });
    ASK_OUT.with(|o| *o.borrow_mut() = None);
    let dpi = GetDpiForWindow(owner).max(96) as i32;
    let (ww, hh) = ask_outer(dpi);
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    // Shown only after the dark caption attributes are set, or the first frame stays light.
    let h = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("MaguroAsk"), &HSTRING::from(title), ask_style(), x, y, ww, hh, Some(owner), None, None, None).unwrap_or_default();
    if h.0.is_null() {
        return None;
    }
    style_window(h);
    // Mockup B is a square dialog, same as the other Maguro popups. About keeps style_window as it is.
    let square = DWMWCP_DONOTROUND;
    let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    let font = apply_ask_font(h, dpi);
    let edit = child_at(h, w!("EDIT"), initial, 0x80 | 0x00010000, ASK_EDIT_ID, font);
    theme_ctl(edit);
    ASK_EDIT.with(|e| e.set(edit));
    // Mockup B: secondary 취소, then primary 저장, both on the right.
    let _ = child_at(h, w!("BUTTON"), tx(crate::Key::Save), 0xB | 0x00010000, ASK_OK, font);
    let _ = child_at(h, w!("BUTTON"), tx(crate::Key::Cancel), 0xB | 0x00010000, ASK_CANCEL, font);
    layout_ask(h);
    let _ = ShowWindow(h, SW_SHOW);
    crate::keys::set_modal(true);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, false);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(edit));
    if select_stem {
        // EM_SETSEL = 0xB1; Explorer F2 selects the stem only.
        let _ = SendMessageW(edit, 0xB1, Some(WPARAM(0)), Some(LPARAM(stem_utf16_len(initial))));
    }
    let mut msg = MSG::default();
    while ASK_OUT.with(|o| o.borrow().is_none()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            PostQuitMessage(0);
            break;
        }
        // No dialog manager: Enter and Esc never become IDOK / IDCANCEL on their own.
        let mine = msg.hwnd == h || IsChild(h, msg.hwnd).as_bool();
        if msg.message == WM_KEYDOWN && mine {
            let cmd: usize = if msg.wParam.0 == VK_RETURN.0 as usize {
                ASK_OK as usize
            } else if msg.wParam.0 == VK_ESCAPE.0 as usize {
                ASK_CANCEL as usize
            } else {
                0
            };
            if cmd != 0 {
                let _ = SendMessageW(h, WM_COMMAND, Some(WPARAM(cmd)), Some(LPARAM(0)));
                continue;
            }
        }
        // Owner-draw buttons have no hot state. Track the pointer the way the session list does.
        if msg.message == WM_MOUSEMOVE || msg.message == WM_MOUSELEAVE {
            let on = msg.message == WM_MOUSEMOVE
                && crate::is_icon_button(msg.hwnd)
                && GetParent(msg.hwnd).unwrap_or_default() == h;
            crate::set_hover(if on { msg.hwnd } else { HWND::default() });
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    crate::set_hover(HWND::default());
    drop_ask_font();
    crate::keys::set_modal(false);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, true);
    let _ = SetForegroundWindow(owner);
    ASK_OUT.with(|o| o.borrow_mut().take().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_body_is_name_version_description_license_and_plain_repo() {
        let copy = about_copy("0.1.77", "Fast multi-pane file manager for Windows", "MIT License · © 2026 efww, Samek86");
        let body = about_body(&copy);
        assert_eq!(copy.title, "Maguro");
        assert_eq!(copy.repo, "github.com/Samek86/maguro");
        assert!(body.starts_with("Maguro\n0.1.77\n"));
        assert!(body.contains("Fast multi-pane file manager for Windows"));
        assert!(body.contains("MIT License · © 2026 efww, Samek86"));
        assert!(body.contains("github.com/Samek86/maguro"));
        // Plain address only — never a URL scheme that would invite a browser open.
        assert!(!body.contains("http"));
        assert!(!body.contains("https"));
        assert!(!body.contains("efww/"));
        assert!(!body.contains('?'));
        assert_eq!(crate::ICON_GROUP_EXE, 1);
        assert_eq!(crate::ICON_GROUP_DARK, 2);
        assert_eq!(crate::ICON_GROUP_LIGHT, 3);
        assert!(crate::ICON_GROUP_DARK > crate::ICON_GROUP_EXE);
        assert!(crate::ICON_GROUP_LIGHT > crate::ICON_GROUP_EXE);
        assert_eq!(about_icon_group(false), crate::ICON_GROUP_LIGHT);
        assert_eq!(about_icon_group(true), crate::ICON_GROUP_DARK);
        assert_eq!(crate::window_icon_group(false), about_icon_group(false));
        assert_eq!(crate::window_icon_group(true), about_icon_group(true));
    }

    #[test]
    fn about_layout_stacks_the_ok_button_under_the_text() {
        for dpi in [96, 144, 192] {
            let cw = sc(dpi, ABOUT_W_DIP);
            let desc_h = sc(dpi, 20);
            let lic_h = sc(dpi, 18);
            let repo_h = sc(dpi, 18);
            let ch = about_client_h(dpi, desc_h, lic_h, repo_h);
            let lay = about_lay(cw, ch, dpi, desc_h, lic_h, repo_h);
            let m = sc(dpi, 16);
            assert_eq!(lay.icon.left, m);
            assert_eq!(lay.icon.top, m);
            assert_eq!(lay.icon.right - lay.icon.left, sc(dpi, ABOUT_ICON_DIP));
            assert_eq!(lay.icon.bottom - lay.icon.top, sc(dpi, ABOUT_ICON_DIP));
            assert!(lay.title.left >= lay.icon.right);
            assert_eq!(lay.title.top, lay.icon.top);
            assert!(lay.version.top >= lay.title.bottom);
            assert!(lay.version.bottom <= lay.icon.bottom);
            assert_eq!(lay.desc.top, lay.icon.bottom + m);
            assert_eq!(lay.desc.bottom - lay.desc.top, desc_h);
            assert_eq!(lay.license.top, lay.desc.bottom + sc(dpi, 8));
            assert_eq!(lay.repo.top, lay.license.bottom + sc(dpi, 4));
            assert_eq!(lay.repo.bottom - lay.repo.top, repo_h);
            assert_eq!(lay.ok.top, lay.repo.bottom + m);
            assert_eq!(lay.ok.right, cw - m);
            assert_eq!(ch - lay.ok.bottom, m);
            assert_eq!(lay.ok.bottom - lay.ok.top, sc(dpi, ABOUT_BTN_H_DIP));
            assert_eq!(lay.ok.right - lay.ok.left, sc(dpi, ABOUT_BTN_W_DIP));
            assert!(lay.desc.left == m && lay.license.left == m && lay.repo.left == m);
            assert!(lay.ok.top >= lay.repo.bottom);
        }
    }
    

    fn span(line: &[i32], gap: i32) -> i32 {
        if line.is_empty() {
            0
        } else {
            line.iter().sum::<i32>() + gap * (line.len() as i32 - 1)
        }
    }

    fn column_w(client: i32, bar: bool) -> i32 {
        let nav = sc(96, 220);
        let pad = sc(96, 28);
        let right_pad = if bar { sc(96, 18) } else { 0 };
        let row = RECT { left: nav + pad, top: 0, right: client - pad - right_pad, bottom: 0 };
        chord_box(row, 96).1
    }

    #[test]
    fn shortcut_chords_wrap_inside_the_cell() {
        let gap = chord_gap(96);
        // Four Alt+Shift arrow chips, about 110px each in Consolas 12.
        let arrows = [110, 110, 110, 110];
        for (client, bar) in [(1000, false), (1000, true), (780, true)] {
            let max_w = column_w(client, bar);
            let lines = pack_chords(&arrows, max_w, gap);
            assert!(lines.len() >= 2, "client {client} bar {bar} max {max_w} stayed one line");
            assert_eq!(lines.iter().map(|l| l.len()).sum::<usize>(), 4);
            for line in &lines {
                assert!(span(line, gap) <= max_w, "line {line:?} exceeds {max_w}");
            }
            assert!(key_row_h_from_lines(lines.len() as i32, 96) > sc(96, 40));
        }
        // A chord wider than the cell is clamped to the cell.
        assert_eq!(pack_chords(&[900], 200, gap), vec![vec![200]]);
        // Short chords stay on one line and keep the 40px row.
        assert_eq!(pack_chords(&[70, 60, 40], column_w(1000, false), gap).len(), 1);
        assert_eq!(key_row_h_from_lines(1, 96), sc(96, 40));
        assert!(key_row_h_from_lines(2, 96) > key_row_h_from_lines(1, 96));
    }

    #[test]
    fn ask_prompt_matches_the_name_field_mockup() {
        assert_eq!(crate::BG_LIGHT, 0xF3F1EC);
        assert_eq!(crate::BG_DARK, 0x1C1D20);
        assert_eq!(crate::FIELD_LIGHT, 0xFFFFFF);
        assert_eq!(crate::FIELD_DARK, 0x1A1B1E);
        assert_eq!(crate::ACCENT, 0x0A9BD6);
        assert_ne!(crate::ACCENT, 0x2EC4A6);
        let ground = crate::BG_LIGHT;
        let rest = crate::button_colors(ask_button_kind(ASK_OK), crate::ButtonState::Normal, false, ground);
        let hot = crate::button_colors(ask_button_kind(ASK_OK), crate::ButtonState::Hover, false, ground);
        let down = crate::button_colors(ask_button_kind(ASK_OK), crate::ButtonState::Pressed, false, ground);
        assert_eq!(ask_button_kind(ASK_OK), crate::ButtonKind::Primary);
        assert_eq!(ask_button_kind(ASK_CANCEL), crate::ButtonKind::Secondary);
        assert_eq!(rest.fill, Some(crate::ACCENT));
        assert_eq!(rest.ink, 0xFFFFFF);
        assert_eq!(hot.fill, Some(crate::ACCENT_HOVER_LIGHT));
        assert_eq!(down.fill, Some(crate::ACCENT_PRESSED));
        assert_ne!(hot.fill, down.fill);
        let sec = crate::button_colors(ask_button_kind(ASK_CANCEL), crate::ButtonState::Normal, false, ground);
        assert_eq!(sec.fill, Some(crate::CARD_LIGHT));
        assert_eq!(sec.ink, crate::TEXT_LIGHT);
        let dark_ground = crate::BG_DARK;
        let dark_hot = crate::button_colors(ask_button_kind(ASK_OK), crate::ButtonState::Hover, true, dark_ground);
        let dark_down = crate::button_colors(ask_button_kind(ASK_OK), crate::ButtonState::Pressed, true, dark_ground);
        assert_eq!(dark_hot.fill, Some(crate::ACCENT_HOVER_DARK));
        assert_eq!(dark_down.fill, Some(crate::ACCENT_PRESSED));
        assert_ne!(dark_hot.fill, Some(crate::ACCENT));
        let dark_sec = crate::button_colors(ask_button_kind(ASK_CANCEL), crate::ButtonState::Normal, true, dark_ground);
        assert_eq!(dark_sec.fill, Some(crate::CARD_DARK));
        assert_eq!(dark_sec.ink, crate::TEXT_DARK);
        for c in [rest.fill, hot.fill, down.fill, dark_hot.fill, dark_down.fill].into_iter().flatten() {
            assert_ne!(c, 0x2EC4A6);
        }
        assert_eq!(ask_btn_w(10, 96), sc(96, crate::BTN_MIN_W));
        assert_eq!(ask_btn_w(80, 96), 80 + 2 * sc(96, crate::BTN_PAD_X));
        for dpi in [96, 120, 144, 192] {
            let cw = sc(dpi, ASK_W_DIP);
            let ch = ask_client_h(dpi);
            let ok_w = sc(dpi, 80);
            let cancel_w = sc(dpi, 96);
            let lay = ask_lay(cw, dpi, ok_w, cancel_w);
            let mx = sc(dpi, crate::PAD_BODY_X);
            let my = sc(dpi, crate::PAD_BODY_Y);
            let inset_d = sc(dpi, FIELD_INSET_DIP);
            assert_eq!(lay.label.top, my);
            assert_eq!(lay.label.left, mx);
            assert_eq!(cw - lay.field.right, mx);
            assert_eq!(lay.label.bottom - lay.label.top, sc(dpi, ASK_LABEL_H_DIP));
            assert_eq!(lay.field.top - lay.label.bottom, sc(dpi, ASK_LABEL_GAP_DIP));
            assert_eq!(lay.field.bottom - lay.field.top, sc(dpi, crate::FIELD_H));
            assert_eq!(lay.field.left, mx);
            assert_eq!(lay.edit.top, lay.field.top + inset_d);
            assert_eq!(lay.edit.bottom, lay.field.bottom - inset_d);
            assert_eq!((lay.edit.left, lay.edit.right), (lay.field.left + inset_d, lay.field.right - inset_d));
            assert_eq!(lay.rule.top, lay.field.bottom + sc(dpi, crate::SPACE_12));
            assert_eq!(lay.rule.bottom - lay.rule.top, 1);
            assert_eq!(lay.rule.left, mx);
            assert_eq!(lay.ok.top, lay.rule.bottom + sc(dpi, crate::SPACE_12));
            assert_eq!(lay.ok.bottom - lay.ok.top, sc(dpi, crate::BTN_H_FOOTER));
            assert_eq!(lay.cancel.bottom - lay.cancel.top, sc(dpi, crate::BTN_H_FOOTER));
            assert_eq!(lay.cancel.top, lay.ok.top);
            assert_eq!(lay.ok.right, lay.field.right);
            assert_eq!(lay.ok.right - lay.ok.left, ok_w);
            assert_eq!(lay.cancel.right - lay.cancel.left, cancel_w);
            assert_eq!(lay.ok.left - lay.cancel.right, sc(dpi, crate::BTN_GAP));
            assert_eq!(ch - lay.ok.bottom, my);
            assert!(lay.cancel.left >= mx);
            assert!(lay.ok.top >= lay.field.bottom);
            for px in [15, 16, 19] {
                let line = (px * dpi / 96).max(1);
                let text = crate::centered_edit_rect(lay.edit, line);
                let slot_h = lay.edit.bottom - lay.edit.top;
                let used = line.min(slot_h);
                assert_eq!(text.bottom - text.top, used);
                assert_eq!(text.top, lay.edit.top + (slot_h - used) / 2);
                assert!(text.top >= lay.field.top && text.bottom <= lay.field.bottom);
                assert!(text.bottom - text.top < lay.field.bottom - lay.field.top);
            }
        }
        // A pair wider than the row still shares the gap and stays inside the pads.
        let narrow = ask_lay(sc(96, 160), 96, 200, 200);
        let mx = sc(96, crate::PAD_BODY_X);
        assert!(narrow.cancel.left >= mx);
        assert_eq!(narrow.ok.right, sc(96, 160) - mx);
        assert_eq!(narrow.ok.left - narrow.cancel.right, sc(96, crate::BTN_GAP));
        assert!(narrow.ok.right > narrow.cancel.right);
    }

    fn point_in_capsule(x: f32, y: f32, w: f32, h: f32) -> bool {
        let rad = h / 2.0;
        if y < 0.0 || y > h {
            return false;
        }
        if x >= rad && x <= w - rad {
            return true;
        }
        let cx = if x < rad { rad } else { w - rad };
        let (dx, dy) = (x - cx, y - rad);
        dx * dx + dy * dy <= rad * rad + 0.01
    }

    #[test]
    fn toggle_knob_stays_inside_the_capsule_at_96_120_144() {
        for dpi in [96, 120, 144] {
            let w = sc(dpi, 40);
            let h = sc(dpi, 20);
            let r = RECT { left: 0, top: 0, right: w, bottom: h };
            for on in [false, true] {
                let k = crate::toggle_knob_rect(r, on, dpi);
                assert_eq!(k.right - k.left, sc(dpi, 12));
                assert_eq!(k.bottom - k.top, sc(dpi, 12));
                assert!(k.left >= r.left && k.right <= r.right && k.top >= r.top && k.bottom <= r.bottom);
                for (x, y) in [(k.left, k.top), (k.right - 1, k.top), (k.left, k.bottom - 1), (k.right - 1, k.bottom - 1)] {
                    assert!(point_in_capsule(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32), "dpi {dpi} on {on} at {x},{y}");
                }
            }
            // The blit pad stays on the card (16 DIP) and inside the segment gap (8 DIP).
            assert!(crate::AA_PAD < sc(dpi, 16));
            assert!(crate::AA_PAD * 2 < sc(dpi, 8));
        }
    }

    #[test]
    fn box_down_keeps_a_flat_fill_and_blends_an_edge() {
        let scale = 3usize;
        let (sw, sh) = (6usize, 3usize);
        let mut src = vec![0u8; sw * sh * 4];
        for y in 0..sh {
            for x in 0..sw {
                let i = (y * sw + x) * 4;
                if x < 3 {
                    src[i + 2] = 255;
                } else {
                    src[i] = 255;
                }
                src[i + 3] = 255;
            }
        }
        let mut dst = vec![0u8; 8];
        crate::box_down(&src, sw, &mut dst, 2, 1, scale);
        assert_eq!(&dst[0..4], &[0, 0, 255, 255]);
        assert_eq!(&dst[4..8], &[255, 0, 0, 255]);
        for y in 0..sh {
            let i = (y * sw + 3) * 4;
            src[i] = 0;
            src[i + 2] = 255;
        }
        crate::box_down(&src, sw, &mut dst, 2, 1, scale);
        assert_eq!(dst[4], (255u32 * 6 / 9) as u8);
        assert_eq!(dst[5], 0);
        assert_eq!(dst[6], (255u32 * 3 / 9) as u8);
        assert_eq!(dst[7], 255);
    }
}