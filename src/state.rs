use crate::*;

// ---------- app state ----------
// Single UI thread. Rule: never call into Win32/COM that can re-enter our window procs while inside app().

pub(crate) struct Pane {
    pub(crate) hwnd: HWND,
    pub(crate) eb: IExplorerBrowser,
    pub(crate) cookie: u32,
    pub(crate) disp: String,
    pub(crate) pending: Option<String>, // columns to put on when the next folder shows (session restore, open in)
    pub(crate) settle: Option<(String, u8)>, // columns just put on, and passes left to hold narrow widths (TIMER_SETTLE)
    pub(crate) recover: Option<(String, String)>, // missing folder, parent being opened instead (go_near)
    pub(crate) follow: Option<(String, String)>,  // folder renamed elsewhere, its new name being opened (shell_tick)
    pub(crate) icon: i32,   // current folder's icon in the system image list, -1 = none (yet)
    pub(crate) ticket: u32, // its lookup (WM_PANE_ICON); 0 = none asked
    pub(crate) slot: Arc<AtomicUsize>, // where it is now (swap_panes moves it); its Explorer callbacks read it
    pub(crate) spin: Option<(Instant, Instant)>, // F5: its up arrow spins as ↻ from .0, stopping on a whole turn at .1
}

pub(crate) struct App {
    pub(crate) s: Session,
    pub(crate) main: HWND,
    pub(crate) tree: Option<INameSpaceTreeControl>,
    pub(crate) tree_hwnd: HWND,
    pub(crate) tree_input: bool, // a click/key reached the tree since its last selection change
    pub(crate) font: HFONT,
    pub(crate) icon_font: HFONT,
    pub(crate) fh: i32, // UI font line height in px
    pub(crate) dpi: i32,
    pub(crate) panes: Vec<Option<Pane>>,
    pub(crate) drag: Option<Hit>,
    pub(crate) nudge: Option<Hit>, // Ctrl+Alt+arrow moved this splitter: its guide bar shows, the panes follow on key release
    pub(crate) gap_hot: Option<Hit>, // splitter under the mouse, marked in the accent colour
    pub(crate) sizing: bool, // inside the window's own move/size loop
    pub(crate) was_max: bool, // maximized when last not minimized (saved as such while minimized)
    pub(crate) saved_ini: String, // grids.ini text last written
    pub(crate) save_after: Option<Instant>, // a failed write: autosave waits until then
    pub(crate) status: StatusLine,
    pub(crate) status_key: String,
    // items text before the selection size, (file bytes, has folders, partial), middle zone, drive (label, free, total)
    #[allow(clippy::type_complexity)]
    pub(crate) status_base: (String, Option<(u64, bool, bool)>, Vec<String>, Option<(String, u64, u64)>),
    pub(crate) view_menu: HMENU,
    pub(crate) base: Option<Session>, // last loaded/saved (or startup) state; differs -> green dot
    pub(crate) dirty: bool,
    pub(crate) small_font: HFONT,
    pub(crate) load: HWND, // load popup while open
    pub(crate) load_closed: Option<Instant>,
    pub(crate) load_hot: Option<usize>, // card index, or LOAD_SAVE_AS / LOAD_FOLDER / LOAD_DEL + card
    pub(crate) load_edit: HWND,              // rename box on a card (made on first use, hidden when done)
    pub(crate) load_editing: Option<usize>, // card being renamed
    pub(crate) fav_popup: HWND,
    pub(crate) fav_closed: Option<Instant>,
    pub(crate) fav_hot: Option<(usize, bool)>, // (row, on its ✕)
    pub(crate) fav_prev: HWND,                 // focus before the popup opened; Esc goes back there
    pub(crate) fav_press: Option<FavPress>,    // mouse press / drag in the list
    pub(crate) fav_top: usize,                 // first favourite shown when the list is taller than the screen
    pub(crate) fav_max_h: i32,                 // the popup's height limit (room on screen)
    pub(crate) fav_above: Option<i32>,         // opened above ★: its bottom edge stays here (screen y)
    pub(crate) fav_wheel: i32,                 // wheel delta short of a notch
    pub(crate) keys_popup: HWND, // keyboard shortcuts window while open
    pub(crate) keys_prev: HWND,  // focus before it opened; Esc / ✕ / Ctrl+/ go back there
    pub(crate) cards: Vec<(PathBuf, Session, String)>,
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
    pub(crate) home: Homes,         // cell each off-screen slot's pane last had, for when it comes back (runtime only)
    pub(crate) focus_req: HWND,     // pane to focus once its folder shows (set_layout gave it the active pane's place)
    pub(crate) note: Option<(String, Instant)>, // status line confirmation and when it goes (runtime only)
    pub(crate) lost: usize,                     // missing folders the shown note counts (gone_note)
    pub(crate) tips_now: u8,                    // shortcut tips (bit = TIP_* action) already shown since Grids started (runtime only)
    pub(crate) said: Option<Instant>,           // the note (by its end time) last handed to screen readers
    pub(crate) status_hwnd: HWND,               // the status line control (screen readers); null = the main window paints it
    pub(crate) note_hwnd: HWND,                 // the note's live region (zero size, nothing drawn); null = no announcements
    pub(crate) drop_hover: HWND,                         // pane whose top bar something openable is being dragged over
    pub(crate) drop_open: Option<(HWND, usize, IShellItem)>, // folder dropped on a top bar, opened by WM_DROP_OPEN
    pub(crate) crumb_hot: Option<(HWND, usize)>,             // address bar crumb under the mouse (pane, crumb_layout index)
    pub(crate) head_hot: Option<(HWND, HeadHot)>,            // tab, its ×, '+' or the filter icon under the mouse (runtime only)
    pub(crate) kids: Option<KidsAsk>,                        // '›' clicked, its subfolder list not back yet
    pub(crate) held: Option<u16>,        // key whose auto-repeats are swallowed until it is released (runtime only)
    pub(crate) matcher: Option<Matcher>, // select-by-pattern box while open
    pub(crate) match_last: String,       // last pattern used (runtime only, never saved)
    pub(crate) peek: HWND,               // F3 quick preview while open
    pub(crate) peek_pane: usize,         // pane whose focused item it shows
    pub(crate) peek_name: String,        // that item's display name
    pub(crate) ilist: Option<IImageList>, // system small icons, for the address bar (shared, never freed)
    pub(crate) drives: Vec<(u8, u32, HWND)>, // top toolbar drive buttons, A→Z: letter, drive type, button
    pub(crate) drive_mask: u32,              // GetLogicalDrives() they were made for
    pub(crate) drive_mark: Option<u8>,       // the active pane's drive (underlined)
    pub(crate) hot_on: bool,                 // Win+Alt+E actually registered (s.hotkey is only the wish)
    pub(crate) focus_keep: HWND,             // focus when Grids last went to the back; coming back puts it there again
    pub(crate) click_nc: bool,               // the click that is bringing Grids back is outside the client area (title bar)
    pub(crate) focus_wait: bool,             // TIMER_FOCUS running
    pub(crate) shell_q: Vec<Change>,         // folder renames / removals from the shell, waiting for TIMER_SHELL
    pub(crate) quiet: Quiet,                 // started minimized at sign-in, not yet shown (runtime only)
    pub(crate) quiet_max: bool,              // that start was saved maximized: shown maximized
    pub(crate) late: Vec<(usize, String)>,   // panes whose folder could not open at that start, retried when shown
}

// The Num+ / Num− box: popup, its edit, the pane it acts on, that pane's folder when it opened, deselect mode.
pub(crate) struct Matcher {
    pub(crate) h: HWND,
    pub(crate) edit: HWND,
    pub(crate) pane: usize,
    pub(crate) path: String,
    pub(crate) desel: bool,
}

thread_local! {
    pub(crate) static APP: RefCell<App> = RefCell::new(App {
        s: Session::default(),
        main: HWND::default(),
        tree: None,
        tree_hwnd: HWND::default(),
        tree_input: false,
        font: HFONT::default(),
        icon_font: HFONT::default(),
        fh: 16,
        dpi: 96,
        panes: (0..MAXP).map(|_| None).collect(),
        drag: None,
        nudge: None,
        gap_hot: None,
        sizing: false,
        was_max: false,
        saved_ini: String::new(),
        save_after: None,
        status: StatusLine::default(),
        status_key: String::new(),
        status_base: Default::default(),
        view_menu: HMENU::default(),
        base: None,
        dirty: false,
        small_font: HFONT::default(),
        load: HWND::default(),
        load_closed: None,
        load_hot: None,
        load_edit: HWND::default(),
        load_editing: None,
        fav_popup: HWND::default(),
        fav_closed: None,
        fav_hot: None,
        fav_prev: HWND::default(),
        fav_press: None,
        fav_top: 0,
        fav_max_h: i32::MAX,
        fav_above: None,
        fav_wheel: 0,
        keys_popup: HWND::default(),
        keys_prev: HWND::default(),
        cards: Vec::new(),
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
        tips_now: 0,
        lost: 0,
        said: None,
        status_hwnd: HWND::default(),
        note_hwnd: HWND::default(),
        drop_hover: HWND::default(),
        drop_open: None,
        crumb_hot: None,
        head_hot: None,
        kids: None,
        held: None,
        matcher: None,
        match_last: String::new(),
        peek: HWND::default(),
        peek_pane: 0,
        peek_name: String::new(),
        ilist: None,
        drives: Vec::new(),
        drive_mask: 0,
        drive_mark: None,
        hot_on: false,
        focus_keep: HWND::default(),
        click_nc: false,
        focus_wait: false,
        shell_q: Vec::new(),
        quiet: Quiet::No,
        quiet_max: false,
        late: Vec::new(),
    });
}

// Per-pane filter spec. Not in APP: the shell view may call IncludeObject from its own thread.
pub(crate) static FILTERS: Mutex<[String; MAXP]> = Mutex::new([const { String::new() }; MAXP]);

pub(crate) fn filter_of(i: usize) -> String {
    FILTERS.lock().map(|f| f[i].clone()).unwrap_or_default()
}

pub(crate) fn app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(&mut a.borrow_mut()))
}

pub(crate) fn eb_of(i: usize) -> Option<IExplorerBrowser> {
    app(|a| a.panes.get(i)?.as_ref().map(|p| p.eb.clone()))
}

pub(crate) unsafe fn view_of<T: Interface>(i: usize) -> Option<T> {
    eb_of(i)?.GetCurrentView::<T>().ok()
}

pub(crate) fn pane_of(h: HWND) -> Option<usize> {
    app(|a| {
        a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h || unsafe { IsChild(p.hwnd, h).as_bool() }))
    })
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

// scroll an unfocused, untouched address bar to its end, so a cut-off path shows the current folder
pub(crate) unsafe fn show_path_end(e: HWND, disp: &str) {
    let (mut s, mut en) = (0u32, 0u32);
    SendMessageW(e, EM_GETSEL, Some(WPARAM(&mut s as *mut u32 as usize)), Some(LPARAM(&mut en as *mut u32 as isize)));
    if GetFocus() == e || s != en || text_of(e) != disp {
        return;
    }
    let len = GetWindowTextLengthW(e) as usize;
    // back to the start first: drops any scroll left from when the box was 0 px wide
    for at in [0, len] {
        SendMessageW(e, EM_SETSEL, Some(WPARAM(at)), Some(LPARAM(at as isize)));
        SendMessageW(e, EM_SCROLLCARET, None, None);
    }
}
