use crate::*;

// ---------- session (grids.ini next to the exe; named sessions are the same format as .grids) ----------

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Session {
    pub(crate) win: Option<[i32; 5]>, // normal rect l,t,r,b + maximized flag
    pub(crate) places: Vec<(String, [i32; 5])>, // per monitor setup (mon_sig): its last `win`, newest first, WIN_PLACES at most
    pub(crate) side: i32,             // sidebar width in px
    pub(crate) side_dpi: i32,         // the DPI `side` was measured at (a.dpi when saved); 0 = not known (an older ini)
    pub(crate) heights: Vec<usize>,   // panes per column, left to right (1..=4 columns of 1..=4)
    pub(crate) colr: Vec<f64>,        // column width ratios
    pub(crate) rowr: Vec<Vec<f64>>,   // row height ratios, per column
    pub(crate) active: usize,
    pub(crate) paths: Vec<String>,     // parsing names per pane slot (numbered down each column, left to right)
    pub(crate) views: Vec<(i32, i32)>, // (FOLDERVIEWMODE, icon size); 0 = shell default
    pub(crate) accent: u32,            // selection colour 0xRRGGBB, light theme
    pub(crate) accent_dark: u32,       // selection colour, dark theme
    pub(crate) accent_sys: bool,       // shown instead: Windows' accent colour, in a shade for the theme (sys_accent)
    pub(crate) dark: bool,
    pub(crate) follow: bool,           // dark follows Windows' app light/dark setting (on for a new install)
    pub(crate) dblup: bool,            // double-click on empty space goes to the parent folder
    pub(crate) tree: bool,             // folder tree shown (side keeps its width while hidden)
    pub(crate) tree_quick: bool,       // Explorer's Quick access (its pinned folders) as the tree's first root
    pub(crate) hotkey: bool,          // Win+Alt+E brings Grids to the front, app-wide (not part of a layout)
    pub(crate) dlgjump: bool,          // Ctrl+Alt+G in other apps' file dialogs lists Grids' folders (dialog_jump.rs)
    pub(crate) bring_here: bool,       // called (Win+Alt+E, the icon, a second start) from another virtual desktop: Grids moves there, app-wide
    pub(crate) to_mouse: bool,         // Win+Alt+E with the mouse on another monitor: Grids moves to that one (window_ui.rs), app-wide
    pub(crate) tray: bool,             // minimized, Grids leaves the taskbar for a notification area icon (tray.rs), app-wide
    pub(crate) file: String,          // named session (.grids) the layout was last loaded from / saved to
    pub(crate) favs: Vec<String>,      // favourite folders (parsing names), app-wide
    pub(crate) recent: Vec<String>,    // recently visited folders from every pane, newest first, app-wide
    pub(crate) ko: bool,               // Korean UI
    pub(crate) cols: Vec<String>,      // per pane: columns and sort of its view (cols_text)
    pub(crate) tints: Vec<u8>,         // per pane: colour label, 0 = none, 1..=6 = selection colour preset (this theme's)
    pub(crate) order: Vec<u8>,         // slot i shows the pane slot order[i] held when the session was last saved / loaded; empty = as saved (grids.ini only)
    pub(crate) tabs_on: bool,          // tab strips shown (View menu), app-wide; the tabs themselves are kept while off
    pub(crate) tabs: Vec<Vec<String>>, // per pane: its tabs' folders; the current one's entry is stale, paths[i] is the truth. <2 = no tabs
    pub(crate) tab: Vec<usize>,        // per pane: current tab
    pub(crate) rn_presets: Vec<String>, // batch rename presets (rename_rules::preset_text), app-wide
    pub(crate) ev_here: bool,          // Everything search: only under the active pane's folder (off = whole PC), app-wide
    pub(crate) ev_cols: Option<[f64; 3]>, // Everything search: column borders dragged, as parts of the row width
    pub(crate) ev_sort: u32,           // Everything search: its sort code (1 name ascending ... 14 newest first), app-wide
    pub(crate) updates: bool,          // look for a newer release now and then (off = no network at all), app-wide
    pub(crate) key_tips: bool,         // a mouse action with a shortcut names it on the status line (View menu), app-wide
    pub(crate) key_tips_seen: [u8; TIP_N], // per action (TIP_KEYS): times its tip showed; TIP_LEARNED = its key was used
    pub(crate) ctrl_hints: bool,       // Ctrl held alone shows each visible button's shortcut as a keycap (View menu), app-wide
}

impl Default for Session {
    fn default() -> Self {
        Session {
            win: None,
            places: Vec::new(),
            side: 0,
            side_dpi: 0,
            heights: vec![2, 2],
            colr: even(2),
            rowr: vec![even(2); 2],
            active: 0,
            paths: vec![String::new(); MAXP],
            views: vec![(0, 0); MAXP],
            accent: LIGHT.accents[1].1, // gray
            accent_dark: DARK.accents[1].1,
            accent_sys: false,
            dark: false,
            follow: false,
            dblup: true,
            tree: true,
            tree_quick: true,
            hotkey: true,
            dlgjump: true,
            bring_here: true,
            to_mouse: true,
            tray: false,
            file: String::new(),
            favs: Vec::new(),
            recent: Vec::new(),
            ko: false,
            cols: vec![String::new(); MAXP],
            tints: vec![0; MAXP],
            order: Vec::new(),
            tabs_on: true, // a new install shows the tab strips; an ini without tabs= keeps them off (parse)
            tabs: vec![Vec::new(); MAXP],
            tab: vec![0; MAXP],
            rn_presets: Vec::new(),
            ev_here: false,
            ev_sort: 1,
            ev_cols: None,
            updates: true,
            key_tips: true,
            key_tips_seen: [0; TIP_N],
            ctrl_hints: true,
        }
    }
}

// ---------- shortcut tips (Key Promoter): a mouse action that has a key says so, a few times ----------

pub(crate) const TIP_N: usize = 9;
pub(crate) const TIP_BACK: usize = 0;
pub(crate) const TIP_FWD: usize = 1;
pub(crate) const TIP_UP: usize = 2;
pub(crate) const TIP_FAVS: usize = 3;
pub(crate) const TIP_EV: usize = 4;
pub(crate) const TIP_ZOOM: usize = 5;
pub(crate) const TIP_NEW_TAB: usize = 6;
pub(crate) const TIP_CLOSE_TAB: usize = 7;
pub(crate) const TIP_LOAD: usize = 8;
pub(crate) const TIP_KEYS: [&str; TIP_N] = ["Alt+←", "Alt+→", "Alt+↑", "Ctrl+B", "Ctrl+Shift+F", "Ctrl+Shift+M", "Ctrl+T", "Ctrl+W", "Ctrl+O"];
pub(crate) const TIP_MAX: u8 = 3; // shown this many times at most
pub(crate) const TIP_LEARNED: u8 = 4; // the key itself was used: never again

/// The tip for a mouse action, if it is still due: tips on, its key not used yet, shown fewer than TIP_MAX times.
pub(crate) fn key_tip(action: usize, seen: u8, on: bool, ko: bool) -> Option<String> {
    let key = TIP_KEYS.get(action)?;
    (on && seen < TIP_MAX).then(|| if ko { format!("팁: {key} 로도 할 수 있어요") } else { format!("Tip: {key} does this too") })
}

/// key_tips_seen=01240000, digit by digit so a list that grows later keeps what was learned:
/// missing digits are 0, extra ones are dropped, anything but 0..=4 is 0.
pub(crate) fn tips_seen_parse(v: &str) -> [u8; TIP_N] {
    let mut out = [0; TIP_N];
    for (o, c) in out.iter_mut().zip(v.trim().chars()) {
        *o = c.to_digit(10).filter(|&d| d <= TIP_LEARNED as u32).unwrap_or(0) as u8;
    }
    out
}

pub(crate) fn even(n: usize) -> Vec<f64> {
    vec![1.0 / n as f64; n]
}

pub(crate) fn nums<T: std::str::FromStr>(v: &str) -> Vec<T> {
    v.split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

/// One win_at= value: "<mon_sig>|l,t,r,b,max". The setup is digits, '-', ',' and ';' only; exactly five numbers.
pub(crate) fn place_parse(v: &str) -> Option<(String, [i32; 5])> {
    let (g, w) = v.trim().split_once('|')?;
    let ok = !g.is_empty() && g.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | ',' | ';'));
    let n: Vec<i32> = w.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
    Some((g.to_string(), n.try_into().ok()?)).filter(|_| ok)
}

pub(crate) fn join<T: Display>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

// Ratios from a hand-edited ini can be anything: wrong length, zero, negative, NaN, inf (f64 parses "inf",
// "NaN", "1e309"), or finite values whose sum overflows to inf -> fall back to equal.
pub(crate) fn fix(r: Vec<f64>, n: usize) -> Vec<f64> {
    if r.len() != n || r.iter().any(|x| !(x.is_finite() && *x > 0.0)) {
        return even(n);
    }
    let sum: f64 = r.iter().sum();
    if !sum.is_finite() {
        return even(n);
    }
    r.iter().map(|x| x / sum).collect()
}

// "2 × 2" for an even grid, "1 | 3" when the columns differ.
pub(crate) fn layout_label(h: &[usize]) -> String {
    match h.iter().all(|&n| n == h[0]) {
        true => format!("{} × {}", h[0], h.len()),
        false => h.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" | "),
    }
}

// High contrast's highlight, then Windows' accent colour (followed and read), then the saved colour.
pub(crate) fn accent_pick(hc: Option<u32>, sys: Option<u32>, saved: u32) -> u32 {
    hc.or(sys).unwrap_or(saved)
}

impl Session {
    // The selection colour on screen: Windows' highlight under high contrast, else Windows' accent colour
    // when followed (and read), else the saved one.
    pub(crate) fn accent_now(&self) -> u32 {
        self.accent_in(hc_colors().map(|c| c.hl))
    }

    pub(crate) fn accent_in(&self, hc: Option<u32>) -> u32 {
        accent_pick(hc, self.accent_sys.then(|| sys_accent(self.dark)).flatten(), self.accent_saved())
    }

    // The theme's own selection colour, as in grids.ini (menu ticks, the colour picker's start).
    pub(crate) fn accent_saved(&self) -> u32 {
        if self.dark {
            self.accent_dark
        } else {
            self.accent
        }
    }

    pub(crate) fn panes(&self) -> usize {
        self.heights.iter().sum()
    }

    /// New layout with even splits.
    pub(crate) fn set_heights(&mut self, h: Vec<usize>) {
        self.colr = even(h.len());
        self.rowr = h.iter().map(|&n| even(n)).collect();
        self.heights = h;
        self.active = self.active.min(self.panes() - 1);
    }

    pub(crate) fn parse(text: &str) -> Session {
        // tabs= came later: a file without it reads as off, as it always did (the new-install default is on)
        let mut s = Session { tabs_on: false, ..Session::default() };
        let mut rowr: Vec<Vec<f64>> = Vec::new();
        // Notepad's "UTF-8 with BOM" (or PowerShell 5's -Encoding UTF8) would otherwise hide the first key (win=)
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let slot = |p: char| k.strip_prefix(p).and_then(|n| n.parse::<usize>().ok()).filter(|&i| i < MAXP);
            match k {
                "win" => s.win = nums(v).try_into().ok(),
                // win_at=<mon_sig>|l,t,r,b,max ; a damaged line, a repeated setup or one past WIN_PLACES is dropped
                "win_at" => {
                    if let Some((g, w)) = place_parse(v) {
                        if s.places.len() < WIN_PLACES && !s.places.iter().any(|(h, _)| *h == g) {
                            s.places.push((g, w));
                        }
                    }
                }
                "side" => s.side = v.parse().unwrap_or(0),
                "side_dpi" => s.side_dpi = v.parse().unwrap_or(0),
                "cols" => {
                    let h: Vec<usize> = nums::<usize>(v).into_iter().take(4).map(|n| n.clamp(1, 4)).collect();
                    if !h.is_empty() {
                        s.heights = h;
                    }
                }
                // older ini: even rows x cols grid
                "grid" => {
                    if let [r, c] = nums::<usize>(v)[..] {
                        s.heights = vec![r.clamp(1, 4); c.clamp(1, 4)];
                    }
                }
                "colr" => s.colr = nums(v),
                "rowr" => rowr = v.split('|').map(nums).collect(),
                "active" => s.active = v.parse().unwrap_or(0),
                "accent" => s.accent = u32::from_str_radix(v.trim().trim_start_matches('#'), 16).unwrap_or(s.accent) & 0xFFFFFF,
                "accent_dark" => s.accent_dark = u32::from_str_radix(v.trim().trim_start_matches('#'), 16).unwrap_or(s.accent_dark) & 0xFFFFFF,
                "accent_sys" => s.accent_sys = v.trim() == "1",
                "dark" => s.dark = v.trim() == "1",
                "follow_theme" => s.follow = v.trim() == "1",
                "dblup" => s.dblup = v.trim() != "0",
                "tree" => s.tree = v.trim() != "0",
                "tree_quick" => s.tree_quick = v.trim() != "0",
                "hotkey" => s.hotkey = v.trim() != "0",
                "dialog_jump" => s.dlgjump = v.trim() != "0",
                "follow_desktop" => s.bring_here = v.trim() != "0",
                "follow_monitor" => s.to_mouse = v.trim() != "0",
                "tray" => s.tray = v.trim() == "1",
                "tabs" => s.tabs_on = v.trim() == "1",
                "ev_here" => s.ev_here = v.trim() == "1",
                "ev_sort" => s.ev_sort = ev_sort_ok(v.trim().parse().unwrap_or(1)),
                "ev_cols" => s.ev_cols = ev_cols_parse(v),
                "update_check" => s.updates = v.trim() != "0",
                "key_tips" => s.key_tips = v.trim() != "0",
                "key_tips_seen" => s.key_tips_seen = tips_seen_parse(v),
                "ctrl_hints" => s.ctrl_hints = v.trim() != "0",
                "lang" => s.ko = v.trim() == "ko",
                "session" => s.file = v.trim().to_string(),
                "fav" if !v.trim().is_empty() => s.favs.push(v.trim().to_string()),
                "rename_preset" if !v.trim().is_empty() => s.rn_presets.push(v.trim().to_string()),
                // older Grids skip it (no p/v/c/t slot key); anything but a full permutation is ignored
                "order" => {
                    let o: Vec<u8> = nums(v);
                    let mut sorted = o.clone();
                    sorted.sort();
                    if sorted.into_iter().eq(0..MAXP as u8) {
                        s.order = o;
                    }
                }
                "recent" if !v.trim().is_empty() && s.recent.len() < RECENT => s.recent.push(v.trim().to_string()),
                _ => {
                    if let Some(i) = slot('p') {
                        s.paths[i] = v.to_string();
                    } else if let (Some(i), [m, z]) = (slot('v'), &nums::<i32>(v)[..]) {
                        s.views[i] = (*m, *z);
                    } else if let Some(i) = slot('c') {
                        s.cols[i] = v.trim().to_string();
                    } else if let Some(i) = slot('b') {
                        // b3=1|C:\a|D:\b : current tab, then every tab's folder ('|' is never in a path)
                        let mut it = v.split('|');
                        let cur = it.next().and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(0);
                        let list: Vec<String> = it.filter(|p| !p.is_empty()).map(String::from).collect();
                        if list.len() > 1 {
                            s.tab[i] = cur.min(list.len() - 1);
                            s.tabs[i] = list;
                        }
                    } else if let Some(i) = slot('t') {
                        s.tints[i] = v.trim().parse().ok().filter(|&k| k <= 6).unwrap_or(0);
                    }
                }
            }
        }
        s.colr = fix(std::mem::take(&mut s.colr), s.heights.len());
        // one group per column; an older single group applies to every column
        let group = |c: usize| if rowr.len() == s.heights.len() { rowr.get(c) } else { rowr.first() };
        s.rowr = s.heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
        if s.active >= s.panes() {
            s.active = 0;
        }
        s
    }

    pub(crate) fn to_ini(&self) -> String {
        // a folder name can hold a line break (made on a share, a USB image, WSL): such a value is left out rather
        // than let its second half become a line of its own (session=, p0= ...) on the next start
        let one = |s: &str| !s.contains(['\r', '\n']);
        let mut o = String::new();
        if let Some(w) = self.win {
            o += &format!("win={}\n", join(&w));
        }
        for (g, w) in &self.places {
            o += &format!("win_at={g}|{}\n", join(w));
        }
        let rowr = self.rowr.iter().map(|r| join(r)).collect::<Vec<_>>().join("|");
        o += &format!("side={}\n", self.side);
        if self.side_dpi != 0 {
            o += &format!("side_dpi={}\n", self.side_dpi); // older Grids skip the key
        }
        o += &format!("cols={}\n", join(&self.heights));
        o += &format!("colr={}\nrowr={rowr}\nactive={}\n", join(&self.colr), self.active);
        o += &format!("accent={:06X}\naccent_dark={:06X}\naccent_sys={}\n", self.accent, self.accent_dark, self.accent_sys as u8);
        o += &format!("dark={}\n", self.dark as u8);
        o += &format!("follow_theme={}\n", self.follow as u8);
        o += &format!("dblup={}\ntree={}\ntree_quick={}\n", self.dblup as u8, self.tree as u8, self.tree_quick as u8);
        o += &format!("hotkey={}\n", self.hotkey as u8);
        o += &format!("dialog_jump={}\n", self.dlgjump as u8);
        o += &format!("follow_desktop={}\n", self.bring_here as u8);
        o += &format!("follow_monitor={}\n", self.to_mouse as u8);
        o += &format!("tray={}\n", self.tray as u8);
        o += &format!("tabs={}\n", self.tabs_on as u8);
        o += &format!("ev_here={}\nev_sort={}\nupdate_check={}\n", self.ev_here as u8, self.ev_sort, self.updates as u8);
        if let Some(c) = self.ev_cols {
            o += &format!("ev_cols={:.4},{:.4},{:.4}\n", c[0], c[1], c[2]);
        }
        o += &format!("key_tips={}\n", self.key_tips as u8);
        if self.key_tips_seen.iter().any(|&n| n != 0) {
            o += &format!("key_tips_seen={}\n", self.key_tips_seen.iter().map(|n| n.to_string()).collect::<String>());
        }
        o += &format!("ctrl_hints={}\n", self.ctrl_hints as u8);
        o += &format!("lang={}\n", if self.ko { "ko" } else { "en" });
        if !self.file.is_empty() && one(&self.file) {
            o += &format!("session={}\n", self.file);
        }
        if !self.order.is_empty() {
            o += &format!("order={}\n", join(&self.order));
        }
        for f in self.favs.iter().filter(|f| one(f)) {
            o += &format!("fav={f}\n");
        }
        for r in self.recent.iter().filter(|r| one(r)) {
            o += &format!("recent={r}\n");
        }
        for p in self.rn_presets.iter().filter(|p| one(p)) {
            o += &format!("rename_preset={p}\n");
        }
        for i in 0..MAXP {
            if !self.paths[i].is_empty() && one(&self.paths[i]) {
                o += &format!("p{i}={}\n", self.paths[i]);
            }
            if self.views[i].0 != 0 {
                o += &format!("v{i}={},{}\n", self.views[i].0, self.views[i].1);
            }
            if !self.cols[i].is_empty() && one(&self.cols[i]) {
                o += &format!("c{i}={}\n", self.cols[i]);
            }
            if self.tints[i] != 0 {
                o += &format!("t{i}={}\n", self.tints[i]);
            }
            let tabs = self.tab_list(i);
            if tabs.len() > 1 && tabs.iter().all(|t| one(t)) {
                o += &format!("b{i}={}|{}\n", self.tab[i], tabs.join("|"));
            }
        }
        o
    }

    pub(crate) fn is_fav(&self, path: &str) -> bool {
        !path.is_empty() && self.favs.iter().any(|f| f.eq_ignore_ascii_case(path))
    }

    /// What goes into a named session file: no window position, no pointer to itself, no favourites or recent folders.
    pub(crate) fn snapshot(&self) -> String {
        // shortcut tips and Ctrl hints are app-wide: a session file carries the default switches and no counts
        let (key_tips, key_tips_seen, ctrl_hints) = (true, [0; TIP_N], true);
        // the tree width is not part of a session's layout (take_layout), so no DPI for it either
        Session { win: None, places: Vec::new(), side_dpi: 0, file: String::new(), favs: Vec::new(), recent: Vec::new(), order: Vec::new(), rn_presets: Vec::new(), key_tips, key_tips_seen, ctrl_hints, ..self.clone() }.to_ini()
    }

    /// Takes grid + folders from a named session; window position and sidebar width stay as they are.
    pub(crate) fn take_layout(&mut self, o: &Session) {
        (self.heights, self.colr, self.rowr) = (o.heights.clone(), o.colr.clone(), o.rowr.clone());
        (self.active, self.paths, self.views, self.cols) = (o.active, o.paths.clone(), o.views.clone(), o.cols.clone());
        self.tints = o.tints.clone();
        (self.tabs, self.tab) = (o.tabs.clone(), o.tab.clone());
        self.order.clear();
    }

    /// Slots i and j trade folders, views, columns and labels; the active pane goes with its slot.
    pub(crate) fn swap_slots(&mut self, i: usize, j: usize) {
        self.paths.swap(i, j);
        self.views.swap(i, j);
        self.cols.swap(i, j);
        self.tints.swap(i, j);
        self.tabs.swap(i, j);
        self.tab.swap(i, j);
        if self.active == i {
            self.active = j;
        } else if self.active == j {
            self.active = i;
        }
    }

    /// Slot i takes what slot order[i] held; an empty order leaves everything where it is.
    pub(crate) fn reorder(&mut self, order: &[u8]) {
        if order.len() != MAXP {
            return;
        }
        let old = self.clone();
        for (i, &k) in order.iter().enumerate() {
            let k = k as usize;
            (self.paths[i], self.views[i], self.cols[i], self.tints[i]) = (old.paths[k].clone(), old.views[k], old.cols[k].clone(), old.tints[k]);
            (self.tabs[i], self.tab[i]) = (old.tabs[k].clone(), old.tab[k]);
        }
    }
}

impl Session {
    /// Pane i's tabs as they are now: the current one is the folder on display. One tab when it has none.
    pub(crate) fn tab_list(&self, i: usize) -> Vec<String> {
        let mut t = self.tabs[i].clone();
        match self.tab[i] {
            k if t.len() > 1 && k < t.len() => t[k] = self.paths[i].clone(),
            _ => t = vec![self.paths[i].clone()],
        }
        t
    }

    /// Ctrl+T: a copy of the current tab right after it, made current. Nothing to browse, it is the same folder.
    pub(crate) fn add_tab(&mut self, i: usize) {
        let mut t = self.tab_list(i);
        let k = self.tab[i].min(t.len() - 1) + 1;
        t.insert(k, self.paths[i].clone());
        (self.tabs[i], self.tab[i]) = (t, k);
    }

    /// Ctrl+Enter / Ctrl+Shift+Enter in a file list: `paths` as new tabs right after the current one, in order
    /// (at most OPEN_TABS_MAX). `switch`: the first new one becomes current, its folder to browse to; otherwise
    /// the current tab and the folder on display stay (None). Nothing to open: nothing changes.
    pub(crate) fn open_tabs(&mut self, i: usize, paths: &[String], switch: bool) -> Option<String> {
        let paths = &paths[..paths.len().min(OPEN_TABS_MAX)];
        let first = paths.first()?.clone();
        let mut t = self.tab_list(i);
        let cur = self.tab[i].min(t.len() - 1);
        t.splice(cur + 1..cur + 1, paths.iter().cloned());
        (self.tabs[i], self.tab[i]) = (t, if switch { cur + 1 } else { cur });
        switch.then_some(first)
    }

    /// Ctrl+K / the tab menu's Duplicate: a copy of tab k right after it, made current. The folder to browse to,
    /// None when k was the current one (same folder) or there is no such tab.
    pub(crate) fn dup_tab(&mut self, i: usize, k: usize) -> Option<String> {
        let mut t = self.tab_list(i);
        if k >= t.len() {
            return None;
        }
        let cur = if t.len() > 1 { self.tab[i] } else { 0 };
        let to = t[k].clone();
        t.insert(k + 1, to.clone());
        (self.tabs[i], self.tab[i]) = (t, k + 1);
        (k != cur).then_some(to)
    }

    /// Tab k becomes current: the folder to browse to, None when it already is (or there is no such tab).
    pub(crate) fn pick_tab(&mut self, i: usize, k: usize) -> Option<String> {
        let t = self.tab_list(i);
        if k >= t.len() || k == self.tab[i] {
            return None;
        }
        let to = t[k].clone();
        (self.tabs[i], self.tab[i]) = (t, k);
        Some(to)
    }

    /// Ctrl+Shift+PgUp / PgDn, the tab menu's Move left / right: tab k trades places with its neighbour d (±1)
    /// away; the current tab follows its own entry. No browsing. False (nothing changes) past either end or with one tab.
    /// The raw entries swap (not tab_list's), so the current one's stale entry stays its own and paths[i] still shows it.
    pub(crate) fn move_tab(&mut self, i: usize, k: usize, d: i32) -> bool {
        let n = self.tabs[i].len();
        if n < 2 || self.tab[i] >= n || k >= n {
            return false;
        }
        let Some(j) = k.checked_add_signed(d as isize).filter(|&j| j < n && j != k) else { return false };
        self.tabs[i].swap(k, j);
        let cur = &mut self.tab[i];
        if *cur == k {
            *cur = j;
        } else if *cur == j {
            *cur = k;
        }
        true
    }

    /// Closes tab k; the last one stays. Closing the current one moves to its right neighbour (else its left):
    /// that folder to browse to.
    pub(crate) fn close_tab(&mut self, i: usize, k: usize) -> Option<String> {
        let mut t = self.tab_list(i);
        if t.len() < 2 || k >= t.len() {
            return None;
        }
        let cur = self.tab[i];
        t.remove(k);
        let (next, go) = match k.cmp(&cur) {
            std::cmp::Ordering::Less => (cur - 1, None),
            std::cmp::Ordering::Greater => (cur, None),
            std::cmp::Ordering::Equal => {
                let n = k.min(t.len() - 1);
                (n, Some(t[n].clone()))
            }
        };
        (self.tabs[i], self.tab[i]) = if t.len() > 1 { (t, next) } else { (Vec::new(), 0) };
        go
    }

    /// Ctrl+Shift+T: a closed tab's folder back as a tab at place `at` (0..=len; none or past the end: right after
    /// the current one), made current. The folder to browse to, None when it is the one on display already.
    pub(crate) fn reopen_tab(&mut self, i: usize, at: Option<usize>, path: &str) -> Option<String> {
        let mut t = self.tab_list(i);
        let cur = if t.len() > 1 { self.tab[i].min(t.len() - 1) } else { 0 };
        let k = at.filter(|&k| k <= t.len()).unwrap_or(cur + 1);
        t.insert(k, path.to_string());
        (self.tabs[i], self.tab[i]) = (t, k);
        (!path.eq_ignore_ascii_case(&self.paths[i])).then(|| path.to_string())
    }
}

/// Panes i and j trade places in the current state and in the save baseline (so the two still line up pane
/// by pane); the current order notes the move, which the session file doesn't know about.
pub(crate) fn swap_in(cur: &mut Session, base: Option<&mut Session>, i: usize, j: usize) {
    cur.swap_slots(i, j);
    if let Some(b) = base {
        b.swap_slots(i, j);
    }
    if cur.order.is_empty() {
        cur.order = (0..MAXP as u8).collect();
    }
    cur.order.swap(i, j);
    if !moved(&cur.order) {
        cur.order.clear();
    }
}

/// A layout change moved panes (slot j now shows what slot perm[j] held): as swap_in, for any permutation.
pub(crate) fn permute_in(cur: &mut Session, base: Option<&mut Session>, perm: &[usize; MAXP]) {
    let p = perm.map(|k| k as u8);
    for s in std::iter::once(&mut *cur).chain(base) {
        let act = s.active;
        s.reorder(&p);
        s.active = perm.iter().position(|&k| k == act).unwrap_or(0);
    }
    if cur.order.is_empty() {
        cur.order = (0..MAXP as u8).collect();
    }
    cur.order = p.iter().map(|&k| cur.order[k as usize]).collect();
    if !moved(&cur.order) {
        cur.order.clear();
    }
}

fn moved(order: &[u8]) -> bool {
    order.iter().enumerate().any(|(k, &o)| k != o as usize)
}

/// Tree width `side` measured at DPI `from`, at DPI `to`. Rounded, so 96 to 120 and back does not drift a pixel,
/// and in i64, so a huge saved width cannot overflow. An unknown DPI (0) leaves it as it is.
pub(crate) fn side_at(side: i32, from: i32, to: i32) -> i32 {
    if from <= 0 || to <= 0 {
        return side;
    }
    let (s, f, t) = (side as i64, from as i64, to as i64);
    let v = if s >= 0 { (2 * s * t + f) / (2 * f) } else { -((-2 * s * t + f) / (2 * f)) };
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// The DPI grids.ini's tree width was saved at: its side_dpi=, else (an older ini) the DPI of the monitor its last
/// window position lies on (`win_dpi`), else not known.
pub(crate) fn side_from(side_dpi: i32, win_dpi: Option<i32>) -> Option<i32> {
    let ok = |d: i32| (48..=960).contains(&d);
    if ok(side_dpi) {
        Some(side_dpi)
    } else {
        win_dpi.filter(|&d| ok(d))
    }
}

/// Startup tree width at `made`, the DPI of the monitor the window is created on, before it moves (hidden) to its
/// saved place, whose WM_DPICHANGED (side_at) takes it back to `from`'s. 0 or less: the default 240 DIP at `made`.
/// A width blown up by older starts (each multiplied it by the DPI ratio) is held to 1200 DIP of the DPI it was saved at.
pub(crate) fn start_side(side: i32, from: Option<i32>, made: i32) -> i32 {
    if side <= 0 {
        return 240 * made / 96;
    }
    let from = from.unwrap_or(made);
    side_at(side.min(1200 * from / 96), from, made)
}

/// The save dot: panes moved away from where the session file has them, or a layout change.
pub(crate) fn session_dirty(cur: &Session, base: Option<&Session>) -> bool {
    moved(&cur.order) || base.is_some_and(|b| layout_differs(cur, b))
}

/// Baseline at startup: the session file as saved, put into the panes' current order. No file, or an
/// unreadable one: the current state, and the order alone keeps the dot on.
pub(crate) fn startup_base(cur: &Session, saved: Option<&str>) -> Session {
    match saved {
        Some(t) => {
            let mut b = Session::parse(t);
            b.reorder(&cur.order);
            b
        }
        None => cur.clone(),
    }
}

/// Layout changes worth a save prompt: grid, split ratios, view modes. Folder changes don't count.
pub(crate) fn layout_differs(cur: &Session, base: &Session) -> bool {
    let near = |x: &[f64], y: &[f64]| x.len() == y.len() && x.iter().zip(y).all(|(a, b)| (a - b).abs() < 0.005);
    cur.heights != base.heights
        || !near(&cur.colr, &base.colr)
        || cur.rowr.iter().zip(&base.rowr).any(|(a, b)| !near(a, b))
        || (0..cur.panes()).any(|i| base.views[i].0 != 0 && cur.views[i] != base.views[i])
        || (0..cur.panes()).any(|i| !base.cols[i].is_empty() && cur.cols[i] != base.cols[i])
        || (0..cur.panes()).any(|i| cur.tints[i] != base.tints[i])
}

// "X:…", any case: the upper-case letter and what follows the ':'. None for \\server, ::{GUID}, "1:\", "가:\".
// The one rule the drive checks build on (drive_root, drive_of, drive_bit, drive_path, disk_folder …); each then
// says what may follow the ':'. Bytes first: the first two are ASCII, so [2..] is on a character boundary.
pub(crate) fn drive_letter(p: &str) -> Option<(u8, &str)> {
    match p.as_bytes() {
        [c, b':', ..] if c.is_ascii_alphabetic() => Some((c.to_ascii_uppercase(), &p[2..])),
        _ => None,
    }
}

// Drives, folders and shares; not shell places like This PC or the Recycle Bin, whose columns are their own.
pub(crate) fn is_fs(path: &str) -> bool {
    path.get(1..3) == Some(":\\") || path.starts_with(r"\\")
}

// Address bar crumbs of a parsing name: (shown name, folder to go to), drive or \\server\share first.
// Shell places (This PC ...) have none.
pub(crate) fn crumbs(parse: &str) -> Vec<(String, String)> {
    if !is_fs(parse) {
        return Vec::new();
    }
    let (root, go, rest) = match parse.strip_prefix(r"\\") {
        Some(unc) => {
            let mut it = unc.splitn(3, '\\');
            let root = format!(r"\\{}", it.by_ref().take(2).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\\"));
            (root.clone(), root, it.next().unwrap_or(""))
        }
        None => (parse[..2].to_string(), parse[..3].to_string(), &parse[3..]),
    };
    let mut path = root.clone();
    let mut out = vec![(root, go)];
    for part in rest.split('\\').filter(|p| !p.is_empty()) {
        path = format!(r"{path}\{part}");
        out.push((part.to_string(), path.clone()));
    }
    out
}

// Crumb widths `w` into `avail` px: (first crumb shown, '…' shown, width of the last).
// All of it when it fits; else the current folder (cut to fit), then parents nearest first, keeping room for '…'.
pub(crate) fn crumb_fit(w: &[i32], avail: i32, sep: i32, ell: i32, min: i32) -> (usize, bool, i32) {
    let n = w.len();
    if n == 0 || avail < min {
        return (n, false, 0);
    }
    if w.iter().sum::<i32>() + sep * (n as i32 - 1) <= avail {
        return (0, false, w[n - 1]);
    }
    let last = w[n - 1].min(avail);
    let (mut first, mut used) = (n - 1, last);
    for k in (0..n - 1).rev() {
        if used + w[k] + sep + if k > 0 { ell + sep } else { 0 } > avail {
            break;
        }
        used += w[k] + sep;
        first = k;
    }
    (first, first > 0 && used + ell + sep <= avail, last)
}

// crumb_fit with room kept for a '›' after the current folder (its subfolders), when that still leaves `min`.
// Shell places and fields too narrow for it lay out exactly as before. (first, '…' shown, last width, tail '›')
pub(crate) fn crumb_fit_tail(w: &[i32], avail: i32, sep: i32, ell: i32, min: i32, fs: bool) -> (usize, bool, i32, bool) {
    let tail = fs && avail - sep >= min;
    let (first, more, last) = crumb_fit(w, if tail { avail - sep } else { avail }, sep, ell, min);
    (first, more, last, tail)
}

pub(crate) const RECENT: usize = 20;

// A pane moving from `old` to `new` puts both at the front of the recent list. Coming back to the folder it
// already holds (session restore, theme/language rebuild, refresh) is no visit; neither is a fresh pane or a shell place.
pub(crate) fn visit(list: &mut Vec<String>, old: &str, new: &str) {
    if old.is_empty() || old.eq_ignore_ascii_case(new) || !is_fs(new) {
        return;
    }
    for p in [old, new].into_iter().filter(|p| is_fs(p)) {
        list.retain(|x| !x.eq_ignore_ascii_case(p));
        list.insert(0, p.to_string());
    }
    list.truncate(RECENT);
}

// The recent menu of a pane: everything but the folder it shows.
pub(crate) fn recent_items(list: &[String], cur: &str) -> Vec<String> {
    list.iter().filter(|p| !p.eq_ignore_ascii_case(cur)).cloned().collect()
}

// Address bar text that names no path and runs nothing: the recent (newest first), then favourite folder whose
// name is it, else starts with it, else holds it; never the folder on display. Like zoxide's `z name`.
pub(crate) fn jump_pick<'a>(q: &str, recent: &'a [String], favs: &'a [String], cur: &str) -> Option<&'a str> {
    let q = q.trim().to_lowercase();
    if q.is_empty() || q.contains(['\\', '/', ':', '"']) {
        return None;
    }
    let score = |p: &str| {
        let n = short_name(p).to_lowercase();
        if n == q { Some(0) } else if n.starts_with(&q) { Some(1) } else { n.contains(&q).then_some(2) }
    };
    let all = recent.iter().chain(favs).filter(|p| is_fs(p) && !p.eq_ignore_ascii_case(cur));
    all.filter_map(|p| Some((score(p)?, p.as_str()))).min_by_key(|(s, _)| *s).map(|(_, p)| p)
}

// Enter in the address bar fell through (`done` None): a jump only if the text named no path at all;
// a path that resolved but wouldn't open just beeps.
pub(crate) fn addr_jumps(done: Option<bool>, resolved: bool) -> bool {
    done.is_none() && !resolved
}

// A view's columns: visible ones in order with their widths, then the sort columns, each property as
// "format GUID/property id": "GUID/pid=width,...;GUID/pid=direction,...".
pub(crate) fn cols_text(cols: &[(PROPERTYKEY, u32)], sort: &[(PROPERTYKEY, i32)]) -> String {
    let key = |k: &PROPERTYKEY| format!("{:?}/{}", k.fmtid, k.pid);
    let cols: Vec<String> = cols.iter().map(|(k, w)| format!("{}={w}", key(k))).collect();
    let sort: Vec<String> = sort.iter().map(|(k, d)| format!("{}={d}", key(k))).collect();
    format!("{};{}", cols.join(","), sort.join(","))
}

pub(crate) fn parse_cols(t: &str) -> (Vec<(PROPERTYKEY, u32)>, Vec<(PROPERTYKEY, i32)>) {
    fn items<T: std::str::FromStr>(s: &str) -> Vec<(PROPERTYKEY, T)> {
        let item = |e: &str| {
            let (k, v) = e.split_once('=')?;
            let (g, pid) = k.split_once('/')?;
            Some((PROPERTYKEY { fmtid: GUID::try_from(g).ok()?, pid: pid.parse().ok()? }, v.parse().ok()?))
        };
        s.split(',').filter_map(item).collect()
    }
    let (cols, sort) = t.split_once(';').unwrap_or((t, ""));
    (items(cols), items(sort))
}

// Sort key pressed again on the column already on top: the other way round; else that key's own first direction.
pub(crate) fn sort_next(cur: Option<(PROPERTYKEY, i32)>, want: PROPERTYKEY, first_dir: i32) -> i32 {
    match cur {
        Some((k, d)) if k == want => if d < 0 { 1 } else { -1 },
        _ => first_dir,
    }
}

// `text` (cols_text) sorted by one column; the columns and widths half stays as it is, character for character.
pub(crate) fn with_sort(text: &str, key: PROPERTYKEY, dir: i32) -> String {
    let cols = text.split_once(';').map_or(text, |c| c.0);
    format!("{cols}{}", cols_text(&[], &[(key, dir)]))
}

// Existing relative paths become absolute; shell names like ::{GUID} pass through.
pub(crate) fn resolve(p: String) -> String {
    match Path::new(&p).exists() {
        true => std::path::absolute(&p).map(|x| x.to_string_lossy().into_owned()).unwrap_or(p),
        false => p,
    }
}

/// One command-line argument in quotes, read back unchanged by Windows' argument rules:
/// backslashes before a quote and at the end are doubled, so "D:\\" keeps its closing quote.
/// Used for both the jump list's arguments and WinMerge's.
pub(crate) fn quote_arg(s: &str) -> String {
    let (mut o, mut bs) = (String::from("\""), 0);
    for c in s.chars() {
        if c == '\\' {
            bs += 1;
            continue;
        }
        let n = if c == '"' { bs * 2 + 1 } else { bs };
        o.push_str(&"\\".repeat(n));
        o.push(c);
        bs = 0;
    }
    o.push_str(&"\\".repeat(bs * 2));
    o.push('"');
    o
}

/// This running exe's full path, as written to the registry (Run key, Explorer's menu).
pub(crate) fn this_exe() -> Option<String> {
    std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned())
}

/// Same folder, ignoring case and a trailing backslash (C:\x vs c:\X\).
pub(crate) fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

pub(crate) fn is_session_file(p: &Path) -> bool {
    // .qdr: sessions saved before the rename
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("grids") || x.eq_ignore_ascii_case("qdr"))
}

// A lone session-file argument means "load this session".
pub(crate) fn session_arg(args: &[String]) -> Option<&str> {
    match args {
        [f] if is_session_file(Path::new(f)) => Some(f),
        _ => None,
    }
}

// `grids.exe A B C` opens the paths in pane order, growing the grid if needed.
pub(crate) fn apply_args(s: &mut Session, args: Vec<String>) {
    if args.is_empty() {
        return;
    }
    let n = args.len().min(MAXP);
    for (i, p) in args.into_iter().take(MAXP).enumerate() {
        s.paths[i] = p;
    }
    if n > s.panes() {
        let c = (n as f64).sqrt().ceil() as usize;
        s.set_heights(vec![n.div_ceil(c); c]);
    }
    s.active = 0;
}

// The first start's command line. A lone session file brings its layout and becomes the file Ctrl+S saves to,
// as when a running Grids is handed one (load_session_file). One that can't be read changes nothing: false.
pub(crate) fn startup_args(s: &mut Session, mut args: Vec<String>, read: impl Fn(&str) -> Option<String>) -> bool {
    let folders = take_folder_flag(&mut args); // a jump-list folder link: folders only, even one named like a session file
    if let Some(f) = session_arg(&args).filter(|_| !folders) {
        let Some(t) = read(f) else {
            return false;
        };
        s.take_layout(&Session::parse(&t));
        s.file = f.to_string();
        return true;
    }
    apply_args(s, args);
    true
}

// The theme to switch to when following Windows: only when on, Windows says, and it differs from now.
pub(crate) fn theme_target(follow: bool, sys: Option<bool>, cur: bool) -> Option<bool> {
    sys.filter(|&d| follow && d != cur)
}

pub(crate) fn ini_path() -> PathBuf {
    std::env::current_exe().unwrap_or_default().with_file_name("grids.ini")
}

// grids.ini in `dir`, else qdir.ini (settings from before the rename), else None for defaults.
// Only "not there" falls through: a locked or unreadable grids.ini is an error, so it is never replaced by defaults.
pub(crate) fn load_ini(dir: &Path) -> std::io::Result<Option<String>> {
    for name in ["grids.ini", "qdir.ini"] {
        match std::fs::read(dir.join(name)) {
            Ok(b) => return Ok(Some(String::from_utf8_lossy(&b).into_owned())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

// Write next to `path` and swap it in, so a cut-off write never leaves half an ini; on failure the old file stays.
// On failure it tells whether grids.ini.tmp itself could not be made (true: the folder takes no new file)
// or a later step failed (false: writing it, or swapping it in while something holds grids.ini).
pub(crate) fn write_ini(path: &Path, text: &str) -> std::result::Result<(), (bool, std::io::Error)> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut f = std::fs::File::create(&tmp).map_err(|e| (true, e))?;
    let r = f.write_all(text.as_bytes()).and_then(|_| f.sync_all());
    drop(f);
    let r = r.and_then(|_| std::fs::rename(&tmp, path));
    if r.is_err() {
        _ = std::fs::remove_file(&tmp);
    }
    r.map_err(|e| (false, e))
}

pub(crate) fn sessions_dir() -> PathBuf {
    ini_path().with_file_name("sessions")
}

pub(crate) fn human(b: u64) -> String {
    let (mut v, mut k) = (b as f64, 0);
    // step up at 1023.95, not 1024: anything below that constant prints "1023.9" and anything at or above
    // it becomes "1.0" of the next unit, so "1024.0 KB/MB/GB" never shows (bytes are whole, so 1024 B is still 1.0 KB)
    while v >= 1023.95 && k < 4 {
        v /= 1024.0;
        k += 1;
    }
    match k {
        0 => format!("{b} B"),
        _ => format!("{v:.1} {}", ["", "KB", "MB", "GB", "TB"][k]),
    }
}

/// "pdf; *.txt" -> "*pdf*;*.txt" for PathMatchSpec: plain words match anywhere in the name.
pub(crate) fn filter_spec(text: &str) -> String {
    let parts = text.split(';').map(str::trim).filter(|t| !t.is_empty());
    let parts = parts.map(|t| if t.contains(['*', '?']) { t.to_string() } else { format!("*{t}*") });
    parts.collect::<Vec<_>>().join(";")
}

/// Longest filter the empty-list words show before '…': a kind's spec (*.jpg;*.jpeg;…) is far longer.
pub(crate) const EMPTY_SPEC_MAX: usize = 40;

/// The words a filtered list shows when nothing is left in it (no match and no subfolder), instead of
/// Windows' "This folder is empty.": the filter as the status line shows it, kept as typed (& and all).
pub(crate) fn empty_text(spec: &str, ko: bool) -> String {
    let shown = match spec.char_indices().nth(EMPTY_SPEC_MAX) {
        Some((cut, _)) => format!("{}…", &spec[..cut]),
        None => spec.to_string(),
    };
    let (en, kr) = ("Nothing here matches the name filter “{spec}” · Press Ctrl+F, then Esc to clear it", "이름 필터 “{spec}”에 맞는 항목이 없습니다 · Ctrl+F 뒤 Esc로 필터를 지우면 모두 보입니다");
    if ko { kr } else { en }.replace("{spec}", &shown)
}

/// Most tabs one Ctrl+Enter opens: tabs have no least width, so many more at once would leave them threads.
pub(crate) const OPEN_TABS_MAX: usize = 8;

/// A path a tab can keep: tabs are saved as "b{i}=current|path|path", so a '|' would split it into false tabs
/// and a line break would drop the pane's whole tab line.
pub(crate) fn tab_path_ok(p: &str) -> bool {
    !p.is_empty() && !p.contains(['|', '\r', '\n'])
}

/// A folder you browse, not a ZIP/CAB/7z (Windows reports those as FOLDER|STREAM): those match by name like files.
pub(crate) fn real_folder(attr: u32) -> bool {
    attr & SFGAO_FOLDER.0 != 0 && attr & SFGAO_STREAM.0 == 0
}

/// The filter keeps real folders without looking at the name, and anything whose attributes couldn't be read (None).
pub(crate) fn filter_keeps_unnamed(attr: Option<u32>) -> bool {
    attr.map_or(true, real_folder)
}

/// "a.PDF" -> "*.PDF" for the filter's "same type as the selected file"; folders and names without an extension: None.
pub(crate) fn same_type(name: &str, is_dir: bool) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    let bad = is_dir || stem.is_empty() || ext.is_empty() || ext.contains([';', '*', '?']);
    (!bad).then(|| format!("*.{ext}"))
}


// ---------- path names for titles and labels ----------

pub(crate) fn short_name(path: &str) -> String {
    if path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        return tr("This PC", "내 PC").into();
    }
    let t = path.trim_end_matches('\\');
    t.rsplit('\\').next().filter(|x| !x.starts_with("::")).unwrap_or(t).to_string()
}

// Shell places the folder tree opens, whose whole parsing name is "::{GUID}": (GUID, English, Korean).
const PLACES: [(&str, &str, &str); 8] = [
    ("645FF040-5081-101B-9F08-00AA002F954E", "Recycle Bin", "휴지통"),
    ("F02C1A0D-BE21-4350-88B0-7367FC96EF3C", "Network", "네트워크"),
    (CONTROL_PANEL, "Control Panel", "제어판"),
    ("21EC2020-3AEA-1069-A2DD-08002B30309D", "Control Panel", "제어판"),
    (LIBRARIES, "Libraries", "라이브러리"),
    ("F874310E-B6B7-47DC-BC84-B9E6B38F5934", "Home", "홈"),
    ("679F85CB-0220-4080-B29B-5540CC05AAB6", "Quick access", "빠른 액세스"),
    ("E88865EA-0E1C-4E20-9AA6-EDCD0212C87C", "Gallery", "갤러리"),
];
const CONTROL_PANEL: &str = "26EE0668-A00A-44D7-9371-BEB064C98683";
const LIBRARIES: &str = "031E4825-7B94-4DC3-B131-E946B44C8DD5";
const LIBRARY_NAMES: [(&str, &str); 4] = [("Documents", "문서"), ("Music", "음악"), ("Pictures", "사진"), ("Videos", "동영상")];

/// A tab's or session card's label: a shell place by its name instead of "::{GUID}" text, else short_name.
pub(crate) fn place_label(path: &str) -> String {
    shell_place(path, KO.load(Ordering::Relaxed)).unwrap_or_else(|| short_name(path))
}

/// The name of a shell place in one of the exact parsing-name forms Windows gives; None for anything else
/// (folders, UNC, unknown GUIDs, searches without a term), which keeps short_name's label.
pub(crate) fn shell_place(path: &str, ko: bool) -> Option<String> {
    let pick = |en: &str, k: &str| if ko { k } else { en }.to_string();
    if path.len() > 10 && path.is_char_boundary(10) && path[..10].eq_ignore_ascii_case("search-ms:") {
        return search_term(&path[10..]).map(|t| format!("{}: {t}", pick("Search", "검색")));
    }
    let (head, tail) = path.split_once('\\').unwrap_or((path, ""));
    let guid = head.strip_prefix("::{")?.strip_suffix('}')?;
    let &(id, en, k) = PLACES.iter().find(|p| p.0.eq_ignore_ascii_case(guid))?;
    match tail {
        "" => Some(pick(en, k)),
        // Control Panel's category view: "::{26EE0668-…}\0"
        "0" if id == CONTROL_PANEL => Some(pick(en, k)),
        // one library: "::{031E4825-…}\Documents.library-ms"
        t if id == LIBRARIES && !t.contains('\\') && t.len() > 11 && t.is_char_boundary(t.len() - 11)
            && t[t.len() - 11..].eq_ignore_ascii_case(".library-ms") =>
        {
            let stem = &t[..t.len() - 11];
            let known = LIBRARY_NAMES.iter().find(|l| l.0.eq_ignore_ascii_case(stem));
            Some(known.map_or_else(|| stem.to_string(), |l| pick(l.0, l.1)))
        }
        _ => None,
    }
}

// The words searched for in a search-ms query ("query=" or a System.Generic.String crumb), percent-decoded;
// a term that isn't valid UTF-8 once decoded ("%FF") is skipped rather than shown as "�".
fn search_term(q: &str) -> Option<String> {
    let decode = |s: &str| {
        let (b, mut out, mut i) = (s.as_bytes(), Vec::new(), 0);
        while i < b.len() {
            let hex = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok());
            match hex {
                Some(v) if b[i] == b'%' => {
                    out.push(v);
                    i += 3;
                }
                _ => {
                    out.push(b[i]);
                    i += 1;
                }
            }
        }
        String::from_utf8(out).ok()
    };
    q.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        let v = decode(v)?;
        let t = match k.to_ascii_lowercase().as_str() {
            "query" => v,
            "crumb" => v.strip_prefix("System.Generic.String:")?.to_string(),
            _ => return None,
        };
        let t = t.trim();
        (!t.is_empty()).then(|| t.to_string())
    })
}

// Title bar, taskbar and Alt+Tab follow the active pane's folder.
pub(crate) fn window_title(disp: &str) -> String {
    if disp.is_empty() { "Grids".into() } else { format!("{} - Grids", short_name(disp)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots<T: Clone>(empty: T, set: &[(usize, T)]) -> Vec<T> {
        let mut v = vec![empty; MAXP];
        for (i, x) in set {
            v[*i] = x.clone();
        }
        v
    }
    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    // finite, positive, summing to 1 (within rounding)
    fn sane(r: &[f64]) -> bool {
        r.iter().all(|x| x.is_finite() && *x > 0.0) && (r.iter().sum::<f64>() - 1.0).abs() < 1e-9
    }

    #[test]
    fn fix_rejects_inf_nan_and_overflowing_sums() {
        let inf = f64::INFINITY;
        for bad in [vec![inf, 1.0], vec![f64::NAN, 1.0], vec![-inf, 1.0], vec![1e308, 1e308], vec![1.0, 1.0, 1.0]] {
            assert_eq!(fix(bad.clone(), 2), even(2), "{bad:?}");
        }
        // normal ratios keep the same x / sum result, bit for bit
        assert_eq!(fix(vec![0.25, 0.75], 2), vec![0.25, 0.75]);
        assert_eq!(fix(vec![1.0, 3.0], 2), vec![1.0 / 4.0, 3.0 / 4.0]);
    }

    #[test]
    fn inf_and_nan_ratios_in_a_file_open_as_even_panes() {
        // two columns of two panes, so colr and every rowr group line up and reach fix()'s new checks
        for text in [
            "cols=2,2\ncolr=inf,1\nrowr=inf,1|1e309,1",
            "cols=2,2\ncolr=1e308,1e308\nrowr=1e308,1e308|1,NaN",
            "cols=2,2\ncolr= Infinity , 1\nrowr= nan ,1|-inf,1",
        ] {
            let mut s = Session::parse(text);
            assert_eq!(s.heights, vec![2, 2], "{text}");
            assert_eq!(s.colr, even(2), "{text}");
            assert!(s.rowr.iter().all(|g| *g == even(2)), "{text}");
            assert!(sane(&s.colr) && s.rowr.iter().all(|g| sane(g)), "{text}");
            for r in std::iter::once(&s.colr).chain(&s.rowr) {
                assert!(cells(0, 800, r, 4).iter().all(|(a, b)| b > a), "{text}");
            }
            // moving a boundary (splitter drag, Ctrl+Alt+arrow) must not panic and keeps the ratios sane
            drag_ratio(&mut s.colr, 0, 300, 0, 800, 4, 40);
            drag_ratio(&mut s.rowr[1], 0, 9999, 0, 600, 4, 40);
            assert!(sane(&s.colr) && sane(&s.rowr[1]), "{text}");
        }
    }

    #[test]
    fn empty_text_names_the_filter() {
        assert_eq!(empty_text("*pdf*", false), "Nothing here matches the name filter “*pdf*” · Press Ctrl+F, then Esc to clear it");
        assert_eq!(empty_text("*pdf*", true), "이름 필터 “*pdf*”에 맞는 항목이 없습니다 · Ctrl+F 뒤 Esc로 필터를 지우면 모두 보입니다");
        // shown as typed: no '&' mnemonic, no {…} formatting of the user's text
        assert!(empty_text(&filter_spec("R&D"), false).contains("“*R&D*”"));
        assert!(empty_text("*{n}*", true).contains("“*{n}*”"));
        // exactly 40 characters stay whole; a kind's long spec is cut at 40 characters (not bytes) with '…'
        let forty = "가".repeat(EMPTY_SPEC_MAX);
        assert!(empty_text(&forty, true).contains(&format!("“{forty}”")));
        let docs = FILE_KINDS[3].2;
        assert!(docs.chars().count() > EMPTY_SPEC_MAX);
        let cut: String = docs.chars().take(EMPTY_SPEC_MAX).collect();
        for ko in [false, true] {
            let t = empty_text(docs, ko);
            assert!(t.contains(&format!("“{cut}…”")), "{t}");
            assert!(!t.contains(docs));
        }
        let long = "한".repeat(EMPTY_SPEC_MAX + 5);
        assert!(empty_text(&long, false).contains(&format!("“{}…”", "한".repeat(EMPTY_SPEC_MAX))));
    }

    const COLS0: &str = "{B725F130-47EF-101A-A5F1-02608C9EEBAC}/10=300,{B725F130-47EF-101A-A5F1-02608C9EEBAC}/14=120;{B725F130-47EF-101A-A5F1-02608C9EEBAC}/10=1";
    const PRESET: &str = "사진 정리|0|replace,1,0,0,0,IMG_,P_,,";

    // Every field away from its default. No `..Default::default()`: a new field won't compile until it is given
    // a value here, and so a place in to_ini / parse (or a reason to stay out of them).
    fn every() -> Session {
        Session {
            win: Some([100, 50, 1300, 850, 1]),
            places: vec![("-1920,0,0,1080;0,0,2560,1440".into(), [100, 50, 1300, 850, 1]), ("0,0,1920,1080".into(), [-8, 0, 968, 1040, 0])],
            side: 280,
            side_dpi: 144,
            heights: vec![1, 3, 2],
            colr: vec![0.25, 0.5, 0.25],
            rowr: vec![vec![1.0], vec![0.5, 0.25, 0.25], vec![0.75, 0.25]],
            active: 4,
            paths: slots(String::new(), &[(0, r"C:\Users\홍길동\문서".into()), (1, r"D:\a=b".into()), (2, r"\\nas\share\사진".into()), (3, THIS_PC.into()), (4, r"E:\Work".into()), (5, r"C:\".into())]),
            views: slots((0, 0), &[(0, (4, 48)), (2, (1, 96))]),
            accent: 0x02AB34, // leading zeros pin the 6-digit {:06X} form in ini_format_pinned
            accent_dark: 0x004C8D,
            accent_sys: true,
            dark: true,
            follow: true,
            dblup: false,
            tree: false,
            tree_quick: false,
            hotkey: false,
            dlgjump: false,
            bring_here: false,
            to_mouse: false,
            tray: true,
            file: r"C:\Grids\sessions\일.grids".into(),
            favs: strs(&[r"C:\Users\홍길동", r"\\nas\share"]),
            recent: strs(&[r"D:\a=b", r"C:\Temp"]),
            ko: true,
            cols: slots(String::new(), &[(0, COLS0.into())]),
            tints: slots(0, &[(1, 3), (5, 6)]),
            order: (0..MAXP as u8).rev().collect(),
            tabs_on: true,
            tabs: slots(Vec::new(), &[(4, strs(&[r"C:\Temp", r"E:\Work", r"D:\a=b"]))]),
            tab: slots(0, &[(4, 1)]),
            rn_presets: strs(&[PRESET]),
            ev_here: true,
            ev_cols: Some([0.25, 0.5, 0.75]),
            ev_sort: 13,
            updates: false,
            key_tips: false,
            key_tips_seen: [4, 0, 1, 2, 3, 4, 0, 1, 2],
            ctrl_hints: false,
        }
    }

    #[test]
    fn all_fields_round_trip() {
        let s = every();
        assert_eq!(s.tabs[4][s.tab[4]], s.paths[4]);
        assert_eq!(Session::parse(&s.to_ini()), s);
        let d = Session::default();
        assert_eq!(Session::parse(&d.to_ini()), d);
        assert_eq!(Session::parse(""), Session { tabs_on: false, ..d }); // no tabs= : an older ini, strips off
    }

    #[test]
    fn ini_format_pinned() {
        let lines = |v: &[&str]| v.iter().map(|l| format!("{l}\n")).collect::<String>();
        let (preset, cols0, p3) = (format!("rename_preset={PRESET}"), format!("c0={COLS0}"), format!("p3={THIS_PC}"));
        let want = [
            "win=100,50,1300,850,1",
            "win_at=-1920,0,0,1080;0,0,2560,1440|100,50,1300,850,1",
            "win_at=0,0,1920,1080|-8,0,968,1040,0",
            "side=280",
            "side_dpi=144",
            "cols=1,3,2",
            "colr=0.25,0.5,0.25",
            "rowr=1|0.5,0.25,0.25|0.75,0.25",
            "active=4",
            "accent=02AB34",
            "accent_dark=004C8D",
            "accent_sys=1",
            "dark=1",
            "follow_theme=1",
            "dblup=0",
            "tree=0",
            "tree_quick=0",
            "hotkey=0",
            "dialog_jump=0",
            "follow_desktop=0",
            "follow_monitor=0",
            "tray=1",
            "tabs=1",
            "ev_here=1",
            "ev_sort=13",
            "update_check=0",
            "ev_cols=0.2500,0.5000,0.7500",
            "key_tips=0",
            "key_tips_seen=401234012",
            "ctrl_hints=0",
            "lang=ko",
            r"session=C:\Grids\sessions\일.grids",
            "order=15,14,13,12,11,10,9,8,7,6,5,4,3,2,1,0",
            r"fav=C:\Users\홍길동",
            r"fav=\\nas\share",
            r"recent=D:\a=b",
            r"recent=C:\Temp",
            &preset,
            r"p0=C:\Users\홍길동\문서",
            "v0=4,48",
            &cols0,
            r"p1=D:\a=b",
            "t1=3",
            r"p2=\\nas\share\사진",
            "v2=1,96",
            &p3,
            r"p4=E:\Work",
            r"b4=1|C:\Temp|E:\Work|D:\a=b",
            r"p5=C:\",
            "t5=6",
        ];
        assert_eq!(every().to_ini(), lines(&want));
        let want = [
            "side=0",
            "cols=2,2",
            "colr=0.5,0.5",
            "rowr=0.5,0.5|0.5,0.5",
            "active=0",
            "accent=C5C5C5",
            "accent_dark=4B4B4B",
            "accent_sys=0",
            "dark=0",
            "follow_theme=0",
            "dblup=1",
            "tree=1",
            "tree_quick=1",
            "hotkey=1",
            "dialog_jump=1",
            "follow_desktop=1",
            "follow_monitor=1",
            "tray=0",
            "tabs=1",
            "ev_here=0",
            "ev_sort=1",
            "update_check=1",
            "key_tips=1",
            "ctrl_hints=1",
            "lang=en",
        ];
        assert_eq!(Session::default().to_ini(), lines(&want));
    }

    // One start as main.rs runs it: grids.ini read, the window made on the primary monitor (`made`), moved hidden to its
    // saved monitor (`target`, WM_DPICHANGED when the DPI differs), then saved. `win_dpi`: the DPI under its old win=.
    fn start(ini: &str, win_dpi: Option<i32>, made: i32, target: i32) -> Session {
        let mut s = Session::parse(ini);
        let from = side_from(s.side_dpi, win_dpi);
        (s.side, s.side_dpi) = (start_side(s.side, from, made), made);
        if target != made {
            (s.side, s.side_dpi) = (side_at(s.side, made, target), target);
        }
        Session::parse(&s.to_ini())
    }

    #[test]
    fn tree_width_survives_starts_across_dpis() {
        // 150% external monitor, 100% primary: before, 360 -> 540 -> 810 ... per start
        let mut ini = Session { side: 360, side_dpi: 144, ..Default::default() }.to_ini();
        for _ in 0..5 {
            let s = start(&ini, Some(144), 96, 144);
            assert_eq!((s.side, s.side_dpi), (360, 144));
            ini = s.to_ini();
        }
        // the other way round: 100% monitor, 150% primary; it does not shrink
        let mut ini = Session { side: 240, side_dpi: 96, ..Default::default() }.to_ini();
        for _ in 0..5 {
            let s = start(&ini, Some(96), 144, 96);
            assert_eq!((s.side, s.side_dpi), (240, 96));
            ini = s.to_ini();
        }
        // an older ini (no side_dpi=): the DPI under its last window position, then side_dpi= from then on
        let s = start("win=2000,0,3000,800,0\nside=360\n", Some(144), 96, 144);
        assert_eq!((s.side, s.side_dpi), (360, 144));
        assert!(s.to_ini().contains("side=360\nside_dpi=144\ncols="));
        assert_eq!(start(&s.to_ini(), Some(144), 96, 144).side, 360);
        // no saved place, no DPI known: as it was; no width yet: 240 DIP at the DPI it opens at
        assert_eq!(start("side=300\n", None, 120, 120).side, 300);
        assert_eq!(start("", None, 144, 144).side, 360);
        assert_eq!(start("side=0\nside_dpi=96\n", Some(96), 144, 96).side, 240);
        // side_dpi wins over the window's monitor; a damaged one falls back to it
        assert_eq!((side_from(120, Some(144)), side_from(0, Some(144)), side_from(5000, Some(96))), (Some(120), Some(144), Some(96)));
        assert_eq!((side_from(0, None), side_from(0, Some(0)), side_from(-1, Some(9999))), (None, None, None));
    }

    #[test]
    fn tree_width_rounds_and_caps() {
        // 96 <-> 120 back and forth: no pixel lost or gained
        let mut w = 301;
        for _ in 0..10 {
            w = side_at(side_at(w, 96, 120), 120, 96);
            assert_eq!(w, 301);
        }
        // big -> small -> big (the startup path) may move one pixel once, then holds
        let once = side_at(side_at(361, 144, 96), 96, 144);
        assert!((once - 361).abs() <= 1);
        let mut w = once;
        for _ in 0..5 {
            w = side_at(side_at(w, 144, 96), 96, 144);
            assert_eq!(w, once);
        }
        assert_eq!((side_at(360, 144, 96), side_at(360, 0, 96), side_at(360, 96, 0)), (240, 360, 360));
        // no overflow on a width grown huge by older starts
        assert_eq!(side_at(i32::MAX, 96, 960), i32::MAX);
        assert_eq!(side_at(i32::MAX / 2, 960, 96), (i32::MAX / 2) / 10);
        // held to 1200 DIP of the saved DPI, then taken to the DPI made at: back to 1200 DIP of the target
        let ini = Session { side: 1_000_000, side_dpi: 144, ..Default::default() }.to_ini();
        assert_eq!(start(&ini, None, 96, 144).side, 1800);
        assert_eq!(start("side=99999999\n", Some(96), 96, 96).side, 1200);
        assert_eq!(start_side(1100, Some(96), 96), 1100); // under the cap: untouched
    }

    #[test]
    fn oldest_file_loads() {
        // before per-column rows, follow_theme, dblup, tree, hotkey, dialog_jump, tabs, Everything, update_check, lang, order
        let old = "win=0,0,800,600,0\nside=240\ngrid=2,2\ncolr=0.5,0.5\nrowr=0.25,0.75\nactive=1\naccent=#8DB5E8\ndark=1\np0=C:\\\np1=D:\\Data\n";
        let s = Session::parse(old);
        let want = Session {
            win: Some([0, 0, 800, 600, 0]),
            side: 240,
            heights: vec![2, 2],
            rowr: vec![vec![0.25, 0.75]; 2],
            active: 1,
            accent: 0x8DB5E8,
            dark: true,
            paths: slots(String::new(), &[(0, r"C:\".into()), (1, r"D:\Data".into())]),
            tabs_on: false,
            ..Default::default()
        };
        assert_eq!(s, want);
        // the settings that came later read as they do today, not whatever Default may drift to
        assert_eq!((s.follow, s.dblup, s.tree, s.hotkey, s.dlgjump, s.updates), (false, true, true, true, true, true));
        assert_eq!((s.tabs_on, s.ev_here, s.ev_sort, s.ev_cols, s.ko), (false, false, 1, None, false));
        assert_eq!((s.accent_dark, s.order.len(), s.tints.iter().all(|&t| t == 0)), (DARK.accents[1].1, 0, true));
        assert!(!s.accent_sys); // before accent_sys=: the saved colour, as always
        assert_eq!((s.key_tips, s.key_tips_seen), (true, [0; TIP_N]));
        assert!(s.ctrl_hints); // before ctrl_hints=: holding Ctrl shows the keycaps
        assert!(!s.tray); // before tray=: stays on the taskbar
        assert!(s.tree_quick); // before tree_quick=: Quick access shown atop the tree
        assert!(s.bring_here); // before follow_desktop=: called, Grids comes to the desktop in use
        assert!(s.to_mouse); // before follow_monitor=: Win+Alt+E brings Grids to the mouse's monitor
    }

    #[test]
    fn tree_quick_reads_only_zero_as_off() {
        assert!(!Session::parse("tree_quick=0\n").tree_quick);
        for v in ["tree_quick=1\n", "tree_quick=\n", ""] {
            assert!(Session::parse(v).tree_quick, "{v:?}");
        }
        let off = Session { tree_quick: false, ..Session::default() };
        assert!(off.to_ini().contains("\ntree_quick=0\n"));
        assert_eq!(Session::parse(&off.to_ini()), off);
    }

    #[test]
    fn win_at_lines() {
        // an older ini (no win_at=) opens where win= says, as before
        let old = Session::parse("win=10,20,810,620,1\n");
        assert_eq!((old.win, old.places.len()), (Some([10, 20, 810, 620, 1]), 0));
        assert!(!Session::default().to_ini().contains("win_at="));
        // damaged lines are dropped one by one, the good ones kept in order; a repeated setup keeps its first
        let text = "win_at=0,0,1920,1080|1,2,3,4,0\nwin_at=0,0,1920,1080\nwin_at=|1,2,3,4,0\nwin_at=0,0,9,9|1,2,3,4\n\
                    win_at=0,0,9,9|1,2,3,4,5,6\nwin_at=0,0,9,9|1,x,3,4,0\nwin_at=0,0,9|9|1,2,3,4,0\nwin_at=a,b|1,2,3,4,0\n\
                    win_at=-1920,-200,0,880;0,0,1920,1080| -5, 6,7,8,1\nwin_at=0,0,1920,1080|9,9,9,9,1\n";
        let s = Session::parse(text);
        let want = vec![("0,0,1920,1080".to_string(), [1, 2, 3, 4, 0]), ("-1920,-200,0,880;0,0,1920,1080".to_string(), [-5, 6, 7, 8, 1])];
        assert_eq!((s.places.clone(), s.win), (want, None));
        assert_eq!(Session::parse(&s.to_ini()).places, s.places);
        // at most WIN_PLACES are read
        let many: String = (0..9).map(|i| format!("win_at=0,0,{i},1|0,0,1,1,0\n")).collect();
        assert_eq!(Session::parse(&many).places.len(), WIN_PLACES);
        // a named session file carries no window place of any setup
        assert!(!every().snapshot().contains("win_at=") && !every().snapshot().contains("win="));
        assert_eq!(Session::parse(&every().snapshot()).places, Vec::new());
    }

    #[test]
    fn tray_reads_only_one_as_on() {
        assert!(Session::parse("tray=1\n").tray);
        for v in ["tray=0\n", "tray=\n", "tray=yes\n", ""] {
            assert!(!Session::parse(v).tray, "{v:?}");
        }
        assert!(Session { tray: true, ..Session::default() }.to_ini().contains("\ntray=1\n"));
        assert!(Session::default().to_ini().contains("\ntray=0\n"));
    }

    #[test]
    fn newer_file_loads() {
        let text = "side=200\nfuture=1\ncols=2,1\nx3=abc\np0=C:\\A\np16=C:\\Nope\nt99=2\nnew_thing=a=b\np1=C:\\B\n";
        let want = Session {
            side: 200,
            heights: vec![2, 1],
            colr: even(2),
            rowr: vec![even(2), even(1)],
            paths: slots(String::new(), &[(0, r"C:\A".into()), (1, r"C:\B".into())]),
            tabs_on: false,
            ..Default::default()
        };
        assert_eq!(Session::parse(text), want);
        // order: only a full permutation of the 16 slots
        let order = |v: &[u8]| Session::parse(&format!("order={}\n", join(v))).order;
        let full: Vec<u8> = [1, 0].into_iter().chain(2..MAXP as u8).collect();
        assert_eq!(order(&full), full);
        assert!(order(&[1, 0]).is_empty());
        assert!(order(&[0, 0].into_iter().chain(2..MAXP as u8).collect::<Vec<_>>()).is_empty());
        assert!(order(&(0..=MAXP as u8).collect::<Vec<_>>()).is_empty());
        // a bad one later leaves a good one in place
        let both = format!("order={}\norder=1,0\n", join(&full));
        assert_eq!(Session::parse(&both).order, full);
    }

    #[test]
    fn hand_edited_file() {
        let body = "win=10,20,810,620,1\r\nside=300\r\naccent_dark=#4C8DFF\r\np0=C:\\Work\r\n";
        let bom = Session::parse(&format!("\u{feff}{body}"));
        assert_eq!(bom.win, Some([10, 20, 810, 620, 1]));
        assert_eq!((bom.side, bom.accent_dark, bom.paths[0].as_str()), (300, 0x4C8DFF, r"C:\Work"));
        assert_eq!(Session::parse(body), bom);
        assert_eq!(Session::parse("accent_dark=4C8DFF\n").accent_dark, 0x4C8DFF);
        assert_eq!(Session::parse("accent_dark= #4c8dff \n").accent_dark, 0x4C8DFF);
        // only one mark, only at the very start
        assert_eq!(Session::parse("\u{feff}\u{feff}win=10,20,810,620,1\n").win, None);
        let mid = Session::parse("side=300\n\u{feff}win=10,20,810,620,1\np1=C:\\a\u{feff}b\n");
        assert_eq!((mid.side, mid.win, mid.paths[1].as_str()), (300, None, "C:\\a\u{feff}b"));
    }

    #[test]
    fn key_tips_seen_reads_digit_by_digit() {
        let seen = |v: &str| Session::parse(&format!("key_tips_seen={v}\n")).key_tips_seen;
        assert_eq!(seen("401234012"), [4, 0, 1, 2, 3, 4, 0, 1, 2]);
        assert_eq!(seen("40123401"), [4, 0, 1, 2, 3, 4, 0, 1, 0]); // from before Ctrl+O's tip: kept, it starts at 0
        assert_eq!(seen("44"), [4, 4, 0, 0, 0, 0, 0, 0, 0]); // shorter (an older list): the rest 0
        assert_eq!(seen("1234012340"), [1, 2, 3, 4, 0, 1, 2, 3, 4]); // longer (a newer list): the rest dropped
        assert_eq!(seen("9x-4 3"), [0, 0, 0, 4, 0, 3, 0, 0, 0]); // only 0..=4 count, place by place
        assert_eq!(seen(""), [0; TIP_N]);
        assert!(!Session::parse("key_tips=0\n").key_tips);
        assert!(Session::parse("key_tips=1\n").key_tips);
        assert!(!Session::parse("ctrl_hints=0\n").ctrl_hints);
        assert!(Session::parse("ctrl_hints=1\n").ctrl_hints);
        let s = Session { key_tips_seen: [0, 0, 0, 0, 0, 0, 0, 2, 0], ..Session::default() };
        assert!(s.to_ini().contains("key_tips_seen=000000020\n"));
        assert_eq!(Session::parse(&s.to_ini()), s);
    }

    #[test]
    fn key_tip_text_and_limits() {
        assert_eq!(key_tip(TIP_UP, 0, true, false).as_deref(), Some("Tip: Alt+↑ does this too"));
        assert_eq!(key_tip(TIP_UP, 2, true, true).as_deref(), Some("팁: Alt+↑ 로도 할 수 있어요"));
        assert_eq!(key_tip(TIP_CLOSE_TAB, 0, true, false).as_deref(), Some("Tip: Ctrl+W does this too"));
        assert_eq!(key_tip(TIP_UP, TIP_MAX, true, false), None); // shown enough
        assert_eq!(key_tip(TIP_UP, TIP_LEARNED, true, false), None); // the key was used
        assert_eq!(key_tip(TIP_UP, 0, false, false), None); // switched off
        assert_eq!(key_tip(TIP_N, 0, true, false), None);
    }

    #[test]
    fn take_layout_boundary() {
        let b = every();
        let a = Session {
            win: Some([1, 2, 3, 4, 0]),
            places: vec![("0,0,800,600".into(), [5, 6, 7, 8, 1])],
            side: 150,
            side_dpi: 120,
            heights: vec![4],
            colr: vec![1.0],
            rowr: vec![vec![0.25; 4]],
            active: 2,
            paths: slots(String::new(), &[(0, r"F:\x0".into()), (1, r"F:\x1".into()), (2, r"F:\x2".into()), (3, r"F:\x3".into())]),
            views: slots((0, 0), &[(1, (3, 32))]),
            accent: 0x010203,
            accent_dark: 0x040506,
            accent_sys: false,
            dark: false,
            follow: false,
            dblup: true,
            tree: true,
            tree_quick: true,
            hotkey: true,
            dlgjump: true,
            bring_here: true,
            to_mouse: true,
            tray: false,
            file: r"F:\a.grids".into(),
            favs: strs(&[r"F:\fav"]),
            recent: strs(&[r"F:\r"]),
            ko: false,
            cols: slots(String::new(), &[(3, "x".into())]),
            tints: slots(0, &[(0, 2)]),
            order: (0..MAXP as u8).map(|k| k ^ 1).collect(),
            tabs_on: false,
            tabs: slots(Vec::new(), &[(0, strs(&[r"F:\x0", r"F:\y"]))]),
            tab: slots(0, &[(0, 0)]),
            rn_presets: strs(&["p|1"]),
            ev_here: false,
            ev_cols: Some([0.125, 0.25, 0.5]),
            ev_sort: 5,
            updates: true,
            key_tips: true,
            key_tips_seen: [1, 1, 1, 1, 0, 0, 0, 0, 0],
            ctrl_hints: true,
        };
        let snap = b.snapshot();
        for key in ["win=", "win_at=", "session=", "fav=", "recent=", "order=", "rename_preset=", "key_tips_seen=", "key_tips=0", "ctrl_hints=0", "side_dpi="] {
            assert!(!snap.lines().any(|l| l.starts_with(key)), "{key} in a session file");
        }
        let mut got = a.clone();
        got.take_layout(&Session::parse(&snap));
        // a session brings its grid, folders and their looks; every app-wide setting stays as it was
        let want = Session {
            heights: b.heights.clone(),
            colr: b.colr.clone(),
            rowr: b.rowr.clone(),
            active: b.active,
            paths: b.paths.clone(),
            views: b.views.clone(),
            cols: b.cols.clone(),
            tints: b.tints.clone(),
            tabs: b.tabs.clone(),
            tab: b.tab.clone(),
            order: Vec::new(),
            ..a
        };
        assert_eq!(got, want);
    }

    #[test]
    fn startup_session_arg() {
        let (home, work) = (r"F:\s\home.grids".to_string(), r"F:\s\work.grids".to_string());
        let ini = Session { file: home.clone(), ..every() };
        let mut w = Session::default();
        w.set_heights(vec![1, 2]);
        (w.paths[0], w.paths[1], w.paths[2], w.active) = (r"F:\w0".into(), r"F:\w1".into(), r"F:\w2".into(), 1);
        w.views[2] = (4, 48);
        let text = w.snapshot();
        let parsed = Session::parse(&text);
        let read = |f: &str| (f == work).then(|| text.clone());
        let layout = |file: &str| Session {
            heights: parsed.heights.clone(),
            colr: parsed.colr.clone(),
            rowr: parsed.rowr.clone(),
            active: parsed.active,
            paths: parsed.paths.clone(),
            views: parsed.views.clone(),
            cols: parsed.cols.clone(),
            tints: parsed.tints.clone(),
            tabs: parsed.tabs.clone(),
            tab: parsed.tab.clone(),
            order: Vec::new(),
            file: file.into(),
            ..ini.clone()
        };

        // ① the session given becomes the save target; favourites, recent, window and theme stay grids.ini's
        let mut s = ini.clone();
        assert!(startup_args(&mut s, vec![work.clone()], read));
        assert_eq!(s, layout(&work));
        // ② the green dot's baseline is that file: nothing unsaved on opening
        let base = startup_base(&s, read(&s.file).as_deref());
        assert!(!layout_differs(&s, &base) && !session_dirty(&s, Some(&base)));

        // ⑥ the session grids.ini already points at: the same, panes back in their own slots
        let mut s = Session { file: work.clone(), ..ini.clone() };
        assert!(startup_args(&mut s, vec![work.clone()], read));
        assert_eq!(s, layout(&work));
        let base = startup_base(&s, read(&s.file).as_deref());
        assert!(!session_dirty(&s, Some(&base)));

        // ③ unreadable (deleted, locked): grids.ini's state untouched, no session path as pane 1's folder
        let mut s = ini.clone();
        assert!(!startup_args(&mut s, strs(&[r"F:\s\gone.grids"]), read));
        assert_eq!(s, ini);

        // ④ a jump-list folder link: a folder even when named like a session file
        let mut s = ini.clone();
        assert!(startup_args(&mut s, vec![FOLDER_FLAG.into(), work.clone()], |_| unreachable!()));
        let mut want = ini.clone();
        apply_args(&mut want, vec![work.clone()]);
        assert_eq!((&s, s.paths[0].as_str(), s.file.as_str()), (&want, work.as_str(), home.as_str()));

        // ⑤ several folders: as before
        let many = strs(&[r"F:\a", r"F:\b.grids", r"F:\c"]);
        let mut s = ini.clone();
        assert!(startup_args(&mut s, many.clone(), |_| unreachable!()));
        let mut want = ini.clone();
        apply_args(&mut want, many);
        assert_eq!(s, want);
        // no arguments: grids.ini as it is
        let mut s = ini.clone();
        assert!(startup_args(&mut s, Vec::new(), |_| unreachable!()));
        assert_eq!(s, ini);
    }
}
