// Notice shown when a favourite's folder is gone. The pane is not navigated.
// Remove drops that favourite. Change location picks a folder and keeps the name and order.

use crate::app::*;
use crate::i18n::*;
use crate::imports::*;
use crate::session::*;
use crate::shell::*;
use crate::BoxI;
use super::*;

/// 'Folder not found', in the UI language.
pub(crate) fn folder_gone_text() -> &'static str {
    tr(crate::Key::FolderNotFound)
}

/// Short note under a missing favourite's location. Files and folders both use it.
pub(crate) fn fav_missing_note() -> &'static str {
    tr(crate::Key::NotFound)
}

/// Remove, then Change location. The ellipsis is the one character, matching the button.
pub(crate) fn folder_gone_buttons() -> (&'static str, &'static str) {
    (
        tr(crate::Key::RemoveFromFavourites),
        tr(crate::Key::ChangeLocation),
    )
}

/// Message on top, the two buttons centred along the bottom. Widths shrink together when the notice is narrow.
pub(crate) fn miss_layout(w: i32, h: i32, dpi: i32, remove_w: i32, change_w: i32) -> (BoxI, BoxI, BoxI) {
    let s = |v: i32| v * dpi.max(1) / 96;
    let pad = s(PAD_BODY_X);
    let gap = s(BTN_GAP);
    let bh = s(BTN_H_FOOTER).clamp(1, h.max(1));
    let mut rw = (remove_w + s(BTN_PAD_X) * 2).max(s(BTN_MIN_W));
    let mut cw = (change_w + s(BTN_PAD_X) * 2).max(s(BTN_MIN_W));
    let room = (w - pad * 2 - gap).max(0);
    let buttons = rw.saturating_add(cw);
    if buttons > room && buttons > 0 {
        rw = (rw as i64 * i64::from(room) / i64::from(buttons)) as i32;
        cw = room - rw;
    }
    let total = rw + gap + cw;
    let left = ((w - total) / 2).max(0);
    let top = (h - pad - bh).max(0);
    let remove = BoxI { l: left, t: top, r: left + rw, b: (top + bh).min(h.max(top)) };
    let change = BoxI { l: remove.r + gap, t: top, r: (remove.r + gap + cw).min(w.max(0)), b: remove.b };
    let msg_b = (top - s(6)).max(s(8));
    let msg = BoxI { l: pad.min(w.max(0)), t: s(8).min(msg_b), r: (w - pad).max(pad), b: msg_b };
    (msg, remove, change)
}

/// Outer size for the text widths of the two buttons.
pub(crate) fn miss_window(dpi: i32, remove_w: i32, change_w: i32) -> (i32, i32) {
    let s = |v: i32| v * dpi.max(1) / 96;
    let pad = s(PAD_BODY_X);
    let gap = s(BTN_GAP);
    let rw = (remove_w + s(BTN_PAD_X) * 2).max(s(BTN_MIN_W));
    let cw = (change_w + s(BTN_PAD_X) * 2).max(s(BTN_MIN_W));
    let w = rw + gap + cw + pad * 2;
    let h = pad + s(22) + s(SPACE_8) + s(BTN_H_FOOTER) + pad;
    (w.max(s(240)), h)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MissHit {
    Remove,
    Change,
    None,
}

fn hit_of(lay: &(BoxI, BoxI, BoxI), x: i32, y: i32) -> MissHit {
    if lay.1.contains(x, y) {
        MissHit::Remove
    } else if lay.2.contains(x, y) {
        MissHit::Change
    } else {
        MissHit::None
    }
}

fn hit_code(hit: MissHit) -> i32 {
    match hit {
        MissHit::Remove => 1,
        MissHit::Change => 2,
        MissHit::None => 0,
    }
}

static ASK: Mutex<Option<FavRow>> = Mutex::new(None);
static NOTICE: Mutex<isize> = Mutex::new(0);
static PRESS: Mutex<i32> = Mutex::new(0);

fn ask_lock() -> std::sync::MutexGuard<'static, Option<FavRow>> {
    ASK.lock().unwrap_or_else(|e| e.into_inner())
}

fn notice_hwnd() -> HWND {
    HWND(*NOTICE.lock().unwrap_or_else(|e| e.into_inner()) as *mut core::ffi::c_void)
}

/// The favourite that was activated. The current pane is left where it is.
pub(crate) unsafe fn show_fav_missing(row: &FavRow) {
    if row.group || row.path.is_empty() {
        return;
    }
    *ask_lock() = Some(row.clone());
    let owner = app(|a| a.main);
    if owner.0.is_null() {
        return;
    }
    ensure_class();
    let (dpi, font) = app(|a| (a.dpi.max(96), a.font));
    let (remove, change) = folder_gone_buttons();
    let dc = GetDC(Some(owner));
    let (rw, cw) = (text_px(dc, font, remove), text_px(dc, font, change));
    _ = ReleaseDC(Some(owner), dc);
    let (w, h) = miss_window(dpi, rw, cw);
    let (x, y) = notice_at(owner, w, h);
    let existing = notice_hwnd();
    if !existing.0.is_null() && IsWindow(Some(existing)).as_bool() {
        _ = SetWindowPos(existing, Some(HWND_TOP), x, y, w, h, SWP_SHOWWINDOW);
        _ = InvalidateRect(Some(existing), None, false);
        _ = SetFocus(Some(existing));
        return;
    }
    let Ok(hwnd) = CreateWindowExW(
        WS_EX_TOOLWINDOW,
        w!("MaguroFavMiss"),
        w!(""),
        WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN,
        x,
        y,
        w,
        h,
        Some(owner),
        None,
        None,
        None,
    ) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    *NOTICE.lock().unwrap_or_else(|e| e.into_inner()) = hwnd.0 as isize;
    _ = SetFocus(Some(hwnd));
}

fn notice_at(owner: HWND, w: i32, h: i32) -> (i32, i32) {
    unsafe {
        let mut wr = RECT::default();
        _ = GetWindowRect(owner, &mut wr);
        let mut x = wr.left + (wr.right - wr.left - w) / 2;
        let mut y = wr.top + (wr.bottom - wr.top - h) / 2;
        let mon = MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(mon, &mut info).as_bool() {
            let a = info.rcWork;
            x = x.clamp(a.left, (a.right - w).max(a.left));
            y = y.clamp(a.top, (a.bottom - h).max(a.top));
        }
        (x, y)
    }
}

unsafe fn ensure_class() {
    static CLASS: Once = Once::new();
    CLASS.call_once(|| {
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(miss_proc),
            hInstance: hinst.into(),
            hCursor: arrow,
            lpszClassName: w!("MaguroFavMiss"),
            ..Default::default()
        });
    });
}

unsafe fn text_px(dc: HDC, font: HFONT, s: &str) -> i32 {
    let old = SelectObject(dc, font.into());
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    if !w.is_empty() {
        _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    }
    SelectObject(dc, old);
    sz.cx
}

unsafe fn boxes(h: HWND, dc: HDC) -> (BoxI, BoxI, BoxI) {
    let (dpi, font) = app(|a| (a.dpi.max(96), a.font));
    let (remove, change) = folder_gone_buttons();
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    miss_layout(rc.right.max(0), rc.bottom.max(0), dpi, text_px(dc, font, remove), text_px(dc, font, change))
}

unsafe fn close_notice(h: HWND) {
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        _ = DestroyWindow(h);
    }
}

unsafe fn remove_row(row: &FavRow) {
    match row.kind {
        RowKind::Tree => {
            let links = if row.id.is_empty() { app(|a| server_by_path(&a.s.tree, &row.path).into_iter().collect()) } else { servers_at(&row.id) };
            app(|a| {
                a.s.ensure_fav_tree();
                if row.id.is_empty() || !delete_at(&mut a.s.tree, &row.id) {
                    remove_path(&mut a.s.tree, &row.path);
                }
                a.s.sync_favs();
            });
            release_servers(links);
            commit_favs();
        }
        RowKind::Pins | RowKind::Files => {
            let side = app(|a| a.side_hwnd);
            if row.file || row.kind == RowKind::Files {
                // The row stays until a Home re-read omits it. Hiding it now brought a file
                // back when the verb returned success and left the favourite in place.
                shell_unfavorite_file(&row.path);
                if !side.0.is_null() {
                    _ = InvalidateRect(Some(side), None, false);
                }
                return;
            }
            forget_pinned(&row.path);
            let removed = shell_home_verb(&row.path, false);
            request_pins(true);
            if !side.0.is_null() {
                _ = InvalidateRect(Some(side), None, false);
            }
            if !removed {
                _ = MessageBeep(MB_ICONWARNING);
            }
        }
    }
}

unsafe fn relocate_row(owner: HWND, row: &FavRow) {
    let Some(picked) = pick_folder(owner) else { return };
    let path = tidy_path(&picked);
    if path.is_empty() {
        return;
    }
    let ok = match row.kind {
        RowKind::Tree => {
            let ok = app(|a| {
                a.s.ensure_fav_tree();
                let ok = if !row.id.is_empty() && retarget_leaf(&mut a.s.tree, &row.id, &path) {
                    true
                } else {
                    retarget_fav_path(&mut a.s.tree, &row.path, &path)
                };
                if ok {
                    a.s.sync_favs();
                }
                ok
            });
            if ok {
                mark_present(&path);
                commit_favs();
            }
            ok
        }
        RowKind::Pins | RowKind::Files => relocate_home(row, &path),
    };
    if !ok {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

/// Pin the folder the user picked, then drop the old Explorer entry. The new pin is made first,
/// so a failure leaves the old row in place.
unsafe fn relocate_home(row: &FavRow, path: &str) -> bool {
    if path.eq_ignore_ascii_case(&row.path) {
        mark_present(path);
        return true;
    }
    let name = short_name(path);
    if !shell_home_verb(path, true) {
        return false;
    }
    remember_pinned(path, &name, false);
    app(|a| a.s.pins_open = true);
    if row.file || row.kind == RowKind::Files {
        shell_unfavorite_file(&row.path);
    } else {
        forget_pinned(&row.path);
        let _ = shell_home_verb(&row.path, false);
    }
    request_pins(true);
    mark_present(path);
    let side = app(|a| a.side_hwnd);
    if !side.0.is_null() {
        _ = InvalidateRect(Some(side), None, false);
    }
    true
}

unsafe fn paint_miss(h: HWND) {
    let hot = GetWindowLongPtrW(h, GWLP_USERDATA) as i32;
    let down = *PRESS.lock().unwrap_or_else(|e| e.into_inner());
    let (dpi, font) = app(|a| (a.dpi.max(96), a.font));
    let (remove, change) = folder_gone_buttons();
    let hot_id = (hot > 0).then_some(hot);
    let down_id = (down > 0).then_some(down);
    let ground = ui_tokens().bg;
    buffered(h, |dc, rc| {
        paint_window(dc, rc);
        frame(dc, rc, 1, ui_tokens().line);
        let lay = boxes(h, dc);
        text(dc, lay.0.to_rect(), folder_gone_text(), font, pal().text, DT_CENTER);
        paint_button(dc, lay.1.to_rect(), ButtonKind::Danger, visual_from_pointer(1, hot_id, down_id), remove, font, dpi, ground, FocusRing::Outside);
        paint_button(dc, lay.2.to_rect(), ButtonKind::Primary, visual_from_pointer(2, hot_id, down_id), change, font, dpi, ground, FocusRing::Outside);
    });
}

unsafe extern "system" fn miss_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { miss_proc_inner(h, msg, wp, lp) }))
}
unsafe fn miss_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    bind_hwnd(h);
    match msg {
        WM_PAINT => paint_miss(h),
        WM_ERASEBKGND => return LRESULT(1),
        WM_MOUSEMOVE => {
            let (x, y) = xy(lp);
            let dc = GetDC(Some(h));
            let code = hit_code(hit_of(&boxes(h, dc), x, y));
            _ = ReleaseDC(Some(h), dc);
            let prev = GetWindowLongPtrW(h, GWLP_USERDATA) as i32;
            if prev != code {
                _ = SetWindowLongPtrW(h, GWLP_USERDATA, code as isize);
                _ = InvalidateRect(Some(h), None, false);
                if code != 0 && prev == 0 {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: h,
                        dwHoverTime: 0,
                    };
                    _ = TrackMouseEvent(&mut t);
                }
            }
        }
        WM_MOUSELEAVE => {
            _ = SetWindowLongPtrW(h, GWLP_USERDATA, 0);
            _ = InvalidateRect(Some(h), None, false);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = xy(lp);
            let dc = GetDC(Some(h));
            let code = hit_code(hit_of(&boxes(h, dc), x, y));
            _ = ReleaseDC(Some(h), dc);
            *PRESS.lock().unwrap_or_else(|e| e.into_inner()) = code;
            _ = SetCapture(h);
            _ = SetWindowLongPtrW(h, GWLP_USERDATA, code as isize);
            _ = InvalidateRect(Some(h), None, false);
        }
        WM_LBUTTONUP => {
            let (x, y) = xy(lp);
            let dc = GetDC(Some(h));
            let hit = hit_of(&boxes(h, dc), x, y);
            _ = ReleaseDC(Some(h), dc);
            // Read the press before ReleaseCapture. That sends WM_CAPTURECHANGED, which
            // clears a press capture stole. Our own release must not eat the click.
            let down = std::mem::take(&mut *PRESS.lock().unwrap_or_else(|e| e.into_inner()));
            if GetCapture() == h {
                _ = ReleaseCapture();
            }
            if down != 0 && down == hit_code(hit) {
                let row = ask_lock().take();
                let owner = GetWindow(h, GW_OWNER).unwrap_or_else(|_| app(|a| a.main));
                close_notice(h);
                if let Some(row) = row {
                    match hit {
                        MissHit::Remove => remove_row(&row),
                        MissHit::Change => relocate_row(owner, &row),
                        MissHit::None => {}
                    }
                }
            }
        }
        WM_CAPTURECHANGED => {
            let stuck = std::mem::take(&mut *PRESS.lock().unwrap_or_else(|e| e.into_inner()));
            if stuck != 0 {
                _ = SetWindowLongPtrW(h, GWLP_USERDATA, 0);
                _ = InvalidateRect(Some(h), None, false);
            }
        }
        WM_KEYDOWN if VIRTUAL_KEY(wp.0 as u16) == VK_ESCAPE => {
            _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => {
            _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        WM_CLOSE => close_notice(h),
        WM_DESTROY => {
            *PRESS.lock().unwrap_or_else(|e| e.into_inner()) = 0;
            let mut slot = NOTICE.lock().unwrap_or_else(|e| e.into_inner());
            if *slot == h.0 as isize {
                *slot = 0;
            }
            *ask_lock() = None;
        }
        _ => return DefWindowProcW(h, msg, wp, lp),
    }
    LRESULT(0)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::{fav_missing_note, folder_gone_buttons, folder_gone_text, hit_of, miss_layout, miss_window, MissHit};
    use crate::i18n::UI;

    #[test]
    fn folder_not_found_uses_the_approved_words() {
        let prev = UI.load(Ordering::Relaxed);
        UI.store(0, Ordering::Relaxed);
        assert_eq!(folder_gone_text(), "Folder not found");
        assert_eq!(fav_missing_note(), "Not found");
        let (remove, change) = folder_gone_buttons();
        assert_eq!(remove, "Remove from favourites");
        assert_eq!(change, "Change location…");
        UI.store(1, Ordering::Relaxed);
        assert_eq!(folder_gone_text(), "폴더를 찾을 수 없습니다");
        assert_eq!(fav_missing_note(), "찾을 수 없음");
        let (remove, change) = folder_gone_buttons();
        assert_eq!(remove, "즐겨찾기에서 제거");
        assert_eq!(change, "위치 변경…");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(folder_gone_text(), "フォルダーが見つかりません");
        assert_eq!(fav_missing_note(), "見つかりません");
        let (remove, change) = folder_gone_buttons();
        assert_eq!(remove, "お気に入りから削除");
        assert_eq!(change, "場所を変更…");
        UI.store(prev, Ordering::Relaxed);
    }

    #[test]
    fn notice_buttons_stay_inside_and_do_not_overlap() {
        let (w, h) = miss_window(96, 140, 120);
        let (msg, remove, change) = miss_layout(w, h, 96, 140, 120);
        assert!(remove.r <= change.l);
        assert!(remove.l >= 0 && change.r <= w);
        assert!(remove.t >= 0 && remove.b <= h);
        assert!(msg.b <= remove.t);
        assert!(msg.r > msg.l && remove.r > remove.l && change.r > change.l);
        let narrow = miss_layout(160, h, 96, 200, 200);
        assert!(narrow.1.r <= narrow.2.l);
        assert!(narrow.2.r <= 160);
        assert_eq!(remove.b - remove.t, crate::BTN_H_FOOTER);
        assert!(hit_of(&(msg, remove, change), remove.l + 1, remove.t + 1) == MissHit::Remove);
        assert!(hit_of(&(msg, remove, change), change.l + 1, change.t + 1) == MissHit::Change);
        assert!(hit_of(&(msg, remove, change), 0, 0) == MissHit::None);
    }
}
