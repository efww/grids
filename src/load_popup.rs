use crate::*;

// ---------- load popup: saved sessions as cards with a to-scale layout preview ----------

pub(crate) const LOAD_SAVE_AS: usize = 1000;
pub(crate) const LOAD_FOLDER: usize = 1001;
pub(crate) const LOAD_MAX: usize = 9;
pub(crate) const LOAD_DEL: usize = 2000; // + card index: that card's ✕

pub(crate) struct LoadGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) cards: Vec<(RECT, RECT, RECT)>, // (card, thumbnail, delete ✕ on the thumbnail's top-right)
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

// The name line under a card's thumbnail; the rename box goes there.
pub(crate) fn load_name(card: &RECT, thumb: &RECT, dpi: i32) -> RECT {
    let s = |v: i32| v * dpi / 96;
    RECT { left: card.left + s(6), right: card.right - s(6), top: thumb.bottom + s(4), bottom: thumb.bottom + s(24) }
}

pub(crate) fn ago(t: std::time::SystemTime) -> String {
    ago_text(t.elapsed().map(|d| d.as_secs() / 60).unwrap_or(0), KO.load(Ordering::Relaxed))
}

pub(crate) fn ago_text(m: u64, ko: bool) -> String {
    let tr = |en: &'static str, k: &'static str| if ko { k } else { en };
    match m {
        0 => tr("just now", "방금").into(),
        1..=59 => format!("{m}{}", tr(" min ago", "분 전")),
        60..=1439 => format!("{}{}", m / 60, tr(" h ago", "시간 전")),
        _ => format!("{}{}", m / 1440, tr(" d ago", "일 전")),
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
                text(dc, inner, &place_label(&s.paths[i]), font, pal().sub, DT_CENTER);
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
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsLoad"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.load = h);
}

// Saved sessions, most recently changed first: (file, its layout, "5 min ago").
pub(crate) fn session_cards() -> Vec<(PathBuf, Session, String)> {
    let entries = std::fs::read_dir(sessions_dir()).into_iter().flatten().flatten();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .map(|e| e.path())
        .filter(|p| is_session_file(p))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0)); // most recent first
    files
        .into_iter()
        .take(LOAD_MAX)
        .filter_map(|(t, p)| Some((p.clone(), Session::parse(&std::fs::read_to_string(&p).ok()?), ago(t))))
        .collect()
}

// Popup size for the cards it has now, rename box over its card's name: after a delete / rename and on DPI change.
pub(crate) unsafe fn fit_load() {
    let (h, edit, editing, n, dpi, fh) = app(|a| (a.load, a.load_edit, a.load_editing, a.cards.len(), a.dpi, a.fh));
    if h.0.is_null() {
        return;
    }
    let g = load_geo(n, dpi);
    _ = SetWindowPos(h, None, 0, 0, g.w, g.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
    if let Some((card, thumb, _)) = editing.and_then(|k| g.cards.get(k)) {
        let r = load_name(card, thumb, dpi);
        let (inset, eh) = (4 * dpi / 96, fh.min(r.bottom - r.top - 2));
        _ = MoveWindow(edit, r.left + inset, r.top + (r.bottom - r.top - eh) / 2, r.right - r.left - 2 * inset, eh, false);
    }
    _ = InvalidateRect(Some(h), None, false);
}

unsafe fn reload_cards() {
    let cards = session_cards();
    app(|a| (a.cards, a.load_hot) = (cards, None));
    fit_load();
}

// Same file on disk (case, short names); false when either can't be resolved, e.g. no current session.
fn same_file(a: &Path, b: &str) -> bool {
    std::fs::canonicalize(a).is_ok_and(|x| std::fs::canonicalize(b).is_ok_and(|y| x == y))
}

// The current session file after deleting / renaming one. `before`: it when the action began, `now`: it after (a
// second instance may have loaded another session while the shell's confirmation was up: that one keeps its link).
// None = leave it.
pub(crate) fn file_after_delete(before: &str, now: &str, was_cur: bool, deleted: bool) -> Option<String> {
    (deleted && was_cur && now == before).then(String::new)
}

pub(crate) fn file_after_rename(before: &str, now: &str, was_cur: bool, new: &str) -> Option<String> {
    (was_cur && now == before).then(|| new.to_string())
}

// A session name typed on a card, trimmed; None if Windows wouldn't take it as a file name.
pub(crate) fn session_stem_ok(input: &str) -> Option<String> {
    let s = input.trim();
    let bad = s.is_empty() || s.ends_with('.') || s.chars().any(|c| c < ' ' || r#"<>:"/\|?*"#.contains(c));
    let dev = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let numbered = dev.len() == 4 && (dev.starts_with("COM") || dev.starts_with("LPT")) && (b'1'..=b'9').contains(&dev.as_bytes()[3]);
    (!bad && !numbered && !matches!(dev.as_str(), "CON" | "PRN" | "AUX" | "NUL")).then(|| s.to_string())
}

// Renames a session file, keeping its extension (.grids, or .qdr from before the rename). Never replaces another
// file: without MOVEFILE_REPLACE_EXISTING the OS refuses a taken name; a case-only change goes through.
pub(crate) fn rename_session(old: &Path, stem: &str) -> Result<PathBuf> {
    let ext = old.extension().unwrap_or_default().to_string_lossy();
    let new = old.with_file_name(format!("{stem}.{ext}"));
    if new != old {
        unsafe { MoveFileExW(&HSTRING::from(old.as_os_str()), &HSTRING::from(new.as_os_str()), MOVE_FILE_FLAGS(0))? };
    }
    Ok(new)
}

// ✕ / Delete on a card: the shell deletes it (to the Recycle Bin when it can, with its confirmation if that's on).
// Its dialogs run a modal loop, so the current session is read again afterwards. true = gone.
pub(crate) unsafe fn delete_session(path: &Path) -> bool {
    let (main, before) = app(|a| (a.main, a.s.file.clone()));
    let was_cur = same_file(path, &before);
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
    // compare and set in one borrow: nothing can come in between
    let relinked = app(|a| {
        if deleted {
            let note = if KO.load(Ordering::Relaxed) { format!("세션 \"{name}\" 삭제됨") } else { format!("Session \"{name}\" deleted") };
            a.note = Some((note, Instant::now() + Duration::from_secs(3)));
        }
        file_after_delete(&before, &a.s.file, was_cur, deleted).map(|f| a.s.file = f).is_some()
    });
    if relinked {
        refresh_save_button(); // the next Ctrl+S asks for a name
    }
    if deleted {
        update_status();
    }
    deleted
}

// The confirmation may have deactivated (closed) the popup: then it opens again in the same place.
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

// Right-click → Rename: an edit box over the card's name, all of it selected.
unsafe fn start_rename(h: HWND, k: usize) {
    let (mut edit, font, name) = app(|a| (a.load_edit, a.font, a.cards.get(k).map(|c| c.0.file_stem().unwrap_or_default().to_string_lossy().into_owned())));
    let Some(name) = name else { return };
    if edit.0.is_null() {
        edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), 1, font);
    }
    _ = SetWindowTextW(edit, &HSTRING::from(name));
    app(|a| (a.load_edit, a.load_editing, a.load_hot) = (edit, Some(k), None));
    fit_load();
    _ = ShowWindow(edit, SW_SHOW);
    _ = SetFocus(Some(edit));
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
}

// Esc, a click or focus leaving: the box goes, nothing renamed. Taken first, so the EN_KILLFOCUS this causes
// does nothing. Some = was renaming.
pub(crate) unsafe fn end_rename() -> Option<usize> {
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

// Enter in the box. A bad or taken name beeps and keeps the box open.
pub(crate) unsafe fn commit_rename() {
    let (edit, k, before) = app(|a| (a.load_edit, a.load_editing, a.s.file.clone()));
    let Some(old) = k.and_then(|k| app(|a| a.cards.get(k).map(|c| c.0.clone()))) else { return };
    let was_cur = same_file(&old, &before);
    let Some(new) = session_stem_ok(&text_of(edit)).and_then(|s| rename_session(&old, &s).ok()) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    end_rename();
    if new == old {
        return;
    }
    let relinked = app(|a| file_after_rename(&before, &a.s.file, was_cur, &new.to_string_lossy()).map(|f| a.s.file = f).is_some());
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
            text(dc, r, tr("No saved sessions", "저장된 세션이 없습니다"), font, pal().sub, DT_CENTER);
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
        link(g.save_as, tr("+ Save as new", "+ 새 이름으로 저장"), LOAD_SAVE_AS, DT_LEFT);
        link(g.folder, tr("Open sessions folder", "세션 폴더 열기"), LOAD_FOLDER, DT_RIGHT);
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
    // the ✕ sits on the card: checked first, or a click meant to delete would load
    if let Some(k) = g.cards.iter().position(|(_, _, del)| inside(del)) {
        return Some(LOAD_DEL + k);
    }
    g.cards.iter().position(|(c, _, _)| inside(c))
}

pub(crate) extern "system" fn load_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
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
                // a click while renaming only ends the rename
                if end_rename().is_some() {
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
                if end_rename().is_some() {
                    return LRESULT(0);
                }
                let (x, y) = xy(lp);
                let k = match load_hit(&g, x, y) {
                    Some(k) if k >= LOAD_DEL => k - LOAD_DEL,
                    Some(k) if k < LOAD_SAVE_AS => k,
                    _ => return LRESULT(0),
                };
                let Ok(menu) = CreatePopupMenu() else { return LRESULT(0) };
                _ = AppendMenuW(menu, MF_STRING, 1, &HSTRING::from(tr("Rename", "이름 바꾸기")));
                _ = AppendMenuW(menu, MF_STRING, 2, &HSTRING::from(tr("Delete", "삭제")));
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
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_COMMAND if ((wp.0 >> 16) & 0xffff) as u32 == EN_KILLFOCUS => _ = end_rename(),
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
