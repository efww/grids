use crate::*;
use std::collections::HashMap;
use std::io::Read;
use std::os::windows::fs::MetadataExt;

// ---------- ⇄ menu: the files in this folder with the same content as another, all but one of each selected ----------
// Directory Opus' Duplicate File Finder, Total Commander's Find duplicate files, dupeGuru: only files of the same
// size are read (first their first 64 KiB, then whole), by one long-lived worker as the checksum's (hashsum.rs).
// Its own job and generation, so the two never stop each other; while one runs the other's item is grey.
// The worker only reads; the pane is looked at again on the main thread when it is done, and only files still
// listed there with the size they were read at are selected. Deleting is left to the user (Del, Ctrl+Z).

pub(crate) const ID_DUPES: usize = 1159; // ⇄ menu (1155–1157 ID_PHOTO, 1158 ID_RESTORE, 1160 a timer; 1138 stays unused)
pub(crate) const WM_DUPES: u32 = WM_APP + 67; // wParam = gen, lParam = 1 finished / 0 progress
const HEAD: u64 = 64 * 1024;

// A file the worker was given, and what it got: (SHA-256, size it had when read); None = not read.
type Row = (String, Option<(String, u64)>);

#[derive(Default)]
struct DupJob {
    gen: usize,
    busy: bool,
    done: u64,
    total: u64,
    name: String, // file being read
    note: String, // last progress note shown, taken down when it ends
    pane: usize,
    dir: String,
    rows: Vec<Row>,
}

static JOB: Mutex<DupJob> = Mutex::new(DupJob { gen: 0, busy: false, done: 0, total: 0, name: String::new(), note: String::new(), pane: 0, dir: String::new(), rows: Vec::new() });
static DUP_GEN: AtomicUsize = AtomicUsize::new(0);

fn job() -> std::sync::MutexGuard<'static, DupJob> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

/// Percent done while duplicates are looked for (the ⇄ menu shows "Stop" instead).
pub(crate) fn dupes_busy() -> Option<u64> {
    let j = job();
    j.busy.then(|| (j.done * 100 / j.total.max(1)).min(99))
}

// One file of the re-read list: name, SHA-256, size, date modified (None: not read).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DupItem {
    pub(crate) name: String,
    pub(crate) sum: String,
    pub(crate) size: u64,
    pub(crate) time: Option<u64>,
}

// (items to select, groups, bytes they take). Per group of the same content and size, one is always left: the
// oldest (a date not read counts as newest), then the shortest name, then the first by name; the rest selected.
// Empty files are never duplicates of anything worth freeing.
pub(crate) fn dup_marks(items: &[DupItem]) -> (Vec<usize>, usize, u64) {
    let mut groups: HashMap<(&str, u64), Vec<usize>> = HashMap::new();
    for (k, it) in items.iter().enumerate().filter(|(_, it)| it.size > 0) {
        groups.entry((it.sum.as_str(), it.size)).or_default().push(k);
    }
    let (mut pick, mut n, mut bytes) = (Vec::new(), 0, 0u64);
    for ((_, size), ks) in groups.into_iter().filter(|(_, v)| v.len() > 1) {
        let keep = ks.iter().copied().min_by_key(|&k| (items[k].time.unwrap_or(u64::MAX), items[k].name.chars().count(), items[k].name.to_lowercase())).unwrap_or(ks[0]);
        pick.extend(ks.iter().copied().filter(|&k| k != keep));
        n += 1;
        bytes += size * (ks.len() as u64 - 1);
    }
    pick.sort_unstable();
    (pick, n, bytes)
}

// The status line once it is done: `copies` selected in `groups`, `unread` files left out.
pub(crate) fn dupes_note(ko: bool, copies: usize, groups: usize, bytes: u64, unread: usize, filtered: bool) -> String {
    let mut s = match (ko, copies) {
        (false, 0) => "No files with the same content in this folder".to_string(),
        (true, 0) => "이 폴더에 내용이 같은 파일이 없습니다".to_string(),
        (false, _) => {
            let (c, g) = (if copies == 1 { "copy" } else { "copies" }, if groups == 1 { "group" } else { "groups" });
            format!("Selected {copies} duplicate {c} in {groups} {g} · {} can be freed (the oldest of each kept)", human(bytes))
        }
        (true, _) => format!("중복 사본 {copies}개 선택 ({groups}묶음) · {} 확보 가능 (묶음마다 가장 오래된 것은 남김)", human(bytes)),
    };
    if unread > 0 {
        s += &if ko { format!(" · {unread}개는 읽지 못해 뺌 (잠김·온라인 전용·도중에 바뀜)") } else { format!(" · {unread} left out, not read (locked, online-only or changed)") };
    }
    if filtered {
        s += if ko { " · 필터 적용 중" } else { " · filter on" };
    }
    s
}

type Req = (isize, usize, Vec<String>); // (main window, gen, paths)
static QUEUE: OnceLock<Mutex<std::sync::mpsc::Sender<Req>>> = OnceLock::new();

fn submit(req: Req) -> bool {
    let tx = QUEUE.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Req>();
        _ = std::thread::Builder::new().name("grids-dupes".into()).spawn(move || {
            for (main, gen, paths) in rx {
                if DUP_GEN.load(Ordering::Relaxed) == gen {
                    worker(main, gen, paths);
                }
            }
        });
        Mutex::new(tx)
    });
    tx.lock().unwrap_or_else(|e| e.into_inner()).send(req).is_ok()
}

// SHA-256 of the first 64 KiB (online-only files were left out before); None: not read.
fn head_sum(path: &str) -> Option<String> {
    let mut buf = Vec::with_capacity(HEAD as usize);
    std::fs::File::open(path).ok()?.take(HEAD).read_to_end(&mut buf).ok()?;
    sha256_hex(&buf)
}

// Keeps the files that share their key with another; the others' sizes are added to `gone`.
fn shared<K: std::hash::Hash + Eq + Clone>(v: Vec<(String, u64, K)>, gone: &mut u64) -> Vec<(String, u64, K)> {
    let mut n: HashMap<K, usize> = HashMap::new();
    for (.., k) in &v {
        *n.entry(k.clone()).or_default() += 1;
    }
    v.into_iter().filter(|(_, sz, k)| n[k] > 1 || { *gone += sz; false }).collect()
}

fn worker(main: isize, gen: usize, paths: Vec<String>) {
    let stop = || DUP_GEN.load(Ordering::Relaxed) != gen;
    let post = |fin: usize| unsafe { _ = PostMessageW(Some(HWND(main as _)), WM_DUPES, WPARAM(gen), LPARAM(fin as isize)) };
    let set = |f: &mut dyn FnMut(&mut DupJob)| {
        let mut j = job();
        if j.gen == gen {
            f(&mut j);
        }
    };
    let mut rows: Vec<Row> = Vec::new();
    let mut live = Vec::new();
    for p in paths {
        match std::fs::symlink_metadata(&p) {
            // a link or junction: left as the "original", Del on the real file would lose the content
            Ok(m) if m.file_type().is_symlink() => {}
            Ok(m) if m.is_file() && m.file_attributes() & ONLINE_ONLY == 0 => live.push((p, m.len(), m.len())),
            _ => rows.push((p, None)),
        }
    }
    let live = shared(live, &mut 0); // sizes read again on disk
    set(&mut |j| j.total = live.iter().map(|x| x.1).sum());
    post(0); // shown at once, before anything slow is read
    let mut last = Instant::now();
    let mut gone = 0; // bytes not read whole, counted as done
    // the first 64 KiB first: same-sized videos that differ early are never read whole
    let mut heads = Vec::new();
    for (p, sz, _) in live {
        if stop() {
            return;
        }
        if sz <= HEAD {
            heads.push((p, sz, (sz, String::new())));
            continue;
        }
        set(&mut |j| j.name = base_name(&p).to_string());
        if last.elapsed() >= Duration::from_millis(200) {
            last = Instant::now();
            set(&mut |j| j.done = gone);
            post(0);
        }
        match head_sum(&p) {
            Some(h) => heads.push((p, sz, (sz, h))),
            None => {
                gone += sz;
                rows.push((p, None));
            }
        }
    }
    let todo = shared(heads, &mut gone);
    let mut base = gone;
    let mut read: Vec<(String, String, u64, u64)> = Vec::new(); // path, sum, size, date modified when read
    for (p, sz, _) in todo {
        set(&mut |j| j.name = base_name(&p).to_string());
        let mut each = |b: u64| {
            if last.elapsed() >= Duration::from_millis(200) {
                last = Instant::now();
                set(&mut |j| j.done = base + b.min(sz));
                post(0);
            }
        };
        let Some(got) = hash_file(&p, &stop, &mut each) else { return };
        base += sz;
        match (got, std::fs::metadata(&p)) {
            (Got::Sum(s), Ok(m)) => read.push((p, s, m.len(), m.last_write_time())),
            _ => rows.push((p, None)),
        }
    }
    // looked at once more just before it is handed over: a file changed or gone since it was read is left out
    for (p, s, sz, t) in read {
        let same = std::fs::metadata(&p).is_ok_and(|m| m.len() == sz && m.last_write_time() == t);
        rows.push((p, same.then_some((s, sz))));
    }
    let mut j = job();
    if j.gen == gen {
        j.rows = rows;
        drop(j);
        post(1);
    }
}

// Starts a run over `paths` for pane `pane` showing `dir`. false: the worker could not be started.
fn begin(pane: usize, dir: String, paths: Vec<String>) -> bool {
    let gen = {
        let mut j = job();
        *j = DupJob { gen: j.gen + 1, busy: true, pane, dir, ..Default::default() };
        DUP_GEN.store(j.gen, Ordering::Relaxed);
        j.gen
    };
    let main = app(|a| a.main);
    if submit((main.0 as isize, gen, paths)) {
        return true;
    }
    let mut j = job();
    if j.gen == gen {
        j.busy = false;
    }
    false
}

/// The ⇄ menu item: looks for duplicates in pane src's folder, or stops the run going on.
pub(crate) unsafe fn dupes_pick(src: usize) {
    if dupes_busy().is_some() {
        let mine = {
            let mut j = job();
            j.gen += 1;
            j.busy = false;
            DUP_GEN.store(j.gen, Ordering::Relaxed);
            std::mem::take(&mut j.note)
        };
        drop_note(&mine);
        set_note(tr("Stopped looking for duplicates", "중복 찾기를 멈췄습니다").into());
        return;
    }
    let here = app(|a| a.s.paths[src].clone());
    let (Some(fv), true, None) = (view_of::<IFolderView2>(src), is_fs(&here), hash_busy()) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0).max(0);
    if n as u32 > SEND_MAX {
        warn(tr("Too many items to look through (over 10,000)", "항목이 너무 많아 찾지 않음 (1만 개 초과)").into());
        return;
    }
    // only sizes shared by 2 or more files can hold a duplicate; empty files are left out
    let (_, items) = diff_items(&fv, n);
    let mut by: HashMap<u64, usize> = HashMap::new();
    for (_, s, _) in &items {
        if let Some(s) = s.filter(|&s| s > 0) {
            *by.entry(s).or_default() += 1;
        }
    }
    let dir = Path::new(&here);
    let paths: Vec<String> = items.iter().filter(|(_, s, _)| s.is_some_and(|s| by.get(&s).is_some_and(|&c| c > 1))).map(|(name, ..)| dir.join(name).to_string_lossy().into_owned()).collect();
    if paths.is_empty() {
        set_note(dupes_note(KO.load(Ordering::Relaxed), 0, 0, 0, 0, !filter_of(src).is_empty()));
        return;
    }
    if !begin(src, here, paths) {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

struct Done {
    pane: usize,
    dir: String,
    rows: Vec<Row>,
    mine: String,
}

// The run `gen` is over: no longer busy, its rows taken out. None: a stopped or replaced run.
fn finish(gen: usize) -> Option<Done> {
    let mut j = job();
    if j.gen != gen || !j.busy {
        return None;
    }
    j.busy = false;
    Some(Done { pane: j.pane, dir: std::mem::take(&mut j.dir), rows: std::mem::take(&mut j.rows), mine: std::mem::take(&mut j.note) })
}

/// WM_DUPES on the main window; messages from a stopped or replaced run are dropped.
pub(crate) unsafe fn dupes_msg(gen: usize, fin: bool) {
    if !fin {
        let note = {
            let mut j = job();
            if j.gen != gen || !j.busy {
                return;
            }
            let pct = (j.done * 100 / j.total.max(1)).min(99);
            j.note = format!("{} {pct}%", tr("Looking for duplicates", "중복 찾는 중"));
            if !j.name.is_empty() {
                j.note = format!("{} · {}", j.note, j.name);
            }
            j.note.clone()
        };
        set_progress_note(note);
        return;
    }
    let Some(Done { pane, dir, rows, mine }) = finish(gen) else { return };
    drop_note(&mine);
    let now = app(|a| (pane < a.s.panes()).then(|| a.s.paths[pane].clone()));
    let fv = view_of::<IFolderView2>(pane);
    let (Some(fv), true) = (fv, now.as_deref() == Some(dir.as_str())) else {
        warn(tr("Duplicates: that pane shows another folder now · nothing selected", "중복 찾기: 그 패널이 다른 폴더로 바뀌어 선택하지 않음").into());
        return;
    };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0).max(0);
    if n as u32 > SEND_MAX {
        _ = bulk_select(&fv, |_| false); // touches nothing: only says why
        return;
    }
    // the list as it is now: a file is matched by name and only while it has the size it was read at; one the
    // filter hides now is left alone and not counted as unread (the filter named is the one on now)
    let filtered = !filter_of(pane).is_empty();
    let (idx, items) = diff_items(&fv, n);
    let here: HashMap<String, usize> = items.iter().enumerate().map(|(k, (name, ..))| (name.to_lowercase(), k)).collect();
    let (mut list, mut at, mut unread) = (Vec::new(), Vec::new(), 0);
    for (path, got) in &rows {
        let k = here.get(&base_name(path).to_lowercase()).copied();
        match (got, k) {
            (Some((sum, size)), Some(k)) if items[k].1 == Some(*size) => {
                list.push(DupItem { name: items[k].0.clone(), sum: sum.clone(), size: *size, time: items[k].2 });
                at.push(idx[k]);
            }
            (Some(_), None) => {}
            _ => unread += 1,
        }
    }
    let (pick, groups, bytes) = dup_marks(&list);
    let mut hits: Vec<i32> = pick.iter().map(|&k| at[k]).collect();
    hits.sort_unstable();
    if !hits.is_empty() {
        diff_apply(pane, &fv, &hits);
    }
    set_note(dupes_note(KO.load(Ordering::Relaxed), hits.len(), groups, bytes, unread, filtered));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(name: &str, sum: &str, size: u64, time: Option<u64>) -> DupItem {
        DupItem { name: name.into(), sum: sum.into(), size, time }
    }

    #[test]
    fn duplicates_keep_one_of_each() {
        let items = [
            it("report (1).pdf", "aa", 100, Some(20)),
            it("report.pdf", "aa", 100, Some(10)), // oldest: kept
            it("report - Copy.pdf", "aa", 100, Some(30)),
            it("other.pdf", "bb", 100, Some(5)), // same size, other content: alone
            it("b.jpg", "cc", 7, Some(9)),
            it("a.jpg", "cc", 7, Some(9)), // same date and length: a.jpg first by name, kept
            it("IMG_1.jpg", "dd", 9, None), // date not read: counts as newest
            it("IMG_1 (2).jpg", "dd", 9, Some(50)),
            it("e1.txt", "e3", 0, Some(1)), // empty files: never
            it("e2.txt", "e3", 0, Some(2)),
            it("x.bin", "ff", 4, Some(1)),
            it("y.bin", "ff", 5, Some(1)), // same sum, other size: not a pair
        ];
        let (pick, groups, bytes) = dup_marks(&items);
        assert_eq!(pick, [0, 2, 4, 6]);
        assert_eq!(groups, 3);
        assert_eq!(bytes, 200 + 7 + 9);
        // per group exactly one left unselected
        for sum in ["aa", "cc", "dd"] {
            let left = items.iter().enumerate().filter(|(k, i)| i.sum == sum && !pick.contains(k)).count();
            assert_eq!(left, 1, "{sum}");
        }
        // a shorter name wins a tie on date
        let (p, ..) = dup_marks(&[it("photo (1).jpg", "s", 3, Some(1)), it("photo.jpg", "s", 3, Some(1))]);
        assert_eq!(p, [0]);
        assert_eq!(dup_marks(&[]), (vec![], 0, 0));
        assert_eq!(dup_marks(&[it("a", "s", 3, None)]), (vec![], 0, 0));
    }

    #[test]
    fn duplicate_notes_both_languages() {
        let en = dupes_note(false, 7, 3, 1288490189, 0, false);
        assert_eq!(en, "Selected 7 duplicate copies in 3 groups · 1.2 GB can be freed (the oldest of each kept)");
        assert_eq!(dupes_note(true, 7, 3, 1288490189, 0, false), "중복 사본 7개 선택 (3묶음) · 1.2 GB 확보 가능 (묶음마다 가장 오래된 것은 남김)");
        assert_eq!(dupes_note(false, 0, 0, 0, 0, true), "No files with the same content in this folder · filter on");
        assert_eq!(dupes_note(true, 0, 0, 0, 0, true), "이 폴더에 내용이 같은 파일이 없습니다 · 필터 적용 중");
        assert!(dupes_note(false, 1, 1, 5, 2, false).contains(" · 2 left out"));
        assert!(dupes_note(false, 1, 1, 5, 0, false).starts_with("Selected 1 duplicate copy in 1 group · "));
        assert!(dupes_note(false, 2, 1, 5, 0, false).starts_with("Selected 2 duplicate copies in 1 group · "));
        assert!(dupes_note(true, 1, 1, 5, 2, false).contains(" · 2개는 읽지 못해"));
    }

    // The worker reports to the main window (only main_proc handles WM_DUPES); same content pairs up, a file of
    // the same size but other content gets another sum, and the finish frees the ⇄ menu item.
    #[test]
    fn duplicates_report_to_the_main_window() {
        unsafe {
            let main = CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 1, 1, None, None, None, None).unwrap();
            let old = app(|a| std::mem::replace(&mut a.main, main));
            let dir = std::env::temp_dir().join(format!("grids-dupes-test-{}", std::process::id()));
            _ = std::fs::create_dir_all(&dir);
            let big = vec![7u8; HEAD as usize + 10];
            let mut other = big.clone();
            *other.last_mut().unwrap() = 8; // same first 64 KiB, differs at the end
            let files: [(&str, &[u8]); 5] = [("a.txt", b"abc"), ("b.txt", b"abc"), ("c.txt", b"xyz"), ("v1.bin", &big), ("v2.bin", &other)];
            let paths: Vec<String> = files.iter().map(|(n, d)| {
                let p = dir.join(n);
                std::fs::write(&p, d).unwrap();
                p.to_string_lossy().into_owned()
            }).collect();
            assert!(dupes_busy().is_none());
            assert!(begin(0, "x".into(), paths));
            assert!(dupes_busy().is_some());
            let gen = job().gen;
            let (mut to, mut fin, until) = (Vec::new(), false, Instant::now() + Duration::from_secs(5));
            while !fin && Instant::now() < until {
                let mut m = MSG::default();
                if PeekMessageW(&mut m, None, WM_DUPES, WM_DUPES, PM_REMOVE).as_bool() {
                    to.push(m.hwnd);
                    fin = m.wParam.0 == gen && m.lParam.0 == 1;
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            let done = finish(gen);
            app(|a| a.main = old);
            _ = std::fs::remove_dir_all(&dir);
            _ = DestroyWindow(main);
            assert!(fin, "no finished WM_DUPES within 5 s");
            assert!(to.iter().all(|&h| h == main));
            let rows = done.expect("the run that finished").rows;
            let sum = |n: &str| rows.iter().find(|(p, _)| base_name(p) == n).and_then(|(_, g)| g.as_ref().map(|g| g.0.clone()));
            assert_eq!(sum("a.txt"), sha256_hex(b"abc"));
            assert_eq!(sum("a.txt"), sum("b.txt"));
            assert!(sum("c.txt").is_some() && sum("c.txt") != sum("a.txt"));
            assert!(sum("v1.bin").is_some() && sum("v1.bin") != sum("v2.bin"));
            assert!(dupes_busy().is_none(), "the ⇄ menu is free again");
            assert!(finish(gen).is_none(), "a second finish is ignored");
        }
    }
}
