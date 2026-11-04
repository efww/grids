//! Shared item-anchored hover tip: text and position once on enter, quiet while the key stays,
//! hide once on leave. Placement keeps a fixed 4px gap and never covers the item.

use std::cell::Cell;
use std::mem::size_of;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Controls::{
    TTF_ABSOLUTE, TTF_TRACK, TTM_ADDTOOLW, TTM_DELTOOLW, TTM_GETBUBBLESIZE, TTM_POP, TTM_SETMAXTIPWIDTH,
    TTM_TRACKACTIVATE, TTM_TRACKPOSITION, TTM_UPDATETIPTEXTW, TTTOOLINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SendMessageW;

use crate::app;

thread_local! {
    /// Set while tip messages run. Nested WM_MOUSEMOVE must not re-enter tip_enter/leave.
    static TIP_PUMP: Cell<bool> = const { Cell::new(false) };
}

struct PumpGuard;
impl Drop for PumpGuard {
    fn drop(&mut self) {
        TIP_PUMP.with(|c| c.set(false));
    }
}


/// Gap between the item edge and the tip. Only 4/8/16 spacing is used in the UI.
pub(crate) const TIP_GAP: i32 = 4;

/// True while tip_enter/leave are pumping synchronous tooltip messages.
pub(crate) fn tip_pumping() -> bool {
    TIP_PUMP.with(|c| c.get())
}


/// Preferred side of the item for the tip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TipSide {
    /// Horizontal items: window tabs, pane tabs, address bar, toolbar buttons.
    Below,
    /// Vertical list rows (favourites side / pop).
    Right,
}

/// Live tracking tip: same key → no tip messages while shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TipLive {
    pub tool: usize,
    pub key: u64,
    pub text: String,
}

fn tip_info(owner: HWND, id: usize, label: &HSTRING) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: id,
        lpszText: PWSTR(label.as_ptr() as *mut u16),
        ..Default::default()
    }
}

/// Monitor work area containing `r`, or a large fallback.
pub(crate) unsafe fn tip_work_area(r: RECT) -> RECT {
    let mut mi = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool()
        && mi.rcWork.right > mi.rcWork.left
        && mi.rcWork.bottom > mi.rcWork.top
    {
        return mi.rcWork;
    }
    RECT {
        left: r.left - 2000,
        top: r.top - 2000,
        right: r.right + 2000,
        bottom: r.bottom + 2000,
    }
}

/// Map a client `RECT` on `hwnd` to screen coordinates.
pub(crate) unsafe fn tip_screen_rect(hwnd: HWND, client: RECT) -> RECT {
    let mut tl = POINT {
        x: client.left,
        y: client.top,
    };
    let mut br = POINT {
        x: client.right,
        y: client.bottom,
    };
    _ = ClientToScreen(hwnd, &mut tl);
    _ = ClientToScreen(hwnd, &mut br);
    RECT {
        left: tl.x,
        top: tl.y,
        right: br.x,
        bottom: br.y,
    }
}

fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if hi < lo {
        return lo;
    }
    v.max(lo).min(hi)
}

/// Top-left of the tip so it sits outside `item` with [`TIP_GAP`], preferred `side`,
/// flipping to the opposite side when the work area is tight. Never overlaps `item`.
pub(crate) fn tip_place(
    item: RECT,
    tip_w: i32,
    tip_h: i32,
    prefer: TipSide,
    work: RECT,
) -> (i32, i32) {
    let tip_w = tip_w.max(1);
    let tip_h = tip_h.max(1);
    let x_left = clamp(item.left, work.left, work.right - tip_w);
    // Favourites rows: sit beside the row, vertically centred on it (still 4px from the side).
    // When the tip is taller than the row, pin to the row top.
    let y_row = {
        let row_h = (item.bottom - item.top).max(0);
        let ideal = if tip_h < row_h {
            item.top + (row_h - tip_h) / 2
        } else {
            item.top
        };
        clamp(ideal, work.top, work.bottom - tip_h)
    };

    let try_below = || {
        let y = item.bottom + TIP_GAP;
        (y + tip_h <= work.bottom && y >= item.bottom + TIP_GAP).then_some((x_left, y))
    };
    let try_above = || {
        let y = item.top - TIP_GAP - tip_h;
        (y >= work.top && y + tip_h <= item.top - TIP_GAP).then_some((x_left, y))
    };
    let try_right = || {
        let x = item.right + TIP_GAP;
        (x + tip_w <= work.right && x >= item.right + TIP_GAP).then_some((x, y_row))
    };
    let try_left = || {
        let x = item.left - TIP_GAP - tip_w;
        (x >= work.left && x + tip_w <= item.left - TIP_GAP).then_some((x, y_row))
    };

    match prefer {
        TipSide::Below => try_below()
            .or_else(try_above)
            .or_else(try_right)
            .or_else(try_left)
            .unwrap_or_else(|| {
                // Last resort: pin just below the item, then clamp into work without covering it.
                let y = (item.bottom + TIP_GAP).min(work.bottom - tip_h).max(item.bottom + TIP_GAP);
                (x_left, y)
            }),
        TipSide::Right => try_right()
            .or_else(try_left)
            .or_else(try_below)
            .or_else(try_above)
            .unwrap_or_else(|| {
                let x = (item.right + TIP_GAP).min(work.right - tip_w).max(item.right + TIP_GAP);
                (x, y_row)
            }),
    }
}

fn pack_xy(x: i32, y: i32) -> LPARAM {
    LPARAM((x as u16 as usize as isize) | ((y as u16 as usize as isize) << 16))
}

/// Show `text` for `tool` once for `key`, anchored to `item` (screen). Same key+text while
/// already shown sends nothing. Empty text hides when that tool is the live one.
pub(crate) unsafe fn tip_enter(
    owner: HWND,
    tool: usize,
    key: u64,
    text: &str,
    item: RECT,
    prefer: TipSide,
    installed: bool,
) -> (bool, bool) {
    let tip = app(|a| a.tip);
    if tip.0.is_null() || owner.0.is_null() {
        return (installed, false);
    }
    if text.is_empty() {
        let (inst, _) = tip_leave(owner, tool, installed);
        return (inst, false);
    }
    // Same item still showing: no tip messages (enter once).
    if app(|a| {
        a.hover_tip
            .as_ref()
            .is_some_and(|l| l.tool == tool && l.key == key && l.text == text)
    }) {
        return (true, true);
    }
    if TIP_PUMP.with(|c| c.replace(true)) {
        return (installed, app(|a| a.hover_tip.as_ref().is_some_and(|l| l.tool == tool)));
    }
    let _pump = PumpGuard;
    // One live tracking tip at a time: deactivate the previous tool if it differs.
    if let Some(prev) = app(|a| a.hover_tip.clone()) {
        if prev.tool != tool {
            let label = HSTRING::from("");
            let ti = tip_info(HWND::default(), prev.tool, &label);
            let lp = LPARAM(&ti as *const _ as isize);
            SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(lp));
            SendMessageW(tip, TTM_POP, None, None);
            app(|a| {
                a.hover_tip = None;
                if prev.tool == 10 {
                    a.fav_tip_shown = false;
                }
                if prev.tool == 9 {
                    a.addr_tip_shown = false;
                    a.addr_tip_pane = HWND::default();
                }
            });
        }
    }
    let label = HSTRING::from(text);
    let ti = tip_info(owner, tool, &label);
    let lp = LPARAM(&ti as *const _ as isize);
    SendMessageW(tip, TTM_SETMAXTIPWIDTH, Some(WPARAM(0)), Some(LPARAM(720)));
    let mut now_installed = installed;
    if !now_installed {
        SendMessageW(tip, TTM_ADDTOOLW, None, Some(lp));
        now_installed = true;
    }
    SendMessageW(tip, TTM_UPDATETIPTEXTW, None, Some(lp));
    // Measure bubble so placement keeps the tip off the item.
    let size = SendMessageW(tip, TTM_GETBUBBLESIZE, None, Some(lp));
    let (tw, th) = if size.0 != 0 {
        ((size.0 & 0xffff) as i32, ((size.0 >> 16) & 0xffff) as i32)
    } else {
        // Fallback estimate before first paint.
        let lines = text.split('\n').count().max(1) as i32;
        let max_chars = text.split('\n').map(|s| s.chars().count()).max().unwrap_or(8) as i32;
        (max_chars * 8 + 16, lines * 18 + 8)
    };
    let work = tip_work_area(item);
    let (x, y) = tip_place(item, tw, th, prefer, work);
    SendMessageW(tip, TTM_TRACKPOSITION, None, Some(pack_xy(x, y)));
    SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(1)), Some(lp));
    app(|a| {
        a.hover_tip = Some(TipLive {
            tool,
            key,
            text: text.to_string(),
        });
    });
    (now_installed, true)
}

/// Hide the tracking tip for `tool` once. No-op when that tool is not the live tip.
/// Returns `(installed_unchanged, was_shown)`.
pub(crate) unsafe fn tip_leave(owner: HWND, tool: usize, installed: bool) -> (bool, bool) {
    let (tip, was) = app(|a| {
        let was = a
            .hover_tip
            .as_ref()
            .is_some_and(|l| l.tool == tool);
        if was {
            a.hover_tip = None;
        }
        (a.tip, was)
    });
    if !was || tip.0.is_null() || owner.0.is_null() {
        return (installed, false);
    }
    if TIP_PUMP.with(|c| c.replace(true)) {
        return (installed, true);
    }
    let _pump = PumpGuard;
    if installed {
        let label = HSTRING::from("");
        let ti = tip_info(owner, tool, &label);
        let lp = LPARAM(&ti as *const _ as isize);
        SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(lp));
        SendMessageW(tip, TTM_POP, None, None);
    }
    (installed, true)
}

/// The owner window is going away. Take `tool` off the shared tooltip.
pub(crate) unsafe fn tip_drop(owner: HWND, tool: usize) {
    let (tip, live) = app(|a| {
        let live = a.hover_tip.as_ref().is_some_and(|l| l.tool == tool);
        if live {
            a.hover_tip = None;
        }
        (a.tip, live)
    });
    if tip.0.is_null() || owner.0.is_null() {
        return;
    }
    let label = HSTRING::from("");
    let ti = tip_info(owner, tool, &label);
    let lp = LPARAM(&ti as *const _ as isize);
    if live {
        SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(lp));
        SendMessageW(tip, TTM_POP, None, None);
    }
    SendMessageW(tip, TTM_DELTOOLW, None, Some(lp));
}


#[cfg(test)]
mod tests {
    use super::*;

    fn r(l: i32, t: i32, right: i32, b: i32) -> RECT {
        RECT {
            left: l,
            top: t,
            right,
            bottom: b,
        }
    }

    #[test]
    fn below_keeps_four_px_and_left_align() {
        let item = r(100, 50, 200, 80);
        let work = r(0, 0, 1000, 800);
        let (x, y) = tip_place(item, 120, 24, TipSide::Below, work);
        assert_eq!(x, 100);
        assert_eq!(y, 80 + TIP_GAP);
    }

    #[test]
    fn below_flips_above_near_bottom() {
        let item = r(100, 760, 200, 790);
        let work = r(0, 0, 1000, 800);
        let (x, y) = tip_place(item, 120, 24, TipSide::Below, work);
        assert_eq!(x, 100);
        assert_eq!(y, 760 - TIP_GAP - 24);
        // Tip bottom stays above item top.
        assert!(y + 24 <= item.top - TIP_GAP);
    }

    #[test]
    fn right_keeps_four_px_and_row_center() {
        let item = r(10, 100, 180, 124); // row_h=24, tip_h=24 -> pin top
        let work = r(0, 0, 1000, 800);
        let (x, y) = tip_place(item, 200, 24, TipSide::Right, work);
        assert_eq!(x, 180 + TIP_GAP);
        assert_eq!(y, 100);
        // Shorter tip centres on the row.
        let (x2, y2) = tip_place(item, 200, 16, TipSide::Right, work);
        assert_eq!(x2, 180 + TIP_GAP);
        assert_eq!(y2, 100 + (24 - 16) / 2);
    }

    #[test]
    fn right_flips_left_near_edge() {
        let item = r(800, 100, 980, 124);
        let work = r(0, 0, 1000, 800);
        let (x, y) = tip_place(item, 200, 24, TipSide::Right, work);
        assert_eq!(x, 800 - TIP_GAP - 200);
        assert_eq!(y, 100);
        assert!(x + 200 <= item.left - TIP_GAP);
    }

    #[test]
    fn right_falls_to_below_when_sides_blocked() {
        let item = r(10, 100, 990, 124);
        let work = r(0, 0, 1000, 800);
        let (x, y) = tip_place(item, 200, 24, TipSide::Right, work);
        assert_eq!(x, 10);
        assert_eq!(y, 124 + TIP_GAP);
    }

    #[test]
    fn never_overlaps_item_horizontally_or_vertically() {
        let item = r(100, 100, 300, 140);
        let work = r(0, 0, 1000, 800);
        for prefer in [TipSide::Below, TipSide::Right] {
            let (x, y) = tip_place(item, 80, 30, prefer, work);
            let tip = r(x, y, x + 80, y + 30);
            let overlap_x = tip.left < item.right && tip.right > item.left;
            let overlap_y = tip.top < item.bottom && tip.bottom > item.top;
            assert!(
                !(overlap_x && overlap_y),
                "tip overlaps item for {prefer:?}: tip={tip:?} item={item:?}"
            );
            // Gap on the preferred contact edge when that side was used.
            if prefer == TipSide::Below && tip.top >= item.bottom {
                assert_eq!(tip.top - item.bottom, TIP_GAP);
            }
            if prefer == TipSide::Right && tip.left >= item.right {
                assert_eq!(tip.left - item.right, TIP_GAP);
            }
        }
    }
}