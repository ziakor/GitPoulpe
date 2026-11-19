use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;

use crate::Git;
use crate::cancel::Cancel;
use crate::process::RunError;

const MAX_STATUS_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepoOverview {
    pub branch: Option<String>,
    pub head_oid: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
    pub staged: u64,
    pub unstaged: u64,
    pub untracked: u64,
    pub conflicts: u64,
    pub operation: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverviewError {
    Missing,
    NotARepository,
    Cancelled,
    Git(String),
}

impl std::fmt::Display for OverviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("dossier introuvable"),
            Self::NotARepository => f.write_str("ce dossier n'est pas un dépôt Git"),
            Self::Cancelled => f.write_str("lecture annulée"),
            Self::Git(why) => write!(f, "Git a échoué : {why}"),
        }
    }
}

impl std::error::Error for OverviewError {}

pub fn parse_overview(bytes: &[u8]) -> RepoOverview {
    let mut overview = RepoOverview::default();
    let mut fields = bytes.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(field) = fields.next() {
        let text = String::from_utf8_lossy(field);
        if let Some(header) = text.strip_prefix("# ") {
            let (name, value) = header.split_once(' ').unwrap_or((header, ""));
            match name {
                "branch.oid" if value != "(initial)" => overview.head_oid = Some(value.to_owned()),
                "branch.head" if value != "(detached)" => overview.branch = Some(value.to_owned()),
                "branch.upstream" => overview.upstream = Some(value.to_owned()),
                "branch.ab" => {
                    let mut parts = value.split_whitespace();
                    overview.ahead = parts
                        .next()
                        .and_then(|a| a.trim_start_matches('+').parse().ok());
                    overview.behind = parts
                        .next()
                        .and_then(|b| b.trim_start_matches('-').parse().ok());
                }
                _ => {}
            }
            continue;
        }
        let mut chars = text.chars();
        match (chars.next(), chars.nth(1), chars.next()) {
            (Some('1' | '2'), Some(index), Some(worktree)) => {
                if index != '.' {
                    overview.staged += 1;
                }
                if worktree != '.' {
                    overview.unstaged += 1;
                }
                if text.starts_with('2') {
                    fields.next();
                }
            }
            (Some('u'), _, _) => overview.conflicts += 1,
            (Some('?'), _, _) => overview.untracked += 1,
            _ => {}
        }
    }
    overview
}

fn operation_in(git_dir: &Path) -> Option<&'static str> {
    [
        ("MERGE_HEAD", "fusion en cours"),
        ("rebase-merge", "rebase en cours"),
        ("rebase-apply", "rebase en cours"),
        ("CHERRY_PICK_HEAD", "cherry-pick en cours"),
        ("REVERT_HEAD", "annulation (revert) en cours"),
        ("BISECT_LOG", "bisect en cours"),
    ]
    .into_iter()
    .find(|(name, _)| git_dir.join(name).exists())
    .map(|(_, label)| label)
}

impl Git {
    pub async fn repo_overview(
        &self,
        dir: &Path,
        cancel: &Cancel,
    ) -> Result<RepoOverview, OverviewError> {
        if !dir.is_dir() {
            return Err(OverviewError::Missing);
        }
        let env: Vec<(OsString, OsString)> =
            self.filter_overrides(dir, cancel)
                .await
                .map_err(|e| match e {
                    gitpoulpe_core::ReadError::GitFailed(d)
                        if d.message.contains("not a git repository") =>
                    {
                        OverviewError::NotARepository
                    }
                    other => OverviewError::Git(other.to_string()),
                })?;
        let args = [
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=normal",
            "--ignore-submodules=all",
        ]
        .map(OsString::from);
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut out = Vec::new();
        let run = self
            .run_env(dir, &refs, &env, cancel, |chunk| {
                if out.len() + chunk.len() > MAX_STATUS_BYTES {
                    return Err("status output is too large".to_owned());
                }
                out.extend_from_slice(chunk);
                Ok(())
            })
            .await
            .map_err(|e| match e {
                RunError::Cancelled => OverviewError::Cancelled,
                RunError::Spawn(e) | RunError::Io(e) => OverviewError::Git(e.to_string()),
                RunError::Rejected(message) => OverviewError::Git(message),
            })?;
        if !run.succeeded() {
            let message = run.diagnostic().message;
            return Err(if message.contains("not a git repository") {
                OverviewError::NotARepository
            } else {
                OverviewError::Git(message)
            });
        }
        let mut overview = parse_overview(&out);
        let dir_args = ["rev-parse", "--absolute-git-dir"].map(OsString::from);
        let dir_refs: Vec<&OsStr> = dir_args.iter().map(OsString::as_os_str).collect();
        if let Ok((probe, bytes)) = self.run_collect(dir, &dir_refs, cancel).await
            && probe.succeeded()
        {
            let git_dir = String::from_utf8_lossy(&bytes).trim().to_owned();
            overview.operation = operation_in(Path::new(&git_dir));
        }
        Ok(overview)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundedError {
    Cancelled,
    TimedOut,
}

#[derive(Clone)]
pub struct Bounded {
    permits: Arc<Semaphore>,
    timeout: Duration,
}

impl Bounded {
    pub fn with_shared(shared: &Self, timeout: Duration) -> Self {
        Self {
            permits: shared.permits.clone(),
            timeout,
        }
    }

    pub fn new(limit: usize, timeout: Duration) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(limit.max(1))),
            timeout,
        }
    }

    pub async fn run<T, Fut>(
        &self,
        cancel: &Cancel,
        work: impl FnOnce(Cancel) -> Fut,
    ) -> Result<T, BoundedError>
    where
        Fut: Future<Output = T>,
    {
        let permit = tokio::select! {
            permit = self.permits.clone().acquire_owned() => permit.map_err(|_| BoundedError::Cancelled)?,
            () = cancel.cancelled() => return Err(BoundedError::Cancelled),
        };
        let outcome = tokio::select! {
            result = tokio::time::timeout(self.timeout, work(cancel.clone())) => match result {
                Ok(value) => Ok(value),
                Err(_) => {
                    cancel.cancel();
                    Err(BoundedError::TimedOut)
                }
            },
            () = cancel.cancelled() => Err(BoundedError::Cancelled),
        };
        drop(permit);
        outcome
    }
}

pub fn remote_urls(dir: &Path) -> Vec<String> {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["config", "--get-regexp", r"^remote\..*\.url$"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_once(' ').map(|(_, url)| url.trim().to_owned()))
        .collect()
}

fn quiet_git(dir: &Path) -> std::process::Command {
    let mut command = std::process::Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null());
    command
}

pub fn local_branches(dir: &Path) -> Vec<String> {
    let Ok(output) = quiet_git(dir)
        .args(["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

pub fn commit_exists(dir: &Path, sha: &str) -> bool {
    if !(4..=64).contains(&sha.len()) || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    quiet_git(dir)
        .args(["cat-file", "-e", &format!("{sha}^{{commit}}")])
        .output()
        .is_ok_and(|o| o.status.success())
}
