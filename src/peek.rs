use crate::*;
use windows::Win32::UI::Shell::PropertiesSystem::{IInitializeWithFile, IInitializeWithStream};

// ---------- F3 quick preview ----------

#[derive(Clone)]
pub(crate) struct PeekReq {
    pub(crate) path: String,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) hwnd: isize, // popup told by WM_PEEK_DONE
    pub(crate) plain: bool, // the preview handler failed: read the file ourselves
}

pub(crate) struct Pix {
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) bgra: Vec<u8>, // premultiplied, top-down
}

// One worker, newest request only. gen moves on every want, so a result for an item already left
// (or a closed popup) is dropped instead of shown.
pub(crate) struct PeekJob {
    pub(crate) gen: usize,
    pub(crate) req: Option<PeekReq>,
    pub(crate) done: Option<Peeked>,
    pub(crate) zoom: Option<(u64, Sharp)>, // sharper pictures wanted for the zoom on screen (seq, what)
    pub(crate) zseq: u64,                  // moves on every zoom request; older results are dropped
    pub(crate) sharp: Vec<(u32, Pix)>,     // (page, picture) rendered for the zoom; layout stays on the fitted ones
}

// The picture, or the PDF pages in view, again at w x h: zoomed in, the fitted ones would only be enlarged.
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Sharp {
    pub(crate) gen: usize,
    pub(crate) pdf: bool,
    pub(crate) pages: Vec<u32>,
    pub(crate) w: i32,
    pub(crate) h: i32,
}

pub(crate) struct Peeked {
    pub(crate) body: Body,
    pub(crate) icon: i32,          // system image list index for the header, -1 = none
    pub(crate) meta: Vec<String>,  // header facts: type, dimensions / pages / lines, size, date
}

impl PeekJob {
    pub(crate) fn want(&mut self, req: Option<PeekReq>) {
        (self.gen, self.req, self.done, self.zoom) = (self.gen + 1, req, None, None);
        self.sharp.clear();
    }

    pub(crate) fn want_sharp(&mut self, z: Option<Sharp>) {
        self.zseq += 1;
        self.sharp.clear();
        self.zoom = z.map(|z| (self.zseq, z));
    }

    pub(crate) fn publish(&mut self, gen: usize, r: Peeked) -> bool {
        if self.gen != gen {
            return false;
        }
        self.done = Some(r);
        true
    }

    // One more PDF page for the item still on screen.
    pub(crate) fn add_page(&mut self, gen: usize, p: Pix) -> bool {
        match &mut self.done {
            Some(Peeked { body: Body::Pages { pages, .. }, .. }) if self.gen == gen => {
                pages.push(p);
                true
            }
            _ => false,
        }
    }
}

pub(crate) static PEEK: Mutex<PeekJob> = Mutex::new(PeekJob { gen: 0, req: None, done: None, zoom: None, zseq: 0, sharp: Vec::new() });
pub(crate) static PEEK_WAKE: Condvar = Condvar::new();

pub(crate) fn peek_job() -> std::sync::MutexGuard<'static, PeekJob> {
    PEEK.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn want_peek(req: Option<PeekReq>) {
    let wake = req.is_some();
    peek_job().want(req);
    if wake {
        static WORKER: Once = Once::new();
        WORKER.call_once(|| _ = std::thread::Builder::new().spawn(peek_worker));
        PEEK_WAKE.notify_one();
    }
}

// Lives until the process ends. COM is up only while one preview is made; the lock is never held then.
// A new item goes first; a zoom's sharper pictures are made for the item already shown.
pub(crate) fn peek_worker() {
    let mut seen = 0;
    loop {
        let (gen, req, zoom) = {
            let mut j = peek_job();
            loop {
                let z = j.zoom.take();
                if let Some(r) = j.req.clone() {
                    if j.gen != seen {
                        break (Some(j.gen), r, None);
                    }
                    if z.is_some() {
                        break (None, r, z);
                    }
                }
                j = PEEK_WAKE.wait(j).unwrap_or_else(|e| e.into_inner());
            }
        };
        unsafe {
            let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            match (gen, zoom) {
                (Some(gen), _) => {
                    seen = gen;
                    peek_item(gen, &req);
                }
                (None, Some((seq, z))) => sharpen(seq, z, &req),
                _ => {}
            }
            if com {
                CoUninitialize();
            }
        }
    }
}

unsafe fn peek_item(gen: usize, req: &PeekReq) {
    let body = content(req);
    let (icon, meta) = meta(&req.path, &body);
    let more = match &body {
        Body::Pages { pages, total } if (pages.len() as u32) < *total => Some(*total),
        _ => None,
    };
    let tell = || _ = PostMessageW(Some(HWND(req.hwnd as _)), WM_PEEK_DONE, WPARAM(gen), LPARAM(0));
    if peek_job().publish(gen, Peeked { body, icon, meta }) {
        tell();
        // the first page is up; the rest follow while this is still the item shown
        if let (Some(total), Ok(doc)) = (more, pdf_open(&req.path)) {
            for i in 1..total {
                let Some(p) = (peek_job().gen == gen).then(|| pdf_page(&doc, i, req.w, req.h)).flatten() else { break };
                if !peek_job().add_page(gen, p) {
                    break;
                }
                tell();
            }
        }
    }
}

unsafe fn sharpen(seq: u64, z: Sharp, req: &PeekReq) {
    let doc = if z.pdf { pdf_open(&req.path).ok() } else { None };
    for &i in &z.pages {
        let current = |j: &PeekJob| j.gen == z.gen && j.zseq == seq;
        if !current(&peek_job()) {
            return;
        }
        let p = match &doc {
            Some(d) => pdf_page(d, i, z.w, z.h),
            None => shell_pic(&req.path, z.w, z.h, SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY),
        };
        let Some(p) = p else { return };
        let mut j = peek_job();
        if !current(&j) {
            return;
        }
        j.sharp.retain(|(k, _)| *k != i);
        j.sharp.push((i, p));
        drop(j);
        _ = PostMessageW(Some(HWND(req.hwnd as _)), WM_PEEK_DONE, WPARAM(z.gen), LPARAM(0));
    }
}

// Photos come back with every alpha byte 0: make them opaque. Icons come back with straight alpha (a colour
// brighter than its alpha says so), which AlphaBlend would draw as a dirty fringe: premultiply those.
pub(crate) fn fix_alpha(bgra: &mut [u8]) {
    if bgra.chunks_exact(4).all(|p| p[3] == 0) {
        bgra.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    } else if bgra.chunks_exact(4).any(|p| p[0].max(p[1]).max(p[2]) > p[3]) {
        for p in bgra.chunks_exact_mut(4) {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

// The popup is laid out like the main window: the frame colour around, content on a plate.
pub(crate) struct PeekGeo {
    pub(crate) icon: RECT,  // file icon in the header
    pub(crate) title: RECT, // name, then the facts
    pub(crate) close: RECT,
    pub(crate) stage: RECT, // pictures and pages sit here; everything else on a plate filling it
    pub(crate) inner: RECT, // the plate's content box
    pub(crate) prev: RECT,  // ‹ and › in the side margins
    pub(crate) next: RECT,
}

pub(crate) fn peek_geo(rc: RECT, dpi: i32) -> PeekGeo {
    let s = |v: i32| v * dpi / 96;
    let (head, side, b, ic, pad) = (s(48), s(56), s(32), s(16), s(20));
    let close = RECT { left: rc.right - s(12) - b, top: (head - b) / 2, right: rc.right - s(12), bottom: (head + b) / 2 };
    let icon = RECT { left: side, top: (head - ic) / 2, right: side + ic, bottom: (head + ic) / 2 };
    let title = RECT { left: icon.right + s(10), top: 0, right: close.left - s(12), bottom: head };
    let stage = RECT { left: side, top: head, right: rc.right - side, bottom: rc.bottom - s(24) };
    let inner = RECT { left: stage.left + pad, top: stage.top + pad, right: stage.right - pad, bottom: stage.bottom - pad };
    let mid = (stage.top + stage.bottom) / 2;
    let arrow = |x: i32| RECT { left: x, top: mid - b / 2, right: x + b, bottom: mid + b / 2 };
    PeekGeo { icon, title, close, stage, inner, prev: arrow((side - b) / 2), next: arrow(rc.right - side + (side - b) / 2) }
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(crate) enum Hot {
    #[default]
    None,
    Prev,
    Next,
    Close,
}

pub(crate) fn peek_hit(g: &PeekGeo, x: i32, y: i32) -> Hot {
    match () {
        _ if inside(g.close, x, y) => Hot::Close,
        _ if inside(g.prev, x, y) => Hot::Prev,
        _ if inside(g.next, x, y) => Hot::Next,
        _ => Hot::None,
    }
}

unsafe fn geo_of(h: HWND) -> PeekGeo {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    peek_geo(rc, app(|a| a.dpi))
}

// UI-thread state of the open preview: scroll position, laid-out text, the hosted handler.
#[derive(Default)]
pub(crate) struct PeekView {
    pub(crate) y: i32, // scrolled down, px
    pub(crate) x: i32, // scrolled right (tables, code), px
    pub(crate) max_y: i32, // how far it can scroll down, from the last paint; 0 = it all fits
    pub(crate) edge_at: Option<Instant>, // the wheel reached the top / bottom then
    pub(crate) wheel: i32, // wheel delta toward the next file step (120 a notch)
    pub(crate) layout: Option<(usize, i32, Vec<Line>, i32)>, // (gen, width, lines, total height)
    pub(crate) fonts: Option<(i32, [HFONT; 4])>,              // (dpi, [mono, bold, h1, h2])
    pub(crate) hosted: Option<usize>, // generation a preview handler is showing
    pub(crate) host_win: Option<HWND>, // the picture area's child window the handler draws in
    pub(crate) hover: bool,            // the mouse is over the popup: ‹ › show
    pub(crate) hot: Hot,
    pub(crate) canvas: Canvas,         // what the last paint showed, for zoom, pan and the bars
    pub(crate) content: (i32, i32),    // its size at this zoom, px
    pub(crate) zoom: i32,              // zoom steps from fit (0), see zoom_k
    pub(crate) zwheel: i32,            // Ctrl+wheel delta toward the next step
    pub(crate) zwant: Option<Sharp>,   // sharper pictures this zoom would like (from the last paint)
    pub(crate) zsent: Option<Sharp>,   // ... and the ones asked for
    pub(crate) drag: Option<Drag>,
    pub(crate) rename: Option<(HWND, Option<&'static str>)>, // the name box in the header, its complaint
    pub(crate) busy: bool, // the shell's own dialogs are up: going inactive doesn't close the preview
    pub(crate) settle: i32, // timer ticks waited for the shell's progress window to go
}

#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(crate) enum Canvas {
    #[default]
    None,
    Pic,
    Pages,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Drag {
    Pan { at: POINT, x: i32, y: i32 }, // where the press was, the scroll then
    Bar { vertical: bool, grab: i32 }, // grab: press point within the thumb
}

thread_local! {
    pub(crate) static VIEW: RefCell<PeekView> = RefCell::new(PeekView::default());
}

fn view<R>(f: impl FnOnce(&mut PeekView) -> R) -> R {
    VIEW.with(|v| f(&mut v.borrow_mut()))
}

pub(crate) unsafe fn open_peek(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let (main, dpi, open) = app(|a| (a.main, a.dpi, a.peek));
    if !open.0.is_null() {
        return;
    }
    if fv.GetFocusedItem().unwrap_or(-1) < 0 {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let s = |v: i32| v * dpi / 96;
    let mut rc = RECT::default();
    _ = GetClientRect(main, &mut rc);
    let (w, h) = ((rc.right * 86 / 100).min(s(1120)), (rc.bottom * 88 / 100).min(s(860)));
    let mut pt = POINT { x: (rc.right - w) / 2, y: (rc.bottom - h) / 2 };
    _ = ClientToScreen(main, &mut pt);
    let style = WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsPeek"), w!(""), style, pt.x, pt.y, w, h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| (a.peek, a.peek_pane) = (h, i));
    load_peek(false);
}

// Asks the worker for the pane's focused item at the picture area's size. plain: skip the preview handler.
pub(crate) unsafe fn load_peek(plain: bool) {
    let Some(fv) = view_of::<IFolderView2>(app(|a| a.peek_pane)) else { return };
    show_item(fv.GetItem::<IShellItem>(fv.GetFocusedItem().unwrap_or(-1)).ok(), plain);
}

unsafe fn show_item(it: Option<IShellItem>, plain: bool) {
    let (h, dpi) = app(|a| (a.peek, a.dpi));
    unload_handler();
    view(|v| {
        (v.y, v.x, v.layout, v.max_y, v.edge_at, v.wheel) = (0, 0, None, 0, None, 0);
        (v.zoom, v.zwheel, v.zwant, v.zsent, v.drag) = (0, 0, None, None, None);
    });
    let shown = |kind| it.as_ref().and_then(|it| it.GetDisplayName(kind).ok()).map(|p| pwstr(p));
    app(|a| a.peek_name = shown(SIGDN_NORMALDISPLAY).unwrap_or_default());
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let st = peek_geo(rc, dpi).stage;
    match shown(SIGDN_FILESYSPATH).or_else(|| shown(SIGDN_DESKTOPABSOLUTEPARSING)) {
        Some(path) => want_peek(Some(PeekReq { path, w: st.right - st.left, h: st.bottom - st.top, hwnd: h.0 as isize, plain })),
        None => {
            let mut j = peek_job();
            j.want(None);
            j.done = Some(Peeked { body: Body::Card { icon: None, rows: Vec::new() }, icon: -1, meta: Vec::new() });
        }
    }
    _ = InvalidateRect(Some(h), None, false);
}

// Next / previous in the view's own order. Only a real neighbour (S_OK, in range) that got selected moves it.
pub(crate) unsafe fn step_peek(prev: bool) {
    close_rename();
    let Some(fv) = view_of::<IFolderView2>(app(|a| a.peek_pane)) else { return };
    let cur = fv.GetFocusedItem().unwrap_or(-1);
    let mut next = -1;
    let hr = (Interface::vtable(&fv).GetVisibleItem)(Interface::as_raw(&fv), cur, prev.into(), &mut next);
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let flags = SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0;
    if hr == S_OK && (0..n).contains(&next) && fv.SelectItem(next, flags as u32).is_ok() {
        load_peek(false);
    }
    refocus();
}

// Selecting puts the focus in the list; the preview stays open and keeps the keys, like Quick Look.
unsafe fn refocus() {
    let h = app(|a| a.peek);
    if !h.0.is_null() && GetActiveWindow() != h {
        _ = SetActiveWindow(h);
        _ = SetFocus(Some(h));
    }
}

pub(crate) unsafe fn close_peek(h: HWND) {
    let i = app(|a| a.peek_pane);
    _ = DestroyWindow(h);
    focus_view(i);
}

// ---------- Del and F2 on the file shown ----------

// The shell does it, as Explorer would: its prompts, the Recycle Bin, undo. Its dialogs run a modal loop and
// take the activation; busy keeps that from closing the preview. false = failed or cancelled.
unsafe fn shell_op(h: HWND, path: &str, flags: FILEOPERATION_FLAGS, add: impl FnOnce(&IFileOperation, &IShellItem) -> Result<()>) -> bool {
    view(|v| v.busy = true);
    let done = (|| -> Result<bool> {
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        op.SetOwnerWindow(h)?;
        op.SetOperationFlags(flags)?;
        add(&op, &SHCreateItemFromParsingName(&HSTRING::from(path), None)?)?;
        op.PerformOperations()?;
        Ok(!op.GetAnyOperationsAborted()?.as_bool())
    })()
    .unwrap_or(false);
    if IsWindow(Some(h)).as_bool() {
        _ = SetTimer(Some(h), SETTLE_TIMER, 100, None);
    } else {
        view(|v| v.busy = false);
    }
    done
}

const SETTLE_TIMER: usize = 2;

// The shell's progress window stays up a moment after the operation and has the activation: once it has gone,
// the preview takes it back, or closes if another app has it now.
unsafe fn settle(h: HWND) {
    let fg = GetForegroundWindow();
    let mut cls = [0u16; 32];
    let n = GetClassNameW(fg, &mut cls) as usize;
    if String::from_utf16_lossy(&cls[..n]) == "OperationStatusWindow" && view(|v| (v.settle < 50).then(|| v.settle += 1)).is_some() {
        return;
    }
    _ = KillTimer(Some(h), SETTLE_TIMER);
    view(|v| (v.busy, v.settle) = (false, 0));
    let mut pid = 0;
    GetWindowThreadProcessId(fg, Some(&mut pid));
    if fg.0.is_null() || pid == windows::Win32::System::Threading::GetCurrentProcessId() {
        _ = SetForegroundWindow(h);
        refocus();
    } else {
        _ = DestroyWindow(h);
    }
}

fn shown_path() -> Option<String> {
    peek_job().req.as_ref().map(|r| r.path.clone())
}

// Del: to the Recycle Bin (Shift+Del: for good), then the next file in the pane, or the previous, or closed.
unsafe fn delete_peek(h: HWND, permanent: bool) {
    close_rename();
    let i = app(|a| a.peek_pane);
    let (Some(path), Some(fv), Some(sv)) = (shown_path(), view_of::<IFolderView2>(i), view_of::<IShellView>(i)) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    // the neighbour by its ID, not its position: the list may or may not have dropped the deleted one yet
    let cur = fv.GetFocusedItem().unwrap_or(-1);
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let near = |prev: bool| {
        let mut k = -1;
        let hr = (Interface::vtable(&fv).GetVisibleItem)(Interface::as_raw(&fv), cur, prev.into(), &mut k);
        (hr == S_OK && (0..n).contains(&k)).then(|| fv.Item(k).ok()).flatten()
    };
    let next = near(false).or_else(|| near(true));
    // a hosted handler may hold the file open
    let hosted = view(|v| v.host_win.is_some());
    unload_handler();
    let flags = if permanent { FOF_WANTNUKEWARNING } else { FOF_ALLOWUNDO | FOF_WANTNUKEWARNING };
    let gone = shell_op(h, &path, flags, |op, it| op.DeleteItem(it, None)) && !Path::new(&path).exists();
    match next {
        _ if !IsWindow(Some(h)).as_bool() => {}
        Some(pidl) if gone => {
            let flags = SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0;
            _ = sv.SelectItem(pidl, flags as u32);
            load_peek(false);
        }
        None if gone => return close_peek(h),
        _ if hosted => load_peek(false),
        _ => {}
    }
    if let Some(pidl) = next {
        CoTaskMemFree(Some(pidl as _));
    }
    refocus();
    _ = SetFocus(Some(h));
}

/// Why Windows won't take this as a file name, None = fine. Clashes are checked against the folder.
pub(crate) fn name_problem(name: &str) -> Option<&'static str> {
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    match () {
        _ if name.trim().is_empty() => Some(tr("Type a name", "이름을 입력하세요")),
        _ if name.chars().any(|c| c < ' ' || r#"\/:*?"<>|"#.contains(c)) => Some(tr(r#"A name can't contain \ / : * ? " < > |"#, r#"이름에 \ / : * ? " < > | 는 쓸 수 없습니다"#)),
        _ if name.ends_with(['.', ' ']) => Some(tr("A name can't end with a dot or a space", "이름은 점이나 공백으로 끝날 수 없습니다")),
        _ if device => Some(tr("Windows keeps this name for itself", "Windows가 예약한 이름입니다")),
        _ if name.encode_utf16().count() > 255 => Some(tr("The name is too long", "이름이 너무 깁니다")),
        _ => None,
    }
}

/// The part F2 selects: the name without its extension (all of it for folders and dot files), in UTF-16 units.
pub(crate) fn stem_len(name: &str, dir: bool) -> usize {
    let stem = match name.rfind('.') {
        Some(k) if k > 0 && !dir => &name[..k],
        _ => name,
    };
    stem.encode_utf16().count()
}

// The name box sits where the name is drawn, on a field like the address bar's.
fn rename_rects(title: RECT, fh: i32, dpi: i32) -> (RECT, RECT) {
    let s = |v: i32| v * dpi / 96;
    let mid = (title.top + title.bottom) / 2;
    let field = RECT { left: title.left - s(6), top: mid - fh / 2 - s(5), right: title.left + (title.right - title.left) * 55 / 100, bottom: mid + fh / 2 + s(5) };
    (field, RECT { left: field.left + s(6), top: mid - fh / 2, right: field.right - s(6), bottom: mid + fh / 2 + 1 })
}

unsafe fn start_rename(h: HWND) {
    if view(|v| v.rename.is_some()) {
        return;
    }
    let Some(path) = shown_path() else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let p = Path::new(&path);
    let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let (font, fh, dpi) = app(|a| (a.font, a.fh, a.dpi));
    let (_, r) = rename_rects(geo_of(h).title, fh, dpi);
    let edit = child(h, w!("EDIT"), PCWSTR(HSTRING::from(name.as_str()).as_ptr()), ES_AUTOHSCROLL as u32, WINDOW_EX_STYLE(0), 1, font);
    _ = SetWindowPos(edit, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER);
    SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(stem_len(&name, p.is_dir()) as isize)));
    view(|v| v.rename = Some((edit, None)));
    _ = SetFocus(Some(edit));
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) unsafe fn close_rename() {
    let Some((edit, _)) = view(|v| v.rename.take()) else { return };
    let h = app(|a| a.peek);
    if GetFocus() == edit {
        _ = SetFocus(Some(h));
    }
    _ = DestroyWindow(edit);
    _ = InvalidateRect(Some(h), None, false);
}

// Enter in the name box: a bad name or a clash only says so and keeps the box; the shell renames the rest.
unsafe fn apply_rename(h: HWND) {
    let (Some((edit, _)), Some(path)) = (view(|v| v.rename), shown_path()) else { return };
    let typed = text_of(edit);
    let name = typed.trim();
    let old = Path::new(&path);
    let to = old.with_file_name(name);
    if Some(name) == old.file_name().and_then(|n| n.to_str()) {
        return close_rename();
    }
    // only a change of case may land on the name that is already there: it is the same file
    let clash = to.exists() && to.to_string_lossy().to_lowercase() != path.to_lowercase();
    let problem = name_problem(name).or_else(|| clash.then(|| tr("Something with this name is already here", "같은 이름이 이미 있습니다")));
    let done = problem.is_none() && {
        let hosted = view(|v| v.host_win.is_some());
        unload_handler();
        let ok = shell_op(h, &path, FOF_ALLOWUNDO | FOFX_ADDUNDORECORD, |op, it| op.RenameItem(it, &HSTRING::from(name), None)) && to.exists();
        if !ok && hosted {
            load_peek(false);
        }
        ok
    };
    if done {
        close_rename();
        // the pane learns it from the shell; the preview goes to the new name now
        show_item(SHCreateItemFromParsingName(&HSTRING::from(to.as_os_str()), None).ok(), false);
        _ = SetFocus(Some(h));
        return;
    }
    _ = MessageBeep(MB_ICONWARNING);
    view(|v| v.rename = Some((edit, Some(problem.unwrap_or(tr("Windows didn't rename it", "이름을 바꾸지 못했습니다"))))));
    _ = SetFocus(Some(edit));
    _ = InvalidateRect(Some(h), None, false);
}

// Keys for the preview: false sends them on to it (none of the panes' shortcuts act behind it); the name box
// takes Enter and Esc here, as a one-line edit would only beep at them.
pub(crate) unsafe fn peek_key(m: &MSG) -> Option<bool> {
    let peek = app(|a| a.peek);
    if peek.0.is_null() {
        return None;
    }
    if m.hwnd == peek {
        return Some(false);
    }
    view(|v| v.rename.map(|r| r.0)).filter(|&e| e == m.hwnd)?;
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        VK_RETURN if m.message == WM_KEYDOWN => {
            app(|x| x.held = Some(VK_RETURN.0));
            apply_rename(peek);
        }
        VK_ESCAPE => close_rename(),
        _ => return Some(false),
    }
    Some(true)
}

// ---------- zoom: pictures and PDF pages ----------

pub(crate) const ZOOM_MIN: i32 = -4; // 41 %
pub(crate) const ZOOM_MAX: i32 = 10; // 931 %
const ZOOM_TIMER: usize = 1;
const SHARP_MAX: f32 = 16e6; // pixels in one sharper picture

pub(crate) fn zoom_k(steps: i32) -> f32 {
    1.25f32.powi(steps)
}

/// The scroll that keeps the content under the cursor (`at` px into the view) where it is when `len` px of
/// content becomes `new_len`. Centred content (smaller than the view) is placed in the middle.
pub(crate) fn zoom_scroll(at: i32, scroll: i32, len: i32, new_len: i32, view: i32, centred: bool) -> i32 {
    let pad = |l: i32| if centred { ((view - l) / 2).max(0) } else { 0 };
    let on = (scroll + at - pad(len)) as f64 / len.max(1) as f64;
    ((on * new_len as f64) as i32 - at + pad(new_len)).clamp(0, (new_len - view).max(0))
}

/// Where content `len` px long starts in a view from `lo`, `view` px long: centred if it fits, else scrolled.
pub(crate) fn origin(lo: i32, view: i32, len: i32, scroll: i32, centred: bool) -> i32 {
    if len <= view && centred {
        lo + (view - len) / 2
    } else {
        lo - scroll
    }
}

/// Scrollbars of zoomed content along the stage's right and bottom edges, each only where it scrolls.
pub(crate) fn zoom_bars(st: RECT, content: (i32, i32), dpi: i32) -> [Option<RECT>; 2] {
    let s = |v: i32| v * dpi / 96;
    let (vert, horz) = (content.1 > st.bottom - st.top, content.0 > st.right - st.left);
    let (w, m) = (s(8), s(3));
    [
        vert.then(|| RECT { left: st.right - m - w, top: st.top + m, right: st.right - m, bottom: st.bottom - m - if horz { w + m } else { 0 } }),
        horz.then(|| RECT { left: st.left + m, top: st.bottom - m - w, right: st.right - m - if vert { w + m } else { 0 }, bottom: st.bottom - m }),
    ]
}

/// The thumb on a track lo..hi for `view` px of `len` px of content, scrolled `pos`.
pub(crate) fn thumb(lo: i32, hi: i32, view: i32, len: i32, pos: i32, min: i32) -> (i32, i32) {
    let track = hi - lo;
    let t = (track as i64 * view as i64 / len.max(1) as i64) as i32;
    let t = t.max(min).min(track);
    let room = (len - view).max(1);
    let at = lo + ((track - t) as i64 * pos.clamp(0, room) as i64 / room as i64) as i32;
    (at, at + t)
}

/// The scroll for a thumb whose top (left) is dragged to `at`.
pub(crate) fn thumb_scroll(lo: i32, hi: i32, view: i32, len: i32, at: i32, min: i32) -> i32 {
    let (a, b) = thumb(lo, hi, view, len, 0, min);
    let free = (hi - lo) - (b - a);
    if free <= 0 {
        return 0;
    }
    ((at - lo).clamp(0, free) as i64 * (len - view).max(0) as i64 / free as i64) as i32
}

// Ctrl+wheel (centred on the cursor), Ctrl+= / Ctrl+- (on the middle); None: Ctrl+0, back to fit.
unsafe fn zoom_peek(h: HWND, by: Option<i32>, at: Option<POINT>) {
    let st = geo_of(h).stage;
    let (vw, vh) = (st.right - st.left, st.bottom - st.top);
    let p = at.unwrap_or(POINT { x: (st.left + st.right) / 2, y: (st.top + st.bottom) / 2 });
    let fit = view(|v| {
        let old = v.zoom;
        let new = by.map_or(0, |d| (old + d).clamp(ZOOM_MIN, ZOOM_MAX));
        if v.canvas == Canvas::None || new == old {
            return None;
        }
        let r = zoom_k(new) / zoom_k(old);
        let (cw, ch) = v.content;
        let grown = |l: i32| (l as f32 * r).round() as i32;
        v.x = zoom_scroll(p.x - st.left, v.x, cw, grown(cw), vw, true);
        v.y = zoom_scroll(p.y - st.top, v.y, ch, grown(ch), vh, v.canvas == Canvas::Pic);
        (v.zoom, v.content, v.edge_at, v.wheel) = (new, (grown(cw), grown(ch)), None, 0);
        Some(new <= 0)
    });
    match fit {
        // fitted (or smaller): the first pictures are sharp enough
        Some(true) => {
            peek_job().want_sharp(None);
            view(|v| (v.zwant, v.zsent) = (None, None));
        }
        Some(false) => {}
        None => return,
    }
    _ = InvalidateRect(Some(h), None, false);
}

// The last paint wanted sharper pictures than those asked for: ask, once the zoom or scroll has settled.
unsafe fn want_sharper(h: HWND) {
    _ = KillTimer(Some(h), ZOOM_TIMER);
    let Some(z) = view(|v| (v.zwant != v.zsent).then(|| v.zwant.clone()).flatten()) else { return };
    view(|v| v.zsent = Some(z.clone()));
    peek_job().want_sharp(Some(z));
    PEEK_WAKE.notify_one();
}

// How big the sharper picture of a fitted one (w x h, shown on a stage aw x ah) is made at zoom k.
pub(crate) fn sharp_size(w: i32, h: i32, aw: i32, ah: i32, k: f32) -> Option<(i32, i32)> {
    let k = k.min((SHARP_MAX / (w.max(1) * h.max(1)) as f32).sqrt());
    (k > 1.15).then(|| ((aw as f32 * k) as i32, (ah as f32 * k) as i32))
}

// ---------- Windows preview handlers (Office, Outlook, HTML, fonts, PDF readers) ----------

// The handler wanted now (its preview generation), 0 = none. A host thread whose generation is no longer
// this one unloads and quits, so Esc or an arrow never waits for a slow handler.
pub(crate) static HOST_GEN: AtomicUsize = AtomicUsize::new(0);

pub(crate) unsafe fn unload_handler() {
    HOST_GEN.store(0, Ordering::SeqCst);
    if let Some(w) = view(|v| {
        v.hosted = None;
        v.host_win.take()
    }) {
        _ = DestroyWindow(w); // the handler's window goes with it, even while it is still starting
    }
}

// UTF-8 HTML that doesn't name its charset: the old HTML previewer reads it as the ANSI code page, so it
// gets a BOM in front. Everything else is handed over as the file.
unsafe fn handler_stream(path: &str) -> Result<IStream> {
    let name = HSTRING::from(path);
    if matches!(ext_of(path).as_str(), "html" | "htm") {
        if let Ok(b) = std::fs::read(path) {
            let head = String::from_utf8_lossy(&b[..b.len().min(4096)]).to_ascii_lowercase();
            if !b.starts_with(&[0xEF, 0xBB, 0xBF]) && !head.contains("charset") && std::str::from_utf8(&b).is_ok() && !b.is_ascii() {
                let with_bom = [&[0xEF, 0xBB, 0xBF][..], &b].concat();
                return SHCreateMemStream(Some(&with_bom)).ok_or_else(|| Error::from(E_FAIL));
            }
        }
    }
    SHCreateStreamOnFileEx(&name, (STGM_READ | STGM_SHARE_DENY_NONE).0, 0, false, None)
}

// Made where Explorer makes them (prevhost.exe, out of process), given the file as a stream when they take one.
unsafe fn host_handler(h: HWND, clsid: &GUID, path: &str, area: RECT) -> Result<IPreviewHandler> {
    let ph: IPreviewHandler = CoCreateInstance(clsid, None, CLSCTX_LOCAL_SERVER).or_else(|_| CoCreateInstance(clsid, None, CLSCTX_INPROC_SERVER))?;
    let name = HSTRING::from(path);
    if let Ok(init) = ph.cast::<IInitializeWithStream>() {
        init.Initialize(&handler_stream(path)?, STGM_READ.0)?;
    } else if let Ok(init) = ph.cast::<IInitializeWithItem>() {
        let item: IShellItem = SHCreateItemFromParsingName(&name, None)?;
        init.Initialize(&item, STGM_READ.0)?;
    } else {
        ph.cast::<IInitializeWithFile>()?.Initialize(&name, STGM_READ.0)?;
    }
    if let Ok(vis) = ph.cast::<IPreviewHandlerVisuals>() {
        _ = vis.SetBackgroundColor(cref(pal().card));
        _ = vis.SetTextColor(cref(pal().text));
    }
    ph.SetWindow(h, &area)?;
    ph.DoPreview()?;
    Ok(ph)
}

// One thread per hosted preview: it starts the handler, tells the popup (WM_PEEK_HOSTED, lParam 1 = shown),
// keeps it while it is the one wanted, then unloads it on its own apartment.
fn host_thread(gen: usize, clsid: GUID, path: String, hwnd: isize, popup: isize, area: RECT) {
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let h = HWND(hwnd as _);
        let ph = host_handler(h, &clsid, &path, area);
        let wanted = || HOST_GEN.load(Ordering::SeqCst) == gen;
        if wanted() {
            _ = PostMessageW(Some(HWND(popup as _)), WM_PEEK_HOSTED, WPARAM(gen), LPARAM(ph.is_ok() as isize));
        }
        if let Ok(ph) = ph {
            let mut msg = MSG::default();
            while wanted() && IsWindow(Some(h)).as_bool() {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                _ = MsgWaitForMultipleObjects(None, false, 100, QS_ALLINPUT);
            }
            _ = ph.Unload();
        }
        if com {
            CoUninitialize();
        }
    }
}

// The worker said "a handler shows this": start it on its own thread, once per item.
unsafe fn start_handler(h: HWND, gen: usize) {
    let (clsid, path) = {
        let j = peek_job();
        match (&j.done, &j.req) {
            (Some(Peeked { body: Body::Handler(g), .. }), Some(r)) if j.gen == gen => (*g, r.path.clone()),
            _ => return,
        }
    };
    if HOST_GEN.swap(gen, Ordering::SeqCst) == gen {
        return;
    }
    // handlers size themselves to their parent: give them the plate inside its 1px edge, nothing else
    let st = geo_of(h).stage;
    let (w, ht) = (st.right - st.left - 2, st.bottom - st.top - 2);
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(win) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsPeekHost"), w!(""), style, st.left + 1, st.top + 1, w, ht, Some(h), None, None, None) else {
        return load_peek(true);
    };
    view(|v| v.host_win = Some(win));
    let (hwnd, popup, area) = (win.0 as isize, h.0 as isize, RECT { left: 0, top: 0, right: w, bottom: ht });
    if std::thread::Builder::new().spawn(move || host_thread(gen, clsid, path, hwnd, popup, area)).is_err() {
        load_peek(true);
    }
}

// The handler's parent: the plate's colour (and "Opening…") until the handler draws over it.
pub(crate) extern "system" fn peek_host_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let font = app(|a| a.font);
                buffered(h, |dc, rc| {
                    fill(dc, rc, pal().card);
                    text(dc, rc, tr("Opening…   Esc: cancel", "여는 중…   Esc: 취소"), font, pal().sub, DT_CENTER);
                });
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// The host thread is done starting: shown, or read the file ourselves.
unsafe fn handler_ready(h: HWND, gen: usize, ok: bool) {
    if HOST_GEN.load(Ordering::SeqCst) != gen || peek_job().gen != gen {
        return;
    }
    if ok {
        view(|v| v.hosted = Some(gen));
        // the handler's window may have taken the focus: arrows and Esc stay ours
        if GetActiveWindow() == h {
            _ = SetFocus(Some(h));
        }
    } else {
        load_peek(true);
    }
    _ = InvalidateRect(Some(h), None, false);
}

// ---------- text layout ----------

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Style {
    Plain,
    Mono, // not wrapped
    H1,
    H2,
    H3,
    Bullet,
    Quote,
}

pub(crate) struct Line {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) y: i32,
    pub(crate) h: i32,
    pub(crate) no: usize, // source line number, for the code gutter
    table: Option<(Vec<Vec<String>>, Vec<i32>)>,
}

/// Markdown lines with their look: headings, bullets, quotes, fenced code. Inline marks are dropped.
pub(crate) fn md_lines(text: &str) -> Vec<(String, Style)> {
    let mut out = Vec::new();
    let mut code = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            code = !code;
            continue;
        }
        if code {
            out.push((l.to_string(), Style::Mono));
            continue;
        }
        let clean = |s: &str| s.replace("**", "").replace("__", "").replace('`', "");
        let h3 = t.strip_prefix("### ").or_else(|| t.strip_prefix("#### ")).or_else(|| t.strip_prefix("##### "));
        let item = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ "));
        let (text, style) = if let Some(h) = h3 {
            (clean(h), Style::H3)
        } else if let Some(h) = t.strip_prefix("## ") {
            (clean(h), Style::H2)
        } else if let Some(h) = t.strip_prefix("# ") {
            (clean(h), Style::H1)
        } else if let Some(b) = item {
            let indent = "    ".repeat((l.len() - t.len()) / 2);
            (format!("{indent}• {}", clean(b)), Style::Bullet)
        } else if let Some(q) = t.strip_prefix("> ").or_else(|| t.strip_prefix('>')) {
            (clean(q), Style::Quote)
        } else if t.len() >= 3 && t.chars().all(|c| c == '-') {
            ("────────".into(), Style::Quote)
        } else {
            (clean(l), Style::Plain)
        };
        out.push((text, style));
    }
    out
}

unsafe fn fonts(dpi: i32, base: HFONT) -> [HFONT; 4] {
    if let Some((d, f)) = view(|v| v.fonts) {
        if d == dpi {
            return f;
        }
        for x in f {
            _ = DeleteObject(x.into());
        }
    }
    let mut lf = LOGFONTW::default();
    GetObjectW(base.into(), size_of::<LOGFONTW>() as i32, Some(&mut lf as *mut _ as _));
    let mono = CreateFontIndirectW(&LOGFONTW { lfFaceName: logfont("Consolas", 0).lfFaceName, ..lf });
    let bold = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, ..lf });
    let h1 = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, lfHeight: lf.lfHeight * 16 / 10, ..lf });
    let h2 = CreateFontIndirectW(&LOGFONTW { lfWeight: 600, lfHeight: lf.lfHeight * 13 / 10, ..lf });
    let f = [mono, bold, h1, h2];
    view(|v| v.fonts = Some((dpi, f)));
    f
}

fn style_font(st: Style, font: HFONT, f: &[HFONT; 4]) -> HFONT {
    match st {
        Style::Mono => f[0],
        Style::H1 => f[2],
        Style::H2 => f[3],
        Style::H3 => f[1],
        _ => font,
    }
}

// Heights of every line at this width, measured once per item and width; painting then draws only what shows.
unsafe fn layout(dc: HDC, gen: usize, width: i32, text: &str, kind: TextKind, font: HFONT, f: &[HFONT; 4], dpi: i32) -> i32 {
    if let Some(total) = view(|v| v.layout.as_ref().filter(|(g, w, ..)| *g == gen && *w == width).map(|l| l.3)) {
        return total;
    }
    let src: Vec<(String, Style)> = match kind {
        TextKind::Markdown => md_lines(text),
        TextKind::Code => text.lines().map(|l| (l.to_string(), Style::Mono)).collect(),
        TextKind::Plain => text.lines().map(|l| (l.to_string(), Style::Plain)).collect(),
        TextKind::Document => text.lines().map(|l| (l.to_string(), Style::Plain)).collect(),
    };
    let mut y = 0;
    let mut lines = Vec::with_capacity(src.len().min(40_000));
    let mut source = src.into_iter().take(40_000).enumerate().peekable();
    while let Some((no, (t, st))) = source.next() {
        if kind == TextKind::Document && t.starts_with("| ") {
            let mut rows = Vec::new();
            let parse = |line: &str| -> Vec<String> {
                line.strip_prefix("| ").unwrap_or(line).strip_suffix('|').unwrap_or(line)
                    .split(" | ").map(|cell| cell.trim().to_string()).collect()
            };
            rows.push(parse(&t));
            while source.peek().is_some_and(|(_, (line, _))| line.starts_with("| ")) {
                if let Some((_, (line, _))) = source.next() {
                    rows.push(parse(&line));
                }
            }
            let mut widths = Vec::<i32>::new();
            let scale = |v: i32| v * dpi / 96;
            for row in &rows {
                for (col, cell) in row.iter().enumerate() {
                    if widths.len() <= col {
                        widths.push(scale(36));
                    }
                    widths[col] = widths[col].max((text_w(dc, cell, font) + scale(14)).min(scale(260)));
                }
            }
            let rh = app(|a| a.fh) + scale(8);
            let h = rh * rows.len() as i32;
            lines.push(Line { text: String::new(), style: st, y, h, no: no + 1, table: Some((rows, widths)) });
            y += h;
            continue;
        }
        let old = SelectObject(dc, style_font(st, font, f).into());
        let mut tm = TEXTMETRICW::default();
        _ = GetTextMetricsW(dc, &mut tm);
        let lh = tm.tmHeight + tm.tmExternalLeading;
        let h = if st == Style::Mono || t.is_empty() {
            lh
        } else {
            let mut w: Vec<u16> = t.encode_utf16().collect();
            let mut r = RECT { left: 0, top: 0, right: width, bottom: 0 };
            DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_WORDBREAK | DT_EXPANDTABS | DT_NOPREFIX | DT_EDITCONTROL);
            (r.bottom - r.top).max(lh)
        };
        SelectObject(dc, old);
        y += match st {
            Style::H1 | Style::H2 => lh / 2,
            Style::H3 => lh / 3,
            _ => 0,
        };
        lines.push(Line { text: t, style: st, y, h, no: no + 1, table: None });
        y += h + if matches!(st, Style::H1 | Style::H2) { dpi / 24 } else { 0 };
    }
    view(|v| v.layout = Some((gen, width, lines, y)));
    y
}

// ---------- painting ----------

fn clamp_scroll(total: i32, page: i32) {
    view(|v| {
        v.max_y = (total - page).max(0);
        v.y = v.y.min(v.max_y).max(0);
    });
}

#[derive(Debug, PartialEq)]
pub(crate) enum Wheel {
    Scroll,
    Step,
    Hold, // at the end, but only just: a fling to the bottom doesn't also jump to the next file
}

// A wheel turn scrolls while there is room that way; past the end it moves to the previous / next file,
// at once when nothing scrolls, else once the view has rested at that end for a moment.
pub(crate) fn wheel_action(y: i32, max_y: i32, down: bool, rested: bool) -> Wheel {
    let room = if down { y < max_y } else { y > 0 };
    match () {
        _ if room => Wheel::Scroll,
        _ if max_y == 0 || rested => Wheel::Step,
        _ => Wheel::Hold,
    }
}

unsafe fn wheel_peek(h: HWND, raw: i32) {
    let down = raw < 0;
    let px = -raw * app(|a| a.fh) * 3 / 120;
    let act = view(|v| {
        let rested = v.edge_at.is_some_and(|t| t.elapsed() >= Duration::from_millis(350));
        let act = wheel_action(v.y, v.max_y, down, rested);
        match act {
            Wheel::Scroll => {
                v.y = (v.y + px).clamp(0, v.max_y);
                let end = if down { v.y >= v.max_y } else { v.y <= 0 };
                (v.edge_at, v.wheel) = (end.then(Instant::now), 0);
            }
            Wheel::Hold => {
                v.edge_at.get_or_insert_with(Instant::now);
            }
            Wheel::Step => {}
        }
        act
    });
    match act {
        Wheel::Scroll => _ = InvalidateRect(Some(h), None, false),
        Wheel::Step => step_by_wheel(raw),
        Wheel::Hold => {}
    }
}

// One file per notch; a touchpad's small deltas add up to one.
unsafe fn step_by_wheel(raw: i32) {
    let go = view(|v| {
        if (v.wheel < 0) != (raw < 0) {
            v.wheel = 0;
        }
        v.wheel += raw;
        let go = v.wheel.abs() >= 120;
        if go {
            v.wheel = 0;
        }
        go
    });
    if go {
        step_peek(raw > 0);
    }
}

// DrawTextW without text()'s single-line, end-ellipsis defaults.
pub(crate) unsafe fn text_flags(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, flags: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return; // see text()
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(rgb));
    DrawTextW(dc, &mut w, &mut r, flags | DT_NOPREFIX);
    SelectObject(dc, old);
}

unsafe fn text_w(dc: HDC, s: &str, font: HFONT) -> i32 {
    let w: Vec<u16> = s.chars().take(200).collect::<String>().encode_utf16().collect();
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    SelectObject(dc, old);
    sz.cx
}

unsafe fn clip(dc: HDC, r: RECT, draw: impl FnOnce()) {
    let rgn = CreateRectRgn(r.left, r.top, r.right, r.bottom);
    SelectClipRgn(dc, Some(rgn));
    draw();
    SelectClipRgn(dc, None);
    _ = DeleteObject(rgn.into());
}

unsafe fn paint_text(dc: HDC, area: RECT, gen: usize, text: &str, kind: TextKind, cut: bool, font: HFONT, dpi: i32, pic: Option<&Pix>) {
    let s = |v: i32| v * dpi / 96;
    let f = fonts(dpi, font);
    let gutter = if kind == TextKind::Code { s(44) } else { 0 };
    let width = area.right - area.left - gutter - s(8);
    let pic_size = pic.map(|p| {
        let k = ((area.right - area.left) as f32 / p.w as f32)
            .min((area.bottom - area.top) as f32 / p.h as f32);
        ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32)
    });
    let offset = pic_size.map_or(0, |(_, h)| h + s(20));
    let total = offset + layout(dc, gen, width, text, kind, font, &f, dpi) + if cut { s(28) } else { 0 };
    clamp_scroll(total, area.bottom - area.top);
    if kind == TextKind::Document {
        view(|v| {
            let full = v.layout.as_ref().map(|(_, _, lines, _)| {
                lines.iter().filter_map(|l| l.table.as_ref())
                    .map(|(_, widths)| widths.iter().sum::<i32>()).max().unwrap_or(0)
            }).unwrap_or(0);
            v.x = v.x.clamp(0, (full - (area.right - area.left)).max(0));
        });
    }
    let (sy, sx) = view(|v| (v.y, v.x));
    clip(dc, area, || {
        if let (Some(p), Some((w, h))) = (pic, pic_size) {
            let x = (area.left + area.right - w) / 2;
            let y = area.top - sy;
            blit_bgra(dc, RECT { left: x, top: y, right: x + w, bottom: y + h }, p.w, p.h, &p.bgra);
        }
        let sy = sy - offset;
        view(|v| {
            let Some((_, _, lines, _)) = v.layout.as_ref() else { return };
            let first = lines.partition_point(|l| l.y + l.h < sy);
            for l in lines[first..].iter().take_while(|l| l.y - sy < area.bottom - area.top) {
                let top = area.top + l.y - sy;
                if let Some((rows, widths)) = &l.table {
                    let rh = app(|a| a.fh) + s(8);
                    let first = ((area.top - top).max(0) / rh) as usize;
                    let shown = ((area.bottom - area.top) / rh + 2) as usize;
                    for (row_no, row) in rows.iter().enumerate().skip(first).take(shown) {
                        let row_top = top + row_no as i32 * rh;
                        let mut x = area.left - sx;
                        for (col, w) in widths.iter().enumerate() {
                            if x + w > area.left && x < area.right {
                                let cell = row.get(col).map(String::as_str).unwrap_or("");
                                let rect = RECT { left: x + s(7), top: row_top, right: x + w - s(7), bottom: row_top + rh };
                                crate::draw::text(dc, rect, cell, font, pal().text, DT_LEFT);
                                fill(dc, RECT { left: x + w - 1, top: row_top, right: x + w, bottom: row_top + rh }, pal().line);
                                fill(dc, RECT { left: x, top: row_top + rh - 1, right: x + w, bottom: row_top + rh }, pal().line);
                            }
                            x += w;
                        }
                        fill(dc, RECT { left: area.left - sx, top: row_top, right: area.left - sx + 1, bottom: row_top + rh }, pal().line);
                    }
                    let table_width: i32 = widths.iter().sum();
                    fill(dc, RECT { left: area.left - sx, top, right: area.left - sx + table_width, bottom: top + 1 }, pal().line);
                    continue;
                }
                if gutter > 0 {
                    let g = RECT { left: area.left, top, right: area.left + gutter - s(10), bottom: top + l.h };
                    text_flags(dc, g, &l.no.to_string(), f[0], pal().sub, DT_RIGHT | DT_SINGLELINE);
                }
                let mono = l.style == Style::Mono;
                let (ink, fnt) = match l.style {
                    Style::Quote => (pal().sub, font),
                    st => (pal().text, style_font(st, font, &f)),
                };
                let left = area.left + gutter - if mono { sx } else { 0 };
                let r = RECT { left, top, right: if mono { left + 100_000 } else { area.right - s(8) }, bottom: top + l.h };
                let flags = if mono { DT_SINGLELINE | DT_EXPANDTABS } else { DT_WORDBREAK | DT_EXPANDTABS | DT_EDITCONTROL };
                text_flags(dc, r, &l.text, fnt, ink, flags);
            }
        });
        if cut {
            let y = area.top + total - offset - s(24) - sy;
            let r = RECT { left: area.left, top: y, right: area.right, bottom: y + s(24) };
            text_flags(dc, r, tr("… only the first 512 KB is shown", "… 앞부분 512 KB만 표시"), font, pal().sub, DT_SINGLELINE);
        }
    });
}

// A grid: column widths from the first rows' text, the first row as a header, scrolled both ways.
unsafe fn paint_table(dc: HDC, area: RECT, rows: &[Vec<String>], cut: bool, font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let f = fonts(dpi, font);
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        text(dc, area, tr("Empty", "비어 있음"), font, pal().sub, DT_CENTER);
        return;
    }
    let mut widths = vec![s(36); cols];
    for (k, r) in rows.iter().take(200).enumerate() {
        for (c, cell) in r.iter().enumerate() {
            let w = text_w(dc, cell, if k == 0 { f[1] } else { font }) + s(14);
            widths[c] = widths[c].max(w.min(s(260)));
        }
    }
    let num = text_w(dc, &rows.len().to_string(), font) + s(14);
    let rh = app(|a| a.fh) + s(8);
    let total = rh * rows.len() as i32 + if cut { rh } else { 0 };
    clamp_scroll(total, area.bottom - area.top);
    let full: i32 = num + widths.iter().sum::<i32>();
    view(|v| v.x = v.x.min((full - (area.right - area.left)).max(0)).max(0));
    let (sy, sx) = view(|v| (v.y, v.x));
    clip(dc, area, || {
        let first = (sy / rh) as usize;
        let shown = ((area.bottom - area.top) / rh + 2) as usize;
        for (k, r) in rows.iter().enumerate().skip(first).take(shown) {
            let top = area.top + k as i32 * rh - sy;
            let row = RECT { left: area.left, top, right: area.right, bottom: top + rh };
            if k == 0 {
                fill(dc, row, pal().pill);
            }
            fill(dc, RECT { top: row.bottom - 1, ..row }, pal().line);
            text(dc, RECT { left: area.left, right: area.left + num - s(6), ..row }, &(k + 1).to_string(), font, pal().sub, DT_RIGHT);
            let mut x = area.left + num - sx;
            for (c, w) in widths.iter().enumerate() {
                if x + w > area.left + num && x < area.right {
                    let cell = r.get(c).map(String::as_str).unwrap_or("");
                    let cr = RECT { left: (x + s(7)).max(area.left + num), top, right: x + w - s(7), bottom: top + rh };
                    let right = k > 0 && !cell.is_empty() && cell.replace(',', "").parse::<f64>().is_ok();
                    text(dc, cr, cell, if k == 0 { f[1] } else { font }, pal().text, if right { DT_RIGHT } else { DT_LEFT });
                    fill(dc, RECT { left: x + w - 1, top, right: x + w, bottom: top + rh }, pal().line);
                }
                x += w;
            }
        }
        if cut {
            let top = area.top + rows.len() as i32 * rh - sy;
            let r = RECT { left: area.left + s(6), top, right: area.right, bottom: top + rh };
            text(dc, r, tr("… more rows not shown", "… 이후 행은 생략"), font, pal().sub, DT_LEFT);
        }
    });
}

unsafe fn paint_list(dc: HDC, area: RECT, rows: &[(String, String)], font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    let rh = app(|a| a.fh) + s(8);
    let top0 = area.top;
    if rows.is_empty() {
        text(dc, area, tr("Empty", "비어 있음"), font, pal().sub, DT_CENTER);
        return;
    }
    let page = area.bottom - top0;
    clamp_scroll(rh * rows.len() as i32, page);
    let sy = view(|v| v.y);
    let sw = s(90);
    clip(dc, RECT { top: top0, ..area }, || {
        for (k, (name, size)) in rows.iter().enumerate().skip((sy / rh) as usize).take((page / rh + 2) as usize) {
            let top = top0 + k as i32 * rh - sy;
            let r = RECT { left: area.left, top, right: area.right, bottom: top + rh };
            if k % 2 == 1 {
                fill(dc, RECT { left: r.left - s(8), right: r.right + s(8), ..r }, blend(pal().card, pal().bg, 0.45));
            }
            text(dc, RECT { right: r.right - sw - s(8), ..r }, name, font, pal().text, DT_LEFT);
            text(dc, RECT { left: r.right - sw, ..r }, size, font, pal().sub, DT_RIGHT);
        }
    });
}

// Big icon (or thumbnail) on the left, the facts beside it.
unsafe fn paint_card(dc: HDC, area: RECT, icon: Option<&Pix>, rows: &[(String, String)], font: HFONT, dpi: i32) {
    let s = |v: i32| v * dpi / 96;
    // drawn at its own size (only shrunk if it doesn't fit): scaling an icon blurs and fringes it
    let side = icon.map_or(s(96), |p| p.w.max(p.h)).min((area.right - area.left) / 3).min(area.bottom - area.top);
    // icon and facts as one block, centred
    let lw = rows.iter().map(|(l, _)| text_w(dc, l, font)).max().unwrap_or(0) + s(16);
    let vw = rows.iter().map(|(_, v)| text_w(dc, v, font)).max().unwrap_or(0);
    let block = side + s(32) + lw + vw;
    let x0 = area.left + ((area.right - area.left - block) / 2).max(s(8));
    if let Some(p) = icon {
        let k = (side as f32 / p.w.max(p.h) as f32).min(1.0);
        let (dw, dh) = ((p.w as f32 * k) as i32, (p.h as f32 * k) as i32);
        let (x, y) = (x0 + (side - dw) / 2, area.top + (area.bottom - area.top - dh) / 2);
        blit_bgra(dc, RECT { left: x, top: y, right: x + dw, bottom: y + dh }, p.w, p.h, &p.bgra);
    }
    let left = x0 + side + s(32);
    let rh = app(|a| a.fh) + s(8);
    let mut y = area.top + ((area.bottom - area.top) - rh * rows.len() as i32).max(0) / 2;
    for (label, value) in rows {
        text(dc, RECT { left, top: y, right: left + lw, bottom: y + rh }, label, font, pal().sub, DT_LEFT);
        let v = RECT { left: left + lw, top: y + s(4), right: area.right, bottom: y + rh * 3 };
        let mut w: Vec<u16> = value.encode_utf16().collect();
        let mut calc = v;
        let old = SelectObject(dc, font.into());
        DrawTextW(dc, &mut w, &mut calc, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL);
        SelectObject(dc, old);
        text_flags(dc, v, value, font, pal().text, DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);
        y += (calc.bottom - calc.top).clamp(rh - s(8), rh * 3 - s(8)) + s(8);
    }
}

/// The part of a picture drawn at `to` that falls inside `clip`: (source x0, y0, x1, y1) and where that part goes.
/// Zoomed in, only what shows is copied and stretched, not the whole picture.
pub(crate) fn visible_part(to: RECT, clip: RECT, w: i32, h: i32) -> Option<([i32; 4], RECT)> {
    let (dw, dh) = ((to.right - to.left).max(1) as i64, (to.bottom - to.top).max(1) as i64);
    let (l, t) = (clip.left.max(to.left), clip.top.max(to.top));
    let (r, b) = (clip.right.min(to.right), clip.bottom.min(to.bottom));
    if l >= r || t >= b || w <= 0 || h <= 0 {
        return None;
    }
    let src = |d: i32, o: i32, n: i64, len: i64| ((d - o) as i64 * n / len) as i32;
    let x0 = src(l, to.left, w as i64, dw);
    let y0 = src(t, to.top, h as i64, dh);
    let x1 = (((r - to.left) as i64 * w as i64 + dw - 1) / dw).min(w as i64) as i32;
    let y1 = (((b - to.top) as i64 * h as i64 + dh - 1) / dh).min(h as i64) as i32;
    let dst = |sv: i32, o: i32, n: i64, len: i64| o + (sv as i64 * len / n) as i32;
    let at = RECT { left: dst(x0, to.left, w as i64, dw), top: dst(y0, to.top, h as i64, dh), right: dst(x1, to.left, w as i64, dw), bottom: dst(y1, to.top, h as i64, dh) };
    Some(([x0, y0, x1, y1], at))
}

unsafe fn draw_pix(dc: HDC, to: RECT, clip: RECT, p: &Pix) {
    let Some(([x0, y0, x1, y1], at)) = visible_part(to, clip, p.w, p.h) else { return };
    if (x0, y0, x1, y1) == (0, 0, p.w, p.h) {
        return blit_bgra(dc, to, p.w, p.h, &p.bgra);
    }
    let row = (x1 - x0) as usize * 4;
    let mut part = Vec::with_capacity(row * (y1 - y0) as usize);
    for y in y0..y1 {
        let from = (y * p.w + x0) as usize * 4;
        part.extend_from_slice(&p.bgra[from..from + row]);
    }
    blit_bgra(dc, at, x1 - x0, y1 - y0, &part);
}

// Records what is shown and how big, keeps the scroll inside it; returns the scroll.
fn canvas(c: Canvas, area: RECT, cw: i32, ch: i32) -> (i32, i32) {
    view(|v| {
        (v.canvas, v.content) = (c, (cw, ch));
        v.max_y = (ch - (area.bottom - area.top)).max(0);
        v.x = v.x.clamp(0, (cw - (area.right - area.left)).max(0));
        v.y = v.y.clamp(0, v.max_y);
        (v.x, v.y)
    })
}

// A picture shrunk to fit if the shell handed back something bigger, never enlarged; then zoomed.
unsafe fn paint_pic(dc: HDC, st: RECT, p: &Pix, sharp: Option<&Pix>, k: f32) -> Option<Sharp> {
    let (aw, ah) = (st.right - st.left, st.bottom - st.top);
    let fit = (aw as f32 / p.w as f32).min(ah as f32 / p.h as f32).min(1.0);
    let (dw, dh) = ((p.w as f32 * fit * k) as i32, (p.h as f32 * fit * k) as i32);
    let (sx, sy) = canvas(Canvas::Pic, st, dw, dh);
    let (x, y) = (origin(st.left, aw, dw, sx, true), origin(st.top, ah, dh, sy, true));
    let r = RECT { left: x, top: y, right: x + dw, bottom: y + dh };
    clip(dc, st, || {
        draw_pix(dc, r, st, sharp.unwrap_or(p));
        frame(dc, RECT { left: x - 1, top: y - 1, right: x + dw + 1, bottom: y + dh + 1 }, 1, pal().line);
    });
    // a picture smaller than the stage came at its own size: there is no sharper one
    let whole = p.w < aw - 2 && p.h < ah - 2;
    (!whole).then(|| sharp_size(p.w, p.h, aw, ah, k)).flatten().map(|(w, h)| Sharp { gen: 0, pdf: false, pages: vec![0], w, h })
}

// PDF pages one under another, each its own sheet of paper on the stage.
unsafe fn paint_pages(dc: HDC, area: RECT, pages: &[Pix], sharp: &[(u32, Pix)], total: u32, font: HFONT, dpi: i32, k: f32) -> Option<Sharp> {
    let s = |v: i32| v * dpi / 96;
    let aw = area.right - area.left;
    let gap = (s(16) as f32 * k) as i32;
    let more = (pages.len() as u32) < total;
    let size = |p: &Pix| {
        let w = p.w.min(aw);
        ((w as f32 * k) as i32, ((p.h * w / p.w.max(1)) as f32 * k) as i32)
    };
    let cw = pages.iter().map(|p| size(p).0).max().unwrap_or(0);
    let height: i32 = pages.iter().map(|p| size(p).1 + gap).sum::<i32>() - gap + if more { s(48) } else { 0 };
    let (sx, sy) = canvas(Canvas::Pages, area, cw, height);
    let x0 = origin(area.left, aw, cw, sx, true);
    let mut shown = Vec::new();
    clip(dc, area, || {
        let mut y = area.top - sy;
        for (i, p) in pages.iter().enumerate() {
            let (w, h) = size(p);
            let x = x0 + (cw - w) / 2;
            if y + h > area.top && y < area.bottom {
                let r = RECT { left: x, top: y, right: x + w, bottom: y + h };
                let better = sharp.iter().find(|(n, _)| *n == i as u32).map(|(_, p)| p);
                draw_pix(dc, r, area, better.unwrap_or(p));
                frame(dc, RECT { left: r.left - 1, top: r.top - 1, right: r.right + 1, bottom: r.bottom + 1 }, 1, pal().line);
                shown.push(i as u32);
            }
            y += h + gap;
        }
        if more {
            let r = RECT { left: area.left, top: y, right: area.right, bottom: y + s(32) };
            let t = format!("{} {} / {}", tr("Rendering pages…", "페이지 그리는 중…"), pages.len(), total);
            text(dc, r, &t, font, pal().sub, DT_CENTER);
        }
    });
    let p = pages.get(*shown.first()? as usize)?;
    shown.truncate(4);
    sharp_size(p.w, p.h, aw, area.bottom - area.top, k).map(|(w, h)| Sharp { gen: 0, pdf: true, pages: shown, w, h })
}

pub(crate) unsafe fn paint_peek(h: HWND) {
    let (dpi, font, icon_font, name, ilist, fh, accent) = app(|a| (a.dpi, a.font, a.icon_font, a.peek_name.clone(), a.ilist.clone(), a.fh, a.s.accent_now()));
    let (hosted, hover, hot, steps, renaming) = view(|v| (v.hosted, v.hover, v.hot, v.zoom, v.rename.map(|r| r.1)));
    let k = zoom_k(steps);
    let mut want = None;
    view(|v| v.canvas = Canvas::None);
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let g = peek_geo(rc, dpi);
        let f = fonts(dpi, font);
        fill(dc, rc, pal().bg);
        frame(dc, rc, 1, pal().line);
        let j = peek_job();
        let gen = j.gen;
        // header: icon, name in bold, the facts after it in the secondary colour
        if let (Some(p), Some(il)) = (&j.done, &ilist) {
            if let Some(icon) = (p.icon >= 0).then(|| il.GetIcon(p.icon, ILD_NORMAL.0).ok()).flatten() {
                let r = g.icon;
                _ = DrawIconEx(dc, r.left, r.top, icon, r.right - r.left, r.bottom - r.top, 0, None, DI_NORMAL);
                _ = DestroyIcon(icon);
            }
        }
        let room = g.title.right - g.title.left;
        if let Some(problem) = renaming {
            // F2: the name box on a field, its complaint (or how to finish) after it
            let (field, _) = rename_rects(g.title, fh, dpi);
            fill(dc, field, pal().pill);
            frame(dc, field, 1, accent);
            let after = RECT { left: field.right + s(16), ..g.title };
            match problem {
                Some(p) => text(dc, after, p, f[1], pal().text, DT_LEFT),
                None => text(dc, after, tr("Enter: rename   Esc: cancel", "Enter: 바꾸기   Esc: 취소"), font, pal().sub, DT_LEFT),
            }
        } else {
            let name_w = text_w(dc, &name, f[1]).min(room * 6 / 10);
            text(dc, RECT { right: g.title.left + name_w, ..g.title }, &name, f[1], pal().text, DT_LEFT);
            if let Some(p) = &j.done {
                let facts = RECT { left: g.title.left + name_w + s(16), ..g.title };
                let mut meta = p.meta.join("   ·   ");
                if steps != 0 && matches!(p.body, Body::Pic(_) | Body::Pages { .. }) {
                    meta = format!("{meta}   ·   {:.0}%", k * 100.0);
                }
                text(dc, facts, &meta, font, pal().sub, DT_LEFT);
            }
        }
        if hot == Hot::Close {
            fill(dc, g.close, blend(pal().bg, pal().text, 0.08));
        }
        text(dc, g.close, ICON_CLOSE, icon_font, if hot == Hot::Close { pal().text } else { pal().sub }, DT_CENTER);
        // content: pictures and pages on the stage; everything else on a plate, as panes are
        let plate = || {
            fill(dc, g.stage, pal().card);
            frame(dc, g.stage, 1, pal().line);
        };
        match j.done.as_ref().map(|p| &p.body) {
            // zoomed in, the sharper pictures stand in for the fitted ones
            Some(Body::Pic(p)) => want = paint_pic(dc, g.stage, p, j.sharp.first().filter(|_| steps > 0).map(|x| &x.1), k),
            Some(Body::Pages { pages, total }) => {
                want = paint_pages(dc, g.stage, pages, if steps > 0 { &j.sharp } else { &[] }, *total, font, dpi, k);
            }
            Some(Body::Text { text: t, kind, cut }) => {
                plate();
                paint_text(dc, g.inner, gen, t, *kind, *cut, font, dpi, None);
            }
            Some(Body::Document { pic, text: t, cut }) => {
                plate();
                paint_text(dc, g.inner, gen, t, TextKind::Document, *cut, font, dpi, pic.as_ref());
            }
            Some(Body::Table { rows, cut }) => {
                plate();
                paint_table(dc, g.inner, rows, *cut, font, dpi);
            }
            Some(Body::List { rows, .. }) => {
                plate();
                paint_list(dc, g.inner, rows, font, dpi);
            }
            Some(Body::Card { icon, rows }) => {
                plate();
                paint_card(dc, g.inner, icon.as_ref(), rows, font, dpi);
            }
            Some(Body::Handler(_)) => {
                plate();
                if hosted != Some(gen) {
                    text(dc, g.stage, tr("Opening…   Esc: cancel", "여는 중…   Esc: 취소"), font, pal().sub, DT_CENTER);
                }
            }
            None => text(dc, g.stage, tr("Loading…", "불러오는 중…"), font, pal().sub, DT_CENTER),
        }
        drop(j);
        if steps != 0 {
            let (content, sx, sy) = view(|v| (v.content, v.x, v.y));
            let (vw, vh) = (g.stage.right - g.stage.left, g.stage.bottom - g.stage.top);
            let [vb, hb] = zoom_bars(g.stage, content, dpi);
            for (bar, vertical) in [(vb, true), (hb, false)] {
                let Some(b) = bar else { continue };
                fill(dc, b, pal().bg);
                frame(dc, b, 1, pal().line);
                let t = if vertical {
                    let (a, z) = thumb(b.top, b.bottom, vh, content.1, sy, s(24));
                    RECT { top: a, bottom: z, ..b }
                } else {
                    let (a, z) = thumb(b.left, b.right, vw, content.0, sx, s(24));
                    RECT { left: a, right: z, ..b }
                };
                fill(dc, t, pal().sub);
            }
        }
        // ‹ › while the mouse is over the popup, square like the app's buttons
        if hover {
            for (r, glyph, which) in [(g.prev, ICON_PREV, Hot::Prev), (g.next, ICON_NEXT, Hot::Next)] {
                let on = hot == which;
                fill(dc, r, if on { pal().card } else { pal().bg });
                frame(dc, r, 1, if on { pal().sub } else { pal().line });
                text(dc, r, glyph, icon_font, if on { pal().text } else { pal().sub }, DT_CENTER);
            }
        }
    });
    // what this zoom would look sharper with, asked for once it settles
    let gen = peek_job().gen;
    let want = want.filter(|_| steps > 0).map(|z| Sharp { gen, ..z });
    view(|v| v.zwant = want.clone());
    if want.is_some() && want != view(|v| v.zsent.clone()) {
        _ = SetTimer(Some(h), ZOOM_TIMER, 200, None);
    }
}

// Wheel, PgUp / PgDn, Home / End scroll; Shift+wheel sideways (tables, code).
unsafe fn scroll_peek(h: HWND, dy: i32, dx: i32) {
    view(|v| {
        v.y = v.y.saturating_add(dy).max(0);
        v.x = v.x.saturating_add(dx).max(0);
    });
    _ = InvalidateRect(Some(h), None, false);
}

// Zoomed and bigger than the stage: dragging it moves it around.
unsafe fn pannable(h: HWND) -> bool {
    let mut pt = POINT::default();
    _ = GetCursorPos(&mut pt);
    _ = ScreenToClient(h, &mut pt);
    let st = geo_of(h).stage;
    let on_bar = |dpi| zoom_bars(st, view(|v| v.content), dpi).iter().flatten().any(|b| inside(*b, pt.x, pt.y));
    view(|v| v.zoom != 0 && v.canvas != Canvas::None && (v.content.0 > st.right - st.left || v.content.1 > st.bottom - st.top))
        && inside(st, pt.x, pt.y)
        && !on_bar(app(|a| a.dpi))
}

// A press on a scrollbar grabs its thumb (on the track: the thumb jumps there first); on the picture, pans.
unsafe fn press(h: HWND, lp: LPARAM) {
    let (x, y) = ((lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
    let (st, dpi) = (geo_of(h).stage, app(|a| a.dpi));
    let (content, sx, sy, zoomed) = view(|v| (v.content, v.x, v.y, v.zoom != 0 && v.canvas != Canvas::None));
    if !zoomed {
        return;
    }
    let min = 24 * dpi / 96;
    let [vb, hb] = zoom_bars(st, content, dpi);
    let grab = |(a, b): (i32, i32), at: i32| if (a..b).contains(&at) { at - a } else { (b - a) / 2 };
    let drag = match (vb, hb) {
        (Some(b), _) if inside(b, x, y) => Drag::Bar { vertical: true, grab: grab(thumb(b.top, b.bottom, st.bottom - st.top, content.1, sy, min), y) },
        (_, Some(b)) if inside(b, x, y) => Drag::Bar { vertical: false, grab: grab(thumb(b.left, b.right, st.right - st.left, content.0, sx, min), x) },
        _ if pannable(h) => Drag::Pan { at: POINT { x, y }, x: sx, y: sy },
        _ => return,
    };
    view(|v| v.drag = Some(drag));
    SetCapture(h);
    drag_to(h, lp);
}

unsafe fn drag_to(h: HWND, lp: LPARAM) {
    let (x, y) = ((lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
    let (st, dpi) = (geo_of(h).stage, app(|a| a.dpi));
    let (vw, vh) = (st.right - st.left, st.bottom - st.top);
    let min = 24 * dpi / 96;
    let [vb, hb] = view(|v| zoom_bars(st, v.content, dpi));
    view(|v| {
        let (cw, ch) = v.content;
        match v.drag {
            Some(Drag::Pan { at, x: x0, y: y0 }) => {
                v.x = (x0 - (x - at.x)).clamp(0, (cw - vw).max(0));
                v.y = (y0 - (y - at.y)).clamp(0, (ch - vh).max(0));
            }
            Some(Drag::Bar { vertical: true, grab }) => {
                if let Some(b) = vb {
                    v.y = thumb_scroll(b.top, b.bottom, vh, ch, y - grab, min);
                }
            }
            Some(Drag::Bar { vertical: false, grab }) => {
                if let Some(b) = hb {
                    v.x = thumb_scroll(b.left, b.right, vw, cw, x - grab, min);
                }
            }
            None => {}
        }
    });
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) extern "system" fn peek_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let page = || {
            let st = geo_of(h).stage;
            (st.bottom - st.top) * 9 / 10
        };
        let notch = |w: usize| ((w >> 16) as i16 as i32) * app(|a| a.fh) * 3 / 120;
        match msg {
            WM_PAINT => paint_peek(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_PEEK_DONE => {
                start_handler(h, wp.0);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_PEEK_HOSTED => handler_ready(h, wp.0, lp.0 != 0),
            WM_MOUSEMOVE if view(|v| v.drag.is_some()) => drag_to(h, lp),
            WM_LBUTTONDOWN => press(h, lp),
            WM_CAPTURECHANGED => view(|v| v.drag = None),
            WM_SETCURSOR if (lp.0 & 0xffff) as u32 == HTCLIENT && pannable(h) => _ = SetCursor(LoadCursorW(None, IDC_SIZEALL).ok()),
            WM_TIMER if wp.0 == ZOOM_TIMER => want_sharper(h),
            WM_TIMER if wp.0 == SETTLE_TIMER => settle(h),
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_MOUSEMOVE => {
                let hot = peek_hit(&geo_of(h), (lp.0 & 0xffff) as i16 as i32, (lp.0 >> 16 & 0xffff) as i16 as i32);
                let (first, changed) = view(|v| {
                    let r = (!v.hover, !v.hover || v.hot != hot);
                    (v.hover, v.hot) = (true, hot);
                    r
                });
                if first {
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    _ = TrackMouseEvent(&mut t);
                }
                if changed {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_MOUSELEAVE => {
                view(|v| (v.hover, v.hot) = (false, Hot::None));
                _ = InvalidateRect(Some(h), None, false);
            }
            // only × and ‹ › act on a click; the content itself never closes the preview
            WM_LBUTTONUP if view(|v| v.drag.is_some()) => _ = ReleaseCapture(),
            WM_LBUTTONUP => match view(|v| v.hot) {
                Hot::Close => close_peek(h),
                Hot::Prev => step_peek(true),
                Hot::Next => step_peek(false),
                Hot::None => {}
            },
            WM_MOUSEWHEEL if (wp.0 & 0xffff) as u32 & MK_CONTROL.0 != 0 => {
                let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: (lp.0 >> 16 & 0xffff) as i16 as i32 };
                _ = ScreenToClient(h, &mut pt);
                // one step a notch; a touchpad's small deltas add up to one
                let steps = view(|v| {
                    v.zwheel += (wp.0 >> 16) as i16 as i32;
                    let n = v.zwheel / 120;
                    v.zwheel -= n * 120;
                    n
                });
                if steps != 0 {
                    zoom_peek(h, Some(steps), Some(pt));
                }
            }
            WM_MOUSEWHEEL if (wp.0 & 0xffff) as u32 & MK_SHIFT.0 != 0 => scroll_peek(h, 0, -notch(wp.0)),
            WM_MOUSEWHEEL => wheel_peek(h, (wp.0 >> 16) as i16 as i32),
            // tilting the wheel: previous / next file, like the arrows
            WM_MOUSEHWHEEL => step_by_wheel(-((wp.0 >> 16) as i16 as i32)),
            WM_KEYDOWN => match VIRTUAL_KEY(wp.0 as u16) {
                // a held F3 only opens it; its repeats must not close it again
                VK_ESCAPE | VK_SPACE | VK_F3 if lp.0 & (1 << 30) == 0 => close_peek(h),
                VK_DELETE if lp.0 & (1 << 30) == 0 => delete_peek(h, GetKeyState(VK_SHIFT.0 as i32) < 0),
                VK_F2 => start_rename(h),
                k if GetKeyState(VK_CONTROL.0 as i32) < 0 => match k {
                    VK_OEM_PLUS | VK_ADD => zoom_peek(h, Some(1), None),
                    VK_OEM_MINUS | VK_SUBTRACT => zoom_peek(h, Some(-1), None),
                    VK_0 | VK_NUMPAD0 => zoom_peek(h, None, None),
                    _ => {}
                },
                k @ (VK_LEFT | VK_UP | VK_RIGHT | VK_DOWN) => step_peek(k == VK_LEFT || k == VK_UP),
                VK_NEXT => scroll_peek(h, page(), 0),
                VK_PRIOR => scroll_peek(h, -page(), 0),
                VK_HOME => scroll_peek(h, i32::MIN, i32::MIN),
                VK_END => scroll_peek(h, i32::MAX, 0),
                _ => {}
            },
            // another app or a click behind closes it; stepping takes the activation away only for a moment
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_PEEK_LOST, WPARAM(0), LPARAM(0)),
            WM_PEEK_LOST if GetActiveWindow() != h && !view(|v| v.busy) => _ = DestroyWindow(h),
            WM_DESTROY => {
                _ = KillTimer(Some(h), ZOOM_TIMER);
                _ = KillTimer(Some(h), SETTLE_TIMER);
                view(|v| (v.rename, v.drag, v.zoom, v.zwant, v.zsent, v.busy, v.settle) = (None, None, 0, None, None, false, 0));
                unload_handler();
                want_peek(None);
                view(|v| (v.layout, v.y, v.x) = (None, 0, 0));
                app(|a| a.peek = HWND::default());
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}
