use crate::*;
use std::collections::BTreeMap;
use std::os::windows::fs::MetadataExt;
use windows::Win32::Storage::EnhancedStorage::PKEY_Photo_DateTaken;
use windows::Win32::System::SystemInformation::GetLocalTime;

// ---------- ⇄ menu: the selected files into year / month / day folders ----------
// Photos go by the date they were taken, everything else (and photos without one) by the date modified.
// One undoable shell move; a name clash gets Windows' own conflict box. Only folders made here are ever
// removed again, and only when empty (remove_dir).

// ⇄ menu commands: ID_ORG + 0 year, + 1 month, + 2 day
pub(crate) const ID_ORG: usize = 1134;
pub(crate) const ORG_LEVELS: usize = 3;

pub(crate) fn is_org_pick(p: usize) -> bool {
    (ID_ORG..ID_ORG + ORG_LEVELS).contains(&p)
}

// a run is under way: another ⇄ click meanwhile only beeps
static ORGANIZING: AtomicBool = AtomicBool::new(false);

struct OrganizingDone;
impl Drop for OrganizingDone {
    fn drop(&mut self) {
        ORGANIZING.store(false, Ordering::SeqCst);
    }
}

// "2026", "2026-11", "2026-11-26"; None for a date that couldn't be read (year 0).
pub(crate) fn bucket_name(d: [u16; 6], level: usize) -> Option<String> {
    let [y, m, day, ..] = d;
    if y == 0 || m == 0 || day == 0 {
        return None;
    }
    Some(match level {
        0 => format!("{y:04}"),
        1 => format!("{y:04}-{m:02}"),
        _ => format!("{y:04}-{m:02}-{day:02}"),
    })
}

// The date taken when there is one, else the date modified.
pub(crate) fn pick_date(taken: Option<[u16; 6]>, modified: [u16; 6]) -> [u16; 6] {
    taken.filter(|t| t[0] != 0).unwrap_or(modified)
}

// Extensions whose date taken is worth asking for (camera JPEG / HEIF / PNG / TIFF and raw files).
pub(crate) fn photo_ext(path: &str) -> bool {
    const EXTS: [&str; 18] = ["jpg", "jpeg", "jpe", "heic", "heif", "png", "tif", "tiff", "webp", "dng", "cr2", "cr3", "nef", "arw", "orf", "rw2", "raf", "srw"];
    let name = path.rsplit('\\').next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(_, e)| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

// The folder every path sits in (compared without case), None for none or several.
// A drive root comes back as "E:\" — a bare "E:" would fail is_fs, and joined with a name it would mean
// "E:2026", relative to that drive's current folder.
pub(crate) fn common_parent(srcs: &[String]) -> Option<String> {
    let parent = |p: &String| {
        p.trim_end_matches('\\').rsplit_once('\\').map(|(d, _)| match d.len() == 2 && d.ends_with(':') {
            true => format!("{d}\\"),
            false => d.to_string(),
        })
    };
    let first = parent(srcs.first()?)?;
    srcs.iter().all(|s| parent(s).is_some_and(|d| d.trim_end_matches('\\').eq_ignore_ascii_case(first.trim_end_matches('\\')))).then_some(first)
}

// (folder name, its files in the given order), sorted by name; then the files without a date.
pub(crate) fn group_plan(items: &[(String, Option<String>)]) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    let (mut groups, mut undated) = (BTreeMap::<String, Vec<String>>::new(), Vec::new());
    for (path, bucket) in items {
        match bucket {
            Some(b) => groups.entry(b.clone()).or_default().push(path.clone()),
            None => undated.push(path.clone()),
        }
    }
    (groups.into_iter().collect(), undated)
}

// "Move 37 files into 3 folders (2026-09, 2026-10, 2026-11)?", the folder list cut after 4.
pub(crate) fn ask_text(ko: bool, files: usize, folders: &[String], undated: usize) -> String {
    let mut list = folders.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
    if folders.len() > 4 {
        list.push_str(", …");
    }
    let (n, k) = (folders.len(), files);
    let mut text = match ko {
        true => format!("파일 {k}개를 폴더 {n}개({list})로 옮길까요?"),
        false => {
            let f = if k == 1 { "file" } else { "files" };
            let d = if n == 1 { "folder" } else { "folders" };
            format!("Move {k} {f} into {n} {d} ({list})?")
        }
    };
    text.push_str(match ko {
        true => "\n\n사진은 찍은 날짜, 나머지는 수정한 날짜를 따릅니다.",
        false => "\n\nPhotos go by the date taken, other files by the date modified.",
    });
    if undated > 0 {
        text.push_str(&match ko {
            true => format!("\n날짜를 읽을 수 없는 파일 {undated}개는 그대로 둡니다."),
            false => format!("\n{undated} file(s) without a readable date stay where they are."),
        });
    }
    text
}

// The menu entry is live for a selection of files only (no folders), all in one file system folder.
pub(crate) fn org_ok(picked: &std::result::Result<Vec<String>, Pick>, no_dirs: bool, here: &str) -> bool {
    no_dirs && is_fs(here) && picked.as_ref().ok().and_then(|p| common_parent(p)).is_some_and(|d| is_fs(&d))
}

// "Sort the selected files into date folders ▸ By year (2026) / By month (2026-11) / By day (2026-11-26)"
pub(crate) unsafe fn org_menu(menu: HMENU, ok: bool) {
    let Ok(sub) = CreatePopupMenu() else { return };
    let t = GetLocalTime();
    let today = [t.wYear, t.wMonth, t.wDay, 0, 0, 0];
    let hint = tr("Photos by date taken, other files by date modified", "사진은 찍은 날짜, 나머지는 수정한 날짜 기준");
    _ = AppendMenuW(sub, MF_STRING | MF_GRAYED, 0, &HSTRING::from(hint));
    _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
    // One underlined letter each (Y / M / D) so the keyboard picks them in one press.
    let names = [tr("By &year", "연도별(&Y)"), tr("By &month", "월별(&M)"), tr("By &day", "날짜별(&D)")];
    for (k, name) in names.into_iter().enumerate() {
        let example = bucket_name(today, k).unwrap_or_default();
        _ = AppendMenuW(sub, MF_STRING, ID_ORG + k, &HSTRING::from(format!("{name}  ({example})")));
    }
    let busy = ORGANIZING.load(Ordering::SeqCst);
    let flags = if ok && !busy { MF_POPUP } else { MF_POPUP | MF_GRAYED };
    _ = AppendMenuW(menu, flags, sub.0 as usize, &HSTRING::from(tr2(M_ORG)));
}

// Picked from the ⇄ menu: the selection it counted, checked again; the dates are read on a worker thread.
pub(crate) unsafe fn organize(src: usize, level: usize, picked: std::result::Result<Vec<String>, Pick>) {
    let here = app(|a| a.s.paths[src].clone());
    let parent = picked.as_ref().ok().and_then(|p| common_parent(p)).filter(|d| is_fs(d) && is_fs(&here));
    let (Ok(srcs), Some(parent)) = (picked, parent) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if ORGANIZING.swap(true, Ordering::SeqCst) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let done = OrganizingDone; // released when the worker ends, or right away if it can't start
    set_note(tr("Reading dates…", "날짜 읽는 중…").to_string());
    let ko = KO.load(Ordering::Relaxed);
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let _done = done;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        organize_work(&srcs, &parent, level, ko);
        if com {
            CoUninitialize();
        }
    });
    match job {
        Ok(job) => keep_job(job),
        Err(_) => _ = MessageBeep(MB_ICONWARNING),
    }
}

// Worker thread: dates, one question, the folders, then the move.
unsafe fn organize_work(srcs: &[String], parent: &str, level: usize, ko: bool) {
    let warn = |text: String| _ = MessageBoxW(None, &HSTRING::from(text), w!("Grids"), MB_ICONWARNING | MB_SETFOREGROUND);
    let mut items = Vec::with_capacity(srcs.len());
    for s in srcs {
        let Ok(m) = std::fs::metadata(s) else {
            items.push((s.clone(), None)); // gone or unreadable: left alone
            continue;
        };
        if m.is_dir() {
            _ = MessageBeep(MB_ICONWARNING); // became a folder since the menu: nothing is moved
            return;
        }
        // online-only cloud files: the date taken would download the whole file
        let taken = (photo_ext(s) && !cloud_only(m.file_attributes())).then(|| date_taken(s)).flatten();
        let date = pick_date(taken, local_date(m.last_write_time()));
        items.push((s.clone(), bucket_name(date, level)));
    }
    let (groups, undated) = group_plan(&items);
    if groups.is_empty() {
        warn(tr("None of the selected files has a readable date.", "선택한 파일 가운데 날짜를 읽을 수 있는 것이 없습니다.").to_string());
        return;
    }
    let names: Vec<String> = groups.iter().map(|(n, _)| n.clone()).collect();
    let files: usize = groups.iter().map(|(_, f)| f.len()).sum();
    let ask = HSTRING::from(ask_text(ko, files, &names, undated.len()));
    let flags = MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_SETFOREGROUND | MB_TOPMOST;
    if MessageBoxW(None, &ask, w!("Grids"), flags) != IDYES {
        return;
    }
    // only the folders that aren't there yet; any failure: nothing is moved, the ones just made go again
    let mut made: Vec<PathBuf> = Vec::new();
    let remove_made = |made: &[PathBuf]| {
        for d in made {
            if std::fs::remove_dir(d).is_ok() {
                let wide = HSTRING::from(d.to_string_lossy().as_ref());
                SHChangeNotify(SHCNE_RMDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
            }
        }
    };
    for name in &names {
        let p = Path::new(parent).join(name);
        if p.is_dir() {
            continue;
        }
        match std::fs::create_dir(&p) {
            Ok(()) => {
                let wide = HSTRING::from(p.to_string_lossy().as_ref());
                SHChangeNotify(SHCNE_MKDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
                made.push(p);
            }
            Err(_) if p.is_dir() => {} // made meanwhile by someone else
            Err(e) => {
                remove_made(&made);
                let what = tr("Couldn't make the folder “{name}” (nothing was moved): ", "‘{name}’ 폴더를 만들지 못했습니다 (아무것도 옮기지 않음): ");
                warn(format!("{}{e}", what.replace("{name}", name)));
                return;
            }
        }
    }
    let op = move_groups(parent, &groups);
    let aborted = op.as_ref().map_or(true, |op| op.GetAnyOperationsAborted().map_or(true, |b| b.as_bool()));
    if let Err(e) = &op {
        if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
            warn(format!("{}{}", tr("Moving failed: ", "이동하지 못했습니다: "), e.message()));
        }
    }
    // remove_dir only takes an empty folder: whatever did get moved stays put
    remove_made(&made);
    if !aborted {
        let note = match ko {
            true => format!("파일 {files}개를 날짜별 폴더 {}개로 옮김", names.len()),
            false => format!("Moved {files} files into {} date folders", names.len()),
        };
        *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(note);
    }
}

// The photo's date taken (EXIF), as a local date; None when it has none.
unsafe fn date_taken(path: &str) -> Option<[u16; 6]> {
    let item: IShellItem2 = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
    let ft = item.GetFileTime(&PKEY_Photo_DateTaken).ok()?;
    let ft = (ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64;
    (ft != 0).then(|| local_date(ft)).filter(|d| d[0] != 0)
}

// One shell operation for every file, each into its own date folder: one progress window, one undo record.
// No owner window: the progress and conflict dialogs belong to the worker thread.
unsafe fn move_groups(parent: &str, groups: &[(String, Vec<String>)]) -> Result<IFileOperation> {
    let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
    op.SetOperationFlags(FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD)?;
    for (name, files) in groups {
        let dir = Path::new(parent).join(name);
        let to: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(dir.to_string_lossy().as_ref()), None)?;
        for f in files {
            let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(f.as_str()), None)?;
            op.MoveItem(&item, &to, PCWSTR::null(), None)?;
        }
    }
    op.PerformOperations()?;
    Ok(op)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_folder_names() {
        let d = [2026, 3, 7, 14, 5, 9];
        assert_eq!(bucket_name(d, 0).as_deref(), Some("2026"));
        assert_eq!(bucket_name(d, 1).as_deref(), Some("2026-03"));
        assert_eq!(bucket_name(d, 2).as_deref(), Some("2026-03-07"));
        assert_eq!(bucket_name([2026, 11, 26, 0, 0, 0], 2).as_deref(), Some("2026-11-26"));
        // a time that couldn't be converted: no "0000-00" folder
        assert_eq!(bucket_name([0; 6], 1), None);
        // the date taken first, the date modified when there is none (or it is empty)
        let modified = [2025, 12, 31, 23, 0, 0];
        assert_eq!(pick_date(Some(d), modified), d);
        assert_eq!(pick_date(None, modified), modified);
        assert_eq!(pick_date(Some([0; 6]), modified), modified);
    }

    #[test]
    fn date_folder_plan() {
        let it = |p: &str, b: Option<&str>| (p.to_string(), b.map(str::to_string));
        let items = [it(r"C:\p\c.jpg", Some("2026-11")), it(r"C:\p\a.jpg", Some("2026-09")), it(r"C:\p\x.txt", None), it(r"C:\p\b.jpg", Some("2026-11"))];
        let (groups, undated) = group_plan(&items);
        assert_eq!(groups, [("2026-09".to_string(), vec![r"C:\p\a.jpg".to_string()]), ("2026-11".into(), vec![r"C:\p\c.jpg".into(), r"C:\p\b.jpg".into()])]);
        assert_eq!(undated, [r"C:\p\x.txt"]);
        // only files in one folder
        assert_eq!(common_parent(&[r"C:\p\a.jpg".into(), r"c:\P\b.jpg".into()]).as_deref(), Some(r"C:\p"));
        assert_eq!(common_parent(&[r"C:\p\a.jpg".into(), r"C:\q\b.jpg".into()]), None);
        assert_eq!(common_parent(&[]), None);
        // a USB / SD card root: "E:\" (a file system folder), never the drive-relative "E:"
        assert_eq!(common_parent(&[r"E:\a.jpg".into(), r"e:\b.jpg".into()]).as_deref(), Some(r"E:\"));
        assert!(is_fs(&common_parent(&[r"E:\a.jpg".into()]).unwrap()));
        assert!(photo_ext(r"C:\p\IMG_1.JPG") && photo_ext("x.heic") && photo_ext("raw.CR3") && !photo_ext("a.txt") && !photo_ext(r"C:\a.jpg\noext"));
        assert!(is_shell_pick(ID_ORG) && is_shell_pick(ID_ORG + 2) && !is_org_pick(ID_ORG + 3) && !is_shell_pick(ID_ORG + 3));
    }

    #[test]
    fn date_folder_question() {
        let f = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let three = f(&["2026-09", "2026-10", "2026-11"]);
        assert!(ask_text(false, 37, &three, 0).starts_with("Move 37 files into 3 folders (2026-09, 2026-10, 2026-11)?"));
        assert!(ask_text(true, 37, &three, 0).starts_with("파일 37개를 폴더 3개(2026-09, 2026-10, 2026-11)로 옮길까요?"));
        assert!(ask_text(false, 1, &f(&["2026"]), 0).starts_with("Move 1 file into 1 folder (2026)?"));
        let six = f(&["2026-01", "2026-02", "2026-03", "2026-04", "2026-05", "2026-06"]);
        assert!(ask_text(false, 6, &six, 0).starts_with("Move 6 files into 6 folders (2026-01, 2026-02, 2026-03, 2026-04, …)?"));
        assert!(ask_text(true, 6, &six, 2).contains("날짜를 읽을 수 없는 파일 2개는 그대로 둡니다."));
        assert!(!ask_text(false, 6, &six, 0).contains("readable"));
    }
}
