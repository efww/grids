// Quick search for the pane's filter box. The box still filters the open folder.
// A result list streams from one background walker. A subfolder search asks Everything
// when that setting is on and Everything is running; otherwise it walks. A new query or Esc drops the previous one.
// Everything results are a table: name, path, size, date modified. A column title asks Everything
// to sort every match, not only the rows on screen. Drag a column border to resize it; double-click
// that border and the column returns to its default width. The widths and the sort live in maguro.json.
// Hovering a result shows the full name, folder, exact size and date in the same tooltip as the rest of Maguro.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, Once};
use std::time::{Duration, Instant};

use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

use crate::app::*;
use crate::everything::{self, ev_cols_ok, ev_cols_quant, ev_sort_click, ev_sort_col, ev_sort_ok, forces_everywhere, search_text, without_scope_prefix, MAX_RESULTS};
use crate::i18n::*;
use crate::imports::*;
use crate::session::*;
use crate::ui::*;

use super::{apply_filter, focus_view, hidden, local_directory, navigate, new_pane_tab, set_active, share_path, sync_net_fail};

pub(crate) const FIND_CAP: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FindHit {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) dir: bool,
    pub(crate) size: Option<u64>,
    pub(crate) modified: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindEngine {
    Walk,
    Everything,
}

pub(crate) fn find_engine(recursive: bool, enabled: bool, everything_up: bool) -> FindEngine {
    if recursive && enabled && everything_up { FindEngine::Everything } else { FindEngine::Walk }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FindHitOpen {
    Folder(String),
    File { path: String, folder: String },
}

pub(crate) fn hit_open(path: &str, dir: bool) -> FindHitOpen {
    if dir {
        FindHitOpen::Folder(path.to_string())
    } else {
        let folder = Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        FindHitOpen::File { path: path.to_string(), folder }
    }
}

pub(crate) fn query_parts(q: &str) -> Vec<String> {
    q.split(';').map(str::trim).filter(|t| !t.is_empty()).map(|t| t.to_string()).collect()
}

pub(crate) fn name_matches(name: &str, query: &str) -> bool {
    let parts = query_parts(query);
    !parts.is_empty() && parts.iter().any(|t| term_match(name, t))
}

fn term_match(name: &str, term: &str) -> bool {
    if term.contains(['*', '?']) { wild_ci(name, term) } else { contains_ci(name, term) }
}

fn lower(s: &str) -> Vec<char> {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    let (h, n) = (lower(hay), lower(needle));
    !n.is_empty() && h.windows(n.len()).any(|w| w == n.as_slice())
}

fn wild_ci(name: &str, pat: &str) -> bool {
    wild(&lower(name), &lower(pat))
}

fn wild(n: &[char], p: &[char]) -> bool {
    let (mut i, mut j) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while i < n.len() {
        if j < p.len() && (p[j] == '?' || p[j] == n[i]) {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '*' {
            star = Some(j);
            j += 1;
            mark = i;
        } else if let Some(s) = star {
            j = s + 1;
            mark += 1;
            i = mark;
        } else {
            return false;
        }
    }
    while j < p.len() && p[j] == '*' {
        j += 1;
    }
    j == p.len()
}

pub(crate) fn hit_label(root: &str, hit: &FindHit) -> String {
    let parent = Path::new(&hit.path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let root = root.trim_end_matches('\\');
    let parent_trim = parent.trim_end_matches('\\');
    if parent_trim.eq_ignore_ascii_case(root) || parent.is_empty() {
        return hit.name.clone();
    }
    let rel = parent_trim.get(root.len()..).unwrap_or(parent_trim).trim_start_matches('\\');
    if rel.is_empty() { hit.name.clone() } else { format!("{}  ·  {rel}", hit.name) }
}

pub(crate) fn clamp_sel(sel: i32, n: usize) -> usize {
    if n == 0 { 0 } else { sel.clamp(0, n as i32 - 1) as usize }
}

/// Keep row `sel` inside the scrolled area under the sticky header. `view_h` is that area's height.
pub(crate) fn scroll_to_sel(sel: usize, scroll: i32, view_h: i32, row: i32) -> i32 {
    if row <= 0 || view_h <= 0 {
        return scroll.max(0);
    }
    let top = sel as i32 * row;
    let bot = top + row;
    let mut s = scroll;
    if top < s {
        s = top;
    }
    if bot > s + view_h {
        s = bot - view_h;
    }
    s.max(0)
}

pub(crate) struct Walked {
    pub(crate) more: bool,
    pub(crate) stopped: bool,
}

pub(crate) fn walk_search(root: &Path, query: &str, recursive: bool, show_hidden: bool, cap: usize, stop: &dyn Fn() -> bool, mut emit: impl FnMut(&[FindHit], bool)) -> Walked {
    let mut hits = Vec::new();
    let mut pending = Vec::new();
    let mut more = false;
    let mut stopped = false;
    let mut stack = vec![root.to_path_buf()];
    let mut last = Instant::now();
    let flush = |pending: &mut Vec<FindHit>, hits: &[FindHit], more: bool, emit: &mut dyn FnMut(&[FindHit], bool)| {
        if !pending.is_empty() {
            emit(pending, more && hits.len() >= cap);
            pending.clear();
        }
    };
    while let Some(dir) = stack.pop() {
        if stop() {
            stopped = true;
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut kids = Vec::new();
        for e in rd {
            if stop() {
                stopped = true;
                break;
            }
            let Ok(e) = e else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if name.is_empty() {
                continue;
            }
            let path = e.path();
            if !show_hidden && is_hidden(&path) {
                continue;
            }
            let dir_ent = is_dir_follow(&e);
            if dir_ent && recursive && !is_reparse(&path) {
                kids.push(path.clone());
            }
            if !name_matches(&name, query) {
                continue;
            }
            if hits.len() >= cap {
                more = true;
                stopped = true;
                break;
            }
            let hit = FindHit { name, path: path.to_string_lossy().into_owned(), dir: dir_ent, size: None, modified: None };
            hits.push(hit.clone());
            pending.push(hit);
            if pending.len() >= 24 || last.elapsed() >= Duration::from_millis(50) {
                flush(&mut pending, &hits, more, &mut emit);
                last = Instant::now();
            }
        }
        if stopped {
            break;
        }
        stack.extend(kids);
    }
    flush(&mut pending, &hits, more, &mut emit);
    Walked { more, stopped }
}

/// A directory, including a directory symlink. `is_dir` and `is_symlink` come from `file_type`
/// (they do not follow). `followed` is `metadata` and is only consulted when the entry is a
/// symlink or the type could not be read. A plain file ignores `followed`.
pub(crate) fn entry_is_dir(is_dir: Option<bool>, is_symlink: Option<bool>, followed: Option<bool>) -> bool {
    if is_dir == Some(true) {
        return true;
    }
    if is_symlink == Some(true) || is_dir.is_none() {
        followed == Some(true)
    } else {
        false
    }
}

fn is_dir_follow(e: &std::fs::DirEntry) -> bool {
    let ft = e.file_type().ok();
    let is_dir = ft.as_ref().map(|t| t.is_dir());
    let is_symlink = ft.as_ref().map(|t| t.is_symlink());
    // `metadata` follows the link. Skip it for a real directory or a plain file.
    let need = is_dir != Some(true) && (is_symlink == Some(true) || is_dir.is_none());
    let followed = need.then(|| e.metadata().ok().map(|m| m.is_dir())).flatten();
    entry_is_dir(is_dir, is_symlink, followed)
}

fn is_reparse(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const REPARSE: u32 = 0x400;
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & REPARSE != 0)
}

fn is_hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0)
}

struct FindUi {
    gen: usize,
    query: String,
    root: String,
    recursive: bool,
    hits: Vec<FindHit>,
    more: bool,
    running: bool,
    everywhere: bool,
    // Scope and subfolder flag of the search that produced `hits`. A refresh reuses those
    // hits only when both still match; the pill used to flip and then resume the old search.
    searched_sub: bool,
    searched_all: bool,
    engine: FindEngine,
    searched_sort: u32,
    hot_col: Option<usize>,
    /// Column border being dragged. 0 is name|path, 1 path|size, 2 size|date.
    drag: Option<usize>,
    /// The result tooltip tool is installed on the shared tooltip window.
    tip: bool,
    sel: usize,
    scroll: i32,
    list: isize,
    show: bool,
}

fn hwnd_of(v: isize) -> HWND {
    HWND(v as _)
}

struct Job {
    hwnd: isize,
    gen: usize,
    root: String,
    query: String,
    recursive: bool,
    hidden: bool,
    everything: bool,
    everywhere: bool,
    sort: u32,
}

struct Batch {
    hwnd: isize,
    gen: usize,
    hits: Vec<FindHit>,
    more: bool,
    done: bool,
    engine: FindEngine,
}

fn find_map() -> std::sync::MutexGuard<'static, HashMap<isize, FindUi>> {
    static FIND: std::sync::OnceLock<Mutex<HashMap<isize, FindUi>>> = std::sync::OnceLock::new();
    FIND.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}
static INBOX: Mutex<Vec<Batch>> = Mutex::new(Vec::new());
static JOB: Mutex<Option<Job>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();

fn live_gen(hwnd: isize) -> usize {
    find_map().get(&hwnd).map(|u| u.gen).unwrap_or(0)
}

fn row_h(dpi: i32) -> i32 {
    (22 * dpi.max(96)) / 96
}

pub(crate) unsafe fn drop_find(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }
    let list = {
        let mut map = find_map();
        let list = map.get(&(hwnd.0 as isize)).map(|u| hwnd_of(u.list)).unwrap_or_default();
        if let Some(u) = map.get_mut(&(hwnd.0 as isize)) {
            u.gen = u.gen.wrapping_add(1);
            u.running = false;
            u.show = false;
            u.drag = None;
        }
        map.remove(&(hwnd.0 as isize));
        list
    };
    if !list.0.is_null() {
        tip_drop(list, FIND_TIP);
    }
}

pub(crate) unsafe fn find_cancel(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let mut map = find_map();
    if let Some(u) = map.get_mut(&(hwnd.0 as isize)) {
        u.gen = u.gen.wrapping_add(1);
        u.running = false;
    }
}

pub(crate) unsafe fn find_clear(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let list = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&(hwnd.0 as isize)) else { return };
        u.gen = u.gen.wrapping_add(1);
        u.query.clear();
        u.hits.clear();
        u.more = false;
        u.running = false;
        u.show = false;
        u.sel = 0;
        u.scroll = 0;
        hwnd_of(u.list)
    };
    if !list.0.is_null() {
        hide_find_tip(list, hwnd);
        _ = ShowWindow(list, SW_HIDE);
    }
}

pub(crate) unsafe fn hide_find(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let list = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&(hwnd.0 as isize)) else { return };
        u.show = false;
        hwnd_of(u.list)
    };
    if !list.0.is_null() {
        hide_find_tip(list, hwnd);
        _ = ShowWindow(list, SW_HIDE);
    }
}

pub(crate) unsafe fn focus_is_find(pane: HWND, focus: HWND) -> bool {
    if pane.0.is_null() || focus.0.is_null() {
        return false;
    }
    find_map().get(&(pane.0 as isize)).is_some_and(|u| {
        let list = hwnd_of(u.list);
        !list.0.is_null() && (list == focus || IsChild(list, focus).as_bool())
    })
}

/// The search-result list of this pane has the keyboard focus.
pub(crate) unsafe fn find_focused(i: usize) -> bool {
    let Some(hwnd) = pane_hwnd(i) else { return false };
    focus_is_find(hwnd, GetFocus())
}

/// Put the keyboard on the result list, when that list is on screen.
pub(crate) unsafe fn focus_find_list(i: usize) -> bool {
    let Some(hwnd) = pane_hwnd(i) else { return false };
    let list = {
        let map = find_map();
        let Some(u) = map.get(&(hwnd.0 as isize)) else { return false };
        if !u.show {
            return false;
        }
        hwnd_of(u.list)
    };
    if list.0.is_null() || !IsWindow(Some(list)).as_bool() || !IsWindowVisible(list).as_bool() {
        return false;
    }
    _ = SetFocus(Some(list));
    true
}

/// The next or previous result, or nothing at either end. `sel` outside `0..n` has no neighbour.
pub(crate) fn find_neighbour(sel: usize, n: usize, prev: bool) -> Option<usize> {
    if n == 0 || sel >= n {
        return None;
    }
    if prev { sel.checked_sub(1) } else { (sel + 1 < n).then_some(sel + 1) }
}

/// Neighbour of the current find selection without moving it (next, else previous).
pub(crate) unsafe fn peek_find_neighbour(i: usize) -> Option<FindHit> {
    let hwnd = pane_hwnd(i)?;
    let map = find_map();
    let u = map.get(&(hwnd.0 as isize))?;
    if !u.show {
        return None;
    }
    let next = find_neighbour(u.sel, u.hits.len(), false).or_else(|| find_neighbour(u.sel, u.hits.len(), true))?;
    Some(u.hits[next].clone())
}

/// Point the find list at `path` if it is still a hit (after peek delete advances).
pub(crate) unsafe fn peek_find_select_path(i: usize, path: &str) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let mut map = find_map();
    let Some(u) = map.get_mut(&(hwnd.0 as isize)) else { return };
    if let Some(k) = u.hits.iter().position(|h| h.path == path) {
        u.sel = k;
        let list = hwnd_of(u.list);
        drop(map);
        if !list.0.is_null() {
            _ = InvalidateRect(Some(list), None, false);
        }
    }
}

/// Move the result selection by one row and return that hit. Stays put at either end.
pub(crate) unsafe fn step_find_sel(i: usize, prev: bool) -> Option<FindHit> {
    let hwnd = pane_hwnd(i)?;
    let dpi = app(|a| a.dpi);
    let mut map = find_map();
    let u = map.get_mut(&(hwnd.0 as isize))?;
    if !u.show {
        return None;
    }
    let next = find_neighbour(u.sel, u.hits.len(), prev)?;
    u.sel = next;
    let view_h = list_view_h(hwnd_of(u.list), dpi, sticky_rows(u.engine));
    u.scroll = scroll_to_sel(u.sel, u.scroll, view_h, row_h(dpi));
    let hit = u.hits[u.sel].clone();
    let list = hwnd_of(u.list);
    drop(map);
    if !list.0.is_null() {
        _ = InvalidateRect(Some(list), None, false);
    }
    Some(hit)
}

/// Quick look at the selected result. Nothing is selected: the same beep as an empty file list.
pub(crate) unsafe fn preview_find_sel(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let hit = {
        let map = find_map();
        let Some(u) = map.get(&(hwnd.0 as isize)) else { return };
        if !u.show { None } else { u.hits.get(u.sel).cloned() }
    };
    let Some(hit) = hit else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    super::open_peek_path(i, &hit.path, &hit.name);
}

pub(crate) unsafe fn find_retarget(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let focused = pane_ctl(i, ID_FILTER).is_some_and(|e| GetFocus() == e);
    let showing = find_map().get(&(hwnd.0 as isize)).is_some_and(|u| u.show);
    if focused || showing {
        refresh_find(i);
    }
}

pub(crate) unsafe fn refresh_find(i: usize) {
    refresh_find_in(i, false);
}

/// `force` is a scope or subfolder click. The words and folder can be unchanged; the click still runs.
unsafe fn refresh_find_in(i: usize, force: bool) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let (text, root) = (pane_ctl(i, ID_FILTER).map(|e| text_of(e)).unwrap_or_default(), app(|a| a.s.paths.get(i).cloned().unwrap_or_default()));
    let query = text.trim().to_string();
    if query_parts(&query).is_empty() || !is_fs(&root) {
        find_clear(i);
        return;
    }
    let show_hidden = hidden();
    let sort = app(|a| ev_sort_ok(a.s.ev_sort));
    let resume = {
        let mut map = find_map();
        let u = map.entry(hwnd.0 as isize).or_insert_with(|| FindUi {
            gen: 0,
            query: String::new(),
            root: String::new(),
            recursive: false,
            hits: Vec::new(),
            more: false,
            running: false,
            everywhere: false,
            searched_sub: false,
            searched_all: false,
            engine: FindEngine::Walk,
            searched_sort: 1,
            hot_col: None,
            drag: None,
            tip: false,
            sel: 0,
            scroll: 0,
            list: 0,
            show: false,
        });
        let all = u.everywhere || forces_everywhere(&query);
        let same = same_find(&query, &u.query, &root, &u.root, u.recursive, u.searched_sub, all, u.searched_all, u.running, u.hits.len());
        if !force && reuse_hits(same, sort, u.searched_sort) {
            u.show = true;
            true
        } else {
            false
        }
    };
    if resume {
        show_list(hwnd);
        return;
    }
    let enabled = app(|a| a.s.use_everything);
    if enabled && find_map().get(&(hwnd.0 as isize)).is_some_and(|u| u.recursive) {
        ask_install_everything_once(hwnd);
    }
    let (gen, recursive, everywhere, use_ev) = {
        let mut map = find_map();
        // The install prompt above is modal: the pane may have closed while it was up.
        let Some(u) = map.get_mut(&(hwnd.0 as isize)) else { return };
        u.gen = u.gen.wrapping_add(1);
        u.query = query.clone();
        u.root = root.clone();
        u.hits.clear();
        u.more = false;
        u.running = true;
        u.sel = 0;
        u.scroll = 0;
        u.show = true;
        let up = u.recursive && enabled && everything::running();
        // All drives has nowhere to walk. Try Everything even when the probe raced.
        let all = u.recursive && (u.everywhere || forces_everywhere(&query));
        let use_ev = all || find_engine(u.recursive, enabled, up) == FindEngine::Everything;
        u.engine = if use_ev { FindEngine::Everything } else { FindEngine::Walk };
        u.searched_sub = u.recursive;
        u.searched_all = u.everywhere || forces_everywhere(&query);
        u.searched_sort = sort;
        u.hot_col = None;
        (u.gen, u.recursive, u.everywhere, use_ev)
    };
    // Any other pane's walk is stale once this one starts.
    {
        let mut map = find_map();
        for (h, u) in map.iter_mut() {
            if *h != hwnd.0 as isize && u.running {
                u.gen = u.gen.wrapping_add(1);
                u.running = false;
            }
        }
    }
    show_list(hwnd);
    let job = Job { hwnd: hwnd.0 as isize, gen, root, query, recursive, hidden: show_hidden, everything: use_ev, everywhere, sort };
    {
        let mut slot = JOB.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(job);
    }
    static WORKER: Once = Once::new();
    WORKER.call_once(|| _ = std::thread::Builder::new().name("maguro-find".into()).spawn(find_worker));
    WAKE.notify_one();
}

/// One click on the scope pill, from the caption the user sees.
/// That caption is "all drives" when `everywhere` is set or the query starts with `all:` / `everywhere:`.
/// Turning it off removes the prefix. Leaving the prefix in the edit kept the old caption until a later refresh.
/// `(everywhere, replacement edit text)`.
pub(crate) fn scope_after_click(everywhere: bool, query: &str) -> (bool, Option<String>) {
    if everywhere || forces_everywhere(query) {
        (false, without_scope_prefix(query))
    } else {
        (true, None)
    }
}

/// The in-flight search is already what the pill asked for. Text and folder are not enough:
/// the same words in the other scope must not reuse those hits.
pub(crate) fn same_find(query: &str, prev_query: &str, root: &str, prev_root: &str, sub: bool, searched_sub: bool, all: bool, searched_all: bool, running: bool, hits: usize) -> bool {
    query == prev_query && root.eq_ignore_ascii_case(prev_root) && sub == searched_sub && all == searched_all && (running || hits > 0)
}

/// `same_find` plus the Everything sort. A new column order must ask again.
pub(crate) fn reuse_hits(same: bool, sort: u32, searched_sort: u32) -> bool {
    same && sort == searched_sort
}

/// All drives is Everything or an empty Everything answer. A folder walk must not fill that pill.
pub(crate) fn should_walk(asked_everything: bool, everywhere: bool, everything_answered: bool) -> bool {
    if everywhere {
        return false;
    }
    !(asked_everything && everything_answered)
}

/// A folder-walk batch is stale once this search is all drives, even when the generation still matches.
pub(crate) fn keep_batch(searched_all: bool, engine: FindEngine) -> bool {
    !(searched_all && engine == FindEngine::Walk)
}

/// Everything sends one full snapshot. A walk streams deltas, including a final empty one.
pub(crate) fn merge_hits(prev: &[FindHit], batch: Vec<FindHit>, engine: FindEngine, cap: usize) -> Vec<FindHit> {
    if engine == FindEngine::Everything {
        return batch.into_iter().take(cap).collect();
    }
    let mut v = prev.to_vec();
    let room = cap.saturating_sub(v.len());
    v.extend(batch.into_iter().take(room));
    v
}

pub(crate) unsafe fn toggle_find_scope(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let Some(edit) = pane_ctl(i, ID_FILTER) else { return };
    let text = text_of(edit);
    let cur = find_map().get(&(hwnd.0 as isize)).map(|u| u.everywhere).unwrap_or(false);
    let (everywhere, replace) = scope_after_click(cur, &text);
    {
        let mut map = find_map();
        if let Some(u) = map.get_mut(&(hwnd.0 as isize)) {
            u.everywhere = everywhere;
            u.query.clear();
        }
    }
    _ = KillTimer(Some(hwnd), TIMER_FILTER);
    if let Some(rest) = replace {
        // EN_CHANGE cancels the running search and arms the filter timer. Killed below.
        _ = SetWindowTextW(edit, &HSTRING::from(rest));
    }
    _ = KillTimer(Some(hwnd), TIMER_FILTER);
    refresh_find_in(i, true);
    repaint_find(hwnd);
}

pub(crate) unsafe fn toggle_find_sub(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    {
        let mut map = find_map();
        if let Some(u) = map.get_mut(&(hwnd.0 as isize)) {
            u.recursive = !u.recursive;
            u.query.clear(); // force refresh_find to see a change
        }
    }
    refresh_find_in(i, true);
    repaint_find(hwnd);
}

unsafe fn repaint_find(pane: HWND) {
    let list = find_map().get(&(pane.0 as isize)).map(|u| hwnd_of(u.list)).unwrap_or_default();
    if !list.0.is_null() {
        _ = InvalidateRect(Some(list), None, false);
    }
}

pub(crate) unsafe fn nudge_find(i: usize, delta: i32) -> bool {
    let Some(hwnd) = pane_hwnd(i) else { return false };
    let dpi = app(|a| a.dpi);
    let mut map = find_map();
    let Some(u) = map.get_mut(&(hwnd.0 as isize)) else { return false };
    if !u.show {
        return false;
    }
    u.sel = clamp_sel(u.sel as i32 + delta, u.hits.len());
    let view_h = list_view_h(hwnd_of(u.list), dpi, sticky_rows(u.engine));
    let prev = u.scroll;
    u.scroll = scroll_to_sel(u.sel, u.scroll, view_h, row_h(dpi));
    let scrolled = u.scroll != prev;
    let list = hwnd_of(u.list);
    drop(map);
    if scrolled && !list.0.is_null() {
        hide_find_tip(list, hwnd);
    }
    if !list.0.is_null() {
        _ = InvalidateRect(Some(list), None, false);
    }
    true
}

pub(crate) unsafe fn open_find_sel(i: usize) -> bool {
    let Some(hwnd) = pane_hwnd(i) else { return false };
    let hit = {
        let map = find_map();
        let Some(u) = map.get(&(hwnd.0 as isize)) else { return false };
        if !u.show || u.hits.is_empty() {
            return false;
        }
        u.hits.get(u.sel).cloned()
    };
    let Some(hit) = hit else { return false };
    activate_hit(i, &hit);
    true
}

pub(crate) unsafe fn place_find(pane: HWND) {
    if pane.0.is_null() {
        return;
    }
    let (list, show) = {
        let map = find_map();
        match map.get(&(pane.0 as isize)) {
            Some(u) if u.list != 0 => (hwnd_of(u.list), u.show),
            _ => return,
        }
    };
    if !show {
        _ = ShowWindow(list, SW_HIDE);
        sync_net_fail(pane);
        return;
    }
    let mut rc = RECT::default();
    _ = GetClientRect(pane, &mut rc);
    let view = pane_geo_of(pane, rc.right, rc.bottom, app(|a| a.dpi)).view;
    let (w, h) = ((view.right - view.left).max(0), (view.bottom - view.top).max(0));
    _ = SetWindowPos(list, Some(HWND_TOP), view.left, view.top, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    sync_net_fail(pane);
}

pub(crate) unsafe fn on_find_msg(pane: HWND) {
    let key = pane.0 as isize;
    let batches: Vec<Batch> = {
        let mut boxq = INBOX.lock().unwrap_or_else(|e| e.into_inner());
        let mut keep = Vec::new();
        let mut mine = Vec::new();
        for b in boxq.drain(..) {
            if b.hwnd == key { mine.push(b) } else { keep.push(b) }
        }
        *boxq = keep;
        mine
    };
    if batches.is_empty() {
        return;
    }
    let list = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&key) else { return };
        for b in batches {
            if b.gen != u.gen || !keep_batch(u.searched_all, b.engine) {
                continue;
            }
            let cap = if b.engine == FindEngine::Everything { MAX_RESULTS as usize } else { FIND_CAP };
            u.hits = merge_hits(&u.hits, b.hits, b.engine, cap);
            u.engine = b.engine;
            u.more = b.more || u.hits.len() >= cap;
            if b.done {
                u.running = false;
            }
            u.sel = clamp_sel(u.sel as i32, u.hits.len());
        }
        hwnd_of(u.list)
    };
    if !list.0.is_null() {
        _ = InvalidateRect(Some(list), None, false);
        // Results can change under a pointer that has not moved. Refresh the tip, or hide it.
        match cursor_in_list(list) {
            Some((x, y)) => hover_find_tip(list, pane, x, y),
            None => hide_find_tip(list, pane),
        }
    }
}

unsafe fn cursor_in_list(h: HWND) -> Option<(i32, i32)> {
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_err() || !ScreenToClient(h, &mut pt).as_bool() {
        return None;
    }
    let mut rc = RECT::default();
    if GetClientRect(h, &mut rc).is_err() {
        return None;
    }
    (pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom).then_some((pt.x, pt.y))
}

fn pane_hwnd(i: usize) -> Option<HWND> {
    app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)).filter(|h| !h.0.is_null())
}

fn list_view_h(list: HWND, dpi: i32, sticky: i32) -> i32 {
    let head = row_h(dpi).saturating_mul(sticky.max(1));
    if list.0.is_null() {
        return row_h(dpi) * 8;
    }
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(list, &mut rc) };
    (rc.bottom - head).max(row_h(dpi))
}

unsafe fn show_list(pane: HWND) {
    let list = ensure_list(pane);
    {
        let mut map = find_map();
        if let Some(u) = map.get_mut(&(pane.0 as isize)) {
            u.list = list.0 as isize;
            u.show = true;
        }
    }
    place_find(pane);
}

unsafe fn ensure_list(pane: HWND) -> HWND {
    if let Some(h) = find_map().get(&(pane.0 as isize)).map(|u| hwnd_of(u.list)) {
        if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
            return h;
        }
    }
    static CLASS: Once = Once::new();
    CLASS.call_once(|| {
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        _ = RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(find_proc),
            hInstance: hinst.into(),
            hCursor: arrow,
            lpszClassName: w!("MaguroFind"),
            ..Default::default()
        });
    });
    CreateWindowExW(WINDOW_EX_STYLE(0), w!("MaguroFind"), w!(""), WS_CHILD | WS_CLIPSIBLINGS, 0, 0, 0, 0, Some(pane), None, None, None).unwrap_or_default()
}

/// Enter. A folder is browsed to in this pane. A file is launched and the pane moves to its folder.
/// Double-click does not come here: a file stays in the result list, a folder opens in a new pane tab.
unsafe fn activate_hit(i: usize, hit: &FindHit) {
    let dir = hit.dir || local_directory(&hit.path);
    let open = hit_open(&hit.path, dir);
    clear_find_query(i);
    match open {
        FindHitOpen::Folder(p) => {
            navigate(i, &p);
        }
        FindHitOpen::File { path, folder } => {
            let cur = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
            let share = share_path(&path);
            launch_file(i, &path);
            // A share that was labelled a file can still be a folder. The handoff browses it
            // in this pane; moving to the parent afterwards would undo that.
            if !(share && crate::folderapp::shown_on()) && !folder.is_empty() && !same_dir(&folder, &cur) {
                navigate(i, &folder);
            }
        }
    }
    focus_view(i);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FindDbl {
    Launch(String),
    NewTab(String),
}

/// Double-click. A file is launched and the result list stays. A folder opens in a new tab of this pane.
pub(crate) fn dblclick_hit(path: &str, dir: bool) -> FindDbl {
    if dir { FindDbl::NewTab(path.to_string()) } else { FindDbl::Launch(path.to_string()) }
}

fn launch_file(pane: usize, path: &str) {
    unsafe {
        if local_directory(path) {
            navigate(pane, path);
            return;
        }
        // A UNC folder the walker could not classify still starts our exe. The handoff
        // sees that parent and browses this pane.
        if crate::folderapp::shown_on() {
            crate::folderapp::note_activate(app(|a| a.main.0 as isize), pane, false);
        }
        ShellExecuteW(None, w!("open"), &HSTRING::from(path), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

unsafe fn clear_find_query(i: usize) {
    if let Some(e) = pane_ctl(i, ID_FILTER) {
        _ = SetWindowTextW(e, w!(""));
    }
    if let Some(h) = pane_hwnd(i) {
        // SetWindowText arms the filter timer. Drop it before find_clear hides the list.
        _ = KillTimer(Some(h), TIMER_FILTER);
        _ = KillTimer(Some(h), TIMER_FIND);
    }
    find_clear(i);
    apply_filter(i);
}

unsafe fn open_find_dbl(i: usize, hit: &FindHit) {
    let dir = hit.dir || local_directory(&hit.path);
    match dblclick_hit(&hit.path, dir) {
        FindDbl::Launch(path) => {
            // The filter's hide timer is still pending on a fast double-click. Launching the file
            // moves focus away, and that timer would then take the result list down.
            if let Some(h) = pane_hwnd(i) {
                _ = KillTimer(Some(h), TIMER_FIND);
            }
            launch_file(i, &path);
        }
        FindDbl::NewTab(folder) => {
            clear_find_query(i);
            set_active(i);
            new_pane_tab(i);
            if !navigate(i, &folder) {
                _ = MessageBeep(MB_ICONWARNING);
            }
            focus_view(i);
        }
    }
}

fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

fn publish(hwnd: isize, gen: usize, hits: Vec<FindHit>, more: bool, done: bool, engine: FindEngine) {
    if live_gen(hwnd) != gen {
        return;
    }
    INBOX.lock().unwrap_or_else(|e| e.into_inner()).push(Batch { hwnd, gen, hits, more, done, engine });
    unsafe { _ = PostMessageW(Some(HWND(hwnd as _)), WM_FIND, WPARAM(0), LPARAM(0)) };
}

fn find_worker() {
    loop {
        let job = {
            let mut slot = JOB.lock().unwrap_or_else(|e| e.into_inner());
            while slot.is_none() {
                slot = WAKE.wait(slot).unwrap_or_else(|e| e.into_inner());
            }
            slot.take().unwrap()
        };
        let stop = || live_gen(job.hwnd) != job.gen;
        if stop() {
            continue;
        }
        let all = job.recursive && (job.everywhere || forces_everywhere(&job.query));
        if job.everything || all {
            let mut answered = false;
            if let Some(plan) = search_text(&job.root, &job.query, all, job.hidden) {
                if !stop() {
                    if let Some(list) = everything::search(&plan.text, MAX_RESULTS, job.sort) {
                        answered = true;
                        if !stop() {
                            let hits = list.hits.into_iter().map(|h| FindHit { name: h.name, path: h.path, dir: h.dir, size: h.size, modified: h.modified }).collect();
                            publish(job.hwnd, job.gen, hits, list.more, true, FindEngine::Everything);
                        }
                    }
                }
            }
            // A failed all-drives query must not publish the folder walk under the new pill.
            if !should_walk(true, all, answered) {
                if !answered && !stop() {
                    publish(job.hwnd, job.gen, Vec::new(), false, true, FindEngine::Everything);
                }
                continue;
            }
        }
        let root = PathBuf::from(&job.root);
        let walked = walk_search(&root, &job.query, job.recursive, job.hidden, FIND_CAP, &stop, |delta, more| {
            if !stop() {
                publish(job.hwnd, job.gen, delta.to_vec(), more, false, FindEngine::Walk);
            }
        });
        // A cancel sets stopped without filling the cap. A full cap sets more and is still delivered.
        if !stop() && (!walked.stopped || walked.more) {
            publish(job.hwnd, job.gen, Vec::new(), walked.more, true, FindEngine::Walk);
        }
    }
}

pub(crate) unsafe extern "system" fn find_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { find_proc_inner(h, msg, wp, lp) }))
}
unsafe fn find_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let pane = GetParent(h).unwrap_or_default();
    bind_hwnd(h);
    match msg {
        WM_PAINT => paint_find(h, pane),
        WM_ERASEBKGND => return LRESULT(1),
        WM_SETCURSOR => {
            if (lp.0 & 0xffff) as u32 != HTCLIENT {
                return DefWindowProcW(h, msg, wp, lp);
            }
            let drag = find_map().get(&(pane.0 as isize)).is_some_and(|u| u.drag.is_some());
            let on_border = !drag && {
                let mut p = POINT::default();
                GetCursorPos(&mut p).is_ok() && ScreenToClient(h, &mut p).as_bool() && title_border(h, pane, p.x, p.y).is_some()
            };
            if find_cursor(drag, on_border) == FindCursor::SizeWe {
                _ = SetCursor(Some(LoadCursorW(None, IDC_SIZEWE).unwrap_or_default()));
                return LRESULT(1);
            }
            return DefWindowProcW(h, msg, wp, lp);
        }
        WM_MOUSEMOVE => {
            let (x, y) = crate::xy(lp);
            let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
            _ = TrackMouseEvent(&mut t);
            let drag = find_map().get(&(pane.0 as isize)).and_then(|u| u.drag);
            if let Some(k) = drag {
                resize_border(h, k, x);
                _ = SetCursor(Some(LoadCursorW(None, IDC_SIZEWE).unwrap_or_default()));
                hide_find_tip(h, pane);
            } else {
                hover_find_col(h, pane, x, y);
                hover_find_tip(h, pane, x, y);
            }
        }
        WM_MOUSELEAVE => {
            let changed = {
                let mut map = find_map();
                map.get_mut(&(pane.0 as isize)).and_then(|u| u.hot_col.take()).is_some()
            };
            hide_find_tip(h, pane);
            if changed {
                _ = InvalidateRect(Some(h), None, false);
            }
        }
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) as i16 as i32) / 120;
            scroll_find(pane, -delta * row_h(app(|a| a.dpi)) * 3);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = crate::xy(lp);
            if let Some(k) = title_border(h, pane, x, y) {
                // A border resizes. It does not sort, and it does not move the caret.
                hide_find_tip(h, pane);
                if msg == WM_LBUTTONDBLCLK {
                    if let Some(u) = find_map().get_mut(&(pane.0 as isize)) {
                        u.drag = None;
                    }
                    reset_border(h, k);
                } else {
                    let started = {
                        let mut map = find_map();
                        match map.get_mut(&(pane.0 as isize)) {
                            Some(u) => {
                                u.drag = Some(k);
                                true
                            }
                            None => false,
                        }
                    };
                    if started {
                        SetCapture(h);
                    }
                }
            } else {
                let row = click_find(h, pane, x, y, msg == WM_LBUTTONDBLCLK);
                if row && msg != WM_LBUTTONDBLCLK {
                    // A result row takes keyboard focus. The header, its checkbox, and empty space
                    // leave the caret in the search box. Space and F3 are read in the key loop
                    // once this window is focused, and preview the selected row.
                    _ = SetFocus(Some(h));
                } else if !row {
                    if let Ok(e) = GetDlgItem(Some(pane), ID_FILTER) {
                        _ = SetFocus(Some(e));
                    }
                }
            }
        }
        WM_LBUTTONUP => {
            let ended = find_map().get_mut(&(pane.0 as isize)).and_then(|u| u.drag.take()).is_some();
            if ended {
                _ = ReleaseCapture();
                note_save();
            }
        }
        WM_CAPTURECHANGED => {
            // Alt-tab and the like. The widths already moved with the pointer; keep them.
            let ended = find_map().get_mut(&(pane.0 as isize)).and_then(|u| u.drag.take()).is_some();
            if ended {
                note_save();
            }
        }
        WM_DESTROY => tip_drop(h, FIND_TIP),
        // DefWindowProc turns a right-button up into this. Keyboard (Shift+F10) passes lp = -1.
        WM_CONTEXTMENU => {
            let screen = if lp.0 == -1 {
                let mut rc = RECT::default();
                _ = GetWindowRect(h, &mut rc);
                POINT { x: (rc.left + rc.right) / 2, y: (rc.top + rc.bottom) / 2 }
            } else {
                POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 }
            };
            let y = if lp.0 == -1 {
                None
            } else {
                let mut p = screen;
                ScreenToClient(h, &mut p).as_bool().then_some(p.y)
            };
            hide_find_tip(h, pane);
            find_context(pane, y, screen);
        }
        // Up and Down move the row. Enter opens it. Esc clears the search, as in the box.
        // Space and F3 are not here: the key loop previews the selected row while this window is focused.
        WM_KEYDOWN => {
            let Some(i) = pane_of(pane) else { return DefWindowProcW(h, msg, wp, lp) };
            let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
            let plain = !down(VK_CONTROL) && !down(VK_SHIFT) && !down(VK_MENU);
            match VIRTUAL_KEY(wp.0 as u16) {
                VK_UP if plain => { nudge_find(i, -1); }
                VK_DOWN if plain => { nudge_find(i, 1); }
                VK_PRIOR if plain => { nudge_find(i, -8); }
                VK_NEXT if plain => { nudge_find(i, 8); }
                VK_RETURN => {
                    if !open_find_sel(i) {
                        apply_filter(i);
                        focus_view(i);
                    }
                }
                VK_ESCAPE => {
                    if let Ok(e) = GetDlgItem(Some(pane), ID_FILTER) {
                        _ = SetWindowTextW(e, w!(""));
                    }
                    find_clear(i);
                    apply_filter(i);
                    focus_view(i);
                }
                _ => return DefWindowProcW(h, msg, wp, lp),
            }
        }
        _ => return DefWindowProcW(h, msg, wp, lp),
    }
    LRESULT(0)
}

unsafe fn scroll_find(pane: HWND, dy: i32) {
    let dpi = app(|a| a.dpi);
    let mut map = find_map();
    let Some(u) = map.get_mut(&(pane.0 as isize)) else { return };
    let rows = u.hits.len() as i32 + if u.more { 1 } else { 0 };
    let view = list_view_h(hwnd_of(u.list), dpi, sticky_rows(u.engine));
    let max = (rows * row_h(dpi) - view).max(0);
    let prev = u.scroll;
    u.scroll = (u.scroll + dy).clamp(0, max);
    let scrolled = u.scroll != prev;
    let list = hwnd_of(u.list);
    drop(map);
    if scrolled && !list.0.is_null() {
        hide_find_tip(list, pane);
    }
    if !list.0.is_null() {
        _ = InvalidateRect(Some(list), None, false);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HeaderGeom {
    pub sub: RECT,
    pub scope: Option<RECT>,
    pub status_left: i32,
}

/// Header controls from measured label widths (already in pixels) plus a small DIP pad.
/// The checkbox square sits in `sub` (it starts at 8 DIP and is 14 DIP wide; the label starts at 26).
/// `scope` is the pill, only while the search includes subfolders, inset 3 DIP from the header row.
pub(crate) fn header_geom(dpi: i32, width: i32, header_h: i32, sub_text: i32, scope_text: i32, recursive: bool) -> HeaderGeom {
    let s = |v: i32| v.saturating_mul(dpi.max(1)) / 96;
    let width = width.max(0);
    let sub_right = (s(26) + sub_text.max(0) + s(8)).min(width);
    let sub = RECT { left: 0, top: 0, right: sub_right, bottom: header_h.max(0) };
    if !recursive {
        return HeaderGeom { sub, scope: None, status_left: (width / 2).max(sub_right) };
    }
    let (pad, gap) = (s(8), s(6));
    let left = sub_right + gap;
    let right = (left + scope_text.max(0) + pad * 2).min(width.max(left));
    let top = s(3).min(header_h.max(0));
    let bottom = (header_h - s(3)).max(top);
    HeaderGeom { sub, scope: Some(RECT { left, top, right, bottom }), status_left: right + gap }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeaderHit {
    Sub,
    Scope,
    None,
}

/// Click target in the search-list header. The checkbox square and the Subfolders label
/// are `Sub`. The scope pill is `Scope` only on the rectangle it is drawn in.
/// The gap before the pill and the status text are `None`.
pub(crate) fn header_hit(x: i32, y: i32, g: &HeaderGeom) -> HeaderHit {
    if inside(g.sub, x, y) {
        HeaderHit::Sub
    } else if g.scope.is_some_and(|r| inside(r, x, y)) {
        HeaderHit::Scope
    } else {
        HeaderHit::None
    }
}

/// Row under a click in the result list. `y` is in the list's client area; the header occupies `rh`.
pub(crate) fn row_at(y: i32, scroll: i32, rh: i32, n: usize) -> Option<usize> {
    row_under(y, scroll, rh, rh, n)
}

/// Like [`row_at`], when the sticky header is taller than one result row.
pub(crate) fn row_under(y: i32, scroll: i32, head: i32, row: i32, n: usize) -> Option<usize> {
    if row <= 0 || y < head || n == 0 {
        return None;
    }
    let idx = (y - head + scroll) / row;
    (idx >= 0 && (idx as usize) < n).then_some(idx as usize)
}

fn sticky_rows(engine: FindEngine) -> i32 {
    if engine == FindEngine::Everything { 2 } else { 1 }
}

/// Narrowest column, at 96 DPI. A drag cannot close a column past this.
const EV_COL_MIN_DIP: i32 = 40;

/// X edges of Name, Path, Size, Date modified inside a list `width` px wide.
/// A wide list uses Everything's own proportions. A narrow pane keeps the four columns
/// in order and gives each a share of the width, so the table still fits.
pub(crate) fn ev_col_edges(width: i32, dpi: i32) -> [i32; 5] {
    ev_col_edges_saved(width, dpi, None)
}

/// Same edges when the user has dragged the borders. `cols` is three fractions of the row.
/// A value that does not fit, or a pane too narrow for the 40 DIP minimum, uses the defaults.
pub(crate) fn ev_col_edges_saved(width: i32, dpi: i32, cols: Option<[f64; 3]>) -> [i32; 5] {
    let s = |v: i32| v.saturating_mul(dpi.max(1)) / 96;
    let width = width.max(0);
    let pad = s(6);
    let x0 = pad.min(width);
    let xr = width.saturating_sub(pad).saturating_sub(s(6)).max(x0);
    let span = (xr - x0).max(0);
    let [b0, b1, b2] = ev_borders(span, dpi, cols);
    let x1 = x0.saturating_add(b0).min(xr).max(x0);
    let x2 = x0.saturating_add(b1).min(xr).max(x1);
    let x3 = x0.saturating_add(b2).min(xr).max(x2);
    [x0, x1, x2, x3, xr]
}

fn default_borders(span: i32, dpi: i32) -> [i32; 3] {
    let s = |v: i32| v.saturating_mul(dpi.max(1)) / 96;
    let date_w = s(126).max(span.saturating_mul(16) / 100);
    let size_w = s(80).max(span.saturating_mul(10) / 100);
    if date_w.saturating_add(size_w).saturating_add(s(48)) <= span && span > 0 {
        let b2 = span - date_w;
        let b1 = b2 - size_w;
        let b0 = b1.saturating_mul(42) / 100;
        return [b0, b1, b2];
    }
    [span.saturating_mul(34) / 100, span.saturating_mul(62) / 100, span.saturating_mul(78) / 100]
}

/// 40 DIP, or 0 when the row is too narrow for four columns of that width.
fn col_min_px(span: i32, dpi: i32) -> i32 {
    let want = (EV_COL_MIN_DIP.saturating_mul(dpi.max(1)) / 96).max(1);
    if span >= want.saturating_mul(4) { want } else { 0 }
}

fn clamp_borders(b: [i32; 3], span: i32, m: i32) -> [i32; 3] {
    let b0 = b[0].min(span - 3 * m).max(m);
    let b1 = b[1].min(span - 2 * m).max(b0 + m);
    [b0, b1, b[2].min(span - m).max(b1 + m)]
}

/// The three borders, in px from the left of the row. Saved fractions are kept; every column
/// stays at least 40 DIP wide. A narrow row, or a bad fraction, uses the default widths.
pub(crate) fn ev_borders(span: i32, dpi: i32, cols: Option<[f64; 3]>) -> [i32; 3] {
    if span <= 0 {
        return [0, 0, 0];
    }
    let defaults = default_borders(span, dpi);
    let Some(cols) = cols.and_then(ev_cols_ok) else { return defaults };
    let m = col_min_px(span, dpi);
    if m == 0 {
        return defaults;
    }
    let raw = cols.map(|f| (f * span as f64).round() as i32);
    clamp_borders(raw, span, m)
}

/// Border `k` dragged to `x` px from the row's left. The neighbours stay. None when the row
/// cannot hold four minimum columns, or the new fractions would not be a legal save.
pub(crate) fn ev_drag_cols(cur: [i32; 3], k: usize, x: i32, span: i32, dpi: i32) -> Option<[f64; 3]> {
    if k >= 3 || span <= 0 {
        return None;
    }
    let m = col_min_px(span, dpi);
    if m == 0 {
        return None;
    }
    let mut b = cur;
    let lo = if k == 0 { m } else { b[k - 1].saturating_add(m) };
    let hi = if k == 2 { span.saturating_sub(m) } else { b[k + 1].saturating_sub(m) };
    if hi < lo {
        return None;
    }
    b[k] = x.clamp(lo, hi);
    ev_cols_quant(b.map(|v| v as f64 / span as f64))
}

/// Double-click border `k`: that border goes back to the default layout, so the column it closes
/// returns to its default width. The other borders stay. None when the table is back to the defaults
/// and nothing needs to be stored.
pub(crate) fn ev_reset_border(cols: Option<[f64; 3]>, k: usize, span: i32, dpi: i32) -> Option<[f64; 3]> {
    let defaults = ev_borders(span, dpi, None);
    let Some(cols) = cols.and_then(ev_cols_ok) else { return None };
    if k >= 3 || span <= 0 {
        return Some(cols);
    }
    let m = col_min_px(span, dpi);
    if m == 0 {
        return None;
    }
    let mut cur = ev_borders(span, dpi, Some(cols));
    let lo = if k == 0 { m } else { cur[k - 1].saturating_add(m) };
    let hi = if k == 2 { span.saturating_sub(m) } else { cur[k + 1].saturating_sub(m) };
    if hi < lo {
        return Some(cols);
    }
    cur[k] = defaults[k].clamp(lo, hi);
    if cur == defaults {
        return None;
    }
    let stored = ev_cols_quant(cur.map(|v| v as f64 / span as f64))?;
    (ev_borders(span, dpi, Some(stored)) != defaults).then_some(stored)
}

/// The border under the pointer in the column-title row, within 4 DIP of the line.
pub(crate) fn ev_border_at(edges: [i32; 5], x: i32, y: i32, top: i32, bottom: i32, dpi: i32) -> Option<usize> {
    if y < top || y >= bottom {
        return None;
    }
    let slop = (4 * dpi.max(1) / 96).max(1);
    (0..3).find(|&k| (x - edges[k + 1]).abs() <= slop)
}

/// A press in the column-title row. A border wins over the sort click it sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColPress {
    Border(usize),
    Sort(usize),
    Miss,
}

pub(crate) fn col_press(x: i32, y: i32, width: i32, edges: [i32; 5], top: i32, bottom: i32, dpi: i32) -> ColPress {
    if let Some(k) = ev_border_at(edges, x, y, top, bottom, dpi) {
        return ColPress::Border(k);
    }
    if y >= top && y < bottom {
        if let Some(c) = col_at(width, edges, x) {
            return ColPress::Sort(c);
        }
    }
    ColPress::Miss
}

/// Resize cursor while a border is dragged or the pointer is on one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindCursor {
    Arrow,
    SizeWe,
}

pub(crate) fn find_cursor(dragging: bool, on_border: bool) -> FindCursor {
    if dragging || on_border { FindCursor::SizeWe } else { FindCursor::Arrow }
}

/// What a result tooltip says. Lines are `\r\n`, the same break the other tracking tips use.
/// Size under 1024 bytes is the short form. A larger size adds the exact byte count.
pub(crate) fn ev_tip_text(name: &str, folder: &str, size: Option<u64>, date: &str) -> String {
    let mut t = format!("{name}\r\n{folder}");
    let size = size.map(|b| match b {
        0..=1023 => human(b),
        _ => format!("{} ({} B)", human(b), super::preview::group_digits(b)),
    });
    let facts: Vec<String> = size.into_iter().chain((!date.is_empty()).then(|| date.to_string())).collect();
    if !facts.is_empty() {
        t.push_str("\r\n");
        t.push_str(&facts.join("  ·  "));
    }
    t
}

/// Column under `x`, including the padding at either side of the titles. `None` outside the list.
pub(crate) fn col_at(width: i32, edges: [i32; 5], x: i32) -> Option<usize> {
    if width <= 0 || x < 0 || x >= width {
        return None;
    }
    let mut e = edges;
    e[0] = 0;
    e[4] = width;
    (0..4).find(|&i| x >= e[i] && x < e[i + 1])
}

fn hit_dir(path: &str) -> String {
    let parent = Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    if parent.len() == 2 && parent.as_bytes().get(1) == Some(&b':') {
        format!("{parent}\\")
    } else {
        parent
    }
}

/// A FILETIME (UTC) as local "2026-11-03 14:05".
fn modified_text(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let (mut utc, mut local) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            return String::new();
        }
    }
    format!("{:04}-{:02}-{:02} {:02}:{:02}", local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute)
}

unsafe fn text_px(dc: HDC, font: HFONT, s: &str) -> i32 {
    if s.is_empty() || dc.is_invalid() {
        return 0;
    }
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    let old = SelectObject(dc, font.into());
    let _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    SelectObject(dc, old);
    sz.cx
}

/// `dc` is the paint DC when we already have one. A click measures on a fresh DC.
/// A failed measure falls back to the old fixed text widths so the controls stay clickable.
unsafe fn header_now(h: HWND, dc: Option<HDC>, dpi: i32, header_h: i32, width: i32, recursive: bool, all: bool) -> HeaderGeom {
    let font = app(|a| a.font);
    let sub = tr(crate::Key::Subfolders);
    let scope = if all { tr(crate::Key::AllDrives) } else { tr(crate::Key::ThisFolder) };
    let (release, dc) = match dc {
        Some(dc) => (false, dc),
        None => (true, GetDC(Some(h))),
    };
    let (sub_w, scope_w) = if dc.is_invalid() {
        (0, 0)
    } else {
        (text_px(dc, font, sub), if recursive { text_px(dc, font, scope) } else { 0 })
    };
    if release && !dc.is_invalid() {
        _ = ReleaseDC(Some(h), dc);
    }
    let dip = |v: i32| v * dpi.max(1) / 96;
    let sub_w = if sub_w > 0 { sub_w } else { dip(112) };
    let scope_w = if !recursive || scope_w > 0 { scope_w } else { dip(110) };
    header_geom(dpi, width, header_h, sub_w, scope_w, recursive)
}

/// Result-row tooltip. Same id space as the other tracking tips (lock 7 … list 12).
const FIND_TIP: usize = 13;

unsafe fn saved_edges(h: HWND) -> (i32, i32, [i32; 5]) {
    let (dpi, cols) = app(|a| (a.dpi, a.s.ev_cols));
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let width = rc.right.max(0);
    (dpi, width, ev_col_edges_saved(width, dpi, cols))
}

/// Column border in the title row, or None on the subfolder row, a result, or a walk list.
unsafe fn title_border(h: HWND, pane: HWND, x: i32, y: i32) -> Option<usize> {
    let dpi = app(|a| a.dpi);
    let rh = row_h(dpi);
    let engine = find_map().get(&(pane.0 as isize)).map(|u| u.engine);
    if engine != Some(FindEngine::Everything) {
        return None;
    }
    let (_, width, edges) = saved_edges(h);
    match col_press(x, y, width, edges, rh, rh * 2, dpi) {
        ColPress::Border(k) => Some(k),
        _ => None,
    }
}

unsafe fn resize_border(h: HWND, k: usize, x: i32) {
    let (dpi, cols) = app(|a| (a.dpi, a.s.ev_cols));
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let edges = ev_col_edges_saved(rc.right.max(0), dpi, cols);
    let span = edges[4] - edges[0];
    if span <= 0 {
        return;
    }
    let cur = [edges[1] - edges[0], edges[2] - edges[0], edges[3] - edges[0]];
    let Some(next) = ev_drag_cols(cur, k, x - edges[0], span, dpi) else { return };
    if cols == Some(next) {
        return;
    }
    app(|a| a.s.ev_cols = Some(next));
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn reset_border(h: HWND, k: usize) {
    let (dpi, cols) = app(|a| (a.dpi, a.s.ev_cols));
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let edges = ev_col_edges_saved(rc.right.max(0), dpi, cols);
    let span = (edges[4] - edges[0]).max(0);
    let next = ev_reset_border(cols, k, span, dpi);
    if next == cols {
        return;
    }
    app(|a| a.s.ev_cols = next);
    note_save();
    _ = InvalidateRect(Some(h), None, false);
}

fn find_tip_key(row: usize, path: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64 ^ ((row as u64) << 32);
    for b in path.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

unsafe fn hide_find_tip(list: HWND, pane: HWND) {
    if tip_pumping() || list.0.is_null() || pane.0.is_null() {
        return;
    }
    let installed = find_map().get(&(pane.0 as isize)).is_some_and(|u| u.tip);
    let (now, _) = tip_leave(list, FIND_TIP, installed);
    if let Some(u) = find_map().get_mut(&(pane.0 as isize)) {
        u.tip = now;
    }
}

/// Full name, folder, exact size and date for the Everything row under the pointer.
/// Same tracking tip as the other lists: anchored once, no messages while the row stays, hidden on leave.
unsafe fn hover_find_tip(list: HWND, pane: HWND, _x: i32, y: i32) {
    if tip_pumping() {
        return;
    }
    let dpi = app(|a| a.dpi);
    let rh = row_h(dpi);
    let snap = {
        let map = find_map();
        let Some(u) = map.get(&(pane.0 as isize)) else { return };
        if u.drag.is_some() || u.engine != FindEngine::Everything {
            None
        } else {
            let head = rh * 2;
            let idx = row_under(y, u.scroll, head, rh, u.hits.len());
            let hit = idx.and_then(|i| u.hits.get(i).cloned());
            Some((idx, hit, u.scroll, u.tip, head))
        }
    };
    let Some((idx, hit, scroll, installed, head)) = snap else {
        hide_find_tip(list, pane);
        return;
    };
    let Some((idx, hit)) = idx.zip(hit) else {
        hide_find_tip(list, pane);
        return;
    };
    let mut rc = RECT::default();
    _ = GetClientRect(list, &mut rc);
    let top = head + idx as i32 * rh - scroll;
    let row = RECT { left: 0, top, right: rc.right.max(0), bottom: top + rh };
    let screen = tip_screen_rect(list, row);
    let date = hit.modified.map(modified_text).unwrap_or_default();
    let text = ev_tip_text(&hit.name, &hit_dir(&hit.path), hit.size, &date);
    let (now, _) = tip_enter(list, FIND_TIP, find_tip_key(idx, &hit.path), &text, screen, TipSide::Right, installed);
    if let Some(u) = find_map().get_mut(&(pane.0 as isize)) {
        u.tip = now;
    }
}

unsafe fn hover_find_col(h: HWND, pane: HWND, x: i32, y: i32) {
    let (dpi, width, edges) = saved_edges(h);
    let rh = row_h(dpi);
    let hot = {
        let map = find_map();
        let engine = map.get(&(pane.0 as isize)).map(|u| u.engine);
        // A border is a resize handle, not a column title.
        if engine == Some(FindEngine::Everything) && !matches!(col_press(x, y, width, edges, rh, rh * 2, dpi), ColPress::Border(_)) {
            match col_press(x, y, width, edges, rh, rh * 2, dpi) {
                ColPress::Sort(c) => Some(c),
                _ => None,
            }
        } else {
            None
        }
    };
    let changed = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&(pane.0 as isize)) else { return };
        if u.hot_col == hot {
            false
        } else {
            u.hot_col = hot;
            true
        }
    };
    if changed {
        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
        _ = TrackMouseEvent(&mut t);
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn sort_find_col(i: usize, col: usize) {
    app(|a| a.s.ev_sort = ev_sort_click(a.s.ev_sort, col));
    note_save();
    refresh_find_in(i, true);
}

unsafe fn click_find(list: HWND, pane: HWND, x: i32, y: i32, dbl: bool) -> bool {
    let i = match pane_of(pane) {
        Some(i) => i,
        None => return false,
    };
    let dpi = app(|a| a.dpi);
    let rh = row_h(dpi);
    let (recursive, all, engine) = {
        let map = find_map();
        match map.get(&(pane.0 as isize)) {
            Some(u) => (u.recursive, u.everywhere || forces_everywhere(&u.query), u.engine),
            None => return false,
        }
    };
    if y < rh {
        // CS_DBLCLKS turns the second press into WM_LBUTTONDBLCLK, not another WM_LBUTTONDOWN.
        // Toggling on both flipped the pill back inside the double-click time, so a quick
        // second click looked like the first had done nothing. Toggle once, on the first down.
        if !dbl {
            let mut rc = RECT::default();
            _ = GetClientRect(list, &mut rc);
            let g = header_now(list, None, dpi, rh, rc.right, recursive, all);
            match header_hit(x, y, &g) {
                HeaderHit::Sub => toggle_find_sub(i),
                HeaderHit::Scope => toggle_find_scope(i),
                HeaderHit::None => {}
            }
        }
        return false;
    }
    if engine == FindEngine::Everything && y < rh * 2 {
        // A border is resized by the window proc. The rest of the title row sorts.
        if !dbl && title_border(list, pane, x, y).is_none() {
            let (_, width, edges) = saved_edges(list);
            if let ColPress::Sort(c) = col_press(x, y, width, edges, rh, rh * 2, dpi) {
                sort_find_col(i, c);
            }
        }
        return false;
    }
    let hit = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&(pane.0 as isize)) else { return false };
        let head = rh * sticky_rows(engine);
        let Some(idx) = row_under(y, u.scroll, head, rh, u.hits.len()) else { return false };
        u.sel = idx;
        let hit = u.hits.get(u.sel).cloned();
        let list = hwnd_of(u.list);
        drop(map);
        if !list.0.is_null() {
            _ = InvalidateRect(Some(list), None, false);
        }
        hit
    };
    // True for a result row: the caller focuses this list. A double-click launches a file
    // in place, or opens a folder in a new pane tab. The header returns false above.
    if dbl {
        if let Some(hit) = hit {
            open_find_dbl(i, &hit);
        }
    }
    true
}

/// Right-click (or the keyboard menu) on a result row: the Explorer menu for that full path.
/// `y` is the client y; None is the keyboard menu for the row that is already selected.
unsafe fn find_context(pane: HWND, y: Option<i32>, screen: POINT) {
    let Some(i) = pane_of(pane) else { return };
    let dpi = app(|a| a.dpi);
    let rh = row_h(dpi);
    let hit = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&(pane.0 as isize)) else { return };
        if !u.show || u.hits.is_empty() {
            return;
        }
        if let Some(y) = y {
            let head = rh * sticky_rows(u.engine);
            let Some(idx) = row_under(y, u.scroll, head, rh, u.hits.len()) else { return };
            u.sel = idx;
        }
        let hit = u.hits.get(u.sel).cloned();
        let list = hwnd_of(u.list);
        drop(map);
        if !list.0.is_null() {
            _ = InvalidateRect(Some(list), None, false);
        }
        hit
    };
    let Some(hit) = hit else { return };
    let items = [(hit.path, !hit.dir)];
    _ = crate::shell::show_paths_ctx(i, &items, screen);
}

unsafe fn paint_find(h: HWND, pane: HWND) {
    let (dpi, font, icon_font, small, accent, sort, cols) = app(|a| (a.dpi, a.font, a.icon_font, a.small_font, a.s.accent_now(), a.s.ev_sort, a.s.ev_cols));
    let snap = {
        let map = find_map();
        map.get(&(pane.0 as isize)).map(|u| (u.query.clone(), u.root.clone(), u.recursive, u.everywhere, u.engine, u.hits.clone(), u.more, u.running, u.sel, u.scroll, u.hot_col))
    };
    let Some((query, root, recursive, everywhere, engine, hits, more, running, sel, scroll, hot_col)) = snap else {
        buffered(h, |dc, rc| fill(dc, rc, pal().card));
        return;
    };
    let all = everywhere || forces_everywhere(&query);
    let table = engine == FindEngine::Everything;
    let (sort_col, sort_desc) = ev_sort_col(sort);
    buffered(h, |dc, rc| {
        let rh = row_h(dpi);
        let s = |v: i32| v * dpi / 96;
        let head_px = rh * sticky_rows(engine);
        fill(dc, rc, pal().card);
        let edges = ev_col_edges_saved(rc.right, dpi, cols);
        let mut y = head_px - scroll;
        if hits.is_empty() && !running {
            text(dc, RECT { left: s(8), top: head_px, right: rc.right - s(8), bottom: head_px + rh }, tr(crate::Key::NoMatches), font, pal().sub, DT_LEFT);
        }
        for (i, hit) in hits.iter().enumerate() {
            let r = RECT { left: 0, top: y, right: rc.right, bottom: y + rh };
            if r.bottom <= head_px || r.top >= rc.bottom {
                y += rh;
                continue;
            }
            if i == sel {
                fill(dc, r, blend(pal().card, accent, 0.28));
            }
            if table {
                let cell = |c: usize| RECT { left: edges[c] + s(8), right: edges[c + 1] - s(6), top: r.top, bottom: r.bottom };
                text(dc, cell(0), &hit.name, font, pal().text, DT_LEFT);
                text_path(dc, cell(1), &hit_dir(&hit.path), small, pal().sub, DT_LEFT);
                if let Some(b) = hit.size {
                    text(dc, cell(2), &human(b), small, pal().sub, DT_RIGHT);
                }
                if let Some(t) = hit.modified {
                    text(dc, cell(3), &modified_text(t), small, pal().sub, DT_LEFT);
                }
            } else {
                let label = hit_label(&root, hit);
                text(dc, RECT { left: s(8), right: rc.right - s(8), ..r }, &label, font, pal().text, DT_LEFT);
            }
            y += rh;
        }
        if more {
            let r = RECT { left: s(8), top: y, right: rc.right - s(8), bottom: y + rh };
            if r.bottom > head_px {
                text(dc, r, tr(crate::Key::MoreResultsAreNotShown), font, pal().sub, DT_LEFT);
            }
        }
        // Drawn last, so a scrolled row never covers the controls or the column titles.
        let head = RECT { left: 0, top: 0, right: rc.right, bottom: rh };
        fill(dc, head, pal().bg);
        let g = header_now(h, Some(dc), dpi, rh, rc.right, recursive, all);
        let mark = RECT { left: s(8), top: (rh - s(14)) / 2, right: s(8) + s(14), bottom: (rh - s(14)) / 2 + s(14) };
        fill(dc, mark, if recursive { accent } else { pal().card });
        frame(dc, mark, 1, if recursive { accent } else { pal().line });
        if recursive {
            text(dc, mark, "\u{E73E}", icon_font, on_color(accent), DT_CENTER);
        }
        let sub_label = tr(crate::Key::Subfolders);
        text(dc, RECT { left: s(26), right: g.sub.right, ..head }, sub_label, font, pal().text, DT_LEFT);
        if let Some(pill) = g.scope {
            fill(dc, pill, pal().card);
            frame(dc, pill, 1, if all { accent } else { pal().line });
            let scope = if all { tr(crate::Key::AllDrives) } else { tr(crate::Key::ThisFolder) };
            text(dc, pill, scope, font, if all { accent } else { pal().text }, DT_CENTER);
        }
        let status = find_status(running, hits.len(), more, engine, all && engine == FindEngine::Everything);
        text(dc, RECT { left: g.status_left, right: rc.right - s(8), ..head }, &status, font, pal().sub, DT_RIGHT);
        if table {
            let band = RECT { left: 0, top: rh, right: rc.right, bottom: rh * 2 };
            fill(dc, band, pal().bg);
            let titles = [tr(crate::Key::Name), tr(crate::Key::Path), tr(crate::Key::Size), tr(crate::Key::DateModified)];
            for (c, title) in titles.iter().enumerate() {
                let col = RECT { left: edges[c], top: rh, right: edges[c + 1], bottom: rh * 2 };
                if hot_col == Some(c) {
                    fill(dc, col, pal().pill);
                }
                if c > 0 {
                    fill(dc, RECT { left: col.left, right: col.left + 1, top: col.top + s(4), bottom: col.bottom - s(4) }, pal().line);
                }
                let on = c == sort_col;
                let arrow = if sort_desc { "\u{25BE}" } else { "\u{25B4}" };
                let label = if on { format!("{title} {arrow}") } else { (*title).to_string() };
                let ink = if on || hot_col == Some(c) { pal().text } else { pal().sub };
                text(dc, RECT { left: col.left + s(8), right: col.right - s(6), ..col }, &label, small, ink, if c == 2 { DT_RIGHT } else { DT_LEFT });
                if on {
                    fill(dc, RECT { left: col.left, top: col.bottom - s(2).max(1), right: col.right, bottom: col.bottom }, accent);
                }
            }
            fill(dc, RECT { left: 0, top: band.bottom - 1, right: rc.right, bottom: band.bottom }, pal().line);
        }
    });
}

fn find_status(running: bool, n: usize, more: bool, engine: FindEngine, everywhere: bool) -> String {
    let engine = match engine {
        FindEngine::Everything if everywhere => tr(crate::Key::EverythingAllDrives),
        FindEngine::Everything => "Everything",
        FindEngine::Walk => tr(crate::Key::Folders),
    };
    if running && n == 0 {
        return format!("{engine} · {}", tr(crate::Key::Searching));
    }
    let mut s = format!("{engine} · {n} {}", tr(crate::Key::Results));
    if more {
        s.push_str(" · ");
        s.push_str(tr(crate::Key::More));
    } else if running {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_substring_wildcard_and_either_term() {
        assert!(name_matches("MyReport.PDF", "report"));
        assert!(name_matches("보고서2026.txt", "보고서"));
        assert!(!name_matches("notes.txt", "report"));
        assert!(!name_matches("a.txt", "   ;  "));
        assert!(name_matches("a.txt", "*.TXT"));
        assert!(name_matches("abc", "a*c") && name_matches("abbc", "a*c") && !name_matches("ab", "a*c"));
        assert!(name_matches("abc", "a?c") && !name_matches("ac", "a?c"));
        assert!(name_matches("song.mp3", "*.md;*.mp3"));
        assert!(!name_matches("song.mp3", "*.md;*.txt"));
        assert_eq!(find_engine(false, true, true), FindEngine::Walk);
        assert_eq!(find_engine(true, false, true), FindEngine::Walk);
        assert_eq!(find_engine(true, true, false), FindEngine::Walk);
        assert_eq!(find_engine(true, true, true), FindEngine::Everything);
    }

    #[test]
    fn opening_a_hit_goes_to_the_folder_or_the_file() {
        assert_eq!(hit_open(r"C:\a\b", true), FindHitOpen::Folder(r"C:\a\b".into()));
        assert_eq!(hit_open(r"C:\a\b.txt", false), FindHitOpen::File { path: r"C:\a\b.txt".into(), folder: r"C:\a".into() });
        let hit = FindHit { name: "b.txt".into(), path: r"C:\Work\sub\b.txt".into(), dir: false, size: None, modified: None };
        assert_eq!(hit_label(r"C:\Work", &hit), "b.txt  ·  sub");
        let here = FindHit { name: "b.txt".into(), path: r"C:\Work\b.txt".into(), dir: false, size: None, modified: None };
        assert_eq!(hit_label(r"C:\Work\", &here), "b.txt");
    }

    #[test]
    fn header_fits_the_label_and_the_pill_matches_its_rect() {
        // 96 dpi. 52px is about "하위 폴더", 84px about "모든 드라이브", in the UI font.
        // The old hit ran to 146 and the old pill was 142 wide, with empty space after the words.
        let g = header_geom(96, 800, 22, 52, 84, true);
        assert_eq!(g.sub, RECT { left: 0, top: 0, right: 86, bottom: 22 });
        assert!(g.sub.right <= 146 * 70 / 100);
        let pill = g.scope.unwrap();
        assert_eq!((pill.left, pill.right, pill.top, pill.bottom), (92, 192, 3, 19));
        assert_eq!(pill.right - pill.left, 100); // text 84 + 8 DIP of pad on each side; the old pill was 142
        assert!(pill.left >= g.sub.right);
        assert_eq!(g.status_left, 198);
        assert_eq!(header_hit(0, 10, &g), HeaderHit::Sub);
        assert_eq!(header_hit(8, 10, &g), HeaderHit::Sub);
        assert_eq!(header_hit(85, 10, &g), HeaderHit::Sub);
        assert_eq!(header_hit(86, 10, &g), HeaderHit::None);
        assert_eq!(header_hit(91, 10, &g), HeaderHit::None);
        assert_eq!(header_hit(92, 10, &g), HeaderHit::Scope);
        assert_eq!(header_hit(191, 10, &g), HeaderHit::Scope);
        assert_eq!(header_hit(192, 10, &g), HeaderHit::None);
        // The pill is inset; the row above and below it is not the button.
        assert_eq!(header_hit(100, 0, &g), HeaderHit::None);
        assert_eq!(header_hit(100, 2, &g), HeaderHit::None);
        assert_eq!(header_hit(100, 3, &g), HeaderHit::Scope);
        assert_eq!(header_hit(100, 18, &g), HeaderHit::Scope);
        assert_eq!(header_hit(100, 19, &g), HeaderHit::None);
        assert_eq!(header_hit(-1, 10, &g), HeaderHit::None);
        // No pill until the search includes subfolders. The label stays fitted.
        let flat = header_geom(96, 800, 22, 52, 84, false);
        assert!(flat.scope.is_none());
        assert_eq!(flat.sub.right, 86);
        assert_eq!(flat.status_left, 400);
        assert_eq!(header_hit(92, 10, &flat), HeaderHit::None);
        // 192 dpi doubles the pads and the insets. Text width is already in physical pixels.
        let hi = header_geom(192, 1600, 44, 104, 168, true);
        assert_eq!(hi.sub.right, 172);
        let hi_pill = hi.scope.unwrap();
        assert_eq!((hi_pill.left, hi_pill.right, hi_pill.top, hi_pill.bottom), (184, 384, 6, 38));
        assert_eq!(header_hit(183, 20, &hi), HeaderHit::None);
        assert_eq!(header_hit(184, 20, &hi), HeaderHit::Scope);
        assert_eq!(header_hit(184, 5, &hi), HeaderHit::None);
        // 120 dpi truncates the same way the paint scale does: 26*120/96 = 32, 8*120/96 = 10, 6*120/96 = 7, 3*120/96 = 3.
        let mid = header_geom(120, 1000, 27, 65, 105, true);
        assert_eq!(mid.sub.right, 107);
        let mid_pill = mid.scope.unwrap();
        assert_eq!((mid_pill.left, mid_pill.right, mid_pill.top, mid_pill.bottom), (114, 239, 3, 24));
        assert_eq!(header_hit(113, 10, &mid), HeaderHit::None);
        assert_eq!(header_hit(114, 10, &mid), HeaderHit::Scope);
        assert_eq!(header_hit(238, 10, &mid), HeaderHit::Scope);
        assert_eq!(header_hit(239, 10, &mid), HeaderHit::None);
        assert_eq!(header_hit(114, 10, &header_geom(120, 1000, 27, 65, 105, false)), HeaderHit::None);
    }

    #[test]
    fn scope_click_follows_the_caption_and_a_refresh_keeps_that_scope() {
        assert_eq!(scope_after_click(false, "report"), (true, None));
        assert_eq!(scope_after_click(true, "report"), (false, None));
        // The pill already says "all drives" because of the prefix. One click turns that off
        // and takes the prefix with it, instead of flipping the flag and leaving the caption.
        assert_eq!(scope_after_click(false, "all: report"), (false, Some("report".into())));
        assert_eq!(scope_after_click(true, "  Everywhere: report"), (false, Some("report".into())));
        assert_eq!(scope_after_click(true, "all: report"), (false, Some("report".into())));
        assert!(same_find("report", "report", r"C:\Work", r"c:\work", true, true, false, false, false, 4));
        // Same words, other scope: do not reuse the hits. This is what made the pill snap back.
        assert!(!same_find("report", "report", r"C:\Work", r"C:\Work", true, true, true, false, true, 4));
        assert!(!same_find("report", "report", r"C:\Work", r"C:\Work", false, true, false, false, true, 4));
        assert!(!same_find("report", "other", r"C:\Work", r"C:\Work", true, true, false, false, true, 4));
        assert!(!same_find("report", "report", r"C:\Work", r"D:\", true, true, false, false, true, 4));
        assert!(!same_find("report", "report", r"C:\Work", r"C:\Work", true, true, false, false, false, 0));
        let (to_all, _) = scope_after_click(false, "findme");
        assert!(to_all);
        assert!(!same_find("findme", "findme", r"C:\Work", r"C:\Work", true, true, true, false, false, 3));
        let (to_folder, _) = scope_after_click(true, "findme");
        assert!(!to_folder);
        assert!(!same_find("findme", "findme", r"C:\Work", r"C:\Work", true, true, false, true, false, 1));
        assert!(!should_walk(true, true, true) && !should_walk(true, true, false) && !should_walk(false, true, false));
        assert!(!should_walk(true, false, true));
        assert!(should_walk(true, false, false) && should_walk(false, false, false));
        assert!(!keep_batch(true, FindEngine::Walk));
        assert!(keep_batch(true, FindEngine::Everything) && keep_batch(false, FindEngine::Walk));
        let folder = vec![FindHit { name: "findme_note.txt".into(), path: r"C:\t\findme_note.txt".into(), dir: false, size: None, modified: None }];
        let glyph = vec![FindHit { name: "\u{3131}".into(), path: "\u{3131}".into(), dir: false, size: None, modified: None }];
        let real = vec![
            FindHit { name: "findme_dir".into(), path: r"C:\Users\Jun\dev\maguro-test\search\findme_dir".into(), dir: true, size: None, modified: None },
            FindHit { name: "findme_note.txt".into(), path: r"C:\Users\Jun\dev\maguro-test\search\findme_note.txt".into(), dir: false, size: None, modified: None },
        ];
        assert_eq!(merge_hits(&folder, glyph.clone(), FindEngine::Everything, 10), glyph);
        assert_eq!(merge_hits(&folder, real.clone(), FindEngine::Everything, 10), real);
        let walked = merge_hits(&folder, vec![FindHit { name: "findme_deep.txt".into(), path: r"C:\t\findme_dir\findme_deep.txt".into(), dir: false, size: None, modified: None }], FindEngine::Walk, 10);
        assert_eq!(walked.len(), 2);
        assert_eq!(merge_hits(&folder, Vec::new(), FindEngine::Walk, 10).len(), 1);
    }

    #[test]
    fn double_click_launches_a_file_and_opens_a_folder_in_a_new_tab() {
        assert_eq!(dblclick_hit(r"C:\a\b.txt", false), FindDbl::Launch(r"C:\a\b.txt".into()));
        assert_eq!(dblclick_hit(r"C:\a\Sub", true), FindDbl::NewTab(r"C:\a\Sub".into()));
    }

    #[test]
    fn a_directory_symlink_or_an_unread_type_is_a_folder() {
        assert!(entry_is_dir(Some(true), Some(false), None));
        assert!(entry_is_dir(Some(true), Some(true), None));
        assert!(!entry_is_dir(Some(false), Some(false), Some(true)));
        assert!(entry_is_dir(Some(false), Some(true), Some(true)));
        assert!(!entry_is_dir(Some(false), Some(true), Some(false)));
        assert!(entry_is_dir(None, None, Some(true)));
        assert!(!entry_is_dir(None, None, Some(false)));
        assert!(!entry_is_dir(None, None, None));
    }

    #[test]
    fn result_row_ignores_the_header_and_the_space_below_the_list() {
        assert_eq!(row_at(21, 0, 22, 3), None);
        assert_eq!(row_at(22, 0, 22, 3), Some(0));
        assert_eq!(row_at(43, 0, 22, 3), Some(0));
        assert_eq!(row_at(44, 0, 22, 3), Some(1));
        assert_eq!(row_at(22, 22, 22, 3), Some(1));
        assert_eq!(row_at(88, 0, 22, 3), None);
        assert_eq!(row_at(30, 0, 0, 3), None);
        assert_eq!(row_at(30, 0, 22, 0), None);
        // Everything adds a column-title row. Those 22px are not a result.
        assert_eq!(row_under(22, 0, 44, 22, 3), None);
        assert_eq!(row_under(43, 0, 44, 22, 3), None);
        assert_eq!(row_under(44, 0, 44, 22, 3), Some(0));
        assert_eq!(row_under(66, 0, 44, 22, 3), Some(1));
        assert_eq!(row_under(44, 22, 44, 22, 3), Some(1));
    }

    #[test]
    fn everything_columns_fit_a_narrow_pane_and_a_new_sort_is_not_reused() {
        let wide = ev_col_edges(860, 96);
        assert!(wide.windows(2).all(|p| p[0] < p[1]), "{wide:?}");
        assert_eq!(wide[0], 6);
        assert!(wide[4] <= 860);
        assert!(wide[4] - wide[3] >= 126 && wide[3] - wide[2] >= 80, "{wide:?}");
        let mid = ev_col_edges(628, 96);
        assert!(mid.windows(2).all(|p| p[0] < p[1]) && mid[4] <= 628, "{mid:?}");
        assert!(mid[4] - mid[3] >= 126 && mid[3] - mid[2] >= 80, "{mid:?}");
        let narrow = ev_col_edges(200, 96);
        assert!(narrow.windows(2).all(|p| p[0] < p[1]) && narrow[4] <= 200, "{narrow:?}");
        let hi = ev_col_edges(1720, 192);
        assert!(hi.windows(2).all(|p| p[0] < p[1]) && hi[4] - hi[3] >= 252, "{hi:?}");
        assert_eq!(col_at(860, wide, 0), Some(0));
        assert_eq!(col_at(860, wide, wide[2] + 4), Some(2));
        assert_eq!(col_at(860, wide, 859), Some(3));
        assert_eq!(col_at(860, wide, 860), None);
        assert_eq!(col_at(0, wide, 10), None);
        assert!(reuse_hits(true, 1, 1));
        assert!(!reuse_hits(true, 14, 1));
        assert!(!reuse_hits(false, 6, 6));
        assert_eq!(hit_dir(r"C:\Work\a.txt"), r"C:\Work");
        assert_eq!(hit_dir(r"C:\a.txt"), r"C:\");
        assert_eq!(hit_dir(r"D:\サンプル\📄.txt"), r"D:\サンプル");
    }

    #[test]
    fn column_borders_drag_reset_and_the_row_tip_lists_the_facts() {
        let span = {
            let e = ev_col_edges(860, 96);
            e[4] - e[0]
        };
        assert_eq!(span, 842);
        let d = ev_borders(span, 96, None);
        assert_eq!(ev_col_edges(860, 96)[1], 6 + d[0]);
        // A dragged border is kept as a fraction of the width. Neighbours stay. 40px is the floor.
        let dragged = ev_drag_cols(d, 0, 500, span, 96).unwrap();
        assert_eq!(ev_borders(span, 96, Some(dragged)), [500, d[1], d[2]]);
        assert_eq!(ev_borders(span, 96, ev_drag_cols(d, 0, 9999, span, 96))[0], d[1] - 40);
        assert_eq!(ev_borders(span, 96, ev_drag_cols(d, 2, 9999, span, 96))[2], span - 40);
        assert_eq!(ev_borders(span, 96, ev_drag_cols(d, 0, -50, span, 96))[0], 40);
        // 192 DPI doubles the floor and the hit slop.
        let hi_span = ev_col_edges(1720, 192);
        let hs = hi_span[4] - hi_span[0];
        let hd = ev_borders(hs, 192, None);
        assert_eq!(ev_borders(hs, 192, ev_drag_cols(hd, 0, -10, hs, 192))[0], 80);
        assert_eq!(ev_borders(hs, 192, ev_drag_cols(hd, 2, 99999, hs, 192))[2], hs - 80);
        // Too narrow for four 40px columns: the saved fractions wait, and the proportional layout is used.
        let squeezed = ev_col_edges_saved(100, 96, Some([0.2, 0.5, 0.8]));
        assert_eq!(squeezed, ev_col_edges(100, 96));
        // A wide pane keeps the fractions, and every column stays at least 40px.
        let wide = ev_col_edges_saved(860, 96, Some(dragged));
        assert_eq!(wide[1], 6 + 500);
        assert!(wide.windows(2).all(|p| p[1] - p[0] >= 40), "{wide:?}");
        // The same fractions on a wider row.
        let again = ev_borders(hs, 96, Some(dragged));
        assert_eq!(again[0], (dragged[0] * hs as f64).round() as i32);
        assert!(again.windows(2).all(|p| p[1] - p[0] >= 40) && hs - again[2] >= 40);
        // Double-click the border that moved: that column returns to its default width.
        assert_eq!(ev_reset_border(Some(dragged), 0, span, 96), None);
        assert_eq!(ev_borders(span, 96, ev_reset_border(Some(dragged), 0, span, 96)), d);
        // Another border that was also dragged stays where it is.
        let both_px = ev_borders(span, 96, ev_drag_cols(d, 0, 200, span, 96));
        let both = ev_drag_cols(both_px, 2, 780, span, 96).unwrap();
        let kept = ev_reset_border(Some(both), 0, span, 96).unwrap();
        let back = ev_borders(span, 96, Some(kept));
        assert_eq!(back[0], d[0]);
        assert_eq!(back[2], 780);
        assert_eq!(ev_reset_border(None, 1, span, 96), None);
        // A bad fraction is the default layout.
        assert_eq!(ev_borders(span, 96, Some([0.9, 0.4, 0.2])), d);
        // The border, not the column title, owns the press. Only the title row hits.
        let edges = ev_col_edges_saved(860, 96, Some(dragged));
        assert_eq!(col_press(edges[1], 30, 860, edges, 22, 44, 96), ColPress::Border(0));
        assert_eq!(col_press(edges[1] + 3, 22, 860, edges, 22, 44, 96), ColPress::Border(0));
        assert_eq!(col_press(edges[3] - 4, 43, 860, edges, 22, 44, 96), ColPress::Border(2));
        assert_eq!(col_press(edges[1] + 20, 30, 860, edges, 22, 44, 96), ColPress::Sort(1));
        assert_eq!(col_press(edges[1], 10, 860, edges, 22, 44, 96), ColPress::Miss); // subfolder row
        assert_eq!(col_press(edges[1], 44, 860, edges, 22, 44, 96), ColPress::Miss); // a result row
        let hi_edges = ev_col_edges(1720, 192);
        assert_eq!(ev_border_at(hi_edges, hi_edges[1] + 8, 50, 44, 88, 192), Some(0));
        assert_eq!(ev_border_at(hi_edges, hi_edges[1] + 9, 50, 44, 88, 192), None);
        assert_eq!(find_cursor(false, false), FindCursor::Arrow);
        assert_eq!(find_cursor(true, false), FindCursor::SizeWe);
        assert_eq!(find_cursor(false, true), FindCursor::SizeWe);
        // The tip: whole name, folder, exact bytes, date. Same line break as the other tips.
        assert_eq!(
            ev_tip_text("아주 긴 보고서 이름.pdf", r"D:\문서", Some(1_234_567), "2026-11-03 09:00"),
            "아주 긴 보고서 이름.pdf\r\nD:\\문서\r\n1.2 MB (1,234,567 B)  ·  2026-11-03 09:00"
        );
        assert_eq!(ev_tip_text("아주 긴 보고서 이름.pdf", r"D:\문서", Some(10), ""), "아주 긴 보고서 이름.pdf\r\nD:\\문서\r\n10 B");
        assert_eq!(ev_tip_text("レポート.txt", r"C:\", None, ""), "レポート.txt\r\nC:\\");
        assert_eq!(ev_tip_text("a.bin", r"E:\data", Some(1024), ""), "a.bin\r\nE:\\data\r\n1.0 KB (1,024 B)");
    }

    #[test]
    fn result_neighbour_stops_at_either_end() {
        assert_eq!(find_neighbour(0, 3, true), None);
        assert_eq!(find_neighbour(0, 3, false), Some(1));
        assert_eq!(find_neighbour(1, 3, true), Some(0));
        assert_eq!(find_neighbour(2, 3, false), None);
        assert_eq!(find_neighbour(2, 3, true), Some(1));
        assert_eq!(find_neighbour(0, 0, false), None);
        assert_eq!(find_neighbour(0, 1, false), None);
        assert_eq!(find_neighbour(4, 3, false), None);
    }

    #[test]
    fn selection_stays_in_range_and_in_view() {
        assert_eq!(clamp_sel(-3, 4), 0);
        assert_eq!(clamp_sel(9, 4), 3);
        assert_eq!(clamp_sel(1, 0), 0);
        assert_eq!(scroll_to_sel(0, 40, 100, 20), 0);
        assert_eq!(scroll_to_sel(8, 0, 60, 20), 120);
        assert_eq!(scroll_to_sel(1, 0, 80, 20), 0);
    }

    #[test]
    fn walk_is_case_blind_capped_and_stops() {
        let root = std::env::temp_dir().join(format!("maguro-find-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Sub")).unwrap();
        std::fs::write(root.join("Report.TXT"), b"a").unwrap();
        std::fs::write(root.join("other.md"), b"b").unwrap();
        std::fs::write(root.join("Sub").join("deep.TXT"), b"c").unwrap();
        let mut flat_hits = Vec::new();
        let flat = walk_search(&root, "txt", false, true, 10, &|| false, |d, _| flat_hits.extend_from_slice(d));
        let names = flat_hits.iter().map(|h| h.name.to_lowercase()).collect::<Vec<_>>();
        assert!(names.contains(&"report.txt".to_string()));
        assert!(!names.iter().any(|n| n == "deep.txt"));
        assert!(!flat.more && !flat.stopped);
        let mut deep_hits = Vec::new();
        let deep = walk_search(&root, "*.txt", true, true, 10, &|| false, |d, _| deep_hits.extend_from_slice(d));
        assert_eq!(deep_hits.len(), 2);
        assert!(!deep.more && !deep.stopped);
        let mut cap_hits = Vec::new();
        let cap = walk_search(&root, "*", true, true, 1, &|| false, |d, _| cap_hits.extend_from_slice(d));
        assert_eq!(cap_hits.len(), 1);
        assert!(cap.more);
        let mut stopped_hits = Vec::new();
        let stopped = walk_search(&root, "*", true, true, 10, &|| true, |d, _| stopped_hits.extend_from_slice(d));
        assert!(stopped.stopped);
        assert!(stopped_hits.is_empty());
        _ = std::fs::remove_dir_all(&root);
    }
}

/// The first subfolder search on a PC without Everything asks once whether to install it. Either answer
/// is remembered, so it is never asked again. Settings keeps an install button to do it later.
unsafe fn ask_install_everything_once(owner: HWND) {
    if app(|a| a.s.ev_asked) || everything::running() || !everything::not_installed() {
        return;
    }
    app(|a| a.s.ev_asked = true);
    crate::save_session();
    let q = tr(crate::Key::InstallEverythingMakeSubfolderSearchMuch);
    if MessageBoxW(Some(owner), &HSTRING::from(q), w!("Maguro"), MB_YESNO | MB_ICONQUESTION) == IDYES {
        everything::install_async();
    }
}
#[cfg(test)]
mod utf8_boundary_tests {
    #[test]
    fn japanese_korean_and_emoji_hit_labels_do_not_panic() {
        let hit = super::FindHit {
            name: "샘플.txt".into(),
            path: r"F:\한글폴더\샘플.txt".into(),
            dir: false, size: None, modified: None,
        };
        assert_eq!(super::hit_label(r"F:\한글폴더", &hit), "샘플.txt");
        let nested = super::FindHit {
            name: "📄.txt".into(),
            path: r"D:\サンプル\📁\📄.txt".into(),
            dir: false, size: None, modified: None,
        };
        let label = super::hit_label(r"D:\サンプル", &nested);
        assert!(label.contains("📄.txt"), "{label}");
        let odd = super::FindHit { name: "😀あ".into(), path: r"E:\abcdサ\😀あ".into(), dir: false, size: None, modified: None };
        let label = super::hit_label("abcdefgh", &odd);
        assert!(label.contains("😀あ"), "{label}");
    }
}
