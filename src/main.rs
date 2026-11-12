// Grids: multi-pane file manager. Shell tree on the left, up to 4 columns of up to 4 Explorer views on the right.
// Each pane hosts the real Explorer control (IExplorerBrowser), so context menus, drag & drop, rename,
// thumbnails, columns etc. come from Windows itself.
#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::cell::{Cell, RefCell};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Once, OnceLock};
use std::time::{Duration, Instant};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, MoveFileExW, FILE_FLAGS_AND_ATTRIBUTES, MOVE_FILE_FLAGS};
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS, SFGAO_FLAGS, SFGAO_FOLDER, SFGAO_STREAM};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, OpenProcess, INFINITE,QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{AccessibleObjectFromPoint, CLSID_AccPropServices, IAccPropServices, IAccessible, PROPID_ACC_NAME, ROLE_SYSTEM_LIST, ROLE_SYSTEM_LISTITEM};
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

fn main() -> Result<()> {
    unsafe {
        let args: Vec<String> = std::env::args().skip(1).map(resolve).collect();
        // Single instance: hand the arguments to the running window and leave.
        if let Some(prev) = running_instance() {
            if !args.is_empty() {
                let data: Vec<u16> = args.join("\n").encode_utf16().collect();
                let cd = CopyData { dw: COPY_ID, cb: (data.len() * 2) as u32, lp: data.as_ptr() as _ };
                let lp = LPARAM(&cd as *const CopyData as isize);
                SendMessageTimeoutW(prev, WM_COPYDATA, WPARAM(0), lp, SMTO_ABORTIFHUNG, 5000, None);
            }
            if IsIconic(prev).as_bool() {
                _ = ShowWindow(prev, SW_RESTORE);
            }
            _ = SetForegroundWindow(prev);
            return Ok(());
        }

        OleInitialize(None)?; // OLE (not just COM) for drag & drop
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
        // a new install (no settings yet) follows Windows' light/dark
        let mut s = ini.map(|t| Session::parse(&t)).unwrap_or(Session { follow: true, ..Session::default() });
        if let Some(d) = theme_target(s.follow, system_dark(), s.dark) {
            s.dark = d;
        }
        match session_arg(&args).and_then(|f| std::fs::read_to_string(f).ok()) {
            Some(t) => s.take_layout(&Session::parse(&t)),
            None => apply_args(&mut s, args),
        }

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
        KO.store(s.ko, Ordering::Relaxed);
        TABS_ON.store(s.tabs_on, Ordering::Relaxed);
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
        // caption in the ground colour, so title bar and toolbar read as one surface
        let caption = cref(pal().bg);
        _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
        let square = DWMWCP_DONOTROUND;
        _ = DwmSetWindowAttribute(main, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
        let dpi = GetDpiForWindow(main) as i32;
        if s.side <= 0 {
            s.side = 240 * dpi / 96;
        }
        let win = s.win;
        let tip_style = WS_POPUP | WINDOW_STYLE(0x01 | 0x02); // TTS_ALWAYSTIP | TTS_NOPREFIX
        let tip = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, None, tip_style, 0, 0, 0, 0, Some(main), None, None, None)?;
        app(|a| (a.s, a.main, a.dpi, a.view_menu, a.tip) = (s, main, dpi, view_menu, tip));
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
        if let Ok((t, th)) = create_tree(main) {
            app(|a| (a.tree, a.tree_hwnd) = (Some(t), th));
        }
        apply_theme();
        apply_font();
        refresh_drives(); // laid out by the relayout below
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
                if GetMonitorInfoW(MonitorFromRect(&saved, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
                    fixed = fit_saved(saved, mi.rcMonitor, mi.rcWork);
                    // still hidden: move onto the target monitor first, so a DPI change happens here and not after
                    // the placement below (WM_DPICHANGED would apply Windows' scaled size on top of it)
                    let (dx, dy) = (mi.rcWork.left - mi.rcMonitor.left, mi.rcWork.top - mi.rcMonitor.top);
                    let (w, h) = (fixed.right - fixed.left, fixed.bottom - fixed.top);
                    _ = SetWindowPos(main, None, fixed.left + dx, fixed.top + dy, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                let wp = WINDOWPLACEMENT {
                    length: size_of::<WINDOWPLACEMENT>() as u32,
                    showCmd: if max != 0 { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNORMAL.0 as u32 },
                    rcNormalPosition: fixed,
                    ..Default::default()
                };
                _ = SetWindowPlacement(main, &wp);
            }
            None => {
                let mut r = RECT::default();
                _ = GetWindowRect(main, &mut r);
                let ok = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool();
                let f = fit(r, mi.rcWork);
                if ok && f != r {
                    _ = SetWindowPos(main, None, f.left, f.top, f.right - f.left, f.bottom - f.top, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                _ = ShowWindow(main, SW_SHOWNORMAL);
            }
        }
        relayout();
        focus_view(app(|a| a.s.active));
        let file = app(|a| a.s.file.clone());
        let saved = std::fs::read_to_string(&file).ok();
        app(|a| a.base = Some(startup_base(&a.s, saved.as_deref())));
        refresh_save_button();
        SetTimer(Some(main), TIMER_STATUS, 300, None);
        // taken by another app: the menu shows it off, s.hotkey stays on so the next start tries again
        if app(|a| a.s.hotkey) {
            let on = set_hotkey(main, true);
            app(|a| a.hot_on = on);
        }
        if app(|a| a.s.dlgjump) {
            dialog_jump::enable(true);
        }

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
        // The window is gone; copies still running finish first. Waiting keeps COM calls and messages
        // moving, in case a shell extension calls back into this apartment.
        for job in JOBS.take() {
            if !job.is_finished() {
                let h = HANDLE(std::os::windows::io::AsRawHandle::as_raw_handle(&job));
                _ = CoWaitForMultipleHandles((COWAIT_DISPATCH_CALLS | COWAIT_DISPATCH_WINDOW_MESSAGES).0 as u32, INFINITE, &[h]);
            }
            _ = job.join();
        }
        restart_updated(); // an update was just put in place: the new exe takes over
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let g = fav_geo(3, 96);
        let r = |k: usize| g.rows[k].0;
        assert_eq!((fav_slot(&g, r(0).top - 1), fav_slot(&g, r(2).bottom + 5), fav_slot(&fav_geo(0, 96), 50)), (0, 3, 0));
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

        // only plain Alt+↑/↓
        assert_eq!((fav_step(VK_UP, false, true, false), fav_step(VK_DOWN, false, true, false)), (Some(-1), Some(1)));
        for (c, a, s) in [(true, true, false), (false, true, true), (true, true, true), (false, false, false)] {
            assert_eq!((fav_step(VK_UP, c, a, s), fav_step(VK_DOWN, c, a, s)), (None, None));
        }
        assert_eq!((fav_step(VK_LEFT, false, true, false), fav_step(VK_F4, false, true, false)), (None, None));
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
        let names = |show| kid_names(all().into_iter().map(|(n, a)| (n.to_string(), a)), show);
        assert_eq!(names(false), (vec!["2".to_string(), "10".into(), "a".into()], false));
        assert_eq!(names(true), (vec!["2".to_string(), "10".into(), "a".into(), "Secret".into()], false));
        let (v, cut) = kid_names((0..501).map(|k| (format!("f{k}"), d)), false);
        assert_eq!((v.len(), cut, v[0].as_str(), v[499].as_str()), (500, true, "f0", "f499"));
        assert_eq!(kid_names((0..500).map(|k| (format!("f{k}"), d)), false).1, false);
    }

    #[test]
    fn kids_late_results_dropped() {
        let mut j = KidsJob { id: 0, hwnd: 0, dir: String::new(), hidden: false, result: None };
        let a = j.ask(1, r"C:\Users", false);
        assert!(j.publish(a, Ok((vec!["A".into()], false))));
        let b = j.ask(1, r"C:\", false);
        assert!(!j.publish(a, Ok((vec!["A".into()], false))));
        assert!(j.take(a).is_none() && j.take(b).is_none());
        assert!(j.publish(b, Err(())));
        assert!(j.take(b).is_some() && j.take(b).is_none());
        // A's late WM_KIDS while B waits: not B's owner, so it leaves B alone
        let w = HWND(1 as _);
        let ask = KidsAsk { id: b, hwnd: w, i: 0, path: r"C:\Users\Public".into(), gen: 0, dir: r"C:\".into(), next: None };
        assert!(!ask.owns(a, w) && ask.owns(b, w) && !ask.owns(b, HWND(2 as _)));
        j.cancel();
        assert!(!j.publish(b, Err(())) && j.dir.is_empty());
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
        let walk = |back, tree| (0..4).map(|p| focus_ring(p, back, tree)).collect::<Vec<_>>();
        assert_eq!(walk(false, true), [1, 2, 3, 0]);
        assert_eq!(walk(true, true), [3, 0, 1, 2]);
        assert_eq!(walk(false, false)[..3], [1, 2, 0]);
        assert_eq!(walk(true, false)[..3], [2, 0, 1]);
    }

    #[test]
    fn hidden_tree_keeps_its_width() {
        // hidden, narrow window, column drag: the clamped 536 must not replace 600
        assert_eq!(kept_side(false, 600, 536), 600);
        assert_eq!(kept_side(true, 600, 536), 536); // shown: as before
        let s = Session::parse(&Session { tree: false, side: 600, ..Session::default() }.to_ini());
        assert_eq!((s.tree, s.side), (false, 600));
        assert!(Session::parse("side=300\ndblup=1\n").tree); // older ini: shown
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
    fn tree_input_kept_only_for_tree_keys() {
        assert!(tree_input_after(false, true, false)); // arrow in the tree
        assert!(!tree_input_after(true, true, true)); // Ctrl+Tab / F6 in the tree
        assert!(tree_input_after(true, false, false) && !tree_input_after(false, false, false)); // elsewhere: unchanged
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
        let req = |p: &str| Some(PeekReq { path: p.into(), w: 10, h: 10, hwnd: 0, plain: false });
        let pix = || Peeked { body: Body::Pic(Pix { w: 1, h: 1, bgra: vec![0; 4] }), icon: -1, meta: Vec::new() };
        let card = || Peeked { body: Body::Card { icon: None, rows: Vec::new() }, icon: -1, meta: Vec::new() };
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
    fn folder_size_text() {
        let kb = human(1024);
        assert_eq!(size_text(false, 0, true, None, false), " (calculating…)");
        assert_eq!(size_text(false, 0, true, Some((0, false, false)), false), " (calculating…)");
        assert_eq!(size_text(false, 1024, true, Some((1024, false, false)), false), format!(" ({}…)", human(2048)));
        assert_eq!(size_text(false, 0, true, Some((1024, true, false)), false), format!(" ({kb})"));
        assert_eq!(size_text(false, 0, true, Some((1024, true, true)), false), format!(" ({kb}, partial)"));
        // something missed while collecting the selection isn't shown as a full total
        assert_eq!(size_text(false, 0, true, Some((1024, true, false)), true), format!(" ({kb}, partial)"));
        assert_eq!(size_text(true, 0, true, Some((1024, true, false)), true), format!(" ({kb}, 일부만)"));
        assert_eq!(size_text(false, 1024, false, None, false), format!(" ({kb})"));
        assert_eq!(size_text(false, 1024, false, None, true), format!(" ({kb}, partial)"));
        assert_eq!(size_text(false, 0, false, None, false), "");
        assert_eq!(size_text(false, 0, false, None, true), " (size unknown)");
        assert_eq!(size_text(true, 0, false, None, true), " (크기 모름)");
    }

    // WCAG 2 contrast ratio of two 0xRRGGBB colours.
    fn contrast(a: u32, b: u32) -> f64 {
        let lum = |c: u32| {
            let lin = |v: u32| {
                let s = v as f64 / 255.0;
                if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * lin(c >> 16 & 0xFF) + 0.7152 * lin(c >> 8 & 0xFF) + 0.0722 * lin(c & 0xFF)
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
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
        v
    }

    // Today's contrast of each pair, rounded down. A change may raise these; lowering one means fixing the
    // colour or saying in review why that pair may get harder to read, then lowering just that value.
    #[rustfmt::skip]
    const CONTRAST_FLOOR: [[f64; 31]; 2] = [
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
        let (f, d) = (path("f.bin"), path("d"));
        let none = Sel { file_bytes: 0, dirs: vec![], partial: true };
        assert_eq!(collect_sel(None, |_| None), none); // GetSelection / GetCount failed
        assert_eq!(collect_sel(Some(1), |_| None), none); // GetItemAt / no file system path
        assert_eq!(collect_sel(Some(1), |_| Some(path("gone"))), none);
        assert_eq!(collect_sel(Some(1), |_| Some(r"C:\".into())), none); // a drive root isn't walked
        let both = [f.clone(), d.clone()];
        assert_eq!(collect_sel(Some(2), |k| Some(both[k as usize].clone())), Sel { file_bytes: 7, dirs: vec![d], partial: false });
        let calls = std::cell::Cell::new(0);
        let s = collect_sel(Some(10_001), |_| {
            calls.set(calls.get() + 1);
            Some(f.clone())
        });
        assert_eq!((s.file_bytes, s.partial, calls.get()), (70_000, true, 10_000));
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
                for text in [KEYS_EN, KEYS_KO] {
                    let g = keys_layout(&key_rows(text), dip * dpi / 96, dpi, &measure);
                    let (pad, gap) = (KEYS_PAD * dpi / 96, 16 * dpi / 96);
                    let mut prev = 0;
                    for r in &g.rows {
                        assert!(r.row.top >= prev, "rows overlap at {dpi} dpi, {dip} dip");
                        for k in &r.keys {
                            // key column: from the padding to a gap before the description
                            assert!(k.left >= pad && k.right + gap <= r.desc.left && k.bottom <= r.row.bottom);
                        }
                        let cap_lines = r.keys.iter().map(|k| k.bottom).max().unwrap() - r.row.top;
                        assert!(r.row.bottom - r.row.top >= cap_lines.max(r.desc.bottom - r.desc.top));
                        assert!(r.desc.bottom <= r.row.bottom && r.desc.right <= dip * dpi / 96 - pad);
                        prev = r.row.bottom;
                    }
                    assert!(prev <= g.h && g.titles[1].top > g.titles[0].bottom);
                }
            }
        }
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
                    assert_eq!(fit_saved(saved, mon, work), saved);
                }
            }
            // too big or off screen: in screen coordinates the result lies inside rcWork
            for saved in [r(-300, -200, 3000, 2000), r(1500, 900, 2300, 1500)] {
                let f = fit_saved(saved, mon, work);
                let (dx, dy) = (work.left - mon.left, work.top - mon.top);
                assert_eq!(fit(r(f.left + dx, f.top + dy, f.right + dx, f.bottom + dy), work), r(f.left + dx, f.top + dy, f.right + dx, f.bottom + dy));
            }
        }
        assert_eq!(fit_saved(r(1500, 900, 2300, 1500), mon, r(0, 40, 1920, 1080)), r(1120, 440, 1920, 1040));
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
                let g = fav_geo(n, dpi);
                assert_eq!(g.rows.len(), n);
                if n == 0 {
                    assert_eq!(g.h, fav_geo(1, dpi).h); // room for the "no favourites" line
                }
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
        let rows = csv_rows("a,\"b,c\",\"d \"\"q\"\"\"\r\n1,2\n", ',', 100);
        assert_eq!(rows, vec![vec!["a", "b,c", "d \"q\""], vec!["1", "2"]]);
        assert_eq!(csv_rows("x\n\"multi\nline\",y", ',', 100)[1], vec!["multi\nline", "y"]);
        assert_eq!(csv_rows("1\n2\n3\n", ',', 2).len(), 2);
        assert_eq!((csv_sep("a;b;c\n"), csv_sep("a,b;c\n")), (';', ','));
    }

    // A ZIP made by hand: one stored entry, one deflated, a Korean name without the UTF-8 flag.
    fn zip_bytes(files: &[(&[u8], &[u8], bool, u16)]) -> Vec<u8> {
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
        let text = xml_text(&decode_text(&doc), &["w:t"], &["w:p"], &[], &[]);
        assert_eq!(text.lines().next(), Some("안녕 & hi"));
        assert!(zip_entries(&mut std::io::Cursor::new(b"not a zip at all".to_vec())).is_none());
    }

    #[test]
    fn xml_and_sheets() {
        let x = r#"<a:p><a:r><a:t>One</a:t></a:r><a:br/><a:r><a:t xml:space="preserve"> two</a:t></a:r></a:p><a:p><a:t/><a:t>&#xD55C;&lt;</a:t></a:p>"#;
        assert_eq!(xml_text(x, &["a:t"], &["a:p"], &[], &["a:br"]), "One\n two\n한<\n");
        assert_eq!((cell_ref("A1"), cell_ref("AB12"), cell_ref("1A")), (Some((0, 0)), Some((27, 11)), None));
        let ss = xlsx_strings("<sst><si><t>이름</t></si><si><r><t>A</t></r><r><t>B</t></r></si></sst>");
        assert_eq!(ss, vec!["이름", "AB"]);
        let sheet = r#"<row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row><row r="2"><c r="A2"><v>0.30000000000000004</v></c><c r="B2" t="inlineStr"><is><t>x</t></is></c></row>"#;
        assert_eq!(xlsx_rows(sheet, &ss, 100, 100), vec![vec!["이름", "", "AB"], vec!["0.3", "x"]]);
        assert_eq!(xlsx_rows(sheet, &ss, 1, 2), vec![vec!["이름"]]); // limits
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

    #[test]
    fn markdown_lines() {
        let l = md_lines("# T\n\nsome **bold** `x`\n- a\n  - b\n> q\n```\ncode **\n```\n---");
        let styles: Vec<Style> = l.iter().map(|x| x.1).collect();
        use Style::*;
        assert_eq!(styles, vec![H1, Plain, Plain, Bullet, Bullet, Quote, Mono, Quote]);
        assert_eq!((l[2].0.as_str(), l[3].0.as_str(), l[4].0.as_str(), l[6].0.as_str()), ("some bold x", "• a", "    • b", "code **"));
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
}
