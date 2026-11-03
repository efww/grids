// Folders open in Explorer windows and tabs, brought into panes that do not already show one.
// ShellWindows lists Windows 11 tabs one by one. Home, This PC and Control Panel have no
// file-system path and are skipped. The shell call pumps messages, so it stays outside app().

use std::time::Duration;

use crate::app::*;
use crate::i18n::*;
use crate::imports::*;
use crate::pane::*;
use crate::ui::*;

static GATHERING: AtomicBool = AtomicBool::new(false);

fn path_key(p: &str) -> String {
    p.trim_end_matches('\\').to_lowercase()
}

/// Explorer folders not shown yet, paired with live panes that do not hold one of them, from the active pane on.
/// Each folder already open reserves one live pane showing it. `open` is `(slot, folder)` of the live panes.
/// Returns `(slot, folder)` to open and how many new folders found no pane.
pub(crate) fn gather_plan(found: &[String], open: &[(usize, String)], active: usize, n: usize) -> (Vec<(usize, String)>, usize) {
    let (mut seen, mut new, mut kept) = (Vec::new(), Vec::new(), vec![false; open.len()]);
    for f in found {
        let k = path_key(f);
        if seen.contains(&k) {
            continue;
        }
        match (0..open.len()).find(|&j| !kept[j] && path_key(&open[j].1) == k) {
            Some(j) => kept[j] = true,
            None => new.push(f.clone()),
        }
        seen.push(k);
    }
    if n == 0 {
        return (Vec::new(), new.len());
    }
    let free: Vec<usize> = (0..n)
        .map(|k| (active + k) % n)
        .filter(|&s| (0..open.len()).any(|j| open[j].0 == s && !kept[j]))
        .collect();
    let short = new.len().saturating_sub(free.len());
    (free.into_iter().zip(new).collect(), short)
}

/// Status line for one gather. `opened` is the 1-based pane numbers that accepted a folder.
/// `none` means Explorer had no file-system folder.
pub(crate) fn gather_note(opened: &[usize], failed: usize, short: usize, none: bool) -> String {
    let mut parts = Vec::new();
    if !opened.is_empty() {
        let k = opened.len();
        let list = opened.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
        let key = if k == 1 { Key::OpeningOne } else { Key::OpeningMany };
        parts.push(tf(key, &[("k", &k.to_string()), ("list", &list)]));
    }
    if failed > 0 {
        parts.push(tf(Key::GatherFailed, &[("failed", &failed.to_string())]));
    }
    if short > 0 {
        parts.push(tf(Key::GatherShort, &[("short", &short.to_string())]));
    }
    if parts.is_empty() {
        return tr(if none { Key::GatherNone } else { Key::GatherAlready }).to_string();
    }
    parts.join(" · ")
}

/// File-system folders of the open Explorer windows. A window that does not answer is skipped.
unsafe fn explorer_folders() -> Vec<String> {
    let Ok(wins) = CoCreateInstance::<_, IShellWindows>(&ShellWindows, None, CLSCTX_ALL) else { return Vec::new() };
    let folder = |d: IDispatch| -> Result<String> {
        let sb: IShellBrowser = d.cast::<IServiceProvider>()?.QueryService(&SID_STopLevelBrowser)?;
        let it = sb.QueryActiveShellView()?.cast::<IFolderView>()?.GetFolder::<IShellItem>()?;
        Ok(pwstr(it.GetDisplayName(SIGDN_FILESYSPATH)?))
    };
    let n = wins.Count().unwrap_or(0);
    (0..n).filter_map(|i| folder(wins.Item(&VARIANT::from(i)).ok()?).ok()).collect()
}

/// Ctrl+Shift+O. Panes already showing one of the folders stay. The rest are filled from the active pane on.
pub(crate) unsafe fn gather_explorer() {
    if GATHERING.swap(true, Ordering::Relaxed) {
        return;
    }
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            GATHERING.store(false, Ordering::Relaxed);
        }
    }
    let _guard = Guard;
    let found = explorer_folders();
    let (active, n, open) = app(|a| {
        let n = a.s.panes();
        let open = (0..n).filter(|&i| a.panes[i].is_some()).map(|i| (i, a.s.paths[i].clone())).collect::<Vec<_>>();
        (a.s.active, n, open)
    });
    let (plan, short) = gather_plan(&found, &open, active, n);
    let mut opened = Vec::new();
    let mut failed = 0usize;
    if !plan.is_empty() {
        if app(|x| x.zoom.take()).is_some() {
            relayout();
            redraw_all();
        }
        for (i, p) in &plan {
            if navigate(*i, p) {
                opened.push(i + 1);
            }
        }
        failed = plan.len() - opened.len();
        focus_view(active);
    }
    let note = gather_note(&opened, failed, short, found.is_empty());
    if plan.is_empty() || failed + short > 0 {
        _ = MessageBeep(MB_ICONWARNING);
    }
    app(|x| {
        x.note = Some((note, Instant::now() + Duration::from_secs(3)));
        x.lost = 0;
    });
    update_status();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|p| (*p).to_string()).collect()
    }

    fn o(v: &[(usize, &str)]) -> Vec<(usize, String)> {
        v.iter().map(|&(i, p)| (i, p.to_string())).collect()
    }

    fn plan(found: &[&str], open: &[(usize, &str)], active: usize, n: usize) -> (Vec<(usize, String)>, usize) {
        gather_plan(&s(found), &o(open), active, n)
    }

    #[test]
    fn gather_assigns_from_the_active_pane_and_counts_a_shortfall() {
        // A stays in pane 0; E takes the next pane, and a second run finds nothing to do.
        assert_eq!(plan(&["A", "E"], &[(0, "A"), (1, "B"), (2, "C"), (3, "D")], 0, 4), (o(&[(1, "E")]), 0));
        assert_eq!(plan(&["A", "E"], &[(0, "A"), (1, "E"), (2, "C"), (3, "D")], 0, 4), (o(&[]), 0));
        // Case and a trailing \ do not matter.
        assert_eq!(plan(&["C:\\A", "c:\\a\\", "D:\\X"], &[(0, "C:\\A"), (1, "B")], 0, 2), (o(&[(1, "D:\\X")]), 0));
        assert_eq!(plan(&["C:\\A", "c:\\a\\"], &[(0, "B"), (1, "B")], 0, 2), (o(&[(0, "C:\\A")]), 0));
        // The same folder in two panes keeps one of them.
        assert_eq!(plan(&["A", "E"], &[(0, "A"), (1, "A")], 0, 2), (o(&[(1, "E")]), 0));
        // From the active pane on, wrapping round.
        assert_eq!(plan(&["E", "F", "G"], &[(0, "A"), (1, "B"), (2, "C"), (3, "D")], 3, 4), (o(&[(3, "E"), (0, "F"), (1, "G")]), 0));
        // Too few panes.
        assert_eq!(plan(&["A", "E", "F", "G"], &[(0, "A"), (1, "B")], 0, 2), (o(&[(1, "E")]), 2));
        assert_eq!(plan(&["A", "B", "C"], &[(0, "A"), (1, "B")], 0, 2), (o(&[]), 1));
        // Slots without a live pane are neither kept nor filled.
        assert_eq!(plan(&["A", "E"], &[(0, "B"), (2, "C")], 0, 3), (o(&[(0, "A"), (2, "E")]), 0));
        // A folder named like a session file is just a folder here.
        assert_eq!(plan(&["D:\\Backup.maguro"], &[(0, "A")], 0, 1), (o(&[(0, "D:\\Backup.maguro")]), 0));
        assert_eq!(plan(&[], &[(0, "A")], 0, 1), (o(&[]), 0));
        assert_eq!(plan(&["A"], &[], 0, 0), (o(&[]), 1));
    }

    #[test]
    fn gather_note_names_panes_and_a_shortfall_in_each_language() {
        let prev = UI.load(Ordering::Relaxed);
        UI.store(0, Ordering::Relaxed);
        assert_eq!(gather_note(&[2, 3], 0, 0, false), "Opening 2 Explorer folders in panes 2, 3");
        assert_eq!(gather_note(&[2], 1, 2, false), "Opening 1 Explorer folder in pane 2 · 1 failed · 2 not opened (not enough panes)");
        assert_eq!(gather_note(&[], 0, 1, false), "1 not opened (not enough panes)");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(gather_note(&[], 0, 2, false), "패널이 모자라 2개 안 엶");
        assert_eq!(gather_note(&[2], 0, 0, false), "탐색기 폴더 1개 열기 요청 → 패널 2");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(gather_note(&[], 0, 1, false), "ペインが足りず 1 個は開かず");
        assert_eq!(gather_note(&[2, 3], 1, 0, false), "エクスプローラーのフォルダー 2 個を開く要求 → ペイン 2, 3 · 1 個失敗");
        UI.store(0, Ordering::Relaxed);
        assert_eq!(gather_note(&[], 0, 0, true), "No Explorer folders to bring in");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(gather_note(&[], 0, 0, true), "가져올 탐색기 폴더가 없습니다");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(gather_note(&[], 0, 0, true), "取り込むエクスプローラーのフォルダーがありません");
        UI.store(0, Ordering::Relaxed);
        assert_eq!(gather_note(&[], 0, 0, false), "Already open in panes");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(gather_note(&[], 0, 0, false), "이미 모두 패널에 열려 있습니다");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(gather_note(&[], 0, 0, false), "すでにすべてペインで開いています");
        UI.store(prev, Ordering::Relaxed);
    }
}
