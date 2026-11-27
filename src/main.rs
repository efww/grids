// Grids: multi-pane file manager. Shell tree on the left, up to 4 columns of up to 4 Explorer views on the right.
// Each pane hosts the real Explorer control (IExplorerBrowser), so context menus, drag & drop, rename,
// thumbnails, columns etc. come from Windows itself.
#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, MoveFileExW, FILE_FLAGS_AND_ATTRIBUTES, MOVE_FILE_FLAGS};
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Diagnostics::Debug::{GetErrorMode, MessageBeep, SetErrorMode, SEM_FAILCRITICALERRORS, THREAD_ERROR_MODE};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS, SFGAO_FLAGS, SFGAO_FOLDER, SFGAO_STREAM};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, ReleaseMutex, WaitForSingleObject, SetEvent, OpenProcess, INFINITE,QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{
    AccessibleObjectFromPoint, LiveSetting_Property_GUID, NotifyWinEvent, Polite, CLSID_AccPropServices, IAccPropServices, IAccessible, PROPID_ACC_DESCRIPTION, PROPID_ACC_NAME, PROPID_ACC_ROLE,
    ROLE_SYSTEM_DIALOG, ROLE_SYSTEM_LIST, ROLE_SYSTEM_LISTITEM, ROLE_SYSTEM_STATUSBAR,
};
use windows::Win32::UI::Controls::Dialogs::{ChooseColorW, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW};
use windows::Win32::UI::Controls::{
    SetWindowTheme, BeginBufferedPaint, BufferedPaintInit, EndBufferedPaint, BPBF_TOPDOWNDIB, DRAWITEMSTRUCT, ODS_SELECTED, TOOLTIPS_CLASSW,
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDDS_SUBITEM, CDIS_FOCUS, CDIS_HOT, CDIS_SELECTED, CDRF_NOTIFYITEMDRAW, CDRF_NOTIFYSUBITEMDRAW, CDRF_SKIPDEFAULT,
    LVCDI_GROUP, LVGA_HEADER_CENTER, LVGA_HEADER_RIGHT, LVGF_ALIGN, LVGF_EXTENDEDIMAGE, LVGF_HEADER, LVGF_STATE, LVGF_TITLEIMAGE, LVGGR_HEADER,
    LVGROUP, LVGROUP_MASK, LVGS_COLLAPSED, LVGS_COLLAPSIBLE,
    LVGS_FOCUSED, LVGS_SELECTED, LVGS_SUBSETED, LIST_VIEW_GROUP_STATE_FLAGS, LVM_GETGROUPINFO, LVM_GETGROUPRECT, LVM_GETIMAGELIST, LVSIL_GROUPHEADER,
    HDF_CENTER, HDF_RIGHT, HDF_SORTDOWN, HDF_SORTUP, HDITEMW, HDI_FORMAT, HDI_TEXT, HDM_GETITEMW, HDN_BEGINTRACKW, HDN_ITEMCHANGINGW, HDN_TRACKW, HDI_WIDTH, NMHEADERW, LVCF_MINWIDTH, LVCOLUMNW, LVM_GETCOLUMNW, LVM_SETCOLUMNW,
    LVM_GETCOLUMNWIDTH, LVM_SETCOLUMNWIDTH, EM_GETMARGINS, NMCUSTOMDRAW, NMHDR, NMLVCUSTOMDRAW, NM_CUSTOMDRAW,
    TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_DELTOOLW, TTM_UPDATETIPTEXTW, TTTOOLINFOW, WM_MOUSELEAVE, IImageList, ILD_NORMAL,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::Common::{COMDLG_FILTERSPEC, ITEMIDLIST, STRRET};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;


mod ui_defs;
mod session;
mod geometry;
mod state;
mod fonts;
mod draw;
mod nav;
mod tree;
mod panes;
mod sessions_ui;
mod status;
mod paint;
mod layout_picker;
mod load_popup;
mod favourites;
mod docparse;
mod preview;
mod hwpdoc;
mod peek;
mod keys_window;
mod paste;
mod select_pattern;
mod rename_rules;
mod rename;
mod keyboard;
mod tabs;
mod window_ui;
mod dialog_jump;
mod shellx;
mod everything;
mod update;
mod star;
mod hashsum;
mod organize;
mod jumplist;
mod autostart;
mod tour;
mod tray;
mod usage;
mod listing;
mod clipadd;
mod switcher;
mod photos;

use ui_defs::*;
// explicit, so it wins over the same name glob-imported from windows (a glob clash would turn match patterns into bindings)
use ui_defs::EN_CHANGE;
use session::*;
use geometry::*;
use state::*;
use fonts::*;
use draw::*;
use nav::*;
use tree::*;
use panes::*;
use sessions_ui::*;
use status::*;
use paint::*;
use layout_picker::*;
use load_popup::*;
use favourites::*;
use docparse::*;
use preview::*;
use peek::*;
use keys_window::*;
use paste::*;
use select_pattern::*;
use rename::*;
use keyboard::*;
use tabs::*;
use window_ui::*;
use shellx::*;
use everything::*;
use update::*;
use star::*;
use hashsum::*;
use organize::*;
use jumplist::*;
use autostart::*;
use tour::*;
use tray::*;
use usage::*;
use listing::*;
use clipadd::*;
use switcher::*;
use photos::*;

/// Let a drive with no disk (empty card reader slot, DVD drive, pulled USB stick) just fail the file call,
/// instead of Windows' "No Disk" system box freezing the calling thread until it is answered.
/// Set once before any thread starts, so every thread follows it; bits already set are kept,
/// and the GP-fault box is left alone (crash reporting stays as it was).
fn quiet_critical_errors() {
    unsafe {
        SetErrorMode(THREAD_ERROR_MODE(GetErrorMode()) | SEM_FAILCRITICALERRORS);
    }
}

fn main() -> Result<()> {
    quiet_critical_errors();
    unsafe {
        let mut raw: Vec<String> = std::env::args().skip(1).collect();
        let startup = take_startup_flag(&mut raw); // the Run value at sign-in: minimized, no focus taken
        let args: Vec<String> = menu_args(raw).into_iter().map(resolve).collect();
        // One start at a time, held until the message loop runs (released there, or on any early return).
        let Some(mut start) = StartLock::take() else {
            // another start has held it for 10 s (slow drive, a dialog): going on would open a second window.
            // At sign-in (--startup) leave quietly: no box popping up on the fresh desktop.
            if startup {
                return Ok(());
            }
            MessageBoxW(
                None,
                w!("Grids is still starting. Try again once its window is open.\n\nGrids가 아직 시작하는 중입니다. 창이 열린 뒤 다시 실행해 주세요."),
                w!("Grids"),
                MB_OK | MB_ICONINFORMATION,
            );
            return Ok(());
        };
        // Single instance: hand the arguments to the running window and leave.
        if let Some(prev) = running_instance() {
            start.release();
            // Grids on another virtual desktop comes to this one (only its own process can move it): asked before
            // the paths, which may focus a pane or open a dialog. Not at sign-in with nothing to open.
            let quiet = startup && args.is_empty();
            let follow = || {
                if !quiet {
                    let mut r = 0usize;
                    _ = SendMessageTimeoutW(prev, follow_msg(), WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 2000, Some(&mut r));
                }
            };
            follow();
            if !args.is_empty() {
                let data: Vec<u16> = args.join("\n").encode_utf16().collect();
                let cd = CopyData { dw: COPY_ID, cb: (data.len() * 2) as u32, lp: data.as_ptr() as _ };
                let lp = LPARAM(&cd as *const CopyData as isize);
                let mut reply = 0usize;
                let sent = SendMessageTimeoutW(prev, WM_COPYDATA, WPARAM(0), lp, SMTO_ABORTIFHUNG, 5000, Some(&mut reply));
                // the window answers 1 once it has the paths; a hung or busy Grids must not lose them silently
                if sent.0 == 0 || reply != 1 {
                    let shown = args.iter().filter(|a| *a != FOLDER_FLAG).cloned().collect::<Vec<_>>().join("\n");
                    let text = HSTRING::from(format!(
                        "Grids is running but answered late, so this may not have opened:\n{shown}\n\n\
                         Grids가 실행 중이지만 응답이 늦어 다음이 열리지 않았을 수 있습니다:\n{shown}"
                    ));
                    MessageBoxW(None, &text, w!("Grids"), MB_OK | MB_ICONWARNING);
                }
            }
            if quiet {
                return Ok(()); // sign-in start with Grids already open: leave it where it is
            }
            if IsIconic(prev).as_bool() {
                _ = ShowWindow(prev, SW_RESTORE);
                follow(); // hidden in the notification area it could only move once shown
            }
            _ = SetForegroundWindow(prev);
            return Ok(());
        }

        OleInitialize(None)?; // OLE (not just COM) for drag & drop
        let shell_refs = hold_shell_refs(); // Windows' own copies count on it; waited for after the message loop
        // A grids.ini that is there but unreadable (locked, no access) must not be overwritten with defaults:
        // retry, or quit before anything could save. The language is not known yet, so both.
        let ini = loop {
            match load_ini(ini_path().parent().unwrap_or(Path::new("."))) {
                Ok(t) => break t,
                Err(e) => {
                    let text = HSTRING::from(format!(
                        "Grids could not read its settings file (grids.ini): {e}. Retry, or Cancel to quit without changing it.\n\n\
                         Grids 설정 파일(grids.ini)을 읽지 못했습니다: {e}. 다시 시도하거나, 취소하면 파일을 바꾸지 않고 종료합니다."
                    ));
                    if MessageBoxW(None, &text, w!("Grids"), MB_RETRYCANCEL | MB_ICONWARNING) != IDRETRY {
                        return Ok(());
                    }
                }
            }
        };
        let fresh = ini.is_none(); // a new install: the getting started tour once the window is up
        // a new install (no settings yet) follows Windows' light/dark
        let mut s = ini.map(|t| Session::parse(&t)).unwrap_or(Session { follow: true, ..Session::default() });
        if let Some(d) = theme_target(s.follow, system_dark(), s.dark) {
            s.dark = d;
        }
        refresh_sys_accent(); // Windows' accent colour, when followed, from the first paint
        let bare = args.is_empty(); // no folder or session given
        // a session file becomes the save target; one that can't be read is named on the status line once the window is up
        let named = session_arg(&args).map(PathBuf::from);
        // folders from "Open in Grids" fill panes 1.. (apply_args); more selected folders, each its own start, go on after them
        let burst = Some(args.len().saturating_sub(1).min(MAXP)).filter(|&k| k > 0 && args[0] == FOLDER_FLAG);
        let opened = startup_args(&mut s, args, |f| std::fs::read_to_string(f).ok());
        let unread = named.filter(|_| !opened);

        _ = BufferedPaintInit();

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(main_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default(), // app.ico, group id 1
            lpszClassName: w!("GridsMain"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS, // the plate is drawn to the window size
            lpfnWndProc: Some(pane_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsPane"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(bar_proc),
            hInstance: hinst,
            lpszClassName: w!("GridsBar"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(fav_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsFav"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(keys_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsKeys"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(tour_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsTour"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(load_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsLoad"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(grid_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsPicker"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(match_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsMatch"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(peek_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsPeek"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(peek_host_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsPeekHost"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(rename_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default(),
            lpszClassName: w!("GridsRename"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW | CS_DBLCLKS, // double-click a column border: default widths
            lpfnWndProc: Some(ev_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsEv"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW | CS_DBLCLKS, // double-click a folder row: into it
            lpfnWndProc: Some(usage_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsUsage"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(sw_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsSwitch"),
            ..Default::default()
        });
        KO.store(s.ko, Ordering::Relaxed);
        TABS_ON.store(s.tabs_on, Ordering::Relaxed);
        refresh_hc(); // a contrast theme already on: its colours from the first paint
        let view_menu = make_menus()?;
        let main = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("GridsMain"),
            w!("Grids"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1400,
            900,
            None,
            None,
            Some(hinst),
            None,
        )?;
        set_caption(main);
        let square = DWMWCP_DONOTROUND;
        _ = DwmSetWindowAttribute(main, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
        let dpi = GetDpiForWindow(main) as i32;
        // grids.ini's tree width is at the DPI it was saved at, and the window is made on the primary monitor first:
        // taken to this DPI here, the hidden move to its saved place below (WM_DPICHANGED) brings it back exactly
        let win_dpi = || {
            let [l, t, r, b, _] = s.win?;
            mon_dpi(MonitorFromRect(&RECT { left: l, top: t, right: r, bottom: b }, MONITOR_DEFAULTTONEAREST)).map(|d| d as i32)
        };
        let from = side_from(s.side_dpi, win_dpi());
        (s.side, s.side_dpi) = (start_side(s.side, from, dpi), dpi);
        // the place last used with the monitors connected now, else the last place; either goes through fit_saved below
        let sig = monitor_sig();
        let win = pick_place(&s.places, sig.as_deref(), s.win);
        let tip_style = WS_POPUP | WINDOW_STYLE(0x01 | 0x02); // TTS_ALWAYSTIP | TTS_NOPREFIX
        let tip = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, None, tip_style, 0, 0, 0, 0, Some(main), None, None, None)?;
        app(|a| (a.s, a.main, a.dpi, a.view_menu, a.tip) = (s, main, dpi, view_menu, tip));
        // Grids places the window itself below, so the first save is not a setup change to hold for
        app(|a| a.place_sig = sig.unwrap_or_default());
        if startup {
            // from here panes whose folder is not reachable yet are noted for a retry (sync_panes)
            app(|a| (a.quiet, a.quiet_max) = (Quiet::Placing, matches!(win, Some([.., m]) if m != 0)));
        }
        let bar_ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let mut bars = [HWND::default(); 2];
        for b in &mut bars {
            *b = CreateWindowExW(bar_ex, w!("GridsBar"), w!(""), WS_POPUP, 0, 0, 0, 0, Some(main), None, None, None)?;
            _ = SetLayeredWindowAttributes(*b, COLORREF(0), 170, LWA_ALPHA);
        }
        app(|a| a.bars = bars);
        for (id, label, hint) in toolbar_tips() {
            add_tip(child(main, w!("BUTTON"), PCWSTR(HSTRING::from(label).as_ptr()), BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0), id, HFONT::default()), hint);
        }
        make_bin(main); // after the drive buttons; made once here, never by refresh_drives
        create_status(main);
        if let Ok((t, th)) = create_tree(main) {
            app(|a| (a.tree, a.tree_hwnd) = (Some(t), th));
        }
        apply_theme();
        apply_font();
        refresh_drives(); // laid out by the relayout below
        refresh_clouds(); // after them, before the bin
        cache_folder_icon(); // tabs of folders not opened yet this run
        cache_pc_icon(); // This PC in the Ctrl+G and recent folder menus
        if app(|a| a.s.updates) {
            check_star(); // ticks View > Star once gh says so; network, so only with updates on
        }
        start_updates(main); // its button, hidden until a newer release turns up
        // keep the whole window on the work area, whatever screen it was saved on
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        match win {
            Some([l, t, r, b, max]) => {
                let saved = RECT { left: l, top: t, right: r, bottom: b };
                let mut fixed = saved;
                let mon = MonitorFromRect(&saved, MONITOR_DEFAULTTONEAREST);
                if GetMonitorInfoW(mon, &mut mi).as_bool() {
                    // a rect saved snapped to a half or quarter comes back as it was, its invisible border included
                    // (also when maximized: its restore rect may be such a snap rect, reopened there earlier)
                    fixed = fit_saved(saved, mi.rcMonitor, mi.rcWork, snap_slack(mon));
                    // still hidden: move onto the target monitor first, so a DPI change happens here and not after
                    // the placement below (WM_DPICHANGED would apply Windows' scaled size on top of it)
                    let (dx, dy) = (mi.rcWork.left - mi.rcMonitor.left, mi.rcWork.top - mi.rcMonitor.top);
                    let (w, h) = (fixed.right - fixed.left, fixed.bottom - fixed.top);
                    _ = SetWindowPos(main, None, fixed.left + dx, fixed.top + dy, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                let mut wp = WINDOWPLACEMENT {
                    length: size_of::<WINDOWPLACEMENT>() as u32,
                    showCmd: if max != 0 { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNORMAL.0 as u32 },
                    rcNormalPosition: fixed,
                    ..Default::default()
                };
                if startup {
                    // minimized without taking the focus; restored, it comes back maximized if it was
                    // (quiet_wake maximizes it should Windows ignore the flag with this showCmd)
                    wp.showCmd = SW_SHOWMINNOACTIVE.0 as u32;
                    if max != 0 {
                        wp.flags = WPF_RESTORETOMAXIMIZED;
                    }
                }
                _ = SetWindowPlacement(main, &wp);
                if startup {
                    app(|a| a.was_max = max != 0); // no WM_SIZE says so while minimized; autosave keeps it
                }
            }
            None => {
                let mut r = RECT::default();
                _ = GetWindowRect(main, &mut r);
                let ok = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool();
                let f = fit(r, mi.rcWork);
                if ok && f != r {
                    _ = SetWindowPos(main, None, f.left, f.top, f.right - f.left, f.bottom - f.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                _ = ShowWindow(main, if startup { SW_SHOWMINNOACTIVE } else { SW_SHOWNORMAL });
            }
        }
        relayout();
        if startup {
            quiet_placed(main); // focus comes with the first restore (WM_SIZE → restore_focus)
        } else {
            focus_view(app(|a| a.s.active));
            if fresh && bare {
                _ = PostMessageW(Some(main), WM_TOUR, WPARAM(0), LPARAM(0)); // after the first paint
            }
        }
        let file = app(|a| a.s.file.clone());
        let saved = std::fs::read_to_string(&file).ok();
        app(|a| a.base = Some(startup_base(&a.s, saved.as_deref())));
        refresh_save_button();
        if let Some(path) = unread {
            let missing = std::fs::metadata(&path).is_err_and(|e| not_found(&e));
            let text = session_unread_note(KO.load(Ordering::Relaxed), &path, missing);
            app(|a| a.note = Some((text, Instant::now() + Duration::from_secs(5))));
            _ = MessageBeep(MB_ICONWARNING);
        }
        if exe_in_temp() {
            app(|a| a.ini_nag.queue(IniWarn::Temp)); // shown by the status tick once the tour and notes are gone
        }
        SetTimer(Some(main), TIMER_STATUS, TICK_FAST, None); // a minimized start slows it on its first tick
        watch_folders(main); // folders renamed / removed elsewhere (shell_change)
        watch_bin(main); // the Recycle Bin button's icon follows the bin (bin_change)
        // taken by another app: the menu shows it off, s.hotkey stays on so the next start tries again
        if app(|a| a.s.hotkey) {
            let on = set_hotkey(main, true);
            app(|a| a.hot_on = on);
        }
        if app(|a| a.s.dlgjump) {
            dialog_jump::enable(true);
        }
        repair_default_open(); // folders set to open in Grids, and Grids moved: they follow this exe
        // no taskbar yet at sign-in: s.tray stays on, a few timed retries and TaskbarCreated try again
        tray_init(main);
        if app(|a| a.s.tray) {
            tray_start(main); // a minimized sign-in start leaves the taskbar once the icon is there
        }
        refresh_jump_list(true); // taskbar / Start jump list: sessions, favourites, recent folders
        if let Some(placed) = burst {
            app(|a| a.menu_burst = Some(Burst { at: Instant::now(), first: 0, placed, missed: 0 }));
        }
        start.release(); // panes are up: the next start finds this window and hands it its folders
        register_restart(); // a Windows update reboot opens it again, quietly (--startup)

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !pre_translate(&msg) {
                let up = up_candidate(&msg);
                _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                if let Some((i, g, svh)) = up {
                    let query = || {
                        let view = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok()).map(|h| h.0 as isize);
                        let sel = view_of::<IFolderView2>(i).and_then(|fv| fv.ItemCount(SVGIO_SELECTION).ok()).map(|n| n as u32);
                        (view, sel)
                    };
                    if confirm((g, svh), query, gen) {
                        browse(i, SBSP_PARENT);
                    }
                }
            }
        }
        unregister_restart(); // closed by the user: not to come back after a reboot
        // The window is gone; copies still running finish first. Waiting keeps COM calls and messages
        // moving, in case a shell extension calls back into this apartment.
        PHOTOS_STOP.store(true, Ordering::Relaxed); // smaller photo copies: the one being made is the last
        for job in JOBS.take() {
            if !job.is_finished() {
                let h = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&job));
                _ = CoWaitForMultipleHandles((COWAIT_DISPATCH_CALLS | COWAIT_DISPATCH_WINDOW_MESSAGES).0 as u32, INFINITE, &[h]);
            }
            _ = job.join();
        }
        restart_updated(); // an update was just put in place: the new exe takes over
        // then Windows' own copies (cut + paste, drags between panes) run to the end, as Explorer's do
        wait_shell_ops(shell_refs);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Three separate number spaces: the ⇄ menu (TPM_RETURNCMD, read by open_in_pick), main_proc's WM_COMMAND,
    // and each window's timers. The lists are written by hand: a new constant left out of them is not checked;
    // open_in_pick turning a stray number into Unknown (never a copy / move) holds whatever the lists say.
    #[test]
    fn command_ids_dont_collide() {
        use std::collections::HashSet;
        fn distinct(what: &str, ids: &[usize]) {
            let mut seen = HashSet::new();
            for id in ids {
                assert!(seen.insert(*id), "{what}: {id} used twice");
            }
        }
        // (a) ⇄ menu: every number it puts up reads back as its own action
        let mut menu = vec![];
        for k in 0..MAXP {
            assert_eq!(open_in_pick(ID_OPEN_TO + k), OpenPick::Open(k));
            assert_eq!(open_in_pick(ID_COPY_TO + k), OpenPick::Send { mv: false, k });
            assert_eq!(open_in_pick(ID_MOVE_TO + k), OpenPick::Send { mv: true, k });
            assert_eq!(open_in_pick(ID_SWAP_TO + k), OpenPick::Swap(k));
            assert_eq!(open_in_pick(ID_WM_PANE + k), OpenPick::Shell);
            assert_eq!(open_in_pick(ID_DIFF_TO + k), OpenPick::Diff(k));
            menu.extend([ID_OPEN_TO + k, ID_COPY_TO + k, ID_MOVE_TO + k, ID_SWAP_TO + k, ID_WM_PANE + k, ID_DIFF_TO + k]);
        }
        let shell = [ID_INUSE, ID_WM_PAIR, ID_HASH, ID_GROUP, ID_STAMP, ID_ORG, ID_ORG + 1, ID_ORG + 2, ID_EMPTY, ID_USAGE, ID_LIST, ID_CLIPADD, ID_RESTORE, ID_EXPLORER, ID_PHOTO, ID_PHOTO + 1, ID_PHOTO + 2];
        for p in shell {
            assert_eq!(open_in_pick(p), OpenPick::Shell, "{p}");
        }
        menu.extend(shell);
        for n in 0..SEND_SLOTS {
            assert_eq!(open_in_pick(ID_SEND_COPY + n), OpenPick::Elsewhere);
            assert_eq!(open_in_pick(ID_SEND_MOVE + n), OpenPick::Elsewhere);
            menu.extend([ID_SEND_COPY + n, ID_SEND_MOVE + n]);
        }
        for p in [ID_CHOOSE_COPY, ID_CHOOSE_MOVE] {
            assert_eq!(open_in_pick(p), OpenPick::Elsewhere);
        }
        menu.extend([ID_CHOOSE_COPY, ID_CHOOSE_MOVE]);
        distinct("⇄ menu", &menu);
        assert_eq!(open_in_pick(0), OpenPick::Nothing);
        // just past each range: never an open in, or a copy / move to, a pane
        for p in [17, 100, 117, 200, 217, 317, 441, 600, 617, 1129, 1138, 3116, 5000] {
            assert_eq!(open_in_pick(p), OpenPick::Unknown, "{p}");
        }
        for p in 0..5000 {
            if let OpenPick::Send { k, .. } | OpenPick::Open(k) | OpenPick::Swap(k) | OpenPick::Diff(k) = open_in_pick(p) {
                assert!(k < MAXP && menu.contains(&p), "{p}");
            }
        }
        // (b) main_proc's WM_COMMAND: toolbar and status line children, drive buttons, view menu, selection colours
        let mut main = [ID_TB_LAYOUT, ID_TB_SAVE, ID_TB_VIEW, ID_TB_LOAD, ID_TB_THEME, ID_TB_FAV, ID_TB_EV, ID_TB_UPDATE, ID_STATUS, ID_STATUS_NOTE]
            .map(|id| id as usize)
            .to_vec();
        main.extend((0..26).map(|d| ID_DRIVE as usize + d));
        main.push(ID_BIN as usize);
        main.extend((0..MAX_CLOUD).map(|k| ID_CLOUD as usize + k));
        main.extend(VIEW_TOP.iter().chain(&VIEW_END).map(|v| v.0));
        main.push(ID_LANG);
        main.extend((0..LIGHT.accents.len()).map(|k| ID_ACCENT + k));
        main.extend([ID_ACCENT_PICK, ID_ACCENT_SYS]);
        distinct("main WM_COMMAND", &main);
        // the view menu's copies of toolbar buttons send those buttons' commands, already counted once above
        assert!(VIEW_BUTTONS.iter().all(|v| main.iter().filter(|&&id| id == v.0).count() == 1));
        // (c) timers, per window: SetTimer with a number already running replaces that timer
        distinct("main timers", &[TIMER_STATUS, TIMER_REVEAL, TIMER_FOCUS, dialog_jump::TIMER_DLG_RETRY, dialog_jump::TIMER_DLG_RESTORE, TIMER_SHELL, TIMER_EV_SELECT, TIMER_BIN, TIMER_TREE_SYNC]);
        distinct("pane timers", &[TIMER_FILTER, TIMER_SETTLE, TIMER_SPIN, TIMER_LOAD]);
        // (d) WM_APP messages main_proc handles (not other windows' own: WM_TOUR_END, WM_RENAMED, the switcher's)
        let msgs = [
            WM_NARROW_AGAIN, WM_DROP_OPEN, WM_PEEK_DONE, WM_PANE_ICON, WM_DRIVES, WM_KIDS, WM_THEME_SYNC, WM_PEEK_LOST, WM_PEEK_HOSTED,
            WM_HASH, WM_SHELL_CHANGE, WM_GROUPED, WM_TOUR, WM_TRAY, WM_COPY_DONE, WM_USAGE, WM_FONT_SYNC, WM_BIN_COUNT, WM_BIN_CHANGE,
            WM_STAMPED, WM_SIZE_DONE, WM_DISK_DONE, WM_EDITED, WM_OPEN_ARGS, WM_UPDATE, WM_STAR,
        ];
        distinct("main WM_APP messages", &msgs.map(|m| m as usize));
    }

    // A slow folder's bar: nothing for a quick move, then a run inside the list's width that flows and repeats.
    #[test]
    fn load_bar_flows() {
        let ms = Duration::from_millis;
        for e in [0, 100, 299] {
            assert_eq!(load_bar(400, ms(e), true), None, "{e} ms");
            assert_eq!(load_bar(400, ms(e), false), None, "{e} ms");
        }
        for w in [1, 2, 7, 400, 3000] {
            for e in (300..5000).step_by(17) {
                let (l, r) = load_bar(w, ms(e), true).unwrap();
                assert!(0 <= l && l <= r && r <= w, "{w} px {e} ms: {l}..{r}");
                assert!(r - l <= (w * 3 / 10).max(1), "{w} px {e} ms: {l}..{r}");
            }
        }
        // left to right within a cycle, then again from the left
        let at = |e| load_bar(400, ms(e), true).unwrap();
        assert!(at(500).1 < at(800).1 && at(800).1 < at(1100).1);
        assert!(at(500).0 < at(1100).0);
        for e in [300, 450, 900, 1400] {
            assert_eq!(at(e), at(e + 1200), "{e} ms");
            assert_eq!(at(e), at(e + 3600), "{e} ms");
        }
        // somewhere in each cycle the run is whole on screen
        assert!((300..1500).any(|e| at(e).1 - at(e).0 == 120));
        // animation effects off: still, full width
        for e in [300, 777, 5000] {
            assert_eq!(load_bar(400, ms(e), false), Some((0, 400)));
        }
        // no width, or a move that never ended: nothing
        for w in [0, -5] {
            assert_eq!(load_bar(w, ms(800), true), None);
            assert_eq!(load_bar(w, ms(800), false), None);
        }
        assert_eq!(load_bar(400, Duration::from_secs(60), true), None);
        assert_eq!(load_bar(400, Duration::from_secs(3600), false), None);
    }

    // A second instance's hand-over waits under a modal dialog, a menu or a drag / resize, and opens otherwise.
    #[test]
    fn open_args_wait_for_dialogs() {
        for enabled in [false, true] {
            for menu in [false, true] {
                for busy in [false, true] {
                    assert_eq!(defer_args(enabled, menu, busy), !enabled || menu || busy, "{enabled} {menu} {busy}");
                }
            }
        }
        assert!(!defer_args(true, false, false));
        // each hand-over stays one entry, in arrival order, its arguments as sent
        let mut q = Vec::new();
        queue_args(&mut q, r"C:\a.grids");
        queue_args(&mut q, &format!("{FOLDER_FLAG}\nC:\\x\nD:\\y"));
        assert_eq!(q, [vec![r"C:\a.grids".to_string()], vec![FOLDER_FLAG.to_string(), r"C:\x".into(), r"D:\y".into()]]);
        assert_eq!(q.remove(0), [r"C:\a.grids"]);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn start_lock_name_per_exe() {
        let a = start_lock_name(r"C:\Tools\Grids\grids.exe");
        // case does not matter on Windows paths; the same path always gives the same name
        assert_eq!(a, start_lock_name(r"c:\tools\GRIDS\Grids.EXE"));
        assert_eq!(a, start_lock_name(r"C:\Tools\Grids\grids.exe"));
        assert_ne!(a, start_lock_name(r"D:\Portable\grids.exe"));
        // only the namespace prefix has a '\'
        let rest = a.strip_prefix("Local\\").unwrap();
        assert!(!rest.contains('\\'), "{a}");
        assert!(rest.starts_with("Grids.Start.") && rest.len() == "Grids.Start.".len() + 16, "{a}");
    }

    #[test]
    fn start_turn_got_or_not() {
        assert_eq!(start_turn(WAIT_OBJECT_0), StartTurn::Ours);
        assert_eq!(start_turn(WAIT_ABANDONED), StartTurn::Ours); // the last owner died holding it
        assert_eq!(start_turn(WAIT_TIMEOUT), StartTurn::Busy); // told and quits
        assert_eq!(start_turn(WAIT_FAILED), StartTurn::Failed); // starts unlocked, as before
    }

    /// The error mode is process-wide, so the check runs in a child copy of this test binary
    /// that runs only this test; the shared test process keeps its own mode untouched.
    #[test]
    fn critical_error_box_off() {
        const CHILD: &str = "GRIDS_ERRMODE_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let before = unsafe { GetErrorMode() };
            quiet_critical_errors();
            let after = unsafe { GetErrorMode() };
            // the "No Disk" box is off; every other bit (the GP-fault box included) is as it was
            assert_eq!(after, before | SEM_FAILCRITICALERRORS.0);
            quiet_critical_errors();
            assert_eq!(unsafe { GetErrorMode() }, after);
            return;
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::critical_error_box_off", "--test-threads=1", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "child failed:\n{text}\n{}", String::from_utf8_lossy(&out.stderr));
        assert!(text.contains("1 passed"), "child did not run the test:\n{text}");
    }

    #[test]
    fn peek_speech_reads_name_then_facts() {
        let meta: Vec<String> = ["PDF document", "12 pages", "2.3 MB", "2026-11-20"].map(String::from).to_vec();
        assert_eq!(peek_speech("report.pdf", &meta, false, false), "report.pdf, Preview, PDF document, 12 pages, 2.3 MB, 2026-11-20");
        let ko: Vec<String> = ["PDF 문서", "12쪽", "2.3 MB"].map(String::from).to_vec();
        assert_eq!(peek_speech("보고서.pdf", &ko, false, true), "보고서.pdf, 미리보기, PDF 문서, 12쪽, 2.3 MB");
        // before the facts come, or with none; nothing at all to show
        assert_eq!(peek_speech("notes.txt", &[], false, false), "notes.txt, Preview");
        assert_eq!(peek_speech("", &[], false, true), "미리보기");
        // an online-only card: the facts already say so, the tail says how to fetch it
        let cloud: Vec<String> = ["Online-only".into(), "4 MB".into()].to_vec();
        assert_eq!(peek_speech("a.mp4", &cloud, true, false), "a.mp4, Preview, Online-only, 4 MB, Enter: download");
        assert_eq!(peek_speech("a.mp4", &[], true, true), "a.mp4, 미리보기, Enter로 내려받기");
        // a comma in the name stays as it is
        assert_eq!(peek_speech("Smith, J. - CV.docx", &[], false, false), "Smith, J. - CV.docx, Preview");
    }

    #[test]
    fn fav_speech_names() {
        // the whole list: count and keys, or the empty list's note exactly as drawn
        assert_eq!(fav_list_speech(5, false), "Favourites, 5 folders · ↑/↓ to pick, Enter opens, 1 … 9 opens that one, Del removes");
        assert_eq!(fav_list_speech(1, false), "Favourites, 1 folder · ↑/↓ to pick, Enter opens, 1 … 9 opens that one, Del removes");
        assert_eq!(fav_list_speech(5, true), "즐겨찾기, 5개 · ↑/↓로 고르기, Enter 열기, 1~9 바로 열기, Del 빼기");
        assert_eq!((fav_list_speech(0, false).as_str(), fav_list_speech(0, true).as_str()), FAV_EMPTY);
        // one row: folder name, whole path, place in the list; a shell place by its name, not "::{GUID}"
        let docs = "C:\\Users\\me\\Documents";
        assert_eq!(fav_speech(docs, 1, 5, false), "Documents, C:\\Users\\me\\Documents, 2/5");
        assert_eq!(fav_speech(docs, 1, 5, true), "Documents, C:\\Users\\me\\Documents, 2/5");
        assert_eq!(fav_speech("D:\\", 0, 1, false), "D:, D:\\, 1/1");
        let bin = "::{645FF040-5081-101B-9F08-00AA002F954E}";
        assert_eq!((fav_speech(bin, 0, 1, false), fav_speech(bin, 0, 1, true)), ("Recycle Bin, 1/1".into(), "휴지통, 1/1".into()));
        let pc = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
        assert_eq!(fav_speech(pc, 0, 3, false), format!("{}, 1/3", short_name(pc)));
    }

    #[test]
    fn load_speech_names() {
        // one card: name, panes, when, place in the list, and the session you're in
        assert_eq!(load_speech("Work", 6, "5 min ago", 1, 4, false, false), "Work, 6 panes, 5 min ago, 2/4");
        assert_eq!(load_speech("업무", 6, "5분 전", 1, 4, true, true), "업무, 패널 6개, 5분 전, 2/4, 지금 세션");
        assert_eq!(load_speech("Solo", 1, "just now", 0, 1, true, false), "Solo, 1 pane, just now, 1/1, current session");
        assert_eq!(load_speech("Last", 16, "2 d ago", 8, LOAD_MAX, false, false), "Last, 16 panes, 2 d ago, 9/9");
        // English and Korean carry the same numbers, in the same order
        let nums = |s: String| s.split(|c: char| !c.is_ascii_digit()).filter(|t| !t.is_empty()).map(String::from).collect::<Vec<_>>();
        for (p, k, n, cur) in [(1, 0, 1, false), (6, 1, 4, true), (16, 8, 9, false)] {
            assert_eq!(nums(load_speech("a", p, "", k, n, cur, false)), nums(load_speech("a", p, "", k, n, cur, true)));
            assert_eq!(nums(load_popup_speech(n, None, true, false)), nums(load_popup_speech(n, None, true, true)));
        }
        // the whole popup: the count, then the pick on opening; the pick alone after that
        assert_eq!(load_popup_speech(3, None, true, false), "Sessions, 3 saved");
        assert_eq!(load_popup_speech(1, None, false, false), "Sessions, 1 saved");
        assert_eq!(load_popup_speech(3, Some("업무, 패널 6개, 5분 전, 2/3"), true, true), "세션, 3개 · 업무, 패널 6개, 5분 전, 2/3");
        assert_eq!(load_popup_speech(3, Some("Work, 6 panes, 5 min ago, 2/3"), false, false), "Work, 6 panes, 5 min ago, 2/3");
        // the two links read as drawn
        assert_eq!(load_popup_speech(2, Some(LOAD_FOLDER_LABEL.0), false, false), "Open sessions folder");
        assert_eq!(load_popup_speech(2, Some(LOAD_SAVE_LABEL.1), true, true), "세션, 2개 · + 새 이름으로 저장");
        // no cards: the empty note as drawn and the way on, never said twice
        assert_eq!(load_popup_speech(0, None, true, false), "No saved sessions · + Save as new");
        assert_eq!(load_popup_speech(0, Some(LOAD_SAVE_LABEL.1), true, true), "저장된 세션이 없습니다 · + 새 이름으로 저장");
        assert_eq!(load_popup_speech(0, Some(LOAD_FOLDER_LABEL.0), false, false), "No saved sessions · Open sessions folder");
    }

    #[test]
    fn fav_reorder() {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // gaps are 0..=n; a row's own two gaps leave it in place
        let mut f = v(&["a", "b", "c"]);
        assert_eq!((move_fav(&mut f, 0, 3), f.clone()), (Some(2), v(&["b", "c", "a"])));
        assert_eq!((move_fav(&mut f, 2, 0), f.clone()), (Some(0), v(&["a", "b", "c"])));
        for (from, slot) in [(1, 1), (1, 2)] {
            assert_eq!((move_fav(&mut f, from, slot), f.clone()), (Some(1), v(&["a", "b", "c"])));
        }
        assert_eq!((move_fav(&mut f, 3, 0), move_fav(&mut f, 0, 4), move_fav(&mut vec![], 0, 0)), (None, None, None));
        assert_eq!(f, v(&["a", "b", "c"]));

        // drop gap: rows whose middle is above y
        let g = fav_geo(3, 0, i32::MAX, 96);
        let r = |k: usize| g.rows[k].0;
        assert_eq!((fav_slot(&g, r(0).top - 1), fav_slot(&g, r(2).bottom + 5), fav_slot(&fav_geo(0, 0, i32::MAX, 96), 50)), (0, 3, 0));
        for k in 0..3 {
            assert_eq!((fav_slot(&g, r(k).top + 2), fav_slot(&g, r(k).bottom - 2)), (k, k + 1));
        }

        // release: outside cancels, a drag drops at the release y, a click must start and end on the same spot
        let p = |from, on_x, live, slot| Some(FavPress { from, on_x, x0: 0, y0: 0, live, slot });
        assert_eq!(fav_release(p(0, false, true, Some(2)), false, None, 0), Release::None);
        let y = r(2).bottom - 2;
        assert_eq!(fav_release(p(0, false, true, Some(0)), true, fav_hit(&g, 20, y), fav_slot(&g, y)), Release::Move(0, 3));
        assert_eq!(fav_release(None, true, Some((1, false)), 1), Release::None);
        assert_eq!(fav_release(p(1, true, false, None), true, Some((1, true)), 1), Release::Remove(1));
        assert_eq!(fav_release(p(1, false, false, None), true, Some((1, false)), 1), Release::Open(1));
        assert_eq!(fav_release(p(1, false, false, None), true, Some((2, false)), 2), Release::None);
        assert_eq!(fav_release(p(1, false, false, None), true, Some((1, true)), 1), Release::None);

        // a list taller than the room: only whole rows that fit, a thumb in the right margin, every index in the whole list
        let g = fav_geo(30, 10, 300, 96);
        let r = |k: usize| g.rows[k].0;
        assert!(g.h <= 300);
        assert_eq!((g.rows.len(), g.first, g.n), (7, 10, 30));
        let bar = g.bar.unwrap();
        assert!(bar.left >= r(0).right && bar.right < g.w && bar.top >= r(0).top && bar.bottom <= r(6).bottom);
        assert_eq!((fav_hit(&g, 20, r(0).top + 1), fav_hit(&g, r(0).right - 1, r(0).top + 1)), (Some((10, false)), Some((10, true))));
        assert_eq!((fav_slot(&g, r(0).top - 1), fav_slot(&g, r(0).bottom - 2), fav_slot(&g, r(6).bottom + 5)), (10, 11, 17));
        let y = r(6).bottom - 2;
        assert_eq!(fav_release(p(12, false, true, Some(0)), true, fav_hit(&g, 20, y), fav_slot(&g, y)), Release::Move(12, 17));
        assert_eq!(fav_release(p(16, false, false, None), true, fav_hit(&g, 20, y), fav_slot(&g, y)), Release::Open(16));
        // the thumb runs from the top to the bottom; a top past the end leaves no row empty; always one row
        let (t0, tn) = (fav_geo(30, 0, 300, 96), fav_geo(30, 99, 300, 96));
        assert_eq!((t0.bar.unwrap().top, tn.first, tn.bar.unwrap().bottom), (t0.rows[0].0.top, 23, tn.rows[6].0.bottom));
        assert_eq!((fav_geo(30, 0, 0, 96).rows.len(), fav_geo(30, 0, 0, 96).h), (1, fav_geo(1, 0, i32::MAX, 96).h));
        assert_eq!(fav_geo(29, 23, 300, 96).first, 22); // Del of the last row pulls the view back

        // scroll the least to show k
        assert_eq!((fav_scroll_to(0, 6, 7, 30), fav_scroll_to(0, 7, 7, 30), fav_scroll_to(10, 3, 7, 30)), (0, 1, 3));
        assert_eq!((fav_scroll_to(10, 29, 7, 30), fav_scroll_to(23, 0, 7, 30), fav_scroll_to(5, 2, 7, 4)), (23, 0, 0));
        assert_eq!(fav_scroll_to(23, 28, 7, 29), 22);

        // below ★ unless it doesn't fit there, below is under three rows and above is roomier
        assert_eq!((fav_room(200, 500, 900, 132), fav_room(1000, 400, 900, 132)), ((500, false), (400, false)));
        assert_eq!((fav_room(1000, 100, 900, 132), fav_room(1000, 100, 50, 132)), ((900, true), (100, false)));

        // only plain Alt+↑/↓
        assert_eq!((fav_step(VK_UP, false, true, false), fav_step(VK_DOWN, false, true, false)), (Some(-1), Some(1)));
        for (c, a, s) in [(true, true, false), (false, true, true), (true, true, true), (false, false, false)] {
            assert_eq!((fav_step(VK_UP, c, a, s), fav_step(VK_DOWN, c, a, s)), (None, None));
        }
        assert_eq!((fav_step(VK_LEFT, false, true, false), fav_step(VK_F4, false, true, false)), (None, None));

        // plain 1 … 9 (number pad too) open that row; past the list, 0, other keys or any modifier: nothing
        assert_eq!((fav_digit(VK_1, 5, false, false, false), fav_digit(VK_9, 9, false, false, false)), (Some(0), Some(8)));
        assert_eq!((fav_digit(VK_NUMPAD1, 5, false, false, false), fav_digit(VK_NUMPAD5, 5, false, false, false)), (Some(0), Some(4)));
        assert_eq!((fav_digit(VK_6, 5, false, false, false), fav_digit(VK_NUMPAD9, 3, false, false, false)), (None, None));
        assert_eq!((fav_digit(VK_1, 0, false, false, false), fav_digit(VK_0, 30, false, false, false), fav_digit(VK_NUMPAD0, 30, false, false, false)), (None, None, None));
        assert_eq!((fav_digit(VK_B, 30, false, false, false), fav_digit(VK_HOME, 30, false, false, false)), (None, None));
        for (c, a, s) in [(true, false, false), (false, true, false), (false, false, true), (true, true, false)] {
            assert_eq!((fav_digit(VK_3, 30, c, a, s), fav_digit(VK_NUMPAD3, 30, c, a, s)), (None, None));
        }
    }

    #[test]
    fn gather() {
        let s = |v: &[&str]| v.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        let o = |v: &[(usize, &str)]| v.iter().map(|&(i, p)| (i, p.to_string())).collect::<Vec<_>>();
        let plan = |found: &[&str], open: &[(usize, &str)], active, n| gather_plan(&s(found), &o(open), active, n);
        // A stays in pane 0; E takes the next pane, and a second run finds nothing to do
        assert_eq!(plan(&["A", "E"], &[(0, "A"), (1, "B"), (2, "C"), (3, "D")], 0, 4), (o(&[(1, "E")]), 0));
        assert_eq!(plan(&["A", "E"], &[(0, "A"), (1, "E"), (2, "C"), (3, "D")], 0, 4), (vec![], 0));
        // case and a trailing \ don't matter
        assert_eq!(plan(&["C:\\A", "c:\\a\\", "D:\\X"], &[(0, "C:\\A"), (1, "B")], 0, 2), (o(&[(1, "D:\\X")]), 0));
        assert_eq!(plan(&["C:\\A", "c:\\a\\"], &[(0, "B"), (1, "B")], 0, 2), (o(&[(0, "C:\\A")]), 0));
        // the same folder in two panes keeps one of them
        assert_eq!(plan(&["A", "E"], &[(0, "A"), (1, "A")], 0, 2), (o(&[(1, "E")]), 0));
        // from the active pane on, wrapping round
        assert_eq!(plan(&["E", "F", "G"], &[(0, "A"), (1, "B"), (2, "C"), (3, "D")], 3, 4), (o(&[(3, "E"), (0, "F"), (1, "G")]), 0));
        // too few panes
        assert_eq!(plan(&["A", "E", "F", "G"], &[(0, "A"), (1, "B")], 0, 2), (o(&[(1, "E")]), 2));
        assert_eq!(plan(&["A", "B", "C"], &[(0, "A"), (1, "B")], 0, 2), (vec![], 1));
        // slots without a live pane are neither kept nor filled
        assert_eq!(plan(&["A", "E"], &[(0, "B"), (2, "C")], 0, 3), (o(&[(0, "A"), (2, "E")]), 0));
        // a folder named like a session file is just a folder here
        assert_eq!(plan(&["D:\\Backup.grids"], &[(0, "A")], 0, 1), (o(&[(0, "D:\\Backup.grids")]), 0));
        assert_eq!(plan(&[], &[(0, "A")], 0, 1), (vec![], 0));
    }

    #[test]
    fn neighbours() {
        let r = |left, top, right, bottom| RECT { left, top, right, bottom };
        // 2×2 with an 8px gap: slots run down each column
        let grid = [(0, r(0, 0, 100, 100)), (1, r(0, 108, 100, 208)), (2, r(108, 0, 208, 100)), (3, r(108, 108, 208, 208))];
        let n = |cells: &[(usize, RECT)], cur, dx, dy| neighbour(cells, cur, dx, dy);
        assert_eq!([n(&grid, 3, 0, -1), n(&grid, 3, -1, 0), n(&grid, 0, 1, 0), n(&grid, 0, 0, 1)], [Some(2), Some(1), Some(2), Some(1)]);
        // edges
        assert_eq!([n(&grid, 0, -1, 0), n(&grid, 0, 0, -1), n(&grid, 3, 1, 0), n(&grid, 3, 0, 1)], [None; 4]);
        // [1,3]: one tall cell on the left, three on the right
        let tall = [(0, r(0, 0, 100, 316)), (1, r(108, 0, 208, 100)), (2, r(108, 108, 208, 208)), (3, r(108, 216, 208, 316))];
        assert_eq!([n(&tall, 0, 1, 0), n(&tall, 3, -1, 0), n(&tall, 2, 0, -1), n(&tall, 2, 0, 1)], [Some(2), Some(0), Some(1), Some(3)]);
        assert_eq!(n(&tall, 1, 0, -1), None); // stays in its column
        // a pane that failed to open: neither it nor the diagonal one is picked
        let gone = [grid[0], grid[2], grid[3]];
        assert_eq!([n(&gone, 0, 0, 1), n(&gone, 3, -1, 0), n(&gone, 2, 0, 1)], [None, None, Some(3)]);
        assert_eq!(n(&grid, 7, 1, 0), None);
    }

    #[test]
    fn drives() {
        assert_eq!(mask_letters(0b1100), b"CD");
        assert_eq!(mask_letters(1 | 1 << 25), b"AZ");
        assert!(mask_letters(0).is_empty());
        assert_eq!([drive_of(r"c:\x"), drive_of(r"D:\"), drive_of("e:")], [Some(b'C'), Some(b'D'), Some(b'E')]);
        for p in [r"\\server\share", THIS_PC, "search-ms:query=x", ""] {
            assert_eq!(drive_of(p), None, "{p}");
        }
        // GetDriveTypeW kinds: removable, fixed, remote, CD-ROM, RAM disk; unknown / no root / out of range = fixed
        let icons = [2, 3, 4, 5, 6].map(drive_icon_id);
        assert_eq!(icons, [SIID_DRIVEREMOVE, SIID_DRIVEFIXED, SIID_DRIVENET, SIID_DRIVECD, SIID_DRIVERAM]);
        for k in [0, 1, 7, u32::MAX] {
            assert_eq!(drive_icon_id(k), SIID_DRIVEFIXED, "{k}");
            assert_eq!(drive_icon(k), -1, "{k}"); // never asked: no icon, the letter alone
        }
    }

    // Cloud folder buttons: only pinned ones on a drive path, not a drive root, one per folder, by name, at most 8.
    #[test]
    fn cloud_buttons() {
        let reg = |name: &str, pinned: bool, path: &str| CloudReg { name: name.into(), pinned, path: path.into(), icon: String::new() };
        let got = cloud_list(vec![
            reg("OneDrive - Personal", true, r"C:\Users\a\OneDrive\"),
            reg("Dropbox", true, r"C:\Users\a\Dropbox"),
            reg("Hidden", false, r"C:\Users\a\Hidden"),          // not pinned to the navigation pane
            reg("Google Drive", true, r"G:\"),                   // a drive root: G: has its own button
            reg("Empty", true, ""),                              // no folder registered
            reg("Share", true, r"\\server\share"),               // not a drive path
            reg("Shell", true, "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"),
            reg("dropbox again", true, r"c:\users\A\DROPBOX"),   // the same folder in another case
            reg("", true, r"D:\Box"),                            // no name: its folder's
            reg("@shell32.dll,-1", true, r"D:\iCloud Drive"),    // a resource string: its folder's
        ]);
        let names: Vec<&str> = got.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Box", "Dropbox", "iCloud Drive", "OneDrive - Personal"]);
        assert_eq!(got[3].path, r"C:\Users\a\OneDrive"); // trailing backslash off
        let many: Vec<CloudReg> = (0..12).rev().map(|k| reg(&format!("c{k:02}"), true, &format!(r"C:\c{k}"))).collect();
        let got = cloud_list(many);
        assert_eq!(got.len(), MAX_CLOUD);
        assert_eq!((got[0].name.as_str(), got[7].name.as_str()), ("c00", "c07"));
        assert!(cloud_list(Vec::new()).is_empty());
        // the underline: whole names only, any case, the deepest of nested ones
        let roots = [r"C:\Users\a\OneDrive", r"C:\Users\a\OneDrive - Work", r"C:\Users\a\OneDrive\Sub"];
        let at = |p: &str| cloud_mark(p, roots);
        assert_eq!(at(r"C:\Users\a\OneDrive"), Some(0));
        assert_eq!(at(r"c:\users\A\onedrive\Docs"), Some(0));
        assert_eq!(at(r"C:\Users\a\OneDrive - Work\x"), Some(1));
        assert_eq!(at(r"C:\Users\a\OneDrive2"), None);
        assert_eq!(at(r"C:\Users\a\OneDrive\Sub\deep"), Some(2));
        assert_eq!(at(r"C:\Users\a"), None);
        assert_eq!(at(THIS_PC), None);
        assert_eq!(cloud_mark(r"C:\x", [""]), None);
    }

    // Every drive check on one path, side by side, all built on drive_letter: what may follow "X:" is each one's
    // own (drive_root: "\" alone; drive_of: anything; drive_bit: nothing or "\"; vol_key: anything; drive_path,
    // disk_folder, clip_paths: "\…"). is_fs keeps its own looser test ("1:\" and "\\?\" pass, "C:" doesn't).
    #[test]
    fn drive_rules_one_table() {
        const C: Option<u32> = Some(1 << 2);
        type Row<'a> = (Option<(u8, &'a str)>, Option<u8>, Option<u8>, Option<u32>, Option<&'a str>, bool, bool, bool, Vec<&'a str>);
        #[rustfmt::skip]
        let table: [(&str, Row); 16] = [
            //  input                letter              root       of         bit   vol_key           path   disk   is_fs  clip_paths
            (r"C:\",          (Some((b'C', r"\")),      Some(b'C'), Some(b'C'), C,    Some("c:"),          true,  true,  true,  vec![r"C:\"])),
            (r"c:\Users",     (Some((b'C', r"\Users")), None,       Some(b'C'), C,    Some("c:"),          true,  true,  true,  vec![r"c:\Users"])),
            ("C:",            (Some((b'C', "")),        None,       Some(b'C'), C,    Some("c:"),          false, false, false, vec![r"C:\"])),
            ("c:",            (Some((b'C', "")),        None,       Some(b'C'), C,    Some("c:"),          false, false, false, vec![r"c:\"])),
            ("C:x",           (Some((b'C', "x")),       None,       Some(b'C'), None, Some("c:"),          false, false, false, vec![])),
            ("C:/x",          (Some((b'C', "/x")),      None,       Some(b'C'), None, Some("c:"),          false, false, false, vec![r"C:\x"])),
            (r"\\srv\share",  (None,                    None,       None,       None, Some(r"\\srv\share"), false, true,  true,  vec![r"\\srv\share"])),
            (r"\\srv",        (None,                    None,       None,       None, Some(r"\\srv\"),     false, true,  true,  vec![r"\\srv"])),
            (r"\\?\C:\x",     (None,                    None,       None,       None, Some(r"\\?\c:"),     false, false, true,  vec![])),
            (THIS_PC,         (None,                    None,       None,       None, None,                false, false, false, vec![])),
            ("search-ms:q",   (None,                    None,       None,       None, None,                false, false, false, vec![])),
            ("shell:Downloads", (None,                  None,       None,       None, None,                false, false, false, vec![])),
            (r"1:\",          (None,                    None,       None,       None, None,                false, false, true,  vec![])),
            (r"가:\",         (None,                    None,       None,       None, None,                false, false, false, vec![])),
            ("",              (None,                    None,       None,       None, None,                false, false, false, vec![])),
            ("C",             (None,                    None,       None,       None, None,                false, false, false, vec!["C"])),
        ];
        for (p, want) in table {
            let clip = clip_paths(p, |_| None);
            let vol = vol_key(p);
            let got: Row = (
                drive_letter(p),
                drive_root(p),
                drive_of(p),
                drive_bit(p),
                vol.as_deref(),
                drive_path(p),
                disk_folder(p),
                is_fs(p),
                clip.iter().map(String::as_str).collect(),
            );
            assert_eq!(got, want, "{p:?}");
        }
    }

    #[test]
    fn address_crumbs() {
        let c = |p: &str| crumbs(p).into_iter().map(|(n, g)| format!("{n}={g}")).collect::<Vec<_>>();
        assert_eq!(c("C:\\"), [r"C:=C:\"]);
        assert_eq!(c(r"C:\Users\Public\"), [r"C:=C:\", r"Users=C:\Users", r"Public=C:\Users\Public"]);
        assert_eq!(c(r"\\srv\share\a"), [r"\\srv\share=\\srv\share", r"a=\\srv\share\a"]);
        assert!(crumbs(THIS_PC).is_empty());
        let p = r"D:\x\y y\z";
        assert!(crumbs(p).iter().all(|(_, g)| p.starts_with(g.as_str())));
    }

    #[test]
    fn crumb_fits() {
        let f = |w: &[i32], avail| crumb_fit(w, avail, 4, 20, 16);
        assert_eq!(f(&[30, 40, 50], 500), (0, false, 50));
        assert_eq!(f(&[14, 15, 15], 52), (0, false, 15)); // exactly fits: nothing folded
        assert_eq!(f(&[14, 15, 15], 51), (2, true, 15));
        assert_eq!(f(&[10, 200], 100), (1, false, 100)); // current folder longer than the field
        assert_eq!(f(&[30, 10, 50], 70), (2, false, 50)); // no room left for '…'
        assert_eq!(f(&[30, 10, 50], 78), (2, true, 50));
        assert_eq!(f(&[30, 40], 15), (2, false, 0));
        assert_eq!(f(&[30, 40], 0), (2, false, 0));
        assert_eq!(f(&[], 100), (0, false, 0));
    }

    #[test]
    fn crumb_tail_room() {
        let f = |avail, fs| crumb_fit_tail(&[30, 40], avail, 4, 20, 16, fs);
        assert_eq!(f(16, true), (1, false, 16, false)); // too narrow for the '›': current folder still shown
        assert_eq!(f(19, true), (1, false, 19, false));
        assert_eq!(f(20, true), (1, false, 16, true));
        assert_eq!(f(500, true), (0, false, 40, true));
        assert!(!f(500, false).3); // shell place: never
        assert!(!f(16, false).3);
    }

    #[test]
    fn kid_name_rules() {
        use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
        let (d, h, sy) = (FILE_ATTRIBUTE_DIRECTORY.0, FILE_ATTRIBUTE_HIDDEN.0, FILE_ATTRIBUTE_SYSTEM.0);
        let all = || [("10", d), ("2", d), ("file.txt", FILE_ATTRIBUTE_ARCHIVE.0), ("Secret", d | h), ("Config.Msi", d | h | sy), ("a", d | sy)];
        let names = |show| kid_names(all().into_iter().map(|(n, a)| (n.to_string(), a)), show, KIDS_MAX);
        assert_eq!(names(false), (vec!["2".to_string(), "10".into(), "a".into()], false));
        assert_eq!(names(true), (vec!["2".to_string(), "10".into(), "a".into(), "Secret".into()], false));
        // the '›' menu keeps KIDS_MAX; Alt+PgDn / Alt+PgUp asks for all
        let (v, cut) = kid_names((0..501).map(|k| (format!("f{k}"), d)), false, KIDS_MAX);
        assert_eq!((v.len(), cut, v[0].as_str(), v[499].as_str()), (500, true, "f0", "f499"));
        assert_eq!(kid_names((0..500).map(|k| (format!("f{k}"), d)), false, KIDS_MAX).1, false);
        let (v, cut) = kid_names((0..1200).map(|k| (format!("f{k}"), d)), false, usize::MAX);
        assert_eq!((v.len(), cut, v[733].as_str(), v[1199].as_str()), (1200, false, "f733", "f1199"));
    }

    #[test]
    fn kids_late_results_dropped() {
        let mut j = KidsJob { id: 0, hwnd: 0, dir: String::new(), hidden: false, cap: KIDS_MAX, result: None };
        let a = j.ask(1, r"C:\Users", false, KIDS_MAX);
        assert!(j.publish(a, Ok((vec!["A".into()], false))));
        let b = j.ask(1, r"C:\", false, usize::MAX);
        assert_eq!(j.cap, usize::MAX);
        assert!(!j.publish(a, Ok((vec!["A".into()], false))));
        assert!(j.take(a).is_none() && j.take(b).is_none());
        assert!(j.publish(b, Err(())));
        assert!(j.take(b).is_some() && j.take(b).is_none());
        // A's late WM_KIDS while B waits: not B's owner, so it leaves B alone
        let w = HWND(1 as _);
        let ask = KidsAsk { id: b, hwnd: w, i: 0, path: r"C:\Users\Public".into(), gen: 0, dir: r"C:\".into(), next: None, step: 0 };
        assert!(!ask.owns(a, w) && ask.owns(b, w) && !ask.owns(b, HWND(2 as _)));
        j.cancel();
        assert!(!j.publish(b, Err(())) && j.dir.is_empty());
    }

    #[test]
    fn sibling_steps() {
        let v = |s: &[&str]| s.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        let names = v(&["1", "2", "10", "b"]);
        // Explorer's order ('2' before '10'), any case
        assert_eq!(pick_sibling(&names, "2", 1, false), Sibling::Go(2));
        assert_eq!(pick_sibling(&names, "10", -1, false), Sibling::Go(1));
        assert_eq!(pick_sibling(&names, "B", -1, false), Sibling::Go(2));
        // first / last
        assert_eq!(pick_sibling(&names, "1", -1, false), Sibling::First);
        assert_eq!(pick_sibling(&names, "b", 1, false), Sibling::Last);
        // a hidden current folder (not listed) is placed among the rest
        assert_eq!(pick_sibling(&names, "3", 1, false), Sibling::Go(2));
        assert_eq!(pick_sibling(&names, "3", -1, false), Sibling::Go(1));
        assert_eq!(pick_sibling(&names, "0", -1, false), Sibling::First);
        // nothing else there
        assert_eq!(pick_sibling(&[], "a", 1, false), Sibling::Last);
        assert_eq!(pick_sibling(&[], "a", -1, false), Sibling::First);
        // a list cut short: past its end is unknown, not the last
        assert_eq!(pick_sibling(&names, "b", 1, true), Sibling::Unknown);
        assert_eq!(pick_sibling(&names, "c", -1, true), Sibling::Unknown);
        assert_eq!(pick_sibling(&names, "b", -1, true), Sibling::Go(2));
        // more folders than the '›' menu shows: Alt+PgDn / Alt+PgUp lists them all, so every one steps
        let many = (0..1200).map(|k| format!("f{k}")).collect::<Vec<_>>();
        assert_eq!(pick_sibling(&many, "f733", 1, false), Sibling::Go(734));
        assert_eq!(pick_sibling(&many, "f733", -1, false), Sibling::Go(732));
        assert_eq!(pick_sibling(&many, "f1199", 1, false), Sibling::Last);
        assert_eq!(sibling_of(r"C:\Users\Public"), Some((r"C:\Users".into(), "Public".into())));
        assert_eq!(sibling_of(r"C:\Users"), Some((r"C:\".into(), "Users".into())));
        assert_eq!(sibling_of(r"C:\"), None);
        assert_eq!(sibling_of("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"), None);
    }

    #[test]
    fn kids_anchor_follows_layout() {
        let r = |l| RECT { left: l, top: 0, right: l + 10, bottom: 20 };
        let kids = |d: &str| Crumb::Kids(d.into(), None);
        let wide = vec![(r(0), "C:".into(), Crumb::Go(r"C:\".into())), (r(10), "›".into(), kids(r"C:\")), (r(20), "Users".into(), Crumb::Here)];
        assert_eq!(kids_anchor(&wide, r"C:\"), Some(r(10)));
        let folded = vec![(r(0), "…".into(), Crumb::More(Vec::new())), (r(30), "›".into(), kids(r"C:\Users")), (r(40), "Users".into(), Crumb::Here)];
        assert_eq!(kids_anchor(&folded, r"C:\"), None);
    }

    #[test]
    fn focus_rings() {
        // list → address → filter → tree → toolbar → list; Shift+F6 the other way
        let walk = |back, tree| (0..5).map(|p| focus_ring(p, back, tree)).collect::<Vec<_>>();
        assert_eq!(walk(false, true), [1, 2, 3, 4, 0]);
        assert_eq!(walk(true, true), [4, 0, 1, 2, 3]);
        // no tree: skipped both ways (the tree's place, its window gone, counts as the filter)
        assert_eq!(walk(false, false), [1, 2, 4, 4, 0]);
        assert_eq!(walk(true, false), [4, 0, 1, 1, 2]);
        // the toolbar's ←/→ wrap at the ends, Home / End go to the ends
        assert_eq!([0, 1, 4].map(|c| toolbar_step(5, c, VK_RIGHT)), [1, 2, 0]);
        assert_eq!([0, 1, 4].map(|c| toolbar_step(5, c, VK_LEFT)), [4, 0, 3]);
        assert_eq!((toolbar_step(5, 3, VK_HOME), toolbar_step(5, 1, VK_END)), (0, 4));
        assert_eq!(toolbar_step(1, 0, VK_RIGHT), 0);
    }

    #[test]
    fn hidden_tree_keeps_its_width() {
        // hidden, narrow window, column drag: the clamped 536 must not replace 600
        assert_eq!(kept_side(false, 600, 536), 600);
        assert_eq!(kept_side(true, 600, 536), 536); // shown: as before
        let s = Session::parse(&Session { tree: false, side: 600, ..Session::default() }.to_ini());
        assert_eq!((s.tree, s.side), (false, 600));
        assert!(Session::parse("side=300\ndblup=1\n").tree); // older ini: shown
        // a folder name with a line break never turns into keys of its own
        let mut evil = Session::default();
        evil.paths[0] = "C:\\x\nsession=C:\\victim.txt".into();
        evil.recent = vec!["D:\\ok".into(), "\\\\h\\s\np1=\\\\evil\\x".into()];
        let ini = evil.to_ini();
        let back = Session::parse(&ini);
        assert!(back.file.is_empty() && back.paths[0].is_empty() && back.paths[1].is_empty(), "{ini}");
        assert_eq!(back.recent, ["D:\\ok"]);
        // grid starting at pad: the old sidebar splitter is no longer a splitter
        let g = Geo { w: 1000, h: 600, top: 34, bot: 576, pad: 8, gap: 8, side: 240, tree: false, gx: 8, xs: vec![(8, 992)], ys: vec![vec![(34, 576)]], zoom: false };
        assert!((240..264).all(|x| !matches!(hit(&g, x, 100), Some(Hit::Side))));
    }

    #[test]
    fn follow_theme_setting() {
        assert!(!Session::parse("side=300\ndark=1\n").follow); // older ini: off
        let s = Session::parse(&Session { follow: true, ..Session::default() }.to_ini());
        assert!(s.follow);
        assert_eq!(theme_target(false, Some(true), false), None); // not following
        assert_eq!(theme_target(true, None, false), None); // Windows doesn't say
        assert_eq!(theme_target(true, Some(true), true), None); // already there
        assert_eq!(theme_target(true, Some(true), false), Some(true));
        assert_eq!(theme_target(true, Some(false), true), Some(false));
    }

    #[test]
    fn hotkey_setting() {
        assert!(Session::parse("side=300\ndblup=1\n").hotkey); // older ini: on
        assert!(!Session::parse("hotkey=0\n").hotkey);
        for on in [true, false] {
            assert_eq!(Session::parse(&Session { hotkey: on, ..Session::default() }.to_ini()).hotkey, on);
        }
        assert!(hotkey_flip(false, |w| w)); // failed at start, the menu click retries and succeeds
        assert!(!hotkey_flip(false, |_| false)); // retry fails again
        assert!(!hotkey_flip(true, |w| {
            assert!(!w);
            false
        })); // turned off
        // app-wide: no unsaved-layout dot, a loaded session doesn't change it
        let base = Session::default();
        assert!(!layout_differs(&Session { hotkey: false, ..base.clone() }, &base));
        let mut cur = Session::default();
        cur.take_layout(&Session::parse("hotkey=0\n"));
        assert!(cur.hotkey);
    }

    #[test]
    fn bring_here_moves_only_when_elsewhere() {
        let d = GUID::from_u128(0x1111_2222_3333_4444_5555_6666_7777_8888);
        let pinned = GUID::from_u128(0xbb64_d5b7_4de3_4ab2_a87c_db76_01ae_a7dc); // not in Windows' list
        // switched off, already on this desktop (or shown on all), or nothing found: stays
        assert_eq!(desk_target(false, false, Some(d)), None);
        assert_eq!(desk_target(true, true, Some(d)), None);
        assert_eq!(desk_target(true, false, None), None);
        assert_eq!(desk_target(true, false, Some(d)), Some(d));
        // a reference desktop counts only if it is one Windows lists
        assert!(desk_ok(d, &[pinned, d]));
        assert!(!desk_ok(pinned, &[d]));
        assert!(!desk_ok(d, &[]));
        assert!(!desk_ok(GUID::zeroed(), &[GUID::zeroed()]));
        // VirtualDesktopIDs: GUIDs as in memory, back to back; a cut-off tail is dropped
        let mut b = Vec::new();
        for g in [d, pinned] {
            b.extend(g.data1.to_le_bytes());
            b.extend(g.data2.to_le_bytes());
            b.extend(g.data3.to_le_bytes());
            b.extend(g.data4);
        }
        assert_eq!(desk_ids(&b), vec![d, pinned]);
        assert_eq!(desk_ids(&b[..20]), vec![d]);
        assert_eq!(desk_ids(&[]), vec![]);
        assert_eq!(desk_ids(&[0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x99, 0, 0, 0, 0, 0, 0, 0])[0].data1, 0x5566_7788);
        // the setting: older ini on, saved either way, app-wide (a loaded session keeps it)
        assert!(Session::parse("side=300\n").bring_here);
        assert!(!Session::parse("follow_desktop=0\n").bring_here);
        for on in [true, false] {
            assert_eq!(Session::parse(&Session { bring_here: on, ..Session::default() }.to_ini()).bring_here, on);
        }
        let mut cur = Session::default();
        cur.take_layout(&Session::parse("follow_desktop=0\n"));
        assert!(cur.bring_here);
    }

    #[test]
    fn tree_input_kept_only_for_tree_keys() {
        assert!(tree_input_after(false, true, false)); // arrow in the tree
        assert!(!tree_input_after(true, true, true)); // Ctrl+Tab / F6 in the tree
        assert!(!tree_input_after(true, false, false) && !tree_input_after(false, false, false)); // elsewhere: cleared
    }

    #[test]
    fn tree_input_clicks() {
        assert!(!tree_input_click(true, false, true) && !tree_input_click(true, false, false)); // another pane: cleared
        assert!(tree_input_click(false, true, true)); // left click in the tree
        assert!(tree_input_click(true, true, false) && !tree_input_click(false, true, false)); // right click in the tree: unchanged
    }

    #[test]
    fn sort_keys() {
        let k = |pid| PROPERTYKEY { fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC), pid };
        // the same column on top flips; another column, or no sort, takes the key's first direction
        assert_eq!([sort_next(Some((k(14), -1)), k(14), -1), sort_next(Some((k(14), 1)), k(14), -1)], [1, -1]);
        assert_eq!([sort_next(Some((k(12), -1)), k(10), 1), sort_next(Some((k(12), 1)), k(14), -1)], [1, -1]);
        assert_eq!(sort_next(None, k(12), -1), -1);
        // only the sort half changes: columns and widths stay character for character
        let text = cols_text(&[(k(10), 220), (k(12), 80)], &[(k(12), -1), (k(10), 1)]);
        let new = with_sort(&text, k(14), -1);
        assert_eq!(new.split_once(';').unwrap().0, text.split_once(';').unwrap().0);
        assert_eq!(parse_cols(&new), (vec![(k(10), 220), (k(12), 80)], vec![(k(14), -1)]));
        assert_eq!(parse_cols(&with_sort("", k(10), 1)), (vec![], vec![(k(10), 1)]));
        assert!(with_sort("", k(10), 1).starts_with(';'));
        // name, date modified, size: A→Z, newest, largest first
        assert_eq!(nav::SORT_KEYS.map(|s| s.1), [1, -1, -1]);
    }

    #[test]
    fn columns() {
        let k = |pid| PROPERTYKEY { fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC), pid };
        let text = cols_text(&[(k(10), 220), (k(12), 80)], &[(k(12), -1)]);
        assert_eq!(parse_cols(&text), (vec![(k(10), 220), (k(12), 80)], vec![(k(12), -1)]));
        assert_eq!(parse_cols("junk;"), (vec![], vec![]));
        // kept in the session; a change counts wherever the pane is (columns are a pane setting)
        let mut base = Session { paths: vec![r"C:\a".into(); MAXP], ..Default::default() };
        base.cols[0] = text.clone();
        let base = Session::parse(&base.to_ini());
        assert_eq!(base.cols[0], text);
        let mut cur = base.clone();
        cur.cols[0] = cols_text(&[(k(12), 80), (k(10), 220)], &[(k(12), -1)]);
        assert!(layout_differs(&cur, &base));
        cur.paths[0] = r"C:\b".into();
        assert!(layout_differs(&cur, &base));
        assert!(is_fs(r"C:\") && is_fs(r"\\srv\share") && !is_fs("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"));
        let titles = [r"C:\Users\x\Downloads", "", r"C:\"].map(window_title);
        assert_eq!(titles, ["Downloads - Grids", "Grids", "C: - Grids"]);
        // "Users - Grids" is what the runner's double-click-for-parent test waits for
        let titles = [r"C:\Users", r"C:\Users\", r"C:\Users\Public", r"\\srv\share"].map(window_title);
        assert_eq!(titles, ["Users - Grids", "Users - Grids", "Public - Grids", "share - Grids"]);
    }

    #[test]
    fn dblclick_up() {
        let c = |time, x, y| Click { win: 7, gen: 1, time, x, y };
        let next = |prev, now, bg| next_click(prev, now, bg, 500, 4, 4);
        // background, then background in time and box: up, and the pair is consumed
        let (rec, up) = next(None, c(100, 10, 10), true);
        assert!(rec == Some(c(100, 10, 10)) && !up);
        let (rec, up) = next(rec, c(400, 12, 8), true);
        assert!(rec.is_none() && up);
        assert!(!next(rec, c(450, 12, 8), true).1); // third click
        // first press on an item: the next one only records
        let (rec, up) = next(None, c(100, 10, 10), false);
        assert!(rec.is_none() && !up);
        assert!(!next(rec, c(200, 10, 10), true).1);
        // out of time, x, y
        let p = Some(c(100, 10, 10));
        assert!(!next(p, c(601, 10, 10), true).1);
        assert!(!next(p, c(200, 13, 10), true).1);
        assert!(!next(p, c(200, 10, 7), true).1);
        // MSG.time wraps
        assert!(next(Some(c(u32::MAX - 10, 10, 10)), c(20, 10, 10), true).1);
        // a navigation between the clicks
        assert!(!next(p, Click { gen: 2, ..c(200, 10, 10) }, true).1);

        assert!(bg_role(33, 917, 917) && bg_role(34, 19, 917));
        assert!([20, 43, 25, 3, 62].iter().all(|&r| !bg_role(r, 917, 917)));
        assert!(!bg_role(33, 25, 917)); // the column header bar (its dividers and gaps)

        // a navigation during the probe voids it; g0 is the value before
        let g = Cell::new(5);
        assert_eq!(bg_probe(|| g.get(), || { g.set(6); true }), (5, false));
        assert_eq!(bg_probe(|| g.get(), || true), (6, true));

        // the generation is read after the queries (sol regression)
        let g = Cell::new(5);
        assert!(!confirm((5, 9), || { g.set(6); (Some(9), Some(0)) }, || g.get()));
        let g = || 5;
        assert!(!confirm((5, 9), || (Some(8), Some(0)), g));
        assert!(!confirm((5, 9), || (Some(9), Some(1)), g));
        assert!(!confirm((5, 9), || (Some(9), None), g));
        assert!(!confirm((5, 9), || (None, Some(0)), g));
        assert!(confirm((5, 9), || (Some(9), Some(0)), g));
    }

    #[test]
    fn addr() {
        let x = r"C:\Program Files\x.exe";
        assert_eq!(addr_parts("cmd"), Some((Some("cmd"), Some(("cmd", "")))));
        assert_eq!(addr_parts("powershell -NoExit").unwrap().1, Some(("powershell", "-NoExit")));
        assert_eq!(addr_parts(r#""C:\Program Files\x.exe" -a"#), Some((None, Some((x, "-a")))));
        assert_eq!(addr_parts(r#""C:\Program Files\x.exe""#), Some((Some(x), Some((x, "")))));
        assert_eq!(addr_parts(r#""C:\Windows\notepad.exe"#), None);
        assert_eq!(addr_parts(r"C:\Program Files\x.exe -a").unwrap().1, None);
        assert_eq!(addr_parts(r#"cmd /k "echo hi""#), Some((None, Some(("cmd", r#"/k "echo hi""#)))));
        assert_eq!(addr_parts(r#""a"b"#), Some((None, None)));
        assert!(is_rel("..") && is_rel(r"sub\x"));
        assert!(!is_rel(r"C:\") && !is_rel(r"\\srv\s") && !is_rel("shell:Downloads") && !is_rel("::{GUID}"));
    }

    #[test]
    fn addr_program_lookup() {
        let exts = path_exts(".COM;.EXE;.BAT;.CMD");
        assert_eq!(path_exts(""), exts);
        assert_eq!(path_exts(" .exe ; ;x;.BAT"), [".exe", ".BAT"]);
        // relative entries (. too) and repeats are left out; System32 and Windows come first
        let dirs = prog_dirs(r"C:\Windows\", r#"C:\Tools;.;bin;"C:\Program Files\Git\cmd\";c:\windows\system32;;"#);
        assert_eq!(dirs, [r"C:\Windows\System32", r"C:\Windows", r"C:\Tools", r"C:\Program Files\Git\cmd"]);
        let files = [r"C:\Windows\System32\cmd.exe", r"C:\Tools\code.cmd", r"C:\Windows\notepad.exe"];
        // the file system ignores case (PATHEXT gives .EXE, the file is cmd.exe)
        let is_file = |p: &Path| files.iter().any(|f| f.eq_ignore_ascii_case(&p.to_string_lossy()));
        let found = |p: &str| find_prog(p, &dirs, &exts, is_file).map(|f| f.to_ascii_lowercase());
        for (typed, file) in [("cmd", files[0]), ("cmd.exe", files[0]), ("code", files[1]), ("notepad", files[2])] {
            assert_eq!(found(typed), Some(file.to_ascii_lowercase()), "{typed}");
        }
        assert_eq!(found("chrome"), None);
        assert_eq!(find_prog(".exe", &dirs, &exts, |_| false), None);
    }

    // "Star on GitHub" runs gh by full path from System32, Windows or PATH's absolute folders: never one next to
    // Grids or in a relative PATH entry, and nothing (the page opens) when there is none.
    #[test]
    fn gh_by_full_path() {
        let path = r#".;bin;"C:\Program Files\GitHub CLI\";C:\Other"#;
        let files = [r"C:\Program Files\GitHub CLI\gh.exe", r"C:\Other\gh.exe", r"bin\gh.exe", r".\gh.exe", r"C:\Tools\gh.cmd"];
        let is_file = |p: &Path| files.iter().any(|f| f.eq_ignore_ascii_case(&p.to_string_lossy()));
        assert_eq!(gh_path(r"C:\Windows", path, is_file).as_deref(), Some(files[0]));
        assert_eq!(gh_path(r"C:\Windows", r".;bin;C:\Tools", is_file), None);
        assert_eq!(gh_path(r"C:\Windows", "", |_| false), None);
    }

    // Repro: a bare command typed in the address bar must never run a same-named file planted in the pane's folder.
    // A cmd.bat that leaves hit.txt sits in the folder; the address bar's launch of "cmd /c exit" there must not run it.
    #[test]
    fn addr_command_ignores_planted_file() {
        use windows::Win32::System::Threading::WaitForSingleObject;
        let d = std::env::temp_dir().join(format!("grids-plant-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("cmd.bat"), "@echo planted> \"%~dp0hit.txt\"\r\n").unwrap();
        let dir = HSTRING::from(d.as_os_str());
        let (prog, args) = (HSTRING::from(addr_program("cmd", Some(&d.to_string_lossy())).unwrap()), HSTRING::from("/c exit"));
        let mut sei = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOCLOSEPROCESS,
            lpFile: PCWSTR(prog.as_ptr()),
            lpParameters: PCWSTR(args.as_ptr()),
            lpDirectory: PCWSTR(dir.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        unsafe {
            ShellExecuteExW(&mut sei).unwrap();
            if !sei.hProcess.is_invalid() {
                WaitForSingleObject(sei.hProcess, 10_000);
                _ = CloseHandle(sei.hProcess);
            }
        }
        let hit = d.join("hit.txt").exists();
        _ = std::fs::remove_dir_all(&d);
        assert!(!hit, "the planted cmd.bat in the pane's folder ran");
    }

    #[test]
    fn shell_ops_outlive_window() {
        // restart / sign-out: refused only while a copy of ours runs (ours, or the shell's progress window in this process)
        assert!(!block_shutdown(0, false));
        assert!(block_shutdown(1, false));
        assert!(block_shutdown(0, true));
        // after the window: wait while a shell thread holds a reference, unless no window of ours showed for 30 s
        assert!(keep_waiting(1, Duration::ZERO));
        assert!(keep_waiting(1, Duration::from_secs(29)));
        assert!(!keep_waiting(1, Duration::from_secs(30)));
        assert!(!keep_waiting(0, Duration::ZERO));
        assert!(!keep_waiting(-1, Duration::ZERO));

        // the real thing: a shell thread made with CTF_PROCESS_REF (as the copy engine's are) holds the count up
        // while it runs, and wait_shell_ops returns once it ends, with nothing left
        unsafe extern "system" fn work(p: *mut std::ffi::c_void) -> u32 {
            WaitForSingleObject(HANDLE(p), 10_000);
            0
        }
        unsafe {
            _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let held = hold_shell_refs().expect("SHCreateThreadRef");
            let base = SHELL_REFS.load(Ordering::SeqCst);
            assert!(base >= 1, "{base}");
            let go = CreateEventW(None, true, false, None).unwrap();
            SHCreateThread(Some(work), Some(go.0 as *const _), CTF_PROCESS_REF as u32, None).unwrap();
            let t = Instant::now();
            while SHELL_REFS.load(Ordering::SeqCst) <= base && t.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(SHELL_REFS.load(Ordering::SeqCst) > base, "the shell thread took no reference");
            _ = SetEvent(go);
            let t = Instant::now();
            wait_shell_ops(Some(held));
            assert_eq!(SHELL_REFS.load(Ordering::SeqCst), 0);
            assert!(t.elapsed() < Duration::from_secs(10), "waited out the give-up instead: {:?}", t.elapsed());
            _ = CloseHandle(go);
            CoUninitialize();
        }
    }

    #[test]
    fn no_idle_sleep_while_copying() {
        // the same rule as a refused restart: a copy of ours, or the shell's progress window
        assert!(!block_shutdown(0, false));
        assert!(block_shutdown(2, false));
        assert!(block_shutdown(0, true));
        // the progress window is only looked for while the shell holds a reference beyond ours
        assert!(!shell_op_maybe(1, 1));
        assert!(shell_op_maybe(2, 1));
        assert!(!shell_op_maybe(5, i32::MAX)); // hold_shell_refs failed: never
        // the status line says so only when the request is really set, in both languages
        assert_eq!(copy_note(false, false), "Copying / moving…");
        assert_eq!(copy_note(true, false), "복사·이동 중…");
        assert!(copy_note(false, true).starts_with(copy_note(false, false)) && copy_note(false, true).ends_with("sleep held off"));
        assert!(copy_note(true, true).starts_with(copy_note(true, false)) && copy_note(true, true).ends_with("절전 안 함"));
        // Set / Clear only on a change, in pairs; the guard counts a run while it lives
        unsafe {
            stay_awake(true);
            assert!(AWAKE.load(Ordering::Relaxed), "PowerSetRequest failed");
            stay_awake(true);
            stay_awake(false);
            assert!(!AWAKE.load(Ordering::Relaxed));
            stay_awake(false);
        }
        let before = COPY_RUNS.load(Ordering::Relaxed);
        {
            let _run = CopyRun::new();
            assert_eq!(COPY_RUNS.load(Ordering::Relaxed), before + 1);
        }
        assert_eq!(COPY_RUNS.load(Ordering::Relaxed), before);
    }

    #[test]
    fn paste() {
        let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(paste_mode(&v(&[r"C:\a.txt"]), r"C:\"), Some(true));
        assert_eq!(paste_mode(&v(&[r"c:\X\a.txt", r"C:\x\b\"]), r"C:\X\"), Some(true));
        assert_eq!(paste_mode(&v(&[r"C:\a.txt"]), r"D:\x"), Some(false));
        assert_eq!(paste_mode(&v(&[r"\\srv\share\a.txt"]), r"\\SRV\share"), Some(true));
        assert_eq!(paste_mode(&v(&[r"\\srv\share\a.txt"]), r"C:\x"), Some(false));
        assert_eq!(paste_mode(&v(&[r"C:\a.txt", r"D:\b.txt"]), r"C:\"), None); // mixed
        assert_eq!(paste_mode(&[], r"C:\"), None);
        assert_eq!(paste_mode(&v(&["::{x}", r"C:\a"]), r"D:\"), None);
        assert_eq!(paste_mode(&v(&[r"C:\"]), r"D:\"), None); // a drive itself has no parent

        // (tries, shell): first press starts one copy, no shell; a held key's repeat: neither; unsupported: shell
        let run = |repeat: bool, ok: bool| {
            let mut tries = 0;
            let handled = paste_key(repeat, || {
                tries += 1;
                ok
            });
            (tries, !handled)
        };
        assert_eq!(run(false, true), (1, false));
        assert_eq!(run(true, true), (0, false));
        assert_eq!(run(false, false), (1, true));

        // ⇄ copy / move: a move into the same folder and mixed paths are refused
        assert_eq!(send_mode(&v(&[r"C:\x\a.txt"]), r"C:\x", false), Some(true));
        assert_eq!(send_mode(&v(&[r"C:\x\a.txt"]), r"C:\x", true), None);
        assert_eq!(send_mode(&v(&[r"C:\x\a.txt"]), r"D:\y", true), Some(false));
        assert_eq!(send_mode(&v(&[r"C:\x\a.txt", r"D:\y\b.txt"]), r"D:\y", false), None);

        // ⇄ copy / move to a favourite or recent folder: menu numbers never reach the pane rows' ranges
        for p in 0..5000 {
            let pane_row = (1..=100).contains(&p) || (101..=116).contains(&p) || (201..=216).contains(&p) || (301..=316).contains(&p);
            assert!(!(pane_row || is_shell_pick(p)) || send_pick(p).is_none(), "{p}");
        }
        assert_eq!(send_pick(401), Some(SendPick::To(false, 0)));
        assert_eq!(send_pick(440), Some(SendPick::To(false, 39)));
        assert_eq!(send_pick(501), Some(SendPick::To(true, 0)));
        assert_eq!(send_pick(540), Some(SendPick::To(true, 39)));
        assert_eq!(send_pick(450), Some(SendPick::Choose(false)));
        assert_eq!(send_pick(550), Some(SendPick::Choose(true)));
        assert_eq!(send_pick(441), None);
        assert_eq!(send_pick(216), None);
        assert!(SEND_FAVS + SEND_RECENT <= SEND_SLOTS && ID_SEND_COPY + SEND_SLOTS <= ID_CHOOSE_COPY && ID_SEND_MOVE + SEND_SLOTS <= ID_CHOOSE_MOVE);
        // favourites first, then recent; no shell places, nothing a pane shows, nothing twice (any case, trailing \)
        let shown = v(&[r"C:\Here", r"D:\pane2"]);
        let favs = v(&[r"c:\here\", r"E:\Backup", "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", r"\\nas\share"]);
        let recent = v(&[r"e:\backup", r"D:\PANE2", r"F:\Photos", r"C:\"]);
        let want = vec![(true, r"E:\Backup".to_string()), (true, r"\\nas\share".to_string()), (false, r"F:\Photos".to_string()), (false, r"C:\".to_string())];
        assert_eq!(send_targets(&shown, &favs, &recent), want);
        // capped: 12 favourites, 8 recent
        let many = |c: char| (0..30).map(|k| format!(r"{c}:\f{k}")).collect::<Vec<_>>();
        let got = send_targets(&[], &many('E'), &many('F'));
        assert_eq!((got.iter().filter(|t| t.0).count(), got.iter().filter(|t| !t.0).count()), (SEND_FAVS, SEND_RECENT));
        assert_eq!(got[SEND_FAVS].1, r"F:\f0");
        // a selected folder into itself or below it is refused; a sibling with the same start is not
        let sel = v(&[r"C:\x\Docs", r"C:\x\a.txt"]);
        let keys = src_keys(&sel);
        assert_eq!(send_check(&sel, &keys, r"c:\x\docs", false), None);
        assert_eq!(send_check(&sel, &keys, r"C:\x\Docs\sub", true), None);
        assert_eq!(send_check(&sel, &keys, r"C:\x\Docs2", false), Some(false));
        assert_eq!(send_check(&sel, &keys, r"C:\x", true), None); // a move where they already are
        assert_eq!(send_check(&sel, &keys, r"C:\x", false), Some(true));
        assert_eq!(send_check(&sel, &keys, "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", false), None);
        // the folder picker answering with its starting folder is not sent
        assert!(same_dir(r"C:\x\", r"c:\X") && !same_dir(r"C:\x", r"C:\x2"));
        // shown as the recent folders menu does
        assert_eq!(folder_label(r"D:\R&D\Specs"), "Specs\tD:\\R&&D");
        assert_eq!(folder_label(r"D:\"), "D:\\\t");
        assert_eq!(mnemonic(M_CHOOSE.0), Some('O'));
        assert!(M_CHOOSE.1.contains("(&O)"));

        // the whole selection or nothing
        let mut calls = 0;
        let all = collect_all(10_000, |k| {
            calls += 1;
            Some(k.to_string())
        });
        assert_eq!((all.map(|v| v.len()), calls), (Ok(10_000), 10_000));
        let mut calls = 0;
        let many = collect_all(10_001, |k| {
            calls += 1;
            Some(k.to_string())
        });
        assert_eq!((many, calls), (Err(Pick::TooMany), 0));
        assert_eq!(collect_all(0, |_| Some(String::new())), Err(Pick::Empty));
        assert_eq!(collect_all(5, |k| (k != 2).then(|| k.to_string())), Err(Pick::Unreadable));
    }

    #[test]
    fn paste_picture() {
        // a BITMAPINFOHEADER (size, bit count, compression, colours used) and `extra` bytes after it
        let dib = |size: u32, bits: u16, comp: u32, used: u32, extra: usize| {
            let mut d = vec![0u8; size as usize + extra];
            d[0..4].copy_from_slice(&size.to_le_bytes());
            d[14..16].copy_from_slice(&bits.to_le_bytes());
            d[16..20].copy_from_slice(&comp.to_le_bytes());
            d[32..36].copy_from_slice(&used.to_le_bytes());
            d
        };
        let off = |b: Vec<u8>| (u32::from_le_bytes(b[2..6].try_into().unwrap()), u32::from_le_bytes(b[10..14].try_into().unwrap()), b[..2].to_vec());
        assert_eq!(dib_to_bmp(&dib(40, 24, 0, 0, 12)).map(off), Some((66, 54, b"BM".to_vec())));
        assert_eq!(dib_to_bmp(&dib(40, 32, 3, 0, 16)).map(off), Some((70, 66, b"BM".to_vec()))); // BI_BITFIELDS masks
        assert_eq!(dib_to_bmp(&dib(124, 32, 3, 0, 16)).map(|b| off(b).1), Some(138)); // V5: masks inside the header
        assert_eq!(dib_to_bmp(&dib(40, 8, 0, 0, 1024 + 4)).map(|b| off(b).1), Some(1078)); // 256-colour palette
        assert_eq!(dib_to_bmp(&dib(40, 8, 0, 2, 8 + 4)).map(|b| off(b).1), Some(62));
        assert_eq!(dib_to_bmp(&dib(40, 8, 0, 0, 1024)), None); // palette but no pixels
        assert_eq!(dib_to_bmp(&dib(40, 24, 0, 0, 0)), None);
        assert_eq!(dib_to_bmp(&dib(40, 24, 0, 0, 4)[..20]), None);
        assert_eq!(dib_to_bmp(&[]), None);

        assert!(picture_only(false, false, false, false));
        for k in 0..4 {
            assert!(!picture_only(k == 0, k == 1, k == 2, k == 3));
        }

        let t = SYSTEMTIME { wYear: 2026, wMonth: 10, wDay: 21, wHour: 15, wMinute: 30, wSecond: 2, ..Default::default() };
        assert_eq!(image_name(false, &t, 1), "Clipboard image 2026-10-21 153002.png");
        assert_eq!(image_name(true, &t, 2), "클립보드 이미지 2026-10-21 153002 (2).png");

        // text_only(hdrop items, idlist, fgd, text, private): text alone, a picture beside it doesn't count
        assert!(text_only(false, false, false, true, false));
        assert!(!text_only(false, false, false, false, false));
        for k in 0..4 {
            assert!(!text_only(k == 0, k == 1, k == 2, true, k == 3));
        }

        let name = |text: &str, k: u32| text_name(false, text, &t, k);
        assert_eq!(name("Hello world\r\nsecond line", 1), "Hello world.txt");
        assert_eq!(name("\r\n  \n\t  Meeting notes  \nrest", 1), "Meeting notes.txt");
        assert_eq!(name("a/b\\c:d*e?f\"g<h>i|j", 1), "a b c d e f g h i j.txt");
        assert_eq!(name("tab\there   and  \u{1}ctl", 1), "tab here and ctl.txt");
        assert_eq!(name("Ends with dots...", 1), "Ends with dots.txt");
        assert_eq!(name(&"x".repeat(50), 1), format!("{}.txt", "x".repeat(40)));
        assert_eq!(name(&format!("{}. {}", "y".repeat(38), "z".repeat(9)), 1), format!("{}.txt", "y".repeat(38))); // cut at 40, then ". " goes
        assert_eq!(name(&"가".repeat(45), 1), format!("{}.txt", "가".repeat(40)));
        assert_eq!(name("Hello", 3), "Hello (3).txt");
        for dev in ["CON", "con", "Nul", "aux.cert", "com1", "LPT9.txt", "con .x", "CONOUT$"] {
            assert_eq!(name(dev, 1), "Clipboard text 2026-10-21 153002.txt", "{dev}");
        }
        for ok in ["COM0", "COM10", "Console", "conx"] {
            assert_eq!(name(ok, 1), format!("{ok}.txt"));
        }
        assert_eq!(name("  \r\n ... \n", 2), "Clipboard text 2026-10-21 153002 (2).txt");
        assert_eq!(name("???", 1), "Clipboard text 2026-10-21 153002.txt");
        assert_eq!(text_name(true, "", &t, 1), "클립보드 텍스트 2026-10-21 153002.txt");

        let dir = tree("picture", &[("a.png".into(), 3)]);
        let name = |k: u32| if k == 1 { "a.png".to_string() } else { format!("a ({k}).png") };
        assert_eq!(save_unique(&dir, name, b"new").unwrap(), "a (2).png");
        assert_eq!((std::fs::read(dir.join("a.png")).unwrap(), std::fs::read(dir.join("a (2).png")).unwrap()), (vec![0; 3], b"new".to_vec()));
        assert_eq!(save_unique(&dir, name, b"x").unwrap(), "a (3).png");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_layout_args() {
        let mut s = Session { side: 200, active: 3, ..Default::default() };
        s.set_heights(vec![1, 3]);
        (s.colr, s.rowr[1]) = (vec![0.25, 0.75], vec![0.5, 0.25, 0.25]);
        s.win = Some([1, 2, 300, 400, 1]);
        s.paths[1] = r"C:\a=b\폴더".into();
        s.views[1] = (4, 16);
        assert_eq!(Session::parse(&s.to_ini()), s);
        // garbage in the ini falls back to sane values
        let bad = Session::parse("cols=9,0,2\ncolr=1,-1\nrowr=x|1|1,1\nactive=99\nv3=x\np99=C:\\");
        assert_eq!((bad.heights.clone(), bad.colr.clone(), bad.active), (vec![4, 1, 2], even(3), 0));
        assert_eq!(bad.rowr, vec![even(4), vec![1.0], even(2)]);
        // an older even-grid ini still loads: rows x cols, one row-ratio group for every column
        let old = Session::parse("grid=3,2\nrowr=0.5,0.25,0.25");
        assert_eq!((old.heights.clone(), old.rowr[1].clone()), (vec![3, 3], vec![0.5, 0.25, 0.25]));
        // a named session brings grid + folders but not the window
        let mut cur = Session { side: 300, win: Some([0, 0, 9, 9, 0]), ..Default::default() };
        cur.take_layout(&s);
        assert_eq!((cur.heights.clone(), cur.paths[1].as_str(), cur.side, cur.win), (vec![1, 3], s.paths[1].as_str(), 300, Some([0, 0, 9, 9, 0])));

        // panes are numbered down each column; the picker drag sets one column at a time
        assert_eq!([slot(&[1, 3], 0), slot(&[1, 3], 1), slot(&[1, 3], 3)], [(0, 0), (1, 0), (1, 2)]);
        assert_eq!(drag_heights(drag_heights(vec![1], 0, 1), 2, 1), vec![1, 3]); // left 1, then right column down to row 3
        assert_eq!(drag_heights(vec![2, 2, 2], 1, 0), vec![2]); // moving back left drops columns
        assert_eq!((layout_label(&[2, 2]), layout_label(&[1, 3])), ("2 × 2".into(), "1 | 3".into()));
        // the picker's label and what a screen reader hears: the same words, then the pane count
        assert_eq!((grid_label(&[2, 2, 2], false), grid_label(&[2, 2, 2], true)), ("2 × 3".into(), "2행 × 3열".into()));
        assert_eq!((grid_label(&[2, 1], false), grid_label(&[2, 1], true)), ("Per column  2 | 1".into(), "열마다  2 | 1".into()));
        assert_eq!((grid_speech(&[2, 2, 2], false), grid_speech(&[2, 2, 2], true)), ("2 × 3, 6 panes".into(), "2행 × 3열, 패널 6개".into()));
        assert_eq!((grid_speech(&[1], false), grid_speech(&[1], true)), ("1 × 1, 1 pane".into(), "1행 × 1열, 패널 1개".into()));
        assert_eq!(grid_intro(&[2, 1], false), "Layout, now Per column  2 | 1, 3 panes · arrow keys pick, Enter applies");
        assert_eq!(grid_intro(&[2, 2], true), "레이아웃, 지금 2행 × 2열, 패널 4개 · 방향키로 고르고 Enter로 적용");
        assert_eq!(grid_label(&[], false), "");

        // green dot: layout, ratios and view modes count; folders and unknown (0) baseline views don't
        let base = s.clone();
        let mut cur = s.clone();
        (cur.paths[0], cur.views[2]) = ("D:\\".into(), (1, 48));
        assert!(!layout_differs(&cur, &base));
        cur.colr = vec![0.3, 0.7];
        assert!(layout_differs(&cur, &base));
        assert!(!s.snapshot().contains("win=") && !s.snapshot().contains("session="));
        // colour labels: round trip, garbage and older ini -> none; a label change alone counts; named sessions keep them
        let mut t = s.clone();
        t.tints[3] = 4;
        assert_eq!(Session::parse(&t.to_ini()).tints[3], 4);
        assert!(t.snapshot().contains("t3=4\n") && !s.to_ini().contains("t3="));
        assert_eq!(Session::parse("t5=9\nt6=x\nt99=2").tints, vec![0; MAXP]);
        assert_eq!(Session::parse(&s.to_ini()).tints, vec![0; MAXP]);
        assert!(layout_differs(&t, &s));
        // the label strip sits inside the border and above the buttons and fields
        for dpi in [96, 120, 144, 192] {
            for w in [120, 400] {
                for active in [false, true] {
                    let (b, top) = (tint_band(w, dpi, active), pane_geo(w, 300, dpi).num.top);
                    let border = if active { 2 * dpi / 96 } else { 1 };
                    assert!(b.top >= border && b.bottom <= top && b.bottom - b.top >= 2, "{dpi} {w} {active}");
                }
            }
        }
        // the drawn star: solid middle and empty corners when filled; hollow middle as an outline
        let (full, ring) = (star_coverage(15, Star::Filled), star_coverage(15, Star::Outline));
        assert_eq!((full[7 * 15 + 7], full[0], full[14 * 15 + 14]), (255, 0, 0));
        assert!(ring[8 * 15 + 7] == 0 && ring.iter().any(|&a| a == 255));
        for n in [15, 22] {
            // the + (what Plus adds to the outline) sits exactly on the box centre in both directions
            let (plus, ring) = (star_coverage(n, Star::Plus), star_coverage(n, Star::Outline));
            let added: Vec<(i32, i32)> = (0..n * n).filter(|&k| plus[k as usize] != ring[k as usize]).map(|k| (k % n, k / n)).collect();
            let span = |f: fn(&(i32, i32)) -> i32| added.iter().map(f).min().unwrap() + added.iter().map(f).max().unwrap();
            assert_eq!((span(|p| p.0), span(|p| p.1)), (n - 1, n - 1), "{n}");
        }
        // the F5 ↻ turns about the box centre exactly: a quarter turn is the rest image rotated 90° clockwise,
        // and the ink stays the same however far round it is (no shake, no breathing)
        for n in [16, 24, 32] {
            let rest = refresh_coverage(n, 0.0);
            let quarter = refresh_coverage(n, 0.25);
            for j in 0..n {
                for i in 0..n {
                    let (a, b) = (quarter[(j * n + i) as usize] as i32, rest[((n - 1 - i) * n + j) as usize] as i32);
                    assert!((a - b).abs() <= 1, "{n} ({i},{j}) {a} vs {b}");
                }
            }
            let ink = |t: f32| refresh_coverage(n, t).iter().map(|&a| a as f32).sum::<f32>();
            for t in [0.07, 0.13, 0.31, 0.5, 0.77] {
                assert!((ink(t) / ink(0.0) - 1.0).abs() < 0.04, "{n} {t}");
            }
            assert!(rest.iter().any(|&a| a >= 200) && rest[(n / 2 * n + n / 2) as usize] == 0, "{n}: a ring, hollow middle");
        }
        // favourites: kept in grids.ini, case-insensitive, never in a named session
        let mut f = Session { favs: vec![r"C:\Work".into(), r"D:\".into()], ..Default::default() };
        f = Session::parse(&f.to_ini());
        assert!(f.is_fav(r"c:\work") && !f.is_fav(r"C:\Work\x") && !f.is_fav(""));
        assert!(!f.snapshot().contains("fav="));
        assert_eq!((short_name(r"C:\Users\x\"), short_name("C:\\"), short_name(THIS_PC)), ("x".into(), "C:".into(), "This PC".into()));
    }

    // What a screen reader hears on Ctrl+Tab, Ctrl+1 … 9, Alt+Shift+arrows and the tab keys
    #[test]
    fn pane_speech_words() {
        assert_eq!(pane_speech(false, 2, 4, "Documents", 1, 3, true), "Pane 3 of 4 · Documents · tab 2 of 3");
        assert_eq!(pane_speech(true, 2, 4, "문서", 1, 3, true), "패널 3/4 · 문서 · 탭 2/3");
        // one tab, or tabs off: no tab part
        assert_eq!(pane_speech(false, 0, 2, "Downloads", 0, 1, true), "Pane 1 of 2 · Downloads");
        assert_eq!(pane_speech(true, 0, 2, "Downloads", 2, 3, false), "패널 1/2 · Downloads");
        // no folder yet: no folder part
        assert_eq!(pane_speech(false, 1, 2, "", 0, 2, true), "Pane 2 of 2 · tab 1 of 2");
        assert_eq!(pane_speech(true, 1, 2, "", 0, 1, true), "패널 2/2");
        // ten panes and more count in full
        assert_eq!(pane_speech(false, 11, 16, "x", 9, 12, true), "Pane 12 of 16 · x · tab 10 of 12");
        assert_eq!(pane_speech(true, 15, 16, "x", 0, 1, true), "패널 16/16 · x");
        // a drive root and shell places by their tab strip names, not "::{GUID}"
        assert_eq!(pane_speech(false, 0, 1, &short_name("C:\\"), 0, 1, true), "Pane 1 of 1 · C:");
        let bin = "::{645FF040-5081-101B-9F08-00AA002F954E}";
        assert_eq!(pane_speech(true, 0, 3, &shell_place(bin, true).unwrap(), 0, 1, true), "패널 1/3 · 휴지통");
        assert_eq!(pane_speech(false, 0, 3, &shell_place(bin, false).unwrap(), 1, 2, true), "Pane 1 of 3 · Recycle Bin · tab 2 of 2");
        assert_eq!(pane_speech(false, 0, 1, &short_name(THIS_PC), 0, 1, true), "Pane 1 of 1 · This PC");
    }

    #[test]
    fn shell_place_names() {
        let lib = "::{031E4825-7B94-4DC3-B131-E946B44C8DD5}";
        for (path, en, ko) in [
            ("::{645FF040-5081-101B-9F08-00AA002F954E}", "Recycle Bin", "휴지통"),
            ("::{f02c1a0d-be21-4350-88b0-7367fc96ef3c}", "Network", "네트워크"),
            (r"::{26EE0668-A00A-44D7-9371-BEB064C98683}\0", "Control Panel", "제어판"),
            ("::{21EC2020-3AEA-1069-A2DD-08002B30309D}", "Control Panel", "제어판"),
            (lib, "Libraries", "라이브러리"),
            (&format!(r"{lib}\Documents.library-ms"), "Documents", "문서"),
            (&format!(r"{lib}\Work.LIBRARY-MS"), "Work", "Work"),
            ("::{f874310e-b6b7-47dc-bc84-b9e6b38f5934}", "Home", "홈"),
            ("::{679f85cb-0220-4080-b29b-5540cc05aab6}", "Quick access", "빠른 액세스"),
            ("::{e88865ea-0e1c-4e20-9aa6-edcd0212c87c}", "Gallery", "갤러리"),
            ("search-ms:query=x", "Search: x", "검색: x"),
            ("search-ms:displayname=Search%20Results&crumb=System.Generic.String%3A%EB%B3%B4%EA%B3%A0%20%EC%84%9C&crumb=location:C%3A%5Cx", "Search: 보고 서", "검색: 보고 서"),
        ] {
            assert_eq!(shell_place(path, false).as_deref(), Some(en), "{path}");
            assert_eq!(shell_place(path, true).as_deref(), Some(ko), "{path}");
        }
        // folders, UNC, This PC, unknown GUIDs, other library forms and searches without a term keep short_name's label
        let unknown = "::{11111111-2222-3333-4444-555555555555}";
        for (path, label) in [
            (r"C:\Users\x\", "x"), ("C:\\", "C:"), (r"\\srv\share", "share"), (THIS_PC, "This PC"),
            (r"C:\Libraries\Documents.library-ms", "Documents.library-ms"), (unknown, unknown),
            (r"::{645FF040-5081-101B-9F08-00AA002F954E}\0", "0"), (&format!(r"{lib}\a.library-ms\b"), "b"),
            (r"::{26EE0668-A00A-44D7-9371-BEB064C98683}\123", "123"), ("search-ms:query=%FF", "search-ms:query=%FF"),
            ("search-ms:displayname=Found&crumb=location:C%3A%5Cx", "search-ms:displayname=Found&crumb=location:C%3A%5Cx"),
        ] {
            assert_eq!(shell_place(path, false), None, "{path}");
            assert_eq!(place_label(path), short_name(path), "{path}");
            assert_eq!(short_name(path), label, "{path}");
        }

        // cells tile the length exactly, with `gap` between neighbours
        let c = cells(10, 305, &[0.25, 0.25, 0.5], 5);
        assert_eq!((c[0].0, c[2].1), (10, 315));
        assert!(c.windows(2).all(|w| w[0].1 + 5 == w[1].0));

        // dragging respects the minimum size and keeps the pair's total
        let mut r = vec![0.5, 0.5];
        drag_ratio(&mut r, 0, -1000, 0, 205, 5, 20);
        assert_eq!(r[0], 0.1);
        assert!((r[0] + r[1] - 1.0).abs() < 1e-12);

        let mut s = Session::default();
        apply_args(&mut s, (0..5).map(|i| format!("::{{x{i}}}")).collect());
        assert_eq!((s.heights.clone(), s.paths[4].as_str()), (vec![2, 2, 2], "::{x4}"));
        assert_eq!((session_arg(&["a.GRIDS".into()]), session_arg(&["old.qdr".into()])), (Some("a.GRIDS"), Some("old.qdr")));
        assert_eq!(session_arg(&["a.grids".into(), "b".into()]), None);

        assert_eq!(filter_spec(" pdf; *.TXT ;"), "*pdf*;*.TXT");
        // filter / Num+: ZIPs (FOLDER|STREAM) match by name like files; an unreadable item stays in the filtered list
        let (f, s) = (SFGAO_FOLDER.0, SFGAO_STREAM.0);
        assert_eq!([real_folder(f), real_folder(f | s), real_folder(s), real_folder(0)], [true, false, false, false]);
        let keeps = [None, Some(f), Some(f | s), Some(0)].map(filter_keeps_unnamed);
        assert_eq!(keeps, [true, true, false, false]);
        assert_eq!((human(10), human(1536), human(5 << 30)), ("10 B".into(), "1.5 KB".into(), "5.0 GB".into()));
        // the Recycle Bin button's tooltip: a failed count says nothing about the contents (never "empty")
        // (and says files dropped on it are deleted to it)
        let (en, ko) = ("Drop files here to delete them (Shift: for good) · Right-click: Empty, Properties", "파일을 끌어다 놓으면 휴지통으로(Shift: 영구 삭제) · 오른쪽 클릭: 비우기·속성");
        assert_eq!(bin_tip(false, None), format!("Recycle Bin · {en}"));
        assert_eq!(bin_tip(true, None), format!("휴지통 · {ko}"));
        assert_eq!(bin_tip(false, Some((0, 0))), format!("Recycle Bin — empty · {en}"));
        assert_eq!(bin_tip(true, Some((0, 0))), format!("휴지통 — 비어 있음 · {ko}"));
        assert_eq!(bin_tip(false, Some((1, 10))), format!("Recycle Bin — 1 item · 10 B · {en}"));
        assert_eq!(bin_tip(false, Some((12, 5 << 30))), format!("Recycle Bin — 12 items · 5.0 GB · {en}"));
        assert_eq!(bin_tip(true, Some((12, 1536))), format!("휴지통 — 항목 12개 · 1.5 KB · {ko}"));

        // pane top bar: number, back, fwd, up, address, ⇄, filter left to right, nothing overlapping, inside the plate
        for (w, dpi) in [(800, 96), (260, 96), (1200, 144)] {
            let p = pane_geo(w, 500, dpi);
            let row = [p.num, p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.filt, p.fav];
            assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi}");
            assert!(p.num.right > p.num.left && p.fav.right <= w && p.view.top >= p.btn[0].bottom);
        }
        // the number only shows while the address edit keeps s(32); otherwise the bar is laid out as before
        for dpi in [96, 144] {
            let s = |v: i32| v * dpi / 96;
            for w in 60..=600 {
                let p = pane_geo(w, 500, dpi);
                if p.num.right > p.num.left {
                    let row = [p.num, p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.filt, p.fav];
                    assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi}");
                    assert!((p.addr.right - s(6)) - (p.addr.left + s(8)) >= s(32), "{w}@{dpi}");
                } else {
                    assert!(p.btn[0].left == s(4) && p.addr.left == s(4) + 3 * s(24) + s(4), "{w}@{dpi}");
                }
            }
        }
        let shown = |w, dpi| {
            let p = pane_geo(w, 500, dpi);
            p.num.right > p.num.left
        };
        assert!(!shown(240, 96) && shown(241, 96) && !shown(361, 144) && shown(362, 144));
    }

    #[test]
    fn missing_folders() {
        assert!(gone(HRESULT::from_win32(2)) && gone(HRESULT::from_win32(3)));
        for c in [E_ACCESSDENIED, HRESULT(0x80070035u32 as i32), HRESULT(0x80070015u32 as i32)] {
            assert!(!gone(c)); // denied, bad network path, not ready
        }
        use std::io::{Error as E, ErrorKind as K};
        let only_a = |p: &Path| if p == Path::new(r"C:\a") { Ok(true) } else { Err(E::from(K::NotFound)) };
        assert_eq!(near_dir(r"C:\a\b\c", only_a).as_deref(), Some(r"C:\a"));
        let denied = |p: &Path| if p == Path::new(r"C:\a\b") { Err(E::from(K::PermissionDenied)) } else { Ok(true) };
        assert_eq!(near_dir(r"C:\a\b\c", denied), None);
        assert_eq!(near_dir(r"C:\a\b\c", |_| Err(E::from(K::NotFound))), None);
        assert_eq!(near_dir(r"C:\a\b\c", |_| Ok(false)), None); // a file in the way

        let (m, t) = (r"C:\a\b".to_string(), r"C:\a".to_string());
        let start = || Some((m.clone(), t.clone()));
        let mut r = start(); // first navigation, synchronous
        assert_eq!(rec_step(&mut r, Nav::Pending, &t), None);
        assert_eq!(rec_step(&mut r, Nav::Complete, r"c:\A"), Some(m.clone())); // case only differs
        assert!(r.is_none());
        let mut r = start(); // another navigation took over
        rec_step(&mut r, Nav::Pending, &t);
        rec_step(&mut r, Nav::Pending, r"C:\x");
        assert!(r.is_none() && rec_step(&mut r, Nav::Complete, r"C:\x").is_none());
        let mut r = start(); // an earlier navigation's failure
        rec_step(&mut r, Nav::Failed, r"C:\y");
        assert_eq!(rec_step(&mut r, Nav::Complete, &t), Some(m.clone()));
        let mut r = start(); // the parent itself failed
        rec_step(&mut r, Nav::Failed, &t);
        assert!(r.is_none());
        let mut r = start(); // some other folder completed
        assert!(rec_step(&mut r, Nav::Complete, r"C:\x").is_none() && r.is_none());
    }

    #[test]
    fn renamed_folders() {
        // whole names only, any case
        assert_eq!(remap_dir(r"C:\a\b", r"C:\a\b", r"C:\a\z").as_deref(), Some(r"C:\a\z"));
        assert_eq!(remap_dir(r"c:\A\B\x\y", r"C:\a\b", r"C:\a\z").as_deref(), Some(r"C:\a\z\x\y"));
        assert_eq!(remap_dir(r"C:\a\bc", r"C:\a\b", r"C:\a\z"), None);
        assert_eq!(remap_dir(r"C:\a", r"C:\a\b", r"C:\a\z"), None);
        assert_eq!(remap_dir(r"C:\a\b", "", r"C:\z"), None);
        assert_eq!(remap_dir(r"C:\한글\b", r"C:\한", r"C:\z"), None); // a longer name, any script
        assert_eq!(remap_dir(r"C:\한", r"C:\한글", r"C:\z"), None);
        assert!(in_bin(r"D:\$Recycle.Bin\S-1-5-21\$RX1") && in_bin(r"d:\$RECYCLE.BIN") && !in_bin(r"D:\$Recycle.Binder"));

        use Change::{Removed as D, Renamed as R};
        let r = |o: &str, n: &str| R(o.into(), n.into());
        let to = |p: &str| Some(Follow::To(p.into()));
        assert_eq!(follow(r"C:\w\a\x", &[r(r"C:\w\a", r"C:\w\b")]), to(r"C:\w\b\x"));
        assert_eq!(follow(r"C:\w\ab", &[r(r"C:\w\a", r"C:\w\b")]), None); // a look-alike name
        // a rename of a rename, in any order of parent and child
        assert_eq!(follow(r"C:\w\a\x", &[r(r"C:\w\a", r"C:\w\b"), r(r"C:\w\b", r"C:\w\c")]), to(r"C:\w\c\x"));
        assert_eq!(follow(r"C:\w\a\x", &[r(r"C:\w\a\x", r"C:\w\a\y"), r(r"C:\w\a", r"C:\w\b")]), to(r"C:\w\b\y"));
        assert_eq!(follow(r"C:\w\a", &[r(r"C:\w\a", r"C:\w\b"), r(r"C:\w\b", r"C:\w\a")]), None); // back where it was
        assert_eq!(follow(r"C:\w\Foo", &[r(r"C:\w\Foo", r"C:\w\foo")]), to(r"C:\w\foo")); // case only
        // removed, or moved into the Recycle Bin (shell_change makes that a removal)
        assert_eq!(follow(r"C:\w\a\x", &[D(r"C:\w\a".into())]), Some(Follow::Gone));
        assert_eq!(follow(r"C:\w\a\x", &[r(r"C:\w\a", r"C:\w\b"), D(r"C:\w\b".into())]), Some(Follow::Gone));
        assert_eq!(follow(r"C:\w\ab", &[D(r"C:\w\a".into())]), None);
        // path reused: another folder took its name, or a removed one came back renamed: hold off
        assert_eq!(follow(r"C:\w\a", &[r(r"C:\w\n", r"C:\w\a"), r(r"C:\w\a", r"C:\w\b")]), None);
        assert_eq!(follow(r"C:\w\a", &[D(r"C:\w\a".into()), r(r"C:\w\a", r"C:\w\b")]), None);
        // the pane already shows the new name (the view followed by itself): nothing to do
        assert_eq!(follow(r"C:\w\b", &[r(r"C:\w\a", r"C:\w\b")]), None);

        // the follow mark ends like a recovery: only the new name completing confirms it
        let mut f = Some((r"C:\w\a".to_string(), r"C:\w\b".to_string()));
        rec_step(&mut f, Nav::Pending, r"C:\elsewhere"); // the user went somewhere else first
        assert!(f.is_none());
        let mut f = Some((r"C:\w\a".to_string(), r"C:\w\b".to_string()));
        rec_step(&mut f, Nav::Failed, r"C:\w\b");
        assert!(f.is_none());
    }

    // A drive letter that goes moves the panes on it to This PC; when it comes back, the ones still there go home.
    #[test]
    fn pulled_drive_panes() {
        const E: u32 = 1 << 4;
        assert_eq!([drive_bit(r"E:\x"), drive_bit("e:"), drive_bit(r"c:\")], [Some(E), Some(E), Some(1 << 2)]);
        for p in [r"\\srv\E$", r"\\?\E:\x", THIS_PC, "::{X}", "E:x", "", "E", r"1:\x"] {
            assert_eq!(drive_bit(p), None, "{p}");
        }
        let v = |s: &[&str]| s.iter().map(|p| p.to_string()).collect::<Vec<_>>();
        // E: went: its panes (any case) move; F:\E:, a share, This PC, an empty slot and a marked pane stay
        let paths = v(&[r"E:\사진\2026", r"F:\E", r"\\nas\E", THIS_PC, "", r"e:\", r"E:\a"]);
        let marks = [None, None, None, None, None, None, Some(r"E:\b")];
        assert_eq!(drive_moves(E, 0, &paths, &marks), (vec![0, 5], vec![]));
        assert_eq!(drive_moves(1 << 5, 0, &paths, &marks).0, [1]);
        // E: back: only marked panes on E: still on This PC (a pane the user moved on keeps its place)
        let paths = v(&[THIS_PC, r"C:\Users", &THIS_PC.to_lowercase(), THIS_PC, THIS_PC]);
        let marks = [Some(r"E:\사진"), Some(r"E:\a"), Some(r"e:\b"), Some(r"F:\c"), None];
        assert_eq!(drive_moves(0, E, &paths, &marks), (vec![], vec![0, 2]));
        assert_eq!(drive_moves(0, 0, &paths, &marks), (vec![], vec![]));
        // the mark lives through its own This PC opening, and any later navigation (This PC too) drops it
        let mut m = Some((r"E:\x".to_string(), false));
        unplug_step(&mut m, Nav::Pending, THIS_PC);
        unplug_step(&mut m, Nav::Complete, &THIS_PC.to_lowercase());
        assert_eq!(m, Some((r"E:\x".to_string(), true)));
        unplug_step(&mut m, Nav::Pending, THIS_PC); // the user (or a session) opens This PC again
        assert_eq!(m, None);
        for (ev, to) in [(Nav::Failed, THIS_PC), (Nav::Pending, r"C:\"), (Nav::Complete, r"D:\")] {
            let mut m = Some((r"E:\x".to_string(), false));
            unplug_step(&mut m, ev, to);
            assert_eq!(m, None);
        }
        // the note: letters once each, one count for several panes
        assert_eq!(unplug_text(false, false, "E:", 1), "Drive E: was removed · moved to This PC");
        assert_eq!(unplug_text(true, false, "E:, F:", 3), "E:, F: 드라이브가 빠져 패널 3개를 내 PC로 옮김");
        assert_eq!(unplug_text(false, true, "E:", 2), "Drive E: is back · reopened the folders of 2 panes");
        assert_eq!(unplug_text(true, true, "E:", 1), "E: 드라이브가 돌아와 원래 폴더를 다시 엶");
    }

    #[test]
    fn status_notes() {
        // more missing folders while the note still shows add up; else the count starts again
        assert_eq!([lost_next(0, true), lost_next(1, true), lost_next(3, false), lost_next(0, false)], [1, 2, 1, 1]);
        assert_eq!(gone_text(true, 1, r"C:\a\b", r"C:\a"), r"없는 폴더: C:\a\b · C:\a 엶");
        assert_eq!(gone_text(false, 1, r"C:\a\b", r"C:\a"), r"Not found: C:\a\b · opened C:\a");
        assert_eq!(gone_text(false, 3, r"C:\a\b", r"C:\a"), "3 saved folders were missing · opened the nearest parent folders");
        assert_eq!(gone_text(true, 3, r"C:\a\b", r"C:\a"), "저장된 폴더 3개가 없어 가장 가까운 상위 폴더를 엶");
        // the renamed folder's own name: trailing backslash, drive root, share
        assert_eq!(follow_text(false, r"C:\Work\Report 2026\"), "Followed renamed folder: Report 2026");
        assert_eq!(follow_text(true, r"C:\Work\Report 2026\"), "폴더 이름이 바뀌어 따라감: Report 2026");
        assert_eq!(follow_text(false, r"D:\"), "Followed renamed folder: D:");
        assert_eq!(follow_text(true, r"\\srv\share\new"), "폴더 이름이 바뀌어 따라감: new");
    }

    #[test]
    fn jump_picks() {
        let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let recent = v(&[r"C:\Users\me\Downloads", r"C:\Work\down", r"D:\Old Downloads", r"D:\문서\보고서 2026", THIS_PC]);
        let favs = v(&[r"E:\Down", r"\\srv\share", r"C:\"]);
        let pick = |q: &str, cur: &str| jump_pick(q, &recent, &favs, cur);
        // exact beats prefix beats inside; ties go to recent before favourites, newer before older
        assert_eq!(pick("down", ""), Some(r"C:\Work\down"));
        assert_eq!(pick("DOWNL", ""), Some(r"C:\Users\me\Downloads"));
        assert_eq!(pick("loads", ""), Some(r"C:\Users\me\Downloads"));
        assert_eq!(pick("  down  ", r"c:\work\DOWN"), Some(r"E:\Down")); // the folder on display is skipped
        assert_eq!(pick("보고", ""), Some(r"D:\문서\보고서 2026"));
        assert_eq!(pick("share", ""), Some(r"\\srv\share"));
        assert_eq!(pick("c", ""), Some(r"C:\")); // a drive root is named C:
        assert_eq!(pick("this", ""), None); // shell places aren't candidates
        for q in ["", "   ", r"down\x", "a/b", "c:", "\"down\"", "nothing"] {
            assert_eq!(pick(q, ""), None, "{q:?}");
        }
        // only text that named no path falls to a jump; a resolved path that won't open just beeps
        assert!(addr_jumps(None, false));
        assert!(!addr_jumps(None, true));
        assert!(!addr_jumps(Some(true), false) && !addr_jumps(Some(false), false));
    }

    #[test]
    fn recent_visits() {
        let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut r = Vec::new();
        visit(&mut r, r"C:\a", r"C:\b");
        assert_eq!(r, v(&[r"C:\b", r"C:\a"]));
        visit(&mut r, r"C:\b", r"c:\A"); // case-insensitive: moves up, no duplicate
        assert_eq!(r, v(&[r"c:\A", r"C:\b"]));
        let before = r.clone();
        for (old, new) in [(r"C:\b", r"c:\B"), ("", r"C:\x"), (r"C:\b", THIS_PC)] {
            visit(&mut r, old, new); // refresh / restore, fresh pane, shell place: no visit
        }
        assert_eq!(r, before);
        visit(&mut r, THIS_PC, r"D:\"); // leaving a shell place records only the folder
        assert_eq!(r[0], r"D:\");
        assert!(!r.iter().any(|p| p == THIS_PC));
        for k in 0..30 {
            visit(&mut r, r"C:\b", &format!(r"C:\n{k}"));
        }
        assert_eq!((r.len(), r[0].as_str(), r[1].as_str()), (RECENT, r"C:\n29", r"C:\b"));
    }

    #[test]
    fn recent_survives_restore() {
        let full: Vec<String> = (0..RECENT).map(|k| format!(r"C:\f{k}")).collect();
        let mut r = full.clone();
        // 16 panes come back to their folders, twice (start-up, then a theme / language rebuild)
        for _ in 0..2 {
            for p in &full[..16] {
                visit(&mut r, p, p);
            }
        }
        assert_eq!(r, full);
        let items = recent_items(&r, r"c:\F0");
        assert_eq!(items.len(), RECENT - 1);
        assert_eq!(items.last(), full.last()); // the oldest is still offered
    }

    #[test]
    fn recent_ini() {
        let mut s = Session { recent: vec![r"C:\b".into(), r"\\srv\share".into(), r"C:\a".into()], ..Default::default() };
        s = Session::parse(&s.to_ini());
        assert_eq!(s.recent, [r"C:\b", r"\\srv\share", r"C:\a"]);
        assert!(!s.snapshot().contains("recent="));
        let many: String = (0..30).map(|k| format!("recent=C:\\{k}\n")).collect();
        assert_eq!(Session::parse(&many).recent.len(), RECENT);
    }

    #[test]
    fn ini_write_and_load() {
        let dir = tree("ini", &[]);
        let ini = dir.join("grids.ini");
        let tmp = dir.join("grids.ini.tmp");
        // nothing there: defaults; only qdir.ini: that one
        assert!(load_ini(&dir).unwrap().is_none());
        std::fs::write(dir.join("qdir.ini"), "dark=1\n").unwrap();
        assert_eq!(load_ini(&dir).unwrap().as_deref(), Some("dark=1\n"));
        // the second write wins, round-trips, and leaves no .tmp
        let s = Session { recent: vec![r"C:\a".into()], ..Default::default() };
        write_ini(&ini, "dark=0\n").unwrap();
        write_ini(&ini, &s.to_ini()).unwrap();
        let back = load_ini(&dir).unwrap().unwrap();
        assert_eq!(back, s.to_ini());
        assert_eq!(Session::parse(&back).to_ini(), s.to_ini());
        assert!(!tmp.exists());
        // a grids.ini that cannot be read is an error, not a fall back to qdir.ini / defaults
        std::fs::remove_file(&ini).unwrap();
        std::fs::create_dir(&ini).unwrap();
        assert!(load_ini(&dir).is_err());
        // and a swap that fails reports it and cleans up
        assert!(write_ini(&ini, "dark=1\n").is_err());
        assert!(!tmp.exists() && ini.is_dir());
        _ = std::fs::remove_dir_all(&dir);
    }

    // A fresh folder under %TEMP% holding `files` (relative path, size).
    fn tree(name: &str, files: &[(String, usize)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("grids-test-{}-{name}", std::process::id()));
        _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        for (f, n) in files {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, vec![0u8; *n]).unwrap();
        }
        root
    }

    fn many(prefix: &str, count: usize, size: usize) -> Vec<(String, usize)> {
        (0..count).map(|k| (format!("{prefix}{k}.bin"), size)).collect()
    }

    #[test]
    fn folder_size_walk() {
        use std::cell::Cell;
        let never = || false;
        let mut quiet = |_, _| {};
        let root = tree("walk", &[("a.bin".into(), 100), (r"sub\b.bin".into(), 20), (r"sub\deep\c.bin".into(), 3)]);
        assert_eq!(dir_bytes(&root, &never, &never, &mut quiet), Some((123, false)));
        assert_eq!(dir_bytes(&root.join("missing"), &never, &never, &mut quiet), Some((0, true)));
        // a junction pointing back up is not followed
        let made = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(root.join(r"sub\loop")).arg(&root).output();
        assert!(made.is_ok_and(|o| o.status.success()) && root.join(r"sub\loop\a.bin").exists());
        assert_eq!(dir_bytes(&root, &never, &never, &mut quiet), Some((123, false)));
        // a stop mid-walk gives no total
        let calls = Cell::new(0);
        let later = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        assert_eq!(dir_bytes(&root, &later, &never, &mut quiet), None);

        // many files, no subfolders: a copy that starts and ends between status ticks only pauses the walk
        let flat = tree("flat", &many("f", 300, 2));
        let seen = Cell::new(0);
        let copying = || {
            seen.set(seen.get() + 1);
            (100..103).contains(&seen.get())
        };
        assert_eq!(dir_bytes(&flat, &never, &copying, &mut quiet), Some((600, false)));
        assert!(seen.get() > 300);
        // a cancel still gets through while paused
        let paused = Cell::new(0);
        let copy_on = || {
            paused.set(paused.get() + 1);
            paused.get() > 50
        };
        let cancel = || paused.get() > 52;
        assert_eq!(dir_bytes(&flat, &cancel, &copy_on, &mut quiet), None);
        assert!(paused.get() < 60);
        // progress comes with the running total
        let big = tree("big", &many("g", 2100, 1));
        let mut last = 0;
        let mut progress = |b, _| last = b;
        assert_eq!(dir_bytes(&big, &never, &never, &mut progress), Some((2100, false)));
        assert!(last >= 2000);

        // F5 after a file was added: the same selection is summed again
        let mut job = SizeJob::default();
        let target = vec![flat.to_string_lossy().into_owned()];
        assert!(job.want(&target, false));
        assert!(job.publish(job.gen, 600, true, false));
        std::fs::write(flat.join("new.bin"), [0u8; 50]).unwrap();
        assert!(!job.want(&target, false));
        job.want(&[], true); // what resum_sizes does
        assert!(job.want(&target, false) && !job.done);
        assert_eq!(dir_bytes(&flat, &never, &never, &mut quiet), Some((650, false)));
        for r in [root, flat, big] {
            _ = std::fs::remove_dir_all(r);
        }
    }

    #[test]
    fn selection_paths_step() {
        let ms = Duration::from_millis;
        // the same selection (only the item count, filter or a copy changed): no shell calls
        let big = BIG_SEL as i32 + 1;
        assert_eq!(sel_step(true, 50_000, None), SelStep::Reuse);
        assert_eq!(sel_step(true, 5, Some(ms(0))), SelStep::Reuse);
        // up to BIG_SEL (2000) items: gathered at once, as before
        assert_eq!(sel_step(false, 1, None), SelStep::Collect);
        assert_eq!(sel_step(false, 2000, None), SelStep::Collect);
        // a bigger one (2001+) that just changed waits until the same key stayed for SEL_REST
        assert_eq!(sel_step(false, 2001, None), SelStep::Wait);
        assert_eq!(sel_step(false, big, Some(ms(249))), SelStep::Wait);
        assert_eq!(sel_step(false, big, Some(SEL_REST)), SelStep::Collect);
        assert_eq!(sel_step(false, big, Some(ms(300))), SelStep::Collect);
    }

    #[test]
    fn folder_size_job() {
        let (a, b) = (vec![r"C:\a".to_string()], vec![r"C:\b".to_string()]);
        let mut j = SizeJob::default();
        assert!(j.want(&a, false));
        let g = j.gen;
        assert!(!j.want(&a, false)); // same target: keeps running
        assert!(j.publish(g, 10, true, false) && j.done);
        assert!(!j.want(&a, false) && j.done && j.bytes == 10); // and keeps its result
        assert!(!j.want(&a, true)); // a copy starts: cancelled
        assert!(!j.publish(g, 99, true, false) && j.bytes == 10); // the old walk's result is dropped
        assert!(j.want(&a, false) && j.bytes == 0 && !j.done); // the copy ended: summed again
        assert!(!j.want(&[], false) && j.target.is_empty()); // no folders: cancelled
        assert!(j.want(&a, false));
        let g = j.gen;
        assert!(j.want(&b, false) && j.gen != g);
        assert!(!j.publish(g, 5, false, false));
        assert!(j.publish(j.gen, 5, false, true) && j.partial);
    }

    #[test]
    fn peek_newest_only() {
        let req = |p: &str| Some(PeekReq { path: p.into(), w: 10, h: 10, hwnd: 0, plain: false, fetch: false });
        let pix = || Peeked { body: Body::Pic(Pix { w: 1, h: 1, bgra: vec![0; 4] }), icon: -1, meta: Vec::new(), cloud: false };
        let card = || Peeked { body: Body::Card { icon: None, rows: Vec::new() }, icon: -1, meta: Vec::new(), cloud: false };
        let mut j = PeekJob { gen: 0, req: None, done: None, zoom: None, zseq: 0, sharp: Vec::new() };
        j.want(req("a"));
        let a = j.gen;
        j.want(req("b")); // arrow pressed before a's picture came
        assert!(!j.publish(a, pix()) && j.done.is_none()); // a's picture is dropped
        assert!(j.publish(j.gen, card()) && matches!(j.done, Some(Peeked { body: Body::Card { .. }, .. }))); // b: the facts only
        let b = j.gen;
        j.want(None); // closed
        assert!(j.gen != b && j.req.is_none() && j.done.is_none());
        assert!(!j.publish(b, pix()) && j.done.is_none());
        // a zoom's sharper pictures belong to the item they were made for
        j.want(req("c"));
        let z = Sharp { gen: j.gen, pdf: false, pages: vec![0], w: 20, h: 20 };
        j.want_sharp(Some(z.clone()));
        let seq = j.zseq;
        assert_eq!(j.zoom, Some((seq, z)));
        j.sharp.push((0, Pix { w: 1, h: 1, bgra: vec![0; 4] }));
        j.want_sharp(None); // back to fit
        assert!(j.zseq != seq && j.zoom.is_none() && j.sharp.is_empty());
        j.sharp.push((0, Pix { w: 1, h: 1, bgra: vec![0; 4] }));
        j.want(req("d"));
        assert!(j.zoom.is_none() && j.sharp.is_empty());
    }

    #[test]
    fn peek_alpha() {
        let mut photo = [10u8, 20, 30, 0, 40, 50, 60, 0];
        fix_alpha(&mut photo);
        assert_eq!(photo, [10u8, 20, 30, 255, 40, 50, 60, 255]);
        let mut icon = [0u8, 0, 0, 0, 40, 50, 60, 128];
        fix_alpha(&mut icon);
        assert_eq!(icon, [0u8, 0, 0, 0, 40, 50, 60, 128]); // real alpha: kept, transparent corners too
    }

    #[test]
    fn human_unit_boundaries() {
        // just under the next unit shows as "1.0" of it, never "1024.0" of the one below
        let table: [(u64, &str); 12] = [
            (0, "0 B"),
            (1023, "1023 B"),
            (1024, "1.0 KB"),
            (1_048_524, "1023.9 KB"),
            (1_048_525, "1.0 MB"),
            (1_048_575, "1.0 MB"),
            (1_073_689_395, "1023.9 MB"),
            (1_073_689_396, "1.0 GB"),
            ((1 << 30) - 1, "1.0 GB"),
            ((1 << 40) - 1, "1.0 TB"),
            (1 << 40, "1.0 TB"),
            (5 << 40, "5.0 TB"),
        ];
        for (b, want) in table {
            assert_eq!(human(b), want, "{b}");
        }
        assert!(human(u64::MAX).ends_with(" TB")); // TB is the top unit, no panic

        // property: sizes up to 2^50 around every unit edge and at random never print "1024.0 X" below TB,
        // and a bigger size never gets a smaller unit
        let units = ["B", "KB", "MB", "GB", "TB"];
        let mut sizes: Vec<u64> = Vec::new();
        for e in 10..=50 {
            let edge = 1u64 << e;
            sizes.extend((0..64).flat_map(|d| [edge - d, edge + d]));
        }
        for k in 1..=3 {
            let t = (1023.95 * 1024f64.powi(k)) as u64;
            sizes.extend(t - 3..=t + 3);
        }
        let mut x = 0x9E37_79B9_7F4A_7C15u64; // fixed-seed xorshift
        for _ in 0..5000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            // 50 random bits shifted down 0..=40 places, so B..TB all get random sizes (not ~all TB)
            sizes.push((x >> 14) >> (x % 41));
        }
        sizes.sort_unstable();
        sizes.dedup();
        let mut last = 0;
        for b in sizes {
            let s = human(b);
            let (num, unit) = s.rsplit_once(' ').unwrap();
            let u = units.iter().position(|&n| n == unit).unwrap();
            assert!(u == 4 || !num.starts_with("1024.0"), "{b} -> {s}");
            assert!(u >= last, "{b} -> {s}");
            last = u;
        }
    }

    #[test]
    fn folder_size_text() {
        let kb = human(1024);
        // running: a folder met shows "calculating", bytes so far show with "…"
        assert_eq!(size_text(false, Some((0, false, false, true)), false, None), " (calculating…)");
        assert_eq!(size_text(true, Some((0, false, false, true)), false, None), " (계산 중…)");
        assert_eq!(size_text(false, Some((2048, false, false, true)), false, None), format!(" ({}…)", human(2048)));
        assert_eq!(size_text(false, Some((2048, false, false, false)), false, None), format!(" ({}…)", human(2048)));
        // files only, nothing summed yet: nothing shows (no blink on a local disk)
        assert_eq!(size_text(false, Some((0, false, false, false)), false, None), "");
        assert_eq!(size_text(false, Some((0, false, false, false)), true, None), "");
        // done
        assert_eq!(size_text(false, Some((1024, true, false, true)), false, None), format!(" ({kb})"));
        assert_eq!(size_text(false, Some((1024, true, true, true)), false, None), format!(" ({kb}, partial)"));
        assert_eq!(size_text(false, Some((0, true, false, true)), false, None), format!(" ({})", human(0)));
        // something missed while collecting the selection isn't shown as a full total
        assert_eq!(size_text(false, Some((1024, true, false, true)), true, None), format!(" ({kb}, partial)"));
        assert_eq!(size_text(true, Some((1024, true, false, true)), true, None), format!(" ({kb}, 일부만)"));
        assert_eq!(size_text(false, Some((1024, true, false, false)), false, None), format!(" ({kb})"));
        assert_eq!(size_text(false, Some((1024, true, false, false)), true, None), format!(" ({kb}, partial)"));
        assert_eq!(size_text(false, Some((0, true, false, false)), false, None), "");
        assert_eq!(size_text(false, Some((0, true, true, false)), false, None), " (size unknown)");
        // no file system path at all (This PC, the bin, inside a zip): no job, done at once
        assert_eq!(size_text(false, None, true, None), " (size unknown)");
        assert_eq!(size_text(true, None, true, None), " (크기 모름)");
        assert_eq!(size_text(false, None, false, None), "");
    }

    #[test]
    fn folder_size_text_last() {
        let gb = human(5 << 30);
        // counted again: the last full count shows as is, from the first look (no folder met yet, nothing summed)
        assert_eq!(size_text(false, Some((0, false, false, false)), false, Some(5 << 30)), format!(" (last {gb} · recounting…)"));
        assert_eq!(size_text(true, Some((0, false, false, true)), false, Some(5 << 30)), format!(" (지난번 {gb} · 다시 세는 중…)"));
        // bytes so far don't replace it, even past it (the folder grew): it stays the last count
        assert_eq!(size_text(false, Some((6 << 30, false, false, true)), false, Some(5 << 30)), format!(" (last {gb} · recounting…)"));
        // done: the new count only
        assert_eq!(size_text(false, Some((1024, true, false, true)), false, Some(5 << 30)), format!(" ({})", human(1024)));
        assert_eq!(size_text(true, Some((1024, true, true, true)), false, Some(5 << 30)), format!(" ({}, 일부만)", human(1024)));
        assert_eq!(size_text(false, None, false, Some(5 << 30)), "");
    }

    #[test]
    fn size_memo_keeps_the_newest() {
        let mut m = SizeMemo::new();
        m.put(r"C:\Data\Big", 100, false);
        // any case finds it; a new count replaces it
        assert_eq!(m.get(r"c:\data\BIG"), Some((100, false)));
        m.put(r"c:\data\big", 200, true);
        assert_eq!(m.get(r"C:\Data\Big"), Some((200, true)));
        // past the cap the oldest goes; one noted again moved to the back and stays
        for k in 0..SIZE_MEMO_MAX {
            m.put(&format!(r"C:\f{k}"), k as u64, false);
            if k == 0 {
                m.put(r"C:\Data\Big", 300, false);
            }
        }
        assert_eq!(m.get(r"C:\f0"), None);
        assert_eq!(m.get(r"C:\Data\Big"), Some((300, false)));
        assert_eq!(m.get(r"C:\f1"), Some((1, false)));
        assert_eq!(m.get(&format!(r"C:\f{}", SIZE_MEMO_MAX - 1)), Some((SIZE_MEMO_MAX as u64 - 1, false)));
        // now Big is the oldest: f1 noted again and one more push it out
        m.put(r"C:\f1", 11, false);
        m.put(r"C:\new", 1, false);
        assert_eq!(m.get(r"C:\Data\Big"), None);
        assert_eq!(m.get(r"C:\f1"), Some((11, false)));
        assert_eq!(m.get(r"C:\f2"), Some((2, false)));
        m.clear();
        assert_eq!(m.get(r"C:\f1"), None);
    }

    #[test]
    fn size_memo_from_the_worker() {
        let mut m = SizeMemo::new();
        let one = [r"C:\Data".to_string()];
        // one folder counted to the end is noted
        memo_result(&mut m, &one, 42, false, true);
        assert_eq!(m.get(r"c:\data"), Some((42, false)));
        // more than one item: nothing noted, nothing dropped
        memo_result(&mut m, &[r"C:\Data".to_string(), r"C:\x".to_string()], 7, false, true);
        assert_eq!(m.get(r"C:\Data"), Some((42, false)));
        memo_result(&mut m, &[], 0, false, false);
        assert_eq!(m.get(r"C:\Data"), Some((42, false)));
        // no longer a folder (gone, a file of that name, unreadable): what was noted goes
        memo_result(&mut m, &one, 0, true, false);
        assert_eq!(m.get(r"C:\Data"), None);
    }

    #[test]
    fn free_space_volume() {
        assert_eq!(vol_key(r"C:\Users\Public").as_deref(), Some("c:"));
        assert_eq!(vol_key(r"c:\").as_deref(), Some("c:"));
        assert_eq!(vol_key(r"\\NAS\Share\docs").as_deref(), Some(r"\\nas\share"));
        assert_eq!(vol_key(r"\\nas\share"), vol_key(r"\\NAS\SHARE\x\y"));
        assert_ne!(vol_key(r"\\nas\a\x"), vol_key(r"\\nas\b\x"));
        assert_eq!(vol_key("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"), None);
        assert_eq!(vol_key(""), None);
    }

    // Status line zones: the middle zone gives way first, then the items zone (down to min_head), then the
    // drive loses its bar, then its total; the note chip stays in front, at most half the line.
    #[test]
    fn status_line_fit() {
        let fit = |right, note, mid| status_fit(0, right, 12, 60, note, 200, mid, &[250, 180, 120]);
        // wide: everything at full width, the drive at the right end with its bar
        let f = fit(1000, Some(100), Some(150));
        assert_eq!(f, StatusFit { note: Some((0, 100)), head: (112, 312), mid: Some((324, 474)), drive: Some((750, 1000, 0)) });
        // narrower: the middle zone goes, the items zone is cut, the bar stays
        let f = fit(500, Some(100), Some(150));
        assert_eq!(f, StatusFit { note: Some((0, 100)), head: (112, 238), mid: None, drive: Some((250, 500, 0)) });
        // a middle zone cut short but still readable
        assert_eq!(fit(562, None, Some(150)).mid, Some((212, 300)));
        // narrow: no bar
        let f = fit(300, None, Some(150));
        assert_eq!(f, StatusFit { note: None, head: (0, 108), mid: None, drive: Some((120, 300, 1)) });
        // very narrow: the short form, cut to half of the room
        let f = fit(150, None, None);
        assert_eq!(f, StatusFit { note: None, head: (0, 75), mid: None, drive: Some((87, 150, 2)) });
        // nothing left: no drive, nothing negative
        let f = fit(10, None, Some(50));
        assert_eq!(f, StatusFit { note: None, head: (0, 10), mid: None, drive: None });
        // a long note (a missing folder's path) takes what the items and the full drive zone leave over
        let f = fit(1000, Some(900), None);
        assert_eq!(f, StatusFit { note: Some((0, 526)), head: (538, 738), mid: None, drive: Some((750, 1000, 0)) });
        // ... but at least half the line when they need more
        assert_eq!(fit(600, Some(900), None).note, Some((0, 300)));
        // no drive zone (copying, a shell place): the items zone runs to the end
        assert_eq!(status_fit(0, 1000, 12, 60, None, 2000, Some(100), &[]), StatusFit { note: None, head: (0, 1000), mid: None, drive: None });
        // a short items text never pushes the drive into a smaller form
        assert_eq!(status_fit(0, 300, 12, 60, None, 40, None, &[248]).drive, Some((52, 300, 0)));
    }

    #[test]
    fn status_line_for_screen_readers() {
        let st = StatusLine {
            note: Some("No match: *.tmp".into()),
            head: format!("12 items{ZONE_DOT}3 selected"),
            mid: vec!["Filter pdf".into()],
            drive: Some(("C:".into(), 1 << 30, 4 << 30)),
        };
        let words = status_speech(&st);
        assert!(words.starts_with("No match: *.tmp, 12 items, 3 selected, Filter pdf, C: "), "{words}"); // the note is heard first
        assert!(words.contains("1.0 GB") && words.contains("4.0 GB") && !words.contains('·'), "{words}");
        // no note, nothing in the middle, no drive: no empty pieces
        let st = StatusLine { head: "0 items".into(), ..Default::default() };
        assert_eq!(status_speech(&st), "0 items");
    }

    #[test]
    fn drive_usage_bar() {
        assert_eq!(drive_use(500, 0), None); // no total: no bar
        assert_eq!(drive_use(0, 0), None);
        assert_eq!(drive_use(101, 1000), Some((0.899, false))); // 89.9% used
        assert_eq!(drive_use(100, 1000), Some((0.9, true))); // 90%
        assert_eq!(drive_use(0, 1000), Some((1.0, true)));
        assert_eq!(drive_use(1000, 1000), Some((0.0, false)));
        assert_eq!(drive_use(2000, 1000), Some((0.0, false))); // more free than total (quota): empty, not negative
        let tb = 1u64 << 40;
        assert_eq!(drive_use(tb / 10, tb).map(|u| u.1), Some(true));
        assert_eq!(drive_use(tb / 10 + 1, tb).map(|u| u.1), Some(false));
        // the bar reads on its track in every theme and colour; a full drive is the theme's red
        for p in [&LIGHT, &DARK] {
            for (name, c) in p.accents {
                assert!(contrast(bar_fill(p, c, false), p.line) >= 1.5, "{name}");
                assert_eq!(bar_fill(p, c, true), p.warn);
            }
            assert_eq!(bar_fill(p, p.line, false), p.sub); // a custom colour equal to the track
            assert_eq!(bar_fill(p, p.warn, false), p.sub); // a custom colour equal to the warning red
        }
    }

    // Every text-on-fill pair painted from a palette, in CONTRAST_FLOOR's order.
    fn contrast_pairs(p: &Palette) -> Vec<(String, f64)> {
        let mut v = vec![];
        let mut add = |what: String, fg: u32, bg: u32| v.push((what, contrast(fg, bg)));
        for (f, fname) in [(p.text, "text"), (p.sub, "sub")] {
            for (b, bname) in [(p.bg, "bg"), (p.card, "card"), (p.pill, "pill")] {
                add(format!("{fname} on {bname}"), f, b);
            }
        }
        // selection colours (active header, colour label number, filter badge): plain, then hot/pressed header button
        for (name, c) in p.accents {
            let ink = on_color(c);
            add(name.to_string(), ink, c);
            add(format!("{name} hot"), ink, blend(c, ink, press_tint(true, false)));
            add(format!("{name} pressed"), ink, blend(c, ink, press_tint(true, true)));
        }
        // uncoloured buttons: toolbar on bg, inactive header on card
        for (b, bname) in [(p.bg, "bg"), (p.card, "card")] {
            add(format!("text on {bname} hot"), p.text, blend(b, p.text, press_tint(false, false)));
            add(format!("text on {bname} pressed"), p.text, blend(b, p.text, press_tint(false, true)));
        }
        // a filled ★ in an inactive header stays sub, also while hot/pressed
        add("star".into(), p.sub, p.card);
        add("star hot".into(), p.sub, blend(p.card, p.text, press_tint(false, false)));
        add("star pressed".into(), p.sub, blend(p.card, p.text, press_tint(false, true)));
        // a nearly full drive's bar on its track, and next to it on the status line
        add("warn on line".into(), p.warn, p.line);
        add("warn on bg".into(), p.warn, p.bg);
        v
    }

    // Today's contrast of each pair, rounded down. A change may raise these; lowering one means fixing the
    // colour or saying in review why that pair may get harder to read, then lowering just that value.
    #[rustfmt::skip]
    const CONTRAST_FLOOR: [[f64; 33]; 2] = [
        [
            13.02, 14.69, 13.14, 3.93, 4.44, 3.97, // text, sub on bg/card/pill
            6.93, 5.18, 4.18, // Sky: plain, hot, pressed
            8.51, 6.24, 4.97, // Gray
            5.91, 4.46, 3.68, // Lavender
            7.46, 5.53, 4.44, // Sage
            7.79, 5.76, 4.61, // Peach
            7.11, 5.27, 4.26, // Rose
            11.46, 10.22, 12.89, 11.45, // text on bg hot/pressed, card hot/pressed
            4.44, 3.89, 3.46, // star: plain, hot, pressed
            4.3, 5.0, // drive bar red on line, on bg
        ],
        [
            13.72, 12.02, 10.32, 6.39, 5.60, 4.81,
            5.06, 3.55, 2.85,
            8.72, 5.32, 3.89,
            5.76, 3.90, 3.06,
            4.30, 3.15, 2.60,
            4.65, 3.36, 2.74,
            4.82, 3.45, 2.78,
            11.53, 9.69, 10.00, 8.33,
            5.60, 4.66, 3.88,
            4.1, 6.0,
        ],
    ];

    #[test]
    fn palette_contrast() {
        for ((theme, p), floor) in [("light", &LIGHT), ("dark", &DARK)].into_iter().zip(&CONTRAST_FLOOR) {
            let got = contrast_pairs(p);
            assert_eq!(got.len(), floor.len());
            for ((what, c), min) in got.iter().zip(floor) {
                assert!(c >= min, "{theme} {what}: contrast {c:.2} < {min}");
            }
            // on_color picks whichever of INK and white reads better
            for (name, c) in p.accents {
                let ink = on_color(c);
                let other = if ink == INK { 0xFFFFFF } else { INK };
                assert!(contrast(ink, c) >= contrast(other, c), "{theme} {name}");
            }
        }
        assert_eq!(blend(0x123456, 0xABCDEF, 0.0), 0x123456);
        assert_eq!(blend(0x123456, 0xABCDEF, 1.0), 0xABCDEF);
    }

    // While another app has the keyboard the active header takes a quiet grey: its ink still reads, it still
    // stands apart from an inactive header (card), and it differs from the selection colour (Gray included).
    #[test]
    fn quiet_accent_reads() {
        for (theme, p) in [("light", &LIGHT), ("dark", &DARK)] {
            for (name, c) in p.accents {
                let q = quiet_accent(c, p.card);
                assert!(contrast(on_color(q), q) >= 4.5, "{theme} {name}: ink {:.2}", contrast(on_color(q), q));
                assert!(contrast(q, p.card) >= 1.2, "{theme} {name}: card {:.2}", contrast(q, p.card));
                assert_ne!(q, c, "{theme} {name}");
            }
        }
    }

    // A pane's faint ← / → / ↑ reads less than a usable one, on an inactive header (sub on card) and on every
    // selection colour's header (its ink), but still at 1.5:1 or better.
    #[test]
    fn faint_nav_reads() {
        for (theme, p) in [("light", &LIGHT), ("dark", &DARK)] {
            let heads = std::iter::once(("card", p.card, p.sub)).chain(p.accents.iter().map(|&(n, c)| (n, c, on_color(c))));
            for (name, bg, fg) in heads {
                let (faint, usual) = (contrast(faint_ink(bg, fg), bg), contrast(fg, bg));
                assert!(faint >= 1.5 && faint < usual, "{theme} {name}: faint {faint:.2}, usual {usual:.2}");
            }
        }
    }

    // Steps back / forward Grids counts for the faint buttons, and which buttons go faint.
    #[test]
    fn nav_dim_counts() {
        // the first folder: nothing either way, whatever the kind
        assert_eq!(hist_step(None, Go::New), (0, 0));
        assert_eq!(hist_step(None, Go::Back), (0, 0));
        assert_eq!(nav_dim(None, false), [true, true, false]);
        // a new folder: one more back, forward gone
        assert_eq!(hist_step(Some((0, 0)), Go::New), (1, 0));
        assert_eq!(hist_step(Some((2, 3)), Go::New), (3, 0));
        // back and forward trade a step; never below 0 when the count was short
        assert_eq!(hist_step(Some((1, 0)), Go::Back), (0, 1));
        assert_eq!(hist_step(Some((0, 1)), Go::Fwd), (1, 0));
        assert_eq!(hist_step(Some((0, 0)), Go::Back), (0, 1));
        assert_eq!(hist_step(Some((2, 0)), Go::Fwd), (3, 0));
        // refused back / forward: none that way; accepted or another kind: unchanged
        assert_eq!(hist_refused(3, 2, Go::Back, false), (0, 2));
        assert_eq!(hist_refused(3, 2, Go::Fwd, false), (3, 0));
        assert_eq!(hist_refused(3, 2, Go::Back, true), (3, 2));
        assert_eq!(hist_refused(3, 2, Go::New, false), (3, 2));
        // 0 = faint, 1 or more = usual; ↑ faint at the desktop only
        assert_eq!(nav_dim(Some((0, 0)), false), [true, true, false]);
        assert_eq!(nav_dim(Some((1, 0)), false), [false, true, false]);
        assert_eq!(nav_dim(Some((0, 2)), true), [true, false, true]);
    }

    // Windows' contrast themes (white and black ground among them): every text in Grids' own chrome reads
    // at 4.5:1 or better, and every selection colour is the theme's highlight.
    #[test]
    fn high_contrast_palette() {
        let sys = |face, face_text, window, window_text, gray, hl, hl_text| SysColors { face, face_text, window, window_text, gray, hl, hl_text };
        let themes = [
            ("aquatic", sys(0x202020, 0xFFFFFF, 0x202020, 0xFFFFFF, 0xA6A6A6, 0x8EE3F0, 0x263B50)),
            ("desert", sys(0xFFFAEF, 0x3D3D3D, 0xFFFAEF, 0x3D3D3D, 0x676767, 0x903909, 0xFFF5E3)),
            ("dusk", sys(0x2D3236, 0xB6F6F0, 0x2D3236, 0xB6F6F0, 0xA6A6A6, 0xA1BFDE, 0x212D3B)),
            ("night sky", sys(0x000000, 0xFFFFFF, 0x000000, 0xFFFFFF, 0xA6A6A6, 0xD6B4FD, 0x2B2B2B)),
            ("hc black", sys(0x000000, 0xFFFFFF, 0x000000, 0xFFFFFF, 0x3FF23F, 0x1AEBFF, 0x000000)),
            ("hc white", sys(0xFFFFFF, 0x000000, 0xFFFFFF, 0x000000, 0x600000, 0x37006E, 0xFFFFFF)),
            // made up: a face whose own text differs, and a grey too faint to read
            ("odd", sys(0x404040, 0xFFFFFF, 0xFFFFFF, 0x000000, 0xC8C8C8, 0x0050A0, 0xFFFFFF)),
        ];
        for (name, c) in themes {
            let p = hc_palette(&c);
            for f in [p.text, p.sub] {
                for b in [p.bg, p.card, p.pill] {
                    assert!(contrast(f, b) >= 4.5, "{name}: {f:06X} on {b:06X}");
                }
            }
            assert!(p.accents.iter().all(|&(_, a)| a == c.hl), "{name}");
            // the drive bar: a full drive (the highlight) still differs from the usual fill, which reads on its track
            let usual = bar_fill(&p, c.hl, false);
            assert!(usual != bar_fill(&p, c.hl, true) && contrast(usual, p.line) >= 1.5, "{name}: {usual:06X}");
            assert_eq!(p.accents.map(|(n, _)| n), LIGHT.accents.map(|(n, _)| n));
        }
        let odd = hc_palette(&themes[6].1);
        assert_eq!((odd.bg, odd.sub), (0xFFFFFF, 0x000000)); // the window ground and its text instead
        let desert = hc_palette(&themes[1].1);
        assert_eq!((desert.bg, desert.sub), (0xFFFAEF, 0x676767));
    }

    // High contrast shows Windows' highlight but never writes it (or a theme) into grids.ini.
    #[test]
    fn high_contrast_keeps_saved_colours() {
        for dark in [false, true] {
            let s = Session { dark, accent: 0x8DB5E8, accent_dark: 0x4C6FA8, ..Default::default() };
            let before = s.to_ini();
            assert_eq!(s.accent_in(Some(0x1AEBFF)), 0x1AEBFF);
            assert_eq!(s.accent_in(None), if dark { 0x4C6FA8 } else { 0x8DB5E8 });
            assert_eq!(s.accent_saved(), s.accent_in(None));
            assert_eq!(s.to_ini(), before);
        }
    }

    // Windows' accent colour: a shade per theme near the Sky preset whose text reads; high contrast still wins.
    #[test]
    fn windows_accent_shades() {
        // Windows 10's default blue, R G B A per colour; the 8th (orange) is never a candidate
        let blue = [0xA6, 0xD8, 0xFF, 0, 0x76, 0xB9, 0xED, 0, 0x42, 0x9C, 0xE3, 0, 0x00, 0x78, 0xD7, 0, 0x00, 0x5A, 0x9E, 0, 0x00, 0x42, 0x75, 0, 0x00, 0x26, 0x42, 0, 0xF7, 0x63, 0x0C, 0];
        let shades = accent_palette(&blue).unwrap();
        assert_eq!(shades, [0xA6D8FF, 0x76B9ED, 0x429CE3, 0x0078D7, 0x005A9E, 0x004275, 0x002642]);
        assert_eq!(accent_palette(&blue[..28]), None);
        let mut long = blue.to_vec();
        long.extend([0; 4]);
        assert_eq!(accent_palette(&long), None);
        // a yellow and a green palette too (yellows are light, their middle shades read with neither ink)
        let yellow = [0xFFF4B0, 0xFFE680, 0xFFD43B, 0xE6B800, 0xB38F00, 0x806600, 0x4D3D00];
        let green = [0x9EF0B5, 0x6ADB8C, 0x3CC46A, 0x16A34A, 0x0F7A37, 0x0A5527, 0x053317];
        for p in [&shades[..], &yellow, &green, &shades_of(0x0078D7), &shades_of(0xFFB900)] {
            for dark in [false, true] {
                let c = sys_accent_shade(p, dark).unwrap();
                assert!(p.contains(&c));
                assert!(contrast(on_color(c), c) >= 4.5, "{c:06X} dark={dark}");
                // the nearest legible shade to Sky in lightness
                let sky = rel_lum(theme_pal(dark).accents[0].1);
                let gap = |x: u32| (rel_lum(x) - sky).abs();
                assert!(p.iter().filter(|&&x| contrast(on_color(x), x) >= 4.5).all(|&x| gap(c) <= gap(x)));
            }
        }
        assert_eq!(sys_accent_shade(&shades, false), Some(0x76B9ED));
        assert_eq!(sys_accent_shade(&shades, true), Some(0x005A9E));
        assert_eq!(sys_accent_shade(&[], false), None);
        // none legible: the most legible one
        assert_eq!(sys_accent_shade(&[0x808080, 0x8A8A8A], false), Some(0x808080));
        assert_eq!(shades_of(0x0078D7)[3], 0x0078D7);
        // high contrast, then Windows' accent, then the saved colour; turning it off brings the saved one back
        assert_eq!(accent_pick(Some(0x1AEBFF), Some(0x76B9ED), 0x8DB5E8), 0x1AEBFF);
        assert_eq!(accent_pick(None, Some(0x76B9ED), 0x8DB5E8), 0x76B9ED);
        assert_eq!(accent_pick(None, Some(0x000000), 0x8DB5E8), 0x000000); // black is a colour
        assert_eq!(accent_pick(None, None, 0x8DB5E8), 0x8DB5E8);
        // saved as accent_sys=, read back; only 1 is on, an older file is off
        let s = Session { accent_sys: true, ..Session::default() };
        assert!(Session::parse(&s.to_ini()).accent_sys);
        for v in ["accent_sys=0\n", "accent_sys=\n", "accent_sys=yes\n", "dark=1\n"] {
            assert!(!Session::parse(v).accent_sys, "{v:?}");
        }
        // a session file carries it, but loading one keeps the app's own setting
        let mut cur = Session::default();
        cur.take_layout(&Session::parse(&s.snapshot()));
        assert!(!cur.accent_sys);
    }

    #[test]
    fn accent_names_line_up() {
        let want = [("Sky", "하늘"), ("Gray", "그레이"), ("Lavender", "라벤더"), ("Sage", "세이지"), ("Peach", "피치"), ("Rose", "로즈")];
        for (k, (en, ko)) in want.into_iter().enumerate() {
            assert_eq!((ACCENT_EN[k], LIGHT.accents[k].0, DARK.accents[k].0), (en, ko, ko));
        }
        // the view menu's range ends at the literal 1405, below Custom...
        assert_eq!(ID_ACCENT + want.len() - 1, 1405);
        assert!(1405 < ID_ACCENT_PICK);
        let s = Session::default();
        assert_eq!((s.accent, s.accent_dark), (LIGHT.accents[1].1, DARK.accents[1].1));
        assert_eq!(LIGHT.accents[1].0, "그레이");
    }

    #[test]
    fn session_card_age() {
        let cases = [
            (0, "just now", "방금"),
            (1, "1 min ago", "1분 전"),
            (59, "59 min ago", "59분 전"),
            (60, "1 h ago", "1시간 전"),
            (1439, "23 h ago", "23시간 전"),
            (1440, "1 d ago", "1일 전"),
            (10080, "7 d ago", "7일 전"),
        ];
        for (m, en, ko) in cases {
            assert_eq!((ago_text(m, false).as_str(), ago_text(m, true).as_str()), (en, ko), "{m} min");
        }
    }

    #[test]
    fn folder_size_collect() {
        let root = tree("collect", &[("f.bin".into(), 7), (r"d\x.bin".into(), 1)]);
        let path = |p: &str| root.join(p).to_string_lossy().into_owned();
        let (f, d, gone) = (path("f.bin"), path("d"), path("gone"));
        // the UI thread only gathers paths: a gone one is kept for the worker, nothing is read here
        let none = Sel { paths: vec![], partial: true };
        assert_eq!(collect_sel(None, |_| None), none); // GetSelection / GetCount failed
        assert_eq!(collect_sel(Some(1), |_| None), none); // GetItemAt / no file system path
        assert_eq!(collect_sel(Some(1), |_| Some(gone.clone())), Sel { paths: vec![gone.clone()], partial: false });
        let both = [f.clone(), d.clone()];
        assert_eq!(collect_sel(Some(2), |k| Some(both[k as usize].clone())), Sel { paths: both.to_vec(), partial: false });
        let calls = std::cell::Cell::new(0);
        let s = collect_sel(Some(10_001), |_| {
            calls.set(calls.get() + 1);
            Some(f.clone())
        });
        assert_eq!((s.paths.len(), s.partial, calls.get()), (10_000, true, 10_000));

        // the worker sorts them: a file by its size, a folder walked, a root or a gone path partial
        assert_eq!(sel_kind(&f), SelKind::File(7));
        assert_eq!(sel_kind(&d), SelKind::Dir);
        assert_eq!(sel_kind(r"C:\"), SelKind::Unknown); // a drive root isn't walked
        assert_eq!(sel_kind(&gone), SelKind::Unknown);
        let never = || false;
        let mut quiet = |_, _, _| {};
        let sum = |t: &[String], quiet: &mut dyn FnMut(u64, bool, bool)| sel_bytes(t, &never, &never, quiet);
        assert_eq!(sum(&[f.clone()], &mut quiet), Some((7, false, false)));
        assert_eq!(sum(&both, &mut quiet), Some((8, false, true)));
        assert_eq!(sum(&[f.clone(), gone.clone()], &mut quiet), Some((7, true, false)));
        assert_eq!(sum(&[r"C:\".into()], &mut quiet), Some((0, true, false)));
        // many files: progress comes with the running total, and a new selection stops the old sum
        let files = vec![f.clone(); 2100];
        let mut last = 0;
        assert_eq!(sum(&files, &mut |b, _, _| last = b), Some((14_700, false, false)));
        assert!(last >= 14_000);
        let n = std::cell::Cell::new(0);
        let later = || {
            n.set(n.get() + 1);
            n.get() > 5
        };
        assert_eq!(sel_bytes(&files, &later, &never, &mut quiet), None);
        assert!(n.get() < 10);
        // a copy only pauses it; a cancel still gets through while paused
        let seen = std::cell::Cell::new(0);
        let copying = || {
            seen.set(seen.get() + 1);
            (10..13).contains(&seen.get())
        };
        assert_eq!(sel_bytes(&files[..50], &never, &copying, &mut quiet), Some((350, false, false)));
        let held = std::cell::Cell::new(0);
        let copy_on = || {
            held.set(held.get() + 1);
            held.get() > 5
        };
        let cancel = || held.get() > 7;
        assert_eq!(sel_bytes(&files, &cancel, &copy_on, &mut quiet), None);
        _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn shortcut_rows() {
        let (en, ko) = (key_rows(KEYS_EN), key_rows(KEYS_KO));
        assert_eq!([en[0].len(), en[1].len()], [ko[0].len(), ko[1].len()]);
        assert!(en[0].len() >= 10 && en[1].len() >= 5);
        for r in en.iter().chain(&ko).flatten() {
            assert!(!r.desc.is_empty() && !r.keys.is_empty() && r.keys.iter().all(|k| !k.is_empty()));
        }
        let caps = |k: &str| en[0].iter().find(|r| r.keys.join(", ").starts_with(k)).map(|r| r.keys.len());
        assert_eq!([caps("Ctrl+L"), caps("F2"), caps("Ctrl+1"), caps("Ctrl+/")], [Some(3), Some(4), Some(1), Some(1)]);
        assert_eq!(en[0].iter().find(|r| r.keys[0] == "F2").unwrap().keys, ["F2", "Del", "Ctrl+C/X/V", "F5"]);
        // the last mouse line has no "name: " and continues the one above
        assert!(en[1].last().unwrap().desc.ends_with("If Grids is already running, they open in that window."));
        assert!(ko[1].last().unwrap().desc.ends_with("이미 실행 중이면 그 창에서 엽니다."));
    }

    // Folder menus' icons: the shell place's own → seen this run (any case) → drive kind → known folder → plain folder.
    #[test]
    fn menu_icon_order() {
        assert_eq!(menu_icon_pick(7, Some(3), 4, 5, 6), 7); // the bin as its button shows it now, not as visited
        assert_eq!(menu_icon_pick(-1, Some(3), 4, 5, 6), 3);
        assert_eq!(menu_icon_pick(-1, None, 4, 5, 6), 4);
        assert_eq!(menu_icon_pick(-1, Some(-1), -1, 5, 6), 5);
        assert_eq!(menu_icon_pick(-1, None, -1, -1, 6), 6);
        assert_eq!(menu_icon_pick(-1, None, -1, -1, -1), -1); // text alone
        let mut seen = IconMemo::default();
        seen.put(r"C:\Users\Me\Downloads", 42);
        assert_eq!(menu_icon_pick(-1, seen.get(r"c:\users\me\DOWNLOADS"), -1, 5, 6), 42);
        assert_eq!(menu_icon_pick(-1, seen.get(r"C:\Users\Me\Music"), -1, 5, 6), 5);
    }

    // A known folder's registered icon location → (file, index), %NAME% expanded, unknown names left as they are.
    #[test]
    fn icon_locations() {
        let var = |v: &str| (v.eq_ignore_ascii_case("SystemRoot")).then(|| r"C:\Windows".to_string());
        let sys = r"C:\Windows\system32\imageres.dll";
        assert_eq!(icon_location(r"%SystemRoot%\system32\imageres.dll,-184", var), Some((sys.into(), -184)));
        assert_eq!(icon_location(r#""%SystemRoot%\system32\imageres.dll", -183"#, var), Some((sys.into(), -183)));
        assert_eq!(icon_location(r"%SystemRoot%\system32\imageres.dll", var), Some((sys.into(), 0)));
        assert_eq!(icon_location(r"imageres.dll,3", var), Some(("imageres.dll".into(), 3)));
        assert_eq!(icon_location(r"%NOPE%\a,b.dll", var), Some((r"%NOPE%\a,b.dll".into(), 0)));
        assert_eq!(icon_location(r"50%%\x.dll,1", var), Some((r"50%%\x.dll".into(), 1)));
        assert_eq!(icon_location("", var), None);
        assert_eq!(icon_location(r#""" ,-1"#, var), None);
    }

    // A menu icon: centred on an s(16) square at every scale, and its DIB holds the premultiplied pixels unchanged.
    #[test]
    fn menu_icon_bitmaps() {
        for dpi in [72, 96, 144, 192] {
            let px = MenuIcons::new(dpi).px;
            assert_eq!(px, 16 * dpi / 96);
            let small = Pix { w: px / 2, h: px / 4, bgra: vec![200; (px / 2 * (px / 4) * 4) as usize] };
            let c = menu_canvas(&small, px).expect("fits");
            assert_eq!((c.w, c.h, c.bgra.len()), (px, px, (px * px * 4) as usize));
            let at = |x: i32, y: i32| c.bgra[((y * px + x) * 4) as usize];
            assert_eq!((at(0, 0), at(px / 4, px * 3 / 8), at(px - 1, px - 1)), (0, 200, 0)); // in the middle
            assert!(menu_canvas(&Pix { w: px + 1, h: px, bgra: vec![0; ((px + 1) * px * 4) as usize] }, px).is_none());
            let mut p = Pix { w: px, h: px, bgra: (0..px * px * 4).map(|k| (k * 7 % 256) as u8).collect() };
            p.bgra.chunks_exact_mut(4).for_each(|q| q[3] = q[0].max(q[1]).max(q[2])); // premultiplied
            unsafe {
                let b = pix_bitmap(&p).expect("DIB");
                let back = bitmap_raw(b);
                _ = DeleteObject(b.into());
                let back = back.expect("read back");
                assert_eq!((back.w, back.h), (px, px));
                assert!(back.bgra == p.bgra, "{dpi}");
            }
        }
    }

    // Ctrl+G's menu: one underlined letter per row, the same rows and letters in both languages, and Ctrl+letter
    // (its control code, as WM_MENUCHAR gets it) picking the row the letter does.
    #[test]
    fn go_menu_letters() {
        let (en, ko) = (go_targets(false), go_targets(true));
        assert_eq!(en.len(), ko.len());
        let letters = |rows: &[Option<(&str, GoTarget)>]| rows.iter().map(|r| r.map(|(l, _)| go_letter(l).unwrap())).collect::<Vec<_>>();
        assert_eq!(letters(&en), letters(&ko));
        for (e, k) in en.iter().zip(&ko) {
            assert_eq!(e.map(|r| r.1), k.map(|r| r.1));
        }
        let mut seen: Vec<char> = letters(&en).into_iter().flatten().collect();
        let n = seen.len();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), n, "a letter is used twice");
        assert!(!seen.contains(&'G')); // Ctrl+G held: its repeats pick nothing
        assert_eq!(go_letter("Down&loads"), Some('L'));
        let at = |rows: &[Option<(&str, GoTarget)>], ch: u16| go_pick_char(rows, ch, true).map(|k| go_letter(rows[k].unwrap().0).unwrap());
        for rows in [&en, &ko] {
            assert_eq!((at(rows, 0x0C), at(rows, 'L' as u16), at(rows, 'l' as u16)), (Some('L'), Some('L'), Some('L')));
            assert_eq!((at(rows, 0x08), at(rows, 0x01), at(rows, 0x14)), (Some('H'), Some('A'), Some('T')));
            assert_eq!((at(rows, 0x07), at(rows, 'Z' as u16), at(rows, '1' as u16)), (None, None, None));
            assert!(go_pick_char(rows, 0x0C, true).is_some_and(|k| rows[k].is_some_and(|r| r.1 == GoTarget::Known(&FOLDERID_Downloads))));
            // without Ctrl, Backspace (0x08) and Tab (0x09) are not Ctrl+H / Ctrl+I; letters still pick
            assert_eq!((go_pick_char(rows, 0x08, false), go_pick_char(rows, 0x09, false)), (None, None));
            assert!(go_pick_char(rows, 'l' as u16, false).is_some());
        }
        let places: Vec<GoTarget> = en.iter().flatten().map(|r| r.1).collect();
        assert!(places.contains(&GoTarget::Place(THIS_PC)) && places.contains(&GoTarget::Place(RECYCLE_BIN)));
        // F (Ctrl+F) is the recent files row, the second menu
        for rows in [&en, &ko] {
            assert!(go_pick_char(rows, 0x06, true).is_some_and(|k| rows[k].is_some_and(|r| r.1 == GoTarget::RecentFiles)));
            assert_eq!(at(rows, 'f' as u16), Some('F'));
        }
    }

    // Ctrl+G, F: Recent Items' shortcuts read newest first; folders, paths without a folder and repeats left out.
    #[test]
    fn recent_files_list() {
        let v = vec![("old", 5u64), ("new", 9), ("mid", 7), ("older", 1)];
        assert_eq!(newest_first(v.clone(), 3), vec!["new", "mid", "old"]);
        assert_eq!(newest_first(v, 10).len(), 4);
        let t = |p: &str, dir: bool| (p.to_string(), dir);
        let got = recent_keep(
            vec![
                t(r"C:\Docs\a.docx", false),
                t(r"C:\Docs", true),
                t(r"c:\docs\A.DOCX", false),
                t("", false),
                t("nofolder.txt", false),
                t(r"D:\b.pdf", false),
                t(r"\\srv\share\c.hwp", false),
                t(r"C:\e.txt", false),
            ],
            3,
        );
        assert_eq!(got, vec![r"C:\Docs\a.docx", r"D:\b.pdf", r"\\srv\share\c.hwp"]);
        assert_eq!(split_file(r"D:\b.pdf"), Some((r"D:\".into(), "b.pdf".into())));
        assert_eq!(split_file(r"C:\Docs\a.docx"), Some((r"C:\Docs".into(), "a.docx".into())));
        assert_eq!((split_file(r"C:\Docs\"), split_file("a.txt")), (None, None));
        assert_eq!(recent_label(r"C:\Work\R&D\plan.docx").as_deref(), Some("plan.docx\tR&&D"));
        assert_eq!(recent_label(r"D:\b.pdf").as_deref(), Some("b.pdf\tD:\\"));
        assert_eq!(mid_cut("abcdefghij", 5), "ab…ij");
        assert_eq!(mid_cut("가나다라마바", 4), "가나…바");
        assert_eq!(mid_cut("short", 60), "short");
    }

    // The shortcut window is the in-app manual: mouse-only actions (menus on right-click, drops, the session
    // card's ✕) must stay described there in both languages, not only the keys the README tables check.
    // A Korean row is taken at the English row's index, as shortcut_rows and the find box pair them.
    #[test]
    fn mouse_help_lists_mouse_actions() {
        let (en, ko) = (key_rows(KEYS_EN), key_rows(KEYS_KO));
        let check = |sec: usize, pick: &dyn Fn(&KeyRow) -> bool, en_words: &[&str], ko_words: &[&str]| {
            let i = en[sec].iter().position(pick).expect("row not found");
            let (e, k) = (&en[sec][i], &ko[sec][i]);
            for w in en_words {
                assert!(e.desc.contains(w), "{:?} lacks {w:?}: {}", e.keys, e.desc);
            }
            for w in ko_words {
                assert!(k.desc.contains(w), "{:?} lacks {w:?}: {}", k.keys, k.desc);
            }
        };
        let name = |n: &'static str| move |r: &KeyRow| r.keys[0].starts_with(n);
        check(1, &name("Tabs"), &["right-click", "move left / right"], &["오른쪽 클릭", "옮기기"]);
        check(1, &name("Drive buttons"), &["right-click", "Eject"], &["오른쪽 클릭", "꺼내기"]);
        check(1, &name("Load"), &["✕", "rename", "shortcut"], &["✕", "이름 바꾸기", "바로 가기"]);
        check(1, &name("Pane header"), &["right-click", "double-click", "drop a folder"], &["오른쪽 클릭", "더블클릭", "끌어다 놓"]);
        let key = |k: &'static str| move |r: &KeyRow| r.keys[0] == k;
        check(0, &key("Alt+↓"), &["right-click"], &["오른쪽 클릭"]);
        check(0, &key("F3"), &["Enter", "online-only"], &["Enter", "온라인 전용"]);
        // a mouse row's name ends at its first ": ", so a ": " inside its brackets cuts it ("Drive buttons (C: D: …)" did)
        for r in en[1].iter().chain(&ko[1]) {
            assert_eq!(r.keys[0].matches('(').count(), r.keys[0].matches(')').count(), "name cut short: {:?}", r.keys);
        }
    }

    // One row's keycaps as "context:key", context "" (anywhere), "filter" or "preview", so that losing
    // the filter's Alt+↓ or the preview's F2 / Del shows even while the plain one is still listed.
    // Keys break at ", " and " / " as key_rows does; a bare arrow keeps the modifiers before it
    // ("Alt+Shift+← / →" is Alt+Shift+→) so the two pane-arrow rows stay apart.
    fn doc_caps(ctx: &str, keys: &str) -> Vec<String> {
        let keys = keys.replace("Ctrl+C / X / V", "Ctrl+C/X/V").replace('휠', "wheel");
        let mut caps: Vec<String> = Vec::new();
        for k in keys.split(", ").flat_map(|p| p.split(" / ")) {
            let k = match caps.last() {
                Some(prev) if ["←", "→", "↑", "↓"].contains(&k) => format!("{}{k}", &prev[..prev.rfind('+').map_or(0, |i| i + 1)]),
                _ => k.to_string(),
            };
            caps.push(k);
        }
        caps.into_iter().map(|k| format!("{ctx}:{k}")).collect()
    }

    // The in-app list (KEYS_EN / KEYS_KO): the keyboard section, plus the Save button's line in the
    // mouse & buttons section, "Save (Ctrl+S)" / "저장 (Ctrl+S)", which the READMEs list as a key.
    fn app_key_rows(text: &str) -> Vec<Vec<String>> {
        let [kb, mouse] = key_rows(text);
        let mut rows: Vec<Vec<String>> = kb
            .iter()
            .map(|r| {
                let k = r.keys.join(" / ");
                let ctx = if r.desc.starts_with("In the preview:") || r.desc.starts_with("미리보기에서:") {
                    "preview"
                } else if k.contains(" in the filter") || k.contains("필터 칸에서 ") {
                    "filter"
                } else {
                    ""
                };
                doc_caps(ctx, &k.replace(" in the filter", "").replace("필터 칸에서 ", ""))
            })
            .collect();
        rows.extend(mouse.iter().filter_map(|r| r.keys[0].split_once(" (Ctrl+")).map(|(_, k)| doc_caps("", &format!("Ctrl+{}", k.trim_end_matches(')')))));
        rows
    }

    // The first column of the table under `heading`, "(preview)" / "(in the filter)" tails as the context.
    fn readme_key_rows(md: &str, heading: &str) -> Vec<Vec<String>> {
        let table = md.split_once(heading).unwrap_or_else(|| panic!("no {heading:?} heading")).1;
        let tails = [(" (in the filter)", "filter"), (" (필터 칸에서)", "filter"), (" (preview)", "preview"), (" (미리보기)", "preview")];
        let rows = table.lines().skip_while(|l| !l.starts_with('|')).skip(2).take_while(|l| l.starts_with('|'));
        rows.map(|l| {
            let k = l[1..].split(" | ").next().unwrap().trim();
            let (k, ctx) = tails.iter().find_map(|(tail, ctx)| k.strip_suffix(tail).map(|k| (k, *ctx))).unwrap_or((k, ""));
            doc_caps(ctx, k)
        })
        .collect()
    }

    // The keys in the app's shortcut window (English and Korean) and in the README.md / README.ko.md
    // tables must agree, so a new shortcut can't ship missing from one of the four.
    #[test]
    fn shortcut_docs_match() {
        let same_rows = |what: &str, a: &[Vec<String>], b: &[Vec<String>]| {
            if let Some(i) = (0..a.len().max(b.len())).find(|&i| a.get(i) != b.get(i)) {
                panic!("{what}: row {} differs: {:?} vs {:?}", i + 1, a.get(i), b.get(i));
            }
        };
        let (en, ko) = (app_key_rows(KEYS_EN), app_key_rows(KEYS_KO));
        let md = readme_key_rows(include_str!("../README.md"), "\n## Keyboard shortcuts\n");
        let md_ko = readme_key_rows(include_str!("../README.ko.md"), "\n## 단축키\n");
        assert!(en.len() >= 40 && md.len() >= 40, "{} app rows, {} README rows", en.len(), md.len());
        same_rows("KEYS_EN vs KEYS_KO", &en, &ko);
        same_rows("README.md vs README.ko.md", &md, &md_ko);
        let set = |rows: &[Vec<String>]| rows.iter().flatten().cloned().collect::<std::collections::BTreeSet<_>>();
        let (app, doc) = (set(&en), set(&md));
        let only_app: Vec<_> = app.difference(&doc).collect();
        let only_doc: Vec<_> = doc.difference(&app).collect();
        assert!(
            only_app.is_empty() && only_doc.is_empty(),
            "in the app's shortcut list but not the README tables: {only_app:?}; in the README tables but not the app's list: {only_doc:?}"
        );
        for k in [":F1", ":Ctrl+S", "filter:Alt+↓", "preview:F2", "preview:Del", ":Ctrl+Alt+Shift+→", ":Ctrl+C/X/V"] {
            assert!(app.contains(k), "{k}");
        }
    }

    // Every view menu item is named in README.md / README.ko.md as "View > <its name>" / "보기 > <이름>",
    // word for word as on the menu, so a reader can find it there (and a renamed item can't leave the docs behind).
    #[test]
    fn view_menu_names_in_readmes() {
        let (md, md_ko) = (include_str!("../README.md"), include_str!("../README.ko.md"));
        // Help is the F1 row of the shortcut table and About only shows the version: neither is a setting to find.
        // The language item ("한국어 (Korean)") is not in these tables; the READMEs name it as English or Korean.
        let skip = [ID_HELP, ID_ABOUT];
        let mut names: Vec<(&str, &str)> = VIEW_TOP.iter().chain(&VIEW_END).chain(&VIEW_BUTTONS).filter(|v| !skip.contains(&v.0)).map(|v| (v.1, v.2)).collect();
        names.push(("Selection colour", "선택 색"));
        let name = |s: &'static str| s.split('\t').next().unwrap();
        let missing: Vec<String> = names
            .iter()
            .flat_map(|&(en, ko)| {
                let en = format!("View > {}", name(en));
                let ko = format!("보기 > {}", name(ko));
                [(md, en, "README.md"), (md_ko, ko, "README.ko.md")]
            })
            .filter(|(doc, want, _)| !doc.contains(want.as_str()))
            .map(|(_, want, file)| format!("{file}: {want:?}"))
            .collect();
        assert!(missing.is_empty(), "view menu items the READMEs don't name as on the menu:\n{}", missing.join("\n"));
    }

    // A menu item's name as the menu shows it: no key after a tab or in a "  (Ctrl+…)" tail, no Korean "(&X)"
    // wherever it is, no '&' mark ("&&" is a plain '&'); a count {n} reads "N".
    fn menu_shown(s: &str) -> String {
        let s = s.split('\t').next().unwrap();
        let (mut out, mut rest) = (String::new(), s.split("  (").next().unwrap());
        while let Some(i) = rest.find("(&") {
            let tail = &rest[i + 2..];
            let mut cs = tail.chars();
            match (cs.next(), cs.next()) {
                (Some(c), Some(')')) if c != '&' => {
                    out.push_str(&rest[..i]);
                    rest = &tail[c.len_utf8() + 1..];
                }
                _ => {
                    out.push_str(&rest[..i + 2]);
                    rest = tail;
                }
            }
        }
        out.push_str(rest);
        out.replace("&&", "\0").replace('&', "").replace('\0', "&").replace("{n}", "N")
    }

    // Every ⇄ menu item, and every item of a tab's and a session card's right-click menu, is named in README.md /
    // README.ko.md word for word as the menu shows it, so a reader can find it there (and a renamed item can't
    // leave the docs behind). A ⇄ slot's "{n} selected" variant and the checksum's Stop item stand for the plain
    // one; the tab and card items are looked for in the Tabs / Sessions line only ("Delete" is found anywhere).
    #[test]
    fn menu_names_in_readmes() {
        assert_eq!(menu_shown(M_SWAP.0), "Swap places with");
        assert_eq!(menu_shown(M_SWAP.1), "자리 바꾸기");
        assert_eq!(menu_shown(M_COPY.1), "선택한 N개 복사");
        assert_eq!(menu_shown(M_LIST.1), "이 폴더 파일 목록을 표로 복사 (엑셀용)");
        assert_eq!(menu_shown(M_HASH.1), "선택한 파일의 체크섬(SHA-256)");
        assert_eq!(menu_shown("Duplicate tab\tCtrl+K"), "Duplicate tab");
        assert_eq!(menu_shown("R&&D (&X)"), "R&D ");
        // the menus' own ids, which their pick match reads
        let ids = |v: Vec<usize>| -> Vec<usize> {
            let mut v = v;
            v.sort();
            v
        };
        assert_eq!(ids(TAB_MENU.iter().map(|m| m.0).collect()), [1, 2, 3, 4, 5]);
        assert_eq!(ids(CARD_MENU.iter().map(|m| m.0).collect()), [1, 2, 3]);

        let (md, md_ko) = (include_str!("../README.md"), include_str!("../README.ko.md"));
        let line = |doc: &'static str, start: &str| -> &'static str { doc.lines().find(|l| l.starts_with(start)).unwrap_or_else(|| panic!("no {start:?} line")) };
        let mut open_in = vec![M_CHOOSE];
        for slot in menu_slots() {
            let plain: Vec<_> = slot.iter().filter(|t| **t != M_HASH_STOP && !t.0.contains("{n}")).copied().collect();
            open_in.extend(if plain.is_empty() { slot.to_vec() } else { plain });
        }
        let groups = [
            ("⇄ menu", open_in, md, md_ko),
            ("tab menu", TAB_MENU.iter().map(|m| (m.1, m.2)).collect(), line(md, "- **Tabs**"), line(md_ko, "- **탭**")),
            ("session card menu", CARD_MENU.iter().map(|m| (m.1, m.2)).collect(), line(md, "- **Sessions**"), line(md_ko, "- **세션**")),
        ];
        let mut missing = Vec::new();
        for (what, items, en_doc, ko_doc) in &groups {
            for (en, ko) in items {
                for (doc, name, file) in [(en_doc, menu_shown(en), "README.md"), (ko_doc, menu_shown(ko), "README.ko.md")] {
                    if !doc.contains(name.as_str()) {
                        missing.push(format!("{file}, {what}: {name:?}"));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "menu items the READMEs don't name as on the menu:\n{}", missing.join("\n"));
    }

    // README.md's and README.ko.md's feature lists ("- **" lines) pair up line for line: as many on each side, and
    // each pair names the same keys, so a line added on one side only, or out of order, shows up here.
    #[test]
    fn readme_feature_pairs() {
        let lines = |md: &'static str, heading: &str| -> Vec<&'static str> {
            let sec = md.split_once(heading).unwrap_or_else(|| panic!("no {heading:?} heading")).1;
            sec.split("\n## ").next().unwrap().lines().filter(|l| l.starts_with("- **")).collect()
        };
        let en = lines(include_str!("../README.md"), "\n## Features\n");
        let ko = lines(include_str!("../README.ko.md"), "\n## 기능\n");
        assert!(en.len() >= 40, "{} feature lines", en.len());
        assert_eq!(en.len(), ko.len(), "README.md has {} feature lines, README.ko.md {}", en.len(), ko.len());
        // key_words as for the UI pairs, then the prose around keys evened out: a '·' or '(' run on in Korean,
        // 클릭 / 화살표 spelled out, and mouse words (Ctrl+click, Shift+drop) that only one side may name
        let keys = |l: &str| -> std::collections::BTreeSet<String> {
            key_words(&l.replace("화살표", "arrow").replace("클릭", "click"))
                .into_iter()
                .map(|w| w.split(['·', '(']).next().unwrap().to_string())
                .filter(|w| !w.ends_with('+') && !["click", "drop", "wheel"].iter().any(|m| w.ends_with(m)))
                .collect()
        };
        for (i, (e, k)) in en.iter().zip(&ko).enumerate() {
            assert_eq!(keys(e), keys(k), "feature line {} differs in keys:\n{e}\n{k}", i + 1);
        }
    }

    // ---------- English / Korean UI text pairs ----------

    // Every source file, compiled in: the test exe runs on the Windows runner without the source tree.
    // A new module must be listed here too (ui_text_pairs_match checks this against main.rs's `mod` lines).
    const SOURCES: [(&str, &str); 44] = [
        ("main.rs", include_str!("main.rs")),
        ("autostart.rs", include_str!("autostart.rs")),
        ("clipadd.rs", include_str!("clipadd.rs")),
        ("dialog_jump.rs", include_str!("dialog_jump.rs")),
        ("docparse.rs", include_str!("docparse.rs")),
        ("draw.rs", include_str!("draw.rs")),
        ("everything.rs", include_str!("everything.rs")),
        ("favourites.rs", include_str!("favourites.rs")),
        ("fonts.rs", include_str!("fonts.rs")),
        ("geometry.rs", include_str!("geometry.rs")),
        ("hashsum.rs", include_str!("hashsum.rs")),
        ("hwpdoc.rs", include_str!("hwpdoc.rs")),
        ("jumplist.rs", include_str!("jumplist.rs")),
        ("keyboard.rs", include_str!("keyboard.rs")),
        ("keys_window.rs", include_str!("keys_window.rs")),
        ("layout_picker.rs", include_str!("layout_picker.rs")),
        ("listing.rs", include_str!("listing.rs")),
        ("load_popup.rs", include_str!("load_popup.rs")),
        ("nav.rs", include_str!("nav.rs")),
        ("organize.rs", include_str!("organize.rs")),
        ("paint.rs", include_str!("paint.rs")),
        ("panes.rs", include_str!("panes.rs")),
        ("paste.rs", include_str!("paste.rs")),
        ("peek.rs", include_str!("peek.rs")),
        ("photos.rs", include_str!("photos.rs")),
        ("preview.rs", include_str!("preview.rs")),
        ("rename.rs", include_str!("rename.rs")),
        ("rename_rules.rs", include_str!("rename_rules.rs")),
        ("select_pattern.rs", include_str!("select_pattern.rs")),
        ("session.rs", include_str!("session.rs")),
        ("sessions_ui.rs", include_str!("sessions_ui.rs")),
        ("shellx.rs", include_str!("shellx.rs")),
        ("star.rs", include_str!("star.rs")),
        ("switcher.rs", include_str!("switcher.rs")),
        ("state.rs", include_str!("state.rs")),
        ("status.rs", include_str!("status.rs")),
        ("tabs.rs", include_str!("tabs.rs")),
        ("tour.rs", include_str!("tour.rs")),
        ("tray.rs", include_str!("tray.rs")),
        ("tree.rs", include_str!("tree.rs")),
        ("ui_defs.rs", include_str!("ui_defs.rs")),
        ("update.rs", include_str!("update.rs")),
        ("usage.rs", include_str!("usage.rs")),
        ("window_ui.rs", include_str!("window_ui.rs")),
    ];

    fn hangul(c: char) -> bool {
        ('\u{AC00}'..='\u{D7A3}').contains(&c) || ('\u{3131}'..='\u{318E}').contains(&c)
    }

    // The string literal at the start of s (s[0] is '"'), escapes decoded, and the code after it.
    fn str_lit(s: &str) -> Option<(String, &str)> {
        let (mut out, mut it) = (String::new(), s.char_indices().skip(1).peekable());
        while let Some((i, c)) = it.next() {
            match c {
                '"' => return Some((out, s[i + 1..].trim_start())),
                '\\' => match it.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'u' => {
                        let hex: String = it.by_ref().map(|(_, c)| c).take_while(|&c| c != '}').filter(|&c| c != '{').collect();
                        out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                    }
                    '\n' | '\r' => while it.next_if(|(_, c)| c.is_whitespace()).is_some() {},
                    c => out.push(c), // \" \\ \'
                },
                c => out.push(c),
            }
        }
        None
    }

    // The (line, English, Korean) pairs of a file's app code, not its tests (which hold wrong pairs on
    // purpose): tr("…", "…") with two plain literals, also across lines; and within one line, a literal
    // without Hangul then, after a comma, one with (one("…", "…"), the ⇄ menu's ("…", "…") consts).
    // A line that opens a string it doesn't close (KEYS_EN, a raw string) is left alone.
    fn ui_pairs(src: &str) -> Vec<(usize, String, String)> {
        let cut = src.match_indices("\n#[cfg(test)]").map(|(i, _)| i).find(|&i| src[i + 13..].trim_start().starts_with("mod "));
        let code = &src[..cut.unwrap_or(src.len())];
        let lit = |s: &str| s.strip_prefix('"').and_then(|_| str_lit(s)).map(|(t, rest)| (t, rest.len()));
        let mut out = Vec::new();
        for (at, _) in code.match_indices("tr(") {
            if code[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                continue; // as_str(
            }
            let rest = code[at + 3..].trim_start();
            let Some((en, n)) = lit(rest) else { continue };
            let Some(rest) = rest[rest.len() - n..].strip_prefix(',').map(str::trim_start) else { continue };
            let Some((ko, n)) = lit(rest) else { continue };
            if rest[rest.len() - n..].trim_start_matches(',').trim_start().starts_with(')') {
                out.push((code[..at].matches('\n').count() + 1, en, ko));
            }
        }
        for (n, l) in code.lines().enumerate() {
            let (b, mut i, mut lits) = (l.as_bytes(), 0, Vec::new());
            let whole = loop {
                match b.get(i) {
                    None => break true,
                    Some(b'/') if b.get(i + 1) == Some(&b'/') => break true, // a comment
                    Some(b'\'') if b.get(i + 2) == Some(&b'\'') => i += 3, // '"'
                    Some(b'\'') if b.get(i + 1) == Some(&b'\\') && b.get(i + 3) == Some(&b'\'') => i += 4,
                    Some(b'r') if matches!(b.get(i + 1), Some(b'"' | b'#')) && !l[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_') => {
                        let close = format!("\"{}", "#".repeat(b[i + 1..].iter().take_while(|&&c| c == b'#').count()));
                        match l.get(i + close.len() + 1..).and_then(|t| t.find(&close)) {
                            Some(e) => i += close.len() + 1 + e + close.len(),
                            None => break false,
                        }
                    }
                    Some(b'"') => match str_lit(&l[i..]) {
                        Some((t, rest)) => {
                            lits.push((i, l.len() - rest.len(), t));
                            i = l.len() - rest.len();
                        }
                        None => break false,
                    },
                    Some(_) => i += 1,
                }
            };
            for w in lits.windows(2).filter(|_| whole) {
                let ((_, end, en), (start, _, ko)) = (&w[0], &w[1]);
                let pair = (n + 1, en.clone(), ko.clone());
                if l[*end..*start].trim() == "," && !en.chars().any(hangul) && ko.chars().any(hangul) && !out.contains(&pair) {
                    out.push(pair);
                }
            }
        }
        out
    }

    // Shortcut words in order: Ctrl/Alt/Shift/Win+… combos and lone F1–F12. Korean key names read as
    // English (방향키 arrows, 휠 wheel), a range keeps its first key (Ctrl+1–9 and Ctrl+1~9 are Ctrl+1),
    // and a word ends at a space, Hangul (Ctrl+V를) or ) , ; : and drops a sentence's closing '.'.
    fn key_words(s: &str) -> Vec<String> {
        let s = s.replace("방향키", "arrows").replace('휠', "wheel");
        let (mut out, mut i) = (Vec::new(), 0);
        while let Some(c) = s[i..].chars().next() {
            let rest = &s[i..];
            let start = !s[..i].ends_with(|c: char| c.is_ascii_alphanumeric());
            let digits = rest.strip_prefix('F').map_or(0, |r| r.len() - r.trim_start_matches(|c: char| c.is_ascii_digit()).len());
            let len = if start && ["Ctrl+", "Alt+", "Shift+", "Win+"].iter().any(|m| rest.starts_with(m)) {
                rest.find(|c: char| c.is_whitespace() || hangul(c) || "),;:".contains(c)).unwrap_or(rest.len())
            } else if start
                && (1..=2).contains(&digits)
                && (1..=12).contains(&rest[1..=digits].parse::<u32>().unwrap())
                && !rest[1 + digits..].starts_with(|c: char| c.is_ascii_alphanumeric())
            {
                1 + digits
            } else {
                0
            };
            if len > 0 {
                out.push(rest[..len].split(['–', '~', '…']).next().unwrap().trim_end_matches('.').to_string());
                i += len;
            } else {
                i += c.len_utf8();
            }
        }
        out
    }

    // Why an English / Korean pair disagrees, if it does: an empty side, other {name} placeholders, menu
    // keys (after \t) or shortcut words, or a closing '…' on one side only (a Korean "(&O)" tail aside).
    fn pair_diff(en: &str, ko: &str) -> Option<String> {
        let holes = |s: &str| {
            let mut v: Vec<String> = s.split('{').skip(1).filter_map(|t| t.split_once('}')).map(|(h, _)| h).filter(|h| h.chars().all(|c| c.is_ascii_lowercase() || c == '_')).map(String::from).collect();
            v.sort();
            v
        };
        let tail = |s: &str| s.split_once('\t').map(|(_, t)| t.to_string());
        let keys = |s: &str| key_words(s).into_iter().collect::<std::collections::BTreeSet<_>>();
        let dots = |s: &str| {
            let h = s.split('\t').next().unwrap();
            h.strip_suffix(')').and_then(|t| t.rfind("(&").map(|i| &t[..i])).unwrap_or(h).trim_end().ends_with('…')
        };
        if en.trim().is_empty() || ko.trim().is_empty() {
            Some("an empty side".into())
        } else if holes(en) != holes(ko) {
            Some(format!("placeholders {:?} vs {:?}", holes(en), holes(ko)))
        } else if tail(en) != tail(ko) {
            Some(format!("menu keys {:?} vs {:?}", tail(en), tail(ko)))
        } else if keys(en) != keys(ko) {
            Some(format!("shortcuts {:?} vs {:?}", keys(en), keys(ko)))
        } else if dots(en) != dots(ko) {
            Some("'…' at the end on one side only".into())
        } else {
            None
        }
    }

    // The keys a text advertises: its menu tail after \t, and the first key of a "(Ctrl+B …" hint, with
    // "arrows" as the shortcut list's "→" row.
    fn advertised(s: &str) -> Vec<String> {
        let hints = s.split('(').skip(1).chain(s.split_once('\t').map(|(_, t)| t));
        hints.filter_map(|h| key_words(h).into_iter().next().filter(|k| h.starts_with(k.as_str()))).map(|k| k.replace("arrows", "→")).collect()
    }

    // The two languages of each UI text must agree where a slip shows: the same {placeholders} (else "{n}"
    // shows as is, or a count goes missing), menu key, shortcuts and closing '…'. And the key a menu item or
    // tooltip advertises must be in the shortcut list (Ctrl+/). The compiler catches none of this.
    #[test]
    fn ui_text_pairs_match() {
        for (en, ko) in [
            ("Copied {n} files", "파일을 복사함"),
            ("Open\tCtrl+O", "열기\tCtrl+P"),
            ("Back  (Alt+←)", "뒤로  (Alt+→)"),
            ("Choose a folder…", "폴더 고르기"),
            ("Save", " "),
        ] {
            assert!(pair_diff(en, ko).is_some(), "{en:?} / {ko:?} should differ");
        }
        for (en, ko) in [
            ("Swap  (Ctrl+Alt+Shift+arrows) · Ctrl+V.", "자리 바꾸기(&S)  (Ctrl+Alt+Shift+방향키) · Ctrl+V를"),
            ("Panes: Ctrl+Tab, Ctrl+1–9. Help: F1.", "패널: Ctrl+Tab, Ctrl+1~9. 도움말: F1."),
            ("Zoom: Ctrl+wheel", "확대: Ctrl+휠"),
            ("Choose an&other folder…", "다른 폴더 고르기…(&O)"),
            ("{m} of {n} into {name}", "{name}에 {n}개 중 {m}개"),
        ] {
            assert_eq!(pair_diff(en, ko), None, "{en:?} / {ko:?}");
        }
        assert_eq!(advertised("Favourites (Ctrl+B; open) · Help (online)\tF1"), ["Ctrl+B", "F1"]);
        assert_eq!(advertised("&Swap places  (Ctrl+Alt+Shift+arrows)"), ["Ctrl+Alt+Shift+→"]);
        let sample = r#"a(tr("Say \"hi\" {n}", "{n} \"안녕\""));
let t = tr(
    "Two",
    "둘",
);
const M: (&str, &str) = ("Pick", "고르기"); // ("x", "엑스")
let c = '"'; tr(name, "이름"); as_str("x", "y");
#[cfg(test)]
mod tests { tr("in", "안") }
"#;
        let got = ui_pairs(sample);
        let got: Vec<(usize, &str, &str)> = got.iter().map(|(l, en, ko)| (*l, en.as_str(), ko.as_str())).collect();
        assert_eq!(got, [(1, "Say \"hi\" {n}", "{n} \"안녕\""), (2, "Two", "둘"), (6, "Pick", "고르기")]);

        let main = SOURCES[0].1;
        let mods: Vec<&str> = main.lines().filter_map(|l| l.strip_prefix("mod ")?.strip_suffix(';')).collect();
        assert!(mods.len() >= 30, "{} mod lines in main.rs", mods.len());
        for m in mods {
            assert!(SOURCES.iter().any(|(f, _)| f.strip_suffix(".rs") == Some(m)), "src/{m}.rs is missing from SOURCES");
        }
        let listed: std::collections::BTreeSet<String> = app_key_rows(KEYS_EN).iter().flatten().flat_map(|k| key_words(&k[k.find(':').unwrap() + 1..])).collect();
        let (mut pairs, mut bad) = (0, Vec::new());
        for (file, src) in SOURCES {
            for (line, en, ko) in ui_pairs(src) {
                pairs += 1;
                let unlisted = advertised(&en).into_iter().filter(|k| !listed.contains(k)).map(|k| format!("{k} is not in the shortcut list (Ctrl+/)"));
                bad.extend(pair_diff(&en, &ko).into_iter().chain(unlisted).map(|why| format!("src/{file}:{line}: {why}\n    en: {en:?}\n    ko: {ko:?}")));
            }
        }
        assert!(pairs >= 250, "only {pairs} English / Korean pairs found: does ui_pairs still see tr(\"…\", \"…\")?");
        assert!(bad.is_empty(), "{} English / Korean pairs disagree:\n{}", bad.len(), bad.join("\n"));
    }

    // The status-line shortcut tips (TIP_*): each tip's key is in the shortcut list (Ctrl+/), so a changed or
    // dropped shortcut can't leave a tip naming the old key; each is shown by a mouse action (mouse_tip) and
    // retired by its key (key_used) somewhere in the app code; and the READMEs' tip paragraph names the mouse
    // action in both languages. Calls are matched as written, `mouse_tip(TIP_X)`: an expression in the
    // parentheses (`if … { TIP_A } else { TIP_B }`) is not seen, so keep one call per tip.
    #[test]
    fn key_tips_consistent() {
        // (tip, its name in the code, the README.md words, the README.ko.md words) for each mouse action
        let tips = [
            (TIP_BACK, "TIP_BACK", "←", "←"),
            (TIP_FWD, "TIP_FWD", "→", "→"),
            (TIP_UP, "TIP_UP", "↑", "↑"),
            (TIP_FAVS, "TIP_FAVS", "★", "★"),
            (TIP_EV, "TIP_EV", "the Everything magnifier", "Everything 돋보기"),
            (TIP_ZOOM, "TIP_ZOOM", "double-clicking a pane header", "패널 머리줄 더블클릭"),
            (TIP_NEW_TAB, "TIP_NEW_TAB", "a tab's +", "탭의 +"),
            (TIP_CLOSE_TAB, "TIP_CLOSE_TAB", "the current tab's ×", "지금 탭의 ×"),
            (TIP_LOAD, "TIP_LOAD", "Load", "불러오기"),
        ];
        let mut ids: Vec<usize> = tips.iter().map(|t| t.0).collect();
        ids.sort();
        assert_eq!(ids, (0..TIP_N).collect::<Vec<_>>(), "the table must name each TIP_* (0..TIP_N) once");

        let listed: std::collections::BTreeSet<String> = app_key_rows(KEYS_EN).iter().flatten().flat_map(|k| key_words(&k[k.find(':').unwrap() + 1..])).collect();
        // app code only: a file's tests (and this one) are cut off as in ui_pairs, and // comments are skipped
        let code: Vec<(&str, &str)> = SOURCES
            .iter()
            .map(|&(file, src)| {
                let cut = src.match_indices("\n#[cfg(test)]").map(|(i, _)| i).find(|&i| src[i + 13..].trim_start().starts_with("mod "));
                (file, &src[..cut.unwrap_or(src.len())])
            })
            .collect();
        let called = |call: &str| code.iter().any(|(_, src)| src.lines().any(|l| l.split("//").next().unwrap().contains(call)));
        let tip_line = |md: &'static str, start: &str, eg: &str| {
            let l = md.lines().find(|l| l.starts_with(start)).unwrap_or_else(|| panic!("no line starting {start:?}"));
            l.split(eg).next().unwrap() // the actions, not the example tip after them
        };
        let (md, md_ko) = (tip_line(include_str!("../README.md"), "Shortcut tips:", "(e.g."), tip_line(include_str!("../README.ko.md"), "단축키 팁:", "(예:"));

        let mut bad = Vec::new();
        for (tip, name, en, ko) in tips {
            let key = TIP_KEYS[tip];
            if !key_words(key).first().is_some_and(|k| k == key && listed.contains(k)) {
                bad.push(format!("{name}: its key {key:?} is not in the shortcut list (Ctrl+/)"));
            }
            for call in [format!("mouse_tip({name})"), format!("key_used({name})")] {
                if !called(&call) {
                    bad.push(format!("{name}: no `{call};` in the app code (written as is, one call per tip)"));
                }
            }
            for (doc, file, words) in [(md, "README.md 'Shortcut tips:'", en), (md_ko, "README.ko.md '단축키 팁:'", ko)] {
                if !doc.contains(words) {
                    bad.push(format!("{name}: {file} doesn't name its mouse action {words:?}"));
                }
            }
        }
        assert!(bad.is_empty(), "shortcut tips out of step:\n{}", bad.join("\n"));
    }

    // Every reason a beep now comes with: both languages, one status line, the name in both, and seen by the
    // pair check above (one ("en", "ko") line each in select_pattern.rs).
    #[test]
    fn fail_texts() {
        let seen = ui_pairs(include_str!("select_pattern.rs"));
        for kind in FAILS {
            let (en, ko) = fail_pair(kind);
            assert!(seen.iter().any(|(_, e, k)| e == en && k == ko), "{kind:?} is not one (\"en\", \"ko\") line");
            assert_eq!(en.contains("{name}"), ko.contains("{name}"), "{kind:?}");
            for lang in [false, true] {
                let t = fail_text(kind, "D:\\문서\\Docuemnts", lang);
                assert!(!t.trim().is_empty() && !t.contains(['\n', '\r', '{']), "{kind:?}: {t:?}");
                assert_eq!(t.contains("D:\\문서\\Docuemnts"), en.contains("{name}"), "{kind:?}: {t:?}");
            }
        }
        assert!(FAILS.iter().enumerate().all(|(n, k)| !FAILS[..n].contains(k)), "a kind twice in FAILS");
        assert_eq!(FAILS.map(|k| fail_pair(k).0).iter().collect::<std::collections::BTreeSet<_>>().len(), FAILS.len());
        // a long typed address is cut by characters (Korean ones too), never mid-letter; a pasted line break is dropped
        let long = "가".repeat(200);
        let t = fail_text(Fail::AddrNotFound, &long, true);
        assert!(t.contains(&format!("{}…", "가".repeat(79))) && !t.contains(&"가".repeat(80)), "{t}");
        assert_eq!(fail_text(Fail::AddrCantOpen, " a\r\nb ", false), "Can't open 'a'");
        assert_eq!(fail_text(Fail::DriveCantOpen, "E:", true), "E: 드라이브에 디스크가 없거나 열 수 없음");
    }

    #[test]
    fn file_kinds() {
        for (_, _, spec) in FILE_KINDS {
            assert_eq!(filter_spec(spec), spec);
        }
        assert_eq!(same_type("a.PDF", false).as_deref(), Some("*.PDF"));
        assert_eq!(same_type("x.tar.gz", false).as_deref(), Some("*.gz"));
        for (name, dir) in [("Makefile", false), (".gitignore", false), ("a.", false), ("a.b;c", false), ("photos.2024", true)] {
            assert_eq!(same_type(name, dir), None, "{name}");
        }
        let filt = RECT { left: 100, top: 4, right: 250, bottom: 26 };
        assert_eq!(filter_icon_rect(filt, 24), Some(RECT { right: 124, ..filt }));
        assert_eq!(filter_icon_rect(RECT { right: 110, ..filt }, 24), Some(RECT { right: 110, ..filt }));
        assert_eq!(filter_icon_rect(RECT { right: 100, ..filt }, 24), None);
    }

    #[test]
    fn held_keys() {
        let (enter, a) = (VK_RETURN.0, VK_A.0);
        assert_eq!(held_key(Some(enter), enter, true), (true, Some(enter))); // its repeat: swallowed
        assert_eq!(held_key(Some(enter), enter, false), (false, None)); // a fresh press: keyup was missed
        assert_eq!(held_key(Some(enter), a, false), (false, Some(enter)));
        assert_eq!(held_key(Some(enter), a, true), (false, Some(enter)));
        assert_eq!(held_key(None, enter, true), (false, None));
    }

    // Alt let go alone or F10 opens the view menu; Alt with any other key, a modifier or a click does not.
    #[test]
    fn menu_keys() {
        let (alt, f10, left, shift, ctrl, hangul) = (VK_MENU.0, VK_F10.0, VK_LEFT.0, VK_SHIFT.0, VK_CONTROL.0, VK_HANGUL.0);
        let none = [false; 4];
        let with_alt = [false, false, true, false];
        // presses in order (key, down, repeat, mods); the last release's verdict
        let run = |keys: &[(u16, bool, bool, [bool; 4])]| {
            let mut armed = None;
            let mut open = false;
            for &(vk, down, repeat, mods) in keys {
                (armed, open) = menu_key_step(armed, down, vk, repeat, mods);
            }
            open
        };
        assert!(run(&[(alt, true, false, with_alt), (alt, false, false, none)])); // Alt alone
        assert!(run(&[(alt, true, false, with_alt), (alt, true, true, with_alt), (alt, true, true, with_alt), (alt, false, false, none)])); // held a while
        assert!(run(&[(f10, true, false, none), (f10, true, true, none), (f10, false, false, none)]));
        // Alt+←, Alt+Space, Alt+1 ... : the other key disarms it
        assert!(!run(&[(alt, true, false, with_alt), (left, true, false, with_alt), (left, false, false, with_alt), (alt, false, false, none)]));
        assert!(!run(&[(alt, true, false, with_alt), (VK_SPACE.0, true, false, with_alt), (alt, false, false, none)]));
        // Alt+Shift (input language), Ctrl+Alt / AltGr (Ctrl comes first), Win+Alt, Ctrl then Alt
        assert!(!run(&[(alt, true, false, with_alt), (shift, true, false, [false, true, true, false]), (shift, false, false, with_alt), (alt, false, false, none)]));
        assert!(!run(&[(ctrl, true, false, [true, false, false, false]), (alt, true, false, [true, false, true, false]), (alt, false, false, [true, false, false, false])]));
        assert!(!run(&[(alt, true, false, [false, false, true, true]), (alt, false, false, none)]));
        // Shift+F10 (context menu), Alt+F10, Ctrl+F10: not the view menu
        assert!(!run(&[(f10, true, false, [false, true, false, false]), (f10, false, false, none)]));
        assert!(!run(&[(f10, true, false, with_alt), (f10, false, false, none)]));
        assert!(!run(&[(f10, true, false, [true, false, false, false]), (f10, false, false, none)]));
        // a key pressed before Alt and let go while Alt is down
        assert!(!run(&[(alt, true, false, with_alt), (left, false, false, with_alt), (alt, false, false, none)]));
        // a Korean keyboard's right Alt is 한/영 (VK_HANGUL), never Alt
        assert!(!run(&[(hangul, true, false, none), (hangul, false, false, none)]));
        // a repeat that comes after a disarm (Ctrl+Alt held, Ctrl let go) does not arm again
        assert_eq!(menu_key_step(None, true, alt, true, with_alt), (None, false));
        // the release of an unarmed Alt (after a click, Alt+Tab: armed was cleared) opens nothing
        assert_eq!(menu_key_step(None, false, alt, false, none), (None, false));
        assert_eq!(menu_key_step(Some(f10), false, alt, false, none), (None, false));
    }

    // The letter a menu item underlines: the char after a lone '&' ("&&" is a literal '&').
    fn mnemonic(s: &str) -> Option<char> {
        let mut it = s.chars();
        while let Some(c) = it.next() {
            if c == '&' {
                match it.next() {
                    Some('&') => {}
                    n => return n.map(|c| c.to_ascii_uppercase()),
                }
            }
        }
        None
    }

    #[test]
    fn open_in_menu_pane_labels() {
        assert_eq!(pane_label("Pane", 2, "Sage", r"C:\R&D"), r"Pane &3 (Sage)    C:\R&&D");
        assert_eq!(pane_label("패널", 0, "", "A && B"), "패널 &1    A &&&& B");
        assert_eq!(pane_label("Pane", 8, "", "x"), "Pane &9    x");
        assert_eq!(pane_label("Pane", 9, "", "R&D"), "Pane 10    R&&D"); // 10–16: no digit to give
        for k in 0..MAXP {
            let want = (k < 9).then(|| char::from(b'1' + k as u8));
            assert_eq!(mnemonic(&pane_label("Pane", k, "Rose", "Tom & Jerry")), want, "pane {}", k + 1);
        }
    }

    // Every ⇄ menu item that can show at the top level at the same time: one letter each, none a pane digit,
    // none the first letter of an unmarked item (the grey title, "Pane 10" …), in both languages.
    fn menu_slots() -> [&'static [(&'static str, &'static str)]; 17] {
        [&[M_SWAP], &[M_COPY], &[M_MOVE], &[M_INUSE], &[M_HASH, M_HASH_STOP], &[M_ORG], &[M_PHOTO], &[M_GROUP_N, M_GROUP], &[M_STAMP_N, M_STAMP], &[M_EMPTY], &[M_USAGE], &[M_LIST_N, M_LIST], &[M_CLIPADD_N, M_CLIPADD], &[M_RESTORE], &[M_DIFF], &[M_EXPLORER_SEL, M_EXPLORER], &[M_WM_PAIR, M_WM_PANE]]
    }

    // X is Explorer's own letter: no other ⇄ item (nor the title, a pane row or a digit) takes it
    #[test]
    fn open_in_menu_explorer_letter() {
        for ko in [false, true] {
            let pick = |t: (&str, &str)| mnemonic(if ko { t.1 } else { t.0 });
            assert_eq!(pick(M_EXPLORER_SEL), Some('X'));
            assert_eq!(pick(M_EXPLORER), Some('X'));
            let others = menu_slots().into_iter().flatten().filter(|t| **t != M_EXPLORER_SEL && **t != M_EXPLORER).filter(|t| pick(**t) == Some('X')).count();
            assert_eq!(others, 0, "ko {ko}");
        }
        assert_eq!(open_in_pick(ID_EXPLORER), OpenPick::Shell);
    }

    #[test]
    fn open_in_menu_letters() {
        for ko in [false, true] {
            let mut seen: Vec<char> = "123456789".chars().collect();
            seen.extend(["Open the selected folder", "Pane"].iter().map(|s| s.chars().next().unwrap()));
            for slot in menu_slots() {
                let letters: Vec<_> = slot.iter().map(|t| mnemonic(if ko { t.1 } else { t.0 })).collect();
                let c = letters[0].unwrap_or_else(|| panic!("no letter: {:?}", slot[0]));
                assert!(letters.iter().all(|l| *l == Some(c)), "{slot:?}");
                assert!(!seen.contains(&c), "{c} used twice ({slot:?}, ko {ko})");
                seen.push(c);
                for t in slot {
                    let s = if ko { t.1 } else { t.0 };
                    assert_eq!(s.matches('&').count(), 1, "{s}");
                    // Korean marks it in a tail, "…(&C)", the way Windows menus do for non-Latin text
                    assert!(!ko || s.contains(&format!("(&{c})")), "{s}");
                }
            }
        }
    }

    // Every ⇄ menu letter is named in the Ctrl+M row of the shortcut window (both languages) and of both
    // README tables, after "Then one key" / "키 하나로", so a new ⇄ item can't leave its letter undocumented.
    #[test]
    fn open_in_menu_letters_documented() {
        let row = |what: &str, text: &str, start: &str, marker: &str| -> String {
            let line = text.lines().find(|l| l.starts_with(start)).unwrap_or_else(|| panic!("{what}: no Ctrl+M row"));
            let (_, tail) = line.split_once(marker).unwrap_or_else(|| panic!("{what}: no {marker:?} in the Ctrl+M row"));
            tail.to_string()
        };
        // an upper-case letter on its own: "S swaps", "S는" count; "SHA-256", "WinMerge", "Ctrl+F7" don't
        let alone = |s: &str, c: char| {
            let v: Vec<char> = s.chars().collect();
            (0..v.len()).any(|i| v[i] == c && !(i > 0 && v[i - 1].is_ascii_alphanumeric()) && !v.get(i + 1).is_some_and(|n| n.is_ascii_alphanumeric()))
        };
        let docs = [
            ("KEYS_EN", row("KEYS_EN", KEYS_EN, "Ctrl+M\t", "Then one key")),
            ("KEYS_KO", row("KEYS_KO", KEYS_KO, "Ctrl+M\t", "키 하나로")),
            ("README.md", row("README.md", include_str!("../README.md"), "| Ctrl+M |", "Then one key")),
            ("README.ko.md", row("README.ko.md", include_str!("../README.ko.md"), "| Ctrl+M |", "키 하나로")),
        ];
        for slot in menu_slots() {
            let c = mnemonic(slot[0].0).unwrap_or_else(|| panic!("no letter: {:?}", slot[0]));
            for (what, tail) in &docs {
                assert!(alone(tail, c), "{what}: the Ctrl+M row doesn't name the ⇄ menu letter {c} ({:?})", slot[0].0);
            }
        }
    }

    #[test]
    fn insert_marks_and_steps() {
        let (sel, focus, show) = (SVSI_SELECT.0 as u32, SVSI_FOCUSED.0 as u32, SVSI_ENSUREVISIBLE.0 as u32);
        // the focused item flips and keeps the focus; the next one takes it and keeps its own selection
        assert_eq!(ins_flags(false, false), (sel | focus, focus | show));
        assert_eq!(ins_flags(false, true), (sel | focus, sel | focus | show));
        assert_eq!(ins_flags(true, false), (focus, focus | show));
        assert_eq!(ins_flags(true, true), (focus, sel | focus | show));
        assert_eq!(SVSI_DESELECT.0, 0); // why the next item's SVSI_SELECT matters
    }

    #[test]
    fn match_box_fits() {
        for dpi in [96, 144, 192] {
            let s = |v: i32| v * dpi / 96;
            for label_w in [0, s(40), s(400)] {
                let g = match_geo(label_w, s(16), dpi);
                assert!(g.w >= s(320) && g.label.right - g.label.left == label_w);
                assert!(g.field.left >= g.label.right && g.field.right - g.field.left >= s(120) && g.field.right < g.w);
                assert!(g.edit.left > g.field.left && g.edit.right < g.field.right);
                assert!(g.edit.top >= g.field.top && g.edit.bottom <= g.field.bottom && g.field.bottom < g.h);
            }
        }
    }

    #[test]
    fn shortcut_layout() {
        for dpi in [96, 192] {
            for dip in [300, 760] {
                let (cw, lh) = (7 * dpi / 96, 16 * dpi / 96);
                let measure = |t: &str, max_w: i32, cap: bool| {
                    let w = t.chars().count() as i32 * cw;
                    if cap {
                        (w, lh)
                    } else {
                        let lines = t.split('\n').map(|l| ((l.chars().count() as i32 * cw + max_w - 1) / max_w).max(1)).sum::<i32>();
                        (w.min(max_w), lines * lh)
                    }
                };
                for (text, stacked) in [KEYS_EN, KEYS_KO].into_iter().flat_map(|t| [(t, false), (t, true)]) {
                    let g = keys_layout(&key_rows(text), dip * dpi / 96, dpi, stacked, &measure);
                    let (pad, gap) = (KEYS_PAD * dpi / 96, 16 * dpi / 96);
                    let mut prev = 0;
                    for r in &g.rows {
                        assert!(r.row.top >= prev, "rows overlap at {dpi} dpi, {dip} dip");
                        for k in &r.keys {
                            // key column: from the padding to a gap before the description, or (stacked) above it
                            let clear = if stacked { k.bottom <= r.desc.top } else { k.right + gap <= r.desc.left };
                            assert!(k.left >= pad && k.right <= dip * dpi / 96 - pad && clear && k.bottom <= r.row.bottom);
                        }
                        let cap_lines = r.keys.iter().map(|k| k.bottom).max().unwrap() - r.row.top;
                        assert!(r.row.bottom - r.row.top >= cap_lines.max(r.desc.bottom - r.desc.top));
                        assert!(r.desc.bottom <= r.row.bottom && r.desc.right <= dip * dpi / 96 - pad);
                        prev = r.row.bottom;
                    }
                    assert!(prev <= g.h && g.titles[1].top > g.titles[0].bottom);
                    // list items: back to back from 0 to the bottom, each holding its row (and a section's title)
                    let bands = keys_bands(&g, dpi);
                    assert_eq!(bands.len(), g.rows.len());
                    let mut end = 0;
                    for (i, (b, r)) in bands.iter().zip(&g.rows).enumerate() {
                        assert!(b.0 == end && b.0 <= r.row.top && r.row.bottom <= b.1, "item {i} at {dpi} dpi, {dip} dip");
                        if i == 0 || g.rows[i - 1].sec != r.sec {
                            assert!(b.0 <= g.titles[r.sec].top);
                        }
                        end = b.1;
                    }
                    assert_eq!(end, g.h);
                }
            }
        }
        // what a screen reader hears: one item per drawn row, a section's name before its first
        for text in [KEYS_EN, KEYS_KO] {
            let items = key_items(text, ["Keyboard", "Mouse"]);
            let rows = key_rows(text);
            assert_eq!(items.len(), rows[0].len() + rows[1].len());
            assert_eq!(items[0], "Keyboard. Ctrl+Tab / Ctrl+Shift+Tab: ".to_string() + &rows[0][0].desc);
            assert!(items[1].starts_with("Ctrl+1"));
            assert!(items[rows[0].len()].starts_with("Mouse. "));
            assert!(items.iter().all(|t| !t.contains('\n') && !t.contains('\t')));
        }
        // a mouse line without ": " continues the one above
        assert_eq!(key_items("A\ta\n\nB: b\nmore", ["K", "M"]), ["K. A: a", "M. B: b more"]);
    }

    // The shortcut window's find box: every word, any case, in either language's keys or description.
    #[test]
    fn shortcut_find() {
        let row = |k: &str, d: &str| KeyRow { keys: vec![k.into()], desc: d.into() };
        let (en, ko) = (row("Ctrl+Shift+R", "Batch rename the selected items"), row("Ctrl+Shift+R", "선택한 항목 일괄 이름 바꾸기"));
        for q in ["", "  ", "RENAME", "rename  batch", "이름 바꾸기", "ctrl+shift+r 일괄", "Rename 이름"] {
            assert!(keys_match(q, &en, &ko) && keys_match(q, &ko, &en), "{q:?}");
        }
        for q in ["zip", "rename zip", "이름 압축", "ctrl+shift+q"] {
            assert!(!keys_match(q, &en, &ko), "{q:?}");
        }
        let (en, ko) = (key_rows(KEYS_EN), key_rows(KEYS_KO));
        assert_eq!(keys_filter("", &ko, &en).map(|s| s.len()), [ko[0].len(), ko[1].len()]);
        assert_eq!(keys_filter("qqq zzz", &ko, &en).map(|s| s.len()), [0, 0]);
        // an English word on the Korean list (and a Korean one on the English list) finds the same rows
        let ren = en[0].iter().position(|r| r.keys[0] == "Ctrl+Shift+R").unwrap();
        let hit = keys_filter("RENAME", &ko, &en);
        assert!(hit[0].contains(&ren) && hit == keys_filter("rename", &en, &ko));
        assert!(keys_filter("일괄", &en, &ko)[0].contains(&ren));

        // A filtered list: the key column as for the whole list, an empty section without its title or gap.
        let dpi = 144;
        let s = |v: i32| v * dpi / 96;
        let (cw, lh) = (s(7), s(16));
        let measure = |t: &str, max_w: i32, cap: bool| {
            let w = t.chars().count() as i32 * cw;
            let lines = if cap { 1 } else { t.split('\n').map(|l| ((l.chars().count() as i32 * cw + max_w - 1) / max_w).max(1)).sum::<i32>() };
            (if cap { w } else { w.min(max_w) }, lines * lh)
        };
        let width = s(760);
        let full = keys_layout_in(&en, &en[0], width, dpi, false, &measure);
        let whole = keys_layout(&en, width, dpi, false, &measure);
        assert_eq!((full.h, full.rows.len(), full.titles), (whole.h, whole.rows.len(), whole.titles));
        let pad = s(KEYS_PAD);
        let pick = |a: &[usize], b: &[usize]| [a.iter().map(|&j| en[0][j].clone()).collect(), b.iter().map(|&j| en[1][j].clone()).collect()];
        let first_mouse = full.rows.iter().find(|r| r.sec == 1).unwrap();
        // only mouse rows: no keyboard title above, the description column where it was
        let g = keys_layout_in(&pick(&[], &[0, 1]), &en[0], width, dpi, false, &measure);
        assert_eq!(g.rows.len(), 2);
        assert!(g.rows.iter().all(|r| r.sec == 1) && g.titles[1].top == pad && g.titles[0] == RECT::default());
        assert_eq!((g.rows[0].desc.left, g.rows[0].keys[0].right), (first_mouse.desc.left, first_mouse.keys[0].right));
        assert_eq!(g.h, g.rows[1].row.bottom + pad);
        // one row of each: the mouse title after the usual gap
        let g = keys_layout_in(&pick(&[ren], &[0]), &en[0], width, dpi, false, &measure);
        assert_eq!(g.rows.len(), 2);
        assert_eq!(g.titles[1].top, g.rows[0].row.bottom + s(6) + s(14));
        assert_eq!(g.rows[0].desc.left, full.rows[ren].desc.left);
        let bands = keys_bands(&g, dpi);
        assert_eq!((bands[0].0, bands[1].1, bands[0].1), (0, g.h, bands[1].0));
        // without titles: the first row starts at the padding and its item is shorter by the title and its gap
        let bare = keys_layout_at(&pick(&[ren], &[0]), &en[0], width, dpi, false, false, &measure);
        assert_eq!(bare.titles, [RECT::default(); 2]);
        assert_eq!(bare.rows[0].row.top, pad);
        assert_eq!(keys_bands(&bare, dpi)[0].1 + lh + s(6), bands[0].1);
        // nothing left: no rows, no items
        let g = keys_layout_in(&pick(&[], &[]), &en[0], width, dpi, false, &measure);
        assert!(g.rows.is_empty() && keys_bands(&g, dpi).is_empty());
        // the find box sits between the title and ✕, its edit inside it
        let (field, edit) = keys_find_rects(width, dpi, s(130), lh);
        let close = keys_close(width, dpi);
        assert!(field.left >= pad + s(130) && field.right < close.left && field.right - field.left >= s(200));
        assert!(edit.left > field.left && edit.right < field.right && edit.top >= field.top && edit.bottom <= field.bottom);
        assert!(field.bottom <= keys_head(dpi));
    }

    #[test]
    fn address_icon() {
        // the square sits inside the pill, left of where crumbs and edit start; none in a narrow pill
        for dpi in [96, 144] {
            let s = |v: i32| v * dpi / 96;
            let addr = |w| RECT { left: 100, top: s(4), right: 100 + w, bottom: s(4) + s(22) };
            assert_eq!(addr_icon(addr(s(90) - 1), dpi), None);
            assert_eq!(crumb_field(addr(s(90) - 1), dpi).left, 100 + s(8)); // as before
            let a = addr(s(90));
            let r = addr_icon(a, dpi).unwrap();
            assert!(r.left > a.left && r.right < a.left + s(26) && r.top >= a.top && r.bottom <= a.bottom, "{dpi}");
            assert_eq!((r.right - r.left, r.bottom - r.top), (s(16), s(16)));
            assert_eq!(crumb_field(a, dpi).left, a.left + s(26));
            // a press on the icon drags the folder out; just off it (or in a narrow pill) it stays a crumb-pill press
            assert!(on_addr_icon(a, dpi, r.left, r.top) && on_addr_icon(a, dpi, r.right - 1, r.bottom - 1), "{dpi}");
            assert!(on_addr_icon(a, dpi, (r.left + r.right) / 2, (r.top + r.bottom) / 2));
            for (x, y) in [(r.left - 1, r.top), (r.right, r.top), (r.left, r.top - 1), (r.left, r.bottom), (crumb_field(a, dpi).left, r.top)] {
                assert!(!on_addr_icon(a, dpi, x, y), "{dpi}: ({x}, {y})");
            }
            let narrow = addr(s(90) - 1);
            assert!(!on_addr_icon(narrow, dpi, narrow.left + s(6), narrow.top + s(8)), "{dpi}");
        }
        // one waiting lookup per pane: a newer one replaces it, its PIDL comes back exactly once
        let mut q = Vec::new();
        let mut freed = Vec::new();
        for (h, t, pidl) in [(1, 1, 101), (2, 2, 102), (1, 3, 103), (1, 4, 104), (2, 5, 105), (3, 6, 106)] {
            freed.extend(enqueue(&mut q, (h, t, pidl)));
        }
        assert_eq!(q, [(1, 4, 104), (2, 5, 105), (3, 6, 106)]);
        assert_eq!(freed, [101, 103, 102]);
        for k in 0..100 {
            freed.extend(enqueue(&mut q, (k % MAXP as isize, 10 + k as u32, 1000 + k as usize)));
        }
        assert!(q.len() <= MAXP);
        // a closed pane's waiting lookup is taken out; nothing for a pane without one
        let before = q.len();
        assert_eq!(unqueue(&mut q, 5), Some(1085));
        assert_eq!((unqueue(&mut q, 5), q.len()), (None, before - 1));
        // every PIDL handed in is either still queued or came back, never both, never twice
        let mut all: Vec<usize> = freed.iter().copied().chain([1085]).chain(q.iter().map(|r| r.2)).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!((n, all.len()), (106, 106));
        // results: only the pane's current lookup counts
        assert_eq!(take_icon(6, 5, 3), None); // an older folder's, arriving late
        assert_eq!(take_icon(0, 0, 3), None); // nothing asked yet (or the clone failed: the icon stays -1)
        assert_eq!(take_icon(7, 5, 3), None); // a closed pane's, sent to a new one with the same HWND value
        assert_eq!(take_icon(6, 6, 3), Some(3));
        assert_eq!(take_icon(6, 6, -1), Some(-1)); // lookup failed: no icon
        // tickets skip 0 when they wrap
        let t = AtomicU32::new(u32::MAX);
        assert_eq!((next_ticket(&t), next_ticket(&t)), (u32::MAX, 1));
    }

    #[test]
    fn window_fits_work_area() {
        let r = |left, top, right, bottom| RECT { left, top, right, bottom };
        let work = r(0, 0, 1024, 728);
        let inside = r(100, 50, 900, 650);
        assert_eq!(fit(inside, work), inside);
        // the 1400×900 default on a 1024×768 screen
        assert_eq!(fit(r(26, 26, 1426, 926), work), work);
        // spilling right and down: same size, slid back in
        assert_eq!(fit(r(500, 400, 1300, 1000), work), r(224, 128, 1024, 728));
        // a secondary monitor left of the primary
        let left_mon = r(-1920, 0, 0, 1040);
        assert_eq!(fit(r(-2100, -50, -1300, 550), left_mon), r(-1920, 0, -1120, 600));
        assert_eq!(fit(r(-300, 100, 500, 700), left_mon), r(-800, 100, 0, 700));
        // taskbar on top or on the left: a saved rect that fits comes back unchanged, so it never creeps
        let mon = r(0, 0, 1920, 1080);
        for work in [r(0, 40, 1920, 1080), r(60, 0, 1920, 1080)] {
            for saved in [r(0, 0, 800, 600), r(100, 100, 900, 700), r(1060, 400, 1860, 1000), r(1120, 440, 1920, 1040)] {
                if saved.right + work.left <= work.right && saved.bottom + work.top <= work.bottom {
                    assert_eq!(fit_saved(saved, mon, work, 0), saved);
                }
            }
            // too big or off screen: in screen coordinates the result lies inside rcWork
            for saved in [r(-300, -200, 3000, 2000), r(1500, 900, 2300, 1500)] {
                let f = fit_saved(saved, mon, work, 0);
                let (dx, dy) = (work.left - mon.left, work.top - mon.top);
                assert_eq!(fit(r(f.left + dx, f.top + dy, f.right + dx, f.bottom + dy), work), r(f.left + dx, f.top + dy, f.right + dx, f.bottom + dy));
            }
        }
        assert_eq!(fit_saved(r(1500, 900, 2300, 1500), mon, r(0, 40, 1920, 1080), 0), r(1120, 440, 1920, 1040));
        // snapped left half / bottom-right quarter on a 1920x1040 work area: the 7 px invisible border past the left,
        // right and bottom edges stays within an 8 px slack, so the rect comes back unchanged
        let work = r(0, 0, 1920, 1040);
        for snapped in [r(-7, 0, 967, 1047), r(953, 520, 1927, 1047)] {
            assert_eq!(fit_saved(snapped, mon, work, 8), snapped);
            assert_ne!(fit_saved(snapped, mon, work, 0), snapped);
        }
        // no slack on top, and 200 px off screen still comes back in (#78)
        assert_eq!(fit_saved(r(100, -7, 900, 600), mon, work, 8), r(100, 0, 900, 607));
        assert_eq!(fit_saved(r(1500, 900, 2300, 1500), mon, work, 8), r(1128, 448, 1928, 1048));
        // taskbar on top: the same snapped rect in workspace coordinates also stays
        assert_eq!(fit_saved(r(-7, 0, 967, 1047), mon, r(0, 40, 1920, 1080), 8), r(-7, 0, 967, 1047));
    }

    #[test]
    fn snapped_rect_in_workspace_coordinates() {
        let r = |left, top, right, bottom| RECT { left, top, right, bottom };
        let mon = r(0, 0, 1920, 1080);
        // taskbar at the bottom: workspace and screen coordinates agree
        assert_eq!(placed_rect(r(-7, 0, 967, 1047), mon, r(0, 0, 1920, 1040)), r(-7, 0, 967, 1047));
        // taskbar on top / on the left: shifted back by its thickness, as rcNormalPosition is
        assert_eq!(placed_rect(r(-7, 40, 967, 1087), mon, r(0, 40, 1920, 1080)), r(-7, 0, 967, 1047));
        assert_eq!(placed_rect(r(53, 0, 1027, 1047), mon, r(60, 0, 1920, 1040)), r(-7, 0, 967, 1047));
        // a floating window: its rcNormalPosition, so nothing is remembered (arranged stays None)
        assert_eq!(placed_rect(r(100, 140, 900, 740), mon, r(0, 40, 1920, 1080)), r(100, 100, 900, 700));
        // a monitor left of the primary, snapped to its left half, and back at startup with the slack
        let (left, left_work) = (r(-1920, 0, 0, 1080), r(-1920, 0, 0, 1040));
        let p = placed_rect(r(-1927, 0, -953, 1047), left, left_work);
        assert_eq!(p, r(-1927, 0, -953, 1047));
        assert_eq!(fit_saved(p, left, left_work, 8), p);
    }

    #[test]
    fn session_card_hits() {
        for dpi in [96, 144] {
            let g = load_geo(5, dpi);
            for (k, (card, thumb, del)) in g.cards.iter().enumerate() {
                assert_eq!(del.right - del.left, 20 * dpi / 96);
                assert!(del.right == thumb.right && del.top == thumb.top && card.left < del.left);
                // ✕ wins over the card around it; the rest of the card loads
                for (x, y) in [(del.left, del.top), (del.right - 1, del.bottom - 1)] {
                    assert_eq!(load_hit(&g, x, y), Some(LOAD_DEL + k));
                }
                for (x, y) in [(del.left - 1, del.top), (del.left, del.bottom), (card.left, card.bottom - 1), (thumb.left + 1, thumb.top + 1)] {
                    assert_eq!(load_hit(&g, x, y), Some(k));
                }
            }
            assert_eq!(load_hit(&g, g.save_as.left + 1, g.save_as.top + 1), Some(LOAD_SAVE_AS));
            assert_eq!(load_hit(&g, g.folder.right - 1, g.folder.top + 1), Some(LOAD_FOLDER));
        }
    }

    // Ctrl+O's pick moves over the cards as load_geo lays them out (three to a row), then down to the links.
    #[test]
    fn session_card_keys() {
        let (s, f) = (LOAD_SAVE_AS, LOAD_FOLDER);
        // 7 cards: rows 0 1 2 / 3 4 5 / 6
        let step = |cur: Option<usize>, vk| load_step(cur, 7, vk);
        assert_eq!(step(Some(2), VK_RIGHT), Some(3)); // on into the next row
        assert_eq!(step(Some(3), VK_LEFT), Some(2));
        assert_eq!(step(Some(0), VK_LEFT), Some(0));
        assert_eq!(step(Some(6), VK_RIGHT), Some(6));
        assert_eq!(step(Some(1), VK_DOWN), Some(4));
        assert_eq!(step(Some(5), VK_DOWN), Some(6)); // nothing right below: the last card
        assert_eq!(step(Some(6), VK_DOWN), Some(s)); // under the last row: the links
        assert_eq!(step(Some(4), VK_UP), Some(1));
        assert_eq!(step(Some(1), VK_UP), Some(1));
        assert_eq!(step(Some(s), VK_RIGHT), Some(f));
        assert_eq!(step(Some(f), VK_LEFT), Some(s));
        assert_eq!(step(Some(s), VK_UP), Some(6));
        assert_eq!(step(Some(f), VK_UP), Some(6));
        assert_eq!(step(Some(f), VK_DOWN), Some(f));
        assert_eq!((step(Some(4), VK_HOME), step(Some(4), VK_END)), (Some(0), Some(6)));
        assert_eq!((step(None, VK_DOWN), step(None, VK_END)), (Some(0), Some(6))); // nothing picked yet
        assert_eq!(step(Some(8), VK_LEFT), Some(0)); // a card that's gone
        assert_eq!(step(Some(1), VK_F2), None); // not a key that moves
        // 3 cards: the third column's ↓ is the folder link, and back
        assert_eq!(load_step(Some(2), 3, VK_DOWN), Some(f));
        assert_eq!(load_step(Some(1), 3, VK_DOWN), Some(s));
        assert_eq!(load_step(Some(f), 3, VK_UP), Some(2));
        // no cards: only the links
        assert_eq!(load_step(None, 0, VK_DOWN), Some(s));
        assert_eq!(load_step(Some(s), 0, VK_UP), Some(s));
        assert_eq!(load_step(Some(s), 0, VK_RIGHT), Some(f));
        // opened with Ctrl+O: the session you're in, else the first card, else "+ Save as new"
        assert_eq!((load_first_pick(Some(4), 7), load_first_pick(None, 7), load_first_pick(Some(9), 7), load_first_pick(None, 0)), (4, 0, 0, s));
    }

    #[test]
    fn session_card_preview() {
        let within = |a: &RECT, b: &RECT| a.left >= b.left && a.top >= b.top && a.right <= b.right && a.bottom <= b.bottom;
        let apart = |a: &RECT, b: &RECT| a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top;
        for h in [vec![1], vec![2, 2], vec![1, 3], vec![4, 4, 4, 4], vec![3, 1, 2]] {
            let mut s = Session::default();
            s.set_heights(h.clone());
            s.tints[0] = 3;
            for dpi in [96, 120, 144, 192] {
                let k = |v: i32| v * dpi / 96;
                let thumb = load_geo(1, dpi).cards[0].1;
                let g = thumb_geo(thumb, &s, dpi);
                assert_eq!(g.len(), s.panes(), "{h:?} {dpi}");
                // margin clear of the "session you're in" frame; dpi-scaled gaps, kept in slot order
                let edge = RECT { left: thumb.left + k(2), top: thumb.top + k(2), right: thumb.right - k(2), bottom: thumb.bottom - k(2) };
                for (i, p) in g.iter().enumerate() {
                    assert!(within(&p.plate, &edge), "{h:?} {dpi} {i}");
                    for q in &g[i + 1..] {
                        assert!(apart(&p.plate, &q.plate), "{h:?} {dpi} {i}");
                    }
                    for part in [p.head, p.band, p.name].into_iter().flatten() {
                        assert!(within(&part, &p.plate) && part.bottom > part.top, "{h:?} {dpi} {i}");
                    }
                    // a name wherever there was one before (over 24 × 12, now scaled); the header gives way to it
                    let (w, ht) = (p.plate.right - p.plate.left, p.plate.bottom - p.plate.top);
                    assert_eq!(p.name.is_some(), w > k(24) && ht > k(12), "{h:?} {dpi} {i}");
                    if let (Some(hd), Some(n)) = (p.head, p.name) {
                        assert!(n.top >= hd.bottom && n.bottom - n.top > k(12));
                    }
                    assert_eq!(p.band.is_some(), i == 0, "{h:?} {dpi} {i}");
                }
                let (mut cols, mut i) = (Vec::new(), 0);
                for &n in &h {
                    cols.push(g[i].plate.left);
                    for r in 1..n {
                        assert_eq!(g[i + r].plate.top - g[i + r - 1].plate.bottom, k(2));
                        assert_eq!(g[i + r].plate.left, g[i].plate.left);
                    }
                    i += n;
                }
                assert!(cols.windows(2).all(|c| c[0] < c[1]));
                // two-row columns have room for a header; four rows keep the names instead
                if h == [2, 2] {
                    assert!(g.iter().all(|p| p.head.is_some() && p.name.is_some()), "{dpi}");
                }
                if h == [4, 4, 4, 4] {
                    assert!(g.iter().all(|p| p.head.is_none() && p.name.is_some()), "{dpi}");
                    // the colour band alone doesn't cut into the name's rows
                    assert_eq!(g[0].name.map(|n| n.top), Some(g[0].plate.top), "{dpi}");
                }
            }
        }
        // a tiny preview: nothing spills, no header taller than its pane
        let mut s = Session::default();
        s.set_heights(vec![4, 4, 4, 4]);
        s.tints = vec![1; MAXP];
        for (w, ht) in [(0, 0), (5, 5), (20, 12), (40, 30)] {
            let r = RECT { left: 10, top: 10, right: 10 + w, bottom: 10 + ht };
            for p in thumb_geo(r, &s, 96) {
                for part in [p.head, p.band, p.name].into_iter().flatten() {
                    assert!(within(&part, &p.plate) && part.bottom > part.top && part.right > part.left, "{w} {ht}");
                }
            }
        }
    }

    #[test]
    fn click_targets() {
        let apart = |a: &RECT, b: &RECT| a.right <= b.left || b.right <= a.left || a.bottom <= b.top || b.bottom <= a.top;
        let mid = |r: &RECT| ((r.left + r.right) / 2, (r.top + r.bottom) / 2);
        // splitters: tree shown, one pane on the left, three stacked on the right
        let h = [1, 3];
        let mk = |zoom| {
            let (top, bot, gx) = (34, 576, 256);
            let ys = h.iter().map(|&n| cells(top, bot - top, &vec![1.0 / n as f64; n], 8)).collect();
            Geo { w: 1000, h: 600, top, bot, pad: 8, gap: 8, side: 240, tree: true, gx, xs: cells(gx, 1000 - 8 - gx, &[0.5, 0.5], 8), ys, zoom }
        };
        let g = mk(false);
        let (c0, c1) = (g.xs[0].1, g.xs[1].0);
        assert_eq!(hit(&g, 250, 100), Some(Hit::Side));
        assert_eq!((hit(&g, c0, 100), hit(&g, c1 - 1, 100)), (Some(Hit::Col(0)), Some(Hit::Col(0))));
        assert_eq!((hit(&g, c0 - 1, 100), hit(&g, c1, 100)), (None, None));
        for y in [g.top - 1, g.bot] {
            assert_eq!((hit(&g, 250, y), hit(&g, c0, y)), (None, None));
        }
        // the gap between two stacked panes is that column's row splitter, and nothing in the other column
        for i in 1..3 {
            let (a, b) = (cell_rect(&g, &h, i), cell_rect(&g, &h, i + 1));
            assert!(a.bottom < b.top);
            for y in a.bottom..b.top {
                assert_eq!(hit(&g, a.left + 5, y), Some(Hit::Row(1, i - 1)));
                assert_eq!(hit(&g, g.xs[0].0 + 5, y), None);
            }
        }
        // a click inside a pane never grabs a splitter
        for i in 0..4 {
            let r = cell_rect(&g, &h, i);
            for x in (r.left..r.right).step_by(7).chain([r.right - 1]) {
                for y in (r.top..r.bottom).step_by(7).chain([r.bottom - 1]) {
                    assert_eq!(hit(&g, x, y), None, "pane {i} at {x},{y}");
                }
            }
        }
        let z = mk(true);
        assert_eq!((hit(&z, 250, 100), hit(&z, c0, 100), hit(&z, g.xs[1].0 + 5, g.ys[1][0].1)), (Some(Hit::Side), None, None));

        // favourites: the ✕ is the row's right s(32), rows tile without overlap, the frame is nobody's
        for dpi in [96, 144] {
            let s32 = 32 * dpi / 96;
            for n in [0, 1, 5] {
                let g = fav_geo(n, 0, i32::MAX, dpi);
                assert_eq!((g.rows.len(), g.first, g.n, g.bar), (n, 0, n, None));
                if n == 0 {
                    assert_eq!(g.h, fav_geo(1, 0, i32::MAX, dpi).h); // room for the "no favourites" line
                }
                // a limit the list fits in changes nothing
                let fit = fav_geo(n, 0, g.h, dpi);
                assert_eq!((fit.h, fit.rows.len(), fit.bar), (g.h, n, None));
                assert_eq!((fav_hit(&g, 0, 0), fav_hit(&g, g.w - 1, g.h / 2), fav_hit(&g, g.w / 2, g.h - 1)), (None, None, None));
                for (k, (row, x)) in g.rows.iter().enumerate() {
                    assert!(row.left > 0 && row.right < g.w && row.top > 0 && row.bottom < g.h);
                    assert_eq!((x.right - x.left, x.right, x.top, x.bottom), (s32, row.right, row.top, row.bottom));
                    for (px, py) in [(row.right - 1, row.top), (row.right - s32, row.bottom - 1)] {
                        assert_eq!(fav_hit(&g, px, py), Some((k, true)));
                    }
                    for (px, py) in [(row.right - s32 - 1, row.top), (row.left, row.bottom - 1)] {
                        assert_eq!(fav_hit(&g, px, py), Some((k, false)));
                    }
                    assert_eq!(fav_hit(&g, row.left, row.bottom), (k + 1 < n).then_some((k + 1, false)));
                    assert!(g.rows[k + 1..].iter().all(|(o, _)| apart(row, o)));
                }
            }
        }

        // session cards: centres load, gaps between cards don't, the two links stay clear of every card
        for dpi in [96, 144] {
            for n in [0, 3, 4, LOAD_MAX] {
                let g = load_geo(n, dpi);
                let mut all: Vec<RECT> = g.cards.iter().map(|c| c.0).collect();
                all.extend([g.save_as, g.folder]);
                for (k, r) in all.iter().enumerate() {
                    assert!(r.left >= 0 && r.top >= 0 && r.right <= g.w && r.bottom <= g.h);
                    assert!(all[k + 1..].iter().all(|o| apart(r, o)));
                }
                for (k, (card, ..)) in g.cards.iter().enumerate() {
                    let (x, y) = mid(card);
                    assert_eq!(load_hit(&g, x, y), Some(k));
                    if let Some((next, ..)) = g.cards.get(k + 1).filter(|_| k % 3 < 2) {
                        assert_eq!(load_hit(&g, (card.right + next.left) / 2, y), None);
                    }
                    if let Some((below, ..)) = g.cards.get(k + 3) {
                        assert_eq!(load_hit(&g, x, (card.bottom + below.top) / 2), None);
                    }
                }
                let ((sx, sy), (fx, fy)) = (mid(&g.save_as), mid(&g.folder));
                assert_eq!((load_hit(&g, sx, sy), load_hit(&g, fx, fy)), (Some(LOAD_SAVE_AS), Some(LOAD_FOLDER)));
            }
        }

        // layout picker: every cell answers to itself, a gap goes to the cell left / above, outside only when dragging
        for dpi in [96, 144] {
            let g = pick_geo(dpi);
            let end = g.pad + 4 * g.cell + 3 * g.gap;
            for r in 0..4 {
                for c in 0..4 {
                    let cell = pick_cell(&g, r, c);
                    let (x, y) = mid(&cell);
                    for clamp in [false, true] {
                        assert_eq!(pick_at(&g, x, y, clamp), Some((r, c)));
                    }
                    if c < 3 {
                        assert_eq!(pick_at(&g, cell.right + g.gap - 1, y, false), Some((r, c)));
                    }
                    if r < 3 {
                        assert_eq!(pick_at(&g, x, cell.bottom + g.gap - 1, false), Some((r, c)));
                    }
                }
            }
            let y = mid(&pick_cell(&g, 1, 0)).1;
            assert_eq!((pick_at(&g, g.pad - 1, y, false), pick_at(&g, end, y, false), pick_at(&g, g.pad, end, false)), (None, None, None));
            assert_eq!((pick_at(&g, -50, y, true), pick_at(&g, g.w + 50, y, true)), (Some((1, 0)), Some((1, 3))));
            assert_eq!((pick_at(&g, -50, -50, true), pick_at(&g, g.pad, g.h + 50, true)), (Some((0, 0)), Some((3, 0))));
            for (i, p) in g.presets.iter().enumerate() {
                assert!(p.top >= g.label.bottom && p.left >= 0 && p.right <= g.w && p.bottom <= g.h);
                assert!(g.presets[i + 1..].iter().all(|o| apart(p, o)));
            }
        }

        // header drop: LINK, else COPY, never MOVE (the shell would delete the source folder)
        let (n, c, m, l) = (DROPEFFECT_NONE, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_LINK);
        assert_eq!(drop_pick(m), n);
        assert_eq!(drop_pick(DROPEFFECT(c.0 | m.0)), c);
        assert_eq!(drop_pick(DROPEFFECT(l.0 | c.0 | m.0)), l);
        assert_eq!(drop_pick(n), n);
        for bits in 0..8u32 {
            for extra in [0, DROPEFFECT_SCROLL.0] {
                assert_eq!(drop_pick(DROPEFFECT(bits | extra)).0 & m.0, 0);
            }
        }
    }

    #[test]
    fn session_names() {
        assert_eq!(session_stem_ok("  my work "), Some("my work".into()));
        for ok in ["v1.2", "CONSOLE", "COM10", "lpt", "세션"] {
            assert_eq!(session_stem_ok(ok).as_deref(), Some(ok));
        }
        for bad in ["", "   ", "a/b", r"a\b", "a:b", "a*", "a?", "a\"b", "a<b", "a>b", "a|b", "a\tb", "x.", "CON", "con", "Aux ", "nul.x", "PRN", "COM1", "lpt9"] {
            assert_eq!(session_stem_ok(bad), None, "{bad:?}");
        }
    }

    // Every place that names a file or folder turns away the same Windows device names, and lets look-alikes through.
    #[test]
    fn reserved_names_one_table() {
        let t = SYSTEMTIME { wYear: 2026, wMonth: 10, wDay: 21, wHour: 15, wMinute: 30, wSecond: 2, ..Default::default() };
        let (nf, dated) = ("New folder", "Clipboard text 2026-10-21 153002.txt");
        let device_msg = name_problem("CON");
        let reserved = ["CON", "con", "Aux ", "nul.txt", "COM1", "lpt9.log", "COM¹", "LPT³.txt", "CONIN$", "CONOUT$", "con .x"];
        let allowed = ["COM0", "COM10", "LPT10", "CONSOLE", "conx", "cone.txt", "세션"];
        for (name, bad) in reserved.iter().map(|n| (*n, true)).chain(allowed.iter().map(|n| (*n, false))) {
            assert_eq!(rename_rules::reserved_name(name), bad, "reserved_name {name:?}");
            assert_eq!(rename_rules::invalid(name).is_some(), bad, "invalid {name:?}");
            assert_eq!(text_name(false, name, &t, 1) == dated, bad, "text_name {name:?}");
            assert_eq!(session_stem_ok(name).is_none(), bad, "session_stem_ok {name:?}");
            assert_eq!(name_problem(name).is_some(), bad, "name_problem {name:?}");
            assert_eq!(group_base(&[name], nf) == nf, bad, "group_base {name:?}");
            // the reason itself only where no trailing space or dot comes first
            if bad && !name.ends_with([' ', '.']) {
                assert_eq!(rename_rules::invalid(name), Some(rename_rules::Status::Reserved), "{name:?}");
                assert_eq!(name_problem(name), device_msg, "{name:?}");
            }
        }
    }

    #[test]
    fn session_link_after_delete_or_rename() {
        assert_eq!(file_after_delete("A", "A", true, true), Some(String::new())); // the next Ctrl+S asks for a name
        assert_eq!(file_after_delete("A", "B", true, true), None); // B opened while A's delete was confirmed: B stays
        assert_eq!(file_after_delete("A", "B", true, false), None);
        assert_eq!(file_after_delete("A", "A", true, false), None); // cancelled
        assert_eq!(file_after_delete("A", "A", false, true), None); // another session deleted
        assert_eq!(file_after_rename("A", "A", true, "N"), Some("N".into()));
        assert_eq!(file_after_rename("A", "B", true, "N"), None);
        assert_eq!(file_after_rename("A", "A", false, "N"), None);
        assert_eq!(file_after_rename("", "", false, "N"), None);
    }

    #[test]
    fn session_rename_never_replaces() {
        let d = std::env::temp_dir().join(format!("grids-rename-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let read = |n: &str| std::fs::read_to_string(d.join(n)).unwrap();
        std::fs::write(d.join("a.grids"), "A").unwrap();
        std::fs::write(d.join("b.grids"), "B").unwrap();
        assert!(rename_session(&d.join("a.grids"), "b").is_err());
        assert_eq!((read("a.grids"), read("b.grids")), ("A".into(), "B".into()));
        // case only
        assert_eq!(rename_session(&d.join("a.grids"), "A").unwrap(), d.join("A.grids"));
        let names = || std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert!(names().contains(&"A.grids".into()) && !names().contains(&"a.grids".into()));
        assert_eq!(read("A.grids"), "A");
        assert_eq!(rename_session(&d.join("A.grids"), "A").unwrap(), d.join("A.grids"));
        // sessions from before the rename keep .qdr
        std::fs::write(d.join("old.qdr"), "Q").unwrap();
        assert_eq!(rename_session(&d.join("old.qdr"), "new").unwrap(), d.join("new.qdr"));
        assert!(!d.join("old.qdr").exists());
        assert_eq!(read("new.qdr"), "Q");
        _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn swap_slots_and_order() {
        let mut s = Session { paths: ["A", "B", "C", "D"].iter().map(|p| p.to_string()).chain(vec![String::new(); MAXP - 4]).collect(), ..Default::default() };
        (s.views[0], s.cols[0], s.tints[0], s.active) = ((4, 16), "x".into(), 2, 0);
        let mut t = s.clone();
        t.swap_slots(0, 2);
        assert_eq!((&t.paths[..4], t.views[2], t.cols[2].as_str(), t.tints[2], t.active), (&["C", "B", "A", "D"].map(String::from)[..], (4, 16), "x", 2, 2));
        t.swap_slots(1, 1);
        assert_eq!(t.paths[1], "B");
        assert_eq!(t.active, 2);
        // the order goes through grids.ini; anything but a full permutation of the slots is dropped
        let mut cur = s.clone();
        swap_in(&mut cur, None, 0, 2);
        assert_eq!(cur.order[..4], [2, 1, 0, 3]);
        assert_eq!(Session::parse(&cur.to_ini()).order, cur.order);
        let ord = |v: &str| Session::parse(&format!("order={v}\n")).order;
        let ok = (0..MAXP).rev().map(|k| k.to_string()).collect::<Vec<_>>().join(",");
        assert_eq!(ord(&ok).len(), MAXP);
        for bad in ["0,0,2,3,4,5,6,7,8,9,10,11,12,13,14,15", "0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,16", "1,0,2,3", "", "x"] {
            assert!(ord(bad).is_empty(), "{bad}");
        }
        assert!(!s.to_ini().contains("order="));
        // never in a session file, and a loaded one starts as saved
        assert!(!cur.snapshot().contains("order="));
        let mut l = cur.clone();
        l.take_layout(&s);
        assert!(l.order.is_empty());
        // swapping back is no move at all
        swap_in(&mut cur, None, 2, 0);
        assert!(cur.order.is_empty() && cur.paths == s.paths);
        // a layout change's permutation moves the same things; done twice, nothing moved
        let mut perm: [usize; MAXP] = std::array::from_fn(|k| k);
        perm.swap(0, 2);
        let (mut a, mut b) = (s.clone(), s.clone());
        swap_in(&mut a, None, 0, 2);
        permute_in(&mut b, None, &perm);
        assert_eq!(a, b);
        permute_in(&mut b, None, &perm);
        assert_eq!(b, s);
    }

    #[test]
    fn swap_dot_survives_restart() {
        // a session file in the temp folder, and one that has been deleted
        let dir = std::env::temp_dir().join(format!("grids-swap-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gone = dir.join("gone.grids");
        std::fs::write(&gone, "cols=2,2\n").unwrap();
        std::fs::remove_file(&gone).unwrap();
        // what check_dirty does each tick: fill the baseline's unknown columns, then compare
        let tick = |cur: &Session, base: &mut Session| {
            for i in 0..cur.panes() {
                if base.cols[i].is_empty() {
                    base.cols[i] = cur.cols[i].clone();
                }
            }
            session_dirty(cur, Some(base))
        };
        for differ in [false, true] {
            let mut s = Session { paths: ["A", "B", "C", "D"].iter().map(|p| p.to_string()).chain(vec![String::new(); MAXP - 4]).collect(), ..Default::default() };
            s.views = vec![(4, 16); MAXP];
            s.cols = vec!["c".into(); MAXP];
            if differ {
                (s.views[0], s.cols[0], s.tints[0]) = ((1, 48), "x".into(), 3);
                (s.views[2], s.cols[2]) = ((6, 96), "y".into());
            }
            let file = dir.join("s.grids");
            std::fs::write(&file, s.snapshot()).unwrap();
            for path in [file.clone(), PathBuf::new(), gone.clone()] {
                let mut cur = Session { file: path.to_string_lossy().into_owned(), ..s.clone() };
                let mut base = cur.clone();
                assert!(!tick(&cur, &mut base));
                swap_in(&mut cur, Some(&mut base), 0, 2);
                assert!(!layout_differs(&cur, &base) && tick(&cur, &mut base));
                // restart: grids.ini back in, the baseline from the session file (if there is one) in this order
                let mut cur = Session::parse(&cur.to_ini());
                let mut base = startup_base(&cur, std::fs::read_to_string(&cur.file).ok().as_deref());
                assert!(!layout_differs(&cur, &base), "{differ} {path:?}");
                assert!(tick(&cur, &mut base) && tick(&cur, &mut base), "{differ} {path:?}");
                let mut kept = (cur.clone(), base.clone());
                swap_in(&mut cur, Some(&mut base), 2, 0);
                assert!(!tick(&cur, &mut base) && !tick(&cur, &mut base), "{differ} {path:?}");
                assert_eq!(cur.paths, s.paths);
                // a save: the file has them where they are now
                kept.0.order.clear();
                kept.1 = kept.0.clone();
                assert!(!tick(&kept.0, &mut kept.1));
            }
        }
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tabs_model() {
        let mut s = Session::default();
        s.paths[0] = r"C:\a".into();
        assert_eq!(s.tab_list(0), vec![r"C:\a"]);
        assert_eq!(s.close_tab(0, 0), None); // the last tab stays
        s.add_tab(0);
        assert_eq!((s.tab_list(0).len(), s.tab[0]), (2, 1));
        s.paths[0] = r"C:\b".into(); // the new tab browsed on
        assert_eq!(s.tab_list(0), vec![r"C:\a", r"C:\b"]);
        assert_eq!(s.pick_tab(0, 0).as_deref(), Some(r"C:\a"));
        assert_eq!(s.pick_tab(0, 0), None); // already current
        s.paths[0] = r"C:\a".into();
        s.add_tab(0); // a, a, b: a copy right after the current one
        assert_eq!((s.tab_list(0), s.tab[0]), (vec![r"C:\a".to_string(), r"C:\a".into(), r"C:\b".into()], 1));
        assert_eq!(s.close_tab(0, 2), None); // another tab: no browsing
        assert_eq!(s.tab[0], 1);
        assert_eq!(s.close_tab(0, 1).as_deref(), Some(r"C:\a")); // the current one: its neighbour is browsed to
        assert!(s.tabs[0].is_empty() && s.tab[0] == 0);

        // closing the current tab moves right, or left at the end
        s.tabs[0] = vec!["x".into(), "y".into(), "z".into()];
        s.tab[0] = 1;
        s.paths[0] = "y".into();
        assert_eq!(s.close_tab(0, 1).as_deref(), Some("z"));
        assert_eq!(s.tab[0], 1);
        s.paths[0] = "z".into();
        assert_eq!(s.close_tab(0, 1).as_deref(), Some("x"));
        assert!(s.tabs[0].is_empty());
        // a tab before the current one shifts it left
        s.tabs[0] = vec!["x".into(), "y".into(), "z".into()];
        (s.tab[0], s.paths[0]) = (2, "z".into());
        assert_eq!((s.close_tab(0, 0), s.tab[0]), (None, 1));

        // duplicate: a copy right after the tab, made current; browsing only when it was another tab
        let mut s = Session::default();
        s.paths[0] = "a".into();
        assert_eq!(s.dup_tab(0, 0), None); // a single tab: same folder
        assert_eq!((s.tab_list(0), s.tab[0]), (vec!["a".to_string(), "a".into()], 1));
        s.tabs[0] = vec!["x".into(), "y".into(), "z".into()];
        (s.tab[0], s.paths[0]) = (2, "z".into());
        assert_eq!(s.dup_tab(0, 0).as_deref(), Some("x"));
        assert_eq!((s.tab_list(0).len(), s.tab[0]), (4, 1));
        assert_eq!(s.tabs[0], vec!["x", "x", "y", "z"]);
        assert_eq!(s.dup_tab(0, 9), None); // no such tab
        assert_eq!(s.tab[0], 1);
    }

    // Ctrl+Shift+T: a closed tab back at its place (or after the current one), made current
    #[test]
    fn tabs_reopen() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // closed from the middle, back at the same place; the current tab's stale entry takes the folder on display
        let mut s = Session::default();
        s.tabs[0] = v(&["x", "old", "z"]);
        (s.tab[0], s.paths[0]) = (1, "y".into());
        assert_eq!(s.reopen_tab(0, Some(1), "q").as_deref(), Some("q"));
        assert_eq!((s.tabs[0].clone(), s.tab[0]), (v(&["x", "q", "y", "z"]), 1));
        // at the end (k == len) is a place; past it: right after the current one
        let mut s = Session::default();
        s.tabs[0] = v(&["x", "y"]);
        (s.tab[0], s.paths[0]) = (0, "x".into());
        assert_eq!(s.reopen_tab(0, Some(2), "q").as_deref(), Some("q"));
        assert_eq!((s.tabs[0].clone(), s.tab[0]), (v(&["x", "y", "q"]), 2));
        s.paths[0] = "q".into();
        assert_eq!(s.reopen_tab(0, Some(9), "r").as_deref(), Some("r"));
        assert_eq!((s.tabs[0].clone(), s.tab[0]), (v(&["x", "y", "q", "r"]), 3));
        // one tab (none saved, a stale tab number): the reopened one after it; the same folder: nothing to browse
        let mut s = Session::default();
        (s.paths[0], s.tab[0]) = (r"C:\a".into(), 5);
        assert_eq!(s.reopen_tab(0, None, r"c:\A"), None);
        assert_eq!((s.tabs[0].clone(), s.tab[0]), (v(&[r"C:\a", r"c:\A"]), 1));
        let mut s = Session::default();
        s.paths[0] = "a".into();
        assert_eq!(s.reopen_tab(0, Some(0), "b").as_deref(), Some("b"));
        assert_eq!((s.tabs[0].clone(), s.tab[0]), (v(&["b", "a"]), 0));

        // the list keeps the newest CLOSED_TABS_MAX, newest last
        let mut list = Vec::new();
        for n in 0..CLOSED_TABS_MAX + 5 {
            note_closed(&mut list, ClosedTab { pane: 0, at: n, path: format!("p{n}") });
        }
        assert_eq!(list.len(), CLOSED_TABS_MAX);
        assert_eq!((list[0].at, list.last().map(|t| t.at)), (5, Some(CLOSED_TABS_MAX + 4)));
    }

    // Ctrl+Enter / Ctrl+Shift+Enter: folders as tabs right after the current one
    #[test]
    fn tabs_open_from_list() {
        let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // one tab (none saved, a stale tab number): it stays current and keeps its folder
        let mut s = Session::default();
        (s.paths[0], s.tab[0]) = ("a".into(), 5);
        assert_eq!(s.open_tabs(0, &v(&["p", "q"]), false), None);
        assert_eq!((s.tabs[0].clone(), s.tab[0], s.paths[0].as_str()), (v(&["a", "p", "q"]), 0, "a"));
        // a middle tab: in order after it; its stale entry takes the folder on display
        s.tabs[0] = v(&["x", "old", "z"]);
        (s.tab[0], s.paths[0]) = (1, "y".into());
        assert_eq!(s.open_tabs(0, &v(&["p", "q"]), false), None);
        assert_eq!((s.tabs[0].clone(), s.tab[0]), (v(&["x", "y", "p", "q", "z"]), 1));
        // the last tab, switching: the first new one is current and browsed to; the display waits for on_nav
        s.tabs[0] = v(&["x", "y"]);
        s.tab[0] = 1;
        assert_eq!(s.open_tabs(0, &v(&["p"]), true).as_deref(), Some("p"));
        assert_eq!((s.tabs[0].clone(), s.tab[0], s.paths[0].as_str()), (v(&["x", "y", "p"]), 2, "y"));
        // nothing to open: nothing changes
        assert_eq!(s.open_tabs(0, &[], true), None);
        assert_eq!((s.tabs[0].len(), s.tab[0]), (3, 2));
        // at most OPEN_TABS_MAX at a time
        let mut s = Session::default();
        s.paths[0] = "a".into();
        let many: Vec<String> = (0..20).map(|k| format!("d{k}")).collect();
        s.open_tabs(0, &many, false);
        assert_eq!(s.tab_list(0).len(), 1 + OPEN_TABS_MAX);
        assert_eq!(s.tabs[0][1..], many[..OPEN_TABS_MAX]);
        // a path a tab can't keep
        assert!(tab_path_ok(r"C:\Users\Public") && tab_path_ok("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"));
        for bad in ["", r"C:\a|b", "C:\\a\nb", "C:\\a\rb"] {
            assert!(!tab_path_ok(bad), "{bad:?}");
        }
        assert_eq!(opened_tabs_note(1, false), "Opened 1 tab · Ctrl+PgDn to go there");
        assert_eq!(opened_tabs_note(3, false), "Opened 3 tabs · Ctrl+PgDn to go there");
        assert_eq!(opened_tabs_note(3, true), "탭 3개를 열었습니다 · Ctrl+PgDn으로 이동");
    }

    #[test]
    fn tabs_move() {
        // one tab: nothing to move, nothing indexed
        let mut s = Session::default();
        s.paths[0] = "a".into();
        assert!(!s.move_tab(0, 0, 1) && !s.move_tab(0, 0, -1));
        assert!(s.tabs[0].is_empty());
        // x y z, y current (its entry stale: the pane is still showing "Y" while a switch is on its way)
        s.tabs[0] = vec!["x".into(), "stale".into(), "z".into()];
        (s.tab[0], s.paths[0]) = (1, "Y".into());
        assert!(s.move_tab(0, 1, 1)); // the current one goes right
        assert_eq!((s.tabs[0].clone(), s.tab[0]), (vec!["x".to_string(), "z".into(), "stale".into()], 2));
        assert_eq!(s.tab_list(0), vec!["x", "z", "Y"]); // paths[0] stays in the current tab, no other one gets it
        assert!(!s.move_tab(0, 2, 1)); // past the end: no wrap
        assert!(s.move_tab(0, 1, 1)); // its neighbour moves onto it: the current one steps back
        assert_eq!((s.tab_list(0), s.tab[0]), (vec!["x".to_string(), "Y".into(), "z".into()], 1));
        assert!(!s.move_tab(0, 0, -1)); // before the first: nothing (no usize underflow)
        assert!(s.move_tab(0, 0, 1)); // a tab other than the current one
        assert_eq!((s.tab_list(0), s.tab[0]), (vec!["Y".to_string(), "x".into(), "z".into()], 0));
        assert!(s.move_tab(0, 2, -1)); // the last one left; the current one stays put
        assert_eq!((s.tab_list(0), s.tab[0]), (vec!["Y".to_string(), "z".into(), "x".into()], 0));
        assert!(!s.move_tab(0, 9, -1) && !s.move_tab(0, 1, 0)); // no such tab, no step
        assert_eq!(s.paths[0], "Y"); // no browsing
        // the new order is what the session saves
        s.paths[0] = r"C:\a".into();
        s.tabs[0] = vec![r"C:\a".into(), r"D:\b".into(), r"E:\c".into()];
        s.tab[0] = 0;
        assert!(s.move_tab(0, 0, 1));
        assert!(s.to_ini().contains("b0=1|D:\\b|C:\\a|E:\\c\n"), "{}", s.to_ini());
    }

    #[test]
    fn tabs_ini() {
        let mut s = Session::default();
        s.tabs_on = true;
        (s.paths[2], s.tabs[2], s.tab[2]) = (r"D:\now".into(), vec![r"C:\a".into(), "old".into(), "::{x}".into()], 1);
        let t = s.to_ini();
        assert!(t.contains("tabs=1\n") && t.contains("b2=1|C:\\a|D:\\now|::{x}\n"), "{t}");
        let back = Session::parse(&t);
        assert!(back.tabs_on);
        assert_eq!((back.tab_list(2), back.tab[2]), (vec![r"C:\a".to_string(), r"D:\now".into(), "::{x}".into()], 1));
        // one tab, a bad index, an older ini: no tabs
        assert!(Session::parse("p0=C:\\a\nb0=0|C:\\a\nb1=9|x|y\n").tabs[0].is_empty());
        assert_eq!(Session::parse("b1=9|x|y\n").tab[1], 1);
        assert!(!Session::parse("side=300\n").tabs_on);
        // a new install (no grids.ini) starts with the strips on; a saved tabs=0 stays off
        assert!(Session::default().tabs_on);
        assert!(!Session::parse("tabs=0\n").tabs_on && Session::parse("tabs=1\n").tabs_on);
        assert!(!Session::parse(&Session { tabs_on: false, ..Session::default() }.to_ini()).tabs_on);
        // tabs move with their pane
        let mut m = back.clone();
        m.swap_slots(2, 0);
        assert_eq!((m.tabs[0].len(), m.tab[0], m.tabs[2].len()), (3, 1, 0));
        let mut l = Session::default();
        l.take_layout(&back);
        assert_eq!(l.tabs[2].len(), 3);
    }

    #[test]
    fn tab_strip() {
        let strip = RECT { left: 2, top: 30, right: 402, bottom: 56 };
        let (tabs, plus) = tab_rects(strip, 2, 96);
        assert_eq!((tabs[0].left, tabs[0].right - tabs[0].left, tabs[1].right, plus.left, plus.right), (6, 170, 346, 346, 370));
        let (tabs, plus) = tab_rects(strip, 10, 96); // squeezed evenly, '+' still inside
        assert!(tabs.iter().all(|r| r.right - r.left == 36) && plus.right <= strip.right);
        assert!(tab_close_rect(tabs[0], 96).is_none() && tab_close_rect(RECT { left: 0, top: 0, right: 64, bottom: 20 }, 96).is_some());
    }

    // A tab's folder icon: only while the name keeps s(24) after it (up to the current tab's × with several tabs).
    #[test]
    fn tab_icons() {
        for dpi in [96, 144] {
            let s = |v: i32| v * dpi / 96;
            let tab = |w: i32| RECT { left: 10, top: 33, right: 10 + w, bottom: 33 + s(23) };
            for close in [false, true] {
                let end = if close { s(22) } else { s(8) };
                let least = s(28) + end + s(24); // narrowest tab with an icon
                assert!(tab_icon_rect(tab(least - 1), close, dpi).is_none(), "{dpi} {close}");
                let r = tab_icon_rect(tab(least), close, dpi).expect("wide enough");
                assert_eq!((r.left, r.right - r.left, r.bottom - r.top), (10 + s(8), s(16), s(16)), "{dpi} {close}");
                assert!((r.top - 33 - (33 + s(23) - r.bottom)).abs() <= 1); // centred top to bottom
                // the name after it (s(28) on) keeps s(24) before the ×, where there is one
                for w in [least, least + 1, s(170)] {
                    if tab_icon_rect(tab(w), close, dpi).is_some() {
                        let right = tab_close_rect(tab(w), dpi).filter(|_| close).map_or(tab(w).right - s(8), |c| c.left);
                        assert!(right - (10 + s(28)) >= s(24), "{dpi} {close} {w}");
                    }
                }
            }
            // one decision for the strip: every tab as wide, so none drops its icon alone
            for width in [200, 300, 400, 1200] {
                let strip = RECT { left: 2, top: 30, right: 2 + width, bottom: 30 + s(26) };
                for n in [1, 2, 3, 12] {
                    let (tabs, _) = tab_rects(strip, n, dpi);
                    let on: Vec<bool> = tabs.iter().map(|r| tab_icon_rect(*r, n > 1, dpi).is_some()).collect();
                    assert!(on.iter().all(|&b| b == on[0]), "{dpi} {width} {n}");
                }
            }
        }
        // 96 DPI: 170 px tabs have icons; ten in 400 px (36 px each) don't
        let strip = RECT { left: 2, top: 30, right: 402, bottom: 56 };
        assert!(tab_icon_rect(tab_rects(strip, 2, 96).0[0], true, 96).is_some());
        assert!(tab_icon_rect(tab_rects(strip, 10, 96).0[0], true, 96).is_none());
    }

    #[test]
    fn tab_icon_choice() {
        // current tab: the pane's icon, else the one saved for its folder
        assert_eq!(tab_icon_pick(true, 7, Some(3), 9, 1), 7);
        assert_eq!(tab_icon_pick(true, -1, Some(3), 9, 1), 3);
        // others: saved → drive kind → plain folder; -1s fall through
        assert_eq!(tab_icon_pick(false, 7, Some(3), 9, 1), 3);
        assert_eq!(tab_icon_pick(false, 7, None, 9, 1), 9);
        assert_eq!(tab_icon_pick(false, 7, Some(-1), -1, 1), 1);
        assert_eq!(tab_icon_pick(true, -1, None, -1, 1), 1);
        assert_eq!(tab_icon_pick(false, -1, None, -1, -1), -1); // nothing known: no icon
        assert_eq!(tab_icon_pick(false, -1, None, -1, i32::MIN), -1);
        // drive roots only as X:\ exactly
        assert_eq!(drive_root("d:\\"), Some(b'D'));
        for p in ["D:", "D:\\x", "\\\\srv\\share", "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", "", "1:\\"] {
            assert_eq!(drive_root(p), None, "{p}");
        }
    }

    #[test]
    fn tab_icon_memo() {
        let mut m = IconMemo::default();
        m.put("C:\\Users\\Public\\Downloads", 12);
        assert_eq!(m.get("c:\\users\\public\\DOWNLOADS"), Some(12)); // case-insensitive
        m.put("C:\\x", -1); // a failed lookup isn't kept
        m.put("", 4);
        assert_eq!((m.get("C:\\x"), m.len()), (None, 1));
        for k in 1..IconMemo::MAX {
            m.put(&format!("C:\\f{k}"), 5);
        }
        assert_eq!(m.len(), IconMemo::MAX);
        m.put("c:\\F1", 6); // already there: replaced, nothing dropped
        assert_eq!((m.len(), m.get("C:\\f1")), (IconMemo::MAX, Some(6)));
        m.put("C:\\one more", 8); // full: emptied first
        assert_eq!((m.len(), m.get("C:\\one more"), m.get("C:\\f1")), (1, Some(8), None));
    }

    // The tab strip's mouse: a click closes only where a × is painted, and a double click never closes.
    #[test]
    fn tab_clicks() {
        use TabClick as Click;
        use TabHit::*;
        for dpi in [96, 144] {
            let s = |v: i32| v * dpi / 96;
            // 400 px: 12 tabs are under 64 DIP each (no ×); 1200 px: they keep their ×
            for width in [400, 1200] {
                let strip = RECT { left: 2, top: 30, right: 2 + width, bottom: 30 + s(26) };
                let ym = strip.top + s(3) + (strip.bottom - strip.top - s(3)) / 2;
                for n in [1, 2, 3, 12] {
                    let (tabs, plus) = tab_rects(strip, n, dpi);
                    for cur in 0..n {
                        let hit = |x: i32, y: i32| tab_hit(strip, n, cur, x, y, dpi);
                        for (k, r) in tabs.iter().enumerate() {
                            assert_eq!(hit((r.left + r.right) / 2, ym), Some(Tab(k)), "{dpi} {width} {n} {cur} {k}");
                            // the pixels either side of a tab's edges
                            assert_eq!(hit(r.left, ym), Some(Tab(k)));
                            assert_eq!(hit(r.right - 1, ym), Some(Tab(k)));
                            let next = if k + 1 < n { Tab(k + 1) } else { Plus };
                            assert_eq!(hit(r.right, ym), Some(next));
                            if let Some(c) = tab_close_rect(*r, dpi) {
                                let at = ((c.left + c.right) / 2, ym);
                                let want = if k == cur && n > 1 { Close(k) } else { Tab(k) }; // the bug: other tabs' ends closed
                                assert_eq!(hit(at.0, at.1), Some(want), "{dpi} {width} {n} {cur} {k}");
                            }
                        }
                        // every pixel: Close exactly where the painter puts the ×
                        let mut closes = 0;
                        for y in strip.top..strip.bottom {
                            for x in strip.left..strip.right {
                                let drawn = tabs.iter().enumerate().find(|(k, r)| tab_close_at(**r, *k, cur, n, dpi).is_some_and(|c| inside(c, x, y)));
                                match hit(x, y) {
                                    Some(Close(k)) => {
                                        assert_eq!(drawn.map(|d| d.0), Some(k), "{dpi} {width} {n} {cur} ({x},{y})");
                                        closes += 1;
                                    }
                                    _ => assert!(drawn.is_none(), "{dpi} {width} {n} {cur} ({x},{y})"),
                                }
                            }
                        }
                        let wide = tabs[0].right - tabs[0].left >= s(64);
                        assert_eq!(closes > 0, n > 1 && wide, "{dpi} {width} {n}");
                        // '+', the empty strip after it and the band above the tabs
                        assert_eq!(hit((plus.left + plus.right) / 2, ym), Some(Plus));
                        if plus.right < strip.right {
                            assert_eq!(hit(plus.right, ym), Some(Strip));
                        }
                        let x0 = (tabs[0].left + tabs[0].right) / 2;
                        assert_eq!(hit(x0, strip.top), Some(Strip));
                        assert_eq!(hit(x0, strip.top + s(3) - 1), Some(Strip));
                        // outside the strip, or tabs off (a strip of no height)
                        assert_eq!(hit(x0, strip.top - 1), None);
                        assert_eq!(hit(x0, strip.bottom), None);
                        assert_eq!(hit(strip.left - 1, ym), None);
                        assert_eq!(tab_hit(RECT { bottom: strip.top, ..strip }, n, cur, x0, strip.top, dpi), None);
                        // double click on another tab's end: the first press picks it, the second
                        // lands on its fresh × and must not close it
                        for (k, r) in tabs.iter().enumerate().filter(|(k, _)| *k != cur) {
                            let Some(c) = tab_close_rect(*r, dpi) else { continue };
                            let at = ((c.left + c.right) / 2, ym);
                            let first = tab_action(hit(at.0, at.1).unwrap(), Click::Left);
                            assert_eq!(first, TabAct::Pick(k));
                            let now = tab_hit(strip, n, k, at.0, at.1, dpi).unwrap();
                            assert_eq!(now, Close(k));
                            assert_eq!(tab_action(now, Click::Double), TabAct::Pick(k));
                        }
                    }
                }
            }
        }
        // every hit × every click
        let table = [
            (Tab(2), Click::Left, TabAct::Pick(2)),
            (Tab(2), Click::Middle, TabAct::Close(2)),
            (Tab(2), Click::Double, TabAct::Pick(2)),
            (Close(1), Click::Left, TabAct::Close(1)),
            (Close(1), Click::Middle, TabAct::Close(1)),
            (Close(1), Click::Double, TabAct::Pick(1)),
            (Plus, Click::Left, TabAct::New),
            (Plus, Click::Middle, TabAct::Nothing),
            (Plus, Click::Double, TabAct::New),
            (Strip, Click::Left, TabAct::Nothing),
            (Strip, Click::Middle, TabAct::Nothing),
            (Strip, Click::Double, TabAct::New),
        ];
        for (hit, click, want) in table {
            assert_eq!(tab_action(hit, click), want, "{hit:?} {click:?}");
        }
    }

    // Files dropped on a pane's header: another tab of a folder on disk takes them (the shell copies / moves),
    // a shell place's tab refuses, everything else keeps #36 (open the folder in this pane).
    #[test]
    fn header_drop_places() {
        use TabHit::*;
        let tabs: Vec<String> = [r"C:\a", r"D:\b", r"\\nas\share\c", THIS_PC, RECYCLE_BIN, r"::{031E4825-7B94-4DC3-B131-E946B44C8DD5}\Documents.library-ms"]
            .map(String::from)
            .to_vec();
        let into = |k: usize| DropAt::Into(k, tabs[k].clone());
        for cur in 0..tabs.len() {
            for k in 0..tabs.len() {
                let want = match k {
                    _ if k == cur => DropAt::Open, // the current tab: #36 as before
                    0..=2 => into(k),
                    _ => DropAt::Refuse, // This PC, the Recycle Bin, a library: no folder on disk
                };
                assert_eq!(drop_at(Some(Tab(k)), cur, &tabs), want, "{cur} {k}");
                assert_eq!(drop_at(Some(Close(k)), cur, &tabs), want, "{cur} {k}");
            }
            // '+', the strip's empty end, outside the strip (fields, buttons) or tabs off: #36
            for hit in [Some(Plus), Some(Strip), None] {
                assert_eq!(drop_at(hit, cur, &tabs), DropAt::Open, "{cur} {hit:?}");
            }
        }
        // a stale hit past the list: nothing goes anywhere new
        assert_eq!(drop_at(Some(Tab(9)), 0, &tabs), DropAt::Open);
        // one tab (or tabs off: the list is the pane's folder alone)
        assert_eq!(drop_at(Some(Tab(0)), 0, &tabs[..1]), DropAt::Open);
        let disk = [r"C:\", r"c:\x", r"\\server\share", r"\\server"].map(disk_folder);
        assert_eq!(disk, [true; 4]);
        let not = ["", "C:", r"C:x", THIS_PC, r"\\?\C:\x", "shell:Downloads", r"1:\x"].map(disk_folder);
        assert_eq!(not, [false; 7]);
    }

    // The header's mouse shading names exactly what a click there hits: tab_hit in the strip (the × only
    // on the current tab, nothing on the bare strip), the filter icon where on_kind_icon opens the menu.
    #[test]
    fn head_hover_targets() {
        for dpi in [96, 144] {
            let s = |v: i32| v * dpi / 96;
            for width in [400, 1200] {
                let mut pg = pane_geo(width + 4, s(300), dpi);
                pg.tabs = RECT { left: s(2), top: s(30), right: width + 4 - s(2), bottom: s(30) + s(26) }; // tabs on
                let kind = filter_icon_rect(pg.filt, s(24)).unwrap();
                for n in [1, 2, 3, 12] {
                    for cur in 0..n {
                        for y in 0..pg.tabs.bottom + 2 {
                            for x in 0..width + 4 {
                                let want = if inside(kind, x, y) {
                                    Some(HeadHot::Kind)
                                } else {
                                    match tab_hit(pg.tabs, n, cur, x, y, dpi) {
                                        Some(TabHit::Tab(k)) => Some(HeadHot::Tab(k)),
                                        Some(TabHit::Close(k)) => Some(HeadHot::Close(k)),
                                        Some(TabHit::Plus) => Some(HeadHot::Plus),
                                        Some(TabHit::Strip) | None => None,
                                    }
                                };
                                let got = head_hot_at(&pg, n, cur, x, y, dpi);
                                assert_eq!(got, want, "{dpi} {width} {n} {cur} ({x},{y})");
                                if let Some(HeadHot::Close(k)) = got {
                                    assert!(k == cur && n > 1);
                                }
                            }
                        }
                    }
                }
                // tabs off: a strip of no height shades nothing below the bar
                let off = PaneGeo { tabs: RECT { bottom: pg.tabs.top, ..pg.tabs }, ..pane_geo(width + 4, s(300), dpi) };
                let (tabs, _) = tab_rects(pg.tabs, 3, dpi);
                assert_eq!(head_hot_at(&off, 3, 0, (tabs[1].left + tabs[1].right) / 2, pg.tabs.top + s(10), dpi), None);
            }
        }
        // a pill too narrow for the icon: shaded only inside it
        let mut pg = pane_geo(800, 300, 96);
        pg.filt.right = pg.filt.left + 10;
        assert_eq!(head_hot_at(&pg, 1, 0, pg.filt.left + 9, pg.filt.top + 5, 96), Some(HeadHot::Kind));
        assert_eq!(head_hot_at(&pg, 1, 0, pg.filt.left + 10, pg.filt.top + 5, 96), None);
    }

    // The hover shades must show against what they cover, and keep the text on them readable, in both themes:
    // another tab (on bg), the × (on card), '+' (on bg), the filter icon slot / the ring round the badge (on pill).
    #[test]
    fn head_hover_shades() {
        for (theme, p) in [("light", &LIGHT), ("dark", &DARK)] {
            for (name, under, t) in [
                ("tab", p.bg, press_tint(false, false)),
                ("×", p.card, press_tint(false, true)),
                ("+", p.bg, press_tint(false, true)),
                ("filter icon", p.pill, press_tint(false, false)),
                ("badge ring", p.pill, press_tint(false, true)),
            ] {
                let shade = blend(under, p.text, t);
                assert!(contrast(shade, under) >= 1.1, "{theme} {name}: shade {:.3}", contrast(shade, under));
                assert!(contrast(p.text, shade) >= 4.5, "{theme} {name}: text {:.2}", contrast(p.text, shade));
            }
            // the badge keeps its colour: each selection colour's ink on it is what palette_contrast checks
            for (name, c) in p.accents {
                assert!(contrast(on_color(c), c) >= 4.3, "{theme} {name}");
            }
        }
    }

    #[test]
    fn text_decoding() {
        assert_eq!(decode_text(b"\xEF\xBB\xBFhi"), "hi");
        assert_eq!(decode_text(&[0xFF, 0xFE, b'h', 0, b'i', 0]), "hi");
        assert_eq!(decode_text("한글".as_bytes()), "한글");
        assert_eq!(decode_text(&"한글".as_bytes()[..5]), "한"); // cut inside a character
        assert_eq!(decode_text(&[0xC7, 0xD1, 0xB1, 0xDB]), "한글"); // cp949
        assert!(looks_text(b"a\tb\r\n") && !looks_text(b"MZ\x90\x00\x03"));
        assert_eq!(ext_of(r"C:\a\b.TXT"), "txt");
        assert_eq!(ext_of(r"C:\a\Makefile"), "makefile");
        assert_eq!(ext_of(r"C:\a\.gitignore"), ".gitignore");
    }

    #[test]
    fn csv_parsing() {
        let (rows, more) = csv_rows("a,\"b,c\",\"d \"\"q\"\"\"\r\n1,2\n", ',', 100);
        assert_eq!(rows, vec![vec!["a", "b,c", "d \"q\""], vec!["1", "2"]]);
        assert!(!more);
        assert_eq!(csv_rows("x\n\"multi\nline\",y", ',', 100).0[1], vec!["multi\nline", "y"]);
        let (rows, more) = csv_rows("1\n2\n3\n", ',', 2);
        assert!(rows.len() == 2 && more);
        assert_eq!((csv_sep("a;b;c\n"), csv_sep("a,b;c\n")), (';', ','));
    }

    // Whether rows were left out at the limit: 1999, 2000 and 2001 rows, with and without the last line break.
    #[test]
    fn csv_limit_says_when_rows_left_out() {
        let lines = |n: usize, end: &str| (1..=n).map(|i| i.to_string()).collect::<Vec<_>>().join("\n") + end;
        for (n, end, cut) in [(1999, "\n", false), (2000, "\n", false), (2000, "\r\n\r\n", false), (2000, "", false), (2001, "\n", true), (2001, "", true)] {
            let (rows, more) = csv_rows(&lines(n, end), ',', 2000);
            assert_eq!((rows.len(), more), (n.min(2000), cut), "{n} rows ending {end:?}");
        }
        // a quoted line break after the limit is still a row left out
        assert!(csv_rows("a\nb\n\"x\ny\"", ',', 2).1);
    }

    // A ZIP made by hand: one stored entry, one deflated, a Korean name without the UTF-8 flag.
    pub(crate) fn zip_bytes(files: &[(&[u8], &[u8], bool, u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, data, deflate, flags) in files {
            let body = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let off = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            let mut local = vec![0x50, 0x4b, 3, 4, 20, 0];
            local.extend(flags.to_le_bytes());
            local.extend(method.to_le_bytes());
            local.extend([0u8; 8]); // time, date, crc
            local.extend((body.len() as u32).to_le_bytes());
            local.extend((data.len() as u32).to_le_bytes());
            local.extend((name.len() as u16).to_le_bytes());
            local.extend(0u16.to_le_bytes());
            local.extend(*name);
            out.extend(local);
            out.extend(&body);
            let mut c = vec![0x50, 0x4b, 1, 2, 20, 0, 20, 0];
            c.extend(flags.to_le_bytes());
            c.extend(method.to_le_bytes());
            c.extend([0u8; 8]);
            c.extend((body.len() as u32).to_le_bytes());
            c.extend((data.len() as u32).to_le_bytes());
            c.extend((name.len() as u16).to_le_bytes());
            c.extend([0u8; 12]); // extra, comment, disk, attrs
            c.extend(off.to_le_bytes());
            c.extend(*name);
            cd.extend(c);
        }
        let start = out.len() as u32;
        out.extend(&cd);
        out.extend([0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((cd.len() as u32).to_le_bytes());
        out.extend(start.to_le_bytes());
        out.extend([0, 0]);
        out
    }

    #[test]
    fn zip_reading() {
        let big = "<w:p><w:r><w:t>안녕 &amp; hi</w:t></w:r></w:p>".repeat(50);
        let z = zip_bytes(&[(b"a.txt", b"stored", false, 0x800), (b"word/document.xml", big.as_bytes(), true, 0x800), (&[0xC7, 0xD1, b'.', b't'], b"x", false, 0)]);
        let mut cur = std::io::Cursor::new(z);
        let es = zip_entries(&mut cur).unwrap();
        assert_eq!(es.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), vec!["a.txt", "word/document.xml", "한.t"]);
        assert_eq!(zip_read(&mut cur, &es[0], 100).unwrap(), b"stored");
        let doc = zip_read(&mut cur, zip_find(&es, "WORD/document.xml").unwrap(), 1 << 20).unwrap();
        assert_eq!(doc.len(), big.len());
        assert_eq!(zip_read(&mut cur, &es[1], 10).unwrap().len(), 10); // capped
        let text = xml_text(&decode_text(&doc), &["w:t"], &["w:p"], &[], &[], &[]);
        assert_eq!(text.lines().next(), Some("안녕 & hi"));
        assert!(zip_entries(&mut std::io::Cursor::new(b"not a zip at all".to_vec())).is_none());
    }

    #[test]
    fn xml_and_sheets() {
        let x = r#"<a:p><a:r><a:t>One</a:t></a:r><a:br/><a:r><a:t xml:space="preserve"> two</a:t></a:r></a:p><a:p><a:t/><a:t>&#xD55C;&lt;</a:t></a:p>"#;
        assert_eq!(xml_text(x, &["a:t"], &["a:p"], &[], &["a:br"], &[]), "One\n two\n한<\n");
        // tab-stop definitions are not tabs; a text box's Fallback copy is not shown twice
        let (tab, br, skip) = (&["w:tab", "a:tab", "text:tab"][..], &["w:br", "a:br"][..], &["w:tabs", "a:tabLst", "mc:Fallback"][..]);
        let w = r#"<w:p><w:pPr><w:tabs><w:tab w:val="right" w:pos="9016"/></w:tabs></w:pPr><w:r><w:t>목차</w:t></w:r></w:p><w:p><w:r><w:t>A</w:t><w:tab/><w:t>1</w:t></w:r></w:p>"#;
        assert_eq!(xml_text(w, &["w:t"], &["w:p"], tab, br, skip), "목차\nA\t1\n");
        let bx = r#"<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wps:txbx><w:txbxContent><w:p><w:r><w:t>글상자</w:t></w:r></w:p></w:txbxContent></wps:txbx></w:drawing></mc:Choice><mc:Fallback><w:pict><v:textbox><w:txbxContent><w:p><w:r><w:t>글상자</w:t></w:r></w:p></w:txbxContent></v:textbox></w:pict></mc:Fallback></mc:AlternateContent></w:r><w:r><w:t>뒤</w:t></w:r></w:p>"#;
        assert_eq!(xml_text(bx, &["w:t"], &["w:p"], tab, br, skip), "글상자\n뒤\n");
        let nested = r#"<w:p><mc:Fallback><w:t>a</w:t><mc:AlternateContent><mc:Fallback><w:t>b</w:t></mc:Fallback></mc:AlternateContent><w:t>c</w:t></mc:Fallback><w:t>끝</w:t></w:p>"#;
        assert_eq!(xml_text(nested, &["w:t"], &["w:p"], tab, br, skip), "끝\n");
        let s = "<a:p><a:pPr><a:tabLst><a:tab pos=\"914400\" algn=\"l\"/></a:tabLst></a:pPr><a:r><a:t>슬라이드\t글</a:t></a:r></a:p>";
        assert_eq!(xml_text(s, &["a:t"], &["a:p"], tab, br, skip), "슬라이드\t글\n");
        let e = r#"<w:p><w:pPr><w:tabs/></w:pPr><w:r><w:t>살아 있음</w:t></w:r></w:p>"#;
        assert_eq!(xml_text(e, &["w:t"], &["w:p"], tab, br, skip), "살아 있음\n");
        assert_eq!((cell_ref("A1"), cell_ref("AB12"), cell_ref("1A")), (Some((0, 0)), Some((27, 11)), None));
        assert_eq!(cell_ref(&format!("{}1", "A".repeat(20))), None); // too many letters: no cell, no overflow
        let ss = xlsx_strings("<sst><si><t>이름</t></si><si><r><t>A</t></r><r><t>B</t></r></si></sst>");
        assert_eq!(ss, vec!["이름", "AB"]);
        let sheet = r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row><row r="2"><c r="A2"><v>0.30000000000000004</v></c><c r="B2" t="inlineStr"><is><t>x</t></is></c></row>"#;
        let (all, more) = xlsx_rows(sheet, &ss, 100, 100);
        assert!(all == vec![vec!["이름", "", "AB"], vec!["0.3", "x"]] && !more);
        let (front, more) = xlsx_rows(sheet, &ss, 1, 2);
        assert!(front == vec![vec!["이름"]] && more); // limits, with values past them
        assert!(xlsx_rows(sheet, &ss, 2, 2).1); // C1 (third column) has a value
        assert!(xlsx_rows(sheet, &ss, 1, 3).1); // the inline string in row 2
    }

    // Values past the row or column limit say "more"; cells only formatted out there do not.
    #[test]
    fn xlsx_limit_says_when_cells_left_out() {
        let rows = |n: usize| (1..=n).map(|r| format!(r#"<row r="{r}"><c r="A{r}"><v>{r}</v></c></row>"#)).collect::<String>();
        for (n, cut) in [(1999, false), (2000, false), (2001, true)] {
            let (r, more) = xlsx_rows(&rows(n), &[], 2000, 60);
            assert_eq!((r.len(), more), (n.min(2000), cut), "{n} rows");
        }
        let formatted = rows(2000) + r#"<row r="3000"><c r="A3000" s="1"/><c r="B3000" s="2"></c></row>"#;
        assert!(!xlsx_rows(&formatted, &[], 2000, 60).1);
        let wide = |col: &str, cell: &str| format!(r#"<row r="1"><c r="A1"><v>1</v></c><c r="{col}1"{cell}</row>"#);
        assert!(xlsx_rows(&wide("BI", "><v>61</v></c>"), &[], 2000, 60).1); // column 61
        assert!(!xlsx_rows(&wide("BH", "><v>60</v></c>"), &[], 2000, 60).1); // column 60 is shown
        assert!(xlsx_rows(&wide("BI", r#" t="inlineStr"><is><t>x</t></is></c>"#), &[], 2000, 60).1);
        assert!(!xlsx_rows(&wide("BI", r#" s="3"/>"#), &[], 2000, 60).1);
    }

    // The preview's header and notes say when only the front of a file is shown.
    #[test]
    fn preview_cut_words() {
        assert_eq!(pages_label(50, 312, true), "312쪽 · 앞 50쪽만 미리보기");
        assert_eq!(pages_label(50, 312, false), "312 pages · first 50 previewed");
        assert_eq!((pages_label(12, 12, true), pages_label(50, 50, false)), ("12쪽".into(), "50 pages".into()));
        assert_eq!(pages_note(50, 312, true), "앞 50쪽까지만 미리보기 (전체 312쪽)");
        assert_eq!(pages_note(50, 312, false), "Only the first 50 pages are previewed (312 in all)");
        assert_eq!((lines_label(120, false, true), lines_label(120, false, false)), ("120줄".into(), "120 lines".into()));
        assert_eq!((lines_label(8000, true, true), lines_label(8000, true, false)), ("8000+줄".into(), "8000+ lines".into()));
        assert_eq!(lines_label(TEXT_LINES, false, true), "40000줄");
        assert_eq!(lines_label(60000, false, true), "60000줄 · 앞 40000줄만");
        assert_eq!(lines_label(60000, true, false), "60000+ lines · first 40000 shown");
        assert_eq!((lines_note(true), lines_note(false)), ("… 앞 40000줄만 표시".into(), "… only the first 40000 lines are shown".into()));
        assert_eq!((rows_label(2000, true, true), rows_label(2000, true, false)), ("2000+행".into(), "2000+ rows".into()));
        assert_eq!((rows_label(1999, false, true), rows_label(1999, false, false)), ("1999행".into(), "1999 rows".into()));
    }

    #[test]
    fn mail_and_html() {
        assert_eq!(mime_words("=?UTF-8?B?7JWI64WV?= =?utf-8?Q?_hi?= there"), "안녕 hi there");
        assert_eq!(mime_words("=?ks_c_5601-1987?B?x9Gx2w==?="), "한글");
        let eml = b"Subject: =?UTF-8?B?7JWI64WV?=\r\nFrom: a@b.c\r\nContent-Type: multipart/alternative; boundary=\"XX\"\r\n\r\n--XX\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>html</p>\r\n--XX\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n=EC=95=88 plain=\r\n text\r\n--XX\r\nContent-Type: application/pdf; name=\"r.pdf\"\r\nContent-Disposition: attachment; filename=\"r.pdf\"\r\n\r\nJVBE\r\n--XX--\r\n";
        let t = eml_text(eml, true);
        assert!(t.starts_with("제목: 안녕\n보낸 사람: a@b.c\n첨부: r.pdf\n\n"), "{t}");
        assert!(t.ends_with("안 plain text"), "{t}");
        let h = html_text("<html><head><title>x</title></head><body><script>var a;</script><h1>Title</h1><p>a &amp; b</p><ul><li>one</li></ul></body></html>");
        assert_eq!(h, "Title\n\na & b\n\n• one"); // blocks a blank line apart
    }

    // Crafted files a preview must survive quickly: deep MIME nesting, "&&&&…", unclosed =? words, many script
    // blocks, one huge shared string behind every cell.
    #[test]
    fn hostile_documents() {
        let t0 = Instant::now();
        let mut eml = String::from("Subject: x\r\nContent-Type: multipart/mixed; boundary=L00000\r\n\r\n");
        for k in 0..20_000 {
            eml += &format!("--L{k:05}\r\nContent-Type: multipart/mixed; boundary=L{:05}\r\n\r\n", k + 1);
        }
        let t = eml_text(eml.as_bytes(), false); // used to overflow the stack and take Grids down
        assert!(t.starts_with("Subject: x"), "{t}");
        let amp = "&".repeat(300_000) + "amp;";
        assert_eq!(unescape(&amp), "&".repeat(300_000));
        let open = "=?a?b?x ".repeat(50_000);
        assert_eq!(mime_words(&open), open);
        assert_eq!(mime_words("=?UTF-8?B?7JWI64WV?= =?x"), "안녕=?x"); // as before: the space after a word goes
        let html = "<script>a</SCRIPT>b".repeat(50_000);
        assert_eq!(html_text(&html), "b".repeat(50_000));
        assert_eq!((find_ci("ab</ScRiPt", "</script"), find_ci("ab", "</script"), find_ci("가</a", "</a")), (Some(2), None, Some(3)));
        let ss = vec!["A".repeat(16 << 20)];
        let sheet = (1..=2000).map(|r| format!(r#"<c r="A{r}" t="s"><v>0</v></c>"#)).collect::<String>();
        let (rows, _) = xlsx_rows(&sheet, &ss, 2000, 60); // was 2000 copies of 16 MB
        assert!(rows.len() == 2000 && rows.iter().all(|r| r[0].len() == CELL_MAX));
        assert!(t0.elapsed() < Duration::from_secs(20), "{:?}", t0.elapsed());
    }

    // A mail whose body is text/plain under n multiparts, each with its own fixed-width boundary.
    fn nested_eml(n: usize) -> Vec<u8> {
        let mut m = String::from("Subject: deep\r\nContent-Type: multipart/mixed; boundary=\"b00000\"\r\n\r\n");
        for i in 1..n {
            m += &format!("--b{:05}\r\nContent-Type: multipart/mixed; boundary=\"b{i:05}\"\r\n\r\n", i - 1);
        }
        m += &format!("--b{:05}\r\nContent-Type: text/plain\r\n\r\ndeep body\r\n", n - 1);
        for i in (0..n).rev() {
            m += &format!("--b{i:05}--\r\n");
        }
        m.into_bytes()
    }

    #[test]
    fn mail_nesting() {
        // thousands deep used to run the preview worker's stack out and end Grids
        let t = eml_text(&nested_eml(20_000), true);
        assert!(t.starts_with("제목: deep\n") && !t.contains("deep body"), "{}", &t[..t.len().min(200)]);
        assert!(eml_text(&nested_eml(16), false).ends_with("deep body"));
        assert!(!eml_text(&nested_eml(17), false).contains("deep body"));
        // an ordinary mail: mixed > related > alternative, a picture inside and a file attached
        let eml = b"Subject: hi\r\nContent-Type: multipart/mixed; boundary=\"M\"\r\n\r\n--M\r\nContent-Type: multipart/related; boundary=\"R\"\r\n\r\n--R\r\nContent-Type: multipart/alternative; boundary=\"A\"\r\n\r\n--A\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nplain body\r\n--A\r\nContent-Type: text/html\r\n\r\n<p>html</p>\r\n--A--\r\n--R\r\nContent-Type: image/png; name=\"logo.png\"\r\n\r\niVBO\r\n--R--\r\n--M\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"r.pdf\"\r\n\r\nJVBE\r\n--M--\r\n";
        let t = eml_text(eml, false);
        assert!(t.starts_with("Subject: hi\nAttachments: logo.png, r.pdf\n\n") && t.ends_with("plain body"), "{t}");
    }

    #[test]
    fn peek_fail_card() {
        let req = |p: &str| Some(PeekReq { path: p.into(), w: 10, h: 10, hwnd: 0, plain: false, fetch: false });
        let card = || Peeked { body: Body::Card { icon: None, rows: Vec::new() }, icon: -1, meta: Vec::new(), cloud: false };
        let pages = || Peeked { body: Body::Pages { pages: Vec::new(), total: 3, all: 3 }, icon: -1, meta: Vec::new(), cloud: false };
        let mut j = PeekJob { gen: 0, req: None, done: None, zoom: None, zseq: 0, sharp: Vec::new() };
        j.want(req("a"));
        let a = j.gen;
        j.want(req("b"));
        assert!(!j.fail(a, card()) && j.done.is_none()); // left already
        assert!(j.publish(j.gen, pages()));
        assert!(!j.fail(j.gen, card()) && matches!(j.done, Some(Peeked { body: Body::Pages { .. }, .. }))); // first page stays
        j.want(req("c"));
        assert!(j.fail(j.gen, card()) && matches!(j.done, Some(Peeked { body: Body::Card { .. }, .. })));
    }

    #[test]
    fn cloud_only_attributes() {
        assert!(!cloud_only(0x20)); // archive: on this PC
        assert!(cloud_only(0x20 | 0x40_0000)); // RECALL_ON_DATA_ACCESS: OneDrive "online-only"
        assert!(cloud_only(0x4_0000)); // RECALL_ON_OPEN
        assert!(cloud_only(0x1000)); // OFFLINE
        assert!(!cloud_only(0x10 | 0x40_0000)); // a folder is listed, not downloaded
        assert!(!cloud_only(0x400 | 0x80000 | 0x20)); // reparse point + pinned, downloaded already
        let p = Pix { w: 4, h: 2, bgra: vec![200; 32] };
        let s = shrink(&p, 2);
        assert!(s.w == 2 && s.h == 1 && s.bgra.iter().all(|&b| b == 200));
        // a 48 px icon in a 256 px square's corner, versus one drawn out to the edge
        let mut j = Pix { w: 256, h: 256, bgra: vec![0; 256 * 256 * 4] };
        j.bgra[(47 * 256 + 47) * 4 + 3] = 255;
        assert!(opaque_within(&j, 48));
        j.bgra[(200 * 256 + 10) * 4 + 3] = 255;
        assert!(!opaque_within(&j, 48));
        assert!(!opaque_within(&Pix { w: 256, h: 256, bgra: vec![0; 256 * 256 * 4] }, 48)); // nothing drawn
        let long = format!(r"C:\{}\a.txt", "d".repeat(300));
        assert_eq!(long_path(&long), format!(r"\\?\{long}"));
        assert_eq!(long_path(&format!(r"\\srv\s\{}", "d".repeat(300))), format!(r"\\?\UNC\srv\s\{}", "d".repeat(300)));
        assert_eq!(long_path(r"C:\short.txt"), r"C:\short.txt");
    }

    // An online-only file's preview reads nothing until Enter asks for it.
    #[test]
    fn peek_cloud_card() {
        use windows::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE};
        let d = std::env::temp_dir().join(format!("grids-cloud-{}", std::process::id()));
        _ = std::fs::create_dir_all(&d);
        let f = d.join("note.txt");
        std::fs::write(&f, "hello cloud").unwrap();
        let path = f.to_string_lossy().into_owned();
        let req = |fetch| PeekReq { path: path.clone(), w: 100, h: 100, hwnd: 0, plain: false, fetch };
        let text = |b: &Body| matches!(b, Body::Text { text, .. } if text.contains("hello cloud"));
        unsafe {
            assert!(text(&content(&req(false))), "a local file is read as before");
            SetFileAttributesW(&HSTRING::from(path.as_str()), FILE_ATTRIBUTE_OFFLINE).unwrap();
            assert!(online(&req(false)).is_some() && online(&req(true)).is_none());
            let card = content(&req(false));
            assert!(matches!(&card, Body::Card { rows, .. } if rows[0].1.contains("Enter") && rows.iter().any(|r| r.1 == "11 B")), "the card, not the text");
            let (_, m) = meta(&path, &card, online(&req(false)).as_ref());
            assert!(m.iter().any(|s| s == "Online-only" || s == "온라인 전용"), "{m:?}");
            assert!(text(&content(&req(true))), "Enter: read it");
            SetFileAttributesW(&HSTRING::from(path.as_str()), FILE_ATTRIBUTE_NORMAL).unwrap();
        }
        _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn peek_worker_survives_panic() {
        use std::sync::mpsc;
        static JOB: Mutex<PeekJob> = Mutex::new(PeekJob { gen: 0, req: None, done: None, zoom: None, zseq: 0, sharp: Vec::new() });
        static WAKE: Condvar = Condvar::new();
        static QUIT: AtomicBool = AtomicBool::new(false);
        let lock = || JOB.lock().unwrap_or_else(|e| e.into_inner());
        let (tx, rx) = mpsc::channel::<usize>();
        let told = tx.clone();
        // quit and join even when a wait or a check below fails, so no worker outlives the test
        struct Stop(Option<std::thread::JoinHandle<()>>);
        impl Drop for Stop {
            fn drop(&mut self) {
                {
                    let _j = JOB.lock().unwrap_or_else(|e| e.into_inner()); // set under the lock, so the worker can't miss the wake between its check and its wait
                    QUIT.store(true, Ordering::Relaxed);
                }
                WAKE.notify_all();
                if let Some(w) = self.0.take() {
                    let ended = w.join();
                    if !std::thread::panicking() {
                        ended.expect("the worker ends on quit");
                    }
                }
            }
        }
        let worker = Stop(Some(std::thread::spawn(move || {
            let serve = move |gen: Option<usize>, _: Option<(u64, Sharp)>, r: &PeekReq| {
                let Some(gen) = gen else { return };
                let mut j = JOB.lock().unwrap_or_else(|e| e.into_inner());
                if r.path == "boom" {
                    panic!("test: a preview fails with the lock held");
                }
                if j.publish(gen, Peeked { body: Body::Text { text: r.path.clone(), kind: TextKind::Plain, cut: false }, icon: -1, meta: Vec::new(), cloud: false }) {
                    drop(j);
                    _ = tx.send(gen);
                }
            };
            peek_loop(&JOB, &WAKE, &QUIT, serve, move |gen, _| _ = told.send(gen));
        })));
        let want = |p: &str| {
            lock().want(Some(PeekReq { path: p.into(), w: 10, h: 10, hwnd: 0, plain: false, fetch: false }));
            WAKE.notify_one();
        };
        want("boom");
        let g = rx.recv_timeout(Duration::from_secs(5)).expect("the failed item is told");
        {
            let j = lock();
            assert!(JOB.is_poisoned() && j.gen == g);
            assert!(matches!(&j.done, Some(Peeked { body: Body::Card { rows, .. }, .. }) if rows.len() == 1), "failure card");
        }
        want("ok"); // the same worker carries on
        let g = rx.recv_timeout(Duration::from_secs(5)).expect("the next item is made");
        {
            let j = lock();
            assert!(j.gen == g && matches!(&j.done, Some(Peeked { body: Body::Text { text, .. }, .. }) if text == "ok"));
        }
        drop(worker); // quits, joins and checks the worker ended cleanly
    }

    #[test]
    fn markdown_lines() {
        let l = md_lines("# T\n\nsome **bold** `x`\n- a\n  - b\n> q\n```\ncode **\n```\n---");
        let styles: Vec<Style> = l.iter().map(|x| x.1).collect();
        use Style::*;
        assert_eq!(styles, vec![H1, Plain, Plain, Bullet, Bullet, Quote, Mono, Quote]);
        assert_eq!((l[2].0.as_str(), l[3].0.as_str(), l[4].0.as_str(), l[6].0.as_str()), ("some bold x", "• a", "    • b", "code **"));
    }

    #[test]
    fn markdown_lines_more() {
        use Style::*;
        let rule = "────────";
        let want: &[(&str, &str, Style)] = &[
            ("## 설치 ##", "설치", H2),
            ("### 셋", "셋", H3),
            ("#### 넷 #", "넷", H3),
            ("# C#", "C#", H1),
            ("#", "#", Plain),
            ("#태그", "#태그", Plain),
            ("* 별", "• 별", Bullet),
            ("+ 더하기", "• 더하기", Bullet),
            ("    * 깊이", "        • 깊이", Bullet),
            (">붙은 인용", "붙은 인용", Quote),
            ("***", rule, Quote),
            ("___", rule, Quote),
            ("* * *", rule, Quote),
            ("- - -", rule, Quote),
            ("--- ", rule, Quote),
            ("\t_ _\t_", rule, Quote),
            ("- -", "• -", Bullet),
            ("**", "**", Plain),
            ("* *", "• *", Bullet),
            ("--", "--", Plain),
            ("*-*", "*-*", Plain),
            ("`__init__`", "__init__", Plain),
            ("`2 ** 3`", "2 ** 3", Plain),
            ("**굵게**", "굵게", Plain),
            ("__굵게__", "굵게", Plain),
            ("이름: ______", "이름: ______", Plain),
            ("짝 없는 ` 하나", "짝 없는 ` 하나", Plain),
            ("2 ** 3 그리고 **굵게**", "2 ** 3 그리고 굵게", Plain),
            ("``a`b``", "a`b", Plain),
            ("**굵게**한글", "굵게한글", Plain),
            ("`코드`🙂", "코드🙂", Plain),
            ("🙂**", "🙂**", Plain),
            ("__", "__", Plain),
            ("`", "`", Plain),
            ("", "", Plain),
            ("~~~", "", Mono), // fence: no output line
            ("***", "***", Mono),
            ("~~~", "", Mono),
            ("```", "", Mono),
            ("열린 울타리", "열린 울타리", Mono),
            ("# 아님", "# 아님", Mono),
        ];
        let src = want.iter().map(|w| w.0).collect::<Vec<_>>().join("\r\n");
        let got = md_lines(&src);
        let fences = want.iter().filter(|w| w.0.starts_with("~~~") || w.0.starts_with("```")).count();
        assert_eq!(got.len(), src.lines().count() - fences);
        let expect: Vec<(String, Style)> = want
            .iter()
            .filter(|w| !(w.0.starts_with("~~~") || w.0.starts_with("```")))
            .map(|w| (w.1.to_string(), w.2))
            .collect();
        assert_eq!(got, expect);
    }

    #[test]
    fn icon_alpha() {
        let mut photo = vec![10, 20, 30, 0, 40, 50, 60, 0];
        fix_alpha(&mut photo);
        assert_eq!(photo, vec![10, 20, 30, 255, 40, 50, 60, 255]); // no alpha at all: opaque
        let mut icon = vec![255, 255, 255, 128, 200, 100, 0, 255, 0, 0, 0, 0];
        fix_alpha(&mut icon);
        assert_eq!(icon, vec![128, 128, 128, 128, 200, 100, 0, 255, 0, 0, 0, 0]); // straight: premultiplied
        let mut pre = vec![100, 100, 100, 128, 0, 0, 0, 0];
        fix_alpha(&mut pre);
        assert_eq!(pre, vec![100, 100, 100, 128, 0, 0, 0, 0]); // already premultiplied: left alone
    }

    #[test]
    fn wheel_notch_rest() {
        let mut acc = 0;
        assert_eq!(wheel_notches(&mut acc, -120), -1); // a plain mouse notch: one step, as before
        assert_eq!(acc, 0);
        let got: Vec<i32> = (0..4).map(|_| wheel_notches(&mut acc, 30)).collect();
        assert_eq!(got, vec![0, 0, 0, 1]); // a touchpad's small deltas add up to a notch
        assert_eq!((wheel_notches(&mut acc, 240), acc), (2, 0));
        for _ in 0..3 {
            assert_eq!(wheel_notches(&mut acc, -30), 0);
        }
        assert_eq!(acc, -90);
        assert_eq!((wheel_notches(&mut acc, 30), acc), (0, 30)); // turning back starts over from 0
        assert_eq!((wheel_notches(&mut acc, 0), acc), (0, 30)); // nothing sent: the rest stays
        let mut acc = 119;
        assert_eq!((wheel_notches(&mut acc, i16::MAX as i32), acc), (274, 6));
        let mut acc = -119;
        assert_eq!((wheel_notches(&mut acc, i16::MIN as i32), acc), (-274, -7));
        let mut acc = i32::MAX - 1;
        assert_eq!(wheel_notches(&mut acc, i16::MAX as i32), i32::MAX / 120); // no overflow
    }

    #[test]
    fn peek_wheel() {
        use Wheel::*;
        assert_eq!(wheel_action(0, 0, true, false), Step); // a picture: straight to the next file
        assert_eq!(wheel_action(0, 0, false, false), Step);
        assert_eq!(wheel_action(0, 500, true, false), Scroll); // long text: scroll first
        assert_eq!(wheel_action(0, 500, false, false), Hold); // at the top, just arrived
        assert_eq!(wheel_action(0, 500, false, true), Step); // rested there: previous file
        assert_eq!(wheel_action(500, 500, true, false), Hold);
        assert_eq!(wheel_action(500, 500, true, true), Step);
        assert_eq!(wheel_action(500, 500, false, true), Scroll); // back up is still scrolling
    }

    #[test]
    fn peek_layout() {
        let rc = RECT { left: 0, top: 0, right: 1120, bottom: 810 };
        let g = peek_geo(rc, 96);
        // header 48 high; stage between 56 px side margins; plate padding 20
        assert_eq!((g.stage.left, g.stage.top, g.stage.right, g.stage.bottom), (56, 48, 1064, 786));
        assert_eq!((g.inner.left, g.inner.top), (76, 68));
        assert!(g.close.right <= rc.right && g.close.bottom <= 48 && g.title.right < g.close.left);
        // the arrows sit in the side margins, centred on the stage
        assert!(g.prev.right <= g.stage.left && g.next.left >= g.stage.right);
        assert_eq!((g.prev.top + g.prev.bottom) / 2, (g.stage.top + g.stage.bottom) / 2);
        assert_eq!(peek_hit(&g, g.next.left + 4, g.next.top + 4), Hot::Next);
        assert_eq!(peek_hit(&g, g.prev.left + 4, g.prev.top + 4), Hot::Prev);
        assert_eq!(peek_hit(&g, g.close.left + 4, g.close.top + 4), Hot::Close);
        assert_eq!(peek_hit(&g, 500, 400), Hot::None); // content: no action
        assert_eq!(group_digits(30360942), "30,360,942");
        assert_eq!(group_digits(999), "999");
    }

    #[test]
    fn peek_names() {
        for ok in ["photo.jpg", "a", ".gitignore", "cone.txt", "COM10", "LPT10", "사진 1.png", "a..b"] {
            assert_eq!(name_problem(ok), None, "{ok}");
        }
        for bad in ["", "   ", "a/b", "a\\b", "what?", "x:y", "a*", "<a>", "a|b", "\"q\"", "tab\there", "end.", "CON", "nul.txt", "Com1.jpg", "lpt9", "con.txt.bak"] {
            assert!(name_problem(bad).is_some(), "{bad}");
        }
        assert!(name_problem(&"a".repeat(256)).is_some() && name_problem(&"a".repeat(255)).is_none());
        // F2 selects the stem: not the extension, all of a folder or a dot file
        assert_eq!(stem_len("photo.final.jpg", false), 11);
        assert_eq!(stem_len("photo.final.jpg", true), 15);
        assert_eq!(stem_len(".gitignore", false), 10);
        assert_eq!(stem_len("README", false), 6);
        assert_eq!(stem_len("사진😀.png", false), 4); // UTF-16 units, as the edit counts them
    }

    #[test]
    fn peek_zoom() {
        assert_eq!(zoom_k(0), 1.0);
        assert!((zoom_k(ZOOM_MAX) - 9.31).abs() < 0.01 && (zoom_k(ZOOM_MIN) - 0.41).abs() < 0.01);
        // the point under the cursor stays put: 200 px into a 1000 px picture, zoomed twice as big
        assert_eq!(zoom_scroll(200, 0, 1000, 2000, 1000, true), 200);
        assert_eq!(zoom_scroll(0, 0, 1000, 2000, 1000, true), 0);
        assert_eq!(zoom_scroll(1000, 0, 1000, 2000, 1000, true), 1000);
        assert_eq!(zoom_scroll(500, 300, 2000, 1000, 1000, true), 0); // back to fitting: no scroll
        // a centred 600 px picture in a 1000 px stage: its middle (500) stays in the middle
        assert_eq!(zoom_scroll(500, 0, 600, 1200, 1000, true), 100);
        // pages start at the top, not centred
        assert_eq!(zoom_scroll(100, 0, 600, 1200, 1000, false), 100);
        assert_eq!(origin(10, 1000, 600, 0, true), 210);
        assert_eq!(origin(10, 1000, 600, 0, false), 10);
        assert_eq!(origin(10, 1000, 3000, 700, true), -690);
        // thumbs: proportional, never below the minimum, at the ends of the track at the ends
        assert_eq!(thumb(0, 100, 500, 1000, 0, 10), (0, 50));
        assert_eq!(thumb(0, 100, 500, 1000, 500, 10), (50, 100));
        assert_eq!(thumb(0, 100, 10, 100_000, 0, 10), (0, 10));
        assert_eq!(thumb_scroll(0, 100, 500, 1000, 25, 10), 250);
        assert_eq!(thumb_scroll(0, 100, 500, 1000, 90, 10), 500);
        assert_eq!(thumb_scroll(0, 100, 500, 1000, -5, 10), 0);
        // bars only along the way it scrolls, inside the stage, not crossing each other
        let st = RECT { left: 56, top: 48, right: 1064, bottom: 786 };
        assert_eq!(zoom_bars(st, (500, 500), 96), [None, None]);
        let [v, h] = zoom_bars(st, (2000, 2000), 96);
        let (v, h) = (v.unwrap(), h.unwrap());
        assert!(v.right <= st.right && v.left > st.right - 20 && h.bottom <= st.bottom && h.top > st.bottom - 20);
        assert!(v.bottom <= h.top && h.right <= v.left);
        let [v, h] = zoom_bars(st, (2000, 500), 96); // sideways only: the bar runs the full width
        assert!(v.is_none() && h.is_some_and(|b| b.right == st.right - 3) && zoom_bars(st, (500, 2000), 96)[1].is_none());
        // drawing zoomed: only the visible part of the picture is copied
        let to = RECT { left: -1000, top: -500, right: 3000, bottom: 1500 }; // 400x200 shown 10x
        let ([x0, y0, x1, y1], at) = visible_part(to, RECT { left: 0, top: 0, right: 1000, bottom: 600 }, 400, 200).unwrap();
        assert_eq!((x0, y0, x1, y1), (100, 50, 200, 110));
        assert_eq!(at, RECT { left: 0, top: 0, right: 1000, bottom: 600 });
        assert_eq!(visible_part(to, RECT { left: 3000, top: 0, right: 3100, bottom: 10 }, 400, 200), None);
        let fit = RECT { left: 10, top: 10, right: 410, bottom: 210 };
        assert_eq!(visible_part(fit, RECT { left: 0, top: 0, right: 999, bottom: 999 }, 400, 200).unwrap().0, [0, 0, 400, 200]);
        // sharper pictures: only when zoomed in enough, and no bigger than the cap
        assert_eq!(sharp_size(800, 600, 1000, 700, 1.0), None);
        assert_eq!(sharp_size(800, 600, 1000, 700, 2.0), Some((2000, 1400)));
        let (w, h) = sharp_size(800, 600, 800, 600, 50.0).unwrap();
        assert!(w * h <= 16_100_000 && w > 800 * 4);
    }

    #[test]
    fn space_previews() {
        assert!(space_peeks(None) && space_peeks(Some(Duration::from_millis(1500))));
        assert!(!space_peeks(Some(Duration::from_millis(300)))); // "my f|": the space is part of the name
        assert!(types_char(VK_A) && types_char(VK_7) && types_char(VIRTUAL_KEY(0xE5))); // 0xE5: Korean IME
        assert!(!types_char(VK_SPACE) && !types_char(VK_DOWN) && !types_char(VK_F3) && !types_char(VK_RETURN));
    }

    #[test]
    fn altgr_letters_are_typing() {
        // AltGr = Ctrl+Alt: ł ś đ [ ħ ę @ { ... are characters, not Ctrl+L / Ctrl+S / Alt+D / Ctrl+F / Ctrl+H
        for vk in [VK_L, VK_S, VK_D, VK_F, VK_G, VK_H, VK_E, VK_Q, VK_7, VK_OEM_4] {
            assert!(key_types(vk, true, true), "{:#x}", vk.0);
            assert!(key_types(vk, false, false), "{:#x}", vk.0);
            assert!(!key_types(vk, true, false) && !key_types(vk, false, true), "{:#x}", vk.0);
        }
        // the shortcuts that stay: Ctrl+L, Alt+D, Ctrl+F, Ctrl+H, Ctrl+S
        assert!(!key_types(VK_L, true, false) && !key_types(VK_D, false, true) && !key_types(VK_F, true, false));
        assert!(!key_types(VK_H, true, false) && !key_types(VK_S, true, false));
        // not characters even with Ctrl+Alt (Ctrl+Alt+arrows move splitters, Ctrl+Alt+Shift+arrows swap panes)
        for vk in [VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN, VK_TAB, VK_F4, VK_RETURN, VK_SPACE] {
            assert!(!key_types(vk, true, true), "{:#x}", vk.0);
        }
    }

    // A keycap as the shortcut list writes it ("Ctrl+Shift+PgDn", "Num−", "Ctrl+/") as (vk, ctrl, shift, alt).
    fn cap_key(cap: &str) -> (VIRTUAL_KEY, bool, bool, bool) {
        let (mut ctrl, mut shift, mut alt, mut k) = (false, false, false, cap);
        loop {
            if let Some(r) = k.strip_prefix("Ctrl+") {
                (ctrl, k) = (true, r);
            } else if let Some(r) = k.strip_prefix("Shift+") {
                (shift, k) = (true, r);
            } else if let Some(r) = k.strip_prefix("Alt+") {
                (alt, k) = (true, r);
            } else {
                break;
            }
        }
        let f = k.strip_prefix('F').and_then(|n| n.parse::<u16>().ok()).filter(|n| (1..=12).contains(n));
        let vk = match k {
            "Tab" => VK_TAB,
            "Enter" => VK_RETURN,
            "Esc" => VK_ESCAPE,
            "Space" => VK_SPACE,
            "Insert" => VK_INSERT,
            "PgDn" => VK_NEXT,
            "PgUp" => VK_PRIOR,
            "←" => VK_LEFT,
            "→" => VK_RIGHT,
            "↑" => VK_UP,
            "↓" => VK_DOWN,
            "/" => VK_OEM_2,
            "Num+" => VK_ADD,
            "Num\u{2212}" => VK_SUBTRACT,
            "Num*" => VK_MULTIPLY,
            "Num/" => VK_DIVIDE,
            _ if f.is_some() => VIRTUAL_KEY(VK_F1.0 + f.unwrap() - 1),
            _ if k.len() == 1 && (k.as_bytes()[0].is_ascii_uppercase() || k.as_bytes()[0].is_ascii_digit()) => VIRTUAL_KEY(k.as_bytes()[0] as u16),
            _ => panic!("can't read the key {cap:?}"),
        };
        (vk, ctrl, shift, alt)
    }

    // key_route for a keycap with the focus in `at`: "" the file list, "addr" the address bar, "filter" the filter.
    fn route_with(at: &str, cap: &str, tabs: bool, panes: usize, typed_ago: Option<Duration>) -> KeyRoute {
        let (vk, ctrl, shift, alt) = cap_key(cap);
        let (list, in_main) = (|| at.is_empty(), || true);
        key_route(vk, ctrl, shift, alt, &KeyCtx { addr: at == "addr", filt: at == "filter", tabs, panes, typed_ago, list: &list, in_main: &in_main })
    }

    fn route(at: &str, cap: &str) -> KeyRoute {
        route_with(at, cap, true, 9, None)
    }

    // Every key the in-app shortcut list names is one Grids acts on (not typing, not the shell view's own): in the
    // file list with tabs on and 9 panes, an "in the filter" row in the filter, Enter / Esc in either box.
    #[test]
    fn listed_keys_are_handled() {
        // not key()'s to route: Alt / F10 (pre_translate opens the view menu when let go), Windows-wide hotkeys
        // (Win+Alt+E, Ctrl+Alt+G), and Explorer's own keys handed to the view (of that row F5 is ours: Refresh);
        // the preview's rows are its own (peek_key)
        let skip = [":Alt", ":F10", ":Win+Alt+E", ":Ctrl+Alt+G", ":F2", ":Del", ":Ctrl+C/X/V"];
        let caps: Vec<String> = app_key_rows(KEYS_EN).into_iter().flatten().collect();
        for s in skip {
            assert!(caps.iter().any(|c| c == s), "{s} is no longer listed: take it out of skip");
        }
        let mut seen = 0;
        for cap in caps.iter().filter(|c| !c.starts_with("preview:") && !skip.contains(&c.as_str())) {
            let (ctx, key) = cap.split_once(':').unwrap();
            let spots: &[&str] = match (ctx, key) {
                ("filter", _) => &["filter"],
                (_, "Enter" | "Esc") => &["addr", "filter"],
                _ => &[""],
            };
            // "Ctrl+1 … Ctrl+9": both ends
            for k in key.split('…').map(str::trim) {
                for &at in spots {
                    let r = route(at, k);
                    assert!(!matches!(r, KeyRoute::Shell | KeyRoute::Typing | KeyRoute::BoxKey), "{cap} (focus {at:?}) goes to {r:?}");
                    seen += 1;
                }
            }
        }
        assert!(seen >= 60, "only {seen} keys checked");
    }

    // The arms whose order or conditions decide a key: one moved or loosened sends it elsewhere without a compile error.
    #[test]
    fn key_routes() {
        use KeyRoute::*;
        let l = |k| route("", k);
        assert_eq!(l("Ctrl+Shift+F"), Everything);
        assert_eq!(l("Ctrl+F"), FocusFilter);
        assert_eq!(route("filter", "Alt+↓"), KindMenu);
        assert_eq!(l("Alt+↓"), Recent);
        assert_eq!((l("F5"), route("addr", "F5")), (Refresh, Refresh));
        assert_eq!((l("Shift+F5"), route("addr", "Shift+F5"), l("Ctrl+Shift+F5")), (Stamp, BoxKey, Shell));
        // Shift+F2: the list's compare; F2 stays the view's rename, the boxes keep theirs
        assert_eq!((l("Shift+F2"), l("F2"), route("addr", "Shift+F2"), l("Ctrl+Shift+F2")), (Diff, Shell, BoxKey, Shell));
        assert_eq!((l("Ctrl+F3"), l("Ctrl+F5"), l("Ctrl+F6")), (Sort(0), Sort(1), Sort(2)));
        assert_eq!(route_with("", "Ctrl+T", false, 9, None), Shell);
        // Ctrl+Shift+T: from the list and the boxes; tabs off or AltGr+Shift+T: not ours
        assert_eq!((l("Ctrl+Shift+T"), route("addr", "Ctrl+Shift+T"), route("filter", "Ctrl+Shift+T")), (ReopenTab, ReopenTab, ReopenTab));
        assert_eq!((route_with("", "Ctrl+Shift+T", false, 9, None), l("Ctrl+T")), (Shell, NewTab));
        assert_ne!(l("Ctrl+Alt+Shift+T"), ReopenTab);
        assert_eq!(route_with("", "Ctrl+Enter", false, 9, None), Shell);
        assert_eq!((l("Ctrl+Enter"), l("Ctrl+Shift+Enter")), (OpenTabs(false), OpenTabs(true)));
        assert_eq!((route_with("", "Ctrl+4", true, 4, None), route_with("", "Ctrl+5", true, 4, None)), (Pane(3), Shell));
        for k in ["Alt+PgDn", "F3", "F7", "Insert"] {
            assert_eq!(route("addr", k), BoxKey, "{k}");
        }
        assert_eq!(route("addr", "Ctrl+Alt+L"), Typing); // AltGr+L
        assert_eq!((l("Ctrl+Alt+←"), l("Alt+Shift+←"), l("Ctrl+Alt+Shift+↓")), (Nudge(-1, 0), MovePane(-1, 0), Swap(0, 1)));
        assert_eq!((l("Ctrl+Insert"), l("Ctrl+Num+")), (Shell, Shell));
        // Num/: the list's; the boxes type a '/', Ctrl+Num/ stays the view's
        assert_eq!((l("Num/"), l("Shift+Num/"), route("addr", "Num/"), l("Ctrl+Num/")), (RestoreSel, RestoreSel, BoxKey, Shell));
        assert_eq!(route_with("", "Space", true, 9, Some(Duration::from_millis(300))), Shell);
        assert_eq!(l("Space"), Peek);
        assert_eq!((l("Ctrl+Shift+S"), l("Ctrl+F4"), l("Shift+Num+"), l("Num\u{2212}")), (Save, FocusAddr, Match(false), Match(true)));
        assert_eq!((l("Alt+1"), l("Alt+9"), l("Ctrl+Shift+Tab"), l("Ctrl+Shift+PgUp")), (NthTab(1), NthTab(9), CyclePane(-1), MoveTab(-1)));
        // Ctrl+E: the list's own (editor); the tree keeps Ctrl+Shift+E, the boxes keep their Ctrl+E
        assert_eq!((l("Ctrl+E"), l("Ctrl+Shift+E")), (Edit, Tree));
        assert_eq!((route("addr", "Ctrl+E"), route("filter", "Ctrl+E"), route("addr", "Ctrl+Shift+E")), (BoxKey, BoxKey, Tree));
        // Ctrl+P: from the list and both boxes; Ctrl+Shift+P stays the view's / the box's
        assert_eq!((l("Ctrl+P"), route("addr", "Ctrl+P"), route("filter", "Ctrl+P")), (Switch, Switch, Switch));
        assert_eq!((l("Ctrl+Shift+P"), route("addr", "Ctrl+Shift+P")), (Shell, BoxKey));
        // Ctrl+Shift+V: the list's paste-and-go; the boxes and an edit box in the list (F2 rename) keep their paste
        assert_eq!((l("Ctrl+Shift+V"), l("Ctrl+V"), route("addr", "Ctrl+Shift+V"), route("filter", "Ctrl+Shift+V")), (PasteGo, Paste, BoxKey, BoxKey));
        assert_eq!((route("rename", "Ctrl+Shift+V"), l("Ctrl+Alt+Shift+V")), (Shell, Typing)); // AltGr+Shift+V: a character
    }

    // Ctrl+Shift+V's reading of the clipboard: the paths to try, best first; none for what is no path.
    #[test]
    fn clip_path_reads() {
        let var = |v: &str| match v {
            "TEMP" => Some(r"C:\Users\x\AppData\Local\Temp".to_string()),
            "USERPROFILE" => Some(r"C:\Users\x".to_string()),
            _ => None,
        };
        let p = |t: &str| clip_paths(t, var);
        assert_eq!(p(r#""C:\My Docs\a.txt""#), [r"C:\My Docs\a.txt"]);
        assert_eq!(p("  <C:\\a b\\c>  "), [r"C:\a b\c"]);
        assert_eq!(p("'D:\\x'"), [r"D:\x"]);
        assert_eq!(p("file:///C:/My%20Docs/a.txt"), [r"C:\My Docs\a.txt"]);
        assert_eq!(p("FILE:///C:/%EB%AC%B8%EC%84%9C/x.txt#top"), [r"C:\문서\x.txt"]);
        assert_eq!(p("file://server/share/a%25b.txt"), [r"\\server\share\a%b.txt"]);
        assert_eq!(p("file://localhost/C:/x"), [r"C:\x"]);
        assert_eq!(p(r"src\main.rs:120:5"), [r"src\main.rs"]);
        assert_eq!(p(r"C:\proj\a.cs:120"), [r"C:\proj\a.cs"]);
        assert_eq!(p("src/lib.rs:7:3:"), [r"src\lib.rs"]);
        assert_eq!(p("C:/Users/x"), [r"C:\Users\x"]);
        assert_eq!(p(r"%TEMP%\a"), [r"C:\Users\x\AppData\Local\Temp\a"]);
        assert_eq!(p(r"%USERPROFILE%\Desktop\"), [r"C:\Users\x\Desktop\"]);
        assert_eq!(p(r"\\server\share\x"), [r"\\server\share\x"]);
        assert_eq!(p("\r\n\n  C:\\first\\line  \r\nC:\\second\r\n"), [r"C:\first\line"]);
        // the name as it is first, less its closing punctuation only if that isn't there
        assert_eq!(p(r"C:\a\b.txt."), [r"C:\a\b.txt.", r"C:\a\b.txt"]);
        assert_eq!(p("notes.txt"), ["notes.txt"]);
        assert_eq!(p(".."), [".."]);
        assert_eq!((p("C:"), p("c:/")), (vec![r"C:\".to_string()], vec![r"c:\".to_string()]));
        // an unknown variable stays as it is (as ExpandEnvironmentStrings)
        assert_eq!(p(r"%NOPE%\x"), [r"%NOPE%\x"]);
        for no in ["https://example.com/a.txt", "mailto:a@b.c", "Note: see the log", "Is this a path?", "", "  \r\n ", "C:foo", "\\x", "/usr/bin", "<>", "a\tb", "file://"] {
            assert!(p(no).is_empty(), "{no:?} → {:?}", p(no));
        }
    }

    // Ctrl+E never runs a file: its edit verb, else text by its first bytes (by its type when online-only, never read).
    #[test]
    fn edit_picks() {
        use EditHow::*;
        let unread = || -> Option<Vec<u8>> { panic!("read") };
        assert_eq!(edit_how(true, false, false, unread), Verb); // .bat, .ps1, a picture: its own "edit"
        assert_eq!(edit_how(true, true, false, unread), Verb);
        assert_eq!(edit_how(false, false, false, || Some(b"[core]\r\n\tbare = false\r\n".to_vec())), Text); // a config without an extension
        assert_eq!(edit_how(false, false, false, || Some(Vec::new())), Text); // a new empty file
        assert_eq!(edit_how(false, false, false, || Some([0xFF, 0xFE, b'a', 0].to_vec())), Text); // UTF-16
        assert_eq!(edit_how(false, false, true, || Some(b"MZ\x90\0\x03\0\0\0".to_vec())), Binary); // an exe, whatever its name
        assert_eq!(edit_how(false, false, false, || Some(b"PK\x03\x04\x14\0\0\0".to_vec())), Binary);
        assert_eq!((edit_how(false, false, true, || None), edit_how(false, false, false, || None)), (Text, Binary)); // locked
        assert_eq!((edit_how(false, true, true, unread), edit_how(false, true, false, unread)), (Text, Online));
    }

    #[test]
    fn edited_notes() {
        assert_eq!(edited_note(false, 3, 3, Some(3), 0), "Opened 3 in an editor");
        assert_eq!(edited_note(true, 3, 3, Some(3), 0), "편집기로 3개를 열었습니다");
        assert_eq!(edited_note(true, 10, 10, Some(14), 0), "편집기로 10/14개만 열었습니다 (한 번에 10개까지)");
        assert_eq!(edited_note(false, 8, 10, Some(14), 2), "Opened 8 of 14 files in an editor (10 at a time) · 2 skipped");
        assert_eq!(edited_note(false, 10, 10, Some(10), 0), "Opened 10 in an editor");
        assert_eq!(edited_note(true, 10, 10, None, 0), "편집기로 10개만 열었습니다 (한 번에 10개까지)");
        assert_eq!(edited_note(true, 1, 2, Some(2), 1), "편집기로 1개를 열었습니다 · 1개 건너뜀");
    }

    // The file list / main window are asked only by the arms that need them (a shell view lookup per key otherwise).
    #[test]
    fn key_route_asks_lazily() {
        let calls = Cell::new(0);
        let ask = || {
            calls.set(calls.get() + 1);
            true
        };
        let ctx = KeyCtx { addr: false, filt: false, tabs: true, panes: 9, typed_ago: None, list: &ask, in_main: &ask };
        for k in ["Ctrl+Tab", "Ctrl+T", "Alt+←", "Ctrl+L", "Ctrl+Shift+F", "F5", "Ctrl+1", "F1"] {
            let (vk, ctrl, shift, alt) = cap_key(k);
            key_route(vk, ctrl, shift, alt, &ctx);
        }
        assert_eq!(calls.get(), 0);
        assert_eq!(key_route(VK_F6, false, false, false, &ctx), KeyRoute::FocusRing);
        assert_eq!(calls.get(), 1);
    }
}
