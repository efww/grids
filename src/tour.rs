use crate::*;
use std::cell::Cell;
use windows::Win32::UI::Controls::ODS_FOCUS;

// ---------- getting started tour: five bubbles pointing at the real buttons ----------
// Shown once on a new install (no grids.ini, no folder or session given), and from the View menu any time.
// Like the shortcut list it is an owned tool window that closes when it loses activation: a click elsewhere
// still lands where it was aimed, the tour just goes.

pub(crate) const WM_TOUR: u32 = WM_APP + 53; // posted to main once the first window is up: open the tour
const WM_TOUR_END: u32 = WM_APP + 1; // to the bubble, after a button press: close (wParam 1: then the shortcut list)

const STEPS: usize = 5;
const ID_T_SKIP: i32 = 1;
const ID_T_KEYS: i32 = 2;
const ID_T_BACK: i32 = 3;
const ID_T_NEXT: i32 = 4;
const BUTTONS: [i32; 4] = [ID_T_SKIP, ID_T_KEYS, ID_T_BACK, ID_T_NEXT]; // also the Tab order

#[derive(Clone, Copy)]
struct Tour {
    h: HWND,
    step: usize,
    prev: HWND,   // focus before it opened: Esc, Skip and Done go back there
    target: HWND, // the button pointed at, drawn hot with a ring (draw_button)
    bold: HFONT,
    tail: Option<(bool, i32)>, // (tail on top, i.e. bubble below the target; tip x in px); None: centred, no tail
}

const NO_TOUR: Tour = Tour { h: HWND(std::ptr::null_mut()), step: 0, prev: HWND(std::ptr::null_mut()), target: HWND(std::ptr::null_mut()), bold: HFONT(std::ptr::null_mut()), tail: None };

thread_local! {
    static TOUR: Cell<Tour> = const { Cell::new(NO_TOUR) };
}

fn update(f: impl FnOnce(&mut Tour)) {
    let mut t = TOUR.get();
    f(&mut t);
    TOUR.set(t);
}

pub(crate) fn tour_hwnd() -> HWND {
    TOUR.get().h
}

pub(crate) fn tour_target() -> HWND {
    TOUR.get().target
}

// (title, body) of a step; at most a few short lines each
fn step_text(i: usize) -> (&'static str, &'static str) {
    match i {
        0 => (
            tr("Layout", "레이아웃"),
            tr(
                "Click: even · drag: panes per column. Border: drag to resize, double-click to even. Replay: View menu.",
                "클릭: 균등 · 끌기: 열마다 칸 수 · 경계선: 끌어 크기, 두 번 눌러 고르게. 다시 보기: 보기 메뉴",
            ),
        ),
        1 => (
            tr("Address bar", "주소창"),
            tr(
                "Path: click a folder to go, › to look inside, Ctrl+L to type. Header: double-click zooms, right-click colours.",
                "경로 폴더를 누르면 이동, ›는 하위 폴더, Ctrl+L 입력. 머리줄 두 번 = 이 패널만 크게, 오른쪽 클릭 = 색 라벨.",
            ),
        ),
        2 => (
            tr("⇄ Other panes", "⇄ 다른 패널"),
            tr(
                "Open the selected folder in another pane, copy or move files there, or swap places. Switch panes: Ctrl+Tab, Ctrl+1–9.",
                "선택한 폴더를 다른 패널에서 열기, 파일 복사·이동, 자리 바꾸기. 패널 옮겨 가기: Ctrl+Tab, Ctrl+1~9.",
            ),
        ),
        3 => (
            tr("Save", "저장"),
            tr(
                "Layout and folders are saved by themselves. Save with a name to keep this setup as a card under Load.",
                "배치와 폴더는 저절로 저장됩니다. 이름을 붙여 저장하면 불러오기에 카드로 남습니다.",
            ),
        ),
        _ => (
            tr("View menu", "보기 메뉴"),
            tr(
                "Folder tree, tabs, hidden files, colours, language and more. All shortcuts: Ctrl+/ · Help: F1.",
                "폴더 트리·탭·숨김 파일·색·언어 등. 모든 단축키: Ctrl+/ · 도움말: F1.",
            ),
        ),
    }
}

fn label(id: i32, last: bool) -> &'static str {
    match id {
        ID_T_SKIP => tr("Skip", "건너뛰기"),
        ID_T_KEYS => tr("Shortcuts  Ctrl+/", "단축키 보기  Ctrl+/"),
        ID_T_BACK => tr("Back", "이전"),
        _ if last => tr("Done", "마치기"),
        _ => tr("Next", "다음"),
    }
}

// What a screen reader reads on the Next button: the whole step, then the button.
pub(crate) fn step_name(i: usize) -> String {
    let (title, body) = step_text(i);
    format!("{} {}/{STEPS}, {title}: {} {}", tr("Step", "단계"), i + 1, body.replace('\n', " "), label(ID_T_NEXT, i + 1 == STEPS))
}

// Where the bubble goes: under the target (tail up), else over it (tail down), else centred on the owner
// without a tail. The bubble stays on the work area; the tip stays over the target, `edge` in from the corners.
// Returns the window's top-left and the tail; the window is h tall plus `tail` when it has one.
pub(crate) fn tour_place_at(target: Option<RECT>, w: i32, h: i32, work: RECT, owner: RECT, tail: i32, edge: i32) -> (i32, i32, Option<(bool, i32)>) {
    let fit_x = |x: i32| x.clamp(work.left, (work.right - w).max(work.left));
    if let Some(t) = target {
        let cx = (t.left + t.right) / 2;
        let x = fit_x(cx - w / 2);
        let tip = (cx - x).clamp(edge, (w - edge).max(edge));
        if t.bottom + tail + h <= work.bottom {
            return (x, t.bottom, Some((true, tip)));
        }
        if t.top - tail - h >= work.top {
            return (x, t.top - tail - h, Some((false, tip)));
        }
    }
    let x = fit_x((owner.left + owner.right - w) / 2);
    let y = ((owner.top + owner.bottom - h) / 2).clamp(work.top, (work.bottom - h).max(work.top));
    (x, y, None)
}

// The bubble's outline in client px: the card plus its tail. For the window region (`region`) the right and
// bottom edges sit one past the last pixel; for drawing, on it.
fn outline(w: i32, h: i32, tail: Option<(bool, i32)>, th: i32, region: bool) -> Vec<POINT> {
    let e = if region { 0 } else { 1 };
    let p = |x: i32, y: i32| POINT { x, y };
    let r = w - e;
    match tail {
        None => vec![p(0, 0), p(r, 0), p(r, h - e), p(0, h - e)],
        Some((true, tip)) => vec![p(0, th), p(tip - th, th), p(tip, 0), p(tip + th + (1 - e), th), p(r, th), p(r, th + h - e), p(0, th + h - e)],
        Some((false, tip)) => {
            let b = h - e;
            vec![p(0, 0), p(r, 0), p(r, b), p(tip + th + (1 - e), b), p(tip, b + th), p(tip - th, b), p(0, b)]
        }
    }
}

struct Geo {
    w: i32,
    h: i32, // the card, without the tail
    title: RECT,
    dots: RECT,
    body: RECT,
    btns: [(i32, RECT, bool); 4], // id, place, shown
}

fn tail_h(dpi: i32) -> i32 {
    9 * dpi / 96
}

// The card's layout for a step, from its top-left.
unsafe fn geo(step: usize) -> Geo {
    let (dpi, font, lh) = app(|a| (a.dpi, a.font, a.fh));
    let s = |v: i32| v * dpi / 96;
    let (w, pad) = (s(380), s(16));
    let last = step + 1 == STEPS;
    let body = step_text(step).1;
    let dc = GetDC(None);
    let (dot, dgap) = (s(6), s(4));
    let dots_w = STEPS as i32 * dot + (STEPS as i32 - 1) * dgap;
    let title_r = RECT { left: pad, top: pad, right: w - pad - dots_w - s(8), bottom: pad + lh };
    let dots = RECT { left: w - pad - dots_w, top: pad + (lh - dot) / 2, right: w - pad, bottom: pad + (lh - dot) / 2 + dot };
    let bh = wrap_text(dc, RECT { right: w - 2 * pad, ..Default::default() }, body, font, 0, true).1;
    let body_r = RECT { left: pad, top: title_r.bottom + s(6), right: w - pad, bottom: title_r.bottom + s(6) + bh };
    let (top, btn_h) = (body_r.bottom + s(14), (lh + s(10)).max(s(28)));
    let bw = |id: i32| (text_w(dc, label(id, last), font) + s(24)).max(s(72));
    ReleaseDC(None, dc);
    let row = |l: i32, r: i32| RECT { left: l, top, right: r, bottom: top + btn_h };
    let next = row(w - pad - bw(ID_T_NEXT), w - pad);
    let back = row(next.left - s(8) - bw(ID_T_BACK), next.left - s(8));
    let skip = row(pad, pad + bw(ID_T_SKIP));
    let keys = row(pad, pad + bw(ID_T_KEYS));
    let btns = [(ID_T_SKIP, skip, !last), (ID_T_KEYS, keys, last), (ID_T_BACK, back, step > 0), (ID_T_NEXT, next, true)];
    Geo { w, h: top + btn_h + pad, title: title_r, dots, body: body_r, btns }
}

unsafe fn button(id: i32) -> HWND {
    GetDlgItem(Some(TOUR.get().h), id).unwrap_or_default()
}

// The step's target: toolbar buttons on main, the active pane's address field and ⇄ button.
unsafe fn target_of(step: usize) -> HWND {
    let (main, active) = app(|a| (a.main, a.s.active));
    let tb = |id: i32| GetDlgItem(Some(main), id).unwrap_or_default();
    match step {
        0 => tb(ID_TB_LAYOUT),
        1 => pane_ctl(active, ID_EDIT).unwrap_or_default(),
        2 => pane_ctl(active, ID_OPEN_IN).unwrap_or_default(),
        3 => tb(ID_TB_SAVE),
        _ => tb(ID_TB_VIEW),
    }
}

// Where the target is on screen; None when it has no size or what holds it is hidden (a narrow pane, a zoomed-away
// pane). The address field counts while hidden behind the path crumbs: it is placed all the same and marks the bar.
unsafe fn target_rect(t: HWND) -> Option<RECT> {
    if t.0.is_null() {
        return None;
    }
    let parent = GetParent(t).unwrap_or_default();
    if !IsWindowVisible(parent).as_bool() || (GetDlgCtrlID(t) != ID_EDIT && !IsWindowVisible(t).as_bool()) {
        return None;
    }
    let mut r = RECT::default();
    GetWindowRect(t, &mut r).ok()?;
    (r.right > r.left && r.bottom > r.top).then_some(r)
}

// Sizes, points and shapes the bubble for its step; also after the main window was resized.
pub(crate) unsafe fn tour_place() {
    let t = TOUR.get();
    if t.h.0.is_null() {
        return;
    }
    let (main, dpi) = app(|a| (a.main, a.dpi));
    if IsIconic(main).as_bool() {
        return; // its buttons are off screen; placed again on restore
    }
    let g = geo(t.step);
    let th = tail_h(dpi);
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let mut owner = RECT::default();
    _ = GetWindowRect(main, &mut owner);
    let (x, y, tail) = tour_place_at(target_rect(t.target), g.w, g.h, mi.rcWork, owner, th, 18 * dpi / 96);
    update(|t| t.tail = tail);
    let off = if matches!(tail, Some((true, _))) { th } else { 0 };
    let total = g.h + if tail.is_some() { th } else { 0 };
    _ = SetWindowPos(t.h, None, x, y, g.w, total, SWP_NOZORDER | SWP_NOACTIVATE);
    let rgn = CreatePolygonRgn(&outline(g.w, g.h, tail, th, true), WINDING);
    SetWindowRgn(t.h, Some(rgn), true); // the system owns the region from here
    for (id, r, shown) in g.btns {
        let b = button(id);
        _ = MoveWindow(b, r.left, r.top + off, r.right - r.left, r.bottom - r.top, false);
        _ = ShowWindow(b, if shown { SW_SHOWNA } else { SW_HIDE });
    }
    _ = RedrawWindow(Some(t.h), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
}

unsafe fn show_step(i: usize) {
    let t = TOUR.get();
    if t.h.0.is_null() {
        return;
    }
    let target = target_of(i);
    update(|t| (t.step, t.target) = (i, target));
    for w in [t.target, target].into_iter().filter(|w| !w.0.is_null()) {
        _ = InvalidateRect(Some(w), None, false);
    }
    let last = i + 1 == STEPS;
    for id in BUTTONS {
        _ = SetWindowTextW(button(id), &HSTRING::from(label(id, last)));
    }
    tour_place();
    // the focus stays on Next from step to step, so a changed name alone is not read: say it changed
    let next = button(ID_T_NEXT);
    a11y_name(next, Some(&step_name(i)));
    NotifyWinEvent(EVENT_OBJECT_NAMECHANGE, next, OBJID_CLIENT.0, CHILDID_SELF as i32);
    if IsWindowVisible(t.h).as_bool() {
        _ = SetFocus(Some(next));
    }
}

pub(crate) unsafe fn open_tour() {
    let open = tour_hwnd();
    if !open.0.is_null() {
        _ = SetFocus(Some(button(ID_T_NEXT)));
        return;
    }
    let (main, font) = app(|a| (a.main, a.font));
    if main.0.is_null() || IsIconic(main).as_bool() || !IsWindowVisible(main).as_bool() {
        return;
    }
    let mut lf = LOGFONTW::default();
    GetObjectW(font.into(), size_of::<LOGFONTW>() as i32, Some(&mut lf as *mut _ as _));
    let bold = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, ..lf });
    let prev = GetFocus();
    let title = HSTRING::from(tr("Getting started tour", "처음 사용 안내"));
    let Ok(p) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsTour"), &title, WS_POPUP | WS_CLIPCHILDREN, 0, 0, 0, 0, Some(main), None, None, None) else {
        _ = DeleteObject(bold.into());
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(p, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    TOUR.set(Tour { h: p, step: 0, prev, target: HWND::default(), bold, tail: None });
    for id in BUTTONS {
        child(p, w!("BUTTON"), w!(""), BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0), id, font);
    }
    show_step(0);
    _ = ShowWindow(p, SW_SHOW);
    _ = SetFocus(Some(button(ID_T_NEXT)));
}

// Esc, Skip, Done: back to where the user was. (Losing activation just closes: focus is already elsewhere.)
pub(crate) unsafe fn close_tour(restore: bool) {
    let t = TOUR.get();
    if t.h.0.is_null() {
        return;
    }
    _ = DestroyWindow(t.h);
    if restore {
        if !t.prev.0.is_null() && IsWindow(Some(t.prev)).as_bool() {
            _ = SetFocus(Some(t.prev));
        } else {
            focus_view(app(|a| a.s.active));
        }
    }
}

// key() hands over keys on the bubble: Enter presses the focused button, → / ← step, Tab moves between the
// buttons (no dialog manager here), Esc closes, Ctrl+/ swaps it for the shortcut list. Everything else stays
// the bubble's, so no pane shortcut runs behind it. Holding a key steps once.
pub(crate) unsafe fn tour_key(m: &MSG) -> Option<bool> {
    let t = TOUR.get();
    if t.h.0.is_null() || !(m.hwnd == t.h || IsChild(t.h, m.hwnd).as_bool()) {
        return None;
    }
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    let (plain, repeat) = (!ctrl && !alt, m.lParam.0 & (1 << 30) != 0);
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        VK_ESCAPE => close_tour(true),
        VK_RETURN if plain => {
            if !repeat {
                // held down, it stays here: after Done its repeats must not open the list's item
                app(|x| x.held = Some(VK_RETURN.0));
                let id = Some(GetDlgCtrlID(m.hwnd)).filter(|id| GetParent(m.hwnd).unwrap_or_default() == t.h && BUTTONS.contains(id)).unwrap_or(ID_T_NEXT);
                SendMessageW(t.h, WM_COMMAND, Some(WPARAM((BN_CLICKED as usize) << 16 | id as usize)), Some(LPARAM(button(id).0 as isize)));
            }
        }
        VK_RIGHT if plain => {
            if !repeat && t.step + 1 < STEPS {
                show_step(t.step + 1);
            }
        }
        VK_LEFT if plain => {
            if !repeat && t.step > 0 {
                show_step(t.step - 1);
            }
        }
        VK_TAB if plain => {
            let shown: Vec<HWND> = BUTTONS.iter().map(|&id| button(id)).filter(|&b| IsWindowVisible(b).as_bool()).collect();
            if !shown.is_empty() {
                let n = shown.len();
                let at = shown.iter().position(|&b| b == GetFocus()).unwrap_or(n - 1);
                _ = SetFocus(Some(shown[if shift { (at + n - 1) % n } else { (at + 1) % n }]));
            }
        }
        VK_OEM_2 if ctrl && !shift && !alt => {
            if !repeat {
                close_tour(true);
                open_keys();
            }
        }
        _ => return Some(false),
    }
    Some(true)
}

unsafe fn paint_tour(h: HWND) {
    let t = TOUR.get();
    let (dpi, font, accent) = app(|a| (a.dpi, a.font, a.s.accent_now()));
    let bold = if t.bold.is_invalid() { font } else { t.bold };
    let g = geo(t.step);
    let th = tail_h(dpi);
    let off = if matches!(t.tail, Some((true, _))) { th } else { 0 };
    let mv = |r: RECT| RECT { top: r.top + off, bottom: r.bottom + off, ..r };
    let (title, body) = step_text(t.step);
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        // the card and its tail in one outline
        let old_pen = SelectObject(dc, GetStockObject(DC_PEN));
        let old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
        SetDCPenColor(dc, cref(pal().line));
        SetDCBrushColor(dc, cref(pal().card));
        _ = Polygon(dc, &outline(g.w, g.h, t.tail, th, false));
        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        text(dc, mv(g.title), title, bold, pal().text, DT_LEFT);
        // progress: done steps half way to the selection colour, this one in it
        let (dot, dgap) = (g.dots.bottom - g.dots.top, (g.dots.right - g.dots.left - STEPS as i32 * (g.dots.bottom - g.dots.top)) / (STEPS as i32 - 1));
        for k in 0..STEPS {
            let x = g.dots.left + k as i32 * (dot + dgap);
            let c = if k == t.step { accent } else if k < t.step { blend(pal().line, accent, 0.5) } else { pal().line };
            fill(dc, mv(RECT { left: x, right: x + dot, ..g.dots }), c);
        }
        wrap_text(dc, mv(g.body), body, font, pal().text, false);
    });
}

// Next in the selection colour, the rest as pills; the focused one gets a double edge.
unsafe fn draw_tour_button(d: &DRAWITEMSTRUCT) {
    let (dc, rc, id) = (d.hDC, d.rcItem, d.CtlID as i32);
    let (font, hover, accent) = app(|a| (a.font, a.hover, a.s.accent_now()));
    let primary = id == ID_T_NEXT;
    let pressed = d.itemState.0 & ODS_SELECTED.0 != 0;
    let focus = d.itemState.0 & ODS_FOCUS.0 != 0 || GetFocus() == d.hwndItem;
    let base = if primary { accent } else { pal().pill };
    let ink = if primary { on_color(accent) } else { pal().text };
    fill(dc, rc, base);
    if pressed || hover == d.hwndItem {
        fill(dc, rc, blend(base, ink, press_tint(primary, pressed)));
    }
    let edge = if primary { ink } else { accent };
    frame(dc, rc, 1, if focus { edge } else { pal().line });
    if focus {
        frame(dc, RECT { left: rc.left + 1, top: rc.top + 1, right: rc.right - 1, bottom: rc.bottom - 1 }, 1, edge);
    }
    text(dc, rc, &text_of(d.hwndItem), font, ink, DT_CENTER);
}

pub(crate) extern "system" fn tour_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => paint_tour(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_SETFOCUS => _ = SetFocus(Some(button(ID_T_NEXT))), // a click on the card leaves the keys with Next
            WM_DRAWITEM => {
                draw_tour_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            // an owner-drawn button sends a quick second click as BN_DBLCLK: it is a click all the same
            WM_COMMAND if matches!((wp.0 >> 16 & 0xffff) as u32, BN_CLICKED | BN_DBLCLK) => {
                let step = TOUR.get().step;
                match (wp.0 & 0xffff) as i32 {
                    ID_T_NEXT if step + 1 < STEPS => show_step(step + 1),
                    ID_T_BACK if step > 0 => show_step(step - 1),
                    // closing waits until the button is done with its click
                    ID_T_NEXT | ID_T_SKIP => _ = PostMessageW(Some(h), WM_TOUR_END, WPARAM(0), LPARAM(0)),
                    ID_T_KEYS => _ = PostMessageW(Some(h), WM_TOUR_END, WPARAM(1), LPARAM(0)),
                    _ => {}
                }
            }
            WM_TOUR_END => {
                close_tour(true);
                if wp.0 == 1 {
                    open_keys();
                }
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_CLOSE => _ = DestroyWindow(h),
            WM_DESTROY => {
                let t = TOUR.get();
                if t.h == h {
                    a11y_name(button(ID_T_NEXT), None);
                    TOUR.set(NO_TOUR);
                    if !t.target.0.is_null() {
                        _ = InvalidateRect(Some(t.target), None, false); // its ring goes
                    }
                    if !t.bold.is_invalid() {
                        _ = DeleteObject(t.bold.into());
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

    const WORK: RECT = RECT { left: 0, top: 0, right: 1000, bottom: 800 };
    const OWNER: RECT = RECT { left: 100, top: 100, right: 900, bottom: 700 };

    #[test]
    fn under_the_target_tip_over_it() {
        let t = RECT { left: 10, top: 5, right: 80, bottom: 29 };
        let (x, y, tail) = tour_place_at(Some(t), 380, 150, WORK, OWNER, 9, 18);
        assert_eq!((x, y), (0, 29)); // kept on the work area
        assert_eq!(tail, Some((true, 45))); // tip at the target's centre
    }

    #[test]
    fn over_the_target_when_no_room_below() {
        let t = RECT { left: 500, top: 700, right: 532, bottom: 724 };
        let (x, y, tail) = tour_place_at(Some(t), 380, 150, WORK, OWNER, 9, 18);
        assert_eq!((x, y), (326, 700 - 9 - 150));
        assert_eq!(tail, Some((false, 190)));
    }

    #[test]
    fn centred_without_a_target() {
        let (x, y, tail) = tour_place_at(None, 380, 150, WORK, OWNER, 9, 18);
        assert_eq!((x, y, tail), (310, 325, None));
    }

    #[test]
    fn tip_stays_off_the_corners() {
        let t = RECT { left: 990, top: 5, right: 1000, bottom: 29 };
        let (x, _, tail) = tour_place_at(Some(t), 380, 150, WORK, OWNER, 9, 18);
        assert_eq!(x, 620);
        assert_eq!(tail, Some((true, 380 - 18)));
    }

    #[test]
    fn every_step_names_its_place() {
        for i in 0..STEPS {
            let n = step_name(i);
            assert!(n.contains(&format!("{}/{STEPS}", i + 1)), "{n}");
        }
        assert!(step_name(STEPS - 1).ends_with(label(ID_T_NEXT, true)));
    }
}
