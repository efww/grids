use crate::*;

// ---------- session (grids.ini next to the exe; named sessions are the same format as .grids) ----------

#[derive(Debug, PartialEq, Clone)]
pub(crate) struct Session {
    pub(crate) win: Option<[i32; 5]>, // normal rect l,t,r,b + maximized flag
    pub(crate) side: i32,             // sidebar width in px
    pub(crate) heights: Vec<usize>,   // panes per column, left to right (1..=4 columns of 1..=4)
    pub(crate) colr: Vec<f64>,        // column width ratios
    pub(crate) rowr: Vec<Vec<f64>>,   // row height ratios, per column
    pub(crate) active: usize,
    pub(crate) paths: Vec<String>,     // parsing names per pane slot (numbered down each column, left to right)
    pub(crate) views: Vec<(i32, i32)>, // (FOLDERVIEWMODE, icon size); 0 = shell default
    pub(crate) accent: u32,            // selection colour 0xRRGGBB, light theme
    pub(crate) accent_dark: u32,       // selection colour, dark theme
    pub(crate) dark: bool,
    pub(crate) follow: bool,           // dark follows Windows' app light/dark setting (on for a new install)
    pub(crate) dblup: bool,            // double-click on empty space goes to the parent folder
    pub(crate) tree: bool,             // folder tree shown (side keeps its width while hidden)
    pub(crate) hotkey: bool,           // Win+Alt+E brings Grids to the front, app-wide (not part of a layout)
    pub(crate) dlgjump: bool,          // Ctrl+Alt+G in other apps' file dialogs lists Grids' folders (dialog_jump.rs)
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
}

impl Default for Session {
    fn default() -> Self {
        Session {
            win: None,
            side: 0,
            heights: vec![2, 2],
            colr: even(2),
            rowr: vec![even(2); 2],
            active: 0,
            paths: vec![String::new(); MAXP],
            views: vec![(0, 0); MAXP],
            accent: LIGHT.accents[1].1, // gray
            accent_dark: DARK.accents[1].1,
            dark: false,
            follow: false,
            dblup: true,
            tree: true,
            hotkey: true,
            dlgjump: true,
            file: String::new(),
            favs: Vec::new(),
            recent: Vec::new(),
            ko: false,
            cols: vec![String::new(); MAXP],
            tints: vec![0; MAXP],
            order: Vec::new(),
            tabs_on: false,
            tabs: vec![Vec::new(); MAXP],
            tab: vec![0; MAXP],
            rn_presets: Vec::new(),
        }
    }
}

pub(crate) fn even(n: usize) -> Vec<f64> {
    vec![1.0 / n as f64; n]
}

pub(crate) fn nums<T: std::str::FromStr>(v: &str) -> Vec<T> {
    v.split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

pub(crate) fn join<T: Display>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

// Ratios from a hand-edited ini can be anything: wrong length, zero, negative -> fall back to equal.
pub(crate) fn fix(r: Vec<f64>, n: usize) -> Vec<f64> {
    if r.len() != n || r.iter().any(|x| !(*x > 0.0)) {
        return even(n);
    }
    let sum: f64 = r.iter().sum();
    r.iter().map(|x| x / sum).collect()
}

// "2 × 2" for an even grid, "1 | 3" when the columns differ.
pub(crate) fn layout_label(h: &[usize]) -> String {
    match h.iter().all(|&n| n == h[0]) {
        true => format!("{} × {}", h[0], h.len()),
        false => h.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" | "),
    }
}

impl Session {
    pub(crate) fn accent_now(&self) -> u32 {
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
        let mut s = Session::default();
        let mut rowr: Vec<Vec<f64>> = Vec::new();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let slot = |p: char| k.strip_prefix(p).and_then(|n| n.parse::<usize>().ok()).filter(|&i| i < MAXP);
            match k {
                "win" => s.win = nums(v).try_into().ok(),
                "side" => s.side = v.parse().unwrap_or(0),
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
                "accent_dark" => s.accent_dark = u32::from_str_radix(v.trim(), 16).unwrap_or(s.accent_dark) & 0xFFFFFF,
                "dark" => s.dark = v.trim() == "1",
                "follow_theme" => s.follow = v.trim() == "1",
                "dblup" => s.dblup = v.trim() != "0",
                "tree" => s.tree = v.trim() != "0",
                "hotkey" => s.hotkey = v.trim() != "0",
                "dialog_jump" => s.dlgjump = v.trim() != "0",
                "tabs" => s.tabs_on = v.trim() == "1",
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
        let mut o = String::new();
        if let Some(w) = self.win {
            o += &format!("win={}\n", join(&w));
        }
        let rowr = self.rowr.iter().map(|r| join(r)).collect::<Vec<_>>().join("|");
        o += &format!("side={}\ncols={}\n", self.side, join(&self.heights));
        o += &format!("colr={}\nrowr={rowr}\nactive={}\n", join(&self.colr), self.active);
        o += &format!("accent={:06X}\naccent_dark={:06X}\ndark={}\n", self.accent, self.accent_dark, self.dark as u8);
        o += &format!("follow_theme={}\n", self.follow as u8);
        o += &format!("dblup={}\ntree={}\nhotkey={}\n", self.dblup as u8, self.tree as u8, self.hotkey as u8);
        o += &format!("dialog_jump={}\n", self.dlgjump as u8);
        o += &format!("tabs={}\n", self.tabs_on as u8);
        o += &format!("lang={}\n", if self.ko { "ko" } else { "en" });
        if !self.file.is_empty() {
            o += &format!("session={}\n", self.file);
        }
        if !self.order.is_empty() {
            o += &format!("order={}\n", join(&self.order));
        }
        for f in &self.favs {
            o += &format!("fav={f}\n");
        }
        for r in &self.recent {
            o += &format!("recent={r}\n");
        }
        for p in &self.rn_presets {
            o += &format!("rename_preset={p}\n");
        }
        for i in 0..MAXP {
            if !self.paths[i].is_empty() {
                o += &format!("p{i}={}\n", self.paths[i]);
            }
            if self.views[i].0 != 0 {
                o += &format!("v{i}={},{}\n", self.views[i].0, self.views[i].1);
            }
            if !self.cols[i].is_empty() {
                o += &format!("c{i}={}\n", self.cols[i]);
            }
            if self.tints[i] != 0 {
                o += &format!("t{i}={}\n", self.tints[i]);
            }
            let tabs = self.tab_list(i);
            if tabs.len() > 1 {
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
        Session { win: None, file: String::new(), favs: Vec::new(), recent: Vec::new(), order: Vec::new(), rn_presets: Vec::new(), ..self.clone() }.to_ini()
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

// Existing relative paths become absolute; shell names like ::{GUID} pass through.
pub(crate) fn resolve(p: String) -> String {
    match Path::new(&p).exists() {
        true => std::path::absolute(&p).map(|x| x.to_string_lossy().into_owned()).unwrap_or(p),
        false => p,
    }
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
pub(crate) fn write_ini(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let r = std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(text.as_bytes()).and_then(|_| f.sync_all()))
        .and_then(|_| std::fs::rename(&tmp, path));
    if r.is_err() {
        _ = std::fs::remove_file(&tmp);
    }
    r
}

pub(crate) fn sessions_dir() -> PathBuf {
    ini_path().with_file_name("sessions")
}

pub(crate) fn human(b: u64) -> String {
    let (mut v, mut k) = (b as f64, 0);
    while v >= 1024.0 && k < 4 {
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

// Title bar, taskbar and Alt+Tab follow the active pane's folder.
pub(crate) fn window_title(disp: &str) -> String {
    if disp.is_empty() { "Grids".into() } else { format!("{} - Grids", short_name(disp)) }
}
