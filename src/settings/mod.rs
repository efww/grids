// Settings window: Win32, no webview. Nav, cards, toggles and the footer are drawn here;
// edits, combos and buttons are real child controls sitting on that paint.

use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::{app, tr, Lang};

const S_SEARCH: i32 = 1;
const S_CLOSE: i32 = 2;
const S_KEYQ: i32 = 3;
const S_RESET: i32 = 4;
const S_LANG: i32 = 10;
const S_START: i32 = 11;
const S_THEME: i32 = 12;
const S_VIEW: i32 = 14;
const S_DIR: i32 = 15;
const S_DEL: i32 = 16;
const S_NEW: i32 = 17;
const S_DROP: i32 = 18;
const S_SESS: i32 = 20;
const S_ACCENT: i32 = 21;
const S_ADV_OPEN: i32 = 23;
const S_ADV_EX: i32 = 24;
const S_ADV_IM: i32 = 25;
const S_ADV_RESET: i32 = 26;
const S_EV: i32 = 27;
const S_FACE: i32 = 28;
const S_FV_DEF: i32 = 33;
const S_FV_RESET: i32 = 34;
const S_FV_CLEAR: i32 = 35;
pub(crate) const S_CTX_RESET: i32 = 36;
pub(crate) const S_CTX_REFRESH: i32 = 38;
const S_LAYOUT: i32 = 29;
const S_SIDE: i32 = 37;
const S_WM: i32 = 30;
const S_WM_BROWSE: i32 = 31;
const S_FAV0: i32 = 20000;
/// Delete buttons are S_FAV0 + i, rename buttons S_REN0 + i. Control ids stay under 0x8000.
const FAV_MAX: i32 = 4096;
const S_REN0: i32 = S_FAV0 + FAV_MAX;
/// One past the highest favourite row whose buttons were created, so loops skip the empty range.
static FAV_MADE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

const TG_SINGLE: u8 = 1;
const TG_DBLUP: u8 = 2;
const TG_BAR: u8 = 3;
const TG_STATUS: u8 = 4;
const TG_HIDDEN: u8 = 5;
const TG_EXT: u8 = 6;
const TG_TREE: u8 = 7;
const TG_EDGE: u8 = 8;
const TG_CONFIRM: u8 = 9;
const TG_AUTOSAVE: u8 = 11;
const TG_DOT: u8 = 12;
const TG_ONE: u8 = 13;
const TG_EV: u8 = 15;
const TG_DRIVES: u8 = 16;
const TG_SPACE: u8 = 17;
const TG_WHEEL: u8 = 18;
const TG_UPDATE: u8 = 19;
const TG_REMEMBER: u8 = 20;
/// Default folder app. The knob follows HKCU, not a saved preference.
const TG_FOLDER: u8 = 21;
const TG_FSIZE: u8 = 22;
const TG_FSIZE_NET: u8 = 23;
const TG_FAST: u8 = 24;
const TG_DIALOG: u8 = 25;
const TG_PREVIEW: u8 = 26;

const CB_SETMINVISIBLE: u32 = 0x1701;
const EM_SETCUEBANNER: u32 = 0x1501;

fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

/// Settings search frame, DIP at 96 DPI. The rect runs from `SEARCH_TOP_DIP` to 44.
const SEARCH_TOP_DIP: i32 = 12;
const SEARCH_FRAME_DIP: i32 = 32;
/// Gap between that frame and the borderless edit, and the corner radius. Shared with the name prompt
/// and the favourites window search box.
pub(crate) const FIELD_INSET_DIP: i32 = 2;
pub(crate) const FIELD_RADIUS_DIP: i32 = 4;

/// Painted height of the settings search frame. `sc(44) - sc(12)`, so it matches `Geo.search` at every DPI.
pub(crate) fn search_frame_h(dpi: i32) -> i32 {
    sc(dpi, SEARCH_TOP_DIP + SEARCH_FRAME_DIP) - sc(dpi, SEARCH_TOP_DIP)
}

fn tx(t: crate::Key) -> &'static str {
    tr(t)
}

/// Classic handlers start on. Windows 11 packaged apps start off, so that view uses its own note.
fn ctx_note_key() -> crate::Key {
    if app(|a| a.s.ctx_win11_view) { CTX_WIN11_NOTE } else { CTX_NOTE }
}

const CHROME_LIGHT: u32 = 0x0067C0;
const CHROME_DARK: u32 = 0x4CC2FF;
const WHITE: u32 = 0xFFFFFF;

/// Accent used by this window. A chosen colour is used as-is. The old built-in gray
/// selection colour reads as no accent, so that default is Windows blue (#0067c0 / #4cc2ff).
pub(crate) fn chrome_color() -> u32 {
    chrome()
}

fn chrome() -> u32 {
    let (c, dark) = app(|a| (a.s.accent_now(), a.s.dark));
    let plain = if dark { super::DARK.accents[1].1 } else { super::LIGHT.accents[1].1 };
    if c == plain {
        if dark { CHROME_DARK } else { CHROME_LIGHT }
    } else {
        c
    }
}

fn on_chrome(c: u32) -> u32 {
    if c == CHROME_LIGHT {
        WHITE
    } else {
        super::on_color(c)
    }
}

const TITLE: crate::Key = crate::Key::MaguroSettings;
const FOOT: crate::Key = crate::Key::ChangesApplyRightAway;
const CLOSE_L: crate::Key = crate::Key::Close;
const FIND: crate::Key = crate::Key::FindSetting;
const NONE: crate::Key = crate::Key::NoMatches;
/// Content shown when the nav query matches no page. Distinct from `NONE`, which is the nav label and an empty shortcut row.
const NO_RESULTS: crate::Key = crate::Key::NoResults;
pub(crate) const PAGE_N: usize = 10;
/// Right-click menu page. About stays last so earlier page numbers do not move.
pub(crate) const CTX_PAGE: usize = 8;
const PAGES: [crate::Key; PAGE_N] = [
    crate::Key::General,
    crate::Key::Appearance,
    crate::Key::PanesView,
    crate::Key::FileOperations,
    crate::Key::Favourites,
    crate::Key::Shortcuts,
    crate::Key::Sessions,
    crate::Key::Advanced,
    crate::Key::RightClickMenuItems,
    crate::Key::About,
];

const CTX_NOTE: crate::Key = crate::Key::TheseItemsAppearInMaguroSRight;
const CTX_RESET_L: crate::Key = crate::Key::ResetAllEnabled;
const CTX_REFRESH_L: crate::Key = crate::Key::Refresh;
const CTX_EMPTY: crate::Key = crate::Key::NoThirdPartyMenuItems;
const CTX_CLASSIC: crate::Key = crate::Key::ClassicMenu;
const CTX_WIN11: crate::Key = crate::Key::Windows11Menu;
const CTX_WIN11_EMPTY: crate::Key = crate::Key::CtxWin11Empty;
const CTX_WIN11_NOTE: crate::Key = crate::Key::CtxWin11Note;

#[derive(Clone, Copy, PartialEq)]
enum Ctl {
    Lang,
    Face,
    Start,
    Theme,
    ViewMode,
    StartDir,
    DelMode,
    NewName,
    Drop,
    Layout,
    Accent,
    CopyEng,
    Sess,
    About,
    Adv,
    AdvReset,
    EvStatus,
    EvAct,
    WmStatus,
    WmPath,
    Toggle(u8),
    FolderDefault,
    FolderReset,
    FolderClear,
    SideOrder,
}

#[derive(Clone, Copy)]
struct Spec {
    title: crate::Key,
    desc: crate::Key,
    live: bool,
    ctl: Ctl,
}

const GEN: &[Spec] = &[
    Spec { title: crate::Key::DisplayLanguage, desc: crate::Key::DefaultFollowsWindowsDisplayLanguageKorea, live: true, ctl: Ctl::Lang },
    Spec { title: crate::Key::Font, desc: crate::Key::DefaultFollowsDisplayLanguageChosenFaceA, live: true, ctl: Ctl::Face },
    Spec { title: crate::Key::OnStartup, desc: crate::Key::RestoreEveryOpenTabAndWindowOr, live: true, ctl: Ctl::Start },
    Spec { title: crate::Key::UseAsDefaultFolderApp, desc: crate::Key::WinEAndSpecialFoldersStayWith, live: true, ctl: Ctl::Toggle(TG_FOLDER) },
    Spec { title: crate::Key::DefaultLayout, desc: crate::Key::PanesInNewWindowAndOnFirst, live: true, ctl: Ctl::Layout },
    Spec { title: crate::Key::SingleInstance, desc: crate::Key::SecondLaunchOpensNewWindowInThis, live: true, ctl: Ctl::Toggle(TG_SINGLE) },
    Spec { title: crate::Key::SingleWindowMode, desc: crate::Key::OneWindowAnotherLaunchBringsItForward, live: true, ctl: Ctl::Toggle(TG_ONE) },
    Spec { title: crate::Key::FoldersInOpenAndSaveDialogs, desc: crate::Key::WhileAnotherAppSOpenOrSaveDialogIsInFront, live: true, ctl: Ctl::Toggle(TG_DIALOG) },
    Spec { title: crate::Key::DoubleClickEmptySpaceForParentFolder, desc: crate::Key::InFileListDoubleClickOnEmpty, live: true, ctl: Ctl::Toggle(TG_DBLUP) },
    Spec { title: crate::Key::Status, desc: crate::Key::InstalledRunningVersion, live: true, ctl: Ctl::EvStatus },
    Spec { title: crate::Key::UseEverythingForSearch, desc: crate::Key::OnByDefaultSubfolderSearchAsksEverything, live: true, ctl: Ctl::Toggle(TG_EV) },
    Spec { title: crate::Key::GetEverything, desc: crate::Key::StartItWhenItIsInstalledInstall, live: true, ctl: Ctl::EvAct },
    Spec { title: crate::Key::AutoDetect, desc: crate::Key::VendorWinmerge, live: true, ctl: Ctl::WmStatus },
    Spec { title: crate::Key::WinmergePath, desc: crate::Key::EmptyUsesAutoDetectFolderOrWinmergeu, live: true, ctl: Ctl::WmPath },
];

const EV_HEAD: crate::Key = crate::Key::Everything;
const WM_HEAD: crate::Key = crate::Key::VendorWinmerge;
const PREVIEW_HEAD: crate::Key = crate::Key::Preview;
const LOOK: &[Spec] = &[
    Spec { title: crate::Key::Theme, desc: crate::Key::Blank, live: true, ctl: Ctl::Theme },
    Spec { title: crate::Key::AccentColour, desc: crate::Key::LightAndDarkEachKeepTheirOwn, live: true, ctl: Ctl::Accent },
    Spec { title: crate::Key::ShowToolbar, desc: crate::Key::Blank, live: true, ctl: Ctl::Toggle(TG_BAR) },
    Spec { title: crate::Key::ShowDriveButtons, desc: crate::Key::CDEOnTabRowOff, live: true, ctl: Ctl::Toggle(TG_DRIVES) },
    Spec { title: crate::Key::ShowStatusBar, desc: crate::Key::ItemCountAndFreeSpaceOnDrive, live: true, ctl: Ctl::Toggle(TG_STATUS) },
];
const VIEW: &[Spec] = &[
    Spec { title: crate::Key::DefaultViewForNewPane, desc: crate::Key::UsedWhenPaneHasNoViewOf, live: true, ctl: Ctl::ViewMode },
    Spec { title: crate::Key::RememberViewPerFolder, desc: crate::Key::SortColumnsViewModeIconSizeAnd, live: true, ctl: Ctl::Toggle(TG_REMEMBER) },
    Spec { title: crate::Key::DefaultViewForEveryFolder, desc: crate::Key::FoldersWithNoSavedViewUseThis, live: true, ctl: Ctl::FolderDefault },
    Spec { title: crate::Key::ResetThisFolderSView, desc: crate::Key::ForgetsActivePaneSFolderItUses, live: true, ctl: Ctl::FolderReset },
    Spec { title: crate::Key::ClearAllRememberedViews, desc: crate::Key::ForgetsEveryFolderDefaultViewStays, live: true, ctl: Ctl::FolderClear },
    Spec { title: crate::Key::StartingFolderForNewPane, desc: crate::Key::UsedWhenNewPaneHasNoFolder, live: true, ctl: Ctl::StartDir },
    Spec { title: crate::Key::ShowHiddenFiles, desc: crate::Key::SameAsCtrlH, live: true, ctl: Ctl::Toggle(TG_HIDDEN) },
    Spec { title: crate::Key::ShowFileExtensions2, desc: crate::Key::ThisIsWindowsOwnSettingSoExplorer, live: true, ctl: Ctl::Toggle(TG_EXT) },
    Spec { title: crate::Key::ShowFolderSizes, desc: crate::Key::StatusBarShowsTotalOfSelectedFolder, live: true, ctl: Ctl::Toggle(TG_FSIZE) },
    Spec { title: crate::Key::FolderSizesForNetworkFolders, desc: crate::Key::UncAndMappedDrivesOffByDefault, live: true, ctl: Ctl::Toggle(TG_FSIZE_NET) },
    Spec { title: crate::Key::ShowLeftPanel, desc: crate::Key::FavouritesAndFolderTreeShareItDrag, live: true, ctl: Ctl::Toggle(TG_TREE) },
    Spec { title: crate::Key::LeftPanelOrder, desc: crate::Key::FavouritesOnTopOrFolderTreeOn, live: true, ctl: Ctl::SideOrder },
    Spec { title: crate::Key::EmphasiseActivePaneBorder, desc: crate::Key::Blank, live: true, ctl: Ctl::Toggle(TG_EDGE) },
    Spec { title: crate::Key::ShowPreviewPane, desc: crate::Key::PreviewPaneDesc, live: true, ctl: Ctl::Toggle(TG_PREVIEW) },
    Spec { title: crate::Key::PreviewWithSpace, desc: crate::Key::SameAsF3InFileListSpace, live: true, ctl: Ctl::Toggle(TG_SPACE) },
    Spec { title: crate::Key::WheelMovesNextPreviousFile, desc: crate::Key::ScrollingPastEndOfContentMovesNext, live: true, ctl: Ctl::Toggle(TG_WHEEL) },
];
const FILE: &[Spec] = &[
    Spec { title: crate::Key::CopyMoveProgress, desc: crate::Key::WindowsProgressWindowRunsOnItsOwn, live: true, ctl: Ctl::CopyEng },
    Spec { title: crate::Key::FastCopyForNetworkFolders, desc: crate::Key::PasteAndDragUseRobocopyWhenANetwork, live: true, ctl: Ctl::Toggle(TG_FAST) },
    Spec { title: crate::Key::ConfirmDelete, desc: crate::Key::DelInPaneShiftDelIsAlways, live: true, ctl: Ctl::Toggle(TG_CONFIRM) },
    Spec { title: crate::Key::DefaultDelete, desc: crate::Key::Blank, live: true, ctl: Ctl::DelMode },
    Spec { title: crate::Key::NewFileName, desc: crate::Key::UsedByCtrlAltNEmptyFollows, live: true, ctl: Ctl::NewName },
    Spec { title: crate::Key::DefaultDragAndDrop, desc: crate::Key::MenuDragInsidePaneStillFollowsExplorer, live: true, ctl: Ctl::Drop },
];
const SESS: &[Spec] = &[
    Spec { title: crate::Key::SessionFolder, desc: crate::Key::AppdataMaguroMaguroSessions, live: true, ctl: Ctl::Sess },
    Spec { title: crate::Key::SaveOnExit, desc: crate::Key::WritesEachTabThatAlreadyHasIts, live: true, ctl: Ctl::Toggle(TG_AUTOSAVE) },
    Spec { title: crate::Key::UnsavedChangesMarkGreenDot, desc: crate::Key::OnSaveButton, live: true, ctl: Ctl::Toggle(TG_DOT) },
];
const ADV: &[Spec] = &[
    Spec { title: crate::Key::CheckForUpdatesOnStartup, desc: crate::Key::AsksGithubForNewerMaguroReleaseAnd, live: true, ctl: Ctl::Toggle(TG_UPDATE) },
    Spec { title: crate::Key::SettingsFile, desc: crate::Key::PortableTxtNextExeKeepsThisFile, live: true, ctl: Ctl::Adv },
    Spec { title: crate::Key::ResetAllSettings, desc: crate::Key::PreferencesReturnTheirDefaultsLayoutFolde, live: true, ctl: Ctl::AdvReset },
];

const KEY_NOTE: crate::Key = crate::Key::DoubleClickRowOrPressChangeThen;
const LIST_H: crate::Key = crate::Key::ViewList;
const SIDE_FAV: crate::Key = crate::Key::FavouritesOnTop;
const SIDE_TREE: crate::Key = crate::Key::FolderTreeOnTop;
const ABOUT: &[Spec] = &[Spec {
    title: crate::Key::Maguro,
    desc: crate::Key::FastMultiPaneFileManagerForWindows,
    live: true,
    ctl: Ctl::About,
}];

fn page_cards(p: usize) -> &'static [Spec] {
    match p {
        0 => GEN,
        1 => LOOK,
        2 => VIEW,
        3 => FILE,
        6 => SESS,
        7 => ADV,
        9 => ABOUT,
        _ => &[],
    }
}

fn has(hay: &str, q: &str) -> bool {
    q.is_empty() || hay.to_lowercase().contains(&q.to_lowercase())
}

fn card_hit(c: &Spec, q: &str) -> bool {
    has(tx(c.title), q) || has(tx(c.desc), q)
}

fn key_hit_i(i: usize, q: &str) -> bool {
    let (cat, act, chord) = crate::keys::row(i);
    has(&cat, q) || has(&act, q) || has(&chord, q)
}

fn page_match(q: &str, page: usize) -> bool {
    if q.is_empty() || has(tx(PAGES[page]), q) {
        return true;
    }
    if page_cards(page).iter().any(|c| card_hit(c, q)) {
        return true;
    }
    if page == 9 {
        return has(tx(crate::Key::MitLicense2026Efww), q)
            || has("Maguro", q)
            || has(env!("CARGO_PKG_VERSION"), q)
            || has("github.com/Samek86/maguro", q);
    }
    if page == 4 {
        let rows = fav_tree_rows();
        return !crate::filter_fav_rows(&rows, q).is_empty() || has(tx(LIST_H), q);
    }
    if page == 5 {
        return (0..crate::keys::len()).any(|i| key_hit_i(i, q)) || has(tx(KEY_NOTE), q);
    }
    if page == CTX_PAGE {
        return has(tx(CTX_NOTE), q)
            || has(tx(CTX_WIN11_NOTE), q)
            || has(tx(CTX_RESET_L), q)
            || has(tx(CTX_REFRESH_L), q)
            || has(tx(CTX_EMPTY), q)
            || has(tx(CTX_CLASSIC), q)
            || has(tx(CTX_WIN11), q)
            || has(tx(CTX_WIN11_EMPTY), q)
            || crate::shell::ctxmenu::name_hit(q)
            || crate::shell::ctxpack::name_hit(q);
    }
    false
}

/// Empty when the page title already matched, so the whole page stays visible.
fn row_q(q: &str, page: usize) -> &str {
    if q.is_empty() || has(tx(PAGES[page]), q) {
        ""
    } else {
        q
    }
}

/// True when `q` matches no settings page. An empty query matches every page.
fn search_miss(q: &str) -> bool {
    !q.is_empty() && !(0..PAGES.len()).any(|p| page_match(q, p))
}

/// Page to show for a nav-search change.
///
/// `anchor` is the page the user had before this search auto-navigated (a nav click,
/// or the page when the box was last empty). A query that matches nothing restores
/// that page and does not stay on a page a shorter prefix opened. A prefix such as
/// "PageU" matches only Shortcuts, because chord text includes PageUp / PageDown.
/// "Ag" also hits the Maguro sessions path, so it is not a unique Shortcuts prefix.
/// The next letter of a unique prefix must not leave Shortcuts selected.
fn pick_search(anchor: usize, page: usize, q: &str) -> usize {
    if q.is_empty() || page_match(q, page) {
        page
    } else {
        (0..PAGES.len()).find(|&p| page_match(q, p)).unwrap_or(anchor)
    }
}

/// `(anchor, page)` after one search-box change. Clearing the box keeps the page
/// that was showing and makes it the next anchor.
fn commit_search(anchor: usize, page: usize, q: &str) -> (usize, usize) {
    let page = pick_search(anchor, page, q);
    let anchor = if q.is_empty() { page } else { anchor };
    (anchor, page)
}

struct St {
    hwnd: HWND,
    page: usize,
    /// Page to restore when a search matches nothing. Not updated when search auto-switches.
    anchor: usize,
    scroll: i32,
    scroll_max: i32,
    dpi: i32,
    query: String,
    keyq: String,
    hot: Option<usize>,
    font: HFONT,
    /// Body face at 14px, weight 700. Accent button labels. `bold` stays the 16px group face.
    btn_bold: HFONT,
    small: HFONT,
    bold: HFONT,
    title_font: HFONT,
    mono: HFONT,
    icon: HFONT,
    /// Installed families. `None` until this window asks; dropped when the window closes.
    faces: Option<Vec<String>>,
    track: RECT,
    thumb_h: i32,
    drag: Option<i32>,
    /// Tree id of the favourites row being dragged. Indices shift as the node moves.
    fav_drag: Option<Vec<usize>>,
    /// Handler ids expanded on the right-click menu page.
    ctx_open: Vec<String>,
}

impl Default for St {
    fn default() -> Self {
        St {
            hwnd: HWND::default(),
            page: 0,
            anchor: 0,
            scroll: 0,
            scroll_max: 0,
            dpi: 96,
            query: String::new(),
            keyq: String::new(),
            hot: None,
            font: HFONT::default(),
            btn_bold: HFONT::default(),
            small: HFONT::default(),
            bold: HFONT::default(),
            title_font: HFONT::default(),
            mono: HFONT::default(),
            icon: HFONT::default(),
            faces: None,
            track: RECT::default(),
            thumb_h: 0,
            drag: None,
            fav_drag: None,
            ctx_open: Vec::new(),
        }
    }
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

struct Snap {
    page: usize,
    scroll: i32,
    dpi: i32,
    query: String,
    keyq: String,
    hot: Option<usize>,
    font: HFONT,
    /// Body face at 14px, weight 700. Selected accent segment. Same face as `font`.
    btn_bold: HFONT,
    small: HFONT,
    bold: HFONT,
    title_font: HFONT,
    mono: HFONT,
    icon: HFONT,
    ctx_open: Vec<String>,
}

fn snap() -> Snap {
    ST.with(|s| {
        let st = s.borrow();
        Snap {
            page: st.page,
            scroll: st.scroll,
            dpi: st.dpi,
            query: st.query.clone(),
            keyq: st.keyq.clone(),
            hot: st.hot,
            font: st.font,
            btn_bold: st.btn_bold,
            small: st.small,
            bold: st.bold,
            title_font: st.title_font,
            mono: st.mono,
            icon: st.icon,
            ctx_open: st.ctx_open.clone(),
        }
    })
}

fn hwnd() -> HWND {
    ST.with(|s| s.borrow().hwnd)
}

struct CardItem {
    spec: Spec,
    rect: RECT,
    ctl: RECT,
    sw: Vec<RECT>,
    adv: [RECT; 4],
}

enum Item {
    Card(CardItem),
    Head(RECT, crate::Key),
    Table(RECT),
    FavHead(RECT),
    FavRow { i: usize, rect: RECT, rename: RECT, btn: RECT },
    Note(RECT, crate::Key),
    CtxNote(RECT),
    /// Classic | Windows 11. One list is visible. The active pill is filled in paint.
    CtxSeg { classic: RECT, win11: RECT },
    CtxBar { reset: RECT, refresh: RECT },
    CtxRow {
        id: String,
        ids: Vec<String>,
        scopes: Vec<u8>,
        rect: RECT,
        mark: RECT,
        chev: RECT,
        name: String,
        detail: String,
        raw: String,
        tick: crate::shell::ctxmenu::Tick,
        chips: Vec<(u8, RECT)>,
        /// Packaged row: one on/off for the CLSID. No chevron, no per-target expand.
        packaged: bool,
    },
    CtxSub { ids: Vec<String>, scopes: Vec<u8>, bit: u8, rect: RECT, mark: RECT, tick: crate::shell::ctxmenu::Tick },
    CtxEmpty(RECT, crate::Key),
    KeyBar { edit: RECT, reset: RECT },
    KeyHead(RECT),
    KeyRow { i: usize, rect: RECT, change: RECT, reset: RECT },
}

struct Geo {
    nav_w: i32,
    nav: [RECT; PAGE_N],
    show: [bool; PAGE_N],
    search: RECT,
    content: RECT,
    title: RECT,
    footer: RECT,
    close: RECT,
    hint: RECT,
    items: Vec<Item>,
    scroll_max: i32,
    track: RECT,
    thumb: RECT,
    has_bar: bool,
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn right_box(card: RECT, w: i32, h: i32, pad: i32) -> RECT {
    let cy = (card.top + card.bottom) / 2;
    RECT { left: card.right - pad - w, top: cy - h / 2, right: card.right - pad, bottom: cy + h / 2 }
}

fn card_h(spec: &Spec, dpi: i32) -> i32 {
    // 12px vertical padding, 14px label, optional 12px description.
    // The settings-file card adds a line for the live path.
    // About stacks version, description, license, and repo.
    if spec.ctl == Ctl::About {
        sc(dpi, 120)
    } else if spec.ctl == Ctl::WmPath {
        sc(dpi, 120)
    } else if spec.ctl == Ctl::Adv || spec.ctl == Ctl::EvStatus || spec.ctl == Ctl::EvAct || spec.ctl == Ctl::WmStatus {
        sc(dpi, 84)
    } else if tx(spec.desc).is_empty() {
        sc(dpi, 48)
    } else {
        sc(dpi, 64)
    }
}

fn geo(rc: RECT, sn: &Snap) -> Geo {
    let dpi = sn.dpi.max(96);
    let nav_w = sc(dpi, 220);
    let foot_h = sc(dpi, 56);
    let footer = RECT { left: 0, top: rc.bottom - foot_h, right: rc.right, bottom: rc.bottom };
    let content = RECT { left: nav_w, top: 0, right: rc.right, bottom: footer.top };
    let search = RECT { left: sc(dpi, 8), top: sc(dpi, SEARCH_TOP_DIP), right: nav_w - sc(dpi, 8), bottom: sc(dpi, SEARCH_TOP_DIP + SEARCH_FRAME_DIP) };
    let close = right_box(footer, sc(dpi, 88), sc(dpi, 32), sc(dpi, 16));
    let hint = RECT { left: sc(dpi, 16), top: footer.top, right: close.left - sc(dpi, 8), bottom: footer.bottom };
    let q = sn.query.as_str();
    let mut show = [false; PAGE_N];
    let mut nav = [RECT::default(); PAGE_N];
    let mut y = search.bottom + sc(dpi, 8);
    // Keep every visible nav row (including About) above the footer so the last
    // sidebar item stays hittable when the window is short or DPI is high.
    let n_show = (0..PAGE_N).filter(|&i| page_match(q, i)).count() as i32;
    let avail = (footer.top - y).max(0);
    let row_ideal = sc(dpi, 36);
    let row_min = sc(dpi, 22);
    let row_h = if n_show > 0 {
        (avail / n_show).clamp(row_min, row_ideal)
    } else {
        row_ideal
    };
    for i in 0..PAGE_N {
        show[i] = page_match(q, i);
        if !show[i] {
            continue;
        }
        nav[i] = RECT { left: sc(dpi, 8), top: y, right: nav_w - sc(dpi, 8), bottom: y + row_h };
        y += row_h;
    }
    // No page matches: leave the body empty so paint can show the miss message
    // instead of the retained page (Shortcuts would otherwise lay out an empty table).
    let miss = search_miss(q);
    let mut g = Geo {
        nav_w,
        nav,
        show,
        search,
        content,
        title: RECT::default(),
        footer,
        close,
        hint,
        items: Vec::new(),
        scroll_max: 0,
        track: RECT::default(),
        thumb: RECT::default(),
        has_bar: false,
    };
    if miss {
        return g;
    }
    // Lay out once to learn the height, then again inset for the scrollbar if needed.
    // A scrollbar narrows the shortcut column, and that can wrap another chord line,
    // so the height that is painted (not the wider first pass) decides the scroll range.
    let natural = fill_items(&mut g, sn, false);
    let view_h = (content.bottom - content.top).max(1);
    let mut has_bar = natural > view_h;
    g.items.clear();
    let mut content_h = fill_items(&mut g, sn, has_bar);
    if !has_bar && content_h > view_h {
        has_bar = true;
        g.items.clear();
        content_h = fill_items(&mut g, sn, true);
    }
    g.has_bar = has_bar;
    g.scroll_max = (content_h - view_h).max(0);
    if g.has_bar {
        let track = RECT {
            left: rc.right - sc(dpi, 10),
            top: content.top + sc(dpi, 8),
            right: rc.right - sc(dpi, 4),
            bottom: content.bottom - sc(dpi, 8),
        };
        let span = (track.bottom - track.top).max(1);
        let thumb_h = (span * view_h / content_h.max(view_h)).clamp(sc(dpi, 24), span);
        let travel = (span - thumb_h).max(1);
        let top = track.top + if g.scroll_max == 0 { 0 } else { sn.scroll.clamp(0, g.scroll_max) * travel / g.scroll_max };
        g.thumb = RECT { left: track.left, top, right: track.right, bottom: top + thumb_h };
        g.track = track;
    }
    g
}

fn fill_items(g: &mut Geo, sn: &Snap, bar: bool) -> i32 {
    let dpi = sn.dpi.max(96);
    let pad = sc(dpi, 28);
    let right_pad = if bar { sc(dpi, 18) } else { 0 };
    let left = g.content.left + pad;
    let right = g.content.right - pad - right_pad;
    let start = g.content.top + sc(dpi, 16);
    let mut y = start - sn.scroll;
    g.title = RECT { left, top: y, right, bottom: y + sc(dpi, 32) };
    y += sc(dpi, 32) + sc(dpi, 16);
    let page = sn.page;
    let rq = row_q(&sn.query, page);
    let gap = sc(dpi, 4);
    let mut ev_head = false;
    let mut wm_head = false;
    let mut preview_head = false;
    for spec in page_cards(page) {
        if !rq.is_empty() && !card_hit(spec, rq) {
            continue;
        }
        if is_ev_card(spec) {
            if !ev_head {
                g.items.push(Item::Head(RECT { left, top: y, right, bottom: y + sc(dpi, 28) }, EV_HEAD));
                y += sc(dpi, 28);
                ev_head = true;
            }
        }
        if is_wm_card(spec) {
            if !wm_head {
                g.items.push(Item::Head(RECT { left, top: y, right, bottom: y + sc(dpi, 28) }, WM_HEAD));
                y += sc(dpi, 28);
                wm_head = true;
            }
        }
        if is_preview_card(spec) {
            if !preview_head {
                g.items.push(Item::Head(RECT { left, top: y, right, bottom: y + sc(dpi, 28) }, PREVIEW_HEAD));
                y += sc(dpi, 28);
                preview_head = true;
            }
        }
        let h = card_h(spec, dpi);
        let rect = RECT { left, top: y, right, bottom: y + h };
        g.items.push(Item::Card(card_item(rect, *spec, dpi)));
        y += h + gap;
    }
    if page == 4 {
        y = fav_items(g, sn, left, right, y, rq);
    }
    if page == 5 {
        y = key_items(g, sn, left, right, y, rq);
    }
    if page == CTX_PAGE {
        y = ctx_items(g, sn, left, right, y, rq);
    }
    (y + sn.scroll) - start + sc(dpi, 16)
}

/// ASCII is one cell. Hangul and kana are two, same rule as the target chips.
fn seg_w(label: &str, dpi: i32) -> i32 {
    let mut text_w = 0i32;
    for ch in label.chars() {
        text_w += if ch.is_ascii() { sc(dpi, 7) } else { sc(dpi, 14) };
    }
    text_w + sc(dpi, 32)
}

fn ctx_items(g: &mut Geo, sn: &Snap, left: i32, right: i32, mut y: i32, rq: &str) -> i32 {
    let dpi = sn.dpi.max(96);
    if rq.is_empty() || has(tx(CTX_NOTE), rq) || has(tx(CTX_WIN11_NOTE), rq) {
        let h = sc(dpi, 72);
        g.items.push(Item::CtxNote(RECT { left, top: y, right, bottom: y + h }));
        y += h + sc(dpi, 8);
    }
    let sh = sc(dpi, 32);
    let gap = sc(dpi, 8);
    let classic = RECT { left, top: y, right: left + seg_w(tx(CTX_CLASSIC), dpi), bottom: y + sh };
    let win11 = RECT { left: classic.right + gap, top: y, right: classic.right + gap + seg_w(tx(CTX_WIN11), dpi), bottom: y + sh };
    g.items.push(Item::CtxSeg { classic, win11 });
    y += sh + gap;
    let (win11_view, pack_on, off, on, seen, quiet) = app(|a| {
        (
            a.s.ctx_win11_view,
            a.s.ctx_packaged_on.clone(),
            a.s.ctx_off.clone(),
            a.s.ctx_on.clone(),
            a.s.ctx_labels.clone(),
            a.s.ctx_quiet.clone(),
        )
    });
    let rows = if win11_view { Vec::new() } else { crate::shell::ctxmenu::ui_rows(&off, &on, &seen, &quiet, rq) };
    let pack_rows = if win11_view { crate::shell::ctxpack::ui_rows(&pack_on, rq) } else { Vec::new() };
    let reset_w = sc(dpi, 220);
    let refresh_w = sc(dpi, 140);
    let bh = sc(dpi, 32);
    let reset = RECT { left: (right - reset_w).max(left), top: y, right, bottom: y + bh };
    let refresh = RECT {
        left: (reset.left - gap - refresh_w).max(left),
        top: y,
        right: (reset.left - gap).max(left),
        bottom: y + bh,
    };
    g.items.push(Item::CtxBar { reset, refresh });
    y += bh + sc(dpi, 12);
    if win11_view {
        if pack_rows.is_empty() {
            g.items.push(Item::CtxEmpty(RECT { left, top: y, right, bottom: y + sc(dpi, 36) }, CTX_WIN11_EMPTY));
            return y + sc(dpi, 44);
        }
        for row in pack_rows {
            let sub = !row.detail.is_empty();
            let h = sc(dpi, if sub { 64 } else { 48 });
            let rect = RECT { left, top: y, right, bottom: y + h };
            let tick_px = sc(dpi, 16);
            let mid = if sub { y + sc(dpi, 8) + sc(dpi, 11) } else { y + h / 2 };
            let mark = RECT { left: left + sc(dpi, 14), top: mid - tick_px / 2, right: left + sc(dpi, 30), bottom: mid + tick_px / 2 };
            let chips = ctx_chips(row.targets, rect, dpi);
            g.items.push(Item::CtxRow {
                id: row.id,
                ids: Vec::new(),
                scopes: Vec::new(),
                rect,
                mark,
                chev: RECT { left: 0, top: 0, right: 0, bottom: 0 },
                name: row.name,
                detail: row.detail,
                raw: String::new(),
                tick: if row.on { crate::shell::ctxmenu::Tick::On } else { crate::shell::ctxmenu::Tick::Off },
                chips,
                packaged: true,
            });
            y += h + sc(dpi, 6);
        }
        return y;
    }
    if rows.is_empty() {
        g.items.push(Item::CtxEmpty(RECT { left, top: y, right, bottom: y + sc(dpi, 36) }, CTX_EMPTY));
        return y + sc(dpi, 44);
    }
    for row in rows {
        let sub = !row.detail.is_empty() || !row.raw.is_empty();
        let h = sc(dpi, if sub { 64 } else { 48 });
        let rect = RECT { left, top: y, right, bottom: y + h };
        let tick = sc(dpi, 16);
        let mid = if sub { y + sc(dpi, 8) + sc(dpi, 11) } else { y + h / 2 };
        let mark = RECT { left: left + sc(dpi, 14), top: mid - tick / 2, right: left + sc(dpi, 30), bottom: mid + tick / 2 };
        let chev = RECT { left: mark.right + sc(dpi, 2), top: y, right: mark.right + sc(dpi, 26), bottom: y + h };
        let chips = ctx_chips(row.targets, rect, dpi);
        let open = sn.ctx_open.iter().any(|id| id == &row.id);
        g.items.push(Item::CtxRow {
            id: row.id.clone(),
            ids: row.ids.clone(),
            scopes: row.scopes.clone(),
            rect,
            mark,
            chev,
            name: row.name,
            detail: row.detail,
            raw: row.raw,
            tick: row.tick,
            chips,
            packaged: false,
        });
        y += h;
        if open {
            for (i, bit) in crate::shell::ctxmenu::target_bits().into_iter().enumerate() {
                if row.targets & bit == 0 {
                    continue;
                }
                let sh = sc(dpi, 36);
                let sub = RECT { left: left + sc(dpi, 36), top: y, right, bottom: y + sh };
                let sbox = RECT {
                    left: sub.left + sc(dpi, 14),
                    top: y + (sh - sc(dpi, 16)) / 2,
                    right: sub.left + sc(dpi, 30),
                    bottom: y + (sh + sc(dpi, 16)) / 2,
                };
                g.items.push(Item::CtxSub {
                    ids: row.ids.clone(),
                    scopes: row.scopes.clone(),
                    bit,
                    rect: sub,
                    mark: sbox,
                    tick: row.subs[i],
                });
                y += sh;
            }
        }
        y += sc(dpi, 6);
    }
    y
}

fn ctx_chips(targets: u8, rect: RECT, dpi: i32) -> Vec<(u8, RECT)> {
    let mut x = rect.right - sc(dpi, 12);
    let mut out = Vec::new();
    for bit in crate::shell::ctxmenu::target_bits().into_iter().rev() {
        if targets & bit == 0 {
            continue;
        }
        let label = crate::shell::ctxmenu::target_label(bit);
        // Hangul and kana are about twice an ASCII cell. A flat 7px count clips "드라이브" to "드라…".
        let mut text_w = 0i32;
        for ch in label.chars() {
            text_w += if ch.is_ascii() { sc(dpi, 7) } else { sc(dpi, 14) };
        }
        let w = sc(dpi, 20) + text_w;
        let chip_h = sc(dpi, 20);
        let row_h = rect.bottom - rect.top;
        let top = if row_h > sc(dpi, 52) { rect.top + sc(dpi, 8) } else { rect.top + (row_h - chip_h) / 2 };
        let r = RECT { left: x - w, top, right: x, bottom: top + chip_h };
        x = r.left - sc(dpi, 6);
        out.push((bit, r));
    }
    out.reverse();
    out
}

fn card_item(rect: RECT, spec: Spec, dpi: i32) -> CardItem {
    let pad = sc(dpi, 16);
    let mut sw = Vec::new();
    let mut adv = [RECT::default(); 4];
    let ctl = match spec.ctl {
        Ctl::Toggle(_) => right_box(rect, sc(dpi, 40), sc(dpi, 20), pad),
        Ctl::EvStatus | Ctl::WmStatus => RECT { left: rect.right - pad, top: rect.top, right: rect.right - pad, bottom: rect.bottom },
        Ctl::EvAct => right_box(rect, sc(dpi, 200), sc(dpi, 32), pad),
        Ctl::WmPath => {
            let bw = sc(dpi, 108);
            let top = rect.top + sc(dpi, 12);
            adv[0] = RECT { left: rect.right - pad - bw, top, right: rect.right - pad, bottom: top + sc(dpi, 32) };
            let eh = sc(dpi, 32);
            RECT { left: rect.left + pad, top: rect.bottom - sc(dpi, 12) - eh, right: rect.right - pad, bottom: rect.bottom - sc(dpi, 12) }
        }
        Ctl::CopyEng => right_box(rect, sc(dpi, 140), sc(dpi, 22), pad),
        Ctl::Lang | Ctl::Face | Ctl::Start | Ctl::Theme | Ctl::ViewMode | Ctl::StartDir | Ctl::DelMode | Ctl::NewName | Ctl::Drop | Ctl::Layout | Ctl::SideOrder => {
            right_box(rect, sc(dpi, 230), sc(dpi, 32), pad)
        }
        Ctl::Sess => right_box(rect, sc(dpi, 108), sc(dpi, 32), pad),
        Ctl::About => RECT { left: rect.right - pad, top: rect.top, right: rect.right - pad, bottom: rect.top },
        Ctl::Accent => {
            let btn = right_box(rect, sc(dpi, 108), sc(dpi, 32), pad);
            let mut x = btn.left - sc(dpi, 10) - sc(dpi, 22);
            for _ in 0..6 {
                sw.push(RECT { left: x, top: (rect.top + rect.bottom) / 2 - sc(dpi, 11), right: x + sc(dpi, 22), bottom: (rect.top + rect.bottom) / 2 + sc(dpi, 11) });
                x -= sc(dpi, 28);
            }
            sw.reverse();
            RECT { left: sw.first().map(|r| r.left).unwrap_or(btn.left), ..btn }
        }
        Ctl::Adv | Ctl::AdvReset | Ctl::FolderDefault | Ctl::FolderReset | Ctl::FolderClear => {
            let (n, bw) = match spec.ctl {
                Ctl::Adv => (3, sc(dpi, 108)),
                Ctl::FolderDefault => (1, sc(dpi, 188)),
                Ctl::FolderReset | Ctl::FolderClear => (1, sc(dpi, 132)),
                _ => (1, sc(dpi, 96)),
            };
            let gap = sc(dpi, 6);
            let total = bw * n + gap * (n - 1);
            let mut x = rect.right - pad - total;
            // The settings-file card is taller so the path can use the full width under the buttons.
            let top = if spec.ctl == Ctl::Adv { rect.top + sc(dpi, 14) } else { (rect.top + rect.bottom) / 2 - sc(dpi, 16) };
            for i in 0..n as usize {
                adv[i] = RECT { left: x, top, right: x + bw, bottom: top + sc(dpi, 32) };
                x += bw + gap;
            }
            RECT { left: adv[0].left, top: adv[0].top, right: rect.right - pad, bottom: adv[0].bottom }
        }
    };
    // The accent button is the right-hand part of `ctl`; keep that as the button rect via adv[0] unused.
    // Swatches live in `sw`. The button itself is ctl's right sc(108).
    let ctl = if spec.ctl == Ctl::Accent {
        RECT { left: ctl.right - sc(dpi, 108), ..ctl }
    } else {
        ctl
    };
    CardItem { spec, rect, ctl, sw, adv }
}

/// Stored favourites tree, depth-first, including children of a collapsed group.
fn fav_tree_rows() -> Vec<crate::FavRow> {
    app(|a| {
        a.s.ensure_fav_tree();
        crate::tree_rows(&a.s.tree)
    })
}

fn fav_node(i: usize) -> Option<crate::FavRow> {
    fav_tree_rows().get(i).cloned()
}

fn fav_items(g: &mut Geo, sn: &Snap, left: i32, right: i32, mut y: i32, rq: &str) -> i32 {
    let dpi = sn.dpi.max(96);
    let all = fav_tree_rows();
    let rows = crate::filter_fav_rows(&all, rq);
    if !rq.is_empty() && rows.is_empty() && !has(tx(LIST_H), rq) {
        return y;
    }
    g.items.push(Item::Head(RECT { left, top: y, right, bottom: y + sc(dpi, 28) }, LIST_H));
    y += sc(dpi, 28);
    let board = crate::layout_fav_board(&rows, left, right, y, dpi);
    g.items.push(Item::Table(board.table));
    g.items.push(Item::FavHead(board.head));
    if let Some(rect) = board.empty {
        g.items.push(Item::FavRow { i: usize::MAX, rect, rename: RECT::default(), btn: RECT::default() });
    }
    for slot in board.slots {
        g.items.push(Item::FavRow { i: slot.index, rect: slot.rect, rename: slot.rename, btn: slot.delete });
    }
    board.bottom + sc(dpi, 8)
}

fn key_items(g: &mut Geo, sn: &Snap, left: i32, right: i32, mut y: i32, rq: &str) -> i32 {
    // A nav query that matches no page must not fall through to this empty table.
    // `rq` filters rows, but the bar, header and a blank row were still emitted.
    if search_miss(&sn.query) {
        return y;
    }
    let dpi = sn.dpi.max(96);
    // Both queries apply: a key typed in the page box, and a nav query that did not match the title.
    let rows: Vec<usize> = (0..crate::keys::len()).filter(|&i| {
        (rq.is_empty() || key_hit_i(i, rq)) && (sn.keyq.is_empty() || key_hit_i(i, &sn.keyq))
    }).collect();
    let reset = right_box(RECT { left, top: y, right, bottom: y + sc(dpi, 40) }, sc(dpi, 210), sc(dpi, 32), 0);
    let edit = RECT { left, top: reset.top, right: reset.left - sc(dpi, 8), bottom: reset.bottom };
    g.items.push(Item::KeyBar { edit, reset });
    y += sc(dpi, 48);
    let head_h = sc(dpi, 32);
    let table_top = y;
    let probe = RECT { left, top: 0, right, bottom: 0 };
    let labels: Vec<String> = rows.iter().map(|&i| crate::keys::chord_text(i)).collect();
    let heights = unsafe { win::key_row_heights(&labels, probe, dpi, sn.mono) };
    let body: i32 = heights.iter().sum();
    let table = RECT { left, top: table_top, right, bottom: table_top + head_h + body };
    g.items.push(Item::Table(table));
    g.items.push(Item::KeyHead(RECT { left, top: table_top, right, bottom: table_top + head_h }));
    y = table_top + head_h;
    if rows.is_empty() {
        let h = heights.first().copied().unwrap_or(sc(dpi, 40));
        g.items.push(Item::KeyRow { i: usize::MAX, rect: RECT { left, top: y, right, bottom: y + h }, change: RECT::default(), reset: RECT::default() });
        y += h;
    }
    for (n, i) in rows.into_iter().enumerate() {
        let h = heights.get(n).copied().unwrap_or(sc(dpi, 40));
        let rect = RECT { left, top: y, right, bottom: y + h };
        let (change, reset) = win::key_btn_rects(rect, dpi);
        g.items.push(Item::KeyRow { i, rect, change, reset });
        y += h;
    }
    y += sc(dpi, 8);
    g.items.push(Item::Note(RECT { left, top: y, right, bottom: y + sc(dpi, 40) }, KEY_NOTE));
    y + sc(dpi, 40)
}

fn is_ev_card(spec: &Spec) -> bool {
    matches!(spec.ctl, Ctl::EvStatus | Ctl::EvAct | Ctl::Toggle(TG_EV))
}

fn is_wm_card(spec: &Spec) -> bool {
    matches!(spec.ctl, Ctl::WmStatus | Ctl::WmPath)
}

fn is_preview_card(spec: &Spec) -> bool {
    matches!(spec.ctl, Ctl::Toggle(TG_PREVIEW) | Ctl::Toggle(TG_SPACE) | Ctl::Toggle(TG_WHEEL))
}

fn toggle_on(id: u8) -> bool {
    match id {
        TG_SINGLE => app(|a| a.s.single),
        TG_ONE => app(|a| a.s.one_window),
        TG_DBLUP => app(|a| a.s.dblup),
        TG_HIDDEN => unsafe { super::hidden() },
        TG_BAR => app(|a| a.s.bar),
        TG_DRIVES => app(|a| a.s.drive_buttons),
        TG_STATUS => app(|a| a.s.stat),
        TG_EXT => app(|a| a.s.ext),
        TG_TREE => app(|a| a.s.tree_on),
        TG_PREVIEW => app(|a| a.s.preview_on),
        TG_EDGE => app(|a| a.s.edge),
        TG_CONFIRM => app(|a| a.s.confirm),
        TG_AUTOSAVE => app(|a| a.s.autosave),
        TG_DOT => app(|a| a.s.dot),
        TG_UPDATE => app(|a| a.s.update_check),
        TG_EV => app(|a| a.s.use_everything),
        TG_SPACE => app(|a| a.s.space_preview),
        TG_WHEEL => app(|a| a.s.peek_wheel_nav),
        TG_REMEMBER => app(|a| a.s.remember_views),
        TG_FSIZE => app(|a| a.s.folder_sizes),
        TG_FSIZE_NET => app(|a| a.s.folder_sizes_network),
        TG_FAST => app(|a| a.s.fast_copy),
        TG_DIALOG => app(|a| a.s.dialog_jump),
        TG_FOLDER => crate::folderapp::shown_on(),
        _ => false,
    }
}

fn ctl_id(c: Ctl) -> Option<i32> {
    Some(match c {
        Ctl::Lang => S_LANG,
        Ctl::Face => S_FACE,
        Ctl::Start => S_START,
        Ctl::Theme => S_THEME,
        Ctl::ViewMode => S_VIEW,
        Ctl::StartDir => S_DIR,
        Ctl::DelMode => S_DEL,
        Ctl::NewName => S_NEW,
        Ctl::Drop => S_DROP,
        Ctl::Layout => S_LAYOUT,
        Ctl::SideOrder => S_SIDE,
        Ctl::Sess => S_SESS,
        Ctl::About => return None,
        Ctl::Accent => S_ACCENT,
        _ => return None,
    })
}


pub(crate) mod store;
mod win;
pub(crate) use win::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn ui_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct Lang(u8);
    impl Drop for Lang {
        fn drop(&mut self) {
            crate::UI.store(self.0, Ordering::Relaxed);
        }
    }

    fn lang(v: u8) -> Lang {
        let prev = crate::UI.load(Ordering::Relaxed);
        crate::UI.store(v, Ordering::Relaxed);
        Lang(prev)
    }

    #[test]
    fn about_nav_fits_above_footer_when_short() {
        let sn = shot(9, "");
        // Client shorter than 10*36 + search + footer — rows must compress.
        let rc = RECT { left: 0, top: 0, right: 1000, bottom: 420 };
        let g = geo(rc, &sn);
        assert!(g.show[9], "About nav must stay visible");
        assert!(
            g.nav[9].bottom <= g.footer.top,
            "About nav must sit above footer: {:?} vs footer {:?}",
            g.nav[9],
            g.footer
        );
        let cx = (g.nav[9].left + g.nav[9].right) / 2;
        let cy = (g.nav[9].top + g.nav[9].bottom) / 2;
        assert!(inside(g.nav[9], cx, cy));
    }

    #[test]
    fn about_card_has_no_link_button() {
        assert_eq!(ctl_id(Ctl::About), None);
        assert_eq!(ABOUT[0].title, crate::Key::Maguro);
        assert_eq!(ABOUT[0].desc, crate::Key::FastMultiPaneFileManagerForWindows);
        let desc = tx(ABOUT[0].desc);
        let low = desc.to_ascii_lowercase();
        assert!(!low.contains("github"), "{desc}");
        assert!(!desc.contains("http"), "{desc}");
        assert!(!low.contains("samek86"), "{desc}");
        assert!(!desc.contains("efww/grids"), "{desc}");
    }

    fn shot(page: usize, query: &str) -> Snap {
        Snap {
            page,
            scroll: 0,
            dpi: 96,
            query: query.to_string(),
            keyq: String::new(),
            hot: None,
            font: HFONT::default(),
            btn_bold: HFONT::default(),
            small: HFONT::default(),
            bold: HFONT::default(),
            title_font: HFONT::default(),
            mono: HFONT::default(),
            icon: HFONT::default(),
            ctx_open: Vec::new(),
        }
    }

    fn client() -> RECT {
        RECT { left: 0, top: 0, right: 1000, bottom: 700 }
    }

    #[test]
    fn no_results_copy_follows_the_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        assert_eq!(tx(NO_RESULTS), "No results");
        crate::UI.store(1, Ordering::Relaxed);
        assert_eq!(tx(NO_RESULTS), "검색 결과 없음");
        crate::UI.store(2, Ordering::Relaxed);
        assert_eq!(tx(NO_RESULTS), "検索結果なし");
    }

    #[test]
    fn typing_agy_does_not_open_shortcuts_or_lay_out_its_table() {
        let _lock = ui_lock();
        let _keep = lang(1);
        // "Ag" still opens Shortcuts first (PageUp / PageDown). The sessions path
        // "%APPDATA%\\Maguro\\maguro-sessions" also contains "ag", so "Ag" is not unique.
        // "Agy" matches nothing. Keeping the page a prefix opened is what showed the empty table.
        assert!(page_match("Ag", 5));
        assert!(page_match("Ag", 6));
        assert_eq!((0..PAGES.len()).find(|&p| page_match("Ag", p)), Some(5));
        assert!(search_miss("Agy"));
        assert!((0..PAGES.len()).all(|p| !page_match("Agy", p)));

        crate::UI.store(2, Ordering::Relaxed);
        assert_eq!((0..PAGES.len()).find(|&p| page_match("Ag", p)), Some(5));
        assert!(search_miss("Agy"));
        crate::UI.store(1, Ordering::Relaxed);

        let mut anchor = 0usize;
        let mut page = 0usize;
        for q in ["A", "Ag", "Agy"] {
            (anchor, page) = commit_search(anchor, page, q);
        }
        assert_eq!(anchor, 0);
        assert_eq!(page, 0, "Agy must not leave Shortcuts selected");

        let g = geo(client(), &shot(page, "Agy"));
        assert!(g.show.iter().all(|on| !*on), "nav has no items");
        assert!(g.items.is_empty(), "content must not lay out the shortcuts table");
        assert!(!g.has_bar);

        (anchor, page) = commit_search(anchor, page, "");
        assert_eq!((anchor, page), (0, 0));
        let restored = geo(client(), &shot(page, ""));
        assert!(restored.show.iter().all(|on| *on));
        assert!(!restored.items.is_empty(), "clearing the search restores the page");
    }

    #[test]
    fn a_miss_keeps_shortcuts_when_that_was_the_page_but_hides_the_table() {
        let _lock = ui_lock();
        let _keep = lang(0);
        assert_eq!(pick_search(5, 5, "Agy"), 5);
        assert_eq!(commit_search(5, 5, "Agy"), (5, 5));
        assert!(search_miss("Agy"));
        let g = geo(client(), &shot(5, "Agy"));
        assert!(g.items.is_empty());
        assert!(g.show.iter().all(|on| !*on));
        // The page the user picked still wins when the query actually matches it.
        crate::UI.store(1, Ordering::Relaxed);
        assert_eq!(pick_search(0, 0, "단축키"), 5);
        assert_eq!(pick_search(2, 5, "단축키"), 5);
        assert!(!search_miss("단축키"));
        assert!(!search_miss(""));
    }

    #[test]
    fn default_folder_app_toggle_is_on_general_in_each_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let hit = |g: &Geo| g.items.iter().any(|it| matches!(it, Item::Card(c) if c.spec.ctl == Ctl::Toggle(TG_FOLDER)));
        let (anchor, page) = commit_search(2, 2, "Use as default folder app");
        assert_eq!((anchor, page), (2, 0));
        let g = geo(client(), &shot(page, "Use as default folder app"));
        assert!(g.show[0] && hit(&g));
        assert!(hit(&geo(client(), &shot(0, "Win+E and special folders"))));

        crate::UI.store(1, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "기본 폴더 앱으로 사용");
        assert_eq!(page, 0);
        assert!(hit(&geo(client(), &shot(page, "기본 폴더 앱으로 사용"))));
        assert!(hit(&geo(client(), &shot(0, "특수 폴더"))));

        crate::UI.store(2, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "既定のフォルダーアプリとして使用");
        assert_eq!(page, 0);
        assert!(hit(&geo(client(), &shot(page, "既定のフォルダーアプリとして使用"))));
        assert!(hit(&geo(client(), &shot(0, "特殊フォルダー"))));
    }

    #[test]
    fn folder_view_memory_is_on_the_panes_page_in_each_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let hit = |g: &Geo, ctl: Ctl| g.items.iter().any(|it| matches!(it, Item::Card(c) if c.spec.ctl == ctl));
        let (anchor, page) = commit_search(0, 0, "Remember view per folder");
        assert_eq!((anchor, page), (0, 2));
        let g = geo(client(), &shot(page, "Remember view per folder"));
        assert!(hit(&g, Ctl::Toggle(TG_REMEMBER)));
        assert!(hit(&geo(client(), &shot(2, "folder-views.json")), Ctl::Toggle(TG_REMEMBER)));
        assert!(hit(&geo(client(), &shot(2, "Reset this folder")), Ctl::FolderReset));
        assert!(hit(&geo(client(), &shot(2, "Clear all remembered")), Ctl::FolderClear));
        assert!(hit(&geo(client(), &shot(2, "Default view for every folder")), Ctl::FolderDefault));

        crate::UI.store(1, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "폴더마다 보기");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "폴더마다 보기")), Ctl::Toggle(TG_REMEMBER)));
        assert!(hit(&geo(client(), &shot(2, "이 폴더의 보기")), Ctl::FolderReset));
        assert!(hit(&geo(client(), &shot(2, "기억한 보기를 모두")), Ctl::FolderClear));

        crate::UI.store(2, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "フォルダーごとに表示");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "フォルダーごとに表示")), Ctl::Toggle(TG_REMEMBER)));
        assert!(hit(&geo(client(), &shot(2, "このフォルダーの表示")), Ctl::FolderReset));
        assert!(hit(&geo(client(), &shot(2, "記憶した表示をすべて")), Ctl::FolderClear));
    }

    #[test]
    fn folder_size_toggles_are_on_the_panes_page_in_each_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let hit = |g: &Geo, id: u8| g.items.iter().any(|it| matches!(it, Item::Card(c) if c.spec.ctl == Ctl::Toggle(id)));
        let (anchor, page) = commit_search(0, 0, "Show folder sizes");
        assert_eq!((anchor, page), (0, 2));
        assert!(hit(&geo(client(), &shot(page, "Show folder sizes")), TG_FSIZE));
        assert!(hit(&geo(client(), &shot(2, "Computed in the background")), TG_FSIZE));
        let (_, page) = commit_search(0, 0, "Folder sizes for network");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "Folder sizes for network")), TG_FSIZE_NET));

        crate::UI.store(1, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "폴더 크기 표시");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "폴더 크기 표시")), TG_FSIZE));
        let (_, page) = commit_search(0, 0, "네트워크 폴더 크기");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "네트워크 폴더 크기")), TG_FSIZE_NET));

        crate::UI.store(2, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "フォルダーのサイズを表示");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "フォルダーのサイズを表示")), TG_FSIZE));
        let (_, page) = commit_search(0, 0, "ネットワーク フォルダーのサイズ");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "ネットワーク フォルダーのサイズ")), TG_FSIZE_NET));
    }

    #[test]
    fn fast_copy_toggle_is_on_the_file_operations_page_in_each_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let hit = |g: &Geo, id: u8| g.items.iter().any(|it| matches!(it, Item::Card(c) if c.spec.ctl == Ctl::Toggle(id)));
        let (anchor, page) = commit_search(0, 0, "Fast copy for network");
        assert_eq!((anchor, page), (0, 3));
        assert!(hit(&geo(client(), &shot(page, "Fast copy for network")), TG_FAST));
        assert!(hit(&geo(client(), &shot(3, "Paste and drag use robocopy")), TG_FAST));

        crate::UI.store(1, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "네트워크 폴더 고속");
        assert_eq!(page, 3);
        assert!(hit(&geo(client(), &shot(page, "네트워크 폴더 고속")), TG_FAST));

        crate::UI.store(2, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "ネットワーク フォルダーの高速");
        assert_eq!(page, 3);
        assert!(hit(&geo(client(), &shot(page, "ネットワーク フォルダーの高速")), TG_FAST));
    }

    #[test]
    fn wheel_file_step_is_found_from_the_search_box() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let hit = |g: &Geo| g.items.iter().any(|it| matches!(it, Item::Card(c) if c.spec.ctl == Ctl::Toggle(TG_WHEEL)));
        let (anchor, page) = commit_search(0, 0, "Wheel moves to next");
        assert_eq!((anchor, page), (0, 2));
        let g = geo(client(), &shot(page, "Wheel moves to next"));
        assert!(g.show[2] && hit(&g));
        let g = geo(client(), &shot(2, "Scrolling past the end"));
        assert!(hit(&g));

        crate::UI.store(1, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "휠로 다음/이전");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "휠로 다음/이전"))));
        assert!(hit(&geo(client(), &shot(2, "내용 끝에서 마우스 휠"))));

        crate::UI.store(2, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "ホイールで次");
        assert_eq!(page, 2);
        assert!(hit(&geo(client(), &shot(page, "ホイールで次"))));
        assert!(hit(&geo(client(), &shot(2, "内容の端でさらにホイール"))));
    }

    #[test]
    fn clearing_a_real_shortcut_hit_stays_on_that_page() {
        let _lock = ui_lock();
        let _keep = lang(1);
        let (anchor, page) = commit_search(0, 0, "Ag");
        assert_eq!((anchor, page), (0, 5));
        assert_eq!(commit_search(anchor, page, ""), (5, 5));
    }

    #[test]
    fn a_miss_restores_the_page_from_before_search_moved() {
        let _lock = ui_lock();
        let _keep = lang(0);
        // "PageU" hits Shortcuts only. "Ag" also matches this Sessions page via the Maguro path.
        assert!(!page_match("PageU", 6));
        assert!(page_match("PageU", 5));
        let (anchor, page) = commit_search(6, 6, "PageU");
        assert_eq!(anchor, 6);
        assert_ne!(page, 6);
        assert_eq!(commit_search(anchor, page, "PageUx"), (6, 6));
        // Current page is kept when it still matches, even if an earlier page matches too.
        assert!(page_match("a", 3));
        assert_eq!(pick_search(0, 3, "a"), 3);
        let body = geo(client(), &shot(0, "language"));
        assert!(body.show[0]);
        assert!(!body.items.is_empty());
    }

    #[test]
    fn right_click_menu_page_is_found_in_each_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        assert_eq!(CTX_PAGE, 8);
        assert_eq!(PAGES.len(), PAGE_N);
        assert_eq!(tx(PAGES[CTX_PAGE]), "Right-click menu items");
        assert_eq!(tx(CTX_WIN11_NOTE), "Windows 11 apps start off. Turn on only what you need. Items already shown are not added again. Enabled ones sit with Open. Properties stays last.");
        assert_eq!(tx(CTX_RESET_L), "Reset to all enabled");
        assert_eq!(tx(CTX_REFRESH_L), "Refresh");
        assert!(page_match("Refresh", CTX_PAGE));
        let (anchor, page) = commit_search(0, 0, "Right-click menu");
        assert_eq!((anchor, page), (0, CTX_PAGE));
        let g = geo(client(), &shot(page, "Right-click menu"));
        assert!(g.show[CTX_PAGE]);
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxNote(_))));
        let (reset, refresh) = g.items.iter().find_map(|it| match it {
            Item::CtxBar { reset, refresh } => Some((*reset, *refresh)),
            _ => None,
        }).expect("ctx bar");
        assert_eq!(reset.right - reset.left, 220);
        assert_eq!(refresh.right - refresh.left, 140);
        assert_eq!(reset.bottom - reset.top, 32);
        assert_eq!(refresh.bottom - refresh.top, 32);
        assert_eq!(refresh.top, reset.top);
        assert_eq!(reset.left - refresh.right, 8);
        assert!(refresh.left >= g.content.left);

        crate::UI.store(1, Ordering::Relaxed);
        assert_eq!(tx(PAGES[CTX_PAGE]), "우클릭 메뉴 항목");
        assert_eq!(tx(CTX_WIN11_NOTE), "Windows 11 앱은 꺼진 채로 시작해요. 필요한 것만 켜세요. 이미 있는 항목은 다시 넣지 않아요. 켠 항목은 열기 옆에 나오고, 속성은 맨 마지막이에요.");
        assert_eq!(tx(CTX_RESET_L), "모두 사용");
        assert_eq!(tx(CTX_REFRESH_L), "새로고침");
        assert!(page_match("새로고침", CTX_PAGE));
        assert_eq!(tx(CTX_EMPTY), "타사 메뉴 항목이 없어요");
        let (_, page) = commit_search(0, 0, "우클릭 메뉴 항목");
        assert_eq!(page, CTX_PAGE);
        let g = geo(client(), &shot(page, "우클릭"));
        assert!(g.show[CTX_PAGE]);
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxBar { .. })));

        crate::UI.store(2, Ordering::Relaxed);
        assert_eq!(tx(PAGES[CTX_PAGE]), "右クリックメニュー項目");
        assert_eq!(tx(CTX_WIN11_NOTE), "Windows 11 のアプリはオフから始まります。必要なものだけオンにしてください。既にある項目は重ねません。オンにした項目は「開く」の近くに入り、プロパティは最後のままです。");
        assert_eq!(tx(CTX_RESET_L), "すべて有効に戻す");
        assert_eq!(tx(CTX_NOTE), "Maguro の右クリックメニューに出る項目です。開く、コピー、プロパティなどの Windows の項目はそのままです。新しい項目はオンで始まります。エクスプローラーは変わりません。");
        let (_, page) = commit_search(0, 0, "右クリックメニュー");
        assert_eq!(page, CTX_PAGE);
        let g = geo(client(), &shot(page, "右クリック"));
        assert!(g.show[CTX_PAGE]);
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxNote(_))));
    }

    struct View(bool);
    impl Drop for View {
        fn drop(&mut self) {
            app(|a| a.s.ctx_win11_view = self.0);
            crate::shell::ctxpack::set_test_rows(None);
        }
    }

    fn pin_view(win11: bool) -> View {
        let prev = app(|a| a.s.ctx_win11_view);
        app(|a| a.s.ctx_win11_view = win11);
        View(prev)
    }

    #[test]
    fn ctx_segment_sits_above_refresh_with_an_8_dip_gap() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let _view = pin_view(false);
        assert_eq!(ctx_note_key(), CTX_NOTE);
        assert!(page_match("Classic menu", CTX_PAGE));
        assert!(page_match("Windows 11 menu", CTX_PAGE));
        assert_eq!(tx(CTX_CLASSIC), "Classic menu");
        assert_eq!(tx(CTX_WIN11), "Windows 11 menu");
        let g = geo(client(), &shot(CTX_PAGE, "Right-click menu"));
        let note = g.items.iter().find_map(|it| match it {
            Item::CtxNote(r) => Some(*r),
            _ => None,
        }).expect("note");
        let (classic, win11) = g.items.iter().find_map(|it| match it {
            Item::CtxSeg { classic, win11 } => Some((*classic, *win11)),
            _ => None,
        }).expect("segment");
        let (reset, refresh) = g.items.iter().find_map(|it| match it {
            Item::CtxBar { reset, refresh } => Some((*reset, *refresh)),
            _ => None,
        }).expect("ctx bar");
        assert_eq!(classic.bottom - classic.top, 32);
        assert_eq!(win11.bottom - win11.top, 32);
        assert_eq!(classic.top, win11.top);
        assert_eq!(win11.left - classic.right, 8);
        assert_eq!(classic.left, note.left);
        assert_eq!(classic.right - classic.left, 116);
        assert_eq!(win11.right - win11.left, 137);
        assert_eq!(refresh.top - classic.bottom, 8);
        assert_eq!(refresh.top, reset.top);
        assert_eq!(refresh.right - refresh.left, 140);
        assert_eq!(reset.right - reset.left, 220);
        assert_eq!(reset.left - refresh.right, 8);
    }

    #[test]
    fn win11_view_uses_the_empty_state_key() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let _view = pin_view(true);
        crate::shell::ctxpack::set_test_rows(Some(Vec::new()));
        assert_eq!(tx(CTX_WIN11_EMPTY), "No Windows 11 packaged context-menu apps found on this PC.");
        assert_eq!(ctx_note_key(), crate::Key::CtxWin11Note);
        let g = geo(client(), &shot(CTX_PAGE, "Right-click menu"));
        let key = g.items.iter().find_map(|it| match it {
            Item::CtxEmpty(_, k) => Some(*k),
            _ => None,
        }).expect("empty state");
        assert_eq!(key, crate::Key::CtxWin11Empty);
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxSeg { .. })));
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxBar { .. })));
        assert!(g.items.iter().any(|it| matches!(it, Item::CtxNote(_))));
    }

    #[test]
    fn drive_chip_fits_the_korean_label() {
        let _lock = ui_lock();
        let _keep = lang(1);
        let bits = crate::shell::ctxmenu::FILE | crate::shell::ctxmenu::FOLDER | crate::shell::ctxmenu::BACKGROUND | crate::shell::ctxmenu::DRIVE;
        let chips = ctx_chips(bits, RECT { left: 0, top: 0, right: 900, bottom: 48 }, 96);
        assert_eq!(chips.len(), 4);
        let (_, r) = chips.iter().find(|(b, _)| *b == crate::shell::ctxmenu::DRIVE).copied().unwrap();
        let w = r.right - r.left;
        assert!(w >= 18 + 13 * 4, "drive chip width {w}");
        assert_eq!(crate::shell::ctxmenu::target_label(crate::shell::ctxmenu::DRIVE), "드라이브");
    }

    #[test]
    fn left_panel_order_sits_under_the_panel_toggle_in_each_language() {
        let _lock = ui_lock();
        let _keep = lang(0);
        let panel = VIEW.iter().position(|c| c.ctl == Ctl::Toggle(TG_TREE)).unwrap();
        let order = VIEW.iter().position(|c| c.ctl == Ctl::SideOrder).unwrap();
        assert_eq!(order, panel + 1);
        let show = &VIEW[panel];
        assert_eq!(show.title, crate::Key::ShowLeftPanel);
        let show_desc = |i: u8| {
            let prev = crate::UI.load(Ordering::Relaxed);
            crate::UI.store(i, Ordering::Relaxed);
            let s = tx(show.desc);
            crate::UI.store(prev, Ordering::Relaxed);
            s
        };
        assert!(!show_desc(0).contains("on top") && !show_desc(0).contains("below"));
        assert!(!show_desc(1).contains("위는 폴더"));
        assert!(!show_desc(2).contains("上はフォルダー"));
        assert_eq!(VIEW[order].title, crate::Key::LeftPanelOrder);
        assert_eq!(SIDE_FAV, crate::Key::FavouritesOnTop);
        assert_eq!(SIDE_TREE, crate::Key::FolderTreeOnTop);
        let order_desc = |i: u8| {
            let prev = crate::UI.load(Ordering::Relaxed);
            crate::UI.store(i, Ordering::Relaxed);
            let s = tx(VIEW[order].desc);
            crate::UI.store(prev, Ordering::Relaxed);
            s
        };
        assert!(order_desc(0).contains("Favourites on top") && order_desc(0).contains("folder tree on top"));
        assert!(order_desc(1).contains("즐겨찾기 위") && order_desc(1).contains("폴더 트리 위"));
        assert!(order_desc(2).contains("お気に入りを上") && order_desc(2).contains("フォルダー ツリーを上"));
        assert_eq!(ctl_id(Ctl::SideOrder), Some(S_SIDE));
        assert!(VIEW[order].live);

        let ctls = |g: &Geo| {
            g.items.iter().filter_map(|it| match it {
                Item::Card(c) => Some(c.spec.ctl),
                _ => None,
            }).collect::<Vec<_>>()
        };
        let ordered = |g: &Geo| {
            let ctls = ctls(g);
            let p = ctls.iter().position(|c| *c == Ctl::Toggle(TG_TREE));
            ctls.iter().position(|c| *c == Ctl::SideOrder) == p.map(|i| i + 1)
        };
        assert!(ordered(&geo(client(), &shot(2, ""))));
        let has = |q: &str| ctls(&geo(client(), &shot(2, q))).contains(&Ctl::SideOrder);
        let (anchor, page) = commit_search(0, 0, "Left panel order");
        assert_eq!((anchor, page), (0, 2));
        assert!(has("Left panel order"));
        assert!(has("Favourites on top"));
        assert!(has("folder tree on top"));
        assert!(ctls(&geo(client(), &shot(2, "Show the left panel"))).contains(&Ctl::Toggle(TG_TREE)));

        crate::UI.store(1, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "왼쪽 패널 순서");
        assert_eq!(page, 2);
        assert!(ordered(&geo(client(), &shot(2, ""))));
        assert!(has("왼쪽 패널 순서") && has("즐겨찾기 위") && has("폴더 트리 위"));

        crate::UI.store(2, Ordering::Relaxed);
        let (_, page) = commit_search(0, 0, "左のパネルの順番");
        assert_eq!(page, 2);
        assert!(ordered(&geo(client(), &shot(2, ""))));
        assert!(has("左のパネルの順番") && has("お気に入りを上") && has("フォルダー ツリーを上"));
    }
}
