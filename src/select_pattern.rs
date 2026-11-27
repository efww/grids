use crate::*;

// ---------- select by pattern (Num+ / Num− / Num*, like Total Commander's Select group) ----------

// borderless edits sit on the field colour
pub(crate) unsafe fn edit_colors(wp: WPARAM) -> LRESULT {
    let dc = HDC(wp.0 as _);
    SetBkColor(dc, cref(pal().pill));
    SetTextColor(dc, cref(pal().text));
    SetDCBrushColor(dc, cref(pal().pill));
    LRESULT(GetStockObject(DC_BRUSH).0 as isize)
}

// A key that opened a popup or ran something swallows its own auto-repeats until released, so holding it
// neither types into the popup nor reaches the shell. A fresh press of it clears the hold (keyup missed,
// e.g. after Alt+Tab) and goes through. (swallow, next hold)
pub(crate) fn held_key(held: Option<u16>, vk: u16, repeat: bool) -> (bool, Option<u16>) {
    match held {
        Some(k) if k == vk => (repeat, held.filter(|_| repeat)),
        _ => (false, held),
    }
}

pub(crate) struct MatchGeo {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) label: RECT,
    pub(crate) field: RECT,
    pub(crate) edit: RECT,
}

// 320 DIP by default, wider when the label needs it: the field always keeps at least 120 DIP.
pub(crate) fn match_geo(label_w: i32, fh: i32, dpi: i32) -> MatchGeo {
    let s = |v: i32| v * dpi / 96;
    let h = s(32).max(fh + s(14));
    let label = RECT { left: s(12), top: 0, right: s(12) + label_w, bottom: h };
    let w = s(320).max(label.right + s(10) + s(120) + s(6));
    let field = RECT { left: label.right + s(10), top: s(5), right: w - s(6), bottom: h - s(5) };
    let top = (h - fh) / 2;
    let edit = RECT { left: field.left + s(8), top, right: field.right - s(8), bottom: top + fh };
    MatchGeo { w, h, label, field, edit }
}

pub(crate) fn match_label(desel: bool) -> &'static str {
    if desel {
        tr("Deselect", "선택 해제")
    } else {
        tr("Select", "선택")
    }
}

pub(crate) unsafe fn match_geo_now(desel: bool) -> MatchGeo {
    let (font, fh, dpi) = app(|a| (a.font, a.fh, a.dpi));
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &match_label(desel).encode_utf16().collect::<Vec<_>>(), &mut sz);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    match_geo(sz.cx, fh, dpi)
}

// Opens over the top-left of pane i's file list, kept inside the monitor's work area.
pub(crate) unsafe fn open_match(i: usize, desel: bool) {
    let (main, font, open, last, path) = app(|a| (a.main, a.font, a.matcher.is_some(), a.match_last.clone(), a.s.paths[i].clone()));
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    if open {
        return;
    }
    let g = match_geo_now(desel);
    let mut rc = RECT::default();
    _ = GetWindowRect(view_list(&fv), &mut rc);
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let work = mi.rcWork;
    let x = rc.left.min(work.right - g.w).max(work.left);
    let y = rc.top.min(work.bottom - g.h).max(work.top);
    let style = WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsMatch"), w!(""), style, x, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    let edit = child(h, w!("EDIT"), PCWSTR(HSTRING::from(last).as_ptr()),ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), 1, font);
    let e = g.edit;
    _ = MoveWindow(edit, e.left, e.top, e.right - e.left, e.bottom - e.top, false);
    app(|a| a.matcher = Some(Matcher { h, edit, pane: i, path, desel }));
    _ = SetFocus(Some(edit));
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
}

pub(crate) unsafe fn close_match(back: bool) {
    let Some((h, i)) = app(|a| a.matcher.as_ref().map(|m| (m.h, m.pane))) else { return };
    _ = DestroyWindow(h);
    if back {
        focus_view(i);
    }
}

pub(crate) unsafe fn set_note(note: String) {
    // the status line keeps a note until this time; a past time would drop it on the next redraw
    app(|x| x.note = Some((note, Instant::now() + Duration::from_secs(5))));
    update_status();
}

// A note that keeps updating in place (checksum progress): shown like set_note, but not read out at every step.
pub(crate) unsafe fn set_progress_note(note: String) {
    app(|x| {
        let until = Instant::now() + Duration::from_secs(5);
        x.note = Some((note, until));
        x.said = Some(until);
    });
    update_status();
}

// Why something the user just tried didn't work, said in the status line (warn) next to the beep.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Fail {
    AddrNotFound,
    AddrQuotes,
    AddrCantOpen,
    FolderCantOpen,
    LastTab,
    DriveCantOpen,
    SortHere,
    PaneMin,
    NoBorder,
    GroupEmpty,
    GroupTooMany,
    GroupCant,
    GroupBusy,
    NewFileHere,
    NewFolder,
    SendChanged,
    SendCant,
    PatternEmpty,
    StampEmpty,
    StampTooMany,
    StampCant,
    StampBusy,
    EditNone,
    EditOnline,
    DragCant,
    ClipNoPath,
    NoClosedTab,
    DlgBusy,
    ExplorerCant,
}

#[cfg(test)] // every kind, for the fail_texts test
pub(crate) const FAILS: [Fail; 29] = {
    use Fail::*;
    [
        AddrNotFound, AddrQuotes, AddrCantOpen, FolderCantOpen, LastTab, DriveCantOpen, SortHere, PaneMin, NoBorder, GroupEmpty, GroupTooMany,
        GroupCant, GroupBusy, NewFileHere, NewFolder, SendChanged, SendCant, PatternEmpty, StampEmpty, StampTooMany, StampCant, StampBusy,
        EditNone, EditOnline, DragCant, ClipNoPath, NoClosedTab, DlgBusy, ExplorerCant,
    ]
};

// (English, Korean), one line each; {name} is the path, name or drive the text is about.
pub(crate) fn fail_pair(kind: Fail) -> (&'static str, &'static str) {
    match kind {
        Fail::AddrNotFound => ("No folder, file or command named '{name}' · check the spelling", "'{name}' 폴더·파일·명령을 찾을 수 없음 · 철자를 확인하세요"),
        Fail::AddrQuotes => ("The quotes in the address don't pair up", "주소의 따옴표 짝이 맞지 않음"),
        Fail::AddrCantOpen => ("Can't open '{name}'", "'{name}'을(를) 열 수 없음"),
        Fail::FolderCantOpen => ("Can't open {name} · it may be gone or not connected", "{name}을(를) 열 수 없음 · 없어졌거나 연결되지 않았을 수 있음"),
        Fail::LastTab => ("The last tab can't be closed", "마지막 탭은 닫을 수 없음"),
        Fail::NoClosedTab => ("No closed tab to reopen", "다시 열 닫은 탭이 없음"),
        Fail::DriveCantOpen => ("Drive {name} has no disc or can't be opened", "{name} 드라이브에 디스크가 없거나 열 수 없음"),
        Fail::SortHere => ("This sort isn't available here", "여기서는 이 정렬을 쓸 수 없음"),
        Fail::PaneMin => ("The pane on that side is already at its smallest", "그쪽 패널이 이미 최소 크기임"),
        Fail::NoBorder => ("No pane border to move that way", "그 방향으로는 옮길 패널 경계가 없음"),
        Fail::GroupEmpty => ("Select the items to group first", "묶을 항목을 먼저 고르세요"),
        Fail::GroupTooMany => ("Too many items to group (over 10,000)", "묶을 항목이 너무 많음 (1만 개 초과)"),
        Fail::GroupCant => ("These items can't be grouped into a folder here", "이 항목들은 여기서 폴더로 묶을 수 없음"),
        Fail::GroupBusy => ("Already grouping · wait until it's done", "이미 묶는 중 · 끝날 때까지 기다리세요"),
        Fail::NewFileHere => ("Can't make a file in this place", "이 위치에는 파일을 만들 수 없음"),
        Fail::NewFolder => ("Couldn't make a new folder", "새 폴더를 만들지 못함"),
        Fail::SendChanged => ("Not sent · that pane now shows another folder", "보내지 않음 · 그 패널이 다른 폴더로 바뀌었음"),
        Fail::SendCant => ("Can't send there", "그곳으로는 보낼 수 없음"),
        Fail::PatternEmpty => ("Type a pattern first, e.g. *.jpg", "패턴을 먼저 입력하세요 (예: *.jpg)"),
        Fail::StampEmpty => ("Select the items to make dated copies of first", "날짜 붙인 사본을 만들 항목을 먼저 고르세요"),
        Fail::StampTooMany => ("Too many items to copy (over 10,000)", "사본을 만들 항목이 너무 많음 (1만 개 초과)"),
        Fail::StampCant => ("Dated copies can't be made of these items here", "이 항목들은 여기서 날짜 붙인 사본을 만들 수 없음"),
        Fail::StampBusy => ("Already making dated copies · wait until it's done", "이미 사본을 만드는 중 · 끝날 때까지 기다리세요"),
        Fail::EditNone => ("No text file to edit · folders and binary files (programs, archives …) aren't opened", "편집할 글자 파일이 없음 · 폴더와 이진 파일(프로그램·압축 파일 등)은 열지 않음"),
        Fail::EditOnline => ("Online-only files are skipped · keep them on this device first", "온라인 전용 파일은 건너뜀 · 먼저 이 장치에 내려받으세요"),
        Fail::DragCant => ("This place can't be dragged out · drag a folder from the file list instead", "이 위치는 끌어 낼 수 없음 · 파일 목록에서 폴더를 끌어 보세요"),
        Fail::ClipNoPath => ("No folder or file path on the clipboard · copy a path, then Ctrl+Shift+V", "클립보드에 폴더·파일 경로가 없음 · 경로를 복사한 뒤 Ctrl+Shift+V"),
        Fail::DlgBusy => ("The dialog isn't responding · try again in a moment", "대화 상자가 응답하지 않음 · 잠시 뒤 다시 해 보세요"),
        Fail::ExplorerCant => ("Couldn't open {name} in Explorer · it may be gone or not connected", "{name}을(를) 탐색기에서 열지 못함 · 없어졌거나 연결되지 않았을 수 있음"),
    }
}

// The text for `kind` with `arg` (cut to 80 characters, by char: never inside a Korean letter) in place of {name}.
pub(crate) fn fail_text(kind: Fail, arg: &str, ko: bool) -> String {
    let (en, k) = fail_pair(kind);
    let arg: String = arg.split(['\r', '\n']).next().unwrap_or_default().trim().to_string();
    let arg = if arg.chars().count() > 80 { arg.chars().take(79).chain(['…']).collect() } else { arg };
    if ko { k } else { en }.replace("{name}", &arg)
}

// A beep with its reason in the status line (read out like any note). Not from inside app(): set_note borrows it.
pub(crate) unsafe fn warn(text: String) {
    _ = MessageBeep(MB_ICONWARNING);
    app(|a| a.lost = 0); // a "Not found" note right after starts its own count
    set_note(text);
}

pub(crate) unsafe fn fail(kind: Fail, arg: &str) {
    warn(fail_text(kind, arg, KO.load(Ordering::Relaxed)));
}

// Calls `each` for every item in the view with redraw off; true = that item changed. None: over SEND_MAX,
// nothing touched (and the status line says so).
pub(crate) unsafe fn bulk_select(fv: &IFolderView2, mut each: impl FnMut(i32) -> bool) -> Option<usize> {
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0).max(0);
    if n as u32 > SEND_MAX {
        set_note(tr("Too many items to match (over 10,000)", "항목이 너무 많아 선택하지 않음 (1만 개 초과)").into());
        return None;
    }
    _ = fv.SetRedraw(false);
    let done = (0..n).filter(|&k| each(k)).count();
    _ = fv.SetRedraw(true);
    Some(done)
}

// Enter in the box (Shift+Enter: the other mode). Files and ZIPs whose name matches; folders are left alone.
pub(crate) unsafe fn run_match(flip: bool) {
    let Some((h, edit, i, path, desel)) = app(|a| a.matcher.as_ref().map(|m| (m.h, m.edit, m.pane, m.path.clone(), m.desel))) else { return };
    let desel = desel != flip;
    let fv = view_of::<IFolderView2>(i).filter(|_| app(|a| i < a.s.panes() && a.s.paths[i] == path));
    let Some(fv) = fv else {
        _ = MessageBeep(MB_ICONWARNING); // pane gone or moved on since the box opened
        _ = DestroyWindow(h);
        focus_view(app(|a| a.s.active));
        return;
    };
    let text = text_of(edit).trim().to_string();
    let spec = filter_spec(&text);
    if spec.is_empty() {
        fail(Fail::PatternEmpty, "");
        return;
    }
    app(|a| a.match_last = text.clone());
    let spec = HSTRING::from(spec);
    let mut first = !desel;
    let n = bulk_select(&fv, |k| {
        let Ok(it) = fv.GetItem::<IShellItem>(k) else { return false };
        let attr = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0)).unwrap_or_default().0;
        if real_folder(attr) {
            return false;
        }
        let Ok(name) = it.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).map(|p| pwstr(p)) else { return false };
        if !PathMatchSpecW(&HSTRING::from(name), &spec).as_bool() {
            return false;
        }
        let flags = match (desel, first) {
            (true, _) => SVSI_DESELECT.0,
            (false, true) => SVSI_SELECT.0 | SVSI_FOCUSED.0 | SVSI_ENSUREVISIBLE.0,
            (false, false) => SVSI_SELECT.0,
        };
        let ok = fv.SelectItem(k, flags as u32).is_ok();
        first &= !ok;
        ok
    });
    let Some(n) = n else { return };
    let ko = KO.load(Ordering::Relaxed);
    if n == 0 {
        _ = MessageBeep(MB_ICONWARNING);
        set_note(if ko { format!("맞는 파일 없음: {text}") } else { format!("No match: {text}") });
        return;
    }
    _ = DestroyWindow(h);
    focus_view(i);
    set_note(match (ko, desel) {
        (true, false) => format!("{text} {n}개 선택"),
        (true, true) => format!("{text} {n}개 선택 해제"),
        (false, false) => format!("Selected {n} · {text}"),
        (false, true) => format!("Deselected {n} · {text}"),
    });
}

// Num*: every visible item, folders too, flips its selection.
pub(crate) unsafe fn invert_selection(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let n = bulk_select(&fv, |k| {
        let Ok(pidl) = fv.Item(k) else { return false };
        let state = fv.GetSelectionState(pidl);
        CoTaskMemFree(Some(pidl as _));
        let Ok(state) = state else { return false };
        let flag = if state & SVSI_SELECT.0 as u32 != 0 { SVSI_DESELECT } else { SVSI_SELECT };
        fv.SelectItem(k, flag.0 as u32).is_ok()
    });
    if let Some(n) = n {
        set_note(if KO.load(Ordering::Relaxed) { format!("선택 반전 ({n}개)") } else { format!("Selection inverted ({n})") });
    }
}

// ---------- Num/: the earlier selection back (Total Commander's / Directory Opus's "Restore selection") ----------

pub(crate) const ID_RESTORE: usize = 1158; // ⇄ menu (1155–1157 are ID_PHOTO's)
const PREV_PER_DIR: usize = 2; // two per folder: the one just lost and the one before, so Num/ goes back and forth
const PREV_DIRS: usize = 16;
const PREV_PATHS: usize = 20_000; // in all folders; the oldest selections go first

// The same items, in any order and case (the view hands them back in its own order).
pub(crate) fn same_sel(a: &[String], b: &[String]) -> bool {
    if a == b {
        return true;
    }
    if a.len() != b.len() {
        return false;
    }
    let set: std::collections::HashSet<String> = a.iter().map(|p| p.to_lowercase()).collect();
    b.iter().all(|p| set.contains(&p.to_lowercase()))
}

// Every item of `a` is also in `b` (any order and case): a selection being grown by Ctrl+click or Shift+arrow.
pub(crate) fn sub_sel(a: &[String], b: &[String]) -> bool {
    if a.len() > b.len() {
        return false;
    }
    let set: std::collections::HashSet<String> = b.iter().map(|p| p.to_lowercase()).collect();
    a.iter().all(|p| set.contains(&p.to_lowercase()))
}

fn same_dir(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.to_lowercase() == b.to_lowercase()
}

// A selection the status line lets go of (deselected, replaced, its pane or folder left) is kept, newest first:
// not one item (the click that entered a folder must not push out a real choice), not a partial one (over 10k items,
// items with no path), not the same items as `now` (the selection taking its place), each only once per folder.
// Two per folder, 16 folders, 20k paths in all.
pub(crate) fn remember_sel(mem: &mut Vec<(String, Vec<String>)>, dir: &str, s: Sel, now: &[String]) {
    if s.paths.len() < 2 || s.partial || same_sel(&s.paths, now) {
        return;
    }
    mem.retain(|(d, p)| !(same_dir(d, dir) && same_sel(p, &s.paths)));
    mem.insert(0, (dir.to_string(), s.paths));
    let mut dirs: Vec<String> = Vec::new();
    let mut per = Vec::new(); // how many of each kept folder's
    mem.retain(|(d, _)| match dirs.iter().position(|x| same_dir(x, d)) {
        Some(k) => {
            per[k] += 1;
            per[k] <= PREV_PER_DIR
        }
        None if dirs.len() < PREV_DIRS => {
            dirs.push(d.clone());
            per.push(1);
            true
        }
        None => false,
    });
    let mut total: usize = mem.iter().map(|(_, p)| p.len()).sum();
    while total > PREV_PATHS && mem.len() > 1 {
        total -= mem.pop().map_or(0, |(_, p)| p.len());
    }
}

// The selection Num/ brings back in `dir`: the newest kept there that isn't what is selected now.
pub(crate) fn prev_sel<'a>(mem: &'a [(String, Vec<String>)], dir: &str, now: &[String]) -> Option<&'a Vec<String>> {
    mem.iter().find(|(d, p)| same_dir(d, dir) && !same_sel(p, now)).map(|(_, p)| p)
}

// The status line after Num/: `got` of the `want` kept items were found in the folder.
pub(crate) fn restore_note(ko: bool, got: usize, want: usize) -> String {
    match (ko, got == want) {
        (false, true) => format!("Restored the earlier selection ({got})"),
        (true, true) => format!("이전 선택 {got}개 되살림"),
        (false, false) => format!("Restored {got} of {want} ({} not here)", want - got),
        (true, false) => format!("이전 선택 {want}개 중 {got}개 되살림 ({}개 없음)", want - got),
    }
}

// The ⇄ menu's item, only while this folder keeps a selection other than the one now (`picked`).
// Not when the selection now can't be read whole (over 10k items, items with no path): it might be the kept one.
pub(crate) fn restore_menu(menu: HMENU, src: usize, picked: &std::result::Result<Vec<String>, Pick>) {
    let Ok(now) = picked else { return };
    let n = app(|a| prev_sel(&a.sel_prev, &a.s.paths[src], now).map(|p| p.len()));
    if let Some(n) = n {
        let label = tr2(M_RESTORE).replace("{n}", &n.to_string());
        _ = unsafe { AppendMenuW(menu, MF_STRING, ID_RESTORE, &HSTRING::from(label)) };
    }
}

// Num/ (or the ⇄ menu's T) in pane i: the kept selection's items still in the view are selected, the rest not,
// the first one focused and shown. What was selected before (2 or more) is kept in turn, so Num/ again goes back.
// Nothing kept or none of it here: a beep, and the selection stays as it is.
pub(crate) unsafe fn restore_selection(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let dir = app(|a| a.s.paths[i].clone());
    let now = selected_size(&fv);
    let none = || warn(tr("No earlier selection in this folder", "이 폴더에 되살릴 이전 선택 없음").into());
    let Some(want) = app(|a| prev_sel(&a.sel_prev, &dir, &now.paths).cloned()) else { return none() };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0).max(0);
    if n as u32 > SEND_MAX {
        _ = bulk_select(&fv, |_| false); // touches nothing: only says why
        return;
    }
    let keys: std::collections::HashSet<String> = want.iter().map(|p| p.to_lowercase()).collect();
    let path = |k: i32| Some(pwstr(fv.GetItem::<IShellItem>(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?).to_lowercase());
    // first which items are here, so a folder where none is left keeps its selection
    let hits: Vec<i32> = (0..n).filter(|&k| path(k).is_some_and(|p| keys.contains(&p))).collect();
    let Some(&first) = hits.first() else { return none() };
    app(|a| remember_sel(&mut a.sel_prev, &dir, now, &want));
    // the first one clears the rest (SVSI_DESELECTOTHERS), so nothing is deselected one by one
    _ = bulk_select(&fv, |k| {
        let flags = if k == first {
            SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0 | SVSI_ENSUREVISIBLE.0
        } else if hits.binary_search(&k).is_ok() {
            SVSI_SELECT.0
        } else {
            return false;
        };
        fv.SelectItem(k, flags as u32).is_ok()
    });
    set_note(restore_note(KO.load(Ordering::Relaxed), hits.len(), want.len()));
}

// Insert, as in Total Commander: (the focused item's flags, the next item's). The focused item flips and keeps
// the focus (so the last item stays focused); the next one takes the focus without losing its own selection,
// since SelectItem without SVSI_SELECT deselects (SVSI_DESELECT is 0).
pub(crate) fn ins_flags(cur_sel: bool, next_sel: bool) -> (u32, u32) {
    let flip = if cur_sel { SVSI_DESELECT.0 } else { SVSI_SELECT.0 };
    let keep = if next_sel { SVSI_SELECT.0 } else { SVSI_DESELECT.0 };
    ((flip | SVSI_FOCUSED.0) as u32, (keep | SVSI_FOCUSED.0 | SVSI_ENSUREVISIBLE.0) as u32)
}

pub(crate) unsafe fn is_selected(fv: &IFolderView2, k: i32) -> Option<bool> {
    let pidl = fv.Item(k).ok()?;
    let state = fv.GetSelectionState(pidl);
    CoTaskMemFree(Some(pidl as _));
    state.ok().map(|s| s & SVSI_SELECT.0 as u32 != 0)
}

// Insert: the focused item's selection flips and the focus moves down to the next visible item (in view order).
// No focused item yet: the first one takes the focus, nothing is selected.
pub(crate) unsafe fn mark_and_next(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let visible_after = |k: i32| {
        let mut next = -1;
        let hr = (Interface::vtable(&fv).GetVisibleItem)(Interface::as_raw(&fv), k, false.into(), &mut next);
        (hr == S_OK && (0..n).contains(&next)).then_some(next)
    };
    let cur = fv.GetFocusedItem().unwrap_or(-1);
    if !(0..n).contains(&cur) {
        let Some(first) = visible_after(-1).or((n > 0).then_some(0)) else { return };
        let Some(sel) = is_selected(&fv, first) else { return };
        _ = fv.SelectItem(first, ins_flags(false, sel).1);
        return;
    }
    let Some(cur_sel) = is_selected(&fv, cur) else { return };
    let next = visible_after(cur).and_then(|k| is_selected(&fv, k).map(|s| (k, s)));
    let (cur_flags, next_flags) = ins_flags(cur_sel, next.is_some_and(|(_, s)| s));
    if fv.SelectItem(cur, cur_flags).is_err() {
        return;
    }
    if let Some((k, _)) = next {
        _ = fv.SelectItem(k, next_flags);
    }
}

pub(crate) unsafe fn paint_match(h: HWND) {
    let desel = app(|a| a.matcher.as_ref().is_some_and(|m| m.desel));
    let g = match_geo_now(desel);
    let font = app(|a| a.font);
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        text(dc, g.label, match_label(desel), font, pal().sub, DT_LEFT);
        fill(dc, g.field, pal().pill);
    });
}

pub(crate) extern "system" fn match_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => paint_match(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            // the hold stays: a held Enter must not open files once the list has focus again
            WM_DESTROY => app(|a| a.matcher = None),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(names: &[&str]) -> Sel {
        Sel { paths: names.iter().map(|n| format!(r"C:\a\{n}")).collect(), partial: false }
    }

    fn paths(names: &[&str]) -> Vec<String> {
        sel(names).paths
    }

    #[test]
    fn remember_keeps_real_choices() {
        let mut m = Vec::new();
        // one item (the click into a folder), a partial selection, the same items as now: not kept
        remember_sel(&mut m, r"C:\a", sel(&["x"]), &[]);
        remember_sel(&mut m, r"C:\a", Sel { partial: true, ..sel(&["x", "y"]) }, &[]);
        remember_sel(&mut m, r"C:\a", sel(&["x", "y"]), &paths(&["Y", "x"]).iter().map(|p| p.to_uppercase()).collect::<Vec<_>>());
        assert!(m.is_empty());
        remember_sel(&mut m, r"C:\a", sel(&["x", "y"]), &paths(&["x"]));
        remember_sel(&mut m, r"C:\a", sel(&["p", "q", "r"]), &[]);
        // the same items again move to the front instead of taking a second place
        remember_sel(&mut m, r"c:\A", sel(&["y", "x"]), &[]);
        assert_eq!(m.iter().map(|(_, p)| p.len()).collect::<Vec<_>>(), [2, 3]);
        // a third in that folder pushes out its oldest
        remember_sel(&mut m, r"C:\a", sel(&["s", "t"]), &[]);
        assert_eq!(m, [(r"C:\a".to_string(), paths(&["s", "t"])), (r"c:\A".to_string(), paths(&["y", "x"]))]);
    }

    #[test]
    fn remember_bounds() {
        let mut m = Vec::new();
        for k in 0..20 {
            remember_sel(&mut m, &format!(r"C:\d{k}"), sel(&["x", "y"]), &[]);
        }
        // 16 folders, the ones used longest ago gone
        assert_eq!(m.len(), 16);
        assert_eq!((m[0].0.as_str(), m[15].0.as_str()), (r"C:\d19", r"C:\d4"));
        // 20k paths in all: the oldest selections go, the newest always stays
        let big = |n: usize| Sel { paths: (0..n).map(|k| format!(r"C:\b\{k}")).collect(), partial: false };
        remember_sel(&mut m, r"C:\b", big(10_000), &[]);
        remember_sel(&mut m, r"C:\c", big(9_990), &[]);
        assert!(m.iter().map(|(_, p)| p.len()).sum::<usize>() <= 20_000);
        assert_eq!((m[0].0.as_str(), m[1].0.as_str(), m.len()), (r"C:\c", r"C:\b", 7));
    }

    #[test]
    fn growing_is_a_subset() {
        // Ctrl+click / Shift+arrow adding items: the step before is not kept
        assert!(sub_sel(&paths(&["x", "y"]), &paths(&["Y", "z", "x"]).iter().map(|p| p.to_uppercase()).collect::<Vec<_>>()));
        assert!(sub_sel(&paths(&["x", "y"]), &paths(&["y", "x"])));
        // one taken away, or another choice: kept
        assert!(!sub_sel(&paths(&["x", "y", "z"]), &paths(&["x", "y"])));
        assert!(!sub_sel(&paths(&["x", "y"]), &paths(&["x", "z", "w"])));
    }

    #[test]
    fn restore_picks_another() {
        let mut m = Vec::new();
        remember_sel(&mut m, r"C:\a", sel(&["x", "y"]), &[]);
        remember_sel(&mut m, r"C:\a", sel(&["p", "q", "r"]), &[]);
        remember_sel(&mut m, r"C:\b", sel(&["z", "w"]), &[]);
        assert_eq!(prev_sel(&m, r"C:\a", &[]), Some(&paths(&["p", "q", "r"])));
        // what is selected now is skipped: Num/ again goes back and forth
        assert_eq!(prev_sel(&m, r"c:\a", &paths(&["r", "q", "p"])), Some(&paths(&["x", "y"])));
        assert_eq!(prev_sel(&m, r"C:\c", &[]), None);
        assert_eq!(prev_sel(&m, r"C:\b", &paths(&["w", "z"])), None);
    }

    #[test]
    fn restore_notes() {
        assert_eq!(restore_note(false, 30, 30), "Restored the earlier selection (30)");
        assert_eq!(restore_note(false, 28, 30), "Restored 28 of 30 (2 not here)");
        assert_eq!(restore_note(true, 30, 30), "이전 선택 30개 되살림");
        assert_eq!(restore_note(true, 28, 30), "이전 선택 30개 중 28개 되살림 (2개 없음)");
    }
}
