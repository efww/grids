use crate::*;
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
use windows::Win32::UI::Controls::EM_SETCUEBANNER;

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
    // Num+ and Num− sound different to a screen reader (taken off in WM_DESTROY, however the popup goes)
    a11y_name(edit, Some(if desel { tr("Deselect by pattern", "패턴으로 선택 해제") } else { tr("Select by pattern", "패턴으로 선택") }));
    a11y_desc(edit, Some(tr("*.jpg;*.png, part of a name, >10MB or <7d · Enter apply · Shift+Enter the other way · Esc close", "*.jpg;*.png, 이름 일부, >10MB 또는 <7d · Enter 적용 · Shift+Enter 반대로 · Esc 닫기")));
    let cue = HSTRING::from(tr("e.g. *.jpg; >5MB; <7d", "예: *.jpg; >5MB; <7d"));
    SendMessageW(edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
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
    AddrElevateCancelled,
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
    PatternCond,
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
    SessionCantSave,
    SessionKept,
}

#[cfg(test)] // every kind, for the fail_texts test
pub(crate) const FAILS: [Fail; 33] = {
    use Fail::*;
    [
        AddrNotFound, AddrQuotes, AddrCantOpen, FolderCantOpen, LastTab, DriveCantOpen, SortHere, PaneMin, NoBorder, GroupEmpty, GroupTooMany,
        GroupCant, GroupBusy, NewFileHere, NewFolder, SendChanged, SendCant, PatternEmpty, StampEmpty, StampTooMany, StampCant, StampBusy,
        EditNone, EditOnline, DragCant, ClipNoPath, NoClosedTab, DlgBusy, ExplorerCant, PatternCond, SessionCantSave,
        SessionKept, AddrElevateCancelled,
    ]
};

// (English, Korean), one line each; {name} is the path, name or drive the text is about.
pub(crate) fn fail_pair(kind: Fail) -> (&'static str, &'static str) {
    match kind {
        Fail::AddrNotFound => ("No folder, file or command named '{name}' · check the spelling", "'{name}' 폴더·파일·명령을 찾을 수 없음 · 철자를 확인하세요"),
        Fail::AddrQuotes => ("The quotes in the address don't pair up", "주소의 따옴표 짝이 맞지 않음"),
        Fail::AddrCantOpen => ("Can't open '{name}'", "'{name}'을(를) 열 수 없음"),
        Fail::AddrElevateCancelled => ("Not run: administrator permission was declined", "실행 안 함: 관리자 권한 요청을 거절함"),
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
        Fail::PatternCond => ("Can't read '{name}' · use >10MB, <500KB, <7d, >2w", "'{name}'을(를) 읽을 수 없음 · >10MB, <500KB, <7d, >2w 처럼"),
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
        Fail::SessionCantSave => ("Couldn't save the session '{name}'", "세션 '{name}'을(를) 저장하지 못함"),
        Fail::SessionKept => ("Couldn't save the session '{name}' · the saved file is unchanged", "세션 '{name}'을(를) 저장하지 못함 · 저장돼 있던 파일은 그대로입니다"),
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

// A size or age piece of the select box (XYplorer's size:/age: selection filter, Directory Opus' advanced select).
// Sizes in bytes; ages in FILETIME's 100 ns steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Cond {
    Bigger(u64),
    Smaller(u64),
    Older(u64),  // modified longer ago than this
    Newer(u64),  // modified within this
}

const HOUR: u64 = 3600 * 10_000_000;

// `t` starts with '<' or '>'. ">10MB", "<1.5gb", "<7d", "> 2 w": a number (digits and one '.') and a unit, B/KB/MB/GB/TB (1024 steps, as the
// status line) or h/d/w. No unit, a bare "m" (minutes? months? MB?), signs, "1e3", "inf": None, nothing guessed.
fn parse_cond(t: &str) -> Option<Cond> {
    let (op, rest) = t.split_at(1);
    let rest = rest.trim_start();
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
    let (num, unit) = rest.split_at(end);
    if !num.contains(|c: char| c.is_ascii_digit()) || num.matches('.').count() > 1 {
        return None;
    }
    let num: f64 = num.parse().ok()?;
    let k = 1024u64;
    let (mult, age) = match unit.trim().to_ascii_lowercase().as_str() {
        "b" => (1, false),
        "kb" => (k, false),
        "mb" => (k * k, false),
        "gb" => (k * k * k, false),
        "tb" => (k * k * k * k, false),
        "h" => (HOUR, true),
        "d" => (24 * HOUR, true),
        "w" => (7 * 24 * HOUR, true),
        _ => return None,
    };
    let v = (num * mult as f64).min(u64::MAX as f64) as u64; // `as` keeps it in u64 (and rounds down)
    Some(match (op == ">", age) {
        (true, false) => Cond::Bigger(v),
        (false, false) => Cond::Smaller(v),
        (true, true) => Cond::Older(v),
        (false, true) => Cond::Newer(v),
    })
}

// The select box's text: the name pattern (filter_spec of the pieces that aren't conditions) and the conditions,
// pieces starting with '<' or '>' (neither can be in a Windows file name, so no pattern reads differently).
// Err: the first condition that can't be read.
pub(crate) fn parse_match(text: &str) -> std::result::Result<(String, Vec<Cond>), String> {
    let (mut names, mut conds) = (Vec::new(), Vec::new());
    for part in text.split(';').map(str::trim) {
        if part.starts_with(['<', '>']) {
            conds.push(parse_cond(part).ok_or_else(|| part.to_string())?);
        } else {
            names.push(part);
        }
    }
    Ok((filter_spec(&names.join(";")), conds))
}

// One condition against a file's size and date modified (FILETIME, UTC like `now`); a value not read (or a zero
// date) never matches. A date after `now` counts as just modified: never "older than".
pub(crate) fn cond_ok(c: Cond, size: Option<u64>, time: Option<u64>, now: u64) -> bool {
    let age = time.filter(|&t| t != 0).map(|t| now.saturating_sub(t));
    match c {
        Cond::Bigger(b) => size.is_some_and(|s| s > b),
        Cond::Smaller(b) => size.is_some_and(|s| s < b),
        Cond::Older(d) => age.is_some_and(|a| a > d),
        Cond::Newer(d) => age.is_some_and(|a| a < d),
    }
}

// Enter in the box (Shift+Enter: the other mode). Files and ZIPs whose name matches (and every condition);
// folders are left alone.
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
    let (spec, conds) = match parse_match(&text) {
        Ok(v) => v,
        Err(bad) => return fail(Fail::PatternCond, &bad),
    };
    if spec.is_empty() && conds.is_empty() {
        fail(Fail::PatternEmpty, "");
        return;
    }
    app(|a| a.match_last = text.clone());
    // conditions only: every file; an empty spec would match nothing
    let spec = (!spec.is_empty()).then(|| HSTRING::from(spec));
    let need_size = conds.iter().any(|c| matches!(c, Cond::Bigger(_) | Cond::Smaller(_)));
    let need_time = conds.iter().any(|c| matches!(c, Cond::Older(_) | Cond::Newer(_)));
    let ft = GetSystemTimeAsFileTime();
    let now = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    let mut first = !desel;
    let n = bulk_select(&fv, |k| {
        let Ok(it) = fv.GetItem::<IShellItem>(k) else { return false };
        let attr = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0)).unwrap_or_default().0;
        if real_folder(attr) {
            return false;
        }
        if let Some(spec) = &spec {
            let Ok(name) = it.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).map(|p| pwstr(p)) else { return false };
            if !PathMatchSpecW(&HSTRING::from(name), spec).as_bool() {
                return false;
            }
        }
        // the properties the view already holds, read only when a condition asks (and the name matched)
        if !conds.is_empty() {
            use windows::Win32::Storage::EnhancedStorage::{PKEY_DateModified, PKEY_Size};
            let Ok(it2) = it.cast::<IShellItem2>() else { return false };
            let size = if need_size { it2.GetUInt64(&PKEY_Size).ok() } else { None };
            let time = if need_time { it2.GetFileTime(&PKEY_DateModified).ok().map(|t| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64) } else { None };
            if !conds.iter().all(|&c| cond_ok(c, size, time, now)) {
                return false;
            }
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

// ---------- Shift+F2: the files that differ from another pane's, selected on both sides ----------
// Total Commander's Mark > Compare Directories (Shift+F2), Directory Opus' and FreeCommander's Compare: files only
// on one side, or newer there, are selected in both panes, so Ctrl+M, C copies them over next.

pub(crate) const ID_DIFF_TO: usize = 601; // ⇄ menu: + pane index (601–616)
const DIFF_SLACK: u64 = 20_000_000; // 2 seconds in FILETIME's 100 ns steps: FAT keeps times to 2 seconds

// One file of a pane: its name on disk (extension and all), size and date modified (FILETIME); None: not read.
pub(crate) type DiffItem = (String, Option<u64>, Option<u64>);

#[derive(Debug, PartialEq)]
pub(crate) struct DiffMarks {
    pub(crate) a: Vec<bool>, // a's files to select
    pub(crate) b: Vec<bool>,
    pub(crate) same: usize, // files on both sides left alone
}

// Names match in any case. Only on one side: marked there. On both: the one more than 2 seconds newer is marked;
// times alike (or one not read) but sizes known and different: both. A date not read never counts as newer.
pub(crate) fn diff_marks(a: &[DiffItem], b: &[DiffItem]) -> DiffMarks {
    let index: std::collections::HashMap<String, usize> = b.iter().enumerate().map(|(j, (n, ..))| (n.to_lowercase(), j)).collect();
    let mut m = DiffMarks { a: vec![false; a.len()], b: vec![true; b.len()], same: 0 };
    for (i, (name, size, time)) in a.iter().enumerate() {
        let Some(&j) = index.get(&name.to_lowercase()) else {
            m.a[i] = true;
            continue;
        };
        m.b[j] = false;
        let (_, bsize, btime) = &b[j];
        match (time, btime) {
            (Some(x), Some(y)) if x.abs_diff(*y) > DIFF_SLACK => {
                if x > y {
                    m.a[i] = true;
                } else {
                    m.b[j] = true;
                }
            }
            _ if size.is_some() && bsize.is_some() && size != bsize => {
                m.a[i] = true;
                m.b[j] = true;
            }
            _ => m.same += 1,
        }
    }
    m
}

// The status line after a compare with pane `pane` (0-based): how many were selected on each side.
pub(crate) fn diff_note(ko: bool, here: usize, there: usize, pane: usize, same: usize, filtered: bool) -> String {
    let p = pane + 1;
    let note = match (ko, here + there == 0, same == 0) {
        (false, false, _) => format!("Selected the different files: {here} here · {there} in pane {p} ({same} the same)"),
        (true, false, _) => format!("다른 파일 선택: 여기 {here}개 · 패널 {p}에 {there}개 (같은 파일 {same}개)"),
        // only folders (or nothing) on both sides
        (false, true, true) => "No files to compare in either folder".to_string(),
        (true, true, true) => "두 폴더 모두 비교할 파일이 없습니다".to_string(),
        (false, true, false) => format!("The files in both folders are the same (by name, size and date modified, {same})"),
        (true, true, false) => format!("두 폴더의 파일이 같습니다 (이름·크기·수정한 날짜 기준, {same}개)"),
    };
    // a filter hides files from the compare: one hidden on one side shows up as "only on the other"
    match (filtered, ko) {
        (false, _) => note,
        (true, false) => note + " · filter on",
        (true, true) => note + " · 필터 적용 중",
    }
}

// Something to compare: both folders on disk, and not the same one.
pub(crate) fn diff_ok(here: &str, there: &str) -> bool {
    is_fs(here) && is_fs(there) && !crate::session::same_dir(here, there)
}

// The pane rows (pane, menu label, folder it showed) under ID_DIFF_TO, grey where there is nothing to compare.
pub(crate) unsafe fn diff_rows(menu: HMENU, here: &str, others: &[(usize, HSTRING, String)]) {
    for (k, label, path) in others {
        let flags = if diff_ok(here, path) { MF_STRING } else { MF_STRING | MF_GRAYED };
        _ = AppendMenuW(menu, flags, ID_DIFF_TO + k, label);
    }
}

// The ⇄ menu's submenu (letter I), before the WinMerge lookup so it is there without WinMerge too.
pub(crate) unsafe fn diff_submenu(menu: HMENU, src: usize, others: &[(usize, HSTRING, String)]) {
    let here = app(|a| a.s.paths[src].clone());
    let Ok(sub) = CreatePopupMenu() else { return };
    diff_rows(sub, &here, others);
    if others.is_empty() {
        // a grey filler row (the heading is grey, so it never shows) lets the status line tell what this is
        _ = AppendMenuW(sub, MF_STRING | MF_GRAYED, ID_DIFF_TO + src, &HSTRING::from(tr2(M_DIFF)));
    }
    let flags = if others.iter().any(|(_, _, p)| diff_ok(&here, p)) { MF_POPUP } else { MF_POPUP | MF_GRAYED };
    _ = AppendMenuW(menu, flags, sub.0 as usize, &HSTRING::from(tr2(M_DIFF)));
}

// The other live panes: (pane, shown name, folder, colour label), and this pane's folder.
unsafe fn diff_panes(src: usize) -> (String, Vec<(usize, String, String, u8)>) {
    app(|a| {
        let live = a.panes.iter().enumerate().take(a.s.panes()).filter(|(k, _)| *k != src);
        let rows = live.filter_map(|(k, p)| Some((k, p.as_ref()?.disp.clone(), a.s.paths[k].clone(), a.s.tints[k]))).collect();
        (a.s.paths[src].clone(), rows)
    })
}

// The view's files by index (folders and items with no name left out), from the properties it already holds.
pub(crate) unsafe fn diff_items(fv: &IFolderView2, n: i32) -> (Vec<i32>, Vec<DiffItem>) {
    use windows::Win32::Storage::EnhancedStorage::{PKEY_DateModified, PKEY_Size};
    let one = |k: i32| -> Option<DiffItem> {
        let it = fv.GetItem::<IShellItem>(k).ok()?;
        let attr = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0)).unwrap_or_default().0;
        if real_folder(attr) {
            return None;
        }
        // the parsing name: with extensions hidden, a.txt and a.doc are both "a" on display
        let name = pwstr(it.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).ok()?);
        let it2 = it.cast::<IShellItem2>().ok();
        let size = it2.as_ref().and_then(|i| i.GetUInt64(&PKEY_Size).ok());
        let time = it2.as_ref().and_then(|i| i.GetFileTime(&PKEY_DateModified).ok()).map(|t| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64);
        Some((name, size, time.filter(|&t| t != 0)))
    };
    (0..n).filter_map(|k| one(k).map(|d| (k, d))).unzip()
}

// `hits` (ascending) selected in pane i's view and nothing else; the first one focused and shown in that view
// (the keyboard stays where it is). No hits: its selection cleared.
pub(crate) unsafe fn diff_apply(i: usize, fv: &IFolderView2, hits: &[i32]) {
    let Some(&first) = hits.first() else {
        if let Some(sv) = view_of::<IShellView>(i) {
            _ = sv.SelectItem(std::ptr::null(), SVSI_DESELECTOTHERS.0 as u32);
        }
        return;
    };
    _ = bulk_select(fv, |k| {
        let flags = if k == first {
            SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_FOCUSED.0 | SVSI_ENSUREVISIBLE.0
        } else if hits.binary_search(&k).is_ok() {
            SVSI_SELECT.0
        } else {
            return false;
        };
        fv.SelectItem(k, flags as u32).is_ok()
    });
}

// Compares pane src (showing `here` when asked) with pane k (showing `there`): a beep if either moved on since.
// Over 10,000 items on either side: nothing touched on both. Otherwise both selections are replaced.
pub(crate) unsafe fn diff_select(src: usize, k: usize, here: &str, there: &str) {
    let now = app(|a| (src < a.s.panes() && k < a.s.panes()).then(|| (a.s.paths[src].clone(), a.s.paths[k].clone())));
    let views = (view_of::<IFolderView2>(src), view_of::<IFolderView2>(k));
    let (Some((h, t)), (Some(fa), Some(fb))) = (now, views) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if h != here || t != there || !diff_ok(&h, &t) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let count = |fv: &IFolderView2| fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0).max(0);
    let (na, nb) = (count(&fa), count(&fb));
    if na as u32 > SEND_MAX || nb as u32 > SEND_MAX {
        warn(tr("Too many items to compare (over 10,000)", "항목이 너무 많아 비교하지 않음 (1만 개 초과)").into());
        return;
    }
    let ((ia, a), (ib, b)) = (diff_items(&fa, na), diff_items(&fb, nb));
    let m = diff_marks(&a, &b);
    let hits = |idx: &[i32], marks: &[bool]| idx.iter().zip(marks).filter(|(_, &on)| on).map(|(&k, _)| k).collect::<Vec<i32>>();
    let (ha, hb) = (hits(&ia, &m.a), hits(&ib, &m.b));
    diff_apply(src, &fa, &ha);
    diff_apply(k, &fb, &hb);
    let filtered = !filter_of(src).is_empty() || !filter_of(k).is_empty();
    set_note(diff_note(KO.load(Ordering::Relaxed), ha.len(), hb.len(), k, m.same, filtered));
}

// A diff row picked (⇄ menu or Shift+F2's list): only the folder the menu showed for that pane.
pub(crate) unsafe fn diff_pick(src: usize, k: usize, here: &str, others: &[(usize, HSTRING, String)]) {
    match others.iter().find(|(i, ..)| *i == k) {
        Some((_, _, there)) => diff_select(src, k, here, there),
        None => _ = MessageBeep(MB_ICONWARNING),
    }
}

thread_local! {
    // Shift+F2 with several panes to compare: the pane it was pressed in; its list opens once F2 and Shift are up
    pub(crate) static DIFF_KEY: Cell<Option<usize>> = const { Cell::new(None) };
}

// Shift+F2 in pane src's list: with one other pane to compare, at once; with more, their list when the keys are
// let go (a Shift still held would type the pane digit as '!', '@' …); with none, why not.
pub(crate) unsafe fn diff_key(src: usize) {
    let (here, rows) = diff_panes(src);
    let ok: Vec<&(usize, String, String, u8)> = rows.iter().filter(|(_, _, p, _)| diff_ok(&here, p)).collect();
    match ok.as_slice() {
        [] if !is_fs(&here) => warn(tr("Compare: this pane doesn't show a folder on disk", "비교: 이 패널은 디스크의 폴더가 아님").into()),
        [] => warn(tr("Compare: no other pane shows another folder on disk", "비교: 다른 폴더를 보여 주는 패널이 없음").into()),
        [(k, _, there, _)] => diff_select(src, *k, &here, there),
        _ => DIFF_KEY.set(Some(src)),
    }
}

// Shift+F2's pane list, under the pane's ⇄ button as Ctrl+M's menu opens.
pub(crate) unsafe fn diff_menu(h: HWND, src: usize) {
    let (here, rows) = diff_panes(src);
    let label = |k: usize, t: u8, disp: &str| HSTRING::from(pane_label(tr("Pane", "패널"), k, if t == 0 { "" } else { tint_name(t) }, disp));
    let others: Vec<(usize, HSTRING, String)> = rows.iter().map(|(k, disp, path, t)| (*k, label(*k, *t, disp), path.clone())).collect();
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Compare with (select the files only there or newer):", "비교할 패널 (한쪽에만 있거나 더 새 파일 선택):")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    diff_rows(menu, &here, &others);
    let mut chips = MenuSwatches::new(app(|a| a.dpi)); // dropped after DestroyMenu
    for (k, _, _, t) in &rows {
        if let Some(c) = tint_color(*t) {
            chips.put(menu, ID_DIFF_TO + k, Some(c));
        }
    }
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(src, ID_OPEN_IN) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0 as usize;
    // picked with Enter: its auto-repeats would open a file in the list
    if GetKeyState(VK_RETURN.0 as i32) < 0 {
        app(|a| a.held = Some(VK_RETURN.0));
    }
    _ = DestroyMenu(menu);
    match open_in_pick(pick) {
        OpenPick::Nothing => {}
        OpenPick::Diff(k) => diff_pick(src, k, &here, &others),
        _ => _ = MessageBeep(MB_ICONWARNING),
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
            WM_DESTROY => {
                // the box's name comes off out of app() (the reader may call back in), so a reused HWND value
                // doesn't inherit it
                if let Some(edit) = app(|a| a.matcher.as_ref().map(|m| m.edit)) {
                    a11y_clear(edit);
                }
                app(|a| a.matcher = None);
            }
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
    fn match_conditions() {
        use Cond::*;
        const DAY: u64 = 24 * HOUR;
        // 1024 steps, any case, a fraction, spaces around the number
        assert_eq!(parse_match(">1KB"), Ok((String::new(), vec![Bigger(1024)])));
        assert_eq!(parse_match("<1.5gb"), Ok((String::new(), vec![Smaller(1024 * 1024 * 1024 * 3 / 2)])));
        assert_eq!(parse_match("> 10 Mb ; <2B; >1tb"), Ok((String::new(), vec![Bigger(10 << 20), Smaller(2), Bigger(1 << 40)])));
        assert_eq!(parse_match("<7d;>2W;>12h;<.5d"), Ok((String::new(), vec![Newer(7 * DAY), Older(14 * DAY), Older(12 * HOUR), Newer(DAY / 2)])));
        // names and conditions mixed: the names make the same spec as before
        assert_eq!(parse_match("*.mp4; >500MB; holiday"), Ok(("*.mp4;*holiday*".to_string(), vec![Bigger(500 << 20)])));
        // what can't be read is said back as typed (no guessing what a bare m or no unit means)
        for bad in [">abc", "<5XB", "<5m", ">10", ">", "<-5MB", ">1e3KB", ">infKB", ">1.2.3MB", "<7 days", ">=5MB"] {
            assert_eq!(parse_match(&format!("*.jpg; {bad}")), Err(bad.to_string()), "{bad}");
        }
        // no '<' or '>': exactly the spec it was before
        for t in ["*.jpg;*.png", "pdf; *.txt", "a b", "", " ; ", "x?.doc"] {
            assert_eq!(parse_match(t), Ok((filter_spec(t), vec![])), "{t}");
        }
        // far too big: kept in u64
        assert_eq!(parse_match(">99999999999999999999TB"), Ok((String::new(), vec![Bigger(u64::MAX)])));
    }

    #[test]
    fn condition_checks() {
        use Cond::*;
        const NOW: u64 = 133_000_000_000_000_000;
        const DAY: u64 = 24 * HOUR;
        assert!(cond_ok(Bigger(10), Some(11), None, NOW) && !cond_ok(Bigger(10), Some(10), None, NOW));
        assert!(cond_ok(Smaller(10), Some(9), None, NOW) && !cond_ok(Smaller(10), Some(10), None, NOW));
        assert!(cond_ok(Newer(7 * DAY), None, Some(NOW - 6 * DAY), NOW) && !cond_ok(Newer(7 * DAY), None, Some(NOW - 8 * DAY), NOW));
        assert!(cond_ok(Older(30 * DAY), None, Some(NOW - 31 * DAY), NOW) && !cond_ok(Older(30 * DAY), None, Some(NOW - DAY), NOW));
        // a date in the future is just modified, never "older"
        assert!(!cond_ok(Older(30 * DAY), None, Some(NOW + DAY), NOW) && cond_ok(Newer(DAY), None, Some(NOW + DAY), NOW));
        // not read, or a zero date: no match either way
        for c in [Bigger(0), Smaller(u64::MAX), Older(0), Newer(u64::MAX)] {
            assert!(!cond_ok(c, None, None, NOW) && !cond_ok(c, None, Some(0), NOW), "{c:?}");
        }
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

    fn item(name: &str, size: Option<u64>, time: Option<u64>) -> DiffItem {
        (name.to_string(), size, time)
    }

    #[test]
    fn diff_marks_rules() {
        const T: u64 = 133_000_000_000_000_000;
        // names in any case; a file only on one side is marked there
        let a = [item("Report.DOCX", Some(10), Some(T)), item("only-a.txt", Some(1), Some(T))];
        let b = [item("report.docx", Some(10), Some(T)), item("only-b.txt", Some(1), Some(T))];
        assert_eq!(diff_marks(&a, &b), DiffMarks { a: vec![false, true], b: vec![false, true], same: 1 });
        // 2 seconds apart (FAT): the same; a step more: the newer side only
        let a = [item("x", Some(5), Some(T + DIFF_SLACK)), item("y", Some(5), Some(T)), item("z", Some(5), Some(T + DIFF_SLACK + 1))];
        let b = [item("x", Some(5), Some(T)), item("y", Some(5), Some(T + DIFF_SLACK + 1)), item("z", Some(5), Some(T))];
        assert_eq!(diff_marks(&a, &b), DiffMarks { a: vec![false, false, true], b: vec![false, true, false], same: 1 });
        // same time, another size: both; a date not read is never newer (sizes alike: the same; size not read: the same)
        let a = [item("s", Some(5), Some(T)), item("d", Some(5), None), item("e", Some(5), None), item("n", None, Some(T))];
        let b = [item("s", Some(6), Some(T)), item("d", Some(5), Some(T + 9 * DIFF_SLACK)), item("e", Some(7), Some(T)), item("n", Some(8), Some(T))];
        assert_eq!(diff_marks(&a, &b), DiffMarks { a: vec![true, false, true, false], b: vec![true, false, true, false], same: 2 });
        // extensions keep files apart (parsing names, not display names)
        let a = [item("a.txt", Some(1), Some(T))];
        let b = [item("a.doc", Some(1), Some(T))];
        assert_eq!(diff_marks(&a, &b), DiffMarks { a: vec![true], b: vec![true], same: 0 });
        // empty folders
        assert_eq!(diff_marks(&[], &[]), DiffMarks { a: vec![], b: vec![], same: 0 });
        assert_eq!(diff_marks(&a, &[]), DiffMarks { a: vec![true], b: vec![], same: 0 });
        assert_eq!(diff_marks(&[], &b), DiffMarks { a: vec![], b: vec![true], same: 0 });
    }

    #[test]
    fn diff_notes() {
        assert_eq!(diff_note(false, 5, 3, 1, 120, false), "Selected the different files: 5 here · 3 in pane 2 (120 the same)");
        assert_eq!(diff_note(true, 5, 3, 1, 120, true), "다른 파일 선택: 여기 5개 · 패널 2에 3개 (같은 파일 120개) · 필터 적용 중");
        assert_eq!(diff_note(false, 0, 0, 3, 120, true), "The files in both folders are the same (by name, size and date modified, 120) · filter on");
        assert_eq!(diff_note(true, 0, 0, 3, 120, false), "두 폴더의 파일이 같습니다 (이름·크기·수정한 날짜 기준, 120개)");
        assert_eq!(diff_note(false, 0, 0, 1, 0, false), "No files to compare in either folder");
        assert_eq!(diff_note(true, 0, 0, 1, 0, true), "두 폴더 모두 비교할 파일이 없습니다 · 필터 적용 중");
        // only on disk folders, not the same one
        assert!(diff_ok(r"C:\a", r"D:\a") && diff_ok(r"C:\a", r"\\nas\share\a"));
        assert!(!diff_ok(r"C:\a", r"c:\A\") && !diff_ok("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", r"C:\a"));
    }
}
