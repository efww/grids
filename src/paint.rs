use crate::*;
use std::rc::Rc;

// ---------- painting ----------

// Ground, the sidebar plate and the status line. Pane plates are child windows (paint_pane).
pub(crate) unsafe fn paint_main(h: HWND) {
    let (g, status, font, dpi, hot, accent) = app(|a| {
        let g = geo(a);
        let hot = a.gap_hot.filter(|_| a.drag.is_none() && a.nudge.is_none()).and_then(|d| gap_line(&g, d, 2 * a.dpi / 96));
        // with its own control (draw_status) the line is not painted here
        let status = a.status_hwnd.0.is_null().then(|| a.status.clone());
        (g, status, a.font, a.dpi, hot.map(|r| (r, a.s.accent_now())), a.s.accent_now())
    });
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().bg);
        if g.tree {
            let side = side_rect(&g);
            fill(dc, side, pal().card);
            frame(dc, side, 1, pal().line);
        }
        if let Some((r, c)) = hot {
            fill(dc, r, c); // where to grab; dragging swaps it for the guide bar in the same colour
        }
        if let Some(st) = &status {
            paint_status(dc, &g, status_rect(&g), st, font, dpi, accent);
        }
    });
}

// WM_DRAWITEM for the status line control, which sits on status_rect: the same zones, from its own top.
pub(crate) unsafe fn draw_status(d: &DRAWITEMSTRUCT) {
    let (g, status, font, dpi, accent) = app(|a| (geo(a), a.status.clone(), a.font, a.dpi, a.s.accent_now()));
    let rc = d.rcItem;
    let mut mem = HDC::default();
    let pb = BeginBufferedPaint(d.hDC, &rc, BPBF_TOPDOWNDIB, None, &mut mem);
    let dc = if pb == 0 { d.hDC } else { mem };
    fill(dc, rc, pal().bg);
    paint_status(dc, &g, RECT { left: 0, right: g.w, ..rc }, &status, font, dpi, accent);
    if pb != 0 {
        _ = EndBufferedPaint(pb, true);
    }
}

// The status line in zones (status_fit): the note in an accent-tinted chip, items and selection, a thin line,
// filter / copying / maximized, and the drive with its usage bar at the right end.
unsafe fn paint_status(dc: HDC, g: &Geo, line: RECT, st: &StatusLine, font: HFONT, dpi: i32, accent: u32) {
    let s = |v: i32| v * dpi / 96;
    let p = pal();
    let (left, right) = (g.pad + s(4), g.w - g.pad - s(4));
    let (gap, chip_pad, bar_w, bar_gap) = (s(12), s(8), s(64), s(6));
    let w = |t: &str| text_w(dc, t, font);
    let span = |(l, r): (i32, i32)| RECT { left: l, right: r, ..line };
    let mid = st.mid.join(ZONE_DOT);
    // the drive's forms, widest first: label + bar + "free x / y", label + "free x / y", label + "free x"
    let drive = st.drive.as_ref().map(|(label, free, total)| {
        let (full, short) = (format!("{} {} / {}", tr("free", "여유"), human(*free), human(*total)), format!("{} {}", tr("free", "여유"), human(*free)));
        let usage = drive_use(*free, *total);
        let (lw, fw, sw) = (w(label) + bar_gap, w(&full), w(&short));
        let mut forms = Vec::new();
        if usage.is_some() {
            forms.push((true, full.clone(), lw + bar_w + bar_gap + fw));
        }
        forms.push((false, full, lw + fw));
        forms.push((false, short, lw + sw));
        (label, lw, usage, forms)
    });
    let widths: Vec<i32> = drive.as_ref().map(|d| d.3.iter().map(|f| f.2).collect()).unwrap_or_default();
    let fit = status_fit(
        left,
        right,
        gap,
        s(60),
        st.note.as_ref().map(|n| w(n) + 2 * chip_pad),
        w(&st.head),
        (!mid.is_empty()).then(|| w(&mid)),
        &widths,
    );
    let rule = |x: i32| fill(dc, RECT { left: x, right: x + 1, top: line.top + s(6), bottom: line.bottom - s(6) }, p.line);
    if let (Some(n), Some(r)) = (&st.note, fit.note) {
        let chip = RECT { top: line.top + s(3), bottom: line.bottom - s(3), ..span(r) };
        round_fill(dc, chip, s(3), blend(p.bg, accent, 0.18), p.bg);
        text(dc, RECT { left: chip.left + chip_pad, right: chip.right - chip_pad, ..chip }, n, font, p.text, DT_LEFT);
    }
    text(dc, span(fit.head), &st.head, font, p.sub, DT_LEFT);
    if let Some(r) = fit.mid {
        rule(r.0 - gap / 2);
        text(dc, span(r), &mid, font, p.sub, DT_LEFT);
    }
    if let (Some((label, lw, usage, forms)), Some((l, r, k))) = (drive, fit.drive) {
        rule(l - gap / 2);
        let (bar, words, _) = &forms[k];
        text(dc, RECT { left: l, right: (l + lw).min(r), ..line }, label, font, p.sub, DT_LEFT);
        let mut x = l + lw;
        if let (true, Some((used, warn))) = (*bar, usage) {
            let top = line.top + (line.bottom - line.top - s(6)) / 2;
            let track = RECT { left: x, top, right: x + bar_w, bottom: top + s(6) };
            fill(dc, track, p.line);
            let done = ((bar_w as f64 * used).round() as i32).clamp(if used > 0.0 { 1 } else { 0 }, bar_w);
            fill(dc, RECT { right: x + done, ..track }, bar_fill(&p, accent, warn));
            x += bar_w + bar_gap;
        }
        if x < r {
            text(dc, RECT { left: x, right: r, ..line }, words, font, p.sub, DT_LEFT);
        }
    }
}

#[derive(Clone, PartialEq)]
pub(crate) enum Crumb {
    Kids(String, Option<String>), // '›': the folder on its left, and the shown crumb after it (bold in the menu)
    Here,
    Go(String),
    More(Vec<(String, String)>), // folded parents, nearest first
}

// Whose shell menu a right-click on the address pill gives, from crumb_at's answer (`hit`) and the pane's folder.
#[derive(Debug, PartialEq)]
pub(crate) enum AddrMenu {
    Here,        // the folder the pane shows: the icon, its own crumb, '…' or the pill's empty space
    Dir(String), // a parent crumb, or the folder left of a '›'
}

// None: edit showing or outside the pill (crumb_at's None), the header's colour menu as before.
pub(crate) fn addr_menu_target(hit: Option<Option<&Crumb>>, here: &str) -> Option<AddrMenu> {
    match hit? {
        Some(Crumb::Go(p) | Crumb::Kids(p, _)) if !same_dir(p, here) => Some(AddrMenu::Dir(p.clone())),
        _ => Some(AddrMenu::Here),
    }
}

// The system small icon list, loaded on first use and kept: address bar, Everything results and drive buttons.
// Called outside app(..), since the shell may send messages back.
pub(crate) unsafe fn small_ilist() -> Option<IImageList> {
    app(|a| a.ilist.clone()).or_else(|| {
        let l = SHGetImageList::<IImageList>(SHIL_SMALL as i32).ok();
        app(|a| a.ilist = l.clone());
        l
    })
}

// ---------- icons at the monitor's own size ----------
// The small list's size is fixed at sign-in, by the main monitor's scale. On another monitor (or after a scale
// change) its icons would be stretched blurry or shrunk ragged: take the list at least as large and box-average
// it down, like Explorer, VS Code and Chrome pick per monitor. Jumbo is left out: a 16 dip icon is 48 px even at
// 300 %, and a type without a 256 px picture sits in its top-left corner (preview.rs type_icon).
const ICON_LISTS: [u32; 3] = [SHIL_SMALL, SHIL_LARGE, SHIL_EXTRALARGE];
const SHRUNK_MAX: usize = 256;

thread_local! {
    // (SHIL_*, its list, its icons' side in px), looked up once; a list that fails is left out
    static ICON_SIZES: RefCell<Option<Vec<(u32, IImageList, i32)>>> = const { RefCell::new(None) };
    // (icon index, px) -> the larger list's icon averaged down to px; None: draw it the old way
    static SHRUNK: RefCell<std::collections::HashMap<(i32, i32), Option<Rc<Pix>>>> = RefCell::new(Default::default());
}

/// Which list (their icon sides) a px icon comes from: the smallest at least px, else the largest.
pub(crate) fn pick_list(sides: &[i32], px: i32) -> Option<usize> {
    let k = sides.iter().enumerate();
    k.clone().filter(|(_, &s)| s >= px).min_by_key(|(_, &s)| s).or_else(|| k.max_by_key(|(_, &s)| s)).map(|(i, _)| i)
}

unsafe fn icon_lists() -> Vec<(u32, IImageList, i32)> {
    if let Some(v) = ICON_SIZES.with(|c| c.borrow().clone()) {
        return v;
    }
    let mut v = Vec::new();
    for shil in ICON_LISTS {
        if let Ok(l) = SHGetImageList::<IImageList>(shil as i32) {
            let (mut cx, mut cy) = (0, 0);
            if l.GetIconSize(&mut cx, &mut cy).is_ok() && cx > 0 {
                v.push((shil, l, cx));
            }
        }
    }
    ICON_SIZES.with(|c| *c.borrow_mut() = Some(v.clone()));
    v
}

unsafe fn draw_list_icon(dc: HDC, list: &IImageList, index: i32, r: RECT) -> bool {
    let Ok(icon) = list.GetIcon(index, ILD_NORMAL.0) else { return false };
    _ = DrawIconEx(dc, r.left, r.top, icon, r.right - r.left, r.bottom - r.top, 0, None, DI_NORMAL);
    _ = DestroyIcon(icon);
    true
}

// A larger list's icon averaged down to px, kept: premultiplied by fix_alpha (only when it comes straight),
// never twice. A mask-only icon (alpha all 0 as stored) gives None, so it is drawn the old way, not as a black square.
// A failed read is not kept: it may be passing, and keeping it would leave the small list stretched until the cache fills.
unsafe fn shrunk(list: &IImageList, index: i32, px: i32) -> Option<Rc<Pix>> {
    if let Some(p) = SHRUNK.with(|c| c.borrow().get(&(index, px)).cloned()) {
        return p;
    }
    let mut src = icon_pix(list, index)?;
    let p = src.bgra.chunks_exact(4).any(|q| q[3] != 0).then(|| {
        fix_alpha(&mut src.bgra);
        Rc::new(shrink(&src, px))
    });
    SHRUNK.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() >= SHRUNK_MAX {
            c.clear();
        }
        c.insert((index, px), p.clone());
    });
    p
}

/// System icon `index` in the square r at r's width in px, from the list that suits it. A list exactly that size
/// is drawn as before (DrawIconEx), so at the sign-in scale nothing changes; any failure falls back to the small
/// list stretched over r. False: no icon could be had, nothing drawn. Outside app(..): the shell may send messages back.
pub(crate) unsafe fn shell_icon(dc: HDC, small: &IImageList, index: i32, r: RECT) -> bool {
    let px = r.right - r.left;
    let lists = icon_lists();
    let sides: Vec<i32> = lists.iter().map(|l| l.2).collect();
    if let Some((shil, list, side)) = pick_list(&sides, px).map(|k| &lists[k]) {
        if *side > px {
            if let Some(p) = shrunk(list, index, px) {
                blit_bgra(dc, RECT { left: r.left, top: r.top, right: r.left + p.w, bottom: r.top + p.h }, p.w, p.h, &p.bgra);
                return true;
            }
        } else if draw_list_icon(dc, if *shil == SHIL_SMALL { small } else { list }, index, r) {
            return true; // exactly px, or the largest list stretched past it
        }
    }
    draw_list_icon(dc, small, index, r)
}

// ---------- folder icons in the folder menus (Ctrl+G, recent folders, '›', '…') ----------
// As Explorer's address bar drop-downs and Finder's Go menu: each place shows its icon, picked from what this run
// already knows (menu_icon_of), never by reading the folder.

/// A picture centred on a px × px canvas: every item of one menu takes the same square, so their text lines up.
/// None when it is larger than px (or empty).
pub(crate) fn menu_canvas(p: &Pix, px: i32) -> Option<Pix> {
    if px <= 0 || p.w <= 0 || p.h <= 0 || p.w > px || p.h > px || p.bgra.len() != (p.w * p.h * 4) as usize {
        return None;
    }
    let mut bgra = vec![0u8; (px * px * 4) as usize];
    let (x0, y0, row) = ((px - p.w) / 2, (px - p.h) / 2, (p.w * 4) as usize);
    for y in 0..p.h {
        let (s, d) = ((y * p.w * 4) as usize, (((y0 + y) * px + x0) * 4) as usize);
        bgra[d..d + row].copy_from_slice(&p.bgra[s..s + row]);
    }
    Some(Pix { w: px, h: px, bgra })
}

// System icon `index` at px × px, premultiplied, from the list shell_icon would take: a larger one averaged down,
// else the list's own (centred when even the largest is smaller). A mask-only icon gives None: it would be a black square.
unsafe fn menu_pix(index: i32, px: i32) -> Option<Pix> {
    let lists = icon_lists();
    let sides: Vec<i32> = lists.iter().map(|l| l.2).collect();
    let (_, list, side) = &lists[pick_list(&sides, px)?];
    if *side > px {
        return menu_canvas(shrunk(list, index, px)?.as_ref(), px);
    }
    let mut p = icon_pix(list, index)?;
    if p.bgra.chunks_exact(4).all(|q| q[3] == 0) {
        return None;
    }
    fix_alpha(&mut p.bgra);
    menu_canvas(&p, px)
}

/// Premultiplied top-down pixels as a 32-bit DIB, which a menu draws with its alpha. The caller deletes it.
pub(crate) unsafe fn pix_bitmap(p: &Pix) -> Option<HBITMAP> {
    if p.w <= 0 || p.h <= 0 || p.bgra.len() != (p.w * p.h * 4) as usize {
        return None;
    }
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: p.w,
            biHeight: -p.h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bmp = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    if bits.is_null() {
        _ = DeleteObject(bmp.into());
        return None;
    }
    std::slice::from_raw_parts_mut(bits as *mut u8, p.bgra.len()).copy_from_slice(&p.bgra);
    _ = GdiFlush();
    Some(bmp)
}

/// One menu's item icons, s(16) square: one bitmap per icon index, shared by all its items (a '›' list may hold
/// hundreds of plain folders). DestroyMenu leaves item bitmaps alone, so they go when this is dropped: keep it
/// alive until after DestroyMenu. Outside app(..): reading an icon calls the shell.
pub(crate) struct MenuIcons {
    pub(crate) px: i32,
    made: std::collections::HashMap<i32, Option<HBITMAP>>,
}

impl MenuIcons {
    pub(crate) fn new(dpi: i32) -> Self {
        Self { px: 16 * dpi / 96, made: Default::default() }
    }

    /// Icon `index` before the item of command id `id`; -1, or an icon that can't be had, leaves it text alone.
    pub(crate) unsafe fn put(&mut self, menu: HMENU, id: usize, index: i32) {
        if index < 0 {
            return;
        }
        let px = self.px;
        let bmp = *self.made.entry(index).or_insert_with(|| menu_pix(index, px).and_then(|p| pix_bitmap(&p)));
        if let Some(b) = bmp {
            set_item_bitmap(menu, id, b);
        }
    }
}

impl Drop for MenuIcons {
    fn drop(&mut self) {
        for b in self.made.values().flatten() {
            unsafe { _ = DeleteObject((*b).into()) };
        }
    }
}

/// The picture before the item of command id `id` (also found in submenus); a null bitmap takes it away.
pub(crate) unsafe fn set_item_bitmap(menu: HMENU, id: usize, b: HBITMAP) {
    let mi = MENUITEMINFOW { cbSize: size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_BITMAP, hbmpItem: b, ..Default::default() };
    _ = SetMenuItemInfoW(menu, id as u32, false, &mi);
}

// ---------- colour chips in the colour menus (pane colour, selection colour, ⇄ pane rows) ----------
// As Windows Terminal's tab colours, Chrome's tab groups and Office's font colour menu: the colour itself beside
// its name, so it is picked by eye, in the shade the current theme paints.

/// A chip of colour `fill` (0xRRGGBB) with a rim of `edge`, centred on a px × px canvas, premultiplied: s(12) of
/// s(16), corners rounded and smoothed. None: the rim alone (no colour).
pub(crate) fn swatch_pix(px: i32, fill: Option<u32>, edge: u32) -> Pix {
    let px = px.max(1);
    let mut side = (px * 3 / 4).max(1);
    if (px - side) % 2 == 1 {
        side += 1; // the same margin on both sides
    }
    let (o, e, r) = (((px - side) / 2) as f32, (px / 16).max(1) as f32, side as f32 / 4.0);
    let ch = |c: u32, sh: u32| (c >> sh & 0xFF) as f32;
    let mut bgra = vec![0u8; (px * px * 4) as usize];
    for y in 0..px {
        for x in 0..px {
            let outer = round_cover(x, y, (o, o, side as f32, side as f32), r);
            if outer == 0.0 {
                continue;
            }
            let inside = side as f32 - 2.0 * e;
            let inner = round_cover(x, y, (o + e, o + e, inside, inside), (r - e).max(0.0));
            let (rim, body) = (outer - inner, if fill.is_some() { inner } else { 0.0 });
            let i = ((y * px + x) * 4) as usize;
            for (k, sh) in [0, 8, 16].into_iter().enumerate() {
                let v = ch(edge, sh) * rim + fill.map_or(0.0, |f| ch(f, sh)) * body;
                bgra[i + k] = v.round().min(255.0) as u8;
            }
            bgra[i + 3] = ((rim + body) * 255.0).round().min(255.0) as u8;
        }
    }
    Pix { w: px, h: px, bgra }
}

// A chip's rim on the theme's menu: its colour a third of the way to the menu text, so a grey chip still stands
// off the menu; the empty chip in the theme's grey text.
pub(crate) fn chip_edge(fill: Option<u32>, p: &Palette) -> u32 {
    fill.map_or(p.sub, |c| blend(c, p.text, 0.35))
}

/// One menu's colour chips, one bitmap per colour. As MenuIcons: DestroyMenu leaves them, so keep this alive
/// until the menu is destroyed (or, for a kept menu, until its items hold other bitmaps).
pub(crate) struct MenuSwatches {
    px: i32,
    made: std::collections::HashMap<Option<u32>, Option<HBITMAP>>,
}

impl MenuSwatches {
    pub(crate) fn new(dpi: i32) -> Self {
        Self { px: 16 * dpi / 96, made: Default::default() }
    }

    /// A chip of `fill` (None: the empty one) before the item of command id `id`.
    pub(crate) unsafe fn put(&mut self, menu: HMENU, id: usize, fill: Option<u32>) {
        let px = self.px;
        let bmp = *self.made.entry(fill).or_insert_with(|| {
            // under high contrast menus take Windows' colours (app mode Default): the rim in its menu text
            let edge = if HC_ON.load(Ordering::Relaxed) { crate::draw::cref(GetSysColor(COLOR_MENUTEXT)).0 } else { chip_edge(fill, theme_pal(DARK_ON.load(Ordering::Relaxed))) };
            pix_bitmap(&swatch_pix(px, fill, edge))
        });
        if let Some(b) = bmp {
            set_item_bitmap(menu, id, b);
        }
    }
}

impl Drop for MenuSwatches {
    fn drop(&mut self) {
        for b in self.made.values().flatten() {
            unsafe { _ = DeleteObject((*b).into()) };
        }
    }
}

// The folder icon's square at the front of the address pill, like Explorer's address bar; none in a narrow pill.
// Fixed by the geometry alone, so crumbs and edit never shift when an icon arrives or goes.
pub(crate) fn addr_icon(addr: RECT, dpi: i32) -> Option<RECT> {
    let s = |v: i32| v * dpi / 96;
    if addr.right - addr.left < s(90) {
        return None;
    }
    let (left, top) = (addr.left + s(6), addr.top + (addr.bottom - addr.top - s(16)) / 2);
    Some(RECT { left, top, right: left + s(16), bottom: top + s(16) })
}

// A pane-client point on that icon: where a press starts dragging the pane's folder out. False in a narrow pill.
pub(crate) fn on_addr_icon(addr: RECT, dpi: i32, x: i32, y: i32) -> bool {
    addr_icon(addr, dpi).is_some_and(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

// Where crumbs go in the address pill: the edit's horizontal span, a little inside the pill vertically.
pub(crate) fn crumb_field(addr: RECT, dpi: i32) -> RECT {
    let s = |v: i32| v * dpi / 96;
    let left = addr.left + if addr_icon(addr, dpi).is_some() { s(26) } else { s(8) };
    RECT { left, top: addr.top + s(3), right: (addr.right - s(6)).max(left), bottom: addr.bottom - s(3) }
}

// Pane i's crumbs laid out in `field`: (rect, label, target). Painting and mouse hits both go through here.
pub(crate) unsafe fn crumb_layout(dc: HDC, font: HFONT, i: usize, field: RECT) -> Vec<(RECT, String, Crumb)> {
    let Some((parse, disp, dpi)) = app(|a| Some((a.s.paths[i].clone(), a.panes[i].as_ref()?.disp.clone(), a.dpi))) else {
        return Vec::new();
    };
    let s = |v: i32| v * dpi / 96;
    let mut items = crumbs(&parse);
    let fs = !items.is_empty();
    if items.is_empty() {
        items.push((disp, String::new())); // a shell place: its name, to click into the edit
    }
    let old = SelectObject(dc, font.into());
    let width = |t: &str| {
        let w: Vec<u16> = t.encode_utf16().collect();
        let mut sz = SIZE::default();
        _ = GetTextExtentPoint32W(dc, &w, &mut sz);
        sz.cx
    };
    let w: Vec<i32> = items.iter().map(|(n, _)| width(n) + 2 * s(4)).collect();
    let (sep, ell) = (width("›") + 2 * s(2), width("…") + 2 * s(4));
    SelectObject(dc, old);
    let (first, more, last, tail) = crumb_fit_tail(&w, field.right - field.left, sep, ell, s(16), fs);
    let mut out = Vec::new();
    let mut x = field.left;
    let mut put = |wd: i32, label: &str, c: Crumb| {
        out.push((RECT { left: x, right: x + wd, ..field }, label.to_string(), c));
        x += wd;
    };
    let kids = |k: usize| Crumb::Kids(items[k].1.clone(), items.get(k + 1).map(|n| n.0.clone()));
    if more {
        put(ell, "…", Crumb::More(items[..first].iter().rev().cloned().collect()));
        put(sep, "›", kids(first - 1));
    }
    for k in first..items.len() {
        if k + 1 == items.len() {
            put(last, &items[k].0, Crumb::Here);
            if tail {
                put(sep, "›", kids(k));
            }
        } else {
            put(w[k], &items[k].0, Crumb::Go(items[k].1.clone()));
            put(sep, "›", kids(k));
        }
    }
    out.retain_mut(|(r, ..)| {
        (r.left, r.right) = (r.left.max(field.left), r.right.min(field.right));
        r.right > r.left
    });
    out
}

pub(crate) fn addr_rect(h: HWND, dpi: i32) -> RECT {
    unsafe { pane_geo_of(h, dpi) }.addr
}

// The crumb under a pane-client point: None off the crumbs (edit showing, or outside the address pill),
// Some(None) on the pill's empty space.
pub(crate) unsafe fn crumb_at(h: HWND, i: usize, x: i32, y: i32) -> Option<Option<(usize, RECT, Crumb)>> {
    if GetDlgItem(Some(h), ID_EDIT).is_ok_and(|e| IsWindowVisible(e).as_bool()) {
        return None;
    }
    let (dpi, font) = app(|a| (a.dpi, a.font));
    let inr = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    let addr = addr_rect(h, dpi);
    if !inr(&addr) {
        return None;
    }
    let dc = GetDC(Some(h));
    let items = crumb_layout(dc, font, i, crumb_field(addr, dpi));
    ReleaseDC(Some(h), dc);
    Some(items.into_iter().enumerate().find(|(_, (r, ..))| inr(r)).map(|(k, (r, _, c))| (k, r, c)))
}

// The address edit shows only while in use (focused, or holding an edit); otherwise the pill draws crumbs.
pub(crate) unsafe fn sync_addr(h: HWND) {
    let Ok(e) = GetDlgItem(Some(h), ID_EDIT) else { return };
    let disp = app(|a| {
        if a.crumb_hot.is_some_and(|(w, _)| w == h) {
            a.crumb_hot = None; // the crumbs change under it
        }
        a.panes.iter().flatten().find(|p| p.hwnd == h).map(|p| p.disp.clone())
    });
    let show = GetFocus() == e || text_of(e) != disp.unwrap_or_default();
    _ = ShowWindow(e, if show { SW_SHOWNA } else { SW_HIDE });
    _ = InvalidateRect(Some(h), None, false);
}

// Click on the address pill: a parent crumb goes there, '…' lists the folded parents, the rest starts typing.
pub(crate) unsafe fn crumb_click(h: HWND, i: usize, x: i32, y: i32) {
    let target = match crumb_at(h, i, x, y) {
        None => return,
        Some(Some((_, _, Crumb::Go(p)))) => Some(p),
        Some(Some((_, r, Crumb::More(list)))) => {
            let (ix, dpi): (Vec<i32>, i32) = app(|a| (list.iter().map(|(_, p)| menu_icon_of(a, p, -1)).collect(), a.dpi));
            let mut icons = MenuIcons::new(dpi); // dropped after DestroyMenu
            let Ok(menu) = CreatePopupMenu() else { return };
            for (k, (n, _)) in list.iter().enumerate() {
                _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(n.replace('&', "&&")));
                icons.put(menu, k + 1, ix[k]);
            }
            let mut p = POINT { x: r.left, y: r.bottom };
            _ = ClientToScreen(h, &mut p);
            let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, p.x, p.y, None, h, None).0;
            _ = DestroyMenu(menu);
            (pick > 0).then(|| list[pick as usize - 1].1.clone())
        }
        // listed by the worker; the menu comes with WM_KIDS. Focus moves first, so keys pressed meanwhile
        // (which cancel it) go to this pane, and the request is made for the pane as it is after that.
        Some(Some((_, _, Crumb::Kids(dir, next)))) => {
            kids_cancel();
            focus_view(i);
            let Some((h, path, dpi)) = app(|a| Some((a.panes[i].as_ref()?.hwnd, a.s.paths[i].clone(), a.dpi))) else { return };
            let id = want_kids(h, &dir, hidden(), KIDS_MAX);
            app(|a| a.kids = Some(KidsAsk { id, hwnd: h, i, path, gen: gen(), dir, next, step: 0 }));
            _ = InvalidateRect(Some(h), Some(&addr_rect(h, dpi)), false);
            return;
        }
        Some(_) => {
            focus_ctl(i, ID_EDIT);
            return;
        }
    };
    if let Some(p) = target.filter(|p| !navigate(i, p)) {
        fail(Fail::FolderCantOpen, &p); // gone or unreachable
    }
    focus_view(i);
}

// A '›' click waiting for its list: what it was asked for, checked again before the menu shows.
pub(crate) struct KidsAsk {
    pub(crate) id: usize,
    pub(crate) hwnd: HWND,
    pub(crate) i: usize,
    pub(crate) path: String, // the pane's folder then
    pub(crate) gen: usize,
    pub(crate) dir: String, // folder listed
    pub(crate) next: Option<String>,
    pub(crate) step: i32, // 0: the '›' menu; +1 / -1: Alt+PgDn / Alt+PgUp, go to the next / previous folder of dir
}

impl KidsAsk {
    // A WM_KIDS of an older request (or another pane) must leave this one alone.
    pub(crate) fn owns(&self, id: usize, h: HWND) -> bool {
        self.id == id && self.hwnd == h
    }
}

pub(crate) type KidsList = std::result::Result<(Vec<String>, bool), ()>; // (names, cut short), Err = unreadable

// One worker, newest request only (as DIR_SIZE / PEEK): id moves on every ask or cancel, a result is kept
// only for the current id and handed out only to it.
// A worker stuck on a dead share's read_dir is left behind by KIDS_RELAY once another drive's '›' waits.
pub(crate) struct KidsJob {
    pub(crate) id: usize,
    pub(crate) hwnd: isize,
    pub(crate) dir: String,
    pub(crate) hidden: bool,
    pub(crate) cap: usize, // most names kept: KIDS_MAX for the '›' menu, all for Alt+PgDn / Alt+PgUp
    pub(crate) result: Option<(usize, KidsList)>,
}

impl KidsJob {
    pub(crate) fn ask(&mut self, hwnd: isize, dir: &str, hidden: bool, cap: usize) -> usize {
        (self.id, self.hwnd, self.dir, self.hidden, self.cap, self.result) = (self.id + 1, hwnd, dir.to_string(), hidden, cap, None);
        self.id
    }

    pub(crate) fn publish(&mut self, id: usize, r: KidsList) -> bool {
        if self.id != id {
            return false;
        }
        self.result = Some((id, r));
        true
    }

    pub(crate) fn take(&mut self, id: usize) -> Option<KidsList> {
        self.result.take_if(|(k, _)| *k == id).map(|(_, r)| r)
    }

    pub(crate) fn cancel(&mut self) {
        (self.id, self.result) = (self.id + 1, None);
        self.dir.clear();
    }
}

pub(crate) static KIDS: Mutex<KidsJob> = Mutex::new(KidsJob { id: 0, hwnd: 0, dir: String::new(), hidden: false, cap: KIDS_MAX, result: None });
pub(crate) static KIDS_WAKE: Condvar = Condvar::new();
pub(crate) const KIDS_MAX: usize = 500;

pub(crate) fn kids_job() -> std::sync::MutexGuard<'static, KidsJob> {
    KIDS.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn want_kids(h: HWND, dir: &str, hidden: bool, cap: usize) -> usize {
    let id = kids_job().ask(h.0 as isize, dir, hidden, cap);
    KIDS_RELAY.ensure(Some(vol_key(dir)), kids_worker);
    KIDS_WAKE.notify_one();
    id
}

// Lives until the process ends, or until KIDS_RELAY leaves it behind (then it posts nothing). The lock is
// never held across file system calls.
pub(crate) fn kids_worker(me: usize) {
    use std::os::windows::fs::MetadataExt;
    let mut seen = 0;
    loop {
        let (id, hwnd, dir, hidden, cap) = {
            let mut j = kids_job();
            while j.id == seen || j.dir.is_empty() || KIDS_RELAY.blocked(&vol_key(&j.dir)) {
                j = KIDS_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
            (j.id, j.hwnd, j.dir.clone(), j.hidden, j.cap)
        };
        seen = id;
        let vol = vol_key(&dir);
        KIDS_RELAY.busy(me, vol.clone());
        let all = std::fs::read_dir(&dir).map_err(|_| ());
        let r = all.map(|rd| {
            let named = rd.flatten().filter_map(|e| Some((e.file_name().to_string_lossy().into_owned(), e.metadata().ok()?.file_attributes())));
            kid_names(named, hidden, cap)
        });
        if !KIDS_RELAY.done(me, &vol) {
            wake_owner(&KIDS, &KIDS_WAKE);
            return;
        }
        if kids_job().publish(id, r) {
            unsafe { _ = PostMessageW(Some(HWND(hwnd as _)), WM_KIDS, WPARAM(id), LPARAM(0)) };
        }
    }
}

// Subfolders out of (name, attributes): hidden ones only when shown, hidden+system never (as Explorer),
// in Explorer's order ('2' before '10'), at most `cap`. (names, cut short)
pub(crate) fn kid_names(all: impl Iterator<Item = (String, u32)>, show_hidden: bool, cap: usize) -> (Vec<String>, bool) {
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
    let (dir, hid, hs) = (FILE_ATTRIBUTE_DIRECTORY.0, FILE_ATTRIBUTE_HIDDEN.0, FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0);
    let mut v: Vec<HSTRING> =
        all.filter(|&(_, a)| a & dir != 0 && a & hs != hs && (show_hidden || a & hid == 0)).map(|(n, _)| HSTRING::from(n)).collect();
    v.sort_by(|a, b| unsafe { StrCmpLogicalW(a, b) }.cmp(&0));
    let cut = v.len() > cap;
    (v.iter().take(cap).map(|n| n.to_string_lossy()).collect(), cut)
}

// Where the '›' of `dir` is in a fresh layout; None once it folded away.
pub(crate) fn kids_anchor(items: &[(RECT, String, Crumb)], dir: &str) -> Option<RECT> {
    items.iter().find(|(_, _, c)| matches!(c, Crumb::Kids(d, _) if d == dir)).map(|(r, ..)| *r)
}

// Any click, key, window resize or DPI change drops a waiting '›' list.
pub(crate) unsafe fn kids_cancel() {
    let Some((h, dpi)) = app(|a| a.kids.take().map(|k| (k.hwnd, a.dpi))) else { return };
    kids_job().cancel();
    _ = InvalidateRect(Some(h), Some(&addr_rect(h, dpi)), false);
}

// WM_KIDS: the list is in. Shown only if it is still this request's, the pane still shows the same folder,
// nothing is being resized, Grids is in front, and the '›' is still laid out (placed where it is now).
pub(crate) unsafe fn kids_show(h: HWND, id: usize) {
    if !app(|a| a.kids.as_ref().is_some_and(|k| k.owns(id, h))) {
        return;
    }
    let Some(list) = kids_job().take(id) else { return };
    let Some(k) = app(|a| a.kids.take()) else { return };
    let (dpi, font, main) = app(|a| (a.dpi, a.font, a.main));
    let addr = addr_rect(h, dpi);
    _ = InvalidateRect(Some(h), Some(&addr), false); // the waiting mark goes either way
    let same = app(|a| a.panes[k.i].as_ref().is_some_and(|p| p.hwnd == h) && a.s.paths[k.i] == k.path && !a.sizing);
    // Alt+PgDn / Alt+PgUp: no menu, so no '›' needed (it may have folded into '…'), nor the address bar shut
    if k.step != 0 {
        if same && gen() == k.gen && GetForegroundWindow() == main {
            sibling_go(&k, &list);
        }
        return;
    }
    if !same || gen() != k.gen || GetForegroundWindow() != main || GetDlgItem(Some(h), ID_EDIT).is_ok_and(|e| IsWindowVisible(e).as_bool()) {
        return;
    }
    let dc = GetDC(Some(h));
    let items = crumb_layout(dc, font, k.i, crumb_field(addr, dpi));
    ReleaseDC(Some(h), dc);
    let Some(r) = kids_anchor(&items, &k.dir) else { return };
    let kid = |name: &str| format!("{}\\{name}", k.dir.trim_end_matches('\\'));
    let ix: Vec<i32> = match &list {
        Ok((names, _)) => app(|a| names.iter().map(|n| menu_icon_of(a, &kid(n), -1)).collect()),
        Err(()) => Vec::new(),
    };
    let mut icons = MenuIcons::new(dpi); // dropped after DestroyMenu
    let Ok(menu) = CreatePopupMenu() else { return };
    let gray = |t: &str| _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(t));
    let names = match &list {
        Ok((names, cut)) => {
            for (n, name) in names.iter().enumerate() {
                _ = AppendMenuW(menu, MF_STRING, n + 1, &HSTRING::from(name.replace('&', "&&")));
                icons.put(menu, n + 1, ix[n]);
                if k.next.as_ref().is_some_and(|x| x.to_lowercase() == name.to_lowercase()) {
                    _ = SetMenuDefaultItem(menu, n as u32 + 1, 0); // where this pane is now: bold
                }
            }
            if *cut {
                gray("…");
            } else if names.is_empty() {
                gray(tr("No subfolders", "하위 폴더 없음"));
            }
            &names[..]
        }
        Err(()) => {
            gray(tr("Can't open this folder", "이 폴더를 열 수 없음"));
            &[]
        }
    };
    let mut p = POINT { x: r.left, y: r.bottom };
    _ = ClientToScreen(h, &mut p);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, p.x, p.y, None, h, None).0;
    _ = DestroyMenu(menu);
    if let Some(name) = names.get((pick as usize).wrapping_sub(1)) {
        let to = kid(name);
        if !navigate(k.i, &to) {
            fail(Fail::FolderCantOpen, &to); // gone or unreachable
        }
    }
    focus_view(k.i);
}

// The parent folder of a file system folder and its name there; None at a drive's or share's top and in shell places.
pub(crate) fn sibling_of(path: &str) -> Option<(String, String)> {
    if !is_fs(path) {
        return None;
    }
    let p = std::path::Path::new(path);
    Some((p.parent()?.to_str()?.to_string(), p.file_name()?.to_str()?.to_string()))
}

// Alt+PgDn / Alt+PgUp (as a photo viewer's next folder): the parent is listed by the '›' worker, the step
// is taken when its WM_KIDS comes in (kids_show), if the pane is still where it was. The whole list is
// asked for (not the menu's KIDS_MAX), so the step works from any folder of a parent with thousands.
pub(crate) unsafe fn sibling_step(i: usize, step: i32) {
    let Some((h, path)) = app(|a| Some((a.panes[i].as_ref()?.hwnd, a.s.paths[i].clone()))) else { return };
    let Some((dir, name)) = sibling_of(&path) else {
        warn(tr("This folder has no sibling folders", "이 폴더에는 같은 단계 폴더가 없음").into());
        return;
    };
    kids_cancel();
    let id = want_kids(h, &dir, hidden(), usize::MAX);
    app(|a| a.kids = Some(KidsAsk { id, hwnd: h, i, path, gen: gen(), dir, next: Some(name), step }));
}

#[derive(Debug, PartialEq)]
pub(crate) enum Sibling {
    Go(usize),
    First,
    Last,
    Unknown, // past a list cut short (the '›' menu's KIDS_MAX; Alt+PgDn / Alt+PgUp lists all)
}

// The folder `step` away from `cur` in a parent's sorted list. A hidden `cur` (not listed) is placed by
// Explorer's order; an exact (case-insensitive) match comes first, so names that compare equal aren't skipped.
pub(crate) fn pick_sibling(names: &[String], cur: &str, step: i32, cut: bool) -> Sibling {
    let low = cur.to_lowercase();
    let (before, after) = match names.iter().position(|n| n.to_lowercase() == low) {
        Some(p) => (p.checked_sub(1), p + 1),
        None => {
            let c = HSTRING::from(cur);
            let q = names.iter().take_while(|n| unsafe { StrCmpLogicalW(&HSTRING::from(n.as_str()), &c) } < 0).count();
            if cut && q == names.len() {
                return Sibling::Unknown; // cur sorts somewhere past the cut
            }
            (q.checked_sub(1), q)
        }
    };
    match (step > 0, before) {
        (true, _) if after < names.len() => Sibling::Go(after),
        (true, _) if cut => Sibling::Unknown,
        (true, _) => Sibling::Last,
        (false, Some(b)) => Sibling::Go(b),
        (false, None) => Sibling::First,
    }
}

unsafe fn sibling_go(k: &KidsAsk, list: &KidsList) {
    let Ok((names, cut)) = list else { return warn(tr("Can't read this folder list", "이 폴더 목록을 읽을 수 없음").into()) };
    match pick_sibling(names, k.next.as_deref().unwrap_or_default(), k.step, *cut) {
        Sibling::Go(n) => {
            if !navigate(k.i, &format!("{}\\{}", k.dir.trim_end_matches('\\'), names[n])) {
                return warn(tr("Can't open that folder", "그 폴더를 열 수 없음").into());
            }
            let total = if *cut { format!("{}+", names.len()) } else { names.len().to_string() };
            let dir = k.dir.trim_end_matches('\\');
            let parent = dir.rsplit('\\').next().filter(|s| !s.is_empty()).unwrap_or(dir);
            set_note(format!("{} {}/{total} · {parent}", tr("Folder", "폴더"), n + 1));
        }
        Sibling::First => warn(tr("First folder", "첫 폴더").into()),
        Sibling::Last => warn(tr("Last folder", "마지막 폴더").into()),
        Sibling::Unknown => warn(tr("Can't read this folder list", "이 폴더 목록을 읽을 수 없음").into()),
    }
}

pub(crate) unsafe fn crumb_hover(h: HWND, k: Option<usize>) {
    let hot = k.map(|k| (h, k));
    let (old, dpi) = app(|a| (std::mem::replace(&mut a.crumb_hot, hot), a.dpi));
    if old == hot {
        return;
    }
    for w in [old.map(|o| o.0), hot.map(|o| o.0)].into_iter().flatten() {
        _ = InvalidateRect(Some(w), Some(&addr_rect(w, dpi)), false);
    }
    if hot.is_some() && old.is_none_or(|(w, _)| w != h) {
        track_leave(h);
    }
}

// WM_MOUSELEAVE for whichever of the crumbs / the header targets is hot in h.
unsafe fn track_leave(h: HWND) {
    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
    _ = TrackMouseEvent(&mut t);
}

// What in pane window h's header is under a client point (tabs, ×, '+', filter icon).
pub(crate) unsafe fn head_hot_of(h: HWND, x: i32, y: i32) -> Option<HeadHot> {
    let i = app(|a| a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h)))?;
    let dpi = app(|a| a.dpi);
    let (wants, cur) = tab_wants_of(i);
    head_hot_at(&pane_geo_of(h, dpi), &wants, cur, x, y, dpi)
}

// Where a header target is painted in h: the whole tab strip (a tab lightening moves its neighbours' hairlines),
// or the filter icon's slot.
unsafe fn head_hot_rect(h: HWND, t: HeadHot, dpi: i32) -> RECT {
    let pg = pane_geo_of(h, dpi);
    match t {
        HeadHot::Kind => RECT { right: pg.filt.left + 24 * dpi / 96, ..pg.filt }, // the icon can be wider than a narrow pill
        _ => pg.tabs,
    }
}

pub(crate) unsafe fn head_hover(h: HWND, t: Option<HeadHot>) {
    let hot = t.map(|t| (h, t));
    let (old, dpi) = app(|a| (std::mem::replace(&mut a.head_hot, hot), a.dpi));
    if old == hot {
        return;
    }
    for (w, t) in [old, hot].into_iter().flatten() {
        let r = head_hot_rect(w, t, dpi);
        if r.bottom > r.top && r.right > r.left {
            _ = InvalidateRect(Some(w), Some(&r), false);
        }
    }
    if hot.is_some() && old.is_none_or(|(w, _)| w != h) {
        track_leave(h);
    }
}

// After the tabs or the pane's size changed under a still pointer: shade what is under it now, but only while
// the pointer is really on this pane window (not on another window, a child control or off it).
pub(crate) unsafe fn head_rehover(h: HWND) {
    let mut p = POINT::default();
    let over = GetCursorPos(&mut p).is_ok() && WindowFromPoint(p) == h && ScreenToClient(h, &mut p).as_bool();
    let t = if over { head_hot_of(h, p.x, p.y) } else { None };
    if t.is_some() || app(|a| a.head_hot.is_some_and(|(w, _)| w == h)) {
        head_hover(h, t);
    }
}

// One white plate per pane: a single accent border when active, flat fields in the top bar.
pub(crate) unsafe fn paint_pane(h: HWND) {
    let (active, i, dpi, icon_font, small_font, font, accent, focus_c, hot, hot_head, tint, folder, waiting, loading) = app(|a| {
        let i = a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h));
        // a drop target-to-be looks active too
        let active = a.panes[a.s.active].as_ref().is_some_and(|p| p.hwnd == h) || a.drop_hover == h;
        let hot = a.crumb_hot.filter(|(w, _)| *w == h).map(|(_, k)| k);
        let hot_head = a.head_hot.filter(|(w, _)| *w == h).map(|(_, t)| t);
        let tint = i.and_then(|i| tint_color(a.s.tints[i]));
        let folder = i.and_then(|i| a.panes[i].as_ref()).map_or(-1, |p| p.icon);
        let waiting = a.kids.as_ref().filter(|k| k.hwnd == h && k.step == 0).map(|k| k.dir.clone());
        let loading = i.and_then(|i| a.panes[i].as_ref()).and_then(|p| p.loading).map(|t| t.elapsed());
        // border and current tab edge follow the header; the filter badge, '…' wait shade and field ring keep the colour.
        // Only the active (or drop target) pane quiets: the others' tab edge keeps the colour, since app_away
        // changes repaint just the active pane
        let accent = a.s.accent_now();
        let focus_c = if active { focus_accent(a, h) } else { accent };
        (active, i, a.dpi, a.icon_font, a.small_font, a.font, accent, focus_c, hot, hot_head, tint, folder, waiting, loading)
    });
    let tabs = i.filter(|_| tabs_on()).map(|i| {
        app(|a| {
            let (list, cur) = (a.s.tab_list(i), a.s.tab[i]);
            let icons = tab_icons_of(a, i, &list, cur);
            (list, cur, icons)
        })
    });
    let ilist = if folder >= 0 || tabs.is_some() { small_ilist() } else { app(|a| a.ilist.clone()) };
    let crumbs_on = GetDlgItem(Some(h), ID_EDIT).is_ok_and(|e| !IsWindowVisible(e).as_bool());
    let filtered = i.is_some_and(|i| FILTERS.lock().is_ok_and(|f| !f[i].is_empty()));
    let linked = i.is_some_and(|i| app(|a| lock_partner(a, i).is_some())); // navigating together (Ctrl+Shift+Y)
    let (head, ink) = header_colors(h);
    let focus = GetFocus();
    let focused = |id: i32| GetDlgItem(Some(h), id).is_ok_and(|e| e == focus);
    let (addr_on, filt_on) = (focused(ID_EDIT), focused(ID_FILTER));
    let animate = loading.is_some_and(|e| e >= LOAD_SHOW) && load_animate();
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let pg = pane_geo_of(h, dpi);
        fill(dc, rc, pal().card);
        // active: header bar in the selection colour, same colour as the border
        fill(dc, RECT { bottom: pg.view.top, ..rc }, head);
        if active {
            frame(dc, rc, s(2), focus_c);
        } else {
            frame(dc, rc, 1, pal().line);
            fill(dc, RECT { left: pg.view.left, top: pg.view.top - 1, right: pg.view.right, bottom: pg.view.top }, pal().line);
        }
        // colour label: a strip above the buttons, and the number on a chip of that colour
        if let Some(t) = tint {
            fill(dc, tint_band(rc.right, dpi, active), t);
        }
        if let Some(i) = i.filter(|_| pg.num.right > pg.num.left) {
            let ink = match tint {
                Some(t) => {
                    round_fill(dc, pg.num, s(3), t, head);
                    on_color(t)
                }
                None if active => ink,
                None => pal().sub,
            };
            if linked {
                // navigating together: the number moves up and a small link mark sits under it, inside the same slot
                let split = pg.num.top + (pg.num.bottom - pg.num.top) * 3 / 5;
                text(dc, RECT { bottom: split + s(1), ..pg.num }, &(i + 1).to_string(), small_font, ink, DT_CENTER);
                let mut lf = LOGFONTW::default();
                if GetObjectW(icon_font.into(), size_of::<LOGFONTW>() as i32, Some(&mut lf as *mut _ as _)) != 0 {
                    lf.lfHeight = -s(9);
                    let f = CreateFontIndirectW(&lf);
                    text(dc, RECT { top: split - s(1), ..pg.num }, ICON_LINK, f, ink, DT_CENTER);
                    _ = DeleteObject(f.into());
                }
            } else {
                text(dc, pg.num, &(i + 1).to_string(), small_font, ink, DT_CENTER);
            }
        }
        // the icon opens the file-type menu: under the mouse its slot is shaded (inside the pill and its focus frame),
        // a ring round the filter badge, whose own colour stays.
        // A narrow pane's filter field can be under s(24) wide: the shade, icon and badge all stay inside the icon slot
        let kind_hot = hot_head == Some(HeadHot::Kind);
        let slot = filter_icon_rect(pg.filt, s(24));
        let shade = blend(pal().pill, pal().text, press_tint(false, filtered));
        // a folded field is zero wide: no pill, no ring. Windows 11: rounded s(4), as Explorer's address bar
        for (p, on) in [(pg.addr, addr_on), (pg.filt, filt_on)].into_iter().filter(|(p, _)| p.right > p.left) {
            round_fill(dc, p, s(4), pal().pill, head);
            // the shade goes under the focus ring: drawn after it, its corner pixels would cut the ring's inner curve.
            // It sits 1 px inside the pill, so its corners 1 px tighter keep to the pill's (and its ring's) curve
            if let (Some(slot), true, true) = (slot, kind_hot, p == pg.filt) {
                round_fill(dc, RECT { left: slot.left + 1, top: slot.top + 1, right: slot.right, bottom: slot.bottom - 1 }, s(4) - 1, shade, pal().pill);
            }
            if on {
                round_ring(dc, p, 1, s(4), if active { ink } else { accent }, head, pal().pill);
            }
        }
        if let (Some(il), Some(r), true) = (&ilist, addr_icon(pg.addr, dpi), folder >= 0) {
            shell_icon(dc, il, folder, r);
        }
        // idle address bar: C: › Users › Public, parents dimmed, the crumb under the mouse shaded,
        // a '›' whose subfolder list is on its way tinted and showing '…'
        if let Some(i) = i.filter(|_| crumbs_on && pg.addr.right > pg.addr.left) {
            for (k, (r, label, c)) in crumb_layout(dc, font, i, crumb_field(pg.addr, dpi)).into_iter().enumerate() {
                let wait = matches!(&c, Crumb::Kids(d, _) if waiting.as_ref() == Some(d));
                if wait {
                    round_fill(dc, r, s(3), blend(pal().pill, accent, 0.25), pal().pill);
                } else if hot == Some(k) {
                    round_fill(dc, r, s(3), blend(pal().pill, pal().text, 0.08), pal().pill);
                }
                if matches!(c, Crumb::Kids(..)) {
                    // '…' is a little wider than the '›' slot: let it reach into the neighbours' padding
                    text(dc, r, if wait { "…" } else { &label }, font, pal().sub, if wait { DT_CENTER | DT_NOCLIP } else { DT_CENTER });
                    continue;
                }
                let ink = if c == Crumb::Here { pal().text } else { pal().sub };
                text(dc, RECT { left: r.left + s(4), right: r.right - s(4), ..r }, &label, font, ink, DT_LEFT);
            }
        }
        if let Some((list, cur, icons)) = &tabs {
            paint_tabs(dc, &pg, list, *cur, dpi, font, focus_c, hot_head, ilist.as_ref().map(|il| (il, icons.as_slice())));
        }
        // a folder slow to open: the bar flows just above the list, over the tab strip's foot or the header's.
        // On tabs (strip or current tab card) and on an idle header the focus colour; on the active header its ink
        if let Some((l, r)) = loading.and_then(|e| load_bar(pg.view.right - pg.view.left, e, animate)) {
            let c = if tabs.is_some() || !active { focus_c } else { ink };
            fill(dc, RECT { left: pg.view.left + l, top: pg.view.top - s(2), right: pg.view.left + r, bottom: pg.view.top }, c);
        }
        if let Some(slot) = slot {
            if filtered {
                // filter applied: accent chip left of the edit (which starts at filt.left + s(24))
                let chip = RECT {
                    left: pg.filt.left + s(3),
                    top: pg.filt.top + s(3),
                    right: (pg.filt.left + s(21)).min(slot.right),
                    bottom: pg.filt.bottom - s(3),
                };
                if chip.right > chip.left {
                    round_fill(dc, chip, s(3), accent, if kind_hot { shade } else { pal().pill });
                    text(dc, chip, ICON_FILTER, icon_font, on_color(accent), DT_CENTER);
                }
            } else {
                let icon = RECT { left: pg.filt.left + s(4), right: (pg.filt.left + s(22)).min(slot.right), ..pg.filt };
                if icon.right > icon.left {
                    text(dc, icon, ICON_SEARCH, icon_font, if kind_hot { pal().text } else { pal().sub }, DT_CENTER);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;

    #[test]
    fn list_for_size() {
        let sides = [16, 32, 48];
        for (px, k) in [(12, 0), (16, 0), (20, 1), (24, 1), (32, 1), (40, 2), (48, 2), (64, 2)] {
            assert_eq!(pick_list(&sides, px), Some(k), "{px} px");
        }
        // signed in at 150 %: the small list already fits 16 px; lists in any order
        assert_eq!(pick_list(&[24, 48, 72], 16), Some(0));
        assert_eq!(pick_list(&[48, 16, 32], 24), Some(2));
        assert_eq!(pick_list(&[48, 16, 32], 96), Some(0));
        assert_eq!(pick_list(&[], 16), None);
    }

    // Colour menu chips: centred, the colour inside, nothing outside, smoothed corners, the empty chip a rim only.
    #[test]
    fn colour_chips() {
        let (fill, edge) = (0x8CC5A8u32, 0x112233u32);
        let at = |p: &Pix, x: i32, y: i32| {
            let i = ((y * p.w + x) * 4) as usize;
            [p.bgra[i], p.bgra[i + 1], p.bgra[i + 2], p.bgra[i + 3]]
        };
        let bgra = |c: u32| [(c & 0xFF) as u8, (c >> 8 & 0xFF) as u8, (c >> 16 & 0xFF) as u8, 255];
        for px in [16, 20, 24, 32] {
            for f in [Some(fill), None] {
                let p = swatch_pix(px, f, edge);
                assert_eq!((p.w, p.h, p.bgra.len()), (px, px, (px * px * 4) as usize), "{px} px");
                for y in 0..px {
                    for x in 0..px {
                        let a = at(&p, x, y);
                        assert!(a[..3].iter().all(|&c| c <= a[3]), "{px} px ({x}, {y}): not premultiplied");
                        assert_eq!(a, at(&p, px - 1 - x, y), "{px} px ({x}, {y}): not centred across");
                        assert_eq!(a, at(&p, x, px - 1 - y), "{px} px ({x}, {y}): not centred down");
                    }
                }
                // the margin around the chip is clear, the canvas corners too
                let o = (0..px).position(|x| at(&p, x, px / 2)[3] > 0).expect("a chip") as i32;
                let side = px - 2 * o;
                assert!(side >= px * 3 / 4 && side <= px * 3 / 4 + 1, "{px} px: chip {side} px wide");
                assert!(o >= 1, "{px} px: no margin");
                assert_eq!(at(&p, 0, 0)[3], 0);
                // straight sides crisp, corners rounded (not a full square corner) and smoothed (partly covered pixels)
                assert!((0..px).all(|x| matches!(at(&p, x, px / 2)[3], 0 | 255)), "{px} px: blurred side");
                assert!(at(&p, o, o)[3] < 255, "{px} px: square corner");
                let soft = (0..px / 2).flat_map(|y| (0..px / 2).map(move |x| (x, y))).filter(|&(x, y)| (1..255).contains(&at(&p, x, y)[3])).count();
                assert!(soft >= 2, "{px} px: {soft} smoothed corner pixels");
                // the rim along the top edge in the edge colour, opaque
                assert_eq!(at(&p, px / 2, o), bgra(edge), "{px} px: rim");
                let mid = at(&p, px / 2, px / 2);
                match f {
                    Some(c) => assert_eq!(mid, bgra(c), "{px} px: chip colour"),
                    None => assert_eq!(mid[3], 0, "{px} px: the empty chip is filled"),
                }
            }
        }
        // the rim stands off the menu even where the chip barely does: Gray on light and on dark, the empty chip
        for (p, gray) in [(&LIGHT, 0xC5C5C5), (&DARK, 0x4B4B4B)] {
            assert_eq!(p.accents[1].1, gray);
            for f in [Some(gray), None] {
                let k = contrast(chip_edge(f, p), p.card);
                assert!(k >= 3.0, "{f:?}: rim contrast {k:.2}");
            }
        }
    }

    const BG: [u8; 3] = [200, 120, 40]; // B, G, R: a colour, so a dark or light fringe shows

    // What f draws into a side x side 32-bit DIB filled with BG: B, G, R per pixel.
    unsafe fn drawn(side: i32, f: impl FnOnce(HDC, RECT)) -> Vec<[u8; 3]> {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: side,
                biHeight: -side,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let dc = CreateCompatibleDC(None);
        let mut bits = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).expect("DIB");
        let old = SelectObject(dc, bmp.into());
        let px = std::slice::from_raw_parts_mut(bits as *mut u8, (side * side * 4) as usize);
        for p in px.chunks_exact_mut(4) {
            p[..3].copy_from_slice(&BG);
            p[3] = 255;
        }
        f(dc, RECT { left: 0, top: 0, right: side, bottom: side });
        _ = GdiFlush();
        let out = px.chunks_exact(4).map(|p| [p[0], p[1], p[2]]).collect();
        SelectObject(dc, old);
        _ = DeleteDC(dc);
        _ = DeleteObject(bmp.into());
        out
    }

    // Premultiplied pixels laid over BG from the top-left, as shell_icon places them.
    fn over_bg(p: &Pix, side: i32) -> Vec<[u8; 3]> {
        let mut out = vec![BG; (side * side) as usize];
        for y in 0..p.h.min(side) {
            for x in 0..p.w.min(side) {
                let i = ((y * p.w + x) * 4) as usize;
                let a = p.bgra[i + 3] as u32;
                for c in 0..3 {
                    out[(y * side + x) as usize][c] = (p.bgra[i + c] as u32 + (BG[c] as u32 * (255 - a) + 127) / 255).min(255) as u8;
                }
            }
        }
        out
    }

    // How many pixels differ by more than `by` in some channel.
    fn far(a: &[[u8; 3]], b: &[[u8; 3]], by: u8) -> usize {
        a.iter().zip(b).filter(|(p, q)| (0..3).any(|c| p[c].abs_diff(q[c]) > by)).count()
    }

    // The folder icon as the address bar draws it at 200 % (dpi 192: s(16) = 32 px) and 150 % (24 px): the pixels
    // of the list that suits (on a 100 % runner the large list as is, then averaged down 32 -> 24 and alpha-blended),
    // not the small list stretched as before.
    #[test]
    fn icons_at_monitor_size() {
        unsafe {
            _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let mut fi = SHFILEINFOW::default();
            let flags = SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES;
            SHGetFileInfoW(&HSTRING::from(r"C:\Windows"), FILE_ATTRIBUTE_DIRECTORY, Some(&mut fi), size_of::<SHFILEINFOW>() as u32, flags);
            let folder = fi.iIcon;
            assert!(folder >= 0, "no folder icon index");
            let small = SHGetImageList::<IImageList>(SHIL_SMALL as i32).expect("small icon list");
            let lists = icon_lists();
            let sides: Vec<i32> = lists.iter().map(|l| l.2).collect();
            let mut averaged = false;
            for dpi in [192, 144] {
                let px = 16 * dpi / 96;
                let (shil, _, side) = &lists[pick_list(&sides, px).expect("icon lists")];
                assert!(*side >= px, "lists {sides:?}: none as large as {px} px");
                let src = list_icon(*shil as i32, folder).expect("folder icon");
                averaged |= *side > px;
                let want = over_bg(&if *side > px { shrink(&src, px) } else { src }, px);
                let got = drawn(px, |dc, r| assert!(shell_icon(dc, &small, folder, r)));
                assert_eq!(far(&got, &want, 4), 0, "dpi {dpi}: {px} px icon is not the {side} px list's (lists {sides:?})");
                if sides[0] != px {
                    let old = drawn(px, |dc, r| assert!(draw_list_icon(dc, &small, folder, r)));
                    let off = far(&old, &got, 8);
                    assert!(off > (px * px / 50) as usize, "dpi {dpi}: only {off} pixels differ from the small list stretched");
                }
            }
            assert!(averaged, "lists {sides:?}: the averaged-down path was not reached");
        }
    }
}
