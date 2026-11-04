// Favourites are a tree of groups and leaves. Leaves are folders or files.
// The flat `favs` list is the leaves, depth-first, so the star and an old
// `fav=` line keep working. The settings page and the left panel read the tree.

use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{fav_path, short_name, Session};

/// A saved server. The password is never a field: it lives in Credential Manager.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServerLink {
    pub(crate) kind: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) remote: String,
    pub(crate) user: String,
    pub(crate) tls: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FavNode {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) file: bool,
    pub(crate) open: bool,
    pub(crate) children: Vec<FavNode>,
    /// Set on a server favourite. Absent on every older leaf.
    pub(crate) server: Option<ServerLink>,
}

impl FavNode {
    pub(crate) fn group(name: &str) -> Self {
        FavNode { name: name.to_string(), path: String::new(), file: false, open: true, children: Vec::new(), server: None }
    }

    pub(crate) fn leaf(path: &str, name: &str, file: bool) -> Self {
        FavNode { name: name.to_string(), path: path.to_string(), file, open: false, children: Vec::new(), server: None }
    }

    pub(crate) fn server_leaf(name: &str, path: &str, link: ServerLink) -> Self {
        FavNode { name: name.to_string(), path: path.to_string(), file: false, open: false, children: Vec::new(), server: Some(link) }
    }

    pub(crate) fn is_group(&self) -> bool {
        self.path.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FavRow {
    pub(crate) id: Vec<usize>,
    pub(crate) depth: u8,
    pub(crate) group: bool,
    pub(crate) file: bool,
    pub(crate) open: bool,
    pub(crate) name: String,
    pub(crate) path: String,
    /// Explorer pins, Explorer favourite files, or a node of `favorites_tree`.
    pub(crate) kind: RowKind,
}

impl FavRow {
    pub(crate) fn in_tree(&self) -> bool {
        self.kind == RowKind::Tree
    }
}

/// One child of Quick Access, before the pinned-folder filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PinCand {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) pinned: bool,
    pub(crate) folder: bool,
}

/// A filesystem folder that is pinned in Explorer. Order is Quick Access order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pinned {
    pub(crate) path: String,
    pub(crate) name: String,
}

/// Which list a side-panel row belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    Tree,
    Pins,
    Files,
}

/// What an internal favourites drag should do when it lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SideAct {
    /// Move a node inside the favourites tree.
    Move,
    /// Pin this folder in Explorer. The favourites node stays where it is.
    Pin,
    /// Pin this file on Explorer Home. The favourites node stays where it is.
    PinFile,
    /// Copy an Explorer pin into the favourites tree. The pin stays pinned.
    Copy,
    /// No indicator and no change. Explorer rows do not reorder, and the wrong kind is refused.
    None,
}

/// Path kept for navigation and for case-insensitive identity. Drive roots keep their slash.
pub(crate) fn tidy_path(path: &str) -> String {
    let t = path.trim();
    let stripped = t.trim_end_matches(['\\', '/']);
    if stripped.is_empty() {
        return String::new();
    }
    if stripped.ends_with(':') {
        format!("{stripped}\\")
    } else {
        stripped.to_string()
    }
}

fn pin_key(path: &str) -> String {
    tidy_path(path).to_ascii_lowercase()
}

fn drive_root(path: &str) -> bool {
    let bare = path.trim_end_matches(['\\', '/']);
    let b = bare.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// A filesystem file path. Blank paths and drive roots are not files.
fn file_path(path: &str) -> Option<String> {
    let path = tidy_path(path);
    if path.is_empty() || drive_root(&path) { None } else { Some(path) }
}

/// Pinned filesystem folders, in enumeration order. Files, unpinned items, empty paths, and
/// a later copy of the same path are left out.
pub(crate) fn pinned_folders(items: &[PinCand]) -> Vec<Pinned> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for it in items {
        if !it.pinned || !it.folder {
            continue;
        }
        let path = tidy_path(&it.path);
        let key = pin_key(&path);
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        let name = it.name.trim();
        let name = if name.is_empty() { short_name(&path) } else { name.to_string() };
        out.push(Pinned { path, name });
    }
    out
}

/// Pinned filesystem files, in enumeration order. Folders, unpinned items, empty paths,
/// drive roots, and a later copy of the same path are left out.
pub(crate) fn pinned_files(items: &[PinCand]) -> Vec<Pinned> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for it in items {
        if !it.pinned || it.folder {
            continue;
        }
        let Some(path) = file_path(&it.path) else { continue };
        let key = pin_key(&path);
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        let name = it.name.trim();
        let name = if name.is_empty() { short_name(&path) } else { name.to_string() };
        out.push(Pinned { path, name });
    }
    out
}

/// One Quick Access enumeration, split into pinned folders and pinned files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PinSplit {
    pub(crate) folders: Vec<Pinned>,
    pub(crate) files: Vec<Pinned>,
}

pub(crate) fn split_pins(items: &[PinCand]) -> PinSplit {
    PinSplit { folders: pinned_folders(items), files: pinned_files(items) }
}

/// The favourite-files group is drawn only when at least one pinned file came back.
pub(crate) fn file_group_visible(files: &[Pinned]) -> bool {
    !files.is_empty()
}

/// A drop onto the pins group is accepted only when every item is a filesystem folder.
/// Any file, or a folder with no filesystem path, refuses the whole drop.
pub(crate) fn pin_drop_paths(items: &[(String, bool)]) -> Option<Vec<String>> {
    if items.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for (path, folder) in items {
        let path = tidy_path(path);
        if !folder || path.is_empty() {
            return None;
        }
        if !out.iter().any(|p: &String| p.eq_ignore_ascii_case(&path)) {
            out.push(path);
        }
    }
    Some(out)
}

/// A drop onto the favourite-files group is accepted only when every item is a filesystem file.
/// Any folder, or a path that is not a file, refuses the whole drop.
pub(crate) fn file_drop_paths(items: &[(String, bool)]) -> Option<Vec<String>> {
    if items.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for (path, folder) in items {
        if *folder {
            return None;
        }
        let Some(path) = file_path(path) else { return None };
        if !out.iter().any(|p: &String| p.eq_ignore_ascii_case(&path)) {
            out.push(path);
        }
    }
    Some(out)
}

/// `src_path_empty` is a group or a leaf that has nothing to pin.
pub(crate) fn side_act(src: RowKind, src_group: bool, src_file: bool, src_path_empty: bool, dst: RowKind) -> SideAct {
    match dst {
        RowKind::Pins => {
            if src == RowKind::Tree && !src_group && !src_file && !src_path_empty {
                SideAct::Pin
            } else {
                SideAct::None
            }
        }
        RowKind::Files => {
            if src == RowKind::Tree && !src_group && src_file && !src_path_empty {
                SideAct::PinFile
            } else {
                SideAct::None
            }
        }
        RowKind::Tree => match src {
            RowKind::Tree => SideAct::Move,
            RowKind::Pins | RowKind::Files if !src_group && !src_path_empty => SideAct::Copy,
            RowKind::Pins | RowKind::Files => SideAct::None,
        },
    }
}

/// True when a Quick Access re-read is allowed. `last` is when the previous read finished.
pub(crate) fn pins_due(last: Option<Instant>, now: Instant, wait: Duration) -> bool {
    match last {
        None => true,
        Some(t) => now.saturating_duration_since(t) >= wait,
    }
}

fn home_header(label: &str, open: bool, kind: RowKind) -> FavRow {
    FavRow {
        id: Vec::new(),
        depth: 0,
        group: true,
        file: false,
        open,
        name: label.to_string(),
        path: String::new(),
        kind,
    }
}

fn home_leaf(i: usize, p: &Pinned, file: bool, kind: RowKind) -> FavRow {
    FavRow {
        id: vec![i],
        depth: 1,
        group: false,
        file,
        open: false,
        name: p.name.clone(),
        path: p.path.clone(),
        kind,
    }
}

/// Explorer pins, then favourite files when that list is not empty.
/// `force_children` lists the leaves of a collapsed section (a search still has to see them).
/// The header keeps the real open flag either way. The files group is omitted when it has nothing.
pub(crate) fn home_rows(
    pins_open: bool,
    pins: &[Pinned],
    pins_label: &str,
    files_open: bool,
    files: &[Pinned],
    files_label: &str,
    force_children: bool,
) -> Vec<FavRow> {
    let mut out = Vec::new();
    out.push(home_header(pins_label, pins_open, RowKind::Pins));
    if pins_open || force_children {
        for (i, p) in pins.iter().enumerate() {
            out.push(home_leaf(i, p, false, RowKind::Pins));
        }
    }
    if file_group_visible(files) {
        out.push(home_header(files_label, files_open, RowKind::Files));
        if files_open || force_children {
            for (i, p) in files.iter().enumerate() {
                out.push(home_leaf(i, p, true, RowKind::Files));
            }
        }
    }
    out
}

/// Pins, then favourite files when that list is not empty, then the favourites tree.
/// The files group is omitted entirely when there is nothing to show. Favourite ids are unchanged.
pub(crate) fn compose_side(
    tree: &[FavNode],
    pins_open: bool,
    pins: &[Pinned],
    pins_label: &str,
    files_open: bool,
    files: &[Pinned],
    files_label: &str,
) -> Vec<FavRow> {
    let mut out = home_rows(pins_open, pins, pins_label, files_open, files, files_label, false);
    out.extend(visible_rows(tree));
    out
}

/// Insert a copy of a folder at the drop position. Does nothing if that path is already a favourite.
pub(crate) fn copy_leaf(tree: &mut Vec<FavNode>, path: &str, name: &str, onto: &[usize], zone: u8, file: bool) -> Option<Vec<usize>> {
    let path = tidy_path(path);
    if path.is_empty() || onto.is_empty() || contains_path(tree, &path) {
        return None;
    }
    let into = zone == 1 && is_group_id(tree, onto);
    if zone == 1 && !into {
        return None;
    }
    let parent: Vec<usize> = if into { onto.to_vec() } else { onto[..onto.len() - 1].to_vec() };
    let index = if into {
        child_len(tree, onto)
    } else if zone == 0 {
        onto[onto.len() - 1]
    } else {
        onto[onto.len() - 1] + 1
    };
    insert_child(tree, &parent, index, FavNode::leaf(&path, name, file))?;
    let index = index.min(child_len(tree, &parent).saturating_sub(1));
    let mut id = parent;
    id.push(index);
    Some(id)
}

/// One filesystem item from an external drop. `file` is how it opens, not a request to copy it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Dropped {
    pub(crate) path: String,
    pub(crate) file: bool,
}

/// Where those items land. `At` is a tree id and a drop zone (0 before, 1 into a group, 2 after).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DropSpot {
    At(Vec<usize>, u8),
    End,
}

/// A visible row, enough to choose a spot without the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DropHit {
    pub(crate) tree: bool,
    pub(crate) group: bool,
    pub(crate) id: Vec<usize>,
}

/// `DROPEFFECT_LINK`. A favourites drop never reports move: the source would delete the files.
pub(crate) const FAV_LINK: u32 = 4;

/// Link whenever the source allows any effect. Move is never returned.
pub(crate) fn fav_link_effect(allowed: u32) -> u32 {
    if allowed == 0 { 0 } else { FAV_LINK }
}

/// Backslashes, no trailing slash except a drive root. Compared as text; nothing is opened.
pub(crate) fn canon_drop_path(path: &str) -> String {
    tidy_path(&path.replace('/', "\\"))
}

fn same_drop_path(a: &str, b: &str) -> bool {
    let a = canon_drop_path(a);
    let b = canon_drop_path(b);
    !a.is_empty() && a.eq_ignore_ascii_case(&b)
}

fn tree_has_path(tree: &[FavNode], path: &str) -> bool {
    fn walk(nodes: &[FavNode], path: &str) -> bool {
        nodes.iter().any(|n| (!n.path.is_empty() && same_drop_path(&n.path, path)) || (n.is_group() && walk(&n.children, path)))
    }
    walk(tree, path)
}

fn fresh_drops(tree: &[FavNode], items: &[Dropped]) -> Vec<FavNode> {
    let mut out = Vec::new();
    for it in items {
        let path = canon_drop_path(&it.path);
        if path.is_empty() || tree_has_path(tree, &path) || out.iter().any(|n: &FavNode| same_drop_path(&n.path, &path)) {
            continue;
        }
        out.push(FavNode::leaf(&path, "", it.file));
    }
    out
}

/// Parent id, index among that parent's children, and whether the drop goes inside a group.
fn spot_place(tree: &[FavNode], spot: &DropSpot) -> Option<(Vec<usize>, usize, bool)> {
    let DropSpot::At(onto, zone) = spot else {
        return Some((Vec::new(), tree.len(), false));
    };
    if onto.is_empty() {
        return Some((Vec::new(), tree.len(), false));
    }
    let mut zone = *zone;
    if zone == 1 && !is_group_id(tree, onto) {
        zone = 2;
    }
    let into = zone == 1;
    let parent: Vec<usize> = if into { onto.clone() } else { onto[..onto.len() - 1].to_vec() };
    if !parent.is_empty() && !is_group_id(tree, &parent) {
        return None;
    }
    let n = child_len(tree, &parent);
    if !into && onto[onto.len() - 1] >= n {
        return None;
    }
    let index = if into {
        child_len(tree, onto)
    } else if zone == 0 {
        onto[onto.len() - 1]
    } else {
        onto[onto.len() - 1] + 1
    };
    Some((parent, index, into))
}

/// Insert `items` at `spot`, in order. A path already in the tree, or repeated in `items`, is skipped.
/// Comparison is ASCII case-insensitive on the whole path. Nothing is sliced out of the string.
/// Returns the id of the last node actually inserted.
pub(crate) fn insert_dropped(tree: &mut Vec<FavNode>, items: &[Dropped], spot: &DropSpot) -> Option<Vec<usize>> {
    let nodes = fresh_drops(tree, items);
    if nodes.is_empty() {
        return None;
    }
    let (parent, mut index, into) = spot_place(tree, spot)?;
    let group = match spot {
        DropSpot::At(id, _) if into => id.clone(),
        _ => Vec::new(),
    };
    let mut last = None;
    for node in nodes {
        if insert_child(tree, &parent, index, node).is_none() {
            break;
        }
        let mut id = parent.clone();
        id.push(index);
        last = Some(id);
        index += 1;
    }
    if into && last.is_some() {
        set_open(tree, &group, true);
    }
    last
}

/// Spot under the pointer. `index` is the visible row, or none for the empty area under the list.
/// A row that is not in the favourites tree inserts at the start of that tree.
pub(crate) fn drop_spot(hits: &[DropHit], index: Option<usize>, zone: u8) -> DropSpot {
    if let Some(row) = index.and_then(|i| hits.get(i)) {
        if row.tree && !row.id.is_empty() {
            let zone = if zone == 1 && !row.group { 2 } else { zone };
            return DropSpot::At(row.id.clone(), zone);
        }
        if let Some(first) = hits.iter().find(|r| r.tree && !r.id.is_empty()) {
            return DropSpot::At(first.id.clone(), 0);
        }
        return DropSpot::End;
    }
    if let Some(last) = hits.iter().rev().find(|r| r.tree && !r.id.is_empty()) {
        return DropSpot::At(last.id.clone(), 2);
    }
    DropSpot::End
}

/// Visible row and zone for the insertion line. None when there is no row to draw under.
pub(crate) fn drop_line(hits: &[DropHit], spot: &DropSpot) -> Option<(usize, u8)> {
    match spot {
        DropSpot::End => hits.len().checked_sub(1).map(|i| (i, 2)),
        DropSpot::At(id, zone) => hits.iter().position(|r| r.tree && r.id == *id).map(|i| (i, *zone)),
    }
}

pub(crate) fn node_label(n: &FavNode) -> String {
    if !n.name.is_empty() {
        return n.name.clone();
    }
    if n.path.is_empty() {
        return String::new();
    }
    short_name(&n.path)
}

/// Leaves, depth-first, in the `path` or `path\tname` form the rest of the app stores.
pub(crate) fn flat_favs(tree: &[FavNode]) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(nodes: &[FavNode], out: &mut Vec<String>) {
        for n in nodes {
            if n.is_group() {
                walk(&n.children, out);
                continue;
            }
            let custom = !n.name.is_empty() && n.name != short_name(&n.path);
            if custom {
                out.push(format!("{}\t{}", n.path, n.name));
            } else {
                out.push(n.path.clone());
            }
        }
    }
    walk(tree, &mut out);
    out
}

/// Folder leaves, depth-first. Files stay out of address-bar jumps. A custom name is `path\tname`.
pub(crate) fn fav_folders(tree: &[FavNode]) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(nodes: &[FavNode], out: &mut Vec<String>) {
        for n in nodes {
            if n.is_group() {
                walk(&n.children, out);
                continue;
            }
            if n.file || !crate::is_fs(&n.path) {
                continue;
            }
            let custom = !n.name.is_empty() && n.name != short_name(&n.path);
            if custom {
                out.push(format!("{}\t{}", n.path, n.name));
            } else {
                out.push(n.path.clone());
            }
        }
    }
    walk(tree, &mut out);
    out
}

pub(crate) fn tree_from_flat(favs: &[String]) -> Vec<FavNode> {
    favs.iter()
        .filter_map(|f| {
            let path = fav_path(f).trim();
            if path.is_empty() {
                return None;
            }
            let name = f.split_once('\t').map(|(_, n)| n.trim().to_string()).unwrap_or_default();
            Some(FavNode::leaf(path, &name, false))
        })
        .collect()
}

pub(crate) fn contains_path(tree: &[FavNode], path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    tree.iter().any(|n| {
        (!n.path.is_empty() && n.path.eq_ignore_ascii_case(path)) || (n.is_group() && contains_path(&n.children, path))
    })
}

pub(crate) fn add_item(tree: &mut Vec<FavNode>, path: &str, file: bool) -> bool {
    let path = path.trim();
    if path.is_empty() || contains_path(tree, path) {
        return false;
    }
    tree.push(FavNode::leaf(path, "", file));
    true
}

/// Drop every leaf with this path. A group that only held it stays.
pub(crate) fn remove_path(tree: &mut Vec<FavNode>, path: &str) -> bool {
    fn rec(nodes: &mut Vec<FavNode>, path: &str) -> bool {
        let before = nodes.len();
        nodes.retain(|n| n.path.is_empty() || !n.path.eq_ignore_ascii_case(path));
        let mut hit = nodes.len() != before;
        for n in nodes.iter_mut() {
            if n.is_group() && rec(&mut n.children, path) {
                hit = true;
            }
        }
        hit
    }
    !path.is_empty() && rec(tree, path)
}

pub(crate) fn add_group(tree: &mut Vec<FavNode>, parent: &[usize], name: &str) -> bool {
    let node = FavNode::group(name);
    insert_child(tree, parent, usize::MAX, node).is_some()
}

/// Point a leaf at a new folder. The display name and the node's place in the tree stay.
/// An empty name is frozen to the old folder's label so it does not follow the new path.
/// The new path is a folder, so a file leaf stops being a file. A path another leaf already
/// uses is refused.
pub(crate) fn retarget_leaf(tree: &mut [FavNode], id: &[usize], path: &str) -> bool {
    let path = tidy_path(path);
    if path.is_empty() || id.is_empty() {
        return false;
    }
    if other_has_path(tree, id, &path) {
        return false;
    }
    let Some(n) = node_mut(tree, id) else { return false };
    if n.is_group() {
        return false;
    }
    freeze_label(n);
    n.path = path;
    n.file = false;
    true
}

/// `retarget_leaf` found by path, for a row whose id is no longer in the tree.
pub(crate) fn retarget_path(tree: &mut [FavNode], old: &str, path: &str) -> bool {
    let path = tidy_path(path);
    let old = tidy_path(old);
    if path.is_empty() || old.is_empty() {
        return false;
    }
    // The leaf being updated already contains `old`. Another leaf owning `path` is a clash.
    if !old.eq_ignore_ascii_case(&path) && contains_path(tree, &path) {
        return false;
    }
    fn apply(nodes: &mut [FavNode], old: &str, path: &str) -> bool {
        for n in nodes.iter_mut() {
            if !n.path.is_empty() && n.path.eq_ignore_ascii_case(old) {
                freeze_label(n);
                n.path = path.to_string();
                n.file = false;
                return true;
            }
            if n.is_group() && apply(&mut n.children, old, path) {
                return true;
            }
        }
        false
    }
    apply(tree, &old, &path)
}

fn freeze_label(n: &mut FavNode) {
    if n.name.trim().is_empty() {
        let label = short_name(&n.path);
        if !label.is_empty() {
            n.name = label;
        }
    }
}

fn other_has_path(nodes: &[FavNode], id: &[usize], path: &str) -> bool {
    fn walk(nodes: &[FavNode], id: &[usize], path: &str, at: &mut Vec<usize>) -> bool {
        for (i, n) in nodes.iter().enumerate() {
            at.push(i);
            if !n.path.is_empty() && n.path.eq_ignore_ascii_case(path) && at.as_slice() != id {
                return true;
            }
            if n.is_group() && walk(&n.children, id, path, at) {
                return true;
            }
            at.pop();
        }
        false
    }
    walk(nodes, id, path, &mut Vec::new())
}

pub(crate) fn rename_at(tree: &mut Vec<FavNode>, id: &[usize], name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    let Some(n) = node_mut(tree, id) else { return false };
    n.name = name.chars().take(80).collect();
    true
}

/// What a rename started on: name, path, group. Ids shift while a prompt is open.
pub(crate) type NodeKey = (String, String, bool);

pub(crate) fn node_key(tree: &[FavNode], id: &[usize]) -> Option<NodeKey> {
    tree_rows(tree).into_iter().find(|r| r.id == id).map(|r| (r.name, r.path, r.group))
}

/// Where the node a rename started on is now: `id` if it still holds it, else the one other
/// node with the same key. None when it is gone or ambiguous.
pub(crate) fn locate(tree: &[FavNode], id: &[usize], key: &NodeKey) -> Option<Vec<usize>> {
    let rows = tree_rows(tree);
    let same = |r: &FavRow| r.name == key.0 && r.path == key.1 && r.group == key.2;
    if rows.iter().any(|r| r.id == id && same(r)) {
        return Some(id.to_vec());
    }
    let mut hits = rows.iter().filter(|r| same(r));
    let first = hits.next()?;
    hits.next().is_none().then(|| first.id.clone())
}

pub(crate) fn delete_at(tree: &mut Vec<FavNode>, id: &[usize]) -> bool {
    take_node(tree, id).is_some()
}

pub(crate) fn node_ref<'a>(tree: &'a [FavNode], id: &[usize]) -> Option<&'a FavNode> {
    let (&i, rest) = id.split_first()?;
    let n = tree.get(i)?;
    if rest.is_empty() {
        Some(n)
    } else {
        node_ref(&n.children, rest)
    }
}

/// Every server on this node and, when it is a group, inside it.
pub(crate) fn servers_in(n: &FavNode) -> Vec<ServerLink> {
    let mut out = Vec::new();
    fn walk(n: &FavNode, out: &mut Vec<ServerLink>) {
        if let Some(s) = &n.server {
            out.push(s.clone());
        }
        for c in &n.children {
            walk(c, out);
        }
    }
    walk(n, &mut out);
    out
}

pub(crate) fn server_by_path(tree: &[FavNode], path: &str) -> Option<ServerLink> {
    fn walk(nodes: &[FavNode], path: &str) -> Option<ServerLink> {
        for n in nodes {
            if !n.path.is_empty() && n.path.eq_ignore_ascii_case(path) {
                return n.server.clone();
            }
            if n.is_group() {
                if let Some(s) = walk(&n.children, path) {
                    return Some(s);
                }
            }
        }
        None
    }
    if path.is_empty() {
        None
    } else {
        walk(tree, path)
    }
}

/// True when another leaf already uses `path`. An empty `skip` checks the whole tree.
pub(crate) fn path_clash(tree: &[FavNode], skip: &[usize], path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if skip.is_empty() {
        contains_path(tree, path)
    } else {
        other_has_path(tree, skip, path)
    }
}

pub(crate) fn append_node(tree: &mut Vec<FavNode>, parent: &[usize], node: FavNode) -> bool {
    if node.path.is_empty() || contains_path(tree, &node.path) {
        return false;
    }
    insert_child(tree, parent, usize::MAX, node).is_some()
}

pub(crate) fn write_server(tree: &mut [FavNode], id: &[usize], name: String, path: String, link: ServerLink) -> bool {
    if path.is_empty() || id.is_empty() || path_clash(tree, id, &path) {
        return false;
    }
    let Some(n) = node_mut(tree, id) else { return false };
    if n.is_group() {
        return false;
    }
    n.name = name;
    n.path = path;
    n.file = false;
    n.server = Some(link);
    true
}

pub(crate) fn set_open(tree: &mut Vec<FavNode>, id: &[usize], open: bool) {
    if let Some(n) = node_mut(tree, id) {
        if n.is_group() {
            n.open = open;
        }
    }
}

/// Open every group, including groups nested inside a collapsed parent.
pub(crate) fn expand_all(tree: &mut [FavNode]) {
    fn walk(nodes: &mut [FavNode]) {
        for n in nodes.iter_mut() {
            if n.is_group() {
                n.open = true;
                walk(&mut n.children);
            }
        }
    }
    walk(tree);
}

pub(crate) fn toggle_open(tree: &mut Vec<FavNode>, id: &[usize]) -> bool {
    let Some(n) = node_mut(tree, id) else { return false };
    if !n.is_group() {
        return false;
    }
    n.open = !n.open;
    true
}

fn collect_rows(tree: &[FavNode], all: bool) -> Vec<FavRow> {
    let mut out = Vec::new();
    fn walk(nodes: &[FavNode], prefix: &mut Vec<usize>, depth: u8, all: bool, out: &mut Vec<FavRow>) {
        for (i, n) in nodes.iter().enumerate() {
            prefix.push(i);
            out.push(FavRow {
                id: prefix.clone(),
                depth,
                group: n.is_group(),
                file: n.file,
                open: n.open,
                name: node_label(n),
                path: n.path.clone(),
                kind: RowKind::Tree,
            });
            if n.is_group() && (all || n.open) {
                walk(&n.children, prefix, depth.saturating_add(1), all, out);
            }
            prefix.pop();
        }
    }
    walk(tree, &mut Vec::new(), 0, all, &mut out);
    out
}

/// Rows the left panel shows. Children of a collapsed group stay hidden.
pub(crate) fn visible_rows(tree: &[FavNode]) -> Vec<FavRow> {
    collect_rows(tree, false)
}

/// Every group and leaf, including children of a collapsed group.
/// Settings manages the stored tree, so a closed group still lists what is inside it.
pub(crate) fn tree_rows(tree: &[FavNode]) -> Vec<FavRow> {
    collect_rows(tree, true)
}

#[cfg(test)]
pub(crate) fn leaf_paths(tree: &[FavNode]) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    fn walk(nodes: &[FavNode], prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (i, n) in nodes.iter().enumerate() {
            prefix.push(i);
            if n.is_group() {
                walk(&n.children, prefix, out);
            } else {
                out.push(prefix.clone());
            }
            prefix.pop();
        }
    }
    walk(tree, &mut Vec::new(), &mut out);
    out
}

#[cfg(test)]
pub(crate) fn remove_leaf(tree: &mut Vec<FavNode>, index: usize) -> bool {
    let Some(id) = leaf_paths(tree).get(index).cloned() else { return false };
    delete_at(tree, &id)
}

#[cfg(test)]
pub(crate) fn rename_leaf(tree: &mut Vec<FavNode>, index: usize, name: &str) -> bool {
    let Some(id) = leaf_paths(tree).get(index).cloned() else { return false };
    rename_at(tree, &id, name)
}

/// Plain Alt+↑/↓. Anything with Ctrl or Shift stays a pane shortcut.
pub(crate) fn fav_step(vk: u16, ctrl: bool, alt: bool, shift: bool) -> Option<i32> {
    match vk {
        0x26 | 0x28 if alt && !ctrl && !shift => Some(if vk == 0x26 { -1 } else { 1 }),
        _ => None,
    }
}

/// Move `id` one place among its siblings. The first child moves out before its group;
/// the last child moves out after it. None at the ends of the root.
pub(crate) fn nudge_node(tree: &mut Vec<FavNode>, id: &[usize], down: bool) -> Option<Vec<usize>> {
    if id.is_empty() {
        return None;
    }
    let parent = &id[..id.len() - 1];
    let index = *id.last()?;
    let n = child_len(tree, parent);
    if index >= n {
        return None;
    }
    if down {
        if index + 1 < n {
            let mut onto = parent.to_vec();
            onto.push(index + 1);
            apply_drop(tree, id, &onto, 2)
        } else if !parent.is_empty() {
            apply_drop(tree, id, parent, 2)
        } else {
            None
        }
    } else if index > 0 {
        let mut onto = parent.to_vec();
        onto.push(index - 1);
        apply_drop(tree, id, &onto, 0)
    } else if !parent.is_empty() {
        apply_drop(tree, id, parent, 0)
    } else {
        None
    }
}

/// Same result as removing the flat leaf `from` and inserting it at `to`.
#[cfg(test)]
pub(crate) fn reorder_leaves(tree: &mut Vec<FavNode>, from: usize, to: usize) -> bool {
    if from == to {
        return false;
    }
    let paths = leaf_paths(tree);
    let (Some(src), Some(dst)) = (paths.get(from).cloned(), paths.get(to).cloned()) else { return false };
    let zone = if to < from { 0 } else { 2 };
    apply_drop(tree, &src, &dst, zone).is_some()
}

/// `zone`: 0 before `onto`, 1 into `onto` when it is a group, 2 after `onto`.
/// On success, the moved node's id in the tree afterwards.
pub(crate) fn apply_drop(tree: &mut Vec<FavNode>, from: &[usize], onto: &[usize], zone: u8) -> Option<Vec<usize>> {
    if from.is_empty() || onto.is_empty() || from == onto || onto.starts_with(from) {
        return None;
    }
    let into = zone == 1 && is_group_id(tree, onto);
    let parent: Vec<usize> = if into { onto.to_vec() } else { onto[..onto.len() - 1].to_vec() };
    let mut index = if into {
        child_len(tree, onto)
    } else if zone == 0 {
        onto[onto.len() - 1]
    } else {
        onto[onto.len() - 1] + 1
    };
    if from[..from.len() - 1] == parent[..] && from[from.len() - 1] < index {
        index -= 1;
    }
    // Work on a copy so a stale id cannot drop the taken node on the floor.
    let mut next = tree.clone();
    let node = take_node(&mut next, from)?;
    let parent = adjust(&parent, from).unwrap_or(parent);
    let index = index.min(child_len(&next, &parent));
    insert_child(&mut next, &parent, index, node)?;
    *tree = next;
    let mut id = parent;
    id.push(index);
    Some(id)
}

/// Top quarter / middle / bottom quarter of a row. A leaf has no middle.
pub(crate) fn drop_zone(group: bool, y: i32, rh: i32) -> u8 {
    if rh <= 0 {
        return 2;
    }
    if !group {
        return if y < rh / 2 { 0 } else { 2 };
    }
    if y < rh / 4 {
        0
    } else if y >= rh * 3 / 4 {
        2
    } else {
        1
    }
}

pub(crate) fn tree_to_values(tree: &[FavNode]) -> Vec<Value> {
    tree.iter().map(node_value).collect()
}

fn node_value(n: &FavNode) -> Value {
    let mut m = serde_json::Map::new();
    if !n.name.is_empty() {
        m.insert("name".into(), json!(n.name));
    }
    if !n.path.is_empty() {
        m.insert("path".into(), json!(n.path));
    }
    if n.file {
        m.insert("file".into(), json!(true));
    }
    if n.is_group() && !n.open {
        m.insert("open".into(), json!(false));
    }
    if !n.children.is_empty() {
        m.insert("children".into(), json!(tree_to_values(&n.children)));
    }
    if let Some(s) = &n.server {
        m.insert("server".into(), server_value(s));
    }
    Value::Object(m)
}

fn server_value(s: &ServerLink) -> Value {
    let mut m = serde_json::Map::new();
    m.insert("kind".into(), json!(s.kind));
    m.insert("host".into(), json!(s.host));
    if s.port != 0 {
        m.insert("port".into(), json!(s.port));
    }
    if !s.remote.is_empty() {
        m.insert("remote".into(), json!(s.remote));
    }
    if !s.user.is_empty() {
        m.insert("user".into(), json!(s.user));
    }
    if s.tls {
        m.insert("tls".into(), json!(true));
    }
    Value::Object(m)
}

fn server_from(v: &Value) -> Option<ServerLink> {
    let o = v.as_object()?;
    let kind = o.get("kind").and_then(|x| x.as_str()).unwrap_or("").trim().to_ascii_lowercase();
    if !matches!(kind.as_str(), "smb" | "webdav" | "sftp" | "ftp") {
        return None;
    }
    let host = o.get("host").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    if host.is_empty() {
        return None;
    }
    let port = o.get("port").and_then(|x| x.as_u64()).unwrap_or(0).min(65535) as u16;
    let remote = o.get("remote").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let user = o.get("user").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let tls = kind == "webdav" && o.get("tls").and_then(|x| x.as_bool()).unwrap_or(false);
    Some(ServerLink { kind, host, port, remote, user, tls })
}

pub(crate) fn tree_from_values(v: &[Value]) -> Vec<FavNode> {
    v.iter().filter_map(node_from_value).collect()
}

fn node_from_value(v: &Value) -> Option<FavNode> {
    let o = v.as_object()?;
    let name = o.get("name").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let path = o.get("path").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    let file = o.get("file").and_then(|x| x.as_bool()).unwrap_or(false);
    let children = o.get("children").and_then(|x| x.as_array()).map(|a| tree_from_values(a)).unwrap_or_default();
    if path.is_empty() && name.is_empty() && children.is_empty() {
        return None;
    }
    let open = if path.is_empty() { o.get("open").and_then(|x| x.as_bool()).unwrap_or(true) } else { false };
    // A password key in an old or hand-edited file is ignored and not written back.
    let server = o.get("server").and_then(server_from);
    Some(FavNode { name, path: path.clone(), file: file && !path.is_empty(), open, children, server })
}

impl Session {
    pub(crate) fn ensure_fav_tree(&mut self) {
        if self.tree.is_empty() && !self.favs.is_empty() {
            self.tree = tree_from_flat(&self.favs);
        }
    }

    pub(crate) fn sync_favs(&mut self) {
        self.favs = flat_favs(&self.tree);
    }
}

fn node_mut<'a>(tree: &'a mut [FavNode], id: &[usize]) -> Option<&'a mut FavNode> {
    let (&i, rest) = id.split_first()?;
    let n = tree.get_mut(i)?;
    if rest.is_empty() {
        Some(n)
    } else {
        node_mut(&mut n.children, rest)
    }
}

fn is_group_id(tree: &[FavNode], id: &[usize]) -> bool {
    let (&i, rest) = match id.split_first() {
        Some(x) => x,
        None => return false,
    };
    let n = match tree.get(i) {
        Some(n) => n,
        None => return false,
    };
    if rest.is_empty() {
        n.is_group()
    } else {
        is_group_id(&n.children, rest)
    }
}

fn child_len(tree: &[FavNode], parent: &[usize]) -> usize {
    if parent.is_empty() {
        return tree.len();
    }
    let (&i, rest) = match parent.split_first() {
        Some(x) => x,
        None => return 0,
    };
    match tree.get(i) {
        Some(n) if rest.is_empty() => n.children.len(),
        Some(n) => child_len(&n.children, rest),
        None => 0,
    }
}

fn insert_child(tree: &mut Vec<FavNode>, parent: &[usize], index: usize, node: FavNode) -> Option<()> {
    fn rec(nodes: &mut Vec<FavNode>, parent: &[usize], index: usize, node: FavNode) -> Option<()> {
        if parent.is_empty() {
            let index = index.min(nodes.len());
            nodes.insert(index, node);
            return Some(());
        }
        let (&i, rest) = parent.split_first()?;
        let n = nodes.get_mut(i)?;
        if !n.is_group() {
            return None;
        }
        rec(&mut n.children, rest, index, node)
    }
    rec(tree, parent, index, node)
}

fn take_node(tree: &mut Vec<FavNode>, id: &[usize]) -> Option<FavNode> {
    let (&i, parent) = id.split_last()?;
    fn rec(nodes: &mut Vec<FavNode>, parent: &[usize], i: usize) -> Option<FavNode> {
        if parent.is_empty() {
            return (i < nodes.len()).then(|| nodes.remove(i));
        }
        let (&p, rest) = parent.split_first()?;
        let n = nodes.get_mut(p)?;
        rec(&mut n.children, rest, i)
    }
    rec(tree, parent, i)
}

/// Visible row that an "after" marker belongs under.
/// An open group's next sibling sits below its children, so the line follows the last one.
pub(crate) fn after_row(rows: &[FavRow], i: usize) -> usize {
    let Some(row) = rows.get(i) else { return i };
    if !(row.group && row.open) {
        return i;
    }
    rows.iter().enumerate().skip(i + 1).find(|(_, r)| r.depth <= row.depth).map(|(j, _)| j - 1).unwrap_or(rows.len().saturating_sub(1))
}

/// Open a group that just accepted a drop. `onto` is its id from before the move.
pub(crate) fn reveal_drop(tree: &mut Vec<FavNode>, from: &[usize], onto: &[usize], zone: u8) {
    if zone != 1 {
        return;
    }
    let Some(id) = adjust(onto, from) else { return };
    set_open(tree, &id, true);
}

/// Path of a node that was not inside `removed`, after `removed` is gone.
fn adjust(path: &[usize], removed: &[usize]) -> Option<Vec<usize>> {
    if !removed.is_empty() && path.starts_with(removed) {
        return None;
    }
    let mut out = path.to_vec();
    let n = removed.len().saturating_sub(1);
    if !removed.is_empty() && path.len() > n && removed[..n] == path[..n] && removed[n] < path[n] {
        out[n] -= 1;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work() -> Vec<FavNode> {
        tree_from_flat(&["C:\\WORK\tWork".into(), "D:\\Photos".into()])
    }

    #[test]
    fn nudge_swaps_siblings_and_steps_out_of_a_group() {
        let mut tree = work();
        assert!(nudge_node(&mut tree, &[0], false).is_none());
        assert_eq!(tree[0].name, "Work");
        assert_eq!(nudge_node(&mut tree, &[0], true).unwrap(), vec![1]);
        assert_eq!(tree[0].path, r"D:\Photos");
        assert_eq!(tree[1].name, "Work");
        assert!(nudge_node(&mut tree, &[1], true).is_none());
        assert_eq!((fav_step(0x26, false, true, false), fav_step(0x28, false, true, false)), (Some(-1), Some(1)));
        for (c, a, s) in [(true, true, false), (false, true, true), (true, true, true), (false, false, false)] {
            assert_eq!((fav_step(0x26, c, a, s), fav_step(0x28, c, a, s)), (None, None));
        }
        assert_eq!(fav_step(0x25, false, true, false), None);

        assert!(add_group(&mut tree, &[], "Jobs"));
        let jobs = tree.iter().position(|n| n.is_group()).unwrap();
        tree[jobs].children.push(FavNode::leaf(r"C:\a", "a", false));
        tree[jobs].children.push(FavNode::leaf(r"C:\b", "b", false));
        assert_eq!(nudge_node(&mut tree, &[jobs, 0], true).unwrap(), vec![jobs, 1]);
        assert_eq!(tree[jobs].children[0].name, "b");
        assert_eq!(tree[jobs].children[1].name, "a");
        // The first child steps out to just before the group.
        assert_eq!(nudge_node(&mut tree, &[jobs, 0], false).unwrap(), vec![jobs]);
        assert_eq!(tree[jobs].name, "b");
        assert!(tree[jobs + 1].is_group());
        assert_eq!(tree[jobs + 1].children[0].name, "a");
    }

    #[test]
    fn folder_leaves_skip_files() {
        let mut tree = work();
        tree.push(FavNode::leaf(r"C:\notes.txt", "notes", true));
        tree.push(FavNode::group("g"));
        tree.last_mut().unwrap().children.push(FavNode::leaf(r"E:\Docs", "보고서", false));
        let folders = fav_folders(&tree);
        assert_eq!(folders, ["C:\\WORK\tWork".to_string(), r"D:\Photos".into(), "E:\\Docs\t보고서".into()]);
    }

    #[test]
    fn flat_list_migrates_into_the_root_and_back() {
        let tree = work();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].name, "Work");
        assert!(!tree[0].file);
        assert_eq!(flat_favs(&tree), vec!["C:\\WORK\tWork".to_string(), "D:\\Photos".to_string()]);
        assert!(contains_path(&tree, "c:\\work"));
        assert!(!contains_path(&tree, "C:\\WORK\\x"));
    }

    #[test]
    fn files_groups_and_drops_round_trip() {
        let mut tree = work();
        assert!(add_group(&mut tree, &[], "Jobs"));
        let jobs = visible_rows(&tree).iter().find(|r| r.group).unwrap().id.clone();
        assert!(add_item(&mut tree, r"C:\a\note.txt", true));
        let file = leaf_paths(&tree).last().unwrap().clone();
        assert!(apply_drop(&mut tree, &file, &jobs, 1).is_some());
        assert!(contains_path(&tree, r"C:\a\note.txt"));
        let rows = visible_rows(&tree);
        let nested = rows.iter().find(|r| r.file).unwrap();
        assert_eq!(nested.depth, 1);
        assert_eq!(nested.path, r"C:\a\note.txt");
        let values = tree_to_values(&tree);
        let back = tree_from_values(&values);
        assert_eq!(back, tree);
        assert!(back.iter().any(|n| n.is_group() && n.children.iter().any(|c| c.file)));
        assert!(apply_drop(&mut tree, &jobs, &jobs, 1).is_none());
        let child = visible_rows(&tree).iter().find(|r| r.file).unwrap().id.clone();
        assert!(apply_drop(&mut tree, &jobs, &child, 1).is_none());
    }

    #[test]
    fn reorder_matches_a_flat_remove_and_insert() {
        let mut tree = tree_from_flat(&["A".into(), "B".into(), "C".into(), "D".into()]);
        assert!(reorder_leaves(&mut tree, 0, 2));
        assert_eq!(flat_favs(&tree), vec!["B".to_string(), "C".to_string(), "A".to_string(), "D".to_string()]);
        assert!(reorder_leaves(&mut tree, 2, 0));
        assert_eq!(flat_favs(&tree), vec!["A".to_string(), "B".to_string(), "C".to_string(), "D".to_string()]);
        assert!(rename_leaf(&mut tree, 1, "Bee"));
        assert_eq!(flat_favs(&tree)[1], "B\tBee");
        assert!(remove_leaf(&mut tree, 1));
        assert_eq!(flat_favs(&tree), vec!["A".to_string(), "C".to_string(), "D".to_string()]);
    }

    #[test]
    fn collapsed_group_hides_its_children_and_junk_json_is_skipped() {
        let mut tree = Vec::new();
        add_group(&mut tree, &[], "Box");
        add_item(&mut tree, r"C:\f", true);
        let file = leaf_paths(&tree).pop().unwrap();
        apply_drop(&mut tree, &file, &[0], 1);
        assert_eq!(visible_rows(&tree).len(), 2);
        assert_eq!(tree_rows(&tree).len(), 2);
        toggle_open(&mut tree, &[0]);
        assert_eq!(visible_rows(&tree).len(), 1);
        expand_all(&mut tree);
        assert!(tree[0].open);
        assert_eq!(visible_rows(&tree).len(), 2);
        let stored = tree_rows(&tree);
        assert_eq!(stored.len(), 2, "expanding a group keeps the child that was hidden while it was closed");
        assert_eq!(stored[1].depth, 1);
        assert!(stored[1].file);
        assert_eq!(drop_zone(true, 5, 28), 0);
        assert_eq!(drop_zone(true, 14, 28), 1);
        assert_eq!(drop_zone(false, 2, 28), 0);
        assert_eq!(drop_zone(true, 22, 28), 2);
        assert_eq!(drop_zone(false, 20, 28), 2);
        let junk = vec![json!("group"), json!({"name": "Box", "children": [{"path": "C:\\f", "file": true}]})];
        let parsed = tree_from_values(&junk);
        assert_eq!(parsed.len(), 1);
        assert!(parsed[0].children[0].file);
    }

    #[test]
    fn old_leaf_loads_and_a_server_password_is_never_stored() {
        let old = tree_from_values(&[json!({"name": "Desk", "path": r"C:\WORK\pgtest\data"})]);
        assert_eq!(old.len(), 1);
        assert!(old[0].server.is_none());
        assert!(!node_value(&old[0]).to_string().contains("server"));
        let leaked = json!({
            "name": "files",
            "path": "sftp://ada@localhost/data",
            "server": {"kind": "sftp", "host": "localhost", "user": "ada", "remote": "/data", "password": "s3cret-pl4in"}
        });
        let node = node_from_value(&leaked).unwrap();
        let link = node.server.clone().unwrap();
        assert_eq!(link.kind, "sftp");
        assert_eq!(link.user, "ada");
        assert!(!link.tls);
        let stored = node_value(&node).to_string();
        assert!(stored.contains("\"server\""));
        assert!(!stored.contains("password"));
        assert!(!stored.contains("s3cret-pl4in"));
        let again = node_from_value(&node_value(&node)).unwrap();
        assert_eq!(again.server, node.server);
    }

    fn jobs_and_desk() -> Vec<FavNode> {
        vec![
            FavNode {
                name: "Jobs".into(),
                path: String::new(),
                file: false,
                open: true,
                children: vec![FavNode::leaf(r"C:\a", "a", false), FavNode::leaf(r"C:\b", "b", false)],
                server: None,
            },
            FavNode {
                name: "Desk".into(),
                path: String::new(),
                file: false,
                open: true,
                children: vec![FavNode::leaf(r"C:\c", "c", false), FavNode::leaf(r"C:\d", "d", true)],
                server: None,
            },
        ]
    }

    fn labels(tree: &[FavNode]) -> String {
        visible_rows(tree).into_iter().map(|r| r.name).collect::<Vec<_>>().join("/")
    }

    #[test]
    fn reorders_groups_leaves_and_moves_a_leaf_between_groups() {
        let mut tree = jobs_and_desk();
        assert_eq!(apply_drop(&mut tree, &[0], &[1], 2), Some(vec![1]), "group moves after the other group");
        assert_eq!(labels(&tree), "Desk/c/d/Jobs/a/b");
        assert!(apply_drop(&mut tree, &[1], &[0], 0).is_some(), "group moves back before the other group");
        assert_eq!(labels(&tree), "Jobs/a/b/Desk/c/d");

        assert!(apply_drop(&mut tree, &[0, 0], &[0, 1], 2).is_some(), "leaf moves after its sibling");
        assert_eq!(labels(&tree), "Jobs/b/a/Desk/c/d");
        assert!(apply_drop(&mut tree, &[0, 1], &[0, 0], 0).is_some(), "leaf moves back before its sibling");
        assert_eq!(labels(&tree), "Jobs/a/b/Desk/c/d");

        assert!(apply_drop(&mut tree, &[0, 0], &[1], 1).is_some(), "leaf appends into the other group");
        assert_eq!(labels(&tree), "Jobs/b/Desk/c/d/a");
        assert_eq!(tree[1].children.last().map(|n| n.name.as_str()), Some("a"));
        assert!(apply_drop(&mut tree, &[1, 2], &[0, 0], 0).is_some(), "leaf moves to the front of the first group");
        assert_eq!(labels(&tree), "Jobs/a/b/Desk/c/d");
        assert!(apply_drop(&mut tree, &[0, 1], &[1, 0], 2).is_some(), "leaf lands between the other group's leaves");
        assert_eq!(labels(&tree), "Jobs/a/Desk/c/b/d");
        assert_eq!(tree[1].children.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(), ["c", "b", "d"]);
    }

    #[test]
    fn after_line_follows_an_open_group_and_a_drop_opens_it() {
        let tree = jobs_and_desk();
        let rows = visible_rows(&tree);
        assert_eq!(after_row(&rows, 0), 2, "open group: line sits under its last child");
        assert_eq!(after_row(&rows, 1), 1, "leaf: line stays on that row");
        assert_eq!(after_row(&rows, 3), 5);
        let mut closed = tree.clone();
        closed[0].open = false;
        let rows = visible_rows(&closed);
        assert_eq!(after_row(&rows, 0), 0, "collapsed group: the next row is already the sibling");

        closed[1].open = false;
        let jobs = vec![0usize];
        let desk = vec![1usize];
        assert!(apply_drop(&mut closed, &jobs, &desk, 1).is_some());
        reveal_drop(&mut closed, &jobs, &desk, 1);
        assert_eq!(closed[0].name, "Desk");
        assert!(closed[0].open, "the group that received the drop is expanded");
        assert!(closed[0].children.iter().any(|n| n.name == "Jobs"));
    }

    fn cand(path: &str, name: &str, pinned: bool, folder: bool) -> PinCand {
        PinCand { path: path.into(), name: name.into(), pinned, folder }
    }

    #[test]
    fn pinned_folders_keep_shell_order_and_drop_files() {
        let pins = pinned_folders(&[
            cand(r"D:\z", "Zed", true, true),
            cand(r"C:\file.txt", "note", true, false),
            cand(r"C:\a", "A", false, true),
            cand("", "This PC", true, true),
            cand(r"  ", "blank", true, true),
            cand(r"d:\z\", "again", true, true),
            cand(r"E:\c", "", true, true),
            cand(r"C:\", "Drive", true, false),
        ]);
        assert_eq!(pins.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), [r"D:\z", r"E:\c"]);
        assert_eq!(pins[0].name, "Zed");
        assert_eq!(pins[1].name, "c", "an empty shell name falls back to the folder name");
        assert_eq!(tidy_path(r"C:\"), r"C:\");
        assert_eq!(tidy_path("C:"), r"C:\");
        assert!(pin_drop_paths(&[(r"C:\a".into(), true), (r"c:\a\".into(), true)]).unwrap() == vec![r"C:\a".to_string()]);
        assert!(pin_drop_paths(&[(r"C:\a".into(), true), (r"C:\b.txt".into(), false)]).is_none());
        assert!(pin_drop_paths(&[(r"C:\b.txt".into(), false)]).is_none());
        assert!(pin_drop_paths(&[(String::new(), true)]).is_none());
        assert!(pin_drop_paths(&[]).is_none());
    }

    #[test]
    fn explorer_pins_lead_the_list_and_are_not_tree_nodes() {
        let tree = jobs_and_desk();
        let pins = pinned_folders(&[cand(r"D:\z", "Zed", true, true), cand(r"E:\c", "Cee", true, true)]);
        let rows = compose_side(&tree, true, &pins, "Explorer pins", true, &[], "Explorer favourite files");
        assert!(rows[0].kind == RowKind::Pins && rows[0].group && rows[0].open && rows[0].id.is_empty());
        assert_eq!(rows[0].name, "Explorer pins");
        assert_eq!((rows[1].path.as_str(), rows[1].depth, rows[1].file, rows[1].kind), (r"D:\z", 1, false, RowKind::Pins));
        assert_eq!(rows[2].path, r"E:\c");
        let jobs = rows.iter().find(|r| r.name == "Jobs").unwrap();
        assert!(jobs.in_tree() && jobs.id == vec![0] && jobs.depth == 0);
        let closed = compose_side(&tree, false, &pins, "Explorer pins", true, &[], "Explorer favourite files");
        assert!(closed.iter().filter(|r| r.kind == RowKind::Pins).all(|r| r.group));
        assert_eq!(closed[1].name, "Jobs");
        assert_eq!(labels(&tree), "Jobs/a/b/Desk/c/d");
    }

    #[test]
    fn pin_rows_do_not_reorder_and_a_folder_drop_pins() {
        assert_eq!(side_act(RowKind::Pins, false, false, false, RowKind::Pins), SideAct::None, "a pin cannot be reordered inside the group");
        assert_eq!(side_act(RowKind::Pins, true, false, true, RowKind::Tree), SideAct::None, "the pins header is not dragged into favourites");
        assert_eq!(side_act(RowKind::Tree, false, true, false, RowKind::Pins), SideAct::None, "a file dropped on the pins group is refused");
        assert_eq!(side_act(RowKind::Tree, true, false, true, RowKind::Pins), SideAct::None, "a group is not a folder to pin");
        assert_eq!(side_act(RowKind::Tree, false, false, false, RowKind::Pins), SideAct::Pin);
        assert_eq!(side_act(RowKind::Pins, false, false, false, RowKind::Tree), SideAct::Copy);
        assert_eq!(side_act(RowKind::Tree, false, false, false, RowKind::Tree), SideAct::Move);
        assert_eq!(side_act(RowKind::Tree, false, true, false, RowKind::Tree), SideAct::Move);

        let now = Instant::now();
        let wait = Duration::from_secs(2);
        assert!(pins_due(None, now, wait));
        assert!(!pins_due(Some(now), now, wait));
        assert!(!pins_due(Some(now.checked_sub(Duration::from_millis(1999)).unwrap()), now, wait));
        assert!(pins_due(Some(now.checked_sub(Duration::from_secs(2)).unwrap()), now, wait));
    }

    #[test]
    fn copying_a_pin_into_favourites_leaves_the_original() {
        let mut tree = jobs_and_desk();
        let before = tree.clone();
        let id = copy_leaf(&mut tree, r"E:\pin\", "Pin", &[1], 1, false).unwrap();
        assert_eq!(id, vec![1, 2]);
        assert_eq!(tree[1].children.last().map(|n| n.path.as_str()), Some(r"E:\pin"));
        assert!(!tree[1].children.last().unwrap().file);
        assert_eq!(tree[0], before[0], "the other group is untouched");
        assert!(copy_leaf(&mut tree, r"e:\pin", "Pin", &[0], 1, false).is_none(), "the same folder is not added twice");
        assert!(copy_leaf(&mut tree, r"E:\other", "O", &[1, 0], 1, false).is_none(), "a leaf has no inside");
        let id = copy_leaf(&mut tree, r"E:\other", "", &[0, 0], 0, false).unwrap();
        assert_eq!(id, vec![0, 0]);
        assert_eq!(tree[0].children[0].path, r"E:\other");
        assert_eq!(tree[0].children[1].name, "a");
        let id = copy_leaf(&mut tree, r"C:\note.txt", "note", &[1], 1, true).unwrap();
        assert_eq!(id, vec![1, 3]);
        assert!(tree[1].children.iter().any(|n| n.file && n.path == r"C:\note.txt"));
    }

    #[test]
    fn favourite_files_split_from_the_same_list_and_hide_when_empty() {
        let items = [
            cand(r"D:\z", "Zed", true, true),
            cand(r"C:\file.txt", "note", true, false),
            cand(r"C:\a", "A", false, true),
            cand(r"C:\skip.txt", "Skip", false, false),
            cand("", "This PC", true, false),
            cand(r"  ", "blank", true, false),
            cand(r"c:\file.txt\", "again", true, false),
            cand(r"E:\c", "", true, true),
            cand(r"C:\", "Drive", true, false),
            cand(r"C:\docs\a.pdf", "", true, false),
        ];
        let split = split_pins(&items);
        assert_eq!(split.folders.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), [r"D:\z", r"E:\c"]);
        assert_eq!(split.files.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(), [r"C:\file.txt", r"C:\docs\a.pdf"]);
        assert_eq!(split.files[0].name, "note");
        assert_eq!(split.files[1].name, "a.pdf", "an empty shell name falls back to the file name");
        assert!(file_group_visible(&split.files));
        assert!(!file_group_visible(&[]));

        assert_eq!(file_drop_paths(&[(r"C:\a.txt".into(), false), (r"c:\a.txt\".into(), false)]).unwrap(), vec![r"C:\a.txt".to_string()]);
        assert!(file_drop_paths(&[(r"C:\a.txt".into(), false), (r"C:\b".into(), true)]).is_none(), "a folder refuses the whole drop");
        assert!(file_drop_paths(&[(r"D:\box".into(), true)]).is_none());
        assert!(file_drop_paths(&[(r"C:\".into(), false)]).is_none(), "a drive root is not a file");
        assert!(file_drop_paths(&[(String::new(), false)]).is_none());
        assert!(file_drop_paths(&[]).is_none());

        let tree = jobs_and_desk();
        let files_label = "Explorer favourite files";
        let open = compose_side(&tree, true, &split.folders, "Explorer pins", true, &split.files, files_label);
        assert_eq!(open[0].name, "Explorer pins");
        assert_eq!(open[1].path, r"D:\z");
        assert_eq!(open[2].path, r"E:\c");
        assert!(open[3].kind == RowKind::Files && open[3].group && open[3].open && open[3].id.is_empty());
        assert_eq!(open[3].name, files_label);
        assert_eq!((open[4].path.as_str(), open[4].file, open[4].kind, open[4].depth), (r"C:\file.txt", true, RowKind::Files, 1));
        assert_eq!(open[5].path, r"C:\docs\a.pdf");
        let jobs = open.iter().find(|r| r.name == "Jobs").unwrap();
        assert!(jobs.in_tree() && jobs.id == vec![0] && jobs.depth == 0, "favourite ids stay the tree's own");

        let closed = compose_side(&tree, false, &split.folders, "Explorer pins", false, &split.files, files_label);
        assert!(closed[0].kind == RowKind::Pins && !closed[0].open);
        assert!(closed[1].kind == RowKind::Files && closed[1].group && !closed[1].open);
        assert!(closed.iter().filter(|r| r.kind == RowKind::Files).all(|r| r.group));
        assert_eq!(closed[2].name, "Jobs");

        let hidden = compose_side(&tree, true, &split.folders, "Explorer pins", true, &[], files_label);
        assert!(hidden.iter().all(|r| r.kind != RowKind::Files));
        assert_eq!(hidden[1].path, r"D:\z");
        assert_eq!(hidden[3].name, "Jobs");

        assert_eq!(side_act(RowKind::Tree, false, true, false, RowKind::Files), SideAct::PinFile);
        assert_eq!(side_act(RowKind::Tree, false, false, false, RowKind::Files), SideAct::None, "a folder dropped on favourite files is refused");
        assert_eq!(side_act(RowKind::Tree, true, false, true, RowKind::Files), SideAct::None);
        assert_eq!(side_act(RowKind::Files, false, true, false, RowKind::Files), SideAct::None, "file rows do not reorder");
        assert_eq!(side_act(RowKind::Pins, false, false, false, RowKind::Files), SideAct::None);
        assert_eq!(side_act(RowKind::Files, true, false, true, RowKind::Tree), SideAct::None, "the files header is not dragged into favourites");
        assert_eq!(side_act(RowKind::Files, false, true, false, RowKind::Tree), SideAct::Copy);
        assert_eq!(side_act(RowKind::Files, false, true, false, RowKind::Pins), SideAct::None);
        assert_eq!(side_act(RowKind::Tree, false, true, true, RowKind::Files), SideAct::None, "a file with no path is refused");
    }

    #[test]
    fn stale_drop_keeps_the_node() {
        let mut t = vec![FavNode::leaf("C:\\a", "a", false), FavNode::leaf("C:\\b", "b", false)];
        let before = t.len();
        assert!(apply_drop(&mut t, &[0], &[7, 3], 1).is_none());
        assert_eq!(t.len(), before);
    }

    #[test]
    fn retarget_keeps_the_name_and_the_order() {
        let mut tree = vec![FavNode::group("Jobs")];
        tree[0].children.push(FavNode::leaf(r"C:\old\docs", "Reports", false));
        tree[0].children.push(FavNode::leaf(r"C:\old\pics", "", true));
        tree.push(FavNode::leaf(r"D:\keep", "Stay", false));
        assert!(retarget_leaf(&mut tree, &[0, 0], r"E:\new\docs\"));
        assert_eq!(tree[0].children[0].name, "Reports");
        assert_eq!(tree[0].children[0].path, r"E:\new\docs");
        assert!(!tree[0].children[0].file);
        assert_eq!(tree[0].children[1].path, r"C:\old\pics");
        assert_eq!(tree[1].path, r"D:\keep");
        assert_eq!(tree[1].name, "Stay");
        assert!(retarget_leaf(&mut tree, &[0, 1], r"E:\elsewhere\photos"));
        assert_eq!(tree[0].children[1].name, "pics");
        assert_eq!(tree[0].children[1].path, r"E:\elsewhere\photos");
        assert!(!tree[0].children[1].file);
        assert_eq!(tree[0].children.len(), 2);
        assert!(!retarget_leaf(&mut tree, &[1], r"E:\new\docs"));
        assert_eq!(tree[1].path, r"D:\keep");
        assert!(retarget_path(&mut tree, r"d:\keep\", r"F:\moved"));
        assert_eq!(tree[1].name, "Stay");
        assert_eq!(tree[1].path, r"F:\moved");
        assert_eq!(tree[0].children[0].path, r"E:\new\docs");
    }

    #[test]
    fn rename_follows_a_shifted_node() {
        let mut t = vec![FavNode::leaf("C:\\a", "a", false), FavNode::leaf("C:\\b", "b", false)];
        let key = node_key(&t, &[1]).unwrap();
        t.remove(0);
        assert_eq!(locate(&t, &[1], &key), Some(vec![0]));
        t.clear();
        assert_eq!(locate(&t, &[1], &key), None);
    }

    fn item(path: &str, file: bool) -> Dropped {
        Dropped { path: path.to_string(), file }
    }

    fn hit(tree: bool, group: bool, id: Vec<usize>) -> DropHit {
        DropHit { tree, group, id }
    }

    #[test]
    fn canon_paths_keep_whole_characters() {
        assert_eq!(canon_drop_path("C:/Work/"), r"C:\Work");
        assert_eq!(canon_drop_path(r"\\server\share\"), r"\\server\share");
        assert_eq!(canon_drop_path("//server/share/a/"), r"\\server\share\a");
        assert_eq!(canon_drop_path("C:"), r"C:\");
        assert_eq!(canon_drop_path("  "), "");
        assert_eq!(canon_drop_path(r"F:\CAC\サンプル\"), r"F:\CAC\サンプル");
        assert!(canon_drop_path(r"f:\cac\サンプル").eq_ignore_ascii_case(r"F:\CAC\サンプル"));
    }

    #[test]
    fn dropped_items_keep_order_and_skip_duplicates() {
        let mut tree = tree_from_flat(&[r"C:\Work".into(), r"D:\Photos".into(), r"E:\Keep".into()]);
        let before = tree[0].clone();
        let id = insert_dropped(
            &mut tree,
            &[
                item(r"c:\work\", false),
                item(r"C:\New", false),
                item(r"C:/new", true),
                item(r"\\server\share\a", true),
                item("  ", false),
                item(r"C:\仕事\メモ.txt", true),
                item(r"c:\仕事\メモ.txt", false),
                item(r"F:\CAC\サンプル", false),
            ],
            &DropSpot::At(vec![1], 0),
        )
        .unwrap();
        assert_eq!(id, vec![4]);
        assert_eq!(
            flat_favs(&tree),
            vec![
                r"C:\Work".to_string(),
                r"C:\New".into(),
                r"\\server\share\a".into(),
                r"C:\仕事\メモ.txt".into(),
                r"F:\CAC\サンプル".into(),
                r"D:\Photos".into(),
                r"E:\Keep".into(),
            ]
        );
        assert!(!tree[1].file, "the first copy of a path wins");
        assert!(tree[2].file);
        assert!(tree[3].file);
        assert_eq!(tree[0], before, "an existing favourite is not replaced");
        assert_eq!(insert_dropped(&mut tree, &[item(r"C:\NEW", false)], &DropSpot::End), None);
        assert_eq!(tree.len(), 7);
    }

    #[test]
    fn a_drop_into_a_group_appends_in_order_and_opens_it() {
        let mut tree = vec![FavNode::group("Jobs"), FavNode::leaf(r"D:\Z", "Z", false)];
        tree[0].open = false;
        tree[0].children.push(FavNode::leaf(r"C:\a", "a", false));
        let id = insert_dropped(&mut tree, &[item(r"C:\b.txt", true), item(r"C:\c", false)], &DropSpot::At(vec![0], 1)).unwrap();
        assert_eq!(id, vec![0, 2]);
        assert!(tree[0].open);
        assert_eq!(tree[0].children.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(), [r"C:\a", r"C:\b.txt", r"C:\c"]);
        assert!(tree[0].children[1].file);
        assert!(!tree[0].children[2].file);
        let id = insert_dropped(&mut tree, &[item(r"E:\after", false)], &DropSpot::At(vec![0], 2)).unwrap();
        assert_eq!(id, vec![1]);
        assert_eq!(tree[1].path, r"E:\after");
        assert_eq!(tree[2].path, r"D:\Z");
    }

    #[test]
    fn an_existing_favourite_row_still_reorders() {
        let mut tree = tree_from_flat(&[r"C:\A".into(), r"C:\B".into(), r"C:\C".into()]);
        let id = insert_dropped(&mut tree, &[item(r"c:\a", false), item(r"C:\D", true)], &DropSpot::At(vec![2], 2)).unwrap();
        assert_eq!(id, vec![3]);
        assert_eq!(flat_favs(&tree), vec![r"C:\A".to_string(), r"C:\B".into(), r"C:\C".into(), r"C:\D".into()]);
        assert!(!tree[0].file, "skipping a duplicate does not rewrite the row");
        assert_eq!(apply_drop(&mut tree, &[0], &[3], 2), Some(vec![3]), "dragging that row still moves it");
        assert_eq!(flat_favs(&tree), vec![r"C:\B".to_string(), r"C:\C".into(), r"C:\D".into(), r"C:\A".into()]);
        assert!(insert_dropped(&mut tree, &[item(r"C:\nope", false)], &DropSpot::At(vec![9], 0)).is_none());
        assert_eq!(tree.len(), 4);
    }

    #[test]
    fn link_effect_is_never_move_and_the_line_follows_the_spot() {
        assert_eq!(fav_link_effect(0), 0);
        assert_eq!(fav_link_effect(1), FAV_LINK, "copy-only still reports link");
        assert_eq!(fav_link_effect(2), FAV_LINK, "move-only must not be reported as move");
        assert_eq!(fav_link_effect(1 | 2), FAV_LINK);
        assert_eq!(fav_link_effect(1 | 2 | 4), FAV_LINK);
        assert_ne!(fav_link_effect(2), 2);

        let hits = vec![
            hit(false, true, vec![]),
            hit(false, false, vec![0]),
            hit(true, true, vec![0]),
            hit(true, false, vec![0, 0]),
            hit(true, false, vec![1]),
        ];
        assert_eq!(drop_spot(&hits, Some(4), 0), DropSpot::At(vec![1], 0));
        assert_eq!(drop_spot(&hits, Some(2), 1), DropSpot::At(vec![0], 1));
        assert_eq!(drop_spot(&hits, Some(3), 1), DropSpot::At(vec![0, 0], 2), "a leaf has no inside");
        assert_eq!(drop_spot(&hits, Some(0), 1), DropSpot::At(vec![0], 0), "a pin row inserts before the tree");
        assert_eq!(drop_spot(&hits, None, 2), DropSpot::At(vec![1], 2), "empty space is after the last favourite");
        assert_eq!(drop_line(&hits, &DropSpot::At(vec![0], 1)), Some((2, 1)));
        assert_eq!(drop_line(&hits, &DropSpot::End), Some((4, 2)));
        let pins = vec![hit(false, true, vec![])];
        assert_eq!(drop_spot(&pins, None, 2), DropSpot::End);
        assert_eq!(drop_line(&pins, &DropSpot::End), Some((0, 2)));
        assert_eq!(drop_line(&[], &DropSpot::End), None);
    }
}
