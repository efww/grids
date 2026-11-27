use crate::*;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegOpenKeyExW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ, RRF_RT_REG_SZ,
};
use windows::Win32::System::RestartManager::*;
use windows::Win32::System::SystemInformation::GetLocalTime;
use std::os::windows::fs::MetadataExt;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Shell::Common::{PERCEIVED, PERCEIVED_TYPE_TEXT};
use windows::Win32::UI::Controls::{
    TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDCBF_CANCEL_BUTTON, TDCBF_CLOSE_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
    TD_INFORMATION_ICON, TD_WARNING_ICON,
};

// Small shell extras: Shift+F7 new text file, "which program is using this?", Compare with WinMerge (⇄ menu).
// Ported from Samek86/maguro, re-implemented for Grids.

// ⇄ menu commands (menu-local, returned by TrackPopupMenu)
pub(crate) const ID_INUSE: usize = 1130;
pub(crate) const ID_WM_PAIR: usize = 1131;
pub(crate) const ID_WM_PANE: usize = 3100; // + pane index: compare this pane's folder with that one
const BTN_RETRY: i32 = 3140;
const BTN_SWITCH: i32 = 3141;
const BTN_CLOSE: i32 = 3142;
const RADIO: i32 = 3120; // + holder index
const LVM_GETEDITCONTROL: u32 = 0x1018;
const LVM_GETNEXTITEM: u32 = 0x100C;
const LVM_EDITLABELW: u32 = 0x1076;
const LVNI_FOCUSED: isize = 1;

// ---------- new text file ----------

// "New Text Document.txt", then "New Text Document (2).txt", … The number goes before the extension, like Explorer.
pub(crate) fn fresh_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (base, String::new()),
    };
    std::iter::once(base.to_string()).chain((2..10_000).map(|n| format!("{stem} ({n}){ext}"))).find(|n| !taken(n)).unwrap_or_default()
}

// file the view should select and rename once it lists it: (pane, full path, ticks left, folder and view
// the pane must still show with the focus in its list, else the rename is dropped: set for Ctrl+F7 only)
pub(crate) type NewFile = (usize, String, u8, Option<(String, usize)>);
thread_local! {
    static NEW_FILE: RefCell<Option<NewFile>> = const { RefCell::new(None) };
}

// Shift+F7: an empty text file in the active pane's folder, selected and in rename, as F7 does for folders.
// create_new: an existing file is never opened, let alone truncated.
pub(crate) unsafe fn new_text_file(i: usize) {
    let dir = app(|a| a.s.paths[i].clone());
    let name = fresh_name(tr("New Text Document.txt", "새 텍스트 문서.txt"), |n| Path::new(&dir).join(n).exists());
    let full = Path::new(&dir).join(&name).to_string_lossy().into_owned();
    if !is_fs(&dir) || name.is_empty() || std::fs::OpenOptions::new().write(true).create_new(true).open(&full).is_err() {
        fail(Fail::NewFileHere, "");
        return;
    }
    let wide = HSTRING::from(full.as_str());
    SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    rename_soon((i, full, 40, None));
}

// the view lists it a moment later: poll on a thread timer (no window of ours needed)
pub(crate) unsafe fn rename_soon(f: NewFile) {
    NEW_FILE.with(|n| *n.borrow_mut() = Some(f));
    SetTimer(None, 0, 50, Some(new_file_tick));
}

unsafe extern "system" fn new_file_tick(_: HWND, _: u32, id: usize, _: u32) {
    let Some((i, full, left, guard)) = NEW_FILE.with(|n| n.borrow().clone()) else {
        _ = KillTimer(None, id);
        return;
    };
    let gone = guard.as_ref().is_some_and(|(dir, view)| !rename_ok(i, dir) || view_id(i) != *view);
    if gone || select_rename(i, &full) || left == 0 {
        NEW_FILE.with(|n| *n.borrow_mut() = None);
        _ = KillTimer(None, id);
    } else {
        NEW_FILE.with(|n| *n.borrow_mut() = Some((i, full, left - 1, guard)));
    }
}

// ---------- Ctrl+F7: the selected items into a new folder ----------

// The new folder's name: one item gives its name without the extension; several give the start of the
// name they all share, once trailing ' ', '-', '_', '.' and '(' are cut, if 3 characters or more are left.
// Otherwise `fallback`, which also stands in for a name Windows reserves ("Con" from Config.ini + Console.log).
pub(crate) fn group_base(names: &[&str], fallback: &str) -> String {
    let cut = |s: &str| s.trim_end_matches([' ', '-', '_', '.', '(']).to_string();
    let base = match names {
        [] => String::new(),
        [one] => cut(match one.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem,
            _ => one,
        }),
        [first, rest @ ..] => {
            let n = rest.iter().map(|r| first.chars().zip(r.chars()).take_while(|(a, b)| a == b).count()).min().unwrap_or(0);
            Some(cut(&first.chars().take(n).collect::<String>())).filter(|b| b.chars().count() >= 3).unwrap_or_default()
        }
    };
    if base.trim().is_empty() || crate::rename_rules::reserved_name(&base) { fallback.to_string() } else { base }
}

// "Name", then "Name (2)", "Name (3)" … A folder: the number goes at the end, not before a '.'.
pub(crate) fn group_name(base: &str, k: u32) -> String {
    if k < 2 { base.to_string() } else { format!("{base} ({k})") }
}

// Only a whole selection that sits right in the pane's own (file system) folder.
pub(crate) fn group_ok(srcs: &[String], dir: &str) -> bool {
    is_fs(dir) && paste_mode(srcs, dir) == Some(true)
}

pub(crate) const ID_GROUP: usize = 1133;

// a finished grouping, for the UI thread (WM_GROUPED)
struct Grouped {
    pane: usize,
    dir: String,
    new: String,
    n: usize,
    view: usize, // the pane's view when it started: another one (navigated away and back) gets no rename
}

static GROUPED: Mutex<Vec<Grouped>> = Mutex::new(Vec::new());

// a grouping is under way: a second Ctrl+F7 or ⇄ click meanwhile only beeps (it would race the first one
// for the same items into a "Name (2)")
static GROUPING: AtomicBool = AtomicBool::new(false);

struct GroupingDone;
impl Drop for GroupingDone {
    fn drop(&mut self) {
        GROUPING.store(false, Ordering::SeqCst);
    }
}

fn base_of(p: &str) -> &str {
    p.trim_end_matches('\\').rsplit('\\').next().unwrap_or(p)
}

unsafe fn view_id(i: usize) -> usize {
    view_of::<IShellView>(i).map_or(0, |v| v.as_raw() as usize)
}

// The pane still shows `dir` and nothing else is being typed into: no focus in this thread (another
// program, or the move's own progress window, is in front), the focus in that pane's file list, or on
// one of its top bar buttons (the ⇄ button keeps it after its menu).
unsafe fn rename_ok(i: usize, dir: &str) -> bool {
    let pane = app(|a| a.panes.get(i)?.as_ref().filter(|_| i < a.s.panes() && same_dir(&a.s.paths[i], dir)).map(|p| p.hwnd));
    let (Some(pane), Some(sv)) = (pane, view_of::<IShellView>(i)) else { return false };
    let f = GetFocus();
    if f.0.is_null() || in_view_list(sv.GetWindow().unwrap_or_default(), f) {
        return true;
    }
    let mut c = [0u16; 16];
    let n = GetClassNameW(f, &mut c) as usize;
    IsChild(pane, f).as_bool() && String::from_utf16_lossy(&c[..n]).eq_ignore_ascii_case("Button")
}

// Ctrl+F7 in a file list: the pane's whole selection, as the ⇄ menu counts it.
pub(crate) unsafe fn group_key(i: usize) {
    let arr = view_of::<IFolderView2>(i).and_then(|fv| fv.GetSelection(false).ok());
    let count = arr.as_ref().and_then(|a| a.GetCount().ok()).unwrap_or(0);
    let picked = collect_all(count, |k| unsafe { arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)) });
    group_into_folder(i, picked);
}

// Makes the folder and moves the items on a worker thread (one undoable shell move, never replacing
// anything); the rename starts only after every item is in. Ctrl+F7 and the ⇄ menu both end up here.
pub(crate) unsafe fn group_into_folder(i: usize, picked: std::result::Result<Vec<String>, Pick>) {
    let (dir, main) = app(|a| (a.s.paths[i].clone(), a.main.0 as usize));
    let srcs = match picked {
        Ok(s) if group_ok(&s, &dir) => s,
        Err(Pick::Empty) => return fail(Fail::GroupEmpty, ""),
        Err(Pick::TooMany) => return fail(Fail::GroupTooMany, ""),
        _ => return fail(Fail::GroupCant, ""), // unreadable items, a zip or shell place, items from elsewhere
    };
    if GROUPING.swap(true, Ordering::SeqCst) {
        return fail(Fail::GroupBusy, "");
    }
    let done = GroupingDone; // released when the worker ends, or right away if it can't start
    let names: Vec<&str> = srcs.iter().map(|s| base_of(s)).collect();
    let base = group_base(&names, tr("New folder", "새 폴더"));
    let view = view_id(i);
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let _done = done;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        if let Some(new) = group_work(&srcs, &dir, &base) {
            GROUPED.lock().unwrap_or_else(|e| e.into_inner()).push(Grouped { pane: i, dir, new, n: srcs.len(), view });
            _ = PostMessageW(Some(HWND(main as _)), WM_GROUPED, WPARAM(0), LPARAM(0));
        }
        if com {
            CoUninitialize();
        }
    });
    match job {
        Ok(job) => keep_job(job),
        Err(_) => _ = MessageBeep(MB_ICONWARNING),
    }
}

// Worker thread. The new folder's path once every item is in it; None after a failure or a cancel, with
// the folder removed again if it is still empty, and said where things are if only some got moved.
unsafe fn group_work(srcs: &[String], dir: &str, base: &str) -> Option<String> {
    let warn = |text: String| _ = MessageBoxW(None, &HSTRING::from(text), w!("Grids"), MB_ICONWARNING);
    let mut new = None;
    for k in 1..=999 {
        let p = Path::new(dir).join(group_name(base, k));
        match std::fs::create_dir(&p) {
            Ok(()) => {
                new = Some(p.to_string_lossy().into_owned());
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                warn(format!("{}{e}", tr("Couldn't make the new folder: ", "새 폴더를 만들지 못했습니다: ")));
                return None;
            }
        }
    }
    let Some(new) = new else {
        _ = MessageBeep(MB_ICONWARNING);
        return None;
    };
    let wide = HSTRING::from(new.as_str());
    SHChangeNotify(SHCNE_MKDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    // FOF_RENAMEONCOLLISION: whatever turns up in the new folder meanwhile is never replaced or merged
    let op = run_op(srcs, &new, true, true);
    let aborted = op.as_ref().map_or(true, |op| op.GetAnyOperationsAborted().map_or(true, |b| b.as_bool()));
    // what actually sits in the new folder now (made empty just above), not just what left the old one
    let moved = std::fs::read_dir(&new).map_or(0, |d| d.count()).min(srcs.len());
    let left = srcs.iter().any(|s| std::fs::symlink_metadata(s).is_ok());
    if !aborted && !left && moved == srcs.len() {
        return Some(new);
    }
    if let Err(e) = &op {
        if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
            warn(format!("{}{}", tr("Moving failed: ", "이동하지 못했습니다: "), e.message()));
        }
    }
    // remove_dir only takes an empty folder: anything that did get moved stays put
    if std::fs::remove_dir(&new).is_err() && moved > 0 {
        let text = tr("{m} of {n} items were moved into\n{new}\n\nThe rest are still in\n{dir}", "{n}개 중 {m}개만 아래 폴더로 옮겨졌습니다.\n{new}\n\n나머지는 그대로 있습니다.\n{dir}");
        warn(text.replace("{m}", &moved.to_string()).replace("{n}", &srcs.len().to_string()).replace("{new}", &new).replace("{dir}", dir));
    }
    None
}

// WM_GROUPED on the main window: says so on the status line, then selects the new folder and opens its
// rename, as F7 does, only while that pane still shows the same view of that folder and nothing else
// has the keyboard.
pub(crate) unsafe fn grouped_msg() {
    let done: Vec<Grouped> = std::mem::take(&mut *GROUPED.lock().unwrap_or_else(|e| e.into_inner()));
    let Some(g) = done.into_iter().last() else { return };
    let note = tr("Moved {n} items into “{name}”", "{n}개를 ‘{name}’ 폴더로 묶음");
    set_note(note.replace("{n}", &g.n.to_string()).replace("{name}", base_of(&g.new)));
    if rename_ok(g.pane, &g.dir) && view_id(g.pane) == g.view {
        rename_soon((g.pane, g.new, 40, Some((g.dir, g.view))));
    }
}

// ---------- Shift+F5: dated copies of the selected items, next to them ----------
// (Total Commander's Shift+F5 copy under a new name in the same folder, XYplorer's "duplicate with date stamp")

pub(crate) const ID_STAMP: usize = 1143; // ⇄ menu (1141 / 1142 are dialog jump timers, 1138 stays unused)

// "Report.docx" → "Report (2026-11-27 14.30).docx"; k ≥ 2 → "Report (2026-11-27 14.30) (2).docx". A folder,
// or a name with no '.', only a leading one (".gitignore") or a space / ')' after the last one ("README (… 14.30)"
// stamped again, "Meeting 3.5 notes"), takes it all at the end. '.' stands in for ':'.
pub(crate) fn stamp_name(name: &str, is_dir: bool, t: &SYSTEMTIME, k: u32) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !is_dir && !s.is_empty() && !e.contains([' ', ')']) => (s, format!(".{e}")),
        _ => (name, String::new()),
    };
    let stamp = format!("{stem} ({:04}-{:02}-{:02} {:02}.{:02})", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute);
    format!("{}{ext}", group_name(&stamp, k))
}

// The first stamp_name not taken; None when 999 are.
pub(crate) fn stamp_free(name: &str, is_dir: bool, t: &SYSTEMTIME, taken: impl Fn(&str) -> bool) -> Option<String> {
    (1..1000).map(|k| stamp_name(name, is_dir, t, k)).find(|n| !taken(n))
}

// a dated copy is under way: a second Shift+F5 meanwhile only beeps (both would pick the same free names)
static STAMPING: AtomicBool = AtomicBool::new(false);

struct StampingDone;
impl Drop for StampingDone {
    fn drop(&mut self) {
        STAMPING.store(false, Ordering::SeqCst);
    }
}

// a finished run, for the UI thread (WM_STAMPED): items picked, the copy's name when there was one,
// and whether anything was skipped or cancelled (then no count is claimed)
static STAMPED: Mutex<Option<(usize, String, bool)>> = Mutex::new(None);

// Shift+F5 in a file list: the pane's whole selection, as the ⇄ menu counts it.
pub(crate) unsafe fn stamp_key(i: usize) {
    let arr = view_of::<IFolderView2>(i).and_then(|fv| fv.GetSelection(false).ok());
    let count = arr.as_ref().and_then(|a| a.GetCount().ok()).unwrap_or(0);
    let picked = collect_all(count, |k| unsafe { arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)) });
    start_stamp_copy(i, picked);
}

// Copies the items into their own folder under dated names, on a worker thread: one undoable shell copy that
// never replaces anything. The selection stays on the originals. Shift+F5 and the ⇄ menu both end up here.
pub(crate) unsafe fn start_stamp_copy(i: usize, picked: std::result::Result<Vec<String>, Pick>) {
    let (dir, main) = app(|a| (a.s.paths[i].clone(), a.main.0 as usize));
    let srcs = match picked {
        Ok(s) if group_ok(&s, &dir) => s,
        Err(Pick::Empty) => return fail(Fail::StampEmpty, ""),
        Err(Pick::TooMany) => return fail(Fail::StampTooMany, ""),
        _ => return fail(Fail::StampCant, ""),
    };
    if STAMPING.swap(true, Ordering::SeqCst) {
        return fail(Fail::StampBusy, "");
    }
    let done = StampingDone;
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let _done = done;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        match stamp_work(&srcs, &dir) {
            Ok((first, aborted)) => {
                *STAMPED.lock().unwrap_or_else(|e| e.into_inner()) = Some((srcs.len(), first, aborted));
                _ = PostMessageW(Some(HWND(main as _)), WM_STAMPED, WPARAM(0), LPARAM(0));
            }
            Err(e) => {
                if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
                    let text = HSTRING::from(format!("{}{}", tr("Copying failed: ", "복사하지 못했습니다: "), e.message()));
                    MessageBoxW(None, &text, w!("Grids"), MB_ICONWARNING);
                }
            }
        }
        if com {
            CoUninitialize();
        }
    });
    match job {
        Ok(job) => keep_job(job),
        Err(_) => _ = MessageBeep(MB_ICONWARNING),
    }
}

// Worker thread: names picked here (the disk is asked off the UI thread), all with the same minute.
// Ok((first new name, anything skipped or cancelled)).
unsafe fn stamp_work(srcs: &[String], dir: &str) -> Result<(String, bool)> {
    let t = GetLocalTime();
    let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
    // FOF_RENAMEONCOLLISION: a name that turns up meanwhile is never replaced, the copy gets Windows' own name
    op.SetOperationFlags(FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD | FOF_RENAMEONCOLLISION)?;
    let to: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(dir), None)?;
    let mut picked: Vec<String> = Vec::new();
    for s in srcs {
        let is_dir = std::fs::metadata(s).or_else(|_| std::fs::symlink_metadata(s)).is_ok_and(|m| m.is_dir());
        // symlink_metadata: a broken link still holds its name
        let taken = |n: &str| picked.iter().any(|p| p.eq_ignore_ascii_case(n)) || std::fs::symlink_metadata(Path::new(dir).join(n)).is_ok();
        let Some(new) = stamp_free(base_of(s), is_dir, &t, taken) else { return Err(ERROR_FILE_EXISTS.to_hresult().into()) };
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(s), None)?;
        op.CopyItem(&item, &to, &HSTRING::from(new.as_str()), None)?;
        picked.push(new);
    }
    op.PerformOperations()?;
    let aborted = op.GetAnyOperationsAborted().map_or(true, |b| b.as_bool());
    Ok((picked.into_iter().next().unwrap_or_default(), aborted))
}

// WM_STAMPED on the main window: said on the status line; the selection stays on the originals, so the next
// Enter opens the original to work on, with the dated copy kept aside.
pub(crate) unsafe fn stamped_msg() {
    let Some((n, first, aborted)) = STAMPED.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let note = stamped_note(KO.load(Ordering::Relaxed), n, &first, aborted);
    // a big folder may end with Grids behind or minimized: said once it is back, as a finished copy is
    let main = app(|a| a.main);
    if !app_in_front() || IsIconic(main).as_bool() || !IsWindowVisible(main).as_bool() {
        say_when_back(note);
    } else {
        set_note(note);
    }
}

pub(crate) fn stamped_note(ko: bool, n: usize, first: &str, aborted: bool) -> String {
    match (ko, aborted, n) {
        (true, true, _) => "날짜 붙인 사본 만들기 끝 (일부 건너뜀 또는 취소)".into(),
        (false, true, _) => "Dated copies ended (some skipped or cancelled)".into(),
        (true, false, 1) => format!("날짜 붙인 사본 만듦: {first}"),
        (false, false, 1) => format!("Made a dated copy: {first}"),
        (true, false, _) => format!("날짜 붙인 사본 {n}개 만듦"),
        (false, false, _) => format!("Made {n} dated copies"),
    }
}

// ---------- Ctrl+E: the chosen files in an editor, never run ----------
// (Total Commander's F4 Edit, Directory Opus / XYplorer's Edit command; F4 is the address bar's here)

pub(crate) const EDIT_MAX: usize = 10; // files handed over per press
const EDIT_SCAN_MAX: u32 = 2000; // how far into the selection files are counted

/// How Ctrl+E opens one file. Never with its own default verb: a .bat, .reg or .vbs would run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum EditHow {
    Verb,   // its type's own "edit" verb (.bat / .cmd in Notepad, .ps1 in its editor, a picture in Paint …)
    Text,   // as a .txt: the user's text editor, else Notepad
    Binary, // not text: left alone
    Online, // online-only and not text by its type: not downloaded just to look
}

/// verb: its type has an "edit" verb; online: only in the cloud (its content is never read here); text_type: its
/// type is perceived as text; head: its first 8 KB, read only when asked. A file that can't be read goes by its type.
pub(crate) fn edit_how(verb: bool, online: bool, text_type: bool, head: impl FnOnce() -> Option<Vec<u8>>) -> EditHow {
    let text = |t: bool| if t { EditHow::Text } else { EditHow::Binary };
    if verb {
        EditHow::Verb
    } else if online {
        if text_type { EditHow::Text } else { EditHow::Online }
    } else {
        text(head().map_or(text_type, |b| looks_text(&b)))
    }
}

/// What Ctrl+E did, for the status line: `opened` handed to an editor out of `picked` sent; `total`: the files
/// chosen (None: more than were counted); `skipped`: left alone (binary, online-only) or not opened.
pub(crate) fn edited_note(ko: bool, opened: usize, picked: usize, total: Option<usize>, skipped: usize) -> String {
    let cut = total.map_or(picked >= EDIT_MAX, |t| t > picked);
    let mut note = match (ko, cut, total) {
        (true, true, Some(t)) => format!("편집기로 {opened}/{t}개만 열었습니다 (한 번에 {EDIT_MAX}개까지)"),
        (true, true, None) => format!("편집기로 {opened}개만 열었습니다 (한 번에 {EDIT_MAX}개까지)"),
        (true, false, _) => format!("편집기로 {opened}개를 열었습니다"),
        (false, true, Some(t)) => format!("Opened {opened} of {t} files in an editor ({EDIT_MAX} at a time)"),
        (false, true, None) => format!("Opened {opened} files in an editor ({EDIT_MAX} at a time)"),
        (false, false, _) => format!("Opened {opened} in an editor"),
    };
    if skipped > 0 {
        note += &if ko { format!(" · {skipped}개 건너뜀") } else { format!(" · {skipped} skipped") };
    }
    note
}

// The files Ctrl+E opens: the selected ones when the focused item is among them (or nothing has the focus), else
// the focused one, in view order. Real folders (not ZIPs) and items without a file path (This PC, inside a ZIP)
// are left out. The first EDIT_MAX, and how many there were (None: the selection is longer than EDIT_SCAN_MAX).
unsafe fn edit_targets(i: usize) -> (Vec<String>, Option<usize>) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return (Vec::new(), Some(0)) };
    let file = |it: &IShellItem| {
        let attr = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0)).unwrap_or_default().0;
        (!real_folder(attr)).then(|| it.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p))).flatten()
    };
    let focus = fv.GetFocusedItem().ok().filter(|&k| k >= 0);
    if let Some(k) = focus.filter(|&k| is_selected(&fv, k) != Some(true)) {
        let one: Vec<String> = fv.GetItem::<IShellItem>(k).ok().and_then(|it| file(&it)).into_iter().collect();
        let n = one.len();
        return (one, Some(n));
    }
    let sel = fv.Items::<IShellItemArray>(_SVGIO(SVGIO_SELECTION.0 | SVGIO_FLAG_VIEWORDER.0)).or_else(|_| fv.GetSelection(false));
    let Ok(arr) = sel else { return (Vec::new(), Some(0)) };
    let n = arr.GetCount().unwrap_or(0);
    let (mut paths, mut count) = (Vec::new(), 0);
    for k in 0..n.min(EDIT_SCAN_MAX) {
        if let Some(p) = arr.GetItemAt(k).ok().and_then(|it| file(&it)) {
            count += 1;
            if paths.len() < EDIT_MAX {
                paths.push(p);
            }
        }
    }
    (paths, (n <= EDIT_SCAN_MAX).then_some(count))
}

// a finished Ctrl+E, for the UI thread (WM_EDITED: boxed in its lParam)
pub(crate) struct Edited {
    picked: usize,
    total: Option<usize>,
    opened: usize,
    binary: usize,
    online: usize,
    failed: Option<String>, // the first file no editor took
}

// Ctrl+E in a file list: the files go to a worker thread (it reads their heads and waits on the shell), the
// status line hears back once.
pub(crate) unsafe fn edit_files(i: usize) {
    let (paths, total) = edit_targets(i);
    if paths.is_empty() {
        return fail(Fail::EditNone, "");
    }
    let main = app(|a| a.main.0 as usize);
    let job = std::thread::Builder::new().spawn(move || {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let mut done = Edited { picked: paths.len(), total, opened: 0, binary: 0, online: 0, failed: None };
        for p in &paths {
            match edit_one(p) {
                Some(EditHow::Binary) => done.binary += 1,
                Some(EditHow::Online) => done.online += 1,
                Some(_) => done.opened += 1,
                None => _ = done.failed.get_or_insert_with(|| base_of(p).to_string()),
            }
        }
        if com {
            CoUninitialize();
        }
        // the box comes back if the message can't be posted (the window is gone)
        let ptr = Box::into_raw(Box::new(done));
        if PostMessageW(Some(HWND(main as _)), WM_EDITED, WPARAM(0), LPARAM(ptr as isize)).is_err() {
            drop(Box::from_raw(ptr));
        }
    });
    match job {
        Ok(job) => keep_job(job),
        Err(_) => _ = MessageBeep(MB_ICONWARNING),
    }
}

// Worker thread: one file in its editor. Some(how it went: Verb / Text opened, Binary / Online left alone),
// None when no editor took it. Order: its "edit" verb, then as a .txt, then Notepad; each step only after the
// one before failed, and never the file's own "open".
unsafe fn edit_one(path: &str) -> Option<EditHow> {
    let ext = Path::new(path).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let dir = Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let online = std::fs::metadata(path).is_ok_and(|m| cloud_only(m.file_attributes()));
    let verb = !ext.is_empty() && has_edit_verb(&ext);
    if verb && shell_run(w!("edit"), path, None, false, &dir) {
        return Some(EditHow::Verb);
    }
    let head = || {
        use std::io::Read;
        let mut b = Vec::new();
        std::fs::File::open(path).ok()?.take(8192).read_to_end(&mut b).ok()?;
        Some(b)
    };
    match edit_how(false, online, text_type(&ext), head) {
        EditHow::Text => {
            let notepad = format!(r"{}\System32\notepad.exe", std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()));
            let ok = shell_run(w!("open"), path, None, true, &dir) || shell_run(w!("open"), &notepad, Some(&quote_arg(path)), false, &dir);
            ok.then_some(EditHow::Text)
        }
        how => Some(how),
    }
}

// ShellExecuteEx that waits for the shell to hand over (the worker ends right after) and shows no error box of
// its own. `as_text`: `file` opened as a .txt is, whatever its own type.
unsafe fn shell_run(verb: PCWSTR, file: &str, params: Option<&str>, as_text: bool, dir: &str) -> bool {
    let (file, dir) = (HSTRING::from(file), HSTRING::from(dir));
    let params = params.map(HSTRING::from);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | if as_text { SEE_MASK_CLASSNAME } else { 0 },
        lpVerb: verb,
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: params.as_ref().map_or(PCWSTR::null(), |p| PCWSTR(p.as_ptr())),
        lpDirectory: if dir.is_empty() { PCWSTR::null() } else { PCWSTR(dir.as_ptr()) },
        lpClass: if as_text { w!(".txt") } else { PCWSTR::null() },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    ShellExecuteExW(&mut info).is_ok()
}

// The command `verb` runs for files of type `ext` (".bat"), as registered.
unsafe fn assoc_command(ext: &str, verb: PCWSTR) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut n = buf.len() as u32;
    let hr = AssocQueryStringW(ASSOCF_INIT_IGNOREUNKNOWN | ASSOCF_NOTRUNCATE, ASSOCSTR_COMMAND, &HSTRING::from(ext), verb, Some(PWSTR(buf.as_mut_ptr())), &mut n);
    hr.is_ok().then(|| String::from_utf16_lossy(&buf[..buf.iter().position(|&c| c == 0).unwrap_or(0)]))
}

// An "edit" verb of its own, not one that is its "open" again (that would run a script).
unsafe fn has_edit_verb(ext: &str) -> bool {
    assoc_command(ext, w!("edit")).is_some_and(|e| !e.trim().is_empty() && assoc_command(ext, w!("open")).is_none_or(|o| !o.eq_ignore_ascii_case(&e)))
}

// Windows perceives the type as text (.txt, .log, .md, .csv … however the user's editor is set up).
unsafe fn text_type(ext: &str) -> bool {
    let (mut kind, mut flag) = (PERCEIVED::default(), 0u32);
    !ext.is_empty() && AssocGetPerceivedType(&HSTRING::from(ext), &mut kind, &mut flag, None).is_ok() && kind == PERCEIVED_TYPE_TEXT
}

// WM_EDITED on the main window: how many went to an editor (not whether it is up yet: that isn't known), or why
// none did, with a beep.
pub(crate) unsafe fn edited_msg(lp: LPARAM) {
    if lp.0 == 0 {
        return;
    }
    let e = *Box::from_raw(lp.0 as *mut Edited);
    match (e.opened, e.failed) {
        (0, Some(name)) => fail(Fail::AddrCantOpen, &name),
        (0, None) if e.online > 0 && e.binary == 0 => fail(Fail::EditOnline, ""),
        (0, None) => fail(Fail::EditNone, ""),
        // the rest were binary, online-only or taken by no editor
        (n, _) => set_note(edited_note(KO.load(Ordering::Relaxed), n, e.picked, e.total, e.picked.saturating_sub(n))),
    }
}

// Select the new file, then open the list's own label edit, as F2 does; true once the box is open.
// Polled: the view lists the file only a moment later. (SVSI_EDIT and the "rename" verb select but open no box here.)
unsafe fn select_rename(i: usize, full: &str) -> bool {
    let (Some(sv), Some(fv)) = (view_of::<IShellView>(i), view_of::<IFolderView2>(i)) else { return true };
    let list = view_list(&fv);
    let editing = || !list.0.is_null() && SendMessageW(list, LVM_GETEDITCONTROL, None, None).0 != 0;
    if editing() {
        return true;
    }
    let sel = fv.GetSelection(false).ok().filter(|a| a.GetCount().is_ok_and(|n| n == 1));
    let got = sel.and_then(|a| a.GetItemAt(0).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)));
    if !got.is_some_and(|p| p.eq_ignore_ascii_case(full)) {
        let mut pidl = std::ptr::null_mut();
        if SHParseDisplayName(&HSTRING::from(full), None, &mut pidl, 0, None).is_err() {
            return true; // gone again
        }
        // a DirectUI list has no label edit to send to: there the view's own SVSI_EDIT is all there is
        let edit = if list.0.is_null() { SVSI_EDIT.0 } else { 0 };
        let flags = edit | SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0;
        _ = sv.SelectItem(ILFindLastID(pidl), flags as u32);
        CoTaskMemFree(Some(pidl as _));
        return false;
    }
    if list.0.is_null() {
        return true;
    }
    let k = SendMessageW(list, LVM_GETNEXTITEM, Some(WPARAM(usize::MAX)), Some(LPARAM(LVNI_FOCUSED))).0;
    if k >= 0 {
        _ = SetFocus(Some(list)); // LVM_EDITLABEL wants the focus in the list
        SendMessageW(list, LVM_EDITLABELW, Some(WPARAM(k as usize)), None);
    }
    editing()
}

// ---------- which program is using this? (Restart Manager) ----------

pub(crate) struct Holder {
    pub(crate) proc: RM_UNIQUE_PROCESS,
    pub(crate) name: String,
    pub(crate) kind: RM_APP_TYPE,
}

// Programs holding any of the files open; None when Restart Manager can't tell.
pub(crate) unsafe fn holders(paths: &[String]) -> Option<Vec<Holder>> {
    let (mut sess, mut key) = (0u32, [0u16; CCH_RM_SESSION_KEY as usize + 1]);
    if RmStartSession(&mut sess, None, PWSTR(key.as_mut_ptr())) != ERROR_SUCCESS {
        return None;
    }
    let wide: Vec<HSTRING> = paths.iter().map(|p| HSTRING::from(p.as_str())).collect();
    let names: Vec<PCWSTR> = wide.iter().map(|h| PCWSTR(h.as_ptr())).collect();
    let mut out = None;
    if RmRegisterResources(sess, Some(&names), None, None) == ERROR_SUCCESS {
        let mut info = vec![RM_PROCESS_INFO::default(); 8];
        for _ in 0..4 {
            let (mut need, mut n, mut why) = (0u32, info.len() as u32, 0u32);
            match RmGetList(sess, &mut need, &mut n, Some(info.as_mut_ptr()), &mut why) {
                e if e == ERROR_MORE_DATA => info.resize(need as usize + 4, RM_PROCESS_INFO::default()),
                e if e == ERROR_SUCCESS => {
                    let list = info[..n as usize].iter().map(|p| {
                        let app = String::from_utf16_lossy(&p.strAppName[..p.strAppName.iter().position(|&c| c == 0).unwrap_or(0)]);
                        let exe = exe_name(p.Process.dwProcessId);
                        Holder { proc: p.Process, name: holder_line(&app, &exe, p.Process.dwProcessId), kind: p.ApplicationType }
                    });
                    out = Some(list.collect());
                    break;
                }
                _ => break,
            }
        }
    }
    _ = RmEndSession(sess);
    out
}

unsafe fn exe_name(pid: u32) -> String {
    let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
    let (mut buf, mut n) = ([0u16; 1024], 1024u32);
    let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
    _ = CloseHandle(p);
    let full = if ok { String::from_utf16_lossy(&buf[..n as usize]) } else { String::new() };
    full.rsplit('\\').next().unwrap_or_default().to_string()
}

// "Notepad  (notepad.exe, PID 1234)"
pub(crate) fn holder_line(app: &str, exe: &str, pid: u32) -> String {
    let app = if app.trim().is_empty() { exe } else { app.trim() };
    match exe {
        "" => format!("{app}  (PID {pid})"),
        e => format!("{app}  ({e}, PID {pid})"),
    }
}

// Never Grids itself, Explorer (the desktop and taskbar) or a critical system process.
pub(crate) fn may_close(kind: RM_APP_TYPE, pid: u32, me: u32) -> bool {
    pid != me && kind != RmExplorer && kind != RmCritical
}

// Visible top-level windows of a process, its main one first.
unsafe fn windows_of(pid: u32) -> Vec<HWND> {
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let (pid, out) = &mut *(lp.0 as *mut (u32, Vec<HWND>));
        let mut p = 0;
        GetWindowThreadProcessId(h, Some(&mut p));
        if p == *pid && IsWindowVisible(h).as_bool() && GetWindow(h, GW_OWNER).is_err() {
            out.push(h);
        }
        true.into()
    }
    let mut st = (pid, Vec::new());
    _ = EnumWindows(Some(each), LPARAM(&mut st as *mut _ as isize));
    st.1
}

// TaskDialogIndirect only exists in common controls v6 (our manifest): looked up at run time so a
// binary without the manifest (the unit tests) still loads.
pub(crate) unsafe fn task_dialog(cfg: &TASKDIALOGCONFIG) -> Option<(i32, i32)> {
    type Tdi = unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut BOOL) -> HRESULT;
    let f = GetProcAddress(LoadLibraryW(w!("comctl32.dll")).ok()?, s!("TaskDialogIndirect"))?;
    let (mut btn, mut radio) = (0, 0);
    let f: Tdi = std::mem::transmute(f);
    f(cfg, &mut btn, &mut radio, std::ptr::null_mut()).is_ok().then_some((btn, radio))
}

// "Which program is using this?" for files; also for later callers (say, after a sharing violation).
// Retry checks again, Switch brings the program forward, Close asks it politely to quit (never forced).
// Returns true when, at the end, no program holds the files any more.
pub(crate) unsafe fn who_uses(owner: HWND, paths: &[String]) -> bool {
    let title = HSTRING::from("Grids");
    let what = match paths {
        [one] => one.rsplit('\\').next().unwrap_or(one).to_string(),
        _ => tr("{n} items", "{n}개 항목").replace("{n}", &paths.len().to_string()),
    };
    let mut cfg = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        ..Default::default()
    };
    let head = HSTRING::from(what);
    loop {
        let Some(list) = holders(paths) else {
            MessageBoxW(Some(owner), &HSTRING::from(tr("Windows could not tell which programs use these files.", "어떤 프로그램이 쓰는지 Windows가 알려 주지 않았습니다.")), &title, MB_ICONWARNING);
            return false;
        };
        if list.is_empty() {
            let free = HSTRING::from(tr("No program is using it.", "사용 중인 프로그램이 없습니다."));
            cfg.pszMainInstruction = PCWSTR(head.as_ptr());
            cfg.pszContent = PCWSTR(free.as_ptr());
            cfg.Anonymous1.pszMainIcon = TD_INFORMATION_ICON;
            cfg.dwCommonButtons = TDCBF_CLOSE_BUTTON;
            (cfg.cButtons, cfg.cRadioButtons) = (0, 0);
            if task_dialog(&cfg).is_none() {
                MessageBoxW(Some(owner), &HSTRING::from(format!("{head}\n\n{free}")), &title, MB_ICONINFORMATION);
            }
            return true;
        }
        let several = list.len() > 1;
        let body = match several {
            true => tr("In use by these programs (pick one for Switch / Close):", "사용 중인 프로그램 (전환·닫기할 것을 고르세요):").to_string(),
            false => format!("{}\n{}", tr("In use by:", "사용 중인 프로그램:"), list[0].name),
        };
        let body = HSTRING::from(body);
        let labels = [
            (BTN_RETRY, tr("Retry", "다시 확인")),
            (BTN_SWITCH, tr("Switch to program", "프로그램으로 전환")),
            (BTN_CLOSE, tr("Close program", "프로그램 닫기")),
        ]
        .map(|(id, t)| (id, HSTRING::from(t)));
        let buttons = labels.each_ref().map(|(id, t)| TASKDIALOG_BUTTON { nButtonID: *id, pszButtonText: PCWSTR(t.as_ptr()) });
        // with several programs, a radio button picks the one Switch / Close act on
        let names: Vec<HSTRING> = list.iter().map(|h| HSTRING::from(h.name.as_str())).collect();
        let radios: Vec<TASKDIALOG_BUTTON> = names.iter().enumerate().map(|(k, t)| TASKDIALOG_BUTTON { nButtonID: RADIO + k as i32, pszButtonText: PCWSTR(t.as_ptr()) }).collect();
        cfg.pszMainInstruction = PCWSTR(head.as_ptr());
        cfg.pszContent = PCWSTR(body.as_ptr());
        cfg.Anonymous1.pszMainIcon = TD_WARNING_ICON;
        cfg.dwCommonButtons = TDCBF_CANCEL_BUTTON;
        (cfg.cButtons, cfg.pButtons, cfg.nDefaultButton) = (3, buttons.as_ptr(), BTN_RETRY);
        (cfg.cRadioButtons, cfg.pRadioButtons) = (if several { radios.len() as u32 } else { 0 }, radios.as_ptr());
        let Some((btn, radio)) = task_dialog(&cfg) else {
            let all = list.iter().map(|h| h.name.as_str()).collect::<Vec<_>>().join("\n");
            MessageBoxW(Some(owner), &HSTRING::from(format!("{head}\n\n{body}\n{all}")), &title, MB_ICONWARNING);
            return false;
        };
        let h = &list[((radio - RADIO).max(0) as usize).min(list.len() - 1)];
        let pid = h.proc.dwProcessId;
        match btn {
            BTN_RETRY => {}
            BTN_SWITCH => {
                let Some(&w) = windows_of(pid).first() else {
                    _ = MessageBeep(MB_ICONWARNING); // no window to show (a service, a hidden helper)
                    continue;
                };
                if IsIconic(w).as_bool() {
                    _ = ShowWindow(w, SW_RESTORE);
                }
                _ = SetForegroundWindow(w);
                return false;
            }
            BTN_CLOSE => {
                if !may_close(h.kind, pid, GetCurrentProcessId()) {
                    MessageBoxW(Some(owner), &HSTRING::from(tr("Grids does not close Windows Explorer, system processes or itself.", "Grids는 Windows 탐색기, 시스템 프로세스, 자기 자신은 닫지 않습니다.")), &title, MB_ICONWARNING);
                    continue;
                }
                let ask = HSTRING::from(format!("{}\n\n{}", tr("Ask this program to close? Unsaved work there may prompt you.", "이 프로그램에 닫기를 요청할까요? 저장하지 않은 작업이 있으면 그 프로그램이 물어봅니다."), h.name));
                if MessageBoxW(Some(owner), &ask, &title, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES {
                    continue;
                }
                close_politely(h);
                std::thread::sleep(Duration::from_millis(700)); // give it a moment, then the list is checked again
            }
            _ => return false,
        }
    }
}

// WM_CLOSE to its windows, as clicking × would; without windows, Restart Manager's shutdown request without the force flag.
unsafe fn close_politely(h: &Holder) {
    let wins = windows_of(h.proc.dwProcessId);
    for w in &wins {
        _ = PostMessageW(Some(*w), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
    if wins.is_empty() {
        let (mut sess, mut key) = (0u32, [0u16; CCH_RM_SESSION_KEY as usize + 1]);
        if RmStartSession(&mut sess, None, PWSTR(key.as_mut_ptr())) == ERROR_SUCCESS {
            if RmRegisterResources(sess, None, Some(&[h.proc]), None) == ERROR_SUCCESS {
                _ = RmShutdown(sess, 0, None); // 0: polite; RmForceShutdown is never used
            }
            _ = RmEndSession(sess);
        }
    }
}

// ---------- Compare with WinMerge ----------

// Where WinMerge may be, in order: its registry value (per user, per machine, 32-bit machine), then the usual folders.
pub(crate) fn winmerge_candidates(reg: &[Option<String>], roots: &[String]) -> Vec<String> {
    let clean = |s: &str| s.trim().trim_matches('"').trim().to_string();
    let from_reg = reg.iter().flatten().map(|s| clean(s));
    let from_dirs = roots.iter().map(|r| r.trim_end_matches('\\')).filter(|r| !r.is_empty()).map(|r| format!(r"{r}\WinMerge\WinMergeU.exe"));
    let mut out: Vec<String> = Vec::new();
    for c in from_reg.chain(from_dirs).filter(|c| !c.is_empty()) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&c)) {
            out.push(c);
        }
    }
    out
}

// /e: Esc closes it, /u: the paths stay out of WinMerge's recent list, /r: whole folder trees.
pub(crate) fn winmerge_args(left: &str, right: &str, folders: bool) -> String {
    format!("/e /u{} {} {}", if folders { " /r" } else { "" }, quote_arg(left), quote_arg(right))
}

unsafe fn reg_sz(root: HKEY, key: PCWSTR) -> Option<String> {
    let (mut buf, mut n) = ([0u16; 1024], 2048u32);
    RegGetValueW(root, key, w!("Executable"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n)).ok().ok()?;
    let s = String::from_utf16_lossy(&buf[..(n as usize / 2).min(1024)]);
    Some(s.trim_end_matches('\0').to_string())
}

// Looked up each time the ⇄ menu opens (cheap), never at startup.
pub(crate) unsafe fn winmerge_exe() -> Option<String> {
    let key = w!(r"Software\Thingamahoochie\WinMerge");
    let reg = [reg_sz(HKEY_CURRENT_USER, key), reg_sz(HKEY_LOCAL_MACHINE, key), reg_sz(HKEY_LOCAL_MACHINE, w!(r"Software\WOW6432Node\Thingamahoochie\WinMerge"))];
    let env = |k: &str| std::env::var(k).unwrap_or_default();
    let roots = [env("ProgramFiles"), env("ProgramFiles(x86)"), format!(r"{}\Programs", env("LOCALAPPDATA"))];
    winmerge_candidates(&reg, &roots).into_iter().find(|p| Path::new(p).is_file())
}

unsafe fn run_winmerge(exe: &str, left: &str, right: &str) {
    let folders = Path::new(left).is_dir() && Path::new(right).is_dir();
    let r = ShellExecuteW(None, w!("open"), &HSTRING::from(exe), &HSTRING::from(winmerge_args(left, right, folders)), PCWSTR::null(), SW_SHOWNORMAL);
    if r.0 as isize <= 32 {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

// ---------- "Open in Grids" in Explorer's right-click menu ----------
// Per user only (HKCU\Software\Classes): no admin rights. The registry is the only record, nothing in grids.ini.

const CTX_BASE: &str = r"Software\Classes";
// fixed names: only these three verbs are ever written or deleted
const CTX_KEYS: [&str; 3] = [r"Directory\shell\Grids", r"Directory\Background\shell\Grids", r"Drive\shell\Grids"];
// value on Grids' own keys: a "Grids" verb without it belongs to something else and is left alone
const CTX_MARK: &str = "GridsMenu";

/// The menu's command: folder mode (never a session), and `\.` after %V so a drive root "D:\"
/// does not end in `\"` and eat its closing quote. menu_args takes the `\.` off again.
pub(crate) fn ctx_command(exe: &str) -> String {
    format!(r#""{exe}" {FOLDER_FLAG} "%V\.""#)
}

/// True = the registered command runs this exe (paths compared without case).
pub(crate) fn ctx_owned(cmd: &str, exe: &str) -> bool {
    cmd.trim().to_lowercase() == ctx_command(exe).to_lowercase()
}

/// Arguments of a folder-mode launch lose the `\.` the menu command adds: "C:\x\." → "C:\x", "D:\\." → "D:\".
pub(crate) fn menu_args(mut args: Vec<String>) -> Vec<String> {
    if args.first().is_some_and(|a| a == FOLDER_FLAG) {
        for a in args.iter_mut().skip(1) {
            if let Some(p) = a.strip_suffix(r"\.") {
                *a = if p.len() == 2 && p.ends_with(':') { format!(r"{p}\") } else { p.to_string() };
            }
        }
    }
    args
}

unsafe fn reg_text(key: &str, name: Option<&str>) -> Option<String> {
    let (mut buf, mut n) = ([0u16; 1024], 2048u32);
    let name = name.map(HSTRING::from);
    let v = name.as_ref().map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr()));
    RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(key), v, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n)).ok().ok()?;
    Some(String::from_utf16_lossy(&buf[..(n as usize / 2).min(1024)]).trim_end_matches('\0').to_string())
}

unsafe fn reg_put(key: &str, name: Option<&str>, text: &str) -> bool {
    let data: Vec<u16> = text.encode_utf16().chain([0]).collect();
    let name = name.map(HSTRING::from);
    let v = name.as_ref().map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr()));
    RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), v, REG_SZ.0, Some(data.as_ptr() as _), (data.len() * 2) as u32).is_ok()
}

unsafe fn reg_has(key: &str) -> bool {
    let mut k = HKEY::default();
    let found = RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(key), None, KEY_READ, &mut k).is_ok();
    if found {
        _ = RegCloseKey(k);
    }
    found
}

// absent, or carrying Grids' mark
unsafe fn ctx_free(key: &str) -> bool {
    !reg_has(key) || reg_text(key, Some(CTX_MARK)).is_some()
}

// on = all three carry Grids' mark and run this exe (same test ctx_remove uses, so "on" can always be removed)
unsafe fn ctx_on_at(base: &str, exe: &str) -> bool {
    CTX_KEYS.iter().all(|k| {
        let key = format!(r"{base}\{k}");
        reg_text(&key, Some(CTX_MARK)).is_some() && reg_text(&format!(r"{key}\command"), None).is_some_and(|c| ctx_owned(&c, exe))
    })
}

unsafe fn ctx_write(base: &str, exe: &str, label: &str) -> bool {
    CTX_KEYS.iter().all(|k| {
        let key = format!(r"{base}\{k}");
        // the mark first, so a half-written key is still Grids' to clean up
        reg_put(&key, Some(CTX_MARK), "1")
            && reg_put(&key, None, label)
            && reg_put(&key, Some("Icon"), &format!(r#""{exe}",0"#))
            && reg_put(&format!(r"{key}\command"), None, &ctx_command(exe))
    })
}

// deletes Grids' own keys (a foreign "Grids" verb stays); true = none of Grids' is left
unsafe fn ctx_remove(base: &str) -> bool {
    CTX_KEYS.iter().fold(true, |ok, k| {
        let key = format!(r"{base}\{k}");
        let gone = !reg_has(&key) || reg_text(&key, Some(CTX_MARK)).is_none() || RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(key.as_str())).is_ok();
        ok && gone
    })
}

// All three or none: a failed half is undone (on: removed again; off: written back for this exe).
unsafe fn set_ctx_at(base: &str, exe: &str, label: &str, on: bool) -> bool {
    if on {
        if !CTX_KEYS.iter().all(|k| ctx_free(&format!(r"{base}\{k}"))) {
            return false; // another program's "Grids" verb: nothing touched
        }
        if ctx_write(base, exe, label) {
            return true;
        }
        ctx_remove(base);
        false
    } else {
        if ctx_remove(base) {
            return true;
        }
        ctx_write(base, exe, label);
        false
    }
}

/// Checked in the View menu only when the menu runs this exe: after moving Grids, one click registers the new path.
pub(crate) unsafe fn ctx_menu_on() -> bool {
    this_exe().is_some_and(|e| ctx_on_at(CTX_BASE, &e))
}

/// Adds or removes the three verbs (in the UI language) and tells Explorer. False = failed, nothing half-done.
pub(crate) unsafe fn set_ctx_menu(on: bool) -> bool {
    let Some(exe) = this_exe() else { return false };
    // the default verb must never point at a verb that is gone: off goes first, and if it can't, the verbs stay
    if !on && default_on_at(CTX_BASE) && !set_default_at(CTX_BASE, &exe, ctx_label(), false) {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None); // one key may have gone back already
        return false;
    }
    let ok = set_ctx_at(CTX_BASE, &exe, ctx_label(), on);
    // a failed rewrite (language switch) removed the verbs: folders go back to Explorer rather than to nothing
    if !ok && on && default_on_at(CTX_BASE) && !CTX_KEYS.iter().all(|k| reg_has(&format!(r"{CTX_BASE}\{k}"))) {
        set_default_at(CTX_BASE, &exe, ctx_label(), false);
    }
    SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    ok
}

// View menu click: flips what is really registered, with a note either way.
pub(crate) unsafe fn toggle_ctx_menu() {
    let on = !ctx_menu_on();
    let was_default = default_on_at(CTX_BASE); // off also turns "open folders in Grids" off: say so
    let ok = set_ctx_menu(on);
    if !ok {
        _ = MessageBeep(MB_ICONWARNING);
    }
    let text = match (ok, on) {
        (true, true) => tr(
            "\"Open in Grids\" added to Explorer's right-click menu (Windows 11: under Show more options)",
            "탐색기 오른쪽 클릭 메뉴에 \"Grids에서 열기\"를 넣음 (Windows 11: '더 많은 옵션 표시' 안)",
        ),
        (true, false) if was_default => tr(
            "\"Open in Grids\" removed from Explorer's right-click menu · folders open in Explorer again",
            "탐색기 오른쪽 클릭 메뉴에서 \"Grids에서 열기\"를 뺌 · 폴더도 다시 탐색기로 열림",
        ),
        (true, false) => tr("\"Open in Grids\" removed from Explorer's right-click menu", "탐색기 오른쪽 클릭 메뉴에서 \"Grids에서 열기\"를 뺌"),
        (false, true) => tr("Could not add \"Open in Grids\" to Explorer's menu", "탐색기 메뉴에 \"Grids에서 열기\"를 등록하지 못함"),
        (false, false) => tr("Could not remove \"Open in Grids\" from Explorer's menu", "탐색기 메뉴에서 \"Grids에서 열기\"를 빼지 못함"),
    };
    app(|a| a.note = Some((text.into(), Instant::now() + Duration::from_secs(3))));
    update_status();
}

fn ctx_label() -> &'static str {
    tr("Open in Grids", "Grids에서 열기")
}

// ---------- folders open in Grids instead of Explorer ----------
// (Directory Opus' Explorer Replacement, XYplorer's / Total Commander's "default file manager")
// The (Default) value of HKCU's Directory\shell and Drive\shell names the verb a double-click runs: "Grids" is
// the "Open in Grids" verb above. What was there before is kept beside it in DEF_PREV and put back on off.

const DEF_KEYS: [&str; 2] = [r"Directory\shell", r"Drive\shell"];
const DEF_VERB: &str = "Grids";
const DEF_PREV: &str = "GridsPrevDefault";

/// A (Default) Grids may take: none, empty, Windows' own "none", or Grids already. Anything else is another
/// file manager's and is left alone.
pub(crate) fn default_free(cur: Option<&str>) -> bool {
    cur.map_or(true, |v| v.is_empty() || v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case(DEF_VERB))
}

/// The program a verb's command runs: `"C:\x\grids.exe" --folder "%V\."` → `C:\x\grids.exe`.
pub(crate) fn command_exe(cmd: &str) -> Option<&str> {
    cmd.trim().strip_prefix('"')?.split('"').next().filter(|s| !s.is_empty())
}

// deletes one value (None: the key's (Default)); true = it is not there now
unsafe fn reg_unset(key: &str, name: Option<&str>) -> bool {
    let name = name.map(HSTRING::from);
    let v = name.as_ref().map_or(PCWSTR::null(), |s| PCWSTR(s.as_ptr()));
    let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), v);
    r.is_ok() || r == ERROR_FILE_NOT_FOUND || !reg_has(key)
}

// on = either key's (Default) is Grids: Grids' own panes then open folders themselves, and off clears what is left
unsafe fn default_on_at(base: &str) -> bool {
    DEF_KEYS.iter().any(|k| reg_text(&format!(r"{base}\{k}"), None).is_some_and(|v| v.eq_ignore_ascii_case(DEF_VERB)))
}

// one key to Grids, the value it had kept first (a second "on" keeps the first one's record)
unsafe fn def_take(key: &str) -> bool {
    let cur = reg_text(key, None);
    let mine = cur.as_deref().is_some_and(|v| v.eq_ignore_ascii_case(DEF_VERB));
    (if mine { reg_text(key, Some(DEF_PREV)).is_some() || reg_put(key, Some(DEF_PREV), "") } else { reg_put(key, Some(DEF_PREV), cur.as_deref().unwrap_or("")) })
        && reg_put(key, None, DEF_VERB)
}

// one key back: only while it still says Grids (another program's later choice stays); nothing kept → no value
unsafe fn def_give_back(key: &str) -> bool {
    if reg_text(key, None).is_some_and(|v| v.eq_ignore_ascii_case(DEF_VERB)) {
        let back = match reg_text(key, Some(DEF_PREV)).filter(|p| !p.is_empty()) {
            Some(p) => reg_put(key, None, &p),
            None => reg_unset(key, None),
        };
        if !back {
            return false;
        }
    }
    reg_unset(key, Some(DEF_PREV))
}

// all keys or none: a half that failed is given back
unsafe fn def_take_all(keys: &[String]) -> bool {
    if keys.iter().all(|k| def_take(k)) {
        return true;
    }
    for k in keys {
        def_give_back(k);
    }
    false
}

// On: "Open in Grids" for this exe first (the verb the default names), then both (Default)s, all or nothing;
// another program's default: nothing touched. Off: both back. True = done as asked.
unsafe fn set_default_at(base: &str, exe: &str, label: &str, on: bool) -> bool {
    let keys = DEF_KEYS.map(|k| format!(r"{base}\{k}"));
    if !on {
        return keys.iter().fold(true, |ok, k| def_give_back(k) && ok);
    }
    if !keys.iter().all(|k| default_free(reg_text(k, None).as_deref())) {
        return false;
    }
    let had_menu = CTX_KEYS.iter().all(|k| reg_text(&format!(r"{base}\{k}"), Some(CTX_MARK)).is_some());
    if !set_ctx_at(base, exe, label, true) {
        return false;
    }
    if def_take_all(&keys) {
        return true;
    }
    if !had_menu {
        set_ctx_at(base, exe, label, false);
    }
    false
}

// The default names Grids, but a Grids verb is gone or runs an exe that is not there (Grids moved or deleted).
// A Grids verb that still works, even another copy's, is left alone; so is a "Grids" verb that isn't Grids'.
unsafe fn default_stale_at(base: &str, exists: impl Fn(&str) -> bool) -> bool {
    default_on_at(base)
        && CTX_KEYS.iter().any(|k| {
            let key = format!(r"{base}\{k}");
            !reg_has(&key)
                || (reg_text(&key, Some(CTX_MARK)).is_some() && !reg_text(&format!(r"{key}\command"), None).as_deref().and_then(command_exe).is_some_and(&exists))
        })
}

// None: nothing to fix. Some(true): the verbs now run this exe. Some(false): could not, so folders went back to
// what opened them before (Explorer) rather than to a command that cannot run.
unsafe fn repair_default_at(base: &str, exe: &str, label: &str, exists: impl Fn(&str) -> bool) -> Option<bool> {
    if !default_stale_at(base, exists) {
        return None;
    }
    if set_ctx_at(base, exe, label, true) {
        return Some(true);
    }
    set_default_at(base, exe, label, false);
    Some(false)
}

/// Checked in the View menu, and asked by a pane at each double-click / Enter on an item: only the (Default)
/// value, so right after Grids was moved its own panes still never launch a path that is gone.
pub(crate) unsafe fn default_open_on() -> bool {
    default_on_at(CTX_BASE)
}

unsafe fn set_default_open(on: bool) -> bool {
    let Some(exe) = this_exe() else { return false };
    let ok = set_default_at(CTX_BASE, &exe, ctx_label(), on);
    SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    ok
}

/// Once at start: folders set to open in Grids whose grids.exe was moved or deleted get this exe; failing that,
/// Explorer opens them again and the status line says so.
pub(crate) unsafe fn repair_default_open() {
    let Some(exe) = this_exe() else { return };
    let Some(ok) = repair_default_at(CTX_BASE, &exe, ctx_label(), |p| Path::new(p).is_file()) else { return };
    SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    if !ok {
        _ = MessageBeep(MB_ICONWARNING);
        let text = tr(
            "Grids was moved and could not take folder opening over again: Explorer opens folders now",
            "Grids 위치가 바뀌어 폴더 열기를 다시 맡지 못함: 지금은 탐색기가 폴더를 엶",
        );
        app(|a| a.note = Some((text.into(), Instant::now() + Duration::from_secs(5))));
    }
}

// View menu click: on asks first (what changes, the exe it points to, turn off before deleting it).
pub(crate) unsafe fn toggle_default_open(owner: HWND) {
    let on = !default_open_on();
    if on {
        let exe = this_exe().unwrap_or_default();
        let title = HSTRING::from(tr("Open folders in Grids instead of Explorer?", "폴더를 탐색기 대신 Grids로 열까요?"));
        let body = HSTRING::from(
            [
                tr(
                    "Double-clicking a folder or drive on the desktop or in Explorer opens it in the active pane of Grids, also inside Explorer windows. For an Explorer window: right-click > Open in new window.",
                    "바탕 화면이나 탐색기에서 폴더·드라이브를 더블클릭하면 탐색기 창 안에서도 Grids의 지금 패널에서 열립니다. 탐색기 창이 필요하면 오른쪽 클릭 > 새 창에서 열기.",
                ),
                tr("\"Open in Grids\" is added to the right-click menu too. It runs:", "오른쪽 클릭 메뉴에 \"Grids에서 열기\"도 함께 들어갑니다. 실행할 파일:"),
                &exe,
                tr(
                    "Turn this off before you delete or move grids.exe. If you forget, run Grids again from anywhere and it is fixed.",
                    "grids.exe를 지우거나 옮기기 전에 이 항목을 끄세요. 잊었다면 Grids를 아무 위치에서나 다시 실행하면 고쳐집니다.",
                ),
            ]
            .join("\n\n"),
        );
        if !ask(owner, &title, &body, tr("Turn on", "켜기"), tr("Cancel", "취소")) {
            return;
        }
    }
    let ok = set_default_open(on);
    if !ok {
        _ = MessageBeep(MB_ICONWARNING);
    }
    let text = match (ok, on) {
        (true, true) => tr(
            "Folders now open in Grids · for an Explorer window: right-click > Open in new window",
            "탐색기에서 폴더를 더블클릭해도 Grids로 열립니다 · 탐색기 창은 오른쪽 클릭 > 새 창에서 열기",
        ),
        (true, false) => tr("Folders open in Explorer again", "폴더가 다시 탐색기로 열립니다"),
        (false, true) => tr(
            "Could not make folders open in Grids (another program may open them already)",
            "폴더를 Grids로 열도록 바꾸지 못함 (다른 프로그램이 이미 맡고 있을 수 있음)",
        ),
        (false, false) => tr("Could not give folder opening back to Explorer", "폴더 열기를 탐색기로 되돌리지 못함"),
    };
    app(|a| a.note = Some((text.into(), Instant::now() + Duration::from_secs(5))));
    update_status();
}

// ---------- ⇄ menu hooks ----------

// Adds "Which program is using this?" and, with WinMerge found, "Compare with WinMerge" to the ⇄ menu.
// others: (pane, menu label, folder it showed). Returns the WinMerge exe for shell_pick.
// no_dirs: none of the selected items is a folder (checksum, up to HASH_MAX, and date folders).
pub(crate) unsafe fn shell_menu(menu: HMENU, src: usize, picked: &std::result::Result<Vec<String>, Pick>, no_dirs: bool, others: &[(usize, HSTRING, String)]) -> Option<String> {
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let flags = if picked.is_ok() { MF_STRING } else { MF_STRING | MF_GRAYED };
    _ = AppendMenuW(menu, flags, ID_INUSE, &HSTRING::from(tr2(M_INUSE)));
    // SHA-256: up to HASH_MAX picked files, no folders; while one runs, the same place stops it
    let (flags, label) = match hash_busy() {
        Some(pct) => (MF_STRING, tr2(M_HASH_STOP).replace("{p}", &pct.to_string())),
        None => {
            let ok = no_dirs && picked.as_ref().is_ok_and(|p| p.len() <= HASH_MAX);
            (if ok { MF_STRING } else { MF_STRING | MF_GRAYED },tr2(M_HASH).to_string())
        }
    };
    _ = AppendMenuW(menu, flags, ID_HASH, &HSTRING::from(label));
    // before the WinMerge lookup: it ends this function early on a PC without WinMerge
    let here = app(|a| a.s.paths[src].clone());
    org_menu(menu, org_ok(picked, no_dirs, &here));
    let label = match picked {
        Ok(p) if group_ok(p, &here) => {
            let t = tr2(M_GROUP_N);
            (MF_STRING, t.replace("{n}", &p.len().to_string()))
        }
        _ => (MF_STRING | MF_GRAYED, tr2(M_GROUP).to_string()),
    };
    _ = AppendMenuW(menu, label.0, ID_GROUP, &HSTRING::from(label.1));
    let label = match picked {
        Ok(p) if group_ok(p, &here) => (MF_STRING, tr2(M_STAMP_N).replace("{n}", &p.len().to_string())),
        _ => (MF_STRING | MF_GRAYED, tr2(M_STAMP).to_string()),
    };
    _ = AppendMenuW(menu, label.0, ID_STAMP, &HSTRING::from(label.1));
    empty_menu(menu, is_fs(&here)); // whatever is selected
    let flags = if is_fs(&here) { MF_STRING } else { MF_STRING | MF_GRAYED };
    _ = AppendMenuW(menu, flags, ID_USAGE, &HSTRING::from(tr2(M_USAGE)));
    list_menu(menu, src); // the selection, or with none the whole view
    clip_menu(menu, picked); // before the WinMerge lookup too
    explorer_menu(menu, src);
    let exe = winmerge_exe()?;
    match picked {
        Ok(two) if two.len() == 2 => {
            _ = AppendMenuW(menu, MF_STRING, ID_WM_PAIR, &HSTRING::from(tr2(M_WM_PAIR)));
        }
        _ => {
            let here = is_fs(&app(|a| a.s.paths[src].clone()));
            if let Ok(sub) = CreatePopupMenu() {
                for (k, label, path) in others {
                    let flags = if here && is_fs(path) { MF_STRING } else { MF_STRING | MF_GRAYED };
                    _ = AppendMenuW(sub, flags, ID_WM_PANE + k, label);
                }
                let flags = if others.is_empty() { MF_POPUP | MF_GRAYED } else { MF_POPUP };
                _ = AppendMenuW(menu, flags, sub.0 as usize, &HSTRING::from(tr2(M_WM_PANE)));
            }
        }
    }
    Some(exe)
}

// ---------- open in Explorer ----------

pub(crate) const ID_EXPLORER: usize = 1154; // ⇄ menu (1150–1153 are the view menu's, 1138 stays unused)
const EXPLORER_MAX: u32 = 1000; // selected items handed over; Explorer slows down on more

// "Open in Explorer": grey only when the pane has no view yet; it says "(with the selection)" when that goes along.
unsafe fn explorer_menu(menu: HMENU, src: usize) {
    let fv = view_of::<IFolderView2>(src);
    let sel = fv.as_ref().and_then(|fv| fv.ItemCount(SVGIO_SELECTION).ok()).unwrap_or(0) > 0;
    let label = if sel && !default_open_on() { M_EXPLORER_SEL } else { M_EXPLORER };
    let flags = if fv.is_some() { MF_STRING } else { MF_STRING | MF_GRAYED };
    _ = AppendMenuW(menu, flags, ID_EXPLORER, &HSTRING::from(tr2(label)));
}

// The folder the pane shows now, by its ID list (This PC, the Recycle Bin, a library, a network place too), in an
// Explorer window with the pane's selection (up to EXPLORER_MAX, in view order) selected there. Only the folder,
// by its "Open in new window" verb ("explore" failing that), when nothing is selected, when the items aren't the
// folder's own (a library, search results), or when folders are set to open in Grids: Explorer's folder default
// would bring them back here. Not from inside app(): the shell may take a moment.
pub(crate) unsafe fn reveal_in_explorer(src: usize) {
    let name = app(|a| a.panes[src].as_ref().map_or_else(|| a.s.paths[src].clone(), |p| p.disp.clone()));
    let fv = view_of::<IFolderView2>(src);
    let dir = fv.as_ref().and_then(|fv| fv.GetFolder::<IShellItem>().ok()).and_then(|f| SHGetIDListFromObject(&f).ok());
    let (Some(fv), Some(dir)) = (fv, dir) else {
        fail(Fail::ExplorerCant, &name);
        return;
    };
    let mut kids: Vec<*mut ITEMIDLIST> = Vec::new();
    if !default_open_on() {
        let sel = fv.Items::<IShellItemArray>(_SVGIO(SVGIO_SELECTION.0 | SVGIO_FLAG_VIEWORDER.0)).or_else(|_| fv.GetSelection(false));
        if let Ok(arr) = sel {
            for k in 0..arr.GetCount().unwrap_or(0).min(EXPLORER_MAX) {
                if let Some(p) = arr.GetItemAt(k).ok().and_then(|it| SHGetIDListFromObject(&it).ok()) {
                    kids.push(p);
                }
            }
        }
    }
    // never with no items: SHOpenFolderAndSelectItems would open the parent with this folder selected
    let own = !kids.is_empty() && kids.iter().all(|&p| ILIsParent(dir, p, true).as_bool());
    let shown = own && {
        let last: Vec<*const ITEMIDLIST> = kids.iter().map(|&p| ILFindLastID(p) as *const ITEMIDLIST).collect();
        SHOpenFolderAndSelectItems(dir, Some(&last), 0).is_ok()
    };
    let shown = shown
        || ["opennewwindow", "explore"].iter().any(|v| {
            let verb = HSTRING::from(*v);
            let mut sei = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI,
                lpVerb: PCWSTR(verb.as_ptr()),
                lpIDList: dir as _,
                nShow: SW_SHOWNORMAL.0,
                ..Default::default()
            };
            ShellExecuteExW(&mut sei).is_ok()
        });
    for p in kids {
        CoTaskMemFree(Some(p as _));
    }
    CoTaskMemFree(Some(dir as _));
    if !shown {
        fail(Fail::ExplorerCant, &name);
    }
}

pub(crate) fn is_shell_pick(p: usize) -> bool {
    p == ID_INUSE || p == ID_WM_PAIR || p == ID_HASH || p == ID_GROUP || p == ID_STAMP || p == ID_EMPTY || p == ID_USAGE || p == ID_LIST || p == ID_CLIPADD || p == ID_EXPLORER || is_org_pick(p) || (ID_WM_PANE..ID_WM_PANE + MAXP).contains(&p)
}

pub(crate) unsafe fn shell_pick(h: HWND, src: usize, p: usize, picked: std::result::Result<Vec<String>, Pick>, exe: Option<String>, others: &[(usize, HSTRING, String)]) {
    match (p, picked, exe) {
        (ID_INUSE, Ok(paths), _) => _ = who_uses(h, &paths),
        (ID_HASH, picked, _) => hash_pick(picked),
        // the selection the menu counted, checked again against the folder the pane shows now
        (ID_GROUP, picked, _) => group_into_folder(src, picked),
        (ID_STAMP, picked, _) => start_stamp_copy(src, picked),
        (p, picked, _) if is_org_pick(p) => organize(src, p - ID_ORG, picked),
        // no selection needed: the folder the pane shows now, checked again there
        (ID_EMPTY, _, _) => remove_empty(src),
        (ID_USAGE, _, _) => open_usage(src),
        (ID_LIST, _, _) => copy_list(src), // the view's selection (or all its items), read again now
        (ID_CLIPADD, picked, _) => add_to_clip(picked), // the selection the menu counted
        (ID_EXPLORER, _, _) => reveal_in_explorer(src), // the pane's folder and selection, read again now
        (ID_WM_PAIR, Ok(two), Some(exe)) if two.len() == 2 => run_winmerge(&exe, &two[0], &two[1]),
        (p, _, Some(exe)) if p >= ID_WM_PANE => {
            // only the folder the menu showed for that pane
            let k = p - ID_WM_PANE;
            let (here, there) = app(|a| (a.s.paths[src].clone(), a.s.paths[k].clone()));
            match others.iter().find(|(i, ..)| *i == k) {
                Some((_, _, shown)) if *shown == there && is_fs(&here) && is_fs(&there) => run_winmerge(&exe, &here, &there),
                _ => _ = MessageBeep(MB_ICONWARNING),
            }
        }
        _ => _ = MessageBeep(MB_ICONWARNING),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A whole command line split by Windows' own CommandLineToArgvW; the first token is the program name.
    fn split(cmd: &str) -> Vec<String> {
        unsafe {
            let mut n = 0;
            let v = CommandLineToArgvW(&HSTRING::from(cmd), &mut n);
            let out: Vec<String> = (0..n as usize).map(|k| (*v.add(k)).to_string().unwrap()).collect();
            _ = windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(v as _)));
            out
        }
    }

    // The one quote_arg (jump list and WinMerge) read back by the real Windows rules, not a look-alike.
    #[test]
    fn quote_arg_matches_windows() {
        // "x " in front: the program name has its own rules, and an empty line returns this exe's path
        let one = |p: &str| split(&format!("x {}", quote_arg(p))).into_iter().skip(1).collect::<Vec<_>>();
        for p in [r"D:\", r"C:\a b\", r"\\srv\s", r"E:\x\\", r#"a"b"#, r#"C:\q\"x"#, "", r"C:\한글 폴더", r"C:\x\\\"] {
            assert_eq!(one(p), [p], "{p}");
        }
        assert_eq!(split(&format!("x {}", winmerge_args(r"D:\", r"C:\a b\", true)))[1..], ["/e", "/u", "/r", r"D:\", r"C:\a b\"]);
    }

    #[test]
    fn new_text_file_names() {
        assert_eq!(fresh_name("New Text Document.txt", |_| false), "New Text Document.txt");
        assert_eq!(fresh_name("New Text Document.txt", |n| n == "New Text Document.txt"), "New Text Document (2).txt");
        let taken = ["새 텍스트 문서.txt", "새 텍스트 문서 (2).txt"];
        assert_eq!(fresh_name("새 텍스트 문서.txt", |n| taken.contains(&n)), "새 텍스트 문서 (3).txt");
        assert_eq!(fresh_name("noext", |n| n == "noext"), "noext (2)");
        assert_eq!(fresh_name(".txt", |n| n == ".txt"), ".txt (2)");
        assert_eq!(fresh_name("a.txt", |_| true), ""); // all taken: nothing, never an existing name
    }

    #[test]
    fn group_folder_names() {
        let nf = "New folder";
        // one item: its name without the extension
        assert_eq!(group_base(&["Report 2026.docx"], nf), "Report 2026");
        assert_eq!(group_base(&["photos"], nf), "photos");
        assert_eq!(group_base(&["archive.tar.gz"], nf), "archive.tar");
        // several: the shared start, trailing separators cut
        assert_eq!(group_base(&["Trip - day 1.jpg", "Trip - day 2.jpg", "Trip - map.png"], nf), "Trip");
        assert_eq!(group_base(&["IMG_2031.jpg", "IMG_2045.jpg"], nf), "IMG_20");
        assert_eq!(group_base(&["invoice (1).pdf", "invoice (2).pdf"], nf), "invoice");
        // too short a start, or none: the fallback
        assert_eq!(group_base(&["ab1.txt", "ab2.txt"], nf), nf);
        assert_eq!(group_base(&["a.txt", "b.txt"], nf), nf);
        assert_eq!(group_base(&["--_.x", "--_.y"], nf), nf);
        assert_eq!(group_base(&[], nf), nf);
        // Korean names: compared by character, not byte
        assert_eq!(group_base(&["회의록 1월.hwp", "회의록 2월.hwp"], "새 폴더"), "회의록");
        assert_eq!(group_base(&["사진1.jpg", "사진2.jpg"], "새 폴더"), "새 폴더");
        // a name Windows reserves: the fallback, from one item or a shared start
        assert_eq!(group_base(&["Con.txt"], nf), nf);
        assert_eq!(group_base(&["con.a", "con.b"], nf), nf);
        assert_eq!(group_base(&["Config.ini", "Console.log"], nf), nf);
        assert_eq!(group_base(&["Console.log"], nf), "Console");
        // a taken name gets a number at the end, never before a '.'
        assert_eq!(group_name("Trip", 1), "Trip");
        assert_eq!(group_name("Trip", 2), "Trip (2)");
        assert_eq!(group_name("archive.tar", 3), "archive.tar (3)");
        // only items that sit right in the pane's folder
        let srcs = [r"C:\w\a.txt".to_string(), r"C:\w\b.txt".into()];
        assert!(group_ok(&srcs, r"C:\w") && group_ok(&srcs, r"c:\W\"));
        assert!(!group_ok(&srcs, r"C:\x") && !group_ok(&[r"C:\w\a.txt".into(), r"C:\x\b.txt".into()], r"C:\w") && !group_ok(&[], r"C:\w"));
        assert!(is_shell_pick(ID_GROUP));
    }

    #[test]
    fn dated_copy_names() {
        let t = SYSTEMTIME { wYear: 2026, wMonth: 11, wDay: 27, wHour: 14, wMinute: 30, wSecond: 59, ..Default::default() };
        assert_eq!(stamp_name("Report.docx", false, &t, 1), "Report (2026-11-27 14.30).docx");
        assert_eq!(stamp_name("보고서.hwp", false, &t, 1), "보고서 (2026-11-27 14.30).hwp");
        assert_eq!(stamp_name("archive.tar.gz", false, &t, 1), "archive.tar (2026-11-27 14.30).gz");
        assert_eq!(stamp_name("README", false, &t, 1), "README (2026-11-27 14.30)");
        assert_eq!(stamp_name(".gitignore", false, &t, 1), ".gitignore (2026-11-27 14.30)");
        assert_eq!(stamp_name("v1.2", true, &t, 1), "v1.2 (2026-11-27 14.30)"); // a folder's '.' is no extension
        // a dated copy stamped again, or a '.' followed by a space: never into the middle of the time
        let again = "README (2026-11-27 14.30) (2026-11-27 14.30)";
        assert_eq!(stamp_name("README (2026-11-27 14.30)", false, &t, 1), again);
        assert_eq!(stamp_name(".gitignore (2026-11-27 14.30)", false, &t, 1), ".gitignore (2026-11-27 14.30) (2026-11-27 14.30)");
        assert_eq!(stamp_name("Report (2026-11-27 14.30).docx", false, &t, 1), "Report (2026-11-27 14.30) (2026-11-27 14.30).docx");
        assert_eq!(stamp_name("회의 3.5 메모", false, &t, 1), "회의 3.5 메모 (2026-11-27 14.30)");
        assert_eq!(stamp_free("README (2026-11-27 14.30)", false, &t, |n| n == again).as_deref(), Some("README (2026-11-27 14.30) (2026-11-27 14.30) (2)"));
        // one-digit month, day, hour and minute padded; midnight is 00.00
        let early = SYSTEMTIME { wYear: 2027, wMonth: 1, wDay: 5, wHour: 0, wMinute: 0, ..Default::default() };
        assert_eq!(stamp_name("a.txt", false, &early, 1), "a (2027-01-05 00.00).txt");
        assert!(!stamp_name("a.txt", false, &t, 1).contains(':'));
        // again within the minute: the number after the stamp, before the extension; at the end otherwise
        for (name, dir, first, second) in [
            ("Report.docx", false, "Report (2026-11-27 14.30).docx", "Report (2026-11-27 14.30) (2).docx"),
            ("v1.2", true, "v1.2 (2026-11-27 14.30)", "v1.2 (2026-11-27 14.30) (2)"),
            ("README", false, "README (2026-11-27 14.30)", "README (2026-11-27 14.30) (2)"),
            (".gitignore", false, ".gitignore (2026-11-27 14.30)", ".gitignore (2026-11-27 14.30) (2)"),
        ] {
            assert_eq!(stamp_free(name, dir, &t, |n| n == first).as_deref(), Some(second), "{name}");
        }
        let two = ["a (2026-11-27 14.30).txt", "a (2026-11-27 14.30) (2).txt"];
        assert_eq!(stamp_free("a.txt", false, &t, |n| two.contains(&n)).as_deref(), Some("a (2026-11-27 14.30) (3).txt"));
        assert_eq!(stamp_free("a.txt", false, &t, |_| true), None); // all taken: nothing, never an existing name
        // the status line counts only a run with nothing skipped
        assert_eq!(stamped_note(false, 1, "a (2026-11-27 14.30).txt", false), "Made a dated copy: a (2026-11-27 14.30).txt");
        assert_eq!(stamped_note(false, 3, "x", false), "Made 3 dated copies");
        assert_eq!(stamped_note(true, 3, "x", false), "날짜 붙인 사본 3개 만듦");
        assert!(!stamped_note(false, 3, "x", true).contains('3') && !stamped_note(true, 3, "x", true).contains('3'));
        assert!(is_shell_pick(ID_STAMP) && open_in_pick(ID_STAMP) == OpenPick::Shell);
    }

    #[test]
    fn in_use_lines_and_close_rules() {
        assert_eq!(holder_line("Notepad", "notepad.exe", 42), "Notepad  (notepad.exe, PID 42)");
        assert_eq!(holder_line("  ", "pwsh.exe", 7), "pwsh.exe  (pwsh.exe, PID 7)");
        assert_eq!(holder_line("Svc", "", 9), "Svc  (PID 9)");
        assert!(may_close(RmMainWindow, 10, 1) && may_close(RmConsole, 10, 1) && may_close(RmService, 10, 1));
        assert!(!may_close(RmExplorer, 10, 1) && !may_close(RmCritical, 10, 1) && !may_close(RmMainWindow, 1, 1));
    }

    #[test]
    fn winmerge_lookup_order() {
        let reg = [None, Some(r#""D:\Tools\WinMerge\WinMergeU.exe""#.to_string()), Some(r"c:\program files\winmerge\winmergeu.exe".into())];
        let roots = [r"C:\Program Files".to_string(), r"C:\Program Files (x86)\".into(), String::new()];
        assert_eq!(
            winmerge_candidates(&reg, &roots),
            [r"D:\Tools\WinMerge\WinMergeU.exe", r"c:\program files\winmerge\winmergeu.exe", r"C:\Program Files (x86)\WinMerge\WinMergeU.exe"]
        );
        assert!(winmerge_candidates(&[None, None], &[]).is_empty());
    }

    #[test]
    fn explorer_menu_command() {
        let exe = r"C:\Program Files\My Tools\grids.exe";
        assert_eq!(ctx_command(exe), r#""C:\Program Files\My Tools\grids.exe" --folder "%V\.""#);
        assert!(ctx_owned(&ctx_command(exe), exe));
        assert!(ctx_owned(&format!("  {}  ", ctx_command(&exe.to_uppercase())), exe)); // case and stray spaces
        assert!(!ctx_owned(&ctx_command(r"D:\old\grids.exe"), exe)); // moved exe: not checked, one click re-registers
        assert!(!ctx_owned(r#""C:\Program Files\My Tools\grids.exe" "%V""#, exe));
        // what Explorer passes after %V is filled in, split by Windows' own rules, then cleaned and resolved
        let run = |v: &str| menu_args(split(&ctx_command(exe).replace("%V", v)).into_iter().skip(1).collect());
        assert_eq!(run(r"D:\"), [FOLDER_FLAG, r"D:\"]);
        assert_eq!(run(r"C:\Users\Public"), [FOLDER_FLAG, r"C:\Users\Public"]);
        assert_eq!(run(r"\\srv\share\a b"), [FOLDER_FLAG, r"\\srv\share\a b"]);
        assert_eq!(resolve(run(r"C:\Users\Public")[1].clone()), r"C:\Users\Public");
        assert_eq!(resolve(run(r"C:\")[1].clone()), r"C:\");
        // only folder mode is touched
        assert_eq!(menu_args(vec![r"C:\x\.".into()]), [r"C:\x\."]);
    }

    #[test]
    fn explorer_menu_registry_round_trip() {
        // a scratch branch of HKCU, never the real Software\Classes
        let (root, base) = (r"Software\GridsTest", r"Software\GridsTest\Classes");
        let exe = r"C:\Tools Dir\grids.exe";
        unsafe {
            _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(root));
            assert!(!ctx_on_at(base, exe));
            assert!(set_ctx_at(base, exe, "Open in Grids", true));
            assert!(ctx_on_at(base, exe) && !ctx_on_at(base, r"D:\other\grids.exe"));
            for k in CTX_KEYS {
                let key = format!(r"{base}\{k}");
                assert_eq!(reg_text(&key, None).as_deref(), Some("Open in Grids"));
                assert_eq!(reg_text(&key, Some("Icon")).as_deref(), Some(r#""C:\Tools Dir\grids.exe",0"#));
                assert_eq!(reg_text(&format!(r"{key}\command"), None), Some(ctx_command(exe)));
            }
            // moved exe: the marked keys are rewritten in place
            assert!(set_ctx_at(base, r"D:\new\grids.exe", "Grids에서 열기", true));
            assert!(ctx_on_at(base, r"D:\new\grids.exe") && !ctx_on_at(base, exe));
            assert!(set_ctx_at(base, exe, "x", false));
            assert!(CTX_KEYS.iter().all(|k| !reg_has(&format!(r"{base}\{k}"))));
            assert!(reg_has(&format!(r"{base}\Directory\shell"))); // only the Grids verbs go
            // someone else's "Grids" verb (no mark): neither taken over nor deleted
            let foreign = format!(r"{base}\{}", CTX_KEYS[2]);
            assert!(reg_put(&foreign, None, "theirs"));
            assert!(!set_ctx_at(base, exe, "Open in Grids", true));
            assert!(!reg_has(&format!(r"{base}\{}", CTX_KEYS[0])));
            assert!(set_ctx_at(base, exe, "x", false));
            assert_eq!(reg_text(&foreign, None).as_deref(), Some("theirs"));
            // same command but no mark: not "on" (off could not delete it either)
            for k in CTX_KEYS {
                assert!(reg_put(&format!(r"{base}\{k}\command"), None, &ctx_command(exe)));
            }
            assert!(!ctx_on_at(base, exe));
            _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(root));
        }
    }

    #[test]
    fn folders_open_in_grids_round_trip() {
        assert!(default_free(None) && default_free(Some("")) && default_free(Some("none")) && default_free(Some("grids")));
        assert!(!default_free(Some("Opus")) && !default_free(Some("open")));
        assert_eq!(command_exe(&ctx_command(r"C:\Tools Dir\grids.exe")), Some(r"C:\Tools Dir\grids.exe"));
        assert_eq!(command_exe(r#"  "D:\g.exe" x"#), Some(r"D:\g.exe"));
        assert_eq!(command_exe(r"D:\g.exe x"), None);
        assert_eq!(command_exe(r#""""#), None);
        // a scratch branch of HKCU, never the real Software\Classes
        let (root, base) = (r"Software\GridsTestDef", r"Software\GridsTestDef\Classes");
        let exe = r"C:\Tools Dir\grids.exe";
        let (dir, drive) = (format!(r"{base}\{}", DEF_KEYS[0]), format!(r"{base}\{}", DEF_KEYS[1]));
        unsafe {
            let fresh = || _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(root));
            fresh();
            // nothing there before (the usual HKCU): on, then off leaves no (Default) and no record
            assert!(!default_on_at(base));
            assert!(set_default_at(base, exe, "Open in Grids", true));
            assert!(default_on_at(base) && ctx_on_at(base, exe)); // the verb the default names is there
            for k in [&dir, &drive] {
                assert_eq!(reg_text(k, None).as_deref(), Some(DEF_VERB));
                assert_eq!(reg_text(k, Some(DEF_PREV)).as_deref(), Some(""));
            }
            assert!(set_default_at(base, exe, "Open in Grids", true)); // twice: the first record stays
            assert_eq!(reg_text(&dir, Some(DEF_PREV)).as_deref(), Some(""));
            assert!(set_default_at(base, exe, "x", false));
            assert!(!default_on_at(base));
            for k in [&dir, &drive] {
                assert!(reg_has(k)); // the shell key itself stays: other verbs live in it
                assert_eq!(reg_text(k, None), None);
                assert_eq!(reg_text(k, Some(DEF_PREV)), None);
            }
            assert!(ctx_on_at(base, exe)); // the right-click item stays until it is turned off itself
            // a value that was there ("none") comes back as it was
            fresh();
            assert!(reg_put(&dir, None, "none"));
            assert!(set_default_at(base, exe, "Open in Grids", true));
            assert!(set_default_at(base, exe, "x", false));
            assert_eq!(reg_text(&dir, None).as_deref(), Some("none"));
            assert_eq!(reg_text(&drive, None), None);
            // another file manager is the default: nothing touched, not even the right-click item
            fresh();
            assert!(reg_put(&drive, None, "Opus"));
            assert!(!set_default_at(base, exe, "Open in Grids", true));
            assert_eq!(reg_text(&drive, None).as_deref(), Some("Opus"));
            assert_eq!(reg_text(&dir, None), None);
            assert!(!reg_has(&format!(r"{base}\{}", CTX_KEYS[0])));
            // changed by another program while on: off leaves its choice and drops only the record
            fresh();
            assert!(set_default_at(base, exe, "Open in Grids", true));
            assert!(reg_put(&drive, None, "Opus"));
            assert!(set_default_at(base, exe, "x", false));
            assert_eq!(reg_text(&drive, None).as_deref(), Some("Opus"));
            assert_eq!(reg_text(&drive, Some(DEF_PREV)), None);
            assert_eq!(reg_text(&dir, None), None);
            // half done (the second key can't be written: a name over 255 characters): the first is given back
            fresh();
            assert!(reg_put(&dir, None, "none"));
            let bad = format!(r"{base}\{}", "x".repeat(300));
            assert!(!def_take_all(&[dir.clone(), bad]));
            assert_eq!(reg_text(&dir, None).as_deref(), Some("none"));
            assert_eq!(reg_text(&dir, Some(DEF_PREV)), None);
            // Grids moved and its verbs can't be written again (another "Grids" verb took one key): folders go back
            fresh();
            assert!(set_default_at(base, exe, "Open in Grids", true));
            let theirs = format!(r"{base}\{}", CTX_KEYS[1]);
            _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(theirs.as_str()));
            assert!(reg_put(&theirs, None, "theirs"));
            assert_eq!(repair_default_at(base, r"D:\new\grids.exe", "Open in Grids", |_| false), Some(false));
            assert!(!default_on_at(base) && reg_text(&dir, None).is_none());
            assert_eq!(reg_text(&theirs, None).as_deref(), Some("theirs"));
            fresh();
            assert!(set_default_at(base, exe, "Open in Grids", true));
            // Grids moved: the start rewrites the verbs for the new exe; a copy that still works is left alone
            let moved = r"D:\new\grids.exe";
            assert_eq!(repair_default_at(base, moved, "Open in Grids", |p| p == exe), None);
            assert!(ctx_on_at(base, exe));
            assert_eq!(repair_default_at(base, moved, "Open in Grids", |p| p == moved), Some(true));
            assert!(ctx_on_at(base, moved) && default_on_at(base));
            // a verb gone while the default still names it: written again
            _ = RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(format!(r"{base}\{}", CTX_KEYS[2]).as_str()));
            assert_eq!(repair_default_at(base, moved, "Open in Grids", |_| true), Some(true));
            assert!(ctx_on_at(base, moved));
            // not on: nothing to repair
            assert!(set_default_at(base, moved, "x", false));
            assert_eq!(repair_default_at(base, exe, "Open in Grids", |_| false), None);
            assert!(ctx_on_at(base, moved));
            fresh();
        }
    }

    #[test]
    fn winmerge_command_line() {
        assert_eq!(quote_arg(r"C:\a b\x.txt"), r#""C:\a b\x.txt""#);
        assert_eq!(quote_arg(r"C:\"), r#""C:\\""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(winmerge_args(r"C:\", r"D:\x y", true), r#"/e /u /r "C:\\" "D:\x y""#);
        assert_eq!(winmerge_args("a.txt", "b.txt", false), r#"/e /u "a.txt" "b.txt""#);
        assert!(is_shell_pick(ID_INUSE) && is_shell_pick(ID_HASH) && is_shell_pick(ID_WM_PANE + 15) && !is_shell_pick(ID_WM_PANE + 16) && !is_shell_pick(301));
    }
}
