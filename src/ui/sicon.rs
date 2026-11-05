// Save and load on the tab row. A session is a stack of windows: the back frame sits up-left
// and the front frame holds a 2×2 pane grid. The corner mark is not a disc. At 100% it is a
// hard 5×5 plus (keep) or a hard 5×5 up-arrow (restore). The arrow's head is an open
// chevron, so at 16px it is not a plus and not a flat bar. One empty pixel sits between the
// mark and the frame. Other DPIs are the same picture, nearest-neighbour, so every pixel stays on or off.
// The bitmap is one colour; a dirty save is that colour painted green.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SessionIcon {
    Save,
    Load,
    /// File basket: a handle over a rim and a tapered basket with two slats.
    Basket,
    /// Folder watch: an open eye with a solid pupil.
    Watch,
}

/// 16×16 masters for the basket and the eye, `#` on. Same 1px line as the session frames.
const BASKET16: [&str; 16] = [
    "................",
    "................",
    "......####......",
    ".....#....#.....",
    "....#......#....",
    "....#......#....",
    ".##############.",
    "..#..#....#..#..",
    "..#..#....#..#..",
    "...#.#....#.#...",
    "...#.#....#.#...",
    "...#.#....#.#...",
    "....##....##....",
    "....########....",
    "................",
    "................",
];
const WATCH16: [&str; 16] = [
    "................",
    "................",
    "................",
    "................",
    ".....######.....",
    "...##......##...",
    "..#....##....#..",
    ".#....####....#.",
    ".#....####....#.",
    "..#....##....#..",
    "...##......##...",
    ".....######.....",
    "................",
    "................",
    "................",
    "................",
];

fn art(rows: &[&str; 16], x: i32, y: i32) -> bool {
    rows[y as usize].as_bytes()[x as usize] == b'#'
}

/// Device pixels for the icon at `dpi` (96 = 100%). 16px at 96, tracking the 14px icon font
/// and the 15px star without crowding the 24px-tall toolbar button.
pub(crate) fn session_icon_px(dpi: i32) -> i32 {
    (16 * dpi.max(1) / 96).max(1)
}

/// Coverage, 0 or 255, row-major `n`×`n`. `n` is [`session_icon_px`]. No partial pixels.
pub(crate) fn session_icon_coverage(n: i32, kind: SessionIcon) -> Vec<u8> {
    let n = n.max(1);
    let mut out = vec![0u8; (n * n) as usize];
    for y in 0..n {
        for x in 0..n {
            let (sx, sy) = if n == 16 {
                (x, y)
            } else {
                ((x * 16 / n).min(15), (y * 16 / n).min(15))
            };
            if ink16(sx, sy, kind) {
                out[(y * n + x) as usize] = 255;
            }
        }
    }
    out
}

/// Premultiplied top-down BGRA of [`session_icon_coverage`], ink `rgb` as 0xRRGGBB.
pub(crate) fn session_icon_bgra(n: i32, kind: SessionIcon, rgb: u32) -> Vec<u8> {
    let cov = session_icon_coverage(n, kind);
    let mut px = vec![0u8; cov.len() * 4];
    for (i, &a) in cov.iter().enumerate() {
        let pre = |ch: u32| ((ch & 0xFF) * a as u32 / 255) as u8;
        px[i * 4..i * 4 + 4].copy_from_slice(&[pre(rgb), pre(rgb >> 8), pre(rgb >> 16), a]);
    }
    px
}

// 16×16 master, one pixel per cell. Front frame ends at column/row 9. Column 10 and row 10
// stay empty (the 1px gap). The mark occupies 11..=15.
fn ink16(x: i32, y: i32, kind: SessionIcon) -> bool {
    if !(0..16).contains(&x) || !(0..16).contains(&y) {
        return false;
    }
    match kind {
        SessionIcon::Basket => return art(&BASKET16, x, y),
        SessionIcon::Watch => return art(&WATCH16, x, y),
        _ => {}
    }
    if x >= 11 && y >= 11 {
        return mark5(x - 11, y - 11, kind);
    }
    if (3..=9).contains(&x) && (3..=9).contains(&y) {
        return frame(x, y, 3, 3, 9, 9)
            || (x == 6 && (4..=8).contains(&y))
            || (y == 6 && (4..=8).contains(&x));
    }
    frame(x, y, 1, 1, 7, 7)
}

fn frame(x: i32, y: i32, l: i32, t: i32, r: i32, b: i32) -> bool {
    x >= l && x <= r && y >= t && y <= b && (x == l || x == r || y == t || y == b)
}

/// 5×5. Save is a plus. Load is an up-arrow: a chevron head over a stem. A filled triangle
/// shares the plus, and a flat bar reads as a T, so the head stays open.
fn mark5(x: i32, y: i32, kind: SessionIcon) -> bool {
    match kind {
        SessionIcon::Save => x == 2 || y == 2,
        SessionIcon::Basket | SessionIcon::Watch => false,
        SessionIcon::Load => {
            (y == 0 && x == 2)
                || (y == 1 && (x == 1 || x == 3))
                || (y == 2 && (x == 0 || x == 4))
                || (y >= 3 && x == 2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(c: &[u8], n: i32, x: i32, y: i32) -> u8 {
        c[(y * n + x) as usize]
    }

    #[test]
    fn sizes_track_the_toolbar_dpi_steps() {
        assert_eq!(session_icon_px(96), 16);
        assert_eq!(session_icon_px(120), 20);
        assert_eq!(session_icon_px(144), 24);
        assert_eq!(session_icon_px(192), 32);
        assert_eq!(session_icon_px(1), 1);
    }

    #[test]
    fn at_100_percent_badges_are_hard_5px_and_clear_of_the_grid() {
        let save = session_icon_coverage(16, SessionIcon::Save);
        let load = session_icon_coverage(16, SessionIcon::Load);
        assert!(save.iter().chain(&load).all(|&a| a == 0 || a == 255));
        // One empty pixel between the front frame (ends at 9) and the mark (starts at 11).
        for i in 0..16 {
            assert_eq!((at(&save, 16, i, 10), at(&save, 16, 10, i)), (0, 0));
            assert_eq!((at(&load, 16, i, 10), at(&load, 16, 10, i)), (0, 0));
        }
        let plus = ["..#..", "..#..", "#####", "..#..", "..#.."];
        let arrow = ["..#..", ".#.#.", "#...#", "..#..", "..#.."];
        for (mark, cov) in [(plus, &save), (arrow, &load)] {
            for dy in 0..5 {
                for dx in 0..5 {
                    let on = mark[dy].as_bytes()[dx] == b'#';
                    assert_eq!(
                        at(cov, 16, 11 + dx as i32, 11 + dy as i32) == 255,
                        on,
                        "{dx},{dy}"
                    );
                }
            }
        }
        // The 2×2 cross is shared, and the two icons match everywhere outside the mark.
        assert_eq!((at(&save, 16, 6, 6), at(&load, 16, 6, 6)), (255, 255));
        for y in 0..16 {
            for x in 0..16 {
                if x >= 11 && y >= 11 {
                    continue;
                }
                assert_eq!(at(&save, 16, x, y), at(&load, 16, x, y), "{x},{y}");
            }
        }
        assert_eq!(at(&save, 16, 1, 1), 255);
        assert_eq!(at(&save, 16, 0, 0), 0);
    }

    #[test]
    fn other_dpis_stay_hard_edged() {
        for n in [20, 24, 32] {
            let save = session_icon_coverage(n, SessionIcon::Save);
            let load = session_icon_coverage(n, SessionIcon::Load);
            assert!(save.iter().chain(&load).all(|&a| a == 0 || a == 255), "{n}");
            assert_ne!(save, load, "{n}");
        }
    }

    #[test]
    fn basket_and_watch_are_hard_edged_line_icons() {
        for n in [16, 20, 24, 32] {
            let b = session_icon_coverage(n, SessionIcon::Basket);
            let w = session_icon_coverage(n, SessionIcon::Watch);
            assert!(b.iter().chain(&w).all(|&a| a == 0 || a == 255), "{n}");
            assert!(b.iter().any(|&a| a == 255) && w.iter().any(|&a| a == 255), "{n}");
            assert_ne!(b, w, "{n}");
            assert_ne!(b, session_icon_coverage(n, SessionIcon::Save), "{n}");
        }
        // The rim spans the basket; the eye's pupil sits at the centre with a clear ring around it.
        let b = session_icon_coverage(16, SessionIcon::Basket);
        assert!((1..15).all(|x| at(&b, 16, x, 6) == 255));
        let w = session_icon_coverage(16, SessionIcon::Watch);
        assert_eq!((at(&w, 16, 7, 7), at(&w, 16, 8, 8)), (255, 255));
        assert_eq!((at(&w, 16, 4, 7), at(&w, 16, 11, 8)), (0, 0));
    }

    #[test]
    fn bgra_is_premultiplied_from_coverage() {
        let cov = session_icon_coverage(16, SessionIcon::Save);
        let px = session_icon_bgra(16, SessionIcon::Save, 0x112233);
        assert!(cov.iter().any(|&a| a == 255));
        assert!(cov.iter().any(|&a| a == 0));
        for (i, &a) in cov.iter().enumerate() {
            let p = &px[i * 4..i * 4 + 4];
            if a == 0 {
                assert_eq!(p, &[0, 0, 0, 0]);
            } else if a == 255 {
                assert_eq!(p, &[0x33, 0x22, 0x11, 255]);
            }
        }
    }
}
