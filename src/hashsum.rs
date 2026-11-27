use crate::*;
use std::io::Read;
use std::os::windows::fs::MetadataExt;
use windows::Win32::Security::Cryptography::{BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptHashData, BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE};
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::System::Memory::GlobalSize;
use windows::Win32::UI::Controls::{TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDCBF_CLOSE_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW, TD_INFORMATION_ICON, TD_WARNING_ICON};

// ---------- ⇄ menu: SHA-256 of the selected files, checked against a hash copied to the clipboard ----------
// One long-lived worker thread takes the requests in turn, so stopping and restarting during a slow network read
// never piles up threads; HASH_GEN moves on with every new request or "stop", and an older run notices within
// 1 MiB and quits (a request already replaced by the time it comes up is skipped). The clipboard is read once when the item is picked, written only by the result box's Copy.
// A lone hash on the clipboard is checked only when one file is selected; a "hash  name" list is checked by name.

pub(crate) const ID_HASH: usize = 1132;
pub(crate) const WM_HASH: u32 = WM_APP + 50; // wParam = gen, lParam = 1 finished / 0 progress
pub(crate) const HASH_MAX: usize = 50;
const BTN_COPY: i32 = 3150;
const CHUNK: usize = 1 << 20;
const CLIP_MAX: usize = 1 << 20; // chars of clipboard text looked at

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Why {
    Folder,
    Online,  // cloud placeholder: reading it would download it
    Changed, // size or time moved while it was read
    Gone,
    Denied,
    Locked,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Got {
    Sum(String), // lowercase hex
    Skip(Why),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mark {
    Same,
    Differs,
    NotListed, // the clipboard list has no line for this name
    Unsure,    // could not be checked (read failed, online only, or the list gives that name two values)
    Clash,     // another selected file has the same name, so a "hash  name" line can't tell them apart
    Plain,     // nothing on the clipboard to check against
}

// What the clipboard holds: "hash  name" lines (also "hash *name", "SHA256 (name) = hash"), else bare 64-hex tokens.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct ClipSums {
    pub(crate) named: Vec<(String, String)>, // (hash, file name without folders)
    pub(crate) lone: Vec<String>,
}

fn is_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(crate) fn base_name(s: &str) -> &str {
    s.rsplit(['/', '\\']).next().unwrap_or(s)
}

pub(crate) fn clip_hashes(text: &str) -> ClipSums {
    let mut out = ClipSums::default();
    for l in text.lines().map(str::trim) {
        let bsd = l.strip_prefix("SHA256 (").or_else(|| l.strip_prefix("SHA2-256 (")).and_then(|r| r.rsplit_once(") = "));
        let named = bsd.map(|(n, h)| (h.trim(), n)).or_else(|| l.split_once(char::is_whitespace).map(|(h, n)| (h, n.trim().trim_start_matches('*'))));
        match named {
            Some((h, n)) if is_sha(h) && !n.is_empty() => {
                out.named.push((h.to_ascii_lowercase(), base_name(n.trim_start_matches("./")).to_string()));
            }
            _ => {
                // "SHA256: AB12…", "Checksum ab12…": any 64-hex run with no letter or digit glued on (not a SHA-512)
                let toks = l.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| is_sha(t)).map(|t| t.to_ascii_lowercase());
                out.lone.extend(toks);
            }
        }
    }
    out
}

// names: the selected files' names, rows: what the worker got for each.
pub(crate) fn marks(names: &[String], rows: &[Got], clip: &ClipSums) -> Vec<Mark> {
    let one = names.len() == 1;
    let clash = |name: &String| names.iter().filter(|n| n.eq_ignore_ascii_case(name)).count() > 1;
    names
        .iter()
        .zip(rows)
        .map(|(name, got)| {
            let want: Vec<&str> = match clip.named.is_empty() {
                false => {
                    let mut v: Vec<&str> = clip.named.iter().filter(|(_, n)| n.eq_ignore_ascii_case(name)).map(|(h, _)| h.as_str()).collect();
                    v.dedup();
                    if v.is_empty() {
                        return Mark::NotListed;
                    }
                    if v.iter().any(|h| *h != v[0]) {
                        return Mark::Unsure; // the list gives this name two different values
                    }
                    if clash(name) {
                        return Mark::Clash; // same name picked from two folders: the one line would cover both
                    }
                    v
                }
                true if one && !clip.lone.is_empty() => clip.lone.iter().map(String::as_str).collect(),
                true => return Mark::Plain,
            };
            match got {
                Got::Sum(s) if want.contains(&s.as_str()) => Mark::Same,
                Got::Sum(_) => Mark::Differs,
                Got::Skip(_) => Mark::Unsure,
            }
        })
        .collect()
}

pub(crate) fn verdict(names: &[String], marks: &[Mark]) -> String {
    let n = names.len();
    let count = |m: Mark| marks.iter().filter(|&&x| x == m).count();
    let (same, bad) = (count(Mark::Same), count(Mark::Differs));
    let one = |en: &'static str, ko: &'static str| format!("{}: {}", tr(en, ko), names.first().map(String::as_str).unwrap_or(""));
    let many = |en: &'static str, ko: &'static str, k: usize| tr(en, ko).replace("{k}", &k.to_string()).replace("{n}", &n.to_string());
    if count(Mark::Plain) == n {
        return if n == 1 { one("SHA-256", "SHA-256") } else { many("Checksums of {n} files", "체크섬 {n}개", 0) };
    }
    match (n, same, bad) {
        (1, 1, _) => one("✓ Same as the clipboard's SHA-256", "✓ 클립보드의 SHA-256과 같습니다"),
        (1, _, 1) => one("✗ Differs from the clipboard's SHA-256", "✗ 클립보드의 SHA-256과 다릅니다"),
        (_, _, b) if b > 0 => many("✗ {k} of {n} files differ from the clipboard's values", "✗ {n}개 중 {k}개가 클립보드의 값과 다릅니다", b),
        (_, s, _) if s == n => many("✓ All {n} files match the clipboard's values", "✓ {n}개 모두 클립보드의 값과 같습니다", 0),
        (_, s, _) if s > 0 => many("✓ {k} of {n} files match the clipboard · the rest could not be checked", "✓ {n}개 중 {k}개가 같습니다 · 나머지는 확인하지 못했습니다", s),
        _ if count(Mark::NotListed) > 0 => tr("✗ No file matches the clipboard's values", "✗ 클립보드의 값과 맞는 파일이 없습니다").to_string(),
        _ => tr("? Could not check against the clipboard", "? 클립보드의 값과 대조하지 못했습니다").to_string(),
    }
}

// sha256sum's own format ("hash  name"), finished files only: `sha256sum -c` and update.rs sum_for read it back.
// LF only: macOS / Linux `sha256sum -c` would take a CR as part of the file name.
pub(crate) fn copy_lines(names: &[String], rows: &[Got]) -> String {
    names.iter().zip(rows).filter_map(|(n, g)| if let Got::Sum(s) = g { Some(format!("{s}  {n}")) } else { None }).collect::<Vec<_>>().join("\n")
}

fn why_text(w: &Why) -> &'static str {
    match w {
        Why::Folder => tr("folder, skipped", "폴더라 건너뜀"),
        Why::Online => tr("online-only, skipped (not downloaded)", "온라인 전용이라 건너뜀 (내려받지 않음)"),
        Why::Changed => tr("changed while it was read, can't be checked", "읽는 동안 바뀌어 확인할 수 없음"),
        Why::Gone => tr("no longer there", "파일이 없어짐"),
        Why::Denied => tr("access denied", "접근 거부"),
        Why::Locked => tr("locked by another program", "다른 프로그램이 잠금"),
        Why::Failed => tr("could not be read", "읽지 못함"),
    }
}

// ---------- hashing ----------

struct Sha(BCRYPT_HASH_HANDLE);

impl Sha {
    fn new() -> Option<Sha> {
        let mut h = BCRYPT_HASH_HANDLE::default();
        unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut h, None, None, 0) }.is_ok().then_some(Sha(h))
    }
    fn data(&mut self, b: &[u8]) -> bool {
        unsafe { BCryptHashData(self.0, b, 0) }.is_ok()
    }
    fn hex(self) -> Option<String> {
        let mut out = [0u8; 32];
        unsafe { BCryptFinishHash(self.0, &mut out, 0) }.is_ok().then(|| out.iter().map(|b| format!("{b:02x}")).collect())
    }
}

impl Drop for Sha {
    fn drop(&mut self) {
        unsafe { _ = BCryptDestroyHash(self.0) };
    }
}

fn io_why(e: &std::io::Error) -> Why {
    match (e.kind(), e.raw_os_error()) {
        (std::io::ErrorKind::NotFound, _) => Why::Gone,
        (_, Some(32 | 33)) => Why::Locked, // sharing / lock violation
        (std::io::ErrorKind::PermissionDenied, _) => Why::Denied,
        _ => Why::Failed,
    }
}

// FILE_ATTRIBUTE_OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS: OneDrive & co. would download it on read
pub(crate) const ONLINE_ONLY: u32 = 0x1000 | 0x4_0000 | 0x40_0000;

// None = cancelled. `each` gets the bytes read so far.
pub(crate) fn hash_file(path: &str, stop: &dyn Fn() -> bool, each: &mut dyn FnMut(u64)) -> Option<Got> {
    let before = match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => return Some(Got::Skip(Why::Folder)),
        Ok(m) if m.file_attributes() & ONLINE_ONLY != 0 => return Some(Got::Skip(Why::Online)),
        Ok(m) => m,
        Err(e) => return Some(Got::Skip(io_why(&e))),
    };
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => return Some(Got::Skip(io_why(&e))),
    };
    let Some(mut sha) = Sha::new() else { return Some(Got::Skip(Why::Failed)) };
    let (mut buf, mut read) = (vec![0u8; CHUNK], 0u64);
    loop {
        if stop() {
            return None;
        }
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(k) if sha.data(&buf[..k]) => read += k as u64,
            Ok(_) => return Some(Got::Skip(Why::Failed)),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Some(Got::Skip(io_why(&e))),
        }
        each(read);
    }
    let after = std::fs::metadata(path).ok();
    if read != before.len() || after.is_none_or(|a| a.len() != before.len() || a.last_write_time() != before.last_write_time()) {
        return Some(Got::Skip(Why::Changed));
    }
    Some(sha.hex().map_or(Got::Skip(Why::Failed), Got::Sum))
}

#[derive(Default)]
pub(crate) struct HashJob {
    gen: usize,
    busy: bool,
    done: u64,
    total: u64,
    name: String, // file being read
    note: String, // last progress note shown, taken down when it ends
    paths: Vec<String>,
    clip: String,
    held: Option<String>, // the clipboard's text could not be read: held by this program ("" unknown)
    rows: Vec<Got>,
}

static JOB: Mutex<HashJob> = Mutex::new(HashJob { gen: 0, busy: false, done: 0, total: 0, name: String::new(), note: String::new(), paths: Vec::new(), clip: String::new(), held: None, rows: Vec::new() });
static HASH_GEN: AtomicUsize = AtomicUsize::new(0);

fn job() -> std::sync::MutexGuard<'static, HashJob> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

/// Percent done while a checksum runs (the ⇄ menu shows "Stop" instead).
pub(crate) fn hash_busy() -> Option<u64> {
    let j = job();
    j.busy.then(|| (j.done * 100 / j.total.max(1)).min(99))
}

type Req = (isize, usize, Vec<String>); // (main window, gen, paths)
static QUEUE: OnceLock<Mutex<std::sync::mpsc::Sender<Req>>> = OnceLock::new();

// Hands the request to the one worker thread (started on first use). false: it could not be started.
fn submit(req: Req) -> bool {
    let tx = QUEUE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Req>();
        _ = std::thread::Builder::new().name("grids-hash".into()).spawn(move || {
            for (main, gen, paths) in rx {
                if HASH_GEN.load(Ordering::Relaxed) == gen {
                    worker(main, gen, paths);
                }
            }
        });
        Mutex::new(tx)
    });
    tx.lock().unwrap_or_else(|e| e.into_inner()).send(req).is_ok()
}

fn worker(main: isize, gen: usize, paths: Vec<String>) {
    let stop = || HASH_GEN.load(Ordering::Relaxed) != gen;
    let post = |fin: usize| unsafe { _ = PostMessageW(Some(HWND(main as _)), WM_HASH, WPARAM(gen), LPARAM(fin as isize)) };
    let size = |p: &String| std::fs::metadata(p).ok().filter(|m| m.is_file() && m.file_attributes() & ONLINE_ONLY == 0).map_or(0, |m| m.len());
    let sizes: Vec<u64> = paths.iter().map(size).collect();
    // only while this run is still the current one
    let set = |f: &mut dyn FnMut(&mut HashJob)| {
        let mut j = job();
        if j.gen == gen {
            f(&mut j);
        }
    };
    set(&mut |j| j.total = sizes.iter().sum());
    let (mut rows, mut base, mut last) = (Vec::new(), 0u64, Instant::now());
    for (p, &sz) in paths.iter().zip(&sizes) {
        set(&mut |j| j.name = base_name(p).to_string());
        let mut each = |b: u64| {
            if last.elapsed() >= Duration::from_millis(200) {
                last = Instant::now();
                set(&mut |j| j.done = base + b.min(sz));
                post(0);
            }
        };
        let Some(got) = hash_file(p, &stop, &mut each) else { return };
        base += sz;
        rows.push(got);
    }
    let mut j = job();
    if j.gen == gen {
        j.rows = rows;
        drop(j);
        post(1);
    }
}

// Ok(""): no text on the clipboard (nothing to check against); Err: there is text but the clipboard stayed held
// by another program (its exe name, or "").
unsafe fn clip_text() -> std::result::Result<String, String> {
    let mut s = String::new();
    if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok() {
        let _open = clip_open(app(|a| a.main)).map_err(|ClipBusy(who)| who)?;
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
            let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
            if !p.is_null() {
                let (mut n, max) = (0, (GlobalSize(HGLOBAL(h.0)) / 2).min(CLIP_MAX));
                while n < max && *p.add(n) != 0 {
                    n += 1;
                }
                s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                _ = GlobalUnlock(HGLOBAL(h.0));
            }
        }
    }
    Ok(s)
}

pub(crate) fn drop_note(mine: &str) {
    app(|a| {
        if a.note.as_ref().is_some_and(|(s, _)| s == mine) {
            a.note = None;
        }
    });
}

/// The ⇄ menu item: starts a checksum of the selection, or stops the one running.
/// The worker reports to the main window (only main_proc handles WM_HASH), never to the pane the menu came from.
pub(crate) unsafe fn hash_pick(picked: std::result::Result<Vec<String>, Pick>) {
    if hash_busy().is_some() {
        let mine = {
            let mut j = job();
            j.gen += 1;
            j.busy = false;
            HASH_GEN.store(j.gen, Ordering::Relaxed);
            std::mem::take(&mut j.note)
        };
        drop_note(&mine);
        set_note(tr("Checksum stopped", "체크섬 계산을 멈췄습니다").into());
        return;
    }
    let Ok(paths) = picked.map_err(|_| ()).and_then(|p| if p.len() <= HASH_MAX { Ok(p) } else { Err(()) }) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let (clip, held) = match clip_text() {
        Ok(t) => (t, None),
        Err(who) => (String::new(), Some(who)),
    };
    let gen = {
        let mut j = job();
        *j = HashJob { gen: j.gen + 1, busy: true, paths: paths.clone(), clip, held, ..Default::default() };
        HASH_GEN.store(j.gen, Ordering::Relaxed);
        j.gen
    };
    if !submit((app(|a| a.main).0 as isize, gen, paths)) {
        let mut j = job();
        if j.gen == gen {
            j.busy = false;
        }
        drop(j);
        _ = MessageBeep(MB_ICONWARNING);
    }
}

/// WM_HASH on the main window; messages from a stopped or replaced run are dropped.
pub(crate) unsafe fn hash_msg(h: HWND, gen: usize, fin: bool) {
    if !fin {
        let note = {
            let mut j = job();
            if j.gen != gen || !j.busy {
                return;
            }
            let pct = (j.done * 100 / j.total.max(1)).min(99);
            j.note = format!("{} {pct}% · {}", tr("SHA-256 in progress", "SHA-256 계산 중"), j.name);
            j.note.clone()
        };
        set_progress_note(note);
        return;
    }
    let Some(Done { paths, rows, clip, held, mine }) = finish(gen) else { return };
    drop_note(&mine);
    update_status();
    show_result(h, &paths, &rows, &clip, held.as_deref());
}

struct Done {
    paths: Vec<String>,
    rows: Vec<Got>,
    clip: String,
    held: Option<String>,
    mine: String, // the progress note to take down
}

// The run `gen` has finished: no longer busy, and what the result box needs taken out. The lock is let go here,
// before any note or the modal box. None: a stopped or replaced run.
fn finish(gen: usize) -> Option<Done> {
    let mut j = job();
    if j.gen != gen || !j.busy {
        return None;
    }
    j.busy = false;
    Some(Done { paths: std::mem::take(&mut j.paths), rows: std::mem::take(&mut j.rows), clip: std::mem::take(&mut j.clip), held: j.held.take(), mine: std::mem::take(&mut j.note) })
}

unsafe fn show_result(h: HWND, paths: &[String], rows: &[Got], clip: &str, held: Option<&str>) {
    let names: Vec<String> = paths.iter().map(|p| base_name(p).to_string()).collect();
    let clip = clip_hashes(clip);
    let ms = marks(&names, rows, &clip);
    let head = verdict(&names, &ms);
    let mut body: Vec<String> = Vec::new();
    for ((name, got), m) in names.iter().zip(rows).zip(&ms) {
        let tag = match m {
            Mark::Same => "✓ ",
            Mark::Differs => "✗ ",
            Mark::Unsure | Mark::Clash => "? ",
            _ => "",
        };
        let extra = match (m, got) {
            (Mark::NotListed, _) => format!("  ({})", tr("not on the clipboard", "클립보드 목록에 없음")),
            (Mark::Clash, _) => format!("  ({})", tr("another selected file has the same name, can't be checked by name", "같은 이름의 파일이 함께 선택되어 이름으로 대조할 수 없음")),
            (Mark::Unsure, Got::Sum(_)) => format!("  ({})", tr("the clipboard gives this name two values", "클립보드 목록에 이 이름이 다른 값으로 두 번 있음")),
            _ => String::new(),
        };
        let line = match got {
            Got::Sum(s) => s.clone(),
            Got::Skip(w) => why_text(w).to_string(),
        };
        body.push(format!("{tag}{name}{extra}\n    {line}"));
    }
    if names.len() > 1 && clip.named.is_empty() && !clip.lone.is_empty() {
        body.push(tr("(A single hash on the clipboard is checked only when one file is selected.)", "(클립보드에 해시 하나만 있으면 파일을 하나 골랐을 때만 대조합니다.)").into());
    }
    // not "no hash on the clipboard": there may well be one, it just couldn't be read
    if let Some(who) = held {
        let not = tr("Couldn't read the clipboard, so nothing was checked against it", "클립보드를 읽지 못해 대조하지 않음");
        body.push(format!("({not}: {})", clip_busy_note(KO.load(Ordering::Relaxed), who)));
    }
    let lines = copy_lines(&names, rows);
    let n = rows.iter().filter(|g| matches!(g, Got::Sum(_))).count();
    let (title, head_h, body_h) = (HSTRING::from("Grids — SHA-256"), HSTRING::from(head.as_str()), HSTRING::from(body.join("\n")));
    let label = HSTRING::from(tr("Copy", "복사"));
    let button = [TASKDIALOG_BUTTON { nButtonID: BTN_COPY, pszButtonText: PCWSTR(label.as_ptr()) }];
    let good = ms.iter().all(|m| matches!(m, Mark::Same | Mark::Plain)) && n == rows.len() && held.is_none();
    let mut cfg = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: h,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
        pszWindowTitle: PCWSTR(title.as_ptr()),
        pszMainInstruction: PCWSTR(head_h.as_ptr()),
        pszContent: PCWSTR(body_h.as_ptr()),
        dwCommonButtons: TDCBF_CLOSE_BUTTON,
        ..Default::default()
    };
    cfg.Anonymous1.pszMainIcon = if good { TD_INFORMATION_ICON } else { TD_WARNING_ICON };
    if n > 0 {
        (cfg.cButtons, cfg.pButtons, cfg.nDefaultButton) = (1, button.as_ptr(), BTN_COPY);
    }
    match task_dialog(&cfg) {
        Some((BTN_COPY, _)) => {
            match put_clip(&lines, CLIP_TRIES) {
                Ok(()) => {
                    let note = tr("Copied {n} checksums", "체크섬 {n}개 복사함").replace("{n}", &n.to_string());
                    app(|x| x.note = Some((note, Instant::now() + Duration::from_secs(3))));
                    update_status();
                }
                Err(e) => clip_failed(e),
            }
        }
        Some(_) => {}
        None => _ = MessageBoxW(Some(h), &HSTRING::from(format!("{head}\n\n{}", body.join("\n"))), &title, if good { MB_ICONINFORMATION } else { MB_ICONWARNING }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn sha_of(pieces: &[&[u8]]) -> String {
        let mut s = Sha::new().unwrap();
        assert!(pieces.iter().all(|p| s.data(p)));
        s.hex().unwrap()
    }

    #[test]
    fn sha256_known_values_in_pieces() {
        assert_eq!(sha_of(&[b"a", b"b", b"c"]), ABC);
        assert_eq!(sha_of(&[b"abc"]), ABC);
        assert_eq!(sha_of(&[]), EMPTY);
        assert_eq!(sha_of(&[b""]), EMPTY);
        let big = vec![b'x'; CHUNK + 1];
        let whole = sha256_hex(&big).unwrap();
        assert_eq!(sha_of(&[&big[..CHUNK], &big[CHUNK..]]), whole);
        assert_eq!(sha_of(&[&big[..1], &big[1..]]), whole);
    }

    #[test]
    fn clipboard_hashes() {
        let up = ABC.to_ascii_uppercase();
        assert_eq!(clip_hashes(&format!("SHA256: {up}")), ClipSums { named: vec![], lone: vec![ABC.into()] });
        assert_eq!(clip_hashes(&format!("  {ABC}  \r\n")).lone, [ABC]);
        assert_eq!(clip_hashes(&format!("{ABC}  setup.iso\n{EMPTY} *dir/b.bin")).named, [(ABC.to_string(), "setup.iso".to_string()), (EMPTY.to_string(), "b.bin".to_string())]);
        assert_eq!(clip_hashes(&format!("SHA256 (my file.iso) = {up}")).named, [(ABC.to_string(), "my file.iso".to_string())]);
        assert_eq!(clip_hashes(&format!("{ABC}{ABC}")), ClipSums::default()); // a SHA-512-sized run is not a SHA-256
        assert_eq!(clip_hashes(&ABC[..63]), ClipSums::default());
        assert_eq!(clip_hashes(""), ClipSums::default());
    }

    #[test]
    fn marks_and_verdicts() {
        let one = vec!["setup.iso".to_string()];
        let lone = clip_hashes(ABC);
        assert_eq!(marks(&one, &[Got::Sum(ABC.into())], &lone), [Mark::Same]);
        assert!(verdict(&one, &[Mark::Same]).starts_with("✓ Same") && verdict(&one, &[Mark::Same]).ends_with(": setup.iso"));
        assert_eq!(marks(&one, &[Got::Sum(EMPTY.into())], &lone), [Mark::Differs]);
        assert!(verdict(&one, &[Mark::Differs]).starts_with("✗ Differs"));
        assert_eq!(marks(&one, &[Got::Skip(Why::Online)], &lone), [Mark::Unsure]);
        assert_eq!(marks(&one, &[Got::Sum(ABC.into())], &ClipSums::default()), [Mark::Plain]);
        assert_eq!(verdict(&one, &[Mark::Plain]), "SHA-256: setup.iso");
        // a lone hash is not checked against several files
        let two = vec!["a.bin".to_string(), "b.bin".to_string()];
        assert_eq!(marks(&two, &[Got::Sum(ABC.into()), Got::Sum(EMPTY.into())], &lone), [Mark::Plain, Mark::Plain]);
        assert_eq!(verdict(&two, &[Mark::Plain, Mark::Plain]), "Checksums of 2 files");
        // the same hash under another name is not a match
        let other = clip_hashes(&format!("{ABC}  other.iso"));
        assert_eq!(marks(&one, &[Got::Sum(ABC.into())], &other), [Mark::NotListed]);
        assert_eq!(verdict(&one, &[Mark::NotListed]), "✗ No file matches the clipboard's values");
        // one name, two values in the list: can't say
        let clash = clip_hashes(&format!("{ABC}  setup.iso\n{EMPTY}  sub/setup.iso"));
        assert_eq!(marks(&one, &[Got::Sum(ABC.into())], &clash), [Mark::Unsure]);
        // partly read: one matches by name, the other could not be read
        let list = clip_hashes(&format!("{ABC}  a.bin\n{EMPTY}  B.BIN"));
        let ms = marks(&two, &[Got::Sum(ABC.into()), Got::Skip(Why::Locked)], &list);
        assert_eq!(ms, [Mark::Same, Mark::Unsure]);
        assert!(verdict(&two, &ms).starts_with("✓ 1 of 2"));
        assert_eq!(verdict(&two, &[Mark::Same, Mark::Same]), "✓ All 2 files match the clipboard's values");
        assert!(verdict(&two, &[Mark::Same, Mark::Differs]).starts_with("✗ 1 of 2"));
        assert_eq!(verdict(&two, &[Mark::Unsure, Mark::Unsure]), "? Could not check against the clipboard");
        // the same name picked from two folders: the list's one line can't vouch for either
        let same = vec!["setup.iso".to_string(), "SETUP.ISO".to_string(), "a.bin".to_string()];
        let list = clip_hashes(&format!("{ABC}  setup.iso\n{EMPTY}  a.bin"));
        let ms = marks(&same, &[Got::Sum(ABC.into()), Got::Sum(ABC.into()), Got::Sum(EMPTY.into())], &list);
        assert_eq!(ms, [Mark::Clash, Mark::Clash, Mark::Same]);
        assert!(verdict(&same, &ms).starts_with("✓ 1 of 3"));
    }

    // The worker's WM_HASH goes to the main window (the only one that handles it), not to the pane whose ⇄ menu
    // started it, and the finishing step frees the ⇄ menu. The result box itself is not shown here.
    #[test]
    fn checksum_reports_to_the_main_window() {
        unsafe {
            let hidden = || CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 1, 1, None, None, None, None).unwrap();
            let (main, pane) = (hidden(), hidden());
            let old = app(|a| std::mem::replace(&mut a.main, main));
            let file = std::env::temp_dir().join(format!("grids-hash-test-{}.txt", std::process::id()));
            std::fs::write(&file, b"abc").unwrap();
            assert!(hash_busy().is_none());
            hash_pick(Ok(vec![file.to_string_lossy().into_owned()]));
            assert!(hash_busy().is_some());
            let gen = job().gen;
            let (mut to, mut fin, until) = (Vec::new(), false, Instant::now() + Duration::from_secs(5));
            while !fin && Instant::now() < until {
                let mut m = MSG::default();
                if PeekMessageW(&mut m, None, WM_HASH, WM_HASH, PM_REMOVE).as_bool() {
                    to.push(m.hwnd);
                    fin = m.wParam.0 == gen && m.lParam.0 == 1; // progress notes (0) may come first
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            let done = finish(gen);
            app(|a| a.main = old);
            _ = std::fs::remove_file(&file);
            _ = DestroyWindow(main);
            _ = DestroyWindow(pane);
            assert!(fin, "no finished WM_HASH within 5 s");
            assert!(to.iter().all(|&h| h == main) && !to.contains(&pane));
            let done = done.expect("the run that finished");
            assert_eq!(done.rows, [Got::Sum(ABC.into())]);
            assert!(hash_busy().is_none(), "the ⇄ menu is free again");
            assert!(finish(gen).is_none(), "a second finish is ignored");
        }
    }

    #[test]
    fn copied_lines_read_back() {
        let names = vec!["setup.iso".to_string(), "bad.bin".to_string(), "b c.txt".to_string()];
        let rows = [Got::Sum(ABC.into()), Got::Skip(Why::Failed), Got::Sum(EMPTY.into())];
        let text = copy_lines(&names, &rows);
        assert_eq!(text, format!("{ABC}  setup.iso\n{EMPTY}  b c.txt"));
        assert!(!text.contains('\r'));
        assert_eq!(sum_for(&text, "setup.iso").as_deref(), Some(ABC));
        assert_eq!(sum_for(&text, "b c.txt").as_deref(), Some(EMPTY));
        assert_eq!(sum_for(&text, "bad.bin"), None); // unfinished files are left out
        assert_eq!(clip_hashes(&text).named, [(ABC.to_string(), "setup.iso".to_string()), (EMPTY.to_string(), "b c.txt".to_string())]);
    }
}
