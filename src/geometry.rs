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

/// `fit` for a saved rcNormalPosition, which is in workspace coordinates (origin at rcWork's corner).
pub(crate) fn fit_saved(saved: RECT, mon: RECT, work: RECT) -> RECT {
    let (dx, dy) = (work.left - mon.left, work.top - mon.top);
    let r = fit(RECT { left: saved.left + dx, top: saved.top + dy, right: saved.right + dx, bottom: saved.bottom + dy }, work);
    RECT { left: r.left - dx, top: r.top - dy, right: r.right - dx, bottom: r.bottom - dy }
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

pub(crate) fn geo(a: &App) -> Geo {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(a.main, &mut rc) };
    let s = |v: i32| v * a.dpi / 96;
    let (pad, gap, top) = (s(8), s(8), s(34));
    let bot = (rc.bottom - s(24)).max(top);
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
    let s = |v: i32| v * dpi / 96;
    let (bar, bh, b, pad) = (s(30), s(22), s(24), s(4));
    let strip = tab_strip_h(dpi);
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
        view: RECT { left: s(2), top: bar + strip, right: w - s(2), bottom: h - s(2) },
    }
}

pub(crate) fn tab_strip_h(dpi: i32) -> i32 {
    if TABS_ON.load(Ordering::Relaxed) { 26 * dpi / 96 } else { 0 }
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
