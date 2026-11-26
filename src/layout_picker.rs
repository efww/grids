use crate::*;

// ---------- layout picker: click = even grid, press and drag across columns = rows per column, presets ----------

pub(crate) const PRESETS: [&[usize]; 6] = [&[1, 2], &[1, 3], &[2, 1], &[3, 1], &[1, 1, 2], &[2, 3]];

pub(crate) struct PickGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) pad: i32,
    pub(crate) cell: i32,
    pub(crate) gap: i32,
    pub(crate) label: RECT,
    pub(crate) presets: [RECT; 6],
}

pub(crate) fn pick_geo(dpi: i32) -> PickGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, cell, gap) = (s(12), s(24), s(6));
    let gw = 4 * cell + 3 * gap;
    let label = RECT { left: pad, top: pad + gw, right: pad + gw, bottom: pad + gw + s(26) };
    let (pw, ph) = ((gw - 2 * gap) / 3, s(24));
    let presets = std::array::from_fn(|i| {
        let (x, y) = (pad + (i as i32 % 3) * (pw + gap), label.bottom + (i as i32 / 3) * (ph + gap));
        RECT { left: x, top: y, right: x + pw, bottom: y + ph }
    });
    PickGeo { w: 2 * pad + gw, h: label.bottom + 2 * ph + gap + pad, pad, cell, gap, label, presets }
}

pub(crate) fn pick_cell(g: &PickGeo, r: usize, c: usize) -> RECT {
    let (x, y) = (g.pad + c as i32 * (g.cell + g.gap), g.pad + r as i32 * (g.cell + g.gap));
    RECT { left: x, top: y, right: x + g.cell, bottom: y + g.cell }
}

/// (row, column) under the point; `clamp` keeps a drag that leaves the cells on the nearest one.
pub(crate) fn pick_at(g: &PickGeo, x: i32, y: i32, clamp: bool) -> Option<(usize, usize)> {
    let span = 4 * g.cell + 3 * g.gap;
    let inside = (g.pad..g.pad + span).contains(&x) && (g.pad..g.pad + span).contains(&y);
    let at = |v: i32| ((v - g.pad).max(0) / (g.cell + g.gap)).min(3) as usize;
    (inside || clamp).then(|| (at(y), at(x)))
}

/// Dragging: columns follow the cursor; the column under it takes the cursor's row.
pub(crate) fn drag_heights(mut h: Vec<usize>, r: usize, c: usize) -> Vec<usize> {
    h.resize(c + 1, r + 1);
    h[c] = r + 1;
    h
}

// The picker's label under the cells: "2 × 3" (Korean "2행 × 3열") for an even grid, "Per column  1 | 2" otherwise.
pub(crate) fn grid_label(h: &[usize], ko: bool) -> String {
    let Some(&first) = h.first() else { return String::new() };
    match h.iter().all(|&n| n == first) {
        true if ko => format!("{first}행 × {}열", h.len()),
        true => format!("{first} × {}", h.len()),
        false => format!("{}  {}", if ko { "열마다" } else { "Per column" }, layout_label(h)),
    }
}

// What a screen reader hears for a layout: the label as drawn, then how many panes it makes.
pub(crate) fn grid_speech(h: &[usize], ko: bool) -> String {
    let n: usize = h.iter().sum();
    match (ko, n) {
        (true, n) => format!("{}, 패널 {n}개", grid_label(h, ko)),
        (false, 1) => format!("{}, 1 pane", grid_label(h, ko)),
        (false, n) => format!("{}, {n} panes", grid_label(h, ko)),
    }
}

// The popup's name on opening: the layout now and the keys.
pub(crate) fn grid_intro(h: &[usize], ko: bool) -> String {
    match ko {
        true => format!("레이아웃, 지금 {} · 방향키로 고르고 Enter로 적용", grid_speech(h, ko)),
        false => format!("Layout, now {} · arrow keys pick, Enter applies", grid_speech(h, ko)),
    }
}

pub(crate) unsafe fn open_grid() {
    let (main, dpi, open, closed, cur) = app(|a| (a.main, a.dpi, a.grid, a.grid_closed, a.s.heights.clone()));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        _ = GetWindowRect(b, &mut rc);
    }
    let g = pick_geo(dpi);
    let intro = grid_intro(&cur, KO.load(Ordering::Relaxed));
    app(|a| (a.grid_preview, a.grid_drag, a.grid_preset) = (cur, None, None));
    let y = rc.bottom + 6 * dpi / 96;
    // hidden until a screen reader has its name (no role: it has no items to count)
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsPicker"), w!(""), WS_POPUP, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    a11y_name(h, Some(&intro));
    _ = ShowWindow(h, SW_SHOW);
    app(|a| a.grid = h);
}

pub(crate) unsafe fn paint_grid(h: HWND) {
    let (preview, cur, hot, dpi, font, accent) =
        app(|a| (a.grid_preview.clone(), a.s.heights.clone(), a.grid_preset, a.dpi, a.font, a.s.accent_now()));
    let soft = blend(accent, pal().card, 0.8);
    let g = pick_geo(dpi);
    let on = |v: &[usize], r: usize, c: usize| c < v.len() && r < v[c];
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        for r in 0..4 {
            for c in 0..4 {
                let col = if on(&preview, r, c) {
                    accent
                } else if on(&cur, r, c) {
                    soft
                } else {
                    pal().pill
                };
                fill(dc, pick_cell(&g, r, c), col);
            }
        }
        text(dc, g.label, &grid_label(&preview, KO.load(Ordering::Relaxed)), font, pal().text, DT_CENTER);
        // preset thumbnails: the panes of each layout on a tile
        for (i, (p, t)) in PRESETS.iter().zip(g.presets).enumerate() {
            let (tile, pane) = if hot == Some(i) { (soft, accent) } else { (pal().pill, blend(pal().card, pal().text, 0.25)) };
            fill(dc, t, tile);
            let inner = RECT { left: t.left + 3, top: t.top + 3, right: t.right - 3, bottom: t.bottom - 3 };
            for (c, (x0, x1)) in cells(inner.left, inner.right - inner.left, &even(p.len()), 2).into_iter().enumerate() {
                for (y0, y1) in cells(inner.top, inner.bottom - inner.top, &even(p[c]), 2) {
                    fill(dc, RECT { left: x0, top: y0, right: x1, bottom: y1 }, pane);
                }
            }
        }
    });
}

pub(crate) extern "system" fn grid_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let g = pick_geo(app(|a| a.dpi));
        // the name follows the preview (Enter's layout); said out loud only when the keys changed it
        let show = |v: Vec<usize>, preset: Option<usize>, keys: bool| {
            let changed = app(|a| {
                let changed = a.grid_preview != v || a.grid_preset != preset;
                (a.grid_preview, a.grid_preset) = (v, preset);
                changed
            });
            if changed {
                _ = InvalidateRect(Some(h), None, false);
                let speech = grid_speech(&app(|a| a.grid_preview.clone()), KO.load(Ordering::Relaxed));
                a11y_say(h, &speech, keys);
            }
        };
        let apply = |v: Vec<usize>| {
            _ = DestroyWindow(h);
            set_layout(v);
        };
        match msg {
            WM_PAINT => paint_grid(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some((r, c)) = pick_at(&g, x, y, false) {
                    SetCapture(h);
                    let v = vec![r + 1; c + 1];
                    app(|a| a.grid_drag = Some(v.clone()));
                    show(v, None, false);
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| a.grid_drag.clone()) {
                    if let Some((r, c)) = pick_at(&g, x, y, true) {
                        let v = drag_heights(d, r, c);
                        app(|a| a.grid_drag = Some(v.clone()));
                        show(v, None, false);
                    }
                } else if let Some((r, c)) = pick_at(&g, x, y, false) {
                    show(vec![r + 1; c + 1], None, false);
                } else if let Some(i) = g.presets.iter().position(|t| x >= t.left && x < t.right && y >= t.top && y < t.bottom) {
                    show(PRESETS[i].to_vec(), Some(i), false);
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                if let Some(v) = app(|a| a.grid_drag.take()) {
                    _ = ReleaseCapture();
                    apply(v);
                } else if let Some(i) = g.presets.iter().position(|t| x >= t.left && x < t.right && y >= t.top && y < t.bottom) {
                    apply(PRESETS[i].to_vec());
                }
            }
            WM_KEYDOWN => {
                // keyboard picks even grids: arrows move the corner, Enter applies
                let v = app(|a| a.grid_preview.clone());
                let (r, c) = (v[v.len() - 1] - 1, v.len() - 1);
                let (r, c) = match VIRTUAL_KEY(wp.0 as u16) {
                    VK_LEFT => (r, c.saturating_sub(1)),
                    VK_RIGHT => (r, (c + 1).min(3)),
                    VK_UP => (r.saturating_sub(1), c),
                    VK_DOWN => ((r + 1).min(3), c),
                    VK_RETURN => {
                        apply(v);
                        return LRESULT(0);
                    }
                    VK_ESCAPE => {
                        _ = DestroyWindow(h);
                        return LRESULT(0);
                    }
                    _ => (r, c),
                };
                show(vec![r + 1; c + 1], None, true);
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => {
                a11y_clear(h);
                app(|a| (a.grid, a.grid_drag, a.grid_closed) = (HWND::default(), None, Some(Instant::now())));
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}
