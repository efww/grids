// Small shell icons for favourite leaves. The system image list is shared with Explorer,
// so a row only stores an index. Lookups run on a background thread; paint reads the cache.

use std::collections::{HashMap, HashSet, VecDeque};

use windows::Win32::Storage::FileSystem::{
    GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, INVALID_FILE_ATTRIBUTES,
};
use windows::Win32::UI::Controls::{
    ImageList_DrawIndirect, HIMAGELIST, ILD_SCALE, ILD_TRANSPARENT, ILS_ALPHA, ILS_NORMAL, IMAGELISTDRAWPARAMS,
};

use crate::app::TIMER_ICON;
use crate::imports::*;

pub(crate) const WM_FAV_ICON: u32 = WM_APP + 43;

/// Last suffix, lowercased. A leading dot (`.gitignore`) and a trailing dot are not extensions.
pub(crate) fn file_ext(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// These icons differ per file, so an extension-wide cache would be wrong.
pub(crate) fn per_file_icon(ext: &str) -> bool {
    matches!(ext, "exe" | "lnk" | "ico" | "url" | "scr" | "cur" | "ani")
}

pub(crate) fn norm_path(path: &str) -> String {
    path.trim().to_lowercase()
}

#[derive(Clone, Debug)]
enum IconJob {
    Folder,
    File,
    Ext(String),
    Path(String),
    Exists(String),
}

fn job_key(job: &IconJob) -> String {
    match job {
        IconJob::Folder => "d".to_string(),
        IconJob::File => "f".to_string(),
        IconJob::Ext(ext) => format!("e:{ext}"),
        IconJob::Path(path) => format!("p:{}", norm_path(path)),
        IconJob::Exists(path) => format!("x:{}", norm_path(path)),
    }
}

#[derive(Default)]
struct IconMem {
    folder: Option<i32>,
    file: Option<i32>,
    ext: HashMap<String, i32>,
    path: HashMap<String, i32>,
    missing: HashSet<String>,
    checked: HashSet<String>,
    failed: HashSet<String>,
}

struct Decision {
    index: Option<i32>,
    dim: bool,
    jobs: Vec<IconJob>,
}

fn consider(mem: &IconMem, jobs: &mut Vec<IconJob>, job: IconJob) {
    if !mem.failed.contains(&job_key(&job)) {
        jobs.push(job);
    }
}

/// What paint should draw, and which lookups are still missing. No shell calls.
fn decide(mem: &IconMem, path: &str, file: bool) -> Decision {
    let path = path.trim();
    let key = norm_path(path);
    let mut jobs = Vec::new();
    if mem.missing.contains(&key) {
        let index = if file { mem.file } else { mem.folder };
        if index.is_none() {
            consider(mem, &mut jobs, if file { IconJob::File } else { IconJob::Folder });
        }
        return Decision { index, dim: true, jobs };
    }
    // This PC, Recycle Bin and other shell places have no disk path. They are never dimmed,
    // and they get their own shell icon (the generic folder until it is known).
    if !file && shell_place(path) {
        if let Some(index) = mem.path.get(&key).copied() {
            return Decision { index: Some(index), dim: false, jobs };
        }
        consider(mem, &mut jobs, IconJob::Path(path.to_string()));
        if mem.folder.is_none() {
            consider(mem, &mut jobs, IconJob::Folder);
        }
        return Decision { index: mem.folder, dim: false, jobs };
    }
    if !file {
        if mem.folder.is_none() {
            consider(mem, &mut jobs, IconJob::Folder);
        }
        if !path.is_empty() && !mem.checked.contains(&key) {
            consider(mem, &mut jobs, IconJob::Exists(path.to_string()));
        }
        return Decision { index: mem.folder, dim: false, jobs };
    }
    let ext = file_ext(path);
    if per_file_icon(&ext) {
        if let Some(index) = mem.path.get(&key).copied() {
            return Decision { index: Some(index), dim: false, jobs };
        }
        if !path.is_empty() {
            consider(mem, &mut jobs, IconJob::Path(path.to_string()));
        }
        if mem.file.is_none() {
            consider(mem, &mut jobs, IconJob::File);
        }
        return Decision { index: mem.file, dim: false, jobs };
    }
    if !ext.is_empty() {
        if let Some(index) = mem.ext.get(&ext).copied() {
            if !path.is_empty() && !mem.checked.contains(&key) {
                consider(mem, &mut jobs, IconJob::Exists(path.to_string()));
            }
            return Decision { index: Some(index), dim: false, jobs };
        }
        consider(mem, &mut jobs, IconJob::Ext(ext));
    }
    if mem.file.is_none() {
        consider(mem, &mut jobs, IconJob::File);
    }
    if !path.is_empty() && !mem.checked.contains(&key) {
        consider(mem, &mut jobs, IconJob::Exists(path.to_string()));
    }
    Decision { index: mem.file, dim: false, jobs }
}

struct Cache {
    mem: IconMem,
    /// System image list from `SHGetFileInfoW`. Shared with the shell. Never destroy it.
    list: isize,
}

/// Most "missing" paths remembered before the set starts over (paths that reappear get rechecked).
const MISSING_CAP: usize = 1024;

fn note_missing(mem: &mut IconMem, norm: String) -> bool {
    if mem.missing.len() >= MISSING_CAP {
        forget_missing_in(mem);
    }
    mem.checked.insert(norm.clone());
    mem.missing.insert(norm)
}

fn forget_missing_in(mem: &mut IconMem) {
    for k in mem.missing.drain() {
        mem.checked.remove(&k);
    }
}

/// Refresh (F5): recheck every path that was missing, so a drive or folder that came back gets its icon.
pub(crate) fn forget_missing() {
    forget_missing_in(&mut cache().mem);
}

/// The background check has already found this path gone. Paint reads this; it does not stat.
pub(crate) fn known_missing(path: &str) -> bool {
    let key = norm_path(path);
    !key.is_empty() && cache().mem.missing.contains(&key)
}

/// A local probe just confirmed the path is gone. The next paint draws the row dimmed.
pub(crate) fn mark_missing(path: &str) {
    let norm = norm_path(path);
    if norm.is_empty() {
        return;
    }
    if note_missing(&mut cache().mem, norm) {
        post_refresh();
    }
}

/// The path is back, or the favourite now points somewhere else. Stop drawing it dimmed.
pub(crate) fn mark_present(path: &str) {
    let key = norm_path(path);
    if key.is_empty() {
        return;
    }
    let mut c = cache();
    if c.mem.missing.remove(&key) {
        c.mem.checked.insert(key);
        drop(c);
        post_refresh();
    }
}

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    static CACHE: std::sync::OnceLock<Mutex<Cache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Cache { mem: IconMem::default(), list: 0 })).lock().unwrap_or_else(|e| e.into_inner())
}

struct Queue {
    jobs: VecDeque<IconJob>,
    pending: HashSet<String>,
}

fn queue() -> std::sync::MutexGuard<'static, Queue> {
    static QUEUE: std::sync::OnceLock<Mutex<Queue>> = std::sync::OnceLock::new();
    QUEUE.get_or_init(|| Mutex::new(Queue { jobs: VecDeque::new(), pending: HashSet::new() })).lock().unwrap_or_else(|e| e.into_inner())
}

static WAKE: Condvar = Condvar::new();
static SIDES: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static ARMED: AtomicBool = AtomicBool::new(false);
static PREWARM_DONE: AtomicBool = AtomicBool::new(false);
static REFRESH: AtomicBool = AtomicBool::new(false);

fn sides() -> std::sync::MutexGuard<'static, Vec<isize>> {
    SIDES.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn note_side(h: HWND) {
    let id = h.0 as isize;
    if id == 0 {
        return;
    }
    let mut v = sides();
    if !v.contains(&id) {
        v.push(id);
    }
}

pub(crate) fn drop_side(h: HWND) {
    let id = h.0 as isize;
    sides().retain(|x| *x != id);
    if !PREWARM_DONE.load(Ordering::Relaxed) {
        ARMED.store(false, Ordering::Relaxed);
    }
}

pub(crate) fn clear_icon_refresh() {
    REFRESH.store(false, Ordering::Relaxed);
}

pub(crate) unsafe fn arm_icon_prewarm(h: HWND) {
    if ARMED.swap(true, Ordering::Relaxed) {
        return;
    }
    if SetTimer(Some(h), TIMER_ICON, 50, None) == 0 {
        ARMED.store(false, Ordering::Relaxed);
    }
}

pub(crate) fn icon_prewarm_done() {
    PREWARM_DONE.store(true, Ordering::Relaxed);
}

/// Generic folder, generic file, and one lookup per ordinary extension. Not per path.
pub(crate) fn prewarm_exts(exts: &[String]) {
    enqueue(IconJob::Folder);
    enqueue(IconJob::File);
    for ext in exts {
        if !ext.is_empty() && !per_file_icon(ext) {
            enqueue(IconJob::Ext(ext.clone()));
        }
    }
}

fn enqueue(job: IconJob) {
    let key = job_key(&job);
    {
        let mut q = queue();
        if !q.pending.insert(key) {
            return;
        }
        q.jobs.push_back(job);
    }
    static WORKER: Once = Once::new();
    WORKER.call_once(|| _ = std::thread::Builder::new().name("fav-icons".into()).spawn(icon_worker));
    WAKE.notify_one();
}

/// Index into the system image list, plus whether the row should draw it dimmed.
/// Does not call the shell. Missing entries are queued for the worker.
pub(crate) fn shell_icon(path: &str, file: bool) -> Option<(isize, i32, bool)> {
    let (dec, list) = {
        let c = cache();
        (decide(&c.mem, path, file), c.list)
    };
    for job in dec.jobs {
        enqueue(job);
    }
    let index = dec.index?;
    if list == 0 || list == -1 {
        return None;
    }
    Some((list, index, dec.dim))
}

fn icon_worker() {
    unsafe { _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    loop {
        let job = {
            let mut q = queue();
            loop {
                if let Some(job) = q.jobs.pop_front() {
                    break job;
                }
                q = WAKE.wait(q).unwrap_or_else(|e| e.into_inner());
            }
        };
        let key = job_key(&job);
        let changed = run_job(job);
        queue().pending.remove(&key);
        if changed {
            post_refresh();
        }
    }
}

fn run_job(job: IconJob) -> bool {
    let key = job_key(&job);
    match job {
        IconJob::Folder => store_shell("folder", FILE_ATTRIBUTE_DIRECTORY, false, key, |m, idx| {
            let changed = m.folder != Some(idx);
            m.folder = Some(idx);
            changed
        }),
        IconJob::File => store_shell("file", FILE_ATTRIBUTE_NORMAL, false, key, |m, idx| {
            let changed = m.file != Some(idx);
            m.file = Some(idx);
            changed
        }),
        IconJob::Ext(ext) => {
            let dummy = format!("file.{ext}");
            store_shell(&dummy, FILE_ATTRIBUTE_NORMAL, false, key, |m, idx| m.ext.insert(ext, idx) != Some(idx))
        }
        IconJob::Path(path) => {
            // A UNC or mapped path is not stat'd here. The icon worker would block on a dead share.
            if share_skip(&path) {
                cache().mem.checked.insert(norm_path(&path));
                false
            } else if path_missing(&path) {
                let norm = norm_path(&path);
                note_missing(&mut cache().mem, norm)
            } else {
                let norm = norm_path(&path);
                store_shell(&path, FILE_ATTRIBUTE_NORMAL, true, key, move |m, idx| {
                    m.checked.insert(norm.clone());
                    m.path.insert(norm, idx) != Some(idx)
                })
            }
        }
        IconJob::Exists(path) => {
            if share_skip(&path) {
                cache().mem.checked.insert(norm_path(&path));
                return false;
            }
            let missing = path_missing(&path);
            let norm = norm_path(&path);
            let mut c = cache();
            if missing {
                note_missing(&mut c.mem, norm)
            } else {
                c.mem.checked.insert(norm);
                false
            }
        }
    }
}

fn store_shell(path: &str, attr: FILE_FLAGS_AND_ATTRIBUTES, real: bool, key: String, apply: impl FnOnce(&mut IconMem, i32) -> bool) -> bool {
    match sys_icon(path, attr, real) {
        Some((list, idx)) => {
            let mut c = cache();
            if list != 0 {
                c.list = list;
            }
            apply(&mut c.mem, idx)
        }
        None => {
            cache().mem.failed.insert(key);
            false
        }
    }
}

/// `SHGFI_SYSICONINDEX` returns the shared list. `USEFILEATTRIBUTES` skips the disk.
/// An `HICON` is destroyed if the shell filled one in; the list itself is not.
fn sys_icon(path: &str, attr: FILE_FLAGS_AND_ATTRIBUTES, real: bool) -> Option<(isize, i32)> {
    let wide = HSTRING::from(path);
    let mut info = SHFILEINFOW::default();
    let flags = if real {
        SHGFI_SYSICONINDEX | SHGFI_SMALLICON
    } else {
        SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES
    };
    let ret = unsafe { SHGetFileInfoW(&wide, attr, Some(&mut info as *mut _), size_of::<SHFILEINFOW>() as u32, flags) };
    if !info.hIcon.is_invalid() {
        unsafe { _ = DestroyIcon(info.hIcon) };
    }
    if ret == 0 || info.iIcon < 0 {
        return None;
    }
    Some((ret as isize, info.iIcon))
}

/// UNC and mapped drives stay off this thread's disk calls. Navigation resolves them in the background.
fn share_skip(path: &str) -> bool {
    crate::unc_path(path) || unsafe { crate::share_path(path) }
}

/// A shell namespace place (`::{CLSID}` or `shell:`), not a file system path.
fn shell_place(path: &str) -> bool {
    let p = path.trim();
    p.starts_with("::") || p.get(..6).is_some_and(|h| h.eq_ignore_ascii_case("shell:"))
}
fn path_missing(path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    if shell_place(path) {
        return false;
    }
    if share_skip(path) {
        return false;
    }
    let wide = HSTRING::from(path);
    let attr = unsafe { GetFileAttributesW(&wide) };
    if attr != INVALID_FILE_ATTRIBUTES {
        return false;
    }
    let err = unsafe { GetLastError() };
    err == ERROR_FILE_NOT_FOUND || err == ERROR_PATH_NOT_FOUND
}

fn post_refresh() {
    if REFRESH.swap(true, Ordering::Relaxed) {
        return;
    }
    let mut alive = false;
    {
        let mut v = sides();
        v.retain(|id| {
            let h = HWND(*id as *mut core::ffi::c_void);
            unsafe {
                if IsWindow(Some(h)).as_bool() {
                    alive = true;
                    _ = PostMessageW(Some(h), WM_FAV_ICON, WPARAM(0), LPARAM(0));
                    true
                } else {
                    false
                }
            }
        });
    }
    if !alive {
        REFRESH.store(false, Ordering::Relaxed);
    }
}

/// Draw one system-image-list slot. `list` is not owned; dropping the params must not destroy it.
pub(crate) unsafe fn draw_fav_icon(dc: HDC, list: isize, index: i32, x: i32, y: i32, slot: i32, dim: bool) {
    if list == 0 || list == -1 || index < 0 || slot <= 0 {
        return;
    }
    let params = IMAGELISTDRAWPARAMS {
        cbSize: size_of::<IMAGELISTDRAWPARAMS>() as u32,
        himl: HIMAGELIST(list),
        i: index,
        hdcDst: dc,
        x,
        y,
        cx: slot,
        cy: slot,
        xBitmap: 0,
        yBitmap: 0,
        rgbBk: COLORREF(0xFFFF_FFFF),
        rgbFg: COLORREF(0xFFFF_FFFF),
        fStyle: ILD_TRANSPARENT.0 | ILD_SCALE.0,
        dwRop: 0,
        fState: if dim { ILS_ALPHA } else { ILS_NORMAL },
        Frame: if dim { 120 } else { 0 },
        crEffect: COLORREF(0),
    };
    _ = ImageList_DrawIndirect(&params);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(d: &Decision) -> Vec<String> {
        d.jobs.iter().map(job_key).collect()
    }

    #[test]
    fn file_ext_is_the_last_suffix() {
        assert_eq!(file_ext(r"C:\a\note.TXT"), "txt");
        assert_eq!(file_ext(r"C:\a\app.Exe"), "exe");
        assert_eq!(file_ext(r"C:\a\.gitignore"), "");
        assert_eq!(file_ext("file."), "");
        assert_eq!(file_ext(r"C:\a\file.tar.gz"), "gz");
        assert_eq!(file_ext(r"C:\a\folder"), "");
        assert_eq!(norm_path(r"  C:\A\B.TXT "), r"c:\a\b.txt");
    }

    #[test]
    fn per_file_icons_are_the_ones_that_vary() {
        assert!(per_file_icon("exe"));
        assert!(per_file_icon("lnk"));
        assert!(per_file_icon("ico"));
        assert!(per_file_icon("url"));
        assert!(per_file_icon("scr"));
        assert!(!per_file_icon("txt"));
        assert!(!per_file_icon("pdf"));
        assert!(!per_file_icon(""));
    }

    #[test]
    fn missing_paths_are_rechecked_after_forget_and_capped() {
        let mut mem = IconMem::default();
        assert!(note_missing(&mut mem, norm_path(r"X:\gone")));
        assert!(decide(&mem, r"X:\gone", false).dim);
        forget_missing_in(&mut mem);
        let d = decide(&mem, r"X:\gone", false);
        assert!(!d.dim && d.jobs.iter().any(|j| matches!(j, IconJob::Exists(_))));
        for i in 0..MISSING_CAP + 5 {
            note_missing(&mut mem, format!("x:\\{i}"));
        }
        assert!(mem.missing.len() <= MISSING_CAP);
    }

    #[test]
    fn shell_places_are_never_dimmed_and_ask_for_their_own_icon() {
        let pc = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
        let bin = "::{645FF040-5081-101B-9F08-00AA002F954E}";
        assert!(shell_place(pc) && shell_place(bin) && shell_place("shell:Downloads"));
        assert!(!shell_place(r"C:\docs") && !shell_place(r"\\srv\share"));
        assert!(!path_missing(pc) && !path_missing(bin));
        let mut mem = IconMem::default();
        mem.folder = Some(3);
        let d = decide(&mem, pc, false);
        assert!(!d.dim);
        assert_eq!(d.index, Some(3));
        assert_eq!(keys(&d), vec![format!("p:{}", norm_path(pc))]);
        mem.path.insert(norm_path(pc), 9);
        let d = decide(&mem, pc, false);
        assert_eq!((d.index, d.dim, d.jobs.len()), (Some(9), false, 0));
    }
    #[test]
    fn decide_queues_only_what_is_missing() {
        let mem = IconMem::default();
        let d = decide(&mem, r"C:\docs", false);
        assert_eq!(d.index, None);
        assert!(!d.dim);
        assert_eq!(keys(&d), vec!["d".to_string(), format!("x:{}", norm_path(r"C:\docs"))]);

        let mut mem = IconMem::default();
        mem.folder = Some(3);
        let d = decide(&mem, r"C:\docs", false);
        assert_eq!(d.index, Some(3));
        assert!(!d.dim);
        assert_eq!(keys(&d), vec![format!("x:{}", norm_path(r"C:\docs"))]);

        mem.folder = Some(0);
        let d = decide(&mem, r"C:\docs", false);
        assert_eq!(d.index, Some(0));
        assert!(!keys(&d).iter().any(|k| k == "d"));

        mem.missing.insert(norm_path(r"C:\docs"));
        let d = decide(&mem, r"C:\docs", false);
        assert_eq!(d.index, Some(0));
        assert!(d.dim);
        assert!(d.jobs.is_empty());

        let mut mem = IconMem::default();
        mem.file = Some(5);
        let d = decide(&mem, r"C:\a\note.txt", true);
        assert_eq!(d.index, Some(5));
        assert!(!d.dim);
        assert_eq!(keys(&d), vec!["e:txt".into(), format!("x:{}", norm_path(r"C:\a\note.txt"))]);

        mem.ext.insert("txt".into(), 9);
        let d = decide(&mem, r"C:\a\note.txt", true);
        assert_eq!(d.index, Some(9));
        assert_eq!(keys(&d), vec![format!("x:{}", norm_path(r"C:\a\note.txt"))]);

        mem.checked.insert(norm_path(r"C:\a\note.txt"));
        let d = decide(&mem, r"C:\a\note.txt", true);
        assert_eq!(d.index, Some(9));
        assert!(d.jobs.is_empty());

        let mut mem = IconMem::default();
        mem.file = Some(5);
        let d = decide(&mem, r"C:\a\app.exe", true);
        assert_eq!(d.index, Some(5));
        assert_eq!(keys(&d), vec![format!("p:{}", norm_path(r"C:\a\app.exe"))]);

        mem.path.insert(norm_path(r"C:\a\app.exe"), 12);
        mem.checked.insert(norm_path(r"C:\a\app.exe"));
        let d = decide(&mem, r"C:\a\app.exe", true);
        assert_eq!(d.index, Some(12));
        assert!(d.jobs.is_empty());

        let mut mem = IconMem::default();
        mem.file = Some(5);
        mem.missing.insert(norm_path(r"C:\a\app.exe"));
        let d = decide(&mem, r"C:\a\app.exe", true);
        assert_eq!(d.index, Some(5));
        assert!(d.dim);
        assert!(d.jobs.is_empty());

        let mut mem = IconMem::default();
        mem.file = Some(5);
        mem.failed.insert("e:txt".into());
        let d = decide(&mem, r"C:\a\note.txt", true);
        assert_eq!(d.index, Some(5));
        assert_eq!(keys(&d), vec![format!("x:{}", norm_path(r"C:\a\note.txt"))]);

        let mut mem = IconMem::default();
        mem.failed.insert("d".into());
        let d = decide(&mem, r"C:\docs", false);
        assert!(d.index.is_none());
        assert!(!keys(&d).iter().any(|k| k == "d"));
        assert_eq!(keys(&d), vec![format!("x:{}", norm_path(r"C:\docs"))]);

        let mem = IconMem::default();
        let d = decide(&mem, r"C:\a\Makefile", true);
        assert_eq!(d.index, None);
        assert_eq!(keys(&d), vec!["f".to_string(), format!("x:{}", norm_path(r"C:\a\Makefile"))]);
    }
}
