use crate::*;

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
        fill(dc, chip, blend(p.bg, accent, 0.18));
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

// The system small icon list, loaded on first use and kept: address bar, Everything results and drive buttons.
// Called outside app(..), since the shell may send messages back.
pub(crate) unsafe fn small_ilist() -> Option<IImageList> {
    app(|a| a.ilist.clone()).or_else(|| {
        let l = SHGetImageList::<IImageList>(SHIL_SMALL as i32).ok();
        app(|a| a.ilist = l.clone());
        l
    })
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
    let mut rc = RECT::default();
    _ = unsafe { GetClientRect(h, &mut rc) };
    pane_geo(rc.right, rc.bottom, dpi).addr
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
            let Ok(menu) = CreatePopupMenu() else { return };
            for (k, (n, _)) in list.iter().enumerate() {
                _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(n.replace('&', "&&")));
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
            let id = want_kids(h, &dir, hidden());
            app(|a| a.kids = Some(KidsAsk { id, hwnd: h, i, path, gen: gen(), dir, next }));
            _ = InvalidateRect(Some(h), Some(&addr_rect(h, dpi)), false);
            return;
        }
        Some(_) => {
            focus_ctl(i, ID_EDIT);
            return;
        }
    };
    if target.is_some_and(|p| !navigate(i, &p)) {
        _ = MessageBeep(MB_ICONWARNING); // gone or unreachable
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
// ponytail: one worker, so a dead share's read_dir holds up the next '›' (shown waiting); a thread per request if that bites
pub(crate) struct KidsJob {
    pub(crate) id: usize,
    pub(crate) hwnd: isize,
    pub(crate) dir: String,
    pub(crate) hidden: bool,
    pub(crate) result: Option<(usize, KidsList)>,
}

impl KidsJob {
    pub(crate) fn ask(&mut self, hwnd: isize, dir: &str, hidden: bool) -> usize {
        (self.id, self.hwnd, self.dir, self.hidden, self.result) = (self.id + 1, hwnd, dir.to_string(), hidden, None);
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

pub(crate) static KIDS: Mutex<KidsJob> = Mutex::new(KidsJob { id: 0, hwnd: 0, dir: String::new(), hidden: false, result: None });
pub(crate) static KIDS_WAKE: Condvar = Condvar::new();
pub(crate) const KIDS_MAX: usize = 500;

pub(crate) fn kids_job() -> std::sync::MutexGuard<'static, KidsJob> {
    KIDS.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn want_kids(h: HWND, dir: &str, hidden: bool) -> usize {
    let id = kids_job().ask(h.0 as isize, dir, hidden);
    static WORKER: Once = Once::new();
    WORKER.call_once(|| _ = std::thread::Builder::new().spawn(kids_worker));
    KIDS_WAKE.notify_one();
    id
}

// Lives until the process ends. The lock is never held across file system calls.
pub(crate) fn kids_worker() {
    use std::os::windows::fs::MetadataExt;
    let mut seen = 0;
    loop {
        let (id, hwnd, dir, hidden) = {
            let mut j = kids_job();
            while j.id == seen || j.dir.is_empty() {
                j = KIDS_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
            (j.id, j.hwnd, j.dir.clone(), j.hidden)
        };
        seen = id;
        let all = std::fs::read_dir(&dir).map_err(|_| ());
        let r = all.map(|rd| {
            let named = rd.flatten().filter_map(|e| Some((e.file_name().to_string_lossy().into_owned(), e.metadata().ok()?.file_attributes())));
            kid_names(named, hidden)
        });
        if kids_job().publish(id, r) {
            unsafe { _ = PostMessageW(Some(HWND(hwnd as _)), WM_KIDS, WPARAM(id), LPARAM(0)) };
        }
    }
}

// Subfolders out of (name, attributes): hidden ones only when shown, hidden+system never (as Explorer),
// in Explorer's order ('2' before '10'), at most KIDS_MAX. (names, cut short)
pub(crate) fn kid_names(all: impl Iterator<Item = (String, u32)>, show_hidden: bool) -> (Vec<String>, bool) {
    use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};
    let (dir, hid, hs) = (FILE_ATTRIBUTE_DIRECTORY.0, FILE_ATTRIBUTE_HIDDEN.0, FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0);
    let mut v: Vec<HSTRING> =
        all.filter(|&(_, a)| a & dir != 0 && a & hs != hs && (show_hidden || a & hid == 0)).map(|(n, _)| HSTRING::from(n)).collect();
    v.sort_by(|a, b| unsafe { StrCmpLogicalW(a, b) }.cmp(&0));
    let cut = v.len() > KIDS_MAX;
    (v.iter().take(KIDS_MAX).map(|n| n.to_string_lossy()).collect(), cut)
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
    if !same || gen() != k.gen || GetForegroundWindow() != main || GetDlgItem(Some(h), ID_EDIT).is_ok_and(|e| IsWindowVisible(e).as_bool()) {
        return;
    }
    let dc = GetDC(Some(h));
    let items = crumb_layout(dc, font, k.i, crumb_field(addr, dpi));
    ReleaseDC(Some(h), dc);
    let Some(r) = kids_anchor(&items, &k.dir) else { return };
    let Ok(menu) = CreatePopupMenu() else { return };
    let gray = |t: &str| _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(t));
    let names = match &list {
        Ok((names, cut)) => {
            for (n, name) in names.iter().enumerate() {
                _ = AppendMenuW(menu, MF_STRING, n + 1, &HSTRING::from(name.replace('&', "&&")));
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
        if !navigate(k.i, &format!("{}\\{name}", k.dir.trim_end_matches('\\'))) {
            _ = MessageBeep(MB_ICONWARNING); // gone or unreachable
        }
    }
    focus_view(k.i);
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
    let (n, cur, dpi) = app(|a| (a.s.tab_list(i).len(), a.s.tab[i], a.dpi));
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    head_hot_at(&pane_geo(rc.right, rc.bottom, dpi), n, cur, x, y, dpi)
}

// Where a header target is painted in h: the whole tab strip (a tab lightening moves its neighbours' hairlines),
// or the filter icon's slot.
unsafe fn head_hot_rect(h: HWND, t: HeadHot, dpi: i32) -> RECT {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let pg = pane_geo(rc.right, rc.bottom, dpi);
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
    let (active, i, dpi, icon_font, small_font, font, accent, hot, hot_head, tint, folder, waiting) = app(|a| {
        let i = a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h));
        // a drop target-to-be looks active too
        let active = a.panes[a.s.active].as_ref().is_some_and(|p| p.hwnd == h) || a.drop_hover == h;
        let hot = a.crumb_hot.filter(|(w, _)| *w == h).map(|(_, k)| k);
        let hot_head = a.head_hot.filter(|(w, _)| *w == h).map(|(_, t)| t);
        let tint = i.and_then(|i| tint_color(a.s.tints[i]));
        let folder = i.and_then(|i| a.panes[i].as_ref()).map_or(-1, |p| p.icon);
        let waiting = a.kids.as_ref().filter(|k| k.hwnd == h).map(|k| k.dir.clone());
        (active, i, a.dpi, a.icon_font, a.small_font, a.font, a.s.accent_now(), hot, hot_head, tint, folder, waiting)
    });
    let tabs = i.filter(|_| tabs_on()).map(|i| app(|a| (a.s.tab_list(i), a.s.tab[i])));
    let ilist = if folder >= 0 { small_ilist() } else { app(|a| a.ilist.clone()) };
    let crumbs_on = GetDlgItem(Some(h), ID_EDIT).is_ok_and(|e| !IsWindowVisible(e).as_bool());
    let filtered = i.is_some_and(|i| FILTERS.lock().is_ok_and(|f| !f[i].is_empty()));
    let (head, ink) = header_colors(h);
    let focus = GetFocus();
    let focused = |id: i32| GetDlgItem(Some(h), id).is_ok_and(|e| e == focus);
    let (addr_on, filt_on) = (focused(ID_EDIT), focused(ID_FILTER));
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let pg = pane_geo(rc.right, rc.bottom, dpi);
        fill(dc, rc, pal().card);
        // active: header bar in the selection colour, same colour as the border
        fill(dc, RECT { bottom: pg.view.top, ..rc }, head);
        if active {
            frame(dc, rc, s(2), accent);
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
                    fill(dc, pg.num, t);
                    on_color(t)
                }
                None if active => ink,
                None => pal().sub,
            };
            text(dc, pg.num, &(i + 1).to_string(), small_font, ink, DT_CENTER);
        }
        for (p, on) in [(pg.addr, addr_on), (pg.filt, filt_on)] {
            fill(dc, p, pal().pill);
            if on {
                frame(dc, p, 1, if active { ink } else { accent });
            }
        }
        if let (Some(il), Some(r), true) = (&ilist, addr_icon(pg.addr, dpi), folder >= 0) {
            if let Ok(icon) = il.GetIcon(folder, ILD_NORMAL.0) {
                _ = DrawIconEx(dc, r.left, r.top, icon, r.right - r.left, r.bottom - r.top, 0, None, DI_NORMAL);
                _ = DestroyIcon(icon);
            }
        }
        // idle address bar: C: › Users › Public, parents dimmed, the crumb under the mouse shaded,
        // a '›' whose subfolder list is on its way tinted and showing '…'
        if let Some(i) = i.filter(|_| crumbs_on) {
            for (k, (r, label, c)) in crumb_layout(dc, font, i, crumb_field(pg.addr, dpi)).into_iter().enumerate() {
                let wait = matches!(&c, Crumb::Kids(d, _) if waiting.as_ref() == Some(d));
                if wait {
                    fill(dc, r, blend(pal().pill, accent, 0.25));
                } else if hot == Some(k) {
                    fill(dc, r, blend(pal().pill, pal().text, 0.08));
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
        if let Some((list, cur)) = &tabs {
            paint_tabs(dc, &pg, list, *cur, dpi, font, accent, hot_head);
        }
        // the icon opens the file-type menu: under the mouse its slot is shaded (inside the pill and its focus frame),
        // a ring round the filter badge, whose own colour stays
        let kind_hot = hot_head == Some(HeadHot::Kind);
        // a narrow pane's filter field can be under s(24) wide: the shade, icon and badge all stay inside the icon slot
        if let Some(slot) = filter_icon_rect(pg.filt, s(24)) {
            if kind_hot {
                let shade = blend(pal().pill, pal().text, press_tint(false, filtered));
                fill(dc, RECT { left: slot.left + 1, top: slot.top + 1, right: slot.right, bottom: slot.bottom - 1 }, shade);
            }
            if filtered {
                // filter applied: accent chip left of the edit (which starts at filt.left + s(24))
                let chip = RECT {
                    left: pg.filt.left + s(3),
                    top: pg.filt.top + s(3),
                    right: (pg.filt.left + s(21)).min(slot.right),
                    bottom: pg.filt.bottom - s(3),
                };
                if chip.right > chip.left {
                    fill(dc, chip, accent);
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
