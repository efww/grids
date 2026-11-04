use crate::imports::*;
use super::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;

// ---------- grid geometry ----------

/// Shrinks `r` into `work` and slides it inside. A rect already inside comes back unchanged.
pub(crate) fn fit(r: RECT, work: RECT) -> RECT {
    let w = (r.right - r.left).min(work.right - work.left).max(0);
    let h = (r.bottom - r.top).min(work.bottom - work.top).max(0);
    let left = r.left.clamp(work.left, work.right - w);
    let top = r.top.clamp(work.top, work.bottom - h);
    RECT { left, top, right: left + w, bottom: top + h }
}

/// `fit` for a saved normal position, which is in workspace coordinates (origin at the work area's corner).
pub(crate) fn fit_saved(saved: RECT, mon: RECT, work: RECT) -> RECT {
    let (dx, dy) = (work.left - mon.left, work.top - mon.top);
    let r = fit(RECT { left: saved.left + dx, top: saved.top + dy, right: saved.right + dx, bottom: saved.bottom + dy }, work);
    RECT { left: r.left - dx, top: r.top - dy, right: r.right - dx, bottom: r.bottom - dy }
}

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

/// Smallest pane plate, at 100% DPI. `pane_min_w` / `pane_min_h` scale it.
pub(crate) const PANE_MIN_W: i32 = 140;
pub(crate) const PANE_MIN_H: i32 = 120;

pub(crate) fn pane_min_w(dpi: i32) -> i32 {
    PANE_MIN_W * dpi.max(1) / 96
}

pub(crate) fn pane_min_h(dpi: i32) -> i32 {
    PANE_MIN_H * dpi.max(1) / 96
}

/// `cells`, but no cell is thinner than `min` when `len` can hold that.
/// A row that is too short keeps the proportional split: the window minimum is what
/// stops that, and a monitor that cannot grow stays inside its work area instead of
/// painting panes past the edge.
pub(crate) fn cells_min(start: i32, len: i32, ratios: &[f64], gap: i32, min: i32) -> Vec<(i32, i32)> {
    let n = ratios.len();
    if n == 0 {
        return Vec::new();
    }
    let raw = cells(start, len, ratios, gap);
    if min <= 0 {
        return raw;
    }
    let gaps = gap * (n as i32 - 1);
    let avail = len - gaps;
    if avail < n as i32 * min || raw.iter().all(|(a, b)| b - a >= min) {
        return raw;
    }
    let mut w: Vec<i32> = raw.iter().map(|(a, b)| (b - a).max(0)).collect();
    let mut need = 0i32;
    for x in &mut w {
        if *x < min {
            need += min - *x;
            *x = min;
        }
    }
    while need > 0 {
        let Some((i, _)) = w.iter().enumerate().filter(|(_, x)| **x > min).max_by_key(|(_, x)| *x) else {
            break;
        };
        let take = (w[i] - min).min(need);
        w[i] -= take;
        need -= take;
    }
    let mut x = start;
    w.into_iter()
        .map(|wd| {
            let cell = (x, x + wd);
            x += wd + gap;
            cell
        })
        .collect()
}

/// Ratios that match `cells_min`, when the proportional split would paint a pane below `min`
/// and the row is long enough to honour it. None leaves the saved ratios alone (a short
/// window must not flatten them).
pub(crate) fn ratios_matching_min(ratios: &[f64], len: i32, gap: i32, min: i32) -> Option<Vec<f64>> {
    if ratios.len() < 2 || min <= 0 {
        return None;
    }
    let raw = cells(0, len, ratios, gap);
    let clamped = cells_min(0, len, ratios, gap, min);
    if raw == clamped {
        return None;
    }
    let w: Vec<f64> = clamped.iter().map(|(a, b)| (b - a).max(1) as f64).collect();
    let sum = w.iter().sum::<f64>();
    (sum > 0.0).then(|| w.into_iter().map(|x| x / sum).collect())
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

/// Minimum height of the folder tree and of the favourites list, in px at this DPI.
pub(crate) fn side_min(dpi: i32) -> i32 {
    80 * dpi.max(1) / 96
}

/// Client size that fits the current tab: every pane at `PANE_MIN_W` × `PANE_MIN_H`,
/// the left panel when it is open, the preview pane when it is open, the tab strip,
/// the status line, and the icon group.
pub(crate) fn client_min(heights: &[usize], dpi: i32, tree_on: bool, bar: bool, stat: bool, preview_on: bool) -> (i32, i32) {
    let s = |v: i32| v * dpi.max(1) / 96;
    let (pad, gap) = (s(8), s(8));
    let cols = heights.len().max(1) as i32;
    let rows = heights.iter().copied().max().unwrap_or(1).max(1) as i32;
    let grid_w = cols * pane_min_w(dpi) + gap * (cols - 1).max(0);
    let grid_h = rows * pane_min_h(dpi) + gap * (rows - 1).max(0);
    let left = if tree_on { s(120) + gap } else { pad };
    let preview = if preview_on { gap + s(PREVIEW_DEF_DIP) } else { 0 };
    let width = (left + grid_w + pad + preview).max(super::tabs::icon_group_min(dpi, bar));
    let side_h = if tree_on { side_min(dpi) * 2 + gap } else { 0 };
    let height = s(32) + grid_h.max(side_h) + if stat { s(24) } else { 0 };
    (width, height)
}

/// Grow `cur` until it is at least `min_w` by `min_h`. The top-left stays, then the rect
/// slides back into `work`. A window that is already large enough is returned unchanged,
/// so a smaller tab does not shrink it. A minimum the monitor cannot hold becomes the work area.
pub(crate) fn grow_into_work(cur: RECT, min_w: i32, min_h: i32, work: RECT) -> RECT {
    let w = (cur.right - cur.left).max(0);
    let h = (cur.bottom - cur.top).max(0);
    if w >= min_w && h >= min_h {
        return cur;
    }
    let work_w = (work.right - work.left).max(0);
    let work_h = (work.bottom - work.top).max(0);
    let nw = w.max(min_w).min(work_w);
    let nh = h.max(min_h).min(work_h);
    let mut left = cur.left;
    let mut top = cur.top;
    if left + nw > work.right {
        left = work.right - nw;
    }
    if top + nh > work.bottom {
        top = work.bottom - nh;
    }
    if left < work.left {
        left = work.left;
    }
    if top < work.top {
        top = work.top;
    }
    RECT { left, top, right: left + nw, bottom: top + nh }
}

/// Outer size whose client area is `client_w` by `client_h` for the main window's style.
pub(crate) fn frame_around(client_w: i32, client_h: i32, dpi: i32) -> (i32, i32) {
    use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;
    let mut r = RECT { left: 0, top: 0, right: client_w.max(0), bottom: client_h.max(0) };
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    if unsafe { AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE(0), dpi.max(1) as u32) }.is_err() {
        return (client_w.max(1), client_h.max(1));
    }
    ((r.right - r.left).max(1), (r.bottom - r.top).max(1))
}

fn monitor_work(h: HWND) -> Option<RECT> {
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() }.then_some(mi.rcWork)
}

/// `WM_GETMINMAXINFO` for the main window. The current tab only. Capped to the work area,
/// so a layout the monitor cannot hold still leaves the window on screen.
pub(crate) unsafe fn min_track_size(h: HWND) -> (i32, i32) {
    let (cw, ch, dpi) = app(|a| {
        let (w, ht) = client_min(&a.s.heights, a.dpi, a.s.tree_on, a.s.bar, a.s.stat, a.s.preview_on);
        (w, ht, a.dpi)
    });
    let (mut tw, mut th) = frame_around(cw, ch, dpi);
    if let Some(work) = monitor_work(h) {
        tw = tw.min((work.right - work.left).max(1));
        th = th.min((work.bottom - work.top).max(1));
    }
    (tw.max(1), th.max(1))
}

thread_local! {
    static FITTING: Cell<bool> = const { Cell::new(false) };
}

struct FittingGuard;
impl Drop for FittingGuard {
    fn drop(&mut self) {
        FITTING.with(|f| f.set(false));
    }
}

/// If the current tab's panes no longer fit, grow the window just enough. Maximized,
/// minimized, and an interactive resize are left alone. A smaller tab only lowers the
/// minimum track; this does not shrink. `SetWindowPos` re-enters `relayout`; the guard
/// stops that from growing again.
pub(crate) unsafe fn ensure_window_fits() {
    if FITTING.with(|f| f.replace(true)) {
        return;
    }
    let _guard = FittingGuard;
    let Some(main) = app(|a| (!a.sizing && !a.main.0.is_null()).then_some(a.main)) else {
        return;
    };
    if IsZoomed(main).as_bool() || IsIconic(main).as_bool() {
        return;
    }
    let mut cur = RECT::default();
    if GetWindowRect(main, &mut cur).is_err() {
        return;
    }
    let Some(work) = monitor_work(main) else { return };
    let (cw, ch, dpi) = app(|a| {
        let (w, ht) = client_min(&a.s.heights, a.dpi, a.s.tree_on, a.s.bar, a.s.stat, a.s.preview_on);
        (w, ht, a.dpi)
    });
    let (min_w, min_h) = frame_around(cw, ch, dpi);
    let next = grow_into_work(cur, min_w, min_h, work);
    if next == cur {
        return;
    }
    _ = SetWindowPos(
        main,
        None,
        next.left,
        next.top,
        next.right - next.left,
        next.bottom - next.top,
        SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

/// Pixel heights of the folder tree and the favourites list inside a plate of `height` px.
/// `ratio` is the tree's share of the space left after the `gap` px splitter.
/// Each part keeps at least `min` px when the plate is tall enough.
pub(crate) fn split_heights(height: i32, ratio: f64, gap: i32, min: i32) -> (i32, i32) {
    let avail = (height - gap).max(0);
    if avail == 0 {
        return (0, 0);
    }
    let min = min.clamp(0, avail / 2);
    let tree = ((avail as f64) * sane_split(ratio)).round() as i32;
    let tree = tree.clamp(min, avail - min);
    (tree, avail - tree)
}

/// Ratio that places the splitter so its top edge is at `pos`. Both parts stay at least `min` px.
/// The value is the upper half's share. `tree_share_at` turns that into the folder tree's share.
pub(crate) fn split_ratio_at(pos: i32, top: i32, height: i32, gap: i32, min: i32) -> f64 {
    let avail = (height - gap).max(1);
    let raw = (pos - top).clamp(0, avail) as f64 / avail as f64;
    let (tree, _) = split_heights(height, raw, gap, min);
    tree as f64 / avail as f64
}

/// Folder tree's share after a drag that puts the splitter's top edge at `pos`.
/// `side_split` is that share in either order. When favourites are on top, `pos` measures
/// their height and the tree keeps what is left.
pub(crate) fn tree_share_at(pos: i32, top: i32, height: i32, gap: i32, min: i32, fav_top: bool) -> f64 {
    let top_share = split_ratio_at(pos, top, height, gap, min);
    if !fav_top {
        return top_share;
    }
    let avail = (height - gap).max(1);
    let (top_h, _) = split_heights(height, top_share, gap, min);
    (avail - top_h).max(0) as f64 / avail as f64
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Hit {
    Side,
    SideSplit,         // horizontal divider between the folder tree and favourites
    Col(usize),        // boundary after column k
    Row(usize, usize), // (column, boundary after row k in it)
    Preview,           // vertical splitter left of the preview pane
}

// Main window: tab strip on top, status line at the bottom, plates (sidebar + grid) in between.
// The gaps between plates are the splitters. Drive buttons and the icon group sit on the strip.
#[derive(Clone)]
pub(crate) struct Geo {
    pub(crate) w: i32,
    pub(crate) h: i32,   // client height
    pub(crate) top: i32, // tab strip bottom = plates top
    pub(crate) bot: i32, // plates bottom = status line top
    pub(crate) pad: i32,
    pub(crate) gap: i32,
    pub(crate) side: i32,   // sidebar plate width
    pub(crate) tree_h: i32, // folder-tree height inside the plate
    pub(crate) fav_top: bool, // favourites above the tree; false puts the tree on top
    pub(crate) gx: i32,     // grid left
    pub(crate) preview: i32, // preview plate width in px; 0 hides it. Zoom keeps it, like the left panel.
    pub(crate) xs: Vec<(i32, i32)>,
    pub(crate) ys: Vec<Vec<(i32, i32)>>, // per column
    pub(crate) zoom: bool, // one pane fills the grid: no splitters
}

/// Width of the preview plate in px. `0` in `stored_dip` means the default until the user drags.
/// A tight window keeps one pane's minimum and shrinks the plate to whatever room is left.
pub(crate) const PREVIEW_MIN_DIP: i32 = 160;
pub(crate) const PREVIEW_DEF_DIP: i32 = 280;
const PREVIEW_MAX_DIP: i32 = 4000;

pub(crate) fn preview_dip(px: i32, dpi: i32) -> i32 {
    let dpi = dpi.max(1);
    let dip = px.saturating_mul(96) / dpi;
    dip.clamp(PREVIEW_MIN_DIP, PREVIEW_MAX_DIP)
}

pub(crate) fn preview_plate(on: bool, stored_dip: i32, dpi: i32, client_w: i32, grid_left: i32, gap: i32, pad: i32) -> i32 {
    if !on {
        return 0;
    }
    let dpi = dpi.max(1);
    let want_dip = if stored_dip <= 0 { PREVIEW_DEF_DIP } else { stored_dip.clamp(PREVIEW_MIN_DIP, PREVIEW_MAX_DIP) };
    let want = want_dip.saturating_mul(dpi) / 96;
    let room = client_w - grid_left - gap - pad - pane_min_w(dpi);
    if room <= 0 {
        return 0;
    }
    want.min(room)
}

/// A splitter release stores the clamped panel width. While the panel is hidden, the kept width stays.
pub(crate) fn kept_side(panel: bool, stored: i32, clamped: i32) -> i32 {
    if panel { clamped } else { stored }
}

pub(crate) fn geo(a: &App) -> Geo {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(a.main, &mut rc) };
    let s = |v: i32| v * a.dpi / 96;
    let (pad, gap) = (s(8), s(8));
    let side = if a.s.tree_on { a.s.side.clamp(s(120), (rc.right - gap - pad - s(240)).max(s(120))) } else { 0 };
    let gx = if a.s.tree_on { side + gap } else { pad };
    let preview = preview_plate(a.s.preview_on, a.s.preview_w, a.dpi, rc.right, gx, gap, pad);
    let grid_w = (rc.right - pad - gx - if preview > 0 { preview + gap } else { 0 }).max(0);
    let top = s(32); // tab strip. `bar` hides the icon group except the gear, not this row.
    let bot = if a.s.stat { (rc.bottom - s(24)).max(top) } else { rc.bottom };
    let plate = (bot - top).max(0);
    let tree_h = if side > 0 { split_heights(plate, a.s.side_split, gap, side_min(a.dpi)).0 } else { 0 };
    Geo {
        xs: cells_min(gx, grid_w, &a.s.colr, gap, pane_min_w(a.dpi)),
        ys: a.s.rowr.iter().map(|r| cells_min(top, bot - top, r, gap, pane_min_h(a.dpi))).collect(),
        w: rc.right,
        h: rc.bottom,
        top,
        bot,
        pad,
        gap,
        side,
        tree_h,
        fav_top: a.s.fav_top,
        gx,
        preview,
        zoom: a.zoom.is_some(),
    }
}

impl Geo {
    /// Right edge of the file grid. The preview plate and its splitter sit past this.
    pub(crate) fn grid_right(&self) -> i32 {
        if self.preview > 0 {
            self.w - self.pad - self.preview - self.gap
        } else {
            self.w - self.pad
        }
    }

    pub(crate) fn preview_rect(&self) -> RECT {
        let left = self.w - self.pad - self.preview;
        RECT { left, top: self.top, right: self.w - self.pad, bottom: self.bot }
    }

    /// Height of the upper half, above the divider. `tree_h` is always the folder tree.
    pub(crate) fn side_top_h(&self) -> i32 {
        if self.fav_top {
            let plate = (self.bot - self.top).max(0);
            (plate - self.gap - self.tree_h).max(0)
        } else {
            self.tree_h
        }
    }
}

/// Gap between the two halves of the left panel. None when the panel or the divider is hidden.
pub(crate) fn side_gap_rect(g: &Geo) -> Option<RECT> {
    if g.side <= 0 || g.tree_h <= 0 || g.gap <= 0 {
        return None;
    }
    let top = g.top + g.side_top_h();
    Some(RECT { left: 0, top, right: g.side, bottom: top + g.gap })
}

/// Folder-tree rect, then the favourites rect. `inset` pads the plate's outer edges, not the gap.
pub(crate) fn side_panes(g: &Geo, inset: i32) -> (RECT, RECT) {
    let sr = side_rect(g);
    let top_h = g.side_top_h();
    let upper = RECT { left: sr.left + inset, top: sr.top + inset, right: sr.right - inset, bottom: sr.top + top_h };
    let lower = RECT { left: sr.left + inset, top: sr.top + top_h + g.gap, right: sr.right - inset, bottom: sr.bottom - inset };
    if g.fav_top { (lower, upper) } else { (upper, lower) }
}

pub(crate) fn side_rect(g: &Geo) -> RECT {
    RECT { left: 0, top: g.top, right: g.side, bottom: g.bot }
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
    (h.len().saturating_sub(1), h.last().copied().unwrap_or(1).saturating_sub(1))
}

/// Columns in one window, and rows in one column. 4 × 4 is the 16-pane grid.
pub(crate) const GRID_COLS: usize = 4;
pub(crate) const GRID_ROWS: usize = 4;

/// One header-menu split or close: the new rows-per-column, and which old pane
/// fills each new slot. `None` is the pane just added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GridChange {
    pub(crate) heights: Vec<usize>,
    pub(crate) slots: Vec<Option<usize>>,
}

/// Permutation that realises `slots`. `perm[new] = old`. `fresh_old` is the free
/// slot a split copies into, before the panes are moved. A pane passed as `leaving`
/// is parked off the grid with no home, so a later layout change does not bring it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Placed {
    pub(crate) perm: [usize; MAXP],
    pub(crate) home: Homes,
    pub(crate) fresh_old: Option<usize>,
}

fn pane_total(h: &[usize]) -> usize {
    h.iter().sum()
}

fn grid_addressed(h: &[usize], pane: usize) -> bool {
    !h.is_empty() && h.iter().all(|&n| n > 0) && pane < pane_total(h)
}

/// A new column of one pane, full height, immediately to the right of `pane`'s column.
/// Panes further right move over. None when the grid already has four columns, or `pane` is not on it.
pub(crate) fn split_right_grid(heights: &[usize], pane: usize) -> Option<GridChange> {
    if !grid_addressed(heights, pane) || heights.len() >= GRID_COLS || pane_total(heights) >= GRID_COLS * GRID_ROWS {
        return None;
    }
    let (col, _) = slot(heights, pane);
    let mut next = heights.to_vec();
    next.insert(col + 1, 1);
    let mut slots = Vec::with_capacity(pane_total(heights) + 1);
    let mut old = 0usize;
    for (c, &n) in heights.iter().enumerate() {
        for _ in 0..n {
            slots.push(Some(old));
            old += 1;
        }
        if c == col {
            slots.push(None);
        }
    }
    Some(GridChange { heights: next, slots })
}

/// A new row directly under `pane`, in that column only. None when the column already has four rows.
pub(crate) fn split_below_grid(heights: &[usize], pane: usize) -> Option<GridChange> {
    if !grid_addressed(heights, pane) || pane_total(heights) >= GRID_COLS * GRID_ROWS {
        return None;
    }
    let (col, row) = slot(heights, pane);
    if heights[col] >= GRID_ROWS {
        return None;
    }
    let mut next = heights.to_vec();
    next[col] += 1;
    let mut slots = Vec::with_capacity(pane_total(heights) + 1);
    let mut old = 0usize;
    for (c, &n) in heights.iter().enumerate() {
        for r in 0..n {
            slots.push(Some(old));
            old += 1;
            if c == col && r == row {
                slots.push(None);
            }
        }
    }
    Some(GridChange { heights: next, slots })
}

/// Remove `pane`. A column that held only that pane goes away; otherwise the rows under it move up.
/// None when it is the last pane.
pub(crate) fn close_pane_grid(heights: &[usize], pane: usize) -> Option<GridChange> {
    if !grid_addressed(heights, pane) || pane_total(heights) <= 1 {
        return None;
    }
    let (col, _) = slot(heights, pane);
    let mut next = heights.to_vec();
    if next[col] == 1 {
        next.remove(col);
    } else {
        next[col] -= 1;
    }
    let slots = (0..pane_total(heights)).filter(|&old| old != pane).map(Some).collect();
    Some(GridChange { heights: next, slots })
}

/// Where focus goes when the closed pane was the active one.
/// The pane below, else the one above, else the next column, else the previous column at the same row.
pub(crate) fn close_neighbour(heights: &[usize], pane: usize) -> Option<usize> {
    if !grid_addressed(heights, pane) || pane_total(heights) <= 1 {
        return None;
    }
    let (col, row) = slot(heights, pane);
    let rows = heights[col];
    if row + 1 < rows {
        return Some(pane + 1);
    }
    if row > 0 {
        return Some(pane - 1);
    }
    if col + 1 < heights.len() {
        return Some(pane + 1);
    }
    if col == 0 {
        return None;
    }
    let prev = heights[col - 1];
    let start = heights[..col - 1].iter().sum::<usize>();
    Some(start + row.min(prev - 1))
}

/// `slots` plus an optional pane that is leaving. The result is a permutation of every slot.
pub(crate) fn place_grid(slots: &[Option<usize>], leaving: Option<usize>, home: &Homes) -> Option<Placed> {
    let n = slots.len();
    if n == 0 || n > MAXP || slots.iter().filter(|s| s.is_none()).count() > 1 {
        return None;
    }
    let mut perm = [0usize; MAXP];
    let mut used = [false; MAXP];
    let mut fresh_at = None;
    for (i, slot) in slots.iter().enumerate() {
        if let Some(old) = *slot {
            if old >= MAXP || used[old] {
                return None;
            }
            perm[i] = old;
            used[old] = true;
        } else {
            fresh_at = Some(i);
        }
    }
    if let Some(old) = leaving {
        if old >= MAXP || used[old] {
            return None;
        }
    }
    let fresh_old = if let Some(at) = fresh_at {
        let pick = (0..MAXP).find(|&j| !used[j] && home[j].is_none()).or_else(|| (0..MAXP).find(|&j| !used[j]))?;
        perm[at] = pick;
        used[pick] = true;
        Some(pick)
    } else {
        None
    };
    let rest: Vec<usize> = (0..MAXP).filter(|&j| !used[j]).collect();
    if rest.len() != MAXP - n {
        return None;
    }
    let mut new_home = [None; MAXP];
    for (k, old) in (n..MAXP).zip(rest) {
        perm[k] = old;
        if Some(old) != leaving && Some(old) != fresh_old {
            new_home[k] = home[old];
        }
    }
    let mut seen = [false; MAXP];
    for &p in &perm {
        if p >= MAXP || seen[p] {
            return None;
        }
        seen[p] = true;
    }
    Some(Placed { perm, home: new_home, fresh_old })
}

/// The cell each off-screen slot's pane last had. Runtime only.
pub(crate) type Homes = [Option<(usize, usize)>; MAXP];

/// Layout `old` to `new`. Slot j shows the pane that was in slot `perm[j]`.
/// A pane whose cell is in both stays there. A cell that leaves is remembered and comes back.
/// New cells take the lowest slots that have no home.
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

// Inside a pane plate: icon buttons, address pill, ⇄, filter pill on top; a pane-tab
// strip under that bar when this pane has two or more tabs; the Explorer view below.
// One tab hides the strip (the list uses it) and shows `add` just to the right of ⇄.
// Window tabs are a different strip, on the main window.
pub(crate) struct PaneGeo {
    pub(crate) num: RECT,      // pane number (Ctrl+N); the header drag handle, kept on a narrow pane
    pub(crate) btn: [RECT; 4], // back, forward, up, open-in
    pub(crate) addr: RECT,
    pub(crate) add: RECT, // add-tab button; zero width while the strip is showing
    pub(crate) filt: RECT,
    pub(crate) fav: RECT,
    pub(crate) tabs: RECT, // pane-tab strip between the bar and the list; empty while this pane has one tab
    pub(crate) view: RECT,
}

/// Address pill that still shows the current folder name. The name is cut with an ellipsis
/// when it is longer than this. Lower-priority chrome gives up space before the pill does.
const ADDR_MIN_DIP: i32 = 80;

struct HeaderChrome {
    back: bool,
    fwd: bool,
    up: bool,
    swap: bool,
    add: bool,
    fav: bool,
    filt: i32,
}

/// `strip` when this pane has two or more tabs. Callers that have a window use `pane_geo_of`.
/// The number stays. The address stays wide enough for the folder name. The filter shrinks
/// first, then up, forward, back, +, ⇄ and the star, in that order.
pub(crate) fn pane_geo(w: i32, h: i32, dpi: i32, strip: bool) -> PaneGeo {
    let s = |v: i32| v * dpi / 96;
    // back, forward, up, ⇄, favourite; the add-tab button joins them while the strip is hidden
    let nbtn = if strip { 5 } else { 6 };
    let mut chrome = HeaderChrome {
        back: true,
        fwd: true,
        up: true,
        swap: true,
        add: !strip,
        fav: true,
        filt: s(150).min((w - 2 * s(4) - nbtn * s(24)) / 3).max(0),
    };
    let addr_min = s(ADDR_MIN_DIP);
    loop {
        let laid = place_header(w, h, dpi, strip, &chrome);
        let addr_w = laid.addr.right - laid.addr.left;
        if addr_w >= addr_min {
            return laid;
        }
        if chrome.filt > 0 {
            // A sliver cannot hold the search icon, so the filter goes away rather than shrink to one.
            let need = addr_min - addr_w;
            let next = if need < chrome.filt { chrome.filt - need } else { 0 };
            chrome.filt = if next < s(24) { 0 } else { next };
            continue;
        }
        if chrome.up {
            chrome.up = false;
            continue;
        }
        if chrome.fwd {
            chrome.fwd = false;
            continue;
        }
        if chrome.back {
            chrome.back = false;
            continue;
        }
        if chrome.add {
            chrome.add = false;
            continue;
        }
        if chrome.swap {
            chrome.swap = false;
            continue;
        }
        if chrome.fav {
            chrome.fav = false;
            continue;
        }
        return laid;
    }
}

fn place_header(w: i32, h: i32, dpi: i32, strip: bool, chrome: &HeaderChrome) -> PaneGeo {
    let s = |v: i32| v * dpi / 96;
    let (bar, bh, b, pad, gap) = (s(30), s(22), s(24), s(4), s(2));
    let strip_h = if strip { s(26) } else { 0 };
    let y = (bar - bh) / 2;
    let rect = |x: i32, wd: i32| RECT { left: x, top: y, right: x + wd.max(0), bottom: y + bh };
    let zero = |x: i32| rect(x, 0);
    let num_left = if w > pad { pad } else { 0 };
    let num = rect(num_left, s(18).min((w - num_left).max(0)));
    let mut x = num.right;
    let take = |x: &mut i32, on: bool| -> RECT {
        if !on {
            return zero(*x);
        }
        let r = rect(*x, b);
        *x += b;
        r
    };
    let btn0 = take(&mut x, chrome.back);
    let btn1 = take(&mut x, chrome.fwd);
    let btn2 = take(&mut x, chrome.up);
    let nav = chrome.back || chrome.fwd || chrome.up;

    // Right side, packed from the window edge: star, filter, +, ⇄.
    let mut edge = w - pad;
    let fav = if chrome.fav {
        edge -= b;
        rect(edge, b)
    } else {
        zero(edge)
    };
    let filt = if chrome.filt > 0 {
        if chrome.fav {
            edge -= gap;
        }
        edge -= chrome.filt;
        rect(edge, chrome.filt)
    } else {
        zero(edge)
    };
    let add = if chrome.add {
        if chrome.fav || chrome.filt > 0 {
            edge -= gap;
        }
        edge -= b;
        rect(edge, b)
    } else {
        zero(edge)
    };
    let swap_on = chrome.swap;
    let btn3 = if swap_on {
        if chrome.fav || chrome.filt > 0 || chrome.add {
            edge -= gap;
        }
        edge -= b;
        rect(edge, b)
    } else {
        zero(edge)
    };
    let right = chrome.fav || chrome.filt > 0 || chrome.add || swap_on;
    let addr_left = if nav { x + pad } else { x };
    let addr_right = if right { edge - pad } else { (w - pad).max(addr_left) };
    PaneGeo {
        num,
        btn: [btn0, btn1, btn2, btn3],
        addr: rect(addr_left, (addr_right - addr_left).max(0)),
        add,
        filt,
        fav,
        tabs: RECT { left: s(2), top: bar, right: w - s(2), bottom: bar + strip_h },
        // 2px in, so the active ring stays visible around the list
        view: RECT { left: s(2), top: bar + strip_h, right: w - s(2), bottom: h - s(2) },
    }
}

/// This window's bar. The strip is on for two or more tabs, and for one tab that is locked or renamed.
pub(crate) fn pane_geo_of(h: HWND, w: i32, height: i32, dpi: i32) -> PaneGeo {
    let strip = pane_of(h).is_some_and(|i| app(|a| a.s.pane_strip_on(i)));
    pane_geo(w, height, dpi, strip)
}

/// Pane tabs share the strip evenly up to a comfortable width, then a '+' after the last. (tabs, '+')
pub(crate) fn pane_tab_rects(strip: RECT, n: usize, dpi: i32) -> (Vec<RECT>, RECT) {
    let s = |v: i32| v * dpi / 96;
    let (pad, plus) = (s(4), s(24));
    let room = (strip.right - strip.left - 2 * pad - plus).max(0);
    let w = (room / n.max(1) as i32).min(s(170));
    let tab = |k: i32| RECT { left: strip.left + pad + k * w, top: strip.top + s(3), right: strip.left + pad + (k + 1) * w, bottom: strip.bottom };
    let tabs: Vec<RECT> = (0..n as i32).map(tab).collect();
    let x = tabs.last().map_or(strip.left + pad, |r| r.right);
    (tabs, RECT { left: x, top: strip.top + s(3), right: x + plus, bottom: strip.bottom })
}

/// The × on a pane tab, at its right end. Only on tabs wide enough to keep some name.
pub(crate) fn pane_tab_close_rect(tab: RECT, dpi: i32) -> Option<RECT> {
    let s = |v: i32| v * dpi / 96;
    (tab.right - tab.left >= s(64)).then(|| RECT { left: tab.right - s(22), right: tab.right - s(4), ..tab })
}

pub(crate) fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// The search icon or filter chip at the left of the filter pill. The edit starts at 24 DIP, so this stays clickable.
pub(crate) fn filter_icon_rect(filt: RECT, s24: i32) -> Option<RECT> {
    let r = RECT { right: filt.right.min(filt.left + s24), ..filt };
    (r.right > r.left).then_some(r)
}

/// Share of a colour label kept on an inactive pane. The active pane uses the solid preset,
/// the way the accent fills an active header; this mix into the card is the quieter one.
const LABEL_INACTIVE: f32 = 0.55;

pub(crate) struct PaneChrome {
    pub(crate) head: u32,
    pub(crate) ink: u32,
    pub(crate) border: u32,
}

/// Header and border colours.
/// A colour label fills both the way the accent fills an active pane: solid while this pane is
/// active, the same colour mixed into the card while it is not. Ink is `on_color` of that fill.
/// Without a label, the accent fills the active header and, when `edge` is on, the active border.
/// The painter uses 2px for that emphasised border and 1px otherwise.
pub(crate) fn pane_chrome(accent: u32, label: Option<u32>, active: bool, edge: bool, p: &Palette) -> PaneChrome {
    if let Some(c) = label {
        let head = if active { c } else { blend(p.card, c, LABEL_INACTIVE) };
        return PaneChrome { head, ink: on_color(head), border: if active { c } else { head } };
    }
    if active {
        return PaneChrome { head: accent, ink: on_color(accent), border: if edge { accent } else { p.line } };
    }
    PaneChrome { head: p.card, ink: p.text, border: p.line }
}

/// A pane's colour label in the current theme: one of the selection-colour presets. 0 is none.
pub(crate) fn tint_color(k: u8) -> Option<u32> {
    let i = usize::from(k).checked_sub(1)?;
    pal().accents.get(i).map(|a| a.1)
}

pub(crate) fn tint_name(k: u8) -> &'static str {
    let i = (k as usize).saturating_sub(1).min(5);
    crate::accent_name(i)
}

/// A `t` px line down the middle of splitter `d`. None when that splitter is not on screen.
pub(crate) fn gap_line(g: &Geo, d: Hit, t: i32) -> Option<RECT> {
    let vbar = |x: i32| RECT { left: x - t / 2, top: g.top, right: x - t / 2 + t, bottom: g.bot };
    match d {
        Hit::Side if g.side > 0 => Some(vbar(g.side + g.gap / 2)),
        Hit::SideSplit => {
            let gap = side_gap_rect(g)?;
            let y = gap.top + (gap.bottom - gap.top) / 2;
            Some(RECT { left: gap.left, top: y - t / 2, right: gap.right, bottom: y - t / 2 + t })
        }
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
        Hit::Preview if g.preview > 0 => {
            let left = g.w - g.pad - g.preview;
            Some(vbar(left - g.gap / 2))
        }
        _ => None,
    }
}

pub(crate) fn hit(g: &Geo, x: i32, y: i32) -> Option<Hit> {
    if y < g.top || y >= g.bot {
        return None;
    }
    if let Some(gap) = side_gap_rect(g) {
        if x >= gap.left && x < gap.right && y >= gap.top && y < gap.bottom {
            return Some(Hit::SideSplit);
        }
    }
    if g.side > 0 && x >= g.side && x < g.gx {
        return Some(Hit::Side);
    }
    // The preview stays while a pane is zoomed, so its splitter is tested before that early return.
    if g.preview > 0 {
        let left = g.w - g.pad - g.preview;
        if x >= left - g.gap && x < left {
            return Some(Hit::Preview);
        }
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
    app(|a| (a.grid_preview, a.grid_drag, a.grid_preset) = (cur, None, None));
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("MaguroPicker"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.grid = h);
}

pub(crate) unsafe fn paint_grid(h: HWND) {
    let (preview, cur, hot, dpi, font, accent) =
        app(|a| (a.grid_preview.clone(), a.s.heights.clone(), a.grid_preset, a.dpi, a.font, a.s.accent_now()));
    let tokens = ui_tokens();
    let soft = blend(accent, tokens.card, 0.8);
    let g = pick_geo(dpi);
    let on = |v: &[usize], r: usize, c: usize| c < v.len() && r < v[c];
    let rad = px(dpi, RADIUS).max(1);
    buffered(h, |dc, rc| {
        paint_window(dc, rc);
        frame(dc, rc, 1, tokens.line);
        for r in 0..4 {
            for c in 0..4 {
                let cell = pick_cell(&g, r, c);
                if on(&preview, r, c) {
                    aa_rfill(dc, cell, rad, accent, None, tokens.bg);
                } else if on(&cur, r, c) {
                    aa_rfill(dc, cell, rad, soft, None, tokens.bg);
                } else {
                    aa_rfill(dc, cell, rad, tokens.card, Some(tokens.line), tokens.bg);
                }
            }
        }
        let label = if preview.iter().all(|&n| n == preview[0]) {
            tf(crate::Key::GridEven, &[("rows", &preview[0].to_string()), ("cols", &preview.len().to_string())])
        } else {
            format!("{}  {}", tr(crate::Key::PerColumn), layout_label(&preview))
        };
        text(dc, g.label, &label, font, tokens.text, DT_CENTER);
        // preset thumbnails: the panes of each layout on a tile
        for (i, (p, tile_r)) in PRESETS.iter().zip(g.presets).enumerate() {
            let pane = if hot == Some(i) { accent } else { blend(tokens.card, tokens.text, 0.25) };
            if hot == Some(i) {
                aa_rfill(dc, tile_r, rad, soft, Some(tokens.line), tokens.bg);
            } else {
                aa_rfill(dc, tile_r, rad, tokens.card, Some(tokens.line), tokens.bg);
            }
            let inner = RECT { left: tile_r.left + 3, top: tile_r.top + 3, right: tile_r.right - 3, bottom: tile_r.bottom - 3 };
            for (c, (x0, x1)) in cells(inner.left, inner.right - inner.left, &even(p.len()), 2).into_iter().enumerate() {
                for (y0, y1) in cells(inner.top, inner.bottom - inner.top, &even(p[c]), 2) {
                    fill(dc, RECT { left: x0, top: y0, right: x1, bottom: y1 }, pane);
                }
            }
        }
    });
}

pub(crate) extern "system" fn grid_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| grid_proc_inner(h, msg, wp, lp)))
}
fn grid_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        let g = pick_geo(app(|a| a.dpi));
        let show = |v: Vec<usize>, preset: Option<usize>| {
            let changed = app(|a| {
                let changed = a.grid_preview != v || a.grid_preset != preset;
                (a.grid_preview, a.grid_preset) = (v, preset);
                changed
            });
            if changed {
                _ = InvalidateRect(Some(h), None, false);
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
                    show(v, None);
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| a.grid_drag.clone()) {
                    if let Some((r, c)) = pick_at(&g, x, y, true) {
                        let v = drag_heights(d, r, c);
                        app(|a| a.grid_drag = Some(v.clone()));
                        show(v, None);
                    }
                } else if let Some((r, c)) = pick_at(&g, x, y, false) {
                    show(vec![r + 1; c + 1], None);
                } else if let Some(i) = g.presets.iter().position(|t| x >= t.left && x < t.right && y >= t.top && y < t.bottom) {
                    show(PRESETS[i].to_vec(), Some(i));
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
                // keyboard picks an even layout: arrows move the corner, Enter applies
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
                show(vec![r + 1; c + 1], None);
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.grid, a.grid_drag, a.grid_closed) = (HWND::default(), None, Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{blend, on_color};

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn address_icon_slot_is_fixed() {
        use crate::{addr_icon, crumb_field};
        for dpi in [96, 144] {
            let s = |v: i32| v * dpi / 96;
            let narrow = RECT { left: 0, top: 0, right: s(90) - 1, bottom: s(22) };
            let wide = RECT { left: 10, top: 4, right: 10 + s(200), bottom: 4 + s(22) };
            assert!(addr_icon(narrow, dpi).is_none());
            assert_eq!(crumb_field(narrow, dpi).left, narrow.left + s(8));
            let r = addr_icon(wide, dpi).unwrap();
            assert_eq!((r.right - r.left, r.bottom - r.top), (s(16), s(16)));
            assert_eq!(crumb_field(wide, dpi).left, wide.left + s(26));
            assert_eq!(crumb_field(wide, dpi).left, crumb_field(wide, dpi).left);
            let note = crate::net_note_rect(wide, dpi);
            assert_eq!((note.left, note.right, note.top, note.bottom), (wide.left + s(8), wide.right - s(8), wide.top, wide.bottom));
            assert!(note.right - note.left > crumb_field(wide, dpi).right - crumb_field(wide, dpi).left);
        }
    }

    #[test]
    fn theme_follow_switches_only_when_it_differs() {
        use crate::theme_target;
        assert_eq!(theme_target(true, Some(true), false), Some(true));
        assert_eq!(theme_target(true, Some(false), true), Some(false));
        assert_eq!(theme_target(true, Some(true), true), None);
        assert_eq!(theme_target(false, Some(true), false), None);
        assert_eq!(theme_target(true, None, false), None);
    }

    #[test]
    fn gap_lines_mark_real_splitters() {
        let g = Geo {
            w: 1000,
            h: 600,
            top: 34,
            bot: 576,
            pad: 8,
            gap: 8,
            side: 240,
            tree_h: 200,
            fav_top: false,
            gx: 248,
            preview: 0,
            zoom: false,
            xs: vec![(256, 620), (628, 992)],
            ys: vec![vec![(34, 300), (308, 576)], vec![(34, 576)]],
        };
        for d in [Hit::Side, Hit::SideSplit, Hit::Col(0), Hit::Row(0, 0)] {
            let r = gap_line(&g, d, 2).unwrap();
            assert!(r.right > r.left && r.bottom > r.top, "{d:?}");
        }
        for d in [Hit::Col(1), Hit::Col(5), Hit::Row(1, 0), Hit::Row(0, 1), Hit::Row(3, 0)] {
            assert_eq!(gap_line(&g, d, 2), None, "{d:?}");
        }
        let hidden = Geo { side: 0, tree_h: 0, gx: 8, ..g.clone() };
        assert_eq!(gap_line(&hidden, Hit::Side, 2), None);
        assert_eq!(gap_line(&hidden, Hit::SideSplit, 2), None);
        let zoomed = Geo { zoom: true, ..g };
        assert_eq!(gap_line(&zoomed, Hit::Col(0), 2), None);
        assert_eq!(gap_line(&zoomed, Hit::Row(0, 0), 2), None);
        assert!(gap_line(&zoomed, Hit::Side, 2).is_some());
    }

    #[test]
    fn filter_icon_is_the_pill_s_left_edge() {
        let wide = RECT { left: 10, top: 2, right: 160, bottom: 24 };
        let r = filter_icon_rect(wide, 24).unwrap();
        assert_eq!((r.left, r.right), (10, 34));
        assert!(filter_icon_rect(RECT { left: 10, right: 10, ..wide }, 24).is_none());
    }

    fn contrast(a: u32, b: u32) -> f32 {
        let chan = |c: u32| {
            let f = c as f32 / 255.0;
            if f <= 0.04045 { f / 12.92 } else { ((f + 0.055) / 1.055).powf(2.4) }
        };
        let rel = |rgb: u32| 0.2126 * chan(rgb >> 16) + 0.7152 * chan(rgb >> 8 & 0xFF) + 0.0722 * chan(rgb & 0xFF);
        let (x, y) = (rel(a), rel(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn dist(a: u32, b: u32) -> u32 {
        let d = |s: u32| (a >> s & 0xFF).abs_diff(b >> s & 0xFF) as u32;
        d(16) + d(8) + d(0)
    }

    #[test]
    fn label_fills_header_and_border_like_the_accent() {
        let accent = 0x112233u32;
        for p in [&crate::LIGHT, &crate::DARK] {
            let bare_on = pane_chrome(accent, None, true, true, p);
            assert_eq!((bare_on.head, bare_on.ink, bare_on.border), (accent, on_color(accent), accent));
            let bare_thin = pane_chrome(accent, None, true, false, p);
            assert_eq!((bare_thin.head, bare_thin.border), (accent, p.line));
            let bare_off = pane_chrome(accent, None, false, true, p);
            assert_eq!((bare_off.head, bare_off.ink, bare_off.border), (p.card, p.text, p.line));
            for (_, c) in p.accents {
                let on = pane_chrome(accent, Some(c), true, true, p);
                let on_thin = pane_chrome(0xAABBCC, Some(c), true, false, p);
                let off = pane_chrome(accent, Some(c), false, false, p);
                assert_eq!((on.head, on.ink, on.border), (c, on_color(c), c));
                // Thickness follows the active-border setting; the colour does not.
                assert_eq!((on_thin.head, on_thin.border), (c, c));
                let quiet = blend(p.card, c, super::LABEL_INACTIVE);
                assert_eq!((off.head, off.ink, off.border), (quiet, on_color(quiet), quiet));
                assert!(dist(off.head, p.card) < dist(on.head, p.card), "{c:06X}");
                assert!(contrast(on.head, on.ink) >= 4.0, "{c:06X} {}", contrast(on.head, on.ink));
                assert!(contrast(off.head, off.ink) >= contrast(on.head, on.ink), "{c:06X}");
            }
        }
    }

    #[test]
    fn split_keeps_a_minimum_and_defaults_to_even() {
        assert_eq!(split_heights(400, 0.5, 8, 80), (196, 196));
        assert_eq!(split_heights(400, 0.0, 8, 80), (80, 312));
        assert_eq!(split_heights(400, 1.0, 8, 80), (312, 80));
        assert_eq!(split_heights(400, f64::NAN, 8, 80), (196, 196));
        // shorter than two minimums: the plate is shared, and neither part vanishes
        let (top, bot) = split_heights(100, 0.9, 8, 80);
        assert!(top >= 1 && bot >= 1);
        assert_eq!(top + bot, 92);
        assert!(side_min(96) == 80 && side_min(192) == 160);
    }

    #[test]
    fn window_fits_work_area() {
        let r = |left, top, right, bottom| RECT { left, top, right, bottom };
        let work = r(0, 0, 1024, 728);
        assert_eq!(fit(r(100, 50, 900, 650), work), r(100, 50, 900, 650));
        assert_eq!(fit(r(26, 26, 1426, 926), work), work);
        assert_eq!(fit(r(500, 400, 1300, 1000), work), r(224, 128, 1024, 728));
        let left_mon = r(-1920, 0, 0, 1040);
        assert_eq!(fit(r(-2100, -50, -1300, 550), left_mon), r(-1920, 0, -1120, 600));
        assert_eq!(fit(r(-300, 100, 500, 700), left_mon), r(-800, 100, 0, 700));
        let mon = r(0, 0, 1920, 1080);
        assert_eq!(fit_saved(r(1500, 900, 2300, 1500), mon, r(0, 40, 1920, 1080)), r(1120, 440, 1920, 1040));
        let saved = r(100, 100, 900, 700);
        assert_eq!(fit_saved(saved, mon, r(0, 40, 1920, 1080)), saved);
    }

    #[test]
    fn cells_keep_the_pane_minimum_when_the_row_can_hold_it() {
        let raw = cells(10, 208, &[0.1, 0.9], 8);
        assert!(raw[0].1 - raw[0].0 < 40);
        let c = cells_min(10, 208, &[0.1, 0.9], 8, 40);
        assert_eq!(c, vec![(10, 50), (58, 218)]);
        assert!(c.iter().all(|(a, b)| b - a >= 40));
        assert_eq!(c[0].1 + 8, c[1].0);
        // already clear of the floor: the proportional split is kept
        assert_eq!(cells_min(0, 208, &[0.5, 0.5], 8, 40), cells(0, 208, &[0.5, 0.5], 8));
        // too short to honour the floor: share the space, do not run past the end
        let short = cells_min(0, 50, &[0.2, 0.8], 8, 40);
        assert_eq!(short, cells(0, 50, &[0.2, 0.8], 8));
        assert!(short.last().unwrap().1 <= 50);
        let snapped = ratios_matching_min(&[0.1, 0.9], 208, 8, 40).unwrap();
        let again = cells(0, 208, &snapped, 8);
        assert!(again.iter().all(|(a, b)| b - a >= 40));
        assert!(ratios_matching_min(&[0.5, 0.5], 208, 8, 40).is_none());
        assert!(ratios_matching_min(&[0.1, 0.9], 50, 8, 40).is_none());
    }

    #[test]
    fn client_min_is_the_current_layout_plus_chrome() {
        // 2×2, tree, toolbar, status, 96 dpi.
        // side 120 + gap 8 + two panes 280 + gap 8 + pad 8 = 424, taller than the icon row.
        assert_eq!(client_min(&[2, 2], 96, true, true, true, false), (424, 304));
        // one pane, no tree, toolbar hidden, no status: the gear row is wider than the pane.
        assert_eq!(client_min(&[1], 96, false, false, false, false), (160, 152));
        let (w, h) = client_min(&[1, 1, 1], 192, false, true, true, false);
        assert_eq!(client_min(&[2, 2], 96, true, true, true, true), (424 + 8 + 280, 304));
        assert_eq!((w, h), (904, 352));
        assert_eq!((pane_min_w(96), pane_min_h(96), pane_min_w(192), pane_min_h(144)), (140, 120, 280, 180));
    }

    #[test]
    fn growing_keeps_the_corner_and_a_smaller_tab_does_not_shrink() {
        let r = |left, top, right, bottom| RECT { left, top, right, bottom };
        let work = r(0, 0, 1920, 1080);
        let big = r(100, 80, 1400, 900);
        assert_eq!(grow_into_work(big, 400, 300, work), big);
        assert_eq!(grow_into_work(r(100, 80, 500, 400), 800, 300, work), r(100, 80, 900, 400));
        // growth would leave the work area: slide, still keeping as much of the corner as fits
        assert_eq!(grow_into_work(r(1500, 800, 1900, 1000), 900, 700, work), r(1020, 380, 1920, 1080));
        // minimum larger than the monitor: the window becomes the work area
        assert_eq!(grow_into_work(r(100, 50, 400, 300), 4000, 3000, work), work);
        let left_mon = r(-1920, 0, 0, 1080);
        assert_eq!(grow_into_work(r(-100, 50, 300, 350), 500, 300, left_mon), r(-500, 50, 0, 350));
    }

    #[test]
    fn preview_plate_uses_dip_and_keeps_a_pane() {
        assert_eq!(preview_plate(false, 400, 96, 2000, 16, 8, 8), 0);
        assert_eq!(preview_plate(true, 0, 96, 2000, 16, 8, 8), 280);
        assert_eq!(preview_plate(true, 400, 96, 2000, 16, 8, 8), 400);
        assert_eq!(preview_plate(true, 50, 96, 2000, 16, 8, 8), 160);
        // 400 wide, grid at 8, gap 8, pad 8, pane min 140: room is 236, so 280 shrinks.
        assert_eq!(preview_plate(true, 0, 96, 400, 8, 8, 8), 236);
        assert_eq!(preview_plate(true, 200, 192, 2000, 16, 16, 16), 400);
        assert_eq!(preview_dip(280, 96), 280);
        assert_eq!(preview_dip(400, 192), 200);
        assert_eq!(preview_dip(10, 96), 160);
    }

    #[test]
    fn preview_splitter_is_the_gap_left_of_the_plate() {
        let g = Geo {
            w: 1000,
            h: 800,
            top: 66,
            bot: 700,
            pad: 8,
            gap: 8,
            side: 0,
            tree_h: 0,
            fav_top: false,
            gx: 8,
            preview: 280,
            xs: vec![(8, 704)],
            ys: vec![vec![(66, 700)]],
            zoom: false,
        };
        // plate left = 1000 - 8 - 280 = 712. Gap is [704, 712).
        assert_eq!(g.grid_right(), 704);
        assert_eq!(hit(&g, 706, 400), Some(Hit::Preview));
        assert_eq!(hit(&g, 720, 400), None);
        assert_eq!(hit(&g, 700, 400), None);
        let zoomed = Geo { zoom: true, ..g.clone() };
        assert_eq!(hit(&zoomed, 706, 400), Some(Hit::Preview));
        assert!(gap_line(&zoomed, Hit::Preview, 2).is_some());
        let hidden = Geo { preview: 0, ..g };
        assert_eq!(hit(&hidden, 706, 400), None);
        assert_eq!(gap_line(&hidden, Hit::Preview, 2), None);
    }

    #[test]
    fn hidden_panel_keeps_its_width() {
        // Hidden, then a column drag: the clamped 0 must not replace 600.
        assert_eq!(kept_side(false, 600, 0), 600);
        assert_eq!(kept_side(true, 600, 536), 536);
    }

    #[test]
    fn drag_ratio_stops_at_the_minimum_and_double_click_is_even() {
        assert!(near(split_ratio_at(0, 0, 400, 8, 80), 80.0 / 392.0));
        assert!(near(split_ratio_at(400, 0, 400, 8, 80), 312.0 / 392.0));
        assert!(near(split_ratio_at(196, 0, 400, 8, 80), 196.0 / 392.0));
        let again = split_heights(400, split_ratio_at(50, 0, 400, 8, 80), 8, 80);
        assert_eq!(again.0, 80);
    }

    #[test]
    fn side_split_hit_is_the_gap_inside_the_plate() {
        let g = Geo {
            w: 1000,
            h: 800,
            top: 66,
            bot: 700,
            pad: 8,
            gap: 8,
            side: 240,
            tree_h: 300,
            fav_top: false,
            gx: 248,
            preview: 0,
            xs: vec![(248, 600), (608, 992)],
            ys: vec![vec![(66, 700)], vec![(66, 700)]],
            zoom: false,
        };
        assert_eq!(hit(&g, 100, 66 + 300 + 3), Some(Hit::SideSplit));
        assert_eq!(hit(&g, 100, 66 + 200), None);
        assert_eq!(hit(&g, 244, 66 + 300 + 3), Some(Hit::Side));
        let closed = Geo { side: 0, tree_h: 0, gx: 8, ..g.clone() };
        assert_eq!(hit(&closed, 4, 400), None);
        let line = gap_line(&g, Hit::SideSplit, 2).unwrap();
        let gap = side_gap_rect(&g).unwrap();
        assert!(line.top >= gap.top && line.bottom <= gap.bottom);
    }

    fn plate(fav_top: bool, tree_h: i32) -> Geo {
        Geo {
            w: 1000,
            h: 800,
            top: 66,
            bot: 700,
            pad: 8,
            gap: 8,
            side: 240,
            tree_h,
            fav_top,
            gx: 248,
            preview: 0,
            xs: vec![(248, 600)],
            ys: vec![vec![(66, 700)]],
            zoom: false,
        }
    }

    #[test]
    fn both_orders_place_the_divider_and_keep_the_tree_share() {
        let (height, gap, min) = (400, 8, 80);
        for fav_top in [false, true] {
            for pos in [0, 50, 196, 250, 400] {
                let share = tree_share_at(pos, 0, height, gap, min, fav_top);
                let (tree, fav) = split_heights(height, share, gap, min);
                let top_h = if fav_top { fav } else { tree };
                let again = tree_share_at(top_h, 0, height, gap, min, fav_top);
                assert!((share - again).abs() < 1e-9, "{fav_top} {pos} {share} {again}");
                assert!(tree >= 1 && fav >= 1, "{tree} {fav}");
            }
        }
        // Dragging onto the top edge leaves the upper half at the minimum.
        assert!((tree_share_at(0, 0, height, gap, min, false) - 80.0 / 392.0).abs() < 1e-9);
        assert!((tree_share_at(0, 0, height, gap, min, true) - 312.0 / 392.0).abs() < 1e-9);
        assert!((tree_share_at(400, 0, height, gap, min, true) - 80.0 / 392.0).abs() < 1e-9);
        // An even split stays even after the halves trade places.
        assert_eq!(split_heights(height, 0.5, gap, min), (196, 196));

        let tree_on_top = plate(false, 300);
        let fav_on_top = plate(true, 300);
        assert_eq!(tree_on_top.side_top_h(), 300);
        assert_eq!(fav_on_top.side_top_h(), 634 - 8 - 300);
        assert_eq!(hit(&tree_on_top, 100, 66 + 300 + 3), Some(Hit::SideSplit));
        assert_eq!(hit(&fav_on_top, 100, 66 + 300 + 3), None);
        let fav_gap = side_gap_rect(&fav_on_top).unwrap();
        assert_eq!(hit(&fav_on_top, 100, fav_gap.top + 3), Some(Hit::SideSplit));
        assert_eq!(hit(&fav_on_top, 100, fav_gap.bottom + 4), None);
        assert_eq!(hit(&fav_on_top, 244, fav_gap.top + 3), Some(Hit::Side));
        let (tree, fav) = side_panes(&fav_on_top, 2);
        assert!(fav.top < tree.top, "favourites sit above the tree");
        assert_eq!(tree.top - fav.bottom, 8);
        assert_eq!((fav.left, fav.right, tree.left, tree.right), (2, 238, 2, 238));
        let (tree, fav) = side_panes(&tree_on_top, 2);
        assert!(tree.top < fav.top);
        assert_eq!(fav.top - tree.bottom, 8);
        for g in [&tree_on_top, &fav_on_top] {
            let gap = side_gap_rect(g).unwrap();
            let line = gap_line(g, Hit::SideSplit, 2).unwrap();
            assert_eq!((line.left, line.right), (gap.left, gap.right));
            assert!(line.top >= gap.top && line.bottom <= gap.bottom, "{line:?} {gap:?}");
            assert_eq!(line.bottom - line.top, 2);
        }
        assert!(side_gap_rect(&Geo { side: 0, tree_h: 0, ..fav_on_top.clone() }).is_none());
        assert_eq!(gap_line(&Geo { side: 0, tree_h: 0, ..fav_on_top }, Hit::SideSplit, 2), None);
    }

    #[test]
    fn remap_keeps_cells() {
        let ident: [usize; MAXP] = std::array::from_fn(|k| k);
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
    }

    #[test]
    fn pane_tab_strip_shares_the_row() {
        let strip = RECT { left: 2, top: 30, right: 402, bottom: 56 };
        let (tabs, plus) = pane_tab_rects(strip, 2, 96);
        assert_eq!((tabs[0].left, tabs[0].right - tabs[0].left, tabs[1].right, plus.left, plus.right), (6, 170, 346, 346, 370));
        let (tabs, plus) = pane_tab_rects(strip, 10, 96); // squeezed evenly, '+' still inside
        assert!(tabs.iter().all(|r| r.right - r.left == 36) && plus.right <= strip.right);
        assert!(pane_tab_close_rect(tabs[0], 96).is_none());
        assert!(pane_tab_close_rect(RECT { left: 0, top: 0, right: 64, bottom: 20 }, 96).is_some());
        let pg = PaneGeo {
            num: RECT::default(),
            btn: [RECT::default(); 4],
            addr: RECT::default(),
            filt: RECT::default(),
            fav: RECT::default(),
            add: RECT::default(),
            tabs: strip,
            view: RECT::default(),
        };
        let (wide, _) = pane_tab_rects(strip, 2, 96);
        assert!(matches!(crate::pane_tab_hit(&pg, 2, &[false, false], 6, 40, 96), Some(crate::PaneTabHit::Tab(0))));
        assert!(matches!(crate::pane_tab_hit(&pg, 2, &[false, false], 350, 40, 96), Some(crate::PaneTabHit::Plus)));
        assert!(crate::pane_tab_hit(&pg, 2, &[false, false], 1, 40, 96).is_none());
        let close = pane_tab_close_rect(wide[0], 96).unwrap();
        assert!(matches!(crate::pane_tab_hit(&pg, 2, &[false, false], close.left + 2, 40, 96), Some(crate::PaneTabHit::Close(0))));
        assert!(matches!(crate::pane_tab_hit(&pg, 2, &[true, false], close.left + 2, 40, 96), Some(crate::PaneTabHit::Lock(0))));
        assert!(matches!(crate::pane_tab_hit(&pg, 1, &[true], close.left + 2, 40, 96), Some(crate::PaneTabHit::Lock(0))));
        assert!(matches!(crate::pane_tab_hit(&pg, 1, &[false], close.left + 2, 40, 96), Some(crate::PaneTabHit::Tab(0))));
    }

    #[test]
    fn one_tab_hides_the_strip_and_keeps_an_add_button() {
        let one = pane_geo(800, 500, 96, false);
        let many = pane_geo(800, 500, 96, true);
        assert_eq!(one.tabs.bottom, one.tabs.top);
        assert_eq!(many.tabs.bottom - many.tabs.top, 26);
        assert_eq!(many.view.top - one.view.top, 26);
        assert_eq!(one.view.bottom, many.view.bottom);
        assert_eq!(one.add.right - one.add.left, 24);
        assert_eq!(many.add.left, many.add.right);
        assert_eq!(one.btn[3].right + 2, one.add.left);
        assert_eq!(one.add.right + 2, one.filt.left);
        assert_eq!(many.btn[3].right + 2, many.filt.left);
        assert!(one.addr.right < many.addr.right);
        // The number is reserved first and stays either way.
        for strip in [false, true] {
            let narrow = pane_geo(80, 400, 96, strip);
            assert_eq!(narrow.num.right - narrow.num.left, 18, "strip={strip}");
        }
        for (dpi, strip) in [(96, false), (144, false), (96, true), (144, true)] {
            let s = |v: i32| v * dpi / 96;
            for w in [400, 800, 1200] {
                let p = pane_geo(w, 400, dpi, strip);
                let row = [p.num, p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.add, p.filt, p.fav];
                assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi} strip={strip}");
                assert!(p.num.right - p.num.left == s(18) && p.fav.right <= w && p.view.top >= p.btn[0].bottom, "{w}@{dpi}");
            }
            for w in 60..=700 {
                let p = pane_geo(w, 400, dpi, strip);
                assert!(p.fav.right <= w && p.view.top >= p.btn[0].bottom, "{w}@{dpi}");
                assert!(p.num.right > p.num.left && p.num.right <= w, "{w}@{dpi}");
                let live: Vec<RECT> = [p.btn[0], p.btn[1], p.btn[2], p.btn[3], p.addr, p.add, p.filt, p.fav]
                    .into_iter()
                    .filter(|c| c.right > c.left)
                    .collect();
                for c in &live {
                    assert!(c.left >= p.num.right, "covers number {w}@{dpi} strip={strip}");
                }
                for (i, a) in live.iter().enumerate() {
                    for b in &live[i + 1..] {
                        assert!(a.right <= b.left || b.right <= a.left, "overlap {w}@{dpi} strip={strip}");
                    }
                }
                if strip {
                    assert_eq!((p.add.right, p.tabs.bottom - p.tabs.top), (p.add.left, s(26)));
                    assert_eq!(p.view.top, p.tabs.bottom);
                } else {
                    let add_w = p.add.right - p.add.left;
                    assert!(add_w == s(24) || add_w == 0, "{w}@{dpi}");
                    assert_eq!((p.tabs.bottom, p.view.top), (p.tabs.top, p.tabs.top));
                    if p.btn[3].left >= p.btn[2].right && p.btn[3].right > p.btn[3].left && p.add.right > p.add.left {
                        assert!(p.btn[3].right <= p.add.left && p.add.right <= p.filt.left, "{w}@{dpi}");
                    }
                }
            }
        }
    }

    #[test]
    fn pane_number_is_reserved_before_the_address() {
        // Wide pane: the number leads, and the address edit still keeps its s(32).
        let wide = pane_geo(800, 500, 96, true);
        assert_eq!((wide.num.left, wide.num.right), (4, 22));
        assert_eq!((wide.btn[0].left, wide.btn[2].right, wide.addr.left, wide.addr.right), (22, 94, 98, 590));
        assert_eq!((wide.btn[3].left, wide.filt.left, wide.filt.right, wide.fav.left), (594, 620, 770, 772));
        // Just below the old cutoff the number used to vanish. It stays, and the address keeps a folder name.
        let above = pane_geo(241, 500, 96, true);
        let below = pane_geo(240, 500, 96, true);
        let edit = |p: &PaneGeo| (p.addr.right - 6) - (p.addr.left + 8);
        assert_eq!((above.num.left, above.num.right), (4, 22));
        assert!(edit(&above) >= 66 && edit(&below) >= 66);
        assert_eq!((below.num.left, below.num.right), (4, 22));
        assert_eq!((below.btn[0].left, below.addr.left), (above.btn[0].left, above.addr.left));
        assert!(below.btn.iter().all(|r| r.right - r.left == 24) && below.fav.right - below.fav.left == 24);
        // A ~20% column at 200% DPI (1x3). The label is the whole grab target.
        let col = pane_geo(384, 500, 192, true);
        assert_eq!(col.num.right - col.num.left, 36);
        assert!(col.addr.right - col.addr.left >= 160);
        for c in [col.btn[0], col.btn[1], col.btn[2], col.btn[3], col.addr, col.filt, col.fav] {
            if c.right > c.left {
                assert!(c.left >= col.num.right);
            }
        }
    }

    #[test]
    fn pane_number_stays_visible_at_the_minimum_width() {
        for &(dpi, strip) in &[(96, false), (96, true), (192, false), (192, true)] {
            let s = |v: i32| v * dpi / 96;
            let w = s(140);
            let p = pane_geo(w, s(120), dpi, strip);
            assert_eq!(p.num.right - p.num.left, s(18), "{dpi} strip={strip}");
            assert!(p.num.left >= 0 && p.num.right <= w, "{dpi} strip={strip}");
            for c in [p.btn[0], p.btn[1], p.btn[2], p.btn[3], p.addr, p.add, p.filt, p.fav] {
                if c.right > c.left {
                    assert!(c.left >= p.num.right, "overlaps the number {dpi} strip={strip} {:?}", c);
                }
            }
            // The pill stays, wide enough that the crumb field can draw the folder name.
            let addr_w = p.addr.right - p.addr.left;
            assert!(addr_w >= s(80), "address dropped {dpi} strip={strip} w={addr_w}");
            let field = (p.addr.right - s(6)) - (p.addr.left + s(8));
            assert!(field >= s(16), "folder name has no room {dpi} strip={strip}");
            // Back, forward, up, ⇄ and + give way before the address does.
            assert!(p.btn.iter().all(|r| r.right == r.left), "nav still up {dpi} strip={strip}");
            assert_eq!(p.add.left, p.add.right, "add still up {dpi} strip={strip}");
        }
    }

    fn perm_of(p: &Placed) -> bool {
        let mut seen = [false; MAXP];
        p.perm.iter().all(|&i| i < MAXP && !std::mem::replace(&mut seen[i], true)) && seen.iter().all(|&s| s)
    }

    #[test]
    fn split_right_inserts_a_full_height_column_after_this_one() {
        let g = split_right_grid(&[2, 1], 0).unwrap();
        assert_eq!(g.heights, vec![2, 1, 1]);
        assert_eq!(g.slots, vec![Some(0), Some(1), None, Some(2)]);
        let g = split_right_grid(&[2, 1], 2).unwrap();
        assert_eq!(g.heights, vec![2, 1, 1]);
        assert_eq!(g.slots, vec![Some(0), Some(1), Some(2), None]);
        let g = split_right_grid(&[1, 2, 1], 0).unwrap();
        assert_eq!(g.heights, vec![1, 1, 2, 1]);
        assert_eq!(g.slots, vec![Some(0), None, Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn split_right_stops_at_four_columns() {
        assert!(split_right_grid(&[1, 1, 1, 1], 0).is_none());
        assert!(split_right_grid(&[1, 1, 1, 1], 3).is_none());
        assert!(split_right_grid(&[4, 4, 4, 4], 0).is_none());
        assert!(split_right_grid(&[1], 5).is_none());
        assert!(split_right_grid(&[], 0).is_none());
    }

    #[test]
    fn split_below_inserts_a_row_under_the_pane() {
        let g = split_below_grid(&[2, 2], 0).unwrap();
        assert_eq!(g.heights, vec![3, 2]);
        assert_eq!(g.slots, vec![Some(0), None, Some(1), Some(2), Some(3)]);
        let g = split_below_grid(&[2, 2], 1).unwrap();
        assert_eq!(g.heights, vec![3, 2]);
        assert_eq!(g.slots, vec![Some(0), Some(1), None, Some(2), Some(3)]);
        let g = split_below_grid(&[2, 2], 2).unwrap();
        assert_eq!(g.heights, vec![2, 3]);
        assert_eq!(g.slots, vec![Some(0), Some(1), Some(2), None, Some(3)]);
        let g = split_below_grid(&[1, 4], 0).unwrap();
        assert_eq!(g.heights, vec![2, 4]);
    }

    #[test]
    fn split_below_stops_at_four_rows_in_that_column() {
        assert!(split_below_grid(&[4], 0).is_none());
        assert!(split_below_grid(&[4], 3).is_none());
        assert!(split_below_grid(&[1, 4], 1).is_none());
        assert!(split_below_grid(&[4, 4, 4, 4], 15).is_none());
        let g = split_below_grid(&[3, 3, 3, 3], 0).unwrap();
        assert_eq!(g.heights, vec![4, 3, 3, 3]);
        assert!(split_right_grid(&[3, 3, 3, 3], 0).is_none());
    }

    #[test]
    fn splits_stay_inside_four_by_four() {
        for cols in 1..=4 {
            for rows in 1..=4 {
                let h = vec![rows; cols];
                if cols == 4 {
                    assert!(split_right_grid(&h, 0).is_none());
                } else {
                    let g = split_right_grid(&h, 0).unwrap();
                    assert_eq!(g.heights.len(), cols + 1);
                    assert_eq!(g.heights[0], rows);
                    assert_eq!(g.heights[1], 1);
                    assert!(g.heights.iter().all(|&n| n <= 4));
                    assert!(g.heights.iter().sum::<usize>() <= 16);
                    assert_eq!(g.slots.iter().filter(|s| s.is_none()).count(), 1);
                }
                if rows == 4 {
                    assert!(split_below_grid(&h, 0).is_none());
                } else {
                    let g = split_below_grid(&h, 0).unwrap();
                    assert_eq!(g.heights[0], rows + 1);
                    assert!(g.heights.iter().all(|&n| n <= 4));
                    assert!(g.heights.iter().sum::<usize>() <= 16);
                }
            }
        }
    }

    #[test]
    fn close_drops_a_row_or_the_whole_column() {
        let g = close_pane_grid(&[3, 2], 1).unwrap();
        assert_eq!(g.heights, vec![2, 2]);
        assert_eq!(g.slots, vec![Some(0), Some(2), Some(3), Some(4)]);
        let g = close_pane_grid(&[1, 2, 1], 0).unwrap();
        assert_eq!(g.heights, vec![2, 1]);
        assert_eq!(g.slots, vec![Some(1), Some(2), Some(3)]);
        let g = close_pane_grid(&[2, 1], 2).unwrap();
        assert_eq!(g.heights, vec![2]);
        assert_eq!(g.slots, vec![Some(0), Some(1)]);
        let g = close_pane_grid(&[2, 1, 2], 2).unwrap();
        assert_eq!(g.heights, vec![2, 2]);
        assert_eq!(g.slots, vec![Some(0), Some(1), Some(3), Some(4)]);
        assert!(close_pane_grid(&[1], 0).is_none());
        assert!(close_pane_grid(&[2, 2], 9).is_none());
    }

    #[test]
    fn close_neighbour_is_below_then_above_then_beside() {
        assert_eq!(close_neighbour(&[3], 0), Some(1));
        assert_eq!(close_neighbour(&[3], 1), Some(2));
        assert_eq!(close_neighbour(&[3], 2), Some(1));
        assert_eq!(close_neighbour(&[1, 2], 0), Some(1));
        assert_eq!(close_neighbour(&[2, 1], 2), Some(0));
        assert_eq!(close_neighbour(&[3, 1], 3), Some(0));
        assert_eq!(close_neighbour(&[1], 0), None);
    }

    #[test]
    fn place_grid_keeps_a_permutation_and_forgets_a_closed_pane() {
        let home = [None; MAXP];
        let change = split_right_grid(&[2, 1], 0).unwrap();
        let placed = place_grid(&change.slots, None, &home).unwrap();
        assert!(perm_of(&placed));
        assert_eq!(placed.fresh_old, Some(3));
        let fresh_new = change.slots.iter().position(|s| s.is_none()).unwrap();
        assert_eq!(placed.perm[fresh_new], 3);
        assert!(placed.home.iter().all(|h| h.is_none()));

        let change = close_pane_grid(&[1, 1], 0).unwrap();
        let mut parked = [None; MAXP];
        parked[0] = Some((0, 0));
        let placed = place_grid(&change.slots, Some(0), &parked).unwrap();
        assert!(perm_of(&placed));
        assert_eq!(placed.perm[0], 1);
        let at = placed.perm.iter().position(|&p| p == 0).unwrap();
        assert!(at >= 1);
        assert!(placed.home[at].is_none());
        assert!(placed.fresh_old.is_none());
    }
}

