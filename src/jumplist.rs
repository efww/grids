use crate::*;
use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

// ---------- taskbar / Start jump list: saved sessions, favourites, recent folders ----------
// Each link runs this grids.exe again; a running copy takes the arguments (WM_COPYDATA → open_args).
// No explicit AppUserModelID: the list belongs to this exe's own taskbar ID, so portable copies stay apart.

/// First argument of a jump-list folder link: the rest are folders, even one named like a session file.
pub(crate) const FOLDER_FLAG: &str = "--folder";
const SESSIONS_MAX: usize = 5;
// recent folders reorder on every move: the list is written at most this often (saving a session is not held back)
const THROTTLE: Duration = Duration::from_secs(3);

/// Takes a leading FOLDER_FLAG off; true = the arguments are folders only.
pub(crate) fn take_folder_flag(args: &mut Vec<String>) -> bool {
    let on = args.first().is_some_and(|a| a == FOLDER_FLAG);
    if on {
        args.remove(0);
    }
    on
}

/// One command-line argument in quotes, read back unchanged by Windows' argument rules:
/// backslashes before a quote and at the end are doubled, so "D:\\" keeps its closing quote.
pub(crate) fn quote_arg(s: &str) -> String {
    let (mut o, mut bs) = (String::from("\""), 0);
    for c in s.chars() {
        if c == '\\' {
            bs += 1;
            continue;
        }
        let n = if c == '"' { bs * 2 + 1 } else { bs };
        o.push_str(&"\\".repeat(n));
        o.push(c);
        bs = 0;
    }
    o.push_str(&"\\".repeat(bs * 2));
    o.push('"');
    o
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JumpLink {
    pub(crate) title: String,
    pub(crate) args: String,
    pub(crate) path: String, // shown as the tooltip
    pub(crate) folder: bool,
}

// "D:\" stays whole, else the last name: C:\Users\Public → Public, \\srv\share → share
fn folder_title(p: &str) -> String {
    let t = p.trim_end_matches('\\');
    if t.is_empty() || t.ends_with(':') {
        return p.to_string();
    }
    t.rsplit('\\').next().unwrap_or(t).to_string()
}

/// The categories to show, in order, each with its links. Sessions (newest first, up to 5), favourites in their
/// order, then recent folders that aren't favourites. Only real paths (X:\…, \\server\…), nothing the user took
/// off the list (`removed`, link arguments), and `max_slots` links in all, shared out one by one in turn.
pub(crate) fn jump_plan(
    sessions: &[PathBuf],
    favs: &[String],
    recent: &[String],
    ko: bool,
    max_slots: usize,
    removed: &[String],
) -> Vec<(String, Vec<JumpLink>)> {
    let gone = |args: &str| removed.iter().any(|r| r.eq_ignore_ascii_case(args));
    let ses: Vec<JumpLink> = sessions
        .iter()
        .map(|p| {
            let path = p.to_string_lossy().into_owned();
            let title = p.file_stem().unwrap_or_default().to_string_lossy().into_owned();
            JumpLink { title, args: quote_arg(&path), path, folder: false }
        })
        .filter(|l| !gone(&l.args))
        .take(SESSIONS_MAX)
        .collect();
    let mut seen: Vec<String> = Vec::new();
    let mut folders = |list: &[String]| -> Vec<JumpLink> {
        let mut out = Vec::new();
        for p in list.iter().filter(|p| is_fs(p)) {
            let k = p.trim_end_matches('\\').to_lowercase();
            if seen.contains(&k) {
                continue;
            }
            seen.push(k);
            let args = format!("{FOLDER_FLAG} {}", quote_arg(p));
            if !gone(&args) {
                out.push(JumpLink { title: folder_title(p), args, path: p.clone(), folder: true });
            }
        }
        out
    };
    let fav = folders(favs);
    let rec = folders(recent);
    let mut lists = [ses, fav, rec];
    let (mut take, mut left) = ([0usize; 3], max_slots);
    while left > 0 {
        let mut moved = false;
        for k in 0..3 {
            if left > 0 && take[k] < lists[k].len() {
                take[k] += 1;
                left -= 1;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    let names = if ko { ["세션", "즐겨찾기", "최근 폴더"] } else { ["Sessions", "Favourites", "Recent folders"] };
    let mut plan = Vec::new();
    for (k, list) in lists.iter_mut().enumerate() {
        list.truncate(take[k]);
        if !list.is_empty() {
            plan.push((names[k].to_string(), std::mem::take(list)));
        }
    }
    plan
}

struct Jump {
    sessions: Option<Vec<PathBuf>>, // the sessions folder's files, newest first (read again only when sessions change)
    removed: Option<Vec<String>>,   // link arguments taken off the list by the user, kept in REMOVED_FILE
    slots: usize,                   // what the taskbar last said it shows
    last: Option<String>,           // the plan last written (or tried)
    at: Option<Instant>,            // when
}

thread_local! {
    static JUMP: RefCell<Jump> = const { RefCell::new(Jump { sessions: None, removed: None, slots: 10, last: None, at: None }) };
}

// Next to grids.ini, so each portable copy keeps its own. Only written once something is taken off the list.
fn removed_path() -> PathBuf {
    ini_path().with_file_name("grids-jumplist-removed.txt")
}

fn session_files() -> Vec<PathBuf> {
    let entries = std::fs::read_dir(sessions_dir()).into_iter().flatten().flatten();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .map(|e| e.path())
        .filter(|p| is_session_file(p))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|f| f.1).collect()
}

/// Writes the jump list when what it would show changed. `sessions_changed`: a session was saved, renamed or
/// deleted (read the folder again, and don't wait out THROTTLE). UI thread only (STA). Failures stay quiet;
/// the next change tries again.
pub(crate) unsafe fn refresh_jump_list(sessions_changed: bool) {
    let (favs, recent) = app(|a| (a.s.favs.clone(), a.s.recent.clone()));
    let ko = KO.load(Ordering::Relaxed);
    let due = JUMP.with_borrow_mut(|j| {
        if sessions_changed || j.sessions.is_none() {
            j.sessions = Some(session_files());
        }
        let removed = j.removed.get_or_insert_with(|| {
            let text = std::fs::read_to_string(removed_path()).unwrap_or_default();
            text.lines().filter(|l| !l.is_empty()).map(String::from).collect()
        });
        let plan = jump_plan(j.sessions.as_deref().unwrap_or_default(), &favs, &recent, ko, j.slots, removed);
        let key = format!("{plan:?}");
        let held = !sessions_changed && j.at.is_some_and(|t| t.elapsed() < THROTTLE);
        (j.last.as_deref() != Some(key.as_str()) && !held).then_some(key)
    });
    let Some(key) = due else {
        return;
    };
    // a failure is not retried until something changes
    let key = write_jump_list(&favs, &recent, ko).unwrap_or(key);
    JUMP.with_borrow_mut(|j| (j.last, j.at) = (Some(key), Some(Instant::now())));
}

// The plan written (with the taskbar's own slot count and removed items), None = no list to write to.
unsafe fn write_jump_list(favs: &[String], recent: &[String], ko: bool) -> Option<String> {
    let list: ICustomDestinationList = CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER).ok()?;
    let mut slots = 0u32;
    let gone: IObjectArray = list.BeginList(&mut slots).ok()?;
    let mut fresh = Vec::new();
    for i in 0..gone.GetCount().unwrap_or(0) {
        if let Ok(link) = gone.GetAt::<IShellLinkW>(i) {
            let mut buf = [0u16; 2048];
            if link.GetArguments(&mut buf).is_ok() {
                let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                fresh.push(String::from_utf16_lossy(&buf[..n]));
            }
        }
    }
    // kept on disk before the commit: after it Windows no longer reports them
    let (sessions, removed) = JUMP.with_borrow_mut(|j| {
        j.slots = (slots as usize).max(1);
        let removed = j.removed.get_or_insert_with(Vec::new);
        let before = removed.len();
        for a in fresh {
            if !removed.iter().any(|r| r.eq_ignore_ascii_case(&a)) {
                removed.push(a);
            }
        }
        if removed.len() != before {
            _ = std::fs::write(removed_path(), removed.join("\n"));
        }
        (j.sessions.clone().unwrap_or_default(), removed.clone())
    });
    let plan = jump_plan(&sessions, favs, recent, ko, (slots as usize).max(1), &removed);
    let exe = HSTRING::from(std::env::current_exe().unwrap_or_default().as_os_str());
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let shell32 = HSTRING::from(format!(r"{root}\System32\shell32.dll"));
    let built = (|| -> Result<()> {
        for (cat, links) in &plan {
            let col: IObjectCollection = CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;
            for l in links {
                let icon = if l.folder { (&shell32, 3) } else { (&exe, 0) }; // shell32 #3: closed folder
                if let Ok(link) = make_link(&exe, l, icon) {
                    col.AddObject(&link)?;
                }
            }
            // refused when the user turned off recent items in jump lists: the other categories still go in
            _ = list.AppendCategory(&HSTRING::from(cat.as_str()), &col.cast::<IObjectArray>()?);
        }
        list.CommitList()
    })();
    if built.is_err() {
        _ = list.AbortList();
    }
    Some(format!("{plan:?}"))
}

unsafe fn make_link(exe: &HSTRING, l: &JumpLink, icon: (&HSTRING, i32)) -> Result<IShellLinkW> {
    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
    link.SetPath(exe)?;
    link.SetArguments(&HSTRING::from(l.args.as_str()))?;
    _ = link.SetDescription(&HSTRING::from(l.path.as_str())); // the tooltip; too long a path just goes without
    link.SetIconLocation(icon.0, icon.1)?;
    let ps: IPropertyStore = link.cast()?;
    ps.SetValue(&PKEY_Title, &PROPVARIANT::from(l.title.as_str()))?;
    ps.Commit()?;
    Ok(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Windows' (and Rust std's) rules for splitting a command line into arguments, program name left out
    fn split(cmd: &str) -> Vec<String> {
        let cs: Vec<char> = cmd.chars().collect();
        let (mut out, mut cur, mut quoted, mut any, mut i) = (Vec::new(), String::new(), false, false, 0);
        while i < cs.len() {
            let c = cs[i];
            if c == '\\' {
                let mut n = 0;
                while i < cs.len() && cs[i] == '\\' {
                    n += 1;
                    i += 1;
                }
                if cs.get(i) == Some(&'"') {
                    cur.push_str(&"\\".repeat(n / 2));
                    if n % 2 == 1 {
                        cur.push('"');
                        i += 1;
                    }
                } else {
                    cur.push_str(&"\\".repeat(n));
                }
                any = true;
                continue;
            }
            if c == '"' {
                if quoted && cs.get(i + 1) == Some(&'"') {
                    cur.push('"');
                    i += 2;
                    continue;
                }
                (quoted, any) = (!quoted, true);
            } else if (c == ' ' || c == '\t') && !quoted {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            } else {
                cur.push(c);
                any = true;
            }
            i += 1;
        }
        if any {
            out.push(cur);
        }
        out
    }

    #[test]
    fn quote_arg_round_trips() {
        for p in [r"D:\", r"C:\a b\", r"\\srv\s", r"C:\Users\Public", r"E:\x\\", r#"a"b"#, r#"C:\q\"x"#, "", r"C:\한글 폴더"] {
            assert_eq!(split(&format!("{FOLDER_FLAG} {}", quote_arg(p))), vec![FOLDER_FLAG.to_string(), p.to_string()], "{p}");
        }
        assert_eq!(quote_arg(r"D:\"), r#""D:\\""#);
        // what plain quotes would do: the closing quote is eaten
        assert_eq!(split(r#""D:\""#), vec![r#"D:""#.to_string()]);
    }

    #[test]
    fn folder_flag() {
        let mut a = vec![FOLDER_FLAG.to_string(), r"D:\x.grids".to_string()];
        assert!(take_folder_flag(&mut a));
        assert_eq!(a, vec![r"D:\x.grids".to_string()]);
        let mut b = vec![r"D:\x.grids".to_string()];
        assert!(!take_folder_flag(&mut b));
        assert_eq!(b.len(), 1);
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn titles(plan: &[(String, Vec<JumpLink>)]) -> Vec<(String, Vec<String>)> {
        plan.iter().map(|(c, l)| (c.clone(), l.iter().map(|x| x.title.clone()).collect())).collect()
    }

    #[test]
    fn plan_categories_and_filters() {
        let ses: Vec<PathBuf> = (1..=7).map(|k| PathBuf::from(format!(r"C:\g\sessions\s{k}.grids"))).collect();
        let favs = s(&[r"D:\", r"C:\Work", "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", r"c:\work\"]);
        let recent = s(&[r"C:\work", r"\\srv\share", r"E:\Photos", "shell:Downloads"]);
        let plan = jump_plan(&ses, &favs, &recent, false, 50, &[]);
        assert_eq!(
            titles(&plan),
            vec![
                ("Sessions".to_string(), s(&["s1", "s2", "s3", "s4", "s5"])),
                ("Favourites".to_string(), s(&[r"D:\", "Work"])),
                ("Recent folders".to_string(), s(&["share", "Photos"])),
            ]
        );
        let fav = &plan[1].1[0];
        assert_eq!((fav.args.as_str(), fav.folder), (r#"--folder "D:\\""#, true));
        assert_eq!(plan[0].1[0].args, r#""C:\g\sessions\s1.grids""#);
        let ko = jump_plan(&ses, &favs, &recent, true, 50, &[]);
        assert_eq!(ko.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), vec!["세션", "즐겨찾기", "최근 폴더"]);
    }

    #[test]
    fn plan_shares_slots_and_skips_removed() {
        let ses: Vec<PathBuf> = (1..=7).map(|k| PathBuf::from(format!(r"C:\s{k}.grids"))).collect();
        let favs: Vec<String> = (1..=9).map(|k| format!(r"D:\f{k}")).collect();
        let recent: Vec<String> = (1..=9).map(|k| format!(r"E:\r{k}")).collect();
        let plan = jump_plan(&ses, &favs, &recent, false, 10, &[]);
        let counts: Vec<usize> = plan.iter().map(|c| c.1.len()).collect();
        assert_eq!(counts, vec![4, 3, 3]);
        assert_eq!(jump_plan(&ses, &favs, &recent, false, 0, &[]).len(), 0);
        // only favourites: they get every slot
        assert_eq!(jump_plan(&[], &favs, &[], false, 6, &[])[0].1.len(), 6);
        let removed = s(&[r#""C:\s1.grids""#, r#"--folder "D:\f1""#]);
        let plan = jump_plan(&ses, &favs, &[], false, 50, &removed);
        assert_eq!(plan[0].1[0].title, "s2");
        assert_eq!(plan[0].1.len(), 5);
        assert_eq!(plan[1].1[0].title, "f2");
    }
}
