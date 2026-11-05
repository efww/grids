//! Tab list: every window tab, its layout, and the folder open in each pane.
//! Each tab is one row until it is opened: the representative folder and a `+N` count.
//! The current tab starts open. A filter opens only the tabs that match.
//! A pane with several pane tabs lists those folders indented under the pane.
//! Folder chips use the same tracking tooltip as tab paths (`tip_enter`, below the chip, 4px gap).

use std::collections::BTreeSet;
use std::mem::size_of;

use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;

use crate::app::*;
use crate::i18n::*;
use crate::imports::*;
use crate::pane::*;
use crate::session::*;
use crate::ui::*;

const CHIP_TIP: usize = 11;
const ID_EDIT: i32 = 1;
const ID_NAME: i32 = 2;
const GO: u8 = 1;
const CLOSE_TAB: u8 = 2;
const DISMISS: u8 = 3;
const EM_SETCUEBANNER: u32 = 0x1501;
const EN_CHANGE: u32 = 0x0300;

#[derive(Clone)]
struct Leaf {
    /// Index in the pane's tab list. Empty folders are skipped, so this is not the row position.
    index: usize,
    name: String,
    tip: String,
    current: bool,
}

#[derive(Clone)]
struct Chip {
    pane: usize,
    name: String,
    tip: String,
    /// Header chip. One folder shows its path. Several folders show the pane number and the current name.
    head: String,
    tint: u8,
    tabs: Vec<Leaf>,
}

#[derive(Clone)]
struct Row {
    index: usize,
    name: String,
    locked: bool,
    color: Option<u32>,
    heights: Vec<usize>,
    grid: String,
    panes: usize,
    chips: Vec<Chip>,
    /// Active pane. Its current folder is the row's representative folder.
    active: usize,
    /// Full nested folders. False is one summary chip plus [`Row::extra`].
    open: bool,
    /// Other pane folders hidden by the summary chip. Zero when the tab has one folder.
    extra: usize,
    current: bool,
}

struct Btn {
    id: u8,
    rect: RECT,
    on: bool,
}

#[derive(Clone)]
struct RowBox {
    tab: usize,
    y: i32,
    h: i32,
}

#[derive(Clone)]
struct ChipBox {
    tab: usize,
    pane: usize,
    /// None is the pane header. Some is that pane tab's index in the pane's list.
    sub: Option<usize>,
    rect: RECT,
}

/// `+N` beside a collapsed tab's representative folder.
#[derive(Clone)]
struct BadgeBox {
    tab: usize,
    rect: RECT,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Arm {
    tab: usize,
    pane: usize,
    sub: Option<usize>,
}

struct State {
    hwnd: HWND,
    owner: HWND,
    edit: HWND,
    /// Inline rename of the selected row. Hidden until F2.
    name_edit: HWND,
    query: String,
    sel: usize,
    /// Tab index whose name cell is the edit box. None when not renaming.
    renaming: Option<usize>,
    /// Stored `Tab::name` when the edit opened, so a moved tab is still the one that was renamed.
    rename_prev: String,
    /// Label placed in the box. Unchanged text is not written, so an automatic name stays automatic.
    rename_shown: String,
    /// Esc hides the box; the kill-focus that follows must not also save.
    rename_skip: bool,
    scroll: i32,
    hot_row: Option<usize>,
    track: Tracker<u8>,
    chip_tip: bool,
    dpi: i32,
    hint: RECT,
    search: RECT,
    chord: RECT,
    frame: RECT,
    head: RECT,
    list: RECT,
    status: RECT,
    buttons: Vec<Btn>,
    shown: Vec<Row>,
    rows: Vec<RowBox>,
    chips: Vec<ChipBox>,
    /// Header or pane-tab pressed. Released on the same target opens it.
    arm: Option<Arm>,
    /// Tabs the user opened. The current tab is open without being listed here.
    expanded: BTreeSet<usize>,
    /// Tabs the user shut, including a collapsed current tab. Wins over [`State::expanded`].
    collapsed: BTreeSet<usize>,
    badges: Vec<BadgeBox>,
    content_h: i32,
}

impl State {
    fn new() -> Self {
        Self {
            hwnd: HWND::default(),
            owner: HWND::default(),
            edit: HWND::default(),
            name_edit: HWND::default(),
            query: String::new(),
            sel: 0,
            renaming: None,
            rename_prev: String::new(),
            rename_shown: String::new(),
            rename_skip: false,
            scroll: 0,
            hot_row: None,
            track: Tracker::new(),
            chip_tip: false,
            dpi: 96,
            hint: RECT::default(),
            search: RECT::default(),
            chord: RECT::default(),
            frame: RECT::default(),
            head: RECT::default(),
            list: RECT::default(),
            status: RECT::default(),
            buttons: Vec::new(),
            shown: Vec::new(),
            rows: Vec::new(),
            chips: Vec::new(),
            arm: None,
            expanded: BTreeSet::new(),
            collapsed: BTreeSet::new(),
            badges: Vec::new(),
            content_h: 0,
        }
    }
}

thread_local! {
    static ST: std::cell::RefCell<State> = std::cell::RefCell::new(State::new());
}

fn with_st<R>(f: impl FnOnce(&mut State) -> R) -> R {
    ST.with(|s| f(&mut s.borrow_mut()))
}

fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi.max(96) / 96
}

/// List row ground. Selection is the shared mist. Hover is a light text wash on the card.
fn row_fill(selected: bool, hot: bool, dark: bool) -> u32 {
    let t = tokens_for(dark);
    if selected {
        t.sel_mist()
    } else if hot {
        blend(t.card, t.text, 0.04)
    } else {
        t.card
    }
}

/// Footer and column titles. The app meta face, or the body face before it exists.
unsafe fn meta_font() -> HFONT {
    if let Some(font) = crate::app_try(|a| a.small_font) {
        if !font.0.is_null() && !font.is_invalid() {
            return font;
        }
    }
    body_font()
}

fn button_width(text_w: i32, dpi: i32) -> i32 {
    (text_w + px(dpi, BTN_PAD_X) * 2).max(px(dpi, BTN_MIN_W))
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// Columns × rows when every column is the same height (`[2, 2]` is `2×2`). Uneven layouts keep the column heights.
pub(crate) fn grid_label(heights: &[usize]) -> String {
    let Some(&first) = heights.first() else { return String::new() };
    if heights.iter().all(|&n| n == first) {
        format!("{}×{}", heights.len(), first)
    } else {
        heights.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" | ")
    }
}

/// Folder name on a chip. This PC uses the same label as a tab, not the shell CLSID.
pub(crate) fn chip_name(path: &str) -> String {
    if path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        return tr(Key::ThisPc).into();
    }
    folder_display_name(path)
}

/// Path when `tip` is one, otherwise the display name. This PC stays the localized name.
pub(crate) fn path_text<'a>(name: &'a str, tip: &'a str) -> &'a str {
    if tip.contains('\\') || tip.contains('/') { tip } else { name }
}

/// Tooltip for a chip: the pane's full path. This PC is the localized name. An empty path shows nothing.
pub(crate) fn chip_tip(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    if path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        return tr(Key::ThisPc).into();
    }
    path.to_string()
}

/// Case-insensitive match on the tab name, a folder's display name, or its full path.
pub(crate) fn row_matches(name: &str, chips: &[(&str, &str)], q: &str) -> bool {
    let q = q.trim();
    if q.is_empty() {
        return true;
    }
    let q = q.to_lowercase();
    let has = |s: &str| s.to_lowercase().contains(&q);
    has(name) || chips.iter().any(|(n, p)| has(n) || has(p))
}

/// One window tab reduced to the strings the filter and the row count need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TabFold {
    pub index: usize,
    pub name: String,
    /// Display name and full path of each pane folder. A pane tab is its own folder.
    pub folders: Vec<(String, String)>,
}

/// What the list draws for one window tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FoldRow {
    /// One row: the representative folder, plus `extra` other pane folders.
    Collapsed { tab: usize, extra: usize },
    Open { tab: usize },
}

/// A pane folder the representative picker can see. `current` is that pane's open folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RepFolder {
    pub pane: usize,
    pub current: bool,
}

/// One painted piece of a label. `hit` is a case-insensitive match of the filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MarkedRun {
    pub text: String,
    pub hit: bool,
}

pub(crate) fn fold_index(row: &FoldRow) -> usize {
    match *row {
        FoldRow::Collapsed { tab, .. } | FoldRow::Open { tab } => tab,
    }
}

fn fold_matches(tab: &TabFold, q: &str) -> bool {
    let chips: Vec<(&str, &str)> = tab.folders.iter().map(|(n, p)| (n.as_str(), p.as_str())).collect();
    row_matches(&tab.name, &chips, q)
}

/// Open when a filter is showing this tab and the user has not shut it,
/// when the user expanded it, or when it is the current tab and still on the default.
/// `collapsed` wins, so Left can shut the current tab and a match.
pub(crate) fn tab_is_open(index: usize, expanded: &BTreeSet<usize>, collapsed: &BTreeSet<usize>, filtering: bool, current: usize) -> bool {
    if collapsed.contains(&index) {
        return false;
    }
    filtering || expanded.contains(&index) || index == current
}

/// Visible tabs, in order. A filter drops misses and opens each match unless the user shut it.
/// With no filter, only `expanded` and the current tab are open. The rest are one summary row.
pub(crate) fn visible_fold_rows(tabs: &[TabFold], expanded: &BTreeSet<usize>, collapsed: &BTreeSet<usize>, filter: &str, current: usize) -> Vec<FoldRow> {
    let filtering = !filter.trim().is_empty();
    let mut out = Vec::new();
    for tab in tabs {
        if filtering && !fold_matches(tab, filter) {
            continue;
        }
        if tab_is_open(tab.index, expanded, collapsed, filtering, current) {
            out.push(FoldRow::Open { tab: tab.index });
        } else {
            out.push(FoldRow::Collapsed { tab: tab.index, extra: tab.folders.len().saturating_sub(1) });
        }
    }
    out
}

/// Typing a filter opens the matches again. Collapses of other tabs stay.
pub(crate) fn reveal_matches(tabs: &[TabFold], collapsed: &mut BTreeSet<usize>, filter: &str) {
    if filter.trim().is_empty() {
        return;
    }
    let hit: Vec<usize> = tabs.iter().filter(|t| fold_matches(t, filter)).map(|t| t.index).collect();
    for i in hit {
        collapsed.remove(&i);
    }
}

/// `+2` for two other pane folders. No badge when there is nothing else to show.
pub(crate) fn extra_label(extra: usize) -> Option<String> {
    if extra == 0 { None } else { Some(format!("+{extra}")) }
}

/// The active pane's current folder. The first folder when that pane has none.
pub(crate) fn representative_index(folders: &[RepFolder], active_pane: usize) -> usize {
    if let Some(i) = folders.iter().position(|f| f.pane == active_pane && f.current) {
        return i;
    }
    if let Some(i) = folders.iter().position(|f| f.pane == active_pane) {
        return i;
    }
    0
}

pub(crate) fn set_fold(index: usize, open: bool, expanded: &mut BTreeSet<usize>, collapsed: &mut BTreeSet<usize>) {
    if open {
        collapsed.remove(&index);
        expanded.insert(index);
    } else {
        expanded.remove(&index);
        collapsed.insert(index);
    }
}

pub(crate) fn toggle_fold(index: usize, open_now: bool, expanded: &mut BTreeSet<usize>, collapsed: &mut BTreeSet<usize>) {
    set_fold(index, !open_now, expanded, collapsed);
}

/// Drop `closed` and move every later tab down one, matching the session after a close.
pub(crate) fn shift_fold_sets(expanded: &mut BTreeSet<usize>, collapsed: &mut BTreeSet<usize>, closed: usize) {
    *expanded = shift_fold_index(expanded, closed);
    *collapsed = shift_fold_index(collapsed, closed);
}

fn shift_fold_index(set: &BTreeSet<usize>, closed: usize) -> BTreeSet<usize> {
    set.iter().filter_map(|&i| if i == closed { None } else if i > closed { Some(i - 1) } else { Some(i) }).collect()
}

/// Case-insensitive pieces of `text`. The letters stay as they were written.
pub(crate) fn mark_runs(text: &str, query: &str) -> Vec<MarkedRun> {
    let query = query.trim();
    if text.is_empty() {
        return Vec::new();
    }
    if query.is_empty() {
        return vec![MarkedRun { text: text.to_string(), hit: false }];
    }
    let spans = match_spans(text, query);
    if spans.is_empty() {
        return vec![MarkedRun { text: text.to_string(), hit: false }];
    }
    let mut out = Vec::new();
    let mut at = 0usize;
    for (a, b) in spans {
        if a > at {
            out.push(MarkedRun { text: text[at..a].to_string(), hit: false });
        }
        if b > a {
            out.push(MarkedRun { text: text[a..b].to_string(), hit: true });
        }
        at = b;
    }
    if at < text.len() {
        out.push(MarkedRun { text: text[at..].to_string(), hit: false });
    }
    out
}

/// Byte ranges of `query` in `text`. A character that lowercases to several characters still matches.
fn match_spans(text: &str, query: &str) -> Vec<(usize, usize)> {
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut low = Vec::new();
    let mut src = Vec::new();
    for (ci, (_, c)) in chars.iter().enumerate() {
        let mut n = 0;
        for lc in c.to_lowercase() {
            low.push(lc);
            src.push(ci);
            n += 1;
        }
        if n == 0 {
            low.push(*c);
            src.push(ci);
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + q.len() <= low.len() {
        if low[i..i + q.len()] == q[..] {
            let a = chars[src[i]].0;
            let last = src[i + q.len() - 1];
            let b = chars.get(last + 1).map(|(byte, _)| *byte).unwrap_or(text.len());
            out.push((a, b));
            i += q.len();
        } else {
            i += 1;
        }
    }
    out
}

fn prefix_fit(text: &str, room: i32, measure: &impl Fn(&str) -> i32) -> String {
    let mut acc = String::new();
    if room <= 0 {
        return acc;
    }
    for c in text.chars() {
        let mut next = acc.clone();
        next.push(c);
        if measure(&next) > room {
            break;
        }
        acc = next;
    }
    acc
}

/// Keep runs that fit. The run that overflows loses its tail to `…`.
pub(crate) fn fit_marked_end(runs: &[MarkedRun], avail: i32, measure: impl Fn(&str) -> i32) -> Vec<MarkedRun> {
    if avail <= 0 {
        return Vec::new();
    }
    let total: i32 = runs.iter().map(|r| measure(&r.text).max(0)).sum();
    if total <= avail {
        return runs.to_vec();
    }
    let ell = "…";
    let ell_w = measure(ell).max(0);
    let mut out = Vec::new();
    let mut used = 0i32;
    for run in runs {
        let w = measure(&run.text).max(0);
        if !run.text.is_empty() && used + w <= avail {
            out.push(run.clone());
            used += w;
            continue;
        }
        let room = avail - used - ell_w;
        let prefix = if room > 0 { prefix_fit(&run.text, room, &measure) } else { String::new() };
        if !prefix.is_empty() {
            used += measure(&prefix).max(0);
            out.push(MarkedRun { text: prefix, hit: run.hit });
        }
        if ell_w > 0 && used + ell_w <= avail {
            out.push(MarkedRun { text: ell.to_string(), hit: false });
        }
        break;
    }
    out
}

/// Full text when it fits. Otherwise keep the last path segment, so a match in the folder name stays visible.
pub(crate) fn fit_marked_path(text: &str, query: &str, avail: i32, measure: impl Fn(&str) -> i32) -> Vec<MarkedRun> {
    let full = mark_runs(text, query);
    if avail <= 0 {
        return Vec::new();
    }
    let total: i32 = full.iter().map(|r| measure(&r.text).max(0)).sum();
    if total <= avail {
        return full;
    }
    let Some(cut) = text.rfind(['\\', '/']) else {
        return fit_marked_end(&full, avail, measure);
    };
    if cut == 0 {
        return fit_marked_end(&full, avail, measure);
    }
    let shown = format!("…{}", &text[cut..]);
    fit_marked_end(&mark_runs(&shown, query), avail, measure)
}

/// Chip rectangles inside the folder cell. `widths` are already capped at the cell width.
pub(crate) fn place_chips(widths: &[i32], avail: i32, chip_h: i32, gap: i32) -> Vec<RECT> {
    let avail = avail.max(1);
    let mut out = Vec::with_capacity(widths.len());
    let mut x = 0;
    let mut y = 0;
    for &w0 in widths {
        let w = w0.max(1).min(avail);
        if x > 0 && x + w > avail {
            x = 0;
            y += chip_h + gap;
        }
        out.push(RECT { left: x, top: y, right: x + w, bottom: y + chip_h });
        x += w + gap;
    }
    out
}

/// One pane in the folder column. `leaves` is 0 when the header chip is the only folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaneBlock {
    pub header: i32,
    pub leaves: usize,
}

/// A header chip or one indented pane-tab line, relative to the folder cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BlockHit {
    /// Index into the `blocks` slice.
    pub group: usize,
    /// None is the pane header. Some is the leaf's position under that header, not its session index.
    pub leaf: Option<usize>,
    pub rect: RECT,
}

/// Header chips wrap like [`place_chips`]. A pane with pane tabs takes its own rows:
/// the header, then one full-width line per pane tab, indented. The next pane starts below that.
pub(crate) fn place_pane_blocks(
    blocks: &[PaneBlock],
    avail: i32,
    chip_h: i32,
    leaf_h: i32,
    gap: i32,
    leaf_gap: i32,
    indent: i32,
) -> Vec<BlockHit> {
    let avail = avail.max(1);
    let indent = indent.clamp(0, (avail - 1).max(0));
    let mut out = Vec::new();
    let mut x = 0;
    let mut y = 0;
    for (group, b) in blocks.iter().enumerate() {
        let w = b.header.max(1).min(avail);
        let nested = b.leaves > 0;
        if x > 0 && (nested || x + w > avail) {
            x = 0;
            y += chip_h + gap;
        }
        out.push(BlockHit {
            group,
            leaf: None,
            rect: RECT { left: x, top: y, right: x + w, bottom: y + chip_h },
        });
        if nested {
            let mut ly = y + chip_h + leaf_gap;
            let left = indent;
            let right = avail.max(left + 1);
            for k in 0..b.leaves {
                out.push(BlockHit {
                    group,
                    leaf: Some(k),
                    rect: RECT { left, top: ly, right, bottom: ly + leaf_h },
                });
                ly += leaf_h;
                if k + 1 < b.leaves {
                    ly += leaf_gap;
                }
            }
            y = ly + gap;
            x = 0;
        } else {
            x += w + gap;
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KbdSeg {
    Text(String),
    Key(String),
}

/// Split a status line on `{Enter}` / `{F2}` / `{Del}` / `{Esc}` so those words draw as key chips.
pub(crate) fn kbd_segs(s: &str) -> Vec<KbdSeg> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        let (head, tail) = rest.split_at(start);
        if !head.is_empty() {
            out.push(KbdSeg::Text(head.to_string()));
        }
        if let Some(end) = tail.find('}') {
            let key = &tail[1..end];
            if !key.is_empty() && !key.contains('{') {
                out.push(KbdSeg::Key(key.to_string()));
                rest = &tail[end + 1..];
                continue;
            }
        }
        out.push(KbdSeg::Text("{".into()));
        rest = &tail[1..];
    }
    if !rest.is_empty() {
        out.push(KbdSeg::Text(rest.to_string()));
    }
    out
}

/// One wrapped run of key-hint pieces. `start`..`end` indexes the measured widths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FooterRow {
    pub start: usize,
    pub end: usize,
    pub width: i32,
}

/// Greedy rows of piece widths. A piece wider than `width` sits on its own row.
/// Each step takes at least one piece, so an empty remainder cannot loop.
pub(crate) fn wrap_runs(widths: &[i32], width: i32) -> Vec<FooterRow> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut i = 0;
    while i < widths.len() {
        let start = i;
        let mut w = 0;
        loop {
            let piece = widths[i].max(0);
            if i > start && w + piece > width {
                break;
            }
            w += piece;
            i += 1;
            if i == widths.len() || w >= width {
                break;
            }
        }
        rows.push(FooterRow { start, end: i, width: w });
    }
    rows
}

/// How the status line shares its width. The key hint is never clipped to make room for the count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FooterPlan {
    /// Width given to the left-hand count. It ellipsizes inside this.
    pub count_w: i32,
    /// The count shares the key row. Otherwise it is a full-width row above the keys.
    pub count_on_keys: bool,
    pub key_rows: Vec<FooterRow>,
    pub rows: i32,
}

/// `width` is the status bar. `text_w` is the count string. `piece_w` are key-hint segments.
/// `gap` sits between the count and the keys when they share a row.
///
/// Keys that fit on one line stay on that line and the count shrinks. Keys that do not fit wrap,
/// and the count takes the row above them.
pub(crate) fn plan_footer(width: i32, text_w: i32, piece_w: &[i32], gap: i32) -> FooterPlan {
    let width = width.max(1);
    let text_w = text_w.max(0);
    let gap = gap.max(0);
    let keys: i32 = piece_w.iter().map(|w| (*w).max(0)).sum();
    if piece_w.is_empty() || keys == 0 {
        return FooterPlan { count_w: width.min(text_w), count_on_keys: true, key_rows: Vec::new(), rows: 1 };
    }
    if keys <= width {
        let room = if text_w == 0 { 0 } else { width.saturating_sub(keys).saturating_sub(gap) };
        return FooterPlan {
            count_w: room.min(text_w),
            count_on_keys: true,
            key_rows: vec![FooterRow { start: 0, end: piece_w.len(), width: keys }],
            rows: 1,
        };
    }
    let key_rows = wrap_runs(piece_w, width);
    let n = key_rows.len().max(1) as i32;
    FooterPlan { count_w: width.min(text_w), count_on_keys: false, key_rows, rows: n + 1 }
}

fn status_caption(n: usize, q: &str) -> String {
    let n = n.to_string();
    if q.trim().is_empty() {
        tf(Key::TabListStatus, &[("n", n.as_str())])
    } else {
        tf(Key::TabListStatusFilter, &[("n", n.as_str()), ("q", q.trim())])
    }
}

struct PaneFolder {
    pane: usize,
    sub: Option<usize>,
    label: String,
    tip: String,
    current: bool,
    tint: u8,
}

fn pane_folders(chips: &[Chip]) -> Vec<PaneFolder> {
    let mut v = Vec::new();
    for c in chips {
        if c.tabs.is_empty() {
            v.push(PaneFolder { pane: c.pane, sub: None, label: c.name.clone(), tip: c.tip.clone(), current: true, tint: c.tint });
        } else {
            for leaf in &c.tabs {
                v.push(PaneFolder { pane: c.pane, sub: Some(leaf.index), label: leaf.name.clone(), tip: leaf.tip.clone(), current: leaf.current, tint: c.tint });
            }
        }
    }
    v
}

fn summary_pick(row: &Row) -> Option<PaneFolder> {
    let folders = pane_folders(&row.chips);
    if folders.is_empty() {
        return None;
    }
    let reps: Vec<RepFolder> = folders.iter().map(|f| RepFolder { pane: f.pane, current: f.current }).collect();
    let i = representative_index(&reps, row.active).min(folders.len() - 1);
    folders.into_iter().nth(i)
}

fn summary_text(row: &Row) -> String {
    summary_pick(row).map(|f| path_text(&f.label, &f.tip).to_string()).unwrap_or_default()
}

fn summary_target(row: &Row) -> (usize, Option<usize>, u8) {
    summary_pick(row).map(|f| (f.pane, f.sub, f.tint)).unwrap_or((0, None, 0))
}

fn tab_fold_of(row: &Row) -> TabFold {
    let folders = pane_folders(&row.chips);
    TabFold { index: row.index, name: row.name.clone(), folders: folders.into_iter().map(|f| (f.label, f.tip)).collect() }
}

#[cfg(test)]
fn row_ok(row: &Row, q: &str) -> bool {
    let mut chips = Vec::new();
    for c in &row.chips {
        chips.push((c.name.as_str(), c.tip.as_str()));
        // A pane-tab match keeps the whole window tab, including its other panes.
        for leaf in &c.tabs {
            chips.push((leaf.name.as_str(), leaf.tip.as_str()));
        }
    }
    row_matches(&row.name, &chips, q)
}

fn snapshot() -> Vec<Row> {
    app(|a| {
        let n = a.s.tabs.len();
        let cur = a.s.tab.min(n.saturating_sub(1));
        let mut rows = Vec::with_capacity(n);
        for i in 0..n {
            let Some(t) = a.s.tabs.get(i) else { continue };
            let live = i == a.s.tab;
            let heights = if live { a.s.heights.clone() } else { t.heights.clone() };
            let paths = if live { a.s.paths.clone() } else { t.paths.clone() };
            let tints = if live { a.s.tints.clone() } else { t.tints.clone() };
            let count = if live { a.s.panes() } else { heights.iter().sum() };
            let count = count.clamp(1, MAXP);
            let active = if live { a.s.active } else { t.active };
            let active = active.min(count - 1);
            let path = paths.get(active).map(String::as_str).unwrap_or("");
            let name = if custom_tab_name(&t.name).is_none() && path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
                tr(Key::ThisPc).to_string()
            } else {
                tab_label(&t.name, path)
            };
            let mut chips = Vec::new();
            for pane in 0..count {
                let (list, cur) = if live {
                    resolve_pane_tabs(
                        a.s.pane_folders.get(pane).map(Vec::as_slice).unwrap_or(&[]),
                        a.s.pane_cur.get(pane).copied().unwrap_or(0),
                        paths.get(pane).map(String::as_str).unwrap_or(""),
                    )
                } else {
                    t.pane_tabs(pane)
                };
                let p = list.get(cur).map(String::as_str).unwrap_or("");
                if p.is_empty() {
                    continue;
                }
                let label = chip_name(p);
                if label.is_empty() {
                    continue;
                }
                let tip = chip_tip(p);
                let mut tabs = Vec::new();
                if list.len() > 1 {
                    for (k, path) in list.iter().enumerate() {
                        if path.is_empty() {
                            continue;
                        }
                        let name = chip_name(path);
                        if name.is_empty() {
                            continue;
                        }
                        tabs.push(Leaf { index: k, name, tip: chip_tip(path), current: k == cur });
                    }
                    // One visible folder is the header chip. Nesting starts at two.
                    if tabs.len() < 2 {
                        tabs.clear();
                    }
                }
                let head = if tabs.is_empty() { path_text(&label, &tip).to_string() } else { format!("{}  {label}", pane + 1) };
                chips.push(Chip { pane, name: label, tip, head, tint: tints.get(pane).copied().unwrap_or(0), tabs });
            }
            let total = chips.iter().map(|c| if c.tabs.is_empty() { 1 } else { c.tabs.len() }).sum::<usize>();
            rows.push(Row {
                index: i,
                name,
                locked: t.locked,
                color: t.color,
                grid: grid_label(&heights),
                heights,
                panes: count,
                chips,
                active,
                open: false,
                extra: total.saturating_sub(1),
                current: i == cur,
            });
        }
        rows
    })
}

pub(crate) fn belongs(main: HWND, hwnd: HWND) -> bool {
    let (h, owner) = with_st(|s| (s.hwnd, s.owner));
    owner == main && !h.0.is_null() && (hwnd == h || unsafe { IsChild(h, hwnd).as_bool() })
}

pub(crate) fn captures(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    let b = with_st(|s| s.hwnd);
    !b.0.is_null() && (h == b || unsafe { IsChild(b, h).as_bool() })
}

fn alive_for(main: HWND) -> bool {
    let (h, owner) = with_st(|s| (s.hwnd, s.owner));
    owner == main && !h.0.is_null() && unsafe { IsWindow(Some(h)).as_bool() }
}

pub(crate) unsafe fn refresh() {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) unsafe fn restyle() {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    let title = HSTRING::from(tr(Key::TabList));
    _ = SetWindowTextW(h, &title);
    style_window(h);
    apply_faces();
    theme_edit();
    cue();
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) unsafe fn close() {
    end_name_edit(true);
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        _ = DestroyWindow(h);
    }
}

/// Shortcut and the tab-bar button: open, or close when this window already has the list.
pub(crate) unsafe fn open_toggle() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        return;
    }
    if alive_for(main) {
        close();
        return;
    }
    reopen(main);
}

/// Tab right-click: open, or focus the list when it is already this window's.
pub(crate) unsafe fn open_menu() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        return;
    }
    if alive_for(main) {
        let h = with_st(|s| s.hwnd);
        _ = ShowWindow(h, SW_SHOWNORMAL);
        _ = SetForegroundWindow(h);
        return;
    }
    reopen(main);
}

unsafe fn reopen(main: HWND) {
    let other = with_st(|s| s.hwnd);
    if !other.0.is_null() && IsWindow(Some(other)).as_bool() {
        _ = DestroyWindow(other);
    }
    open(main);
}

/// Keys the list owns. False leaves the key for the filter edit, or for the name box while renaming.
pub(crate) unsafe fn on_key(vk: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
    let ch = VIRTUAL_KEY(vk);
    if name_editing() {
        return name_edit_key(ch, ctrl, shift, alt);
    }
    match ch {
        VK_F2 if !ctrl && !alt && !shift => {
            let filter = with_st(|s| s.edit);
            if ime_composing(filter) {
                return false;
            }
            begin_name_edit();
            true
        }
        VK_ESCAPE if !ctrl && !alt && !shift => {
            close();
            true
        }
        VK_RETURN if !ctrl && !alt && !shift => {
            go();
            true
        }
        VK_UP if !ctrl && !alt => {
            nudge_sel(-1);
            true
        }
        VK_DOWN if !ctrl && !alt => {
            nudge_sel(1);
            true
        }
        VK_RIGHT if !ctrl && !alt => {
            let edit = with_st(|s| s.edit);
            if ime_composing(edit) {
                return false;
            }
            let sel = with_st(|s| s.sel);
            apply_fold(sel, Some(true));
            true
        }
        VK_LEFT if !ctrl && !alt => {
            let edit = with_st(|s| s.edit);
            if ime_composing(edit) {
                return false;
            }
            let sel = with_st(|s| s.sel);
            apply_fold(sel, Some(false));
            true
        }
        VK_PRIOR if !alt => {
            nudge_sel(-8);
            true
        }
        VK_NEXT if !alt => {
            nudge_sel(8);
            true
        }
        VK_HOME if !ctrl && !alt => {
            nudge_sel(-10_000);
            true
        }
        VK_END if !ctrl && !alt => {
            nudge_sel(10_000);
            true
        }
        VK_DELETE if !ctrl && !alt && !shift => {
            let (edit, q) = with_st(|s| (s.edit, s.query.clone()));
            let focused = GetFocus() == edit;
            if !focused || q.trim().is_empty() {
                close_selected();
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

unsafe fn open(main: HWND) {
    let dpi = GetDpiForWindow(main).max(96) as i32;
    let style = WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN | WS_VISIBLE;
    let mut rc = RECT { left: 0, top: 0, right: sc(dpi, 840), bottom: sc(dpi, 500) };
    _ = AdjustWindowRectExForDpi(&mut rc, style, false, WINDOW_EX_STYLE(0), dpi as u32);
    let (ww, hh) = (rc.right - rc.left, rc.bottom - rc.top);
    let (x, y) = place(main, ww, hh);
    let title = HSTRING::from(tr(Key::TabList));
    let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroTabList"), &title, style, x, y, ww, hh, Some(main), None, None, None) else {
        return;
    };
    let cur = app(|a| a.s.tab);
    with_st(|s| {
        *s = State::new();
        s.hwnd = hwnd;
        s.owner = main;
        s.sel = cur;
        s.dpi = GetDpiForWindow(hwnd).max(96) as i32;
    });
    style_window(hwnd);
    let edit = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("EDIT"),
        w!(""),
        WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        0,
        0,
        0,
        0,
        Some(hwnd),
        Some(HMENU(ID_EDIT as isize as _)),
        None,
        None,
    )
    .unwrap_or_default();
    with_st(|s| s.edit = edit);
    if !edit.0.is_null() {
        _ = SetWindowSubclass(edit, Some(edit_proc), 1, 0);
    }
    let _ = ensure_name_edit();
    apply_faces();
    theme_edit();
    cue();
    layout(hwnd);
    _ = ShowWindow(hwnd, SW_SHOW);
    _ = SetForegroundWindow(hwnd);
    if !edit.0.is_null() {
        _ = SetFocus(Some(edit));
    }
}

fn place(owner: HWND, ww: i32, hh: i32) -> (i32, i32) {
    let mut frame = RECT::default();
    unsafe { _ = GetWindowRect(owner, &mut frame) };
    let mut x = frame.left + (frame.right - frame.left - ww) / 2;
    let mut y = frame.top + (frame.bottom - frame.top - hh) / 2;
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let work = if unsafe { GetMonitorInfoW(MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() } {
        mi.rcWork
    } else {
        RECT { left: 0, top: 0, right: 1920, bottom: 1080 }
    };
    x = x.clamp(work.left, (work.right - ww).max(work.left));
    y = y.clamp(work.top, (work.bottom - hh).max(work.top));
    (x, y)
}

unsafe fn style_window(hwnd: HWND) {
    style_dialog_caption(hwnd);
}

/// Point the filter and the rename box at the shared faces.
unsafe fn apply_faces() {
    let (edit, name) = with_st(|s| (s.edit, s.name_edit));
    let font = body_font();
    let bold = bold_ui_font();
    if !edit.0.is_null() {
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    }
    if !name.0.is_null() {
        SendMessageW(name, WM_SETFONT, Some(WPARAM(bold.0 as usize)), Some(LPARAM(1)));
    }
}

unsafe fn theme_edit() {
    let (edit, name_edit) = with_st(|s| (s.edit, s.name_edit));
    let dark = DARK_ON.load(Ordering::Relaxed);
    let theme = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
    for h in [edit, name_edit] {
        if !h.0.is_null() {
            _ = SetWindowTheme(h, theme, PCWSTR::null());
        }
    }
}

unsafe fn cue() {
    let edit = with_st(|s| s.edit);
    if edit.0.is_null() {
        return;
    }
    let hs = HSTRING::from(tr(Key::TabListFilter));
    SendMessageW(edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(hs.as_ptr() as isize)));
}

unsafe fn text_px(dc: HDC, font: HFONT, s: &str) -> i32 {
    let old = SelectObject(dc, font.into());
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    let n = if w.is_empty() || !GetTextExtentPoint32W(dc, &w, &mut sz).as_bool() { 0 } else { sz.cx };
    SelectObject(dc, old);
    n
}

fn chord_label() -> String {
    crate::keys::index("tab.list").map(crate::keys::chord_text).unwrap_or_default()
}

unsafe fn layout(hwnd: HWND) {
    let dpi = GetDpiForWindow(hwnd).max(96) as i32;
    let mut rc = RECT::default();
    if GetClientRect(hwnd, &mut rc).is_err() {
        return;
    }
    let width = (rc.right - rc.left).max(1);
    let height = (rc.bottom - rc.top).max(1);
    let pad = px(dpi, PAD_BODY_X);
    let gap = px(dpi, SPACE_8);
    let hint_h = px(dpi, FONT_PX + SPACE_8);
    let search_h = px(dpi, FIELD_H);
    let btn_h = px(dpi, BTN_H_FOOTER);
    let y0 = px(dpi, PAD_BODY_Y);
    let hint = RECT { left: pad, top: y0, right: width - pad, bottom: y0 + hint_h };
    let search = RECT { left: pad, top: hint.bottom + gap, right: width - pad, bottom: hint.bottom + gap + search_h };

    let dc = GetDC(Some(hwnd));
    let font = body_font();
    let bold = bold_ui_font();
    let small = meta_font();
    let chord = chord_label();
    let inner = field_edit_rect(search, dpi);
    let chord_w = if chord.is_empty() { 0 } else { text_px(dc, small, &chord) + px(dpi, BTN_PAD_X) };
    let chord_r = if chord_w == 0 {
        RECT::default()
    } else {
        let right = inner.right;
        let h = px(dpi, 16);
        let top = search.top + (search_h - h) / 2;
        RECT { left: (right - chord_w).max(inner.left), top, right, bottom: top + h }
    };
    let edit_left = inner.left + px(dpi, 18);
    let edit_right = if chord_r.right > chord_r.left { chord_r.left - px(dpi, SPACE_4) } else { inner.right };
    let edit_r = RECT { left: edit_left, top: inner.top, right: edit_right.max(edit_left + 1), bottom: inner.bottom };

    let rows_all = snapshot();
    let (edit, prev_q, prev_sel) = with_st(|s| (s.edit, s.query.clone(), s.sel));
    let q = if edit.0.is_null() { prev_q.clone() } else { text_of(edit) };
    let current = rows_all.iter().find(|r| r.current).map(|r| r.index).unwrap_or(0);
    let mut collapsed = with_st(|s| s.collapsed.clone());
    if prev_q.trim() != q.trim() {
        let folds: Vec<TabFold> = rows_all.iter().map(tab_fold_of).collect();
        reveal_matches(&folds, &mut collapsed, &q);
        with_st(|s| s.collapsed = collapsed.clone());
    }
    let expanded = with_st(|s| s.expanded.clone());
    let plan = visible_fold_rows(&rows_all.iter().map(tab_fold_of).collect::<Vec<_>>(), &expanded, &collapsed, &q, current);
    let ids: Vec<usize> = plan.iter().map(fold_index).collect();
    let sel = if ids.iter().any(|&i| i == prev_sel) { prev_sel } else { ids.first().copied().unwrap_or(0) };
    let mut shown = Vec::new();
    for mut row in rows_all {
        let Some(kind) = plan.iter().find(|f| fold_index(f) == row.index) else { continue };
        row.open = matches!(kind, FoldRow::Open { .. });
        if let FoldRow::Collapsed { extra, .. } = *kind {
            row.extra = extra;
        }
        shown.push(row);
    }

    // The key hint stays whole. The count shrinks on the same line, or the keys wrap and the
    // count takes the line above. Measured before the list, so the buttons move up with it.
    let line_h = sc(dpi, 22);
    let status_w = (width - pad * 2).max(1);
    let caption = status_caption(shown.len(), &q);
    let piece_w = kbd_piece_widths(dc, small, dpi);
    let footer = plan_footer(status_w, text_px(dc, small, &caption), &piece_w, sc(dpi, 8));
    let status_h = line_h * footer.rows.max(1);
    let btn_top = height - pad - status_h - gap - btn_h;
    let frame_top = search.bottom + gap;
    let frame_bot = (btn_top - gap).max(frame_top + sc(dpi, 40));
    let frame = RECT { left: pad, top: frame_top, right: width - pad, bottom: frame_bot };
    let head_h = px(dpi, 22);
    let rad = px(dpi, RADIUS).max(1);
    let head = RECT { left: frame.left + 1, top: frame.top + rad, right: frame.right - 1, bottom: (frame.top + rad + head_h).min(frame.bottom - 1) };
    let list = RECT { left: head.left, top: head.bottom, right: head.right, bottom: (frame.bottom - 1).max(head.bottom) };
    let status = RECT { left: pad, top: height - pad - status_h, right: width - pad, bottom: height - pad };

    let col_name = sc(dpi, 112);
    let col_lay = sc(dpi, 56);
    let col_n = sc(dpi, 32);
    let col_gap = sc(dpi, 6);
    let inset = sc(dpi, 8);
    let folders_left = list.left + inset + col_name + col_gap + col_lay + col_gap + col_n + col_gap;
    let folders_right = (list.right - inset).max(folders_left + 1);
    let avail = (folders_right - folders_left).max(1);
    let chip_h = sc(dpi, 16);
    let chip_gap = sc(dpi, 3);
    let leaf_h = sc(dpi, 18);
    let leaf_gap = sc(dpi, 2);
    let indent = sc(dpi, 14);

    let mut y = 0;
    let mut boxes = Vec::new();
    let mut chips = Vec::new();
    let mut badges = Vec::new();
    for row in &shown {
        // The folder column gets the width the name and layout columns do not use.
        // A path chip grows up to that column instead of stopping at a fixed 108 DIP.
        // A shut tab keeps one chip, the representative folder, and leaves room for `+N`.
        let summary = if row.open { None } else { Some(summary_target(row)) };
        let blocks: Vec<PaneBlock> = if row.open {
            row.chips.iter().map(|c| PaneBlock { header: chip_width(dc, small, &c.head, dpi, avail), leaves: c.tabs.len() }).collect()
        } else {
            let label = summary_text(row);
            let badge_w = extra_label(row.extra).map(|s| text_px(dc, small, &s) + sc(dpi, 10)).unwrap_or(0);
            let gap_b = if badge_w > 0 { sc(dpi, 4) } else { 0 };
            let max_w = (avail - badge_w - gap_b).max(sc(dpi, 20));
            if label.is_empty() {
                Vec::new()
            } else {
                vec![PaneBlock { header: chip_width(dc, small, &label, dpi, max_w), leaves: 0 }]
            }
        };
        let placed = place_pane_blocks(&blocks, avail, chip_h, leaf_h, chip_gap, leaf_gap, indent);
        let block = placed.last().map(|r| r.rect.bottom).unwrap_or(0);
        let row_h = (block + sc(dpi, 4)).max(sc(dpi, 24));
        let top_off = if block == 0 { 0 } else { ((row_h - block) / 2).max(sc(dpi, 2)) };
        if let Some((pane, sub, _)) = summary {
            if let Some(hit) = placed.first() {
                let r = hit.rect;
                chips.push(ChipBox {
                    tab: row.index,
                    pane,
                    sub,
                    rect: RECT {
                        left: folders_left + r.left,
                        top: r.top + top_off,
                        right: folders_left + r.right,
                        bottom: r.bottom + top_off,
                    },
                });
                if let Some(label) = extra_label(row.extra) {
                    let bw = text_px(dc, small, &label) + sc(dpi, 10);
                    let left = folders_left + r.right + sc(dpi, 4);
                    badges.push(BadgeBox {
                        tab: row.index,
                        rect: RECT { left, top: r.top + top_off, right: left + bw, bottom: r.bottom + top_off },
                    });
                }
            }
        } else {
            for hit in &placed {
                let Some(chip) = row.chips.get(hit.group) else { continue };
                let sub = match hit.leaf {
                    None => None,
                    Some(k) => match chip.tabs.get(k) {
                        Some(leaf) => Some(leaf.index),
                        None => continue,
                    },
                };
                let r = hit.rect;
                chips.push(ChipBox {
                    tab: row.index,
                    pane: chip.pane,
                    sub,
                    rect: RECT {
                        left: folders_left + r.left,
                        top: r.top + top_off, // content-relative; scroll applied after
                        right: folders_left + r.right,
                        bottom: r.bottom + top_off,
                    },
                });
            }
        }
        boxes.push(RowBox { tab: row.index, y, h: row_h });
        y += row_h;
    }
    let content_h = y;
    let view_h = (list.bottom - list.top).max(0);
    let max_scroll = (content_h - view_h).max(0);
    let mut scroll = with_st(|s| s.scroll.clamp(0, max_scroll));
    // Keep the row being renamed on screen, so hiding the box to scroll it away does not save.
    if let Some(tab) = with_st(|s| s.renaming) {
        if let Some(row) = boxes.iter().find(|b| b.tab == tab) {
            if row.h < view_h {
                if row.y < scroll {
                    scroll = row.y;
                } else if row.y + row.h > scroll + view_h {
                    scroll = row.y + row.h - view_h;
                }
                scroll = scroll.clamp(0, max_scroll);
            }
        }
    }
    for chip in &mut chips {
        let row_y = boxes.iter().find(|b| b.tab == chip.tab).map(|b| b.y).unwrap_or(0);
        chip.rect.top += list.top + row_y - scroll;
        chip.rect.bottom += list.top + row_y - scroll;
    }
    for badge in &mut badges {
        let row_y = boxes.iter().find(|b| b.tab == badge.tab).map(|b| b.y).unwrap_or(0);
        badge.rect.top += list.top + row_y - scroll;
        badge.rect.bottom += list.top + row_y - scroll;
    }

    let go_on = !shown.is_empty();
    let close_on = go_on && app(|a| can_close_tab(&a.s.tabs, sel));
    let labels = [(GO, tr(Key::TabListGo), true), (CLOSE_TAB, tr(Key::CloseTab), false), (DISMISS, tr(Key::Close), false)];
    let mut buttons = Vec::new();
    let mut x = pad;
    for (id, label, primary) in labels {
        if id == DISMISS {
            continue;
        }
        let face = if primary { bold } else { font };
        let w = button_width(text_px(dc, face, label), dpi);
        let on = if id == GO { go_on } else { close_on };
        buttons.push(Btn { id, rect: RECT { left: x, top: btn_top, right: x + w, bottom: btn_top + btn_h }, on });
        x += w + px(dpi, BTN_GAP);
    }
    let dismiss = tr(Key::Close);
    let dw = button_width(text_px(dc, font, dismiss), dpi);
    buttons.push(Btn {
        id: DISMISS,
        rect: RECT { left: width - pad - dw, top: btn_top, right: width - pad, bottom: btn_top + btn_h },
        on: true,
    });
    let _ = ReleaseDC(Some(hwnd), dc);

    with_st(|s| {
        s.dpi = dpi;
        s.query = q;
        s.sel = sel;
        s.scroll = scroll;
        s.hint = hint;
        s.search = search;
        s.chord = chord_r;
        s.frame = frame;
        s.head = head;
        s.list = list;
        s.status = status;
        s.buttons = buttons;
        s.shown = shown;
        s.rows = boxes;
        s.chips = chips;
        s.badges = badges;
        s.content_h = content_h;
    });
    let edit = with_st(|s| s.edit);
    if !edit.0.is_null() {
        place_edit(edit, edit_r, true);
    }
    place_name_edit();
    let info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: (content_h - 1).max(0),
        nPage: view_h.max(0) as u32,
        nPos: scroll,
        nTrackPos: 0,
    };
    _ = SetScrollInfo(hwnd, SB_VERT, &info, false);
}

/// Header chip width. `max_w` is the folder column, so a long path uses the wider list.
fn chip_width(dc: HDC, font: HFONT, name: &str, dpi: i32, max_w: i32) -> i32 {
    let text = unsafe { text_px(dc, font, name) };
    let pad = sc(dpi, 4) + sc(dpi, 5) + sc(dpi, 7) + sc(dpi, 3);
    (text + pad).clamp(sc(dpi, 20), max_w.max(sc(dpi, 20)))
}

fn chip_dot(tint: u8) -> u32 {
    tint_color(tint).unwrap_or_else(|| pal().accents[1].1)
}

/// `open` forces the row. `None` flips it. A filter still hides tabs that do not match.
unsafe fn apply_fold(index: usize, open: Option<bool>) {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    let (q, current, any) = with_st(|s| {
        let current = s.shown.iter().find(|r| r.current).map(|r| r.index).unwrap_or(0);
        let any = s.shown.iter().any(|r| r.index == index);
        (s.query.clone(), current, any)
    });
    if !any {
        return;
    }
    let filtering = !q.trim().is_empty();
    hide_chip_tip();
    with_st(|s| {
        let expanded = s.expanded.clone();
        let collapsed = s.collapsed.clone();
        let open_now = tab_is_open(index, &expanded, &collapsed, filtering, current);
        if let Some(next) = open {
            set_fold(index, next, &mut s.expanded, &mut s.collapsed);
        } else {
            toggle_fold(index, open_now, &mut s.expanded, &mut s.collapsed);
        }
        s.sel = index;
    });
    layout(h);
    reveal();
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn nudge_sel(delta: i32) {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    layout(h);
    let (sel, ids) = with_st(|s| (s.sel, s.shown.iter().map(|r| r.index).collect::<Vec<_>>()));
    if ids.is_empty() {
        return;
    }
    let pos = ids.iter().position(|&i| i == sel).unwrap_or(0);
    let next = (pos as i32 + delta).clamp(0, ids.len() as i32 - 1) as usize;
    with_st(|s| s.sel = ids[next]);
    reveal();
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn reveal() {
    let (list, scroll, sel, rows) = with_st(|s| (s.list, s.scroll, s.sel, s.rows.clone()));
    let Some(row) = rows.iter().find(|r| r.tab == sel) else { return };
    let view = (list.bottom - list.top).max(0);
    let mut next = scroll;
    if row.y < scroll {
        next = row.y;
    } else if row.y + row.h > scroll + view {
        next = row.y + row.h - view;
    }
    let max = with_st(|s| (s.content_h - view).max(0));
    next = next.clamp(0, max);
    if next != scroll {
        hide_chip_tip();
        with_st(|s| s.scroll = next);
    }
}

unsafe fn go() {
    end_name_edit(true);
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() {
        layout(h);
    }
    let (sel, any, owner) = with_st(|s| (s.sel, !s.shown.is_empty(), s.owner));
    if !any {
        return;
    }
    switch_tab(sel);
    close();
    if !owner.0.is_null() && IsWindow(Some(owner)).as_bool() {
        bind_hwnd(owner);
        focus_view(active_pane());
    }
}

/// Open that window tab, focus the pane, and select the pane tab when `sub` is one. Then dismiss.
unsafe fn go_pane(tab: usize, pane: usize, sub: Option<usize>) {
    end_name_edit(true);
    hide_chip_tip();
    let owner = with_st(|s| s.owner);
    switch_tab(tab);
    let n = app(|a| a.s.panes().max(1));
    if pane < n {
        set_active(pane);
        if let Some(k) = sub {
            pick_pane_tab(pane, k);
        }
    }
    close();
    if !owner.0.is_null() && IsWindow(Some(owner)).as_bool() {
        bind_hwnd(owner);
        focus_view(if pane < n { pane } else { active_pane() });
    }
}

unsafe fn close_selected() {
    end_name_edit(true);
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() {
        layout(h);
    }
    let (sel, any) = with_st(|s| (s.sel, s.shown.iter().any(|r| r.index == s.sel)));
    if !any || !app(|a| can_close_tab(&a.s.tabs, sel)) {
        return;
    }
    hide_chip_tip();
    close_tab(sel);
    let n = app(|a| a.s.tabs.len());
    with_st(|s| {
        shift_fold_sets(&mut s.expanded, &mut s.collapsed, sel);
        s.sel = sel.min(n.saturating_sub(1));
    });
    let h = with_st(|s| s.hwnd);
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        layout(h);
        _ = InvalidateRect(Some(h), None, false);
        let edit = with_st(|s| s.edit);
        if !edit.0.is_null() {
            _ = SetFocus(Some(edit));
        }
    }
}

unsafe fn hide_chip_tip() {
    let (h, inst) = with_st(|s| (s.hwnd, s.chip_tip));
    if h.0.is_null() {
        return;
    }
    let (now, _) = tip_leave(h, CHIP_TIP, inst);
    with_st(|s| s.chip_tip = now);
}

fn tip_key(tab: usize, pane: usize, sub: Option<usize>, tip: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in tip.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let sub = sub.unwrap_or(0xFFFF) as u64;
    h ^ ((tab as u64) << 48) ^ ((pane as u64) << 32) ^ (sub << 16)
}

unsafe fn show_chip_tip(x: i32, y: i32) {
    let (h, list, inst, chips) = with_st(|s| (s.hwnd, s.list, s.chip_tip, s.chips.clone()));
    if h.0.is_null() {
        return;
    }
    let hit = inside(list, x, y).then(|| chips.into_iter().rev().find(|c| inside(c.rect, x, y))).flatten();
    let Some(chip) = hit else {
        if inst {
            hide_chip_tip();
        }
        return;
    };
    let tip = snapshot()
        .into_iter()
        .find(|r| r.index == chip.tab)
        .and_then(|r| {
            let c = r.chips.into_iter().find(|c| c.pane == chip.pane)?;
            match chip.sub {
                None => Some(c.tip),
                Some(k) => c.tabs.into_iter().find(|t| t.index == k).map(|t| t.tip),
            }
        })
        .unwrap_or_default();
    let screen = tip_screen_rect(h, chip.rect);
    let (now, _) = tip_enter(h, CHIP_TIP, tip_key(chip.tab, chip.pane, chip.sub, &tip), &tip, screen, TipSide::Below, inst);
    with_st(|s| s.chip_tip = now);
}

fn hit_btn(x: i32, y: i32) -> Option<u8> {
    with_st(|s| s.buttons.iter().find(|b| b.on && inside(b.rect, x, y)).map(|b| b.id))
}

fn hit_chip(x: i32, y: i32) -> Option<ChipBox> {
    let (list, chips) = with_st(|s| (s.list, s.chips.clone()));
    if !inside(list, x, y) {
        return None;
    }
    chips.into_iter().rev().find(|c| inside(c.rect, x, y))
}

fn hit_chevron(x: i32, y: i32) -> Option<usize> {
    let (list, scroll, rows, dpi) = with_st(|s| (s.list, s.scroll, s.rows.clone(), s.dpi));
    if !inside(list, x, y) {
        return None;
    }
    for row in rows {
        let top = list.top + row.y - scroll;
        let r = RECT { left: list.left, top, right: list.right, bottom: top + row.h };
        if inside(chevron_box(r, dpi), x, y) {
            return Some(row.tab);
        }
    }
    None
}

fn hit_row(x: i32, y: i32) -> Option<usize> {
    let (list, scroll, rows) = with_st(|s| (s.list, s.scroll, s.rows.clone()));
    if !inside(list, x, y) {
        return None;
    }
    let y = y - list.top + scroll;
    rows.into_iter().find(|r| y >= r.y && y < r.y + r.h).map(|r| r.tab)
}

fn scroll_to(next: i32) {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    unsafe { hide_chip_tip() };
    with_st(|s| s.scroll = next);
    unsafe {
        layout(h);
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn on_scroll(wp: usize) {
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    layout(h);
    let (view, cur, content) = with_st(|s| ((s.list.bottom - s.list.top).max(1), s.scroll, s.content_h));
    let max = (content - view).max(0);
    let mut info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        nMin: 0,
        nMax: 0,
        nPage: 0,
        nPos: 0,
        nTrackPos: 0,
    };
    _ = GetScrollInfo(h, SB_VERT, &mut info);
    let line = sc(with_st(|s| s.dpi), 24);
    let next = match SCROLLBAR_COMMAND((wp & 0xffff) as i32) {
        SB_LINEUP => cur - line,
        SB_LINEDOWN => cur + line,
        SB_PAGEUP => cur - view,
        SB_PAGEDOWN => cur + view,
        SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos,
        SB_TOP => 0,
        SB_BOTTOM => max,
        _ => cur,
    };
    scroll_to(next.clamp(0, max));
}

fn cols(list: RECT, dpi: i32) -> (i32, i32, i32, i32, i32) {
    let inset = sc(dpi, 8);
    let gap = sc(dpi, 6);
    let name = sc(dpi, 112);
    let lay = sc(dpi, 56);
    let n = sc(dpi, 32);
    let x = list.left + inset;
    (x, x + name + gap, x + name + gap + lay + gap, x + name + gap + lay + gap + n + gap, list.right - inset)
}

unsafe fn paint(dc: HDC, _rc: RECT) {
    let st = with_st(|s| PaintSnap {
        dpi: s.dpi,
        hint: s.hint,
        search: s.search,
        chord: s.chord,
        frame: s.frame,
        head: s.head,
        list: s.list,
        status: s.status,
        buttons: s.buttons.iter().map(|b| (b.id, b.rect, b.on)).collect(),
        shown: s.shown.clone(),
        rows: s.rows.clone(),
        chips: s.chips.clone(),
        badges: s.badges.clone(),
        scroll: s.scroll,
        sel: s.sel,
        hot_row: s.hot_row,
        track: s.track,
        query: s.query.clone(),
        renaming: s.renaming,
        font: unsafe { body_font() },
        bold: unsafe { bold_ui_font() },
        small: unsafe { meta_font() },
        icon: unsafe { icon_font() },
        search_focus: !s.edit.0.is_null() && unsafe { GetFocus() } == s.edit,
        name_focus: !s.name_edit.0.is_null() && unsafe { GetFocus() } == s.name_edit,
    });
    let t = ui_tokens();
    let dpi = st.dpi;
    paint_window(dc, _rc);
    paint_meta(dc, st.hint, tr(Key::TabListHint), st.small);
    paint_field(dc, st.search, dpi, st.search_focus);
    let glyph = RECT { left: st.search.left + px(dpi, FIELD_INSET), top: st.search.top, right: st.search.left + px(dpi, 22), bottom: st.search.bottom };
    text(dc, glyph, ICON_SEARCH, st.icon, t.sub, DT_CENTER);
    if st.chord.right > st.chord.left {
        paint_chip(dc, st.chord, dpi, t.field_bg);
        text(dc, st.chord, &chord_label(), st.small, t.sub, DT_CENTER);
    }
    if st.frame.bottom > st.frame.top {
        crate::ui::paint_card(dc, st.frame, dpi);
    }
    fill(dc, st.head, t.bg);
    fill(dc, RECT { top: st.head.bottom - 1, bottom: st.head.bottom, ..st.head }, t.line);
    let (x0, x1, x2, x3, x4) = cols(st.head, dpi);
    paint_meta(dc, RECT { left: x0, right: x1 - sc(dpi, 6), ..st.head }, tr(Key::Tab), st.small);
    paint_meta(dc, RECT { left: x1, right: x2 - sc(dpi, 6), ..st.head }, tr(Key::TabListLayout), st.small);
    text(dc, RECT { left: x2, right: x3 - sc(dpi, 6), ..st.head }, tr(Key::TabListPanes), st.small, t.sub, DT_RIGHT);
    paint_meta(dc, RECT { left: x3, right: x4, ..st.head }, tr(Key::Folder), st.small);

    let saved = SaveDC(dc);
    IntersectClipRect(dc, st.list.left, st.list.top, st.list.right, st.list.bottom);
    for (row, slot) in st.shown.iter().zip(st.rows.iter()) {
        let top = st.list.top + slot.y - st.scroll;
        let r = RECT { left: st.list.left, top, right: st.list.right, bottom: top + slot.h };
        if r.bottom <= st.list.top || r.top >= st.list.bottom {
            continue;
        }
        let selected = row.index == st.sel;
        let hot = st.hot_row == Some(row.index);
        let bg = row_fill(selected, hot, t.dark);
        fill(dc, r, bg);
        if row.current {
            let inset = px(dpi, 3).min((r.bottom - r.top) / 4);
            fill(dc, RECT { left: r.left, top: r.top + inset, right: r.left + px(dpi, 3).max(2), bottom: r.bottom - inset }, crate::basket::CERULEAN);
        }
        fill(dc, RECT { top: r.bottom - 1, bottom: r.bottom, ..r }, t.line);
        let mark = if row.open { "\u{E70D}" } else { "\u{E76C}" };
        text(dc, chevron_box(r, dpi), mark, st.icon, t.sub, DT_CENTER);
        paint_name(dc, r, row, dpi, &st, bg);
        paint_layout(dc, r, row, dpi, &st);
        let count = RECT { left: x2, top: r.top, right: x3 - sc(dpi, 2), bottom: r.bottom };
        text(dc, count, &row.panes.to_string(), st.small, t.sub, DT_RIGHT);
        paint_chips(dc, row, &st, bg);
    }
    let _ = RestoreDC(dc, saved);

    for (id, rect, on) in &st.buttons {
        let kind = if *id == GO { ButtonKind::Primary } else { ButtonKind::Secondary };
        let label = match *id {
            GO => tr(Key::TabListGo),
            CLOSE_TAB => tr(Key::CloseTab),
            _ => tr(Key::Close),
        };
        let font = if *id == GO && *on { st.bold } else { st.font };
        paint_button(dc, *rect, kind, st.track.visual(*id, !on), label, font, st.dpi, t.bg, FocusRing::Outside);
    }
    paint_footer(dc, st.status, &status_caption(st.shown.len(), &st.query), &st);
}

struct PaintSnap {
    dpi: i32,
    hint: RECT,
    search: RECT,
    chord: RECT,
    frame: RECT,
    head: RECT,
    list: RECT,
    status: RECT,
    buttons: Vec<(u8, RECT, bool)>,
    shown: Vec<Row>,
    rows: Vec<RowBox>,
    chips: Vec<ChipBox>,
    badges: Vec<BadgeBox>,
    scroll: i32,
    sel: usize,
    hot_row: Option<usize>,
    track: Tracker<u8>,
    query: String,
    renaming: Option<usize>,
    font: HFONT,
    bold: HFONT,
    small: HFONT,
    icon: HFONT,
    search_focus: bool,
    name_focus: bool,
}

unsafe fn kbd_piece_widths(dc: HDC, small: HFONT, dpi: i32) -> Vec<i32> {
    kbd_segs(tr(Key::TabListKeys))
        .iter()
        .map(|seg| match seg {
            KbdSeg::Text(s) => text_px(dc, small, s),
            KbdSeg::Key(s) => text_px(dc, small, s) + sc(dpi, 8),
        })
        .collect()
}

/// The count ellipsizes. Every key chip is drawn, on a later row when the bar is too narrow.
unsafe fn paint_footer(dc: HDC, status: RECT, caption: &str, st: &PaintSnap) {
    let dpi = st.dpi;
    let width = (status.right - status.left).max(1);
    let segs = kbd_segs(tr(Key::TabListKeys));
    let widths = kbd_piece_widths(dc, st.small, dpi);
    let plan = plan_footer(width, text_px(dc, st.small, caption), &widths, sc(dpi, 8));
    let line_h = sc(dpi, 22);
    let t = ui_tokens();
    if plan.count_w > 0 {
        let bottom = if plan.count_on_keys { status.bottom } else { (status.top + line_h).min(status.bottom) };
        let count = RECT { left: status.left, top: status.top, right: status.left + plan.count_w, bottom };
        paint_meta(dc, count, caption, st.small);
    }
    let mut y = if plan.count_on_keys { status.top } else { status.top + line_h };
    let saved = SaveDC(dc);
    IntersectClipRect(dc, status.left, status.top, status.right, status.bottom);
    for row in &plan.key_rows {
        let fits = row.width <= width;
        let mut x = if fits { status.right - row.width } else { status.left };
        let row_bot = y + line_h;
        for i in row.start..row.end {
            let w = widths.get(i).copied().unwrap_or(0);
            let r = RECT { left: x, top: y, right: x + w, bottom: row_bot };
            match segs.get(i) {
                Some(KbdSeg::Text(s)) => paint_meta(dc, r, s, st.small),
                Some(KbdSeg::Key(s)) => {
                    let h = sc(dpi, 14);
                    let top = y + (line_h - h) / 2;
                    let kr = RECT { left: r.left, top, right: r.right, bottom: top + h };
                    paint_chip(dc, kr, dpi, t.bg);
                    text(dc, kr, s, st.small, t.sub, DT_CENTER);
                }
                None => {}
            }
            x += w;
        }
        y += line_h;
    }
    let _ = RestoreDC(dc, saved);
}

unsafe fn paint_name(dc: HDC, row: RECT, data: &Row, dpi: i32, st: &PaintSnap, ground: u32) {
    let text_r = name_text_rect(row, data.color.is_some(), dpi);
    if let Some(c) = data.color {
        let d = sc(dpi, 8);
        let mid = (row.top + row.bottom) / 2;
        let dot = RECT { left: text_r.left - sc(dpi, 4) - d, top: mid - d / 2, right: text_r.left - sc(dpi, 4), bottom: mid + d / 2 };
        ellipse(dc, dot, c, None);
    }
    if st.renaming == Some(data.index) {
        let field = name_field(row, data.color.is_some(), dpi);
        paint_field_on(dc, field, dpi, st.name_focus, ground);
        return;
    }
    let t = ui_tokens();
    let mut r = text_r;
    if data.locked {
        let lock_w = sc(dpi, 14);
        let room = (r.right - r.left - lock_w).max(0);
        let tw = text_px(dc, st.bold, &data.name).min(room);
        let lock = RECT { left: r.left + tw, top: r.top, right: r.left + tw + lock_w, bottom: r.bottom };
        r.right = lock.left;
        text(dc, lock, ICON_LOCK, st.icon, t.sub, DT_CENTER);
    }
    paint_marked(dc, r, &data.name, st.bold, &st.query, ground);
}

unsafe fn paint_layout(dc: HDC, row: RECT, data: &Row, dpi: i32, st: &PaintSnap) {
    let (_, x1, x2, _, _) = cols(RECT { left: row.left, right: row.right, ..row }, dpi);
    let iw = sc(dpi, 22);
    let ih = sc(dpi, 14);
    let mid = (row.top + row.bottom) / 2;
    let icon = RECT { left: x1, top: mid - ih / 2, right: x1 + iw, bottom: mid - ih / 2 + ih };
    paint_grid_icon(dc, icon, &data.heights);
    let label = RECT { left: icon.right + sc(dpi, 4), top: row.top, right: x2 - sc(dpi, 2), bottom: row.bottom };
    paint_meta(dc, label, &data.grid, st.small);
}

unsafe fn paint_grid_icon(dc: HDC, r: RECT, heights: &[usize]) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let t = ui_tokens();
    fill(dc, r, t.line);
    let inner = RECT { left: r.left + 1, top: r.top + 1, right: (r.right - 1).max(r.left + 2), bottom: (r.bottom - 1).max(r.top + 2) };
    fill(dc, inner, t.sub);
    let spec: Vec<usize> = if heights.is_empty() { vec![1] } else { heights.to_vec() };
    let cols_n = spec.len().clamp(1, 8) as i32;
    let gap = 1;
    let iw = inner.right - inner.left;
    let ih = inner.bottom - inner.top;
    if iw < cols_n || ih < 1 {
        return;
    }
    let gaps = gap * (cols_n - 1);
    let base = (iw - gaps) / cols_n;
    let extra = (iw - gaps) - base * cols_n;
    let mut x = inner.left;
    for c in 0..cols_n {
        let rows = spec.get(c as usize).copied().unwrap_or(1).clamp(1, 8) as i32;
        let w = base + if c < extra { 1 } else { 0 };
        let rg = gap * (rows - 1);
        let rh = ((ih - rg) / rows).max(1);
        let mut y = inner.top;
        for _ in 0..rows {
            let bottom = (y + rh).min(inner.bottom);
            if w > 0 && bottom > y {
                fill(dc, RECT { left: x, top: y, right: (x + w).min(inner.right), bottom }, t.card);
            }
            y = bottom + gap;
        }
        x += w + gap;
    }
}

unsafe fn paint_chips(dc: HDC, row: &Row, st: &PaintSnap, ground: u32) {
    if !row.open {
        paint_summary(dc, row, st, ground);
        return;
    }
    let dpi = st.dpi;
    for spot in st.chips.iter().filter(|c| c.tab == row.index) {
        let r = spot.rect;
        if r.bottom <= st.list.top || r.top >= st.list.bottom {
            continue;
        }
        let Some(chip) = row.chips.iter().find(|c| c.pane == spot.pane) else { continue };
        match spot.sub {
            None => {
                paint_chip(dc, r, dpi, ground);
                let d = sc(dpi, 7);
                let mid = (r.top + r.bottom) / 2;
                let dot = RECT { left: r.left + sc(dpi, 4), top: mid - d / 2, right: r.left + sc(dpi, 4) + d, bottom: mid + d / 2 };
                ellipse(dc, dot, chip_dot(chip.tint), None);
                let text_r = RECT { left: dot.right + sc(dpi, 3), right: r.right - sc(dpi, 4), ..r };
                paint_marked(dc, text_r, &chip.head, st.small, &st.query, ground);
            }
            Some(k) => {
                let Some(leaf) = chip.tabs.iter().find(|t| t.index == k) else { continue };
                let label = path_text(&leaf.name, &leaf.tip);
                let bar = sc(dpi, 2).max(2);
                let mark_w = sc(dpi, 14);
                let text_r = RECT { left: r.left + bar + mark_w, right: r.right - sc(dpi, 4), ..r };
                if leaf.current {
                    paint_wash(dc, r, dpi, blend(ground, crate::basket::CERULEAN, 0.18), ground);
                    fill(dc, RECT { right: r.left + bar, ..r }, crate::basket::CERULEAN);
                    let mark = RECT { left: r.left + bar, right: r.left + bar + mark_w, ..r };
                    text(dc, mark, "\u{E73E}", st.icon, crate::basket::CERULEAN, DT_CENTER);
                    paint_marked(dc, text_r, label, st.bold, &st.query, ground);
                } else {
                    paint_chip(dc, r, dpi, ground);
                    paint_marked(dc, text_r, label, st.small, &st.query, ground);
                }
            }
        }
    }
}

/// Collapsed tab: one folder chip and, when other pane folders exist, a `+N` badge.
unsafe fn paint_summary(dc: HDC, row: &Row, st: &PaintSnap, ground: u32) {
    let dpi = st.dpi;
    let summary = summary_text(row);
    let tint = summary_target(row).2;
    for spot in st.chips.iter().filter(|c| c.tab == row.index) {
        let r = spot.rect;
        if r.bottom <= st.list.top || r.top >= st.list.bottom {
            continue;
        }
        paint_chip(dc, r, dpi, ground);
        let d = sc(dpi, 7);
        let mid = (r.top + r.bottom) / 2;
        let dot = RECT { left: r.left + sc(dpi, 4), top: mid - d / 2, right: r.left + sc(dpi, 4) + d, bottom: mid + d / 2 };
        ellipse(dc, dot, chip_dot(tint), None);
        let text_r = RECT { left: dot.right + sc(dpi, 3), right: r.right - sc(dpi, 4), ..r };
        paint_marked(dc, text_r, &summary, st.small, &st.query, ground);
    }
    let Some(label) = extra_label(row.extra) else { return };
    let dark = ui_tokens().dark;
    let wash = if dark { blend(ground, crate::basket::CERULEAN, 0.45) } else { blend(ground, crate::basket::CERULEAN, 0.20) };
    let ink = if dark { crate::basket::CERULEAN_MIST } else { crate::basket::CERULEAN_DEEP };
    for b in st.badges.iter().filter(|b| b.tab == row.index) {
        let r = b.rect;
        if r.bottom <= st.list.top || r.top >= st.list.bottom || r.right <= r.left {
            continue;
        }
        paint_wash(dc, r, dpi, wash, ground);
        text(dc, r, &label, st.small, ink, DT_CENTER);
    }
}

/// Filter matches draw in cerulean. A path that does not fit keeps its last segment.
unsafe fn paint_marked(dc: HDC, r: RECT, s: &str, font: HFONT, query: &str, ground: u32) {
    if r.right <= r.left || s.is_empty() {
        return;
    }
    let path = s.contains('\\') || s.contains('/');
    let avail = r.right - r.left;
    let measure = |t: &str| unsafe { text_px(dc, font, t) };
    let runs = if path { fit_marked_path(s, query, avail, measure) } else { fit_marked_end(&mark_runs(s, query), avail, measure) };
    let t = ui_tokens();
    if runs.iter().all(|run| !run.hit) {
        if path {
            text_path(dc, r, s, font, t.text, DT_LEFT);
        } else {
            paint_label(dc, r, s, font);
        }
        return;
    }
    let saved = SaveDC(dc);
    IntersectClipRect(dc, r.left, r.top, r.right, r.bottom);
    let wash = if t.dark { blend(ground, crate::basket::CERULEAN, 0.32) } else { blend(t.card, crate::basket::CERULEAN, 0.28) };
    let ink = if t.dark { crate::basket::CERULEAN_MIST } else { crate::basket::CERULEAN_DEEP };
    let mut x = r.left;
    for run in &runs {
        if run.text.is_empty() {
            continue;
        }
        let w = text_px(dc, font, &run.text).max(0);
        if w == 0 {
            continue;
        }
        let piece = RECT { left: x, top: r.top, right: (x + w).min(r.right), bottom: r.bottom };
        if run.hit {
            let m = line_metrics(dc, font);
            let top = ink_line_top(r.top, r.bottom - r.top, m, ink_kind(&run.text));
            let band = RECT { left: x, top, right: x + w, bottom: top + m.height().max(1) };
            fill(dc, band, wash);
            draw_ink_line(dc, piece, &run.text, font, ink, DT_LEFT | DT_NOCLIP);
        } else {
            draw_ink_line(dc, piece, &run.text, font, t.text, DT_LEFT | DT_NOCLIP);
        }
        x += w;
    }
    let _ = RestoreDC(dc, saved);
}

unsafe extern "system" fn edit_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if msg == WM_MOUSEWHEEL {
        let parent = GetParent(h).unwrap_or_default();
        if !parent.0.is_null() {
            return SendMessageW(parent, msg, Some(wp), Some(lp));
        }
    }
    DefSubclassProc(h, msg, wp, lp)
}

pub(crate) extern "system" fn tablist_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| tablist_proc_inner(h, msg, wp, lp)))
}

fn tablist_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        match msg {
            WM_SIZE => {
                layout(h);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                layout(h);
                buffered(h, |dc, rc| paint(dc, rc));
            }
            WM_VSCROLL => on_scroll(wp.0),
            WM_MOUSEWHEEL => {
                let notch = ((wp.0 >> 16) as i16 as i32) / 120;
                let line = sc(with_st(|s| s.dpi), 24);
                let (cur, content, view) = with_st(|s| (s.scroll, s.content_h, (s.list.bottom - s.list.top).max(0)));
                let max = (content - view).max(0);
                scroll_to((cur - notch * line * 3).clamp(0, max));
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let row = hit_row(x, y);
                let btn = hit_btn(x, y);
                let changed = with_st(|s| {
                    let row_changed = s.hot_row != row;
                    s.hot_row = row;
                    let effect = s.track.pointer_move(btn);
                    row_changed || !matches!(effect, TrackEffect::None)
                });
                show_chip_tip(x, y);
                if changed {
                    _ = InvalidateRect(Some(h), None, false);
                }
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                _ = TrackMouseEvent(&mut t);
            }
            WM_MOUSELEAVE => {
                hide_chip_tip();
                let changed = with_st(|s| {
                    let row = s.hot_row.take().is_some();
                    let effect = s.track.pointer_leave();
                    row || !matches!(effect, TrackEffect::None)
                });
                if changed {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some(id) = hit_btn(x, y) {
                    with_st(|s| {
                        s.arm = None;
                        s.track.pointer_down(id);
                    });
                    SetCapture(h);
                    _ = InvalidateRect(Some(h), None, false);
                } else if let Some(tab) = hit_chevron(x, y) {
                    with_st(|s| s.arm = None);
                    apply_fold(tab, None);
                    let edit = with_st(|s| s.edit);
                    if !edit.0.is_null() {
                        _ = SetFocus(Some(edit));
                    }
                } else if let Some(chip) = hit_chip(x, y) {
                    with_st(|s| {
                        s.sel = chip.tab;
                        s.arm = Some(Arm { tab: chip.tab, pane: chip.pane, sub: chip.sub });
                    });
                    SetCapture(h);
                    _ = InvalidateRect(Some(h), None, false);
                } else if let Some(tab) = hit_row(x, y) {
                    with_st(|s| {
                        s.sel = tab;
                        s.arm = None;
                    });
                    _ = InvalidateRect(Some(h), None, false);
                    let edit = with_st(|s| s.edit);
                    if !edit.0.is_null() {
                        _ = SetFocus(Some(edit));
                    }
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let over = hit_btn(x, y);
                let effect = with_st(|s| s.track.pointer_up(over));
                let arm = with_st(|s| s.arm.take());
                if GetCapture() == h {
                    _ = ReleaseCapture();
                }
                let activated = match effect {
                    TrackEffect::Click(GO) => {
                        go();
                        true
                    }
                    TrackEffect::Click(CLOSE_TAB) => {
                        close_selected();
                        false
                    }
                    TrackEffect::Click(DISMISS) => {
                        close();
                        true
                    }
                    _ => match (arm, hit_chip(x, y)) {
                        (Some(arm), Some(chip)) if chip.tab == arm.tab && chip.pane == arm.pane && chip.sub == arm.sub => {
                            go_pane(chip.tab, chip.pane, chip.sub);
                            true
                        }
                        _ => false,
                    },
                };
                if !activated && IsWindow(Some(h)).as_bool() {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONDBLCLK => {
                let (x, y) = xy(lp);
                if hit_chevron(x, y).is_some() {
                    // The first click already toggled the row.
                } else if let Some(chip) = hit_chip(x, y) {
                    go_pane(chip.tab, chip.pane, chip.sub);
                } else if hit_row(x, y).is_some() {
                    go();
                }
            }
            WM_CAPTURECHANGED => {
                let effect = with_st(|s| {
                    s.arm = None;
                    s.track.capture_lost()
                });
                if !matches!(effect, TrackEffect::None) {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_COMMAND => {
                let id = (wp.0 & 0xffff) as i32;
                let code = ((wp.0 >> 16) & 0xffff) as u32;
                if id == ID_EDIT && code == EN_CHANGE {
                    hide_chip_tip();
                    layout(h);
                    _ = InvalidateRect(Some(h), None, false);
                } else if id == ID_NAME && code == EN_KILLFOCUS {
                    let (skip, on) = with_st(|s| {
                        let skip = s.rename_skip;
                        if skip {
                            s.rename_skip = false;
                        }
                        (skip, s.renaming.is_some())
                    });
                    if !skip && on {
                        end_name_edit(true);
                    }
                }
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                return color_field_edit(dc);
            }
            WM_GETMINMAXINFO => {
                let dpi = GetDpiForWindow(h).max(96);
                let mut rc = RECT { left: 0, top: 0, right: sc(dpi as i32, 420), bottom: sc(dpi as i32, 280) };
                let style = WS_OVERLAPPEDWINDOW | WS_VSCROLL | WS_CLIPCHILDREN;
                _ = AdjustWindowRectExForDpi(&mut rc, style, false, WINDOW_EX_STYLE(0), dpi);
                let m = &mut *(lp.0 as *mut MINMAXINFO);
                m.ptMinTrackSize.x = rc.right - rc.left;
                m.ptMinTrackSize.y = rc.bottom - rc.top;
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                apply_faces();
                layout(h);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_CLOSE => {
                end_name_edit(true);
                hide_chip_tip();
                _ = DestroyWindow(h);
            }
            WM_DESTROY => {
                hide_chip_tip();
                let (edit, name) = with_st(|s| (s.edit, s.name_edit));
                if !edit.0.is_null() && IsWindow(Some(edit)).as_bool() {
                    _ = RemoveWindowSubclass(edit, Some(edit_proc), 1);
                }
                if !name.0.is_null() && IsWindow(Some(name)).as_bool() {
                    _ = RemoveWindowSubclass(name, Some(edit_proc), 2);
                }
                with_st(|s| *s = State::new());
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

/// Disclosure arrow at the left of a tab row. Down is open, right is shut.
pub(crate) fn chevron_box(row: RECT, dpi: i32) -> RECT {
    let w = sc(dpi, 16);
    let h = sc(dpi, 16);
    let left = row.left + sc(dpi, 4);
    let mid = (row.top + row.bottom) / 2;
    RECT { left, top: mid - h / 2, right: left + w, bottom: mid + h / 2 }
}

/// Name column after the disclosure arrow and the colour dot.
/// The lock glyph is drawn in this rect when the row is not being renamed.
fn name_text_rect(row: RECT, color: bool, dpi: i32) -> RECT {
    let (_, x1, _, _, _) = cols(RECT { left: row.left, right: row.right, ..row }, dpi);
    let mut left = chevron_box(row, dpi).right + sc(dpi, 2);
    if color {
        let d = sc(dpi, 8);
        left += d + sc(dpi, 4);
    }
    RECT { left, top: row.top, right: (x1 - sc(dpi, 6)).max(left + 1), bottom: row.bottom }
}

/// One-line field behind the rename box, centred on the name. A tall chip row does not stretch it.
fn name_field(row: RECT, color: bool, dpi: i32) -> RECT {
    let text = name_text_rect(row, color, dpi);
    let h = sc(dpi, 22).min((text.bottom - text.top).max(1));
    let top = text.top + (text.bottom - text.top - h) / 2;
    RECT { left: text.left, top, right: text.right, bottom: top + h }
}

/// `None` when the box still shows the label it opened with, so leaving it does not pin an automatic folder name.
/// `Some` is the title to store. Empty clears a custom name.
pub(crate) fn inline_rename_result(opened: &str, typed: &str) -> Option<String> {
    if typed == opened { None } else { Some(stored_tab_name(typed)) }
}

fn name_editing() -> bool {
    let (edit, on) = with_st(|s| (s.name_edit, s.renaming.is_some()));
    on && !edit.0.is_null() && unsafe { GetFocus() } == edit
}

unsafe fn name_edit_key(ch: VIRTUAL_KEY, ctrl: bool, shift: bool, alt: bool) -> bool {
    let edit = with_st(|s| s.name_edit);
    match ch {
        VK_RETURN if !ctrl && !alt && !shift => {
            if ime_composing(edit) {
                return false;
            }
            end_name_edit(true);
            true
        }
        VK_ESCAPE if !ctrl && !alt && !shift => {
            if ime_composing(edit) {
                return false;
            }
            end_name_edit(false);
            true
        }
        _ => false,
    }
}

unsafe fn ensure_name_edit() -> HWND {
    let existing = with_st(|s| s.name_edit);
    if !existing.0.is_null() && IsWindow(Some(existing)).as_bool() {
        return existing;
    }
    let hwnd = with_st(|s| s.hwnd);
    if hwnd.0.is_null() {
        return HWND::default();
    }
    let edit = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("EDIT"),
        w!(""),
        WS_CHILD | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        0,
        0,
        0,
        0,
        Some(hwnd),
        Some(HMENU(ID_NAME as isize as _)),
        None,
        None,
    )
    .unwrap_or_default();
    with_st(|s| s.name_edit = edit);
    if !edit.0.is_null() {
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(bold_ui_font().0 as usize)), Some(LPARAM(1)));
        _ = SetWindowSubclass(edit, Some(edit_proc), 2, 0);
        let dark = DARK_ON.load(Ordering::Relaxed);
        let theme = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
        _ = SetWindowTheme(edit, theme, PCWSTR::null());
    }
    edit
}

unsafe fn place_name_edit() {
    let (edit, tab, list, scroll, rows, shown, dpi) = with_st(|s| (s.name_edit, s.renaming, s.list, s.scroll, s.rows.clone(), s.shown.clone(), s.dpi));
    if edit.0.is_null() {
        return;
    }
    let Some(tab) = tab else {
        if IsWindowVisible(edit).as_bool() {
            _ = ShowWindow(edit, SW_HIDE);
        }
        return;
    };
    let Some(slot) = rows.iter().find(|r| r.tab == tab) else {
        if IsWindowVisible(edit).as_bool() {
            _ = ShowWindow(edit, SW_HIDE);
        }
        return;
    };
    let color = shown.iter().find(|r| r.index == tab).is_some_and(|r| r.color.is_some());
    let row = RECT { left: list.left, top: list.top + slot.y - scroll, right: list.right, bottom: list.top + slot.y - scroll + slot.h };
    let field = name_field(row, color, dpi);
    place_edit(edit, field_edit_rect(field, dpi), true);
}

unsafe fn begin_name_edit() {
    if name_editing() {
        let edit = with_st(|s| s.name_edit);
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        return;
    }
    if with_st(|s| s.renaming.is_some()) {
        end_name_edit(true);
    }
    let h = with_st(|s| s.hwnd);
    if h.0.is_null() {
        return;
    }
    layout(h);
    let sel = with_st(|s| s.sel);
    let label = with_st(|s| s.shown.iter().find(|r| r.index == sel).map(|r| r.name.clone()));
    let Some(label) = label else { return };
    let Some(prev) = app(|a| a.s.tabs.get(sel).map(|t| t.name.clone())) else { return };
    let edit = ensure_name_edit();
    if edit.0.is_null() {
        return;
    }
    hide_chip_tip();
    with_st(|s| {
        s.renaming = Some(sel);
        s.rename_prev = prev;
        s.rename_shown = label.clone();
        s.rename_skip = false;
    });
    SendMessageW(edit, WM_SETFONT, Some(WPARAM(bold_ui_font().0 as usize)), Some(LPARAM(1)));
    _ = SetWindowTextW(edit, &HSTRING::from(label.as_str()));
    layout(h);
    _ = SetFocus(Some(edit));
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn end_name_edit(save: bool) {
    let (edit, which, prev, shown) = with_st(|s| {
        s.rename_skip = !save;
        let which = s.renaming.take();
        let prev = std::mem::take(&mut s.rename_prev);
        let shown = std::mem::take(&mut s.rename_shown);
        (s.name_edit, which, prev, shown)
    });
    let Some(i) = which else {
        with_st(|s| s.rename_skip = false);
        return;
    };
    let typed = if save && !edit.0.is_null() { Some(text_of(edit)) } else { None };
    if !edit.0.is_null() && IsWindowVisible(edit).as_bool() {
        _ = ShowWindow(edit, SW_HIDE);
    }
    if let Some(typed) = typed {
        if inline_rename_result(&shown, &typed).is_some() {
            apply_tab_rename(i, &prev, &typed);
        }
    }
    let (h, filter) = with_st(|s| {
        s.rename_skip = false;
        (s.hwnd, s.edit)
    });
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    // A click in another window already moved focus. Put it back on the filter only when it is still ours.
    let focus = GetFocus();
    let ours = focus.0.is_null() || focus == h || focus == edit || IsChild(h, focus).as_bool();
    if ours && !filter.0.is_null() && focus != filter {
        _ = SetFocus(Some(filter));
    }
    layout(h);
    _ = InvalidateRect(Some(h), None, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_label_is_columns_by_rows_without_spaces() {
        assert_eq!(grid_label(&[2, 2]), "2×2");
        assert_eq!(grid_label(&[2]), "1×2");
        assert_eq!(grid_label(&[1, 1]), "2×1");
        assert_eq!(grid_label(&[1, 1, 1]), "3×1");
        assert_eq!(grid_label(&[1]), "1×1");
        assert_eq!(grid_label(&[3]), "1×3");
        assert_eq!(grid_label(&[3, 3]), "2×3");
        assert_eq!(grid_label(&[2, 2, 2]), "3×2");
        assert_eq!(grid_label(&[1, 3]), "1 | 3");
        assert_eq!(grid_label(&[]), "");
    }

    #[test]
    fn filter_matches_name_folder_and_full_path() {
        let chips = [("docs", r"C:\Work\docs"), ("src", r"D:\Maguro\src")];
        assert!(row_matches("Project", &chips, ""));
        assert!(row_matches("Project", &chips, "proj"));
        assert!(row_matches("Project", &chips, "DOCS"));
        assert!(row_matches("Project", &chips, r"d:\maguro"));
        assert!(!row_matches("Project", &chips, "missing"));
    }

    #[test]
    fn chips_wrap_when_the_next_one_does_not_fit() {
        let placed = place_chips(&[40, 40, 40], 100, 16, 3);
        assert_eq!(placed.len(), 3);
        assert_eq!((placed[0].left, placed[0].top), (0, 0));
        assert_eq!((placed[1].left, placed[1].top), (43, 0));
        assert_eq!((placed[2].left, placed[2].top), (0, 19));
        assert_eq!(placed[2].bottom - placed[2].top, 16);
    }

    #[test]
    fn a_single_folder_shows_its_path_and_a_pane_tab_match_keeps_the_window_tab() {
        assert_eq!(path_text("docs", r"C:\Work\docs"), r"C:\Work\docs");
        assert_eq!(path_text("This PC", "This PC"), "This PC");
        let row = Row {
            index: 3,
            name: "Project".into(),
            locked: false,
            color: None,
            heights: vec![2],
            grid: "1×2".into(),
            panes: 2,
            chips: vec![Chip {
                pane: 0,
                name: "docs".into(),
                tip: r"C:\Work\docs".into(),
                head: "1  docs".into(),
                tint: 0,
                tabs: vec![
                    Leaf { index: 0, name: "docs".into(), tip: r"C:\Work\docs".into(), current: true },
                    Leaf { index: 1, name: "old".into(), tip: r"D:\archive\old".into(), current: false },
                ],
            }],
            active: 0,
            open: true,
            extra: 1,
            current: false,
        };
        assert!(row_ok(&row, "archive"));
        assert!(row_ok(&row, "old"));
        assert!(row_ok(&row, "project"));
        assert!(!row_ok(&row, "missing"));
    }

    #[test]
    fn pane_tabs_indent_under_their_pane_and_a_wide_path_keeps_its_width() {
        let plain = [
            PaneBlock { header: 40, leaves: 0 },
            PaneBlock { header: 40, leaves: 0 },
            PaneBlock { header: 40, leaves: 0 },
        ];
        let hits = place_pane_blocks(&plain, 100, 16, 18, 3, 2, 16);
        let chips = place_chips(&[40, 40, 40], 100, 16, 3);
        assert_eq!(hits.len(), chips.len());
        for (hit, chip) in hits.iter().zip(&chips) {
            assert_eq!(hit.leaf, None);
            assert_eq!((hit.rect.left, hit.rect.top, hit.rect.right, hit.rect.bottom), (chip.left, chip.top, chip.right, chip.bottom));
        }

        // The old chip cap was 108 DIP. A measured path wider than that stays wide, up to the column.
        let wide = place_pane_blocks(&[PaneBlock { header: 400, leaves: 0 }], 500, 16, 18, 3, 2, 16);
        assert_eq!(wide.len(), 1);
        assert_eq!(wide[0].rect.right - wide[0].rect.left, 400);

        let nested = place_pane_blocks(
            &[PaneBlock { header: 40, leaves: 0 }, PaneBlock { header: 50, leaves: 2 }, PaneBlock { header: 30, leaves: 0 }],
            200,
            16,
            18,
            4,
            2,
            16,
        );
        assert_eq!(nested[0].leaf, None);
        assert_eq!((nested[0].rect.left, nested[0].rect.top), (0, 0));
        // The pane with tabs starts on the next row, then its tabs are indented full-width lines.
        assert_eq!(nested[1].leaf, None);
        assert_eq!((nested[1].rect.left, nested[1].rect.top, nested[1].rect.bottom), (0, 20, 36));
        assert_eq!(nested[2].leaf, Some(0));
        assert_eq!((nested[2].rect.left, nested[2].rect.top, nested[2].rect.right, nested[2].rect.bottom), (16, 38, 200, 56));
        assert_eq!(nested[3].leaf, Some(1));
        assert_eq!((nested[3].rect.top, nested[3].rect.bottom), (58, 76));
        assert_eq!(nested[3].group, 1);
        assert_eq!((nested[4].group, nested[4].leaf, nested[4].rect.top), (2, None, 80));
    }

    #[test]
    fn wider_client_gives_the_extra_width_to_the_folder_column() {
        // 8 DIP of window padding and 1 DIP of card border on each side. Name, layout,
        // and pane-count stay fixed, so the 280 DIP from 560 to 840 lands in the folder column.
        let list = |client: i32| RECT { left: 0, top: 0, right: client - 16 - 2, bottom: 100 };
        let folder = |c: (i32, i32, i32, i32, i32)| c.4 - c.3;
        let narrow = cols(list(560), 96);
        let wide = cols(list(840), 96);
        assert_eq!(folder(wide) - folder(narrow), 280);
        assert_eq!((narrow.0, narrow.1, narrow.2, narrow.3), (wide.0, wide.1, wide.2, wide.3));
    }

    #[test]
    fn footer_offers_f2_to_rename() {
        // English is the compiled table. tr() follows the process language, which other tests change.
        let s = crate::langkey::EN[Key::TabListKeys as usize];
        assert!(s.contains("{F2}"), "{s}");
        assert!(s.contains("rename"), "{s}");
        assert!(s.contains("{Right}"), "{s}");
        assert!(s.contains("{Left}"), "{s}");
        assert!(kbd_segs(s).iter().any(|seg| matches!(seg, KbdSeg::Key(k) if k == "F2")));
    }

    fn ini_value(text: &str, key: &str) -> String {
        text.lines()
            .find_map(|line| {
                let rest = line.trim().strip_prefix(key)?.trim_start();
                Some(rest.strip_prefix('=')?.trim().to_string())
            })
            .unwrap_or_default()
    }

    #[test]
    fn footer_names_f2_in_korean_and_japanese() {
        let ko = ini_value(include_str!("../lang/lang.ko.ini"), "tab_list_keys");
        let ja = ini_value(include_str!("../lang/lang.ja.ini"), "tab_list_keys");
        assert!(ko.contains("{F2}"), "{ko}");
        assert!(ko.contains("이름 바꾸기"), "{ko}");
        assert!(ja.contains("{F2}"), "{ja}");
        assert!(ja.contains("名前の変更"), "{ja}");
        for line in [&ko, &ja] {
            assert!(kbd_segs(line).iter().any(|seg| matches!(seg, KbdSeg::Key(k) if k == "F2")), "{line}");
        }
    }

    #[test]
    fn footer_shrinks_the_count_before_clipping_keys() {
        // The client is 840 DIP. 8 DIP of padding on each side leaves 824 for the status bar.
        // The count keeps its width; the keys stay on the right. The extra client width is the folder column.
        let wide_bar = plan_footer(824, 192, &[70, 80, 70, 64], 8);
        assert_eq!(wide_bar.rows, 1);
        assert!(wide_bar.count_on_keys);
        assert_eq!(wide_bar.count_w, 192);
        assert_eq!(wide_bar.key_rows[0].width, 284);

        // The old 560-wide bar (544 DIP) still fits the count beside the keys.
        let plan = plan_footer(544, 192, &[70, 80, 70, 64], 8);
        assert_eq!(plan.rows, 1);
        assert!(plan.count_on_keys);
        assert_eq!(plan.count_w, 192);
        assert_eq!(plan.key_rows[0].width, 284);
        assert_eq!(plan.key_rows[0].end, 4);

        // A long count gives up space. The three key pieces stay on the one row.
        let tight = plan_footer(360, 200, &[100, 100, 100], 8);
        assert_eq!(tight.rows, 1);
        assert!(tight.count_on_keys);
        assert_eq!(tight.count_w, 52);
        assert_eq!(tight.key_rows[0].width, 300);
    }

    #[test]
    fn footer_wraps_keys_instead_of_clipping_them() {
        let plan = plan_footer(150, 80, &[100, 100], 8);
        assert!(!plan.count_on_keys);
        assert!(plan.rows >= 2);
        let placed: Vec<usize> = plan.key_rows.iter().flat_map(|row| row.start..row.end).collect();
        assert_eq!(placed, vec![0, 1]);
        assert!(plan.key_rows.iter().all(|row| row.width <= 150));

        let wide = wrap_runs(&[200, 40], 100);
        assert_eq!(wide.len(), 2);
        assert_eq!(wide[0].width, 200);
        assert_eq!((wide[1].start, wide[1].end), (1, 2));
    }

    #[test]
    fn rename_field_stays_one_line_on_a_tall_row() {
        let row = RECT { left: 0, top: 40, right: 500, bottom: 160 };
        let field = name_field(row, false, 96);
        assert_eq!(field.bottom - field.top, 22);
        assert_eq!(field.top, 40 + (120 - 22) / 2);
        let short = name_field(RECT { left: 0, top: 0, right: 200, bottom: 24 }, true, 96);
        assert_eq!(short.bottom - short.top, 22);
        assert!(short.left > name_text_rect(RECT { left: 0, top: 0, right: 200, bottom: 24 }, false, 96).left);
    }

    #[test]
    fn inline_rename_leaves_an_unedited_label_and_clears_when_emptied() {
        assert_eq!(inline_rename_result("docs", "docs"), None);
        assert_eq!(inline_rename_result("docs", ""), Some(String::new()));
        assert_eq!(inline_rename_result("Work", "Plan"), Some("Plan".into()));
        assert_eq!(inline_rename_result("Work", "  Work  "), Some("Work".into()));
        assert_eq!(inline_rename_result("docs", "1"), Some(String::new()));
        assert_eq!(inline_rename_result("docs", "   "), Some(String::new()));
    }

    #[test]
    fn status_line_splits_key_chips() {
        let segs = kbd_segs("Double-click / {Enter} go · {Del} close");
        assert_eq!(
            segs,
            vec![
                KbdSeg::Text("Double-click / ".into()),
                KbdSeg::Key("Enter".into()),
                KbdSeg::Text(" go · ".into()),
                KbdSeg::Key("Del".into()),
                KbdSeg::Text(" close".into()),
            ]
        );
    }

    #[test]
    fn chip_tip_is_the_full_path_and_this_pc_is_substituted() {
        assert_eq!(chip_name(r"C:\Work\docs\"), "docs");
        assert_eq!(chip_tip(r"C:\Work\docs\"), r"C:\Work\docs\");
        assert!(chip_tip("").is_empty());
        let pc = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
        let this_pc = tr(Key::ThisPc);
        assert_eq!(chip_name(pc), this_pc);
        assert_eq!(chip_tip(pc), this_pc);
        assert!(!chip_tip(pc).contains("20D04FE0"));
    }

    fn fold(index: usize, name: &str, folders: &[(&str, &str)]) -> TabFold {
        TabFold { index, name: name.into(), folders: folders.iter().map(|(n, p)| (n.to_string(), p.to_string())).collect() }
    }

    #[test]
    fn collapsed_rows_show_the_other_folder_count_and_the_current_tab_opens() {
        let tabs = vec![
            fold(0, "Work", &[("docs", r"C:\work\docs"), ("src", r"C:\work\src"), ("notes", r"C:\work\notes")]),
            fold(1, "Photos", &[("a", r"C:\p\a"), ("b", r"C:\p\b"), ("c", r"C:\p\c")]),
            fold(2, "Inbox", &[("in", r"C:\in")]),
        ];
        let rows = visible_fold_rows(&tabs, &BTreeSet::new(), &BTreeSet::new(), "", 0);
        assert_eq!(
            rows,
            vec![
                FoldRow::Open { tab: 0 },
                FoldRow::Collapsed { tab: 1, extra: 2 },
                FoldRow::Collapsed { tab: 2, extra: 0 },
            ]
        );
    }

    #[test]
    fn an_expanded_tab_opens_and_collapsing_the_current_tab_shuts_it() {
        let tabs = vec![fold(0, "Work", &[("docs", r"C:\docs"), ("src", r"C:\src")]), fold(1, "Photos", &[("a", r"C:\a"), ("b", r"C:\b")])];
        let mut expanded = BTreeSet::new();
        let mut collapsed = BTreeSet::new();
        set_fold(1, true, &mut expanded, &mut collapsed);
        let rows = visible_fold_rows(&tabs, &expanded, &collapsed, "", 0);
        assert_eq!(rows[1], FoldRow::Open { tab: 1 });
        set_fold(0, false, &mut expanded, &mut collapsed);
        let rows = visible_fold_rows(&tabs, &expanded, &collapsed, "", 0);
        assert_eq!(rows[0], FoldRow::Collapsed { tab: 0, extra: 1 });
        toggle_fold(0, false, &mut expanded, &mut collapsed);
        let rows = visible_fold_rows(&tabs, &expanded, &collapsed, "", 0);
        assert_eq!(rows[0], FoldRow::Open { tab: 0 });
        assert!(tab_is_open(0, &expanded, &collapsed, false, 0));
        assert!(!tab_is_open(0, &BTreeSet::new(), &BTreeSet::from([0]), false, 0));
    }

    #[test]
    fn a_filter_hides_misses_and_expands_only_the_matches() {
        let tabs = vec![
            fold(0, "Work", &[("docs", r"C:\work\docs")]),
            fold(1, "Cedar", &[("plans", r"C:\cedar\plans"), ("old", r"C:\other")]),
            fold(2, "Desk", &[("notes", r"C:\desk\cedar-notes")]),
            fold(3, "Inbox", &[("in", r"C:\inbox")]),
        ];
        let expanded: BTreeSet<usize> = BTreeSet::from([3]);
        let mut collapsed: BTreeSet<usize> = BTreeSet::from([1]);
        reveal_matches(&tabs, &mut collapsed, "  cedar  ");
        let rows = visible_fold_rows(&tabs, &expanded, &collapsed, "  cedar  ", 0);
        assert_eq!(rows, vec![FoldRow::Open { tab: 1 }, FoldRow::Open { tab: 2 }]);
        set_fold(2, false, &mut BTreeSet::new(), &mut collapsed);
        let rows = visible_fold_rows(&tabs, &expanded, &collapsed, "cedar", 0);
        assert_eq!(rows, vec![FoldRow::Open { tab: 1 }, FoldRow::Collapsed { tab: 2, extra: 0 }]);
    }

    #[test]
    fn representative_folder_is_the_active_panes_current_folder() {
        let folders = [RepFolder { pane: 0, current: true }, RepFolder { pane: 0, current: false }, RepFolder { pane: 1, current: true }];
        assert_eq!(representative_index(&folders, 1), 2);
        assert_eq!(representative_index(&folders, 0), 0);
        assert_eq!(representative_index(&folders, 4), 0);
        assert_eq!(extra_label(0), None);
        assert_eq!(extra_label(2).as_deref(), Some("+2"));
    }

    #[test]
    fn closing_a_tab_shifts_later_expand_state() {
        let mut expanded: BTreeSet<usize> = BTreeSet::from([0, 2, 4]);
        let mut collapsed: BTreeSet<usize> = BTreeSet::from([2, 3]);
        shift_fold_sets(&mut expanded, &mut collapsed, 2);
        assert_eq!(expanded, BTreeSet::from([0, 3]));
        assert_eq!(collapsed, BTreeSet::from([2]));
    }

    #[test]
    fn match_runs_keep_the_original_letters_and_a_long_path_keeps_its_tail() {
        assert_eq!(
            mark_runs("Projects", "pro"),
            vec![MarkedRun { text: "Pro".into(), hit: true }, MarkedRun { text: "jects".into(), hit: false }]
        );
        assert_eq!(mark_runs("Photos", ""), vec![MarkedRun { text: "Photos".into(), hit: false }]);
        assert!(mark_runs("Photos", "zzz").iter().all(|r| !r.hit));
        let two = mark_runs("Cedar and cedar", "CEDAR");
        assert_eq!(two.iter().filter(|r| r.hit).map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["Cedar", "cedar"]);
        let ko = mark_runs("문서함", "문");
        assert_eq!(ko[0], MarkedRun { text: "문".into(), hit: true });
        let path = fit_marked_path(r"C:\work\very\long\cedar", "cedar", 8, |s| s.chars().count() as i32);
        let shown: String = path.iter().map(|r| r.text.as_str()).collect();
        assert!(shown.contains("cedar"), "{shown}");
        assert!(path.iter().any(|r| r.hit && r.text.to_lowercase().contains("cedar")), "{shown}");
        assert!(shown.chars().count() <= 8, "{shown}");
    }

    #[test]
    fn selected_row_uses_the_shared_mist() {
        assert_eq!(row_fill(true, true, false), LIGHT_TOKENS.sel_mist());
        assert_eq!(row_fill(true, false, true), DARK_TOKENS.sel_mist());
        assert_eq!(row_fill(false, false, false), LIGHT_TOKENS.card);
        assert_eq!(row_fill(false, true, false), blend(LIGHT_TOKENS.card, LIGHT_TOKENS.text, 0.04));
        assert_eq!(button_width(10, 96), BTN_MIN_W);
        assert_eq!(button_width(80, 96), 80 + BTN_PAD_X * 2);
    }

    #[test]
    fn the_chevron_sits_ahead_of_the_tab_name() {
        let row = RECT { left: 10, top: 20, right: 400, bottom: 44 };
        let chev = chevron_box(row, 96);
        assert_eq!(chev.right - chev.left, 16);
        assert_eq!(chev.bottom - chev.top, 16);
        assert!(chev.left >= row.left);
        let name = name_text_rect(row, false, 96);
        assert!(name.left >= chev.right);
        let color = name_text_rect(row, true, 96);
        assert!(color.left > name.left);
    }
}
