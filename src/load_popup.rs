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

// One pane of a card's preview, in slot order (down each column, left to right).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ThumbPane {
    pub(crate) plate: RECT,
    pub(crate) head: Option<RECT>, // header strip; only when the folder name still fits under it
    pub(crate) band: Option<RECT>, // colour label strip along the top, when the pane has one
    pub(crate) name: Option<RECT>, // where its folder name goes, when the pane is big enough for one
}

// The layout to scale inside r, spaced for dpi. The margin is wider than the "session you're in" frame drawn
// over the preview's edge (s(2)), so it never covers an edge pane's label or header. A pane shows its name
// whenever it is over 24 × 12 (scaled); the header goes first when both don't fit.
pub(crate) fn thumb_geo(r: RECT, s: &Session, dpi: i32) -> Vec<ThumbPane> {
    let k = |v: i32| v * dpi / 96;
    let (pad, gap, head_h, band_h) = (k(3), k(2), k(4), k(2).max(1));
    let (name_w, name_h) = (k(24), k(12));
    let xs = cells(r.left + pad, (r.right - r.left - 2 * pad).max(0), &s.colr, gap);
    let mut out = Vec::with_capacity(s.panes());
    for (c, &(x0, x1)) in xs.iter().enumerate() {
        for (y0, y1) in cells(r.top + pad, (r.bottom - r.top - 2 * pad).max(0), &s.rowr[c], gap) {
            let plate = RECT { left: x0, top: y0, right: x1, bottom: y1 };
            let i = out.len();
            // inside the 1 px border
            let inner = RECT { left: x0 + 1, top: y0 + 1, right: x1 - 1, bottom: y1 - 1 };
            let (iw, ih) = (inner.right - inner.left, inner.bottom - inner.top);
            let head = (iw > 0 && ih - head_h > name_h).then_some(RECT { bottom: inner.top + head_h, ..inner });
            let tinted = s.tints.get(i).is_some_and(|t| (1..=6).contains(t));
            let band = (tinted && iw > 0 && ih > 0).then_some(RECT { bottom: inner.top + band_h.min(ih), ..inner });
            // a band alone doesn't push the name down: in a short pane those rows are what keeps it whole
            let top = head.map_or(y0, |h| h.bottom);
            let name = (x1 - x0 > name_w && y1 - y0 > name_h).then_some(RECT { left: x0 + k(3), top, right: x1 - k(3), bottom: y1 });
            out.push(ThumbPane { plate, head, band, name });
        }
    }
    out
}

// The layout drawn to scale like the window it restores: each pane with its folder name and colour label,
// the pane you were last in with its header in the selection colour (or its border, where no header fits).
pub(crate) unsafe fn draw_thumb(dc: HDC, r: RECT, s: &Session, font: HFONT, dpi: i32, accent: u32) {
    fill(dc, r, pal().bg);
    for (i, p) in thumb_geo(r, s, dpi).iter().enumerate() {
        let on = i == s.active;
        fill(dc, p.plate, pal().card);
        frame(dc, p.plate, 1, if on && p.head.is_none() { accent } else { pal().line });
        if let Some(h) = p.head.filter(|_| on) {
            fill(dc, h, accent);
        }
        if let (Some(b), Some(t)) = (p.band, s.tints.get(i).and_then(|&t| tint_color(t))) {
            fill(dc, b, t);
        }
        if let Some(n) = p.name {
            text(dc, n, &place_label(&s.paths[i]), font, pal().sub, DT_CENTER);
        }
    }
}

// The Load button: nothing picked, as before (the mouse picks).
pub(crate) unsafe fn open_load() {
    open_load_as(false);
}

// Ctrl+O: opens the popup with the session you're in picked (the first card if none; "+ Save as new" with no
// cards), or closes it. The button's 300 ms guard is for a click that closed it: a key opens it again at once.
pub(crate) unsafe fn toggle_load() {
    let h = app(|a| a.load);
    if !h.0.is_null() {
        _ = DestroyWindow(h);
        return;
    }
    app(|a| a.load_closed = None);
    open_load_as(true);
}

// The card picked when Ctrl+O opens the popup.
pub(crate) fn load_first_pick(cur: Option<usize>, n: usize) -> usize {
    cur.filter(|&k| k < n).unwrap_or(if n > 0 { 0 } else { LOAD_SAVE_AS })
}

unsafe fn open_load_as(keys: bool) {
    let (main, dpi, open, closed, file) = app(|a| (a.main, a.dpi, a.load, a.load_closed, a.s.file.clone()));
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
    let cur = cards.iter().position(|c| c.0.to_string_lossy() == file.as_str() || same_file(&c.0, &file));
    let hot = keys.then(|| load_first_pick(cur, cards.len()));
    let mut at = POINT::default();
    _ = GetCursorPos(&mut at);
    app(|a| (a.cards, a.load_hot, a.load_keys, a.load_cursor) = (cards, hot, keys, (at.x, at.y)));
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

// Read again after a delete / rename. While the keyboard is choosing, `pick` says which card stays picked
// (clamped to the cards there are now); for the mouse nothing is, as before.
unsafe fn reload_cards(pick: impl FnOnce(&[(PathBuf, Session, String)]) -> Option<usize>) {
    let cards = session_cards();
    let hot = app(|a| a.load_keys).then(|| pick(&cards)).flatten().map(|k| load_first_pick(Some(k.min(cards.len().saturating_sub(1))), cards.len()));
    app(|a| (a.cards, a.load_hot) = (cards, hot));
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
    (!bad && !crate::rename_rules::reserved_name(s)).then(|| s.to_string())
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
        refresh_jump_list(true);
    }
    deleted
}

// The confirmation may have deactivated (closed) the popup: then it opens again in the same place.
unsafe fn delete_card(h: HWND, k: usize) {
    let Some(path) = app(|a| a.cards.get(k).map(|c| c.0.clone())) else { return };
    let keys = app(|a| a.load_keys);
    delete_session(&path);
    if app(|a| a.load == h) {
        reload_cards(|_| Some(k)); // the next card moves up into its place
    } else {
        app(|a| a.load_closed = None);
        open_load_as(keys);
        if keys {
            let h = app(|a| {
                let n = a.cards.len();
                a.load_hot = Some(load_first_pick(Some(k.min(n.saturating_sub(1))), n));
                a.load
            });
            if !h.0.is_null() {
                _ = InvalidateRect(Some(h), None, false);
            }
        }
    }
}

// Right-click → Create desktop shortcut: a link that opens this session (double-click it, or give it a Shortcut
// key in its Properties). The popup stays open; the outcome shows on the main window's status line.
unsafe fn shortcut_card(k: usize) {
    let Some(path) = app(|a| a.cards.get(k).map(|c| c.0.clone())) else { return };
    let name = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let ko = KO.load(Ordering::Relaxed);
    let fail = desktop_shortcut(&path, ko).err();
    if fail.is_some() {
        _ = MessageBeep(MB_ICONWARNING);
    }
    let text = shortcut_note(ko, &name, fail.as_ref());
    let secs = if fail.is_some() { 5 } else { 3 };
    app(|a| a.note = Some((text, Instant::now() + Duration::from_secs(secs))));
    update_status();
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
    let (h, edit, k) = app(|a| {
        let k = a.load_editing.take();
        if a.load_keys && k.is_some() {
            a.load_hot = k; // F2 then Esc: the card stays picked
        }
        (a.load, a.load_edit, k)
    });
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
    refresh_jump_list(true);
    let relinked = app(|a| file_after_rename(&before, &a.s.file, was_cur, &new.to_string_lossy()).map(|f| a.s.file = f).is_some());
    if relinked {
        refresh_save_button();
    }
    reload_cards(|cards| cards.iter().position(|c| c.0 == new)); // renamed: it may have moved
}

pub(crate) unsafe fn paint_load(h: HWND) {
    let (cards, hot, cur, dpi, font, small, icon_font, accent, editing, keys) = app(|a| {
        let cards: Vec<_> = a.cards.iter().map(|(p, s, t)| (p.clone(), s.clone(), t.clone())).collect();
        (cards, a.load_hot, a.s.file.clone(), a.dpi, a.font, a.small_font, a.icon_font, a.s.accent_now(), a.load_editing, a.load_keys)
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
            draw_thumb(dc, *thumb, sess, small, dpi, accent);
            if path.to_string_lossy() == cur.as_str() {
                frame(dc, *thumb, s(2), accent); // the session you're in
            }
            if on {
                fill(dc, *del, pal().card);
                text(dc, *del, "\u{E711}", icon_font, if hot == Some(LOAD_DEL + k) { pal().text } else { pal().sub }, DT_CENTER);
            }
            let mut name_r = load_name(card, thumb, dpi);
            // choosing with the keys: each card's number (1 … 9 opens it), as the favourites list numbers its rows
            if keys && editing != Some(k) {
                text(dc, RECT { right: name_r.left + s(14), ..name_r }, &(k + 1).to_string(), font, accent, DT_LEFT);
                name_r.left += s(14);
            }
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
        // picked (mouse or keys): on a pill like a card's, so a key's pick shows as clearly as the cards' does
        // (around its words, the words where they always are)
        let link = |r: RECT, label: &str, k: usize, align| {
            let on = hot == Some(k);
            if on {
                let w = text_w(dc, label, font);
                let (l, rt) = if align == DT_RIGHT { (r.right - w, r.right) } else { (r.left, r.left + w) };
                fill(dc, RECT { left: l - s(6), top: r.top + s(3), right: rt + s(6), bottom: r.bottom - s(3) }, pal().pill);
            }
            text(dc, r, label, font, if on { pal().text } else { accent }, align);
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

// The pick after an arrow / Home / End, over load_geo's three columns: ←/→ go card to card (across rows),
// ↑/↓ a row, and below the last row are the two links ("+ Save as new" under the first two columns, "Open
// sessions folder" under the third); ↑ from a link goes back to the last row. None = not a key that moves.
pub(crate) fn load_step(cur: Option<usize>, n: usize, vk: VIRTUAL_KEY) -> Option<usize> {
    if !matches!(vk, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END) {
        return None;
    }
    let first = load_first_pick(None, n);
    let last_row = n.saturating_sub(1) / 3;
    let Some(k) = cur else { return Some(if vk == VK_END && n > 0 { n - 1 } else { first }) };
    Some(match (vk, k) {
        (VK_HOME, _) => first,
        (VK_END, _) => n.checked_sub(1).unwrap_or(LOAD_FOLDER),
        (VK_RIGHT, LOAD_SAVE_AS | LOAD_FOLDER) => LOAD_FOLDER,
        (VK_LEFT, LOAD_SAVE_AS | LOAD_FOLDER) => LOAD_SAVE_AS,
        (VK_UP, LOAD_SAVE_AS) if n > 0 => last_row * 3,
        (VK_UP, LOAD_FOLDER) if n > 0 => (last_row * 3 + 2).min(n - 1),
        (_, LOAD_SAVE_AS | LOAD_FOLDER) => k,
        (_, k) if k >= n => first, // a card that's gone
        (VK_LEFT, k) => k.saturating_sub(1),
        (VK_RIGHT, k) => (k + 1).min(n - 1),
        (VK_UP, k) => k.checked_sub(3).unwrap_or(k),
        (VK_DOWN, k) if k / 3 < last_row => (k + 3).min(n - 1),
        (_, k) => if k % 3 == 2 { LOAD_FOLDER } else { LOAD_SAVE_AS },
    })
}

// A card, its ✕ or a link chosen: a click, Enter or the card's number.
unsafe fn activate(h: HWND, k: usize) {
    if k >= LOAD_DEL {
        delete_card(h, k - LOAD_DEL);
        return;
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

// The popup's keys (its rename box's never come here: pre_translate takes them). Esc closes; arrows, Home / End
// pick; Enter opens the pick, 1 … 9 that card; F2 renames, Del deletes (the shell's confirmation as for ✕).
// Esc, Enter, the digits and Del act once per press: auto-repeats (bit 30) are dropped, and a key that opened a
// session is held (app.held) until let go, so its repeats can't reach the pane that has the focus next. With
// Ctrl or Alt down none of them acts (a 1 typed with Ctrl+O's Ctrl still down). false = not the popup's key.
unsafe fn load_key(h: HWND, vk: VIRTUAL_KEY, repeat: bool) -> bool {
    if vk == VK_ESCAPE {
        if !repeat {
            _ = DestroyWindow(h);
        }
        return true;
    }
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    if down(VK_CONTROL) || down(VK_MENU) {
        return false;
    }
    let (n, hot) = app(|a| (a.cards.len(), a.load_hot));
    let cur = hot.map(|k| if k >= LOAD_DEL { k - LOAD_DEL } else { k }); // a ✕ the mouse left lit: its card
    let card = cur.filter(|&k| k < n);
    let digit = match vk.0 {
        d @ 0x31..=0x39 => Some((d - 0x31) as usize),         // 1 … 9
        d @ 0x61..=0x69 => Some((d - VK_NUMPAD1.0) as usize), // the number pad's
        _ => None,
    };
    if digit.is_none() && !matches!(vk, VK_RETURN | VK_F2 | VK_DELETE) && load_step(cur, n, vk).is_none() {
        return false;
    }
    // the keys are choosing from here on: picks are kept and the cards numbered
    if !app(|a| std::mem::replace(&mut a.load_keys, true)) {
        _ = InvalidateRect(Some(h), None, false);
    }
    if let Some(k) = digit {
        if !repeat && k < n {
            app(|a| a.held = Some(vk.0));
            activate(h, k);
        }
        return true;
    }
    match vk {
        VK_RETURN => {
            if let (false, Some(k)) = (repeat, cur) {
                app(|a| a.held = Some(VK_RETURN.0));
                activate(h, k);
            }
        }
        VK_F2 => {
            if let (false, Some(k)) = (repeat, card) {
                start_rename(h, k);
            }
        }
        VK_DELETE => {
            if let (false, Some(k)) = (repeat, card) {
                delete_card(h, k);
            }
        }
        _ => {
            app(|a| a.load_hot = load_step(cur, n, vk));
            _ = InvalidateRect(Some(h), None, false);
        }
    }
    true
}

pub(crate) extern "system" fn load_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let g = app(|a| load_geo(a.cards.len(), a.dpi));
        match msg {
            WM_PAINT => paint_load(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                // one comes when the popup opens under the pointer, and after it resizes: only a real move
                // takes the pick from the keys (a mouse-opened popup still follows the pointer as before)
                let mut at = POINT::default();
                _ = GetCursorPos(&mut at);
                let (still, keys) = app(|a| (std::mem::replace(&mut a.load_cursor, (at.x, at.y)) == (at.x, at.y), a.load_keys));
                if still && keys {
                    return LRESULT(0);
                }
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
                if let Some(k) = load_hit(&g, x, y) {
                    activate(h, k);
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
                _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
                _ = AppendMenuW(menu, MF_STRING, 3, &HSTRING::from(tr("Create desktop shortcut", "바탕 화면에 바로 가기 만들기")));
                let mut at = POINT::default();
                _ = GetCursorPos(&mut at);
                let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, at.x, at.y, None, h, None).0;
                _ = DestroyMenu(menu);
                match pick {
                    1 => start_rename(h, k),
                    2 => delete_card(h, k),
                    3 => shortcut_card(k),
                    _ => {}
                }
            }
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_COMMAND if ((wp.0 >> 16) & 0xffff) as u32 == EN_KILLFOCUS => _ = end_rename(),
            WM_KEYDOWN if load_key(h, VIRTUAL_KEY(wp.0 as u16), lp.0 & (1 << 30) != 0) => {}
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
