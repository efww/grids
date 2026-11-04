use crate::*;

pub(crate) const MAXP: usize = 16;
pub(crate) const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
// child ids inside a pane
pub(crate) const ID_BACK: i32 = 101;
pub(crate) const ID_FWD: i32 = 102;
pub(crate) const ID_UP: i32 = 103;
pub(crate) const ID_EDIT: i32 = 104;
pub(crate) const ID_OPEN_IN: i32 = 105;
pub(crate) const ID_FILTER: i32 = 106;
pub(crate) const ID_FAV: i32 = 107; // ☆+ / ★: add or remove the pane's folder from favourites
// top toolbar buttons (children of the main window)
pub(crate) const ID_TB_LAYOUT: i32 = 201;
pub(crate) const ID_TB_SAVE: i32 = 202;
pub(crate) const ID_TB_LOAD: i32 = 204;
pub(crate) const ID_TB_THEME: i32 = 205;
pub(crate) const ID_TB_FAV: i32 = 206;
pub(crate) const ID_TB_VIEW: i32 = 203;
pub(crate) const ID_TB_EV: i32 = 207; // orange magnifier: Everything search, shown while Everything runs
pub(crate) const ID_TB_UPDATE: i32 = 208; // blue "Update", shown when a newer release is out
pub(crate) const ID_DRIVE: i32 = 1500; // + 0..25: drive buttons A: … Z:
// menu ids
pub(crate) const ID_HIDDEN: usize = 1100;
pub(crate) const ID_KEYS: usize = 1101;
pub(crate) const ID_GATHER: usize = 1102;
pub(crate) const ID_DBLUP: usize = 1103;
pub(crate) const ID_LANG: usize = 1104;
pub(crate) const ID_HELP: usize = 1105;
pub(crate) const ID_ABOUT: usize = 1106;
pub(crate) const ID_TREE: usize = 1107;
pub(crate) const ID_EXT: usize = 1108;
pub(crate) const ID_HOTKEY: usize = 1109;
pub(crate) const ID_FOLLOW: usize = 1110;
pub(crate) const ID_TABS: usize = 1111;
pub(crate) const ID_DLGJUMP: usize = 1140;
pub(crate) const ID_UPDATES: usize = 1150; // view menu: check for updates
pub(crate) const HOME: &str = "https://github.com/efww/grids";
pub(crate) const ID_ACCENT: usize = 1400; // + preset index
pub(crate) const ID_ACCENT_PICK: usize = 1410;
pub(crate) const EM_SETSEL: u32 = 0xB1;
pub(crate) const EM_GETSEL: u32 = 0xB0;
pub(crate) const EM_SCROLLCARET: u32 = 0xB7;
pub(crate) const EN_CHANGE: u32 = 0x0300;
pub(crate) const TIMER_STATUS: usize = 1;
pub(crate) const TIMER_FILTER: usize = 2;
pub(crate) const TIMER_REVEAL: usize = 3;
pub(crate) const TIMER_SETTLE: usize = 4; // on a pane: narrow column widths going back on after a folder change
pub(crate) const TIMER_SPIN: usize = 6; // on a pane: next frame of the F5 spin
pub(crate) const TIMER_FOCUS: usize = 5; // after a click brought Grids back: that click is handled, focus still nowhere?
// tree-view messages (the sidebar's SysTreeView32)
pub(crate) const TVM_GETNEXTITEM: u32 = 0x110A;
pub(crate) const TVM_ENSUREVISIBLE: u32 = 0x1114;
pub(crate) const TVM_GETITEMRECT: u32 = 0x1104;
pub(crate) const TVGN_CARET: usize = 9;
pub(crate) const TVM_SETBKCOLOR: u32 = 0x111D;
pub(crate) const TVM_GETTOOLTIPS: u32 = 0x1119;
// list-view messages (the Explorer list inside each pane)
pub(crate) const LVM_SETBKCOLOR: u32 = 0x1001;
pub(crate) const LVM_GETHEADER: u32 = 0x101F;
pub(crate) const LVM_GETTOOLTIPS: u32 = 0x104E;
pub(crate) const LVM_GETCOLUMNORDERARRAY: u32 = 0x103B;
pub(crate) const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
pub(crate) const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = 0x1037;
pub(crate) const LVS_EX_AUTOSIZECOLUMNS: usize = 0x1000_0000;
pub(crate) const LVS_EX_COLUMNSNAPPOINTS: usize = 0x4000_0000;
pub(crate) const HDM_GETITEMCOUNT: u32 = 0x1200;
pub(crate) const LVM_SETTEXTCOLOR: u32 = 0x1024;
pub(crate) const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
pub(crate) const LVM_SETSELECTEDCOLUMN: u32 = 0x108C;
pub(crate) const LVM_GETSELECTEDCOLUMN: u32 = 0x10AE;
pub(crate) const TVM_SETTEXTCOLOR: u32 = 0x111E;
pub(crate) const COPY_ID: usize = 0x4752_4944; // "GRID": WM_COPYDATA from a second instance
// Move without copying old pixels: copied bits are what left ghost borders/buttons while resizing.
pub(crate) const MOVE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_NOCOPYBITS.0);

// Themes, 0xRRGGBB. Light: an ivory frame around white plates; dark: charcoal. Each theme has its own
// selection colours (same names, tuned to the ground): pastels on light, muted mid tones on dark.
pub(crate) struct Palette {
    pub(crate) bg: u32,   // frame: window, toolbar, gaps, status line
    pub(crate) card: u32, // plates and lists
    pub(crate) line: u32,
    pub(crate) pill: u32, // fields
    pub(crate) text: u32,
    pub(crate) sub: u32,
    pub(crate) accents: [(&'static str, u32); 6],
}

pub(crate) const LIGHT: Palette = Palette {
    bg: 0xF3F1EC,
    card: 0xFFFFFF,
    line: 0xE4E0D8,
    pill: 0xF4F2EE,
    text: 0x2A2825,
    sub: 0x7C776F,
    accents: [("하늘", 0x8DB5E8), ("그레이", 0xC5C5C5), ("라벤더", 0xA99BE0), ("세이지", 0x8CC5A8), ("피치", 0xEFAE92), ("로즈", 0xE7A1B5)],
};

pub(crate) const DARK: Palette = Palette {
    bg: 0x1C1D20,
    card: 0x26282C,
    line: 0x36393E,
    pill: 0x303338,
    text: 0xE6E8EB,
    sub: 0x9AA0A8,
    accents: [("하늘", 0x4C6FA8), ("그레이", 0x4B4B4B), ("라벤더", 0x6A5AA8), ("세이지", 0x3E8676), ("피치", 0xA8624A), ("로즈", 0xA65A78)],
};

pub(crate) static DARK_ON: AtomicBool = AtomicBool::new(false);
pub(crate) static KO: AtomicBool = AtomicBool::new(false);
pub(crate) static TABS_ON: AtomicBool = AtomicBool::new(false); // copy of Session::tabs_on for pane_geo

/// UI text: English unless Korean is picked in the view menu (the Explorer parts follow Windows).
pub(crate) fn tr(en: &'static str, ko: &'static str) -> &'static str {
    if KO.load(Ordering::Relaxed) {
        ko
    } else {
        en
    }
}

pub(crate) const ACCENT_EN: [&str; 6] = ["Sky", "Gray", "Lavender", "Sage", "Peach", "Rose"];

pub(crate) fn pal() -> &'static Palette {
    if DARK_ON.load(Ordering::Relaxed) {
        &DARK
    } else {
        &LIGHT
    }
}

pub(crate) const INK: u32 = 0x2A2825; // dark text on light selection colours
pub(crate) const GREEN: u32 = 0x1FA463; // unsaved layout changes
pub(crate) fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = ((a >> sh & 0xFF) as f32, (b >> sh & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(16) | ch(8) | ch(0)
}

// Text/icons on a coloured fill: near-black on light colours, white on dark ones (black accent -> white text).
pub(crate) fn on_color(bg: u32) -> u32 {
    let l = 0.2126 * (bg >> 16 & 0xFF) as f32 + 0.7152 * (bg >> 8 & 0xFF) as f32 + 0.0722 * (bg & 0xFF) as f32;
    if l > 160.0 {
        INK
    } else {
        0xFFFFFF
    }
}
// Segoe Fluent Icons / MDL2 Assets code points
pub(crate) const ICON_SEARCH: &str = "\u{E721}";
pub(crate) const ICON_FILTER: &str = "\u{E71C}";
pub(crate) const ICON_CLOSE: &str = "\u{E711}";
pub(crate) const ICON_PREV: &str = "\u{E76B}";
pub(crate) const ICON_NEXT: &str = "\u{E76C}";

pub(crate) fn glyph(id: i32) -> &'static str {
    match id {
        ID_BACK => "\u{E72B}",
        ID_FWD => "\u{E72A}",
        ID_UP => "\u{E74A}",
        ID_OPEN_IN => "\u{E8AB}",
        ID_TB_LAYOUT => "\u{E8A9}",
        ID_TB_SAVE => "\u{E74E}",
        ID_TB_LOAD => "\u{E838}",
        ID_TB_FAV => "\u{E734}",
        ID_TB_EV => ICON_SEARCH,
        ID_TB_THEME if DARK_ON.load(Ordering::Relaxed) => "\u{E706}", // sun: back to light
        ID_TB_THEME => "\u{E708}",                                     // moon: go dark
        _ => "\u{E890}",
    }
}

// Filter icon / Alt+↓ menu: (English, Korean, filter spec). No spaces, so filter_spec keeps them as they are.
pub(crate) const FILE_KINDS: [(&str, &str, &str); 6] = [
    ("Pictures", "사진", "*.jpg;*.jpeg;*.png;*.gif;*.bmp;*.webp;*.heic;*.tif;*.tiff;*.svg;*.ico"),
    ("Videos", "동영상", "*.mp4;*.mkv;*.avi;*.mov;*.wmv;*.webm;*.m4v"),
    ("Music", "음악", "*.mp3;*.flac;*.wav;*.m4a;*.aac;*.ogg;*.wma"),
    ("Documents", "문서", "*.pdf;*.doc;*.docx;*.xls;*.xlsx;*.ppt;*.pptx;*.hwp;*.hwpx;*.txt;*.md;*.csv;*.rtf;*.odt"),
    ("Archives", "압축", "*.zip;*.7z;*.rar;*.tar;*.gz;*.iso"),
    ("Programs", "프로그램", "*.exe;*.msi;*.bat;*.cmd;*.ps1"),
];

pub(crate) const KEYS_EN: &str = "Ctrl+Tab / Ctrl+Shift+Tab\tNext / previous pane\n\
Ctrl+1 … Ctrl+9\tGo to pane 1–9 (the number at the left of each pane header; hidden in very narrow panes)\n\
Alt+Shift+← / → / ↑ / ↓\tPane to the left / right / above / below\n\
Ctrl+Alt+Shift+← / → / ↑ / ↓\tSwap the active pane with the one to the left / right / above / below (folder, history and selection stay; also in the ⇄ menu)\n\
Alt+← / Alt+→\tBack / forward\n\
Alt+↑\tParent folder\n\
Alt+↓\tRecent folders (all panes, newest first)\n\
Ctrl+L, Alt+D, F4\tAddress bar: also .. or a subfolder name, a file to open, or a command such as cmd, wt, powershell (runs in this pane's folder), or part of a recent or favourite folder's name\n\
Ctrl+B\tFavourites list (↑/↓ to choose, Enter to open, Del to remove, Alt+↑/↓ or drag to reorder)\n\
Ctrl+/\tKeyboard shortcuts (this list)\n\
Ctrl+Shift+M\tMaximize / restore the active pane (or double-click its header)\n\
Ctrl+Shift+E\tHide / show the folder tree (its width is kept)\n\
Ctrl+Shift+O\tOpen the folders of Explorer windows and tabs in panes (panes already showing one stay put)\n\
Ctrl+Shift+C\tCopy the full path of the selected items (or of the current folder)\n\
Ctrl+Shift+R\tBatch rename the selected items (none selected: every file in the folder): rules such as replace, numbering, date, case; live preview, Undo, presets. In the window: Ctrl+Enter renames, Esc closes; in the rule list Space switches a rule on/off, Ins adds, Del removes, Alt+↑/↓ moves it\n\
Num+ / Ctrl+Shift+A\tSelect files by pattern (*.jpg;*.png or part of a name; Shift+Enter deselects)\n\
Num−\tDeselect files by pattern\n\
Num*\tInvert the selection\n\
Ctrl+F\tFilter (*.pdf;*.txt or part of a name; folders always show)\n\
Alt+↓ in the filter\tFile types: pictures, videos, documents ... (or click the filter's icon)\n\
Ctrl+Shift+F\tSearch the whole PC with Everything, while it runs (or the orange magnifier): ↑/↓ choose, Enter shows the item in the active pane, Ctrl+Enter opens a folder, Alt+O only under this folder\n\
Ctrl+T / Ctrl+W\tNew tab (same folder) / close tab, with tabs on (View menu)\n\
Ctrl+K\tDuplicate the current tab\n\
Ctrl+PgDn / Ctrl+PgUp\tNext / previous tab\n\
F7\tNew folder\n\
Shift+F7\tNew text file (named, selected and ready to rename)\n\
F6 / Shift+F6\tMove between file list, address bar, filter and folder tree\n\
Enter / Esc\tApply / cancel the address or filter\n\
Ctrl+H\tShow hidden files\n\
F3, Space\tQuick preview of the focused file: pictures, text and code, Markdown, CSV tables, Office / PDF / HTML / mail (files up to 10 MB), HWP / HWPX, ZIP contents, folders; anything else as a card of facts (arrows or the wheel: previous / next file without closing; on long text the wheel scrolls first and moves on past the end; PgUp / PgDn: scroll; Esc: close, also while it is still opening)\n\
Del / Shift+Del, F2\tIn the preview: the file shown to the Recycle Bin / deleted for good (Windows asks as in Explorer; the preview moves on to the next file), or renamed in its header (Enter / Esc)\n\
Ctrl+wheel, Ctrl+= / Ctrl+- / Ctrl+0\tIn the preview: zoom a picture or PDF in / out (the wheel zooms on the cursor) / back to fit; zoomed in, drag it or its scrollbars to move around\n\
Ctrl+V\tWith a copied picture (screenshot, browser image): save it here as a PNG file\n\
F1\tHelp (opens the README on GitHub)\n\
Win+Alt+E\tBring Grids to front from any app (again: minimize)\n\
Ctrl+Alt+G\tIn another app's Open / Save dialog: jump to a folder open in Grids\n\
F2, Del, Ctrl+C/X/V, F5\tSame as Explorer\n\n\
Address bar: click a folder in the path to go there; click a › to list the folders inside; click the current folder or empty space, or press Ctrl+L, to type a path\n\
Layout button: click for an even grid, press and drag for rows per column (arrow keys + Enter work too)\n\
⇄ button: open the selected folder (or the current one) in another pane, with this pane's columns; or copy / move the selected files to another pane's folder (up to 10,000); which program is using the selected files; Compare with WinMerge (2 selected items, or this folder with another pane's) when WinMerge is installed\n\
Save (Ctrl+S): overwrite the current session file; green dot = unsaved layout changes (not folder moves)\n\
Load: pick a saved session by its layout preview\n\
Drive buttons (C: D: …): open that drive in the active pane; its drive is underlined\n\
Double-click a splitter: split evenly\n\
Update button (blue, top right): a newer Grids is out; click to restart into it (settings, folders and sessions stay)\n\
Right-click a pane header: colour label (saved with the session)\n\
Tabs: click to switch, × or middle-click to close, + or double-click the empty strip for a new tab, right-click to duplicate or close\n\
Arguments: grids.exe C:\\a D:\\b ... or grids.exe work.grids\n\
If Grids is already running, they open in that window.";

pub(crate) const KEYS_KO: &str = "Ctrl+Tab / Ctrl+Shift+Tab\t다음 / 이전 패널\n\
Ctrl+1 … Ctrl+9\t1~9번 패널로 이동 (패널 머리줄 왼쪽의 번호, 아주 좁은 패널에서는 생략)\n\
Alt+Shift+← / → / ↑ / ↓\t왼쪽·오른쪽·위·아래 패널로 이동\n\
Ctrl+Alt+Shift+← / → / ↑ / ↓\t지금 패널을 왼쪽·오른쪽·위·아래 패널과 자리 바꾸기 (폴더·방문 기록·선택 유지, ⇄ 메뉴에도 있음)\n\
Alt+← / Alt+→\t뒤로 / 앞으로\n\
Alt+↑\t상위 폴더\n\
Alt+↓\t최근 폴더 (모든 패널, 최근 순)\n\
Ctrl+L, Alt+D, F4\t주소창: .. 이나 하위 폴더 이름, 열 파일, cmd·wt·powershell 같은 명령도 가능(이 패널 폴더에서 실행), 다녀온 폴더·즐겨찾기 이름 일부\n\
Ctrl+B\t즐겨찾기 목록 (↑/↓ 고르기, Enter 열기, Del 빼기, Alt+↑/↓ 또는 끌어서 순서 바꾸기)\n\
Ctrl+/\t단축키 목록 (이 창)\n\
Ctrl+Shift+M\t선택한 패널 크게 보기 / 복원 (머리줄 더블클릭도 가능)\n\
Ctrl+Shift+E\t폴더 트리 숨기기 / 보이기 (폭은 그대로 유지)\n\
Ctrl+Shift+O\t탐색기 창·탭의 폴더를 패널로 가져오기 (이미 그 폴더를 연 패널은 그대로)\n\
Ctrl+Shift+C\t선택한 항목(없으면 현재 폴더)의 전체 경로 복사\n\
Ctrl+Shift+R\t선택한 항목(선택 없으면 폴더의 모든 파일) 일괄 이름 바꾸기: 바꾸기·번호·날짜·대소문자 같은 규칙, 실시간 미리보기, 되돌리기, 프리셋. 창 안에서 Ctrl+Enter 바꾸기, Esc 닫기, 규칙 목록에서 Space 켜기/끄기, Ins 추가, Del 빼기, Alt+↑/↓ 순서 바꾸기\n\
Num+ / Ctrl+Shift+A\t패턴으로 파일 선택 (*.jpg;*.png 또는 이름 일부, Shift+Enter는 선택 해제)\n\
Num−\t패턴으로 파일 선택 해제\n\
Num*\t선택 반전\n\
Ctrl+F\t필터 (*.pdf;*.txt 또는 이름 일부, 폴더는 항상 표시)\n\
필터 칸에서 Alt+↓\t파일 종류: 사진·동영상·문서 … (필터 칸 아이콘 클릭도 가능)\n\
Ctrl+Shift+F\tEverything으로 PC 전체 검색, Everything이 실행 중일 때 (주황 돋보기도 가능): ↑/↓ 고르기, Enter는 지금 패널에서 그 항목 보기, Ctrl+Enter는 폴더 열기, Alt+O는 이 폴더 아래에서만\n\
Ctrl+T / Ctrl+W\t새 탭(같은 폴더) / 탭 닫기, 탭을 켰을 때 (보기 메뉴)\n\
Ctrl+K\t지금 탭 복제\n\
Ctrl+PgDn / Ctrl+PgUp\t다음 / 이전 탭\n\
F7\t새 폴더\n\
Shift+F7\t새 텍스트 파일 (이름 짓기 바로 시작)\n\
F6 / Shift+F6\t파일 목록·주소창·필터·폴더 트리 사이 이동\n\
Enter / Esc\t주소창·필터 적용 / 취소\n\
Ctrl+H\t숨김 파일 표시\n\
F3, Space\t포커스한 파일 빠른 미리보기: 사진, 텍스트·코드, 마크다운, CSV 표, Office·PDF·HTML·메일(10 MB까지), HWP·HWPX, ZIP 목록, 폴더. 나머지는 정보 카드 (방향키·휠: 닫지 않고 이전·다음 파일, 긴 텍스트에서는 휠이 먼저 스크롤하고 끝에서 넘어감, PgUp/PgDn: 스크롤, Esc: 닫기, 여는 중에도 바로)\n\
Del / Shift+Del, F2\t미리보기에서: 보고 있는 파일을 휴지통으로 / 완전히 삭제 (탐색기처럼 Windows가 확인, 미리보기는 다음 파일로), 또는 머리줄에서 이름 바꾸기 (Enter / Esc)\n\
Ctrl+휠, Ctrl+= / Ctrl+- / Ctrl+0\t미리보기에서: 사진·PDF 확대 / 축소 (휠은 커서 위치 기준) / 화면에 맞춤, 확대한 상태에서는 끌거나 스크롤바로 이동\n\
Ctrl+V\t복사한 그림(스크린샷, 브라우저 이미지)이 있으면 이 폴더에 PNG 파일로 저장\n\
F1\t도움말 (GitHub README 열기)\n\
Win+Alt+E\t어디서나 Grids 앞으로 (다시 누르면 최소화)\n\
Ctrl+Alt+G\t다른 앱의 열기·저장 대화 상자에서: Grids에 열린 폴더로 이동\n\
F2, Del, Ctrl+C/X/V, F5\t탐색기와 동일\n\n\
주소 칸: 경로의 폴더를 누르면 그리로 이동, › 를 누르면 그 안의 폴더 목록, 현재 폴더·빈 곳을 누르거나 Ctrl+L이면 경로 입력\n\
레이아웃 버튼: 클릭 = 균등 배치, 누른 채 끌기 = 열마다 칸 수 (방향키+Enter도 가능)\n\
⇄ 버튼: 선택한 폴더(없으면 현재 폴더)를 다른 패널에서 열기, 열 순서·폭·정렬도 함께. 또는 선택한 파일을 다른 패널 폴더로 복사·이동 (최대 10,000개). 선택한 파일을 쓰고 있는 프로그램 확인. WinMerge가 있으면 WinMerge로 비교 (선택한 2개, 또는 이 폴더와 다른 패널 폴더)\n\
저장 (Ctrl+S): 지금 세션 파일에 바로 덮어쓰기, 초록 점 = 저장 안 된 배치 변경(폴더 이동은 제외)\n\
불러오기: 저장한 세션을 배치 미리보기로 보고 한 번 클릭\n\
드라이브 버튼 (C: D: …): 지금 패널에서 그 드라이브 열기, 지금 패널의 드라이브에 밑줄\n\
스플리터 더블클릭: 균등 분할\n\
업데이트 버튼 (파란색, 오른쪽 위): 새 Grids가 나옴, 누르면 재시작해 업데이트 (설정·폴더·세션 유지)\n\
패널 머리줄 오른쪽 클릭: 패널 색 라벨 (세션에 저장)\n\
탭: 클릭 = 전환, × 또는 가운데 클릭 = 닫기, + 또는 탭 줄 빈 곳 더블클릭 = 새 탭, 오른쪽 클릭 = 복제·닫기\n\
실행 인자: grids.exe C:\\a D:\\b ... 또는 grids.exe 작업.grids\n\
이미 실행 중이면 그 창에서 엽니다.";
