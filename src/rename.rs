use crate::*;
use crate::rename_rules::*;
use std::collections::{HashMap, HashSet};
use std::os::windows::fs::MetadataExt;
use windows::Win32::Storage::FileSystem::{CreateFileW, DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime, TIME_ZONE_INFORMATION};
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
const ID_LIVE: i32 = 3028; // zero size, a polite live region: the counts once typing stops, a batch's start and outcome
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
    running: Option<Run>,           // a batch (or its undo) on the worker thread: the window is locked
    desc: String,                   // the counts line as last given to screen readers (Rename's, the preview's description)
    said: String,                   // the one last read out (or not to be: at opening, after an outcome)
    taken_read: bool,               // this batch's dates taken are read, or being read (once per batch: start_taken)
    reading: Option<(usize, usize)>, // dates taken being read: files done, of; Rename and Undo stay off meanwhile
}

thread_local! {
    static RN: RefCell<Rn> = RefCell::new(Rn {
        h: HWND::default(), pane: 0, rules: vec![Rule::new(Kind::Replace)], cur: 0, ext: false, dirs: Vec::new(), items: Vec::new(),
        existing: Vec::new(), rows: Vec::new(), err: None, note: String::new(), undo: Vec::new(), loading: false, focus: HWND::default(),
        running: None, desc: String::new(), said: String::new(), taken_read: false, reading: None,
    });
}

// Same rule as app(): no Win32 call that can come back into rn_proc inside.
fn rn<R>(f: impl FnOnce(&mut Rn) -> R) -> R {
    RN.with(|r| f(&mut r.borrow_mut()))
}

// Panes moved (a layout change, a swap): the pane the window was opened from, under its new number. Outside app().
pub(crate) fn rename_pane_moved(to: impl Fn(usize) -> usize) {
    rn(|r| r.pane = to(r.pane));
}

// The UI language now; the _in forms take it, so the tests can ask for both.
fn ko_now() -> bool {
    KO.load(Ordering::Relaxed)
}

fn lang(ko: bool, en: &'static str, k: &'static str) -> &'static str {
    if ko { k } else { en }
}

fn kind_name(k: Kind) -> &'static str {
    kind_name_in(k, ko_now())
}

fn kind_name_in(kd: Kind, ko: bool) -> &'static str {
    let tr = |en, k| lang(ko, en, k);
    match kd {
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
    spec_in(k, ko_now())
}

fn spec_in(k: Kind, ko: bool) -> Spec {
    let tr = |en, k| lang(ko, en, k);
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
            flags: [tr("Photos: date taken (else modified)", "사진은 찍은 날짜 (없으면 수정한 날짜)"), ""],
            hint: tr("Date: yyyy yy MM dd HH mm ss", "날짜: yyyy yy MM dd HH mm ss"),
            ..none
        },
        Kind::Case => Spec { mode: tr("Case", "대소문자"), modes: vec!["lowercase", "UPPERCASE", "Title Case"], ..none },
        Kind::Trim => Spec { hint: tr("Spaces at both ends and doubled spaces", "양 끝 공백과 겹친 공백"), ..none },
    }
}

// One line in the rule list.
fn summary(r: &Rule) -> String {
    summary_in(r, ko_now())
}

fn summary_in(r: &Rule, ko: bool) -> String {
    let sp = spec_in(r.kind, ko);
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
        // words only: this line is what screen readers read for the row (rule_speech)
        Kind::Date if r.x => format!("{} · {mode} · {}", r.f[0], lang(ko, "date taken", "찍은 날짜")),
        Kind::Date => format!("{} · {mode}", r.f[0]),
        Kind::Case => mode.to_string(),
        Kind::Trim => String::new(),
    };
    let kind = kind_name_in(r.kind, ko);
    if detail.is_empty() { kind.into() } else { format!("{kind}: {detail}") }
}

fn on_word(on: bool, ko: bool) -> &'static str {
    if on { lang(ko, "on", "켬") } else { lang(ko, "off", "끔") }
}

// A rule row's text, which screen readers read (the row draws the same number and summary, with a check glyph);
// the reader adds where it is in the list ("2 of 3").
fn rule_speech(num: usize, r: &Rule, ko: bool) -> String {
    format!("{num}. {} · {}", summary_in(r, ko), on_word(r.on, ko))
}

// The counts line above the preview; also the description of Rename and the preview, read out once typing stops.
fn sum_text(n: usize, changed: usize, bad: usize, err: Option<&str>, ko: bool) -> String {
    match (err, ko) {
        (Some(e), false) => format!("Bad regular expression: {}", e.lines().last().unwrap_or("")),
        (Some(e), true) => format!("정규식 오류: {}", e.lines().last().unwrap_or("")),
        (None, false) => format!("{n} items · {changed} to rename · {bad} problems"),
        (None, true) => format!("{n}개 · 바꿀 것 {changed} · 문제 {bad}"),
    }
}

// What a drawn button shows, and whether in the icon font.
fn button_label(id: i32, rule: Option<&Rule>, ko: bool) -> (String, bool) {
    let tr = |en, k| lang(ko, en, k);
    match id {
        ID_ADD => (format!("{} ▾", tr("Add", "추가")), false),
        ID_DEL => (tr("Remove", "빼기").into(), false),
        ID_RUP => ("\u{E74A}".into(), true),
        ID_RDOWN => ("\u{E74B}".into(), true),
        ID_ONOFF => (tr("On / off", "켜기/끄기").into(), false),
        ID_MODE => (rule.map(|r| format!("{} ▾", spec_in(r.kind, ko).modes.get(r.mode as usize).copied().unwrap_or(""))).unwrap_or_default(), false),
        ID_PSAVE => (tr("Save", "저장").into(), false),
        ID_PMENU => (format!("{} ▾", tr("Presets", "프리셋")), false),
        ID_UNDO => (tr("Undo", "되돌리기").into(), false),
        ID_GO => (tr("Rename", "이름 바꾸기").into(), false),
        ID_CLOSE => (tr("Close", "닫기").into(), false),
        _ => (String::new(), false),
    }
}

// The same button's name for a screen reader (its window text): words for the arrows, no ▾, and the mode
// button says what it is for ("Where: At the end").
fn button_name(id: i32, rule: Option<&Rule>, ko: bool) -> String {
    match id {
        ID_RUP => lang(ko, "Move rule up", "규칙 위로").into(),
        ID_RDOWN => lang(ko, "Move rule down", "규칙 아래로").into(),
        ID_MODE => match rule.map(|r| (r, spec_in(r.kind, ko))) {
            Some((r, sp)) if !sp.modes.is_empty() => format!("{}: {}", sp.mode, sp.modes.get(r.mode as usize).copied().unwrap_or("")),
            _ => String::new(), // hidden: this rule has no mode
        },
        _ => button_label(id, rule, ko).0.trim_end_matches(" ▾").to_string(),
    }
}

// The drawn check boxes: on or off, and their label. None: not one, or this rule doesn't use it.
fn check_state(id: i32, rule: Option<&Rule>, ext: bool, ko: bool) -> Option<(bool, &'static str)> {
    let (on, label) = match id {
        ID_EXTOK => (ext, lang(ko, "Include extension", "확장자도 바꾸기")),
        ID_FX | ID_FY => rule.map(|r| (if id == ID_FX { r.x } else { r.y }, spec_in(r.kind, ko).flags[(id - ID_FX) as usize]))?,
        _ => return None,
    };
    (!label.is_empty()).then_some((on, label))
}

// Their name for a screen reader, the state in it ("Include extension: off"): a drawn button has no checked state.
fn check_name(id: i32, rule: Option<&Rule>, ext: bool, ko: bool) -> Option<String> {
    check_state(id, rule, ext, ko).map(|(on, label)| format!("{label}: {}", on_word(on, ko)))
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
    x_row: RECT,      // the first check box of a date rule: the third field row, which that rule leaves empty (place_x)
}

// Left: rule list (takes the height that is left, 70..170 DIP), its buttons, the rule's fields, presets.
// Right: the preview. Bottom: extension option, result line, Undo / Rename / Close.
// A kind has a mode or the two check boxes, never both, so they share a row; the date rule has both, so its
// check box goes to the third field row instead (x_row, place_x).
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
    let x_row = r(x0, labels[2].top, x1, labels[2].bottom);
    Geo { ctl, titles: [t_rules, t_edit, t_prev], summary, labels, fields, hint, note, pname, lists: [out(lb), out(lv)], x_row }
}

// Where the first check box goes: a date rule has a mode (its Place button fills the mode row), so its box takes
// the third field row it leaves empty; every other kind keeps it in the mode row. The tab order follows the place.
unsafe fn place_x(h: HWND, g: &Geo) {
    let date = rn(|r| r.rules.get(r.cur).is_some_and(|x| x.kind == Kind::Date));
    let Some(&(_, row)) = g.ctl.iter().find(|(id, _)| *id == ID_FX) else { return };
    let rc = if date { g.x_row } else { row };
    let after = item(h, if date { ID_F0 + 1 } else { ID_MODE });
    _ = SetWindowPos(item(h, ID_FX), Some(after), rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOACTIVATE);
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

// Smallest window size in pixels at this DPI, never more than the work area (WM_GETMINMAXINFO, rename_refont).
fn min_size(dpi: i32, work: RECT) -> (i32, i32) {
    ((MIN_W * dpi / 96).min(work.right - work.left).max(1), (MIN_H * dpi / 96).min(work.bottom - work.top).max(1))
}

// One rule row: the text plus its padding (WM_MEASUREITEM, and again after a DPI change).
fn rule_row_h(fh: i32, dpi: i32) -> i32 {
    fh + 10 * dpi / 96
}

// After a DPI change, from apply_font before the old font is deleted: every control takes the new font
// (the list view passes it on to its header), the rule rows their new height, the window its new smallest size.
pub(crate) unsafe fn rename_refont() {
    let h = rn(|r| r.h);
    if h.0.is_null() {
        return;
    }
    let (font, fh, dpi) = app(|a| (a.font, a.fh, a.dpi));
    let mut c = GetWindow(h, GW_CHILD).unwrap_or_default();
    while !c.0.is_null() {
        SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        c = GetWindow(c, GW_HWNDNEXT).unwrap_or_default();
    }
    let lb = item(h, ID_RULES);
    SendMessageW(lb, LB_SETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(rule_row_h(fh, dpi) as isize)));
    _ = InvalidateRect(Some(lb), None, true); // the parent clips its children: layout() alone leaves the old rows
    if !IsIconic(h).as_bool() && !IsZoomed(h).as_bool() {
        let work = work_area(h);
        let (mw, mh) = min_size(dpi, work);
        let mut rc = RECT::default();
        _ = GetWindowRect(h, &mut rc);
        let (w, ht) = ((rc.right - rc.left).max(mw), (rc.bottom - rc.top).max(mh));
        if (w, ht) != (rc.right - rc.left, rc.bottom - rc.top) {
            let x = rc.left.min(work.right - w).max(work.left);
            let y = rc.top.min(work.bottom - ht).max(work.top);
            _ = SetWindowPos(h, None, x, y, w, ht, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
    layout(h);
}

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
    let g = geo_now(h);
    for (id, r) in &g.ctl {
        _ = MoveWindow(item(h, *id), r.left, r.top, r.right - r.left, r.bottom - r.top, false);
    }
    place_x(h, &g);
    _ = InvalidateRect(Some(h), None, false);
}

// ---------- opening: the active pane's selection, or every file in its folder ----------

// A FILETIME (UTC) as local [year, month, day, hour, minute, second]; [0; 6] if it can't be converted.
// The one conversion for every date Grids shows (preview, Everything results, rename, table copy): each date
// gets the daylight-saving rule of its own day, as Explorer's Date modified column does. FileTimeToLocalFileTime
// would apply today's offset to a date in the other season and be an hour off.
pub(crate) fn local_date(ft: u64) -> [u16; 6] {
    local_date_in(ft, None)
}

// The same in a given time zone (None = the system's); the tests pin one with daylight saving.
pub(crate) fn local_date_in(ft: u64, tz: Option<&TIME_ZONE_INFORMATION>) -> [u16; 6] {
    let ft = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
    let (mut utc, mut st) = (SYSTEMTIME::default(), SYSTEMTIME::default());
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(tz.map(|t| t as *const _), &utc, &mut st).is_err() {
            return [0; 6];
        }
    }
    [st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond]
}

// [2026, 3, 8, 1, 5, ..] -> "2026-03-08 01:05", the way every date in Grids is written.
pub(crate) fn date_min(d: [u16; 6]) -> String {
    format!("{:04}-{:02}-{:02} {:02}:{:02}", d[0], d[1], d[2], d[3], d[4])
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
        items.push((d, Src { name: name.to_string_lossy().into_owned(), is_dir, date: file_date(&path), taken: None }));
    }
    Some((dirs, items))
}

pub(crate) unsafe fn open_rename(i: usize) {
    // a batch still running: its files stay, the window only comes to the front
    if let Some(h) = rn(|r| r.running.as_ref().map(|_| r.h)) {
        if !h.0.is_null() {
            _ = SetForegroundWindow(h);
        }
        return;
    }
    recover(app(|a| a.main)); // an earlier batch cut off, its drive back since: finished first
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
        new_batch(r);
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
    // rows with their text too (rule_speech): drawn from RN, read out by screen readers
    let lb = (LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32 | WS_VSCROLL.0 | tab;
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
    // not in layout(): stays 0 × 0, and no tab stop; a file name's & in an outcome is no access key
    const SS_NOPREFIX: u32 = 0x80;
    let live = child(h, w!("STATIC"), w!(""), SS_NOPREFIX, WINDOW_EX_STYLE(0), ID_LIVE, HFONT::default());
    live_region(live, true);
    rename_theme();
    layout(h);
    fill_rules();
    load_editor();
    update_preview();
    let lbh = item(h, ID_RULES);
    // the counts at opening are not read out: the window and the rule list are, just then
    rn(|r| (r.focus, r.said) = (lbh, r.desc.clone()));
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
    set_caption(h);
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
    // what screen readers read, in the new language (not read out now); rows only if their text changed
    refresh_rows();
    refresh_names(h);
    name_summary(h, true);
    _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
}

// The live region's annotation, on when made and off before it goes (as the shortcut list's). Failing only loses the reading.
unsafe fn live_region(h: HWND, on: bool) {
    if h.0.is_null() {
        return;
    }
    let Ok(acc) = CoCreateInstance::<_, IAccPropServices>(&CLSID_AccPropServices, None, CLSCTX_INPROC_SERVER) else { return };
    let (obj, me) = (OBJID_CLIENT.0 as u32, CHILDID_SELF);
    _ = if on { acc.SetHwndProp(h, obj, me, LiveSetting_Property_GUID, &VARIANT::from(Polite.0)) } else { acc.ClearHwndProps(h, obj, me, &[LiveSetting_Property_GUID]) };
}

// Read out by itself, wherever the keyboard is. Never inside rn(): the reader may call straight back in.
unsafe fn say(h: HWND, t: &str) {
    let live = item(h, ID_LIVE);
    if t.is_empty() || live.0.is_null() {
        return;
    }
    _ = SetWindowTextW(live, &HSTRING::from(t));
    NotifyWinEvent(EVENT_OBJECT_LIVEREGIONCHANGED, live, OBJID_CLIENT.0, CHILDID_SELF as i32);
}

// Names for screen readers, from the rule picked and the language: the buttons' window text (button_name: the
// drawn ones would read as "button" alone), the check boxes with their state, the fields by their labels.
// After a rule, its mode or the language changes. Outside rn().
unsafe fn refresh_names(h: HWND) {
    let ko = ko_now();
    let (rule, ext) = rn(|r| (r.rules.get(r.cur).cloned(), r.ext));
    for id in [ID_ADD, ID_DEL, ID_RUP, ID_RDOWN, ID_ONOFF, ID_MODE, ID_PSAVE, ID_PMENU, ID_UNDO, ID_GO, ID_CLOSE] {
        let b = item(h, id);
        let t = button_name(id, rule.as_ref(), ko);
        if text_of(b) != t {
            _ = SetWindowTextW(b, &HSTRING::from(t)); // only when it changed: it repaints the button
        }
    }
    for id in [ID_FX, ID_FY, ID_EXTOK] {
        a11y_name(item(h, id), check_name(id, rule.as_ref(), ext, ko).as_deref());
    }
    let sp = rule.as_ref().map(|r| spec_in(r.kind, ko));
    for k in 0..4 {
        let label = sp.as_ref().map_or("", |s| s.fields[k]);
        a11y_name(item(h, ID_F0 + k as i32), (!label.is_empty()).then_some(label));
    }
    a11y_name(item(h, ID_PNAME), Some(lang(ko, "Preset name", "프리셋 이름")));
    let lb = item(h, ID_RULES);
    a11y_name(lb, Some(lang(ko, "Rules", "규칙 목록")));
    a11y_desc(lb, Some(lang(ko, "Space on/off · Ins add · Del remove · Alt+↑/↓ move", "Space 켜기/끄기 · Ins 추가 · Del 빼기 · Alt+↑/↓ 순서")));
    a11y_name(item(h, ID_LIST), Some(lang(ko, "Preview", "미리보기")));
}

// The counts line as the description of Rename and of the preview (Rename is off, and skipped by Tab, while
// there are problems or a batch runs; the preview never is). A new one is read out once typing stops
// (TIMER_SAY, restarted by each change); `settle` gives it without reading it out.
unsafe fn name_summary(h: HWND, settle: bool) {
    if h.0.is_null() {
        return;
    }
    let ko = ko_now();
    let (sum, fresh, quiet) = rn(|r| {
        let (n, changed, bad) = counts(r);
        // while dates taken are read, that is why Rename is off: said instead of the counts
        let s = match r.reading {
            Some((done, total)) => taken_text(done, total, ko),
            None => sum_text(n, changed, bad, r.err.as_deref(), ko),
        };
        let fresh = r.desc != s;
        r.desc = s.clone();
        if settle {
            r.said = s.clone();
        }
        let quiet = r.said == s;
        (s, fresh, quiet)
    });
    if fresh {
        a11y_desc(item(h, ID_GO), Some(&sum));
        a11y_desc(item(h, ID_LIST), Some(&sum));
    }
    if !quiet {
        SetTimer(Some(h), TIMER_SAY, 400, None);
    }
}

// TIMER_SAY: rows typed into get their new text, then the counts, once, if they are still new (never while a
// batch runs: its start and outcome are read).
unsafe fn say_summary(h: HWND) {
    _ = KillTimer(Some(h), TIMER_SAY);
    refresh_rows();
    let t = rn(|r| {
        let new = r.running.is_none() && r.said != r.desc;
        new.then(|| {
            r.said = r.desc.clone();
            r.desc.clone()
        })
    });
    if let Some(t) = t {
        say(h, &t);
    }
}

// ---------- state -> controls ----------

unsafe fn fill_rules() {
    let ko = ko_now();
    let (h, rows, cur) = rn(|r| (r.h, r.rules.iter().enumerate().map(|(k, x)| rule_speech(k + 1, x, ko)).collect::<Vec<_>>(), r.cur));
    let lb = item(h, ID_RULES);
    SendMessageW(lb, LB_RESETCONTENT, None, None);
    for t in rows {
        let t = HSTRING::from(t);
        SendMessageW(lb, LB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
    }
    SendMessageW(lb, LB_SETCURSEL, Some(WPARAM(cur)), None);
}

// Row texts after rules changed in place (on/off, a field, a mode, the language): the ones that differ are put
// in again, the picked one still picked, so a reader on the list reads it anew. Same scroll position; none
// touched when nothing differs (a theme-only switch).
unsafe fn refresh_rows() {
    let ko = ko_now();
    let (h, cur, rows) = rn(|r| (r.h, r.cur, r.rules.iter().enumerate().map(|(k, x)| rule_speech(k + 1, x, ko)).collect::<Vec<_>>()));
    let lb = item(h, ID_RULES);
    if SendMessageW(lb, LB_GETCOUNT, None, None).0 != rows.len() as isize {
        fill_rules();
        return;
    }
    let stale: Vec<usize> = (0..rows.len())
        .filter(|&k| {
            let len = SendMessageW(lb, LB_GETTEXTLEN, Some(WPARAM(k)), None).0;
            if len < 0 {
                return true;
            }
            let mut buf = vec![0u16; len as usize + 1];
            let n = SendMessageW(lb, LB_GETTEXT, Some(WPARAM(k)), Some(LPARAM(buf.as_mut_ptr() as isize))).0.max(0) as usize;
            String::from_utf16_lossy(&buf[..n.min(len as usize)]) != rows[k]
        })
        .collect();
    if stale.is_empty() {
        return;
    }
    let top = SendMessageW(lb, LB_GETTOPINDEX, None, None).0.max(0) as usize;
    SendMessageW(lb, WM_SETREDRAW, Some(WPARAM(0)), None);
    for k in stale {
        SendMessageW(lb, LB_DELETESTRING, Some(WPARAM(k)), None);
        let t = HSTRING::from(rows[k].as_str());
        SendMessageW(lb, LB_INSERTSTRING, Some(WPARAM(k)), Some(LPARAM(t.as_ptr() as isize)));
    }
    SendMessageW(lb, LB_SETTOPINDEX, Some(WPARAM(top)), None);
    SendMessageW(lb, LB_SETCURSEL, Some(WPARAM(cur)), None);
    SendMessageW(lb, WM_SETREDRAW, Some(WPARAM(1)), None);
    _ = InvalidateRect(Some(lb), None, false);
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
    place_x(h, &geo_now(h));
    let any = rule.is_some();
    for id in [ID_DEL, ID_RUP, ID_RDOWN, ID_ONOFF] {
        _ = EnableWindow(item(h, id), any);
    }
    rn(|r| r.loading = false);
    refresh_names(h);
    _ = RedrawWindow(Some(h), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN);
}

// New names and their checks for every item; the Rename button is only on when all of them are fine.
unsafe fn update_preview() {
    start_taken();
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
        // both stay off while a batch runs, whoever calls this, and while dates taken are read (the names wait for
        // them; a file renamed meanwhile would lose its date taken)
        let idle = r.running.is_none() && r.reading.is_none();
        (r.h, r.rows.len(), ok && idle, !r.undo.is_empty() && idle)
    });
    let lv = item(h, ID_LIST);
    SendMessageW(lv, LVM_SETITEMCOUNT, Some(WPARAM(n)), Some(LPARAM(2))); // LVSICF_NOSCROLL
    _ = InvalidateRect(Some(lv), None, false);
    _ = InvalidateRect(Some(item(h, ID_RULES)), None, false);
    _ = EnableWindow(item(h, ID_GO), go);
    _ = EnableWindow(item(h, ID_UNDO), undo);
    _ = InvalidateRect(Some(h), None, false); // summary line
    name_summary(h, false);
}

// ---------- dates taken: read on a worker thread, once per batch, only when a date rule asks for them ----------
// Not when the window opens: most batches have no use for them, and a NAS full of photos takes a while.
// Only reading, so no Busy: closing Grids doesn't wait for it, and closing the window stops it at the next file.

const WM_TAKEN: u32 = WM_APP + 69; // to the rename window: wParam = gen; lParam = files read so far, or -1 when over
static TAKEN_GEN: AtomicUsize = AtomicUsize::new(0); // up with every new batch (or none): a reading of another stops
static TAKEN: Mutex<Option<(usize, Vec<(usize, [u16; 6])>)>> = Mutex::new(None); // gen, (item, date taken)

// The window's files were replaced (or let go), or a reading is no longer wanted: what was read for the old ones
// is dropped, a reading stops.
fn new_batch(r: &mut Rn) {
    TAKEN_GEN.fetch_add(1, Ordering::SeqCst);
    (r.taken_read, r.reading) = (false, None);
}

// A date rule that is on with "Photos: date taken" ticked.
fn wants_taken(rules: &[Rule]) -> bool {
    rules.iter().any(|r| r.on && r.kind == Kind::Date && r.x)
}

fn taken_text(done: usize, total: usize, ko: bool) -> String {
    let (d, t) = (group_digits(done.min(total) as u64), group_digits(total as u64));
    if ko { format!("찍은 날짜 읽는 중… {d} / {t}") } else { format!("Reading dates taken… {d} / {t}") }
}

// Over (also after a panic): the window always hears it and switches Rename back on.
struct TakenPost(isize, usize);

impl Drop for TakenPost {
    fn drop(&mut self) {
        unsafe {
            _ = PostMessageW(Some(HWND(self.0 as _)), WM_TAKEN, WPARAM(self.1), LPARAM(-1));
        }
    }
}

// The one place that decides (from update_preview, so a tick, a preset, a rule switched on again, a reopened
// window and a new batch all come here): a date rule wants the dates taken and this batch's aren't read yet.
// Photos only (by extension, not folders); an online-only file keeps its date modified, as nothing is downloaded.
unsafe fn start_taken() {
    let job = rn(|r| {
        // unticked (or the rule off or gone) while reading: the reading stops, so Rename isn't held off for a
        // NAS full of photos nobody asks the dates of any more; ticking again reads afresh
        if r.reading.is_some() && !wants_taken(&r.rules) {
            new_batch(r);
        }
        if r.taken_read || r.running.is_some() || r.h.0.is_null() || !wants_taken(&r.rules) {
            return None;
        }
        r.taken_read = true;
        let dirs = &r.dirs;
        let files: Vec<(usize, String)> = r
            .items
            .iter()
            .enumerate()
            .filter(|(_, (_, s))| !s.is_dir && photo_ext(&s.name))
            .filter_map(|(k, (d, s))| Some((k, join_path(dirs.get(*d)?, &s.name))))
            .collect();
        if files.is_empty() {
            return None;
        }
        r.reading = Some((0, files.len()));
        Some((r.h.0 as isize, TAKEN_GEN.load(Ordering::SeqCst), files))
    });
    let Some((hw, gen, files)) = job else { return };
    let spawned = std::thread::Builder::new().spawn(move || {
        let _post = TakenPost(hw, gen);
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let mut got = Vec::new();
        for (n, (k, path)) in files.iter().enumerate() {
            if TAKEN_GEN.load(Ordering::SeqCst) != gen {
                break; // the window closed or took other files
            }
            let photo = std::fs::metadata(path).is_ok_and(|m| !m.is_dir() && !cloud_only(m.file_attributes()));
            if let Some(d) = photo.then(|| date_taken(path)).flatten() {
                got.push((*k, d));
            }
            if n % 16 == 15 {
                _ = PostMessageW(Some(HWND(hw as _)), WM_TAKEN, WPARAM(gen), LPARAM(n as isize + 1));
            }
        }
        if TAKEN_GEN.load(Ordering::SeqCst) == gen {
            *TAKEN.lock().unwrap_or_else(|e| e.into_inner()) = Some((gen, got));
        }
        if com {
            CoUninitialize();
        }
    });
    if spawned.is_err() {
        rn(|r| r.reading = None); // no thread: the dates modified stand in
    }
}

// WM_TAKEN: progress, or the dates for this batch (by item, which a rename meanwhile can't reorder).
unsafe fn taken_msg(h: HWND, gen: usize, l: isize) {
    if gen != TAKEN_GEN.load(Ordering::SeqCst) {
        return; // another batch's, or the window's before it closed
    }
    if l >= 0 {
        rn(|r| {
            if let Some(p) = r.reading.as_mut() {
                p.0 = l as usize;
            }
        });
        _ = InvalidateRect(Some(h), None, false);
        return;
    }
    let got = match TAKEN.lock().unwrap_or_else(|e| e.into_inner()).take() {
        Some((g, v)) if g == gen => v,
        _ => Vec::new(),
    };
    rn(|r| {
        for (k, d) in got {
            if let Some((_, s)) = r.items.get_mut(k) {
                s.taken = Some(d);
            }
        }
        r.reading = None;
    });
    update_preview();
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
    let run = rn(|r| r.running.as_ref().map(|x| (x.undo, x.prog.done.load(Ordering::Relaxed), x.prog.total.load(Ordering::Relaxed))));
    let reading = rn(|r| r.reading);
    let ko = KO.load(Ordering::Relaxed);
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        let titles = [tr("Rules, applied in order", "규칙 (위에서부터 차례로)"), "", tr("Preview", "미리보기")];
        let edit_title = rule.as_ref().map_or(String::new(), |r| kind_name(r.kind).to_string());
        for (k, t) in titles.iter().enumerate() {
            text(dc, g.titles[k], if k == 1 { edit_title.as_str() } else { *t }, small, pal().sub, DT_LEFT);
        }
        let sum = match (run, reading) {
            (Some((undo, done, total)), _) => progress_text(undo, done, total, ko),
            (None, Some((done, total))) => taken_text(done, total, ko),
            (None, None) => sum_text(n, changed, bad, err.as_deref(), ko),
        };
        let warn = run.is_none() && reading.is_none() && (err.is_some() || bad > 0);
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

// Warning ink: the red that reads on the backdrop (also on a black or white contrast theme, where DARK_ON
// stays false); the text colour when neither red reaches 4.5:1.
fn red() -> u32 {
    let p = pal();
    let worst = |c: u32| contrast(c, p.bg).min(contrast(c, p.card));
    let c = if worst(0xC0392B) >= worst(0xF08080) { 0xC0392B } else { 0xF08080 };
    if worst(c) >= 4.5 { c } else { p.text }
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
    let ko = ko_now();
    // check boxes: glyph + label on the ground
    if let Some((on, label)) = check_state(id, rule.as_ref(), ext, ko) {
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
    let (label, icon) = button_label(id, rule.as_ref(), ko);
    let align = if id == ID_MODE { DT_LEFT } else { DT_CENTER };
    let r = if id == ID_MODE { RECT { left: rc.left + s(8), ..rc } } else { rc };
    text(dc, r, &label, if icon { icon_font } else { font }, ink, align);
}

// ---------- actions ----------

unsafe fn changed(f: impl FnOnce(&mut Rn)) {
    rn(f);
    refresh_rows();
    update_preview();
}

// A field typed in: the row text waits until typing stops (TIMER_SAY, restarted by each key), not per letter.
unsafe fn typed(h: HWND, f: impl FnOnce(&mut Rn)) {
    rn(f);
    update_preview();
    SetTimer(Some(h), TIMER_SAY, 400, None);
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
        refresh_names(h); // the mode button's name says the new pick
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

/// join_path for the file system calls themselves: with the \\?\ prefix, so a name Win32 would otherwise trim (a
/// trailing dot or space, made on a share or by WSL) is that file, not the one without the dot.
pub(crate) fn api_path(dir: &str, name: &str) -> String {
    let full = join_path(dir, name);
    if full.starts_with(r"\\?\") || full.starts_with(r"\\.\") {
        full
    } else if let Some(unc) = full.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{unc}")
    } else if full.get(1..3) == Some(r":\") {
        format!(r"\\?\{full}")
    } else {
        full
    }
}

// One rename in a folder, never over another file.
unsafe fn move_in(dir: &str, from: &str, to: &str) -> std::result::Result<(), String> {
    MoveFileExW(&HSTRING::from(api_path(dir, from)), &HSTRING::from(api_path(dir, to)), MOVE_FILE_FLAGS(0)).map_err(|e| e.message())
}

// The steps in order; if one fails, the ones done are moved back (newest first), so a batch is all or nothing.
// A move back that fails is tried once more a moment later (a file held for an instant). With a journal it then
// stops there: Err's count is the steps still done, whose temporary names Grids tidies later (recover).
// Without one the rest are still moved back: Err's count is the files left under their new or temporary name.
unsafe fn run_steps(dirs: &[String], steps: &[Move], journal: bool, done: &AtomicUsize) -> std::result::Result<(), (String, String, usize)> {
    run_steps_with(steps, journal, &mut |(d, from, to), back| {
        if !back {
            done.fetch_add(1, Ordering::Relaxed);
            return move_in(&dirs[*d], from, to);
        }
        move_in(&dirs[*d], to, from).or_else(|_| {
            std::thread::sleep(Duration::from_millis(300));
            move_in(&dirs[*d], to, from)
        })
    })
}

// Lets the panes show the new names now rather than when the folder watcher gets to it.
unsafe fn notify(dirs: &[String], moves: &[Move], items: &[(usize, Src)]) {
    for (d, old, new) in moves {
        let dir = items.iter().any(|(id, s)| id == d && (s.name == *old || s.name == *new) && s.is_dir);
        let (a, b) = (HSTRING::from(join_path(&dirs[*d], old)), HSTRING::from(join_path(&dirs[*d], new)));
        SHChangeNotify(if dir { SHCNE_RENAMEFOLDER } else { SHCNE_RENAMEITEM }, SHCNF_PATHW, Some(a.as_ptr() as _), Some(b.as_ptr() as _));
    }
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

// The outcome is read out; the counts that changed with it are not read again after it.
unsafe fn report(h: HWND, note: String, details: Option<String>) {
    rn(|r| r.note = note.clone());
    update_preview();
    rn(|r| r.said = r.desc.clone());
    say(h, &note);
    if let Some(t) = details {
        MessageBoxW(Some(h), &HSTRING::from(t), &HSTRING::from(tr("Batch rename", "일괄 이름 바꾸기")), MB_OK | MB_ICONWARNING);
    }
}

unsafe fn do_rename(h: HWND) {
    update_preview();
    let ready = rn(|r| r.err.is_none() && !r.rows.iter().any(|(_, s)| s.blocks()) && r.reading.is_none());
    let (dirs, moves) = rn(|r| {
        let m: Vec<Move> = r.items.iter().zip(&r.rows).filter(|(_, (_, s))| *s == Status::Ok).map(|((d, s), (n, _))| (*d, s.name.clone(), n.clone())).collect();
        (r.dirs.clone(), m)
    });
    if !ready || moves.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let nonce = nonce();
    let steps = plan(&moves, &tmp_name(nonce));
    let total = moves.len() + steps.len(); // the in-use checks, then the steps
    start(h, Job::Rename { dirs, moves, steps, nonce }, total, Vec::new());
}

// Last batch back to the old names (what is skipped and why: run_job).
unsafe fn do_undo(h: HWND) {
    // not while dates taken are read: a file renamed meanwhile would lose its date taken
    let undo = rn(|r| if r.reading.is_some() { Vec::new() } else { std::mem::take(&mut r.undo) });
    if undo.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    start(h, Job::Undo(undo.clone()), undo.len(), undo);
}

// ---------- a batch runs on a worker thread: thousands of files on a share or a slow USB stick don't freeze Grids ----------
// Like the copies (Busy, keep_job): the status line says files are moving, and closing Grids or a restart waits for
// the batch, so it is never cut off half-way with ~grids-rename-….tmp names left behind.

const WM_RENAMED: u32 = WM_APP + 55; // to the rename window: run wParam is over, its outcome is in DONE
const TIMER_RUN: usize = 1; // the progress line, every 100 ms
const TIMER_SAY: usize = 2; // typed rows refreshed and the counts read out, 400 ms after the last change

#[derive(Default)]
struct Prog {
    done: AtomicUsize,
    total: AtomicUsize, // checks + steps; an undo adds its share once it knows what it can put back
}

struct Run {
    id: usize,
    undo: bool,
    prog: Arc<Prog>,
    back: Vec<(String, String, String)>, // an undo's list: r.undo again when nothing was put back
}

enum Job {
    Rename { dirs: Vec<String>, moves: Vec<Move>, steps: Vec<Move>, nonce: u128 },
    Undo(Vec<(String, String, String)>),
}

enum Outcome {
    InUse(Vec<String>),            // nothing moved: these can't be renamed now
    // a step failed (name, why): the ones done were put back, but for this many files; true: those are under
    // temporary names a journal tidies later, false: no journal, they stay under their new or temporary name
    Failed(String, String, usize, bool),
    Done { dirs: Vec<String>, moves: Vec<Move>, skipped: Vec<String>, existing: Vec<HashSet<String>> },
}

// The worker's outcome, tagged with its run.
static DONE: Mutex<Option<(usize, Outcome)>> = Mutex::new(None);

// Only the run the window waits for; an outcome left from another one is dropped, never applied.
fn take_outcome(slot: &mut Option<(usize, Outcome)>, id: usize) -> Option<Outcome> {
    match slot.take() {
        Some((k, o)) if k == id => Some(o),
        _ => None,
    }
}

// A second batch while one runs is refused (and handed back).
fn begin(slot: &mut Option<Run>, run: Run) -> std::result::Result<(), Run> {
    if slot.is_some() {
        return Err(run);
    }
    *slot = Some(run);
    Ok(())
}

fn end(slot: &mut Option<Run>, id: usize) -> Option<Run> {
    if slot.as_ref().is_some_and(|r| r.id == id) { slot.take() } else { None }
}

// While a batch runs: the keys that would close the window, rename, undo, save a preset or edit the rules.
// Tab, the arrows in the preview and Ctrl+/ still work.
fn refused_key(k: VIRTUAL_KEY, ctrl: bool, in_edit: bool, on_rules: bool) -> bool {
    matches!(k, VK_ESCAPE | VK_RETURN) || (k == VK_Z && ctrl && !in_edit) || on_rules
}

fn progress_text(undo: bool, done: usize, total: usize, ko: bool) -> String {
    let (d, t) = (group_digits(done.min(total) as u64), group_digits(total as u64));
    match (undo, ko) {
        (false, false) => format!("Renaming… {d} / {t}"),
        (false, true) => format!("이름 바꾸는 중… {d} / {t}"),
        (true, false) => format!("Putting back… {d} / {t}"),
        (true, true) => format!("되돌리는 중… {d} / {t}"),
    }
}

// Dropped last on the worker, after a panic too: the window always hears the run is over and unlocks.
// A window already gone (Grids closed meanwhile) makes the post fail, which is fine.
struct Post(isize, usize);

impl Drop for Post {
    fn drop(&mut self) {
        unsafe {
            _ = PostMessageW(Some(HWND(self.0 as _)), WM_RENAMED, WPARAM(self.1), LPARAM(0));
        }
    }
}

// Everything that touches the disk, in the old order: every check first, then the steps (all or nothing),
// the shell told, the folders read again. No RN here (the worker's is empty): all of it comes in.
// `items` and `wdirs` are the window's files and folders.
unsafe fn run_job(job: Job, items: &[(usize, Src)], wdirs: &[String], p: &Prog) -> Outcome {
    let tick = || {
        p.done.fetch_add(1, Ordering::Relaxed);
    };
    let busy = |dirs: &[String], (d, o, _): &Move| in_use(&api_path(&dirs[*d], o)).map(|e| format!("{o}: {}", e.trim()));
    let (dirs, moves, steps, skipped, nonce) = match job {
        Job::Rename { dirs, moves, steps, nonce } => {
            let held: Vec<String> = moves
                .iter()
                .filter_map(|m| {
                    tick();
                    busy(&dirs, m)
                })
                .collect();
            if !held.is_empty() {
                return Outcome::InUse(held);
            }
            (dirs, moves, steps, Vec::new(), nonce)
        }
        // Items renamed or removed since are skipped and listed, as are old names another file has taken.
        Job::Undo(undo) => {
            let mut dirs: Vec<String> = Vec::new();
            let mut moves: Vec<Move> = Vec::new();
            let mut skipped: Vec<String> = Vec::new();
            for (dir, old, new) in &undo {
                tick();
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
            let nonce = nonce();
            let steps = plan(&ok, &tmp_name(nonce));
            p.total.fetch_add(ok.len() + steps.len(), Ordering::Relaxed);
            for m in &ok {
                tick();
                if let Some(b) = busy(&dirs, m) {
                    return Outcome::InUse(vec![b]);
                }
            }
            (dirs, ok, steps, skipped, nonce)
        }
    };
    // only a batch with temporary names (steps beyond one per file) can leave odd names behind
    // names never move in a batch and a tidy-up (recover) at once: a batch started meanwhile waits here
    let moving = MOVES.lock().unwrap_or_else(|e| e.into_inner());
    let journal = if steps.len() > moves.len() { write_journal(nonce, &dirs, &steps) } else { None };
    let ran = run_steps(&dirs, &steps, journal.is_some(), &p.done);
    let journaled = journal.is_some();
    if let Some((f, path)) = journal {
        drop(f);
        if matches!(ran, Err((_, _, left)) if left > 0) {
            PENDING.store(true, Ordering::Relaxed);
        } else {
            _ = std::fs::remove_file(path);
        }
    }
    drop(moving);
    if let Err((name, e, left)) = ran {
        if !journaled {
            return Outcome::Failed(name, e, left, false); // files not put back, nothing to tidy them later
        }
        // files still under a temporary name: moved aside and not yet on
        let pre = format!("~grids-rename-{nonce}-");
        let odd: isize = steps[..left].iter().map(|(_, f, t)| t.starts_with(&pre) as isize - f.starts_with(&pre) as isize).sum();
        return Outcome::Failed(name, e, odd.max(0) as usize, true);
    }
    notify(&dirs, &moves, items);
    Outcome::Done { existing: wdirs.iter().map(|d| folder_names(d)).collect(), dirs, moves, skipped }
}

// Everything that would change the batch goes off while it runs; the preview list and Close stay (Close only beeps).
unsafe fn lock(h: HWND, on: bool) {
    if on {
        _ = SetFocus(Some(item(h, ID_LIST))); // a disabled field would leave the keyboard nowhere
    }
    let ids = [ID_RULES, ID_ADD, ID_DEL, ID_RUP, ID_RDOWN, ID_ONOFF, ID_F0, ID_F0 + 1, ID_F0 + 2, ID_F0 + 3, ID_MODE, ID_FX, ID_FY];
    for id in ids.into_iter().chain([ID_EXTOK, ID_PNAME, ID_PSAVE, ID_PMENU, ID_UNDO, ID_GO]) {
        _ = EnableWindow(item(h, id), !on);
    }
    if !on {
        load_editor(); // Delete / Up / Down / On-off by the rules, Rename / Undo by the preview
        update_preview();
    }
    _ = InvalidateRect(Some(h), None, false);
}

// Locks the window and hands the job to a worker thread; with no thread, it runs right here as it always did.
unsafe fn start(h: HWND, job: Job, total: usize, back: Vec<(String, String, String)>) {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let id = RUNS.fetch_add(1, Ordering::Relaxed) + 1;
    let prog = Arc::new(Prog { done: AtomicUsize::new(0), total: AtomicUsize::new(total) });
    let run = Run { id, undo: matches!(job, Job::Undo(_)), prog: prog.clone(), back };
    if let Err(run) = rn(|r| begin(&mut r.running, run)) {
        if run.undo {
            rn(|r| r.undo = run.back);
        }
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let (items, wdirs) = rn(|r| {
        r.note = String::new();
        (r.items.clone(), r.dirs.clone())
    });
    lock(h, true);
    SetTimer(Some(h), TIMER_RUN, 100, None);
    // read out once at the start (not every 100 ms), the outcome by report
    say(h, &progress_text(matches!(job, Job::Undo(_)), 0, total, ko_now()));
    // the worker takes the job from here; if it never starts, this thread does
    let work = Arc::new(Mutex::new(Some((job, items, wdirs))));
    let (w2, p2, hw, busy) = (work.clone(), prog.clone(), h.0 as isize, Busy::new());
    let spawned = std::thread::Builder::new().spawn(move || {
        let _post = Post(hw, id);
        let _busy = busy;
        let taken = w2.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some((job, items, wdirs)) = taken {
            let out = run_job(job, &items, &wdirs, &p2);
            *DONE.lock().unwrap_or_else(|e| e.into_inner()) = Some((id, out));
        }
    });
    match spawned {
        Ok(j) => keep_job(j),
        Err(_) => {
            let taken = work.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some((job, items, wdirs)) = taken {
                let out = run_job(job, &items, &wdirs, &prog);
                *DONE.lock().unwrap_or_else(|e| e.into_inner()) = Some((id, out));
            }
            finished(h, id);
        }
    }
}

// WM_RENAMED: the window unlocks and shows the outcome the way the batch always did.
unsafe fn finished(h: HWND, id: usize) {
    let Some(run) = rn(|r| end(&mut r.running, id)) else { return };
    _ = KillTimer(Some(h), TIMER_RUN);
    let out = take_outcome(&mut DONE.lock().unwrap_or_else(|e| e.into_inner()), id);
    lock(h, false);
    let ko = KO.load(Ordering::Relaxed);
    let put_back = tr("Nothing put back", "되돌리지 않았음");
    if run.undo && !matches!(out, Some(Outcome::Done { .. })) {
        rn(|r| r.undo = run.back);
    }
    match out {
        // the worker stopped without an outcome (a panic)
        None => report(h, tr("Batch rename stopped unexpectedly: check the files", "일괄 이름 바꾸기가 예기치 않게 멈춤: 파일을 확인하세요").into(), None),
        Some(Outcome::InUse(busy)) if run.undo => {
            let first = busy.first().cloned().unwrap_or_default();
            report(h, format!("{put_back}: {first}"), Some(listing(put_back, &busy)))
        }
        Some(Outcome::InUse(busy)) => {
            let head = if ko { format!("아무것도 바꾸지 않았음: 지금 바꿀 수 없는 파일 {}개", busy.len()) } else { format!("Nothing renamed: can't rename {} of the files now", busy.len()) };
            report(h, head.clone(), Some(listing(&head, &busy)))
        }
        Some(Outcome::Failed(name, e, left, journaled)) => {
            let line = format!("{name}: {}", e.trim());
            let head = match (run.undo, ko, left, journaled) {
                (true, _, 0, _) => put_back.to_string(),
                (false, true, 0, _) => "아무것도 바꾸지 않았음 (모두 원래대로)".to_string(),
                (false, false, 0, _) => "Nothing renamed (all put back)".to_string(),
                (_, true, n, true) => format!("실패: 파일 {n}개가 임시 이름으로 남음 — 드라이브가 다시 연결되면 Grids가 정리합니다"),
                (_, false, n, true) => format!("Failed: {n} files left with temporary names — Grids tidies them once the drive is back"),
                (_, true, n, false) => format!("실패: 파일 {n}개를 원래 이름으로 되돌리지 못함"),
                (_, false, n, false) => format!("Failed: {n} files couldn't be put back to their old names"),
            };
            report(h, format!("{head}: {line}"), Some(listing(&head, &[line])));
            if left > 0 && journaled {
                recover(app(|a| a.main)); // the folder may still be there (a file held a while): tidied now
            }
        }
        Some(Outcome::Done { dirs, moves, skipped, existing }) => {
            // the window's files follow: each looked up once, by the name it had when the batch started
            let to: HashMap<(&str, &str), &str> = moves.iter().map(|(d, from, new)| ((dirs[*d].as_str(), from.as_str()), new.as_str())).collect();
            rn(|r| {
                for (d, s) in r.items.iter_mut() {
                    if let Some(new) = r.dirs.get(*d).and_then(|dir| to.get(&(dir.as_str(), s.name.as_str()))) {
                        s.name = new.to_string();
                    }
                }
                if !run.undo {
                    r.undo = moves.iter().map(|(d, o, n)| (dirs[*d].clone(), o.clone(), n.clone())).collect();
                }
                r.existing = existing;
            });
            let n = moves.len();
            if !run.undo {
                set_note(if ko { format!("{n}개 이름 바꿈") } else { format!("Renamed {n}") });
                return report(h, if ko { format!("{n}개 이름을 바꿨음. 되돌리기로 원래 이름으로.") } else { format!("Renamed {n}. Undo puts the old names back.") }, None);
            }
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
    }
}

// ---------- a batch cut off half-way (drive pulled, power gone, Grids killed): finished on the next chance ----------
// Like a database journal: a batch with temporary names writes its steps next to grids.ini first
// (grids-rename-journal-<nonce>.txt, rename_rules::journal_text) and removes them once it is over.

// A journal may be there (left by a batch, or not looked for yet this run): the next recover looks.
static PENDING: AtomicBool = AtomicBool::new(false);
// A tidy-up thread is running: no second one starts meanwhile.
static MENDING: AtomicBool = AtomicBool::new(false);
// Held while names move, by a batch (run_job) or a tidy-up, so the two never move at once. A tidy-up takes it
// only once its folders answered: a batch doesn't wait on a share that went away, only on the moves themselves.
static MOVES: Mutex<()> = Mutex::new(());
// A journal whose folders stay away this long (a drive never back) is dropped, not looked at forever.
const JOURNAL_DAYS: u64 = 30;
// What it did, for the main window: the status line note and the names it had to leave (temporary ← old).
static MENDED: Mutex<Option<(String, Vec<String>)>> = Mutex::new(None);
pub(crate) const WM_MENDED: u32 = WM_APP + 71; // to main: a tidy-up is over, its outcome is in MENDED

// Written whole and flushed before the first step, and held unshared while the batch runs, so a tidy-up here or
// in another Grids can't open it meanwhile. None: the settings folder takes no new file, the batch runs without.
fn write_journal(nonce: u128, dirs: &[String], steps: &[Move]) -> Option<(std::fs::File, PathBuf)> {
    use std::io::Write;
    use std::os::windows::fs::OpenOptionsExt;
    let path = ini_path().with_file_name(format!("grids-rename-journal-{nonce}.txt"));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).share_mode(0).open(&path).ok()?;
    if f.write_all(journal_text(nonce, dirs, steps).as_bytes()).and_then(|_| f.sync_all()).is_err() {
        drop(f);
        _ = std::fs::remove_file(&path);
        return None;
    }
    Some((f, path))
}

// Once at start, after the first panes are up.
pub(crate) unsafe fn recover_at_start(main: HWND) {
    PENDING.store(true, Ordering::Relaxed);
    recover(main);
}

// Also when a drive arrives and when the rename window opens: only if a journal may be there and no batch runs.
// Not a JOBS job: closing Grids doesn't wait on a share that went away; a tidy-up cut off is done again next time.
pub(crate) unsafe fn recover(main: HWND) {
    if !PENDING.load(Ordering::Relaxed) || rn(|r| r.running.is_some()) || MENDING.swap(true, Ordering::Relaxed) {
        return;
    }
    PENDING.store(false, Ordering::Relaxed);
    let hw = main.0 as isize;
    let spawned = std::thread::Builder::new().spawn(move || {
        let out = mend_journals();
        MENDING.store(false, Ordering::Relaxed);
        if out.is_some() {
            *MENDED.lock().unwrap_or_else(|e| e.into_inner()) = out;
            _ = PostMessageW(Some(HWND(hw as _)), WM_MENDED, WPARAM(0), LPARAM(0));
        }
    });
    if spawned.is_err() {
        MENDING.store(false, Ordering::Relaxed);
        PENDING.store(true, Ordering::Relaxed);
    }
}

// Every journal next to grids.ini. One whose folders aren't all there (drive still out) stays for the next chance;
// the others are tidied (recover_plan) and removed, also when some names had to stay: those are listed once.
unsafe fn mend_journals() -> Option<(String, Vec<String>)> {
    use std::io::Read;
    use std::os::windows::fs::OpenOptionsExt;
    let ko = ko_now();
    let (mut notes, mut left) = (Vec::new(), Vec::new());
    for e in std::fs::read_dir(ini_path().parent()?).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("grids-rename-journal-") || !name.ends_with(".txt") {
            continue;
        }
        // a batch still running (here or in another Grids) holds it: not touched
        let Ok(mut f) = std::fs::OpenOptions::new().read(true).share_mode(0).open(e.path()) else { continue };
        let mut t = String::new();
        let Some((nonce, dirs, steps)) = f.read_to_string(&mut t).ok().and_then(|_| parse_journal(&t)) else { continue };
        let used: HashSet<usize> = steps.iter().map(|s| s.0).collect();
        let all_there = || used.iter().all(|&d| Path::new(&dirs[d]).is_dir());
        if !all_there() {
            let age = f.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
            if age.is_some_and(|a| a > Duration::from_secs(JOURNAL_DAYS * 24 * 3600)) {
                drop(f);
                _ = std::fs::remove_file(e.path());
            } else {
                PENDING.store(true, Ordering::Relaxed);
            }
            continue;
        }
        let moving = MOVES.lock().unwrap_or_else(|e| e.into_inner());
        // only "not found" is absent: any other failure (the drive going mid-look) leaves the journal for later
        let unsure = std::cell::Cell::new(false);
        let there = |d: usize, n: &str| match std::fs::symlink_metadata(api_path(&dirs[d], n)) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => {
                unsure.set(true);
                true
            }
        };
        let Some((mend, moves, mut stuck)) = recover_plan(&steps, nonce, &there) else { continue };
        // a folder gone while looked in also reads as "not found": looked at again next time
        if unsure.get() || !all_there() {
            PENDING.store(true, Ordering::Relaxed);
            continue;
        }
        let busy = Busy::new(); // only while names move: a restart waits for this, not for the look above
        let mut done = Vec::new();
        for m in moves {
            if move_in(&dirs[m.0], &m.1, &m.2).is_ok() {
                done.push(m);
            } else if m.1.starts_with("~grids-rename-") {
                stuck.push(m);
            }
        }
        drop(busy);
        notify(&dirs, &done, &[]);
        drop(f);
        _ = std::fs::remove_file(e.path());
        drop(moving);
        let (n, k) = (done.len(), stuck.len());
        let mut note = match (mend, ko) {
            (None, _) => continue,
            (Some(Mend::Forward), true) => format!("중단됐던 일괄 이름 바꾸기를 마저 끝냄: 파일 {n}개"),
            (Some(Mend::Forward), false) => format!("Finished an interrupted batch rename: {n} files"),
            (Some(Mend::Back), true) => format!("중단됐던 일괄 이름 바꾸기를 되돌림: 파일 {n}개"),
            (Some(Mend::Back), false) => format!("Put back an interrupted batch rename: {n} files"),
        };
        if k > 0 {
            note += &if ko { format!(", {k}개는 이름이 이미 쓰여 남겨 둠") } else { format!(", {k} left: the name is taken") };
        }
        notes.push(note);
        for (d, tmp, _) in stuck {
            let old = steps.iter().find(|s| s.0 == d && s.2 == tmp).map_or("", |s| s.1.as_str());
            left.push(format!("{} ← {old}", join_path(&dirs[d], &tmp)));
        }
    }
    (!notes.is_empty()).then(|| (notes.join(" · "), left))
}

// WM_MENDED on the main window.
pub(crate) unsafe fn mended() {
    let Some((note, left)) = MENDED.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    set_note(note.clone());
    if !left.is_empty() {
        let head = tr("Kept under a temporary name (temporary ← old name):", "임시 이름으로 남겨 둔 파일 (임시 이름 ← 원래 이름):");
        let text = HSTRING::from(listing(&format!("{note}\n\n{head}"), &left));
        MessageBoxW(Some(app(|a| a.main)), &text, &HSTRING::from(tr("Batch rename", "일괄 이름 바꾸기")), MB_OK | MB_ICONWARNING);
    }
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
    // a batch running: nothing changes it and the window stays (keys come here too, as Delete does)
    if rn(|r| r.running.is_some()) {
        if code == BN_CLICKED {
            _ = MessageBeep(MB_ICONWARNING);
        }
        return;
    }
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
            typed(h, |r| {
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
            say_check(h, id);
        }
        (ID_EXTOK, _) => {
            changed(|r| r.ext ^= true);
            _ = InvalidateRect(Some(item(h, id)), None, false);
            say_check(h, id);
        }
        (ID_PSAVE, _) => save_preset(h),
        (ID_PMENU, _) => preset_menu(h),
        (ID_UNDO, _) => do_undo(h),
        (ID_GO, _) => do_rename(h),
        (ID_CLOSE, _) => _ = DestroyWindow(h),
        _ => {}
    }
}

// A check box just switched: its name with the new state, read out (the focus stays on it).
unsafe fn say_check(h: HWND, id: i32) {
    let (rule, ext) = rn(|r| (r.rules.get(r.cur).cloned(), r.ext));
    if let Some(name) = check_name(id, rule.as_ref(), ext, ko_now()) {
        a11y_say(item(h, id), &name, true);
    }
}

unsafe fn toggle_rule() {
    changed(|r| {
        if let Some(rule) = r.rules.get_mut(r.cur) {
            rule.on ^= true;
        }
    });
    // from the list, its row is read anew (refresh_rows); from the On / off button, read out here
    let (h, t) = rn(|r| (r.h, r.rules.get(r.cur).map(|x| rule_speech(r.cur + 1, x, ko_now()))));
    if let (Some(t), false) = (t, GetFocus() == item(h, ID_RULES)) {
        say(h, &t);
    }
}

/// Keys for the rename window, from pre_translate. None: not this window's. Esc closes, Ctrl+Enter renames,
/// Ctrl+Z undoes (outside a text field); in the rule list Space switches a rule on/off, Del removes it,
/// Ins adds one and Alt+↑/↓ moves it. Tab and the rest go the dialog way. Some(_) always: key() and its pane
/// shortcuts (Ctrl+1…9, Ctrl+Tab, Ctrl+Shift+M, Alt+arrows ...) never see a key meant for this window.
/// While a batch runs, the ones that would close, rename, undo or edit only beep (refused_key).
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
        if rn(|r| r.running.is_some()) && refused_key(VIRTUAL_KEY(m.wParam.0 as u16), ctrl, class == "edit", id == ID_RULES) {
            _ = MessageBeep(MB_ICONWARNING);
            return Some(true);
        }
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
                let (mw, mh) = min_size(dpi, work_area(h));
                mm.ptMinTrackSize = POINT { x: mw, y: mh };
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
                mi.itemHeight = rule_row_h(fh, dpi) as u32;
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
                    let (f, running) = rn(|r| (r.focus, r.running.is_some()));
                    if IsWindow(Some(f)).as_bool() && IsWindowVisible(f).as_bool() && IsWindowEnabled(f).as_bool() {
                        _ = SetFocus(Some(f));
                    } else {
                        _ = SetFocus(Some(item(h, if running { ID_LIST } else { ID_RULES })));
                    }
                }
                return LRESULT(0);
            }
            // Esc, Close and Alt+F4 wait for a running batch (closing Grids doesn't: the batch finishes after it)
            WM_CLOSE if rn(|r| r.running.is_some()) => _ = MessageBeep(MB_ICONWARNING),
            WM_CLOSE => _ = DestroyWindow(h),
            WM_TIMER if wp.0 == TIMER_RUN => _ = InvalidateRect(Some(h), None, false),
            WM_TIMER if wp.0 == TIMER_SAY => say_summary(h),
            WM_RENAMED => finished(h, wp.0),
            WM_TAKEN => taken_msg(h, wp.0, lp.0),
            WM_CTLCOLORSTATIC => return edit_colors(wp), // a field switched off while a batch runs
            WM_DESTROY => {
                _ = KillTimer(Some(h), TIMER_RUN);
                _ = KillTimer(Some(h), TIMER_SAY);
                // the names, roles, descriptions and the live region off, so a reused HWND value doesn't inherit them
                live_region(item(h, ID_LIVE), false);
                let mut c = GetWindow(h, GW_CHILD).unwrap_or_default();
                while !c.0.is_null() {
                    a11y_clear(c);
                    c = GetWindow(c, GW_HWNDNEXT).unwrap_or_default();
                }
                let pane = rn(|r| {
                    (r.h, r.focus) = (HWND::default(), HWND::default());
                    r.running = None; // Grids is closing: the outcome has no window left to go to
                    (r.items, r.rows, r.existing) = (Vec::new(), Vec::new(), Vec::new());
                    new_batch(r); // a reading under way stops; the next opening reads again if the rule asks
                    r.pane
                });
                // its pane left the grid (a smaller layout or session): the active one instead
                let (main, pane) = app(|a| (a.main, if pane < a.s.panes() { pane } else { a.s.active }));
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

    // A UTC wall-clock time as a FILETIME (100 ns since 1601-01-01).
    fn ft_utc(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> u64 {
        let (y2, m2) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
        let era = y2.div_euclid(400);
        let yoe = y2 - era * 400;
        let doy = (153 * m2 + 2) / 5 + d - 1;
        let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468; // since 1970-01-01
        ((days * 86_400 + h * 3600 + mi * 60 + 11_644_473_600) * 10_000_000) as u64
    }

    // US Eastern: UTC-5, daylight UTC-4 from the 2nd Sunday of March 02:00 to the 1st Sunday of November 02:00
    // (2026: Mar 8 07:00Z and Nov 1 06:00Z). A date in the other season must get its own offset, not today's.
    #[test]
    fn local_date_uses_each_dates_own_daylight_rule() {
        let rule = |mo, day| SYSTEMTIME { wMonth: mo, wDayOfWeek: 0, wDay: day, wHour: 2, ..Default::default() };
        let tz = TIME_ZONE_INFORMATION { Bias: 300, StandardDate: rule(11, 1), StandardBias: 0, DaylightDate: rule(3, 2), DaylightBias: -60, ..Default::default() };
        for ((y, mo, d, h, mi), want) in [
            ((2026, 1, 15, 12, 0), [2026, 1, 15, 7, 0, 0]),
            ((2026, 7, 15, 12, 0), [2026, 7, 15, 8, 0, 0]),
            ((2026, 3, 8, 6, 30), [2026, 3, 8, 1, 30, 0]),
            ((2026, 3, 8, 7, 30), [2026, 3, 8, 3, 30, 0]),
            ((2026, 11, 1, 5, 30), [2026, 11, 1, 1, 30, 0]),
            ((2026, 11, 1, 6, 30), [2026, 11, 1, 1, 30, 0]),
            ((2026, 3, 1, 3, 0), [2026, 2, 28, 22, 0, 0]),
        ] {
            assert_eq!(local_date_in(ft_utc(y, mo, d, h, mi), Some(&tz)), want, "{y}-{mo}-{d} {h}:{mi}Z");
        }
        // the two instants the preview and Everything tests use
        assert_eq!(ft_utc(2026, 1, 15, 12, 0), 134_129_520_000_000_000);
        let ft = ft_utc(2026, 7, 15, 12, 0);
        assert_eq!(ft, 134_285_904_000_000_000);
        assert_eq!(local_date_in(ft, None), local_date(ft));
        assert_eq!(local_date(u64::MAX), [0; 6]);
    }

    #[test]
    fn date_min_pads_every_field() {
        assert_eq!(date_min([2026, 3, 8, 1, 5, 9]), "2026-03-08 01:05");
        assert_eq!(date_min([2026, 12, 31, 23, 59, 0]), "2026-12-31 23:59");
    }

    #[test]
    fn rename_api_paths() {
        assert_eq!(api_path(r"C:\Users\me", "foo."), r"\\?\C:\Users\me\foo.");
        assert_eq!(api_path(r"C:\", "a b "), r"\\?\C:\a b ");
        assert_eq!(api_path(r"\\srv\share\d", "x."), r"\\?\UNC\srv\share\d\x.");
        assert_eq!(api_path(r"\\?\C:\d", "x"), r"\\?\C:\d\x");
    }

    #[test]
    fn rename_run_states() {
        let run = |id| Run { id, undo: false, prog: Arc::new(Prog::default()), back: Vec::new() };
        let mut slot = None;
        assert!(begin(&mut slot, run(1)).is_ok());
        assert_eq!(begin(&mut slot, run(2)).err().map(|r| r.id), Some(2)); // a second batch while one runs: refused
        assert!(end(&mut slot, 2).is_none() && slot.is_some()); // not the run the window waits for
        assert_eq!(end(&mut slot, 1).map(|r| r.id), Some(1));
        assert!(slot.is_none() && end(&mut slot, 1).is_none());
        assert!(begin(&mut slot, run(3)).is_ok());
        // the outcome: only the waited-for run's is applied, a stale one is dropped
        let mut done = Some((2, Outcome::InUse(vec!["a".into()])));
        assert!(take_outcome(&mut done, 3).is_none() && done.is_none());
        let mut done = Some((3, Outcome::Failed("a".into(), "x".into(), 0, false)));
        assert!(matches!(take_outcome(&mut done, 3), Some(Outcome::Failed(..))) && done.is_none());
    }

    #[test]
    fn rename_run_keys() {
        assert!(refused_key(VK_ESCAPE, false, false, false)); // close
        assert!(refused_key(VK_RETURN, true, false, false)); // Ctrl+Enter
        assert!(refused_key(VK_RETURN, false, false, false)); // a button, Save preset
        assert!(refused_key(VK_Z, true, false, false) && !refused_key(VK_Z, true, true, false));
        assert!(refused_key(VK_SPACE, false, false, true) && refused_key(VK_DELETE, false, false, true));
        assert!(!refused_key(VK_TAB, false, false, false) && !refused_key(VK_DOWN, false, false, false));
        assert!(!refused_key(VK_OEM_2, true, false, false)); // the shortcut list
    }

    #[test]
    fn rename_progress_text() {
        assert_eq!(progress_text(false, 1234, 5000, false), "Renaming… 1,234 / 5,000");
        assert_eq!(progress_text(false, 1234, 5000, true), "이름 바꾸는 중… 1,234 / 5,000");
        assert_eq!(progress_text(true, 0, 12, false), "Putting back… 0 / 12");
        assert_eq!(progress_text(true, 9, 3, true), "되돌리는 중… 3 / 3");
        assert_eq!(taken_text(16, 1200, false), "Reading dates taken… 16 / 1,200");
        assert_eq!(taken_text(5, 3, true), "찍은 날짜 읽는 중… 3 / 3");
    }

    #[test]
    fn rename_date_taken_option() {
        // only a date rule that is on, with the box ticked, asks for the dates taken
        let date = Rule::new(Kind::Date);
        let ticked = Rule { x: true, ..date.clone() };
        assert!(!wants_taken(&[date.clone(), Rule { x: true, ..Rule::new(Kind::Replace) }]));
        assert!(wants_taken(&[date.clone(), ticked.clone()]));
        assert!(!wants_taken(&[Rule { on: false, ..ticked.clone() }]));
        // its box, the row's words (no picture glyph: screen readers read them) and the hint, in both languages
        for ko in [false, true] {
            assert!(!spec_in(Kind::Date, ko).flags[0].is_empty() && spec_in(Kind::Date, ko).flags[1].is_empty());
            assert!(!spec_in(Kind::Date, ko).hint.contains(lang(ko, "Modified", "수정한")));
            assert_eq!(check_name(ID_FY, Some(&ticked), false, ko), None);
        }
        assert_eq!(check_name(ID_FX, Some(&ticked), false, false).as_deref(), Some("Photos: date taken (else modified): on"));
        assert_eq!(check_name(ID_FX, Some(&date), false, true).as_deref(), Some("사진은 찍은 날짜 (없으면 수정한 날짜): 끔"));
        assert_eq!(summary_in(&date, false), "Date: yyyyMMdd · Before the name");
        assert_eq!(summary_in(&ticked, false), "Date: yyyyMMdd · Before the name · date taken");
        assert_eq!(summary_in(&ticked, true), "날짜: yyyyMMdd · 이름 앞 · 찍은 날짜");
    }

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
                // the mode button shares its row with the two check boxes (never shown together there: the date
                // rule, which has both, shows its check box in the third field row, checked below)
                for (b, rb) in g.ctl.iter().filter(|(b, _)| b > a && !(*a == ID_MODE && (*b == ID_FX || *b == ID_FY))) {
                    let apart = ra.right <= rb.left || rb.right <= ra.left || ra.bottom <= rb.top || rb.bottom <= ra.top;
                    assert!(apart, "{a} and {b} overlap at {dpi}");
                }
            }
            // a date rule's check box in the third field row: inside, wide enough for its label, on top of nothing but
            // that row's field (a date rule hides it) and its own place in the mode row (place_x moves it from there)
            assert!(inside(&g.x_row) && g.x_row.right - g.x_row.left >= 300 * dpi / 96, "date check box at {dpi}");
            for (b, rb) in g.ctl.iter().filter(|(b, _)| *b != ID_F0 + 2 && *b != ID_FX) {
                let apart = g.x_row.right <= rb.left || rb.right <= g.x_row.left || g.x_row.bottom <= rb.top || rb.bottom <= g.x_row.top;
                assert!(apart, "date check box and {b} overlap at {dpi}");
            }
            assert!(g.x_row.bottom <= g.fields[4].top && g.x_row.top >= g.fields[1].bottom);
            let lb = g.ctl.iter().find(|(id, _)| *id == ID_RULES).unwrap().1;
            assert!(lb.bottom - lb.top >= 40 * dpi / 96);
        }
    }

    #[test]
    fn rename_a11y_names() {
        let digits = |s: &str| s.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
        let buttons = [ID_ADD, ID_DEL, ID_RUP, ID_RDOWN, ID_ONOFF, ID_MODE, ID_PSAVE, ID_PMENU, ID_UNDO, ID_GO, ID_CLOSE];
        for kind in KINDS {
            let mut r = Rule::new(kind);
            r.f = ["ab".into(), "3".into(), "2".into(), "_".into()];
            for ko in [false, true] {
                // every button has a name, with no & (it would turn into an underlined access key) and no drawn-only marks
                for id in buttons.into_iter().filter(|&id| id != ID_MODE || !spec_in(kind, ko).modes.is_empty()) {
                    let name = button_name(id, Some(&r), ko);
                    let (label, _) = button_label(id, Some(&r), ko);
                    assert!(!name.trim().is_empty() && !label.is_empty(), "{id} {kind:?} {ko}");
                    assert!(!name.contains('&') && !label.contains('&') && !name.contains('▾'), "{id} {kind:?} {ko}");
                    assert!(!name.chars().any(|c| ('\u{E000}'..='\u{F8FF}').contains(&c)), "{id}: icon glyph in its name");
                }
                // check boxes: the state is in the name
                for (id, on) in [(ID_FX, true), (ID_FY, false), (ID_EXTOK, true)] {
                    (r.x, r.y) = (on, on);
                    match check_name(id, Some(&r), on, ko) {
                        Some(n) => assert!(n.ends_with(on_word(on, ko)) && n.len() > on_word(on, ko).len() + 2, "{n}"),
                        None => assert!(id != ID_EXTOK && spec_in(kind, ko).flags[(id - ID_FX) as usize].is_empty()),
                    }
                }
            }
            // a rule row reads the same numbers in both languages, and its state
            for on in [true, false] {
                r.on = on;
                let (en, ko) = (rule_speech(2, &r, false), rule_speech(2, &r, true));
                assert_eq!(digits(&en), digits(&ko), "{en} / {ko}");
                assert!(en.starts_with("2. ") && en.ends_with(on_word(on, false)) && ko.ends_with(on_word(on, true)));
            }
        }
        assert_eq!(button_name(ID_RUP, None, true), "규칙 위로");
        assert_eq!(button_name(ID_ADD, None, false), "Add");
        assert_eq!(button_name(ID_MODE, Some(&Rule::new(Kind::Insert)), true), "넣을 곳: 맨 앞");
        assert_eq!(button_name(ID_MODE, Some(&Rule::new(Kind::Replace)), false), "");
        assert_eq!(check_name(ID_EXTOK, None, false, true).as_deref(), Some("확장자도 바꾸기: 끔"));
        assert_eq!(rule_speech(1, &Rule::new(Kind::Trim), false), "1. Trim spaces · on");
        // the drawn labels stay what they were
        assert_eq!(button_label(ID_ADD, None, true).0, "추가 ▾");
        assert_eq!(button_label(ID_RDOWN, None, false), ("\u{E74B}".into(), true));
        let (en, ko) = (sum_text(12, 10, 2, None, false), sum_text(12, 10, 2, None, true));
        assert_eq!((en.as_str(), ko.as_str()), ("12 items · 10 to rename · 2 problems", "12개 · 바꿀 것 10 · 문제 2"));
        assert!(sum_text(1, 0, 0, Some("regex parse error:\n    (\n    ^\nerror: unclosed group"), true).ends_with("unclosed group"));
    }

    #[test]
    fn rename_row_and_min_size() {
        // rule rows follow the new text height after a DPI change
        assert_eq!(rule_row_h(16, 96), 26);
        assert_eq!(rule_row_h(24, 144), 39);
        assert_eq!(rule_row_h(32, 192), 52);
        let work = |w, h| RECT { left: 0, top: 0, right: w, bottom: h };
        assert_eq!(min_size(96, work(1920, 1040)), (600, 400));
        assert_eq!(min_size(144, work(1920, 1040)), (900, 600));
        assert_eq!(min_size(192, work(800, 560)), (800, 560)); // never more than the work area
        assert_eq!(min_size(96, work(0, 0)), (1, 1));
    }
}
