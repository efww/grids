// Save and load on the tab row. A session is a stack of windows: the back frame sits up-left
// and the front frame holds a 2×2 pane grid. The corner mark is not a disc. At 100% it is a
// hard 5×5 plus (keep) or a hard 5×5 up-arrow (restore). The arrow's head is an open
// chevron, so at 16px it is not a plus and not a flat bar. One empty pixel sits between the
// mark and the frame. Other DPIs are the same picture, nearest-neighbour, so every pixel stays on or off.
// The bitmap is one colour; a dirty save is that colour painted green.
// Gear, the tab list, the layout grid, the favourites-toolbar marks, and the pane arrows are the
// same kind of master. A straight stroke sits on whole pixels. A curve is a single-pixel step.

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

/// Hard toolbar and header glyphs. Same 1px weight as the basket and the eye.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LineIcon {
    /// Cog: 1px ring, six teeth, empty centre.
    Gear,
    /// Four horizontal bars. The tab list.
    List,
    /// 2×2 panes, 1px outer edge and 1px cross.
    Layout,
    /// Plus. Favourites Add, and a new pane tab.
    Add,
    /// Folder with a plus. New group.
    Group,
    /// Up arrow. Favourites move-up, and the pane parent button.
    Up,
    Down,
    /// I-beam. Rename.
    Rename,
    /// Bin. Delete.
    Del,
    Back,
    Fwd,
    /// Two offset frames. Open in another pane.
    OpenIn,
}

// Midpoint circle, radius 5, centred on pixel 7, with a radius-2 hole and a tooth on each
// of six directions. The flat top of the circle is one row, so the ring stays one pixel thick.
const GEAR16: [&str; 16] = [
    "................",
    ".......#........",
    ".....#####......",
    "....#.....#.....",
    "..##.......##...",
    "..#...###...#...",
    "..#..#...#..#...",
    "..#..#...#..#...",
    "..#..#...#..#...",
    "..#...###...#...",
    "..##.......##...",
    "....#.....#.....",
    ".....#####......",
    ".......#........",
    "................",
    "................",
];
const LIST16: [&str; 16] = [
    "................",
    "................",
    "................",
    "..############..",
    "................",
    "................",
    "..############..",
    "................",
    "................",
    "..############..",
    "................",
    "................",
    "..############..",
    "................",
    "................",
    "................",
];
const LAYOUT16: [&str; 16] = [
    "................",
    ".#############..",
    ".#.....#.....#..",
    ".#.....#.....#..",
    ".#.....#.....#..",
    ".#.....#.....#..",
    ".#.....#.....#..",
    ".#############..",
    ".#.....#.....#..",
    ".#.....#.....#..",
    ".#.....#.....#..",
    ".#.....#.....#..",
    ".#.....#.....#..",
    ".#############..",
    "................",
    "................",
];
const ADD16: [&str; 16] = [
    "................",
    "................",
    "................",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    "...#########....",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    "................",
    "................",
    "................",
    "................",
];
const GROUP16: [&str; 16] = [
    "................",
    "................",
    "................",
    "..#####.........",
    ".##############.",
    ".#............#.",
    ".#.....#......#.",
    ".#.....#......#.",
    ".#...#####....#.",
    ".#.....#......#.",
    ".#.....#......#.",
    ".#............#.",
    ".##############.",
    "................",
    "................",
    "................",
];
// Open chevron over a stem. The head is not a filled triangle, so it does not read as a plus.
const UP16: [&str; 16] = [
    "................",
    "................",
    "................",
    ".......#........",
    "......#.#.......",
    ".....#...#......",
    "....#.....#.....",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    "................",
    "................",
    "................",
];
const DOWN16: [&str; 16] = [
    "................",
    "................",
    "................",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    "....#.....#.....",
    ".....#...#......",
    "......#.#.......",
    ".......#........",
    "................",
    "................",
    "................",
];
const RENAME16: [&str; 16] = [
    "................",
    "................",
    "................",
    "...#########....",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    ".......#........",
    "...#########....",
    "................",
    "................",
    "................",
];
const DEL16: [&str; 16] = [
    "................",
    "................",
    "................",
    "......####......",
    "................",
    "..############..",
    "...#........#...",
    "...#..#..#..#...",
    "...#..#..#..#...",
    "...#..#..#..#...",
    "...#........#...",
    "...#........#...",
    "....########....",
    "................",
    "................",
    "................",
];
const BACK16: [&str; 16] = [
    "................",
    "................",
    "................",
    "................",
    "......#.........",
    ".....#..........",
    "....#...........",
    "...##########...",
    "....#...........",
    ".....#..........",
    "......#.........",
    "................",
    "................",
    "................",
    "................",
    "................",
];
const FWD16: [&str; 16] = [
    "................",
    "................",
    "................",
    "................",
    ".........#......",
    "..........#.....",
    "...........#....",
    "...##########...",
    "...........#....",
    "..........#.....",
    ".........#......",
    "................",
    "................",
    "................",
    "................",
    "................",
];
// Back frame at the upper left, front frame stepped down-right, so one pane sits over another.
const OPEN16: [&str; 16] = [
    "................",
    "................",
    "................",
    "...#######......",
    "...#.....#......",
    "...#.....#......",
    "...#..#######...",
    "...#..#.....#...",
    "...#..#.....#...",
    "...####.....#...",
    "......#.....#...",
    "......#.....#...",
    "......#######...",
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
    scale16(n, |x, y| ink16(x, y, kind))
}

/// Premultiplied top-down BGRA of [`session_icon_coverage`], ink `rgb` as 0xRRGGBB.
pub(crate) fn session_icon_bgra(n: i32, kind: SessionIcon, rgb: u32) -> Vec<u8> {
    coverage_bgra(&session_icon_coverage(n, kind), rgb)
}

/// Coverage, 0 or 255, for a [`LineIcon`]. Same nearest-neighbour scale as the session marks.
pub(crate) fn line_icon_coverage(n: i32, kind: LineIcon) -> Vec<u8> {
    scale16(n, |x, y| line16(x, y, kind))
}

/// Premultiplied top-down BGRA of [`line_icon_coverage`].
pub(crate) fn line_icon_bgra(n: i32, kind: LineIcon, rgb: u32) -> Vec<u8> {
    coverage_bgra(&line_icon_coverage(n, kind), rgb)
}

/// Premultiplied top-down BGRA. `rgb` is 0xRRGGBB. A zero coverage pixel stays zero.
pub(crate) fn coverage_bgra(cov: &[u8], rgb: u32) -> Vec<u8> {
    let mut px = vec![0u8; cov.len() * 4];
    for (i, &a) in cov.iter().enumerate() {
        let pre = |ch: u32| ((ch & 0xFF) * a as u32 / 255) as u8;
        px[i * 4..i * 4 + 4].copy_from_slice(&[pre(rgb), pre(rgb >> 8), pre(rgb >> 16), a]);
    }
    px
}

/// Nearest neighbour from the 16×16 master. A source pixel is copied whole, so coverage stays 0 or 255.
fn scale16(n: i32, on: impl Fn(i32, i32) -> bool) -> Vec<u8> {
    let n = n.max(1);
    let mut out = vec![0u8; (n * n) as usize];
    for y in 0..n {
        for x in 0..n {
            let (sx, sy) = if n == 16 {
                (x, y)
            } else {
                ((x * 16 / n).min(15), (y * 16 / n).min(15))
            };
            if on(sx, sy) {
                out[(y * n + x) as usize] = 255;
            }
        }
    }
    out
}

fn line_rows(kind: LineIcon) -> &'static [&'static str; 16] {
    match kind {
        LineIcon::Gear => &GEAR16,
        LineIcon::List => &LIST16,
        LineIcon::Layout => &LAYOUT16,
        LineIcon::Add => &ADD16,
        LineIcon::Group => &GROUP16,
        LineIcon::Up => &UP16,
        LineIcon::Down => &DOWN16,
        LineIcon::Rename => &RENAME16,
        LineIcon::Del => &DEL16,
        LineIcon::Back => &BACK16,
        LineIcon::Fwd => &FWD16,
        LineIcon::OpenIn => &OPEN16,
    }
}

fn line16(x: i32, y: i32, kind: LineIcon) -> bool {
    (0..16).contains(&x) && (0..16).contains(&y) && art(line_rows(kind), x, y)
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

    const LINE: [LineIcon; 12] = [
        LineIcon::Gear,
        LineIcon::List,
        LineIcon::Layout,
        LineIcon::Add,
        LineIcon::Group,
        LineIcon::Up,
        LineIcon::Down,
        LineIcon::Rename,
        LineIcon::Del,
        LineIcon::Back,
        LineIcon::Fwd,
        LineIcon::OpenIn,
    ];

    #[test]
    fn line_icons_stay_hard_edged_and_distinct() {
        for n in [16, 20, 24, 32] {
            let covs: Vec<Vec<u8>> = LINE.iter().map(|&k| line_icon_coverage(n, k)).collect();
            for c in &covs {
                assert!(c.iter().all(|&a| a == 0 || a == 255), "{n}");
                assert!(c.iter().any(|&a| a == 255), "{n}");
            }
            for i in 0..covs.len() {
                for j in (i + 1)..covs.len() {
                    assert_ne!(covs[i], covs[j], "{n} {i} vs {j}");
                }
            }
        }
    }

    #[test]
    fn list_is_four_bars_and_the_gear_has_a_hole() {
        let list = line_icon_coverage(16, LineIcon::List);
        let bars: Vec<i32> = (0..16).filter(|&y| (0..16).any(|x| at(&list, 16, x, y) == 255)).collect();
        assert_eq!(bars, vec![3, 6, 9, 12]);
        for y in bars {
            assert!((2..14).all(|x| at(&list, 16, x, y) == 255), "{y}");
            assert_eq!((at(&list, 16, 0, y), at(&list, 16, 15, y)), (0, 0));
        }
        let gear = line_icon_coverage(16, LineIcon::Gear);
        assert_eq!((at(&gear, 16, 7, 7), at(&gear, 16, 8, 8)), (0, 0));
        assert_eq!(at(&gear, 16, 7, 1), 255);
        let grid = line_icon_coverage(16, LineIcon::Layout);
        assert_eq!(at(&grid, 16, 1, 1), 255);
        assert_eq!(at(&grid, 16, 7, 7), 255);
        assert_eq!(at(&grid, 16, 3, 3), 0);
    }

    #[test]
    fn arrows_mirror_and_the_plus_is_one_pixel() {
        let up = line_icon_coverage(16, LineIcon::Up);
        let down = line_icon_coverage(16, LineIcon::Down);
        let back = line_icon_coverage(16, LineIcon::Back);
        let fwd = line_icon_coverage(16, LineIcon::Fwd);
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(at(&up, 16, x, y), at(&down, 16, x, 15 - y), "ud {x},{y}");
                assert_eq!(at(&back, 16, x, y), at(&fwd, 16, 15 - x, y), "bf {x},{y}");
            }
        }
        let add = line_icon_coverage(16, LineIcon::Add);
        assert!((3..12).all(|i| at(&add, 16, 7, i) == 255));
        assert!((3..12).all(|x| at(&add, 16, x, 7) == 255));
        // The cross is one pixel thick: the cell beside the stem is empty.
        assert_eq!(at(&add, 16, 8, 6), 0);
    }
}
