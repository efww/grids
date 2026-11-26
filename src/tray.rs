use crate::*;

// ---------- minimize to the notification area ----------
// View menu switch (s.tray, off by default). While its icon is really there (a.tray_on), a minimized Grids is
// hidden: off the taskbar and Alt+Tab. Every way back (icon, Win+Alt+E, a second start, the jump list, "Open in
// Grids") restores a minimized window, and SW_RESTORE shows a hidden one too. No icon, no hiding: an icon that
// could not be made (or was lost with Explorer) puts the window back on the taskbar.

pub(crate) const WM_TRAY: u32 = WM_APP + 54; // the icon's callback (NOTIFYICON_VERSION_4: event in LOWORD(lParam))
const TRAY_UID: u32 = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
// the icon's menu comes back through TPM_RETURNCMD, never as WM_COMMAND: its own number space
const TRAY_OPEN: u32 = 1;
const TRAY_QUIT: u32 = 2;
const TRAY_FAV: u32 = 100; // + favourite index
pub(crate) const TRAY_FAVS: usize = 15;

// "TaskbarCreated": Explorer (re)started and has none of the old icons
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

// a start that could not add the icon (sign-in: the taskbar is usually already up, so no TaskbarCreated comes)
// tries again every TRAY_RETRY_MS, TRAY_RETRIES times
pub(crate) const TIMER_TRAY: usize = 1153; // on main (this feature's id)
const TRAY_RETRY_MS: u32 = 3000;
const TRAY_RETRIES: u32 = 20;
static TRAY_TRIES: AtomicU32 = AtomicU32::new(0);

pub(crate) fn taskbar_msg() -> u32 {
    TASKBAR_CREATED.load(Ordering::Relaxed)
}

/// Hide the window: the switch is on, the icon is really there, and the window is minimized.
pub(crate) fn should_hide(tray: bool, icon: bool, minimized: bool) -> bool {
    tray && icon && minimized
}

#[derive(Debug, PartialEq)]
pub(crate) enum TrayPick {
    Nothing,
    Open,
    Fav(usize),
    Quit,
}

/// What a number from the icon's menu means; `favs` = favourites that were in it.
pub(crate) fn tray_pick(id: u32, favs: usize) -> TrayPick {
    match id {
        TRAY_OPEN => TrayPick::Open,
        TRAY_QUIT => TrayPick::Quit,
        n if n >= TRAY_FAV && ((n - TRAY_FAV) as usize) < favs.min(TRAY_FAVS) => TrayPick::Fav((n - TRAY_FAV) as usize),
        _ => TrayPick::Nothing,
    }
}

/// Once, when the main window is up: listen for Explorer restarting (also when Grids runs elevated).
pub(crate) unsafe fn tray_init(main: HWND) {
    let m = RegisterWindowMessageW(w!("TaskbarCreated"));
    TASKBAR_CREATED.store(m, Ordering::Relaxed);
    if m != 0 {
        _ = ChangeWindowMessageFilterEx(main, m, MSGFLT_ALLOW, None);
    }
}

fn nid(main: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: main, uID: TRAY_UID, ..Default::default() }
}

/// Adds the icon; true when it is there. A minimized window then leaves the taskbar.
pub(crate) unsafe fn tray_add(main: HWND) -> bool {
    let mut d = nid(main);
    d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    d.uCallbackMessage = WM_TRAY;
    let hinst: Option<HINSTANCE> = GetModuleHandleW(None).ok().map(|m| m.into());
    let (cx, cy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
    // app.ico, group id 1, at the small icon size (shared: never destroyed)
    d.hIcon = match LoadImageW(hinst, PCWSTR(1 as _), IMAGE_ICON, cx, cy, LR_SHARED) {
        Ok(h) => HICON(h.0),
        Err(_) => LoadIconW(hinst, PCWSTR(1 as _)).unwrap_or_default(),
    };
    for (k, c) in "Grids".encode_utf16().enumerate() {
        d.szTip[k] = c;
    }
    // one that is there after all (an earlier NIM_ADD that timed out, a taskbar DPI change) would fail NIM_ADD
    _ = Shell_NotifyIconW(NIM_DELETE, &nid(main));
    let mut ok = Shell_NotifyIconW(NIM_ADD, &d).as_bool();
    if ok {
        // without version 4 the icon's clicks come in another shape and do nothing: no icon rather than a dead one
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        if !Shell_NotifyIconW(NIM_SETVERSION, &d).as_bool() {
            _ = Shell_NotifyIconW(NIM_DELETE, &nid(main));
            ok = false;
        }
    }
    app(|a| a.tray_on = ok);
    if ok {
        hide_if_min(main); // a sign-in start was minimized before the icon came
    }
    ok
}

/// At start, with the switch on: the icon, or a few more tries on TIMER_TRAY. s.tray stays on either way.
pub(crate) unsafe fn tray_start(main: HWND) {
    if !tray_add(main) {
        TRAY_TRIES.store(0, Ordering::Relaxed);
        SetTimer(Some(main), TIMER_TRAY, TRAY_RETRY_MS, None);
    }
}

/// TIMER_TRAY: again, until the icon is there, the switch went off, or the tries run out.
pub(crate) unsafe fn tray_retry(main: HWND) {
    let done = !app(|a| a.s.tray) || app(|a| a.tray_on) || tray_add(main);
    if done || TRAY_TRIES.fetch_add(1, Ordering::Relaxed) + 1 >= TRAY_RETRIES {
        _ = KillTimer(Some(main), TIMER_TRAY);
    }
}

/// Takes the icon away. `show`: a hidden window comes back to the taskbar, still minimized.
pub(crate) unsafe fn tray_remove(main: HWND, show: bool) {
    if app(|a| std::mem::take(&mut a.tray_on)) {
        _ = Shell_NotifyIconW(NIM_DELETE, &nid(main));
    }
    if show && !IsWindowVisible(main).as_bool() {
        _ = ShowWindow(main, SW_SHOWMINNOACTIVE);
    }
}

/// WM_SIZE (after everything else there) and a new icon: minimized with the icon there, leave the taskbar.
pub(crate) unsafe fn hide_if_min(main: HWND) {
    let (tray, icon) = app(|a| (a.s.tray, a.tray_on));
    if should_hide(tray, icon, IsIconic(main).as_bool()) && IsWindowVisible(main).as_bool() {
        _ = ShowWindow(main, SW_HIDE);
    }
}

/// View menu: flips what is really there, so after a failed start one click tries again.
pub(crate) unsafe fn toggle_tray(main: HWND) {
    if app(|a| a.tray_on) {
        app(|a| a.s.tray = false);
        tray_remove(main, true);
        return;
    }
    app(|a| a.s.tray = true);
    if !tray_add(main) {
        _ = MessageBeep(MB_ICONWARNING);
        let text = tr("Could not add the notification area icon", "알림 영역 아이콘을 만들지 못함");
        app(|a| {
            a.s.tray = false;
            a.note = Some((text.into(), Instant::now() + Duration::from_secs(3)));
        });
    }
}

/// Explorer restarted: the icon again. Not possible: a hidden window goes back on the taskbar, never lost.
pub(crate) unsafe fn taskbar_created(main: HWND) {
    if !app(|a| a.s.tray) {
        return;
    }
    // the old icon went with the old taskbar; also sent on a taskbar DPI change, when it may still be there
    // (tray_add deletes it first)
    if !tray_add(main) {
        tray_remove(main, true);
    }
}

pub(crate) unsafe fn tray_msg(main: HWND, wp: WPARAM, lp: LPARAM) {
    match (lp.0 & 0xffff) as u32 {
        NIN_SELECT | NIN_KEYSELECT => bring_front(main),
        // a save / colour / About box is up: no menu (Exit would close Grids under it), just that box
        WM_CONTEXTMENU if !IsWindowEnabled(main).as_bool() => bring_front(main),
        WM_CONTEXTMENU => {
            let (x, y) = xy(LPARAM(wp.0 as isize)); // the icon's anchor, signed: monitors left of / above the main one
            tray_menu(main, POINT { x, y });
        }
        _ => {}
    }
}

unsafe fn tray_menu(main: HWND, at: POINT) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let item = |m: HMENU, id: u32, text: &str| AppendMenuW(m, MF_STRING, id as usize, &HSTRING::from(text));
    _ = item(menu, TRAY_OPEN, tr("Open Grids", "Grids 열기"));
    _ = SetMenuDefaultItem(menu, TRAY_OPEN, 0); // bold: what a click on the icon does
    let favs: Vec<String> = app(|a| a.s.favs.iter().take(TRAY_FAVS).cloned().collect());
    if !favs.is_empty() {
        if let Ok(sub) = CreatePopupMenu() {
            for (k, f) in favs.iter().enumerate() {
                _ = item(sub, TRAY_FAV + k as u32, &short_name(f).replace('&', "&&"));
            }
            _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, &HSTRING::from(tr("Favourites", "즐겨찾기")));
        }
    }
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    _ = item(menu, TRAY_QUIT, tr("Exit", "끝내기"));
    // foreground first, or the menu stays open when the user clicks elsewhere; WM_NULL after, as documented
    _ = SetForegroundWindow(main);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON, at.x, at.y, None, main, None).0;
    _ = PostMessageW(Some(main), WM_NULL, WPARAM(0), LPARAM(0));
    _ = DestroyMenu(menu); // and the favourites submenu with it
    match tray_pick(pick as u32, favs.len()) {
        TrayPick::Open => bring_front(main),
        // shown first: focus and the panes' checks (IsWindowVisible) need the window on screen
        TrayPick::Fav(k) => {
            bring_front(main);
            if IsWindowEnabled(main).as_bool() {
                let i = app(|a| a.s.active);
                if !go_near(i, &favs[k]) {
                    _ = MessageBeep(MB_ICONWARNING); // gone or unreachable
                }
                focus_view(i);
            }
        }
        // the same way as the title bar's ×: saves, then closes
        TrayPick::Quit if IsWindowEnabled(main).as_bool() => _ = PostMessageW(Some(main), WM_CLOSE, WPARAM(0), LPARAM(0)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_only_minimized_with_an_icon() {
        for tray in [false, true] {
            for icon in [false, true] {
                for min in [false, true] {
                    assert_eq!(should_hide(tray, icon, min), tray && icon && min, "{tray} {icon} {min}");
                }
            }
        }
        // the switch on but the icon not made (sign-in before the taskbar, Explorer gone): stays on the taskbar
        assert!(!should_hide(true, false, true));
    }

    #[test]
    fn tray_menu_numbers() {
        assert_eq!(tray_pick(0, 3), TrayPick::Nothing); // dismissed
        assert_eq!(tray_pick(TRAY_OPEN, 0), TrayPick::Open);
        assert_eq!(tray_pick(TRAY_QUIT, 0), TrayPick::Quit);
        assert_eq!(tray_pick(TRAY_FAV, 3), TrayPick::Fav(0));
        assert_eq!(tray_pick(TRAY_FAV + 2, 3), TrayPick::Fav(2));
        assert_eq!(tray_pick(TRAY_FAV + 3, 3), TrayPick::Nothing); // past the list
        assert_eq!(tray_pick(TRAY_FAV, 0), TrayPick::Nothing);
        // never more than TRAY_FAVS favourites, however many there are
        assert_eq!(tray_pick(TRAY_FAV + TRAY_FAVS as u32 - 1, 40), TrayPick::Fav(TRAY_FAVS - 1));
        assert_eq!(tray_pick(TRAY_FAV + TRAY_FAVS as u32, 40), TrayPick::Nothing);
        assert!(TRAY_FAV > TRAY_QUIT && TRAY_QUIT != TRAY_OPEN);
    }
}
