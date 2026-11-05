use std::ops::{Deref, DerefMut};

use crate::imports::*;
use super::*;
use crate::ui::*;
use crate::session::*;

// ---------- app state ----------
// Single UI thread, one or more top-level windows. `app()` is the window bound to the
// message being handled. Rule: never call `app()` (or a helper such as `pane_of`) from
// inside `app()`, and never call Win32/COM that can re-enter a window proc while borrowed.
// Paint and hit-test use `app_try`, which skips the call when the state is already borrowed.

static NEXT_WIN: AtomicUsize = AtomicUsize::new(1);

/// A UNC or mapped-drive navigation still resolving off the UI thread, or the failure left on screen.
#[derive(Clone)]
pub(crate) struct NetUi {
    pub(crate) id: usize,
    pub(crate) path: String,
    /// Timed out, or the share did not answer. The address keeps `path`; the list shows the error.
    pub(crate) bad: bool,
    /// The wait hit `NET_WAIT_MS`. The message names that many seconds.
    pub(crate) timed_out: bool,
    /// Automatic resume retries still unused after this attempt fails.
    /// `None` is a normal open or F5. `Some(0)` is a resume attempt with nothing left to try.
    pub(crate) resume_left: Option<u8>,
}

/// Child PIDLs and view state held while a pane's shell view is rebuilt for a theme change.
/// The PIDLs are owned; dropping the keep frees them.
pub(crate) struct ThemeKeep {
    pub(crate) path: String,
    pub(crate) snap: Option<crate::folderview::ViewSnap>,
    pub(crate) watch: Option<crate::folderview::ViewWatch>,
    /// Selected child PIDLs. Empty when every row was selected (`select_all`).
    pub(crate) sel: Vec<isize>,
    pub(crate) focus: isize,
    pub(crate) select_all: bool,
    pub(crate) top: i32,
    pub(crate) sx: i32,
    pub(crate) sy: i32,
    pub(crate) mode: i32,
    pub(crate) had_focus: bool,
}

impl Drop for ThemeKeep {
    fn drop(&mut self) {
        unsafe { free_theme_pidls(&mut self.sel, &mut self.focus) };
    }
}

/// Scroll position, selection, and focus put back once the rebuilt list has rows.
pub(crate) struct ThemeScroll {
    pub(crate) top: i32,
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) mode: i32,
    pub(crate) tries: u8,
    pub(crate) x_done: bool,
    pub(crate) y_done: bool,
    pub(crate) select_all: bool,
    /// Selected child PIDLs. Empty when every row was selected (`select_all`).
    pub(crate) sel: Vec<isize>,
    pub(crate) focus: isize,
}

impl Drop for ThemeScroll {
    fn drop(&mut self) {
        unsafe { free_theme_pidls(&mut self.sel, &mut self.focus) };
    }
}

unsafe fn free_theme_pidls(sel: &mut Vec<isize>, focus: &mut isize) {
    let focus_pidl = std::mem::replace(focus, 0);
    let focus_shared = focus_pidl != 0 && sel.contains(&focus_pidl);
    for p in sel.drain(..) {
        if p != 0 {
            CoTaskMemFree(Some(p as *const std::ffi::c_void));
        }
    }
    if focus_pidl != 0 && !focus_shared {
        CoTaskMemFree(Some(focus_pidl as *const std::ffi::c_void));
    }
}

pub(crate) struct Pane {
    pub(crate) hwnd: HWND,
    pub(crate) eb: IExplorerBrowser,
    pub(crate) cookie: u32,
    pub(crate) disp: String,
    pub(crate) pending: Option<String>, // columns to put on when the next folder shows (session restore, open in)
    pub(crate) settle: Option<(String, u8)>, // one later width check; the count is unused. Never SetColumns from the timer.
    /// SetColumns / SetViewMode already used for the folder now showing. Automatic re-apply stops at one.
    pub(crate) view_writes: u8,
    /// The user released a column divider in this folder. The width is stored even during quiet/settle.
    pub(crate) user_width: bool,
    /// Folder whose view we last applied here. None while per-folder memory is off.
    pub(crate) view_watch: Option<crate::folderview::ViewWatch>,
    pub(crate) recover: Option<(String, String)>, // missing folder, and the parent opened in its place
    pub(crate) icon: i32,   // current folder in the system image list, -1 = none yet
    pub(crate) ticket: u32, // that lookup; 0 = none asked
    /// UNC or mapped drive being resolved off this thread. Local folders leave it empty.
    pub(crate) net: Option<NetUi>,
    /// Where this browser sits now. Swap updates it; the Explorer callbacks read it.
    pub(crate) slot: std::sync::Arc<AtomicUsize>,
    /// Theme generation this shell view was built under. A mismatch is rebuilt.
    pub(crate) themed: u64,
    /// Set only while this pane is re-browsing the same folder to pick up a new theme.
    pub(crate) theme_keep: Option<ThemeKeep>,
    pub(crate) theme_scroll: Option<ThemeScroll>,
    /// True between OnNavigationPending and Complete/Failed. Rapid folder activates queue here.
    pub(crate) nav_busy: bool,
    /// At most one folder path waiting to browse when `nav_busy` clears.
    pub(crate) nav_pending: Option<String>,
    /// Folder still on screen when a pane-tab switch stored the destination in `paths`
    /// before the browse. `on_nav` records the visit from here. Empty for an ordinary browse,
    /// where `paths` still names the folder being left.
    pub(crate) tab_left: Option<String>,
    /// F5: the up arrow draws as a refresh ring from `.0`, and the glyph returns at `.1`.
    pub(crate) spin: Option<(Instant, Instant)>,
}

pub(crate) struct Desk {
    #[allow(dead_code)] // stable per window; reserved for later window targeting
    pub(crate) id: u64,
    pub(crate) s: Session,
    pub(crate) main: HWND,
    pub(crate) tree: Option<INameSpaceTreeControl>,
    pub(crate) tree_hwnd: HWND,
    pub(crate) side_hwnd: HWND, // favourites list in the left panel
    pub(crate) preview_hwnd: HWND, // right preview pane; follows the active pane
    pub(crate) side_hot: Option<usize>,
    pub(crate) side_sel: Option<usize>,
    pub(crate) side_scroll: i32,
    pub(crate) side_drag: Option<SideDrag>, // node being dragged
    pub(crate) side_drop: Option<(usize, u8)>, // visible row, drop zone
    pub(crate) side_press: Option<(i32, i32, usize)>,
    /// An OLE drop is over the favourites panel. The location tip stays hidden so it cannot take the drop.
    pub(crate) side_external: bool,
    /// An outside drop just landed and the pointer is still inside. The location tip stays
    /// hidden until `WM_MOUSELEAVE`. `side_external` is cleared before the tree changes, so
    /// without this the hover path shows the tip while the pointer has not moved off.
    pub(crate) fav_tip_after_drop: bool,
    /// Set by the first post-drop move inside the panel; OLE leave before that is ignored.
    pub(crate) fav_tip_leave_armed: bool,
    /// Client point where a scroll or a content change hid the favourites tip.
    /// The same point does not show it again until the pointer moves.
    pub(crate) fav_tip_hold: Option<(i32, i32)>,
    /// Identity of the favourites rows last painted. Unchanged pin refreshes keep the tip.
    pub(crate) side_rows_mark: Option<u64>,
    pub(crate) side_edit: HWND,
    pub(crate) side_edit_at: Vec<usize>,
    pub(crate) side_edit_key: Option<crate::NodeKey>,
    pub(crate) tree_input: bool, // a click/key reached the tree since its last selection change
    pub(crate) focus_keep: HWND, // focus when this window last went to the back
    pub(crate) click_nc: bool,   // the activating click was outside the client (title bar)
    pub(crate) focus_wait: bool, // TIMER_FOCUS is running
    pub(crate) font: HFONT,
    pub(crate) icon_font: HFONT,
    pub(crate) bold_font: HFONT, // favourite group labels; created on first paint, freed on DPI change and close
    pub(crate) group_brush: HBRUSH, // favourite group wash; rebuilt when the accent or theme changes
    pub(crate) group_brush_rgb: u32,
    pub(crate) fh: i32, // UI font line height in px
    pub(crate) dpi: i32,
    pub(crate) panes: Vec<Option<Pane>>,
    pub(crate) drag: Option<Hit>,
    pub(crate) pane_press: Option<PanePress>, // header press; a drag only after it moves
    pub(crate) pane_drag: Option<PaneMove>,   // pane grabbed by its header or tab strip
    pub(crate) pane_tab_drag: Option<usize>,  // pane tab being reordered inside its strip
    pub(crate) pane_tab_tip: bool,            // path tooltip for a pane tab is installed
    pub(crate) pane_tab_lock_tip: bool,       // "Locked" tooltip on a pane tab's lock glyph
    pub(crate) drag_hint: String,             // status while Ctrl copies; empty while moving
    pub(crate) pane_over: HWND,               // accent or red preview of the drop region
    pub(crate) gap_hot: Option<Hit>, // splitter under the pointer, drawn in the accent colour
    pub(crate) sizing: bool, // inside the window's own move/size loop
    pub(crate) was_max: bool, // maximized when last not minimized (kept while minimized)
    pub(crate) status: String,
    pub(crate) status_key: String,
    pub(crate) status_base: (String, Option<(u64, bool, bool)>, String), // text before / after the selection size, (file bytes, has folders, partial)
    /// Selected folders whose totals the status line is waiting on. Empty uses the file-size parenthetical.
    pub(crate) status_folders: Vec<String>,
    /// Local file bytes, file count, and a partial flag mixed into the folder-size line.
    pub(crate) status_files: (u64, u64, bool),
    pub(crate) view_menu: HMENU,
    pub(crate) base: Option<Session>, // last loaded/saved (or startup) state; differs -> green dot
    pub(crate) dirty: bool,
    pub(crate) small_font: HFONT,
    pub(crate) load: HWND, // load popup while open
    pub(crate) load_closed: Option<Instant>,
    pub(crate) load_hot: Option<usize>, // card index, or LOAD_SAVE_AS / LOAD_FOLDER / LOAD_DEL + card
    pub(crate) load_btn: Tracker<usize>, // link and delete buttons on the load popup
    pub(crate) load_edit: HWND,         // rename box on a card; made on first use, hidden when done
    pub(crate) load_editing: Option<usize>, // card being renamed
    pub(crate) fav_popup: HWND,
    /// Large favourites window. One per process; owned by this desk when it opened it.
    pub(crate) fav_mgr: HWND,
    pub(crate) fav_closed: Option<Instant>,
    pub(crate) fav_hot: Option<(usize, bool)>, // (row, on its ✕)
    pub(crate) fav_prev: HWND,                 // focus before the popup opened; Esc goes back there
    pub(crate) keys_popup: HWND,               // shortcut window; null while closed
    pub(crate) keys_prev: HWND,                // focus to restore when that window is closed on purpose
    pub(crate) peek: HWND,                     // F3 quick look while open
    pub(crate) peek_pane: usize,               // pane whose focused item it shows
    pub(crate) peek_name: String,
    pub(crate) keys_scroll: i32,
    pub(crate) keys_btn: Tracker<u8>, // close mark on the shortcut popup
    pub(crate) crumb_hot: Option<(HWND, usize)>, // pane, crumb under the pointer
    pub(crate) kids: Option<KidsAsk>,            // a '›' whose subfolder list has not come back yet
    pub(crate) lost: u32,                        // missing-folder notes merged while one is showing
    pub(crate) cards: Vec<(PathBuf, Session, String)>,
    pub(crate) ilist: Option<windows::Win32::UI::Controls::IImageList>, // system small icons, shared, never freed
    pub(crate) drives: Vec<(u8, u32, HWND)>, // tab-row buttons, A→Z: letter, drive type, button
    pub(crate) drive_mask: u32,              // GetLogicalDrives() they were built for
    pub(crate) drive_mark: Option<u8>,       // the active pane's drive, underlined
    pub(crate) hover: HWND, // owner-drawn button under the mouse
    pub(crate) tip: HWND,
    pub(crate) bars: [HWND; 2], // splitter guide bars (vertical, horizontal)
    pub(crate) grid: HWND,      // layout picker popup while open
    pub(crate) reveal_tries: u8, // sidebar reveal passes left
    pub(crate) grid_preview: Vec<usize>,       // layout shown in the picker
    pub(crate) grid_drag: Option<Vec<usize>>, // press-and-drag in progress
    pub(crate) grid_preset: Option<usize>,    // preset tile under the mouse
    pub(crate) grid_closed: Option<Instant>,
    pub(crate) zoom: Option<usize>, // pane maximized over the grid (runtime only, never saved)
    pub(crate) home: Homes,         // cell each off-screen slot last had, so a returning cell gets its pane back
    pub(crate) focus_req: HWND,     // pane to focus once, when the folder set_layout put in its place opens
    pub(crate) note: Option<(String, Instant)>, // status line confirmation and when it expires (runtime only)
    pub(crate) held: Option<u16>,               // key whose auto-repeat is swallowed until release
    pub(crate) match_last: String,              // last pattern typed into the select box
    pub(crate) matcher: Option<MatchBox>,
    pub(crate) drop_hover: HWND,                         // pane whose top bar something openable is being dragged over
    pub(crate) drop_open: Option<(HWND, usize, IShellItem)>, // folder dropped on a top bar, opened by WM_DROP_OPEN
    pub(crate) park: Vec<Vec<Option<Pane>>>,             // hidden browsers, parallel to s.tabs; empty once evicted
    pub(crate) park_lru: Vec<usize>,                     // parked tabs that still have browsers, oldest first
    /// Clean grid and pane-order of each tab, parallel to `s.tabs`. The save dot reads the active one.
    pub(crate) tab_base: Vec<TabBase>,
    pub(crate) save_since: Option<Instant>,              // first unsaved change in this debounce window
    pub(crate) closed: Vec<Tab>,                         // recently closed tabs, newest first (not saved)
    pub(crate) tabs_hwnd: HWND,
    pub(crate) tab_edit: HWND,
    pub(crate) tab_hot: Option<usize>,
    pub(crate) tab_hot_close: bool,
    pub(crate) tab_lock_tip: bool, // tracking tooltip for a locked tab's lock glyph is installed
    pub(crate) tab_path_tip: bool, // tracking tooltip listing that tab's pane paths is installed
    pub(crate) addr_tip: bool,     // tracking tooltip for a truncated address path is installed
    pub(crate) addr_tip_pane: HWND, // pane that address tooltip is showing for; null while hidden
    pub(crate) addr_tip_shown: bool, // that tooltip is on screen (installed does not mean visible)
    pub(crate) fav_tip: bool,       // tracking tooltip for a favourites row is installed
    pub(crate) fav_tip_shown: bool, // that tooltip is on screen (installed does not mean visible)
    /// Live tracking tip (tool, key, text). Same key+text while shown sends no tip messages.
    pub(crate) hover_tip: Option<crate::ui::TipLive>,
    pub(crate) fav_pop_tip: bool,   // tracking tooltip for the favourites window is installed
    pub(crate) tab_hot_plus: bool,
    pub(crate) tab_hot_over: bool, // pointer over the overflow chevron
    pub(crate) tab_scroll: usize,  // first visible window tab while the strip overflows
    pub(crate) tab_drag: Option<usize>,
    pub(crate) tab_press: Option<(usize, i32, i32, bool)>, // tab, x, y, press landed on its close box
    pub(crate) tab_renaming: Option<usize>,
    pub(crate) tab_rename_skip: bool,
    /// Filter text last seen while this window was current. `FILTERS` holds the current window.
    pub(crate) live_filters: [String; MAXP],
    /// `THEME_EPOCH` last applied to this window. An open preview handler reloads when it moves.
    pub(crate) theme_seen: u64,
}

/// The small pattern box over a file list (Num+ / Num−).
pub(crate) struct MatchBox {
    pub(crate) hwnd: HWND,
    pub(crate) edit: HWND,
    pub(crate) pane: usize,
    pub(crate) path: String,
    pub(crate) desel: bool,
}

/// A favourites row the user is dragging. Explorer rows are copies, not tree moves.
#[derive(Clone)]
pub(crate) struct SideDrag {
    pub(crate) id: Vec<usize>,
    pub(crate) kind: RowKind,
    pub(crate) group: bool,
    pub(crate) file: bool,
    pub(crate) path: String,
    pub(crate) name: String,
}

pub(crate) struct App {
    pub(crate) desks: Vec<Desk>,
    pub(crate) cur: usize,
    /// Standing desk when `desks` is empty (last window torn down; quit messages still drain).
    /// Deref hits this instead of panicking on `desks[cur]`.
    fallback: Desk,
    /// A failed maguro.json write is tried again after this instant. Not mid-drag.
    pub(crate) save_after: Option<Instant>,
    pub(crate) hot_on: bool,    // the global hotkey is really registered (s.hotkey is only the wish)
    pub(crate) hot_hwnd: HWND,  // the window that owns that registration
    pub(crate) clip_on: bool,   // the clipboard-path hotkey is really registered (s.clip_hotkey is only the wish)
    pub(crate) clip_hwnd: HWND, // the window that owns that registration
    pub(crate) last_front: usize, // desk that was active most recently
}

impl App {
    /// Live desk for `cur`, or `fallback` when the desk list is empty (teardown).
    fn desk_ref(&self) -> &Desk {
        match self.desks.get(self.cur) {
            Some(d) => d,
            None => self.desks.first().unwrap_or(&self.fallback),
        }
    }

    fn desk_mut(&mut self) -> &mut Desk {
        if self.cur < self.desks.len() {
            return &mut self.desks[self.cur];
        }
        if !self.desks.is_empty() {
            let i = self.desks.len() - 1;
            return &mut self.desks[i];
        }
        &mut self.fallback
    }
}

impl Deref for App {
    type Target = Desk;
    fn deref(&self) -> &Desk {
        self.desk_ref()
    }
}

impl DerefMut for App {
    fn deref_mut(&mut self) -> &mut Desk {
        self.desk_mut()
    }
}

pub(crate) fn blank_desk() -> Desk {
    Desk {
        id: NEXT_WIN.fetch_add(1, Ordering::Relaxed) as u64,
        s: Session::default(),
        main: HWND::default(),
        tree: None,
        tree_hwnd: HWND::default(),
        side_hwnd: HWND::default(),
        preview_hwnd: HWND::default(),
        side_hot: None,
        side_sel: None,
        side_scroll: 0,
        side_drag: None,
        side_drop: None,
        side_press: None,
        side_external: false,
        fav_tip_after_drop: false,
        fav_tip_leave_armed: false,
        fav_tip_hold: None,
        side_rows_mark: None,
        side_edit: HWND::default(),
        side_edit_at: Vec::new(),
        side_edit_key: None,
        tree_input: false,
        focus_keep: HWND::default(),
        click_nc: false,
        focus_wait: false,
        font: HFONT::default(),
        icon_font: HFONT::default(),
        bold_font: HFONT::default(),
        group_brush: HBRUSH::default(),
        group_brush_rgb: 0,
        fh: 16,
        dpi: 96,
        panes: (0..MAXP).map(|_| None).collect(),
        drag: None,
        pane_press: None,
        pane_drag: None,
        pane_tab_drag: None,
        pane_tab_tip: false,
        pane_tab_lock_tip: false,
        drag_hint: String::new(),
        pane_over: HWND::default(),
        gap_hot: None,
        sizing: false,
        was_max: false,
        status: String::new(),
        status_key: String::new(),
        status_base: Default::default(),
        status_folders: Vec::new(),
        status_files: (0, 0, false),
        view_menu: HMENU::default(),
        base: None,
        dirty: false,
        small_font: HFONT::default(),
        load: HWND::default(),
        load_closed: None,
        load_hot: None,
        load_btn: Tracker::new(),
        load_edit: HWND::default(),
        load_editing: None,
        fav_popup: HWND::default(),
        fav_mgr: HWND::default(),
        fav_closed: None,
        fav_hot: None,
        fav_prev: HWND::default(),
        keys_popup: HWND::default(),
        keys_prev: HWND::default(),
        peek: HWND::default(),
        peek_pane: 0,
        peek_name: String::new(),
        keys_scroll: 0,
        keys_btn: Tracker::new(),
        crumb_hot: None,
        kids: None,
        lost: 0,
        cards: Vec::new(),
        ilist: None,
        drives: Vec::new(),
        drive_mask: 0,
        drive_mark: None,
        hover: HWND::default(),
        tip: HWND::default(),
        bars: [HWND::default(); 2],
        grid: HWND::default(),
        reveal_tries: 0,
        grid_preview: Vec::new(),
        grid_drag: None,
        grid_preset: None,
        grid_closed: None,
        zoom: None,
        home: [None; MAXP],
        focus_req: HWND::default(),
        note: None,
        held: None,
        match_last: String::new(),
        matcher: None,
        drop_hover: HWND::default(),
        drop_open: None,
        park: Vec::new(),
        park_lru: Vec::new(),
        tab_base: Vec::new(),
        save_since: None,
        closed: Vec::new(),
        tabs_hwnd: HWND::default(),
        tab_edit: HWND::default(),
        tab_hot: None,
        tab_hot_close: false,
        tab_lock_tip: false,
        tab_path_tip: false,
        addr_tip: false,
        addr_tip_pane: HWND::default(),
        addr_tip_shown: false,
        fav_tip: false,
        fav_tip_shown: false,
        hover_tip: None,
        fav_pop_tip: false,
        tab_hot_plus: false,
        tab_hot_over: false,
        tab_scroll: 0,
        tab_drag: None,
        tab_press: None,
        tab_renaming: None,
        tab_rename_skip: false,
        live_filters: [const { String::new() }; MAXP],
        theme_seen: 0,
    }
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App { desks: vec![blank_desk()], cur: 0, fallback: blank_desk(), save_after: None, hot_on: false, hot_hwnd: HWND::default(), clip_on: false, clip_hwnd: HWND::default(), last_front: 0 });
}

pub(crate) fn no_panes() -> Vec<Option<Pane>> {
    (0..MAXP).map(|_| None).collect()
}

// Per-pane filter spec. Not in APP: the shell view may call IncludeObject from its own thread.
pub(crate) static FILTERS: Mutex<[String; MAXP]> = Mutex::new([const { String::new() }; MAXP]);

// Folder created by Ctrl+Shift+N, waiting for the view to list it so it can be selected and renamed.
pub(crate) struct PendingNew {
    pub(crate) pane: usize,
    pub(crate) name: String,
    pub(crate) left: u8,
}
pub(crate) static PENDING_NEW: Mutex<Option<PendingNew>> = Mutex::new(None);

/// "New folder", then "New folder (2)", "New folder (3)", … The first free name wins.
pub(crate) fn fresh_folder(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut n = 2u32;
    loop {
        let name = format!("{base} ({n})");
        if !taken(&name) || n == 9999 {
            return name;
        }
        n += 1;
    }
}

pub(crate) fn filter_of(i: usize) -> String {
    FILTERS.lock().map(|f| f[i].clone()).unwrap_or_default()
}

/// Shell-view hwnd → filter. IncludeObject can run off the UI thread, so this is not the current window's array.
pub(crate) static FILTER_VIEWS: Mutex<Vec<(isize, String)>> = Mutex::new(Vec::new());

pub(crate) fn note_filter_view(h: HWND, spec: &str) {
    if h.0.is_null() {
        return;
    }
    if let Ok(mut m) = FILTER_VIEWS.lock() {
        m.retain(|(k, _)| *k != h.0 as isize);
        if !spec.is_empty() {
            m.push((h.0 as isize, spec.to_string()));
        }
    }
}

pub(crate) fn clear_filter_view(h: HWND) {
    if h.0.is_null() {
        return;
    }
    if let Ok(mut m) = FILTER_VIEWS.lock() {
        m.retain(|(k, _)| *k != h.0 as isize);
    }
}

pub(crate) fn filter_for_view(h: HWND) -> Option<String> {
    if h.0.is_null() {
        return None;
    }
    FILTER_VIEWS.lock().ok().and_then(|m| m.iter().find(|(k, _)| *k == h.0 as isize).map(|(_, s)| s.clone()))
}

pub(crate) unsafe fn sync_filter_hwnds() {
    for i in 0..MAXP {
        let spec = filter_of(i);
        if let Some(sv) = view_of::<IShellView>(i) {
            if let Ok(h) = sv.GetWindow() {
                note_filter_view(h, &spec);
            }
        }
    }
}

pub(crate) fn window_count() -> usize {
    app(|a| a.desks.len())
}

/// Point `app()` at desk `i`, keeping each window's live filter text with that window.
pub(crate) fn bind_index(i: usize) {
    app(|a| {
        if a.desks.is_empty() || i >= a.desks.len() || i == a.cur {
            return;
        }
        if let Ok(mut f) = FILTERS.lock() {
            let prev = a.cur;
            a.desks[prev].live_filters = f.clone();
            *f = a.desks[i].live_filters.clone();
        }
        a.cur = i;
    });
}

fn desk_owns(d: &Desk, h: HWND) -> bool {
    if h.0.is_null() || d.main.0.is_null() {
        return false;
    }
    if h == d.main || h == d.tabs_hwnd || h == d.tab_edit || h == d.tree_hwnd || h == d.fav_popup || h == d.fav_mgr || h == d.load || h == d.grid || h == d.tip || h == d.keys_popup || h == d.peek || h == d.pane_over || d.matcher.as_ref().is_some_and(|m| m.hwnd == h || m.edit == h) || d.bars.contains(&h) {
        return true;
    }
    if !d.fav_mgr.0.is_null() && unsafe { IsChild(d.fav_mgr, h).as_bool() } {
        return true;
    }
    // The peek popup's preview host is its child, not the main window's.
    if !d.peek.0.is_null() && unsafe { IsChild(d.peek, h).as_bool() } {
        return true;
    }
    if crate::tablist::belongs(d.main, h) {
        return true;
    }
    unsafe { IsChild(d.main, h).as_bool() }
}

/// Make the window that owns `h` current. A settings window is not a child and does not switch.
pub(crate) fn bind_hwnd(h: HWND) -> bool {
    if h.0.is_null() {
        return false;
    }
    let found = app(|a| {
        if a.desks.is_empty() {
            return None;
        }
        if a.cur < a.desks.len() && desk_owns(&a.desks[a.cur], h) {
            return Some(a.cur);
        }
        a.desks.iter().position(|d| desk_owns(d, h))
    });
    if let Some(i) = found {
        bind_index(i);
        true
    } else {
        false
    }
}

/// Drop the current desk. Loads the neighbour's filters. Fonts and the view menu go back to the caller.
pub(crate) fn drop_current_desk() -> (HFONT, HFONT, HFONT, HFONT, HMENU) {
    app(|a| {
        if a.desks.is_empty() {
            return (HFONT::default(), HFONT::default(), HFONT::default(), HFONT::default(), HMENU::default());
        }
        let i = a.cur.min(a.desks.len() - 1);
        let d = a.desks.remove(i);
        let brush = d.group_brush;
        if !brush.is_invalid() {
            unsafe { _ = DeleteObject(brush.into()) };
        }
        // Last desk: leave `desks` empty so `window_count()==0` posts quit. Shell/COM
        // callbacks (UNC browse, extensions) may still run before WM_QUIT; Deref uses
        // `fallback` so those paths do not panic on `desks[cur]`.
        if a.desks.is_empty() {
            a.cur = 0;
            a.fallback = blank_desk();
        } else {
            if a.cur >= a.desks.len() {
                a.cur = a.desks.len() - 1;
            }
            if let Ok(mut f) = FILTERS.lock() {
                *f = a.desks[a.cur].live_filters.clone();
            }
        }
        (d.font, d.icon_font, d.small_font, d.bold_font, d.view_menu)
    })
}

pub(crate) unsafe fn release_desk_handles(font: HFONT, icon: HFONT, small: HFONT, bold: HFONT, menu: HMENU) {
    for f in [font, icon, small, bold] {
        crate::delete_font(f);
    }
    if !menu.0.is_null() {
        _ = DestroyMenu(menu);
    }
}

fn copy_shared(dst: &mut Session, src: &Session) {
    dst.accent = src.accent;
    dst.accent_dark = src.accent_dark;
    dst.dark = src.dark;
    dst.theme = src.theme;
    dst.font_face = src.font_face.clone();
    dst.bar = src.bar;
    dst.drive_buttons = src.drive_buttons;
    dst.stat = src.stat;
    dst.view_new = src.view_new;
    dst.view_icon = src.view_icon;
    dst.start_new = src.start_new;
    dst.start_path = src.start_path.clone();
    dst.ext = src.ext;
    dst.tree_on = src.tree_on;
    dst.preview_on = src.preview_on;
    dst.preview_w = src.preview_w;
    dst.fav_top = src.fav_top;
    dst.pins_open = src.pins_open;
    dst.files_open = src.files_open;
    dst.edge = src.edge;
    dst.confirm = src.confirm;
    dst.del = src.del;
    dst.new_name = src.new_name.clone();
    dst.dropm = src.dropm;
    dst.fast_copy = src.fast_copy;
    dst.layout_new = src.layout_new;
    dst.boot = src.boot;
    dst.boot_file = src.boot_file.clone();
    dst.autosave = src.autosave;
    dst.dot = src.dot;
    dst.update_check = src.update_check;
    dst.dblup = src.dblup;
    dst.space_preview = src.space_preview;
    dst.peek_wheel_nav = src.peek_wheel_nav;
    dst.remember_views = src.remember_views;
    dst.folder_sizes = src.folder_sizes;
    dst.folder_sizes_network = src.folder_sizes_network;
    dst.peek_image = src.peek_image;
    dst.peek_doc = src.peek_doc;
    dst.hotkey = src.hotkey;
    dst.dialog_jump = src.dialog_jump;
    dst.clip_hotkey = src.clip_hotkey;
    dst.lang = src.lang;
    dst.single = src.single;
    dst.one_window = src.one_window;
    dst.favs = src.favs.clone();
    dst.tree = src.tree.clone();
    dst.recent = src.recent.clone();
    dst.ctx_off = src.ctx_off.clone();
    dst.ctx_labels = src.ctx_labels.clone();
    dst.ctx_on = src.ctx_on.clone();
    dst.ctx_quiet = src.ctx_quiet.clone();
    dst.blocked_shell_extensions = src.blocked_shell_extensions.clone();
    dst.file_basket = src.file_basket.clone();
    dst.excel_tidy_a1 = src.excel_tidy_a1;
    dst.excel_tidy_zoom = src.excel_tidy_zoom;
    dst.excel_tidy_first = src.excel_tidy_first;
    dst.excel_tidy_backup = src.excel_tidy_backup;
    dst.ev_cols = src.ev_cols;
}

/// Copy process-wide preferences from the current window onto the others.
pub(crate) fn publish_shared() {
    app(|a| {
        if a.desks.len() < 2 || a.cur >= a.desks.len() {
            return;
        }
        let src = a.desks[a.cur].s.clone();
        for (k, d) in a.desks.iter_mut().enumerate() {
            if k != a.cur {
                copy_shared(&mut d.s, &src);
            }
        }
    });
}

static ACROSS: AtomicBool = AtomicBool::new(false);

struct AcrossGuard;
impl Drop for AcrossGuard {
    fn drop(&mut self) {
        ACROSS.store(false, Ordering::Relaxed);
    }
}

/// Run `f` once per window, on that window. A nested call runs only for the window already current.
pub(crate) unsafe fn across_windows(mut f: impl FnMut()) {
    if ACROSS.swap(true, Ordering::Relaxed) {
        f();
        return;
    }
    let _guard = AcrossGuard;
    let cur = app(|a| a.cur);
    // Re-read the count each pass: `f` may close a window and shrink the list.
    let mut i = 0;
    while i < app(|a| a.desks.len()) {
        bind_index(i);
        f();
        i += 1;
    }
    if cur < app(|a| a.desks.len()) {
        bind_index(cur);
    }
}

// `file:line:column` of the `app` / `app_try` call that holds `APP`.
thread_local! {
    static HELD_AT: Cell<Option<&'static std::panic::Location<'static>>> = const { Cell::new(None) };
}

struct Hold(Option<&'static std::panic::Location<'static>>);
impl Drop for Hold {
    fn drop(&mut self) {
        HELD_AT.with(|c| c.set(self.0));
    }
}

fn push_hold(here: &'static std::panic::Location<'static>) -> Hold {
    Hold(HELD_AT.with(|c| c.replace(Some(here))))
}

fn nest_message(here: &std::panic::Location<'_>) -> String {
    match HELD_AT.with(|c| c.get()) {
        Some(outer) => format!(
            "app() already borrowed at {}:{}:{}; nested call at {}:{}:{}",
            outer.file(),
            outer.line(),
            outer.column(),
            here.file(),
            here.line(),
            here.column()
        ),
        None => format!("app() already borrowed; nested call at {}:{}:{}", here.file(), here.line(), here.column()),
    }
}

fn borrow_app<R>(here: &'static std::panic::Location<'static>, f: impl FnOnce(&mut App) -> R) -> std::result::Result<R, String> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut a) => {
            let _hold = push_hold(here);
            Ok(f(&mut a))
        }
        Err(_) => Err(nest_message(here)),
    })
}

/// Release builds swallow a nested `app()` that arrived from a window proc.
/// Debug builds, and any nested call outside a window proc, still panic.
pub(crate) fn reenter_skips(debug: bool, in_wnd: bool) -> bool {
    !debug && in_wnd
}

fn refuse_reenter(msg: String) -> ! {
    if reenter_skips(cfg!(debug_assertions), crate::crash::in_wndproc()) {
        crate::crash::skip_reenter(&msg);
    }
    panic!("{msg}");
}

/// Borrow the UI state. A nested call panics and names both call sites.
/// In a release build, a nested call from a window proc is logged and that proc
/// returns its fallback instead of crashing. See `app_try` for paint and hit-test.
#[track_caller]
pub(crate) fn app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    let here = std::panic::Location::caller();
    match borrow_app(here, f) {
        Ok(v) => v,
        Err(msg) => refuse_reenter(msg),
    }
}

/// Like `app`, for paint and hit-test. A nested call is logged and skipped.
#[track_caller]
pub(crate) fn app_try<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    let here = std::panic::Location::caller();
    match borrow_app(here, f) {
        Ok(v) => Some(v),
        Err(msg) => {
            crate::crash::log(&format!("app() skipped: {msg}"));
            None
        }
    }
}

/// Pane that commands act on. A tab strip would return the active tab's pane from here.
pub(crate) fn active_pane() -> usize {
    app(|a| a.s.active)
}

pub(crate) fn eb_of(i: usize) -> Option<IExplorerBrowser> {
    app(|a| a.panes.get(i)?.as_ref().map(|p| p.eb.clone()))
}

pub(crate) unsafe fn view_of<T: Interface>(i: usize) -> Option<T> {
    eb_of(i)?.GetCurrentView::<T>().ok()
}

/// Pane whose window is `h`, read from a borrow the caller already holds.
/// `pane_of` calls `app()` and must not be used inside an `app` closure.
pub(crate) fn pane_at(a: &App, h: HWND) -> Option<usize> {
    a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h || unsafe { IsChild(p.hwnd, h).as_bool() }))
}

pub(crate) fn pane_of(h: HWND) -> Option<usize> {
    app(|a| pane_at(a, h))
}

pub(crate) fn pane_ctl(i: usize, id: i32) -> Option<HWND> {
    let h = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd))?;
    unsafe { GetDlgItem(Some(h), id).ok() }
}

pub(crate) unsafe fn text_of(h: HWND) -> String {
    let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
    let n = GetWindowTextW(h, &mut buf) as usize;
    String::from_utf16_lossy(&buf[..n])
}

#[cfg(test)]
mod empty_desks_deref {
    use super::*;

    #[test]
    fn deref_and_deref_mut_tolerate_empty_desks() {
        app(|a| {
            a.desks.clear();
            a.cur = 0;
            a.fallback = blank_desk();
        });
        // Would panic at desks[0] before the fallback guard.
        let main = app(|a| a.main);
        assert!(main.0.is_null());
        app(|a| {
            a.status_key.clear();
            assert!(a.desks.is_empty());
            assert_eq!(a.cur, 0);
        });
        // Restore a blank desk so later tests in this process see a normal App.
        app(|a| {
            if a.desks.is_empty() {
                a.desks.push(blank_desk());
            }
            a.cur = 0;
        });
    }

    #[test]
    fn drop_last_desk_leaves_empty_and_deref_safe() {
        app(|a| {
            a.desks.clear();
            a.desks.push(blank_desk());
            a.cur = 0;
        });
        let _ = drop_current_desk();
        assert_eq!(window_count(), 0);
        let _ = app(|a| a.main); // must not panic
        app(|a| {
            if a.desks.is_empty() {
                a.desks.push(blank_desk());
            }
            a.cur = 0;
        });
    }
}

#[cfg(test)]
mod borrow_tests {
    use super::*;

    fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
        if let Some(s) = payload.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.clone()
        } else {
            String::new()
        }
    }

    #[test]
    fn nested_app_names_the_outer_call_and_the_inner_call() {
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            app(|_outer| {
                app(|_inner| ());
            });
        }))
        .expect_err("nested app()");
        let msg = panic_text(err);
        assert!(msg.contains("already borrowed"), "{msg}");
        assert!(msg.contains("nested call"), "{msg}");
        assert!(msg.matches("state.rs").count() >= 2, "{msg}");
        // The borrow is dropped with the panic, so the next call still runs.
        assert_eq!(app(|a| a.cur), app(|a| a.cur));
    }

    #[test]
    fn app_try_skips_a_reentrant_call_and_leaves_the_borrow_usable() {
        let ran = Cell::new(false);
        let got = app(|_a| {
            app_try(|_b| {
                ran.set(true);
                1
            })
        });
        assert_eq!(got, None);
        assert!(!ran.get());
        assert_eq!(app(|_| 7), 7);
    }

    #[test]
    fn pane_at_reads_inside_app_and_pane_of_does_not() {
        assert!(app(|a| pane_at(a, HWND::default())).is_none());
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            app(|_a| {
                let _ = pane_of(HWND::default());
            });
        }));
        assert!(err.is_err());
        assert!(app(|a| pane_at(a, HWND::default())).is_none());
    }

    #[test]
    fn release_window_proc_reentry_is_the_only_skip() {
        assert!(!reenter_skips(true, true));
        assert!(!reenter_skips(true, false));
        assert!(!reenter_skips(false, false));
        assert!(reenter_skips(false, true));
    }
}
