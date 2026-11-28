use crate::*;
use std::io::Read as _;
use std::os::windows::fs::MetadataExt;
use std::sync::atomic::AtomicU64;
use windows::Win32::UI::Controls::{EM_SETCUEBANNER, MEASUREITEMSTRUCT};

// ---------- Alt+F7: find text inside the files of this folder and its subfolders ----------
// Total Commander's Find text (Alt+F7), VS Code's Ctrl+Shift+F: every other finder in Grids looks at names only.
// One worker walks the pane's folder; each search has a generation, so a newer one, or closing, stops the last.
// Rows come through GR_JOB (a lock, never a pointer in a message), read by the popup on a timer: a late worker's go
// nowhere. The popup is the Ctrl+P one's frame with a progress line under it.

pub(crate) const ID_GREP: usize = 1161; // ⇄ menu (1160 is TIMER_SHELL)
const GR_EDIT: i32 = 1;
const GR_LIST: i32 = 2;
const GR_ROWS: usize = 12;
const TIMER_TYPE: usize = 1; // on the popup: typing stopped (0.3 s), or a wait for a stuck worker
const TIMER_POLL: usize = 2; // on the popup: the worker's rows and counts
const GR_FILE_MAX: u64 = 16 << 20;
const GR_FILES_MAX: usize = 50_000;
const GR_HITS_MAX: usize = 500;
const GR_STUCK_MAX: usize = 2; // workers left hanging on a dead share before a new search waits for them
const OFFICE: [&str; 4] = ["docx", "docm", "pptx", "pptm"];
const BINARY: [&str; 28] = ["exe", "dll", "sys", "msi", "obj", "lib", "pdb", "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "ico", "mp3", "mp4", "mkv", "avi", "mov", "wav", "flac", "zip", "7z", "rar", "iso", "pdf", "xlsx"]; // never opened

/// A file found: its folder whole and under the searched one ("" for that folder), the first matching line's
/// number (none for Word / PowerPoint: their lines mean nothing) and words round the match (before, match, after),
/// and how many more lines match.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GrHit { pub(crate) name: String, pub(crate) dir: String, pub(crate) rel: String, pub(crate) line: Option<usize>, pub(crate) cut: [String; 3], pub(crate) more: usize }

/// Capped: stopped at GR_FILES_MAX or GR_HITS_MAX, not everything was looked at. Wait: an earlier worker still
/// hangs, this search starts once it lets go.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum GrState { #[default] Idle, Run, Done, Capped, Wait }

/// The counts the progress line shows: files looked at, too large, online-only.
type Counts = [usize; 3];

#[derive(Default)]
struct Job { gen: u64, hits: Vec<GrHit>, n: Counts, end: Option<GrState> }

static GR_JOB: Mutex<Job> = Mutex::new(Job { gen: 0, hits: Vec::new(), n: [0; 3], end: None });
static GR_GEN: AtomicU64 = AtomicU64::new(0);
static GR_LIVE: AtomicUsize = AtomicUsize::new(0);

fn job<R>(f: impl FnOnce(&mut Job) -> R) -> R { f(&mut GR_JOB.lock().unwrap_or_else(|e| e.into_inner())) }

#[derive(Debug, PartialEq)]
pub(crate) enum GrKind { Text, Office, Big, Online, Skip }

/// What a file is to the search, from its folder listing alone (never opened here): hidden ones follow the
/// panes' hidden items setting, known binaries are left out, online-only files are counted, not downloaded.
pub(crate) fn gr_kind(name: &str, size: u64, attr: u32, show_hidden: bool) -> GrKind {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match () {
        _ if attr & 0x2 != 0 && !show_hidden => GrKind::Skip,
        _ if BINARY.contains(&ext.as_str()) => GrKind::Skip,
        _ if cloud_only(attr) => GrKind::Online,
        _ if OFFICE.contains(&ext.as_str()) => GrKind::Office,
        _ if size > GR_FILE_MAX => GrKind::Big,
        _ => GrKind::Text,
    }
}

/// Where `q` is in `line`, as (byte offset, byte length) in `line` itself: ASCII ignoring ASCII case, anything
/// else by to_lowercase, a character at a time (a lower-cased copy's offsets would not be the line's).
pub(crate) fn gr_find(line: &str, q: &str) -> Option<(usize, usize)> {
    if q.is_ascii() {
        return find_ci(line, q).filter(|_| !q.is_empty()).map(|at| (at, q.len()));
    }
    let want: Vec<char> = q.chars().flat_map(char::to_lowercase).collect();
    if !line.to_lowercase().contains(&want.iter().collect::<String>()) {
        return None;
    }
    line.char_indices().find_map(|(at, _)| {
        let mut k = 0;
        for (off, c) in line[at..].char_indices() {
            for l in c.to_lowercase() {
                (want.get(k) == Some(&l)).then_some(())?;
                k += 1;
            }
            if k == want.len() {
                return Some((at, off + c.len_utf8()));
            }
        }
        None
    })
}

/// The line round a match, about `max` characters: (before, match, after), tabs as spaces, a cut end as "…".
/// The text before keeps at most a third, so the match stays in sight at the start of the row.
pub(crate) fn gr_excerpt(line: &str, at: usize, len: usize, max: usize) -> [String; 3] {
    let clean = |s: &str| s.replace('\t', " ");
    let (before, hit, after) = (clean(&line[..at]), clean(&line[at..at + len]), clean(&line[at + len..]));
    let before = before.trim_start();
    let (n, lead) = (before.chars().count(), max / 3);
    let before = if n > lead { format!("…{}", before.chars().skip(n - lead).collect::<String>()) } else { before.to_string() };
    let room = max.saturating_sub(before.chars().count() + hit.chars().count());
    let after = after.trim_end();
    let after = if after.chars().count() > room { format!("{}…", after.chars().take(room).collect::<String>()) } else { after.to_string() };
    [before, hit, after]
}

/// The first matching line of `text`, numbered or not, and how many more lines match.
fn gr_lines(text: &str, q: &str, numbered: bool) -> Option<(Option<usize>, [String; 3], usize)> {
    let mut hits = text.lines().enumerate().filter_map(|(k, line)| gr_find(line, q).map(|m| (k, line, m)));
    let (k, line, (at, len)) = hits.next()?;
    Some((numbered.then_some(k + 1), gr_excerpt(line, at, len, 120), hits.count()))
}

// A text file: its first 8 KB decide (looks_text) before the rest is read. Locked or unreadable: left out.
fn gr_text(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut b = vec![0u8; 8192];
    let n = f.read(&mut b).ok()?;
    b.truncate(n);
    looks_text(&b).then_some(())?;
    f.take(GR_FILE_MAX).read_to_end(&mut b).ok()?;
    Some(decode_text(&b))
}

fn gr_walk(root: PathBuf, q: String, hidden: bool, gen: u64) {
    let mut stack = vec![root.clone()];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let rel = dir.strip_prefix(&root).map(|r| r.to_string_lossy().into_owned()).unwrap_or_default();
        let mut subs = Vec::new();
        for e in rd.flatten() {
            if GR_GEN.load(Ordering::Relaxed) != gen { return; } // a newer search, or the popup closed
            let (Ok(t), Ok(m)) = (e.file_type(), e.metadata()) else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            // is_dir is false for junctions and links: never followed (a loop, or another drive's tree)
            if t.is_dir() && name != ".git" && (hidden || m.file_attributes() & 0x2 == 0) {
                subs.push(e.path());
            }
            if !t.is_file() { continue; }
            seen += 1;
            let kind = gr_kind(&name, m.len(), m.file_attributes(), hidden);
            let found = match kind {
                GrKind::Text => gr_text(&e.path()).and_then(|t| gr_lines(&t, &q, true)),
                GrKind::Office => office_words(&e.path().to_string_lossy()).and_then(|t| gr_lines(&t, &q, false)),
                _ => None,
            };
            let go_on = job(|j| {
                let mine = j.gen == gen; // else its rows go nowhere
                if mine {
                    j.n = [seen, j.n[1] + (kind == GrKind::Big) as usize, j.n[2] + (kind == GrKind::Online) as usize];
                    j.hits.extend(found.map(|(line, cut, more)| GrHit { name, dir: dir.to_string_lossy().into_owned(), rel: rel.clone(), line, cut, more }));
                    if j.hits.len() >= GR_HITS_MAX || seen >= GR_FILES_MAX {
                        j.end = Some(GrState::Capped);
                    }
                }
                mine && j.end.is_none()
            });
            if !go_on { return; }
        }
        stack.extend(subs.into_iter().rev()); // the folder's own files first, then its subfolders in listing order
    }
    job(|j| j.end = j.end.or((j.gen == gen).then_some(GrState::Done)));
}

/// The progress line under the rows.
pub(crate) fn gr_status(st: GrState, [seen, big, online]: Counts, hits: usize, ko: bool) -> String {
    let n = |v: usize| group_digits(v as u64);
    let mut skip: Vec<String> = Vec::new();
    skip.extend((big > 0).then(|| if ko { format!("큰 파일 {}", n(big)) } else { format!("{} large", n(big)) }));
    skip.extend((online > 0).then(|| if ko { format!("온라인 전용 {}", n(online)) } else { format!("{} online-only", n(online)) }));
    let skip = if skip.is_empty() { String::new() } else { format!(" · {} {}", if ko { "건너뜀:" } else { "skipped:" }, skip.join(", ")) };
    let files = if ko { format!("{}개 파일", n(hits)) } else { format!("{} file{}", n(hits), if hits == 1 { "" } else { "s" }) };
    match (st, ko) {
        (GrState::Idle, false) => "Type, then Enter (2 or more characters search as you type) · this folder and its subfolders".into(),
        (GrState::Idle, true) => "입력 후 Enter (2글자부터는 치는 대로 찾음) · 이 폴더와 하위 폴더".into(),
        (GrState::Run, false) => format!("Checked {} · {} found", n(seen), n(hits)),
        (GrState::Run, true) => format!("확인 {}개 · {}개 찾음", n(seen), n(hits)),
        (GrState::Done, false) => format!("Done · {files}{skip}"),
        (GrState::Done, true) => format!("끝 · {files}{skip}"),
        (GrState::Capped, false) => format!("Stopped at the limit (50,000 files or 500 results) · {files}{skip}"),
        (GrState::Capped, true) => format!("상한에서 멈춤 (파일 5만 개 또는 결과 500개) · {files}{skip}"),
        (GrState::Wait, false) => "The previous search isn't responding yet (a slow share?) — waiting".into(),
        (GrState::Wait, true) => "이전 검색이 아직 응답 없음 (느린 공유 폴더?) — 기다리는 중".into(),
    }
}

/// A row as one line: the list box's text for screen readers, and what ↑/↓ read out.
pub(crate) fn gr_line(h: &GrHit, ko: bool) -> String {
    let mut v = vec![h.name.clone()];
    v.extend(h.line.map(|n| if ko { format!("{n}번째 줄") } else { format!("line {n}") }));
    v.push(h.cut.concat());
    v.extend((h.more > 0).then(|| if ko { format!("{}줄 더", h.more) } else { format!("{} more", h.more) }));
    v.extend((!h.rel.is_empty()).then(|| h.rel.clone()));
    v.join(" · ")
}

// the popup; `last`: the text the running (or last) search was started with, `gen` its generation
#[derive(Default)]
struct Gr { h: HWND, edit: HWND, list: HWND, prev: HWND, pane: usize, root: String, w: i32, last: Option<String>, gen: u64, hits: Vec<GrHit>, st: GrState, n: Counts }

thread_local! { static GR: RefCell<Gr> = RefCell::new(Gr::default()); }
fn gr<R>(f: impl FnOnce(&mut Gr) -> R) -> R { GR.with(|s| f(&mut s.borrow_mut())) }

fn gr_geo(fh: i32, dpi: i32, w: i32) -> (SwGeo, RECT) {
    let g = sw_geo(fh, dpi, w, GR_ROWS);
    let s = |v: i32| v * dpi / 96;
    let status = RECT { left: s(12), top: g.list.bottom + s(2), right: w - s(12), bottom: g.list.bottom + s(2) + fh + s(10) };
    (SwGeo { h: status.bottom + s(4), ..g }, status)
}

/// Alt+F7 / the ⇄ menu: the popup over pane i, for its folder and the folders under it. Only a folder on a disk
/// or a share (not This PC, a library, a ZIP): the status line says why otherwise.
pub(crate) unsafe fn open_grep(i: usize) {
    let open = gr(|s| s.edit);
    if !open.0.is_null() {
        _ = SetFocus(Some(open));
        return;
    }
    let (main, font, fh, dpi, root, pane) = app(|a| (a.main, a.font, a.fh, a.dpi, a.s.paths[i].clone(), a.panes[i].as_ref().map(|p| p.hwnd)));
    if !disk_folder(&root) {
        _ = MessageBeep(MB_ICONWARNING);
        return set_note(tr("Find in files: only in a folder on a disk or a network share", "파일 내용에서 찾기: 디스크나 네트워크 공유의 폴더에서만 됩니다").into());
    }
    let s = |v: i32| v * dpi / 96;
    let (mut cr, mut pr) = (RECT::default(), RECT::default());
    _ = GetClientRect(main, &mut cr);
    _ = GetWindowRect(pane.unwrap_or(main), &mut pr);
    let (g, _) = gr_geo(fh, dpi, s(760).min(cr.right - s(32)).max(s(320)));
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    _ = GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let (x, y) = ((pr.left + pr.right - g.w) / 2, pr.top + s(36)); // over the pane it searches
    let at = fit(RECT { left: x, top: y, right: x + g.w, bottom: y + g.h }, mi.rcWork);
    let prev = GetFocus();
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsGrep"), w!(""), WS_POPUP | WS_CLIPCHILDREN, at.left, at.top, at.right - at.left, at.bottom - at.top, Some(main), None, None, None) else { return };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    gr(|x| *x = Gr { h, prev, pane: i, root: root.clone(), w: g.w, ..Gr::default() });
    let edit = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), GR_EDIT, font);
    _ = MoveWindow(edit, g.edit.left, g.edit.top, g.edit.right - g.edit.left, g.edit.bottom - g.edit.top, false);
    let cue = HSTRING::from(format!("{}  ({})", tr("Text to find inside files here and in subfolders", "이 폴더와 하위 폴더의 파일 안에서 찾을 글자"), short_name(&root)));
    SendMessageW(edit, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
    a11y_name(edit, Some(tr("Find text in files (Alt+F7)", "파일 내용에서 찾기 (Alt+F7)")));
    let lb = (LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32 | WS_VSCROLL.0;
    let list = child(h, w!("LISTBOX"), w!(""), lb, WINDOW_EX_STYLE(0), GR_LIST, font);
    _ = SetWindowSubclass(list, Some(sw_list_proc), 1, 0); // a click on a row: WM_SW_PICK, beside them: WM_SW_REFOCUS
    _ = MoveWindow(list, g.list.left, g.list.top, g.list.right - g.list.left, g.list.bottom - g.list.top, false);
    if DARK_ON.load(Ordering::Relaxed) { _ = SetWindowTheme(list, w!("DarkMode_Explorer"), PCWSTR::null()); }
    a11y_name(list, Some(tr("Files containing the text", "그 글자가 든 파일")));
    _ = ShowWindow(list, SW_HIDE);
    gr(|x| (x.edit, x.list) = (edit, list));
    _ = ShowWindow(h, SW_SHOW);
    _ = SetFocus(Some(edit));
}

// Typing stopped (2 characters or more) or Enter: a new search; the one before stops at its next file.
unsafe fn gr_start() {
    let (h, edit, list, root) = gr(|s| (s.h, s.edit, s.list, s.root.clone()));
    if h.0.is_null() { return; }
    _ = KillTimer(Some(h), TIMER_TYPE);
    let q = text_of(edit);
    if q.trim().is_empty() {
        if gr(|s| s.st) == GrState::Wait {
            gr(|s| s.st = GrState::Idle);
            _ = InvalidateRect(Some(h), None, false);
        }
        return;
    }
    let g = GR_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    SendMessageW(list, LB_RESETCONTENT, None, None);
    _ = ShowWindow(list, SW_HIDE);
    _ = InvalidateRect(Some(h), None, false);
    // the new generation already: a poll still due drops the last search's rows
    gr(|s| (s.gen, s.hits) = (g, Vec::new()));
    if GR_LIVE.load(Ordering::Relaxed) > GR_STUCK_MAX {
        gr(|s| (s.st, s.last) = (GrState::Wait, None));
        SetTimer(Some(h), TIMER_TYPE, 500, None); // tried again until one lets go
        return;
    }
    job(|j| *j = Job { gen: g, ..Job::default() });
    gr(|s| (s.last, s.st, s.n) = (Some(q.clone()), GrState::Run, [0; 3]));
    let hidden = hidden();
    GR_LIVE.fetch_add(1, Ordering::Relaxed);
    let spawned = std::thread::Builder::new().name("grids-grep".into()).spawn(move || {
        // a document that trips the reader ends this search, never the window, and the popup still hears the end
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gr_walk(PathBuf::from(root), q, hidden, g))).is_err() {
            job(|j| j.end = j.end.or((j.gen == g).then_some(GrState::Done)));
        }
        GR_LIVE.fetch_sub(1, Ordering::Relaxed);
    });
    if spawned.is_err() { GR_LIVE.fetch_sub(1, Ordering::Relaxed); job(|j| j.end = Some(GrState::Done)); }
    SetTimer(Some(h), TIMER_POLL, 150, None);
}

// TIMER_POLL: the rows found since the last look, and the counts; at the end the count is read out.
unsafe fn gr_poll() {
    let (h, list, gen, have) = gr(|s| (s.h, s.list, s.gen, s.hits.len()));
    let Some((new, n, end)) = job(|j| (j.gen == gen).then(|| (j.hits[have.min(j.hits.len())..].to_vec(), j.n, j.end))) else { return };
    let ko = KO.load(Ordering::Relaxed);
    for hit in &new {
        let t = HSTRING::from(gr_line(hit, ko));
        SendMessageW(list, LB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
    }
    let st = end.unwrap_or(GrState::Run);
    let count = gr(|s| { s.hits.extend(new); (s.n, s.st) = (n, st); s.hits.len() });
    if count > 0 && SendMessageW(list, LB_GETCURSEL, None, None).0 < 0 {
        SendMessageW(list, LB_SETCURSEL, Some(WPARAM(0)), None);
        _ = ShowWindow(list, SW_SHOWNA);
    }
    _ = InvalidateRect(Some(h), None, false);
    if end.is_some() {
        _ = KillTimer(Some(h), TIMER_POLL);
        say_only(&gr_status(st, n, count, ko));
    }
}

unsafe fn gr_move(d: i32) {
    let list = gr(|s| s.list);
    let n = SendMessageW(list, LB_GETCOUNT, None, None).0.max(0) as usize;
    let cur = SendMessageW(list, LB_GETCURSEL, None, None).0;
    let Some(k) = ev_step((cur >= 0).then_some(cur as usize), n, d) else { return };
    SendMessageW(list, LB_SETCURSEL, Some(WPARAM(k)), None);
    let ko = KO.load(Ordering::Relaxed);
    gr(|s| s.hits.get(k).map(|h| gr_line(h, ko))).inspect(|line| say_only(line));
}

/// Keys in the popup: the list keys, Enter, Esc and Alt+F7 (closes); the rest is typing, no pane shortcut acts
/// behind it. None: not the popup's key.
pub(crate) unsafe fn gr_key(m: &MSG) -> Option<bool> {
    let (h, edit, list) = gr(|s| (s.h, s.edit, s.list));
    if h.0.is_null() || !(m.hwnd == edit || m.hwnd == list || m.hwnd == h) { return None; }
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    let rows = GR_ROWS as i32;
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        VK_ESCAPE => gr_close(true),
        VK_F7 if alt && !ctrl && !shift && m.lParam.0 & (1 << 30) == 0 => gr_close(true),
        VK_F7 if alt => {} // held: closes once, the repeats go nowhere
        // new text: search; the same text with a row picked: go there
        VK_RETURN if !alt => {
            app(|a| a.held = Some(VK_RETURN.0));
            let k = SendMessageW(list, LB_GETCURSEL, None, None).0;
            let q = text_of(edit);
            let same = gr(|s| s.last.as_deref() == Some(q.as_str()));
            match usize::try_from(k) {
                Ok(k) if same => gr_go(k),
                _ if !same && !q.trim().is_empty() => gr_start(),
                _ => _ = MessageBeep(MB_ICONWARNING),
            }
        }
        k @ (VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT) if !alt => gr_move(match k { VK_UP => -1, VK_DOWN => 1, VK_PRIOR => -rows, _ => rows }),
        VK_TAB => {}
        _ if m.hwnd != edit && !alt => _ = SetFocus(Some(edit)),
        _ => return Some(false),
    }
    Some(true)
}

// Esc, Alt+F7: back where the user was (back: true). The worker stops with the window (WM_DESTROY).
unsafe fn gr_close(back: bool) {
    let (h, prev) = gr(|s| (s.h, s.prev));
    if h.0.is_null() { return; }
    _ = DestroyWindow(h);
    if back && !prev.0.is_null() && IsWindow(Some(prev)).as_bool() && IsWindowVisible(prev).as_bool() {
        _ = SetFocus(Some(prev));
    } else if back {
        focus_view(app(|a| a.s.active));
    }
}

// Enter / a click on row k: the pane the search was made in shows the file's folder with the file selected. A name
// filter there would hide it: emptied first, and the status line says so.
unsafe fn gr_go(k: usize) {
    let (hit, i) = gr(|s| (s.hits.get(k).cloned(), s.pane));
    gr_close(false);
    let Some(hit) = hit else { return };
    if i >= app(|a| a.s.panes()) || app(|a| a.panes[i].is_none()) {
        _ = MessageBeep(MB_ICONWARNING);
        return focus_view(app(|a| a.s.active));
    }
    set_active(i);
    if let Some(f) = pane_ctl(i, ID_FILTER).filter(|&f| !text_of(f).is_empty()) {
        _ = SetWindowTextW(f, w!(""));
        apply_filter(i);
        set_note(tr("Name filter cleared to show the file", "파일이 보이도록 이름 필터를 지웠습니다").into());
    }
    show_in_pane(i, hit.dir, Some(hit.name));
}

unsafe fn gr_draw(d: &DRAWITEMSTRUCT) {
    let (dc, rc) = (d.hDC, d.rcItem);
    let Some(hit) = gr(|s| s.hits.get(d.itemID as usize).cloned()) else { return fill(dc, rc, pal().card) };
    let (dpi, font, small, accent) = app(|a| (a.dpi, a.font, a.small_font, a.s.accent_now()));
    let s = |v: i32| v * dpi / 96;
    let sel = d.itemState.0 & ODS_SELECTED.0 != 0;
    let (bg, ink, sub) = match sel {
        true => (accent, on_color(accent), blend(on_color(accent), accent, 0.3)),
        false => (pal().card, pal().text, pal().sub),
    };
    fill(dc, rc, bg);
    let measure = |f: HFONT, t: &str| text_w(dc, t, f);
    let width = rc.right - rc.left;
    // the folder under the searched one at the right (a quarter at most), the file name (a third at most) and its
    // line number at the left, the line itself between, its match on a tinted chip
    let rel = if hit.more > 0 { format!("+{}  {}", hit.more, hit.rel) } else { hit.rel.clone() };
    let rw = if rel.is_empty() { 0 } else { (measure(small, &rel) + s(2)).min(width / 4) };
    let right = RECT { left: rc.right - s(10) - rw, right: rc.right - s(10), ..rc };
    text(dc, right, &rel, small, sub, DT_RIGHT | DT_PATH_ELLIPSIS);
    let name_r = RECT { left: rc.left + s(10), right: (rc.left + s(10) + measure(font, &hit.name)).min(rc.left + width / 3), ..rc };
    text(dc, name_r, &hit.name, font, ink, DT_LEFT);
    let mut x = name_r.right + s(6);
    let num = hit.line.map(|n| format!(":{n}")).unwrap_or_default();
    let nw = if num.is_empty() { 0 } else { measure(small, &num) + s(2) }; // never an empty text to GDI
    text(dc, RECT { left: x, right: x + nw, ..rc }, &num, small, sub, DT_LEFT);
    x += nw + s(10);
    let end = right.left - s(10);
    let chip = if sel { blend(accent, on_color(accent), 0.25) } else { blend(pal().card, accent, 0.28) };
    for (k, seg) in hit.cut.iter().enumerate().filter(|(_, t)| !t.is_empty()) {
        if x >= end { break; }
        let w = measure(font, seg);
        if k == 1 { fill(dc, RECT { left: x - s(1), top: rc.top + s(4), right: (x + w + s(1)).min(end), bottom: rc.bottom - s(4) }, chip); }
        text(dc, RECT { left: x, right: end, ..rc }, seg, font, if k == 1 { ink } else { sub }, DT_LEFT);
        x += w;
    }
}

unsafe fn gr_paint(h: HWND) {
    let (dpi, fh, font, small, icon_font, accent) = app(|a| (a.dpi, a.fh, a.font, a.small_font, a.icon_font, a.s.accent_now()));
    let (w, st, n, count) = gr(|s| (s.w, s.st, s.n, s.hits.len()));
    let (g, status) = gr_geo(fh, dpi, w);
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        fill(dc, g.field, pal().pill);
        frame(dc, g.field, 1, accent);
        text(dc, g.glass, ICON_SEARCH, icon_font, pal().sub, DT_CENTER);
        let none = match st {
            GrState::Done | GrState::Capped => tr("No file here contains that text", "그 글자가 든 파일이 없음"),
            GrState::Run => tr("Looking…", "찾는 중…"),
            _ => "",
        };
        if count == 0 { text(dc, g.list, none, font, pal().sub, DT_CENTER); }
        fill(dc, RECT { bottom: status.top + 1, ..status }, pal().line);
        let ink = if matches!(st, GrState::Capped | GrState::Wait) { accent } else { pal().sub };
        text(dc, status, &gr_status(st, n, count, KO.load(Ordering::Relaxed)), small, ink, DT_LEFT);
    });
}

pub(crate) extern "system" fn gr_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => gr_paint(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_CTLCOLORLISTBOX => {
                SetDCBrushColor(HDC(wp.0 as _), cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_MEASUREITEM | WM_DRAWITEM => {
                match msg {
                    WM_DRAWITEM => gr_draw(&*(lp.0 as *const DRAWITEMSTRUCT)),
                    _ => (*(lp.0 as *mut MEASUREITEMSTRUCT)).itemHeight = app(|a| sw_row_h(a.fh, a.dpi)) as u32,
                }
                return LRESULT(1);
            }
            // 2 characters or more search once typing stops; 1 only on Enter (it would match nearly every file)
            WM_COMMAND if (wp.0 & 0xffff) as i32 == GR_EDIT && (wp.0 >> 16 & 0xffff) as u32 == EN_CHANGE => {
                _ = KillTimer(Some(h), TIMER_TYPE);
                if text_of(gr(|s| s.edit)).trim().chars().count() >= 2 {
                    SetTimer(Some(h), TIMER_TYPE, 300, None);
                } else if gr(|s| s.st) == GrState::Wait {
                    // the retry shared this timer: nothing is waiting now
                    gr(|s| s.st = GrState::Idle);
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_TIMER if wp.0 == TIMER_TYPE => gr_start(),
            WM_TIMER if wp.0 == TIMER_POLL => gr_poll(),
            WM_SW_PICK => gr_go(wp.0),
            WM_SW_REFOCUS => _ = SetFocus(Some(gr(|s| s.edit))),
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => {
                GR_GEN.fetch_add(1, Ordering::Relaxed); // the worker stops at its next file; its rows go nowhere
                job(|j| *j = Job::default());
                for c in gr(|s| [s.edit, s.list]) { a11y_name(c, None); }
                gr(|s| *s = Gr::default());
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grep_finds_in_the_line_itself() {
        assert_eq!(gr_find("let x = TODO(fix);", "todo"), Some((8, 4)));
        assert_eq!(gr_find("계약번호 2024-117 확인", "2024-117"), Some(("계약번호 ".len(), 8)));
        assert_eq!((gr_find("nothing", "else"), gr_find("abc", ""), gr_find("한국어", "영어")), (None, None, None));
        // not ASCII: by to_lowercase, the offsets the line's own
        let at = |line: &'static str, q: &str| gr_find(line, q).map(|(a, l)| &line[a..a + l]);
        assert_eq!(at("Größe: ÄRGER und mehr", "ärger"), Some("ÄRGER"));
        assert_eq!(at("Привет МИР", "мир"), Some("МИР"));
        assert_eq!(at("한국어 문서", "문서"), Some("문서"));
        // the excerpt keeps the match in sight
        assert_eq!(gr_excerpt("\t\tfn main() {", 2, 2, 60), ["".to_string(), "fn".into(), " main() {".into()]);
        assert_eq!(gr_excerpt("x\tTODO\tdone  ", 2, 4, 60), ["x ".to_string(), "TODO".into(), " done".into()]);
        let long = format!("{}needle{}", "a".repeat(100), "b".repeat(100));
        let [b, m, a] = gr_excerpt(&long, 100, 6, 60);
        assert_eq!((b.chars().count(), m.as_str(), a.chars().count()), (21, "needle", 34));
        assert!(b.starts_with('…') && a.ends_with('…'));
        // Korean cut by characters, never inside one
        let ko = format!("{}오류{}", "가".repeat(40), "나".repeat(80));
        let [b, _, a] = gr_excerpt(&ko, "가".len() * 40, "오류".len(), 60);
        assert_eq!((b.chars().count(), a.chars().count()), (21, 38));
    }

    #[test]
    fn grep_kinds_lines_and_status() {
        use GrKind::*;
        let k = |name, size, attr| gr_kind(name, size, attr, false);
        assert_eq!((k("main.rs", 10, 0x20), k("Makefile", 10, 0x80), k("ok.log", GR_FILE_MAX, 0x20)), (Text, Text, Text));
        assert_eq!((k("big.log", GR_FILE_MAX + 1, 0x20), k("report.DOCX", 50 << 20, 0x20), k("deck.pptx", 1, 0x20)), (Big, Office, Office));
        // a binary isn't counted as online-only; an online-only document is never opened
        assert_eq!((k("photo.JPG", 1, 0x20), k("tool.exe", 1, 0x40_0000), k("notes.txt", 1, 0x40_0000), k("notes.docx", 1, 0x1000)), (Skip, Skip, Online, Online));
        assert_eq!((k(".env", 1, 0x22), gr_kind(".env", 1, 0x22, true)), (Skip, Text));
        let text = "one\nTwo todo\nthree\nTODO again\nand todo";
        let (n, cut, more) = gr_lines(text, "todo", true).unwrap();
        assert_eq!((n, cut[1].as_str(), more), (Some(2), "todo", 2));
        assert_eq!((gr_lines(text, "todo", false).unwrap().0, gr_lines(text, "zzz", true)), (None, None));
        use GrState::*;
        assert_eq!((gr_status(Run, [1234, 0, 0], 17, false), gr_status(Run, [1234, 0, 0], 17, true)), ("Checked 1,234 · 17 found".into(), "확인 1,234개 · 17개 찾음".into()));
        assert_eq!(gr_status(Done, [9, 3, 2], 17, true), "끝 · 17개 파일 · 건너뜀: 큰 파일 3, 온라인 전용 2");
        assert_eq!((gr_status(Done, [9, 0, 2], 1, false), gr_status(Done, [9, 0, 0], 0, false)), ("Done · 1 file · skipped: 2 online-only".into(), "Done · 0 files".into()));
        // a search cut short never reads as finished
        assert!(gr_status(Capped, [50_000, 0, 0], 3, false).starts_with("Stopped at the limit") && gr_status(Capped, [50_000, 0, 0], 3, true).starts_with("상한에서 멈춤"));
        let hit = GrHit { name: "a.rs".into(), dir: r"C:\p\src".into(), rel: "src".into(), line: Some(12), cut: ["x ".into(), "TODO".into(), " y".into()], more: 2 };
        assert_eq!(gr_line(&hit, false), "a.rs · line 12 · x TODO y · 2 more · src");
        assert_eq!(gr_line(&GrHit { line: None, more: 0, rel: String::new(), ..hit }, true), "a.rs · x TODO y");
    }
}
