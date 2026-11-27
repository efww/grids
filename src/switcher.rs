use crate::*;
use windows::Win32::UI::Controls::{EM_SETCUEBANNER, MEASUREITEMSTRUCT};

// ---------- Ctrl+P: go to an open folder (a pane, or one of its tabs) by part of its name ----------
// Chrome's tab search, VS Code's Ctrl+P / editor list, Windows Terminal's tab switcher: with up to 16 panes and their
// tabs, Ctrl+1 … 9 and Alt+1 … 9 (the active pane's tabs) don't reach panes 10–16 nor another pane's tab. Nothing is
// browsed anew: the focus goes to a pane or a tab already open. The find box keeps the keyboard (↑/↓, PgUp/PgDn,
// Enter, Esc are taken in key() first, as the Everything popup's); the rows are an owner-drawn list box, so the
// wheel, a click and screen readers work on it as on any list.

const SW_EDIT: i32 = 1;
const SW_LIST: i32 = 2;
const SW_ROWS: usize = 12; // rows on screen at most; more scroll
const WM_SW_PICK: u32 = WM_APP + 90; // a row clicked: go there once the list box is done with the click
const WM_SW_REFOCUS: u32 = WM_APP + 91; // the list box took the focus (a click beside the rows): back to the find box

/// One row: a pane, or one tab of it (only with tabs on).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SwItem {
    pub(crate) pane: usize,
    pub(crate) tab: usize,
    pub(crate) tabs: usize, // tabs the pane has (1 with tabs off)
    pub(crate) here: bool,  // the active pane's current tab
    pub(crate) cur: bool,   // its pane's current tab (the folder on display)
    pub(crate) name: String,
    pub(crate) show: String, // the path as shown after the name; empty for shell places (their GUID says nothing)
    hay: String,             // what the find box looks in, lower case: both languages' place name and the whole path
}

/// The rows, in pane order (as the numbers on screen) and tab order within a pane. `panes`: each pane's tab_list and
/// current tab. With tabs off a pane is one row, its folder on display (tab_list also has the hidden tabs).
pub(crate) fn sw_items(panes: &[(Vec<String>, usize)], active: usize, tabs_on: bool, ko: bool) -> Vec<SwItem> {
    let mut out = Vec::new();
    for (i, (list, cur)) in panes.iter().enumerate() {
        let cur = (*cur).min(list.len().saturating_sub(1));
        let tabs: Vec<usize> = if tabs_on { (0..list.len()).collect() } else { vec![cur] };
        for k in tabs {
            let Some(path) = list.get(k) else { continue };
            let place = shell_place(path, ko);
            let name = place.clone().unwrap_or_else(|| short_name(path));
            let show = if place.is_some() || path.starts_with("::") { String::new() } else { path.clone() };
            let mut hay = vec![name.clone(), shell_place(path, !ko).unwrap_or_default(), path.clone()];
            if path.to_ascii_uppercase().contains(&THIS_PC[3..THIS_PC.len() - 1]) {
                hay.push("This PC\n내 PC".into());
            }
            let n = if tabs_on { list.len() } else { 1 };
            out.push(SwItem { pane: i, tab: k, tabs: n, here: i == active && k == cur, cur: k == cur, name, show, hay: hay.join("\n").to_lowercase() });
        }
    }
    out
}

/// The rows the find box keeps: every word typed (any case) is in the name or the path, as the shortcut list's find.
pub(crate) fn sw_filter(items: &[SwItem], query: &str) -> Vec<usize> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    (0..items.len()).filter(|&k| words.iter().all(|w| items[k].hay.contains(w.as_str()))).collect()
}

/// The row picked first (its place in `shown`): with nothing typed the next pane's folder on display (its current
/// tab, wrapping), so Ctrl+P, Enter goes on in pane order as Ctrl+Tab and browses nothing; with one pane only, the
/// row after where you are. Once typed, the first match that isn't where you already are.
pub(crate) fn sw_first(items: &[SwItem], shown: &[usize], typed: bool) -> Option<usize> {
    if shown.is_empty() {
        return None;
    }
    let here = shown.iter().position(|&k| items[k].here);
    match here {
        Some(p) if !typed => {
            let n = shown.len();
            let me = items[shown[p]].pane;
            let next = (1..n).map(|d| (p + d) % n).find(|&q| items[shown[q]].cur && items[shown[q]].pane != me);
            Some(next.unwrap_or((p + 1) % n))
        }
        _ => Some(shown.iter().position(|&k| !items[k].here).unwrap_or(0)),
    }
}

/// The row's right-hand note: "tab 2 of 3" (two tabs or more) and "you are here".
pub(crate) fn sw_badge(it: &SwItem, ko: bool) -> String {
    let mut v = Vec::new();
    if it.tabs > 1 {
        let t = if ko { "탭 {n}/{all}" } else { "tab {n} of {all}" };
        v.push(t.replace("{n}", &(it.tab + 1).to_string()).replace("{all}", &it.tabs.to_string()));
    }
    if it.here {
        v.push(if ko { "지금 여기" } else { "you are here" }.to_string());
    }
    v.join(" · ")
}

/// The row as one line: the list box's text (screen readers) and what ↑/↓ read out. "3 · Downloads · C:\…\Downloads · tab 2 of 3"
pub(crate) fn sw_line(it: &SwItem, ko: bool) -> String {
    let mut v = vec![(it.pane + 1).to_string(), it.name.clone()];
    v.extend([it.show.clone(), sw_badge(it, ko)].into_iter().filter(|t| !t.is_empty()));
    v.join(" · ")
}

pub(crate) struct SwGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) field: RECT,
    pub(crate) glass: RECT,
    pub(crate) edit: RECT,
    pub(crate) list: RECT,
}

pub(crate) fn sw_row_h(fh: i32, dpi: i32) -> i32 {
    fh + 14 * dpi / 96
}

/// The popup `w` wide with `rows` rows: the find field on top, the list under it exactly `rows` rows high.
pub(crate) fn sw_geo(fh: i32, dpi: i32, w: i32, rows: usize) -> SwGeo {
    let s = |v: i32| v * dpi / 96;
    let field = RECT { left: s(8), top: s(8), right: w - s(8), bottom: s(8) + fh + s(14) };
    let glass = RECT { right: field.left + s(30), ..field };
    let top = (field.top + field.bottom - fh) / 2;
    let edit = RECT { left: glass.right, top, right: field.right - s(8), bottom: top + fh };
    let lt = field.bottom + s(6);
    let list = RECT { left: 1, top: lt, right: w - 1, bottom: lt + rows as i32 * sw_row_h(fh, dpi) };
    SwGeo { w, h: list.bottom + s(6), field, glass, edit, list }
}

#[derive(Default)]
struct Sw {
    h: HWND,
    edit: HWND,
    list: HWND,
    prev: HWND,
    w: i32,
    rows: usize,
    items: Vec<SwItem>,
    shown: Vec<usize>,
}

thread_local! {
    static SW: RefCell<Sw> = RefCell::new(Sw::default());
}

fn sw<R>(f: impl FnOnce(&mut Sw) -> R) -> R {
    SW.with(|s| f(&mut s.borrow_mut()))
}

fn ko_now() -> bool {
    KO.load(Ordering::Relaxed)
}

pub(crate) unsafe fn open_switcher() {
    let open = sw(|s| s.edit);
    if !open.0.is_null() {
        _ = SetFocus(Some(open));
        return;
    }
    let (main, font, fh, dpi, active) = app(|a| (a.main, a.font, a.fh, a.dpi, a.s.active));
    let panes: Vec<(Vec<String>, usize)> = app(|a| {
        (0..a.s.panes())
            .map(|i| {
                let t = a.s.tab_list(i);
                let cur = a.s.tab[i].min(t.len() - 1);
                (t, cur)
            })
            .collect()
    });
    let items = sw_items(&panes, active, tabs_on(), ko_now());
    let rows = items.len().clamp(1, SW_ROWS);
    let s = |v: i32| v * dpi / 96;
    let mut cr = RECT::default();
    _ = GetClientRect(main, &mut cr);
    let mut o = POINT::default();
    _ = ClientToScreen(main, &mut o);
    let w = s(680).min(cr.right - s(32)).max(s(280));
    let g = sw_geo(fh, dpi, w, rows);
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let (x, y) = (o.x + (cr.right - g.w) / 2, o.y + s(44));
    let at = fit(RECT { left: x, top: y, right: x + g.w, bottom: y + g.h }, mi.rcWork);
    let prev = GetFocus();
    let style = WS_POPUP | WS_CLIPCHILDREN;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsSwitch"), w!(""), style, at.left, at.top, at.right - at.left, at.bottom - at.top, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    // the rows are in place before the list box asks to draw one
    sw(|x| (x.h, x.prev, x.w, x.rows, x.items, x.shown) = (h, prev, g.w, rows, items, Vec::new()));
    let edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), SW_EDIT, font);
    _ = MoveWindow(edit, g.edit.left, g.edit.top, g.edit.right - g.edit.left, g.edit.bottom - g.edit.top, false);
    let cue = HSTRING::from(tr("Type part of a folder's name or path", "폴더 이름이나 경로 일부를 입력"));
    SendMessageW(edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
    a11y_name(edit, Some(tr("Go to an open pane or tab", "열어 둔 패널·탭으로 가기")));
    let lb = (LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32 | WS_VSCROLL.0;
    let list = child(h, w!("LISTBOX"), w!(""), lb, WINDOW_EX_STYLE(0), SW_LIST, font);
    _ = SetWindowSubclass(list, Some(sw_list_proc), 1, 0);
    _ = MoveWindow(list, g.list.left, g.list.top, g.list.right - g.list.left, g.list.bottom - g.list.top, false);
    if DARK_ON.load(Ordering::Relaxed) {
        _ = SetWindowTheme(list, w!("DarkMode_Explorer"), PCWSTR::null()); // dark scrollbar
    }
    a11y_name(list, Some(tr("Open panes and tabs", "열린 패널·탭")));
    sw(|x| (x.edit, x.list) = (edit, list));
    sw_fill();
    _ = ShowWindow(h, SW_SHOW);
    _ = SetFocus(Some(edit));
}

// The rows matching the find box, and the first pick (EN_CHANGE, and once on opening).
unsafe fn sw_fill() {
    let (h, edit, list) = sw(|s| (s.h, s.edit, s.list));
    if h.0.is_null() {
        return;
    }
    let q = text_of(edit);
    let ko = ko_now();
    let (lines, first) = sw(|s| {
        s.shown = sw_filter(&s.items, &q);
        let first = sw_first(&s.items, &s.shown, !q.trim().is_empty());
        (s.shown.iter().map(|&k| sw_line(&s.items[k], ko)).collect::<Vec<_>>(), first)
    });
    SendMessageW(list, WM_SETREDRAW, Some(WPARAM(0)), None);
    SendMessageW(list, LB_RESETCONTENT, None, None);
    for t in &lines {
        let t = HSTRING::from(t.as_str());
        SendMessageW(list, LB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
    }
    if let Some(k) = first {
        SendMessageW(list, LB_SETCURSEL, Some(WPARAM(k)), None);
    }
    SendMessageW(list, WM_SETREDRAW, Some(WPARAM(1)), None);
    _ = ShowWindow(list, if lines.is_empty() { SW_HIDE } else { SW_SHOWNA });
    _ = InvalidateRect(Some(list), None, true);
    _ = InvalidateRect(Some(h), None, false);
}

// ↑/↓, PgUp/PgDn: the pick moves (no wrap), and screen readers hear the row.
unsafe fn sw_move(d: i32) {
    let list = sw(|s| s.list);
    let n = SendMessageW(list, LB_GETCOUNT, None, None).0.max(0) as usize;
    let cur = SendMessageW(list, LB_GETCURSEL, None, None).0;
    let Some(k) = ev_step((cur >= 0).then_some(cur as usize), n, d) else { return };
    SendMessageW(list, LB_SETCURSEL, Some(WPARAM(k)), None);
    let ko = ko_now();
    if let Some(line) = sw(|s| s.shown.get(k).and_then(|&j| s.items.get(j)).map(|it| sw_line(it, ko))) {
        say_only(&line);
    }
}

/// Keys in the popup: the find box's list keys, Enter, Esc and Ctrl+P (closes; held, it doesn't flicker). The rest is
/// typing; no pane shortcut acts behind it. None: not the popup's key, key() goes on.
pub(crate) unsafe fn sw_key(m: &MSG) -> Option<bool> {
    let (h, edit, list, rows) = sw(|s| (s.h, s.edit, s.list, s.rows));
    if h.0.is_null() || !(m.hwnd == edit || m.hwnd == list || m.hwnd == h) {
        return None;
    }
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    let repeat = m.lParam.0 & (1 << 30) != 0;
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        VK_ESCAPE => sw_close(true),
        VK_P if ctrl && !shift && !alt => {
            if !repeat {
                sw_close(true);
            }
        }
        VK_RETURN if !alt => {
            app(|a| a.held = Some(VK_RETURN.0)); // its repeats must not open an item in the list it goes to
            let k = SendMessageW(list, LB_GETCURSEL, None, None).0;
            match usize::try_from(k) {
                Ok(k) => sw_go(k),
                Err(_) => _ = MessageBeep(MB_ICONWARNING),
            }
        }
        VK_UP if !alt => sw_move(-1),
        VK_DOWN if !alt => sw_move(1),
        VK_PRIOR if !alt => sw_move(-(rows as i32)),
        VK_NEXT if !alt => sw_move(rows as i32),
        VK_TAB => {}
        // the list box would jump to a row by its first letter and take it as a click: typing belongs to the box
        _ if m.hwnd != edit && !alt => _ = SetFocus(Some(edit)),
        _ => return Some(false),
    }
    Some(true)
}

// Esc, Ctrl+P: back to where the user was (back: true). Going to a row: the caller focuses that pane.
pub(crate) unsafe fn sw_close(back: bool) {
    let (h, prev) = sw(|s| (s.h, s.prev));
    if h.0.is_null() {
        return;
    }
    _ = DestroyWindow(h);
    if !back {
        return;
    }
    if !prev.0.is_null() && IsWindow(Some(prev)).as_bool() && IsWindowVisible(prev).as_bool() {
        _ = SetFocus(Some(prev));
    } else {
        focus_view(app(|a| a.s.active));
    }
}

// Enter / a click on row k: the popup closes first (closing gives the old pane its focus back), then that pane becomes
// the active one (as Ctrl+1 … 9: a zoomed pane is let go), its tab is picked and the pane takes the focus. Panes and
// tabs may have changed while the popup was open: a row no longer there only beeps.
unsafe fn sw_go(k: usize) {
    let it = sw(|s| s.shown.get(k).and_then(|&j| s.items.get(j)).cloned());
    sw_close(false);
    let Some(it) = it else { return };
    let (before, panes) = app(|a| (a.s.active, a.s.panes()));
    if it.pane >= panes {
        _ = MessageBeep(MB_ICONWARNING);
        focus_view(before);
        return;
    }
    let (n, cur) = app(|a| {
        let n = a.s.tab_list(it.pane).len();
        (n, a.s.tab[it.pane].min(n - 1))
    });
    set_active(it.pane);
    let tab = tabs_on() && it.tab < n && it.tab != cur;
    if tab {
        // read out once its folder has opened (on_nav), or at once when nothing is browsed
        tab_said(it.pane, false, || pick_tab(it.pane, it.tab));
    }
    focus_view(it.pane);
    if before != it.pane {
        flash_pane(it.pane); // another pane: ringed so the eye finds it (a tab of this one is where the eye is)
    }
    if !tab && before != it.pane {
        say_pane(it.pane);
    }
}

unsafe fn sw_draw(d: &DRAWITEMSTRUCT) {
    let (dc, rc) = (d.hDC, d.rcItem);
    let Some(it) = sw(|s| s.shown.get(d.itemID as usize).and_then(|&k| s.items.get(k)).cloned()) else {
        return fill(dc, rc, pal().card);
    };
    let (dpi, font, small, accent) = app(|a| (a.dpi, a.font, a.small_font, a.s.accent_now()));
    let s = |v: i32| v * dpi / 96;
    let sel = d.itemState.0 & ODS_SELECTED.0 != 0;
    let (bg, ink, sub) = match sel {
        true => (accent, on_color(accent), blend(on_color(accent), accent, 0.3)),
        false => (pal().card, pal().text, pal().sub),
    };
    fill(dc, rc, bg);
    // the pane's number as on its header, then the name, its path (shortened in the middle), the tab note at the right
    let num = RECT { left: rc.left + s(6), right: rc.left + s(34), ..rc };
    text(dc, num, &(it.pane + 1).to_string(), small, sub, DT_CENTER);
    let badge = sw_badge(&it, ko_now());
    // as wide as its words ("tab 12 of 14 · you are here"), but never more than half the row
    let measure = |f: HFONT, t: &str| {
        let old = SelectObject(dc, f.into());
        let mut sz = SIZE::default();
        _ = GetTextExtentPoint32W(dc, &t.encode_utf16().collect::<Vec<_>>(), &mut sz);
        SelectObject(dc, old);
        sz.cx
    };
    let bw = if badge.is_empty() { 0 } else { (measure(small, &badge) + s(2)).min((rc.right - rc.left) / 2) };
    let right = RECT { left: rc.right - s(10) - bw, right: rc.right - s(10), ..rc };
    text(dc, right, &badge, small, if it.here && !sel { accent } else { sub }, DT_RIGHT);
    let body = RECT { left: num.right + s(6), right: right.left - s(8), ..rc };
    let sz = SIZE { cx: measure(font, &it.name), cy: 0 };
    let name_r = RECT { right: (body.left + sz.cx).min(body.left + (body.right - body.left) * 3 / 5), ..body };
    text(dc, name_r, &it.name, font, ink, DT_LEFT);
    if !it.show.is_empty() && name_r.right + s(12) < body.right {
        text(dc, RECT { left: name_r.right + s(12), ..body }, &it.show, small, sub, DT_LEFT | DT_PATH_ELLIPSIS);
    }
}

unsafe fn sw_paint(h: HWND) {
    let (dpi, fh, font, icon_font, accent) = app(|a| (a.dpi, a.fh, a.font, a.icon_font, a.s.accent_now()));
    let (w, rows, none) = sw(|s| (s.w, s.rows, s.shown.is_empty()));
    let g = sw_geo(fh, dpi, w, rows);
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        fill(dc, g.field, pal().pill);
        frame(dc, g.field, 1, accent);
        text(dc, g.glass, ICON_SEARCH, icon_font, pal().sub, DT_CENTER);
        if none {
            text(dc, g.list, tr("No open pane or tab has that name", "그런 이름의 패널·탭이 없음"), font, pal().sub, DT_CENTER);
        }
    });
}

// The list box: a click on a row goes there when the button is let go (the row already picked too, which sends no
// LBN_SELCHANGE); posted, so the popup goes away only after the list box is done with the click. A click beside
// the rows gives the find box its keys back.
unsafe extern "system" fn sw_list_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_LBUTTONUP {
        let parent = GetParent(h).unwrap_or_default();
        let at = DefSubclassProc(h, LB_ITEMFROMPOINT, WPARAM(0), lp).0 as usize;
        let k = at & 0xffff;
        // below the last row it still names the nearest one: only a point inside that row counts
        let mut rc = RECT::default();
        let got = DefSubclassProc(h, LB_GETITEMRECT, WPARAM(k), LPARAM(&mut rc as *mut RECT as isize)).0 != LB_ERR as isize;
        let (x, y) = xy(lp);
        let on = at >> 16 & 0xffff == 0 && got && y >= rc.top && y < rc.bottom && x >= rc.left && x < rc.right;
        let (m, wp) = if on { (WM_SW_PICK, k) } else { (WM_SW_REFOCUS, 0) };
        _ = PostMessageW(Some(parent), m, WPARAM(wp), LPARAM(0));
    }
    r
}

pub(crate) extern "system" fn sw_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => sw_paint(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_CTLCOLORLISTBOX => {
                let dc = HDC(wp.0 as _);
                SetDCBrushColor(dc, cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_MEASUREITEM => {
                let mi = &mut *(lp.0 as *mut MEASUREITEMSTRUCT);
                let (fh, dpi) = app(|a| (a.fh, a.dpi));
                mi.itemHeight = sw_row_h(fh, dpi) as u32;
                return LRESULT(1);
            }
            WM_DRAWITEM => {
                sw_draw(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, (wp.0 >> 16 & 0xffff) as u32) {
                (SW_EDIT, EN_CHANGE) => sw_fill(),
                _ => {}
            },
            WM_SW_PICK => sw_go(wp.0),
            WM_SW_REFOCUS => {
                let edit = sw(|s| s.edit);
                if !edit.0.is_null() {
                    _ = SetFocus(Some(edit));
                }
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => {
                let (edit, list) = sw(|s| (s.edit, s.list));
                a11y_name(edit, None);
                a11y_name(list, None);
                sw(|s| *s = Sw::default());
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(list: &[&str], cur: usize) -> (Vec<String>, usize) {
        (list.iter().map(|s| s.to_string()).collect(), cur)
    }

    #[test]
    fn switcher_rows() {
        let panes = [tabs(&[r"C:\Users\Me\Documents", r"C:\Users\Me\Downloads", r"D:\Work"], 1), tabs(&[r"C:\Users\Public"], 0)];
        // tabs on: one row per tab, in pane then tab order; the active pane's current tab is marked
        let on = sw_items(&panes, 0, true, false);
        let at: Vec<(usize, usize, bool)> = on.iter().map(|r| (r.pane, r.tab, r.here)).collect();
        assert_eq!(at, [(0, 0, false), (0, 1, true), (0, 2, false), (1, 0, false)]);
        assert_eq!(sw_line(&on[1], false), r"1 · Downloads · C:\Users\Me\Downloads · tab 2 of 3 · you are here");
        assert_eq!(sw_line(&on[1], true), r"1 · Downloads · C:\Users\Me\Downloads · 탭 2/3 · 지금 여기");
        assert_eq!(sw_line(&on[3], false), r"2 · Public · C:\Users\Public");
        // tabs off: the hidden tabs stay out, a pane is its folder on display
        let off = sw_items(&panes, 1, false, false);
        let at: Vec<(usize, usize, usize, bool)> = off.iter().map(|r| (r.pane, r.tab, r.tabs, r.here)).collect();
        assert_eq!(at, [(0, 1, 1, false), (1, 0, 1, true)]);
        assert_eq!(sw_line(&off[0], false), r"1 · Downloads · C:\Users\Me\Downloads");
        // 16 panes: every one is there, numbered as on screen
        let many: Vec<_> = (0..16).map(|i| tabs(&[&format!(r"C:\P{i}")], 0)).collect();
        let rows = sw_items(&many, 15, true, false);
        assert_eq!(rows.len(), 16);
        assert!(sw_line(&rows[15], false).starts_with("16 · P15"));
    }

    #[test]
    fn switcher_find_and_first_pick() {
        let panes = [tabs(&[r"C:\Users\Me\Downloads", r"C:\Users\Me\Music"], 0), tabs(&[r"E:\Old\downloads"], 0), tabs(&[r"D:\Work"], 0)];
        let items = sw_items(&panes, 0, true, false);
        let all = sw_filter(&items, "  ");
        assert_eq!(all, [0, 1, 2, 3]);
        // nothing typed: the next pane's current tab, not the same pane's next tab; at the end it wraps
        assert_eq!(sw_first(&items, &all, false), Some(2));
        let last = sw_items(&panes, 2, true, false);
        assert_eq!(sw_first(&last, &all, false), Some(0));
        let mid = [tabs(&[r"C:\A"], 0), tabs(&[r"C:\B1", r"C:\B2", r"C:\B3"], 2)];
        let rows = sw_items(&mid, 0, true, false);
        assert_eq!(sw_first(&rows, &sw_filter(&rows, ""), false), Some(3));
        // one pane only: the row after where you are
        let one = sw_items(&[tabs(&[r"C:\A", r"C:\B"], 0)], 0, true, false);
        assert_eq!(sw_first(&one, &sw_filter(&one, ""), false), Some(1));
        // every word, any case, in the name or the path; the first match that isn't where you are
        let down = sw_filter(&items, "DOWN");
        assert_eq!(down, [0, 2]);
        assert_eq!(sw_first(&items, &down, true), Some(1));
        assert_eq!(sw_filter(&items, "users music"), [1]);
        assert_eq!(sw_filter(&items, r"e:\old"), [2]);
        assert!(sw_filter(&items, "nowhere").is_empty());
        assert_eq!(sw_first(&items, &[], true), None);
        let only_here = sw_filter(&items, r"me\downloads");
        assert_eq!(sw_first(&items, &only_here, true), Some(0));
    }

    #[test]
    fn switcher_shell_places() {
        let bin = "::{645FF040-5081-101B-9F08-00AA002F954E}";
        let panes = [tabs(&[bin, THIS_PC], 0), tabs(&["search-ms:query=report"], 0)];
        let items = sw_items(&panes, 1, true, false);
        // by its name in the UI language, no GUID shown; found by either language's name
        assert_eq!(sw_line(&items[0], false), "1 · Recycle Bin · tab 1 of 2");
        assert_eq!(sw_line(&sw_items(&panes, 1, true, true)[0], true), "1 · 휴지통 · 탭 1/2");
        assert_eq!(sw_filter(&items, "휴지통"), [0]);
        assert_eq!(sw_filter(&items, "recycle"), [0]);
        assert_eq!(sw_filter(&items, "내 pc"), [1]);
        assert_eq!(sw_filter(&items, "this pc"), [1]);
        assert_eq!((items[2].name.as_str(), items[2].show.as_str()), ("Search: report", ""));
        assert_eq!(sw_filter(&items, "report"), [2]);
    }

    #[test]
    fn switcher_layout() {
        for (fh, dpi, rows) in [(16, 96, 1), (16, 96, 12), (24, 144, 5)] {
            let g = sw_geo(fh, dpi, 680 * dpi / 96, rows);
            assert_eq!(g.list.bottom - g.list.top, rows as i32 * sw_row_h(fh, dpi));
            assert!(g.field.bottom <= g.list.top && g.list.bottom < g.h);
            assert!(g.edit.top >= g.field.top && g.edit.bottom <= g.field.bottom && g.edit.left >= g.glass.right);
        }
    }
}
