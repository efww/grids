// Maguro: multi-pane file manager. Shell tree on the left, up to 4 columns of up to 4 Explorer views on the right.
// Each pane hosts the real Explorer control (IExplorerBrowser), so context menus, drag & drop, rename,
// thumbnails, columns etc. come from Windows itself.
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod imports;
mod langkey;
mod i18n;
mod keys;
mod dialogjump;
mod app;
mod basket;
mod ui;
mod pane;
mod session;
mod shell;
mod settings;
mod folderview;
mod everything;
mod rename;
mod winmerge;
mod excel;
mod folderapp;
mod crash;
mod winpath;
mod remote;
mod watch;

use imports::*;

pub(crate) use app::*;
pub(crate) use i18n::*;
pub(crate) use pane::*;
pub(crate) use session::*;
pub(crate) use shell::*;
pub(crate) use ui::*;
pub(crate) use settings::store;

fn main() -> Result<()> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if crash::helper(&raw) {
        return Ok(());
    }
    // Hooks and the fast-fail watcher go in before settings, COM, or a window, so a fault
    // while a folder opens still leaves a report. Helper processes above must not install them.
    crash::install();
    unsafe {
        // A moved exe keeps the folder registration pointed at the binary that is running.
        folderapp::repair_moved_exe();
        let mut raw: Vec<String> = std::env::args().skip(1).collect();
        wait_for_previous(&mut raw);
        // A theme restart asks for the tabs just saved, not the startup-mode layout.
        let keep_tabs = take_keep_tabs(&mut raw);
        let args: Vec<String> = raw.into_iter().map(resolve).collect();
        // shell: and CLSID paths are not filesystem folders. Explorer opens those; this process
        // does not start a window just to hold them.
        let had_args = !args.is_empty();
        let args = folderapp::take_launch_args(args);
        if args.is_empty() && had_args {
            return Ok(());
        }
        // Read first so single=0 can opt out of handing arguments to the running window.
        // one_window still hands off: the running window comes forward and opens paths as tabs.
        // maguro.json is the store. If it is missing, a previous grids.json is copied;
        // otherwise grids.ini or qdir.ini is imported read-only. Those sources stay untouched.
        // A file that is there but unreadable must not be overwritten with defaults.
        let mut s = loop {
            match store::load() {
                Ok(s) => break s,
                Err(e) => {
                    let text = HSTRING::from(format!(
                        "Maguro could not read its settings file (maguro.json): {e}. Retry, or Cancel to quit without changing it.\n\n\
                         Maguro 설정 파일(maguro.json)을 읽지 못했습니다: {e}. 다시 시도하거나, 취소하면 파일을 바꾸지 않고 종료합니다.\n\n\
                         Maguro の設定ファイル (maguro.json) を読めませんでした: {e}。再試行するか、キャンセルするとファイルを変えずに終了します。"
                    ));
                    if MessageBoxW(None, &text, w!("Maguro"), MB_RETRYCANCEL | MB_ICONWARNING) != IDRETRY {
                        return Ok(());
                    }
                }
            }
        };
        // Old named sessions that stored every tab are split into one file per tab.
        // The open window (maguro.json) is only retargeted in memory: its tabs stay.
        let split_maps = migrate_named_sessions();
        if !split_maps.is_empty() {
            retarget_session_links(&mut s, &split_maps);
            let pending = take_stashed().into_iter().map(|mut v| {
                retarget_window_value(&mut v, &split_maps);
                v
            }).collect();
            stash_windows(pending);
        }
        let boot = s.boot;
        let restore = session_arg(&args).is_none() && args.is_empty();
        // `--keep-tabs` is a theme restart: the file already has the windows the user was looking at.
        if restore && !keep_tabs {
            apply_boot(&mut s);
        }
        // Extra windows belong to "restore last state". A session file, paths, or another startup mode do not.
        // Single-window mode never opens the other saved windows. A theme restart brings back every window that was open.
        if !keep_tabs && (!restore || boot != 0 || !allows_another_window(s.one_window)) {
            stash_windows(Vec::new());
        }
        s.dark = resolve_dark(s.theme);
        // single=0 starts another process, unless single-window mode is on: that still hands the launch over.
        if launch_plan(s.single, s.one_window) != LaunchPlan::NewProcess {
            if let Some(prev) = running_instance() {
                // Empty arguments still open a window. The running process is the one that can take focus.
                // The parent id lets that window tell a folder our own view just launched (parent is us)
                // from Explorer or any other program (parent is not us).
                let body = folderapp::handoff_text(folderapp::parent_pid(), &args);
                let data: Vec<u16> = body.encode_utf16().collect();
                let cd = CopyData { dw: COPY_ID, cb: (data.len() * 2) as u32, lp: data.as_ptr() as _ };
                let mut pid = 0u32;
                GetWindowThreadProcessId(prev, Some(&mut pid));
                _ = AllowSetForegroundWindow(pid);
                let lp = LPARAM(&cd as *const CopyData as isize);
                SendMessageTimeoutW(prev, WM_COPYDATA, WPARAM(0), lp, SMTO_ABORTIFHUNG, 5000, None);
                return Ok(());
            }
        }

        OleInitialize(None)?; // OLE (not just COM) for drag & drop
        if let Some(f) = session_arg(&args) {
            match std::fs::read_to_string(f) {
                Ok(t) => {
                    s.take_layout(&Session::parse(&t));
                    link_loaded_window(&mut s, f);
                }
                Err(_) => apply_args(&mut s, args),
            }
        } else {
            apply_args(&mut s, args);
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
            lpszClassName: w!("MaguroMain"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS, // the plate is drawn to the window size
            lpfnWndProc: Some(pane_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroPane"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(bar_proc),
            hInstance: hinst,
            lpszClassName: w!("MaguroBar"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(fav_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroFav"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(load_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroLoad"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(grid_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroPicker"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(side_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroSide"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(match_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroMatch"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(keys_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroKeys"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(help_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroHelp"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(basket::basket_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroBasket"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS, // header maximises; a double-click on a picture or PDF resets zoom
            lpfnWndProc: Some(peek_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroPeek"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(peek_host_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroPeekHost"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(preview_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroPreview"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(preview_host_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroPreviewHost"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(tabs_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("MaguroTabs"),
            ..Default::default()
        });
        install_languages();
        apply(s.lang);
        // After COM is up and before the hosted Explorer view is created, so a listed DLL
        // is not loaded into this process. The registry is not written.
        crash::show_previous();
        shell::blockext::apply(&s.blocked_shell_extensions);
        let main = open_window(s)?;
        watch::start();
        apply_ext(app(|a| a.s.ext));
        if restore && boot != 0 && !keep_tabs {
            save_session();
        }
        // Other saved windows open after this one is on screen, so the first paint is one window.
        SetTimer(Some(main), TIMER_WINS, 50, None);
        startup(main, app(|a| a.s.update_check));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !pre_translate(&msg) {
                let up = up_candidate(&msg);
                let (click_time, click_pt) = (msg.time, msg.pt);
                _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                match up {
                    Some(RowClick::Up(i, g, svh)) => {
                        let query = || {
                            let view = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok()).map(|h| h.0 as isize);
                            let sel = view_of::<IFolderView2>(i).and_then(|fv| fv.ItemCount(SVGIO_SELECTION).ok()).map(|n| n as u32);
                            (view, sel)
                        };
                        if confirm((g, svh), query, gen) {
                            browse(i, SBSP_PARENT);
                        }
                    }
                    // A details cell reported as text or a graphic. The shell's own hit test misses it
                    // once the accessibility probe has run, so open that row when the shell did not.
                    Some(RowClick::Open(i, g)) if gen() == g && !shell_activated(click_time) => open_row_cell(i, click_pt),
                    _ => {}
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
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lang_ini() {
        assert_eq!(Session::default().lang, Lang::Auto);
        assert_eq!(Session::parse("").lang, Lang::Auto);
        assert_eq!(Session::parse("bar=0\n").lang, Lang::Auto);
        assert_eq!(Session::parse("lang=auto").lang, Lang::Auto);
        assert_eq!(Session::parse("lang=en").lang, Lang::En);
        assert_eq!(Session::parse("lang=ko").lang, Lang::Ko);
        assert_eq!(Session::parse("lang=ja").lang, Lang::Ja);
        assert!(Session::parse("dblup=1").single);
        assert!(!Session::parse("single=0").single);
        let fresh = Session::default();
        let round = Session::parse(&fresh.to_ini());
        assert_eq!(round.theme, 0);
        assert_eq!((round.bar, round.stat, round.view_new, round.start_new), (true, true, 4, 0));
        assert_eq!((round.ext, round.tree_on, round.edge, round.confirm, round.del), (true, true, true, true, 0));
        assert!(round.fav_top && fresh.fav_top);
        assert!(!fresh.to_ini().contains("sideorder"));
        let tree_top = Session::parse("sideorder=tree\nsplit=0.3\n");
        assert!(!tree_top.fav_top && tree_top.to_ini().contains("sideorder=tree\n"));
        assert!((Session::parse(&tree_top.to_ini()).side_split - 0.3).abs() < 1e-9);
        assert!(!Session::parse(&tree_top.to_ini()).fav_top);
        assert!(Session::parse("sideorder=fav\n").fav_top);
        assert!(!Session::parse("sideorder=fav\n").to_ini().contains("sideorder"));
        assert!(Session::parse("tree=1\nsplit=0.25\n").fav_top);
        let mut kept = Session::default();
        kept.take_layout(&Session::parse("sideorder=tree\n"));
        assert!(kept.fav_top);
        assert_eq!((round.dropm, round.layout_new, round.boot, round.autosave, round.dot), (0, 3, 0, false, true));
        assert!(round.new_name.is_empty() && round.start_path.is_empty() && round.boot_file.is_empty());
        // An older file has dark= and no theme=. That stays an explicit choice, and new keys keep defaults.
        let old = Session::parse("dark=1\ndblup=1\n");
        assert_eq!(old.theme, 2);
        assert!(old.dark && old.bar && old.dot && old.drive_buttons && old.space_preview);
        assert_eq!(old.layout_new, 0);
        let t = Session::parse("theme=auto\ndark=1\nfont=2\nbar=0\nstat=0\nviewnew=1\nstart=2\nstartp=C:\\Work\next=0\ntree=0\nedge=0\nconfirm=0\ndel=1\nnewname=note.txt\ndrop=1\nboot=1\nbootf=C:\\s.grids\nautosave=1\ndot=0\nfav=C:\\WORK\tWork\n");
        assert_eq!(t.theme, 0);
        assert!(t.dark);
        assert!(!t.to_ini().contains("font="));
        assert!(!t.bar && !t.stat && !t.ext && !t.tree_on && !t.edge && !t.confirm && !t.dot);
        assert_eq!(t.view_new, 1);
        assert_eq!(t.start_new, 2);
        assert_eq!(t.start_path, "C:\\Work");
        assert_eq!(t.del, 1);
        assert_eq!(t.new_name, "note.txt");
        assert_eq!(t.dropm, 1);
        assert_eq!(t.layout_new, 0);
        assert_eq!(t.boot, 1);
        assert_eq!(t.boot_file, "C:\\s.grids");
        assert!(t.autosave);
        assert_eq!(fav_path(&t.favs[0]), "C:\\WORK");
        assert_eq!(fav_label(&t.favs[0]), "Work");
        let back = Session::parse(&t.to_ini());
        assert_eq!((back.new_name, back.start_path, back.boot_file, back.favs), (t.new_name.clone(), t.start_path.clone(), t.boot_file.clone(), t.favs.clone()));
        let mut s = Session::default();
        s.lang = Lang::Ja;
        s.single = false;
        let back = Session::parse(&s.to_ini());
        assert_eq!((back.lang, back.single), (Lang::Ja, false));
        assert!(back.to_ini().contains("lang=ja\n"));
        assert!(Session::parse("lang=ko").to_ini().contains("lang=ko\n"));
        assert!(Session::parse("side=300\ndblup=1\n").hotkey); // an older file is on
        assert!(!Session::parse("hotkey=0\n").hotkey);
        assert!(!Session::parse(&Session { hotkey: false, ..Session::default() }.to_ini()).hotkey);
        let mut cur = Session::default();
        cur.take_layout(&Session::parse("hotkey=0\n"));
        assert!(cur.hotkey); // a loaded layout does not change the app-wide hotkey
        assert!(!layout_differs(&Session { hotkey: false, ..Session::default() }, &Session::default()));
        assert!(Session::parse("side=300\n").dialog_jump);
        assert!(!Session::parse("dialogjump=0\n").dialog_jump);
        assert!(Session::parse("dialogjump=1\n").dialog_jump);
        assert!(!Session::parse(&Session { dialog_jump: false, ..Session::default() }.to_ini()).dialog_jump);
        cur.take_layout(&Session::parse("dialogjump=0\n"));
        assert!(cur.dialog_jump); // a loaded layout does not change the dialog jump
        assert!(!layout_differs(&Session { dialog_jump: false, ..Session::default() }, &Session::default()));
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
        assert_eq!(titles, ["Downloads - Maguro", "Maguro", "C: - Maguro"]);
    }

    #[test]
    fn dblclick_up() {
        let c = |time, x, y| Click { win: 7, gen: 1, time, x, y, kind: 1 };
        let next = |prev, now, bg| next_click(prev, now, if bg { 1 } else { 0 }, 500, 4, 4);
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

        assert!(bg_role(33, None) && bg_role(33, Some(9)) && bg_role(34, Some(33)));
        assert!([20, 43, 25, 3, 62, 40, 41, 42].iter().all(|&r| !bg_role(r, None)));
        // The details-view header: a list inside the item list.
        assert!(!bg_role(33, Some(33)));
        // Windows 11 details cells: icon is a graphic, name/date/size are text. Those are the row, not empty space.
        assert!(cell_role(40) && cell_role(41) && cell_role(42));
        assert!(!cell_role(33) && !cell_role(34) && !cell_role(62));
        let cell = |time| Click { kind: 2, ..c(time, 10, 10) };
        assert!(next_click(Some(cell(100)), cell(200), 2, 500, 4, 4).1);
        assert!(!next_click(Some(c(100, 10, 10)), cell(200), 2, 500, 4, 4).1);

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
    fn tabs_session() {
        let mut s = Session::default();
        s.set_heights(vec![1]);
        s.paths[0] = r"C:\one".into();
        s.views[0] = (4, 16);
        s.anchor();
        let mut other = Session::default();
        other.set_heights(vec![2, 2]);
        other.paths[1] = r"D:\two".into();
        other.views[1] = (3, 16);
        other.cols[1] = "{g}/1=80;".into();
        other.active = 1;
        s.tabs.push(Tab::from_live(&other, "Work".into(), true, vec!["*.pdf".into()]));
        assert_eq!(Session::parse(&s.to_ini()), s);
        assert!(s.snapshot().contains("\"locked\":true") || s.snapshot().contains("\"locked\": true"));
        assert!(!s.snapshot().contains("win="));

        let old = Session::parse("cols=1,3\np0=C:\\a\nactive=1\n");
        assert_eq!((old.tabs.len(), old.tabs[0].name.as_str(), old.tabs[0].locked), (1, "1", false));
        assert_eq!((old.heights.clone(), old.paths[0].as_str(), old.active), (vec![1, 3], r"C:\a", 1));
        assert_eq!(next_tab_name(&old.tabs), "2");
        assert!(!can_close_tab(&old.tabs, 0));

        let mut tabs = old.tabs.clone();
        tabs.push(Tab::from_live(&Session::default(), "2".into(), false, vec![]));
        tabs[0].locked = true;
        assert!(!can_close_tab(&tabs, 0) && can_close_tab(&tabs, 1));
        assert_eq!(close_other_indexes(&tabs, 0), vec![1]);
        assert!(close_other_indexes(&tabs, 1).is_empty());
        assert_eq!(duplicate_name(&tabs, "2"), "3");
        assert_eq!(duplicate_name(&tabs, "Work"), "Work (2)");
        let mut again = tabs.clone();
        again.push(Tab::from_live(&Session::default(), "Work (2)".into(), false, vec![]));
        assert_eq!(duplicate_name(&again, "Work"), "Work (3)");

        let mut names = vec!["1".to_string(), "2".to_string(), "3".to_string()];
        reorder_vec(&mut names, 0, 2);
        assert_eq!(names, ["2", "3", "1"]);
        assert_eq!(moved_active(0, 0, 2), 2);
        assert_eq!(moved_active(0, 2, 0), 1);
        assert_eq!(moved_active(1, 0, 2), 0);

        let lay = tab_lay(800, 3, &[false, true, false], 96, 0);
        assert_eq!(lay.body.len(), 3);
        assert_eq!(lay.body[0].right - lay.body[0].left, 135);
        assert!(lay.overflow.is_none() && lay.first == 0 && lay.count == 3);
        let hidpi = tab_lay(1600, 3, &[false, true, false], 192, 0);
        assert_eq!(hidpi.body[0].right - hidpi.body[0].left, 270);
        // 8 tabs at the 44px minimum need 406px; 406 shrinks exactly to that minimum and still fits.
        let crowd = tab_lay(406, 8, &[false; 8], 96, 0);
        let cw = crowd.body[0].right - crowd.body[0].left;
        assert!(crowd.overflow.is_none() && cw < 135 && cw >= 44);
        assert!(lay.body.windows(2).all(|w| w[0].right <= w[1].left));
        assert!(lay.plus.left >= lay.body[2].right && lay.plus.right <= 800);
        // Past the minimum, a chevron appears and the scroll offset clamps so the last tab stays on screen.
        let over = tab_lay(200, 12, &[false; 12], 96, 0);
        assert!(over.overflow.is_some() && over.count >= 1 && over.count < 12);
        assert!(over.body[0].right - over.body[0].left >= 44);
        assert!(over.body[11].right <= over.body[11].left);
        let o = over.overflow.unwrap();
        assert!(over.plus.right <= o.left && o.right <= 200);
        assert!(matches!(tab_hit(&over, o.left + 1, o.top + 1), TabHit::Overflow));
        assert!(matches!(tab_hit(&over, over.plus.left + 1, over.plus.top + 1), TabHit::Plus));
        let end = tab_lay(200, 12, &[false; 12], 96, 10_000);
        assert_eq!(end.first + end.count, 12);
        assert!(end.first < 10_000 && end.body[11].right > end.body[11].left);
        let active = 11usize;
        let scroll = scroll_to_show(0, active, over.count, 12);
        let shown = tab_lay(200, 12, &[false; 12], 96, scroll);
        assert!(shown.overflow.is_some() && shown.body[active].right > shown.body[active].left);
        assert_eq!(scroll_to_show(scroll, active, shown.count, 12), scroll);
        assert_eq!(scroll_to_show(3, 4, 2, 12), 3);
        assert_eq!(scroll_to_show(0, 11, 2, 12), 10);
        // A locked tab keeps the close slot. The glyph is a lock and it does not close.
        assert!(lay.close.iter().all(|c| c.is_some()));
        assert!(lay.lock[1] && !lay.lock[0]);
        let c = lay.close[0].unwrap();
        let locked = lay.close[1].unwrap();
        assert_eq!(c.right - c.left, locked.right - locked.left);
        assert!(c.left >= lay.body[0].left && c.right <= lay.body[0].right);
        assert!(matches!(tab_hit(&lay, lay.plus.left + 2, lay.plus.top + 2), TabHit::Plus));
        assert!(matches!(tab_hit(&lay, c.left + 1, c.top + 1), TabHit::Close(0)));
        assert!(matches!(tab_hit(&lay, locked.left + 1, locked.top + 1), TabHit::Lock(1)));
        assert!(!middle_closes(&lay, TabHit::Lock(1)) && !middle_closes(&lay, TabHit::Tab(1)));
        assert!(middle_closes(&lay, TabHit::Close(0)) && middle_closes(&lay, TabHit::Tab(0)));
        assert_eq!(close_glyph(true), ICON_LOCK);
        assert_eq!(close_glyph(false), ICON_CLOSE);
        assert_eq!(close_ink(true, true, 1, 2), 2);
        assert_eq!(close_ink(false, true, 1, 2), 1);
        assert!(matches!(tab_hit(&lay, lay.body[1].left + 4, lay.body[1].top + 4), TabHit::Tab(1)));
        let hi = tab_lay(3200, 1, &[true], 192, 0).close[0].unwrap();
        let lo = tab_lay(1600, 1, &[true], 96, 0).close[0].unwrap();
        assert_eq!(hi.right - hi.left, (lo.right - lo.left) * 2);

        let junk = Session::parse("cols=1\ntabs=[{\"id\":\"later\"}]\n");
        assert_eq!((junk.tabs.len(), junk.heights.clone(), junk.tabs[0].name.as_str()), (1, vec![1], "1"));

        let mut wide = other.clone();
        wide.tabs.push(Tab::from_live(&Session::default(), "B".into(), false, vec![]));
        wide.tab = 1;
        wide.anchor();
        let back = Session::parse(&wide.to_ini());
        assert_eq!(back, wide);
        assert_eq!(back.tab, 1);
        assert_eq!(back.tabs[0].name, "1");
        assert_eq!((back.heights.clone(), back.paths[1].as_str(), back.active), (vec![2, 2], r"D:\two", 1));
    }

    #[test]
    fn three_tabs_restore_from_json() {
        assert_eq!(Session::default().boot, 0);
        let mut s = Session::default();
        s.win = Some([10, 20, 1100, 700, 0]);
        s.set_heights(vec![1]);
        s.paths[0] = r"C:\alpha".into();
        s.views[0] = (4, 16);
        s.cols[0] = "{g}/1=40;".into();
        s.anchor();
        s.tabs[0].name = "Docs".into();
        s.tabs[0].filters[0] = "*.txt".into();

        let mut pics = Session::default();
        pics.set_heights(vec![2]);
        pics.paths[0] = r"D:\beta".into();
        pics.paths[1] = r"D:\gamma".into();
        pics.views[1] = (3, 32);
        pics.cols[1] = "{g}/2=80;".into();
        pics.active = 1;
        s.tabs.push(Tab::from_live(&pics, "Pics".into(), true, vec!["*.png".into(), "*.jpg".into()]));

        let mut work = Session::default();
        work.set_heights(vec![1, 1]);
        work.paths[0] = r"E:\delta".into();
        work.paths[1] = r"E:\epsilon".into();
        s.tabs.push(Tab::from_live(&work, "Work".into(), false, vec![]));
        s.tab = 2;
        s.publish();

        let text = store::sync_text(&s);
        let mut back = store::import_text(&text).unwrap();
        apply_boot(&mut back);
        assert_eq!(back.boot, 0);
        assert_eq!(back.tabs.len(), 3);
        assert_eq!(back.tab, 2);
        assert_eq!(back.win, Some([10, 20, 1100, 700, 0]));
        assert_eq!(back.tabs[0].name, "Docs");
        assert!(!back.tabs[0].locked);
        assert_eq!(back.tabs[0].paths[0], r"C:\alpha");
        assert_eq!(back.tabs[0].filters[0], "*.txt");
        assert_eq!(back.tabs[0].cols[0], "{g}/1=40;");
        assert_eq!(back.tabs[0].views[0], (4, 16));
        assert!(back.tabs[1].locked);
        assert_eq!(back.tabs[1].name, "Pics");
        assert_eq!((back.tabs[1].paths[0].as_str(), back.tabs[1].paths[1].as_str()), (r"D:\beta", r"D:\gamma"));
        assert_eq!(back.tabs[1].filters[1], "*.jpg");
        assert_eq!(back.tabs[1].cols[1], "{g}/2=80;");
        assert_eq!(back.tabs[1].views[1], (3, 32));
        assert_eq!((back.tabs[1].heights.clone(), back.tabs[1].active), (vec![2], 1));
        assert_eq!(back.tabs[2].name, "Work");
        assert_eq!((back.paths[0].as_str(), back.paths[1].as_str()), (r"E:\delta", r"E:\epsilon"));
        assert_eq!(back.heights, vec![1, 1]);

        // A different startup mode still stores the whole tab list. Only the next launch's boot step uses it.
        s.boot = 2;
        let forced = store::sync_text(&s);
        assert!(forced.contains("\"Docs\"") && forced.contains("\"Pics\"") && forced.contains("\"Work\""));
        let mut wiped = store::import_text(&forced).unwrap();
        assert_eq!(wiped.tabs.len(), 3);
        apply_boot(&mut wiped);
        assert_eq!((wiped.boot, wiped.tabs.len(), wiped.tabs[0].name.as_str()), (2, 1, "1"));
        // This session never changed layout_new, so the fresh default (one pane) is what opens.
        assert_eq!((wiped.layout_new, wiped.heights.clone(), wiped.active), (3, vec![1], 0));
        s.layout_new = 0;
        let mut side = store::import_text(&store::sync_text(&s)).unwrap();
        apply_boot(&mut side);
        assert_eq!((side.layout_new, side.heights.clone(), side.active), (0, vec![1, 1], 0));
        assert!(side.active < side.panes());
    }

    #[test]
    fn tab_color_persists_and_one_tab_session_drops_the_others() {
        let mut s = Session::default();
        s.set_heights(vec![1]);
        s.paths[0] = r"C:\docs".into();
        s.anchor();
        s.tabs[0].name = "Docs".into();
        s.tabs[0].color = Some(0x3D7EE0);
        let mut pics = Session::default();
        pics.set_heights(vec![1]);
        pics.paths[0] = r"D:\pics-only".into();
        s.tabs.push(Tab::from_live(&pics, "Pics".into(), true, vec![]));
        s.tabs[1].color = Some(0xE15A4A);

        let back = store::import_text(&store::sync_text(&s)).unwrap();
        assert_eq!(back.tabs[0].color, Some(0x3D7EE0));
        assert_eq!(back.tabs[1].color, Some(0xE15A4A));
        assert_eq!(back.tabs[0].paths[0], r"C:\docs");
        let again = Session::parse(&s.to_ini());
        assert_eq!((again.tabs[0].color, again.tabs[1].color), (Some(0x3D7EE0), Some(0xE15A4A)));

        let old = store::import_text(r#"{"version":1,"tabs":[{"name":"1","locked":false,"active":true,"layout":{"columns":[1]},"panes":[{"index":0,"path":"C:\\plain"}]}]}"#).unwrap();
        assert_eq!(old.tabs[0].color, None);

        let file = tab_session_text(&s, 0).unwrap();
        let one = Session::parse(&file);
        assert_eq!(one.tabs.len(), 1);
        assert_eq!(one.tabs[0].name, "Docs");
        assert_eq!(one.tabs[0].color, Some(0x3D7EE0));
        assert_eq!(one.paths[0], r"C:\docs");
        assert!(!file.contains("pics-only"));
        assert!(file.contains("3D7EE0"));
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
        // favourites: kept in grids.ini, case-insensitive, never in a named session
        let mut f = Session { favs: vec![r"C:\Work".into(), r"D:\".into()], ..Default::default() };
        f = Session::parse(&f.to_ini());
        assert!(f.is_fav(r"c:\work") && !f.is_fav(r"C:\Work\x") && !f.is_fav(""));
        assert!(fav_removes(&[r"C:\Work"], |p| p.eq_ignore_ascii_case(r"C:\Work")));
        assert!(!fav_removes(&[], |_| true) && !fav_removes(&[r"C:\Work", r"D:\"], |p| p.eq_ignore_ascii_case(r"C:\Work")));
        assert_eq!((fav_anchor(Some(5), 8), fav_anchor(None, 8), fav_anchor(Some(9), 3), fav_anchor(None, 0)), (5, 7, 2, 0));
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
        assert_eq!(fresh_file("New Text Document.txt", |_| false), "New Text Document.txt");
        assert_eq!(fresh_file("New Text Document.txt", |n| n == "New Text Document.txt"), "New Text Document (2).txt");
        assert_eq!(fresh_file("새 텍스트 문서.txt", |n| n == "새 텍스트 문서.txt" || n == "새 텍스트 문서 (2).txt"), "새 텍스트 문서 (3).txt");

        assert_eq!(fresh_folder("New folder", |_| false), "New folder");
        assert_eq!(fresh_folder("New folder", |n| n == "New folder"), "New folder (2)");
        assert_eq!(fresh_folder("New folder", |n| n == "New folder" || n == "New folder (2)"), "New folder (3)");
        assert_eq!(filter_spec(" pdf; *.TXT ;"), "*pdf*;*.TXT");
        assert_eq!((human(10), human(1536), human(5 << 30)), ("10 B".into(), "1.5 KB".into(), "5.0 GB".into()));

        // pane top bar: number, back, fwd, up, address, ⇄, filter left to right, nothing overlapping, inside the plate
        for (w, dpi) in [(800, 96), (260, 96), (1200, 144)] {
            let p = pane_geo(w, 500, dpi, true);
            let row = [p.num, p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.filt, p.fav];
            assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi}");
            assert!(p.num.right > p.num.left && p.fav.right <= w && p.view.top >= p.btn[0].bottom);
        }
        // The number is reserved first. The address starts just after whichever of
        // back, forward and up are still shown. Those buttons give way, from up
        // toward back, so a narrow pane can still show the folder name.
        for dpi in [96, 144] {
            let s = |v: i32| v * dpi / 96;
            for w in 60..=600 {
                let p = pane_geo(w, 500, dpi, true);
                assert_eq!(p.num.right - p.num.left, s(18), "{w}@{dpi}");
                assert_eq!(p.btn[0].left, p.num.right, "{w}@{dpi}");
                let row = [p.num, p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.filt, p.fav];
                assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi}");
                let shown = p.btn[..3].iter().filter(|b| b.right > b.left).count() as i32;
                let gap = if shown > 0 { s(4) } else { 0 };
                assert_eq!(p.addr.left, p.num.right + shown * s(24) + gap, "{w}@{dpi}");
                for c in [p.btn[0], p.btn[1], p.btn[2], p.btn[3], p.addr, p.filt, p.fav] {
                    if c.right > c.left {
                        assert!(c.left >= p.num.right, "{w}@{dpi}");
                    }
                }
            }
        }
        let edit_of = |w, dpi| {
            let s = |v: i32| v * dpi / 96;
            let p = pane_geo(w, 500, dpi, true);
            (p.addr.right - s(6)) - (p.addr.left + s(8))
        };
        // The old cutoff collapsed the address. It now keeps room for the folder name.
        assert!(edit_of(240, 96) >= 66 && edit_of(241, 96) >= 66);
        assert!(edit_of(360, 144) >= 99 && edit_of(362, 144) >= 99);
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
        let totals = dir_totals(&root, &never, &never, &mut quiet, DIR_STACK_CAP).unwrap();
        assert_eq!((totals.bytes, totals.files, totals.folders, totals.partial), (123, 3, 3, false));
        assert_eq!(dir_bytes(&root.join("missing"), &never, &never, &mut quiet), Some((0, true)));
        let missing = dir_totals(&root.join("missing"), &never, &never, &mut quiet, DIR_STACK_CAP).unwrap();
        assert_eq!((missing.bytes, missing.files, missing.folders, missing.partial), (0, 0, 0, true));
        // a wide folder stops enqueueing past the stack cap and is marked partial; a deep chain stays small
        let wide_files: Vec<(String, usize)> = ["d0", "d1", "d2", "d3", "d4"].into_iter().map(|d| (format!(r"{d}\a.bin"), 1)).collect();
        let wide = tree("wide", &wide_files);
        let capped = dir_totals(&wide, &never, &never, &mut quiet, 2).unwrap();
        assert!(capped.partial);
        assert_eq!(capped.folders, 6);
        assert!(capped.files < 5 && capped.files >= 1, "{capped:?}");
        let mut rel = String::new();
        for _ in 0..40 {
            rel.push_str(r"n\");
        }
        rel.push_str("f.bin");
        let deep = tree("deep", &[(rel, 9)]);
        let chain = dir_totals(&deep, &never, &never, &mut quiet, 2).unwrap();
        assert_eq!((chain.bytes, chain.files, chain.folders, chain.partial), (9, 1, 41, false));
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

        for r in [root, flat, big, wide, deep] {
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
    fn folder_size_text() {
        let kb = human(1024);
        UI.store(0, Ordering::Relaxed);
        assert_eq!(size_text(0, true, None, false), " (calculating…)");
        assert_eq!(size_text(0, true, Some((0, false, false)), false), " (calculating…)");
        assert_eq!(size_text(1024, true, Some((1024, false, false)), false), format!(" ({}…)", human(2048)));
        assert_eq!(size_text(0, true, Some((1024, true, false)), false), format!(" ({kb})"));
        assert_eq!(size_text(0, true, Some((1024, true, true)), false), format!(" ({kb}, partial)"));
        // something missed while collecting the selection isn't shown as a full total
        assert_eq!(size_text(0, true, Some((1024, true, false)), true), format!(" ({kb}, partial)"));
        assert_eq!(size_text(1024, false, None, false), format!(" ({kb})"));
        assert_eq!(size_text(1024, false, None, true), format!(" ({kb}, partial)"));
        assert_eq!(size_text(0, false, None, false), "");
        assert_eq!(size_text(0, false, None, true), " (size unknown)");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(size_text(0, true, Some((1024, true, false)), true), format!(" ({kb}, 일부만)"));
        assert_eq!(size_text(0, false, None, true), " (크기 모름)");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(size_text(0, false, None, true), " (サイズ不明)");
        UI.store(0, Ordering::Relaxed);
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
}
