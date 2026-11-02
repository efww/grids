//! Robocopy copy/move with Maguro's own progress and conflict window.
//!
//! Local-to-local stays on IFileOperation unless the user picks Fast paste.
//! The argument builder never emits `/MIR`, `/PURGE`, `/MOV`, or `/MOVE`.
//! A move is a copy followed by deleting the sources only after every step succeeds.

use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Controls::{TTF_ABSOLUTE, TTF_TRACK, TTM_TRACKACTIVATE, TTM_TRACKPOSITION};
use windows::Win32::UI::HiDpi::GetDpiForWindow;

use crate::imports::*;
use crate::i18n::*;

const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Fast paste always. Otherwise only when the setting is on and a path is on the network.
pub(crate) fn decide_fast(force: bool, setting: bool, network: bool) -> bool {
    force || (setting && network)
}

/// UNC, a live rclone mount, or a caller-supplied drive type (`Some(4)` is `DRIVE_REMOTE`).
/// A UNC path does not use the drive type.
pub(crate) fn classify_network(path: &str, drive_type: Option<u32>) -> bool {
    classify_network_roots(path, drive_type, &crate::remote::mount_roots())
}

/// Same rule as [`classify_network`], with mount roots supplied by the caller so a test can cover a drive letter that is not `DRIVE_REMOTE`.
pub(crate) fn classify_network_roots(path: &str, drive_type: Option<u32>, roots: &[String]) -> bool {
    if crate::remote::covered_by_mount(path, roots) {
        return true;
    }
    let unc = crate::unc_path(path);
    crate::share_kind(unc, if unc { None } else { drive_type })
}

pub(crate) fn network_transfer(srcs: &[String], dst: &str) -> bool {
    path_network(dst) || srcs.iter().any(|s| path_network(s))
}

fn path_network(path: &str) -> bool {
    unsafe { crate::share_path(path) }
}

/// `robocopy` arguments for one step. `tree` adds `/E`. File steps pass names and omit `/E`.
pub(crate) fn robocopy_args(src: &str, dst: &str, files: &[String], tree: bool, log_name: &str) -> Vec<String> {
    let mut args = vec![clean_dir(src), clean_dir(dst)];
    if !tree {
        args.extend(files.iter().cloned());
    }
    if tree {
        args.push("/E".into());
    }
    args.extend(["/MT:8".into(), "/R:1".into(), "/W:1".into(), "/NP".into(), "/BYTES".into(), "/NDL".into(), "/FP".into(), "/IS".into()]);
    // The log name is a file in the process's working directory, so a spaced temp path is not an argument.
    args.push(format!("/UNILOG:{log_name}"));
    args
}

pub(crate) fn args_forbidden(args: &[String]) -> bool {
    args.iter().any(|a| {
        let flag = a.split(':').next().unwrap_or(a).to_ascii_uppercase();
        matches!(flag.as_str(), "/MIR" | "/PURGE" | "/MOV" | "/MOVE")
    })
}

pub(crate) fn exit_ok(code: i32) -> bool {
    (0..8).contains(&code)
}

/// Windows "Name - Copy" / "Name - Copy (2)" with the last extension kept.
pub(crate) fn copy_name(name: &str, n: u32) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && !e.contains('\\') && !e.contains('/') => (s, e),
        _ => (name, ""),
    };
    let tag = if n <= 1 { " - Copy".to_string() } else { format!(" - Copy ({n})") };
    if ext.is_empty() {
        format!("{stem}{tag}")
    } else {
        format!("{stem}{tag}.{ext}")
    }
}

/// A file line from a robocopy `/UNILOG`. Header lines have no tab, so they are ignored.
pub(crate) fn parse_log_line(line: &str) -> Option<(String, u64)> {
    if !line.contains('\t') {
        return None;
    }
    let parts: Vec<&str> = line.split('\t').map(str::trim).filter(|s| !s.is_empty()).collect();
    let path = parts.last().copied().filter(|s| s.contains('\\') || s.contains('/'))?;
    let bytes = parts.iter().rev().skip(1).find_map(|s| s.replace(',', "").parse::<u64>().ok()).unwrap_or(0);
    Some((path.to_string(), bytes))
}

pub(crate) fn parse_error_line(line: &str) -> Option<String> {
    let u = line.to_ascii_uppercase();
    if u.contains(" ERROR ") || u.trim_start().starts_with("ERROR ") {
        let t = line.trim();
        (!t.is_empty()).then(|| t.to_string())
    } else {
        None
    }
}

pub(crate) struct FastJob {
    pub(crate) owner: isize,
    pub(crate) accent: u32,
    pub(crate) face: String,
    pub(crate) srcs: Vec<String>,
    pub(crate) dst: String,
    pub(crate) rename: bool,
    pub(crate) mv: bool,
}

pub(crate) fn run_fast(job: FastJob) {
    let Some(exe) = robocopy_exe() else {
        complain(job.owner, &tr(Key::RobocopyWasNotFound).to_string());
        return;
    };
    let mut ui = Progress::open(&job);
    ui.set_scan();
    ui.pump();
    let mut items = plan_items(&job);
    if ui.cancelled() {
        ui.show_cancelled();
        return;
    }
    if !resolve_conflicts(&mut ui, &mut items) {
        ui.show_cancelled();
        return;
    }
    let steps = build_steps(&items);
    let (files, bytes) = tally(&steps, &ui.cancel);
    ui.set_totals(files, bytes);
    if ui.cancelled() {
        ui.show_cancelled();
        return;
    }
    let dir = match work_dir() {
        Some(d) => d,
        None => {
            ui.close();
            complain(job.owner, &tr(Key::CopyingFailed).to_string());
            return;
        }
    };
    let mut errors = Vec::new();
    let mut copied: Vec<(String, bool)> = Vec::new();
    let mut failed = false;
    for step in &steps {
        if ui.cancelled() {
            break;
        }
        match run_step(&exe, &dir, step, &mut ui, &mut errors) {
            StepEnd::Ok => copied.extend(step.sources()),
            StepEnd::Cancel => break,
            StepEnd::Fail(code) => {
                failed = true;
                if errors.is_empty() {
                    errors.push(format!("robocopy {code}"));
                }
                break;
            }
        }
    }
    let cancelled = ui.cancelled();
    if job.mv && !cancelled && !failed {
        for (path, is_dir) in &copied {
            if let Err(code) = delete_source(path, *is_dir) {
                failed = true;
                let sharing = code == 32 || code == 33;
                if sharing || errors.len() < 40 {
                    if sharing {
                        errors.push(format!("ERROR {code} (0x{code:08X}) {path}"));
                    } else {
                        errors.push(path.clone());
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if !copied.is_empty() || failed {
        notify_dir(&job.dst);
        if job.mv {
            for src in &job.srcs {
                notify_dir(&parent_dir(src));
            }
        }
    }
    if cancelled {
        ui.show_cancelled();
        return;
    }
    ui.close();
    if failed || !errors.is_empty() {
        if errors.iter().any(|line| super::inuse::text_is_in_use(line)) {
            let paths = fast_locked_paths(&job, &errors);
            let retry = unsafe {
                super::inuse::ask_retry(
                    HWND(job.owner as *mut _),
                    if job.mv { super::inuse::OpKind::Move } else { super::inuse::OpKind::Copy },
                    &paths,
                )
            };
            if retry {
                run_fast(job);
            }
            return;
        }
        let list = clip_list(&errors, 12);
        let text = tf(Key::NotCopied, &[("list", &list)]);
        complain(job.owner, &text);
    }
}

/// Sources, the destination folder, and each destination file. Robocopy lines may name the path too.
fn fast_locked_paths(job: &FastJob, errors: &[String]) -> Vec<String> {
    let mut out = job.srcs.clone();
    if !job.dst.is_empty() {
        out.push(job.dst.clone());
    }
    for src in &job.srcs {
        let Some(name) = Path::new(src).file_name() else { continue };
        let name = name.to_string_lossy();
        if name.is_empty() {
            continue;
        }
        let folder = job.dst.trim_end_matches(['\\', '/']);
        out.push(format!("{folder}\\{name}"));
    }
    for line in errors {
        if let Some(idx) = line.find(":\\") {
            let start = idx.saturating_sub(1);
            let path = line[start..].trim().trim_matches('"').to_string();
            if path.len() > 3 {
                out.push(path);
            }
        } else if line.starts_with(r"\\") {
            out.push(line.trim().to_string());
        }
    }
    out
}

fn notify_dir(path: &str) {
    if path.is_empty() {
        return;
    }
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        SHChangeNotify(SHCNE_UPDATEDIR, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    }
}

fn robocopy_exe() -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 320];
    let n = unsafe { GetSystemDirectoryW(Some(&mut buf)) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    let dir = String::from_utf16_lossy(&buf[..n]);
    let path = Path::new(&dir).join("robocopy.exe");
    path.is_file().then_some(path)
}

fn work_dir() -> Option<std::path::PathBuf> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("maguro-fast-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn complain(owner: isize, text: &str) {
    let owner = HWND(owner as *mut _);
    unsafe {
        let _ = MessageBoxW(Some(owner), &HSTRING::from(text), w!("Maguro"), MB_ICONWARNING);
    }
}

fn clip_list(errors: &[String], n: usize) -> String {
    let mut lines: Vec<&str> = errors.iter().take(n).map(String::as_str).collect();
    if errors.len() > n {
        lines.push("…");
    }
    lines.join("\n")
}

#[derive(Clone)]
struct Item {
    src: String,
    dest: String,
    is_dir: bool,
    skip: bool,
}

fn plan_items(job: &FastJob) -> Vec<Item> {
    let mut taken = HashSet::new();
    let mut out = Vec::new();
    for src in &job.srcs {
        let name = file_name(src);
        if name.is_empty() {
            continue;
        }
        let is_dir = Path::new(src).is_dir();
        let dest_name = if job.rename { unique_name(&job.dst, name, &mut taken) } else { name.to_string() };
        if !job.rename {
            taken.insert(dest_name.to_lowercase());
        }
        let dest = join_path(&job.dst, &dest_name);
        out.push(Item { src: src.clone(), dest, is_dir, skip: false });
    }
    out
}

fn unique_name(dir: &str, name: &str, taken: &mut HashSet<String>) -> String {
    let mut n = 1u32;
    loop {
        let cand = copy_name(name, n);
        let key = cand.to_lowercase();
        let full = join_path(dir, &cand);
        if !taken.contains(&key) && !Path::new(&full).exists() {
            taken.insert(key);
            return cand;
        }
        n += 1;
        if n > 9999 {
            return cand;
        }
    }
}

fn resolve_conflicts(ui: &mut Progress, items: &mut [Item]) -> bool {
    let mut policy: Option<u8> = None;
    for item in items.iter_mut() {
        if ui.cancelled() {
            return false;
        }
        if same_path(&item.src, &item.dest) || contains_path(&item.src, &item.dest) {
            item.skip = true;
            continue;
        }
        if !Path::new(&item.dest).exists() {
            continue;
        }
        let act = if let Some(p) = policy {
            p
        } else {
            let name = file_name(&item.dest).to_string();
            let ans = ui.ask(&name);
            if ans.is_none() {
                return false;
            }
            let (act, all) = ans.unwrap();
            if all {
                policy = Some(act);
            }
            act
        };
        match act {
            1 => {}
            2 => item.skip = true,
            _ => return false,
        }
    }
    true
}

enum Step {
    Files { from: String, to: String, names: Vec<String>, sources: Vec<String> },
    Tree { from: String, to: String, source: String },
    AsName { from: String, file: String, dest: String, source: String },
}

impl Step {
    fn sources(&self) -> Vec<(String, bool)> {
        match self {
            Step::Files { sources, .. } => sources.iter().cloned().map(|s| (s, false)).collect(),
            Step::Tree { source, .. } => vec![(source.clone(), true)],
            Step::AsName { source, .. } => vec![(source.clone(), false)],
        }
    }
}

fn build_steps(items: &[Item]) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut batch: Option<Step> = None;
    let flush = |steps: &mut Vec<Step>, batch: &mut Option<Step>| {
        if let Some(step) = batch.take() {
            steps.push(step);
        }
    };
    for item in items.iter().filter(|i| !i.skip) {
        let src_name = file_name(&item.src).to_string();
        let dest_name = file_name(&item.dest).to_string();
        let to = parent_dir(&item.dest);
        if item.is_dir {
            flush(&mut steps, &mut batch);
            steps.push(Step::Tree { from: item.src.clone(), to: item.dest.clone(), source: item.src.clone() });
            continue;
        }
        if src_name != dest_name {
            flush(&mut steps, &mut batch);
            steps.push(Step::AsName { from: parent_dir(&item.src), file: src_name, dest: item.dest.clone(), source: item.src.clone() });
            continue;
        }
        let from = parent_dir(&item.src);
        let extend = match &batch {
            Some(Step::Files { from: f, to: t, names, .. }) => f == &from && t == &to && names.len() < 64 && joined_len(names) + src_name.len() < 24_000,
            _ => false,
        };
        if extend {
            if let Some(Step::Files { names, sources, .. }) = &mut batch {
                names.push(src_name);
                sources.push(item.src.clone());
            }
        } else {
            flush(&mut steps, &mut batch);
            batch = Some(Step::Files { from, to, names: vec![src_name], sources: vec![item.src.clone()] });
        }
    }
    flush(&mut steps, &mut batch);
    steps
}

fn joined_len(names: &[String]) -> usize {
    names.iter().map(String::len).sum::<usize>() + names.len()
}

fn tally(steps: &[Step], cancel: &AtomicBool) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    for step in steps {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        match step {
            Step::Files { sources, .. } => {
                for s in sources {
                    add_file(Path::new(s), &mut files, &mut bytes);
                }
            }
            Step::AsName { source, .. } => add_file(Path::new(source), &mut files, &mut bytes),
            Step::Tree { from, .. } => walk(Path::new(from), &mut files, &mut bytes, cancel, 0),
        }
    }
    (files, bytes)
}

fn add_file(path: &Path, files: &mut u64, bytes: &mut u64) {
    if let Ok(m) = path.metadata() {
        if m.is_file() {
            *files += 1;
            *bytes += m.len();
        }
    }
}

fn walk(path: &Path, files: &mut u64, bytes: &mut u64, cancel: &AtomicBool, depth: u32) {
    if cancel.load(Ordering::Relaxed) || depth > 40 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(path) else { return };
    for ent in rd.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let p = ent.path();
        let Ok(ft) = ent.file_type() else { continue };
        if ft.is_dir() {
            walk(&p, files, bytes, cancel, depth + 1);
        } else {
            add_file(&p, files, bytes);
        }
    }
}

enum StepEnd {
    Ok,
    Cancel,
    Fail(i32),
}

fn run_step(exe: &Path, dir: &Path, step: &Step, ui: &mut Progress, errors: &mut Vec<String>) -> StepEnd {
    match step {
        Step::Files { from, to, names, .. } => run_robocopy(exe, dir, from, to, names, false, ui, errors),
        Step::Tree { from, to, .. } => run_robocopy(exe, dir, from, to, &[], true, ui, errors),
        Step::AsName { from, file, dest, .. } => {
            let stage = dir.join("stage");
            let _ = std::fs::remove_dir_all(&stage);
            if std::fs::create_dir_all(&stage).is_err() {
                return StepEnd::Fail(16);
            }
            let end = run_robocopy(exe, dir, from, &stage.to_string_lossy(), &[file.clone()], false, ui, errors);
            if !matches!(end, StepEnd::Ok) {
                return end;
            }
            let staged = stage.join(file);
            if move_file(&staged, Path::new(dest)) {
                StepEnd::Ok
            } else {
                errors.push(dest.clone());
                StepEnd::Fail(16)
            }
        }
    }
}

fn move_file(from: &Path, to: &Path) -> bool {
    if let Some(parent) = to.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::rename(from, to).is_ok() {
        return true;
    }
    if std::fs::copy(from, to).is_ok() {
        let _ = std::fs::remove_file(from);
        return true;
    }
    false
}

fn delete_source(path: &str, is_dir: bool) -> std::result::Result<(), i32> {
    let result = if is_dir { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    result.map_err(|e| e.raw_os_error().unwrap_or(0))
}

fn run_robocopy(exe: &Path, dir: &Path, src: &str, dst: &str, files: &[String], tree: bool, ui: &mut Progress, errors: &mut Vec<String>) -> StepEnd {
    let log_name = format!("step-{}.log", ui.step_no);
    ui.step_no += 1;
    let log_path = dir.join(&log_name);
    let _ = std::fs::remove_file(&log_path);
    let args = robocopy_args(src, dst, files, tree, &log_name);
    if args_forbidden(&args) {
        return StepEnd::Fail(16);
    }
    let mut child = match Command::new(exe)
        .args(&args)
        .current_dir(dir)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return StepEnd::Fail(16),
    };
    let mut reader = LogRead::default();
    let mut pending: Option<(String, u64)> = None;
    loop {
        ui.pump();
        if ui.cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return StepEnd::Cancel;
        }
        for line in reader.poll(&log_path) {
            if let Some(err) = parse_error_line(&line) {
                if errors.len() < 40 {
                    errors.push(err);
                }
                continue;
            }
            if let Some(hit) = parse_log_line(&line) {
                if let Some(prev) = pending.take() {
                    ui.add_done(prev.1);
                }
                ui.set_current(hit.0.clone());
                pending = Some(hit);
            }
        }
        ui.paint_if_due();
        match child.try_wait() {
            Ok(Some(status)) => {
                for line in reader.poll(&log_path) {
                    if let Some(err) = parse_error_line(&line) {
                        if errors.len() < 40 {
                            errors.push(err);
                        }
                    } else if let Some(hit) = parse_log_line(&line) {
                        if let Some(prev) = pending.take() {
                            ui.add_done(prev.1);
                        }
                        ui.set_current(hit.0.clone());
                        pending = Some(hit);
                    }
                }
                let code = status.code().unwrap_or(16);
                if ui.cancelled() {
                    return StepEnd::Cancel;
                }
                if exit_ok(code) {
                    if let Some(prev) = pending.take() {
                        ui.add_done(prev.1);
                    }
                    return StepEnd::Ok;
                }
                return StepEnd::Fail(code);
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(30)),
            Err(_) => return StepEnd::Fail(16),
        }
    }
}

#[derive(Default)]
struct LogRead {
    off: usize,
    partial: String,
}

impl LogRead {
    fn poll(&mut self, path: &Path) -> Vec<String> {
        let chunk = read_new(path, &mut self.off);
        if chunk.is_empty() {
            return Vec::new();
        }
        self.partial.push_str(&chunk);
        let mut lines = Vec::new();
        while let Some(i) = self.partial.find('\n') {
            let mut line: String = self.partial.drain(..=i).collect();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            lines.push(line);
        }
        lines
    }
}

fn read_new(path: &Path, off: &mut usize) -> String {
    let Ok(bytes) = std::fs::read(path) else { return String::new() };
    if bytes.len() < *off + 2 {
        return String::new();
    }
    let mut end = bytes.len();
    if (end - *off) % 2 == 1 {
        end -= 1;
    }
    let start = if *off == 0 && bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE { 2 } else { *off };
    if start >= end {
        return String::new();
    }
    let wide: Vec<u16> = bytes[start..end].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    *off = end;
    String::from_utf16_lossy(&wide)
}

fn clean_dir(path: &str) -> String {
    let t = path.trim().trim_end_matches(['\\', '/']);
    if t.len() == 2 && t.as_bytes().get(1) == Some(&b':') {
        format!("{t}\\")
    } else if t.is_empty() {
        path.to_string()
    } else {
        t.to_string()
    }
}

fn join_path(dir: &str, name: &str) -> String {
    let d = clean_dir(dir);
    if d.ends_with('\\') {
        format!("{d}{name}")
    } else {
        format!("{d}\\{name}")
    }
}

fn parent_dir(path: &str) -> String {
    let p = path.trim_end_matches(['\\', '/']);
    match p.rsplit_once(['\\', '/']) {
        Some((dir, _)) if dir.ends_with(':') => format!("{dir}\\"),
        Some((dir, _)) if !dir.is_empty() => dir.to_string(),
        _ => p.to_string(),
    }
}

fn file_name(path: &str) -> &str {
    path.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(path)
}

fn norm_path(path: &str) -> String {
    clean_dir(path).replace('/', "\\").to_lowercase()
}

fn same_path(a: &str, b: &str) -> bool {
    norm_path(a) == norm_path(b)
}

fn contains_path(root: &str, child: &str) -> bool {
    let r = norm_path(root);
    let c = norm_path(child);
    c == r || c.starts_with(&format!("{r}\\"))
}

fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let n = n as f64;
    if n >= GB {
        format!("{:.1} GB", n / GB)
    } else if n >= MB {
        format!("{:.1} MB", n / MB)
    } else if n >= KB {
        format!("{:.1} KB", n / KB)
    } else {
        format!("{} B", n as u64)
    }
}

/// `{n} second` / `{m} minutes {s} seconds`, then wrapped by AboutTimeLeft.
fn remain_time(secs: u64) -> String {
    let secs = secs.max(1);
    let n = |k: u64| k.to_string();
    if secs < 60 {
        let key = if secs == 1 { Key::TimeSecond } else { Key::TimeSeconds };
        return tf(key, &[("n", &n(secs))]);
    }
    let mins = secs / 60;
    let rem = secs % 60;
    if rem == 0 {
        let key = if mins == 1 { Key::TimeMinute } else { Key::TimeMinutes };
        return tf(key, &[("n", &n(mins))]);
    }
    let key = match (mins == 1, rem == 1) {
        (true, true) => Key::TimeMinuteSecond,
        (true, false) => Key::TimeMinuteSeconds,
        (false, true) => Key::TimeMinutesSecond,
        (false, false) => Key::TimeMinutesSeconds,
    };
    tf(key, &[("m", &n(mins)), ("s", &n(rem))])
}

fn eta_text(done: u64, total: u64, started: Instant) -> String {
    if done == 0 || total <= done {
        return String::new();
    }
    let elapsed = started.elapsed().as_millis().max(1);
    let remain = elapsed.saturating_mul((total - done) as u128) / done as u128;
    let secs = u64::try_from(remain / 1000).unwrap_or(u64::MAX).max(1);
    tf(Key::AboutTimeLeft, &[("time", &remain_time(secs))])
}

/// Groove plus a darker 1px edge. Both stay off the window ground so a 0% bar is still visible.
fn track_colors(bg: u32, sub: u32) -> (u32, u32) {
    (crate::blend(bg, sub, 0.35), crate::blend(bg, sub, 0.55))
}

/// Keep the head and tail of `text` and fold the middle into `…` until `measure` fits `width`.
fn middle_ellipsis(text: &str, width: i32, measure: impl Fn(&str) -> i32) -> String {
    if text.is_empty() || width <= 0 {
        return String::new();
    }
    if measure(text) <= width {
        return text.to_string();
    }
    if measure("…") > width {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut best = "…".to_string();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo <= hi {
        let keep = (lo + hi) / 2;
        let left_n = keep / 2;
        let right_n = keep - left_n;
        if left_n + right_n >= chars.len() {
            if keep == 0 {
                break;
            }
            hi = keep - 1;
            continue;
        }
        let mut s = String::new();
        s.extend(chars.iter().take(left_n));
        s.push('…');
        s.extend(chars.iter().skip(chars.len() - right_n));
        if measure(&s) <= width {
            best = s;
            lo = keep + 1;
        } else if keep == 0 {
            break;
        } else {
            hi = keep - 1;
        }
    }
    best
}

/// DIP layout. Cancel shares the ETA row, so the window ends one pad under that row.
struct ProgGeom {
    w: i32,
    h: i32,
    pad: i32,
    title_y: i32,
    stats_y: i32,
    bar_y: i32,
    file_y: i32,
    row_y: i32,
    line: i32,
    bar_h: i32,
    btn_w: i32,
    btn_h: i32,
}

fn progress_geom(dpi: i32) -> ProgGeom {
    let pad = sc(dpi, 16);
    let gap = sc(dpi, 8);
    let line = sc(dpi, 16);
    let bar_h = sc(dpi, 8);
    let btn_w = sc(dpi, 96);
    let btn_h = sc(dpi, 32);
    let title_y = pad;
    let stats_y = title_y + line + gap;
    let bar_y = stats_y + line + gap;
    let file_y = bar_y + bar_h + gap;
    let row_y = file_y + line + gap;
    let h = row_y + btn_h + pad;
    ProgGeom { w: sc(dpi, 480), h, pad, title_y, stats_y, bar_y, file_y, row_y, line, bar_h, btn_w, btn_h }
}

fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi.max(96) / 96
}

struct Progress {
    hwnd: HWND,
    accent: u32,
    face: String,
    cancel: Arc<AtomicBool>,
    files_done: u64,
    files_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    current: String,
    started: Instant,
    step_no: u32,
    dirty: bool,
}

impl Progress {
    fn open(job: &FastJob) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let owner = HWND(job.owner as *mut _);
        let hwnd = open_progress(owner, job.accent, &job.face, job.mv, cancel.clone());
        Self {
            hwnd,
            accent: job.accent,
            face: job.face.clone(),
            cancel,
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            current: String::new(),
            started: Instant::now(),
            step_no: 0,
            dirty: true,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn pump(&mut self) {
        pump_messages();
        self.dirty = true;
    }

    fn set_scan(&mut self) {
        if let Some(v) = view(self.hwnd) {
            unsafe {
                (*v).stats = tr(Key::Scanning).to_string();
                (*v).frac = 0.0;
            }
        }
        self.invalidate();
    }

    fn set_totals(&mut self, files: u64, bytes: u64) {
        self.files_total = files;
        self.bytes_total = bytes;
        self.started = Instant::now();
        self.refresh_text();
    }

    fn add_done(&mut self, bytes: u64) {
        self.files_done += 1;
        self.bytes_done += bytes;
        self.dirty = true;
    }

    fn set_current(&mut self, path: String) {
        self.current = path;
        self.dirty = true;
    }

    fn paint_if_due(&mut self) {
        if self.dirty {
            self.refresh_text();
            self.dirty = false;
        }
        self.pump();
    }

    fn refresh_text(&mut self) {
        let Some(v) = view(self.hwnd) else { return };
        let frac = if self.bytes_total > 0 {
            (self.bytes_done as f32 / self.bytes_total as f32).clamp(0.0, 1.0)
        } else if self.files_total > 0 {
            (self.files_done as f32 / self.files_total as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let stats = tf(
            Key::DoneTotalFiles,
            &[
                ("done", &self.files_done.to_string()),
                ("total", &self.files_total.to_string()),
                ("bdone", &format_bytes(self.bytes_done)),
                ("btotal", &format_bytes(self.bytes_total)),
            ],
        );
        let eta = eta_text(self.bytes_done.max(self.files_done), self.bytes_total.max(self.files_total), self.started);
        unsafe {
            (*v).stats = stats;
            (*v).current = self.current.clone();
            (*v).eta = eta;
            (*v).frac = frac;
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn invalidate(&self) {
        unsafe {
            if !self.hwnd.0.is_null() {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }
    }

    fn ask(&mut self, name: &str) -> Option<(u8, bool)> {
        self.pump();
        let choice = Arc::new(AtomicU8::new(0));
        let all = Arc::new(AtomicBool::new(false));
        let hwnd = open_ask(self.hwnd, self.accent, &self.face, name, choice.clone(), all.clone());
        if hwnd.0.is_null() {
            return None;
        }
        unsafe { let _ = EnableWindow(self.hwnd, false); }
        while unsafe { IsWindow(Some(hwnd)).as_bool() } {
            pump_messages();
            if self.cancelled() {
                unsafe { let _ = DestroyWindow(hwnd); }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        }
        unsafe { let _ = EnableWindow(self.hwnd, true); }
        let act = choice.load(Ordering::Relaxed);
        if act == 0 || self.cancelled() {
            None
        } else {
            Some((act, all.load(Ordering::Relaxed)))
        }
    }

    fn show_cancelled(&mut self) {
        if let Some(v) = view(self.hwnd) {
            unsafe {
                (*v).title = tr(Key::FastCopyWasCancelled).to_string();
                (*v).stats = tr(Key::Cancelled).to_string();
                let _ = SetWindowTextW(self.hwnd, &HSTRING::from(&(*v).title));
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }
        let start = Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(700) {
            pump_messages();
            std::thread::sleep(std::time::Duration::from_millis(16));
            if self.hwnd.0.is_null() || unsafe { !IsWindow(Some(self.hwnd)).as_bool() } {
                return;
            }
        }
        self.close();
    }

    fn close(&mut self) {
        unsafe {
            if !self.hwnd.0.is_null() && IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
        self.hwnd = HWND::default();
        pump_messages();
    }
}

struct View {
    hwnd: HWND,
    dpi: i32,
    accent: u32,
    font: HFONT,
    small: HFONT,
    title: String,
    stats: String,
    current: String,
    eta: String,
    frac: f32,
    cancel_rect: RECT,
    file_rect: RECT,
    hot: bool,
    down: bool,
    tip: HWND,
    tip_on: bool,
    cancel: Arc<AtomicBool>,
    mv: bool,
}

fn view(hwnd: HWND) -> Option<*mut View> {
    if hwnd.0.is_null() {
        return None;
    }
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut View;
    if p.is_null() {
        None
    } else {
        Some(p)
    }
}

fn open_progress(owner: HWND, accent: u32, face: &str, mv: bool, cancel: Arc<AtomicBool>) -> HWND {
    ensure_class(w!("MaguroFastCopy"), Some(progress_proc));
    let dpi = dpi_of(owner);
    let geom = progress_geom(dpi);
    let (w, h) = (geom.w, geom.h);
    let (x, y) = center_on(owner, w, h);
    let title = if mv { tr(Key::FastMove) } else { tr(Key::FastCopy) };
    let view = Box::new(View {
        hwnd: HWND::default(),
        dpi,
        accent,
        font: make_font(face, 16, 600, dpi),
        small: make_font(face, 13, 400, dpi),
        title: title.to_string(),
        stats: String::new(),
        current: String::new(),
        eta: String::new(),
        frac: 0.0,
        cancel_rect: RECT::default(),
        file_rect: RECT::default(),
        hot: false,
        down: false,
        tip: HWND::default(),
        tip_on: false,
        cancel,
        mv,
    });
    let ptr = Box::into_raw(view);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("MaguroFastCopy"),
            &HSTRING::from(title),
            WS_POPUP | WS_CLIPCHILDREN | WS_VISIBLE,
            x,
            y,
            w,
            h,
            None,
            None,
            None,
            None,
        )
    };
    let Ok(hwnd) = hwnd else {
        drop(unsafe { Box::from_raw(ptr) });
        return HWND::default();
    };
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        (*ptr).hwnd = hwnd;
        (*ptr).tip = make_tip(hwnd);
        round_window(hwnd);
        raise(hwnd);
    }
    hwnd
}

struct AskView {
    dpi: i32,
    accent: u32,
    font: HFONT,
    small: HFONT,
    name: String,
    all: bool,
    hot: i32,
    down: i32,
    rects: [RECT; 4],
    name_rect: RECT,
    choice: Arc<AtomicU8>,
    all_flag: Arc<AtomicBool>,
    tip: HWND,
    tip_on: bool,
}

fn open_ask(owner: HWND, accent: u32, face: &str, name: &str, choice: Arc<AtomicU8>, all_flag: Arc<AtomicBool>) -> HWND {
    ensure_class(w!("MaguroFastAsk"), Some(ask_proc));
    let dpi = dpi_of(owner);
    let (w, h) = (sc(dpi, 480), sc(dpi, 176));
    let (x, y) = center_on(owner, w, h);
    let view = Box::new(AskView {
        dpi,
        accent,
        font: make_font(face, 16, 600, dpi),
        small: make_font(face, 13, 400, dpi),
        name: name.to_string(),
        all: false,
        hot: -1,
        down: -1,
        rects: [RECT::default(); 4],
        name_rect: RECT::default(),
        choice,
        all_flag,
        tip: HWND::default(),
        tip_on: false,
    });
    let ptr = Box::into_raw(view);
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("MaguroFastAsk"),
            w!("Maguro"),
            WS_POPUP | WS_CLIPCHILDREN | WS_VISIBLE,
            x,
            y,
            w,
            h,
            None,
            None,
            None,
            None,
        )
    };
    let Ok(hwnd) = hwnd else {
        drop(unsafe { Box::from_raw(ptr) });
        return HWND::default();
    };
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        (*ptr).tip = make_tip(hwnd);
        round_window(hwnd);
        raise(hwnd);
    }
    hwnd
}

fn ensure_class(name: PCWSTR, proc: WNDPROC) {
    unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let mut existing = WNDCLASSW::default();
        if GetClassInfoW(Some(hinst), name, &mut existing).is_ok() {
            return;
        }
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DROPSHADOW,
            lpfnWndProc: proc,
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(crate::ICON_GROUP_LIGHT as _)).unwrap_or_default(),
            lpszClassName: name,
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
        let pref: u32 = 2; // DWMWCP_ROUND
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as _, 4);
        let dark = crate::DARK_ON.load(Ordering::Relaxed);
        let on = BOOL::from(dark);
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
        let border = crate::cref(crate::pal().bg);
        let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as _, 4);
    }
}

/// The worker thread does not own the foreground, so attach to it long enough to show the window.
fn raise(hwnd: HWND) {
    unsafe {
        let fore = GetForegroundWindow();
        let tid = GetCurrentThreadId();
        let fore_tid = GetWindowThreadProcessId(fore, None);
        let attached = fore_tid != 0 && fore_tid != tid && AttachThreadInput(fore_tid, tid, true).as_bool();
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(fore_tid, tid, false);
        }
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

fn make_tip(owner: HWND) -> HWND {
    let style = WS_POPUP | WINDOW_STYLE(0x01 | 0x02);
    unsafe { CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, None, style, 0, 0, 0, 0, Some(owner), None, None, None).unwrap_or_default() }
}

fn pump_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn progress_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE | WM_DESTROY => {
            if msg == WM_CLOSE {
                if let Some(v) = view(h) {
                    (*v).cancel.store(true, Ordering::Relaxed);
                }
                return LRESULT(0);
            }
            drop_view(h);
            LRESULT(0)
        }
        WM_PAINT => {
            paint_progress(h);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            hit_progress(h, lp, false, false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            hit_progress(h, lp, true, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            hit_progress(h, lp, false, true);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            if let Some(v) = view(h) {
                (*v).cancel.store(true, Ordering::Relaxed);
            }
            LRESULT(0)
        }
        WM_NCHITTEST => {
            let hit = DefWindowProcW(h, msg, wp, lp);
            if hit.0 == HTCLIENT as isize {
                let (x, y) = lp_point(lp);
                let mut r = RECT::default();
                let _ = GetClientRect(h, &mut r);
                if let Some(v) = view(h) {
                    if y < sc((*v).dpi, 16) + sc((*v).dpi, 16) && !inside((*v).cancel_rect, x, y) {
                        return LRESULT(HTCAPTION as isize);
                    }
                }
            }
            hit
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

unsafe fn paint_progress(h: HWND) {
    let Some(v) = view(h) else {
        let _ = ValidateRect(Some(h), None);
        return;
    };
    let v = &mut *v;
    crate::buffered(h, |dc, rc| {
        let dpi = v.dpi;
        let g = progress_geom(dpi);
        crate::fill(dc, rc, crate::pal().bg);
        let pad = g.pad;
        let title_r = RECT { left: pad, top: g.title_y, right: rc.right - pad, bottom: g.title_y + g.line };
        crate::text(dc, title_r, &v.title, v.font, crate::pal().text, DT_LEFT);
        let stats_r = RECT { left: pad, top: g.stats_y, right: rc.right - pad, bottom: g.stats_y + g.line };
        crate::text(dc, stats_r, &v.stats, v.small, crate::pal().sub, DT_LEFT);
        let bar = RECT { left: pad, top: g.bar_y, right: rc.right - pad, bottom: g.bar_y + g.bar_h };
        let (track, edge) = track_colors(crate::pal().bg, crate::pal().sub);
        crate::aa_rfill(dc, bar, sc(dpi, 4), track, Some(edge), crate::pal().bg);
        let span = (bar.right - bar.left).max(1);
        let fill_w = ((span as f32) * v.frac).round() as i32;
        if fill_w > 0 {
            let fill = RECT { right: (bar.left + fill_w.max(sc(dpi, 8))).min(bar.right), ..bar };
            crate::aa_rfill(dc, fill, sc(dpi, 4), v.accent, None, track);
        }
        v.file_rect = RECT { left: pad, top: g.file_y, right: rc.right - pad, bottom: g.file_y + g.line };
        let avail = (v.file_rect.right - v.file_rect.left).saturating_sub(2).max(0);
        let shown = middle_ellipsis(&v.current, avail, |s| unsafe { measure_dc(dc, v.small, s) });
        crate::text(dc, v.file_rect, &shown, v.small, crate::pal().text, DT_LEFT);
        v.cancel_rect = RECT {
            left: rc.right - pad - g.btn_w,
            top: g.row_y,
            right: rc.right - pad,
            bottom: g.row_y + g.btn_h,
        };
        let eta_r = RECT { left: pad, top: g.row_y, right: v.cancel_rect.left - sc(dpi, 8), bottom: g.row_y + g.btn_h };
        crate::text(dc, eta_r, &v.eta, v.small, crate::pal().sub, DT_LEFT);
        let fill = if v.down && v.hot { crate::blend(crate::pal().card, crate::pal().text, 0.08) } else if v.hot { crate::blend(crate::pal().card, v.accent, 0.12) } else { crate::pal().card };
        crate::aa_rfill(dc, v.cancel_rect, sc(dpi, 8), fill, Some(crate::pal().line), crate::pal().bg);
        crate::text(dc, v.cancel_rect, tr(Key::Cancel), v.small, crate::pal().text, DT_CENTER);
    });
}

fn hit_progress(h: HWND, lp: LPARAM, down: bool, up: bool) {
    let (x, y) = lp_point(lp);
    let Some(v) = view(h) else { return };
    unsafe {
        let on = inside((*v).cancel_rect, x, y);
        (*v).hot = on;
        if down {
            (*v).down = on;
            if on {
                let _ = SetCapture(h);
            }
        }
        if up {
            let fire = (*v).down && on;
            (*v).down = false;
            let _ = ReleaseCapture();
            if fire {
                (*v).cancel.store(true, Ordering::Relaxed);
            }
        }
        track_tip((*v).tip, h, &mut (*v).tip_on, (*v).file_rect, &(*v).current, x, y);
        let _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe extern "system" fn ask_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => {
            finish_ask(h, 3);
            LRESULT(0)
        }
        WM_DESTROY => {
            drop_ask(h);
            LRESULT(0)
        }
        WM_PAINT => {
            paint_ask(h);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            hit_ask(h, lp, false, false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            hit_ask(h, lp, true, false);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            hit_ask(h, lp, false, true);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => {
            finish_ask(h, 3);
            LRESULT(0)
        }
        WM_KEYDOWN if wp.0 == VK_RETURN.0 as usize => {
            finish_ask(h, 1);
            LRESULT(0)
        }
        WM_NCHITTEST => {
            let hit = DefWindowProcW(h, msg, wp, lp);
            if hit.0 == HTCLIENT as isize {
                let (_x, y) = lp_point(lp);
                if let Some(v) = ask_view(h) {
                    if y < sc((*v).dpi, 32) {
                        return LRESULT(HTCAPTION as isize);
                    }
                }
            }
            hit
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

fn ask_view(hwnd: HWND) -> Option<*mut AskView> {
    let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AskView;
    if p.is_null() { None } else { Some(p) }
}

unsafe fn paint_ask(h: HWND) {
    let Some(v) = ask_view(h) else { return };
    let v = &mut *v;
    crate::buffered(h, |dc, rc| {
        let dpi = v.dpi;
        let pad = sc(dpi, 16);
        crate::fill(dc, rc, crate::pal().bg);
        let mut y = pad;
        let title = RECT { left: pad, top: y, right: rc.right - pad, bottom: y + sc(dpi, 16) };
        crate::text(dc, title, tr(Key::FastCopy), v.font, crate::pal().text, DT_LEFT);
        y += sc(dpi, 16) + sc(dpi, 8);
        v.name_rect = RECT { left: pad, top: y, right: rc.right - pad, bottom: y + sc(dpi, 32) };
        let body = tf(Key::NameAlreadyInThisFolder, &[("name", &v.name)]);
        crate::wrap_text(dc, v.name_rect, &body, v.small, crate::pal().text, false);
        y += sc(dpi, 32) + sc(dpi, 8);
        let tick = sc(dpi, 16);
        v.rects[3] = RECT { left: pad, top: y, right: rc.right - pad, bottom: y + tick };
        let mark = RECT { left: pad, top: y, right: pad + tick, bottom: y + tick };
        let fill = if v.all { v.accent } else { crate::pal().card };
        crate::aa_rfill(dc, mark, sc(dpi, 4), fill, Some(if v.all { v.accent } else { crate::pal().line }), crate::pal().bg);
        if v.all {
            crate::text(dc, mark, "✓", v.small, 0xFFFFFF, DT_CENTER);
        }
        let label = RECT { left: mark.right + sc(dpi, 8), top: y, right: rc.right - pad, bottom: y + tick };
        crate::text(dc, label, tr(Key::ApplyToAll), v.small, crate::pal().text, DT_LEFT);
        let bw = (rc.right - pad * 2 - sc(dpi, 8) * 2) / 3;
        let top = rc.bottom - pad - sc(dpi, 32);
        let labels = [tr(Key::Overwrite), tr(Key::Skip), tr(Key::Cancel)];
        for i in 0..3 {
            let left = pad + i as i32 * (bw + sc(dpi, 8));
            v.rects[i] = RECT { left, top, right: left + bw, bottom: top + sc(dpi, 32) };
            let on = v.hot == i as i32;
            let fill = if i == 0 { v.accent } else if on { crate::blend(crate::pal().card, crate::pal().text, 0.06) } else { crate::pal().card };
            let border = if i == 0 { None } else { Some(crate::pal().line) };
            crate::aa_rfill(dc, v.rects[i], sc(dpi, 8), fill, border, crate::pal().bg);
            let ink = if i == 0 { crate::on_color(v.accent) } else { crate::pal().text };
            crate::text(dc, v.rects[i], labels[i], v.small, ink, DT_CENTER);
        }
    });
}

fn hit_ask(h: HWND, lp: LPARAM, down: bool, up: bool) {
    let (x, y) = lp_point(lp);
    let Some(v) = ask_view(h) else { return };
    unsafe {
        let hot = (0..4).find(|&i| inside((*v).rects[i], x, y)).map(|i| i as i32).unwrap_or(-1);
        (*v).hot = hot;
        if down {
            (*v).down = hot;
            if hot >= 0 {
                let _ = SetCapture(h);
            }
        }
        if up {
            let fire = (*v).down;
            (*v).down = -1;
            let _ = ReleaseCapture();
            if fire == hot && hot == 3 {
                (*v).all = !(*v).all;
            } else if fire == hot && (0..=2).contains(&hot) {
                // rect 0 overwrite, 1 skip, 2 cancel → act 1, 2, 3
                finish_ask(h, (hot + 1) as u8);
                return;
            }
        }
        let body = tf(Key::NameAlreadyInThisFolder, &[("name", &(*v).name)]);
        track_tip((*v).tip, h, &mut (*v).tip_on, (*v).name_rect, &body, x, y);
        let _ = InvalidateRect(Some(h), None, false);
    }
}

fn finish_ask(h: HWND, act: u8) {
    if let Some(v) = ask_view(h) {
        unsafe {
            (*v).choice.store(act, Ordering::Relaxed);
            (*v).all_flag.store((*v).all, Ordering::Relaxed);
            let _ = DestroyWindow(h);
        }
    }
}

unsafe fn drop_view(h: HWND) {
    let p = GetWindowLongPtrW(h, GWLP_USERDATA) as *mut View;
    if p.is_null() {
        return;
    }
    SetWindowLongPtrW(h, GWLP_USERDATA, 0);
    let v = Box::from_raw(p);
    if !v.font.0.is_null() {
        let _ = DeleteObject(v.font.into());
    }
    if !v.small.0.is_null() {
        let _ = DeleteObject(v.small.into());
    }
}

unsafe fn drop_ask(h: HWND) {
    let p = GetWindowLongPtrW(h, GWLP_USERDATA) as *mut AskView;
    if p.is_null() {
        return;
    }
    SetWindowLongPtrW(h, GWLP_USERDATA, 0);
    let v = Box::from_raw(p);
    if !v.font.0.is_null() {
        let _ = DeleteObject(v.font.into());
    }
    if !v.small.0.is_null() {
        let _ = DeleteObject(v.small.into());
    }
}

fn inside(r: RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn lp_point(lp: LPARAM) -> (i32, i32) {
    let x = (lp.0 as i16) as i32;
    let y = ((lp.0 >> 16) as i16) as i32;
    (x, y)
}

unsafe fn track_tip(tip: HWND, owner: HWND, on: &mut bool, item: RECT, text: &str, x: i32, y: i32) {
    if tip.0.is_null() || text.is_empty() || !inside(item, x, y) {
        hide_tip(tip, owner, on);
        return;
    }
    let wide = text_px(owner, text) > (item.right - item.left);
    if !wide {
        hide_tip(tip, owner, on);
        return;
    }
    let label = HSTRING::from(text);
    let info = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: 1,
        lpszText: PWSTR(label.as_ptr() as *mut u16),
        ..Default::default()
    };
    let lp = LPARAM(&info as *const _ as isize);
    if !*on {
        SendMessageW(tip, TTM_ADDTOOLW, None, Some(lp));
        *on = true;
    }
    SendMessageW(tip, TTM_UPDATETIPTEXTW, None, Some(lp));
    let mut pt = POINT { x: item.left, y: item.bottom + 4 };
    let _ = ClientToScreen(owner, &mut pt);
    SendMessageW(tip, TTM_TRACKPOSITION, None, Some(LPARAM(((pt.y as u16 as u32) << 16 | (pt.x as u16 as u32)) as isize)));
    SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(1)), Some(lp));
}

unsafe fn hide_tip(tip: HWND, owner: HWND, on: &mut bool) {
    if !*on || tip.0.is_null() {
        return;
    }
    let label = HSTRING::from("");
    let info = TTTOOLINFOW {
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: 1,
        lpszText: PWSTR(label.as_ptr() as *mut u16),
        ..Default::default()
    };
    let lp = LPARAM(&info as *const _ as isize);
    SendMessageW(tip, TTM_TRACKACTIVATE, Some(WPARAM(0)), Some(lp));
    *on = false;
}

unsafe fn text_px(owner: HWND, text: &str) -> i32 {
    let mut buf = [0u16; 64];
    let n = GetClassNameW(owner, &mut buf) as usize;
    let class = String::from_utf16_lossy(&buf[..n]);
    if class == "MaguroFastAsk" {
        if let Some(a) = ask_view(owner) {
            return measure(owner, text, (*a).small);
        }
        return i32::MAX;
    }
    let Some(v) = view(owner) else { return i32::MAX };
    measure(owner, text, (*v).small)
}

unsafe fn measure(owner: HWND, text: &str, font: HFONT) -> i32 {
    let dc = GetDC(Some(owner));
    if dc.is_invalid() {
        return i32::MAX;
    }
    let w = measure_dc(dc, font, text);
    let _ = ReleaseDC(Some(owner), dc);
    w
}

unsafe fn measure_dc(dc: HDC, font: HFONT, text: &str) -> i32 {
    if text.is_empty() || dc.is_invalid() {
        return 0;
    }
    let mut r = RECT { left: 0, top: 0, right: 1, bottom: 1 };
    let mut w: Vec<u16> = text.encode_utf16().collect();
    let old = SelectObject(dc, font.into());
    DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
    (r.right - r.left).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_only_when_forced_or_network_setting() {
        assert!(!decide_fast(false, false, false));
        assert!(!decide_fast(false, false, true));
        assert!(!decide_fast(false, true, false));
        assert!(decide_fast(false, true, true));
        assert!(decide_fast(true, false, false));
        assert!(decide_fast(true, true, false));
    }

    #[test]
    fn network_class_uses_unc_or_remote_drive() {
        assert!(classify_network(r"\\server\share\a", None));
        assert!(classify_network(r"\\?\UNC\server\share\a", Some(3)));
        assert!(!classify_network(r"C:\work\a", Some(3)));
        assert!(classify_network(r"Z:\docs", Some(4)));
        assert!(!classify_network(r"\\?\C:\work", Some(3)));
        assert!(network_transfer(&[r"\\server\share\a".into()], r"C:\dst"));
        assert!(classify_network_roots(r"Z:\pgtest\marker.txt", Some(3), &[r"Z:\".into()]));
        assert!(classify_network_roots(r"\\localhost\C$\WORK\pgtest\data\a.txt", None, &[]));
        assert!(!classify_network_roots(r"C:\WORK\pgtest\data\a.txt", Some(3), &[r"Z:\".into()]));
    }

    #[test]
    fn robocopy_args_copy_trees_and_never_delete() {
        let files = robocopy_args(r"C:\src", r"D:\dst", &["a.txt".into(), "b.txt".into()], false, "fast.log");
        assert!(files.contains(&"/MT:8".into()));
        assert!(files.contains(&"/R:1".into()));
        assert!(files.contains(&"/W:1".into()));
        assert!(files.contains(&"/BYTES".into()));
        assert!(files.contains(&"/UNILOG:fast.log".into()));
        assert!(!files.iter().any(|a| a == "/E"));
        assert_eq!(files[2], "a.txt");
        assert!(!args_forbidden(&files), "{files:?}");

        let tree = robocopy_args(r"C:\src\dir\", r"D:\dst\dir\", &[], true, "fast.log");
        assert!(tree.contains(&"/E".into()));
        assert_eq!(tree[0], r"C:\src\dir");
        assert!(!args_forbidden(&tree), "{tree:?}");
        assert!(!tree.iter().any(|a| a.to_ascii_uppercase().contains("MIR") || a.to_ascii_uppercase().contains("PURGE")));
    }

    #[test]
    fn exit_codes_below_eight_are_success() {
        for c in 0..8 {
            assert!(exit_ok(c));
        }
        assert!(!exit_ok(8));
        assert!(!exit_ok(9));
        assert!(!exit_ok(16));
    }

    #[test]
    fn copy_names_follow_windows() {
        assert_eq!(copy_name("notes.txt", 1), "notes - Copy.txt");
        assert_eq!(copy_name("notes.txt", 2), "notes - Copy (2).txt");
        assert_eq!(copy_name("folder", 1), "folder - Copy");
        assert_eq!(copy_name("a.tar.gz", 1), "a.tar - Copy.gz");
    }

    #[test]
    fn remain_time_uses_localized_units() {
        assert_eq!(remain_time(1), "1 second");
        assert_eq!(remain_time(2), "2 seconds");
        assert_eq!(remain_time(60), "1 minute");
        assert_eq!(remain_time(90), "1 minute 30 seconds");
        assert_eq!(remain_time(61), "1 minute 1 second");
        assert_eq!(remain_time(121), "2 minutes 1 second");
        assert_eq!(remain_time(150), "2 minutes 30 seconds");
        assert_eq!(tf(Key::AboutTimeLeft, &[("time", &remain_time(1))]), "About 1 second left");

        crate::UI.store(1, Ordering::Relaxed);
        assert_eq!(remain_time(1), "1초");
        assert_eq!(remain_time(150), "2분 30초");
        assert_eq!(tf(Key::AboutTimeLeft, &[("time", &remain_time(1))]), "약 1초 남음");

        crate::UI.store(2, Ordering::Relaxed);
        assert_eq!(remain_time(1), "1秒");
        assert_eq!(remain_time(150), "2分30秒");
        assert_eq!(tf(Key::AboutTimeLeft, &[("time", &remain_time(150))]), "残り約 2分30秒");
        crate::UI.store(0, Ordering::Relaxed);

        let fr = include_str!("../../lang/lang.fr.ini");
        let de = include_str!("../../lang/lang.de.ini");
        assert!(fr.contains("time_second = {n} seconde"));
        assert!(fr.contains("time_seconds = {n} secondes"));
        assert!(fr.contains("time_minutes_seconds = {m} minutes {s} secondes"));
        assert!(de.contains("time_second = {n} Sekunde"));
        assert!(de.contains("time_minutes_seconds = {m} Minuten {s} Sekunden"));
    }

    #[test]
    fn middle_ellipsis_keeps_both_ends() {
        let text = r"\\localhost\C$\WORK\pgtest\data\fastcopy\very-long-folder-name\file.txt";
        let width = |s: &str| s.chars().count() as i32;
        let out = middle_ellipsis(text, 24, width);
        assert!(out.contains('…'), "{out}");
        assert!(out.starts_with(r"\\local"), "{out}");
        assert!(out.ends_with("file.txt"), "{out}");
        assert!(width(&out) <= 24, "{out}");
        assert_eq!(middle_ellipsis("short", 24, width), "short");
        assert_eq!(middle_ellipsis("abc", 0, width), "");
    }

    #[test]
    fn progress_window_packs_cancel_on_the_eta_row() {
        let g = progress_geom(96);
        assert_eq!((g.w, g.h), (480, 152));
        assert!(g.h < 184);
        assert_eq!(g.row_y + g.btn_h + g.pad, g.h);
        assert_eq!(g.file_y + g.line + 8, g.row_y);
        let (fill, edge) = track_colors(0xF3F1EC, 0x7C776F);
        let dist = |a: u32, b: u32| {
            let ch = |s: u32| (a >> s & 0xFF).abs_diff(b >> s & 0xFF) as u32;
            ch(16) + ch(8) + ch(0)
        };
        assert!(dist(fill, 0xF3F1EC) > 60, "{fill:06X}");
        assert_ne!(fill, edge);
        let (dark, dark_edge) = track_colors(0x1C1D20, 0x9AA0A8);
        assert!(dist(dark, 0x1C1D20) > 40, "{dark:06X}");
        assert_ne!(dark, dark_edge);
    }

    #[test]
    fn log_lines_pick_the_file_and_errors() {
        let line = "\t    New File  \t\t       4\tC:\\WORK\\pgtest\\data\\src\\hello.txt";
        let (path, bytes) = parse_log_line(line).unwrap();
        assert_eq!(bytes, 4);
        assert!(path.ends_with("hello.txt"));
        assert!(parse_log_line("   Source : C:\\src\\").is_none());
        assert!(parse_error_line("2026/10/30 12:00:00 ERROR 32 (0x00000020) Copying File C:\\a.txt").is_some());
        assert!(parse_error_line("   Bytes :         4         4         0").is_none());
    }
}
