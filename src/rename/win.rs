// Separate window for the selected items. Caption, cards, fields, checks, and
// buttons come from the shared painters. Rules sit on the left and the preview
// on the right. A single-line edit is only as tall as its font cell and is
// centred in a field that matches the combo beside it. The field is one 1px
// border: the token line at rest, cerulean when focused.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Once;

use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
use windows::Win32::UI::HiDpi::GetDpiForWindow;

use crate::imports::*;
use crate::tf;
use crate::tr;

use super::engine::{self, BatchItem, CaseMode, DatePos, DateSrc, Issue, NumPos, PreviewRow, Rule, TrimEdge};
use super::exec;
use super::presets::{self, Preset};

const ID_ADD: i32 = 1;
const ID_CLOSE: i32 = 2;
const ID_APPLY: i32 = 3;
const ID_UNDO: i32 = 4;
const ID_FILTER: i32 = 5;
const ID_EXT: i32 = 6;
const ID_PRESET: i32 = 7;
const ID_SAVE: i32 = 8;
const ID_DELPRE: i32 = 9;
const ID_BASE: i32 = 100;
const STRIDE: i32 = 16;
const MAX_RULES: usize = 24;

/// Outer size. Wider than the old stacked layout so the rules and the preview
/// can share the window side by side, about half the client each.
const DLG_W_DIP: i32 = 1360;
const DLG_H_DIP: i32 = 700;
const DLG_MIN_W_DIP: i32 = 1040;
const DLG_MIN_H_DIP: i32 = 520;
/// Neither column shrinks below this. Width past the two minimums is split evenly.
const COL_MIN_DIP: i32 = 420;
const COL_GAP_DIP: i32 = crate::SPACE_12;
/// Combo, button and edit-frame height on a rule card and in the toolbar.
const FIELD_H_DIP: i32 = crate::FIELD_H;
/// Space inside a rule card, and the gap before the next card.
const CARD_PAD_DIP: i32 = 10;
const CARD_GAP_DIP: i32 = crate::SPACE_8;
const ROW_GAP_DIP: i32 = crate::SPACE_8;
/// Shared draw_tick box. The kind combo matches the mockup's 150 DIP.
const TICK_DIP: i32 = 16;
const KIND_W_DIP: i32 = 150;
const MINI_W_DIP: i32 = 28;
/// Cerulean brand (#0A9BD6). Pressed primary is #0786BC. Changed text on the dark theme is #7FD4F5.
/// Mint (#2EC4A6) is not part of this window. Hover is a separate step in the shared button tokens.
const CERULEAN: u32 = crate::ACCENT;
const CERULEAN_DEEP: u32 = crate::ACCENT_PRESSED;
const MARK_INK_DARK: u32 = 0x7FD4F5;
const ROW_HOVER_LIGHT: u32 = 0xF7F5F1;
const ROW_HOVER_DARK: u32 = 0x2A2C30;

const F_KIND: i32 = 0;
const F_A: i32 = 1;
const F_B: i32 = 2;
const F_CASE: i32 = 3;
const F_EXT: i32 = 4;
const F_UP: i32 = 5;
const F_DOWN: i32 = 6;
const F_DEL: i32 = 7;
const F_MODE: i32 = 8;
const F_POS: i32 = 9;
const F_C: i32 = 10;
const F_SRC: i32 = 11;
const F_AT: i32 = 12;
const F_ON: i32 = 13;

const CB_RESETCONTENT: u32 = 0x014B;
const CB_ADDSTRING: u32 = 0x0143;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const CBN_SELCHANGE: u32 = 1;
const EN_CHANGE: u32 = 0x0300;
const EN_SETFOCUS: u32 = 0x0100;
const EN_KILLFOCUS: u32 = 0x0200;
const BN_CLICKED: u32 = 0;
const EM_SETCUEBANNER: u32 = 0x1501;
const EM_SETMARGINS: u32 = 0x00D3;

fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi.max(96) / 96
}

fn tx(t: crate::Key) -> &'static str {
    tr(t)
}

struct St {
    hwnd: HWND,
    host: HWND,
    dpi: i32,
    rules: Vec<Rule>,
    enabled: Vec<bool>,
    items: Vec<BatchItem>,
    existing: HashSet<(String, String)>,
    rows: Vec<PreviewRow>,
    rule_scroll: i32,
    preview_scroll: i32,
    filling: bool,
    busy: bool,
    note: String,
    /// Names to write onto `items` when the worker reports success. Parallel to `items`.
    pending: Vec<String>,
    pending_note: String,
    clear_rules_on_ok: bool,
    pending_undo: Vec<(String, String, String)>,
    drag_from: Option<usize>,
    drag_to: usize,
    hover: Option<usize>,
}

impl Default for St {
    fn default() -> Self {
        Self {
            hwnd: HWND::default(),
            host: HWND::default(),
            dpi: 96,
            rules: vec![Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false }],
            enabled: vec![true],
            items: Vec::new(),
            existing: HashSet::new(),
            rows: Vec::new(),
            rule_scroll: 0,
            preview_scroll: 0,
            filling: false,
            busy: false,
            note: String::new(),
            pending: Vec::new(),
            pending_note: String::new(),
            clear_rules_on_ok: false,
            pending_undo: Vec::new(),
            drag_from: None,
            drag_to: 0,
            hover: None,
        }
    }
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
}

fn hwnd() -> HWND {
    ST.with(|s| s.borrow().hwnd)
}

pub(crate) unsafe fn open_active() {
    open_pane(crate::active_pane());
}

/// Open batch rename for the selection in pane `i` (not whatever `active` happens to be).
pub(crate) unsafe fn open_pane(i: usize) {
    if crate::pane::recent::locked(i) {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    open_items(collect_pane(i));
}

pub(crate) unsafe fn open_paths(_owner: HWND, items: &[(String, bool)]) {
    let batch = items.iter().filter_map(|(p, file)| item_from_path(p, !file)).collect();
    open_items(batch);
}

pub(crate) fn menu_label() -> String {
    let name = tr(crate::Key::BatchRename2);
    let chord = crate::keys::index("file.rename").map(crate::keys::chord_text).unwrap_or_default();
    if chord.is_empty() {
        name.to_string()
    } else {
        format!("{name}\t{chord}")
    }
}

unsafe fn open_items(items: Vec<BatchItem>) {
    if items.is_empty() {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    ensure_class();
    let existing = load_existing(&items);
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        ST.with(|s| {
            let mut st = s.borrow_mut();
            st.items = items;
            st.existing = existing;
            st.preview_scroll = 0;
            st.note.clear();
        });
        recompute();
        let _ = ShowWindow(h, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(h);
        return;
    }
    let owner = crate::app(|a| a.main);
    let dpi = if owner.0.is_null() { 96 } else { GetDpiForWindow(owner) as i32 };
    let (ww, hh) = (sc(dpi.max(96), DLG_W_DIP), sc(dpi.max(96), DLG_H_DIP));
    let mut orc = RECT::default();
    if !owner.0.is_null() {
        let _ = GetWindowRect(owner, &mut orc);
    }
    let x = orc.left + ((orc.right - orc.left - ww) / 2).max(0);
    let y = orc.top + ((orc.bottom - orc.top - hh) / 2).max(0);
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE;
    let h = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("MaguroRename"),
        &HSTRING::from(tx(crate::Key::BatchRename)),
        style,
        x,
        y,
        ww,
        hh,
        (!owner.0.is_null()).then_some(owner),
        None,
        None,
        None,
    )
    .unwrap_or_default();
    if h.0.is_null() {
        return;
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.items = items;
        st.existing = existing;
    });
    recompute();
}

pub(crate) unsafe fn close() {
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = DestroyWindow(h);
    }
}

pub(crate) unsafe fn captures(h: HWND) -> bool {
    let w = hwnd();
    !w.0.is_null() && IsWindow(Some(w)).as_bool() && (h == w || IsChild(w, h).as_bool())
}

pub(crate) unsafe fn refresh() {
    let h = hwnd();
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    style_window(h);
    let font = crate::body_font();
    apply_font(h, font);
    let host = ST.with(|s| s.borrow().host);
    if !host.0.is_null() {
        apply_font(host, font);
    }
    let _ = SetWindowTextW(h, &HSTRING::from(tx(crate::Key::BatchRename)));
    set_btn(h, ID_ADD, tx(crate::Key::AddRule));
    set_btn(h, ID_CLOSE, tx(crate::Key::Close));
    set_btn(h, ID_APPLY, tx(crate::Key::Rename));
    set_btn(h, ID_UNDO, tx(crate::Key::Undo));
    set_btn(h, ID_SAVE, tx(crate::Key::Save));
    set_btn(h, ID_DELPRE, tx(crate::Key::DeletePreset));
    ST.with(|s| s.borrow_mut().filling = true);
    refill_filter();
    cue(ctl(h, ID_EXT), "jpg, png");
    cue(ctl(h, ID_PRESET), tx(crate::Key::PresetName));
    let n = ST.with(|s| s.borrow().rules.len());
    ST.with(|s| s.borrow_mut().filling = true);
    for i in 0..n {
        refill_kind(i);
        refill_mode(i);
        let rule = ST.with(|s| s.borrow().rules.get(i).cloned());
        if let Some(rule) = rule {
            write_rule(i, &rule);
        }
    }
    ST.with(|s| s.borrow_mut().filling = false);
    layout(h);
    sync_buttons();
    let _ = InvalidateRect(Some(h), None, true);
    crate::ui::combo::invalidate_in(h);
    if !host.0.is_null() {
        let _ = InvalidateRect(Some(host), None, true);
        crate::ui::combo::invalidate_in(host);
    }
}

/// Esc closes. Ctrl+Enter renames. Ctrl+Z undoes the latest batch. Alt+Up/Down moves the focused rule.
pub(crate) unsafe fn on_key(vk: u16, ctrl: bool, alt: bool, shift: bool) -> bool {
    let key = VIRTUAL_KEY(vk);
    if !ctrl && !alt && key == VK_ESCAPE {
        close();
        return true;
    }
    if ctrl && !alt && !shift && key == VK_RETURN {
        apply();
        return true;
    }
    if ctrl && !alt && !shift && vk == 0x5A {
        undo();
        return true;
    }
    if alt && !ctrl && !shift && (key == VK_UP || key == VK_DOWN) {
        if let Some(i) = focused_rule() {
            let field = focused_field();
            let dir = if key == VK_UP { -1 } else { 1 };
            move_rule(i, dir);
            let j = i as isize + dir;
            let n = ST.with(|s| s.borrow().rules.len()) as isize;
            if (0..n).contains(&j) {
                if let Some(f) = field {
                    let host = ST.with(|s| s.borrow().host);
                    let _ = SetFocus(Some(ctl(host, id(j as usize, f))));
                }
            }
            return true;
        }
    }
    if !ctrl && !alt && matches!(key, VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT) && focused_rule().is_none() {
        let dir = if key == VK_UP || key == VK_PRIOR { -1 } else { 1 };
        let step = if key == VK_PRIOR || key == VK_NEXT { 8 } else { 1 };
        let n = ST.with(|s| s.borrow().rows.len());
        let max = preview_max(n);
        let next = (ST.with(|s| s.borrow().preview_scroll) + dir * step).clamp(0, max);
        ST.with(|s| s.borrow_mut().preview_scroll = next);
        let _ = InvalidateRect(Some(hwnd()), None, false);
        return true;
    }
    false
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(rename_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(crate::ICON_GROUP_LIGHT as _)).unwrap_or_default(),
            lpszClassName: w!("MaguroRename"),
            ..Default::default()
        });
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(host_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroRenameRules"),
            ..Default::default()
        });
    });
}

unsafe extern "system" fn rename_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { rename_proc_inner(h, msg, wp, lp) }))
}
unsafe fn rename_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let dpi = GetDpiForWindow(h) as i32;
            ST.with(|s| {
                let mut st = s.borrow_mut();
                let items = std::mem::take(&mut st.items);
                let existing = std::mem::take(&mut st.existing);
                *st = St::default();
                st.hwnd = h;
                st.dpi = dpi.max(96);
                st.items = items;
                st.existing = existing;
            });
            build_shell(h);
            rebuild_rule_controls();
            style_window(h);
            layout(h);
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
            m.ptMinTrackSize.x = sc(dpi, DLG_MIN_W_DIP);
            m.ptMinTrackSize.y = sc(dpi, DLG_MIN_H_DIP);
            return LRESULT(0);
        }
        WM_PAINT => paint(h),
        WM_ERASEBKGND => return LRESULT(1),
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => return ctl_color(wp, lp),
        WM_DRAWITEM => {
            draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
            return LRESULT(1);
        }
        WM_COMMAND => on_command(h, (wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32),
        WM_MOUSEWHEEL => on_wheel(((wp.0 >> 16) as i16 as i32) / 120),
        WM_MOUSEMOVE => hover_at(lp),
        exec::WM_RENAME_DONE => on_done(h),
        WM_DESTROY => {
            ST.with(|s| {
                let mut st = s.borrow_mut();
                st.hwnd = HWND::default();
                st.host = HWND::default();
            });
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

unsafe extern "system" fn host_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { host_proc_inner(h, msg, wp, lp) }))
}
unsafe fn host_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => paint_host(h),
        WM_ERASEBKGND => return LRESULT(1),
        WM_DRAWITEM => {
            draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
            return LRESULT(1);
        }
        WM_COMMAND => {
            on_command(hwnd(), (wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32);
            return LRESULT(0);
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => return ctl_color(wp, lp),
        WM_MOUSEWHEEL => {
            on_wheel(((wp.0 >> 16) as i16 as i32) / 120);
            return LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            begin_drag(h, lp);
            return LRESULT(0);
        }
        WM_MOUSEMOVE => drag_move(h, lp),
        WM_LBUTTONUP => {
            end_drag(h);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

unsafe fn ctl_color(wp: WPARAM, lp: LPARAM) -> LRESULT {
    let dc = HDC(wp.0 as _);
    let ctl = HWND(lp.0 as _);
    let off = control_rule(ctl).is_some_and(|i| !rule_is_on(i));
    if off {
        let t = crate::ui_tokens();
        return crate::color_field_ink(dc, crate::blend(t.field_bg, t.text, 0.55));
    }
    crate::color_field_edit(dc)
}

unsafe fn build_shell(h: HWND) {
    let font = crate::body_font();
    let tab = 0x00010000u32;
    let btn = |id, label| {
        let c = child(h, w!("BUTTON"), label, 0xB | tab, id, font);
        theme_btn(c);
        c
    };
    btn(ID_ADD, tx(crate::Key::AddRule));
    btn(ID_CLOSE, tx(crate::Key::Close));
    btn(ID_APPLY, tx(crate::Key::Rename));
    btn(ID_UNDO, tx(crate::Key::Undo));
    btn(ID_SAVE, tx(crate::Key::Save));
    btn(ID_DELPRE, tx(crate::Key::DeletePreset));
    let list = |id| {
        let c = child(h, w!("COMBOBOX"), "", crate::ui::combo::LIST | tab, id, font);
        crate::ui::combo::adopt(c, combo_face);
        c
    };
    let drop = |id| {
        let c = child(h, w!("COMBOBOX"), "", crate::ui::combo::DROPDOWN | tab, id, font);
        crate::ui::combo::adopt(c, combo_face);
        c
    };
    list(ID_FILTER);
    let ext = child(h, w!("EDIT"), "", 0x80 | tab, ID_EXT, font);
    theme_ctl(ext);
    let m = sc(ST.with(|s| s.borrow().dpi), crate::FIELD_PAD_X - crate::FIELD_INSET) as isize;
    let _ = SendMessageW(ext, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(m | (m << 16))));
    drop(ID_PRESET);
    refill_filter();
    refill_presets(None);
    cue(ext, "jpg, png");
    cue(ctl(h, ID_PRESET), tx(crate::Key::PresetName));
    let host = child(h, w!("MaguroRenameRules"), "", 0x02000000 | 0x04000000, 50, font);
    ST.with(|s| s.borrow_mut().host = host);
}

unsafe fn rebuild_rule_controls() {
    let host = ST.with(|s| s.borrow().host);
    if host.0.is_null() {
        return;
    }
    for i in 0..MAX_RULES {
        for f in 0..STRIDE {
            let c = ctl(host, id(i, f));
            if !c.0.is_null() {
                let _ = DestroyWindow(c);
            }
        }
    }
    let n = ST.with(|s| s.borrow().rules.len());
    let font = crate::body_font();
    ST.with(|s| s.borrow_mut().filling = true);
    for i in 0..n {
        make_rule_controls(host, i, font);
        refill_kind(i);
        refill_mode(i);
        let rule = ST.with(|s| s.borrow().rules.get(i).cloned());
        if let Some(rule) = rule {
            write_rule(i, &rule);
        }
    }
    ST.with(|s| s.borrow_mut().filling = false);
}

unsafe fn make_rule_controls(host: HWND, i: usize, font: HFONT) {
    let tab = 0x00010000u32;
    let combo = |f| {
        let c = child(host, w!("COMBOBOX"), "", crate::ui::combo::LIST | tab, id(i, f), font);
        crate::ui::combo::adopt(c, combo_face);
        c
    };
    let edit = |f| {
        let c = child(host, w!("EDIT"), "", 0x80 | tab, id(i, f), font);
        theme_ctl(c);
        let m = sc(ST.with(|s| s.borrow().dpi), crate::FIELD_PAD_X - crate::FIELD_INSET) as isize;
        let _ = SendMessageW(c, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(m | (m << 16))));
        c
    };
    let check = |f, label: &str| {
        // Owner-draw so the box is the shared paint_check, not a native checkbox.
        let c = child(host, w!("BUTTON"), label, 0xB | tab, id(i, f), font);
        theme_btn(c);
        c
    };
    let mini = |f, label: &str| {
        let c = child(host, w!("BUTTON"), label, 0xB | tab, id(i, f), font);
        theme_btn(c);
        c
    };
    combo(F_KIND);
    edit(F_A);
    edit(F_B);
    edit(F_C);
    edit(F_AT);
    check(F_CASE, tx(crate::Key::MatchCase));
    check(F_EXT, tx(crate::Key::IncludeExtension));
    combo(F_MODE);
    combo(F_POS);
    combo(F_SRC);
    check(F_ON, "");
    mini(F_UP, "↑");
    mini(F_DOWN, "↓");
    mini(F_DEL, "×");
}

unsafe fn refill_kind(i: usize) {
    let host = ST.with(|s| s.borrow().host);
    let c = ctl(host, id(i, F_KIND));
    if c.0.is_null() {
        return;
    }
    let cur = SendMessageW(c, CB_GETCURSEL, None, None).0;
    let _ = SendMessageW(c, CB_RESETCONTENT, None, None);
    for label in [
        tx(crate::Key::ReplaceText),
        tx(crate::Key::AddPrefix),
        tx(crate::Key::AddSuffix),
        tx(crate::Key::ChangeCase),
        tx(crate::Key::ChangeExtension),
        tx(crate::Key::Number),
        tx(crate::Key::EmlDate),
        tx(crate::Key::Insert),
        tx(crate::Key::DeleteCharacters),
        tx(crate::Key::Trim),
        tx(crate::Key::RegularExpression),
    ] {
        let hs = HSTRING::from(label);
        let _ = SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
    let rule = ST.with(|s| s.borrow().rules.get(i).cloned());
    let sel = rule.as_ref().map(kind_index).unwrap_or(cur.max(0) as usize) as isize;
    let _ = SendMessageW(c, CB_SETCURSEL, Some(WPARAM(sel as usize)), None);
}

unsafe fn refill_mode(i: usize) {
    let host = ST.with(|s| s.borrow().host);
    let c = ctl(host, id(i, F_MODE));
    if c.0.is_null() {
        return;
    }
    let _ = SendMessageW(c, CB_RESETCONTENT, None, None);
    for label in [
        tx(crate::Key::Lowercase),
        tx(crate::Key::Uppercase),
        tx(crate::Key::TitleCase),
        tx(crate::Key::Sentence),
    ] {
        let hs = HSTRING::from(label);
        let _ = SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
}

unsafe fn fill_combo(host: HWND, i: usize, field: i32, labels: &[&str]) {
    let c = ctl(host, id(i, field));
    if c.0.is_null() {
        return;
    }
    let _ = SendMessageW(c, CB_RESETCONTENT, None, None);
    for label in labels {
        let hs = HSTRING::from(*label);
        let _ = SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
}

unsafe fn select_combo(host: HWND, i: usize, field: i32, sel: usize) {
    let _ = SendMessageW(ctl(host, id(i, field)), CB_SETCURSEL, Some(WPARAM(sel)), None);
}

unsafe fn write_rule(i: usize, rule: &Rule) {
    let host = ST.with(|s| s.borrow().host);
    let set_edit = |f, text: &str| {
        let c = ctl(host, id(i, f));
        let _ = SetWindowTextW(c, &HSTRING::from(text));
    };
    select_combo(host, i, F_KIND, kind_index(rule));
    match rule {
        Rule::Replace { find, replace, .. } => {
            set_edit(F_A, find);
            set_edit(F_B, replace);
            cue(ctl(host, id(i, F_A)), tx(crate::Key::Find));
            cue(ctl(host, id(i, F_B)), tx(crate::Key::ReplaceWith));
        }
        Rule::Regex { pattern, replace, .. } => {
            set_edit(F_A, pattern);
            set_edit(F_B, replace);
            cue(ctl(host, id(i, F_A)), tx(crate::Key::Pattern));
            cue(ctl(host, id(i, F_B)), tx(crate::Key::Replace1));
        }
        Rule::Prefix { text } => {
            set_edit(F_A, text);
            cue(ctl(host, id(i, F_A)), tx(crate::Key::Prefix));
        }
        Rule::Suffix { text } => {
            set_edit(F_A, text);
            cue(ctl(host, id(i, F_A)), tx(crate::Key::Suffix));
        }
        Rule::Case { mode, .. } => {
            select_combo(host, i, F_MODE, mode_index(*mode));
        }
        Rule::Extension { ext } => {
            set_edit(F_A, ext);
            cue(ctl(host, id(i, F_A)), tx(crate::Key::NewExtension));
        }
        Rule::Number { start, step, pad, pos } => {
            set_edit(F_A, &start.to_string());
            set_edit(F_B, &step.to_string());
            set_edit(F_C, &pad.to_string());
            cue(ctl(host, id(i, F_A)), tx(crate::Key::Start));
            cue(ctl(host, id(i, F_B)), tx(crate::Key::Step));
            cue(ctl(host, id(i, F_C)), tx(crate::Key::Digits));
            fill_combo(host, i, F_POS, &[
                tx(crate::Key::BeforeName),
                tx(crate::Key::AfterName),
                tx(crate::Key::AtPosition),
            ]);
            let (sel, at) = match pos {
                NumPos::Prefix => (0, String::new()),
                NumPos::Suffix => (1, String::new()),
                NumPos::At(n) => (2, n.to_string()),
            };
            select_combo(host, i, F_POS, sel);
            set_edit(F_AT, &at);
            cue(ctl(host, id(i, F_AT)), tx(crate::Key::K0Start1End));
        }
        Rule::Date { src, pattern, pos } => {
            fill_combo(host, i, F_SRC, &[
                tx(crate::Key::Modified),
                tx(crate::Key::Created),
                tx(crate::Key::Accessed),
            ]);
            select_combo(host, i, F_SRC, match src {
                DateSrc::Modified => 0,
                DateSrc::Created => 1,
                DateSrc::Accessed => 2,
            });
            set_edit(F_A, pattern);
            cue(ctl(host, id(i, F_A)), "YYYY-MM-DD hh:mm:ss");
            fill_combo(host, i, F_POS, &[
                tx(crate::Key::BeforeName),
                tx(crate::Key::AfterName),
                tx(crate::Key::ReplaceName),
            ]);
            select_combo(host, i, F_POS, match pos {
                DatePos::Prefix => 0,
                DatePos::Suffix => 1,
                DatePos::Replace => 2,
            });
        }
        Rule::Insert { text, at } => {
            set_edit(F_A, text);
            set_edit(F_AT, &at.to_string());
            cue(ctl(host, id(i, F_A)), tx(crate::Key::Text));
            cue(ctl(host, id(i, F_AT)), tx(crate::Key::K0Start1End));
        }
        Rule::Delete { at, len } => {
            set_edit(F_AT, &at.to_string());
            set_edit(F_C, &len.to_string());
            cue(ctl(host, id(i, F_AT)), tx(crate::Key::K0Start1End));
            cue(ctl(host, id(i, F_C)), tx(crate::Key::PropLength));
        }
        Rule::Trim { chars, edge } => {
            set_edit(F_A, chars);
            cue(ctl(host, id(i, F_A)), tx(crate::Key::CharactersEmptySpaces));
            fill_combo(host, i, F_POS, &[
                tx(crate::Key::BothEnds),
                tx(crate::Key::Left),
                tx(crate::Key::Right),
            ]);
            select_combo(host, i, F_POS, match edge {
                TrimEdge::Both => 0,
                TrimEdge::Left => 1,
                TrimEdge::Right => 2,
            });
        }
    }
}

unsafe fn read_rule(i: usize) -> Option<Rule> {
    let host = ST.with(|s| s.borrow().host);
    let kind = SendMessageW(ctl(host, id(i, F_KIND)), CB_GETCURSEL, None, None).0;
    if kind < 0 {
        return ST.with(|s| s.borrow().rules.get(i).cloned());
    }
    let text = |f| crate::text_of(ctl(host, id(i, f)));
    let on = |f| stored_flag(i, f);
    let sel = |f| SendMessageW(ctl(host, id(i, f)), CB_GETCURSEL, None, None).0;
    Some(match kind {
        1 => Rule::Prefix { text: text(F_A) },
        2 => Rule::Suffix { text: text(F_A) },
        3 => Rule::Case { mode: mode_from(sel(F_MODE)), include_ext: on(F_EXT) },
        4 => Rule::Extension { ext: text(F_A) },
        5 => {
            let pos = match sel(F_POS) {
                1 => NumPos::Suffix,
                2 => NumPos::At(parse_i32(&text(F_AT), 0)),
                _ => NumPos::Prefix,
            };
            Rule::Number { start: parse_i64(&text(F_A), 1), step: parse_i64(&text(F_B), 1), pad: parse_pad(&text(F_C)), pos }
        }
        6 => {
            let src = match sel(F_SRC) {
                1 => DateSrc::Created,
                2 => DateSrc::Accessed,
                _ => DateSrc::Modified,
            };
            let pos = match sel(F_POS) {
                1 => DatePos::Suffix,
                2 => DatePos::Replace,
                _ => DatePos::Prefix,
            };
            Rule::Date { src, pattern: text(F_A), pos }
        }
        7 => Rule::Insert { text: text(F_A), at: parse_i32(&text(F_AT), 0) },
        8 => Rule::Delete { at: parse_i32(&text(F_AT), 0), len: parse_usize(&text(F_C), 1) },
        9 => {
            let edge = match sel(F_POS) {
                1 => TrimEdge::Left,
                2 => TrimEdge::Right,
                _ => TrimEdge::Both,
            };
            Rule::Trim { chars: text(F_A), edge }
        }
        10 => Rule::Regex { pattern: text(F_A), replace: text(F_B), case_sensitive: on(F_CASE), include_ext: on(F_EXT) },
        _ => Rule::Replace { find: text(F_A), replace: text(F_B), case_sensitive: on(F_CASE), include_ext: on(F_EXT) },
    })
}

fn kind_index(rule: &Rule) -> usize {
    match rule {
        Rule::Replace { .. } => 0,
        Rule::Prefix { .. } => 1,
        Rule::Suffix { .. } => 2,
        Rule::Case { .. } => 3,
        Rule::Extension { .. } => 4,
        Rule::Number { .. } => 5,
        Rule::Date { .. } => 6,
        Rule::Insert { .. } => 7,
        Rule::Delete { .. } => 8,
        Rule::Trim { .. } => 9,
        Rule::Regex { .. } => 10,
    }
}

fn default_rule(kind: isize) -> Rule {
    match kind {
        1 => Rule::Prefix { text: String::new() },
        2 => Rule::Suffix { text: String::new() },
        3 => Rule::Case { mode: CaseMode::Lower, include_ext: false },
        4 => Rule::Extension { ext: String::new() },
        5 => Rule::Number { start: 1, step: 1, pad: 0, pos: NumPos::Suffix },
        6 => Rule::Date { src: DateSrc::Modified, pattern: "YYYY-MM-DD".into(), pos: DatePos::Prefix },
        7 => Rule::Insert { text: String::new(), at: 0 },
        8 => Rule::Delete { at: 0, len: 1 },
        9 => Rule::Trim { chars: String::new(), edge: TrimEdge::Both },
        10 => Rule::Regex { pattern: String::new(), replace: String::new(), case_sensitive: false, include_ext: false },
        _ => Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false },
    }
}

/// Empty text and a lone sign stay at the default so a minus can be typed.
fn parse_i64(text: &str, default: i64) -> i64 {
    let t = text.trim();
    if t.is_empty() || t == "-" || t == "+" {
        return default;
    }
    t.parse().unwrap_or(default)
}

fn parse_i32(text: &str, default: i32) -> i32 {
    parse_i64(text, default as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn parse_pad(text: &str) -> u8 {
    parse_i64(text, 0).clamp(0, 16) as u8
}

fn parse_usize(text: &str, default: i64) -> usize {
    parse_i64(text, default).clamp(0, 10_000) as usize
}

fn mode_index(mode: CaseMode) -> usize {
    match mode {
        CaseMode::Lower => 0,
        CaseMode::Upper => 1,
        CaseMode::Title => 2,
        CaseMode::Sentence => 3,
    }
}

fn mode_from(i: isize) -> CaseMode {
    match i {
        1 => CaseMode::Upper,
        2 => CaseMode::Title,
        3 => CaseMode::Sentence,
        _ => CaseMode::Lower,
    }
}

unsafe fn on_command(_h: HWND, id: i32, code: u32) {
    if code == EN_SETFOCUS || code == EN_KILLFOCUS {
        let win = hwnd();
        if !win.0.is_null() {
            let _ = InvalidateRect(Some(win), None, false);
            let host = ST.with(|s| s.borrow().host);
            if !host.0.is_null() {
                let _ = InvalidateRect(Some(host), None, false);
            }
        }
    }
    if ST.with(|s| s.borrow().filling) {
        return;
    }
    match id {
        ID_ADD => add_rule(),
        ID_CLOSE => close(),
        ID_APPLY => apply(),
        ID_UNDO => undo(),
        ID_SAVE if code == BN_CLICKED => save_preset(),
        ID_DELPRE if code == BN_CLICKED => delete_preset(),
        ID_FILTER if code == CBN_SELCHANGE => recompute(),
        ID_EXT if code == EN_CHANGE => recompute(),
        ID_PRESET if code == CBN_SELCHANGE => load_preset(),
        _ if id >= ID_BASE => {
            let i = ((id - ID_BASE) / STRIDE) as usize;
            let f = (id - ID_BASE) % STRIDE;
            if i >= ST.with(|s| s.borrow().rules.len()) {
                return;
            }
            match f {
                F_UP if code == BN_CLICKED => move_rule(i, -1),
                F_DOWN if code == BN_CLICKED => move_rule(i, 1),
                F_DEL if code == BN_CLICKED => remove_rule(i),
                F_KIND if code == CBN_SELCHANGE => change_kind(i),
                F_MODE if code == CBN_SELCHANGE => sync_rule(i),
                F_POS | F_SRC if code == CBN_SELCHANGE => {
                    sync_rule(i);
                    layout(hwnd());
                }
                F_A | F_B | F_C | F_AT if code == EN_CHANGE => sync_rule(i),
                F_CASE | F_EXT if code == BN_CLICKED => {
                    flip_flag(i, f);
                    sync_rule(i);
                }
                F_ON if code == BN_CLICKED => toggle_rule(i),
                _ => {}
            }
        }
        _ => {}
    }
}

unsafe fn sync_rule(i: usize) {
    let Some(rule) = read_rule(i) else { return };
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if let Some(slot) = st.rules.get_mut(i) {
            *slot = rule;
        }
        st.note.clear();
    });
    recompute();
}

unsafe fn change_kind(i: usize) {
    let host = ST.with(|s| s.borrow().host);
    let kind = SendMessageW(ctl(host, id(i, F_KIND)), CB_GETCURSEL, None, None).0;
    let rule = default_rule(kind);
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if let Some(slot) = st.rules.get_mut(i) {
            *slot = rule.clone();
        }
        st.filling = true;
        st.note.clear();
    });
    write_rule(i, &rule);
    ST.with(|s| s.borrow_mut().filling = false);
    layout(hwnd());
    recompute();
}

unsafe fn add_rule() {
    let n = ST.with(|s| s.borrow().rules.len());
    if n >= MAX_RULES {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.rules.push(Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false });
        st.enabled.push(true);
        st.note.clear();
    });
    rebuild_rule_controls();
    let dpi = ST.with(|s| s.borrow().dpi);
    let rules = ST.with(|s| s.borrow().rules.clone());
    let content = rules_span(&rules, dpi);
    let view = host_height();
    let max = (content - view).max(0);
    ST.with(|s| s.borrow_mut().rule_scroll = max);
    layout(hwnd());
    recompute();
    let host = ST.with(|s| s.borrow().host);
    let focus_at = ST.with(|s| s.borrow().rules.len()).saturating_sub(1);
    let _ = SetFocus(Some(ctl(host, id(focus_at, F_A))));
}

unsafe fn remove_rule(i: usize) {
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if i < st.rules.len() {
            st.rules.remove(i);
        }
        if i < st.enabled.len() {
            st.enabled.remove(i);
        }
        st.note.clear();
    });
    rebuild_rule_controls();
    layout(hwnd());
    recompute();
}

fn rule_is_on(i: usize) -> bool {
    ST.with(|s| s.borrow().enabled.get(i).copied().unwrap_or(true))
}

fn rule_enabled(i: usize) -> bool {
    rule_is_on(i)
}

/// Case and extension flags live on the rule. The owner-draw tick paints that state.
fn stored_flag(i: usize, field: i32) -> bool {
    if field == F_ON {
        return rule_is_on(i);
    }
    ST.with(|s| {
        let st = s.borrow();
        match st.rules.get(i) {
            Some(Rule::Replace { case_sensitive, include_ext, .. } | Rule::Regex { case_sensitive, include_ext, .. }) => {
                if field == F_CASE { *case_sensitive } else { *include_ext }
            }
            Some(Rule::Case { include_ext, .. }) => field == F_EXT && *include_ext,
            _ => false,
        }
    })
}

fn flip_flag(i: usize, field: i32) {
    ST.with(|s| {
        let mut st = s.borrow_mut();
        let Some(rule) = st.rules.get_mut(i) else { return };
        match field {
            F_CASE => match rule {
                Rule::Replace { case_sensitive, .. } | Rule::Regex { case_sensitive, .. } => *case_sensitive = !*case_sensitive,
                _ => {}
            },
            F_EXT => match rule {
                Rule::Replace { include_ext, .. } | Rule::Regex { include_ext, .. } | Rule::Case { include_ext, .. } => *include_ext = !*include_ext,
                _ => {}
            },
            _ => {}
        }
    });
}

fn control_rule(h: HWND) -> Option<usize> {
    let id = unsafe { GetDlgCtrlID(h) };
    if id < ID_BASE {
        return None;
    }
    let i = ((id - ID_BASE) / STRIDE) as usize;
    (i < MAX_RULES).then_some(i)
}

unsafe fn toggle_rule(i: usize) {
    ST.with(|s| {
        let mut st = s.borrow_mut();
        let n = st.rules.len();
        if st.enabled.len() < n {
            st.enabled.resize(n, true);
        }
        if let Some(slot) = st.enabled.get_mut(i) {
            *slot = !*slot;
        }
        st.note.clear();
    });
    recompute();
}

unsafe fn focused_rule() -> Option<usize> {
    let focus = GetFocus();
    if focus.0.is_null() {
        return None;
    }
    let host = ST.with(|s| s.borrow().host);
    if focus != host && !IsChild(host, focus).as_bool() {
        return None;
    }
    let id = GetDlgCtrlID(focus);
    if id < ID_BASE {
        return None;
    }
    let i = ((id - ID_BASE) / STRIDE) as usize;
    (i < ST.with(|s| s.borrow().rules.len())).then_some(i)
}

unsafe fn focused_field() -> Option<i32> {
    let focus = GetFocus();
    if focus.0.is_null() {
        return None;
    }
    let id = GetDlgCtrlID(focus);
    (id >= ID_BASE).then_some((id - ID_BASE) % STRIDE)
}

fn lp_point(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

fn card_at_y(y: i32) -> Option<usize> {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let scroll = ST.with(|s| s.borrow().rule_scroll);
    let rules = ST.with(|s| s.borrow().rules.clone());
    let mut top = -scroll;
    for (i, rule) in rules.iter().enumerate() {
        let stride = rule_stride(rule, dpi);
        if y >= top && y < top + stride {
            return Some(i);
        }
        top += stride;
    }
    None
}

fn insert_at_y(y: i32) -> usize {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let scroll = ST.with(|s| s.borrow().rule_scroll);
    let rules = ST.with(|s| s.borrow().rules.clone());
    let mut top = -scroll;
    for (i, rule) in rules.iter().enumerate() {
        let stride = rule_stride(rule, dpi);
        if y < top + stride / 2 {
            return i;
        }
        top += stride;
    }
    rules.len()
}

unsafe fn begin_drag(h: HWND, lp: LPARAM) {
    let (x, y) = lp_point(lp);
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    if x > drag_max_x(dpi) {
        return;
    }
    let Some(i) = card_at_y(y) else { return };
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.drag_from = Some(i);
        st.drag_to = i;
    });
    let _ = SetCapture(h);
}

unsafe fn drag_move(h: HWND, lp: LPARAM) {
    if ST.with(|s| s.borrow().drag_from.is_none()) {
        return;
    }
    let (_, y) = lp_point(lp);
    let to = insert_at_y(y);
    let changed = ST.with(|s| {
        let mut st = s.borrow_mut();
        if st.drag_to == to {
            false
        } else {
            st.drag_to = to;
            true
        }
    });
    if changed {
        let _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn end_drag(h: HWND) {
    let (from, to) = ST.with(|s| {
        let mut st = s.borrow_mut();
        (st.drag_from.take(), st.drag_to)
    });
    let _ = ReleaseCapture();
    if let Some(from) = from {
        reorder_rule(from, to);
    }
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn reorder_rule(from: usize, to: usize) {
    let n = ST.with(|s| s.borrow().rules.len());
    if from >= n || to > n || from == to || from + 1 == to {
        return;
    }
    for i in 0..n {
        if let Some(rule) = read_rule(i) {
            let on = rule_enabled(i);
            ST.with(|s| {
                let mut st = s.borrow_mut();
                if let Some(slot) = st.rules.get_mut(i) {
                    *slot = rule;
                }
                let n = st.rules.len();
                if st.enabled.len() < n {
                    st.enabled.resize(n, true);
                }
                if let Some(slot) = st.enabled.get_mut(i) {
                    *slot = on;
                }
            });
        }
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        let rule = st.rules.remove(from);
        let on = if from < st.enabled.len() { st.enabled.remove(from) } else { true };
        let dest = if to > from { to - 1 } else { to }.min(st.rules.len());
        st.rules.insert(dest, rule);
        let at = dest.min(st.enabled.len());
        st.enabled.insert(at, on);
        st.note.clear();
    });
    rebuild_rule_controls();
    layout(hwnd());
    recompute();
}

unsafe fn hover_at(lp: LPARAM) {
    let (x, y) = lp_point(lp);
    let g = geo_now();
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let head = sc(dpi, 28);
    let row_h = sc(dpi, 36).max(1);
    let next = if x >= g.list.left && x < g.list.right && y >= g.list.top + head && y < g.list.bottom {
        let scroll = ST.with(|s| s.borrow().preview_scroll);
        let i = scroll + (y - g.list.top - head) / row_h;
        let n = ST.with(|s| s.borrow().rows.len()) as i32;
        (i >= 0 && i < n).then_some(i as usize)
    } else {
        None
    };
    let changed = ST.with(|s| {
        let mut st = s.borrow_mut();
        if st.hover == next {
            false
        } else {
            st.hover = next;
            true
        }
    });
    if changed {
        let _ = InvalidateRect(Some(hwnd()), None, false);
    }
}

unsafe fn move_rule(i: usize, dir: isize) {
    let j = i as isize + dir;
    let n = ST.with(|s| s.borrow().rules.len()) as isize;
    if j < 0 || j >= n {
        return;
    }
    // Controls hold the latest text; pull it in before the swap rebuilds them.
    if let Some(rule) = read_rule(i) {
        ST.with(|s| {
            if let Some(slot) = s.borrow_mut().rules.get_mut(i) {
                *slot = rule;
            }
        });
    }
    let on_i = rule_enabled(i);
    let on_j = rule_enabled(j as usize);
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if let Some(slot) = st.enabled.get_mut(i) {
            *slot = on_i;
        }
        if let Some(slot) = st.enabled.get_mut(j as usize) {
            *slot = on_j;
        }
        st.rules.swap(i, j as usize);
        if st.enabled.len() == st.rules.len() {
            st.enabled.swap(i, j as usize);
        }
        st.note.clear();
    });
    rebuild_rule_controls();
    layout(hwnd());
    recompute();
}

unsafe fn apply() {
    if ST.with(|s| s.borrow().busy) {
        return;
    }
    let (pairs, pending, note) = ST.with(|s| {
        let st = s.borrow();
        let mut pairs = Vec::new();
        let mut pending = Vec::new();
        for (it, row) in st.items.iter().zip(st.rows.iter()) {
            pending.push(if row.issues.is_empty() { row.new_name.clone() } else { row.old_name.clone() });
            if !row.skipped && row.issues.is_empty() && row.old_name != row.new_name {
                pairs.push((it.dir.clone(), row.old_name.clone(), row.new_name.clone()));
            }
        }
        let note = renamed_note(pairs.len());
        (pairs, pending, note)
    });
    if pairs.is_empty() || ST.with(|s| engine::error_count(&s.borrow().rows) > 0) {
        return;
    }
    launch(pairs, pending, note, true);
}

unsafe fn undo() {
    if ST.with(|s| s.borrow().busy) {
        return;
    }
    let Some(pairs) = exec::last_undo() else { return };
    // An old batch can name files that were deleted or renamed again. Skipping them here
    // keeps the ones that are still there, instead of failing the whole undo with "file not found".
    let (present, skipped) = exec::partition_undo(&pairs, exec::file_still_there);
    if present.is_empty() {
        if skipped > 0 {
            exec::pop_undo();
            let note = undo_skipped_note(skipped);
            ST.with(|s| s.borrow_mut().note.clone_from(&note));
            sync_buttons();
            let h = hwnd();
            let _ = InvalidateRect(Some(h), None, false);
            tell_skipped(h, &note);
        }
        return;
    }
    let pending = ST.with(|s| {
        s.borrow()
            .items
            .iter()
            .map(|it| {
                present
                    .iter()
                    .find(|(d, from, _)| engine::dir_key(d) == engine::dir_key(&it.dir) && from == &it.name)
                    .map(|(_, _, to)| to.clone())
                    .unwrap_or_else(|| it.name.clone())
            })
            .collect::<Vec<_>>()
    });
    let note = undo_result_note(present.len(), skipped);
    if skipped > 0 {
        tell_skipped(hwnd(), &undo_skipped_note(skipped));
    }
    launch(present, pending, note, false);
}

/// One summary for every name the undo could not find. Not a warning: those items are skipped.
unsafe fn tell_skipped(owner: HWND, note: &str) {
    let parent = if owner.0.is_null() { None } else { Some(owner) };
    let _ = MessageBoxW(parent, &HSTRING::from(note), w!("Maguro"), MB_OK | MB_ICONINFORMATION);
}

unsafe fn launch(pairs: Vec<(String, String, String)>, pending: Vec<String>, note: String, record: bool) {
    let avoid = ST.with(|s| {
        let st = s.borrow();
        let mut occ = st.existing.clone();
        for (dir, from, _) in &pairs {
            occ.remove(&(engine::dir_key(dir), engine::name_key(from)));
        }
        occ
    });
    let waves = engine::plan(&pairs, &avoid);
    if waves.is_empty() {
        return;
    }
    let undo: Vec<(String, String, String)> = pairs.iter().map(|(d, from, to)| (d.clone(), to.clone(), from.clone())).collect();
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.busy = true;
        st.pending = pending;
        st.pending_note = note;
        st.clear_rules_on_ok = record;
        st.pending_undo = undo;
    });
    sync_buttons();
    exec::start(hwnd(), waves);
}

unsafe fn on_done(h: HWND) {
    let finished = exec::take_done();
    let (pending, note, clear_rules, record, undo) = ST.with(|s| {
        let mut st = s.borrow_mut();
        st.busy = false;
        (std::mem::take(&mut st.pending), st.pending_note.clone(), st.clear_rules_on_ok, st.clear_rules_on_ok, std::mem::take(&mut st.pending_undo))
    });
    let ok = finished.as_ref().is_some_and(|f| f.ok);
    if ok {
        if record {
            exec::push_undo(undo);
        } else {
            exec::pop_undo();
        }
        ST.with(|s| {
            let mut st = s.borrow_mut();
            for (it, name) in st.items.iter_mut().zip(pending.iter()) {
                it.name.clone_from(name);
            }
            if clear_rules {
                st.rules = vec![Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false }];
                st.enabled = vec![true];
            }
            st.note.clone_from(&note);
        });
        if clear_rules {
            rebuild_rule_controls();
        }
    } else {
        reconcile(&pending);
        let msg = if finished.as_ref().is_some_and(|f| f.cancel) {
            tr(crate::Key::CancelledFinishedStepsWerePutBack).to_string()
        } else {
            let err = finished.map(|f| f.err).unwrap_or_default();
            format!("{}{err}", tr(crate::Key::RenameFailed))
        };
        ST.with(|s| s.borrow_mut().note.clone_from(&msg));
        let cancelled = msg.starts_with(tr(crate::Key::Cancelled));
        if !cancelled {
            let _ = MessageBoxW(Some(h), &HSTRING::from(&msg), w!("Maguro"), MB_ICONWARNING);
        }
    }
    let items = ST.with(|s| s.borrow().items.clone());
    let existing = load_existing(&items);
    ST.with(|s| s.borrow_mut().existing = existing);
    refresh_views();
    layout(h);
    recompute_keep_note();
    let _ = InvalidateRect(Some(h), None, true);
}

unsafe fn reconcile(pending: &[String]) {
    let items = ST.with(|s| s.borrow().items.clone());
    let mut names = Vec::new();
    for (it, new) in items.iter().zip(pending.iter().chain(std::iter::repeat(&String::new()))) {
        let found = actual_name(&it.dir, &it.name).or_else(|| actual_name(&it.dir, new));
        names.push(found.unwrap_or_else(|| it.name.clone()));
    }
    ST.with(|s| {
        for (it, name) in s.borrow_mut().items.iter_mut().zip(names) {
            it.name = name;
        }
    });
}

fn actual_name(dir: &str, name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let path = if dir.ends_with(':') { format!("{dir}\\") } else { dir.to_string() };
    let want = engine::name_key(name);
    std::fs::read_dir(path).ok()?.flatten().find_map(|ent| {
        let n = ent.file_name().to_string_lossy().into_owned();
        (engine::name_key(&n) == want).then_some(n)
    })
}

unsafe fn refresh_views() {
    let n = crate::app(|a| a.s.panes());
    for i in 0..n {
        if let Some(sv) = crate::view_of::<IShellView>(i) {
            let _ = sv.Refresh();
        }
    }
}

fn tz_offset_secs() -> i32 {
    unsafe {
        let mut info = TIME_ZONE_INFORMATION::default();
        let id = GetTimeZoneInformation(&mut info);
        if id == u32::MAX {
            return 0;
        }
        // Bias is the minutes added to local time to reach UTC. DaylightBias applies during DST.
        let extra = if id == 2 { info.DaylightBias } else { info.StandardBias };
        info.Bias.saturating_add(extra).saturating_mul(-60)
    }
}

fn recompute() {
    ST.with(|s| s.borrow_mut().note.clear());
    recompute_keep_note();
}

fn recompute_keep_note() {
    let (items, rules, existing) = ST.with(|s| {
        let st = s.borrow();
        (st.items.clone(), st.rules.clone(), st.existing.clone())
    });
    let enabled = ST.with(|s| s.borrow().enabled.clone());
    let rows = engine::preview_filtered(&items, &rules, &existing, tz_offset_secs(), &current_filter(), &enabled);
    let max = preview_max(rows.len());
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.rows = rows;
        if st.preview_scroll > max {
            st.preview_scroll = max;
        }
    });
    unsafe { sync_buttons() };
    unsafe {
        let h = hwnd();
        if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
            let _ = InvalidateRect(Some(h), None, false);
            let host = ST.with(|s| s.borrow().host);
            if !host.0.is_null() {
                let _ = InvalidateRect(Some(host), None, false);
                crate::ui::combo::invalidate_in(host);
            }
            crate::ui::combo::invalidate_in(h);
        }
    }
}

fn preview_max(n: usize) -> i32 {
    let dpi = ST.with(|s| s.borrow().dpi);
    let g = geo_now();
    let head = sc(dpi, 28);
    let row_h = sc(dpi, 36).max(1);
    let visible = ((g.list.bottom - g.list.top - head) / row_h).max(1);
    (n as i32 - visible).max(0)
}

unsafe fn sync_buttons() {
    let h = hwnd();
    if h.0.is_null() {
        return;
    }
    let (errs, changes, busy) = ST.with(|s| {
        let st = s.borrow();
        (engine::error_count(&st.rows), engine::change_count(&st.rows), st.busy)
    });
    let undo_n = exec::undo_count();
    let base = tx(crate::Key::Undo);
    let label = if undo_n == 0 { base.to_string() } else { format!("{base} ({undo_n})") };
    set_btn(h, ID_UNDO, &label);
    let _ = EnableWindow(ctl(h, ID_APPLY), (errs == 0 && changes > 0 && !busy).into());
    let _ = EnableWindow(ctl(h, ID_UNDO), (undo_n > 0 && !busy).into());
    let _ = EnableWindow(ctl(h, ID_SAVE), (!busy).into());
    let _ = EnableWindow(ctl(h, ID_DELPRE), (!busy).into());
    let _ = InvalidateRect(Some(ctl(h, ID_APPLY)), None, false);
    let _ = InvalidateRect(Some(ctl(h, ID_UNDO)), None, false);
}

fn host_height() -> i32 {
    let h = hwnd();
    if h.0.is_null() {
        return 0;
    }
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    geo(rc, ST.with(|s| s.borrow().dpi)).host.bottom - geo(rc, ST.with(|s| s.borrow().dpi)).host.top
}

struct Geo {
    rules_lbl: RECT,
    add: RECT,
    tools: RECT,
    host: RECT,
    prev_lbl: RECT,
    summary: RECT,
    list: RECT,
    undo: RECT,
    close: RECT,
    apply: RECT,
}

fn geo_now() -> Geo {
    let h = hwnd();
    let mut rc = RECT::default();
    if !h.0.is_null() {
        unsafe { let _ = GetClientRect(h, &mut rc); }
    }
    geo(rc, ST.with(|s| s.borrow().dpi))
}

/// Split `avail` between the two columns. At or above two minimums the extra
/// pixels are shared; below that the columns still sit side by side.
fn column_widths(avail: i32, min_col: i32) -> (i32, i32) {
    let avail = avail.max(0);
    let min_col = min_col.max(0);
    if avail >= min_col.saturating_mul(2) {
        let left = (avail / 2).clamp(min_col, avail - min_col);
        (left, avail - left)
    } else {
        let left = avail / 2;
        (left, avail - left)
    }
}

/// Toolbar across the top, rules on the left, preview on the right, buttons along the bottom.
/// The filter row is full width: it does not fit in the rules column once the window is split.
fn geo(rc: RECT, dpi: i32) -> Geo {
    let dpi = dpi.max(96);
    let pad = sc(dpi, crate::PAD_BODY_X);
    let gap = sc(dpi, COL_GAP_DIP);
    let top = pad;
    let tool_h = sc(dpi, FIELD_H_DIP);
    let head_h = sc(dpi, 28);
    let add_w = sc(dpi, 120);
    let add = RECT { left: rc.right - pad - add_w, top, right: rc.right - pad, bottom: top + tool_h };
    let tools = RECT { left: rc.left + pad, top, right: (add.left - sc(dpi, 8)).max(rc.left + pad), bottom: top + tool_h };
    let foot_h = sc(dpi, 56);
    let foot_top = rc.bottom - foot_h;
    let head_top = tools.bottom + sc(dpi, 10);
    let body_top = head_top + head_h + sc(dpi, 4);
    let body_bot = (foot_top - sc(dpi, 8)).max(body_top);
    let content_left = rc.left + pad;
    let content_right = rc.right - pad;
    let (left_w, _) = column_widths((content_right - content_left - gap).max(0), sc(dpi, COL_MIN_DIP));
    let split = content_left + left_w;
    let col_right = (split + gap).min(content_right);
    let rules_lbl = RECT { left: content_left, top: head_top, right: split.max(content_left), bottom: head_top + head_h };
    let host = RECT { left: content_left, top: body_top, right: split.max(content_left), bottom: body_bot };
    let prev_w = sc(dpi, 140).min((content_right - col_right).max(0));
    let prev_lbl = RECT { left: col_right, top: head_top, right: col_right + prev_w, bottom: head_top + head_h };
    let summary_left = (prev_lbl.right + sc(dpi, 8)).min(content_right);
    let summary = RECT { left: summary_left, top: head_top, right: content_right, bottom: head_top + head_h };
    let list = RECT { left: col_right, top: body_top, right: content_right, bottom: body_bot };
    let btn_h = sc(dpi, crate::BTN_H_FOOTER);
    let btn_y = foot_top + (foot_h - btn_h) / 2;
    let apply = RECT { left: rc.right - pad - sc(dpi, 128), top: btn_y, right: rc.right - pad, bottom: btn_y + btn_h };
    let close = RECT { left: apply.left - sc(dpi, crate::BTN_GAP) - sc(dpi, 96), top: btn_y, right: apply.left - sc(dpi, crate::BTN_GAP), bottom: btn_y + btn_h };
    let undo = RECT { left: rc.left + pad, top: btn_y, right: rc.left + pad + sc(dpi, 112), bottom: btn_y + btn_h };
    Geo { rules_lbl, add, tools, host, prev_lbl, summary, list, undo, close, apply }
}

unsafe fn layout(h: HWND) {
    if h.0.is_null() {
        return;
    }
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let dpi = ST.with(|s| s.borrow().dpi);
    let g = geo(rc, dpi);
    move_to(ctl(h, ID_ADD), g.add);
    move_to(ctl(h, ID_UNDO), g.undo);
    move_to(ctl(h, ID_CLOSE), g.close);
    move_to(ctl(h, ID_APPLY), g.apply);
    place_tools(h, g.tools, dpi);
    let host = ST.with(|s| s.borrow().host);
    move_to(host, g.host);
    place_rules();
}

struct ToolRects {
    filter: RECT,
    ext: RECT,
    preset: RECT,
    save: RECT,
    del: RECT,
}

/// Filter, extension, preset, Save and Delete preset. The preset box shrinks
/// when the row is tight; the others keep their width. Heights follow `r`.
fn tool_rects(r: RECT, dpi: i32) -> ToolRects {
    let gap = sc(dpi, 8);
    let filter_w = sc(dpi, 128);
    let ext_w = sc(dpi, 148);
    let save_w = sc(dpi, 72);
    let del_w = sc(dpi, 128);
    let preset_pref = sc(dpi, 180);
    let preset_min = sc(dpi, 96);
    let fixed = filter_w + ext_w + save_w + del_w + gap * 4;
    let avail = (r.right - r.left).max(0);
    let room = avail - fixed;
    let preset_w = if room >= preset_pref { preset_pref } else { room.max(0).max(preset_min.min(avail)) };
    let mut x = r.left;
    let slot = |w: i32, x: &mut i32| {
        let rect = RECT { left: *x, top: r.top, right: *x + w, bottom: r.bottom };
        *x += w + gap;
        rect
    };
    ToolRects { filter: slot(filter_w, &mut x), ext: slot(ext_w, &mut x), preset: slot(preset_w, &mut x), save: slot(save_w, &mut x), del: slot(del_w, &mut x) }
}

unsafe fn place_tools(h: HWND, r: RECT, dpi: i32) {
    let t = tool_rects(r, dpi);
    move_to(ctl(h, ID_FILTER), t.filter);
    place_field(h, ID_EXT, t.ext, dpi);
    move_to(ctl(h, ID_PRESET), t.preset);
    move_to(ctl(h, ID_SAVE), t.save);
    move_to(ctl(h, ID_DELPRE), t.del);
}

/// Replace and regex keep the case and extension ticks on their own row.
/// Number and date need a third field row. Everything else is the header plus one row.
fn rule_row_count(rule: &Rule) -> i32 {
    match rule {
        Rule::Replace { .. } | Rule::Regex { .. } | Rule::Number { .. } | Rule::Date { .. } => 3,
        _ => 2,
    }
}

/// Card body plus the gap before the next card. Each piece is scaled on its own
/// so 125% and 150% match the rows placed inside the card.
fn rule_stride(rule: &Rule, dpi: i32) -> i32 {
    let dpi = dpi.max(96);
    let rows = rule_row_count(rule);
    sc(dpi, CARD_PAD_DIP) * 2 + rows * sc(dpi, FIELD_H_DIP) + (rows - 1) * sc(dpi, ROW_GAP_DIP) + sc(dpi, CARD_GAP_DIP)
}

fn rules_span(rules: &[Rule], dpi: i32) -> i32 {
    rules.iter().map(|r| rule_stride(r, dpi)).sum()
}

fn row_top(card: RECT, dpi: i32, index: i32) -> i32 {
    card.top + sc(dpi.max(96), CARD_PAD_DIP) + index * (sc(dpi.max(96), FIELD_H_DIP) + sc(dpi.max(96), ROW_GAP_DIP))
}

fn card_rect(y: i32, stride: i32, width: i32, dpi: i32) -> RECT {
    let gap = sc(dpi.max(96), CARD_GAP_DIP);
    let top_gap = gap / 2;
    RECT { left: sc(dpi, 2), top: y + top_gap, right: width - sc(dpi, 2), bottom: y + stride - (gap - top_gap) }
}

/// Grip is the left cluster of the header row. Clicks to its right belong to the tick.
fn drag_max_x(dpi: i32) -> i32 {
    sc(dpi.max(96), 2) + sc(dpi.max(96), 8) + sc(dpi.max(96), 10) + sc(dpi.max(96), 2)
}

/// Two columns of three dots, centred on the header row beside the enable tick.
fn grip_dots(card: RECT, dpi: i32) -> [RECT; 6] {
    let dpi = dpi.max(96);
    let row = row_top(card, dpi, 0);
    let mid = row + sc(dpi, FIELD_H_DIP) / 2;
    let dot_w = sc(dpi, 3).max(2);
    let dot_h = sc(dpi, 2).max(1);
    let gap = sc(dpi, 2).max(1);
    let block = 3 * dot_h + 2 * gap;
    let top = mid - block / 2;
    let x0 = card.left + sc(dpi, 8);
    let x1 = x0 + dot_w + gap;
    let mut out = [RECT::default(); 6];
    for i in 0..3usize {
        let y = top + i as i32 * (dot_h + gap);
        out[i * 2] = RECT { left: x0, top: y, right: x0 + dot_w, bottom: y + dot_h };
        out[i * 2 + 1] = RECT { left: x1, top: y, right: x1 + dot_w, bottom: y + dot_h };
    }
    out
}

/// An off rule is the card colour at 55% over the window, the same composite as the mockup.
fn plate(on: bool) -> u32 {
    if on { crate::pal().card } else { crate::blend(crate::pal().bg, crate::pal().card, 0.55) }
}

fn plate_line(on: bool) -> u32 {
    if on { crate::pal().line } else { crate::blend(crate::pal().bg, crate::pal().line, 0.55) }
}

/// Rule combos dim with the card. Filter and preset sit on the window and stay solid.
unsafe fn combo_face(hwnd: HWND) -> crate::ui::combo::Face {
    let off = control_rule(hwnd).is_some_and(|i| !rule_is_on(i));
    let fill = plate(!off);
    let ink = if off { crate::blend(fill, crate::pal().text, 0.55) } else { crate::pal().text };
    crate::ui::combo::Face { fill, edge: plate_line(!off), ink, ground: fill }
}

/// Changed-span wash. Light is 28% cerulean on white. Dark is 32% cerulean on the card.
fn mark_wash(dark: bool, card: u32) -> u32 {
    if dark { crate::blend(card, CERULEAN, 0.32) } else { crate::blend(0xFFFFFF, CERULEAN, 0.28) }
}

fn mark_ink(dark: bool) -> u32 {
    if dark { MARK_INK_DARK } else { CERULEAN_DEEP }
}

fn row_hover(dark: bool) -> u32 {
    if dark { ROW_HOVER_DARK } else { ROW_HOVER_LIGHT }
}

/// Inset of a single-line edit inside its painted field. Equal on every side,
/// so the font cell `place_edit` centres in here is also centred in the frame.
fn field_inner(outer: RECT, dpi: i32) -> RECT {
    crate::field_edit_rect(outer, dpi)
}

fn is_text_field(field: i32) -> bool {
    matches!(field, F_A | F_B | F_C | F_AT)
}

fn kind_x(card: RECT, dpi: i32) -> i32 {
    // Pad, grip, gap, tick, gap. The kind combo starts where the mockup's does.
    let dpi = dpi.max(96);
    card.left + sc(dpi, 8) + sc(dpi, 10) + sc(dpi, 6) + sc(dpi, TICK_DIP) + sc(dpi, 6)
}

unsafe fn place_rules() {
    let host = ST.with(|s| s.borrow().host);
    if host.0.is_null() {
        return;
    }
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let mut rc = RECT::default();
    let _ = GetClientRect(host, &mut rc);
    let width = (rc.right - rc.left).max(1);
    let rules = ST.with(|s| s.borrow().rules.clone());
    let content = rules_span(&rules, dpi);
    let view = (rc.bottom - rc.top).max(0);
    let max = (content - view).max(0);
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if st.rule_scroll > max {
            st.rule_scroll = max;
        }
        if st.rule_scroll < 0 {
            st.rule_scroll = 0;
        }
    });
    let scroll = ST.with(|s| s.borrow().rule_scroll);
    let mut y = -scroll;
    for (i, rule) in rules.iter().enumerate() {
        let stride = rule_stride(rule, dpi);
        let card = card_rect(y, stride, width, dpi);
        place_card(host, i, card, dpi, rule);
        y += stride;
    }
    for i in rules.len()..MAX_RULES {
        for f in 0..STRIDE {
            let c = ctl(host, id(i, f));
            if !c.0.is_null() {
                let _ = ShowWindow(c, SW_HIDE);
            }
        }
    }
}

/// Painted frame for each control under the kind row. Text-field rects are the
/// frame, the same height as the combos; the edit window is centred inside later.
fn card_slots(card: RECT, dpi: i32, rule: &Rule) -> Vec<(i32, RECT)> {
    let dpi = dpi.max(96);
    let row2 = row_top(card, dpi, 1);
    let row3 = row_top(card, dpi, 2);
    let h = sc(dpi, FIELD_H_DIP);
    let gap = sc(dpi, 6);
    let x = kind_x(card, dpi);
    let end = card.right - sc(dpi, 10);
    let mut slots = Vec::new();
    let mut add = |field: i32, left: i32, top: i32, right: i32| {
        if right > left {
            slots.push((field, RECT { left, top, right, bottom: top + h }));
        }
    };
    match rule {
        Rule::Replace { .. } | Rule::Regex { .. } => {
            let span = (end - x - gap).max(sc(dpi, 40));
            let w = span / 2;
            add(F_A, x, row2, x + w);
            add(F_B, x + w + gap, row2, (x + w + gap + w).min(end));
            // Ticks share the row under the fields, the same way the mockup draws them.
            let check_gap = sc(dpi, 14);
            let room = (end - x - check_gap).max(0);
            let half = room / 2;
            add(F_CASE, x, row3, x + half);
            add(F_EXT, x + half + check_gap, row3, end);
        }
        Rule::Prefix { .. } | Rule::Suffix { .. } | Rule::Extension { .. } => {
            add(F_A, x, row2, (x + sc(dpi, 280)).min(end));
        }
        Rule::Case { .. } => {
            let ext_w = sc(dpi, 176);
            add(F_EXT, end - ext_w, row2, end);
            let mode_r = (end - ext_w - gap).max(x + sc(dpi, 40));
            add(F_MODE, x, row2, mode_r.min(x + sc(dpi, 220)));
        }
        Rule::Number { pos, .. } => {
            let span = (end - x - gap * 2).max(sc(dpi, 90));
            let w = span / 3;
            add(F_A, x, row2, x + w);
            add(F_B, x + w + gap, row2, x + w + gap + w);
            add(F_C, x + (w + gap) * 2, row2, (x + (w + gap) * 2 + w).min(end));
            let pos_w = sc(dpi, 180);
            add(F_POS, x, row3, (x + pos_w).min(end));
            if matches!(pos, NumPos::At(_)) {
                let at_l = x + pos_w + gap;
                if at_l < end {
                    add(F_AT, at_l, row3, (at_l + sc(dpi, 220)).min(end));
                }
            }
        }
        Rule::Date { .. } => {
            let src_w = sc(dpi, 180);
            add(F_SRC, x, row2, (x + src_w).min(end));
            let pat_l = x + src_w + gap;
            if pat_l < end {
                add(F_A, pat_l, row2, end);
            }
            add(F_POS, x, row3, (x + sc(dpi, 200)).min(end));
        }
        Rule::Insert { .. } => {
            let at_w = sc(dpi, 200);
            add(F_A, x, row2, (end - at_w - gap).max(x + sc(dpi, 40)));
            add(F_AT, end - at_w, row2, end);
        }
        Rule::Delete { .. } => {
            let w = sc(dpi, 180);
            add(F_AT, x, row2, (x + w).min(end));
            add(F_C, x + w + gap, row2, (x + w + gap + w).min(end));
        }
        Rule::Trim { .. } => {
            let edge_w = sc(dpi, 160);
            add(F_A, x, row2, (end - edge_w - gap).max(x + sc(dpi, 40)));
            add(F_POS, end - edge_w, row2, end);
        }
    }
    slots
}

struct HeadCtl {
    tick: RECT,
    kind: RECT,
    up: RECT,
    down: RECT,
    del: RECT,
}

/// Header row: grip (painted), enable tick, kind combo, then up / down / remove.
fn head_controls(card: RECT, dpi: i32) -> HeadCtl {
    let dpi = dpi.max(96);
    let row1 = row_top(card, dpi, 0);
    let h = sc(dpi, FIELD_H_DIP);
    let btn = sc(dpi, MINI_W_DIP);
    let gap = sc(dpi, 6);
    let right = card.right - sc(dpi, 10);
    let del = RECT { left: right - btn, top: row1, right, bottom: row1 + h };
    let down = RECT { left: del.left - gap - btn, top: row1, right: del.left - gap, bottom: row1 + h };
    let up = RECT { left: down.left - gap - btn, top: row1, right: down.left - gap, bottom: row1 + h };
    let side = sc(dpi, TICK_DIP);
    let tick_left = card.left + sc(dpi, 8) + sc(dpi, 10) + sc(dpi, 6);
    // The control is the full row so the tick is easy to hit. draw_check centres the 16 DIP box in it.
    let tick = RECT { left: tick_left, top: row1, right: tick_left + side, bottom: row1 + h };
    let kx = kind_x(card, dpi);
    let kind = RECT { left: kx, top: row1, right: (kx + sc(dpi, KIND_W_DIP)).min(up.left - gap), bottom: row1 + h };
    HeadCtl { tick, kind, up, down, del }
}

unsafe fn place_card(host: HWND, i: usize, card: RECT, dpi: i32, rule: &Rule) {
    let head = head_controls(card, dpi);
    put(host, id(i, F_ON), head.tick, true);
    put(host, id(i, F_KIND), head.kind, true);
    put(host, id(i, F_UP), head.up, true);
    put(host, id(i, F_DOWN), head.down, true);
    put(host, id(i, F_DEL), head.del, true);
    let _ = EnableWindow(ctl(host, id(i, F_UP)), (i > 0).into());
    let last = i + 1 == ST.with(|s| s.borrow().rules.len());
    let _ = EnableWindow(ctl(host, id(i, F_DOWN)), (!last).into());
    for f in [F_A, F_B, F_C, F_AT, F_CASE, F_EXT, F_MODE, F_POS, F_SRC] {
        put(host, id(i, f), RECT::default(), false);
    }
    for (field, outer) in card_slots(card, dpi, rule) {
        if is_text_field(field) {
            place_field(host, id(i, field), outer, dpi);
        } else {
            put(host, id(i, field), outer, true);
        }
    }
}

/// A single-line EDIT draws its font cell from the top of the client. Shrink the
/// control to that cell and centre it in `outer`, which stays the painted frame
/// and matches the combo and button beside it. `place_edit` measures `tmHeight`,
/// so 100% and 150% DPI use the real line rather than a fixed offset.
unsafe fn place_field(parent: HWND, cid: i32, outer: RECT, dpi: i32) {
    let c = ctl(parent, cid);
    if c.0.is_null() {
        return;
    }
    if outer.right <= outer.left || outer.bottom <= outer.top {
        let _ = ShowWindow(c, SW_HIDE);
        return;
    }
    // The edit sits `FIELD_INSET` inside the painted border. The rest of the
    // shared field pad is the edit margin, so the glyphs line up with a combo.
    let m = sc(dpi.max(96), crate::FIELD_PAD_X - crate::FIELD_INSET) as isize;
    let _ = SendMessageW(c, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(m | (m << 16))));
    crate::place_edit(c, field_inner(outer, dpi), true);
}

unsafe fn put(parent: HWND, id: i32, r: RECT, show: bool) {
    let c = ctl(parent, id);
    if c.0.is_null() {
        return;
    }
    if !show || r.right <= r.left || r.bottom <= r.top {
        let _ = ShowWindow(c, SW_HIDE);
        return;
    }
    move_to(c, r);
    let _ = ShowWindow(c, SW_SHOW);
}

unsafe fn move_to(h: HWND, r: RECT) {
    if h.0.is_null() {
        return;
    }
    let _ = MoveWindow(h, r.left, r.top, (r.right - r.left).max(0), (r.bottom - r.top).max(0), true);
}

unsafe fn on_wheel(delta: i32) {
    if delta == 0 {
        return;
    }
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let h = hwnd();
    if h.0.is_null() {
        return;
    }
    let _ = ScreenToClient(h, &mut pt);
    let g = geo_now();
    let over_rules = pt.y >= g.host.top && pt.y < g.host.bottom && pt.x >= g.host.left && pt.x < g.host.right;
    if over_rules {
        let dpi = ST.with(|s| s.borrow().dpi);
        let step = sc(dpi, 48);
        let next = ST.with(|s| s.borrow().rule_scroll) - delta * step;
        ST.with(|s| s.borrow_mut().rule_scroll = next);
        place_rules();
        let host = ST.with(|s| s.borrow().host);
        let _ = InvalidateRect(Some(host), None, true);
    } else {
        let n = ST.with(|s| s.borrow().rows.len());
        let max = preview_max(n);
        let next = (ST.with(|s| s.borrow().preview_scroll) - delta * 3).clamp(0, max);
        ST.with(|s| s.borrow_mut().preview_scroll = next);
        let _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn paint(h: HWND) {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let font = crate::body_font();
    let bold = crate::bold_ui_font();
    let small = meta_font();
    crate::buffered(h, |dc, rc| {
        let g = geo(rc, dpi);
        let t = crate::ui_tokens();
        crate::paint_window(dc, rc);
        let rules_title = tx(crate::Key::Rules);
        crate::paint_section_header(dc, g.rules_lbl, rules_title, bold);
        let hint_left = g.rules_lbl.left + text_w(dc, bold, rules_title) + sc(dpi, 12);
        if hint_left < g.rules_lbl.right {
            crate::paint_meta(dc, RECT { left: hint_left, ..g.rules_lbl }, tx(crate::Key::TopBottom), small);
        }
        let ext_focus = GetFocus() == ctl(h, ID_EXT);
        crate::paint_field(dc, tool_rects(g.tools, dpi).ext, dpi, ext_focus);
        if g.list.left > g.host.right {
            let x = g.host.right + (g.list.left - g.host.right) / 2;
            crate::fill(dc, RECT { left: x, top: g.host.top, right: x + 1, bottom: g.host.bottom }, t.line);
        }
        crate::paint_section_header(dc, g.prev_lbl, tx(crate::Key::Preview), bold);
        let note = ST.with(|s| s.borrow().note.clone());
        let rows = ST.with(|s| s.borrow().rows.clone());
        let summary = if note.is_empty() {
            summary_text(rows.len(), engine::change_count(&rows), engine::error_count(&rows), engine::skipped_count(&rows))
        } else {
            note
        };
        let sum_color = if engine::error_count(&rows) > 0 && ST.with(|s| s.borrow().note.is_empty()) { err_color() } else { t.sub };
        crate::text(dc, g.summary, &summary, small, sum_color, DT_RIGHT);
        paint_list(dc, g.list, dpi, font, bold, small, &rows);
        crate::fill(dc, RECT { left: rc.left, top: g.undo.top - sc(dpi, 12), right: rc.right, bottom: g.undo.top - sc(dpi, 11), ..rc }, t.line);
    });
}

unsafe fn paint_list(dc: HDC, list: RECT, dpi: i32, font: HFONT, bold: HFONT, small: HFONT, rows: &[PreviewRow]) {
    if list.bottom <= list.top {
        return;
    }
    let head = sc(dpi, 28);
    let t = crate::ui_tokens();
    // Card, then a header strip in the window ground so the titles sit on the caption colour.
    // The fill stays one pixel inside the card so the shared hairline remains.
    crate::ui::paint_card(dc, list, dpi);
    let head_r = RECT { bottom: (list.top + head).min(list.bottom), ..list };
    if head_r.bottom > list.top + 1 {
        crate::fill(dc, RECT { left: list.left + 1, top: list.top + 1, right: list.right - 1, bottom: head_r.bottom }, t.bg);
    }
    let split = list.left + (list.right - list.left) * 42 / 100;
    let new_left = split + sc(dpi, 28);
    let err_w = sc(dpi, 168);
    crate::paint_meta(dc, RECT { left: list.left + sc(dpi, 12), right: split, ..head_r }, tx(crate::Key::Current), small);
    crate::paint_meta(dc, RECT { left: new_left, right: list.right - err_w, ..head_r }, tx(crate::Key::NewName), small);
    crate::fill(dc, RECT { left: list.left + 1, top: head_r.bottom - 1, right: list.right - 1, bottom: head_r.bottom }, t.line);
    let _ = SaveDC(dc);
    let _ = IntersectClipRect(dc, list.left + 1, head_r.bottom, list.right - 1, list.bottom - 1);
    let row_h = sc(dpi, 36).max(1);
    let scroll = ST.with(|s| s.borrow().preview_scroll);
    let multi = ST.with(|s| {
        let st = s.borrow();
        let first = st.items.first().map(|it| engine::dir_key(&it.dir));
        first.is_some() && st.items.iter().any(|it| Some(engine::dir_key(&it.dir)) != first)
    });
    let items = ST.with(|s| s.borrow().items.clone());
    for (i, row) in rows.iter().enumerate().skip(scroll.max(0) as usize) {
        let y = head_r.bottom + (i as i32 - scroll) * row_h;
        if y > list.bottom {
            break;
        }
        let r = RECT { left: list.left, top: y, right: list.right, bottom: y + row_h };
        if i > 0 {
            crate::fill(dc, RECT { bottom: r.top + 1, top: r.top, ..r }, t.line);
        }
        let dark = crate::DARK_ON.load(std::sync::atomic::Ordering::Relaxed);
        if ST.with(|s| s.borrow().hover) == Some(i) {
            crate::fill(dc, RECT { left: r.left + 1, right: r.right - 1, top: r.top + 1, bottom: r.bottom - 1 }, row_hover(dark));
        }
        let old_r = RECT { left: r.left + sc(dpi, 12), right: split - sc(dpi, 8), ..r };
        let arrow = RECT { left: split, right: new_left, ..r };
        let new_r = RECT { left: new_left, right: r.right - err_w, ..r };
        let err_r = RECT { left: r.right - err_w, right: r.right - sc(dpi, 10), ..r };
        let wash = mark_wash(dark, crate::pal().card);
        let mark = mark_ink(dark);
        if row.skipped {
            crate::text(dc, old_r, &row.old_name, font, crate::pal().sub, DT_LEFT);
            crate::text(dc, new_r, &row.new_name, font, crate::pal().sub, DT_LEFT);
            crate::text(dc, err_r, tx(crate::Key::Skipped), small, crate::pal().sub, DT_RIGHT);
        } else if row.issues.is_empty() {
            paint_marked(dc, old_r, &row.old_name, row.old_span, font, bold, dpi, crate::pal().text, wash, mark);
            paint_marked(dc, new_r, &row.new_name, row.new_span, font, bold, dpi, crate::pal().text, wash, mark);
        } else {
            crate::text(dc, old_r, &row.old_name, font, crate::pal().text, DT_LEFT);
            crate::text(dc, new_r, &row.new_name, font, err_color(), DT_LEFT);
            if let Some(issue) = row.issues.first() {
                crate::text(dc, err_r, issue_text(*issue), small, err_color(), DT_RIGHT);
            }
        }
        crate::text(dc, arrow, "→", font, crate::pal().sub, DT_CENTER);
        if multi && !row.skipped {
            if let Some(folder) = items.get(i).map(|it| it.dir.rsplit(['\\', '/']).next().unwrap_or("").to_string()) {
                if !folder.is_empty() && row.issues.is_empty() {
                    // Folder tag stays out of the highlighted name; the error column is free.
                    crate::text(dc, err_r, &folder, small, crate::pal().sub, DT_RIGHT);
                }
            }
        }
    }
    let _ = RestoreDC(dc, -1);
    let max = preview_max(rows.len());
    if max > 0 {
        paint_thumb(dc, list, ST.with(|s| s.borrow().preview_scroll), max, dpi, crate::pal().card);
    }
}

unsafe fn paint_host(h: HWND) {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let font = meta_font();
    crate::buffered(h, |dc, rc| {
        crate::paint_window(dc, rc);
        let rules = ST.with(|s| s.borrow().rules.clone());
        if rules.is_empty() {
            let r = RECT { left: sc(dpi, 8), top: sc(dpi, 12), right: rc.right - sc(dpi, 8), bottom: sc(dpi, 48) };
            crate::paint_meta(dc, r, tx(crate::Key::AddRuleNamesStayAsTheyAre), font);
            return;
        }
        let scroll = ST.with(|s| s.borrow().rule_scroll);
        let width = rc.right - rc.left;
        let focus = GetFocus();
        let mut y = -scroll;
        for (i, rule) in rules.iter().enumerate() {
            let stride = rule_stride(rule, dpi);
            let card = card_rect(y, stride, width, dpi);
            y += stride;
            if card.bottom < 0 || card.top > rc.bottom {
                continue;
            }
            let on = rule_is_on(i);
            crate::paint_plate(dc, card, dpi, on);
            let ink = crate::blend(plate(on), crate::pal().text, 0.45);
            for dot in grip_dots(card, dpi) {
                crate::fill(dc, dot, ink);
            }
            // Frame is the combo-height slot. The edit itself is only the font cell, centred in it.
            for (field, outer) in card_slots(card, dpi, rule) {
                if is_text_field(field) {
                    let focused = focus == ctl(h, id(i, field));
                    crate::paint_field_on(dc, outer, dpi, focused, plate(on));
                }
            }
        }
        if let Some(from) = ST.with(|s| s.borrow().drag_from) {
            let to = ST.with(|s| s.borrow().drag_to);
            let mut y = -scroll;
            for (i, rule) in rules.iter().enumerate() {
                if i == to {
                    break;
                }
                y += rule_stride(rule, dpi);
            }
            let _ = from;
            crate::fill(dc, RECT { left: sc(dpi, 8), top: y, right: width - sc(dpi, 8), bottom: y + sc(dpi, 2) }, CERULEAN);
        }
        let content = rules_span(&rules, dpi);
        let view = rc.bottom - rc.top;
        let max = (content - view).max(0);
        if max > 0 {
            paint_thumb(dc, rc, scroll, max, dpi, crate::pal().bg);
        }
    });
}

unsafe fn paint_thumb(dc: HDC, r: RECT, scroll: i32, max: i32, dpi: i32, _ground: u32) {
    let track = RECT { left: r.right - sc(dpi, 8), top: r.top + sc(dpi, 6), right: r.right - sc(dpi, 4), bottom: r.bottom - sc(dpi, 6) };
    let track_c = crate::blend(crate::pal().bg, crate::pal().text, 0.08);
    let rad = sc(dpi, crate::RADIUS);
    crate::rfill(dc, track, rad, track_c, None);
    let span = (track.bottom - track.top).max(1);
    let thumb_h = (span * span / (span + max)).clamp(sc(dpi, 18), span);
    let travel = (span - thumb_h).max(1);
    let y = track.top + scroll.clamp(0, max) * travel / max.max(1);
    crate::rfill(dc, RECT { left: track.left, top: y, right: track.right, bottom: y + thumb_h }, rad, crate::blend(crate::pal().bg, crate::pal().text, 0.35), None);
}

unsafe fn paint_marked(dc: HDC, rect: RECT, text: &str, span: Option<(usize, usize)>, font: HFONT, bold: HFONT, dpi: i32, normal: u32, wash: u32, mark: u32) {
    let Some((a, b)) = span else {
        crate::text(dc, rect, text, font, normal, DT_LEFT);
        return;
    };
    let chars: Vec<char> = text.chars().collect();
    let pre: String = chars.get(..a).unwrap_or(&[]).iter().collect();
    let mid: String = chars.get(a..b).unwrap_or(&[]).iter().collect();
    let post: String = chars.get(b..).unwrap_or(&[]).iter().collect();
    if mid.is_empty() {
        crate::text(dc, rect, text, font, normal, DT_LEFT);
        return;
    }
    let mark_font = if bold.is_invalid() { font } else { bold };
    let width = (rect.right - rect.left).max(0);
    let total = text_w(dc, font, &pre) + text_w(dc, mark_font, &mid) + text_w(dc, font, &post);
    if total > width {
        crate::text(dc, rect, text, font, mark, DT_LEFT);
        return;
    }
    let mut x = rect.left;
    let draw = |dc: HDC, x: i32, s: &str, face: HFONT, fg: u32, bg: Option<u32>| -> i32 {
        if s.is_empty() {
            return x;
        }
        // Same advance draw_line paints. DT_CALCRECT was short on Hangul and the wash cut the glyph.
        let advance = text_w(dc, face, s).max(0);
        if let Some(bg) = bg {
            let m = crate::line_metrics(dc, face);
            let top = crate::ink_line_top(rect.top, (rect.bottom - rect.top).max(0), m, crate::ink_kind(s));
            let raw = span_band(x, advance, top, m.height(), sc(dpi, 1));
            let band = RECT {
                left: raw.left.max(rect.left),
                top: raw.top.max(rect.top),
                right: raw.right.min(rect.right),
                bottom: raw.bottom.min(rect.bottom),
            };
            if band.bottom > band.top && band.right > band.left {
                crate::rfill(dc, band, sc(dpi, 2).max(1), bg, None);
            }
        }
        let r = RECT { left: x, top: rect.top, right: x + advance.max(1), bottom: rect.bottom };
        crate::draw_ink_line(dc, r, s, face, fg, DT_LEFT | DT_NOCLIP);
        x + advance
    };
    x = draw(dc, x, &pre, font, normal, None);
    x = draw(dc, x, &mid, mark_font, mark, Some(wash));
    let _ = draw(dc, x, &post, font, normal, None);
}

/// Wash around one changed run. `advance` is `GetTextExtentPoint32W` of that run.
fn span_band(x: i32, advance: i32, ink_top: i32, ink_h: i32, pad: i32) -> RECT {
    RECT { left: x - pad, top: ink_top, right: x + advance + pad, bottom: ink_top + ink_h.max(0) }
}

/// Advance width on `dc` with `font` selected. Matches the glyphs `draw_line` paints.
unsafe fn text_w(dc: HDC, font: HFONT, s: &str) -> i32 {
    if s.is_empty() || font.is_invalid() || dc.is_invalid() {
        return 0;
    }
    let w: Vec<u16> = s.encode_utf16().collect();
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    let ok = GetTextExtentPoint32W(dc, &w, &mut sz).as_bool();
    SelectObject(dc, old);
    if ok { sz.cx.max(0) } else { 0 }
}

fn is_check(field: i32) -> bool {
    matches!(field, F_ON | F_CASE | F_EXT)
}

unsafe fn draw_btn(d: &DRAWITEMSTRUCT) {
    let id = d.CtlID as i32;
    if id >= ID_BASE && is_check((id - ID_BASE) % STRIDE) {
        draw_check(d);
        return;
    }
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let in_card = id >= ID_BASE;
    let rule_on = if in_card { rule_is_on(((id - ID_BASE) / STRIDE) as usize) } else { true };
    let font = if in_card { meta_font() } else { crate::body_font() };
    let bold = if in_card { font } else { crate::bold_ui_font() };
    let mut visual = crate::visual_from_item(d.itemState.0, crate::app(|a| a.hover) == d.hwndItem);
    if in_card && !rule_on {
        visual.state = crate::ButtonState::Disabled;
        visual.focus = false;
    }
    let kind = if id == ID_APPLY { crate::ButtonKind::Primary } else { crate::ButtonKind::Secondary };
    let face = if kind == crate::ButtonKind::Primary && visual.state != crate::ButtonState::Disabled && !bold.is_invalid() { bold } else { font };
    let ground = if in_card { plate(rule_on) } else { crate::ui_tokens().bg };
    let label = crate::text_of(d.hwndItem);
    crate::paint_button(d.hDC, d.rcItem, kind, visual, &label, face, dpi, ground, crate::FocusRing::Inside);
}

unsafe fn draw_check(d: &DRAWITEMSTRUCT) {
    let id = d.CtlID as i32;
    let i = ((id - ID_BASE) / STRIDE) as usize;
    let field = (id - ID_BASE) % STRIDE;
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let on_rule = rule_is_on(i);
    let ground = plate(on_rule);
    let hot = d.itemState.0 & ODS_DISABLED.0 == 0 && crate::app(|a| a.hover) == d.hwndItem;
    crate::fill(d.hDC, d.rcItem, ground);
    let side = sc(dpi, TICK_DIP);
    let top = d.rcItem.top + ((d.rcItem.bottom - d.rcItem.top) - side) / 2;
    let mark = RECT { left: d.rcItem.left, top, right: d.rcItem.left + side, bottom: top + side };
    crate::paint_check(d.hDC, mark, stored_flag(i, field), hot, dpi, ground);
    let label = crate::text_of(d.hwndItem);
    if label.is_empty() {
        return;
    }
    let font = crate::body_font();
    let lab = RECT { left: mark.right + sc(dpi, 6), ..d.rcItem };
    if on_rule {
        crate::paint_label(d.hDC, lab, &label, font);
    } else {
        let ink = crate::blend(ground, crate::ui_tokens().text, 0.55);
        crate::text(d.hDC, lab, &label, font, ink, DT_LEFT);
    }
}

fn err_color() -> u32 {
    if crate::DARK_ON.load(std::sync::atomic::Ordering::Relaxed) { 0xFF7B72 } else { 0xC4314B }
}

fn current_filter() -> engine::Filter {
    let (files, folders) = filter_flags();
    let h = hwnd();
    let extensions = if h.0.is_null() {
        Vec::new()
    } else {
        engine::parse_extensions(&unsafe { crate::text_of(ctl(h, ID_EXT)) })
    };
    engine::Filter { files, folders, extensions }
}

fn filter_flags() -> (bool, bool) {
    let h = hwnd();
    if h.0.is_null() {
        return (true, true);
    }
    match unsafe { SendMessageW(ctl(h, ID_FILTER), CB_GETCURSEL, None, None).0 } {
        1 => (true, false),
        2 => (false, true),
        _ => (true, true),
    }
}

unsafe fn refill_filter() {
    let h = hwnd();
    let c = ctl(h, ID_FILTER);
    if c.0.is_null() {
        return;
    }
    let cur = SendMessageW(c, CB_GETCURSEL, None, None).0;
    let _ = SendMessageW(c, CB_RESETCONTENT, None, None);
    for label in [
        tx(crate::Key::AllItems),
        tx(crate::Key::FilesOnly),
        tx(crate::Key::FoldersOnly),
    ] {
        let hs = HSTRING::from(label);
        let _ = SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
    let _ = SendMessageW(c, CB_SETCURSEL, Some(WPARAM(if cur < 0 { 0 } else { cur as usize })), None);
}

unsafe fn refill_presets(select: Option<&str>) {
    let c = ctl(hwnd(), ID_PRESET);
    if c.0.is_null() {
        return;
    }
    let _ = SendMessageW(c, CB_RESETCONTENT, None, None);
    for preset in presets::list(&presets::dir()) {
        let hs = HSTRING::from(&preset.name);
        let _ = SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
    if let Some(name) = select {
        let _ = SetWindowTextW(c, &HSTRING::from(name));
    }
}

unsafe fn picked_text(c: HWND, sel: usize) -> String {
    let n = SendMessageW(c, CB_GETLBTEXTLEN, Some(WPARAM(sel)), None).0;
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = SendMessageW(c, CB_GETLBTEXT, Some(WPARAM(sel)), Some(LPARAM(buf.as_mut_ptr() as isize))).0;
    if got <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(got as usize).min(n as usize)])
}

unsafe fn load_preset() {
    let h = hwnd();
    // During CBN_SELCHANGE the edit still holds the old text. Read the picked list item.
    let c = ctl(h, ID_PRESET);
    let sel = SendMessageW(c, CB_GETCURSEL, None, None).0;
    let name = if sel < 0 { crate::text_of(c) } else { picked_text(c, sel as usize) };
    let Some(preset) = presets::list(&presets::dir()).into_iter().find(|p| p.name == name) else { return };
    let rules = if preset.rules.is_empty() {
        vec![Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false }]
    } else {
        preset.rules.into_iter().take(MAX_RULES).collect::<Vec<_>>()
    };
    let mut enabled = preset.enabled;
    enabled.resize(rules.len(), true);
    let sel = if preset.files && !preset.folders {
        1
    } else if !preset.files && preset.folders {
        2
    } else {
        0
    };
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.filling = true;
        st.rules = rules;
        st.enabled = enabled;
        st.note.clear();
        st.rule_scroll = 0;
    });
    let _ = SendMessageW(ctl(h, ID_FILTER), CB_SETCURSEL, Some(WPARAM(sel)), None);
    let _ = SetWindowTextW(ctl(h, ID_EXT), &HSTRING::from(&preset.extensions));
    ST.with(|s| s.borrow_mut().filling = false);
    rebuild_rule_controls();
    layout(h);
    recompute();
}

unsafe fn save_preset() {
    if ST.with(|s| s.borrow().busy) {
        return;
    }
    let h = hwnd();
    let name = crate::text_of(ctl(h, ID_PRESET)).trim().to_string();
    if name.is_empty() {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let n = ST.with(|s| s.borrow().rules.len());
    let mut rules = Vec::with_capacity(n);
    let mut enabled = Vec::with_capacity(n);
    for i in 0..n {
        if let Some(rule) = read_rule(i) {
            rules.push(rule);
            enabled.push(rule_enabled(i));
        }
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.rules.clone_from(&rules);
        st.enabled.clone_from(&enabled);
    });
    let (files, folders) = filter_flags();
    let preset = Preset { name: name.clone(), rules, files, folders, extensions: crate::text_of(ctl(h, ID_EXT)), enabled };
    let note = match presets::save(&presets::dir(), &preset) {
        Ok(()) => {
            ST.with(|s| s.borrow_mut().filling = true);
            refill_presets(Some(&name));
            ST.with(|s| s.borrow_mut().filling = false);
            tf(crate::Key::SavedPreset, &[("name", &name)])
        }
        Err(e) => format!("{}{e}", tr(crate::Key::CouldNotSavePreset)),
    };
    ST.with(|s| s.borrow_mut().note = note);
    let _ = InvalidateRect(Some(h), None, false);
}

unsafe fn delete_preset() {
    if ST.with(|s| s.borrow().busy) {
        return;
    }
    let h = hwnd();
    let name = crate::text_of(ctl(h, ID_PRESET)).trim().to_string();
    if name.is_empty() {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let note = match presets::delete(&presets::dir(), &name) {
        Ok(()) => {
            ST.with(|s| s.borrow_mut().filling = true);
            refill_presets(None);
            let _ = SetWindowTextW(ctl(h, ID_PRESET), &HSTRING::from(""));
            ST.with(|s| s.borrow_mut().filling = false);
            tf(crate::Key::DeletedPreset, &[("name", &name)])
        }
        Err(e) => format!("{}{e}", tr(crate::Key::CouldNotDeletePreset)),
    };
    ST.with(|s| s.borrow_mut().note = note);
    let _ = InvalidateRect(Some(h), None, false);
}

fn summary_text(n: usize, changes: usize, errors: usize, skipped: usize) -> String {
    let n = n.to_string();
    let changes = changes.to_string();
    let base = if errors == 0 {
        tf(crate::Key::SummaryOk, &[("n", &n), ("changes", &changes)])
    } else {
        tf(crate::Key::SummaryErr, &[("n", &n), ("changes", &changes), ("e", &errors.to_string())])
    };
    if skipped == 0 {
        return base;
    }
    format!("{base}{}", tf(crate::Key::SummarySkipped, &[("skipped", &skipped.to_string())]))
}

fn renamed_note(n: usize) -> String {
    tf(crate::Key::RenamedN, &[("n", &n.to_string())])
}

fn undone_note(n: usize) -> String {
    tf(crate::Key::UndoneN, &[("n", &n.to_string())])
}

fn undo_skipped_note(n: usize) -> String {
    tf(crate::Key::UndoSkippedMissing, &[("n", &n.to_string())])
}

/// The summary after an undo. Missing names are one sentence, not a per-file error.
fn undo_result_note(undone: usize, skipped: usize) -> String {
    let base = if undone == 0 { String::new() } else { undone_note(undone) };
    if skipped == 0 {
        return base;
    }
    let skip = undo_skipped_note(skipped);
    if base.is_empty() { skip } else { format!("{base} {skip}") }
}

fn issue_text(issue: Issue) -> &'static str {
    match issue {
        Issue::Empty => tr(crate::Key::EmptyName),
        Issue::InvalidChars => tr(crate::Key::InvalidCharacter),
        Issue::Reserved => tr(crate::Key::ReservedName),
        Issue::Trailing => tr(crate::Key::EndsWithSpaceOrDot),
        Issue::TooLong => tr(crate::Key::PathTooLong),
        Issue::Duplicate => tr(crate::Key::DuplicateName),
        Issue::Exists => tr(crate::Key::AlreadyExists),
        Issue::BadPattern => tr(crate::Key::InvalidPattern),
    }
}

unsafe fn collect_pane(i: usize) -> Vec<BatchItem> {
    let Some(fv) = crate::view_of::<IFolderView2>(i) else { return Vec::new() };
    // GetSelection only. Walking SVGIO_ALLVIEW with GetSelectionState can report every
    // child of a shell namespace (This PC / user profile) as selected, so the preview
    // listed 3D Objects / Downloads / OneDrive instead of the files the user highlighted (L3).
    let mut out = collect_selection(&fv);
    if out.is_empty() {
        out = collect_selection_enum(&fv);
    }
    scope_to_folder(&fv, out)
}

unsafe fn scope_to_folder(fv: &IFolderView2, mut out: Vec<BatchItem>) -> Vec<BatchItem> {
    let Some(folder) = fv
        .GetFolder::<IShellItem>()
        .ok()
        .and_then(|it| it.GetDisplayName(SIGDN_FILESYSPATH).ok())
        .map(|p| crate::pwstr(p))
    else {
        return out;
    };
    let folder = folder.trim_end_matches(['\\', '/']).to_string();
    if !crate::is_fs(&folder) {
        return out;
    }
    out.retain(|it| engine::dir_key(&it.dir) == engine::dir_key(&folder));
    out
}

unsafe fn collect_selection_enum(fv: &IFolderView2) -> Vec<BatchItem> {
    let Ok(en) = fv.Items::<IEnumIDList>(SVGIO_SELECTION) else { return Vec::new() };
    let Ok(sf) = fv.GetFolder::<IShellFolder>() else { return Vec::new() };
    let mut out = Vec::new();
    loop {
        if out.len() >= 10_000 {
            break;
        }
        let mut slot = [std::ptr::null_mut()];
        let mut n = 0u32;
        let hr = en.Next(&mut slot, Some(&mut n));
        if !hr.is_ok() || n == 0 || slot[0].is_null() {
            break;
        }
        let pidl = slot[0];
        let mut attr = SFGAO_FOLDER.0 | SFGAO_STREAM.0;
        let pidl_ref = pidl as *const ITEMIDLIST;
        let _ = sf.GetAttributesOf(&[pidl_ref], &mut attr);
        let is_dir = crate::real_folder(attr);
        let mut sr = STRRET::default();
        let mut name = PWSTR::null();
        let path = if sf.GetDisplayNameOf(pidl_ref, SHGDNF(SHGDN_FORPARSING.0), &mut sr).is_ok()
            && StrRetToStrW(&mut sr, Some(pidl_ref), &mut name).is_ok()
        {
            crate::pwstr(name)
        } else {
            String::new()
        };
        CoTaskMemFree(Some(pidl as _));
        if let Some(it) = item_from_path(&path, is_dir) {
            out.push(it);
        }
    }
    out
}

unsafe fn collect_selection(fv: &IFolderView2) -> Vec<BatchItem> {
    let Ok(arr) = fv.GetSelection(false) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0).min(10_000);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { continue };
        let attr = it.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM).map(|f| f.0).unwrap_or(0);
        let is_dir = crate::real_folder(attr);
        let Ok(p) = it.GetDisplayName(SIGDN_FILESYSPATH) else { continue };
        let path = crate::pwstr(p);
        if let Some(item) = item_from_path(&path, is_dir) {
            out.push(item);
        }
    }
    out
}

fn item_from_path(path: &str, is_dir_hint: bool) -> Option<BatchItem> {
    if !crate::is_fs(path) {
        return None;
    }
    let path = path.trim_end_matches(['\\', '/']);
    let (dir, name) = match path.rsplit_once('\\').or_else(|| path.rsplit_once('/')) {
        Some((dir, name)) if !name.is_empty() => (dir.to_string(), name.to_string()),
        _ => return None,
    };
    let full = engine::join_path(&dir, &name);
    let is_dir = std::fs::metadata(&full).map(|m| m.is_dir()).unwrap_or(is_dir_hint);
    let (modified, created, accessed) = file_times(&full);
    let mut item = BatchItem::new(dir, name, is_dir);
    item.modified = modified;
    item.created = created;
    item.accessed = accessed;
    Some(item)
}

fn file_times(path: &str) -> (Option<i64>, Option<i64>, Option<i64>) {
    let Ok(meta) = std::fs::metadata(path) else { return (None, None, None) };
    let secs = |t: std::io::Result<std::time::SystemTime>| {
        t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64)
    };
    (secs(meta.modified()), secs(meta.created()), secs(meta.accessed()))
}

fn load_existing(items: &[BatchItem]) -> HashSet<(String, String)> {
    let mut dirs: Vec<String> = Vec::new();
    for it in items {
        if !dirs.iter().any(|d| engine::dir_key(d) == engine::dir_key(&it.dir)) {
            dirs.push(it.dir.clone());
        }
    }
    let mut set = HashSet::new();
    for d in &dirs {
        let path = if d.ends_with(':') { format!("{d}\\") } else { d.clone() };
        if let Ok(rd) = std::fs::read_dir(&path) {
            for ent in rd.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                set.insert((engine::dir_key(d), engine::name_key(&name)));
            }
        }
    }
    for it in items {
        set.insert((engine::dir_key(&it.dir), engine::name_key(&it.name)));
    }
    set
}

unsafe fn style_window(h: HWND) {
    crate::style_dialog_caption(h);
}

/// Hints and column titles. The app meta face, or the body face before it exists.
unsafe fn meta_font() -> HFONT {
    if let Some(font) = crate::app_try(|a| a.small_font) {
        if !font.0.is_null() && !font.is_invalid() {
            return font;
        }
    }
    crate::body_font()
}

unsafe fn apply_font(parent: HWND, font: HFONT) {
    let mut c = FindWindowExW(Some(parent), None, PCWSTR::null(), PCWSTR::null()).unwrap_or_default();
    while !c.0.is_null() {
        let _ = SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        theme_ctl(c);
        c = FindWindowExW(Some(parent), Some(c), PCWSTR::null(), PCWSTR::null()).unwrap_or_default();
    }
}

unsafe fn theme_ctl(h: HWND) {
    if crate::ui::combo::is_styled(h) {
        crate::ui::combo::clear_theme(h);
        return;
    }
    let dark = crate::DARK_ON.load(std::sync::atomic::Ordering::Relaxed);
    let name = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

unsafe fn theme_btn(h: HWND) {
    let dark = crate::DARK_ON.load(std::sync::atomic::Ordering::Relaxed);
    let name = if dark { w!("DarkMode_Explorer") } else { w!("Explorer") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

unsafe fn child(parent: HWND, class: PCWSTR, text: &str, style: u32, id: i32, font: HFONT) -> HWND {
    let t = HSTRING::from(text);
    crate::child(parent, class, PCWSTR(t.as_ptr()), style, WINDOW_EX_STYLE(0), id, font)
}

fn id(i: usize, f: i32) -> i32 {
    ID_BASE + i as i32 * STRIDE + f
}

fn ctl(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

unsafe fn cue(h: HWND, text: &str) {
    if h.0.is_null() {
        return;
    }
    let hs = HSTRING::from(text);
    let _ = SendMessageW(h, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(hs.as_ptr() as isize)));
}

unsafe fn set_btn(parent: HWND, id: i32, text: &str) {
    let c = ctl(parent, id);
    if !c.0.is_null() {
        let _ = SetWindowTextW(c, &HSTRING::from(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(w: i32, h: i32) -> RECT {
        RECT { left: 0, top: 0, right: w, bottom: h }
    }

    fn replace_rule() -> Rule {
        Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false }
    }

    #[test]
    fn panels_sit_side_by_side_at_100_and_150_percent() {
        for dpi in [96, 120, 144] {
            let g = geo(client(sc(dpi, DLG_W_DIP), sc(dpi, DLG_H_DIP)), dpi);
            let left_w = g.host.right - g.host.left;
            let right_w = g.list.right - g.list.left;
            assert!(g.host.right <= g.list.left, "dpi {dpi}");
            assert_eq!(g.host.top, g.list.top, "dpi {dpi}");
            assert_eq!(g.host.bottom, g.list.bottom, "dpi {dpi}");
            assert!((left_w - right_w).abs() <= 1, "dpi {dpi} left {left_w} right {right_w}");
            assert!(left_w >= sc(dpi, COL_MIN_DIP), "dpi {dpi} left {left_w}");
            assert!(right_w >= sc(dpi, COL_MIN_DIP), "dpi {dpi} right {right_w}");
            // Toolbar spans the window. Column titles sit under it, panels under those.
            assert!(g.tools.bottom <= g.rules_lbl.top, "dpi {dpi}");
            assert!(g.tools.right > g.host.right, "dpi {dpi}");
            assert_eq!(g.add.top, g.tools.top, "dpi {dpi}");
            assert!(g.add.left >= g.tools.right, "dpi {dpi}");
            assert_eq!(g.prev_lbl.top, g.rules_lbl.top, "dpi {dpi}");
            assert!(g.rules_lbl.bottom <= g.host.top, "dpi {dpi}");
            assert!(g.prev_lbl.left >= g.list.left, "dpi {dpi}");
            assert!(g.summary.right <= g.list.right, "dpi {dpi}");
            assert!(g.undo.top >= g.host.bottom, "dpi {dpi}");
            assert_eq!(g.close.top, g.undo.top, "dpi {dpi}");
            assert_eq!(g.apply.top, g.undo.top, "dpi {dpi}");
            assert!(g.apply.left > g.list.left, "dpi {dpi}");
            let tools = tool_rects(g.tools, dpi);
            assert_eq!(tools.ext.top, tools.filter.top, "dpi {dpi}");
            assert_eq!(tools.ext.bottom, tools.filter.bottom, "dpi {dpi}");
            assert_eq!(tools.ext.bottom - tools.ext.top, g.tools.bottom - g.tools.top, "dpi {dpi}");
            assert!(tools.del.right <= g.tools.right, "dpi {dpi} del {} tools {}", tools.del.right, g.tools.right);
        }
    }

    #[test]
    fn minimum_window_keeps_both_columns_and_the_footer() {
        for dpi in [96, 120, 144] {
            let g = geo(client(sc(dpi, DLG_MIN_W_DIP), sc(dpi, DLG_MIN_H_DIP)), dpi);
            assert!(g.host.right > g.host.left, "dpi {dpi}");
            assert!(g.list.right > g.list.left, "dpi {dpi}");
            assert!(g.host.bottom > g.host.top, "dpi {dpi}");
            assert!(g.host.right <= g.list.left, "dpi {dpi}");
            assert!(g.tools.bottom <= g.host.top, "dpi {dpi}");
            assert!(g.undo.top >= g.host.bottom, "dpi {dpi}");
            assert!(g.apply.right <= sc(dpi, DLG_MIN_W_DIP), "dpi {dpi}");
            let left_w = g.host.right - g.host.left;
            let right_w = g.list.right - g.list.left;
            assert!((left_w - right_w).abs() <= 1, "dpi {dpi}");
        }
    }

    #[test]
    fn edit_frames_match_the_row_and_centre_the_line_at_100_and_150_percent() {
        for dpi in [96, 120, 144] {
            let frame_h = sc(dpi, FIELD_H_DIP);
            let outer = RECT { left: 40, top: 80, right: 220, bottom: 80 + frame_h };
            let inner = field_inner(outer, dpi);
            let inset = sc(dpi, 2);
            assert_eq!(inner.top - outer.top, inset, "dpi {dpi}");
            assert_eq!(outer.bottom - inner.bottom, inset, "dpi {dpi}");
            assert_eq!(inner.left - outer.left, inset, "dpi {dpi}");
            assert_eq!(outer.right - inner.right, inset, "dpi {dpi}");
            // Stand-ins for Segoe UI / Malgun Gothic / Yu Gothic UI tmHeight at 14px.
            for px in [18, 19, 21] {
                let line = (px * dpi / 96).max(1);
                let text = crate::centered_edit_rect(inner, line);
                let above = text.top - outer.top;
                let below = outer.bottom - text.bottom;
                assert!((above - below).abs() <= 1, "dpi {dpi} line {line} above {above} below {below}");
                assert!(text.top >= outer.top && text.bottom <= outer.bottom, "dpi {dpi}");
                assert_eq!(text.bottom - text.top, line.min(inner.bottom - inner.top), "dpi {dpi}");
            }
            let wide = sc(dpi, 800);
            let card = RECT { left: sc(dpi, 2), top: 0, right: wide - sc(dpi, 2), bottom: sc(dpi, 120) };
            let slots = card_slots(card, dpi, &replace_rule());
            let texts: Vec<_> = slots.iter().filter(|(f, _)| is_text_field(*f)).collect();
            assert!(texts.len() >= 2, "dpi {dpi}");
            for (_, r) in texts {
                assert_eq!(r.bottom - r.top, frame_h, "dpi {dpi}");
            }
        }
    }

    #[test]
    fn replace_checks_sit_under_the_fields_at_100_125_and_150_percent() {
        let rule = replace_rule();
        let prefix = Rule::Prefix { text: String::new() };
        let number = Rule::Number { start: 1, step: 1, pad: 0, pos: NumPos::Suffix };
        assert_eq!(rule_row_count(&rule), 3);
        assert_eq!(rule_row_count(&prefix), 2);
        assert_eq!(rule_row_count(&number), 3);
        for dpi in [96, 120, 144] {
            let body = rule_stride(&rule, dpi) - sc(dpi, CARD_GAP_DIP);
            let expect = sc(dpi, CARD_PAD_DIP) * 2 + 3 * sc(dpi, FIELD_H_DIP) + 2 * sc(dpi, ROW_GAP_DIP);
            assert_eq!(body, expect, "dpi {dpi}");
            assert_eq!(rule_stride(&prefix, dpi) - sc(dpi, CARD_GAP_DIP), sc(dpi, CARD_PAD_DIP) * 2 + 2 * sc(dpi, FIELD_H_DIP) + sc(dpi, ROW_GAP_DIP), "dpi {dpi}");
            let card = RECT { left: 0, top: sc(dpi, 4), right: sc(dpi, 700), bottom: sc(dpi, 400) };
            let slots = card_slots(card, dpi, &rule);
            let row_of = |field: i32| slots.iter().find(|(f, _)| *f == field).map(|(_, r)| r.top);
            let text_top = row_of(F_A).unwrap();
            let check_top = row_of(F_CASE).unwrap();
            assert_eq!(text_top, row_top(card, dpi, 1), "dpi {dpi}");
            assert_eq!(check_top, row_top(card, dpi, 2), "dpi {dpi}");
            assert!(check_top > text_top, "dpi {dpi}");
            assert_eq!(row_of(F_EXT), Some(check_top), "dpi {dpi}");
            assert_eq!(row_of(F_B), Some(text_top), "dpi {dpi}");
            for (_, r) in slots.iter().filter(|(f, _)| is_text_field(*f)) {
                assert_eq!(r.bottom - r.top, sc(dpi, FIELD_H_DIP), "dpi {dpi}");
            }
        }
    }

    #[test]
    fn header_tick_grip_and_kind_stay_on_one_row() {
        for dpi in [96, 120, 144] {
            let stride = rule_stride(&replace_rule(), dpi);
            let card = card_rect(0, stride, sc(dpi, 640), dpi);
            let head = head_controls(card, dpi);
            let row = row_top(card, dpi, 0);
            let h = sc(dpi, FIELD_H_DIP);
            assert_eq!(head.kind.top, row, "dpi {dpi}");
            assert_eq!(head.up.top, row, "dpi {dpi}");
            assert_eq!(head.down.top, head.up.top, "dpi {dpi}");
            assert_eq!(head.del.top, head.up.top, "dpi {dpi}");
            assert_eq!(head.tick.top, row, "dpi {dpi}");
            assert_eq!(head.tick.bottom - head.tick.top, h, "dpi {dpi}");
            assert_eq!(head.tick.right - head.tick.left, sc(dpi, TICK_DIP), "dpi {dpi}");
            assert!(head.kind.left >= head.tick.right, "dpi {dpi}");
            assert!(head.kind.right <= head.up.left, "dpi {dpi}");
            assert!(head.up.right <= head.down.left, "dpi {dpi}");
            assert!(head.down.right <= head.del.left, "dpi {dpi}");
            assert!(head.del.right <= card.right, "dpi {dpi}");
            let dots = grip_dots(card, dpi);
            assert_eq!(dots.len(), 6);
            let row_mid = row + h / 2;
            let grip_mid = (dots[0].top + dots[4].bottom) / 2;
            assert!((grip_mid - row_mid).abs() <= 1, "dpi {dpi} grip {grip_mid} row {row_mid}");
            assert!(dots[0].right <= head.tick.left, "dpi {dpi}");
            assert!(drag_max_x(dpi) < head.tick.left, "dpi {dpi}");
            // Footer: undo on the left, close then the primary rename on the right.
            let g = geo(client(sc(dpi, DLG_W_DIP), sc(dpi, DLG_H_DIP)), dpi);
            assert!(g.undo.right < g.close.left, "dpi {dpi}");
            assert!(g.close.right <= g.apply.left, "dpi {dpi}");
            assert_eq!(g.apply.bottom - g.apply.top, sc(dpi, crate::BTN_H_FOOTER), "dpi {dpi}");
            assert_eq!(g.tools.bottom - g.tools.top, sc(dpi, FIELD_H_DIP), "dpi {dpi}");
        }
    }

    #[test]
    fn cerulean_mark_and_primary_never_use_mint() {
        let mint = 0x2EC4A6u32;
        let hover = crate::button_colors(crate::ButtonKind::Primary, crate::ButtonState::Hover, false, 0xF3F1EC).fill;
        let pressed = crate::button_colors(crate::ButtonKind::Primary, crate::ButtonState::Pressed, false, 0xF3F1EC).fill;
        for c in [CERULEAN, CERULEAN_DEEP, MARK_INK_DARK, hover.unwrap_or(0), pressed.unwrap_or(0), mark_ink(false), mark_ink(true), mark_wash(false, 0xFFFFFF), mark_wash(true, 0x26282C), row_hover(false), row_hover(true)] {
            assert_ne!(c, mint, "{c:#08X}");
        }
        assert_eq!(CERULEAN, 0x0A9BD6);
        assert_eq!(CERULEAN_DEEP, 0x0786BC);
        assert_eq!(hover, Some(0x0890C7));
        assert_eq!(pressed, Some(CERULEAN_DEEP));
        assert_ne!(hover, pressed);
        assert_eq!(mark_ink(false), CERULEAN_DEEP);
        assert_eq!(mark_ink(true), 0x7FD4F5);
        // Light wash is 28% cerulean on white. Dark wash is 32% cerulean on the card.
        assert_eq!(mark_wash(false, 0xFFFFFF), crate::blend(0xFFFFFF, CERULEAN, 0.28));
        assert_eq!(mark_wash(true, 0x26282C), crate::blend(0x26282C, CERULEAN, 0.32));
        assert_eq!(row_hover(false), 0xF7F5F1);
        assert_eq!(row_hover(true), 0x2A2C30);
        let off = crate::blend(0xF3F1EC, 0xFFFFFF, 0.55);
        assert_ne!(off, mint);
        assert_ne!(off, 0xFFFFFF);
    }

    #[test]
    fn wash_covers_the_measured_advance() {
        // pre ends at 30, the changed run is 48 wide, pad is 1. The wash contains that run.
        let band = span_band(30, 48, 8, 18, 1);
        assert_eq!(band.left, 29);
        assert_eq!(band.right, 79);
        assert!(band.left <= 30 && band.right >= 78);
        assert_eq!(band.bottom - band.top, 18);
    }

    #[test]
    fn changed_span_width_matches_get_text_extent() {
        unsafe {
            let dc = CreateCompatibleDC(None);
            assert!(!dc.is_invalid());
            let font = HFONT(GetStockObject(DEFAULT_GUI_FONT).0);
            for (face, s) in [(font, "IMG_"), (font, "여행_"), (font, "0012.jpg")] {
                let got = text_w(dc, face, s);
                let w: Vec<u16> = s.encode_utf16().collect();
                let old = SelectObject(dc, face.into());
                let mut sz = SIZE::default();
                let ok = GetTextExtentPoint32W(dc, &w, &mut sz).as_bool();
                SelectObject(dc, old);
                assert!(ok, "{s}");
                assert_eq!(got, sz.cx, "{s}");
                assert!(got > 0, "{s}");
            }
            let _ = DeleteDC(dc);
        }
    }
}
