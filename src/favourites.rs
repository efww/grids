use crate::*;

// ---------- favourites: ☆+ in each pane header, the ★ list in the toolbar ----------

pub(crate) unsafe fn repaint_fav_buttons() {
    for h in app(|a| a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>()) {
        if let Ok(b) = GetDlgItem(Some(h), ID_FAV) {
            _ = InvalidateRect(Some(b), None, false);
        }
    }
}

pub(crate) unsafe fn toggle_fav(i: usize) {
    app(|a| {
        let path = a.s.paths[i].clone();
        if a.s.is_fav(&path) {
            a.s.favs.retain(|f| !f.eq_ignore_ascii_case(&path));
        } else if !path.is_empty() {
            a.s.favs.push(path);
        }
    });
    repaint_fav_buttons();
}

pub(crate) struct FavGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) rows: Vec<(RECT, RECT)>, // (row, remove ✕) of the rows on screen
    pub(crate) first: usize,            // favourite in rows[0]
    pub(crate) n: usize,                // all favourites
    pub(crate) bar: Option<RECT>,       // scroll thumb in the right margin, only when some rows are hidden
}

// At most max_h tall (at least one row); rows[j] is favourite first + j, first = top kept so no row is left empty.
pub(crate) fn fav_geo(n: usize, top: usize, max_h: i32, dpi: i32) -> FavGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, rh, w) = (s(6), s(40), s(380));
    let vis = if n == 0 { 0 } else { (((max_h - 2 * pad) / rh).max(1) as usize).min(n) };
    let first = top.min(n - vis);
    let rows = (0..vis as i32)
        .map(|k| {
            let row = RECT { left: pad, top: pad + k * rh, right: w - pad, bottom: pad + (k + 1) * rh };
            (row, RECT { left: row.right - s(32), ..row })
        })
        .collect();
    let h = 2 * pad + vis.max(1) as i32 * rh;
    let bar = (vis < n).then(|| {
        let (t, b) = (pad, h - pad);
        let th = ((b - t) * vis as i32 / n as i32).max(s(16));
        let y = t + (b - t - th) * first as i32 / (n - vis) as i32;
        RECT { left: w - s(5), top: y, right: w - s(2), bottom: y + th }
    });
    FavGeo { w, h, rows, first, n, bar }
}

pub(crate) fn fav_geo_now() -> FavGeo {
    app(|a| fav_geo(a.s.favs.len(), a.fav_top, a.fav_max_h, a.dpi))
}

// The top that brings favourite k into view, moving as little as possible.
pub(crate) fn fav_scroll_to(top: usize, k: usize, vis: usize, n: usize) -> usize {
    let t = if k < top { k } else if k >= top + vis { k + 1 - vis.min(k + 1) } else { top };
    t.min(n.saturating_sub(vis))
}

// (height limit, opens above ★): below unless the list doesn't fit there, below holds fewer than three rows and above is roomier.
pub(crate) fn fav_room(full_h: i32, below: i32, above: i32, three_h: i32) -> (i32, bool) {
    if full_h > below && below < three_h && above > below { (above, true) } else { (below, false) }
}

// A mouse press on a row: a drag once it passes the system threshold (never from ✕)
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct FavPress {
    pub(crate) from: usize,
    pub(crate) on_x: bool,
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) live: bool,
    pub(crate) slot: Option<usize>, // drop line while dragging (None outside the list); drawing only
}

#[derive(PartialEq, Debug)]
pub(crate) enum Release {
    None,
    Move(usize, usize), // (row, slot)
    Open(usize),
    Remove(usize),
}

// The gap a drop at y lands in: the rows whose middle is above y, counted in the whole list (0..=n).
pub(crate) fn fav_slot(g: &FavGeo, y: i32) -> usize {
    g.first + g.rows.iter().filter(|(r, _)| (r.top + r.bottom) / 2 < y).count()
}

// Moves v[from] into gap `slot` (0..=len); the new index, or None when either is out of range.
pub(crate) fn move_fav(v: &mut Vec<String>, from: usize, slot: usize) -> Option<usize> {
    if from >= v.len() || slot > v.len() {
        return None;
    }
    let to = slot - usize::from(slot > from);
    let f = v.remove(from);
    v.insert(to, f);
    Some(to)
}

// Outside the list nothing happens; a drag drops at the release y; a click needs press and release on the same spot.
pub(crate) fn fav_release(p: Option<FavPress>, inside: bool, hit: Option<(usize, bool)>, slot: usize) -> Release {
    match p {
        Some(p) if inside && p.live => Release::Move(p.from, slot),
        Some(p) if inside && hit == Some((p.from, p.on_x)) => if p.on_x { Release::Remove(p.from) } else { Release::Open(p.from) },
        _ => Release::None,
    }
}

// Plain Alt+↑/↓ moves the highlighted favourite one row
pub(crate) fn fav_step(vk: VIRTUAL_KEY, ctrl: bool, alt: bool, shift: bool) -> Option<i32> {
    match vk {
        VK_UP | VK_DOWN if alt && !ctrl && !shift => Some(if vk == VK_UP { -1 } else { 1 }),
        _ => None,
    }
}

pub(crate) unsafe fn open_favs() {
    let (main, dpi, open, closed, n) = app(|a| (a.main, a.dpi, a.fav_popup, a.fav_closed, a.s.favs.len()));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_FAV) {
        _ = GetWindowRect(b, &mut rc);
    }
    // fit the work area: below ★ as before, above it when below is cramped, scrolling when even that is short
    // (the ★ button's monitor, not the window's: a window spanning two screens may have ★ on the smaller share)
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let mon = if rc.right > rc.left { MonitorFromRect(&rc, MONITOR_DEFAULTTONEAREST) } else { MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST) };
    let work = if GetMonitorInfoW(mon, &mut mi).as_bool() && mi.rcWork.bottom > mi.rcWork.top {
        mi.rcWork
    } else {
        RECT { left: rc.left, top: i32::MIN / 4, right: i32::MAX / 4, bottom: i32::MAX / 4 } // no monitor info: no limit
    };
    let gap = 6 * dpi / 96;
    let (below, above) = (work.bottom - (rc.bottom + gap), rc.top - gap - work.top);
    let (max_h, up) = fav_room(fav_geo(n, 0, i32::MAX, dpi).h, below, above, fav_geo(3, 0, i32::MAX, dpi).h);
    let g = fav_geo(n, 0, max_h, dpi);
    let y = if up { rc.top - gap - g.h } else { rc.bottom + gap };
    let y = y.min(work.bottom - g.h).max(work.top);
    let x = rc.left.min(work.right - g.w).max(work.left);
    let prev = GetFocus();
    app(|a| {
        (a.fav_hot, a.fav_prev, a.fav_top, a.fav_wheel) = (None, prev, 0, 0);
        (a.fav_max_h, a.fav_above) = (max_h, up.then_some(y + g.h));
    });
    let style = WS_POPUP | WS_VISIBLE;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsFav"), w!(""), style, x, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.fav_popup = h);
}

pub(crate) unsafe fn paint_favs(h: HWND) {
    let (favs, hot, press, accent, dpi, font, small, icon_font) =
        app(|a| (a.s.favs.clone(), a.fav_hot, a.fav_press, a.s.accent_now(), a.dpi, a.font, a.small_font, a.icon_font));
    // dragging: the dragged row stays lit whatever the keys did
    let hot = press.filter(|p| p.live).map_or(hot, |p| Some((p.from, false)));
    let g = fav_geo_now();
    let s = |v: i32| v * dpi / 96;
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        if favs.is_empty() {
            text(dc, rc, tr("No favourites yet · add one with ☆+ in a pane header", "즐겨찾기가 없습니다 · 패널 머리줄의 ☆+로 추가"), font, pal().sub, DT_CENTER);
        }
        if let Some(bar) = g.bar {
            fill(dc, bar, pal().line);
        }
        for (j, (f, (row, remove))) in favs.iter().skip(g.first).zip(&g.rows).enumerate() {
            let k = g.first + j;
            if hot.is_some_and(|(i, _)| i == k) {
                fill(dc, *row, pal().pill);
                let x_on = hot == Some((k, true));
                text(dc, *remove, "\u{E711}", icon_font, if x_on { pal().text } else { pal().sub }, DT_CENTER);
            }
            let icon = RECT { left: row.left + s(6), right: row.left + s(28), ..*row };
            text(dc, icon, "\u{E8B7}", icon_font, pal().sub, DT_CENTER);
            let line = |top: i32, hgt: i32| RECT { left: icon.right + s(6), right: remove.left - s(4), top, bottom: top + hgt };
            text(dc, line(row.top + s(3), s(19)), &short_name(f), font, pal().text, DT_LEFT);
            text(dc, line(row.top + s(21), s(16)), f, small, pal().sub, DT_LEFT | DT_PATH_ELLIPSIS);
        }
        // where the drop lands; none in its own place or outside the list (a cancel)
        if let Some((from, Some(slot))) = press.filter(|p| p.live).map(|p| (p.from, p.slot)) {
            let vis = g.rows.len();
            if slot != from && slot != from + 1 && vis > 0 && slot >= g.first && slot <= g.first + vis {
                let j = slot - g.first; // the gap on screen
                let (row, _) = g.rows[j.min(vis - 1)];
                let y = if j < vis { row.top } else { row.bottom };
                fill(dc, RECT { top: y - s(1), bottom: y - s(1) + s(2), ..row }, accent);
            }
        }
    });
}

pub(crate) fn fav_hit(g: &FavGeo, x: i32, y: i32) -> Option<(usize, bool)> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    g.rows.iter().position(|(row, _)| inside(row)).map(|j| (g.first + j, inside(&g.rows[j].1)))
}

pub(crate) extern "system" fn fav_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let g = fav_geo_now();
        let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
        let step = if msg == WM_SYSKEYDOWN { fav_step(VIRTUAL_KEY(wp.0 as u16), down(VK_CONTROL), down(VK_MENU), down(VK_SHIFT)) } else { None };
        // under capture x / y can be negative or past the edge
        let inside = |x: i32, y: i32| {
            let mut rc = RECT::default();
            _ = GetClientRect(h, &mut rc);
            x >= rc.left && x < rc.right && y >= rc.top && y < rc.bottom
        };
        let cancel = || {
            app(|a| a.fav_press = None);
            if GetCapture() == h {
                _ = ReleaseCapture();
            }
            _ = InvalidateRect(Some(h), None, false);
        };
        let moved = |to: Option<usize>| {
            if let Some(to) = to {
                app(|a| (a.fav_hot, a.fav_top) = (Some((to, false)), fav_scroll_to(g.first, to, g.rows.len(), g.n)));
            }
            _ = InvalidateRect(Some(h), None, false);
        };
        match msg {
            WM_PAINT => paint_favs(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some((k, on_x)) = fav_hit(&g, x, y) {
                    app(|a| (a.fav_hot, a.fav_press) = (Some((k, on_x)), Some(FavPress { from: k, on_x, x0: x, y0: y, live: false, slot: None })));
                    SetCapture(h);
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                if let Some(mut p) = app(|a| a.fav_press) {
                    let far = (x - p.x0).abs() > GetSystemMetrics(SM_CXDRAG) || (y - p.y0).abs() > GetSystemMetrics(SM_CYDRAG);
                    p.live |= !p.on_x && far;
                    if p.live {
                        p.slot = inside(x, y).then(|| fav_slot(&g, y));
                    }
                    if app(|a| std::mem::replace(&mut a.fav_press, Some(p))) != Some(p) {
                        _ = InvalidateRect(Some(h), None, false);
                    }
                    return LRESULT(0);
                }
                let hot = fav_hit(&g, x, y);
                if app(|a| std::mem::replace(&mut a.fav_hot, hot)) != hot {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_CAPTURECHANGED => cancel(),
            WM_MOUSEWHEEL => {
                let press = app(|a| a.fav_press);
                if g.rows.len() >= g.n || press.is_some_and(|p| !p.live) {
                    return LRESULT(0); // nothing hidden, or a click still on its row
                }
                let notches = app(|a| {
                    a.fav_wheel += (wp.0 >> 16) as i16 as i32;
                    let d = a.fav_wheel / 120;
                    a.fav_wheel -= d * 120;
                    d
                });
                let top = (g.first as i32 - notches * 3).clamp(0, (g.n - g.rows.len()) as i32) as usize;
                app(|a| a.fav_top = top);
                let g = fav_geo_now();
                let mut pt = POINT::default();
                _ = GetCursorPos(&mut pt);
                _ = ScreenToClient(h, &mut pt);
                if let Some(mut p) = press {
                    // dragging: the dragged favourite stays, the drop gap follows the rows under the cursor
                    p.slot = inside(pt.x, pt.y).then(|| fav_slot(&g, pt.y));
                    app(|a| a.fav_press = Some(p));
                } else {
                    // a highlight scrolled out of view is no target for Enter / Del
                    let shown = |k: usize| k >= g.first && k < g.first + g.rows.len();
                    let hot = if inside(pt.x, pt.y) { fav_hit(&g, pt.x, pt.y) } else { app(|a| a.fav_hot).filter(|&(k, _)| shown(k)) };
                    app(|a| a.fav_hot = hot);
                }
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_SYSKEYDOWN if step.is_some() => {
                let (cur, pressing) = app(|a| (a.fav_hot.map(|(k, _)| k), a.fav_press.is_some()));
                if let (Some(k), false) = (cur, pressing) {
                    // the gap above the previous row / below the next one; nothing past either end
                    let slot = if step == Some(-1) { k.checked_sub(1) } else { Some(k + 2) };
                    if let Some(to) = slot.and_then(|s| app(|a| move_fav(&mut a.s.favs, k, s))) {
                        moved(Some(to));
                    }
                }
            }
            WM_LBUTTONUP | WM_KEYDOWN => {
                // ✕ / Del: drop it and shrink the list in place
                // Del keeps a highlight on the row that took its place, scrolled into view; ✕ leaves it to the mouse
                let remove = |k: usize, keys: bool| {
                    let n = app(|a| {
                        a.s.favs.remove(k);
                        a.s.favs.len()
                    });
                    let g0 = fav_geo_now(); // first already pulled back so no row is left empty
                    let hot = (keys && n > 0).then(|| k.min(n - 1));
                    let top = hot.map_or(g0.first, |k| fav_scroll_to(g0.first, k, g0.rows.len(), n));
                    app(|a| (a.fav_hot, a.fav_top) = (hot.map(|k| (k, false)), top));
                    let g = fav_geo_now();
                    let mut wr = RECT::default();
                    _ = GetWindowRect(h, &mut wr);
                    // opened above ★: shrink toward the top so the bottom stays by the button
                    let y = app(|a| a.fav_above).map_or(wr.top, |b| b - g.h);
                    _ = SetWindowPos(h, None, wr.left, y, g.w, g.h, SWP_NOZORDER | SWP_NOACTIVATE);
                    _ = InvalidateRect(Some(h), None, false);
                    repaint_fav_buttons();
                };
                let open = |k: usize| {
                    let (i, path) = app(|a| (a.s.active, a.s.favs[k].clone()));
                    _ = DestroyWindow(h);
                    if !go_near(i, &path) {
                        _ = MessageBeep(MB_ICONWARNING); // gone or unreachable
                    }
                    focus_view(i);
                };
                if msg == WM_LBUTTONUP {
                    let p = app(|a| a.fav_press.take());
                    if GetCapture() == h {
                        _ = ReleaseCapture();
                    }
                    let (x, y) = xy(lp);
                    match fav_release(p, inside(x, y), fav_hit(&g, x, y), fav_slot(&g, y)) {
                        Release::Move(from, slot) => moved(app(|a| move_fav(&mut a.s.favs, from, slot))),
                        Release::Open(k) => open(k),
                        Release::Remove(k) => remove(k, false),
                        Release::None => _ = InvalidateRect(Some(h), None, false),
                    }
                    return LRESULT(0);
                }
                let (n, cur, pressing) = app(|a| (a.s.favs.len(), a.fav_hot.map(|(k, _)| k), a.fav_press.is_some()));
                match VIRTUAL_KEY(wp.0 as u16) {
                    // Esc mid-press drops the drag, the list stays
                    VK_ESCAPE if pressing => cancel(),
                    VK_ESCAPE => {
                        let (main, prev, active) = app(|a| (a.main, a.fav_prev, a.s.active));
                        _ = DestroyWindow(h);
                        let fav_btn = GetDlgItem(Some(main), ID_TB_FAV).unwrap_or_default();
                        if !prev.0.is_null() && IsWindow(Some(prev)).as_bool() && IsWindowVisible(prev).as_bool() && prev != fav_btn {
                            _ = SetFocus(Some(prev));
                        } else {
                            focus_view(active);
                        }
                    }
                    _ if n == 0 || pressing => {}
                    k @ (VK_DOWN | VK_UP) => {
                        let next = if k == VK_DOWN { cur.map_or(0, |k| (k + 1) % n) } else { cur.map_or(n - 1, |k| (k + n - 1) % n) };
                        app(|a| (a.fav_hot, a.fav_top) = (Some((next, false)), fav_scroll_to(g.first, next, g.rows.len(), n)));
                        _ = InvalidateRect(Some(h), None, false);
                    }
                    VK_RETURN => if let Some(k) = cur { open(k) },
                    VK_DELETE => if let Some(k) = cur { remove(k, true) },
                    _ => {}
                }
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => {
                app(|a| a.fav_press = None);
                _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            WM_DESTROY => app(|a| (a.fav_popup, a.fav_closed) = (HWND::default(), Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}
