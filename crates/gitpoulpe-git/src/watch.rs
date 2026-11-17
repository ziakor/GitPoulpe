use std::collections::HashSet;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use gitpoulpe_core::SessionInfo;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub const QUIET_PERIOD: Duration = Duration::from_millis(250);
pub const MAX_WAIT: Duration = Duration::from_secs(1);

const GIT_FILES: [&str; 6] = [
    "index",
    "HEAD",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REBASE_HEAD",
    "packed-refs",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchError(pub String);

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WatchError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Changed,
    EventsLost,
}

struct Shared {
    watcher: Mutex<RecommendedWatcher>,
    watched: Mutex<HashSet<PathBuf>>,
}

impl Shared {
    fn watch_dir(&self, dir: &Path) -> notify::Result<()> {
        if !self.watched.lock().unwrap().insert(dir.to_path_buf()) {
            return Ok(());
        }
        self.watcher
            .lock()
            .unwrap()
            .watch(dir, RecursiveMode::NonRecursive)
    }
}

#[derive(Default)]
struct Ignored {
    paths: HashSet<PathBuf>,
}

impl Ignored {
    fn covers(&self, path: &Path) -> bool {
        path.ancestors()
            .any(|ancestor| self.paths.contains(ancestor))
    }
}

pub struct RepoWatcher {
    shared: Arc<Shared>,
}

impl RepoWatcher {
    pub fn watched_directories(&self) -> usize {
        self.shared.watched.lock().unwrap().len()
    }
}

fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(not(unix))]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
}

fn git_in(worktree: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(worktree)
        .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
        .stderr(Stdio::null());
    command
}

fn ignored_paths(worktree: &Path) -> Ignored {
    let output = git_in(worktree)
        .args([
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ])
        .output();
    let Ok(output) = output else {
        return Ignored::default();
    };
    if !output.status.success() {
        return Ignored::default();
    }
    let paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let entry = entry.strip_suffix(b"/").unwrap_or(entry);
            worktree.join(path_from_bytes(entry))
        })
        .collect();
    Ignored { paths }
}

fn ignored_among(worktree: &Path, paths: &[PathBuf]) -> HashSet<PathBuf> {
    let mut input = Vec::new();
    let mut asked = Vec::new();
    for path in paths {
        let relative = path.strip_prefix(worktree).unwrap_or(path);
        let mut text = path_bytes(relative);
        if path.is_dir() {
            text.push(b'/');
        }
        asked.push((text.clone(), path.clone()));
        input.extend(text);
        input.push(0);
    }
    let Ok(mut child) = git_in(worktree)
        .args(["check-ignore", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
    else {
        return HashSet::new();
    };
    let Some(mut stdin) = child.stdin.take() else {
        return HashSet::new();
    };
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let output = child.wait_with_output();
    let _ = writer.join();
    let Ok(output) = output else {
        return HashSet::new();
    };
    let printed: HashSet<&[u8]> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .collect();
    asked
        .into_iter()
        .filter(|(text, _)| printed.contains(text.as_slice()))
        .map(|(_, path)| path)
        .collect()
}

fn is_real_directory(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_dir())
}

fn is_nested_repository(dir: &Path, worktree: &Path) -> bool {
    dir != worktree && dir.join(".git").exists()
}

fn add_tree(
    shared: &Shared,
    worktree: &Path,
    start: &Path,
    ignored: &Ignored,
    dynamic: bool,
) -> Result<(), WatchError> {
    let mut level = vec![start.to_path_buf()];
    while !level.is_empty() {
        if dynamic {
            let skip = ignored_among(worktree, &level);
            level.retain(|dir| !skip.contains(dir));
        }
        level.retain(|dir| !ignored.covers(dir) && !is_nested_repository(dir, worktree));
        let mut next = Vec::new();
        for dir in &level {
            if dynamic {
                shared.watched.lock().unwrap().remove(dir);
            }
            match shared.watch_dir(dir) {
                Ok(()) => {}
                Err(error) if !dynamic && dir == worktree => {
                    return Err(WatchError(format!("{}: {error}", dir.display())));
                }
                Err(error) if matches!(error.kind, notify::ErrorKind::MaxFilesWatch) => {
                    return Err(WatchError(format!("{}: {error}", dir.display())));
                }
                Err(_) => continue,
            }
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_name() != OsStr::new(".git")
                    && entry.file_type().is_ok_and(|kind| kind.is_dir())
                {
                    next.push(entry.path());
                }
            }
        }
        level = next;
    }
    Ok(())
}

fn inside_git_directory(path: &Path, info: &SessionInfo) -> bool {
    path.starts_with(&info.git_dir) || path.starts_with(&info.common_dir)
}

pub fn is_relevant(path: &Path, info: &SessionInfo) -> bool {
    relevant(path, info, &Ignored::default())
}

fn relevant(path: &Path, info: &SessionInfo, ignored: &Ignored) -> bool {
    if inside_git_directory(path, info) {
        let name = path.file_name().unwrap_or(OsStr::new(""));
        if name.to_string_lossy().ends_with(".lock") {
            return false;
        }
        let directories = [&info.git_dir, &info.common_dir];
        let in_refs = directories
            .iter()
            .any(|dir| path.starts_with(dir.join("refs")));
        let at_root = path
            .parent()
            .is_some_and(|parent| directories.iter().any(|dir| parent == dir.as_path()));
        return in_refs || (at_root && GIT_FILES.iter().any(|file| name == OsStr::new(file)));
    }
    info.worktree
        .as_deref()
        .is_some_and(|worktree| path.starts_with(worktree) && !ignored.covers(path))
}

impl RepoWatcher {
    pub fn start(info: &SessionInfo, raw: UnboundedSender<Signal>) -> Result<Self, WatchError> {
        let ignored = Arc::new(RwLock::new(match &info.worktree {
            Some(worktree) => ignored_paths(worktree),
            None => Ignored::default(),
        }));
        let (candidates, candidate_receiver) = std::sync::mpsc::channel::<Vec<PathBuf>>();
        let callback_info = info.clone();
        let callback_ignored = ignored.clone();
        let callback_raw = raw.clone();
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| match result {
                Ok(event) => {
                    if event.need_rescan() {
                        let _ = callback_raw.send(Signal::EventsLost);
                        return;
                    }
                    if matches!(event.kind, EventKind::Access(_)) {
                        return;
                    }
                    let mut unknown = Vec::new();
                    for path in &event.paths {
                        if inside_git_directory(path, &callback_info) {
                            if relevant(path, &callback_info, &Ignored::default()) {
                                let _ = callback_raw.send(Signal::Changed);
                            }
                        } else if callback_info
                            .worktree
                            .as_deref()
                            .is_some_and(|worktree| path.starts_with(worktree))
                            && !callback_ignored.read().unwrap().covers(path)
                        {
                            unknown.push(path.clone());
                        }
                    }
                    if !unknown.is_empty() {
                        let _ = candidates.send(unknown);
                    }
                }
                Err(_) => {
                    let _ = callback_raw.send(Signal::EventsLost);
                }
            },
            Config::default(),
        )
        .map_err(|e| WatchError(e.to_string()))?;
        let fail = |path: &Path, e: notify::Error| WatchError(format!("{}: {e}", path.display()));
        for dir in [&info.git_dir, &info.common_dir] {
            watcher
                .watch(dir, RecursiveMode::NonRecursive)
                .map_err(|e| fail(dir, e))?;
            let refs = dir.join("refs");
            if refs.is_dir() {
                watcher
                    .watch(&refs, RecursiveMode::Recursive)
                    .map_err(|e| fail(&refs, e))?;
            }
        }
        let shared = Arc::new(Shared {
            watcher: Mutex::new(watcher),
            watched: Mutex::new(HashSet::new()),
        });
        if let Some(worktree) = &info.worktree {
            add_tree(&shared, worktree, worktree, &ignored.read().unwrap(), false)?;
            let weak: Weak<Shared> = Arc::downgrade(&shared);
            let worktree = worktree.clone();
            std::thread::spawn(move || {
                while let Ok(mut batch) = candidate_receiver.recv() {
                    while let Ok(more) = candidate_receiver.try_recv() {
                        batch.extend(more);
                    }
                    batch.sort();
                    batch.dedup();
                    let Some(shared) = weak.upgrade() else { return };
                    let newly_ignored = ignored_among(&worktree, &batch);
                    ignored
                        .write()
                        .unwrap()
                        .paths
                        .extend(newly_ignored.iter().cloned());
                    let live: Vec<PathBuf> = batch
                        .into_iter()
                        .filter(|path| {
                            !newly_ignored.contains(path) && !ignored.read().unwrap().covers(path)
                        })
                        .collect();
                    if live.is_empty() {
                        continue;
                    }
                    for dir in live.iter().filter(|path| is_real_directory(path)) {
                        let _ = add_tree(&shared, &worktree, dir, &ignored.read().unwrap(), true);
                    }
                    let _ = raw.send(Signal::Changed);
                }
            });
        }
        Ok(Self { shared })
    }
}

pub async fn coalesce(
    mut raw: UnboundedReceiver<Signal>,
    out: UnboundedSender<Signal>,
    quiet: Duration,
    max_wait: Duration,
) {
    while let Some(first) = raw.recv().await {
        let started = Instant::now();
        let mut lost = first == Signal::EventsLost;
        loop {
            let remaining = max_wait.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(quiet.min(remaining), raw.recv()).await {
                Ok(Some(signal)) => lost |= signal == Signal::EventsLost,
                Ok(None) => {
                    let _ = out.send(if lost {
                        Signal::EventsLost
                    } else {
                        Signal::Changed
                    });
                    return;
                }
                Err(_) => break,
            }
        }
        if out
            .send(if lost {
                Signal::EventsLost
            } else {
                Signal::Changed
            })
            .is_err()
        {
            return;
        }
    }
}
