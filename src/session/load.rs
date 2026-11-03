use std::time::Duration;

use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVE_FILE_FLAGS};

use crate::imports::*;
use super::*;
use crate::app::*;
use crate::ui::*;
use crate::pane::*;
use crate::i18n::*;

// ---------- load popup: saved sessions as cards with a to-scale layout preview ----------

pub(crate) const LOAD_SAVE_AS: usize = 1000;
pub(crate) const LOAD_FOLDER: usize = 1001;
pub(crate) const LOAD_MAX: usize = 9;
pub(crate) const LOAD_DEL: usize = 2000; // + card index: that card's ✕

pub(crate) struct LoadGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) cards: Vec<(RECT, RECT, RECT)>, // card, thumbnail, delete ✕ on the thumbnail's top-right
    pub(crate) save_as: RECT,
    pub(crate) folder: RECT,
}

pub(crate) fn load_geo(n: usize, dpi: i32) -> LoadGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, gap, cw, th, ch) = (s(12), s(10), s(140), s(84), s(84 + 44));
    let rows = n.div_ceil(3).max(1) as i32;
    let w = 2 * pad + 3 * cw + 2 * gap;
    let cards = (0..n as i32)
        .map(|k| {
            let (x, y) = (pad + (k % 3) * (cw + gap), pad + (k / 3) * (ch + gap));
            let card = RECT { left: x, top: y, right: x + cw, bottom: y + ch };
            let thumb = RECT { left: x + s(6), top: y + s(6), right: x + cw - s(6), bottom: y + s(6) + th - s(12) };
            (card, thumb, RECT { left: thumb.right - s(20), bottom: thumb.top + s(20), ..thumb })
        })
        .collect();
    let fy = pad + rows * (ch + gap);
    let link = |x0: i32, x1: i32| RECT { left: x0, top: fy, right: x1, bottom: fy + s(30) };
    LoadGeo { w, h: fy + s(30) + s(8), cards, save_as: link(pad, w / 2), folder: link(w / 2, w - pad) }
}

/// The name line under a card's thumbnail. The rename box sits there.
pub(crate) fn load_name(card: &RECT, thumb: &RECT, dpi: i32) -> RECT {
    let s = |v: i32| v * dpi / 96;
    RECT { left: card.left + s(6), right: card.right - s(6), top: thumb.bottom + s(4), bottom: thumb.bottom + s(24) }
}

pub(crate) fn short_name(path: &str) -> String {
    if path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        return tr(crate::Key::ThisPc).into();
    }
    folder_display_name(path)
}

// Title bar, taskbar and Alt+Tab follow the active pane's folder.
pub(crate) fn window_title(disp: &str) -> String {
    if disp.is_empty() { "Maguro".into() } else { format!("{} - Maguro", short_name(disp)) }
}

pub(crate) fn ago(t: std::time::SystemTime) -> String {
    let m = t.elapsed().map(|d| d.as_secs() / 60).unwrap_or(0);
    match m {
        0 => tr(crate::Key::JustNow).into(),
        1..=59 => format!("{m}{}", tr(crate::Key::MinAgo)),
        60..=1439 => format!("{}{}", m / 60, tr(crate::Key::HAgo)),
        _ => format!("{}{}", m / 1440, tr(crate::Key::DAgo)),
    }
}

// The layout drawn to scale, each pane labelled with its folder name.
pub(crate) unsafe fn draw_thumb(dc: HDC, r: RECT, s: &Session, font: HFONT) {
    fill(dc, r, pal().bg);
    let xs = cells(r.left + 2, r.right - r.left - 4, &s.colr, 2);
    let mut i = 0;
    for (c, &(x0, x1)) in xs.iter().enumerate() {
        for (y0, y1) in cells(r.top + 2, r.bottom - r.top - 4, &s.rowr[c], 2) {
            let pane = RECT { left: x0, top: y0, right: x1, bottom: y1 };
            fill(dc, pane, pal().card);
            frame(dc, pane, 1, pal().line);
            if x1 - x0 > 24 && y1 - y0 > 12 {
                let inner = RECT { left: x0 + 3, right: x1 - 3, ..pane };
                text(dc, inner, &short_name(&s.paths[i]), font, pal().sub, DT_CENTER);
            }
            i += 1;
        }
    }
}

pub(crate) unsafe fn open_load() {
    let (main, dpi, open, closed) = app(|a| (a.main, a.dpi, a.load, a.load_closed));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let cards = session_cards();
    let g = load_geo(cards.len(), dpi);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LOAD) {
        _ = GetWindowRect(b, &mut rc);
    }
    app(|a| (a.cards, a.load_hot) = (cards, None));
    let style = WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("MaguroLoad"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.load = h);
}

/// Saved sessions, most recently changed first: `(file, its layout, "5 min ago")`.
pub(crate) fn session_cards() -> Vec<(PathBuf, Session, String)> {
    let entries = std::fs::read_dir(sessions_dir()).into_iter().flatten().flatten();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .map(|e| e.path())
        .filter(|p| is_session_file(p))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .take(LOAD_MAX)
        .filter_map(|(t, p)| Some((p.clone(), Session::parse(&std::fs::read_to_string(&p).ok()?), ago(t))))
        .collect()
}

/// Popup size for the cards it has now, and the rename box over its card's name.
pub(crate) unsafe fn fit_load() {
    let (h, edit, editing, n, dpi) = app(|a| (a.load, a.load_edit, a.load_editing, a.cards.len(), a.dpi));
    if h.0.is_null() {
        return;
    }
    let g = load_geo(n, dpi);
    _ = SetWindowPos(h, None, 0, 0, g.w, g.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    if let Some((card, thumb, _)) = editing.and_then(|k| g.cards.get(k)) {
        let r = load_name(card, thumb, dpi);
        let inset = 4 * dpi / 96;
        let outer = RECT { left: r.left + inset, top: r.top, right: (r.right - inset).max(r.left + inset), bottom: r.bottom };
        place_edit(edit, outer, true);
    }
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn reload_cards() {
    let cards = session_cards();
    app(|a| (a.cards, a.load_hot) = (cards, None));
    fit_load();
}

/// Same file on disk. False when either path cannot be resolved, including no current session.
fn same_file(a: &Path, b: &str) -> bool {
    std::fs::canonicalize(a).is_ok_and(|x| std::fs::canonicalize(b).is_ok_and(|y| x == y))
}

/// The current session file after a delete. `before` is the file when the delete began, `now` is it
/// after a confirmation dialog (another session may have opened meanwhile and keeps its link).
/// None leaves the link alone.
pub(crate) fn file_after_delete(before: &str, now: &str, was_cur: bool, deleted: bool) -> Option<String> {
    (deleted && was_cur && now == before).then(String::new)
}

pub(crate) fn file_after_rename(before: &str, now: &str, was_cur: bool, new: &str) -> Option<String> {
    (was_cur && now == before).then(|| new.to_string())
}

/// A session name typed on a card, trimmed. None if Windows would refuse it as a file name.
pub(crate) fn session_stem_ok(input: &str) -> Option<String> {
    let s = input.trim();
    let bad = s.is_empty() || s.ends_with('.') || s.chars().any(|c| c < ' ' || r#"<>:"/\|?*"#.contains(c));
    let dev = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let numbered = dev.len() == 4 && (dev.starts_with("COM") || dev.starts_with("LPT")) && dev.as_bytes().get(3).is_some_and(|b| (b'1'..=b'9').contains(b));
    (!bad && !numbered && !matches!(dev.as_str(), "CON" | "PRN" | "AUX" | "NUL")).then(|| s.to_string())
}

/// Renames a session file, keeping its `.maguro` extension.
/// Never replaces another file: without `MOVEFILE_REPLACE_EXISTING` the OS refuses a taken name.
/// A case-only change still goes through.
pub(crate) fn rename_session(old: &Path, stem: &str) -> Result<PathBuf> {
    let ext = old.extension().unwrap_or_default().to_string_lossy();
    let new = old.with_file_name(format!("{stem}.{ext}"));
    if new != old {
        unsafe { MoveFileExW(&HSTRING::from(old.as_os_str()), &HSTRING::from(new.as_os_str()), MOVE_FILE_FLAGS(0))? };
    }
    Ok(new)
}

/// ✕ or Delete. The shell deletes it (Recycle Bin when it can, with its confirmation when that is on).
/// Those dialogs run a modal loop, so the current session is read again afterwards. True means the file is gone.
pub(crate) unsafe fn delete_session(path: &Path) -> bool {
    let (main, before_paths) = app(|a| {
        let paths: Vec<String> = a.s.tabs.iter().map(|t| t.session.clone()).collect();
        (a.main, paths)
    });
    let linked: Vec<bool> = before_paths.iter().map(|p| same_file(path, p)).collect();
    let op = (|| -> Result<IFileOperation> {
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        op.SetOwnerWindow(main)?;
        op.SetOperationFlags(FOF_ALLOWUNDO | FOF_WANTNUKEWARNING)?;
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)?;
        op.DeleteItem(&item, None)?;
        Ok(op)
    })();
    let (ok, aborted) = match &op {
        Ok(op) => (op.PerformOperations().is_ok(), op.GetAnyOperationsAborted().is_ok_and(|b| b.as_bool())),
        Err(_) => (false, false),
    };
    let deleted = ok && !aborted && !path.exists();
    if !deleted && !aborted {
        _ = MessageBeep(MB_ICONWARNING);
    }
    let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let relinked = app(|a| {
        if deleted {
            let note = tf(Key::SessionDeleted, &[("name", &name)]);
            a.note = Some((note, Instant::now() + Duration::from_secs(3)));
            a.lost = 0;
        }
        let now: Vec<String> = a.s.tabs.iter().map(|t| t.session.clone()).collect();
        let Some(next) = sessions_after_delete(&before_paths, &now, &linked, deleted) else { return false };
        for (i, p) in next.into_iter().enumerate() {
            if let Some(t) = a.s.tabs.get_mut(i) {
                t.session = p;
            }
        }
        settle_sessions(&mut a.s);
        true
    });
    if relinked {
        refresh_save_button();
    }
    if deleted {
        update_status();
    }
    deleted
}

/// The confirmation may have closed the popup. It opens again in the same place.
unsafe fn delete_card(h: HWND, k: usize) {
    let Some(path) = app(|a| a.cards.get(k).map(|c| c.0.clone())) else { return };
    delete_session(&path);
    if app(|a| a.load == h) {
        reload_cards();
    } else {
        app(|a| a.load_closed = None);
        open_load();
    }
}

/// Right-click → Rename: an edit over the card's name, all of it selected.
unsafe fn start_rename(h: HWND, k: usize) {
    let (mut edit, font, name) = app(|a| {
        (a.load_edit, a.font, a.cards.get(k).map(|c| c.0.file_stem().unwrap_or_default().to_string_lossy().into_owned()))
    });
    let Some(name) = name else { return };
    if edit.0.is_null() {
        edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), 1, font);
        _ = ShowWindow(edit, SW_HIDE);
    }
    _ = SetWindowTextW(edit, &HSTRING::from(&name));
    app(|a| (a.load_edit, a.load_editing, a.load_hot) = (edit, Some(k), None));
    fit_load();
    _ = ShowWindow(edit, SW_SHOW);
    _ = SetFocus(Some(edit));
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
}

/// Esc, a click, or focus leaving. The box goes and nothing is renamed.
/// Taken first, so the `EN_KILLFOCUS` this causes does nothing. Some = was renaming.
pub(crate) unsafe fn end_load_rename() -> Option<usize> {
    let (h, edit, k) = app(|a| (a.load, a.load_edit, a.load_editing.take()));
    if k.is_some() {
        if GetFocus() == edit {
            _ = SetFocus(Some(h));
        }
        _ = ShowWindow(edit, SW_HIDE);
        _ = InvalidateRect(Some(h), None, false);
    }
    k
}

/// Enter in the box. A bad or taken name beeps and leaves the box open.
pub(crate) unsafe fn commit_load_rename() {
    let (edit, k, before_paths) = app(|a| {
        let paths: Vec<String> = a.s.tabs.iter().map(|t| t.session.clone()).collect();
        (a.load_edit, a.load_editing, paths)
    });
    let Some(old) = k.and_then(|k| app(|a| a.cards.get(k).map(|c| c.0.clone()))) else { return };
    let linked: Vec<bool> = before_paths.iter().map(|p| same_file(&old, p)).collect();
    let Some(new) = session_stem_ok(&text_of(edit)).and_then(|s| rename_session(&old, &s).ok()) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    end_load_rename();
    if new == old {
        return;
    }
    let relinked = app(|a| {
        let now: Vec<String> = a.s.tabs.iter().map(|t| t.session.clone()).collect();
        let Some(next) = sessions_after_rename(&before_paths, &now, &linked, &new.to_string_lossy()) else { return false };
        for (i, p) in next.into_iter().enumerate() {
            if let Some(t) = a.s.tabs.get_mut(i) {
                t.session = p;
            }
        }
        settle_sessions(&mut a.s);
        true
    });
    if relinked {
        refresh_save_button();
    }
    reload_cards();
}

pub(crate) unsafe fn paint_load(h: HWND) {
    let (cards, hot, cur, dpi, font, small, icon_font, accent, editing) = app(|a| {
        let cards: Vec<_> = a.cards.iter().map(|(p, s, t)| (p.clone(), s.clone(), t.clone())).collect();
        (cards, a.load_hot, a.s.file.clone(), a.dpi, a.font, a.small_font, a.icon_font, a.s.accent_now(), a.load_editing)
    });
    let g = load_geo(cards.len(), dpi);
    let s = |v: i32| v * dpi / 96;
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        if cards.is_empty() {
            let r = RECT { bottom: g.save_as.top, ..rc };
            text(dc, r, tr(crate::Key::NoSavedSessions), font, pal().sub, DT_CENTER);
        }
        for (k, ((path, sess, when), (card, thumb, del))) in cards.iter().zip(&g.cards).enumerate() {
            let on = hot == Some(k) || hot == Some(LOAD_DEL + k);
            if on {
                fill(dc, *card, pal().pill);
            }
            draw_thumb(dc, *thumb, sess, small);
            if path.to_string_lossy() == cur.as_str() {
                frame(dc, *thumb, s(2), accent); // the session you're in
            }
            if on {
                fill(dc, *del, pal().card);
                text(dc, *del, "\u{E711}", icon_font, if hot == Some(LOAD_DEL + k) { pal().text } else { pal().sub }, DT_CENTER);
            }
            let name_r = load_name(card, thumb, dpi);
            if editing == Some(k) {
                fill(dc, name_r, pal().pill);
                frame(dc, name_r, 1, accent);
            } else {
                text(dc, name_r, &path.file_stem().unwrap_or_default().to_string_lossy(), font, pal().text, DT_LEFT);
            }
            let line = |top: i32, hgt: i32| RECT { left: card.left + s(6), right: card.right - s(6), top, bottom: top + hgt };
            let meta = format!("{}  ·  {}", layout_label(&sess.heights), when);
            text(dc, line(thumb.bottom + s(22), s(16)), &meta, small, pal().sub, DT_LEFT);
        }
        fill(dc, RECT { left: s(12), top: g.save_as.top - 1, right: rc.right - s(12), bottom: g.save_as.top }, pal().line);
        let link = |r: RECT, label: &str, k: usize, align| {
            text(dc, r, label, font, if hot == Some(k) { pal().text } else { accent }, align);
        };
        link(g.save_as, tr(crate::Key::SaveThisTab2), LOAD_SAVE_AS, DT_LEFT);
        link(g.folder, tr(crate::Key::OpenSessionsFolder), LOAD_FOLDER, DT_RIGHT);
    });
}

pub(crate) fn load_hit(g: &LoadGeo, x: i32, y: i32) -> Option<usize> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    if inside(&g.save_as) {
        return Some(LOAD_SAVE_AS);
    }
    if inside(&g.folder) {
        return Some(LOAD_FOLDER);
    }
    // The ✕ sits on the card, so it is checked first. Otherwise a click meant to delete would load.
    if let Some(k) = g.cards.iter().position(|(_, _, del)| inside(del)) {
        return Some(LOAD_DEL + k);
    }
    g.cards.iter().position(|(c, _, _)| inside(c))
}

pub(crate) extern "system" fn load_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| load_proc_inner(h, msg, wp, lp)))
}
fn load_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        let g = app(|a| load_geo(a.cards.len(), a.dpi));
        match msg {
            WM_PAINT => paint_load(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hot = load_hit(&g, x, y);
                if app(|a| std::mem::replace(&mut a.load_hot, hot)) != hot {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP => {
                // A click while renaming only ends the rename.
                if end_load_rename().is_some() {
                    return LRESULT(0);
                }
                let (x, y) = xy(lp);
                let Some(k) = load_hit(&g, x, y) else { return LRESULT(0) };
                if k >= LOAD_DEL {
                    delete_card(h, k - LOAD_DEL);
                    return LRESULT(0);
                }
                let file = app(|a| a.cards.get(k).map(|c| c.0.clone()));
                _ = DestroyWindow(h);
                match k {
                    LOAD_SAVE_AS => _ = save_session_as(),
                    LOAD_FOLDER => {
                        let d = sessions_dir();
                        _ = std::fs::create_dir_all(&d);
                        navigate(app(|a| a.s.active), &d.to_string_lossy());
                    }
                    _ => {
                        if let Some(f) = file {
                            load_session_file(&f);
                        }
                    }
                }
            }
            WM_RBUTTONUP => {
                if end_load_rename().is_some() {
                    return LRESULT(0);
                }
                let (x, y) = xy(lp);
                let k = match load_hit(&g, x, y) {
                    Some(k) if k >= LOAD_DEL => k - LOAD_DEL,
                    Some(k) if k < LOAD_SAVE_AS => k,
                    _ => return LRESULT(0),
                };
                let Ok(menu) = CreatePopupMenu() else { return LRESULT(0) };
                _ = AppendMenuW(menu, MF_STRING, 1, &HSTRING::from(tr(crate::Key::Rename)));
                _ = AppendMenuW(menu, MF_STRING, 2, &HSTRING::from(tr(crate::Key::Delete)));
                let mut at = POINT::default();
                _ = GetCursorPos(&mut at);
                let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, at.x, at.y, None, h, None).0;
                _ = DestroyMenu(menu);
                match pick {
                    1 => start_rename(h, k),
                    2 => delete_card(h, k),
                    _ => {}
                }
            }
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().pill));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().pill));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND if ((wp.0 >> 16) & 0xffff) as u32 == EN_KILLFOCUS => _ = end_load_rename(),
            WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => _ = DestroyWindow(h),
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| {
                (a.load, a.load_closed) = (HWND::default(), Some(Instant::now()));
                (a.load_edit, a.load_editing) = (HWND::default(), None);
            }),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_card_hits() {
        for dpi in [96, 144] {
            let g = load_geo(5, dpi);
            for (k, (card, thumb, del)) in g.cards.iter().enumerate() {
                assert_eq!(del.right - del.left, 20 * dpi / 96);
                assert!(del.right == thumb.right && del.top == thumb.top && card.left < del.left);
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
        assert_eq!(file_after_delete("A", "A", true, true), Some(String::new()));
        assert_eq!(file_after_delete("A", "B", true, true), None);
        assert_eq!(file_after_delete("A", "B", true, false), None);
        assert_eq!(file_after_delete("A", "A", true, false), None);
        assert_eq!(file_after_delete("A", "A", false, true), None);
        assert_eq!(file_after_rename("A", "A", true, "N"), Some("N".into()));
        assert_eq!(file_after_rename("A", "B", true, "N"), None);
        assert_eq!(file_after_rename("A", "A", false, "N"), None);
        assert_eq!(file_after_rename("", "", false, "N"), None);
    }

    #[test]
    fn session_rename_never_replaces() {
        let d = std::env::temp_dir().join(format!("maguro-rename-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let read = |n: &str| std::fs::read_to_string(d.join(n)).unwrap();
        std::fs::write(d.join("a.maguro"), "A").unwrap();
        std::fs::write(d.join("b.maguro"), "B").unwrap();
        assert!(rename_session(&d.join("a.maguro"), "b").is_err());
        assert_eq!((read("a.maguro"), read("b.maguro")), ("A".into(), "B".into()));
        assert_eq!(rename_session(&d.join("a.maguro"), "A").unwrap(), d.join("A.maguro"));
        let names = || std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert!(names().contains(&"A.maguro".into()) && !names().contains(&"a.maguro".into()));
        assert_eq!(read("A.maguro"), "A");
        assert_eq!(rename_session(&d.join("A.maguro"), "A").unwrap(), d.join("A.maguro"));
        _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod utf8_boundary_tests {
    #[test]
    fn japanese_korean_and_emoji_session_names_are_kept() {
        assert_eq!(super::session_stem_ok("샘플").as_deref(), Some("샘플"));
        assert_eq!(super::session_stem_ok("한글폴더").as_deref(), Some("한글폴더"));
        assert_eq!(super::session_stem_ok("😀").as_deref(), Some("😀"));
        assert_eq!(super::session_stem_ok("  サンプル  ").as_deref(), Some("サンプル"));
        assert_eq!(super::session_stem_ok("abcdサ"), Some("abcdサ".into()));
        assert_eq!(super::session_stem_ok("COM1"), None);
        assert_eq!(super::session_stem_ok("lpt9"), None);
    }
}
