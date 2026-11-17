use std::ffi::OsStr;
use std::path::Path;
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

pub struct RepoWatcher {
    _watcher: RecommendedWatcher,
}

fn inside_git_directory(path: &Path, info: &SessionInfo) -> bool {
    path.starts_with(&info.git_dir) || path.starts_with(&info.common_dir)
}

pub fn is_relevant(path: &Path, info: &SessionInfo) -> bool {
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
        .is_some_and(|worktree| path.starts_with(worktree))
}

fn classify(event: &Event, info: &SessionInfo) -> Option<Signal> {
    if event.need_rescan() {
        return Some(Signal::EventsLost);
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return None;
    }
    event
        .paths
        .iter()
        .any(|path| is_relevant(path, info))
        .then_some(Signal::Changed)
}

impl RepoWatcher {
    pub fn start(info: &SessionInfo, raw: UnboundedSender<Signal>) -> Result<Self, WatchError> {
        let filter_info = info.clone();
        let mut watcher = RecommendedWatcher::new(
            move |result: notify::Result<Event>| match result {
                Ok(event) => {
                    if let Some(signal) = classify(&event, &filter_info) {
                        let _ = raw.send(signal);
                    }
                }
                Err(_) => {
                    let _ = raw.send(Signal::EventsLost);
                }
            },
            Config::default(),
        )
        .map_err(|e| WatchError(e.to_string()))?;
        let fail = |path: &Path, e: notify::Error| WatchError(format!("{}: {e}", path.display()));
        if let Some(worktree) = &info.worktree {
            watcher
                .watch(worktree, RecursiveMode::Recursive)
                .map_err(|e| fail(worktree, e))?;
        }
        let covered = |dir: &Path| info.worktree.as_deref().is_some_and(|w| dir.starts_with(w));
        for dir in [&info.git_dir, &info.common_dir] {
            if !covered(dir) {
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
        }
        Ok(Self { _watcher: watcher })
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
