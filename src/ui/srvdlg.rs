//! Add / edit server connection.
//! Chips, fields, the HTTPS check, and the footer are painted by this module.
//! A field is one 1px border: the token line at rest, cerulean while that edit has focus.
//! The password stays in the edit and in Credential Manager. It is not written onto the favourite.

use std::sync::atomic::{AtomicIsize, Ordering};

use crate::app::{app, text_of};
use windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi;

use crate::i18n::tr;
use crate::imports::*;
use crate::remote::{self, cred_target, cred_used, fields_from_link, fields_from_parsed, mount_key, normalize, still_used, Fail, Fields, Parsed, KINDS, KIND_LABELS};
use crate::session::{append_node, node_ref, path_clash, write_server, FavNode, ServerLink};
use super::{
    body_font, bold_ui_font, buffered, color_field_edit, field_edit_rect, paint_button, paint_check, paint_field, paint_label, paint_meta, paint_section_header, paint_segment,
    paint_window, style_popup_chrome, visual_from_pointer, ButtonKind, FocusRing, BTN_GAP, BTN_H_FOOTER, FIELD_H, FIELD_PAD_X, FONT_PX, PAD_BODY_X, PAD_BODY_Y, SPACE_4, SPACE_8,
    px,
};

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
    edits: [HWND; 6],
    hot: i32,
    down: i32,
    kind_r: [RECT; 4],
    https_r: RECT,
    save_r: RECT,
    cancel_r: RECT,
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
    let mut dlg = Dlg {
        dpi,
        kind: 0,
        tls: false,
        tls_user: false,
        editing: false,
        edit_id: Vec::new(),
        parent: parent.to_vec(),
        old: None,
        edits: [HWND::default(); 6],
        hot: -1,
        down: -1,
        kind_r: [RECT::default(); 4],
        https_r: RECT::default(),
        save_r: RECT::default(),
        cancel_r: RECT::default(),
    };
    if let Some(id) = edit {
        let loaded = app(|a| node_ref(&a.s.tree, id).and_then(|n| n.server.clone().map(|s| (n.name.clone(), s))));
        let Some((name, link)) = loaded else {
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
    let client_w = sc(dpi, 400);
    let client_h = client_height(dpi, client_w);
    let (w, h) = outer_size(client_w, client_h, dpi);
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
        return;
    };
    let ptr = Box::into_raw(Box::new(dlg));
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
    let font = body_font();
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
    px(dpi, dip)
}

fn geom(dpi: i32, width: i32) -> Geom {
    let pad = sc(dpi, PAD_BODY_X);
    let gap = sc(dpi, SPACE_8);
    let gap4 = sc(dpi, SPACE_4);
    let lab = sc(dpi, FONT_PX + SPACE_4);
    let edit_h = sc(dpi, FIELD_H);
    let chip_h = sc(dpi, BTN_H_FOOTER);
    let mut y = pad + lab + gap + lab + gap4;
    let inner = (width - pad * 2).max(1);
    let slot = (inner - gap * 3) / 4;
    let mut kind_r = [RECT::default(); 4];
    for i in 0..4 {
        let left = pad + i as i32 * (slot + gap);
        kind_r[i] = RECT { left, top: y, right: left + slot, bottom: y + chip_h };
    }
    y += chip_h + gap;
    let mut edits = [RECT::default(); 6];
    for i in 0..6 {
        y += lab + gap4;
        edits[i] = RECT { left: pad, top: y, right: (width - pad).max(pad + 1), bottom: y + edit_h };
        y += edit_h + gap;
    }
    let https_r = RECT { left: pad, top: y, right: (width - pad).max(pad + 1), bottom: y + lab };
    y += lab + gap + lab + gap;
    let bw = (inner - sc(dpi, BTN_GAP)) / 2;
    let bh = sc(dpi, BTN_H_FOOTER);
    let save_r = RECT { left: pad, top: y, right: pad + bw, bottom: y + bh };
    let cancel_r = RECT { left: save_r.right + sc(dpi, BTN_GAP), top: y, right: (width - pad).max(save_r.right + 1), bottom: y + bh };
    Geom { kind_r, edits, https_r, save_r, cancel_r }
}

fn client_height(dpi: i32, width: i32) -> i32 {
    geom(dpi, width).save_r.bottom + sc(dpi, PAD_BODY_Y)
}

unsafe fn outer_size(client_w: i32, client_h: i32, dpi: i32) -> (i32, i32) {
    let mut r = RECT { left: 0, top: 0, right: client_w, bottom: client_h };
    let style = WS_POPUP | WS_CLIPCHILDREN;
    if AdjustWindowRectExForDpi(&mut r, style, false, WS_EX_DLGMODALFRAME, dpi.max(96) as u32).is_ok() {
        ((r.right - r.left).max(client_w), (r.bottom - r.top).max(client_h))
    } else {
        (client_w, client_h)
    }
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
    let m = sc(dpi, FIELD_PAD_X) as isize;
    for (i, h) in (*v).edits.iter().enumerate() {
        if h.0.is_null() {
            continue;
        }
        _ = SendMessageW(*h, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(m | (m << 16))));
        crate::place_edit(*h, field_edit_rect(g.edits[i], dpi), true);
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
            if view(h).is_none() {
                return DefWindowProcW(h, msg, wp, lp);
            }
            color_field_edit(dc)
        }
        WM_COMMAND => {
            let code = ((wp.0 >> 16) & 0xffff) as u32;
            if code == 0x0100 || code == 0x0200 {
                let _ = InvalidateRect(Some(h), None, false);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            paint(h);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            hit(h, lp, false, false);
            arm_leave(h);
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(v) = view(h) {
                (*v).hot = -1;
                let _ = InvalidateRect(Some(h), None, false);
            }
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
        WM_CAPTURECHANGED => {
            // `hit` clears `down` before ReleaseCapture, so a finished click does not land here.
            // A stolen capture still drops the press and the hover.
            if let Some(v) = view(h) {
                if (*v).down >= 0 {
                    (*v).down = -1;
                    (*v).hot = -1;
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
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
                // The painted title band. Dragging it moves the popup; the chips sit below it.
                if y < sc(dpi_of(h), PAD_BODY_X + FONT_PX + SPACE_4) {
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
        let pad = sc(dpi, PAD_BODY_X);
        let g = geom(dpi, (rc.right - rc.left).max(1));
        v.kind_r = g.kind_r;
        v.https_r = g.https_r;
        v.save_r = g.save_r;
        v.cancel_r = g.cancel_r;
        paint_window(dc, rc);
        let body = body_font();
        let bold = bold_ui_font();
        let ground = super::ui_tokens().bg;
        let title = if v.editing { tr(crate::Key::EditServerConnection) } else { tr(crate::Key::AddServerConnection) };
        let title_r = RECT { left: pad, top: pad, right: rc.right - pad, bottom: pad + sc(dpi, FONT_PX + SPACE_4) };
        paint_section_header(dc, title_r, title, bold);
        let y = g.kind_r[0].top - sc(dpi, SPACE_4) - sc(dpi, FONT_PX + SPACE_4);
        let type_r = RECT { left: pad, top: y, right: rc.right - pad, bottom: y + sc(dpi, FONT_PX + SPACE_4) };
        paint_meta(dc, type_r, tr(crate::Key::ServerType), body);
        let hot = (v.hot >= 0).then_some(v.hot);
        let down = (v.down >= 0).then_some(v.down);
        let focus = GetFocus();
        for i in 0..4 {
            let vis = visual_from_pointer(i as i32, hot, down);
            paint_segment(dc, g.kind_r[i], KIND_LABELS[i], v.kind == i, vis, dpi, ground);
        }
        let names = [tr(crate::Key::DisplayName), tr(crate::Key::Address), tr(crate::Key::Port), tr(crate::Key::RemotePath), tr(crate::Key::UserName), tr(crate::Key::Password)];
        for (i, name) in names.iter().enumerate() {
            let r = g.edits[i];
            let focused = !v.edits[i].0.is_null() && focus == v.edits[i];
            paint_field(dc, r, dpi, focused);
            let lab = RECT { left: pad, top: r.top - sc(dpi, SPACE_4) - sc(dpi, FONT_PX + SPACE_4), right: rc.right - pad, bottom: r.top - sc(dpi, SPACE_4) };
            paint_meta(dc, lab, name, body);
        }
        if KINDS[v.kind] == "webdav" {
            let box_s = sc(dpi, 16);
            let mark = RECT { left: g.https_r.left, top: g.https_r.top, right: g.https_r.left + box_s, bottom: g.https_r.top + box_s };
            paint_check(dc, mark, v.tls, v.hot == 9, dpi, ground);
            let lab = RECT { left: mark.right + sc(dpi, SPACE_8), top: g.https_r.top, right: rc.right - pad, bottom: g.https_r.bottom };
            paint_label(dc, lab, tr(crate::Key::Https), body);
        }
        if v.editing {
            let hint = RECT { left: pad, top: g.https_r.bottom + sc(dpi, SPACE_8), right: rc.right - pad, bottom: g.save_r.top - sc(dpi, SPACE_8) };
            paint_meta(dc, hint, tr(crate::Key::LeaveBlankToKeepTheSavedPassword), body);
        }
        paint_button(dc, g.save_r, ButtonKind::Primary, visual_from_pointer(10, hot, down), tr(crate::Key::Save), bold, dpi, ground, FocusRing::Outside);
        paint_button(dc, g.cancel_r, ButtonKind::Secondary, visual_from_pointer(11, hot, down), tr(crate::Key::Cancel), body, dpi, ground, FocusRing::Outside);
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

unsafe fn arm_leave(h: HWND) {
    let mut t = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE,
        hwndTrack: h,
        dwHoverTime: 0,
    };
    let _ = TrackMouseEvent(&mut t);
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
    unsafe { style_popup_chrome(hwnd) }
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
}

#[cfg(test)]
mod tests {
    #[test]
    fn server_dialog_uses_shared_field_chip_and_footer_metrics() {
        let g = super::geom(96, 400);
        assert_eq!(g.kind_r[0].bottom - g.kind_r[0].top, crate::BTN_H_FOOTER);
        assert_eq!(g.kind_r[1].left - g.kind_r[0].right, crate::SPACE_8);
        assert_eq!(g.edits[0].bottom - g.edits[0].top, crate::FIELD_H);
        assert_eq!(g.edits[0].top - g.kind_r[0].bottom, crate::SPACE_8 + (crate::FONT_PX + crate::SPACE_4) + crate::SPACE_4);
        assert_eq!(g.save_r.bottom - g.save_r.top, crate::BTN_H_FOOTER);
        assert_eq!(g.cancel_r.left - g.save_r.right, crate::BTN_GAP);
        assert_eq!(g.kind_r[0].left, crate::PAD_BODY_X);
        assert!(g.kind_r[3].right <= 400 - crate::PAD_BODY_X);
        assert_eq!(super::client_height(96, 400), g.save_r.bottom + crate::PAD_BODY_Y);
    }

    #[test]
    fn server_dialog_metrics_scale_with_dpi() {
        let dpi = 144;
        let width = super::sc(dpi, 400);
        let g = super::geom(dpi, width);
        assert_eq!(g.edits[0].bottom - g.edits[0].top, super::sc(dpi, crate::FIELD_H));
        assert_eq!(g.kind_r[0].bottom - g.kind_r[0].top, super::sc(dpi, crate::BTN_H_FOOTER));
        assert_eq!(g.kind_r[1].left - g.kind_r[0].right, super::sc(dpi, crate::SPACE_8));
        assert_eq!(g.save_r.bottom - g.save_r.top, super::sc(dpi, crate::BTN_H_FOOTER));
        assert_eq!(g.cancel_r.left - g.save_r.right, super::sc(dpi, crate::BTN_GAP));
    }
}
