use crate::imports::*;

pub(crate) const MAXP: usize = 16;
pub(crate) const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
// child ids inside a pane
pub(crate) const ID_BACK: i32 = 101;
pub(crate) const ID_FWD: i32 = 102;
pub(crate) const ID_UP: i32 = 103;
pub(crate) const ID_EDIT: i32 = 104;
pub(crate) const ID_OPEN_IN: i32 = 105;
pub(crate) const ID_FILTER: i32 = 106;
pub(crate) const ID_FAV: i32 = 107; // ☆+ / ★: add or remove the pane's folder from favourites
pub(crate) const ID_PANE_ADD: i32 = 108; // add a pane tab; shown only while this pane has one tab
pub(crate) const ID_NETFAIL: i32 = 109; // covers the file list after a share does not answer
// Chrome buttons (children of the main window), on the right of the tab strip.
// Drive letters sit in that group, to the left of Layout.
pub(crate) const ID_TB_LAYOUT: i32 = 201;
pub(crate) const ID_TB_SAVE: i32 = 202;
pub(crate) const ID_TB_LOAD: i32 = 204;
pub(crate) const ID_TB_FAV: i32 = 206; // star, immediately left of the gear: favourites window
pub(crate) const ID_TB_SET: i32 = 207; // gear: settings menu (view, appearance, behavior, and settings)
// menu ids
pub(crate) const ID_HIDDEN: usize = 1100;
pub(crate) const ID_KEYS: usize = 1101;
pub(crate) const ID_DBLUP: usize = 1103;
pub(crate) const ID_LANG_AUTO: usize = 1110;
/// Language files, in [`language_choices`] order. English is the first.
pub(crate) const ID_LANG_FIRST: usize = 1800;
pub(crate) const ID_SETTINGS: usize = 1114;
pub(crate) const ID_PANEL: usize = 1116; // left panel (folder tree and favourites)
pub(crate) const ID_PREVIEW: usize = 1126; // right preview pane (active pane's focused file)
pub(crate) const ID_EXT: usize = 1117; // show file extensions (Windows setting)
pub(crate) const ID_FOLLOW: usize = 1118; // follow Windows light / dark
pub(crate) const ID_THEME: usize = 1122; // gear → Appearance: switch to the other of light / dark
pub(crate) const ID_BASKET: usize = 1125; // gear: file basket tool window
pub(crate) const ID_HOTKEY: usize = 1119; // global hotkey, checked only while it is registered
pub(crate) const ID_GATHER: usize = 1120; // folders from open Explorer windows and tabs
pub(crate) const ID_RECENT: usize = 1121; // gear: open the Windows Recent folder in this pane
pub(crate) const ID_REVEAL: usize = 1123; // show the selected recent file in its folder (recent-files menu and Ctrl+Enter, not the gear menu)
pub(crate) const ID_SESS_SAVE: usize = 1130; // gear: save the current tab
pub(crate) const ID_SESS_OPEN: usize = 1131; // gear: open a saved tab as a new tab
pub(crate) const ID_WATCH: usize = 1132; // gear: folder watch
pub(crate) const WM_WATCH: u32 = WM_APP + 75; // a watch thread posted a burst or a status (avoid preview 70-72)
pub(crate) const WM_WATCH_OPEN: u32 = WM_APP + 76; // open a notified folder as a new tab
pub(crate) const WM_WATCH_TOAST: u32 = WM_APP + 77; // balloon failed; show the in-app note
/// Gear → Workspaces. Each range is slot 0..8 (keys 1..9).
pub(crate) const ID_WS_SAVE: usize = 1900;
pub(crate) const ID_WS_LOAD: usize = 1910;
pub(crate) const ID_WS_RENAME: usize = 1920;
pub(crate) const ID_WS_CLEAR: usize = 1930;
pub(crate) const ID_WS_CONFIRM: usize = 1940;
pub(crate) const WM_THEME_SYNC: u32 = WM_APP + 47;
pub(crate) const WM_THEME_PANE: u32 = WM_APP + 63; // rebuild one shell view for the theme that just changed
pub(crate) const WM_THEME_SCROLL: u32 = WM_APP + 64; // scroll and select-all, after that view has rows
pub(crate) const WM_THEME_RELAUNCH: u32 = WM_APP + 65; // DirectUIHWND birth theme: save+relaunch after theme flip
pub(crate) const ID_HELP: usize = 1105;
pub(crate) const ID_ABOUT: usize = 1106;
pub(crate) const ID_ACCENT: usize = 1400; // + preset index
pub(crate) const ID_ACCENT_PICK: usize = 1410;
pub(crate) const EM_SETSEL: u32 = 0xB1;
pub(crate) const TIMER_STATUS: usize = 1;
pub(crate) const TIMER_FILTER: usize = 2;
pub(crate) const TIMER_REVEAL: usize = 3;
pub(crate) const TIMER_SETTLE: usize = 4; // one width pass after a folder change. Does not call SetColumns or SetViewMode.
pub(crate) const TIMER_SAVE: usize = 5; // debounced maguro.json write
pub(crate) const SAVE_SOON_MS: u32 = 1500;
pub(crate) const SAVE_LATEST_MS: u128 = 30_000;
/// Hidden tabs that still hold Explorer windows. Older hidden tabs keep only their saved grid.
pub(crate) const PARK_CACHE: usize = 1;
pub(crate) const TIMER_NEWF: usize = 6; // on a pane: select and rename a folder just created
pub(crate) const TIMER_NEWFILE: usize = 7; // on a pane: select and rename a file just created
pub(crate) const TIMER_WINS: usize = 8; // after the first window is shown, open the other saved windows
pub(crate) const TIMER_ICON: usize = 9; // favourites: pre-warm shell icon indexes after the first paint
pub(crate) const TIMER_PINS: usize = 10; // favourites: re-read Explorer home items after a pin or unpin
pub(crate) const TIMER_FOCUS: usize = 11; // a click brought the window back: restore focus only if nothing took it
pub(crate) const TIMER_FIND: usize = 12; // hide the search list after the filter loses focus
pub(crate) const TIMER_PEEK_ZOOM: usize = 13; // peek: re-render PDF pages after the wheel settles
pub(crate) const TIMER_FVIEWS: usize = 14; // debounced folder-views.json write
pub(crate) const TIMER_NET: usize = 15; // pane: a UNC or mapped drive did not resolve in time
pub(crate) const TIMER_KIDS: usize = 16; // pane: a share's subfolder list did not come back in time
pub(crate) const TIMER_CTX_WARM: usize = 17; // after the window is up, warm shell context-menu handlers off the UI thread
pub(crate) const TIMER_DROP: usize = 18; // re-wrap a shell drop target the view registered after we did
pub(crate) const TIMER_DLG_JUMP: usize = 19; // put a typed filename back after a dialog jump
pub(crate) const TIMER_DLG_WATCH: usize = 20; // register the dialog hotkey only while a file dialog is in front
pub(crate) const TIMER_RECENT: usize = 21; // recent files: sort newest-first, then select Show in folder
pub(crate) const TIMER_PREVIEW: usize = 22; // load the preview pane after the selection rests
pub(crate) const TIMER_CLIPSEL: usize = 23; // pane: select a file after a folder tab opens (folder watch)
pub(crate) const TIMER_RESUME: usize = 24; // pane: wait before the next automatic share reconnect
pub(crate) const TIMER_TAB_HOVER: usize = 25; // file drag resting on a window tab or pane tab
pub(crate) const TIMER_DLG_RETRY: usize = 26; // file dialog has no filename box yet; look again
pub(crate) const TIMER_SPIN: usize = 27; // pane: next frame of the F5 refresh spin
pub(crate) const FVIEW_SAVE_MS: u32 = 700;
pub(crate) const WM_FIND: u32 = WM_APP + 50; // search results arrived for this pane
pub(crate) const WM_EV: u32 = WM_APP + 51; // Everything status changed in the settings window
pub(crate) const WM_UPDATE: u32 = WM_APP + 60; // update worker: wParam 0 exe found, 1 installed, 2 failed, 3 zip only, 4 sha missing, 5 sha mismatch
pub(crate) const WM_ABOUT: u32 = WM_APP + 66; // deferred About so SendMessage(WM_COMMAND, ID_ABOUT) returns before the modal loop
// tree-view messages (the sidebar's SysTreeView32)
pub(crate) const TVM_GETNEXTITEM: u32 = 0x110A;
pub(crate) const TVM_ENSUREVISIBLE: u32 = 0x1114;
pub(crate) const TVM_GETITEMRECT: u32 = 0x1104;
pub(crate) const TVM_GETITEMHEIGHT: u32 = 0x111C;
pub(crate) const TVGN_CARET: usize = 9;
pub(crate) const TVM_SETBKCOLOR: u32 = 0x111D;
pub(crate) const TVM_GETTOOLTIPS: u32 = 0x1119;
// list-view messages (the Explorer list inside each pane)
pub(crate) const LVM_SETBKCOLOR: u32 = 0x1001;
pub(crate) const LVM_GETHEADER: u32 = 0x101F;
pub(crate) const LVM_GETTOOLTIPS: u32 = 0x104E;
pub(crate) const LVM_GETCOLUMNORDERARRAY: u32 = 0x103B;
pub(crate) const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
pub(crate) const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = 0x1037;
pub(crate) const LVS_EX_AUTOSIZECOLUMNS: usize = 0x1000_0000;
pub(crate) const LVS_EX_COLUMNSNAPPOINTS: usize = 0x4000_0000;
pub(crate) const HDM_GETITEMCOUNT: u32 = 0x1200;
pub(crate) const LVM_SETTEXTCOLOR: u32 = 0x1024;
pub(crate) const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
pub(crate) const LVM_SETSELECTEDCOLUMN: u32 = 0x108C;
pub(crate) const LVM_GETSELECTEDCOLUMN: u32 = 0x10AE;
pub(crate) const TVM_SETTEXTCOLOR: u32 = 0x111E;
pub(crate) const COPY_ID: usize = 0x504E_4752; // "PNGR": WM_COPYDATA from a second instance
// Move without copying old pixels: copied bits are what left ghost borders/buttons while resizing.
pub(crate) const MOVE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_NOCOPYBITS.0);

// RT_GROUP_ICON ids written by build.rs. All three groups embed assets/Maguro.ico,
// the cerulean three-panel mark. Explorer shows the lowest id, so group 1 is the exe file icon.
// The window class, title bar, taskbar, and About fallback use group 3 (light) and group 2 (dark).
/// Explorer file icon (`assets/Maguro.ico`). Windows reads this group from the exe.
#[allow(dead_code)]
pub(crate) const ICON_GROUP_EXE: usize = 1;
pub(crate) const ICON_GROUP_DARK: usize = 2;
pub(crate) const ICON_GROUP_LIGHT: usize = 3;

// Explorer picks the lowest RT_GROUP_ICON, so the file icon stays below both in-app groups.
const _: () = assert!(ICON_GROUP_EXE < ICON_GROUP_DARK && ICON_GROUP_EXE < ICON_GROUP_LIGHT);

/// Title bar and About fallback. Light is group 3 and dark is group 2.
/// Both groups embed `assets/Maguro.ico`.
pub(crate) fn window_icon_group(dark: bool) -> usize {
    if dark { ICON_GROUP_DARK } else { ICON_GROUP_LIGHT }
}

