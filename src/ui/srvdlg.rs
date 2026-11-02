//! Add / edit server connection. Owner-drawn card, standard edit fields, bold accent Save.
//! The password stays in the edit and in Credential Manager. It is not written onto the favourite.

use std::sync::atomic::{AtomicIsize, Ordering};

use crate::app::{app, text_of};
use crate::i18n::{title_style, tr, ui_face};
use crate::imports::*;
use crate::remote::{self, cred_target, cred_used, fields_from_link, fields_from_parsed, mount_key, normalize, still_used, Fail, Fields, Parsed, KINDS, KIND_LABELS};
use crate::session::{append_node, node_ref, path_clash, write_server, FavNode, ServerLink};
use super::{aa_rfill, blend, buffered, cref, fill, on_color, pal, text};

const SAVE_MSG: u32 = WM_APP + 1;
/// EC_LEFTMARGIN | EC_RIGHTMARGIN. Same message the settings search box and name prompt use.
const EM_SETMARGINS: u32 = 0x00D3;
const ID_NAME: i32 = 10;
const ID_HOST: i32 = 11;
const ID_PORT: i32 = 12;
const ID_REMOTE: i32 = 13;
const ID_USER: i32 = 14;
const ID_PASS: i32 = 15;

static EDIT_PROC: AtomicIsize = AtomicIsize::new(0);

struct Dlg {
    dpi: i32,
    kind: usize,
    tls: bool,
    tls_user: bool,
    editing: bool,
    edit_id: Vec<usize>,
    parent: Vec<usize>,
    old: Option<ServerLink>,
    font: HFONT,
    bold: HFONT,
    edits: [HWND; 6],
    hot: i32,
    down: i32,
    kind_r: [RECT; 4],
    https_r: RECT,
    save_r: RECT,
    cancel_r: RECT,
    brush: HBRUSH,
    accent: u32,
}

struct Geom {
    kind_r: [RECT; 4],
    edits: [RECT; 6],
    https_r: RECT,
    save_r: RECT,
    cancel_r: RECT,
}

pub(crate) unsafe fn open_server_dialog(owner: HWND, parent: &[usize], edit: Option<&[usize]>, prefill: Option<Parsed>) {
    let dpi = dpi_of(owner);
    let (face, weight) = title_style();
    let body = ui_face();
    let accent = app(|a| a.s.accent_now());
    let mut dlg = Dlg {
        dpi,
        kind: 0,
        tls: false,
        tls_user: false,
        editing: false,
        edit_id: Vec::new(),
        parent: parent.to_vec(),
        old: None,
        font: make_font(body, 14, 400, dpi),
        bold: make_font(face, 14, weight, dpi),
        edits: [HWND::default(); 6],
        hot: -1,
        down: -1,
        kind_r: [RECT::default(); 4],
        https_r: RECT::default(),
        save_r: RECT::default(),
        cancel_r: RECT::default(),
        brush: CreateSolidBrush(cref(pal().card)),
        accent,
    };
    if let Some(id) = edit {
        let loaded = app(|a| node_ref(&a.s.tree, id).and_then(|n| n.server.clone().map(|s| (n.name.clone(), s))));
        let Some((name, link)) = loaded else {
            drop_fonts(&dlg);
            return;
        };
        dlg.editing = true;
        dlg.edit_id = id.to_vec();
        dlg.old = Some(link.clone());
        dlg.tls_user = true;
        apply_fields(&mut dlg, &fields_from_link(&link, &name));
    } else if let Some(p) = prefill.as_ref() {
        dlg.tls_user = true;
        apply_fields(&mut dlg, &fields_from_parsed(p));
    }
    ensure_class();
    let w = sc(dpi, 400);
    let h = sc(dpi, 560);
    let (x, y) = center_on(owner, w, h);
    let title = if dlg.editing { tr(crate::Key::EditServerConnection) } else { tr(crate::Key::AddServerConnection) };
    let hwnd = CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        w!("MaguroServer"),
        &HSTRING::from(title),
        WS_POPUP | WS_CLIPCHILDREN,
        x,
        y,
        w,
        h,
        Some(owner),
        None,
        None,
        None,
    );
    let Ok(hwnd) = hwnd else {
        drop_fonts(&dlg);
        return;
    };
    let ptr = Box::into_raw(Box::new(dlg));
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
    let font = (*ptr).font;
    let ids = [ID_NAME, ID_HOST, ID_PORT, ID_REMOTE, ID_USER, ID_PASS];
    for (i, id) in ids.iter().enumerate() {
        // Borderless. The dialog paints the frame; a WS_BORDER edit sized to that frame
        // draws its glyphs from the top of the client.
        let mut style = 0x80u32 | 0x10000; // ES_AUTOHSCROLL | WS_TABSTOP
        if *id == ID_PASS {
            style |= 0x20; // ES_PASSWORD
        }
        if *id == ID_PORT {
            style |= 0x2000; // ES_NUMBER
        }
        let h = crate::child(hwnd, w!("EDIT"), w!(""), style, WINDOW_EX_STYLE(0), *id, font);
        (*ptr).edits[i] = h;
        subclass_edit(h);
    }
    if let Some(p) = prefill.as_ref() {
        if !p.password.is_empty() {
            _ = SetWindowTextW((*ptr).edits[5], &HSTRING::from(&p.password));
        }
    }
    fill_edits(ptr);
    place_edits(hwnd);
    round_window(hwnd);
    let was_disabled = EnableWindow(owner, false).as_bool();
    _ = ShowWindow(hwnd, SW_SHOW);
    _ = SetForegroundWindow(hwnd);
    if !(*ptr).edits[1].0.is_null() {
        _ = SetFocus(Some((*ptr).edits[1]));
    }
    let mut msg = MSG::default();
    loop {
        let r = GetMessageW(&mut msg, None, 0, 0);
        if !r.as_bool() {
            if msg.message == WM_QUIT {
                PostQuitMessage(msg.wParam.0 as i32);
            }
            break;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
        if !IsWindow(Some(hwnd)).as_bool() {
            break;
        }
    }
    if !was_disabled {
        let _ = EnableWindow(owner, true);
    }
    _ = SetForegroundWindow(owner);
}

fn apply_fields(dlg: &mut Dlg, f: &Fields) {
    dlg.kind = KINDS.iter().position(|k| *k == f.kind).unwrap_or(0);
    dlg.tls = f.tls;
}

unsafe fn fill_edits(ptr: *mut Dlg) {
    let Some(link) = (*ptr).old.clone() else { return };
    let name = app(|a| node_ref(&a.s.tree, &(*ptr).edit_id).map(|n| n.name.clone())).unwrap_or_default();
    let f = fields_from_link(&link, &name);
    let values = [f.name, f.host, f.port_text, f.remote, f.user];
    for (i, v) in values.iter().enumerate() {
        _ = SetWindowTextW((*ptr).edits[i], &HSTRING::from(v));
    }
}

fn sc(dpi: i32, dip: i32) -> i32 {
    (dip * dpi / 96).max(if dip == 0 { 0 } else { 1 })
}

/// Inset of a single-line edit inside its painted frame. Equal on every side,
/// so the font cell `place_edit` centres in here is also centred in the frame.
/// Same 2 DIP inset as the rename dialog's `field_inner` and the settings ask field.
fn field_inner(outer: RECT, dpi: i32) -> RECT {
    let h = outer.bottom - outer.top;
    let w = outer.right - outer.left;
    if h <= 0 || w <= 0 {
        return outer;
    }
    let d = sc(dpi.max(96), 2).min(h / 2).min(w / 2);
    RECT { left: outer.left + d, top: outer.top + d, right: (outer.right - d).max(outer.left + d), bottom: (outer.bottom - d).max(outer.top + d) }
}

fn geom(dpi: i32, width: i32) -> Geom {
    let pad = sc(dpi, 16);
    let gap = sc(dpi, 8);
    let gap4 = sc(dpi, 4);
    let lab = sc(dpi, 16);
    let edit_h = sc(dpi, 32);
    let mut y = pad + lab + gap + lab + gap4;
    let inner = width - pad * 2;
    let slot = (inner - gap * 3) / 4;
    let mut kind_r = [RECT::default(); 4];
    for i in 0..4 {
        let left = pad + i as i32 * (slot + gap);
        kind_r[i] = RECT { left, top: y, right: left + slot, bottom: y + sc(dpi, 32) };
    }
    y += sc(dpi, 32) + gap;
    let mut edits = [RECT::default(); 6];
    for i in 0..6 {
        y += lab + gap4;
        edits[i] = RECT { left: pad, top: y, right: width - pad, bottom: y + edit_h };
        y += edit_h + gap;
    }
    let https_r = RECT { left: pad, top: y, right: width - pad, bottom: y + lab };
    y += lab + gap + lab + gap;
    let bw = (inner - gap) / 2;
    let save_r = RECT { left: pad, top: y, right: pad + bw, bottom: y + sc(dpi, 32) };
    let cancel_r = RECT { left: save_r.right + gap, top: y, right: width - pad, bottom: y + sc(dpi, 32) };
    Geom { kind_r, edits, https_r, save_r, cancel_r }
}

unsafe fn place_edits(hwnd: HWND) {
    let Some(v) = view(hwnd) else { return };
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let dpi = (*v).dpi;
    let g = geom(dpi, (rc.right - rc.left).max(1));
    (*v).kind_r = g.kind_r;
    (*v).https_r = g.https_r;
    (*v).save_r = g.save_r;
    (*v).cancel_r = g.cancel_r;
    // 8 DIP matches the settings search box and the name prompt. The 2 DIP frame inset
    // is outside this margin, so the glyphs sit clearly off the left edge.
    let m = sc(dpi, 8) as isize;
    for (i, h) in (*v).edits.iter().enumerate() {
        if h.0.is_null() {
            continue;
        }
        _ = SendMessageW(*h, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(m | (m << 16))));
        crate::place_edit(*h, field_inner(g.edits[i], dpi), true);
    }
}

unsafe extern "system" fn srv_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_SIZE => {
            place_edits(h);
            LRESULT(0)
        }
        WM_CLOSE => {
            _ = DestroyWindow(h);
            LRESULT(0)
        }
        WM_DESTROY => {
            drop_view(h);
            LRESULT(0)
        }
        SAVE_MSG => {
            commit(h);
            LRESULT(0)
        }
        WM_CTLCOLOREDIT => {
            let dc = HDC(wp.0 as *mut _);
            let Some(v) = view(h) else { return DefWindowProcW(h, msg, wp, lp) };
            SetTextColor(dc, cref(pal().text));
            SetBkColor(dc, cref(pal().card));
            LRESULT((*v).brush.0 as isize)
        }
        WM_PAINT => {
            paint(h);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            hit(h, lp, false, false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            hit(h, lp, true, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            hit(h, lp, false, true);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            _ = DestroyWindow(h);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_RETURN.0 as usize => {
            commit(h);
            LRESULT(0)
        }
        WM_NCHITTEST => {
            let hit = DefWindowProcW(h, msg, wp, lp);
            if hit.0 == HTCLIENT as isize {
                let (_x, y) = lp_xy(lp);
                if y < sc(dpi_of(h), 32) {
                    return LRESULT(HTCAPTION as isize);
                }
            }
            hit
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

fn view(hwnd: HWND) -> Option<*mut Dlg> {
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Dlg;
    if p.is_null() { None } else { Some(p) }
}

unsafe fn paint(h: HWND) {
    let Some(v) = view(h) else { return };
    place_edits(h);
    let v = &mut *v;
    buffered(h, |dc, rc| {
        let dpi = v.dpi;
        let pad = sc(dpi, 16);
        let g = geom(dpi, (rc.right - rc.left).max(1));
        v.kind_r = g.kind_r;
        v.https_r = g.https_r;
        v.save_r = g.save_r;
        v.cancel_r = g.cancel_r;
        fill(dc, rc, pal().bg);
        let title = if v.editing { tr(crate::Key::EditServerConnection) } else { tr(crate::Key::AddServerConnection) };
        let title_r = RECT { left: pad, top: pad, right: rc.right - pad, bottom: pad + sc(dpi, 16) };
        text(dc, title_r, title, v.bold, pal().text, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        let y = g.kind_r[0].top - sc(dpi, 4) - sc(dpi, 16);
        let type_r = RECT { left: pad, top: y, right: rc.right - pad, bottom: y + sc(dpi, 16) };
        text(dc, type_r, tr(crate::Key::ServerType), v.font, pal().sub, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        for i in 0..4 {
            let on = v.kind == i;
            let hot = v.hot == i as i32;
            let fill_c = if on { v.accent } else if hot { blend(pal().card, pal().text, 0.06) } else { pal().card };
            let border = if on { None } else { Some(pal().line) };
            aa_rfill(dc, g.kind_r[i], sc(dpi, 8), fill_c, border, pal().bg);
            let ink = if on { on_color(v.accent) } else { pal().text };
            let font = if on { v.bold } else { v.font };
            text(dc, g.kind_r[i], KIND_LABELS[i], font, ink, DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        }
        let names = [tr(crate::Key::DisplayName), tr(crate::Key::Address), tr(crate::Key::Port), tr(crate::Key::RemotePath), tr(crate::Key::UserName), tr(crate::Key::Password)];
        for (i, name) in names.iter().enumerate() {
            let r = g.edits[i];
            // The frame stays this rect. The borderless edit sits inside `field_inner`.
            aa_rfill(dc, r, sc(dpi, 4), pal().card, Some(pal().line), pal().bg);
            let lab = RECT { left: pad, top: r.top - sc(dpi, 4) - sc(dpi, 16), right: rc.right - pad, bottom: r.top - sc(dpi, 4) };
            text(dc, lab, name, v.font, pal().sub, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        }
        if KINDS[v.kind] == "webdav" {
            let mark = RECT { left: g.https_r.left, top: g.https_r.top, right: g.https_r.left + sc(dpi, 16), bottom: g.https_r.top + sc(dpi, 16) };
            let fill_c = if v.tls { v.accent } else { pal().card };
            aa_rfill(dc, mark, sc(dpi, 4), fill_c, Some(if v.tls { v.accent } else { pal().line }), pal().bg);
            if v.tls {
                text(dc, mark, "✓", v.font, on_color(v.accent), DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
            }
            let lab = RECT { left: mark.right + sc(dpi, 8), top: g.https_r.top, right: rc.right - pad, bottom: g.https_r.bottom };
            text(dc, lab, tr(crate::Key::Https), v.font, pal().text, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        }
        if v.editing {
            let hint = RECT { left: pad, top: g.https_r.bottom + sc(dpi, 8), right: rc.right - pad, bottom: g.save_r.top - sc(dpi, 8) };
            text(dc, hint, tr(crate::Key::LeaveBlankToKeepTheSavedPassword), v.font, pal().sub, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX);
        }
        let save_hot = v.hot == 10;
        let save_fill = if save_hot { blend(v.accent, 0xFFFFFF, 0.12) } else { v.accent };
        aa_rfill(dc, g.save_r, sc(dpi, 8), save_fill, None, pal().bg);
        text(dc, g.save_r, tr(crate::Key::Save), v.bold, on_color(v.accent), DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        let cancel_hot = v.hot == 11;
        let cancel_fill = if cancel_hot { blend(pal().card, pal().text, 0.06) } else { pal().card };
        aa_rfill(dc, g.cancel_r, sc(dpi, 8), cancel_fill, Some(pal().line), pal().bg);
        text(dc, g.cancel_r, tr(crate::Key::Cancel), v.font, pal().text, DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
    });
}

fn hit(h: HWND, lp: LPARAM, down: bool, up: bool) {
    let (x, y) = lp_xy(lp);
    let Some(v) = view(h) else { return };
    unsafe {
        let mut which = -1;
        for (i, r) in (*v).kind_r.iter().enumerate() {
            if inside(*r, x, y) {
                which = i as i32;
            }
        }
        if KINDS[(*v).kind] == "webdav" && inside((*v).https_r, x, y) {
            which = 9;
        }
        if inside((*v).save_r, x, y) {
            which = 10;
        }
        if inside((*v).cancel_r, x, y) {
            which = 11;
        }
        (*v).hot = which;
        if down {
            (*v).down = which;
            if which >= 0 {
                let _ = SetCapture(h);
            }
        }
        if up {
            let fire = (*v).down == which && which >= 0;
            (*v).down = -1;
            let _ = ReleaseCapture();
            if fire {
                if which < 4 {
                    let next = which as usize;
                    if (*v).kind != next {
                        (*v).kind = next;
                        if KINDS[next] == "webdav" && !(*v).tls_user {
                            (*v).tls = true;
                        }
                    }
                } else if which == 9 {
                    (*v).tls = !(*v).tls;
                    (*v).tls_user = true;
                } else if which == 10 {
                    commit(h);
                } else if which == 11 {
                    _ = DestroyWindow(h);
                }
            }
        }
        let _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn commit(h: HWND) {
    let Some(v) = view(h) else { return };
    let kind = KINDS[(*v).kind].to_string();
    let name = text_of((*v).edits[0]);
    let host = text_of((*v).edits[1]);
    let port_text = text_of((*v).edits[2]);
    let remote = text_of((*v).edits[3]);
    let user = text_of((*v).edits[4]);
    let mut pass_units = read_units((*v).edits[5]);
    let mut pass = String::from_utf16_lossy(&pass_units);
    let tls = kind == "webdav" && (*v).tls;
    let fields = Fields { kind, name, host, port_text, remote, user, tls };
    let built = match normalize(&fields) {
        Ok(b) => b,
        Err(Fail::Port) => {
            wipe(&mut pass);
            pass_units.fill(0);
            _ = MessageBeep(MB_ICONWARNING);
            _ = SetFocus(Some((*v).edits[2]));
            return;
        }
        Err(Fail::Host) => {
            wipe(&mut pass);
            pass_units.fill(0);
            let q = tr(crate::Key::AddressIsRequired);
            let _ = MessageBoxW(Some(h), &HSTRING::from(q), w!("Maguro"), MB_ICONWARNING);
            _ = SetFocus(Some((*v).edits[1]));
            return;
        }
    };
    let editing = (*v).editing;
    let edit_id = (*v).edit_id.clone();
    let parent = (*v).parent.clone();
    let old = (*v).old.clone();
    let skip = if editing { edit_id.clone() } else { Vec::new() };
    let clash = app(|a| path_clash(&a.s.tree, &skip, &built.path));
    if clash {
        wipe(&mut pass);
        pass_units.fill(0);
        let q = tr(crate::Key::AlreadyExists);
        let _ = MessageBoxW(Some(h), &HSTRING::from(q), w!("Maguro"), MB_ICONWARNING);
        return;
    }
    if !store_secret(old.as_ref(), &built.link, &pass) {
        wipe(&mut pass);
        pass_units.fill(0);
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    wipe(&mut pass);
    pass_units.fill(0);
    _ = SetWindowTextW((*v).edits[5], w!(""));
    let saved = app(|a| {
        if editing {
            write_server(&mut a.s.tree, &edit_id, built.name.clone(), built.path.clone(), built.link.clone())
        } else {
            let node = FavNode::server_leaf(&built.name, &built.path, built.link.clone());
            append_node(&mut a.s.tree, &parent, node)
        }
    });
    if !saved {
        let q = tr(crate::Key::AlreadyExists);
        let _ = MessageBoxW(Some(h), &HSTRING::from(q), w!("Maguro"), MB_ICONWARNING);
        return;
    }
    app(|a| a.s.sync_favs());
    if let Some(old) = old.as_ref() {
        if cred_target(old) != cred_target(&built.link) {
            let drop_old = !app(|a| cred_used(&a.s.tree, old));
            if drop_old {
                remote::delete_secret(&cred_target(old));
            }
        }
        if mount_key(old) != mount_key(&built.link) && !app(|a| still_used(&a.s.tree, old)) {
            remote::unmount_link(old);
        }
    }
    crate::commit_favs();
    _ = DestroyWindow(h);
}

fn store_secret(old: Option<&ServerLink>, new_link: &ServerLink, pass: &str) -> bool {
    let new_t = cred_target(new_link);
    if !pass.is_empty() {
        return remote::write_secret(&new_t, pass);
    }
    let Some(old) = old else { return true };
    let old_t = cred_target(old);
    if old_t == new_t {
        return true;
    }
    let Some(mut secret) = remote::read_secret(&old_t) else { return true };
    let ok = remote::write_secret(&new_t, &secret);
    wipe(&mut secret);
    ok
}

unsafe fn read_units(h: HWND) -> Vec<u16> {
    let n = GetWindowTextLengthW(h);
    if n <= 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = GetWindowTextW(h, &mut buf[..]);
    if got <= 0 {
        buf.fill(0);
        return Vec::new();
    }
    buf.truncate(got as usize);
    buf
}

fn wipe(s: &mut String) {
    let mut bytes = std::mem::take(s).into_bytes();
    bytes.fill(0);
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn lp_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

unsafe fn subclass_edit(h: HWND) {
    if h.0.is_null() {
        return;
    }
    let prev = SetWindowLongPtrW(h, GWLP_WNDPROC, edit_proc as *const () as usize as isize);
    if prev != 0 && EDIT_PROC.load(Ordering::Relaxed) == 0 {
        EDIT_PROC.store(prev, Ordering::Relaxed);
    }
}

unsafe extern "system" fn edit_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_KEYDOWN && (wp.0 == VK_RETURN.0 as usize || wp.0 == VK_ESCAPE.0 as usize) {
        if let Ok(parent) = GetParent(h) {
            if wp.0 == VK_ESCAPE.0 as usize {
                let _ = PostMessageW(Some(parent), WM_CLOSE, WPARAM(0), LPARAM(0));
            } else {
                let _ = PostMessageW(Some(parent), SAVE_MSG, WPARAM(0), LPARAM(0));
            }
        }
        return LRESULT(0);
    }
    let old = EDIT_PROC.load(Ordering::Relaxed);
    if old == 0 {
        return DefWindowProcW(h, msg, wp, lp);
    }
    CallWindowProcW(std::mem::transmute::<isize, WNDPROC>(old), h, msg, wp, lp)
}

fn ensure_class() {
    unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let mut existing = WNDCLASSW::default();
        if GetClassInfoW(Some(hinst), w!("MaguroServer"), &mut existing).is_ok() {
            return;
        }
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW,
            lpfnWndProc: Some(srv_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(crate::ICON_GROUP_LIGHT as _)).unwrap_or_default(),
            lpszClassName: w!("MaguroServer"),
            ..Default::default()
        });
    }
}

fn dpi_of(owner: HWND) -> i32 {
    let dpi = unsafe { GetDpiForWindow(owner) as i32 };
    if dpi > 0 { dpi } else { 96 }
}

fn center_on(owner: HWND, w: i32, h: i32) -> (i32, i32) {
    let mut r = RECT::default();
    unsafe {
        if GetWindowRect(owner, &mut r).is_ok() && r.right > r.left {
            return ((r.left + r.right - w) / 2, (r.top + r.bottom - h) / 2);
        }
    }
    (80, 80)
}

fn round_window(hwnd: HWND) {
    unsafe {
        let pref: u32 = 2;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as _, 4);
        let dark = crate::DARK_ON.load(Ordering::Relaxed);
        let on = BOOL::from(dark);
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
        let border = cref(pal().bg);
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as _, 4);
    }
}

fn make_font(face: &str, dip: i32, weight: i32, dpi: i32) -> HFONT {
    let mut name = [0u16; 32];
    for (i, u) in face.encode_utf16().take(31).enumerate() {
        name[i] = u;
    }
    let lf = LOGFONTW { lfHeight: -(dip * dpi / 96).max(1), lfWeight: weight, lfFaceName: name, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
    unsafe { CreateFontIndirectW(&lf) }
}

fn drop_fonts(dlg: &Dlg) {
    unsafe {
        if !dlg.font.0.is_null() {
            let _ = DeleteObject(dlg.font.into());
        }
        if !dlg.bold.0.is_null() {
            let _ = DeleteObject(dlg.bold.into());
        }
        if !dlg.brush.0.is_null() {
            let _ = DeleteObject(dlg.brush.into());
        }
    }
}

unsafe fn drop_view(h: HWND) {
    let p = GetWindowLongPtrW(h, GWLP_USERDATA) as *mut Dlg;
    SetWindowLongPtrW(h, GWLP_USERDATA, 0);
    if p.is_null() {
        return;
    }
    let dlg = Box::from_raw(p);
    if !dlg.edits[5].0.is_null() {
        _ = SetWindowTextW(dlg.edits[5], w!(""));
    }
    drop_fonts(&dlg);
}
