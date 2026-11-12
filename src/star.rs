// "Star on GitHub" in the View menu. With the GitHub CLI signed in, the star is given right there (gh api PUT
// user/starred/...); without gh, or if that fails, the project page opens so it can be starred by hand.
use crate::*;
use std::os::windows::process::CommandExt;
use std::sync::atomic::AtomicU8;

pub(crate) const WM_STAR: u32 = WM_APP + 3211; // wParam: 1 starred, 0 couldn't (the page opens instead)
const REPO_PATH: &str = "user/starred/efww/grids";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
// 0 not known, 1 not starred (or no gh), 2 starred
static STARRED: AtomicU8 = AtomicU8::new(0);
static BUSY: AtomicBool = AtomicBool::new(false);

// gh exits 0 only on 2xx: 204 for "starred" on GET, 204 for a PUT that took. Missing gh is an Err.
fn gh(args: &[&str]) -> bool {
    std::process::Command::new("gh")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub(crate) fn starred() -> bool {
    STARRED.load(Ordering::Relaxed) == 2
}

/// Once at start, off the UI thread: is the repo starred already? (ticks the menu item)
pub(crate) fn check_star() {
    std::thread::spawn(|| {
        let yes = gh(&["api", REPO_PATH]);
        _ = STARRED.compare_exchange(0, if yes { 2 } else { 1 }, Ordering::Relaxed, Ordering::Relaxed);
    });
}

/// The menu item.
pub(crate) unsafe fn star_clicked(main: HWND) {
    if starred() {
        set_note(tr("Already starred on GitHub. Thank you!", "이미 GitHub 별을 주셨어요. 고마워요!").into());
        return;
    }
    if BUSY.swap(true, Ordering::Relaxed) {
        return;
    }
    set_note(tr("Starring Grids on GitHub…", "GitHub에 별 주는 중…").into());
    let main = main.0 as isize;
    std::thread::spawn(move || {
        let ok = gh(&["api", "-X", "PUT", REPO_PATH]);
        unsafe { _ = PostMessageW(Some(HWND(main as _)), WM_STAR, WPARAM(ok as usize), LPARAM(0)) };
    });
}

/// WM_STAR on the main window.
pub(crate) unsafe fn star_msg(ok: bool) {
    BUSY.store(false, Ordering::Relaxed);
    if ok {
        STARRED.store(2, Ordering::Relaxed);
        set_note(tr("★ Starred Grids on GitHub. Thank you!", "★ GitHub에 별을 줬어요. 고마워요!").into());
    } else {
        set_note(tr("Opening the GitHub page: press ★ Star there", "GitHub 페이지를 엽니다: 오른쪽 위 ★ Star를 눌러 주세요").into());
        ShellExecuteW(None, w!("open"), &HSTRING::from(HOME), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}
