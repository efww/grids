// Toolbar, favourites-window, and pane-header icons, drawn from vector masters.
// A master is round-capped strokes, arcs, rings, discs, and rounded boxes in icon units:
// 1 unit is 1px at 96 DPI, and the origin is the centre of a 17-unit box. Every stroke centre
// is hinted onto the pixel grid (pixel centres while the stroke core is odd, pixel edges while
// it is even), so a straight stroke keeps a solid core and only its edges and the curves are
// anti-aliased. Coverage is supersampled 3×3, the same factor as the toggles.
// The bitmap is one colour; a dirty save is that colour painted green.

use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LineIcon {
    /// Cog: eight rounded teeth around a hole.
    Gear,
    /// Three bullets and three bars. The tab list.
    List,
    /// 2×2 panes in a rounded frame.
    Layout,
    /// Plus. Favourites Add, and a new pane tab.
    Add,
    /// Folder with a plus. New group.
    Group,
    /// Up arrow. Favourites move-up, and the pane parent button.
    Up,
    Down,
    /// Text field with an I-beam. Rename.
    Rename,
    /// Bin with a lid, a handle, and two slats. Delete.
    Del,
    Back,
    Fwd,
    /// ⇄. Open in another pane.
    OpenIn,
    /// A stack of windows: the back frame up-left, the front frame a 2×2 grid, a plus badge.
    Save,
    /// The same stack with an up-arrow badge.
    Load,
    /// File basket: a handle over a rim and a tapered basket with two slats.
    Basket,
    /// Folder watch: an open eye with a solid pupil.
    Watch,
    /// Five-point star with rounded tips. The favourites toolbar button.
    Star,
    /// The same star, filled. A favourite folder.
    StarFilled,
    /// The outline with a plus in its middle. Add to favourites.
    StarAdd,
}

/// Design box, icon units. Odd, so a 1px stroke through the middle is centred.
const BOX: f32 = 17.0;
/// Stroke width at 96 DPI. A 1px core with a light fringe each side.
const STROKE: f32 = 1.35;
/// Supersample factor per axis.
const AA: i32 = 3;

fn scale(dpi: i32) -> f32 {
    dpi.max(1) as f32 / 96.0
}

/// Whole device pixels in the solid middle of a stroke: 1 up to 125%, 2 from 150%.
fn stroke_core(s: f32) -> i32 {
    ((STROKE * s + 0.2).floor() as i32).max(1)
}

/// Stroke width in device pixels: the core plus a soft fringe of 0.125px to 0.3px each side.
pub(crate) fn line_icon_stroke(dpi: i32) -> f32 {
    let s = scale(dpi);
    let core = stroke_core(s) as f32;
    (STROKE * s).clamp(core + 0.25, core + 0.6)
}

/// Device pixels for the icon box at `dpi` (96 = 100%): 17 at 100%, 21 at 125%, 26 at 150%,
/// 34 at 200%. The parity follows the stroke core, so a centred stroke stays centred.
pub(crate) fn line_icon_px(dpi: i32) -> i32 {
    let s = scale(dpi);
    let want = BOX * s;
    let mut n = want.round() as i32;
    if (n - stroke_core(s)) % 2 != 0 {
        n = if (n as f32) < want { n + 1 } else { n - 1 };
    }
    n.max(1)
}

/// Coverage 0..=255, row-major `n`×`n`. `n` is [`line_icon_px`] for `dpi`.
pub(crate) fn line_icon_coverage(n: i32, dpi: i32, kind: LineIcon) -> Vec<u8> {
    let mut pen = Pen::new(n, dpi);
    master(kind, &mut pen);
    pen.raster()
}

/// Premultiplied top-down BGRA of [`line_icon_coverage`], ink `rgb` as 0xRRGGBB.
pub(crate) fn line_icon_bgra(n: i32, dpi: i32, kind: LineIcon, rgb: u32) -> Vec<u8> {
    coverage_bgra(&line_icon_coverage(n, dpi, kind), rgb)
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

fn master(kind: LineIcon, p: &mut Pen) {
    use LineIcon::*;
    match kind {
        List => {
            for y in [-4.0, 0.0, 4.0] {
                p.dot((-6.0, y), 1.2);
                p.seg((-3.0, y), (6.0, y));
            }
        }
        Layout => {
            p.frame((-6.0, -6.0), (6.0, 6.0), 2.0);
            p.seg((0.0, -6.0), (0.0, 6.0));
            p.seg((-6.0, 0.0), (6.0, 0.0));
        }
        Gear => {
            p.cog((0.0, 0.0), 4.9, 7.0, 1.05, 8);
            p.ring((0.0, 0.0), 2.0);
        }
        Add => p.cross((0.0, 0.0), 6.0),
        Group => {
            p.path(&[(-7.0, 6.0), (-7.0, -6.0), (-2.0, -6.0), (0.0, -4.0), (7.0, -4.0), (7.0, 6.0)], 1.5, true);
            p.cross((0.0, 1.0), 2.0);
        }
        Up | Down | Back | Fwd => {
            p.turn = match kind {
                Down => Turn::FlipY,
                Back => Turn::Left,
                Fwd => Turn::Right,
                _ => Turn::None,
            };
            p.seg((0.0, -6.0), (0.0, 6.0));
            p.path(&[(-5.0, -1.0), (0.0, -6.0), (5.0, -1.0)], 0.0, false);
        }
        Rename => {
            p.frame((-7.0, -3.0), (7.0, 3.0), 1.5);
            p.cut_seg((3.0, -6.0), (3.0, 6.0), 1.0);
            p.seg((3.0, -6.0), (3.0, 6.0));
            p.seg((1.0, -6.0), (5.0, -6.0));
            p.seg((1.0, 6.0), (5.0, 6.0));
        }
        Del => {
            p.seg((-6.0, -4.0), (6.0, -4.0));
            p.path(&[(-2.0, -4.0), (-2.0, -6.0), (2.0, -6.0), (2.0, -4.0)], 1.0, false);
            p.path(&[(-5.0, -4.0), (-4.0, 6.0), (4.0, 6.0), (5.0, -4.0)], 1.5, false);
            p.seg((-2.0, -1.0), (-2.0, 3.0));
            p.seg((2.0, -1.0), (2.0, 3.0));
        }
        OpenIn => {
            p.seg((-6.0, -3.0), (6.0, -3.0));
            p.path(&[(3.0, -6.0), (6.0, -3.0), (3.0, 0.0)], 0.0, false);
            p.seg((6.0, 3.0), (-6.0, 3.0));
            p.path(&[(-3.0, 0.0), (-6.0, 3.0), (-3.0, 6.0)], 0.0, false);
        }
        Save | Load => {
            p.frame((-7.0, -7.0), (1.0, 1.0), 1.5);
            p.cut_slab((-4.0, -4.0), (4.0, 4.0), 1.5);
            p.frame((-4.0, -4.0), (4.0, 4.0), 1.5);
            p.seg((0.0, -4.0), (0.0, 4.0));
            p.seg((-4.0, 0.0), (4.0, 0.0));
            // The badge sits in a round notch, one unit clear of the frames.
            p.cut_disc((5.0, 5.0), 3.0);
            if kind == Save {
                p.cross((5.0, 5.0), 2.0);
            } else {
                p.seg((5.0, 3.0), (5.0, 7.0));
                p.path(&[(3.0, 5.0), (5.0, 3.0), (7.0, 5.0)], 0.0, false);
            }
        }
        Basket => {
            p.seg((-6.5, -1.0), (6.5, -1.0));
            p.arc((0.0, -1.0), 4.0, PI, PI);
            p.path(&[(-5.5, -1.0), (-3.5, 5.0), (3.5, 5.0), (5.5, -1.0)], 1.5, false);
            p.seg((-2.0, 1.0), (-2.0, 3.0));
            p.seg((2.0, 1.0), (2.0, 3.0));
        }
        Watch => {
            p.lens((0.0, 0.0), 6.6, 4.0);
            p.dot((0.0, 0.0), 2.2);
        }
        Star | StarFilled | StarAdd => {
            p.star((0.0, 1.0), kind == StarFilled);
            if kind == StarAdd {
                p.cross((0.0, 1.0), 2.0);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Pt(f32, f32);

impl Pt {
    fn sub(self, o: Pt) -> Pt {
        Pt(self.0 - o.0, self.1 - o.1)
    }
    fn len(self) -> f32 {
        self.0.hypot(self.1)
    }
}

#[derive(Clone, Debug)]
enum Shape {
    /// Round-capped stroke.
    Seg(Pt, Pt),
    /// Stroked arc from `start` through `sweep` radians, clockwise on screen when positive.
    Arc { c: Pt, r: f32, start: f32, sweep: f32 },
    /// Stroked rounded box between two stroke-centre corners.
    Frame(Pt, Pt, f32),
    /// Filled rounded box. Only cut.
    Slab(Pt, Pt, f32),
    Disc(Pt, f32),
    /// Almond outline: two arcs meeting at (±`hw`, 0) around `c`, `hh` high each side.
    Lens { c: Pt, hw: f32, hh: f32 },
    /// Cog outline: a body disc with `teeth` rounded teeth, the first one straight up.
    Cog { c: Pt, body: f32, tip: f32, half: f32, teeth: u32 },
    /// Five-point star with rounded tips: the outline, or the outline and its inside.
    Star { c: Pt, outer: f32, inner: f32, round: f32, fill: bool },
}

enum Op {
    Ink(Shape),
    /// Clear everything drawn so far within `gap` of this shape's ink.
    Cut(Shape, f32),
}

#[derive(Clone, Copy, PartialEq)]
enum Turn {
    None,
    FlipY,
    Left,
    Right,
}

/// Turns design coordinates into device pixels and collects the shapes of one master.
struct Pen {
    n: i32,
    s: f32,
    /// Box centre, device pixels.
    c: f32,
    /// Half the stroke, device pixels.
    hw: f32,
    turn: Turn,
    ops: Vec<Op>,
}

impl Pen {
    fn new(n: i32, dpi: i32) -> Pen {
        let n = n.max(1);
        Pen { n, s: scale(dpi), c: n as f32 / 2.0, hw: line_icon_stroke(dpi) / 2.0, turn: Turn::None, ops: Vec::new() }
    }

    fn turned(&self, (x, y): (f32, f32)) -> (f32, f32) {
        match self.turn {
            Turn::None => (x, y),
            Turn::FlipY => (x, -y),
            Turn::Left => (y, -x),
            Turn::Right => (-y, x),
        }
    }

    /// A whole-unit coordinate is hinted: its offset from the centre is rounded to whole
    /// pixels, symmetrically about zero, so a mirrored master stays mirrored. A fractional
    /// one (a slanted side, the end of a curve) scales freely.
    fn at(&self, v: (f32, f32)) -> Pt {
        let (x, y) = self.turned(v);
        let px = |u: f32| self.c + if u.fract() == 0.0 { (u * self.s).round() } else { u * self.s };
        Pt(px(x), px(y))
    }

    /// Hinted length.
    fn len(&self, v: f32) -> f32 {
        (v * self.s).round().max(1.0)
    }

    fn ink(&mut self, sh: Shape) {
        self.ops.push(Op::Ink(sh));
    }

    fn seg(&mut self, a: (f32, f32), b: (f32, f32)) {
        let sh = Shape::Seg(self.at(a), self.at(b));
        self.ink(sh);
    }

    fn cut_seg(&mut self, a: (f32, f32), b: (f32, f32), gap: f32) {
        let sh = Shape::Seg(self.at(a), self.at(b));
        self.ops.push(Op::Cut(sh, gap * self.s));
    }

    /// Clear a disc of radius `r` units plus half a stroke, so a badge of arm `r - gap` keeps `gap`.
    fn cut_disc(&mut self, c: (f32, f32), r: f32) {
        let sh = Shape::Disc(self.at(c), self.len(r) + self.hw);
        self.ops.push(Op::Cut(sh, 0.0));
    }

    fn cut_slab(&mut self, a: (f32, f32), b: (f32, f32), r: f32) {
        let sh = Shape::Slab(self.at(a), self.at(b), r * self.s);
        self.ops.push(Op::Cut(sh, 0.0));
    }

    /// A plus centred on `c` with arms `arm` units long. The arms are hinted as one length, so
    /// both sides match at every DPI.
    fn cross(&mut self, c: (f32, f32), arm: f32) {
        let o = self.at(c);
        let a = self.len(arm);
        self.ink(Shape::Seg(Pt(o.0 - a, o.1), Pt(o.0 + a, o.1)));
        self.ink(Shape::Seg(Pt(o.0, o.1 - a), Pt(o.0, o.1 + a)));
    }

    fn dot(&mut self, c: (f32, f32), r: f32) {
        let sh = Shape::Disc(self.at(c), r * self.s);
        self.ink(sh);
    }

    fn ring(&mut self, c: (f32, f32), r: f32) {
        let sh = Shape::Arc { c: self.at(c), r: self.len(r), start: 0.0, sweep: TAU };
        self.ink(sh);
    }

    fn arc(&mut self, c: (f32, f32), r: f32, start: f32, sweep: f32) {
        let sh = Shape::Arc { c: self.at(c), r: self.len(r), start, sweep };
        self.ink(sh);
    }

    fn frame(&mut self, a: (f32, f32), b: (f32, f32), r: f32) {
        let sh = Shape::Frame(self.at(a), self.at(b), r * self.s);
        self.ink(sh);
    }

    /// The corners are points, so only the lid height is hinted.
    fn lens(&mut self, c: (f32, f32), hw: f32, hh: f32) {
        let sh = Shape::Lens { c: self.at(c), hw: hw * self.s, hh: self.len(hh) };
        self.ink(sh);
    }

    /// The tip radius is hinted, so the teeth at 12, 3, 6, and 9 o'clock end on a solid row.
    fn cog(&mut self, c: (f32, f32), body: f32, tip: f32, half: f32, teeth: u32) {
        let s = self.s;
        let sh = Shape::Cog { c: self.at(c), body: body * s, tip: self.len(tip), half: half * s, teeth };
        self.ink(sh);
    }

    fn star(&mut self, c: (f32, f32), fill: bool) {
        let s = self.s;
        let sh = Shape::Star { c: self.at(c), outer: 5.95 * s, inner: 2.85 * s, round: 1.0 * s, fill };
        self.ink(sh);
    }

    /// A polyline through hinted points. Each corner is an arc of radius `r` tangent to both
    /// sides, shortened when a side is too short for it.
    fn path(&mut self, pts: &[(f32, f32)], r: f32, closed: bool) {
        let v: Vec<Pt> = pts.iter().map(|&q| self.at(q)).collect();
        let m = v.len();
        if m < 2 {
            return;
        }
        let r = r * self.s;
        // Where the stroke leaves and reaches each point once its corner is rounded.
        let mut enter = v.clone();
        let mut leave = v.clone();
        let corners: Vec<usize> = if closed { (0..m).collect() } else { (1..m - 1).collect() };
        for &i in &corners {
            let (a, b, c) = (v[(i + m - 1) % m], v[i], v[(i + 1) % m]);
            if let Some((t1, t2, arc)) = fillet(a, b, c, r) {
                enter[i] = t1;
                leave[i] = t2;
                if let Some(arc) = arc {
                    self.ink(arc);
                }
            }
        }
        let last = if closed { m } else { m - 1 };
        for i in 0..last {
            let j = (i + 1) % m;
            self.ink(Shape::Seg(leave[i], enter[j]));
        }
    }

    fn raster(&self) -> Vec<u8> {
        let n = self.n;
        let k = AA as f32;
        let mut out = vec![0u8; (n * n) as usize];
        for y in 0..n {
            for x in 0..n {
                let mut acc = 0.0f32;
                for j in 0..AA {
                    for i in 0..AA {
                        let q = Pt(x as f32 + (i as f32 + 0.5) / k, y as f32 + (j as f32 + 0.5) / k);
                        acc += (0.5 - self.dist(q) * k).clamp(0.0, 1.0);
                    }
                }
                out[(y * n + x) as usize] = (acc / (k * k) * 255.0).round() as u8;
            }
        }
        out
    }

    /// Signed distance from `q` to the ink, device pixels. Negative inside.
    fn dist(&self, q: Pt) -> f32 {
        let mut d = f32::INFINITY;
        for op in &self.ops {
            match op {
                Op::Ink(sh) => d = d.min(sh.dist(q, self.hw)),
                Op::Cut(sh, gap) => d = d.max(gap - sh.dist(q, self.hw)),
            }
        }
        d
    }
}

/// Tangent points of a corner at `b` and the arc between them. None for a straight run.
fn fillet(a: Pt, b: Pt, c: Pt, r: f32) -> Option<(Pt, Pt, Option<Shape>)> {
    let (u, v) = (a.sub(b), c.sub(b));
    let (lu, lv) = (u.len(), v.len());
    if lu <= 0.0 || lv <= 0.0 || r <= 0.0 {
        return None;
    }
    let (u, v) = (Pt(u.0 / lu, u.1 / lu), Pt(v.0 / lv, v.1 / lv));
    let cos = (u.0 * v.0 + u.1 * v.1).clamp(-1.0, 1.0);
    let theta = cos.acos();
    if theta >= PI - 1e-3 {
        return None;
    }
    let half = theta / 2.0;
    let t = (r / half.tan()).min(lu / 2.0).min(lv / 2.0);
    let r = t * half.tan();
    let (t1, t2) = (Pt(b.0 + u.0 * t, b.1 + u.1 * t), Pt(b.0 + v.0 * t, b.1 + v.1 * t));
    let bis = Pt(u.0 + v.0, u.1 + v.1);
    let bl = bis.len();
    if bl <= 0.0 {
        return None;
    }
    let off = r / half.sin();
    let o = Pt(b.0 + bis.0 / bl * off, b.1 + bis.1 / bl * off);
    let a1 = (t1.1 - o.1).atan2(t1.0 - o.0);
    let a2 = (t2.1 - o.1).atan2(t2.0 - o.0);
    let mut sweep = a2 - a1;
    if sweep > PI {
        sweep -= TAU;
    } else if sweep < -PI {
        sweep += TAU;
    }
    Some((t1, t2, Some(Shape::Arc { c: o, r, start: a1, sweep })))
}

impl Shape {
    /// Signed distance from `q` to this shape's ink. A stroke is `hw` either side of its line.
    fn dist(&self, q: Pt, hw: f32) -> f32 {
        match *self {
            Shape::Seg(a, b) => seg_dist(q, a, b) - hw,
            Shape::Arc { c, r, start, sweep } => {
                let d = q.sub(c);
                let ang = d.1.atan2(d.0);
                let t = if sweep >= 0.0 { (ang - start).rem_euclid(TAU) } else { (start - ang).rem_euclid(TAU) };
                if t <= sweep.abs() {
                    (d.len() - r).abs() - hw
                } else {
                    let end = start + sweep;
                    let e0 = Pt(c.0 + r * start.cos(), c.1 + r * start.sin());
                    let e1 = Pt(c.0 + r * end.cos(), c.1 + r * end.sin());
                    q.sub(e0).len().min(q.sub(e1).len()) - hw
                }
            }
            Shape::Frame(a, b, r) => round_box(q, a, b, r).abs() - hw,
            Shape::Slab(a, b, r) => round_box(q, a, b, r),
            Shape::Disc(c, r) => q.sub(c).len() - r,
            Shape::Lens { c, hw: w, hh: h } => {
                // Each lid is an arc of the circle through both corners and the lid's middle.
                let big = (w * w + h * h) / (2.0 * h);
                let k = big - h;
                let up = Pt(q.0 - c.0, q.1 - c.1 - k).len() - big;
                let lo = Pt(q.0 - c.0, q.1 - c.1 + k).len() - big;
                up.max(lo).abs() - hw
            }
            Shape::Cog { c, body, tip, half, teeth } => cog(q.sub(c), body, tip, half, teeth).abs() - hw,
            Shape::Star { c, outer, inner, round, fill } => {
                let d = star(q.sub(c), outer, inner) - round;
                if fill { d - hw } else { d.abs() - hw }
            }
        }
    }
}

fn seg_dist(q: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((q.0 - a.0) * dx + (q.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (q.0 - a.0 - t * dx).hypot(q.1 - a.1 - t * dy)
}

/// Signed distance to a box with corners `a`, `b` and corner radius `r`.
fn round_box(q: Pt, a: Pt, b: Pt, r: f32) -> f32 {
    let (cx, cy) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
    let (hx, hy) = ((b.0 - a.0).abs() / 2.0, (b.1 - a.1).abs() / 2.0);
    let r = r.min(hx).min(hy).max(0.0);
    let qx = (q.0 - cx).abs() - hx + r;
    let qy = (q.1 - cy).abs() - hy + r;
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// Polynomial smooth minimum: a fillet `k` wide where the two shapes meet.
fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k / 4.0
}

fn cog(q: Pt, body: f32, tip: f32, half: f32, teeth: u32) -> f32 {
    let r = q.len();
    let sector = TAU / teeth.max(1) as f32;
    let ang = q.1.atan2(q.0) + PI / 2.0;
    let a = (ang + sector / 2.0).rem_euclid(sector) - sector / 2.0;
    let local = Pt(r * a.cos(), r * a.sin());
    let root = body - half;
    let round = half * 0.7;
    let tooth = round_box(local, Pt(root, -half), Pt(tip, half), round);
    smin(r - body, tooth, half * 0.9)
}

/// Signed distance to a five-point star, first tip straight up.
fn star(q: Pt, outer: f32, inner: f32) -> f32 {
    let pts: [Pt; 10] = std::array::from_fn(|k| {
        let r = if k % 2 == 0 { outer } else { inner };
        let a = (-90.0 + 36.0 * k as f32).to_radians();
        Pt(r * a.cos(), r * a.sin())
    });
    let mut d = f32::INFINITY;
    let mut inside = false;
    for i in 0..10 {
        let (a, b) = (pts[i], pts[(i + 1) % 10]);
        d = d.min(seg_dist(q, a, b));
        if (a.1 > q.1) != (b.1 > q.1) && q.0 < a.0 + (q.1 - a.1) * (b.0 - a.0) / (b.1 - a.1) {
            inside = !inside;
        }
    }
    if inside { -d } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [LineIcon; 19] = [
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
        LineIcon::Save,
        LineIcon::Load,
        LineIcon::Basket,
        LineIcon::Watch,
        LineIcon::Star,
        LineIcon::StarFilled,
        LineIcon::StarAdd,
    ];
    const DPIS: [i32; 6] = [96, 120, 144, 168, 192, 288];

    fn at(c: &[u8], n: i32, x: i32, y: i32) -> u8 {
        c[(y * n + x) as usize]
    }

    fn cov(dpi: i32, kind: LineIcon) -> (i32, Vec<u8>) {
        let n = line_icon_px(dpi);
        (n, line_icon_coverage(n, dpi, kind))
    }

    #[test]
    fn box_and_stroke_track_the_dpi_steps() {
        assert_eq!(line_icon_px(96), 17);
        assert_eq!(line_icon_px(120), 21);
        assert_eq!(line_icon_px(144), 26);
        assert_eq!(line_icon_px(192), 34);
        assert_eq!(line_icon_px(1), 1);
        assert!((line_icon_stroke(96) - 1.35).abs() < 1e-6);
        for dpi in DPIS {
            let s = dpi as f32 / 96.0;
            let w = line_icon_stroke(dpi);
            let fringe = (w - stroke_core(s) as f32) / 2.0;
            assert!((0.125 - 1e-6..=0.3 + 1e-6).contains(&fringe), "{dpi}: soft but light fringe {fringe}");
            assert!((w - STROKE * s).abs() <= 0.3, "{dpi}: weight tracks the scale");
            assert_eq!((line_icon_px(dpi) - stroke_core(s)) % 2, 0, "{dpi}: parity");
            assert!(line_icon_px(dpi) as f32 >= BOX * s - 1.0, "{dpi}");
        }
    }

    #[test]
    fn every_icon_has_ink_inside_a_clear_border_and_is_distinct() {
        for dpi in DPIS {
            let covs: Vec<(i32, Vec<u8>)> = ALL.iter().map(|&k| cov(dpi, k)).collect();
            for (i, (n, c)) in covs.iter().enumerate() {
                assert!(c.iter().any(|&a| a == 255), "{dpi} {:?}: a solid core", ALL[i]);
                for k in 0..*n {
                    for (x, y) in [(k, 0), (0, k), (k, n - 1), (n - 1, k)] {
                        assert!(at(c, *n, x, y) < 96, "{dpi} {:?} edge {x},{y}: only a light fringe", ALL[i]);
                    }
                }
            }
            for i in 0..covs.len() {
                for j in (i + 1)..covs.len() {
                    assert_ne!(covs[i].1, covs[j].1, "{dpi} {:?} vs {:?}", ALL[i], ALL[j]);
                }
            }
        }
    }

    #[test]
    fn edges_are_soft_but_straight_strokes_keep_a_solid_core() {
        for kind in ALL {
            let (_, c) = cov(96, kind);
            let partial = c.iter().filter(|&&a| a > 0 && a < 255).count();
            assert!(partial > 0, "{kind:?}: anti-aliased, not a hard master");
        }
        // The list bars sit on pixel rows: full ink on the row, a light fringe either side.
        let (n, list) = cov(96, LineIcon::List);
        for y in [4, 8, 12] {
            assert_eq!(at(&list, n, 10, y), 255, "row {y}");
            let fringe = at(&list, n, 10, y - 1);
            assert!(fringe > 0 && fringe < 80, "row {y}: {fringe}");
            assert_eq!(at(&list, n, 10, y - 1), at(&list, n, 10, y + 1));
        }
        // 200%: a two-pixel core.
        let (n, add) = cov(192, LineIcon::Add);
        let mid = n / 2;
        assert_eq!((at(&add, n, mid - 1, 6), at(&add, n, mid, 6)), (255, 255));
        assert!(at(&add, n, mid - 2, 6) < 128 && at(&add, n, mid + 1, 6) < 128);
    }

    fn near(a: u8, b: u8) -> bool {
        a.abs_diff(b) <= 1
    }

    #[test]
    fn arrows_mirror_and_symmetric_icons_stay_centred() {
        for dpi in DPIS {
            let (n, up) = cov(dpi, LineIcon::Up);
            let (_, down) = cov(dpi, LineIcon::Down);
            let (_, back) = cov(dpi, LineIcon::Back);
            let (_, fwd) = cov(dpi, LineIcon::Fwd);
            for y in 0..n {
                for x in 0..n {
                    assert!(near(at(&up, n, x, y), at(&down, n, x, n - 1 - y)), "{dpi} ud {x},{y}");
                    assert!(near(at(&back, n, x, y), at(&fwd, n, n - 1 - x, y)), "{dpi} bf {x},{y}");
                    assert!(near(at(&up, n, x, y), at(&back, n, y, x)), "{dpi} up/back {x},{y}");
                }
            }
            for kind in [LineIcon::Add, LineIcon::Layout, LineIcon::Gear, LineIcon::Watch, LineIcon::Star, LineIcon::StarAdd, LineIcon::Del, LineIcon::Basket] {
                let (n, c) = cov(dpi, kind);
                for y in 0..n {
                    for x in 0..n {
                        assert!(near(at(&c, n, x, y), at(&c, n, n - 1 - x, y)), "{dpi} {kind:?} {x},{y}");
                    }
                }
            }
        }
    }

    #[test]
    fn plus_crosses_on_the_centre_pixel() {
        let (n, add) = cov(96, LineIcon::Add);
        assert_eq!(n, 17);
        assert!((2..15).all(|i| at(&add, n, 8, i) == 255 && at(&add, n, i, 8) == 255));
        assert!(at(&add, n, 8, 1) < 64 && at(&add, n, 8, 1) == at(&add, n, 8, 15), "round caps, same both ends");
        assert_eq!(at(&add, n, 8, 0), 0);
        assert!(at(&add, n, 9, 6) < 60, "one pixel thick with a light fringe");
    }

    #[test]
    fn holes_stay_open() {
        let (n, gear) = cov(96, LineIcon::Gear);
        assert_eq!(at(&gear, n, 8, 8), 0, "gear hole");
        assert_eq!(at(&gear, n, 8, 6), 255, "hole ring");
        assert_eq!((at(&gear, n, 8, 1), at(&gear, n, 15, 8)), (255, 255), "top and right tooth tips");
        assert!(at(&gear, n, 8, 0) < 64 && at(&gear, n, 8, 2) < 128, "one solid row per tip, the tooth open below it");
        let (n, eye) = cov(96, LineIcon::Watch);
        assert_eq!(at(&eye, n, 8, 8), 255, "pupil");
        assert!(at(&eye, n, 8, 11) < 64 && at(&eye, n, 8, 5) < 64, "an open ring around the pupil");
        assert_eq!(at(&eye, n, 8, 4), 255, "upper lid");
        let (n, star) = cov(96, LineIcon::Star);
        assert_eq!(at(&star, n, 8, 9), 0, "hollow star");
        let (_, full) = cov(96, LineIcon::StarFilled);
        assert_eq!(at(&full, n, 8, 9), 255, "filled star");
        let (_, add) = cov(96, LineIcon::StarAdd);
        assert_eq!(at(&add, n, 8, 9), 255, "plus in the star");
        assert!(at(&add, n, 8, 9 - 3) < 255 && at(&add, n, 8 - 3, 9) < 255, "the plus floats clear");
    }

    #[test]
    fn save_and_load_differ_only_in_the_badge() {
        for dpi in DPIS {
            let (n, save) = cov(dpi, LineIcon::Save);
            let (_, load) = cov(dpi, LineIcon::Load);
            let corner = (n as f32 * 0.55) as i32;
            for y in 0..n {
                for x in 0..n {
                    if x < corner || y < corner {
                        assert_eq!(at(&save, n, x, y), at(&load, n, x, y), "{dpi} {x},{y}");
                    }
                }
            }
            assert_ne!(save, load, "{dpi}");
        }
    }

    #[test]
    fn bgra_is_premultiplied_from_coverage() {
        let (n, c) = cov(96, LineIcon::Save);
        let px = line_icon_bgra(n, 96, LineIcon::Save, 0x112233);
        assert!(c.iter().any(|&a| a == 255) && c.iter().any(|&a| a == 0));
        for (i, &a) in c.iter().enumerate() {
            let p = &px[i * 4..i * 4 + 4];
            match a {
                0 => assert_eq!(p, &[0, 0, 0, 0]),
                255 => assert_eq!(p, &[0x33, 0x22, 0x11, 255]),
                _ => assert!(p[3] == a && p[0] <= 0x33 && p[2] <= 0x11),
            }
        }
    }
}
