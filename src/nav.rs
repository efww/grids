use crate::*;

// ---------- navigation ----------

pub(crate) unsafe fn navigate(i: usize, path: &str) -> bool {
    try_nav(i, path).is_ok()
}

pub(crate) unsafe fn try_nav(i: usize, path: &str) -> Result<()> {
    let eb = eb_of(i).ok_or_else(|| Error::from(E_FAIL))?;
    let mut pidl = std::ptr::null_mut();
    SHParseDisplayName(&HSTRING::from(path), None, &mut pidl, 0, None)?;
    let r = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE);
    CoTaskMemFree(Some(pidl as _));
    r
}

// The shell's answer for a path that isn't there; denied, offline or not ready are something else.
pub(crate) fn gone(code: HRESULT) -> bool {
    code == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) || code == HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)
}

// Nearest folder above `path` that still exists. Stops at anything but "not found" (denied, a file).
pub(crate) fn near_dir(path: &str, probe: impl Fn(&Path) -> std::io::Result<bool>) -> Option<String> {
    for p in Path::new(path).ancestors().skip(1) {
        match probe(p) {
            Ok(true) => return Some(p.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return None,
        }
    }
    None
}

#[derive(Clone, Copy)]
pub(crate) enum Nav {
    Pending,
    Complete,
    Failed,
}

// A pane's recovery (missing folder, parent being opened) through its navigation events. Returns the missing
// folder once the parent has opened; another navigation starting, or the parent failing, drops it quietly.
pub(crate) fn rec_step(rec: &mut Option<(String, String)>, ev: Nav, path: &str) -> Option<String> {
    let hit = rec.as_ref().is_some_and(|(_, t)| t.eq_ignore_ascii_case(path));
    match ev {
        Nav::Pending if !hit => *rec = None,
        Nav::Failed if hit => *rec = None,
        Nav::Complete => return rec.take().filter(|_| hit).map(|(m, _)| m),
        _ => {}
    }
    None
}

// Like Total Commander / XYplorer: a local folder that no longer exists opens its nearest existing parent,
// and the status line says so once that has opened (on_nav). Anything else fails as navigate() does.
// ponytail: local drives only, so the parent probe never blocks; shares would need a worker thread.
pub(crate) unsafe fn go_near(i: usize, path: &str) -> bool {
    let Err(e) = try_nav(i, path) else { return true };
    if !gone(e.code()) {
        return false;
    }
    // a confirmed missing folder leaves the recent list, and the pane's move away doesn't put it back (visit)
    app(|a| {
        a.s.recent.retain(|r| !r.eq_ignore_ascii_case(path));
        if a.s.paths[i].eq_ignore_ascii_case(path) {
            a.s.paths[i].clear();
        }
    });
    let local = path.get(1..3) == Some(":\\") && matches!(GetDriveTypeW(&HSTRING::from(&path[..3])), 2 | 3); // DRIVE_REMOVABLE, DRIVE_FIXED
    if !local {
        return false;
    }
    let Some(up) = near_dir(path, |p| std::fs::metadata(p).map(|m| m.is_dir())) else { return false };
    let Some(eb) = eb_of(i) else { return false };
    let mut pidl = std::ptr::null_mut();
    if SHParseDisplayName(&HSTRING::from(up.as_str()), None, &mut pidl, 0, None).is_err() {
        return false;
    }
    let target = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
    // set before browsing: a pane's first navigation completes inside BrowseToIDList
    let rec = |set: bool| {
        app(|a| {
            if let Some(p) = a.panes[i].as_mut() {
                if set {
                    p.recover = Some((path.to_string(), target.clone()));
                } else if p.recover.as_ref().is_some_and(|(_, t)| *t == target) {
                    p.recover = None;
                }
            }
        })
    };
    rec(true);
    let ok = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE).is_ok();
    if !ok {
        rec(false);
    }
    CoTaskMemFree(Some(pidl as _));
    ok
}

// "Not found: … · opened …" for 8 s; more recoveries while it shows merge into one count.
pub(crate) fn gone_note(a: &mut App, missing: &str, opened: &str) {
    let live = a.lost > 0 && a.note.as_ref().is_some_and(|(_, t)| Instant::now() < *t);
    a.lost = if live { a.lost + 1 } else { 1 };
    let text = match (KO.load(Ordering::Relaxed), a.lost) {
        (false, 1) => format!("Not found: {missing} · opened {opened}"),
        (true, 1) => format!("없는 폴더: {missing} · {opened} 엶"),
        (false, n) => format!("{n} saved folders were missing · opened the nearest parent folders"),
        (true, n) => format!("저장된 폴더 {n}개가 없어 가장 가까운 상위 폴더를 엶"),
    };
    a.note = Some((text, Instant::now() + Duration::from_secs(8)));
}

// Pending / Failed events of a pane with a recovery waiting; Complete goes through on_nav.
pub(crate) unsafe fn rec_event(i: usize, ev: Nav, pidl: *const ITEMIDLIST) {
    if app(|a| a.panes[i].as_ref().is_some_and(|p| p.recover.is_some())) {
        let path = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
        app(|a| a.panes[i].as_mut().map(|p| rec_step(&mut p.recover, ev, &path)));
    }
}

pub(crate) unsafe fn browse(i: usize, flags: u32) {
    if let Some(eb) = eb_of(i) {
        _ = eb.BrowseToIDList(std::ptr::null(), flags);
    }
}

pub(crate) unsafe fn browse_item(i: usize, si: &IShellItem) {
    let Some(eb) = eb_of(i) else { return };
    if let Ok(pidl) = SHGetIDListFromObject(si) {
        _ = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE);
        CoTaskMemFree(Some(pidl as _));
    }
}

pub(crate) unsafe fn focus_view(i: usize) {
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32);
        if let Ok(h) = sv.GetWindow() {
            _ = SetFocus(Some(h));
        }
    }
}

// Grids going to the back (another app, a popup, minimize): remember which of its controls had focus.
pub(crate) unsafe fn keep_focus(main: HWND) {
    let f = GetFocus();
    if IsChild(main, f).as_bool() {
        app(|a| a.focus_keep = f);
    }
}

// Coming back: focus where keep_focus left it if that is still the active pane's address / filter / list or the
// sidebar, else the active list. The edits keep their text, caret and selection (no EM_SETSEL).
pub(crate) unsafe fn restore_focus() {
    let (main, i, k, tree, th) = app(|a| (a.main, a.s.active, a.focus_keep, a.s.tree, a.tree_hwnd));
    let shown = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd)).is_some_and(|p| IsWindowVisible(p).as_bool());
    let within = |root: HWND| root == k || IsChild(root, k).as_bool();
    let ok = IsWindow(Some(k)).as_bool()
        && ((shown && (pane_ctl(i, ID_EDIT) == Some(k) || pane_ctl(i, ID_FILTER) == Some(k)))
            || view_of::<IShellView>(i).and_then(|sv| sv.GetWindow().ok()).is_some_and(within)
            || (tree && IsWindowVisible(th).as_bool() && within(th)));
    if ok {
        if pane_ctl(i, ID_EDIT) == Some(k) {
            _ = ShowWindow(k, SW_SHOWNA); // an idle address edit hides behind its crumbs; EN_SETFOCUS syncs it
        }
        _ = SetFocus(Some(k));
    } else {
        focus_view(i);
    }
    if GetFocus().is_invalid() {
        _ = SetFocus(Some(main)); // as before: keys reach Grids, not WM_SYSKEYDOWN
    }
}

pub(crate) unsafe fn focus_ctl(i: usize, id: i32) -> bool {
    if let Some(e) = pane_ctl(i, id) {
        _ = ShowWindow(e, SW_SHOWNA); // an idle address edit hides behind its crumbs
        _ = SetFocus(Some(e));
        SendMessageW(e, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
    true
}

// Address bar input as (a path, a program and its arguments); either may be missing. Unpaired quotes: nothing.
pub(crate) fn addr_parts(raw: &str) -> Option<(Option<&str>, Option<(&str, &str)>)> {
    let quotes = raw.matches('"').count();
    if quotes % 2 == 1 {
        return None;
    }
    let path = match quotes {
        0 => Some(raw),
        2 if raw.len() > 1 && raw.starts_with('"') && raw.ends_with('"') => Some(&raw[1..raw.len() - 1]),
        _ => None,
    };
    let cmd = if let Some(q) = raw.strip_prefix('"') {
        let (prog, rest) = q.split_once('"')?;
        (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| (prog, rest.trim()))
    } else {
        let (prog, rest) = raw.split_once(char::is_whitespace).unwrap_or((raw, ""));
        (!prog.contains(['\\', '/', ':', '"'])).then(|| (prog, rest.trim()))
    };
    Some((path, cmd.filter(|(prog, _)| !prog.is_empty())))
}

pub(crate) fn is_rel(p: &str) -> bool {
    !p.contains(':') && !p.starts_with(['\\', '/'])
}

pub(crate) unsafe fn addr_reset(i: usize, e: HWND) {
    let disp = app(|x| x.panes[i].as_ref().map(|p| p.disp.clone())).unwrap_or_default();
    _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
    focus_view(i);
    show_path_end(e, &disp);
}

// Enter in the address bar: a folder (relative to this pane's) is browsed to, a file opened, else a command
// (cmd, wt, powershell ...) runs in this pane's folder.
pub(crate) unsafe fn go_edit(i: usize) {
    let Some(e) = pane_ctl(i, ID_EDIT) else { return };
    let raw = text_of(e);
    let Some((path, cmd)) = addr_parts(raw.trim()) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    // none inside a zip or in a shell place; metadata (not GetFileAttributes) also takes a bare \\server\share
    let cur = app(|a| a.s.paths[i].clone());
    let cur = (is_fs(&cur) && std::fs::metadata(&cur).is_ok_and(|m| m.is_dir())).then_some(cur);
    let dir = cur.as_deref().map(HSTRING::from);
    let run = |mut sei: SHELLEXECUTEINFOW| {
        sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.lpDirectory = dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr()));
        sei.nShow = SW_SHOWNORMAL.0;
        ShellExecuteExW(&mut sei).is_ok()
    };
    // Some(true) = browsed, Some(false) = opened or ran
    let mut resolved = false;
    let done = 'go: {
        if let Some(p) = path {
            // std::path::absolute is GetFullPathNameW: folds `..` and `.`
            let full = cur.as_ref().filter(|_| is_rel(p)).and_then(|c| std::path::absolute(Path::new(c).join(p)).ok());
            let target = full.filter(|f| std::fs::metadata(f).is_ok()).map_or(p.to_string(), |f| f.to_string_lossy().into_owned());
            let (mut pidl, mut sfgao) = (std::ptr::null_mut(), 0u32);
            if SHParseDisplayName(&HSTRING::from(target), None, &mut pidl, SFGAO_FOLDER.0, Some(&mut sfgao)).is_ok() {
                let r = if sfgao & SFGAO_FOLDER.0 != 0 {
                    eb_of(i).is_some_and(|eb| eb.BrowseToIDList(pidl, SBSP_ABSOLUTE).is_ok()).then_some(true)
                } else {
                    run(SHELLEXECUTEINFOW { fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI, lpIDList: pidl as _, ..Default::default() }).then_some(false)
                };
                CoTaskMemFree(Some(pidl as _));
                resolved = true;
                break 'go r;
            }
        }
        let Some((prog, args)) = cmd else { break 'go None };
        // wt opens in its own default folder unless told; lpDirectory makes `.` this pane's
        let wt = prog.eq_ignore_ascii_case("wt") || prog.eq_ignore_ascii_case("wt.exe");
        let args = if wt && args.is_empty() && cur.is_some() { "-d ." } else { args };
        let (prog, args) = (HSTRING::from(prog), HSTRING::from(args));
        run(SHELLEXECUTEINFOW { fMask: SEE_MASK_FLAG_NO_UI, lpFile: PCWSTR(prog.as_ptr()), lpParameters: PCWSTR(args.as_ptr()), ..Default::default() }).then_some(false)
    };
    match done {
        Some(true) => focus_view(i),
        Some(false) => addr_reset(i, e),
        None if addr_jumps(done, resolved) => match app(|a| jump_pick(&raw, &a.s.recent, &a.s.favs, &a.s.paths[i]).map(str::to_string)) {
            // as the recent menu and favourites open a folder
            Some(p) => {
                if !go_near(i, &p) {
                    _ = MessageBeep(MB_ICONWARNING);
                }
                focus_view(i);
            }
            None => _ = MessageBeep(MB_ICONWARNING),
        },
        None => _ = MessageBeep(MB_ICONWARNING),
    }
}

// The selected folder (or, with none selected, the folder on display) of `src` opens in `dst`.
pub(crate) unsafe fn open_in(src: usize, dst: usize) {
    let Some(fv) = view_of::<IFolderView2>(src) else { return };
    let is_dir = |it: &IShellItem| it.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
    let sel = fv.GetSelection(false).and_then(|a| a.GetItemAt(0)).ok().filter(is_dir);
    if let Some(item) = sel.or_else(|| fv.GetFolder::<IShellItem>().ok()) {
        // the target takes this pane's columns too (order, widths, sort): set one up, copy it around
        save_view(src);
        app(|a| {
            let cols = a.s.cols[src].clone();
            if let Some(p) = a.panes[dst].as_mut() {
                p.pending = Some(cols);
            }
        });
        browse_item(dst, &item);
        set_active(dst);
        focus_view(dst);
    }
}

pub(crate) unsafe fn open_in_menu(h: HWND, src: usize) {
    // (pane, shown name, folder it held when the menu opened, colour label)
    let items = app(|a| {
        let live = a.panes.iter().enumerate().take(a.s.panes());
        let live = live.filter(|(k, _)| *k != src);
        live.filter_map(|(k, p)| Some((k, p.as_ref()?.disp.clone(), a.s.paths[k].clone(), a.s.tints[k]))).collect::<Vec<_>>()
    });
    // "Pane 3 (Sage)    C:\work"
    let label = |k: usize, t: u8, disp: &str| {
        let tag = if t == 0 { String::new() } else { format!(" ({})", tint_name(t)) };
        HSTRING::from(format!("{} {}{tag}    {disp}", tr("Pane", "패널"), k + 1))
    };
    let arr = view_of::<IFolderView2>(src).and_then(|fv| fv.GetSelection(false).ok());
    let count = arr.as_ref().and_then(|a| a.GetCount().ok()).unwrap_or(0);
    let picked = collect_all(count, |k| unsafe { arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)) });
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Open the selected folder (or this one) in:", "선택한 폴더(없으면 현재 폴더)를 열 패널:")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for (k, disp, _, t) in &items {
        _ = AppendMenuW(menu, MF_STRING, k + 1, &label(*k, *t, disp));
    }
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    if let Ok(sub) = CreatePopupMenu() {
        for (k, disp, _, t) in &items {
            _ = AppendMenuW(sub, MF_STRING, 301 + k, &label(*k, *t, disp));
        }
        let flags = if items.is_empty() { MF_POPUP | MF_GRAYED } else { MF_POPUP };
        _ = AppendMenuW(menu, flags, sub.0 as usize, &HSTRING::from(tr("Swap places with  (Ctrl+Alt+Shift+arrows)", "자리 바꾸기  (Ctrl+Alt+Shift+방향키)")));
    }
    match &picked {
        Ok(srcs) => {
            let n = srcs.len();
            let titles = [
                (false, 101, tr("Copy {n} selected items to", "선택한 {n}개 복사").replace("{n}", &n.to_string())),
                (true, 201, tr("Move {n} selected items to", "선택한 {n}개 이동").replace("{n}", &n.to_string())),
            ];
            for (mv, base, title) in titles {
                let Ok(sub) = CreatePopupMenu() else { continue };
                for (k, disp, path, t) in &items {
                    let ok = is_fs(path) && send_mode(srcs, path, mv).is_some();
                    let flags = if ok { MF_STRING } else { MF_STRING | MF_GRAYED };
                    _ = AppendMenuW(sub, flags, base + k, &label(*k, *t, disp));
                }
                _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, &HSTRING::from(title));
            }
        }
        Err(why) => {
            let text = match why {
                Pick::TooMany => tr("Copy / move: more than 10,000 selected items — use Ctrl+C / Ctrl+V", "복사·이동: 선택이 10,000개를 넘습니다 — Ctrl+C / Ctrl+V를 쓰세요"),
                Pick::Unreadable => tr("Copy / move: some selected items aren't regular files", "복사·이동: 일반 파일이 아닌 항목이 섞여 있습니다"),
                Pick::Empty => tr("Copy / move: select files first", "복사·이동: 먼저 파일을 고르세요"),
            };
            _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(text));
        }
    }
    let others: Vec<(usize, HSTRING, String)> = items.iter().map(|(k, disp, path, t)| (*k, label(*k, *t, disp), path.clone())).collect();
    let wm = shell_menu(menu, src, &picked, &others);
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(src, ID_OPEN_IN) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    _ = DestroyMenu(menu);
    match (pick as usize, picked) {
        (0, _) => {}
        (p @ 1..=100, _) => open_in(src, p - 1),
        (p @ 301..=316, _) => swap_panes(src, p - 301),
        (p, picked) if is_shell_pick(p) => shell_pick(h, src, p, picked, wm, &others),
        (p, Ok(srcs)) => {
            // only to the folder the menu showed, and only the whole selection it counted
            let (mv, k) = (p > 200, (p - 1) % 100);
            let shown = items.iter().find(|(i, ..)| *i == k).map(|(_, _, path, _)| path.clone());
            let sent = shown.filter(|s| *s == app(|a| a.s.paths[k].clone())).and_then(|dst| {
                let rename = send_mode(&srcs, &dst, mv)?;
                Some(start_copy(srcs, dst, rename, mv))
            });
            if sent != Some(true) {
                _ = MessageBeep(MB_ICONWARNING);
            }
        }
        _ => {}
    }
}

// Right-click on a pane header: colour label from the selection colour presets, saved with the session.
pub(crate) unsafe fn tint_menu(h: HWND, i: usize, at: POINT) {
    let cur = app(|a| a.s.tints[i]);
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Pane colour", "패널 색")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for k in 0..=6u8 {
        let flags = if k == cur { MF_STRING | MF_CHECKED } else { MF_STRING };
        let name = if k == 0 { tr("None", "없음") } else { tint_name(k) };
        _ = AppendMenuW(menu, flags, k as usize + 1, &HSTRING::from(name));
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, at.x, at.y, None, h, None).0;
    _ = DestroyMenu(menu);
    if pick > 0 {
        app(|a| a.s.tints[i] = pick as u8 - 1);
        repaint_header(h);
        check_dirty();
    }
}

// Alt+↓ or right-click on ← / →: folders recently visited in any pane, opened in this one.
pub(crate) unsafe fn recent_menu(i: usize) {
    let Some(h) = app(|a| a.panes[i].as_ref().map(|p| p.hwnd)) else { return };
    let items = app(|a| recent_items(&a.s.recent, &a.s.paths[i]));
    let Ok(menu) = CreatePopupMenu() else { return };
    if items.is_empty() {
        _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("No recent folders yet", "최근 폴더가 아직 없습니다")));
    }
    for (k, p) in items.iter().enumerate() {
        // "name<tab>parent"; drive roots and share roots show whole
        let path = std::path::Path::new(p);
        let (leaf, parent) = match (path.file_name(), path.parent()) {
            (Some(n), Some(d)) => (n.to_string_lossy().into_owned(), d.display().to_string()),
            _ => (p.clone(), String::new()),
        };
        _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(format!("{leaf}\t{parent}").replace('&', "&&")));
    }
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(i, ID_BACK) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    _ = DestroyMenu(menu);
    if pick > 0 && !go_near(i, &items[pick as usize - 1]) {
        _ = MessageBeep(MB_ICONWARNING); // gone or unreachable
    }
    focus_view(i);
}

pub(crate) unsafe fn apply_filter(i: usize) {
    let Some(f) = pane_ctl(i, ID_FILTER) else { return };
    let spec = filter_spec(&text_of(f));
    let changed = FILTERS.lock().map(|mut fl| std::mem::replace(&mut fl[i], spec.clone()) != spec).unwrap_or(false);
    if let (true, Some(sv)) = (changed, view_of::<IShellView>(i)) {
        _ = sv.Refresh(); // re-enumerates, asking IncludeObject again
    }
    if let (true, Some(pane)) = (changed, app(|a| a.panes[i].as_ref().map(|p| p.hwnd))) {
        _ = InvalidateRect(Some(pane), None, false); // filter badge on/off
    }
}

// Filter icon or Alt+↓ in the filter: show only one kind of file (folders stay), like Explorer's search "Kind".
pub(crate) unsafe fn kind_menu(i: usize) {
    let (Some(h), Some(f)) = (app(|a| a.panes[i].as_ref().map(|p| p.hwnd)), pane_ctl(i, ID_FILTER)) else { return };
    let cur = filter_of(i);
    // only an actual single selection: the focused item alone doesn't count
    let arr = view_of::<IFolderView2>(i).and_then(|fv| fv.GetSelection(false).ok()).filter(|a| a.GetCount().is_ok_and(|n| n == 1));
    let same = arr.and_then(|a| {
        let it = a.GetItemAt(0).ok()?;
        let dir = it.GetAttributes(SFGAO_FOLDER).ok()?.0 & SFGAO_FOLDER.0 != 0;
        same_type(&pwstr(it.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).ok()?), dir)
    });
    let Ok(menu) = CreatePopupMenu() else { return };
    for (k, (en, ko, spec)) in FILE_KINDS.iter().enumerate() {
        let flags = if cur == *spec { MF_STRING | MF_CHECKED } else { MF_STRING };
        let shown = spec.split(';').take(2).collect::<Vec<_>>().join(", ");
        _ = AppendMenuW(menu, flags, k + 1, &HSTRING::from(format!("{}\t{shown}, …", tr(en, ko))));
    }
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let same_label = tr("Same type as the selected file", "선택한 파일과 같은 형식");
    match &same {
        Some(s) => _ = AppendMenuW(menu, MF_STRING, 100, &HSTRING::from(format!("{same_label} ({})", s.replace('&', "&&")))),
        None => _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 100, &HSTRING::from(same_label)),
    }
    let clear = if cur.is_empty() && text_of(f).is_empty() { MF_STRING | MF_GRAYED } else { MF_STRING };
    _ = AppendMenuW(menu, clear, 101, &HSTRING::from(tr("Clear filter", "필터 지우기")));
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let pg = pane_geo(rc.right, rc.bottom, app(|a| a.dpi));
    let mut at = POINT { x: pg.filt.left, y: pg.filt.bottom };
    _ = ClientToScreen(h, &mut at);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, at.x, at.y, None, h, None).0 as usize;
    _ = DestroyMenu(menu);
    let spec = match pick {
        1..=6 => Some(FILE_KINDS[pick - 1].2.to_string()),
        100 => same,
        101 => Some(String::new()),
        _ => None,
    };
    if let Some(s) = spec {
        _ = SetWindowTextW(f, &HSTRING::from(s));
        apply_filter(i);
    }
    focus_view(i);
}

// Pane-sticky view mode and columns (order, widths, sort): remember them before leaving a folder,
// re-apply after arriving. Columns seen in shell places (This PC ...) are theirs, not the pane's.
pub(crate) unsafe fn save_view(i: usize) {
    if app(|a| a.panes[i].as_ref().is_none_or(|p| p.pending.is_some() || p.settle.is_some())) {
        return; // columns are about to be put on, or still settling: don't overwrite them
    }
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    free_min_widths(view_list(&fv)); // a list made after the folder change gets it here
    let (mut m, mut z) = (FOLDERVIEWMODE(0), 0);
    if fv.GetViewModeAndIconSize(&mut m, &mut z).is_ok() {
        app(|a| a.s.views[i] = (m.0, z));
    }
    if let Some(c) = read_cols(&fv, view_list(&fv)).filter(|_| app(|a| is_fs(&a.s.paths[i]))) {
        app(|a| a.s.cols[i] = c);
    }
}

// Column names in on-screen order, from the pane's list header: dragging a column to a new place
// changes only the header's order, which the view's own column list doesn't follow.
pub(crate) unsafe fn header_names(lv: HWND) -> Vec<String> {
    let n = list_titles(lv).len();
    let mut order = vec![0i32; n];
    SendMessageW(lv, LVM_GETCOLUMNORDERARRAY, Some(WPARAM(n)), Some(LPARAM(order.as_mut_ptr() as isize)));
    let titles = list_titles(lv);
    order.iter().filter_map(|&k| titles.get(k as usize).cloned()).collect()
}

// Column titles of a list, by column index.
pub(crate) unsafe fn list_titles(lv: HWND) -> Vec<String> {
    let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
    let n = SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize;
    (0..n)
        .map(|k| {
            let mut text = [0u16; 128];
            let mut item = HDITEMW { mask: HDI_TEXT, pszText: PWSTR(text.as_mut_ptr()), cchTextMax: text.len() as i32, ..Default::default() };
            SendMessageW(header, HDM_GETITEMW, Some(WPARAM(k)), Some(LPARAM(&mut item as *mut _ as isize)));
            String::from_utf16_lossy(&text[..text.iter().position(|&c| c == 0).unwrap_or(0)])
        })
        .collect()
}

// The list inside a view. During a folder change the old view's list is still around, so it is
// found from the view itself, not by looking through the pane.
pub(crate) unsafe fn view_list(fv: &IFolderView2) -> HWND {
    let view = fv.cast::<IShellView>().and_then(|v| v.GetWindow()).unwrap_or_default();
    FindWindowExW(Some(view), None, w!("SysListView32"), PCWSTR::null()).unwrap_or_default()
}

// Whether pane i's list is a DirectUIHWND: no list message reaches it, so it can't be recoloured.
pub(crate) unsafe fn dui_list(i: usize) -> bool {
    let view = view_of::<IShellView>(i).and_then(|v| v.GetWindow().ok());
    view.is_some_and(|v| FindWindowExW(Some(v), None, w!("DirectUIHWND"), PCWSTR::null()).is_ok())
}

// Whether h is the file list of the view window. The list is a SysListView32 on some systems and a
// DirectUIHWND on others, where view_list finds nothing; both are the list when they sit inside the view.
pub(crate) unsafe fn in_view_list(view: HWND, h: HWND) -> bool {
    let mut c = [0u16; 32];
    let n = GetClassNameW(h, &mut c) as usize;
    !view.0.is_null()
        && matches!(String::from_utf16_lossy(&c[..n]).as_str(), "DirectUIHWND" | "SysListView32")
        && (view == h || IsChild(view, h).as_bool())
}

// Explorer puts a floor under some columns (Size stops at 90 px) so numbers never get cut, and puts it
// back whenever a column changes (through an interface, not messages we could catch). In a pane the
// user decides: the floor is lowered to a sliver right before each resize, a drag on the header or a
// width set on the list.
pub(crate) unsafe fn free_min_widths(lv: HWND) {
    if lv.0.is_null() {
        return;
    }
    let mut was = 0usize;
    let first = !GetWindowSubclass(lv, Some(list_floor_proc), 3, Some(&mut was as *mut _)).as_bool();
    _ = SetWindowSubclass(lv, Some(list_floor_proc), 3, 0);
    if first {
        // the view puts its floor back on the first width change a list sees, and only then: spend that
        // change here (a pixel wider and back) so the user's first drag already goes below it
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        for c in 0..SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize {
            let w = SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0;
            SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w + 1)));
            SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w)));
        }
    }
    // and no snapping to the floor or ideal width while dragging, no automatic sizing: the user's width
    let snap = LVS_EX_COLUMNSNAPPOINTS | LVS_EX_AUTOSIZECOLUMNS;
    if SendMessageW(lv, LVM_GETEXTENDEDLISTVIEWSTYLE, None, None).0 as usize & snap != 0 {
        SendMessageW(lv, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(snap)), Some(LPARAM(0)));
    }
}

pub(crate) unsafe fn lower_floors(lv: HWND) {
    let floor = 20 * GetDpiForWindow(lv) as i32 / 96;
    let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
    for c in 0..SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize {
        let mut col = LVCOLUMNW { mask: LVCF_MINWIDTH, ..Default::default() };
        if SendMessageW(lv, LVM_GETCOLUMNW, Some(WPARAM(c)), Some(LPARAM(&mut col as *mut _ as isize))).0 != 0 && col.cxMin > floor {
            let lower = LVCOLUMNW { mask: LVCF_MINWIDTH, cxMin: floor, ..Default::default() };
            SendMessageW(lv, LVM_SETCOLUMNW, Some(WPARAM(c)), Some(LPARAM(&lower as *const _ as isize)));
        }
    }
}

pub(crate) unsafe extern "system" fn list_floor_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let resize = match msg {
        LVM_SETCOLUMNWIDTH => true,
        WM_NOTIFY => matches!((*(lp.0 as *const NMHDR)).code, HDN_BEGINTRACKW | HDN_TRACKW | HDN_ITEMCHANGINGW),
        _ => false,
    };
    if resize {
        lower_floors(h);
    }
    // the view stops a column coming down from above its floor at the floor (through its own channel,
    // not a message): the narrowing asked for goes on again right after, from the floor, where it isn't stopped
    if msg == WM_NOTIFY && (*(lp.0 as *const NMHDR)).code == HDN_ITEMCHANGINGW {
        let nm = &*(lp.0 as *const NMHEADERW);
        if !nm.pitem.is_null() && (*nm.pitem).mask.0 & HDI_WIDTH.0 != 0 {
            let floor = 20 * GetDpiForWindow(h) as i32 / 96; // never all the way to nothing
            let (col, want) = (nm.iItem, (*nm.pitem).cxy.max(floor));
            let now = SendMessageW(h, LVM_GETCOLUMNWIDTH, Some(WPARAM(col as usize)), None).0 as i32;
            let ask = if want < now { (col as i64) << 32 | want as i64 } else { -1 };
            NARROWING.store(ask, Ordering::Relaxed); // a newer change (or a widening) cancels an older one
            if ask >= 0 {
                _ = PostMessageW(Some(h), WM_NARROW_AGAIN, WPARAM(col as usize), LPARAM(want as isize));
            }
        }
    }
    if msg == WM_NARROW_AGAIN {
        let (col, want) = (wp.0, lp.0 as i32);
        if NARROWING.load(Ordering::Relaxed) == ((col as i64) << 32 | want as i64)
            && SendMessageW(h, LVM_GETCOLUMNWIDTH, Some(WPARAM(col)), None).0 as i32 != want // stopped at the floor, or let through to nothing
        {
            NARROWING.store(-1, Ordering::Relaxed);
            SendMessageW(h, LVM_SETCOLUMNWIDTH, Some(WPARAM(col)), Some(LPARAM(want as isize)));
        }
        return LRESULT(0);
    }
    DefSubclassProc(h, msg, wp, lp)
}

pub(crate) const WM_NARROW_AGAIN: u32 = WM_APP + 41;
pub(crate) const WM_DROP_OPEN: u32 = WM_APP + 42;
pub(crate) const WM_PEEK_DONE: u32 = WM_APP + 43; // wParam = gen; the picture itself stays in PEEK
pub(crate) const WM_PANE_ICON: u32 = WM_APP + 44; // wParam = ticket, lParam = system image list index (-1: none)
pub(crate) const WM_DRIVES: u32 = WM_APP + 45; // drives may have come or gone: refresh_drives
pub(crate) const WM_KIDS: u32 = WM_APP + 46; // wParam = request id; the list itself stays in KIDS
pub(crate) const WM_THEME_SYNC: u32 = WM_APP + 47; // Windows light/dark may have changed: theme_sync
pub(crate) const WM_PEEK_HOSTED: u32 = WM_APP + 49; // wParam = preview generation, lParam = 1 shown / 0 failed
pub(crate) const WM_PEEK_LOST: u32 = WM_APP + 48; // the preview went inactive: close it unless it is active again
pub(crate) static NARROWING: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1); // column << 32 | width


// The view's visible columns with widths, put in on-screen order by name, then its sort columns.
// Widths come from the list: one narrowed below the view's floor is only there (the view still says 90).
pub(crate) unsafe fn read_cols(fv: &IFolderView2, lv: HWND) -> Option<String> {
    let cm: IColumnManager = fv.cast().ok()?;
    let (titles, shown) = (list_titles(lv), header_names(lv));
    let mut keys = vec![PROPERTYKEY::default(); cm.GetColumnCount(CM_ENUM_VISIBLE).ok()? as usize];
    cm.GetColumns(CM_ENUM_VISIBLE, &mut keys).ok()?;
    let info = |k: &PROPERTYKEY| {
        let mask = (CM_MASK_WIDTH.0 | CM_MASK_NAME.0) as u32;
        let mut ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: mask, ..Default::default() };
        _ = cm.GetColumnInfo(k, &mut ci);
        let name = String::from_utf16_lossy(&ci.wszName[..ci.wszName.iter().position(|&c| c == 0).unwrap_or(0)]);
        let on_list = titles.iter().position(|t| *t == name).map(|c| SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0 as u32);
        (*k, on_list.filter(|&w| w > 0).unwrap_or(ci.uWidth), name)
    };
    let mut left: Vec<_> = keys.iter().map(info).collect();
    let mut cols = Vec::new();
    for name in &shown {
        if let Some(at) = left.iter().position(|c| c.2 == *name) {
            cols.push(left.remove(at));
        }
    }
    cols.extend(left); // any the header didn't name keep the view's order
    let cols: Vec<_> = cols.into_iter().map(|(k, w, _)| (k, w)).collect();
    let mut sort = vec![SORTCOLUMN::default(); fv.GetSortColumnCount().ok()?.max(0) as usize];
    fv.GetSortColumns(&mut sort).ok()?;
    Some(cols_text(&cols, &sort.iter().map(|c| (c.propkey, c.direction.0)).collect::<Vec<_>>()))
}

// The view keeps its own floor (Size: 90 px) under widths set through it, also when it builds a new
// folder's columns: narrower ones go straight onto the list, which lets them through (list_floor_proc).
pub(crate) unsafe fn put_widths(fv: &IFolderView2, text: &str, lv: HWND) {
    let Ok(cm) = fv.cast::<IColumnManager>() else { return };
    let titles = list_titles(lv);
    for (k, w) in parse_cols(text).0 {
        let mut ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: CM_MASK_NAME.0 as u32, ..Default::default() };
        _ = cm.GetColumnInfo(&k, &mut ci);
        let name = String::from_utf16_lossy(&ci.wszName[..ci.wszName.iter().position(|&c| c == 0).unwrap_or(0)]);
        if let Some(c) = titles.iter().position(|t| *t == name) {
            if SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0 != w as isize {
                SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w as isize)));
            }
        }
    }
}

pub(crate) unsafe fn apply_cols(fv: &IFolderView2, text: &str, lv: HWND) {
    let (cols, sort) = parse_cols(text);
    if let (false, Ok(cm)) = (cols.is_empty(), fv.cast::<IColumnManager>()) {
        if cm.SetColumns(&cols.iter().map(|c| c.0).collect::<Vec<_>>()).is_ok() {
            for (k, w) in &cols {
                let ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: CM_MASK_WIDTH.0 as u32, uWidth: *w, ..Default::default() };
                _ = cm.SetColumnInfo(k, &ci);
            }
            put_widths(fv, text, lv);
        }
    }
    if !sort.is_empty() {
        _ = fv.SetSortColumns(&sort.iter().map(|&(propkey, d)| SORTCOLUMN { propkey, direction: SORTDIRECTION(d) }).collect::<Vec<_>>());
    }
}

// A few passes after a folder change (50, 150, 400, 1000 ms) put narrow widths back as the view builds
// its columns at its own floor; then the pane's columns are read from the view again.
pub(crate) unsafe fn settle_cols(h: HWND) {
    _ = KillTimer(Some(h), TIMER_SETTLE);
    let Some(i) = pane_of(h) else { return };
    let Some((cols, left)) = app(|a| a.panes[i].as_ref().and_then(|p| p.settle.clone())) else { return };
    if let Some(fv) = view_of::<IFolderView2>(i) {
        let lv = view_list(&fv); // the new folder's list exists by now
        free_min_widths(lv);
        put_widths(&fv, &cols, lv);
    }
    let left = left - 1;
    app(|a| {
        if let Some(p) = a.panes[i].as_mut() {
            p.settle = (left > 0).then(|| (cols, left));
        }
    });
    if left > 0 {
        SetTimer(Some(h), TIMER_SETTLE, [1000, 400, 150][left as usize - 1], None);
    }
}

pub(crate) fn tabs_on() -> bool {
    TABS_ON.load(Ordering::Relaxed)
}

pub(crate) unsafe fn on_nav(i: usize, pidl: *const ITEMIDLIST) {
    let parse = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
    let disp = name(pidl, SIGDN_DESKTOPABSOLUTEEDITING);
    let Some((h, eb, view, cols, ticket)) = app(|a| {
        let fs = is_fs(&parse);
        let old = std::mem::replace(&mut a.s.paths[i], parse.clone());
        visit(&mut a.s.recent, &old, &a.s.paths[i]);
        if let Some(m) = a.panes[i].as_mut().and_then(|p| rec_step(&mut p.recover, Nav::Complete, &parse)) {
            gone_note(a, &m, &disp); // update_status shows it
        }
        let (v, own) = (a.s.views[i], a.s.cols[i].clone());
        a.panes[i].as_mut().map(|p| {
            p.disp = disp.clone();
            // the old folder's icon goes with its address (sync_addr below repaints the pill); a new lookup starts
            (p.icon, p.ticket) = (-1, next_ticket(&ICON_TICKET));
            // waiting columns (session, open in) first, else the pane's own; none in shell places
            let cols = p.pending.take().unwrap_or(own);
            (p.hwnd, p.eb.clone(), v, Some(cols).filter(|c| fs && !c.is_empty()), p.ticket)
        })
    }) else {
        return;
    };
    if let Ok(e) = GetDlgItem(Some(h), ID_EDIT) {
        _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
        sync_addr(h);
        show_path_end(e, &disp);
    }
    request_icon(h, ticket, pidl);
    if tabs_on() {
        repaint_tabs(i); // the current tab's name is this folder's
    }
    if let Ok(b) = GetDlgItem(Some(h), ID_FAV) {
        _ = InvalidateRect(Some(b), None, false);
    }
    if let Ok(fv) = eb.GetCurrentView::<IFolderView2>() {
        if view.0 != 0 {
            _ = fv.SetViewModeAndIconSize(FOLDERVIEWMODE(view.0), view.1);
        }
        let lv = view_list(&fv);
        free_min_widths(lv);
        if let Some(cols) = cols {
            apply_cols(&fv, &cols, lv);
            app(|a| {
                if let Some(p) = a.panes[i].as_mut() {
                    p.settle = Some((cols, 4));
                }
            });
            SetTimer(Some(h), TIMER_SETTLE, 50, None);
        }
        // No full-row select: a drag that starts in the size/date cells draws a selection box
        // (and selects the names it covers) instead of grabbing that row's file.
        _ = fv.SetCurrentFolderFlags(FWF_FULLROWSELECT.0 as u32, 0);
    }
    if DARK_ON.load(Ordering::Relaxed) {
        dark_list(h);
    }
    if app(|a| a.s.active == i) {
        if let Ok(item) = SHCreateItemFromIDList::<IShellItem>(pidl) {
            sync_tree(&item);
        }
    }
    // set_layout's focus for a pane made in place of the active one: once, and only if focus went nowhere else since
    if app(|a| a.focus_req == h && a.s.active == i && std::mem::take(&mut a.focus_req) == h) {
        let f = GetFocus();
        if f.0.is_null() || f == app(|a| a.main) {
            focus_view(i);
        }
    }
}

// ---------- address bar folder icon ----------
// Looked up off the UI thread (a slow share can take seconds). Tickets never repeat (0 = none asked), so a late
// result can't land on a newer folder, nor on a new pane that got the same HWND value.
pub(crate) static ICON_TICKET: AtomicU32 = AtomicU32::new(1);

pub(crate) fn next_ticket(t: &AtomicU32) -> u32 {
    loop {
        let n = t.fetch_add(1, Ordering::Relaxed);
        if n != 0 {
            return n;
        }
    }
}

// A result is taken only for the pane's current lookup.
pub(crate) fn take_icon(cur: u32, msg: u32, idx: i32) -> Option<i32> {
    (cur != 0 && cur == msg).then_some(idx)
}

// Lookups not started yet: (pane HWND, ticket, PIDL copy), at most one per pane, so at most MAXP.
// The PIDL belongs to the queue from push until the worker takes it; one replaced or removed comes back to be freed.
pub(crate) type IconReq = (isize, u32, usize);
pub(crate) static ICON_Q: Mutex<Vec<IconReq>> = Mutex::new(Vec::new());
pub(crate) static ICON_WORKER: OnceLock<Option<isize>> = OnceLock::new(); // its wake event; None = couldn't start

pub(crate) fn icon_q() -> std::sync::MutexGuard<'static, Vec<IconReq>> {
    ICON_Q.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn enqueue(q: &mut Vec<IconReq>, req: IconReq) -> Option<usize> {
    let old = unqueue(q, req.0);
    q.push(req);
    old
}

pub(crate) fn unqueue(q: &mut Vec<IconReq>, h: isize) -> Option<usize> {
    let k = q.iter().position(|r| r.0 == h)?;
    Some(q.remove(k).2)
}

pub(crate) unsafe fn request_icon(h: HWND, ticket: u32, pidl: *const ITEMIDLIST) {
    let c = ILClone(pidl);
    if c.is_null() {
        return;
    }
    let ev = *ICON_WORKER.get_or_init(|| {
        let ev = CreateEventW(None, false, false, PCWSTR::null()).ok()?;
        let raw = ev.0 as isize;
        match std::thread::Builder::new().spawn(move || icon_worker(raw)) {
            Ok(_) => Some(raw),
            Err(_) => {
                _ = CloseHandle(ev);
                None
            }
        }
    });
    let Some(ev) = ev else {
        ILFree(Some(c));
        return;
    };
    let old = enqueue(&mut icon_q(), (h.0 as isize, ticket, c as usize));
    if let Some(old) = old {
        ILFree(Some(old as *const ITEMIDLIST));
    }
    _ = SetEvent(HANDLE(ev as _));
}

// Lives until the process ends (never joined: a dead share mustn't hold up closing). One lookup at a time, oldest
// first; it sleeps only on an empty queue, and pumps messages so COM calls into this apartment keep moving.
pub(crate) fn icon_worker(ev: isize) {
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let mut m = MSG::default();
        loop {
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            let next = {
                let mut q = icon_q();
                (!q.is_empty()).then(|| q.remove(0))
            };
            let Some((h, ticket, pidl)) = next else {
                MsgWaitForMultipleObjectsEx(Some(&[HANDLE(ev as _)]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                continue;
            };
            let pidl = pidl as *const ITEMIDLIST;
            let mut sfi = SHFILEINFOW::default();
            let flags = SHGFI_PIDL | SHGFI_SYSICONINDEX | SHGFI_SMALLICON; // an index only: no HICON to free
            let found = com && SHGetFileInfoW(PCWSTR(pidl as _), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut sfi), size_of::<SHFILEINFOW>() as u32, flags) != 0;
            ILFree(Some(pidl));
            let idx = if found { sfi.iIcon.max(-1) } else { -1 };
            _ = PostMessageW(Some(HWND(h as _)), WM_PANE_ICON, WPARAM(ticket as usize), LPARAM(idx as isize)); // fails if the pane is gone
        }
    }
}

#[implement(IExplorerBrowserEvents)]
pub(crate) struct Events(pub(crate) Arc<AtomicUsize>); // the pane's slot, read at each event (panes can swap places)

impl IExplorerBrowserEvents_Impl for Events_Impl {
    fn OnNavigationPending(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        let i = self.0.load(Ordering::SeqCst);
        unsafe {
            rec_event(i, Nav::Pending, pidl);
            save_view(i)
        };
        Ok(())
    }
    fn OnViewCreated(&self, _: Ref<IShellView>) -> Result<()> {
        Ok(())
    }
    fn OnNavigationComplete(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        unsafe { on_nav(self.0.load(Ordering::SeqCst), pidl) };
        Ok(())
    }
    fn OnNavigationFailed(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        unsafe { rec_event(self.0.load(Ordering::SeqCst), Nav::Failed, pidl) };
        Ok(())
    }
}

// Site of each Explorer control. It asks the site for ICommDlgBrowser, whose IncludeObject is the name filter.
#[implement(IServiceProvider, ICommDlgBrowser)]
pub(crate) struct Host(pub(crate) Arc<AtomicUsize>); // the pane's slot, as in Events

impl IServiceProvider_Impl for Host_Impl {
    fn QueryService(&self, service: *const GUID, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        unsafe {
            *ppv = std::ptr::null_mut();
            // SID_SExplorerBrowserFrame is defined as IID_ICommDlgBrowser.
            if *service == ICommDlgBrowser::IID {
                return self.to_interface::<ICommDlgBrowser>().query(riid, ppv).ok();
            }
        }
        Err(E_NOINTERFACE.into())
    }
}

impl ICommDlgBrowser_Impl for Host_Impl {
    fn OnDefaultCommand(&self, _: Ref<IShellView>) -> Result<()> {
        Err(S_FALSE.into()) // let the view do its default action (open)
    }
    fn OnStateChange(&self, _: Ref<IShellView>, change: u32) -> Result<()> {
        if change == CDBOSC_SELCHANGE {
            SEL_REV[self.0.load(Ordering::SeqCst)].fetch_add(1, Ordering::Relaxed); // same count, other items: the status line still updates
        }
        Ok(())
    }
    fn IncludeObject(&self, sv: Ref<IShellView>, pidl: *const ITEMIDLIST) -> Result<()> {
        // may run on the view's own thread: the slot is read under the lock swap_panes moves it under
        let spec = FILTERS.lock().map(|f| f[self.0.load(Ordering::SeqCst)].clone()).unwrap_or_default();
        if spec.is_empty() {
            return Ok(());
        }
        unsafe {
            let sf: IShellFolder = sv.ok()?.cast::<IFolderView>()?.GetFolder()?;
            let mut attr = SFGAO_FOLDER.0 | SFGAO_STREAM.0;
            if filter_keeps_unnamed(sf.GetAttributesOf(&[pidl], &mut attr).ok().map(|_| attr)) {
                return Ok(()); // folders stay, so a filtered pane can still be navigated
            }
            let mut sr = STRRET::default();
            let mut name = PWSTR::null();
            sf.GetDisplayNameOf(pidl, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0), &mut sr)?;
            StrRetToStrW(&mut sr, Some(pidl), &mut name)?;
            let hit = PathMatchSpecW(PCWSTR(name.0), &HSTRING::from(spec)).as_bool();
            CoTaskMemFree(Some(name.0 as _));
            if hit {
                Ok(())
            } else {
                Err(S_FALSE.into())
            }
        }
    }
}

// A folder dropped on a pane's top bar (its fields and buttons too: OLE walks up to the pane) opens there.
// Never MOVE: a shell source deletes what it moved. LINK or COPY leaves the source alone.
pub(crate) fn drop_pick(allowed: DROPEFFECT) -> DROPEFFECT {
    [DROPEFFECT_LINK, DROPEFFECT_COPY].into_iter().find(|e| allowed.0 & e.0 != 0).unwrap_or(DROPEFFECT_NONE)
}

pub(crate) unsafe fn drop_items(d: &Ref<IDataObject>) -> Option<IShellItemArray> {
    let arr: IShellItemArray = SHCreateShellItemArrayFromDataObject(d.as_ref()?).ok()?;
    (arr.GetCount().unwrap_or(0) > 0).then_some(arr)
}

#[implement(IDropTarget)]
pub(crate) struct PaneDrop {
    pub(crate) slot: Arc<AtomicUsize>,
    pub(crate) hwnd: HWND,
    pub(crate) ok: Cell<bool>, // the data being dragged holds shell items
    pub(crate) helper: Option<IDropTargetHelper>,
}

impl PaneDrop {
    // sets the effect this drop would get, and tints the header only while it isn't NONE
    pub(crate) unsafe fn pick(&self, effect: *mut DROPEFFECT) {
        *effect = if self.ok.get() { drop_pick(*effect) } else { DROPEFFECT_NONE };
        self.hover(*effect != DROPEFFECT_NONE);
    }
    pub(crate) unsafe fn hover(&self, on: bool) {
        let h = self.hwnd;
        let changed = app(|a| {
            let was = a.drop_hover == h;
            if on {
                a.drop_hover = h;
            } else if was {
                a.drop_hover = HWND::default();
            }
            was != on
        });
        if changed {
            repaint_header(h);
        }
    }
    // the folder (or a file's folder) is queued, not opened: a drag from this pane's own list is still running
    pub(crate) unsafe fn queue(&self, d: &Ref<IDataObject>) -> bool {
        let Some(item) = drop_items(d).and_then(|a| a.GetItemAt(0).ok()) else { return false };
        let is_dir = item.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
        let Some(folder) = (if is_dir { Some(item) } else { item.GetParent().ok() }) else { return false };
        _ = app(|a| a.drop_open.replace((self.hwnd, self.slot.load(Ordering::SeqCst), folder)));
        if PostMessageW(Some(self.hwnd), WM_DROP_OPEN, WPARAM(0), LPARAM(0)).is_ok() {
            return true;
        }
        _ = app(|a| a.drop_open.take());
        false
    }
}

impl IDropTarget_Impl for PaneDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.ok.set(drop_items(&d).is_some());
            self.pick(effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragEnter(self.hwnd, d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.pick(effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragOver(&POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe {
            self.hover(false);
            if let Some(hp) = &self.helper {
                _ = hp.DragLeave();
            }
        }
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            let allowed = *effect;
            *effect = DROPEFFECT_NONE;
            self.hover(false);
            let eff = drop_pick(allowed);
            if eff != DROPEFFECT_NONE && self.queue(&d) {
                *effect = eff;
            }
            if let Some(hp) = &self.helper {
                _ = hp.Drop(d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
}


pub(crate) unsafe fn pwstr(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as _));
    s
}

pub(crate) unsafe fn name(pidl: *const ITEMIDLIST, kind: SIGDN) -> String {
    SHGetNameFromIDList(pidl, kind).map(|p| pwstr(p)).unwrap_or_default()
}
