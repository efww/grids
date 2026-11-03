//! Which registry values make Maguro the default for filesystem folders, and
//! which arguments are not filesystem paths. Nothing here touches the registry.

/// Verb name under `Directory\shell` and `Drive\shell`. Not the Folder class.
pub(crate) const VERB: &str = "Maguro";
pub(crate) const DIRECTORY: &str = "Directory";
pub(crate) const DRIVE: &str = "Drive";
/// Previous `(Default)` of the shell key. Empty when that value was absent.
pub(crate) const PREV_DEFAULT: &str = "PrevDefault";
/// 1 when `PrevDefault` was a real value, 0 when the shell key had no default.
pub(crate) const PREV_SET: &str = "PrevDefaultSet";
/// 1 when this registration created `...\shell` (it was absent in HKCU).
pub(crate) const SHELL_CREATED: &str = "ShellCreated";
/// 1 when this registration created `Software\Classes\<class>` itself.
pub(crate) const CLASS_CREATED: &str = "ClassCreated";

const CLASSES: [&str; 2] = [DIRECTORY, DRIVE];

/// One class as it sits in HKCU before a change. Missing keys are absent, not errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClassState {
    pub(crate) class: &'static str,
    pub(crate) class_exists: bool,
    /// Subkeys of the class other than `shell`.
    pub(crate) class_other_subkeys: Vec<String>,
    /// Named values on the class key. The default value is not listed.
    pub(crate) class_values: Vec<String>,
    pub(crate) shell_exists: bool,
    /// `None` when the shell key has no default value.
    pub(crate) shell_default: Option<String>,
    /// Named values on the shell key.
    pub(crate) shell_values: Vec<String>,
    /// Subkeys of `shell` other than our verb.
    pub(crate) shell_other_subkeys: Vec<String>,
    pub(crate) verb_exists: bool,
    pub(crate) prev_default: Option<String>,
    pub(crate) prev_set: Option<u32>,
    pub(crate) shell_created: Option<u32>,
    pub(crate) class_created: Option<u32>,
    /// `(Default)` of `shell\Maguro\command`. `None` when that value is absent.
    pub(crate) command: Option<String>,
}

impl ClassState {
    pub(crate) fn absent(class: &'static str) -> Self {
        Self {
            class,
            class_exists: false,
            class_other_subkeys: Vec::new(),
            class_values: Vec::new(),
            shell_exists: false,
            shell_default: None,
            shell_values: Vec::new(),
            shell_other_subkeys: Vec::new(),
            verb_exists: false,
            prev_default: None,
            prev_set: None,
            shell_created: None,
            class_created: None,
            command: None,
        }
    }
}

/// A registry change. An empty `name` is the key's `(Default)` value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    SetSz { key: String, name: String, data: String },
    SetDword { key: String, name: String, data: u32 },
    DeleteValue { key: String, name: String },
    /// Our verb key and its `command` subkey.
    DeleteTree { key: String },
    /// A key with no subkeys. Used for parents we created, once they are empty.
    DeleteKey { key: String },
}

impl Op {
    #[cfg(test)]
    pub(crate) fn key(&self) -> &str {
        match self {
            Op::SetSz { key, .. } | Op::SetDword { key, .. } | Op::DeleteValue { key, .. } | Op::DeleteTree { key } | Op::DeleteKey { key } => key,
        }
    }
}

pub(crate) fn class_key(class: &str) -> String {
    format!(r"Software\Classes\{class}")
}

pub(crate) fn shell_key(class: &str) -> String {
    format!(r"Software\Classes\{class}\shell")
}

pub(crate) fn verb_key(class: &str) -> String {
    format!(r"Software\Classes\{class}\shell\{VERB}")
}

pub(crate) fn command_key(class: &str) -> String {
    format!(r"Software\Classes\{class}\shell\{VERB}\command")
}

/// Keys whose values this feature writes. Parent class keys are created only as
/// containers, and only when they were missing.
#[cfg(test)]
pub(crate) fn value_keys() -> [&'static str; 6] {
    [
        r"Software\Classes\Directory\shell",
        r"Software\Classes\Directory\shell\Maguro",
        r"Software\Classes\Directory\shell\Maguro\command",
        r"Software\Classes\Drive\shell",
        r"Software\Classes\Drive\shell\Maguro",
        r"Software\Classes\Drive\shell\Maguro\command",
    ]
}

fn is_our_verb(v: &str) -> bool {
    v.eq_ignore_ascii_case(VERB)
}

fn class_is_ours(s: &ClassState) -> bool {
    s.shell_default.as_deref().is_some_and(is_our_verb) && s.command.as_ref().is_some_and(|c| !c.trim().is_empty())
}

/// Both Directory and Drive open here. One of the two is not enough.
pub(crate) fn is_registered(states: &[ClassState]) -> bool {
    CLASSES.iter().all(|class| states.iter().find(|s| s.class == *class).is_some_and(class_is_ours))
}

/// `Drive` roots arrive as `C:\`. A quoted `%1` then ends in `\"`, which
/// CommandLineToArgvW reads as an escaped quote. The extra backslash makes
/// `"C:\\"` parse back to `C:\`. Directory paths have no trailing slash.
pub(crate) fn command_line(exe: &str, class: &str) -> String {
    let exe = exe.trim().trim_matches('"');
    if class == DRIVE {
        format!("\"{exe}\" \"%1\\\"")
    } else {
        format!("\"{exe}\" \"%1\"")
    }
}

pub(crate) fn exe_from_command(command: &str) -> Option<String> {
    let s = command.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix('"') {
        let end = rest.find('"')?;
        let exe = rest.get(..end)?;
        return (!exe.is_empty()).then(|| exe.to_string());
    }
    let exe = s.split_whitespace().next().unwrap_or("");
    (!exe.is_empty()).then(|| exe.to_string())
}

pub(crate) fn same_exe(a: &str, b: &str) -> bool {
    norm_exe(a) == norm_exe(b)
}

fn norm_exe(p: &str) -> String {
    let p = p.trim().trim_matches('"');
    let p = p.strip_prefix(r"\\?\").unwrap_or(p);
    p.replace('/', "\\").trim_end_matches(['\\', '/']).to_ascii_lowercase()
}

fn state_for<'a>(states: &'a [ClassState], class: &'static str) -> ClassState {
    states.iter().find(|s| s.class == class).cloned().unwrap_or_else(|| ClassState::absent(class))
}

/// What to remember so a later restore can put the shell default back.
fn previous_to_store(s: &ClassState) -> (String, u32, u32, u32) {
    let (text, set) = if s.verb_exists {
        match s.prev_set {
            Some(flag) => (s.prev_default.clone().unwrap_or_default(), u32::from(flag != 0)),
            None => match &s.shell_default {
                Some(v) if !is_our_verb(v) => (v.clone(), 1),
                _ => (String::new(), 0),
            },
        }
    } else {
        match &s.shell_default {
            Some(v) if !is_our_verb(v) => (v.clone(), 1),
            _ => (String::new(), 0),
        }
    };
    let shell_created = if s.verb_exists { s.shell_created.unwrap_or(0) } else { u32::from(!s.shell_exists) };
    let class_created = if s.verb_exists { s.class_created.unwrap_or(0) } else { u32::from(!s.class_exists) };
    (text, set, shell_created, class_created)
}

pub(crate) fn register_ops(exe: &str, states: &[ClassState]) -> Vec<Op> {
    let mut ops = Vec::new();
    for class in CLASSES {
        let s = state_for(states, class);
        let (prev, set, shell_created, class_created) = previous_to_store(&s);
        let verb = verb_key(class);
        ops.push(Op::SetSz { key: shell_key(class), name: String::new(), data: VERB.into() });
        ops.push(Op::SetSz { key: verb.clone(), name: String::new(), data: VERB.into() });
        ops.push(Op::SetSz { key: verb.clone(), name: PREV_DEFAULT.into(), data: prev });
        ops.push(Op::SetDword { key: verb.clone(), name: PREV_SET.into(), data: set });
        ops.push(Op::SetDword { key: verb.clone(), name: SHELL_CREATED.into(), data: shell_created });
        ops.push(Op::SetDword { key: verb, name: CLASS_CREATED.into(), data: class_created });
        ops.push(Op::SetSz { key: command_key(class), name: String::new(), data: command_line(exe, class) });
    }
    ops
}

pub(crate) fn restore_ops(states: &[ClassState]) -> Vec<Op> {
    let mut ops = Vec::new();
    for class in CLASSES {
        let s = state_for(states, class);
        let ours = s.verb_exists || s.shell_default.as_deref().is_some_and(is_our_verb);
        if !ours {
            continue;
        }
        let shell = shell_key(class);
        let mut removed_default = false;
        let mut restored_default = false;
        if s.shell_default.as_deref().is_some_and(is_our_verb) || s.shell_default.is_none() {
            match s.prev_set {
                Some(flag) if flag != 0 && s.verb_exists => {
                    let text = s.prev_default.clone().unwrap_or_default();
                    ops.push(Op::SetSz { key: shell.clone(), name: String::new(), data: text });
                    restored_default = true;
                }
                _ => {
                    if s.shell_default.as_deref().is_some_and(is_our_verb) {
                        ops.push(Op::DeleteValue { key: shell.clone(), name: String::new() });
                        removed_default = true;
                    }
                }
            }
        }
        if s.verb_exists {
            ops.push(Op::DeleteTree { key: verb_key(class) });
        }
        let default_left = restored_default || (s.shell_default.is_some() && !removed_default);
        let shell_empty = s.shell_other_subkeys.is_empty() && s.shell_values.is_empty() && !default_left;
        let drop_shell = s.shell_created == Some(1) && shell_empty;
        if drop_shell {
            ops.push(Op::DeleteKey { key: shell });
        }
        let class_empty = s.class_other_subkeys.is_empty() && s.class_values.is_empty();
        if drop_shell && s.class_created == Some(1) && class_empty {
            ops.push(Op::DeleteKey { key: class_key(class) });
        }
    }
    ops
}

/// Rewrite the command when the registered exe is not the one now running.
/// Leaves the verb, the saved previous default and every other key alone.
pub(crate) fn path_fix_ops(exe: &str, states: &[ClassState]) -> Vec<Op> {
    if exe.trim().is_empty() || !is_registered(states) {
        return Vec::new();
    }
    let mut ops = Vec::new();
    for class in CLASSES {
        let s = state_for(states, class);
        let Some(cmd) = s.command.as_deref() else { continue };
        let Some(cur) = exe_from_command(cmd) else { continue };
        if same_exe(&cur, exe) {
            continue;
        }
        ops.push(Op::SetSz { key: command_key(class), name: String::new(), data: command_line(exe, class) });
    }
    ops
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArgRoute {
    Pane,
    Explorer,
}

/// A shell namespace or CLSID is not a filesystem folder. Explorer keeps it.
pub(crate) fn is_virtual_shell(arg: &str) -> bool {
    let s = arg.trim().trim_matches('"').trim();
    if s.is_empty() {
        return false;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("shell:") || lower.starts_with("search-ms:") || lower.starts_with("knownfolder:") {
        return true;
    }
    if s.starts_with("::") || s.contains("::{") {
        return true;
    }
    looks_like_clsid(s)
}

pub(crate) fn route_arg(arg: &str) -> ArgRoute {
    if is_virtual_shell(arg) { ArgRoute::Explorer } else { ArgRoute::Pane }
}

pub(crate) fn split_launch(args: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut pane = Vec::new();
    let mut explorer = Vec::new();
    for arg in args {
        match route_arg(&arg) {
            ArgRoute::Explorer => explorer.push(arg),
            ArgRoute::Pane => pane.push(arg),
        }
    }
    (pane, explorer)
}

fn looks_like_clsid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 38 || b[0] != b'{' || b[37] != b'}' {
        return false;
    }
    let dashes = [9, 14, 19, 24];
    if dashes.iter().any(|&i| b[i] != b'-') {
        return false;
    }
    let parts = [1..9, 10..14, 15..19, 20..24, 25..37];
    parts.iter().all(|r| b[r.clone()].iter().copied().all(|c| c.is_ascii_hexdigit()))
}

/// Where an activation inside Maguro goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenWhere {
    /// Browse the current pane. The shell verb must not run.
    Pane,
    /// The hosted view, or the verb, runs. A file does this. So does Ctrl or a
    /// middle click on a folder: that launch is a new tab while we are registered.
    Shell,
}

/// What the file list knows about the focused item. Shortcut targets are reported
/// as the target (`folder` / `stream` / `browsable`), not as the `.lnk` file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenIn {
    /// Settings > General, "use Maguro as the default folder app".
    pub registered: bool,
    pub ctrl: bool,
    pub middle: bool,
    pub folder: bool,
    pub stream: bool,
    pub browsable: bool,
    /// `FILE_ATTRIBUTE_DIRECTORY` from the property cache or a local attribute read.
    /// Set for a real directory, a junction, and a directory symlink even when the
    /// shell bits above are missing or still wrong.
    pub directory: bool,
}

/// Double-click and Enter in the file list. With the setting off, the view
/// already navigates, so this stays `Shell` and the view is left alone.
pub(crate) fn list_open(a: OpenIn) -> OpenWhere {
    if a.registered && !a.ctrl && !a.middle && (navigable(a.folder, a.stream, a.browsable) || a.directory) {
        OpenWhere::Pane
    } else {
        OpenWhere::Shell
    }
}

/// One activated row, as far as the shell could say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActKind {
    /// A folder, library, zip-like container, junction, or directory symlink.
    Folder,
    /// A file. The verb may run.
    File,
    /// Attributes were not available (still enumerating, not cached, a network item).
    Unknown,
}

/// What the file list does with one activation. Index 0 is the item that was
/// activated (focused, or the row under the pointer). The rest are other selected rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListAct {
    /// The hosted view runs the default verb.
    Shell,
    /// Swallow the activate (S_OK). Blocks Explorer leak when the row is unknown/empty.
    Consume,
    /// Browse `index` in this pane. The verb must not run on any row.
    Browse(usize),
    /// Open the files. Do not let the verb run: a selected folder would become a new tab.
    FilesOnly,
}

/// Enter with several rows selected used to look only at the focused item. When that
/// item was not a proven folder, the view ran the verb on every selected folder.
pub(crate) fn list_activation(registered: bool, ctrl: bool, middle: bool, kinds: &[ActKind]) -> ListAct {
    // Ctrl / middle: let the view run the verb (new tab / handoff). Never fall through for an
    // unresolved folder activate — that is how rapid double-clicks open a real Explorer window.
    if ctrl || middle {
        return ListAct::Shell;
    }
    if kinds.is_empty() || kinds.iter().all(|k| *k == ActKind::Unknown) {
        return ListAct::Consume;
    }
    let any_folder = kinds.iter().any(|k| *k == ActKind::Folder);
    if !any_folder {
        return ListAct::Shell;
    }
    // Folders always browse in-pane, even when the folder-app verb is off (shown_on false).
    let _ = registered;
    if kinds[0] == ActKind::Folder {
        return ListAct::Browse(0);
    }
    if let Some(i) = kinds.iter().position(|k| *k == ActKind::Folder) {
        if kinds[0] == ActKind::File {
            return ListAct::FilesOnly;
        }
        return ListAct::Browse(i);
    }
    ListAct::Consume
}

/// A context-menu command whose verb string could not be read. The default item
/// (offset 0, or the menu's default) is Open. Any other empty verb is left alone.
pub(crate) fn menu_open_item(registered: bool, ctrl: bool, middle: bool, verb: &str, count: usize, file: bool, link: bool, default_item: bool) -> OpenWhere {
    let verb = if verb.trim().is_empty() && default_item { "open" } else { verb };
    menu_open(registered, ctrl, middle, verb, count, file, link)
}

/// A directory, drive, library, or zip-like browsable folder.
/// A file that is also a stream is not a container.
pub(crate) fn navigable(folder: bool, stream: bool, browsable: bool) -> bool {
    folder && (!stream || browsable)
}

/// Right-click Open, Explore, or our own verb, on one item.
/// `link` is a `.lnk`: the caller navigates only when the target is a folder,
/// and otherwise lets the verb open the file. New-window verbs stay `Shell`.
pub(crate) fn menu_open(registered: bool, ctrl: bool, middle: bool, verb: &str, count: usize, file: bool, link: bool) -> OpenWhere {
    if registered && !ctrl && !middle && count == 1 && open_verb(verb) && (!file || link) {
        OpenWhere::Pane
    } else {
        OpenWhere::Shell
    }
}

/// Verbs that open a folder in this browser. `opennew` still launches, so
/// "Open in new window" becomes a tab through the single-instance handoff.
pub(crate) fn open_verb(verb: &str) -> bool {
    matches!(verb.trim().to_ascii_lowercase().as_str(), "open" | "explore" | "maguro")
}

/// Folder tree: double-click and Enter browse this pane while we are registered.
/// Ctrl and the middle button stay with the tree, which runs the verb.
pub(crate) fn tree_activate(registered: bool, ctrl: bool, middle: bool, activating: bool) -> OpenWhere {
    if registered && activating && !ctrl && !middle { OpenWhere::Pane } else { OpenWhere::Shell }
}

/// `C:\a\folder.lnk`. The decision of whether its target is a folder is separate.
pub(crate) fn link_file(path: &str) -> bool {
    path.trim().rsplit(['\\', '/']).next().unwrap_or("").trim_matches('"').to_ascii_lowercase().ends_with(".lnk")
}

/// How long a Ctrl or middle-click "open in a new tab" stays in force.
/// A folder the view still launches inside this window is that tab.
/// After it, a launch whose parent is this process is the double-click that slipped through.
pub(crate) const NEW_TAB_INTENT_MS: u64 = 2_000;

/// What the running instance does with a second `maguro.exe`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Handoff {
    /// Explorer, Win+E, another app, or a Ctrl/middle-click from this process.
    NewTab,
    /// This process spawned the child (the shell view's default verb). Browse in place.
    InPlace,
}

/// `parent_pid` is the parent of the `maguro.exe` that forwarded the launch.
/// `self_pid` is the running instance. `intent_elapsed_ms` is how long ago a
/// Ctrl or middle-click asked for a new tab, or `None` when it never did.
/// `0` is not a process id: a parent that could not be read stays a new tab.
pub(crate) fn handoff_action(parent_pid: u32, self_pid: u32, intent_elapsed_ms: Option<u64>) -> Handoff {
    let intent = matches!(intent_elapsed_ms, Some(ms) if ms <= NEW_TAB_INTENT_MS);
    if parent_pid != 0 && parent_pid == self_pid && !intent {
        Handoff::InPlace
    } else {
        Handoff::NewTab
    }
}

/// First line of a handoff. `\u{1}` cannot appear in a Windows path, so a folder
/// named like this header is still an argument. The line is whole characters.
pub(crate) fn handoff_text(parent_pid: u32, args: &[String]) -> String {
    let mut s = format!("\u{1}pg-parent:{parent_pid}");
    for a in args {
        s.push('\n');
        s.push_str(a);
    }
    s
}

/// Inverse of `handoff_text`. A body from an older build has no header: the parent
/// is 0 and every non-empty line is an argument.
pub(crate) fn parse_handoff(text: &str) -> (u32, Vec<String>) {
    let mut parent = 0u32;
    let mut args = Vec::new();
    for line in text.split('\n') {
        if let Some(rest) = line.strip_prefix('\u{1}') {
            if let Some(n) = rest.strip_prefix("pg-parent:") {
                parent = n.parse().unwrap_or(0);
                continue;
            }
        }
        if !line.is_empty() {
            args.push(line.to_string());
        }
    }
    (parent, args)
}

/// Quote only when Explorer's parameter would split. `shell:` and `::{...}` stay bare.
pub(crate) fn explorer_arg(path: &str) -> String {
    let s = path.trim();
    if s.is_empty() {
        return String::new();
    }
    if s.chars().any(|c| c.is_whitespace()) {
        format!("\"{}\"", s.replace('"', ""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Vec<ClassState> {
        vec![ClassState::absent(DIRECTORY), ClassState::absent(DRIVE)]
    }

    fn ours(class: &'static str, prev: Option<&str>, shell_created: u32, class_created: u32, command: &str) -> ClassState {
        let mut s = ClassState::absent(class);
        s.class_exists = true;
        s.shell_exists = true;
        s.shell_default = Some(VERB.into());
        s.verb_exists = true;
        s.prev_default = Some(prev.unwrap_or("").into());
        s.prev_set = Some(u32::from(prev.is_some()));
        s.shell_created = Some(shell_created);
        s.class_created = Some(class_created);
        s.command = Some(command.into());
        s
    }

    fn assert_classes_only(ops: &[Op]) {
        for op in ops {
            let k = op.key();
            assert!(k.starts_with(r"Software\Classes\Directory") || k.starts_with(r"Software\Classes\Drive"), "{k}");
            assert!(!k.contains(r"\Folder"), "{k}");
            assert!(!k.to_ascii_lowercase().contains("clsid"), "{k}");
        }
    }

    fn sz<'a>(ops: &'a [Op], key: &str, name: &str) -> Option<&'a str> {
        ops.iter().find_map(|op| match op {
            Op::SetSz { key: k, name: n, data } if k == key && n == name => Some(data.as_str()),
            _ => None,
        })
    }

    fn dword(ops: &[Op], key: &str, name: &str) -> Option<u32> {
        ops.iter().find_map(|op| match op {
            Op::SetDword { key: k, name: n, data } if k == key && n == name => Some(*data),
            _ => None,
        })
    }

    #[test]
    fn register_writes_directory_and_drive_and_never_folder() {
        let exe = r"C:\Program Files\Maguro\maguro.exe";
        let ops = register_ops(exe, &fresh());
        assert_classes_only(&ops);
        let keys: Vec<&str> = ops.iter().map(Op::key).collect();
        for key in value_keys() {
            assert!(keys.contains(&key), "{key}");
        }
        assert!(ops.iter().all(|op| op.key() != r"Software\Classes\Directory" && op.key() != r"Software\Classes\Drive"));
        assert!(ops.iter().all(|op| !matches!(op, Op::DeleteKey { .. } | Op::DeleteTree { .. } | Op::DeleteValue { .. })));
        // A Folder row in the snapshot is ignored. The plan never grows a Folder key.
        let mut extra = fresh();
        extra.push(ClassState::absent("Folder"));
        assert_classes_only(&register_ops(exe, &extra));
    }

    #[test]
    fn commands_quote_the_exe_and_drives_keep_the_backslash_inside_the_quotes() {
        let exe = r"C:\Program Files\Maguro\maguro.exe";
        let dir = command_line(exe, DIRECTORY);
        let drive = command_line(exe, DRIVE);
        assert_eq!(dir, format!("\"{exe}\" \"%1\""));
        assert_eq!(drive, format!("\"{exe}\" \"%1\\\""));
        assert!(dir.ends_with("%1\""));
        assert!(drive.ends_with("%1\\\""));
        assert_ne!(dir, drive);
        let ops = register_ops(exe, &fresh());
        assert_eq!(sz(&ops, &command_key(DIRECTORY), ""), Some(dir.as_str()));
        assert_eq!(sz(&ops, &command_key(DRIVE), ""), Some(drive.as_str()));
    }

    #[test]
    fn register_remembers_an_existing_default_and_that_the_parents_were_already_there() {
        let mut dir = ClassState::absent(DIRECTORY);
        dir.class_exists = true;
        dir.shell_exists = true;
        dir.shell_default = Some("open".into());
        dir.shell_other_subkeys = vec!["open".into()];
        let mut drive = ClassState::absent(DRIVE);
        drive.class_exists = true;
        drive.class_other_subkeys = vec!["shellex".into()];
        let ops = register_ops(r"D:\maguro.exe", &[dir, drive]);
        assert_eq!(sz(&ops, &shell_key(DIRECTORY), ""), Some(VERB));
        assert_eq!(sz(&ops, &verb_key(DIRECTORY), ""), Some(VERB));
        assert_eq!(sz(&ops, &verb_key(DIRECTORY), PREV_DEFAULT), Some("open"));
        assert_eq!(dword(&ops, &verb_key(DIRECTORY), PREV_SET), Some(1));
        assert_eq!(dword(&ops, &verb_key(DIRECTORY), SHELL_CREATED), Some(0));
        assert_eq!(dword(&ops, &verb_key(DIRECTORY), CLASS_CREATED), Some(0));
        assert_eq!(sz(&ops, &verb_key(DRIVE), PREV_DEFAULT), Some(""));
        assert_eq!(dword(&ops, &verb_key(DRIVE), PREV_SET), Some(0));
        assert_eq!(dword(&ops, &verb_key(DRIVE), SHELL_CREATED), Some(1));
        assert_eq!(dword(&ops, &verb_key(DRIVE), CLASS_CREATED), Some(0));
    }

    #[test]
    fn a_fresh_hive_records_that_we_created_the_parents() {
        let ops = register_ops(r"C:\maguro.exe", &fresh());
        for class in [DIRECTORY, DRIVE] {
            assert_eq!(dword(&ops, &verb_key(class), PREV_SET), Some(0));
            assert_eq!(sz(&ops, &verb_key(class), PREV_DEFAULT), Some(""));
            assert_eq!(dword(&ops, &verb_key(class), SHELL_CREATED), Some(1));
            assert_eq!(dword(&ops, &verb_key(class), CLASS_CREATED), Some(1));
        }
    }

    #[test]
    fn registering_again_does_not_store_our_own_verb_as_the_previous_default() {
        let cmd = command_line(r"C:\maguro.exe", DIRECTORY);
        let states = vec![
            ours(DIRECTORY, Some("open"), 0, 0, &cmd),
            ours(DRIVE, None, 1, 1, &command_line(r"C:\maguro.exe", DRIVE)),
        ];
        let ops = register_ops(r"C:\moved\maguro.exe", &states);
        assert_eq!(sz(&ops, &verb_key(DIRECTORY), PREV_DEFAULT), Some("open"));
        assert_eq!(dword(&ops, &verb_key(DIRECTORY), PREV_SET), Some(1));
        assert_eq!(dword(&ops, &verb_key(DIRECTORY), SHELL_CREATED), Some(0));
        assert_eq!(sz(&ops, &verb_key(DRIVE), PREV_DEFAULT), Some(""));
        assert_eq!(dword(&ops, &verb_key(DRIVE), PREV_SET), Some(0));
        assert_eq!(dword(&ops, &verb_key(DRIVE), SHELL_CREATED), Some(1));
        assert_eq!(dword(&ops, &verb_key(DRIVE), CLASS_CREATED), Some(1));
    }

    #[test]
    fn restore_puts_the_previous_verb_back_and_deletes_only_our_key() {
        let states = vec![
            {
                let mut s = ours(DIRECTORY, Some("open"), 0, 0, &command_line(r"C:\maguro.exe", DIRECTORY));
                s.shell_other_subkeys = vec!["open".into()];
                s
            },
            {
                let mut s = ours(DRIVE, Some("open"), 0, 0, &command_line(r"C:\maguro.exe", DRIVE));
                s.shell_values = vec!["SomeOther".into()];
                s
            },
        ];
        let ops = restore_ops(&states);
        assert_classes_only(&ops);
        assert_eq!(sz(&ops, &shell_key(DIRECTORY), ""), Some("open"));
        assert_eq!(sz(&ops, &shell_key(DRIVE), ""), Some("open"));
        assert!(ops.iter().any(|op| matches!(op, Op::DeleteTree { key } if key == &verb_key(DIRECTORY))));
        assert!(ops.iter().any(|op| matches!(op, Op::DeleteTree { key } if key == &verb_key(DRIVE))));
        assert!(ops.iter().all(|op| !matches!(op, Op::DeleteKey { .. })));
        assert!(ops.iter().all(|op| !op.key().ends_with(r"\shell\open") && !op.key().contains("SomeOther")));
    }

    #[test]
    fn restore_of_parents_we_created_removes_them_when_they_are_empty() {
        let states = vec![
            ours(DIRECTORY, None, 1, 1, &command_line(r"C:\maguro.exe", DIRECTORY)),
            ours(DRIVE, None, 1, 1, &command_line(r"C:\maguro.exe", DRIVE)),
        ];
        let ops = restore_ops(&states);
        for class in [DIRECTORY, DRIVE] {
            assert!(ops.iter().any(|op| matches!(op, Op::DeleteValue { key, name } if key == &shell_key(class) && name.is_empty())));
            assert!(ops.iter().any(|op| matches!(op, Op::DeleteTree { key } if key == &verb_key(class))));
            assert!(ops.iter().any(|op| matches!(op, Op::DeleteKey { key } if key == &shell_key(class))));
            assert!(ops.iter().any(|op| matches!(op, Op::DeleteKey { key } if key == &class_key(class))));
        }
        // The verb tree goes before the parents, so the shell key is empty when it is removed.
        let dir: Vec<&Op> = ops.iter().filter(|op| op.key().contains("Directory")).collect();
        let at = |pred: fn(&Op) -> bool| dir.iter().position(|op| pred(op)).unwrap();
        let verb_at = at(|op| matches!(op, Op::DeleteTree { key } if key.ends_with(r"\Maguro")));
        let shell_at = at(|op| matches!(op, Op::DeleteKey { key } if key.ends_with(r"\shell")));
        let class_at = at(|op| matches!(op, Op::DeleteKey { key } if key == r"Software\Classes\Directory"));
        assert!(verb_at < shell_at && shell_at < class_at);
    }

    #[test]
    fn restore_leaves_an_empty_shell_we_did_not_create() {
        let mut s = ours(DIRECTORY, None, 0, 0, "cmd");
        s.shell_created = Some(0);
        s.class_created = Some(0);
        let ops = restore_ops(&[s, ClassState::absent(DRIVE)]);
        assert!(ops.iter().any(|op| matches!(op, Op::DeleteValue { key, .. } if key == &shell_key(DIRECTORY))));
        assert!(ops.iter().any(|op| matches!(op, Op::DeleteTree { key } if key == &verb_key(DIRECTORY))));
        assert!(ops.iter().all(|op| !matches!(op, Op::DeleteKey { .. })));
        assert!(restore_ops(&fresh()).is_empty());
    }

    #[test]
    fn restore_does_not_clear_a_default_the_user_chose_later() {
        let mut s = ours(DIRECTORY, Some("open"), 0, 0, "cmd");
        s.shell_default = Some("explore".into());
        let ops = restore_ops(&[s, ClassState::absent(DRIVE)]);
        assert!(ops.iter().all(|op| !matches!(op, Op::SetSz { key, .. } if key == &shell_key(DIRECTORY))));
        assert!(ops.iter().all(|op| !matches!(op, Op::DeleteValue { key, .. } if key == &shell_key(DIRECTORY))));
        assert!(ops.iter().any(|op| matches!(op, Op::DeleteTree { key } if key == &verb_key(DIRECTORY))));
    }

    #[test]
    fn a_dangling_verb_name_is_removed_when_our_key_is_already_gone() {
        let mut s = ClassState::absent(DIRECTORY);
        s.shell_exists = true;
        s.shell_default = Some(VERB.into());
        s.shell_other_subkeys = vec!["open".into()];
        let ops = restore_ops(&[s, ClassState::absent(DRIVE)]);
        assert!(ops.iter().any(|op| matches!(op, Op::DeleteValue { key, name } if key == &shell_key(DIRECTORY) && name.is_empty())));
        assert!(ops.iter().all(|op| !matches!(op, Op::DeleteTree { .. } | Op::DeleteKey { .. })));
    }

    #[test]
    fn moving_the_exe_rewrites_only_the_command() {
        let old = r"C:\Old\maguro.exe";
        let new = r"D:\New\maguro.exe";
        let states = vec![ours(DIRECTORY, Some("open"), 0, 0, &command_line(old, DIRECTORY)), ours(DRIVE, Some("open"), 0, 0, &command_line(old, DRIVE))];
        let ops = path_fix_ops(new, &states);
        assert_eq!(ops.len(), 2);
        assert_eq!(sz(&ops, &command_key(DIRECTORY), ""), Some(command_line(new, DIRECTORY).as_str()));
        assert_eq!(sz(&ops, &command_key(DRIVE), ""), Some(command_line(new, DRIVE).as_str()));
        assert!(ops.iter().all(|op| matches!(op, Op::SetSz { key, .. } if key.ends_with(r"\command"))));
    }

    #[test]
    fn the_same_exe_in_another_case_is_not_rewritten() {
        let exe = r"C:\Maguro\maguro.exe";
        let states = vec![ours(DIRECTORY, None, 1, 1, &command_line(exe, DIRECTORY)), ours(DRIVE, None, 1, 1, &command_line(exe, DRIVE))];
        assert!(is_registered(&states));
        assert!(path_fix_ops(&exe.to_ascii_lowercase(), &states).is_empty());
        assert!(path_fix_ops(&format!(r"\\?\{exe}"), &states).is_empty());
        assert_eq!(exe_from_command(&command_line(exe, DRIVE)).as_deref(), Some(exe));
    }

    #[test]
    fn an_unregistered_hive_is_not_rewritten_on_startup() {
        assert!(!is_registered(&fresh()));
        assert!(path_fix_ops(r"C:\maguro.exe", &fresh()).is_empty());
        let mut only_dir = fresh();
        only_dir[0] = ours(DIRECTORY, None, 1, 1, &command_line(r"C:\old.exe", DIRECTORY));
        assert!(!is_registered(&only_dir));
        assert!(path_fix_ops(r"C:\new.exe", &only_dir).is_empty());
    }

    #[test]
    fn shell_and_clsid_paths_go_back_to_explorer() {
        let explorer = [
            "shell:RecycleBinFolder",
            "Shell:AppsFolder",
            "search-ms:query=hi",
            "knownfolder:{B4BFCC3A-DB2C-424C-B029-7FE99A87C641}",
            "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
            "::{645FF040-5081-101B-9F08-00AA002F954E}",
            r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}\C:\",
            "{645FF040-5081-101B-9F08-00AA002F954E}",
            "  shell:Desktop  ",
        ];
        for p in explorer {
            assert_eq!(route_arg(p), ArgRoute::Explorer, "{p}");
        }
        let pane = [r"C:\", r"C:\Users\Public", r"\\server\share\dir", r"D:", "notes.txt", "session.maguro", r"folder\child"];
        for p in pane {
            assert_eq!(route_arg(p), ArgRoute::Pane, "{p}");
        }
        let (pane, expl) = split_launch(vec![r"C:\Work".into(), "shell:RecycleBinFolder".into(), r"\\srv\share".into()]);
        assert_eq!(pane, vec![r"C:\Work", r"\\srv\share"]);
        assert_eq!(expl, vec!["shell:RecycleBinFolder"]);
        assert_eq!(explorer_arg("shell:RecycleBinFolder"), "shell:RecycleBinFolder");
        assert_eq!(explorer_arg(r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"), "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}");
        assert_eq!(explorer_arg(r"C:\My Folder"), r#""C:\My Folder""#);
    }

    fn folder() -> OpenIn {
        OpenIn { registered: true, ctrl: false, middle: false, folder: true, stream: false, browsable: false, directory: false }
    }

    #[test]
    fn a_folder_in_the_list_stays_in_the_pane_unless_ctrl_or_middle_asks_for_a_tab() {
        let plain = folder();
        assert_eq!(list_open(plain), OpenWhere::Pane);
        assert_eq!(list_open(OpenIn { ctrl: true, ..plain }), OpenWhere::Shell);
        assert_eq!(list_open(OpenIn { middle: true, ..plain }), OpenWhere::Shell);
        // Setting off: the hosted view already navigates. Do not take the click.
        assert_eq!(list_open(OpenIn { registered: false, ..plain }), OpenWhere::Shell);
        // A file keeps its own program.
        assert_eq!(list_open(OpenIn { folder: false, ..plain }), OpenWhere::Shell);
        // Library (browsable folder) and a zip-like container navigate.
        assert_eq!(list_open(OpenIn { browsable: true, ..plain }), OpenWhere::Pane);
        assert_eq!(list_open(OpenIn { stream: true, browsable: true, ..plain }), OpenWhere::Pane);
        // A stream that is not a browsable folder is a file.
        assert!(!navigable(true, true, false));
        assert_eq!(list_open(OpenIn { stream: true, browsable: false, ..plain }), OpenWhere::Shell);
        assert!(navigable(true, false, false));
        assert!(!navigable(false, false, true));
        // Junction, directory symlink, or a folder whose SFGAO bits were not cached yet.
        // The directory attribute is enough. Ctrl and middle still ask for a tab.
        let dir = OpenIn { folder: false, stream: false, browsable: false, directory: true, ..plain };
        assert_eq!(list_open(dir), OpenWhere::Pane);
        assert_eq!(list_open(OpenIn { ctrl: true, ..dir }), OpenWhere::Shell);
        assert_eq!(list_open(OpenIn { middle: true, ..dir }), OpenWhere::Shell);
        assert_eq!(list_open(OpenIn { registered: false, ..dir }), OpenWhere::Shell);
        // A file whose attributes did come back is not taken.
        assert_eq!(list_open(OpenIn { folder: false, directory: false, ..plain }), OpenWhere::Shell);
    }

    #[test]
    fn open_on_the_menu_navigates_one_folder_and_leaves_new_window_verbs() {
        assert_eq!(menu_open(true, false, false, "open", 1, false, false), OpenWhere::Pane);
        assert_eq!(menu_open(true, false, false, "Open", 1, false, false), OpenWhere::Pane);
        assert_eq!(menu_open(true, false, false, "explore", 1, false, false), OpenWhere::Pane);
        assert_eq!(menu_open(true, false, false, "Maguro", 1, false, false), OpenWhere::Pane);
        assert_eq!(menu_open(true, false, false, "  maguro  ", 1, false, false), OpenWhere::Pane);
        // "Open in new window" still launches, which the running window turns into a tab.
        assert_eq!(menu_open(true, false, false, "opennew", 1, false, false), OpenWhere::Shell);
        assert_eq!(menu_open(true, false, false, "opennewwindow", 1, false, false), OpenWhere::Shell);
        assert_eq!(menu_open(true, false, false, "properties", 1, false, false), OpenWhere::Shell);
        assert_eq!(menu_open(true, false, false, "open", 2, false, false), OpenWhere::Shell);
        assert_eq!(menu_open(true, true, false, "open", 1, false, false), OpenWhere::Shell);
        assert_eq!(menu_open(true, false, true, "open", 1, false, false), OpenWhere::Shell);
        assert_eq!(menu_open(false, false, false, "open", 1, false, false), OpenWhere::Shell);
        // A file is not taken. A .lnk is a candidate; the caller keeps it only if the target is a folder.
        assert_eq!(menu_open(true, false, false, "open", 1, true, false), OpenWhere::Shell);
        assert_eq!(menu_open(true, false, false, "open", 1, true, true), OpenWhere::Pane);
        assert!(open_verb("explore") && !open_verb("opennew") && !open_verb(""));
        // The default item with no verb string is Open. A different empty verb is not.
        assert_eq!(menu_open_item(true, false, false, "", 1, false, false, true), OpenWhere::Pane);
        assert_eq!(menu_open_item(true, false, false, "", 1, false, false, false), OpenWhere::Shell);
        assert_eq!(menu_open_item(true, false, false, "open", 1, false, false, false), OpenWhere::Pane);
        assert_eq!(menu_open_item(true, true, false, "", 1, false, false, true), OpenWhere::Shell);
        assert_eq!(menu_open_item(true, false, false, "", 1, true, false, true), OpenWhere::Shell);
        assert_eq!(menu_open_item(true, false, false, "", 1, true, true, true), OpenWhere::Pane);
    }

    #[test]
    fn enter_on_several_rows_does_not_hand_a_folder_to_the_verb() {
        use ActKind::*;
        assert_eq!(list_activation(true, false, false, &[Folder]), ListAct::Browse(0));
        assert_eq!(list_activation(true, false, false, &[File]), ListAct::Shell);
        assert_eq!(list_activation(true, false, false, &[Unknown]), ListAct::Consume);
        // Focused row is a folder; other selected files must not make the verb run.
        assert_eq!(list_activation(true, false, false, &[Folder, File, Folder]), ListAct::Browse(0));
        // Focused row is a file and a folder is also selected. Open the files only.
        assert_eq!(list_activation(true, false, false, &[File, Folder]), ListAct::FilesOnly);
        assert_eq!(list_activation(true, false, false, &[File, File]), ListAct::Shell);
        // Focus was not available. The first known folder is the one to browse.
        assert_eq!(list_activation(true, false, false, &[Unknown, Folder]), ListAct::Browse(1));
        assert_eq!(list_activation(true, true, false, &[Folder, Folder]), ListAct::Shell);
        assert_eq!(list_activation(true, false, true, &[Folder]), ListAct::Shell);
        assert_eq!(list_activation(false, false, false, &[Folder]), ListAct::Browse(0));
        assert_eq!(list_activation(true, false, false, &[]), ListAct::Consume);
    }

    #[test]
    fn a_child_of_this_process_browses_in_place_unless_a_new_tab_was_just_requested() {
        assert_eq!(handoff_action(10, 10, None), Handoff::InPlace);
        assert_eq!(handoff_action(10, 10, Some(0)), Handoff::NewTab);
        assert_eq!(handoff_action(10, 10, Some(NEW_TAB_INTENT_MS)), Handoff::NewTab);
        assert_eq!(handoff_action(10, 10, Some(NEW_TAB_INTENT_MS + 1)), Handoff::InPlace);
        // Explorer, Win+E, another app: the parent is not this process.
        assert_eq!(handoff_action(4, 10, None), Handoff::NewTab);
        assert_eq!(handoff_action(4, 10, Some(5_000)), Handoff::NewTab);
        // A parent that could not be read is not "us", even when both ids are 0.
        assert_eq!(handoff_action(0, 10, None), Handoff::NewTab);
        assert_eq!(handoff_action(0, 0, None), Handoff::NewTab);
    }

    #[test]
    fn handoff_text_roundtrips_non_ascii_paths_without_cutting_a_character() {
        let paths = vec![
            r"D:\한글\사진".to_string(),
            r"\\server\共有\フォルダ".to_string(),
            "E:\\📁\\work".to_string(),
            r"C:\Users\Юзер\Документы".to_string(),
        ];
        let text = handoff_text(4242, &paths);
        assert!(text.starts_with('\u{1}'));
        assert_eq!(parse_handoff(&text), (4242, paths.clone()));
        // An older sender has no header. Every line is still a path.
        let legacy = paths.join("\n");
        assert_eq!(parse_handoff(&legacy), (0, paths));
        assert_eq!(parse_handoff(""), (0, Vec::<String>::new()));
        assert_eq!(parse_handoff("\u{1}pg-parent:not-a-number\nC:\\a"), (0, vec!["C:\\a".to_string()]));
        // A folder whose name merely contains the words of the header is an argument.
        let tricky = "pg-parent:12".to_string();
        assert_eq!(parse_handoff(&handoff_text(7, &[tricky.clone()])), (7, vec![tricky]));
    }

    #[test]
    fn the_folder_tree_browses_on_enter_and_double_click_but_not_on_ctrl_or_middle() {
        assert_eq!(tree_activate(true, false, false, true), OpenWhere::Pane);
        assert_eq!(tree_activate(true, true, false, true), OpenWhere::Shell);
        assert_eq!(tree_activate(true, false, true, true), OpenWhere::Shell);
        assert_eq!(tree_activate(false, false, false, true), OpenWhere::Shell);
        assert_eq!(tree_activate(true, false, false, false), OpenWhere::Shell);
    }

    #[test]
    fn a_shortcut_path_is_recognised_without_opening_it_and_an_outside_launch_is_still_a_tab() {
        assert!(link_file(r"C:\Work\Projects.lnk"));
        assert!(link_file(r"\\server\share\folder.LNK"));
        assert!(link_file("  notes.lnk  "));
        assert!(!link_file(r"C:\Work\Projects"));
        assert!(!link_file(r"C:\Work\file.lnk.txt"));
        assert!(!link_file(""));
        // Another program opening a folder is still an argument for this process.
        // The running window opens that as a tab. It is not an in-place navigation.
        assert_eq!(route_arg(r"D:\Photos"), ArgRoute::Pane);
        assert_eq!(route_arg("shell:Downloads"), ArgRoute::Explorer);
    }
}
#[cfg(test)]
mod utf8_boundary_tests {
    #[test]
    fn japanese_korean_and_emoji_commands_do_not_panic() {
        assert_eq!(
            super::exe_from_command(r#""D:\한글폴더\app.exe" /x"#).as_deref(),
            Some(r"D:\한글폴더\app.exe")
        );
        assert_eq!(
            super::exe_from_command(r#""E:\📁\サンプル.exe""#).as_deref(),
            Some(r"E:\📁\サンプル.exe")
        );
        assert_eq!(super::exe_from_command(r"F:\샘플\abcdサ.exe").as_deref(), Some(r"F:\샘플\abcdサ.exe"));
        assert_eq!(super::exe_from_command("😀.exe").as_deref(), Some("😀.exe"));
        assert!(super::exe_from_command("").is_none());
        assert!(super::exe_from_command("\"").is_none());
    }
}
