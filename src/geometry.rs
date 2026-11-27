use crate::*;

// ---------- grid geometry ----------

/// Splits `len` px starting at `start` into cells by `ratios`, with `gap` px splitters between them.
pub(crate) fn cells(start: i32, len: i32, ratios: &[f64], gap: i32) -> Vec<(i32, i32)> {
    let avail = (len - gap * (ratios.len() as i32 - 1)).max(0) as f64;
    let mut acc = 0.0;
    let mut out = Vec::with_capacity(ratios.len());
    for (i, r) in ratios.iter().enumerate() {
        let a = start + (acc * avail).round() as i32 + i as i32 * gap;
        acc += r;
        out.push((a, start + (acc * avail).round() as i32 + i as i32 * gap));
    }
    out
}

/// Whole wheel notches (120 each) from a `WM_MOUSEWHEEL` delta, keeping the rest in `acc` for the next message:
/// precision touchpads and smooth wheels send 10-60 at a time. Turning back starts the rest over, so it answers at once.
pub(crate) fn wheel_notches(acc: &mut i32, delta: i32) -> i32 {
    if delta == 0 {
        return 0;
    }
    if *acc != 0 && (*acc < 0) != (delta < 0) {
        *acc = 0;
    }
    *acc = acc.saturating_add(delta);
    let n = *acc / 120;
    *acc -= n * 120;
    n
}

/// Moves the boundary after cell `k` to `pos`, keeping both neighbours at least `min` px.
pub(crate) fn drag_ratio(r: &mut [f64], k: usize, pos: i32, start: i32, len: i32, gap: i32, min: i32) {
    let avail = (len - gap * (r.len() as i32 - 1)).max(1) as f64;
    let before: f64 = r[..k].iter().sum();
    let pair = r[k] + r[k + 1];
    let m = (min as f64 / avail).min(pair / 2.0);
    let t = ((pos - start - k as i32 * gap) as f64 / avail - before).clamp(m, pair - m);
    (r[k], r[k + 1]) = (t, pair - t);
}

/// Shrinks `r` to fit `work` and slides it inside; a rect already inside comes back unchanged.
pub(crate) fn fit(r: RECT, work: RECT) -> RECT {
    let w = (r.right - r.left).min(work.right - work.left);
    let h = (r.bottom - r.top).min(work.bottom - work.top);
    let left = r.left.clamp(work.left, work.right - w);
    let top = r.top.clamp(work.top, work.bottom - h);
    RECT { left, top, right: left + w, bottom: top + h }
}

/// `fit` for a saved rcNormalPosition, which is in workspace coordinates (origin at rcWork's corner). `slack` px past
/// the left, right and bottom edges still count as inside: a window snapped to a half or quarter has its invisible
/// resize border there (none on top, where its title bar would be cut off).
pub(crate) fn fit_saved(saved: RECT, mon: RECT, work: RECT, slack: i32) -> RECT {
    let (dx, dy) = (work.left - mon.left, work.top - mon.top);
    let wide = RECT { left: work.left - slack, top: work.top, right: work.right + slack, bottom: work.bottom + slack };
    let r = fit(RECT { left: saved.left + dx, top: saved.top + dy, right: saved.right + dx, bottom: saved.bottom + dy }, wide);
    RECT { left: r.left - dx, top: r.top - dy, right: r.right - dx, bottom: r.bottom - dy }
}

/// The window rect `actual` (GetWindowRect, screen coordinates) in rcNormalPosition's workspace coordinates, `mon` and
/// `work` being its monitor's rcMonitor and rcWork as for `fit_saved`. For a floating window this equals its
/// rcNormalPosition; for one snapped to a half or quarter it is the rect it shows, where rcNormalPosition keeps the
/// size from before the snap. Only for a window neither minimized nor maximized.
pub(crate) fn placed_rect(actual: RECT, mon: RECT, work: RECT) -> RECT {
    let (dx, dy) = (work.left - mon.left, work.top - mon.top);
    RECT { left: actual.left - dx, top: actual.top - dy, right: actual.right - dx, bottom: actual.bottom - dy }
}

// ---------- window place per monitor setup (grids.ini win_at=) ----------

/// Monitor setups remembered, newest first; one pushed out falls back to win=.
pub(crate) const WIN_PLACES: usize = 4;

/// A monitor setup as text: every monitor's rcMonitor (physical px, l,t,r,b), sorted, joined by ';'.
/// The same setup gives the same text whatever order Windows lists the monitors in.
pub(crate) fn mon_sig(rects: &[[i32; 4]]) -> String {
    let mut r = rects.to_vec();
    r.sort();
    r.iter().map(|m| join(m)).collect::<Vec<_>>().join(";")
}

/// Notes `pos` (win= form) as setup `sig`'s place: its entry moves to the front, the oldest past WIN_PLACES goes.
/// `hold`: where the window was when this setup appeared. While the window is still there the setup's existing
/// entry stays as it is (Windows moved it, not the user); a setup seen for the first time is noted at once.
/// Returns the hold still in force (None once the window has moved away from it).
pub(crate) fn remember_place(places: &mut Vec<(String, [i32; 5])>, sig: &str, pos: [i32; 5], hold: Option<[i32; 5]>) -> Option<[i32; 5]> {
    let at = places.iter().position(|(g, _)| g == sig);
    if at.is_some() && hold == Some(pos) {
        return hold;
    }
    if let Some(i) = at {
        places.remove(i);
    }
    places.insert(0, (sig.to_string(), pos));
    places.truncate(WIN_PLACES);
    None
}

/// Where to open: the place noted for the setup there is now, else the last place (win=) as before.
pub(crate) fn pick_place(places: &[(String, [i32; 5])], sig: Option<&str>, win: Option<[i32; 5]>) -> Option<[i32; 5]> {
    sig.and_then(|g| places.iter().find(|(h, _)| h == g)).map(|p| p.1).or(win)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Hit {
    Side,
    Col(usize),        // boundary after column k
    Row(usize, usize), // (column, boundary after row k in it)
}

// Main window: toolbar on top, status line at the bottom, plates (sidebar + grid) in between.
// The gaps between plates are the splitters.
pub(crate) struct Geo {
    pub(crate) w: i32,
    pub(crate) h: i32,   // client height
    pub(crate) top: i32, // toolbar bottom = plates top
    pub(crate) bot: i32, // plates bottom = status line top
    pub(crate) pad: i32,
    pub(crate) gap: i32,
    pub(crate) side: i32, // sidebar plate width
    pub(crate) tree: bool, // sidebar shown; when hidden the grid starts at pad
    pub(crate) gx: i32,   // grid left
    pub(crate) xs: Vec<(i32, i32)>,
    pub(crate) ys: Vec<Vec<(i32, i32)>>, // per column
    pub(crate) zoom: bool, // one pane fills the grid: no splitters
}

/// Main window bars at this DPI, taller by `grow` (TEXT_GROW) for a bigger Text size: (toolbar height, toolbar button
/// height, status line height). The buttons stay s(5) from the top.
pub(crate) fn bars_h(dpi: i32, grow: i32) -> (i32, i32, i32) {
    let s = |v: i32| v * dpi / 96;
    (s(34) + grow, s(24) + grow, s(24) + grow)
}

pub(crate) fn text_grow_now() -> i32 {
    TEXT_GROW.load(Ordering::Relaxed)
}

pub(crate) fn geo(a: &App) -> Geo {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(a.main, &mut rc) };
    let s = |v: i32| v * a.dpi / 96;
    let (top, _, status_h) = bars_h(a.dpi, text_grow_now());
    let (pad, gap) = (s(8), s(8));
    let bot = (rc.bottom - status_h).max(top);
    let side = a.s.side.clamp(s(120), (rc.right - 2 * pad - gap - s(240)).max(s(120)));
    let gx = if a.s.tree { pad + side + gap } else { pad };
    Geo {
        xs: cells(gx, rc.right - pad - gx, &a.s.colr, gap),
        ys: a.s.rowr.iter().map(|r| cells(top, bot - top, r, gap)).collect(),
        w: rc.right,
        h: rc.bottom,
        top,
        bot,
        pad,
        gap,
        side,
        tree: a.s.tree,
        gx,
        zoom: a.zoom.is_some(),
    }
}

pub(crate) fn side_rect(g: &Geo) -> RECT {
    RECT { left: g.pad, top: g.top, right: g.pad + g.side, bottom: g.bot }
}

pub(crate) fn status_rect(g: &Geo) -> RECT {
    RECT { left: 0, top: g.bot, right: g.w, bottom: g.h }
}

/// Pane slot -> (column, row): panes are numbered down each column, left to right.
pub(crate) fn slot(h: &[usize], mut i: usize) -> (usize, usize) {
    for (c, &n) in h.iter().enumerate() {
        if i < n {
            return (c, i);
        }
        i -= n;
    }
    (h.len() - 1, h[h.len() - 1] - 1)
}

/// Ctrl+Alt+arrow, as tmux's resize-pane: the splitter of pane `active` that moves that way, and the sign (+ = right /
/// down). Its right (lower) boundary; the last column (row) has none, so its left (upper) one. None: no splitter on that axis.
pub(crate) fn nudge_target(h: &[usize], active: usize, dx: i32, dy: i32) -> Option<(Hit, i32)> {
    if h.is_empty() {
        return None;
    }
    let (c, r) = slot(h, active);
    let edge = |i: usize, n: usize| (n >= 2).then(|| if i + 1 < n { i } else { i - 1 });
    if dx != 0 {
        edge(c, h.len()).map(|k| (Hit::Col(k), dx.signum()))
    } else if dy != 0 {
        edge(r, h[c]).map(|k| (Hit::Row(c, k), dy.signum()))
    } else {
        None
    }
}

/// (column, row) of each slot off screen: the cell its pane last had (runtime only, never saved).
pub(crate) type Homes = [Option<(usize, usize)>; MAXP];

/// Layout `old` -> `new`, like Q-Dir's layout switch or a tmux/VS Code split: new slot j shows the pane of old slot
/// perm[j]. A pane whose cell is in both stays there; one whose cell goes keeps it as its home and comes back when
/// that cell does; new cells take the lowest slots without a home. Hidden homes stay apart and off the grid.
pub(crate) fn remap(old: &[usize], new: &[usize], home: &Homes) -> ([usize; MAXP], Homes) {
    let has = |(c, r): (usize, usize)| new.get(c).is_some_and(|&n| r < n);
    let (on, n) = (old.iter().sum::<usize>(), new.iter().sum::<usize>());
    let mut home = *home;
    let mut perm = [usize::MAX; MAXP];
    for j in 0..MAXP {
        let Some(c) = (if j < on { Some(slot(old, j)) } else { home[j] }) else { continue };
        if has(c) {
            perm[new[..c.0].iter().sum::<usize>() + c.1] = j;
            home[j] = None;
        } else {
            home[j] = Some(c);
        }
    }
    let free: Vec<usize> = (0..MAXP).filter(|j| !perm.contains(j) && home[*j].is_none()).collect();
    let empty: Vec<usize> = (0..n).filter(|&k| perm[k] == usize::MAX).collect();
    for (k, j) in empty.into_iter().zip(free) {
        perm[k] = j;
    }
    let rest: Vec<usize> = (0..MAXP).filter(|j| !perm[..n].contains(j)).collect();
    let mut out = [None; MAXP];
    for (k, j) in (n..MAXP).zip(rest) {
        (perm[k], out[k]) = (j, home[j]);
    }
    (perm, out)
}

pub(crate) fn cell_rect(g: &Geo, h: &[usize], i: usize) -> RECT {
    let (c, r) = slot(h, i);
    let ((x0, x1), (y0, y1)) = (g.xs[c], g.ys[c][r]);
    RECT { left: x0, top: y0, right: x1, bottom: y1 }
}

// Inside a pane plate: icon buttons, address pill, ⇄, filter pill on top; the Explorer view below.
pub(crate) struct PaneGeo {
    pub(crate) num: RECT,      // pane number (Ctrl+N); zero width when the pane is too narrow
    pub(crate) btn: [RECT; 4], // back, forward, up, open-in
    pub(crate) addr: RECT,
    pub(crate) filt: RECT,
    pub(crate) fav: RECT,
    pub(crate) tabs: RECT, // tab strip between the bar and the list; empty while tabs are off
    pub(crate) view: RECT,
}

pub(crate) fn pane_geo(w: i32, h: i32, dpi: i32) -> PaneGeo {
    pane_geo_at(w, h, dpi, text_grow_now(), TABS_ON.load(Ordering::Relaxed))
}

// pane_geo with the text growth and the tab strip given rather than read: the bar and its pills and buttons are
// `grow` px taller (widths stay; the vertical inset is the same), the tab strip too.
pub(crate) fn pane_geo_at(w: i32, h: i32, dpi: i32, grow: i32, tabs: bool) -> PaneGeo {
    let s = |v: i32| v * dpi / 96;
    let (bar, bh, b, pad) = (s(30) + grow, s(22) + grow, s(24), s(4));
    let strip = tab_strip_h_at(dpi, grow, tabs);
    let y = (bar - bh) / 2;
    let r = |x: i32, wd: i32| RECT { left: x, top: y, right: x + wd.max(0), bottom: y + bh };
    let fav = r(w - pad - b, b);
    let fw = s(150).min((w - 2 * pad - 5 * b) / 3).max(0);
    let filt = r(fav.left - s(2) - fw, fw);
    let open = r(filt.left - s(2) - b, b);
    // pane number at the far left, only while the address edit (pill minus its s(8)/s(6) insets) keeps s(32)
    let nw = s(18);
    let nw = if open.left - s(4) - (pad + 3 * b + s(4) + nw) >= s(8) + s(6) + s(32) { nw } else { 0 };
    let x = pad + nw;
    let addr_x = x + 3 * b + s(4);
    PaneGeo {
        num: r(pad, nw),
        btn: [r(x, b), r(x + b, b), r(x + 2 * b, b), open],
        addr: r(addr_x, open.left - s(4) - addr_x),
        filt,
        fav,
        tabs: RECT { left: s(2), top: bar, right: w - s(2), bottom: bar + strip },
        // 2px in, so the active ring stays visible around the list
        view: RECT { left: s(2), top: bar + strip, right: w - s(2), bottom: (h - s(2)).max(bar + strip) },
    }
}

// The tab strip's height: grown with the Text size as the bar is, none while tabs are off.
pub(crate) fn tab_strip_h_at(dpi: i32, grow: i32, tabs: bool) -> i32 {
    if tabs { 26 * dpi / 96 + grow } else { 0 }
}

// Tabs share the strip evenly up to a comfortable width, then a '+' after the last. (tabs, '+')
pub(crate) fn tab_rects(strip: RECT, n: usize, dpi: i32) -> (Vec<RECT>, RECT) {
    let s = |v: i32| v * dpi / 96;
    let (pad, plus) = (s(4), s(24));
    let room = (strip.right - strip.left - 2 * pad - plus).max(0);
    let w = (room / n.max(1) as i32).min(s(170));
    let tab = |k: i32| RECT { left: strip.left + pad + k * w, top: strip.top + s(3), right: strip.left + pad + (k + 1) * w, bottom: strip.bottom };
    let tabs: Vec<RECT> = (0..n as i32).map(tab).collect();
    let x = tabs.last().map_or(strip.left + pad, |r| r.right);
    (tabs, RECT { left: x, top: strip.top + s(3), right: x + plus, bottom: strip.bottom })
}

// The × on a tab, at its right end; only on tabs wide enough to keep some name.
pub(crate) fn tab_close_rect(tab: RECT, dpi: i32) -> Option<RECT> {
    let s = |v: i32| v * dpi / 96;
    (tab.right - tab.left >= s(64)).then(|| RECT { left: tab.right - s(22), right: tab.right - s(4), ..tab })
}

// The folder icon's square at a tab's left (s(16), centred top to bottom); the name then starts at s(28).
// Decided for the whole strip (all tabs are as wide): `close` = several tabs, so the tightest name room is
// the current tab's, up to its ×. None when that would leave under s(24) of name: names only, as before.
pub(crate) fn tab_icon_rect(tab: RECT, close: bool, dpi: i32) -> Option<RECT> {
    let s = |v: i32| v * dpi / 96;
    let end = if close { s(22) } else { s(8) };
    if tab.right - tab.left - s(28) - end < s(24) {
        return None;
    }
    let (left, top) = (tab.left + s(8), tab.top + (tab.bottom - tab.top - s(16)) / 2);
    Some(RECT { left, top, right: left + s(16), bottom: top + s(16) })
}

pub(crate) fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

// The search icon / filter chip at the filter pill's left, where the edit doesn't reach (it starts at s(24)).
pub(crate) fn filter_icon_rect(filt: RECT, s24: i32) -> Option<RECT> {
    let r = RECT { right: filt.right.min(filt.left + s24), ..filt };
    (r.right > r.left).then_some(r)
}

// Colour label strip along the pane's top edge: inside the border, above the buttons and fields.
pub(crate) fn tint_band(w: i32, dpi: i32, active: bool) -> RECT {
    let s = |v: i32| v * dpi / 96;
    let inset = if active { s(2) } else { 1 };
    let top = pane_geo(w, s(100), dpi).num.top;
    RECT { left: inset, top: inset, right: w - inset, bottom: inset + s(3).min(top - inset) }
}

// A pane's colour label in this theme: one of the selection colour presets (the light ones under high
// contrast, whose presets are all Windows' highlight, so labels stay told apart).
pub(crate) fn tint_color(k: u8) -> Option<u32> {
    (1..=6).contains(&k).then(|| theme_pal(DARK_ON.load(Ordering::Relaxed)).accents[k as usize - 1].1)
}

pub(crate) fn tint_name(k: u8) -> &'static str {
    tr(ACCENT_EN[k as usize - 1], LIGHT.accents[k as usize - 1].0)
}

/// A `t` px line down the middle of splitter `d`: the drag guide and the hover mark. None if `g` has no such splitter.
pub(crate) fn gap_line(g: &Geo, d: Hit, t: i32) -> Option<RECT> {
    let vbar = |x: i32| RECT { left: x - t / 2, top: g.top, right: x - t / 2 + t, bottom: g.bot };
    match d {
        Hit::Side => g.tree.then(|| vbar(g.pad + g.side + g.gap / 2)),
        Hit::Col(k) if !g.zoom => {
            g.xs.get(k + 1)?;
            Some(vbar(g.xs[k].1 + g.gap / 2))
        }
        Hit::Row(c, k) if !g.zoom => {
            let (x0, x1) = *g.xs.get(c)?;
            g.ys.get(c)?.get(k + 1)?;
            let y = g.ys[c][k].1 + g.gap / 2;
            Some(RECT { left: x0, top: y - t / 2, right: x1, bottom: y - t / 2 + t })
        }
        _ => None,
    }
}

pub(crate) fn hit(g: &Geo, x: i32, y: i32) -> Option<Hit> {
    if y < g.top || y >= g.bot {
        return None;
    }
    if x >= g.pad + g.side && x < g.gx {
        return Some(Hit::Side);
    }
    if x < g.gx || g.zoom {
        return None;
    }
    let between = |v: &[(i32, i32)], p: i32| v.windows(2).position(|w| p >= w[0].1 && p < w[1].0);
    if let Some(k) = between(&g.xs, x) {
        return Some(Hit::Col(k));
    }
    // row gaps belong to one column: rows no longer line up across columns
    let c = g.xs.iter().position(|&(a, b)| x >= a && x < b)?;
    between(&g.ys[c], y).map(|k| Hit::Row(c, k))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_setup_places() {
        // listed in any order, the same setup gives the same text; negative corners (a screen left of / above the main one)
        let (lap, desk) = ([0, 0, 1920, 1080], [-2560, -360, 0, 1080]);
        assert_eq!(mon_sig(&[lap, desk]), "-2560,-360,0,1080;0,0,1920,1080");
        assert_eq!(mon_sig(&[desk, lap]), mon_sig(&[lap, desk]));
        assert_eq!(mon_sig(&[lap]), "0,0,1920,1080");
        let (office, home) = (mon_sig(&[lap, desk]), mon_sig(&[lap]));
        let (big, small, moved) = ([-2500, 0, -100, 1000, 1], [100, 100, 1100, 800, 0], [200, 100, 1200, 800, 0]);

        // a new setup is noted at once, at the front; the same one already in front stays put (same ini text)
        let mut p = Vec::new();
        assert_eq!(remember_place(&mut p, &office, big, None), None);
        assert_eq!(remember_place(&mut p, &home, small, Some(small)), None); // first time seen: noted despite the hold
        assert_eq!(p, vec![(home.clone(), small), (office.clone(), big)]);
        let before = p.clone();
        remember_place(&mut p, &home, small, None);
        assert_eq!(p, before);

        // back at the office: where Windows put the window does not overwrite the office place ...
        let squeezed = [0, 0, 1900, 1000, 0];
        assert_eq!(remember_place(&mut p, &office, squeezed, Some(squeezed)), Some(squeezed));
        assert_eq!(p, before);
        // ... until the window moves away from there: then it is noted and goes to the front
        assert_eq!(remember_place(&mut p, &office, big, Some(squeezed)), None);
        assert_eq!(p, vec![(office.clone(), big), (home.clone(), small)]);
        remember_place(&mut p, &home, moved, None);
        assert_eq!(p[0], (home.clone(), moved));

        // WIN_PLACES at most: the setup used longest ago goes
        let sigs: Vec<String> = (1..=WIN_PLACES as i32 + 1).map(|i| mon_sig(&[[0, 0, 1000 + i, 800]])).collect();
        let mut q = Vec::new();
        for (i, g) in sigs.iter().enumerate() {
            remember_place(&mut q, g, [i as i32, 0, 10, 10, 0], None);
        }
        assert_eq!(q.len(), WIN_PLACES);
        assert_eq!(q[0].0, sigs[WIN_PLACES]);
        assert!(!q.iter().any(|(g, _)| *g == sigs[0]));

        // opening: this setup's place, else win= (also with no monitor list at all)
        let win = Some([1, 2, 3, 4, 0]);
        assert_eq!(pick_place(&p, Some(office.as_str()), win), Some(big));
        assert_eq!(pick_place(&p, Some(home.as_str()), None), Some(moved));
        assert_eq!(pick_place(&p, Some("0,0,800,600"), win), win);
        assert_eq!(pick_place(&p, None, win), win);
        assert_eq!(pick_place(&[], Some(home.as_str()), None), None);
    }

    #[test]
    fn bars_grow_with_text_size() {
        let r = |l, t, rr, b| RECT { left: l, top: t, right: rr, bottom: b };
        // default text size (grow 0): the same pixels as before
        assert_eq!(bars_h(96, 0), (34, 24, 24));
        assert_eq!(bars_h(144, 0), (51, 36, 36));
        for dpi in [96, 120, 144, 192] {
            assert_eq!(tab_strip_h_at(dpi, 0, true), 26 * dpi / 96);
            assert_eq!(tab_strip_h_at(dpi, 8, false), 0); // tabs off: no strip, grown or not
        }
        let pg = pane_geo_at(800, 400, 96, 0, true);
        assert_eq!(pg.addr.top, 4);
        assert_eq!(pg.addr.bottom, 26);
        assert_eq!(pg.num, r(4, 4, 22, 26));
        assert_eq!(pg.btn[0], r(22, 4, 46, 26));
        assert_eq!(pg.fav, r(772, 4, 796, 26));
        assert_eq!(pg.tabs, r(2, 30, 798, 56));
        assert_eq!(pg.view, r(2, 56, 798, 398));
        assert_eq!(pane_geo_at(800, 400, 96, 0, false).view, r(2, 30, 798, 398));
        let p0 = pane_geo_at(1000, 500, 144, 0, true);
        assert_eq!((p0.addr.top, p0.addr.bottom, p0.tabs.top, p0.tabs.bottom, p0.view.top), (6, 39, 45, 84, 84));

        // Text size 100 .. 225 % at common DPIs: everything still holds its text
        for dpi in [96, 120, 144, 168, 192] {
            for pct in [100, 125, 150, 175, 200, 225] {
                let lf = (12 * dpi * pct + 4800) / 9600;
                let tm = (lf * 4 + 2) / 3 + 1; // Segoe UI's line is about 4/3 of its em, one more for GDI rounding
                let grow = crate::text_grow(-lf, tm, dpi);
                let ctx = format!("dpi {dpi} text {pct}%: lf {lf} tm {tm} grow {grow}");
                let (top, btn_h, status_h) = bars_h(dpi, grow);
                assert!(btn_h >= tm && 5 * dpi / 96 + btn_h <= top, "toolbar {ctx}");
                assert!(status_h >= tm, "status {ctx}");
                let pg = pane_geo_at(800, 400, dpi, grow, true);
                for p in [pg.addr, pg.filt] {
                    assert!(p.bottom - p.top >= tm + 2, "edit in pill {ctx}"); // layout_pane's edit is fh + 2 tall
                    assert!(p.top >= 0 && p.bottom <= pg.tabs.top, "pill in bar {ctx}");
                }
                // the fields keep their distance from the top, so the colour label band above them stays as it is
                assert_eq!(pg.addr.top, pane_geo_at(800, 400, dpi, 0, true).addr.top, "{ctx}");
                let (tabs, plus) = tab_rects(pg.tabs, 3, dpi);
                for t in tabs.iter().chain([&plus]) {
                    assert!(t.bottom - t.top >= tm && t.bottom <= pg.view.top, "tab {ctx}");
                }
                assert!(pg.view.bottom - pg.view.top > 0, "{ctx}");
                // a squashed pane: the list never gets a negative height
                for h in [0, 20, 60] {
                    let v = pane_geo_at(300, h, dpi, grow, true).view;
                    assert!(v.bottom >= v.top, "list in a {h} px pane {ctx}");
                }
            }
        }
    }

    #[test]
    fn nudge_targets() {
        // columns of 2 and 1 rows: slots 0, 1 in column 0, slot 2 in column 1
        let h = [2, 1];
        assert_eq!(nudge_target(&h, 0, 1, 0), Some((Hit::Col(0), 1)));
        assert_eq!(nudge_target(&h, 1, -1, 0), Some((Hit::Col(0), -1)));
        // the last column moves its left boundary, the same way as the arrow
        assert_eq!(nudge_target(&h, 2, 1, 0), Some((Hit::Col(0), 1)));
        assert_eq!(nudge_target(&h, 2, -1, 0), Some((Hit::Col(0), -1)));
        assert_eq!(nudge_target(&h, 0, 0, 1), Some((Hit::Row(0, 0), 1)));
        assert_eq!(nudge_target(&h, 1, 0, -1), Some((Hit::Row(0, 0), -1)));
        assert_eq!(nudge_target(&h, 2, 0, 1), None); // one row in column 1
        // one pane: nothing to move
        assert_eq!(nudge_target(&[1], 0, 1, 0), None);
        assert_eq!(nudge_target(&[1], 0, 0, -1), None);
        assert_eq!(nudge_target(&[], 0, 1, 0), None);
        // 4x4: slot 6 = column 1, row 2; slot 15 = last column, last row
        let g = [4, 4, 4, 4];
        assert_eq!(nudge_target(&g, 6, 1, 0), Some((Hit::Col(1), 1)));
        assert_eq!(nudge_target(&g, 6, 0, -1), Some((Hit::Row(1, 2), -1)));
        assert_eq!(nudge_target(&g, 15, -1, 0), Some((Hit::Col(2), -1)));
        assert_eq!(nudge_target(&g, 15, 0, 1), Some((Hit::Row(3, 2), 1)));
        // columns of different heights: the row index is the pane's own column's
        assert_eq!(nudge_target(&[1, 3], 3, 0, 1), Some((Hit::Row(1, 1), 1)));
        assert_eq!(nudge_target(&[3, 2], 4, 0, 1), Some((Hit::Row(1, 0), 1)));
    }

    #[test]
    fn drag_ratio_stops_at_the_limit() {
        // 2 columns, 1000 px with an 8 px gap, 60 px minimum: past the edge the ratio stays where it is
        let mut r = vec![0.5, 0.5];
        drag_ratio(&mut r, 0, 5000, 0, 1000, 8, 60);
        let end = r.clone();
        drag_ratio(&mut r, 0, 5032, 0, 1000, 8, 60);
        assert_eq!(r, end);
        assert!((r[1] - 60.0 / 992.0).abs() < 1e-9);
        drag_ratio(&mut r, 0, -500, 0, 1000, 8, 60);
        assert!((r[0] - 60.0 / 992.0).abs() < 1e-9 && (r[0] + r[1] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn gap_lines() {
        // tree shown, two columns of 2 and 1 rows
        let two = |tree: bool, zoom: bool| Geo {
            w: 1000, h: 600, top: 34, bot: 576, pad: 8, gap: 8, side: 240, tree, gx: 256, zoom,
            xs: vec![(256, 620), (628, 992)],
            ys: vec![vec![(34, 300), (308, 576)], vec![(34, 576)]],
        };
        let g = two(true, false);
        for d in [Hit::Side, Hit::Col(0), Hit::Row(0, 0)] {
            let r = gap_line(&g, d, 2).unwrap();
            assert_eq!(hit(&g, (r.left + r.right) / 2, (r.top + r.bottom) / 2), Some(d), "{d:?}");
        }
        // no such splitter: a stale number never reaches past the vectors
        for d in [Hit::Col(1), Hit::Col(5), Hit::Row(1, 0), Hit::Row(0, 1), Hit::Row(3, 0)] {
            assert_eq!(gap_line(&g, d, 2), None, "{d:?}");
        }
        let one = Geo { gx: 8, xs: vec![(8, 992)], ys: vec![vec![(34, 576)]], ..two(false, false) };
        assert_eq!(gap_line(&one, Hit::Row(3, 0), 2), None);
        assert_eq!(gap_line(&one, Hit::Row(0, 0), 2), None);
        assert_eq!(gap_line(&one, Hit::Side, 2), None);
        assert_eq!(gap_line(&two(true, true), Hit::Col(0), 2), None);
        assert_eq!(gap_line(&two(true, true), Hit::Row(0, 0), 2), None);
    }

    #[test]
    fn remap_keeps_cells() {
        let ident: [usize; MAXP] = std::array::from_fn(|k| k);
        // which original pane each slot shows after going through the layouts
        let run = |steps: &[Vec<usize>]| {
            let (mut id, mut home) = (ident, [None; MAXP]);
            for w in steps.windows(2) {
                let (p, h) = remap(&w[0], &w[1], &home);
                (id, home) = (p.map(|k| id[k]), h);
            }
            id
        };
        let tail = |v: &[usize]| v.iter().copied().chain(v.len()..MAXP).collect::<Vec<_>>();
        assert_eq!(run(&[vec![2, 2], vec![3, 2]])[..], tail(&[0, 1, 4, 2, 3])[..]);
        assert_eq!(run(&[vec![1, 1, 1, 1], vec![4]])[..], tail(&[0, 4, 5, 6, 1, 2, 3])[..]);
        for steps in [
            vec![vec![1, 1, 1, 1], vec![4], vec![1, 1, 1, 1]],
            vec![vec![3, 3], vec![2, 2], vec![2, 3], vec![3, 3]],
            vec![vec![3, 2], vec![2, 3], vec![3, 2]],
            vec![vec![2, 2], vec![2, 2]],
        ] {
            assert_eq!(run(&steps), ident, "{steps:?}");
        }
        // random switches: always a permutation, cells in both layouts keep their pane, homes come back
        let (mut seed, mut cur, mut id, mut home) = (7u32, vec![2, 2], ident, [None; MAXP]);
        let mut rnd = |n: u32| {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            (seed >> 16) as usize % n as usize
        };
        for _ in 0..3000 {
            let new: Vec<usize> = (0..rnd(4) + 1).map(|_| rnd(4) + 1).collect();
            let (p, h) = remap(&cur, &new, &home);
            let mut sorted = p;
            sorted.sort();
            assert_eq!(sorted, ident, "{cur:?} -> {new:?}");
            let nid = p.map(|k| id[k]);
            let idx = |l: &[usize], (c, r): (usize, usize)| (c < l.len() && r < l[c]).then(|| l[..c].iter().sum::<usize>() + r);
            let (on, n) = (cur.iter().sum::<usize>(), new.iter().sum::<usize>());
            assert!(h[..n].iter().all(Option::is_none));
            let hidden: Vec<_> = h[n..].iter().flatten().collect();
            assert!(hidden.iter().all(|&&c| idx(&new, c).is_none()));
            assert!(hidden.iter().enumerate().all(|(a, x)| !hidden[a + 1..].contains(x)));
            for j in 0..on {
                if let Some(k) = idx(&new, slot(&cur, j)) {
                    assert_eq!(nid[k], id[j]);
                }
            }
            for j in on..MAXP {
                if let Some(k) = home[j].and_then(|c| idx(&new, c)) {
                    assert_eq!(nid[k], id[j]);
                }
            }
            (cur, id, home) = (new, nid, h);
        }
    }
}
