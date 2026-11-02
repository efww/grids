use crate::*;
use crate::rename_rules::*;
use std::collections::HashSet;
use std::os::windows::fs::MetadataExt;
use windows::Win32::Storage::FileSystem::{CreateFileW, DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Controls::{
    LVCOLUMNW, LVCF_TEXT, LVCF_WIDTH, LVIF_TEXT, LVM_INSERTCOLUMNW, LVM_SETITEMCOUNT, LVN_GETDISPINFOW, LVS_EX_DOUBLEBUFFER, LVS_EX_FULLROWSELECT,
    LVS_NOSORTHEADER, LVS_OWNERDATA, LVS_REPORT, LVS_SHOWSELALWAYS, MEASUREITEMSTRUCT, NMLVDISPINFOW, ODS_DISABLED, ODS_FOCUS, ODT_LISTBOX,
    CDRF_NEWFONT, EM_SETCUEBANNER,
};

// ---------- batch rename (Ctrl+Shift+R): rule stack, live preview, two-phase rename, undo, presets ----------
// The names come from rename_rules.rs; this is the window and the file system part.

const ID_RULES: i32 = 3000;
const ID_ADD: i32 = 3001;
const ID_DEL: i32 = 3002;
const ID_RUP: i32 = 3003;
const ID_RDOWN: i32 = 3004;
const ID_ONOFF: i32 = 3005;
const ID_F0: i32 = 3010; // + 0..3: the rule's text fields
const ID_MODE: i32 = 3014;
const ID_FX: i32 = 3015;
const ID_FY: i32 = 3016;
const ID_LIST: i32 = 3020;
const ID_EXTOK: i32 = 3021;
const ID_PNAME: i32 = 3022;
const ID_PSAVE: i32 = 3023;
const ID_PMENU: i32 = 3024;
const ID_UNDO: i32 = 3025;
const ID_GO: i32 = 3026;
const ID_CLOSE: i32 = 3027;
// popup menu picks (TrackPopupMenu returns them, nothing is sent)
const MENU_ADD: usize = 1120; // + kind
const MENU_MODE: usize = 1127; // + mode
const MENU_LOAD: usize = 3050; // + preset
const MENU_FORGET: usize = 3075; // + preset
const PRESETS_MAX: usize = 25;

struct Rn {
    h: HWND,
    pane: usize,
    rules: Vec<Rule>, // kept after the window closes, for the next batch
    cur: usize,
    ext: bool,
    dirs: Vec<String>,
    items: Vec<(usize, Src)>,       // folder index, file
    existing: Vec<HashSet<String>>, // per folder: lowercased names there now
    rows: Vec<(String, Status)>,    // new name, status, per item
    err: Option<String>,            // regular expression that doesn't compile
    note: String,                   // last result
    undo: Vec<(String, String, String)>, // last batch: folder, old name, new name
    loading: bool,                  // fields being filled from a rule: their EN_CHANGE is not an edit
    focus: HWND,
}

thread_local! {
    static RN: RefCell<Rn> = RefCell::new(Rn {
        h: HWND::default(), pane: 0, rules: vec![Rule::new(Kind::Replace)], cur: 0, ext: false, dirs: Vec::new(), items: Vec::new(),
        existing: Vec::new(), rows: Vec::new(), err: None, note: String::new(), undo: Vec::new(), loading: false, focus: HWND::default(),
    });
}

// Same rule as app(): no Win32 call that can come back into rn_proc inside.
fn rn<R>(f: impl FnOnce(&mut Rn) -> R) -> R {
    RN.with(|r| f(&mut r.borrow_mut()))
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::Replace => tr("Replace", "바꾸기"),
        Kind::Insert => tr("Insert text", "글자 넣기"),
        Kind::Remove => tr("Remove", "지우기"),
        Kind::Number => tr("Numbering", "번호"),
        Kind::Date => tr("Date", "날짜"),
        Kind::Case => tr("Case", "대소문자"),
        Kind::Trim => tr("Trim spaces", "공백 정리"),
    }
}

struct Spec {
    fields: [&'static str; 4], // "" = not used
    mode: &'static str,
    modes: Vec<&'static str>,
    flags: [&'static str; 2],
    hint: &'static str,
}

fn spec(k: Kind) -> Spec {
    let place = || vec![tr("Before the name", "이름 앞"), tr("After the name", "이름 뒤"), tr("Instead of the name", "이름 대신")];
    let none = Spec { fields: [""; 4], mode: "", modes: vec![], flags: ["", ""], hint: "" };
    match k {
        Kind::Replace => Spec {
            fields: [tr("Find", "찾을 글자"), tr("Replace with", "바꿀 글자"), "", ""],
            flags: [tr("Match case", "대소문자 구분"), tr("Regular expression", "정규식")],
            hint: tr("Regular expression: $1, $2 … put back groups", "정규식: $1, $2 … 로 묶음을 다시 넣음"),
            ..none
        },
        Kind::Insert => Spec {
            fields: [tr("Text", "글자"), tr("Position", "위치"), "", ""],
            mode: tr("Where", "넣을 곳"),
            modes: vec![tr("At the start", "맨 앞"), tr("At the end", "맨 뒤"), tr("At position", "위치에")],
            hint: tr("Position: characters before it (0 = start)", "위치: 앞에 남길 글자 수 (0 = 맨 앞)"),
            ..none
        },
        Kind::Remove => Spec {
            fields: [tr("Characters", "글자 수"), tr("Position", "위치"), "", ""],
            mode: tr("Counted from", "기준"),
            modes: vec![tr("Start", "앞에서"), tr("End", "뒤에서")],
            hint: tr("Position: characters skipped first", "위치: 먼저 건너뛸 글자 수"),
            ..none
        },
        Kind::Number => Spec {
            fields: [tr("Start", "시작"), tr("Step", "간격"), tr("Digits", "자릿수"), tr("Separator", "구분자")],
            mode: tr("Place", "자리"),
            modes: place(),
            hint: tr("Digits empty: as many as needed (01 … 12)", "자릿수를 비우면 필요한 만큼 (01 … 12)"),
            ..none
        },
        Kind::Date => Spec {
            fields: [tr("Pattern", "형식"), tr("Separator", "구분자"), "", ""],
            mode: tr("Place", "자리"),
            modes: place(),
            hint: tr("Modified date: yyyy yy MM dd HH mm ss", "수정한 날짜: yyyy yy MM dd HH mm ss"),
            ..none
        },
        Kind::Case => Spec { mode: tr("Case", "대소문자"), modes: vec!["lowercase", "UPPERCASE", "Title Case"], ..none },
        Kind::Trim => Spec { hint: tr("Spaces at both ends and doubled spaces", "양 끝 공백과 겹친 공백"), ..none },
    }
}

// One line in the rule list.
fn summary(r: &Rule) -> String {
    let sp = spec(r.kind);
    let mode = sp.modes.get(r.mode as usize).copied().unwrap_or("");
    let detail = match r.kind {
        Kind::Replace => {
            let mut t = format!("\"{}\" → \"{}\"", r.f[0], r.f[1]);
            if r.x {
                t += " · Aa";
            }
            if r.y {
                t += " · .*";
            }
            t
        }
        Kind::Insert if r.mode == 2 => format!("\"{}\" · {mode} {}", r.f[0], r.f[1].trim()),
        Kind::Insert => format!("\"{}\" · {mode}", r.f[0]),
        Kind::Remove => format!("{} · {mode} {}", r.f[0].trim(), r.f[1].trim()),
        Kind::Number => format!("{}, {} … · {mode}", number_text(r, 0, 2), number_text(r, 1, 2)),
        Kind::Date => format!("{} · {mode}", r.f[0]),
        Kind::Case => mode.to_string(),
        Kind::Trim => String::new(),
    };
    if detail.is_empty() { kind_name(r.kind).into() } else { format!("{}: {detail}", kind_name(r.kind)) }
}

fn status_text(s: Status) -> &'static str {
    match s {
        Status::Same => tr("unchanged", "그대로"),
        Status::Ok => "",
        Status::Empty => tr("empty name", "빈 이름"),
        Status::BadChar => tr("character not allowed in names", "이름에 쓸 수 없는 글자"),
        Status::Reserved => tr("name reserved by Windows", "Windows 예약 이름"),
        Status::TrailingDot => tr("ends with a space or dot", "공백이나 점으로 끝남"),
        Status::TooLong => tr("name too long", "이름이 너무 김"),
        Status::Duplicate => tr("same name twice", "같은 이름이 둘 이상"),
        Status::Exists => tr("a file with this name exists", "같은 이름의 파일이 있음"),
    }
}

// ---------- layout ----------

struct Geo {
    ctl: Vec<(i32, RECT)>,
    titles: [RECT; 3], // rules, rule editor, preview
    summary: RECT,
    labels: [RECT; 5], // text fields 0..3, mode
    fields: [RECT; 5], // their backgrounds (the mode button fills its own)
    hint: RECT,
    note: RECT,
    pname: RECT,
    lists: [RECT; 2], // frames around the rule list and the preview
}

// Left: rule list (takes the height that is left, 70..170 DIP), its buttons, the rule's fields, presets.
// Right: the preview. Bottom: extension option, result line, Undo / Rename / Close.
// A kind has a mode or the two check boxes, never both, so they share a row.
fn geo(w: i32, h: i32, dpi: i32, fh: i32) -> Geo {
    let s = |v: i32| v * dpi / 96;
    let th = fh + s(4);
    // everything left of the preview but the rule list, for these spacings
    let below = |pad: i32, gap: i32, rh: i32, hint_h: i32, tg: i32| pad + th + s(4) + gap + rh + tg + th + s(4) + 5 * (rh + gap) + hint_h + gap + rh + gap + rh + pad;
    let normal = (s(12), s(6), s(28).max(fh + s(10)), fh * 2 + s(4), s(12), s(70));
    // a short window (a 640 × 480 screen): tighter rows, a one-line hint, a shorter rule list
    let tight = (s(8), s(4), s(22).max(fh + s(6)), fh + s(2), s(6), s(40));
    let (n0, n1, n2, n3, n4, _) = normal;
    let (pad, gap, rh, hint_h, tg, lb_min) = if h >= below(n0, n1, n2, n3, n4) + normal.5 { normal } else { tight };
    let lw = s(340);
    let (x0, x1) = (pad, pad + lw);
    let r = |l: i32, t: i32, rr: i32, b: i32| RECT { left: l, top: t, right: rr, bottom: b };
    let mut ctl = Vec::new();
    let t_rules = r(x0, pad, x1, pad + th);
    let lb_h = (h - below(pad, gap, rh, hint_h, tg)).clamp(lb_min, s(170));
    let lb = r(x0, t_rules.bottom + s(4), x1, t_rules.bottom + s(4) + lb_h);
    ctl.push((ID_RULES, lb));
    let by = lb.bottom + gap;
    let bw = (lw - 4 * gap) / 5;
    for (k, id) in [ID_ADD, ID_DEL, ID_RUP, ID_RDOWN, ID_ONOFF].into_iter().enumerate() {
        let l = x0 + k as i32 * (bw + gap);
        ctl.push((id, r(l, by, if k == 4 { x1 } else { l + bw }, by + rh)));
    }
    let t_edit = r(x0, by + rh + tg, x1, by + rh + tg + th);
    let (mut labels, mut fields) = ([RECT::default(); 5], [RECT::default(); 5]);
    let lx = x0 + s(110);
    for k in 0..5 {
        let y = t_edit.bottom + s(4) + k as i32 * (rh + gap);
        labels[k] = r(x0, y, lx - gap, y + rh);
        fields[k] = r(lx, y, x1, y + rh);
        if k < 4 {
            let top = y + (rh - fh) / 2;
            ctl.push((ID_F0 + k as i32, r(lx + s(8), top, x1 - s(8), top + fh)));
        }
    }
    ctl.push((ID_MODE, fields[4]));
    let half = (lw - gap) / 2;
    let fy = fields[4].top;
    ctl.push((ID_FX, r(x0, fy, x0 + half, fy + rh)));
    ctl.push((ID_FY, r(x0 + half + gap, fy, x1, fy + rh)));
    let hint = r(x0, fields[4].bottom + s(2), x1, fields[4].bottom + s(2) + hint_h);
    // bottom bar, and the preset row above it on the left
    let yb = h - pad - rh;
    let yp = yb - gap - rh;
    let pname = r(x0, yp, x0 + s(160), yp + rh);
    let top = yp + (rh - fh) / 2;
    ctl.push((ID_PNAME, r(pname.left + s(8), top, pname.right - s(8), top + fh)));
    let pm = r(x1 - s(100), yp, x1, yp + rh);
    ctl.push((ID_PMENU, pm));
    ctl.push((ID_PSAVE, r(pname.right + gap, yp, pm.left - gap, yp + rh)));
    ctl.push((ID_EXTOK, r(x0, yb, x0 + s(170), yb + rh)));
    let mut xr = w - pad;
    for (id, bw) in [(ID_CLOSE, s(80)), (ID_GO, s(110)), (ID_UNDO, s(90))] {
        ctl.push((id, r(xr - bw, yb, xr, yb + rh)));
        xr -= bw + gap;
    }
    let note = r(x0 + s(170) + s(12), yb, xr - s(6), yb + rh);
    // preview on the right
    let xp = x1 + s(16);
    let t_prev = r(xp, pad, w - pad, pad + th);
    let lv = r(xp, lb.top, w - pad, yb - gap);
    ctl.push((ID_LIST, lv));
    let summary = RECT { left: xp + s(80), ..t_prev };
    let out = |rc: RECT| r(rc.left - 1, rc.top - 1, rc.right + 1, rc.bottom + 1);
    Geo { ctl, titles: [t_rules, t_edit, t_prev], summary, labels, fields, hint, note, pname, lists: [out(lb), out(lv)] }
}

unsafe fn geo_now(h: HWND) -> Geo {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let (dpi, fh) = app(|a| (a.dpi, a.fh));
    geo(rc.right, rc.bottom, dpi, fh)
}

// Smallest window, in DIP: its client (about 584 × 361) still holds every control (rename_layout test).
const MIN_W: i32 = 600;
const MIN_H: i32 = 400;

// Work area of the monitor h is on; the primary one's if that can't be read (never an empty rect).
unsafe fn work_area(h: HWND) -> RECT {
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    if GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() && mi.rcWork.right > mi.rcWork.left {
        return mi.rcWork;
    }
    let mut r = RECT::default();
    if SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut r as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok() && r.right > r.left {
        return r;
    }
    RECT { left: 0, top: 0, right: GetSystemMetrics(SM_CXSCREEN), bottom: GetSystemMetrics(SM_CYSCREEN) }
}

unsafe fn item(h: HWND, id: i32) -> HWND {
    GetDlgItem(Some(h), id).unwrap_or_default()
}

unsafe fn layout(h: HWND) {
    for (id, r) in geo_now(h).ctl {
        _ = MoveWindow(item(h, id), r.left, r.top, r.right - r.left, r.bottom - r.top, false);
    }
    _ = InvalidateRect(Some(h), None, false);
}

// ---------- opening: the active pane's selection, or every file in its folder ----------

fn local_date(ft: u64) -> [u16; 6] {
    let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let (mut utc, mut st) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut st).is_err() {
            return [0; 6];
        }
    }
    [st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond]
}

fn folder_names(dir: &str) -> HashSet<String> {
    std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect()).unwrap_or_default()
}

fn file_date(path: &str) -> [u16; 6] {
    std::fs::metadata(path).map(|m| local_date(m.last_write_time())).unwrap_or([0; 6])
}

// (folders, items). Only real files and folders: a virtual item (a phone, a ZIP's inside) has no path to rename.
unsafe fn gather(i: usize) -> Option<(Vec<String>, Vec<(usize, Src)>)> {
    let fv = view_of::<IFolderView2>(i)?;
    let sel = fv.ItemCount(SVGIO_SELECTION).unwrap_or(0) > 0;
    let what = if sel { SVGIO_SELECTION } else { SVGIO_ALLVIEW };
    let arr: IShellItemArray = fv.Items(_SVGIO(what.0 | SVGIO_FLAG_VIEWORDER.0)).or_else(|_| fv.Items(what)).ok()?;
    let n = arr.GetCount().unwrap_or(0);
    if n > SEND_MAX {
        set_note(tr("Too many items to rename (over 10,000)", "항목이 너무 많아 이름을 바꾸지 않음 (1만 개 초과)").into());
        return None;
    }
    let (mut dirs, mut items) = (Vec::<String>::new(), Vec::new());
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { continue };
        let attr = it.GetAttributes(SFGAO_FLAGS(SFGAO_FOLDER.0 | SFGAO_STREAM.0)).unwrap_or_default().0;
        let is_dir = real_folder(attr);
        if is_dir && !sel {
            continue; // nothing selected: files only
        }
        let Ok(path) = it.GetDisplayName(SIGDN_FILESYSPATH).map(|p| pwstr(p)) else { continue };
        let p = Path::new(&path);
        let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else { continue }; // a drive root
        let dir = dir.to_string_lossy().into_owned();
        let d = dirs.iter().position(|x| x.eq_ignore_ascii_case(&dir)).unwrap_or_else(|| {
            dirs.push(dir);
            dirs.len() - 1
        });
        items.push((d, Src { name: name.to_string_lossy().into_owned(), is_dir, date: file_date(&path) }));
    }
    Some((dirs, items))
}

pub(crate) unsafe fn open_rename(i: usize) {
    let Some((dirs, items)) = gather(i) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if items.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        set_note(tr("Nothing to rename here", "이름을 바꿀 파일이 없음").into());
        return;
    }
    let existing = dirs.iter().map(|d| folder_names(d)).collect();
    let title = HSTRING::from(format!("{} - {}", tr("Batch rename", "일괄 이름 바꾸기"), short_name(&dirs[0])));
    let open = rn(|r| {
        (r.pane, r.dirs, r.items, r.existing, r.note) = (i, dirs, items, existing, String::new());
        r.h
    });
    if !open.0.is_null() {
        // already open: it takes the new files, rules stay
        _ = SetWindowTextW(open, &title);
        update_preview();
        _ = SetForegroundWindow(open);
        return;
    }
    let (main, dpi) = app(|a| (a.main, a.dpi));
    let work = work_area(main);
    let w = (980 * dpi / 96).min(work.right - work.left);
    let ht = (640 * dpi / 96).min(work.bottom - work.top);
    let mut rc = RECT::default();
    _ = GetWindowRect(main, &mut rc);
    let x = ((rc.left + rc.right - w) / 2).clamp(work.left, work.right - w);
    let y = ((rc.top + rc.bottom - ht) / 2).clamp(work.top, work.bottom - ht);
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MAXIMIZEBOX | WS_CLIPCHILDREN;
    let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsRename"), &title, style, x, y, w, ht, Some(main), None, None, None) else {
        return;
    };
    rn(|r| r.h = h);
    let font = app(|a| a.font);
    let tab = WS_TABSTOP.0;
    let lb = (LBS_OWNERDRAWFIXED | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32 | WS_VSCROLL.0 | tab;
    child(h, w!("LISTBOX"), w!(""), lb, WINDOW_EX_STYLE(0), ID_RULES, font);
    let btn = BS_OWNERDRAW as u32 | tab;
    for id in [ID_ADD, ID_DEL, ID_RUP, ID_RDOWN, ID_ONOFF] {
        child(h, w!("BUTTON"), w!(""), btn, WINDOW_EX_STYLE(0), id, font);
    }
    for k in 0..4 {
        child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32 | tab, WINDOW_EX_STYLE(0), ID_F0 + k, font);
    }
    for id in [ID_MODE, ID_FX, ID_FY] {
        child(h, w!("BUTTON"), w!(""), btn, WINDOW_EX_STYLE(0), id, font);
    }
    let lv_style = LVS_REPORT | LVS_OWNERDATA | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER;
    let lv = child(h, w!("SysListView32"), w!(""), lv_style | tab, WINDOW_EX_STYLE(0), ID_LIST, font);
    let ex = (LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER) as isize;
    SendMessageW(lv, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(ex as usize)), Some(LPARAM(ex)));
    _ = SetWindowSubclass(lv, Some(dark_list_proc), 2, 0); // dark column header, as in the panes
    for k in 0..3 {
        // with a text from the start: a column inserted without one keeps an empty header
        let mut c = LVCOLUMNW { mask: LVCF_WIDTH | LVCF_TEXT, cx: [230, 230, 170][k] * dpi / 96, pszText: PWSTR(w!(" ").as_ptr() as *mut u16), ..Default::default() };
        SendMessageW(lv, LVM_INSERTCOLUMNW, Some(WPARAM(k)), Some(LPARAM(&mut c as *mut _ as isize)));
    }
    child(h, w!("BUTTON"), w!(""), btn, WINDOW_EX_STYLE(0), ID_EXTOK, font);
    child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32 | tab, WINDOW_EX_STYLE(0), ID_PNAME, font);
    for id in [ID_PSAVE, ID_PMENU, ID_UNDO, ID_GO, ID_CLOSE] {
        child(h, w!("BUTTON"), w!(""), btn, WINDOW_EX_STYLE(0), id, font);
    }
    rename_theme();
    layout(h);
    fill_rules();
    load_editor();
    update_preview();
    let lbh = item(h, ID_RULES);
    rn(|r| r.focus = lbh);
    _ = ShowWindow(h, SW_SHOW);
    _ = SetFocus(Some(lbh));
}

// Colours, theme parts and language-dependent texts; also after a theme or language switch (theme_chrome).
pub(crate) unsafe fn rename_theme() {
    let h = rn(|r| r.h);
    if h.0.is_null() {
        return;
    }
    let dark = DARK_ON.load(Ordering::Relaxed);
    let on = BOOL::from(dark);
    _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = cref(pal().bg);
    _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    let lv = item(h, ID_LIST);
    let lb = item(h, ID_RULES);
    let theme = if dark { w!("DarkMode_Explorer") } else { w!("Explorer") };
    _ = SetWindowTheme(lv, theme, PCWSTR::null());
    _ = SetWindowTheme(lb, theme, PCWSTR::null());
    let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
    _ = SetWindowTheme(header, if dark { w!("DarkMode_ItemsView") } else { w!("ItemsView") }, PCWSTR::null());
    let (card, text) = (cref(pal().card).0 as isize, cref(pal().text).0 as isize);
    SendMessageW(lv, LVM_SETBKCOLOR, None, Some(LPARAM(card)));
    SendMessageW(lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(card)));
    SendMessageW(lv, LVM_SETTEXTCOLOR, None, Some(LPARAM(text)));
    for (k, name) in [tr("Name", "이름"), tr("New name", "새 이름"), tr("Problem", "문제")].into_iter().enumerate() {
        let name = HSTRING::from(name);
        let c = LVCOLUMNW { mask: LVCF_TEXT, pszText: PWSTR(name.as_ptr() as *mut u16), ..Default::default() };
        SendMessageW(lv, LVM_SETCOLUMNW, Some(WPARAM(k)), Some(LPARAM(&c as *const _ as isize)));
    }
    let cue = HSTRING::from(tr("Preset name", "프리셋 이름"));
    SendMessageW(item(h, ID_PNAME), EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(cue.as_ptr() as isize)));
    let title = HSTRING::from(format!("{} - {}", tr("Batch rename", "일괄 이름 바꾸기"), short_name(&rn(|r| r.dirs.first().cloned().unwrap_or_default()))));
    _ = SetWindowTextW(h, &title);
    _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
}

// ---------- state -> controls ----------

unsafe fn fill_rules() {
    let (h, n, cur) = rn(|r| (r.h, r.rules.len(), r.cur));
    let lb = item(h, ID_RULES);
    SendMessageW(lb, LB_RESETCONTENT, None, None);
    for k in 0..n {
        SendMessageW(lb, LB_ADDSTRING, None, Some(LPARAM(k as isize)));
    }
    SendMessageW(lb, LB_SETCURSEL, Some(WPARAM(cur)), None);
}

// The fields of the current rule; the ones its kind doesn't use are hidden.
unsafe fn load_editor() {
    let (h, rule) = rn(|r| {
        r.loading = true;
        (r.h, r.rules.get(r.cur).cloned())
    });
    let sp = rule.as_ref().map(|r| spec(r.kind));
    for k in 0..4 {
        let e = item(h, ID_F0 + k as i32);
        let used = sp.as_ref().is_some_and(|s| !s.fields[k].is_empty());
        _ = SetWindowTextW(e, &HSTRING::from(rule.as_ref().filter(|_| used).map_or("", |r| r.f[k].as_str())));
        _ = ShowWindow(e, if used { SW_SHOW } else { SW_HIDE });
    }
    let show = |id: i32, on: bool| _ = ShowWindow(item(h, id), if on { SW_SHOW } else { SW_HIDE });
    show(ID_MODE, sp.as_ref().is_some_and(|s| !s.modes.is_empty()));
    show(ID_FX, sp.as_ref().is_some_and(|s| !s.flags[0].is_empty()));
    show(ID_FY, sp.as_ref().is_some_and(|s| !s.flags[1].is_empty()));
    let any = rule.is_some();
    for id in [ID_DEL, ID_RUP, ID_RDOWN, ID_ONOFF] {
        _ = EnableWindow(item(h, id), any);
    }
    rn(|r| r.loading = false);
    _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN);
}

// New names and their checks for every item; the Rename button is only on when all of them are fine.
unsafe fn update_preview() {
    let (h, n, go, undo) = rn(|r| {
        let srcs: Vec<Src> = r.items.iter().map(|(_, s)| s.clone()).collect();
        match new_names(&r.rules, r.ext, &srcs) {
            Ok(names) => {
                let moves: Vec<Move> = r.items.iter().zip(&names).map(|((d, s), n)| (*d, s.name.clone(), n.clone())).collect();
                let ex = &r.existing;
                let st = check(&moves, &|d, name| ex.get(d).is_some_and(|set| set.contains(name)));
                r.rows = names.into_iter().zip(st).collect();
                r.err = None;
            }
            Err(e) => {
                r.rows = r.items.iter().map(|(_, s)| (s.name.clone(), Status::Same)).collect();
                r.err = Some(e);
            }
        }
        let ok = r.err.is_none() && !r.rows.iter().any(|(_, s)| s.blocks()) && r.rows.iter().any(|(_, s)| *s == Status::Ok);
        (r.h, r.rows.len(), ok, !r.undo.is_empty())
    });
    let lv = item(h, ID_LIST);
    SendMessageW(lv, LVM_SETITEMCOUNT, Some(WPARAM(n)), Some(LPARAM(2))); // LVSICF_NOSCROLL
    _ = InvalidateRect(Some(lv), None, false);
    _ = InvalidateRect(Some(item(h, ID_RULES)), None, false);
    _ = EnableWindow(item(h, ID_GO), go);
    _ = EnableWindow(item(h, ID_UNDO), undo);
    _ = InvalidateRect(Some(h), None, false); // summary line
}

fn counts(r: &Rn) -> (usize, usize, usize) {
    let changed = r.rows.iter().filter(|(_, s)| *s == Status::Ok).count();
    let bad = r.rows.iter().filter(|(_, s)| s.blocks()).count();
    (r.rows.len(), changed, bad)
}

// ---------- painting ----------

unsafe fn paint(h: HWND) {
    let g = geo_now(h);
    let (font, small) = app(|a| (a.font, a.small_font));
    let (rule, (n, changed, bad), err, note) = rn(|r| (r.rules.get(r.cur).cloned(), counts(r), r.err.clone(), r.note.clone()));
    let ko = KO.load(Ordering::Relaxed);
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        let titles = [tr("Rules, applied in order", "규칙 (위에서부터 차례로)"), "", tr("Preview", "미리보기")];
        let edit_title = rule.as_ref().map_or(String::new(), |r| kind_name(r.kind).to_string());
        for (k, t) in titles.iter().enumerate() {
            text(dc, g.titles[k], if k == 1 { edit_title.as_str() } else { *t }, small, pal().sub, DT_LEFT);
        }
        let sum = match (&err, ko) {
            (Some(e), false) => format!("Bad regular expression: {}", e.lines().last().unwrap_or("")),
            (Some(e), true) => format!("정규식 오류: {}", e.lines().last().unwrap_or("")),
            (None, false) => format!("{n} items · {changed} to rename · {bad} problems"),
            (None, true) => format!("{n}개 · 바꿀 것 {changed} · 문제 {bad}"),
        };
        let warn = err.is_some() || bad > 0;
        text(dc, g.summary, &sum, small, if warn { red() } else { pal().sub }, DT_RIGHT);
        for r in g.lists {
            frame(dc, r, 1, pal().line);
        }
        if let Some(rule) = &rule {
            let sp = spec(rule.kind);
            for k in 0..4 {
                if !sp.fields[k].is_empty() {
                    text(dc, g.labels[k], sp.fields[k], font, pal().sub, DT_LEFT);
                    fill(dc, g.fields[k], pal().pill);
                }
            }
            if !sp.modes.is_empty() {
                text(dc, g.labels[4], sp.mode, font, pal().sub, DT_LEFT);
            }
            wrap_text(dc, g.hint, sp.hint, small, pal().sub, false);
        }
        fill(dc, g.pname, pal().pill);
        text(dc, g.note, &note, font, pal().text, DT_LEFT);
    });
}

fn red() -> u32 {
    if DARK_ON.load(Ordering::Relaxed) { 0xF08080 } else { 0xC0392B }
}

unsafe fn draw_item(d: &DRAWITEMSTRUCT) {
    let (dc, rc, id) = (d.hDC, d.rcItem, d.CtlID as i32);
    let (dpi, font, icon_font, hover, accent) = app(|a| (a.dpi, a.font, a.icon_font, a.hover, a.s.accent_now()));
    let s = |v: i32| v * dpi / 96;
    let focus = d.itemState.0 & ODS_FOCUS.0 != 0;
    if d.CtlType == ODT_LISTBOX {
        let Some((r, num)) = rn(|x| x.rules.get(d.itemID as usize).map(|r| (r.clone(), d.itemID as usize + 1))) else {
            return fill(dc, rc, pal().card);
        };
        let sel = d.itemState.0 & ODS_SELECTED.0 != 0;
        let bg = if sel { accent } else { pal().card };
        let ink = if sel { on_color(accent) } else if r.on { pal().text } else { pal().sub };
        fill(dc, rc, bg);
        let check = RECT { left: rc.left + s(6), right: rc.left + s(26), ..rc };
        text(dc, check, if r.on { "\u{E73A}" } else { "\u{E739}" }, icon_font, ink, DT_CENTER);
        text(dc, RECT { left: check.right + s(6), right: rc.right - s(6), ..rc }, &format!("{num}. {}", summary(&r)), font, ink, DT_LEFT);
        if focus && GetFocus() == d.hwndItem {
            frame(dc, rc, s(1).max(1), ink);
        }
        return;
    }
    let off = d.itemState.0 & ODS_DISABLED.0 != 0;
    let pressed = d.itemState.0 & ODS_SELECTED.0 != 0;
    let hot = hover == d.hwndItem && !off;
    let (rule, ext) = rn(|r| (r.rules.get(r.cur).cloned(), r.ext));
    // check boxes: glyph + label on the ground
    let check = match id {
        ID_EXTOK => Some((ext, tr("Include extension", "확장자도 바꾸기"))),
        ID_FX | ID_FY => rule.as_ref().map(|r| (if id == ID_FX { r.x } else { r.y }, spec(r.kind).flags[(id - ID_FX) as usize])),
        _ => None,
    };
    if let Some((on, label)) = check {
        fill(dc, rc, if hot { blend(pal().card, pal().text, 0.06) } else { pal().card });
        let gl = RECT { left: rc.left + s(4), right: rc.left + s(24), ..rc };
        text(dc, gl, if on { "\u{E73A}" } else { "\u{E739}" }, icon_font, if on { pal().text } else { pal().sub }, DT_CENTER);
        text(dc, RECT { left: gl.right + s(6), ..rc }, label, font, pal().text, DT_LEFT);
        if focus {
            frame(dc, rc, 1, pal().sub);
        }
        return;
    }
    let primary = id == ID_GO && !off;
    let base = if primary { accent } else { pal().pill };
    let ink = if off { blend(base, pal().sub, 0.6) } else if primary { on_color(accent) } else { pal().text };
    fill(dc, rc, base);
    if pressed || hot {
        fill(dc, rc, blend(base, ink, press_tint(primary, pressed)));
    }
    frame(dc, rc, 1, if focus { if primary { ink } else { accent } } else { pal().line });
    if focus {
        frame(dc, RECT { left: rc.left + 1, top: rc.top + 1, right: rc.right - 1, bottom: rc.bottom - 1 }, 1, if primary { ink } else { accent });
    }
    let (label, icon) = match id {
        ID_ADD => (format!("{} ▾", tr("Add", "추가")), false),
        ID_DEL => (tr("Remove", "빼기").into(), false),
        ID_RUP => ("\u{E74A}".into(), true),
        ID_RDOWN => ("\u{E74B}".into(), true),
        ID_ONOFF => (tr("On / off", "켜기/끄기").into(), false),
        ID_MODE => (rule.map(|r| format!("{} ▾", spec(r.kind).modes.get(r.mode as usize).copied().unwrap_or(""))).unwrap_or_default(), false),
        ID_PSAVE => (tr("Save", "저장").into(), false),
        ID_PMENU => (format!("{} ▾", tr("Presets", "프리셋")), false),
        ID_UNDO => (tr("Undo", "되돌리기").into(), false),
        ID_GO => (tr("Rename", "이름 바꾸기").into(), false),
        ID_CLOSE => (tr("Close", "닫기").into(), false),
        _ => (String::new(), false),
    };
    let align = if id == ID_MODE { DT_LEFT } else { DT_CENTER };
    let r = if id == ID_MODE { RECT { left: rc.left + s(8), ..rc } } else { rc };
    text(dc, r, &label, if icon { icon_font } else { font }, ink, align);
}

// ---------- actions ----------

unsafe fn changed(f: impl FnOnce(&mut Rn)) {
    rn(f);
    update_preview();
}

unsafe fn pick(h: HWND, id: i32, items: &[(usize, String, bool)]) -> usize {
    let Ok(menu) = CreatePopupMenu() else { return 0 };
    for (k, t, on) in items {
        _ = AppendMenuW(menu, if *on { MF_STRING | MF_CHECKED } else { MF_STRING }, *k, &HSTRING::from(t.as_str()));
    }
    let mut rc = RECT::default();
    _ = GetWindowRect(item(h, id), &mut rc);
    let k = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD, rc.left, rc.bottom, None, h, None).0 as usize;
    _ = DestroyMenu(menu);
    k
}

unsafe fn add_rule(h: HWND) {
    let kinds: Vec<(usize, String, bool)> = KINDS.iter().enumerate().map(|(k, &kd)| (MENU_ADD + k, kind_name(kd).to_string(), false)).collect();
    let k = pick(h, ID_ADD, &kinds);
    if !(MENU_ADD..MENU_ADD + KINDS.len()).contains(&k) {
        return;
    }
    rn(|r| {
        let at = (r.cur + 1).min(r.rules.len());
        r.rules.insert(at, Rule::new(KINDS[k - MENU_ADD]));
        r.cur = at;
    });
    fill_rules();
    load_editor();
    update_preview();
    let first = item(h, ID_F0);
    _ = SetFocus(Some(if IsWindowVisible(first).as_bool() { first } else { item(h, ID_RULES) }));
}

unsafe fn edit_rules(f: impl FnOnce(&mut Rn)) {
    rn(|r| {
        f(r);
        r.cur = r.cur.min(r.rules.len().saturating_sub(1));
    });
    fill_rules();
    load_editor();
    update_preview();
}

unsafe fn move_rule(d: isize) {
    edit_rules(|r| {
        let to = r.cur as isize + d;
        if to >= 0 && (to as usize) < r.rules.len() {
            r.rules.swap(r.cur, to as usize);
            r.cur = to as usize;
        }
    });
}

unsafe fn pick_mode(h: HWND) {
    let Some(rule) = rn(|r| r.rules.get(r.cur).cloned()) else { return };
    let modes: Vec<(usize, String, bool)> = spec(rule.kind).modes.iter().enumerate().map(|(k, m)| (MENU_MODE + k, m.to_string(), k == rule.mode as usize)).collect();
    let k = pick(h, ID_MODE, &modes);
    if (MENU_MODE..MENU_MODE + modes.len()).contains(&k) {
        changed(|r| r.rules[r.cur].mode = (k - MENU_MODE) as u8);
        _ = InvalidateRect(Some(item(h, ID_MODE)), None, false);
    }
}

// Opening a file for delete access is what a rename needs too: a file another program holds without
// sharing it fails here, before anything has moved.
unsafe fn in_use(path: &str) -> Option<String> {
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    match CreateFileW(&HSTRING::from(path), DELETE.0, share, None, OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, None) {
        Ok(f) => {
            _ = CloseHandle(f);
            None
        }
        Err(e) => Some(e.message()),
    }
}

fn join_path(dir: &str, name: &str) -> String {
    Path::new(dir).join(name).to_string_lossy().into_owned()
}

// The steps in order; if one fails, the ones done are moved back (newest first), so a batch is all or nothing.
unsafe fn run_steps(dirs: &[String], steps: &[Move]) -> std::result::Result<(), (String, String)> {
    for (k, (d, from, to)) in steps.iter().enumerate() {
        if let Err(e) = MoveFileExW(&HSTRING::from(join_path(&dirs[*d], from)), &HSTRING::from(join_path(&dirs[*d], to)), MOVE_FILE_FLAGS(0)) {
            for (d, from, to) in steps[..k].iter().rev() {
                _ = MoveFileExW(&HSTRING::from(join_path(&dirs[*d], to)), &HSTRING::from(join_path(&dirs[*d], from)), MOVE_FILE_FLAGS(0));
            }
            return Err((from.clone(), e.message()));
        }
    }
    Ok(())
}

// Lets the panes show the new names now rather than when the folder watcher gets to it.
unsafe fn notify(dirs: &[String], moves: &[Move], items: &[(usize, Src)]) {
    for (d, old, new) in moves {
        let dir = items.iter().any(|(id, s)| id == d && (s.name == *old || s.name == *new) && s.is_dir);
        let (a, b) = (HSTRING::from(join_path(&dirs[*d], old)), HSTRING::from(join_path(&dirs[*d], new)));
        SHChangeNotify(if dir { SHCNE_RENAMEFOLDER } else { SHCNE_RENAMEITEM }, SHCNF_PATHW, Some(a.as_ptr() as _), Some(b.as_ptr() as _));
    }
}

fn tmp_name(nonce: u128) -> impl Fn(usize) -> String {
    move |k| format!("~grids-rename-{nonce}-{k}.tmp")
}

fn nonce() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 1_000_000_000).unwrap_or(0)
}

// Up to 20 names, then a count.
fn listing(head: &str, lines: &[String]) -> String {
    let mut t = format!("{head}\n");
    for l in lines.iter().take(20) {
        t += &format!("\n{l}");
    }
    if lines.len() > 20 {
        t += &format!("\n… +{}", lines.len() - 20);
    }
    t
}

unsafe fn report(h: HWND, note: String, details: Option<String>) {
    rn(|r| r.note = note);
    update_preview();
    if let Some(t) = details {
        MessageBoxW(Some(h), &HSTRING::from(t), &HSTRING::from(tr("Batch rename", "일괄 이름 바꾸기")), MB_OK | MB_ICONWARNING);
    }
}

unsafe fn do_rename(h: HWND) {
    update_preview();
    let ready = rn(|r| r.err.is_none() && !r.rows.iter().any(|(_, s)| s.blocks()));
    let (dirs, moves) = rn(|r| {
        let m: Vec<Move> = r.items.iter().zip(&r.rows).filter(|(_, (_, s))| *s == Status::Ok).map(|((d, s), (n, _))| (*d, s.name.clone(), n.clone())).collect();
        (r.dirs.clone(), m)
    });
    if !ready || moves.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let busy: Vec<String> = moves.iter().filter_map(|(d, o, _)| in_use(&join_path(&dirs[*d], o)).map(|e| format!("{o}: {}", e.trim()))).collect();
    let ko = KO.load(Ordering::Relaxed);
    if !busy.is_empty() {
        let head = if ko { format!("아무것도 바꾸지 않았음: 지금 바꿀 수 없는 파일 {}개", busy.len()) } else { format!("Nothing renamed: can't rename {} of the files now", busy.len()) };
        return report(h, head.clone(), Some(listing(&head, &busy)));
    }
    let steps = plan(&moves, &tmp_name(nonce()));
    if let Err((name, e)) = run_steps(&dirs, &steps) {
        let head = if ko { "아무것도 바꾸지 않았음 (모두 원래대로)".to_string() } else { "Nothing renamed (all put back)".to_string() };
        return report(h, format!("{head}: {name}: {}", e.trim()), Some(listing(&head, &[format!("{name}: {}", e.trim())])));
    }
    let items = rn(|r| r.items.clone());
    notify(&dirs, &moves, &items);
    let n = moves.len();
    rn(|r| {
        for ((_, s), (new, st)) in r.items.iter_mut().zip(&r.rows) {
            if *st == Status::Ok {
                s.name = new.clone();
            }
        }
        r.undo = moves.iter().map(|(d, o, n)| (dirs[*d].clone(), o.clone(), n.clone())).collect();
        r.existing = dirs.iter().map(|d| folder_names(d)).collect();
    });
    set_note(if ko { format!("{n}개 이름 바꿈") } else { format!("Renamed {n}") });
    report(h, if ko { format!("{n}개 이름을 바꿨음. 되돌리기로 원래 이름으로.") } else { format!("Renamed {n}. Undo puts the old names back.") }, None);
}

// Last batch back to the old names. Items renamed or removed since are skipped and listed, as are
// old names another file has taken in the meantime.
unsafe fn do_undo(h: HWND) {
    let undo = rn(|r| std::mem::take(&mut r.undo));
    if undo.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let mut dirs: Vec<String> = Vec::new();
    let mut moves: Vec<Move> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (dir, old, new) in &undo {
        if std::fs::symlink_metadata(join_path(dir, new)).is_err() {
            skipped.push(format!("{new}: {}", tr("no longer there", "더 이상 없음")));
            continue;
        }
        let d = dirs.iter().position(|x| x == dir).unwrap_or_else(|| {
            dirs.push(dir.clone());
            dirs.len() - 1
        });
        moves.push((d, new.clone(), old.clone()));
    }
    let existing: Vec<HashSet<String>> = dirs.iter().map(|d| folder_names(d)).collect();
    let st = check(&moves, &|d, n| existing[d].contains(n));
    let mut ok = Vec::new();
    for (m, s) in moves.into_iter().zip(st) {
        if s.blocks() {
            skipped.push(format!("{}: {}", m.1, status_text(s)));
        } else {
            ok.push(m);
        }
    }
    let ko = KO.load(Ordering::Relaxed);
    if let Some(busy) = ok.iter().find_map(|(d, o, _)| in_use(&join_path(&dirs[*d], o)).map(|e| format!("{o}: {}", e.trim()))) {
        rn(|r| r.undo = undo);
        let head = tr("Nothing put back", "되돌리지 않았음");
        return report(h, format!("{head}: {busy}"), Some(listing(head, &[busy])));
    }
    if let Err((name, e)) = run_steps(&dirs, &plan(&ok, &tmp_name(nonce()))) {
        rn(|r| r.undo = undo);
        let head = tr("Nothing put back", "되돌리지 않았음");
        return report(h, format!("{head}: {name}: {}", e.trim()), Some(listing(head, &[format!("{name}: {}", e.trim())])));
    }
    let items = rn(|r| r.items.clone());
    notify(&dirs, &ok, &items);
    // the window's files follow
    rn(|r| {
        for (idir, s) in r.items.iter_mut() {
            let dir = &r.dirs[*idir];
            if let Some((_, _, old)) = ok.iter().find(|(d, now, _)| dirs[*d] == *dir && *now == s.name) {
                s.name = old.clone();
            }
        }
        r.existing = r.dirs.iter().map(|d| folder_names(d)).collect();
    });
    let n = ok.len();
    let note = match (ko, skipped.len()) {
        (true, 0) => format!("{n}개를 원래 이름으로 되돌림"),
        (false, 0) => format!("Put back {n} names"),
        (true, k) => format!("{n}개를 되돌림, {k}개는 건너뜀"),
        (false, k) => format!("Put back {n} names, skipped {k}"),
    };
    set_note(note.clone());
    let details = (!skipped.is_empty()).then(|| listing(&note, &skipped));
    report(h, note, details);
}

unsafe fn save_preset(h: HWND) {
    let name = text_of(item(h, ID_PNAME)).trim().to_string();
    if name.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        _ = SetFocus(Some(item(h, ID_PNAME)));
        return;
    }
    let line = rn(|r| preset_text(&name, r.ext, &r.rules));
    let full = app(|a| {
        let at = a.s.rn_presets.iter().position(|p| parse_preset(p).is_some_and(|(n, _, _)| n.eq_ignore_ascii_case(&name)));
        match at {
            Some(k) => a.s.rn_presets[k] = line,
            None if a.s.rn_presets.len() < PRESETS_MAX => a.s.rn_presets.push(line),
            None => return true,
        }
        false
    });
    if full {
        _ = MessageBeep(MB_ICONWARNING);
        return report(h, tr("Too many presets: delete one first", "프리셋이 너무 많음: 먼저 하나를 지우세요").into(), None);
    }
    report(h, format!("{} {name}", tr("Preset saved:", "프리셋 저장함:")), None);
}

unsafe fn preset_menu(h: HWND) {
    let list: Vec<(String, bool, Vec<Rule>)> = app(|a| a.s.rn_presets.iter().filter_map(|p| parse_preset(p)).collect());
    let Ok(menu) = CreatePopupMenu() else { return };
    let Ok(forget) = CreatePopupMenu() else { return };
    for (k, (name, _, _)) in list.iter().enumerate().take(PRESETS_MAX) {
        let name = HSTRING::from(name.as_str());
        _ = AppendMenuW(menu, MF_STRING, MENU_LOAD + k, &name);
        _ = AppendMenuW(forget, MF_STRING, MENU_FORGET + k, &name);
    }
    if list.is_empty() {
        _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("No presets yet: name the rules and Save", "프리셋 없음: 이름을 적고 저장")));
    } else {
        _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        _ = AppendMenuW(menu, MF_POPUP, forget.0 as usize, &HSTRING::from(tr("Delete", "지우기")));
    }
    let mut rc = RECT::default();
    _ = GetWindowRect(item(h, ID_PMENU), &mut rc);
    let k = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN | TPM_RETURNCMD, rc.left, rc.bottom, None, h, None).0 as usize;
    _ = DestroyMenu(menu); // and the submenu with it
    if (MENU_LOAD..MENU_LOAD + list.len()).contains(&k) {
        let (name, ext, rules) = list[k - MENU_LOAD].clone();
        edit_rules(|r| (r.rules, r.ext, r.cur) = (rules, ext, 0));
        _ = SetWindowTextW(item(h, ID_PNAME), &HSTRING::from(name.as_str()));
        report(h, format!("{} {name}", tr("Preset loaded:", "프리셋 불러옴:")), None);
    } else if (MENU_FORGET..MENU_FORGET + list.len()).contains(&k) {
        let name = list[k - MENU_FORGET].0.clone();
        app(|a| a.s.rn_presets.retain(|p| parse_preset(p).is_none_or(|(n, _, _)| n != name)));
        report(h, format!("{} {name}", tr("Preset deleted:", "프리셋 지움:")), None);
    }
}

unsafe fn command(h: HWND, id: i32, code: u32) {
    match (id, code) {
        (ID_RULES, LBN_SELCHANGE) => {
            let k = SendMessageW(item(h, ID_RULES), LB_GETCURSEL, None, None).0;
            if k >= 0 {
                rn(|r| r.cur = k as usize);
                load_editor();
            }
        }
        (ID_RULES, 2) => toggle_rule(), // LBN_DBLCLK
        (k, EN_CHANGE) if (ID_F0..ID_F0 + 4).contains(&k) => {
            if rn(|r| r.loading) {
                return;
            }
            let t = text_of(item(h, k));
            changed(|r| {
                if let Some(rule) = r.rules.get_mut(r.cur) {
                    rule.f[(k - ID_F0) as usize] = t;
                }
            });
        }
        (_, c) if c != BN_CLICKED => {}
        (ID_ADD, _) => add_rule(h),
        (ID_DEL, _) => edit_rules(|r| {
            if r.cur < r.rules.len() {
                r.rules.remove(r.cur);
            }
        }),
        (ID_RUP, _) => move_rule(-1),
        (ID_RDOWN, _) => move_rule(1),
        (ID_ONOFF, _) => toggle_rule(),
        (ID_MODE, _) => pick_mode(h),
        (ID_FX | ID_FY, _) => {
            changed(|r| {
                if let Some(rule) = r.rules.get_mut(r.cur) {
                    if id == ID_FX { rule.x ^= true } else { rule.y ^= true }
                }
            });
            _ = InvalidateRect(Some(item(h, id)), None, false);
        }
        (ID_EXTOK, _) => {
            changed(|r| r.ext ^= true);
            _ = InvalidateRect(Some(item(h, id)), None, false);
        }
        (ID_PSAVE, _) => save_preset(h),
        (ID_PMENU, _) => preset_menu(h),
        (ID_UNDO, _) => do_undo(h),
        (ID_GO, _) => do_rename(h),
        (ID_CLOSE, _) => _ = DestroyWindow(h),
        _ => {}
    }
}

unsafe fn toggle_rule() {
    changed(|r| {
        if let Some(rule) = r.rules.get_mut(r.cur) {
            rule.on ^= true;
        }
    });
}

/// Keys for the rename window, from pre_translate. None: not this window's. Esc closes, Ctrl+Enter renames,
/// Ctrl+Z undoes (outside a text field); in the rule list Space switches a rule on/off, Del removes it,
/// Ins adds one and Alt+↑/↓ moves it. Tab and the rest go the dialog way. Some(_) always: key() and its pane
/// shortcuts (Ctrl+1…9, Ctrl+Tab, Ctrl+Shift+M, Alt+arrows ...) never see a key meant for this window.
pub(crate) unsafe fn rename_key(m: &MSG) -> Option<bool> {
    let h = rn(|r| r.h);
    if h.0.is_null() || !(m.hwnd == h || IsChild(h, m.hwnd).as_bool()) {
        return None;
    }
    if m.message == WM_KEYDOWN || m.message == WM_SYSKEYDOWN {
        let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
        let (ctrl, alt) = (down(VK_CONTROL), down(VK_MENU));
        let id = GetDlgCtrlID(m.hwnd);
        let mut class = [0u16; 16];
        let n = GetClassNameW(m.hwnd, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]).to_ascii_lowercase();
        let handled = match VIRTUAL_KEY(m.wParam.0 as u16) {
            VK_ESCAPE => {
                _ = DestroyWindow(h);
                true
            }
            // the shortcut list works from here too; it gives the keyboard back to this window
            VK_OEM_2 if ctrl && !alt => {
                if m.lParam.0 & (1 << 30) == 0 {
                    toggle_keys();
                }
                true
            }
            VK_RETURN if ctrl => {
                do_rename(h);
                true
            }
            VK_RETURN if class == "button" => {
                SendMessageW(m.hwnd, BM_CLICK, None, None);
                true
            }
            VK_RETURN if id == ID_PNAME => {
                save_preset(h);
                true
            }
            VK_RETURN => true, // no default button: Enter never renames by accident
            VK_Z if ctrl && class != "edit" => {
                do_undo(h);
                true
            }
            VK_SPACE if id == ID_RULES && !alt => {
                toggle_rule();
                true
            }
            VK_DELETE if id == ID_RULES => {
                command(h, ID_DEL, BN_CLICKED);
                true
            }
            VK_INSERT if id == ID_RULES => {
                add_rule(h);
                true
            }
            k @ (VK_UP | VK_DOWN) if id == ID_RULES && alt => {
                move_rule(if k == VK_UP { -1 } else { 1 });
                true
            }
            _ => false,
        };
        if handled {
            return Some(true);
        }
    }
    Some(IsDialogMessageW(h, m).as_bool())
}

pub(crate) extern "system" fn rename_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => paint(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_SIZE => layout(h),
            WM_GETMINMAXINFO => {
                let dpi = app(|a| a.dpi);
                let mm = &mut *(lp.0 as *mut MINMAXINFO);
                // fits a 640 × 480 screen (geo goes tight); never more than the work area it is on
                let work = work_area(h);
                let (mw, mh) = (MIN_W * dpi / 96, MIN_H * dpi / 96);
                mm.ptMinTrackSize = POINT { x: mw.min(work.right - work.left).max(1), y: mh.min(work.bottom - work.top).max(1) };
            }
            WM_CTLCOLOREDIT => return edit_colors(wp),
            WM_CTLCOLORLISTBOX => {
                let dc = HDC(wp.0 as _);
                SetDCBrushColor(dc, cref(pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_MEASUREITEM => {
                let mi = &mut *(lp.0 as *mut MEASUREITEMSTRUCT);
                let (fh, dpi) = app(|a| (a.fh, a.dpi));
                mi.itemHeight = (fh + 10 * dpi / 96) as u32;
                return LRESULT(1);
            }
            WM_DRAWITEM => {
                draw_item(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_COMMAND => command(h, (wp.0 & 0xffff) as i32, (wp.0 >> 16 & 0xffff) as u32),
            WM_NOTIFY => {
                let hdr = &*(lp.0 as *const NMHDR);
                if hdr.idFrom != ID_LIST as usize {
                    return LRESULT(0);
                }
                if hdr.code == LVN_GETDISPINFOW {
                    let di = &mut *(lp.0 as *mut NMLVDISPINFOW);
                    if di.item.mask.0 & LVIF_TEXT.0 != 0 && !di.item.pszText.is_null() && di.item.cchTextMax > 0 {
                        let (k, col) = (di.item.iItem as usize, di.item.iSubItem);
                        let t = rn(|r| match (r.items.get(k), r.rows.get(k)) {
                            (Some((_, s)), Some((n, st))) => match col {
                                0 => s.name.clone(),
                                1 => n.clone(),
                                _ => status_text(*st).to_string(),
                            },
                            _ => String::new(),
                        });
                        let max = di.item.cchTextMax as usize - 1;
                        let w: Vec<u16> = t.encode_utf16().take(max).chain([0]).collect();
                        std::ptr::copy_nonoverlapping(w.as_ptr(), di.item.pszText.0, w.len());
                    }
                } else if hdr.code == NM_CUSTOMDRAW {
                    let cd = &mut *(lp.0 as *mut NMLVCUSTOMDRAW);
                    let stage = cd.nmcd.dwDrawStage.0;
                    if stage == CDDS_PREPAINT.0 {
                        return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
                    }
                    if stage == CDDS_ITEMPREPAINT.0 {
                        let st = rn(|r| r.rows.get(cd.nmcd.dwItemSpec).map(|x| x.1));
                        let ink = match st {
                            Some(s) if s.blocks() => red(),
                            Some(Status::Same) => pal().sub,
                            _ => pal().text,
                        };
                        (cd.clrText, cd.clrTextBk) = (cref(ink), cref(pal().card));
                        return LRESULT(CDRF_NEWFONT as isize);
                    }
                }
            }
            WM_ACTIVATE => {
                // the field that had the keyboard gets it back (DefWindowProc would leave it on the window)
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE {
                    let f = GetFocus();
                    if IsChild(h, f).as_bool() {
                        rn(|r| r.focus = f);
                    }
                } else {
                    let f = rn(|r| r.focus);
                    if IsWindow(Some(f)).as_bool() && IsWindowVisible(f).as_bool() {
                        _ = SetFocus(Some(f));
                    } else {
                        _ = SetFocus(Some(item(h, ID_RULES)));
                    }
                }
                return LRESULT(0);
            }
            WM_CLOSE => _ = DestroyWindow(h),
            WM_DESTROY => {
                let pane = rn(|r| {
                    (r.h, r.focus) = (HWND::default(), HWND::default());
                    (r.items, r.rows, r.existing) = (Vec::new(), Vec::new(), Vec::new());
                    r.pane
                });
                let main = app(|a| a.main);
                _ = SetForegroundWindow(main);
                focus_view(pane);
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_layout() {
        // at the smallest window size, at 100 / 150 / 200 %: everything inside, nothing on top of anything else
        // client of the smallest window (minus frame and caption), the opening size, and in between
        for (dpi, cw, ch) in [96, 144, 192].into_iter().flat_map(|d| [(d, MIN_W - 16, MIN_H - 39), (d, 720, 470), (d, 964, 601)]) {
            let (w, h, fh) = (cw * dpi / 96, ch * dpi / 96, 16 * dpi / 96);
            let g = geo(w, h, dpi, fh);
            let inside = |r: &RECT| r.left >= 0 && r.top >= 0 && r.right <= w && r.bottom <= h && r.left < r.right && r.top < r.bottom;
            for (id, r) in &g.ctl {
                assert!(inside(r), "{id} at {dpi}");
            }
            assert!(g.note.right - g.note.left > 60 * dpi / 96 && g.hint.bottom <= g.ctl.iter().find(|(id, _)| *id == ID_PNAME).unwrap().1.top);
            for (a, ra) in &g.ctl {
                // the mode button shares its row with the two check boxes (never shown together)
                for (b, rb) in g.ctl.iter().filter(|(b, _)| b > a && !(*a == ID_MODE && (*b == ID_FX || *b == ID_FY))) {
                    let apart = ra.right <= rb.left || rb.right <= ra.left || ra.bottom <= rb.top || rb.bottom <= ra.top;
                    assert!(apart, "{a} and {b} overlap at {dpi}");
                }
            }
            let lb = g.ctl.iter().find(|(id, _)| *id == ID_RULES).unwrap().1;
            assert!(lb.bottom - lb.top >= 40 * dpi / 96);
        }
    }
}
