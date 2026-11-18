use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use gitpoulpe_core::{SessionInfo, Worktree, WorktreeRemoval, WorktreeRemoved};

use crate::exec::{ExecError, ExecOut};
use crate::{Cancel, Git};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorktreeError {
    InvalidPath,
    PathExists,
    ParentMissing,
    InsideCurrent,
    InvalidBranch,
    BranchExists,
    NoSuchBranch,
    BranchInUse(PathBuf),
    NoSuchWorktree,
    IsMain,
    IsCurrent,
    Locked(String),
    NeedsDecision(Box<WorktreeRemoval>),
    NotLocked,
    AlreadyLocked,
    Refused(String),
    Exec(ExecError),
}

impl std::fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath => f.write_str("chemin invalide"),
            Self::PathExists => {
                f.write_str("ce chemin existe déjà : choisissez un dossier qui n'existe pas encore")
            }
            Self::ParentMissing => f.write_str("le dossier parent n'existe pas"),
            Self::InsideCurrent => {
                f.write_str("le nouveau worktree ne peut pas être créé à l'intérieur de celui-ci")
            }
            Self::InvalidBranch => f.write_str("nom de branche invalide pour Git"),
            Self::BranchExists => {
                f.write_str("cette branche existe déjà : choisissez-la comme branche existante")
            }
            Self::NoSuchBranch => f.write_str("cette branche n'existe pas"),
            Self::BranchInUse(path) => write!(
                f,
                "cette branche est déjà extraite dans {} : une branche ne peut l'être que dans un worktree à la fois",
                path.display()
            ),
            Self::NoSuchWorktree => f.write_str("ce worktree n'existe pas ou n'est plus listé"),
            Self::IsMain => f.write_str("le worktree principal ne se supprime pas"),
            Self::IsCurrent => f.write_str("le worktree ouvert ne se supprime pas depuis lui-même"),
            Self::Locked(reason) => write!(
                f,
                "ce worktree est verrouillé{} : déverrouillez-le d'abord",
                if reason.is_empty() {
                    String::new()
                } else {
                    format!(" ({reason})")
                }
            ),
            Self::NeedsDecision(r) => write!(
                f,
                "ce worktree contient {} fichier(s) modifié(s) et {} non suivi(s) : une décision explicite est requise",
                r.changed.len(),
                r.untracked.len()
            ),
            Self::NotLocked => f.write_str("ce worktree n'est pas verrouillé"),
            Self::AlreadyLocked => f.write_str("ce worktree est déjà verrouillé"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for WorktreeError {}

impl From<ExecError> for WorktreeError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

pub enum BranchChoice<'a> {
    Existing(&'a str),
    New {
        name: &'a str,
        start: Option<&'a str>,
    },
}

fn parse_status(out: &ExecOut) -> (Vec<PathBuf>, Vec<PathBuf>, usize, usize) {
    let (mut changed, mut untracked) = (Vec::new(), Vec::new());
    let (mut staged, mut unstaged) = (0, 0);
    let mut records = out.stdout.split(|b| *b == 0).filter(|r| r.len() > 3);
    while let Some(record) = records.next() {
        let text = String::from_utf8_lossy(record).into_owned();
        let (code, path) = text.split_at(2);
        let path = PathBuf::from(path.trim_start());
        if code == "??" {
            untracked.push(path);
            continue;
        }
        if code.starts_with('R') || code.starts_with('C') {
            records.next();
        }
        let bytes = code.as_bytes();
        if bytes[0] != b' ' {
            staged += 1;
        }
        if bytes[1] != b' ' {
            unstaged += 1;
        }
        changed.push(path);
    }
    (changed, untracked, staged, unstaged)
}

impl Git {
    pub(crate) async fn in_dir(&self, dir: &Path, args: &[&str]) -> Result<ExecOut, WorktreeError> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut stdout = Vec::new();
        let run = self
            .run_input(dir, &refs, &[], None, &Cancel::new(), |chunk| {
                stdout.extend_from_slice(chunk);
                Ok(())
            })
            .await
            .map_err(|e| match e {
                crate::process::RunError::Spawn(e) | crate::process::RunError::Io(e) => {
                    WorktreeError::Refused(e.to_string())
                }
                crate::process::RunError::Cancelled => {
                    WorktreeError::Refused("cancelled".to_owned())
                }
                crate::process::RunError::Rejected(m) => WorktreeError::Refused(m),
            })?;
        Ok(ExecOut {
            code: run.code,
            stdout,
            stderr: run.diagnostic().message,
        })
    }

    pub async fn list_worktrees(&self, info: &SessionInfo) -> Result<Vec<Worktree>, WorktreeError> {
        let out = self
            .exec_ok(info, &["worktree", "list", "--porcelain", "-z"])
            .await?;
        let mut list: Vec<Worktree> = Vec::new();
        let mut current: Option<Worktree> = None;
        let here = info
            .worktree
            .as_deref()
            .and_then(|p| std::fs::canonicalize(p).ok());
        for field in out.stdout.split(|b| *b == 0) {
            let text = String::from_utf8_lossy(field).into_owned();
            if text.is_empty() {
                if let Some(done) = current.take() {
                    list.push(done);
                }
                continue;
            }
            let (key, value) = text.split_once(' ').unwrap_or((text.as_str(), ""));
            match key {
                "worktree" => {
                    if let Some(done) = current.take() {
                        list.push(done);
                    }
                    let path = PathBuf::from(value);
                    let is_current = std::fs::canonicalize(&path).ok() == here && here.is_some();
                    current = Some(Worktree {
                        path,
                        head: String::new(),
                        branch: None,
                        bare: false,
                        main: false,
                        current: is_current,
                        locked: None,
                        prunable: None,
                        staged: 0,
                        unstaged: 0,
                        untracked: 0,
                    });
                }
                "HEAD" => {
                    if let Some(w) = current.as_mut() {
                        w.head = value.to_owned();
                    }
                }
                "branch" => {
                    if let Some(w) = current.as_mut() {
                        w.branch = value.strip_prefix("refs/heads/").map(str::to_owned);
                    }
                }
                "bare" => {
                    if let Some(w) = current.as_mut() {
                        w.bare = true;
                    }
                }
                "locked" => {
                    if let Some(w) = current.as_mut() {
                        w.locked = Some(value.to_owned());
                    }
                }
                "prunable" => {
                    if let Some(w) = current.as_mut() {
                        w.prunable = Some(value.to_owned());
                    }
                }
                _ => {}
            }
        }
        if let Some(done) = current.take() {
            list.push(done);
        }
        if let Some(first) = list.first_mut() {
            first.main = true;
        }
        for worktree in &mut list {
            if worktree.bare || worktree.prunable.is_some() || !worktree.path.is_dir() {
                continue;
            }
            let out = self
                .in_dir(
                    &worktree.path,
                    &["status", "--porcelain", "-z", "--untracked-files=normal"],
                )
                .await?;
            if out.ok() {
                let (changed, untracked, staged, unstaged) = parse_status(&out);
                let _ = changed;
                worktree.staged = staged;
                worktree.unstaged = unstaged;
                worktree.untracked = untracked.len();
            }
        }
        Ok(list)
    }

    pub async fn add_worktree(
        &self,
        info: &SessionInfo,
        path: &Path,
        branch: BranchChoice<'_>,
    ) -> Result<(), WorktreeError> {
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(WorktreeError::InvalidPath);
        }
        if path.exists() {
            return Err(WorktreeError::PathExists);
        }
        let parent = path.parent().ok_or(WorktreeError::InvalidPath)?;
        let parent = std::fs::canonicalize(parent).map_err(|_| WorktreeError::ParentMissing)?;
        let target = parent.join(path.file_name().ok_or(WorktreeError::InvalidPath)?);
        if let Some(here) = info
            .worktree
            .as_deref()
            .and_then(|p| std::fs::canonicalize(p).ok())
            && target.starts_with(&here)
        {
            return Err(WorktreeError::InsideCurrent);
        }
        let list = self.list_worktrees(info).await?;
        let target_text = target.display().to_string();
        match branch {
            BranchChoice::Existing(name) => {
                if !self.valid_branch_name(info, name).await {
                    return Err(WorktreeError::InvalidBranch);
                }
                let spec = format!("refs/heads/{name}");
                if !self
                    .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
                    .await?
                    .ok()
                {
                    return Err(WorktreeError::NoSuchBranch);
                }
                if let Some(used) = list.iter().find(|w| w.branch.as_deref() == Some(name)) {
                    return Err(WorktreeError::BranchInUse(used.path.clone()));
                }
                self.worktree_add(info, &["worktree", "add", "--", &target_text, name])
                    .await
            }
            BranchChoice::New { name, start } => {
                if !self.valid_branch_name(info, name).await {
                    return Err(WorktreeError::InvalidBranch);
                }
                let spec = format!("refs/heads/{name}");
                if self
                    .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
                    .await?
                    .ok()
                {
                    return Err(WorktreeError::BranchExists);
                }
                match start {
                    Some(oid) => {
                        if oid.len() < 40 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
                            return Err(WorktreeError::InvalidPath);
                        }
                        self.worktree_add(
                            info,
                            &["worktree", "add", "-b", name, "--", &target_text, oid],
                        )
                        .await
                    }
                    None => {
                        self.worktree_add(
                            info,
                            &["worktree", "add", "-b", name, "--", &target_text],
                        )
                        .await
                    }
                }
            }
        }
    }

    async fn worktree_add(&self, info: &SessionInfo, args: &[&str]) -> Result<(), WorktreeError> {
        let out = self.exec(info, args, &[], None).await?;
        if out.ok() {
            Ok(())
        } else {
            Err(WorktreeError::Refused(out.stderr))
        }
    }

    async fn find_worktree(
        &self,
        info: &SessionInfo,
        path: &Path,
    ) -> Result<Worktree, WorktreeError> {
        let wanted = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.list_worktrees(info)
            .await?
            .into_iter()
            .find(|w| {
                w.path == path || std::fs::canonicalize(&w.path).ok().as_ref() == Some(&wanted)
            })
            .ok_or(WorktreeError::NoSuchWorktree)
    }

    pub async fn lock_worktree(
        &self,
        info: &SessionInfo,
        path: &Path,
        reason: &str,
    ) -> Result<(), WorktreeError> {
        let worktree = self.find_worktree(info, path).await?;
        if worktree.main {
            return Err(WorktreeError::IsMain);
        }
        if worktree.locked.is_some() {
            return Err(WorktreeError::AlreadyLocked);
        }
        let text = worktree.path.display().to_string();
        let reason = reason.replace(['\n', '\r'], " ");
        let mut args = vec!["worktree", "lock"];
        if !reason.trim().is_empty() {
            args.extend(["--reason", reason.trim()]);
        }
        args.extend(["--", &text]);
        let out = self.exec(info, &args, &[], None).await?;
        if out.ok() {
            Ok(())
        } else {
            Err(WorktreeError::Refused(out.stderr))
        }
    }

    pub async fn unlock_worktree(
        &self,
        info: &SessionInfo,
        path: &Path,
    ) -> Result<(), WorktreeError> {
        let worktree = self.find_worktree(info, path).await?;
        if worktree.locked.is_none() {
            return Err(WorktreeError::NotLocked);
        }
        let text = worktree.path.display().to_string();
        let out = self
            .exec(info, &["worktree", "unlock", "--", &text], &[], None)
            .await?;
        if out.ok() {
            Ok(())
        } else {
            Err(WorktreeError::Refused(out.stderr))
        }
    }

    pub async fn worktree_removal(
        &self,
        info: &SessionInfo,
        path: &Path,
    ) -> Result<WorktreeRemoval, WorktreeError> {
        let worktree = self.find_worktree(info, path).await?;
        if worktree.main {
            return Err(WorktreeError::IsMain);
        }
        if worktree.current {
            return Err(WorktreeError::IsCurrent);
        }
        let (changed, untracked) = if worktree.path.is_dir() && worktree.prunable.is_none() {
            let out = self
                .in_dir(
                    &worktree.path,
                    &["status", "--porcelain", "-z", "--untracked-files=all"],
                )
                .await?;
            let (changed, untracked, _, _) = parse_status(&out);
            (changed, untracked)
        } else {
            (Vec::new(), Vec::new())
        };
        Ok(WorktreeRemoval {
            path: worktree.path,
            branch: worktree.branch,
            locked: worktree.locked,
            changed,
            untracked,
        })
    }

    pub async fn remove_worktree(
        &self,
        info: &SessionInfo,
        path: &Path,
        discard_confirmed: bool,
    ) -> Result<WorktreeRemoved, WorktreeError> {
        let removal = self.worktree_removal(info, path).await?;
        if let Some(reason) = &removal.locked {
            return Err(WorktreeError::Locked(reason.clone()));
        }
        let dirty = !removal.changed.is_empty() || !removal.untracked.is_empty();
        if dirty && !discard_confirmed {
            return Err(WorktreeError::NeedsDecision(Box::new(removal)));
        }
        let mut snapshot = None;
        if !removal.changed.is_empty() {
            let out = self.in_dir(&removal.path, &["stash", "create"]).await?;
            let oid = out.text();
            if out.ok() && !oid.is_empty() {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let name = removal
                    .path
                    .file_name()
                    .map(|n| {
                        n.to_string_lossy()
                            .replace(|c: char| !c.is_ascii_alphanumeric(), "-")
                    })
                    .unwrap_or_default();
                let marker = format!("refs/gitpoulpe/recovery/{stamp}-worktree-{name}");
                self.exec_ok(info, &["update-ref", &marker, &oid]).await?;
                snapshot = Some(marker);
            }
        }
        let text = removal.path.display().to_string();
        let mut args = vec!["worktree", "remove"];
        if dirty {
            args.push("--force");
        }
        args.extend(["--", &text]);
        let out = self.exec(info, &args, &[], None).await?;
        if !out.ok() {
            return Err(WorktreeError::Refused(out.stderr));
        }
        Ok(WorktreeRemoved {
            path: removal.path,
            snapshot,
            lost_untracked: removal.untracked.len(),
        })
    }

    pub async fn prune_worktrees(&self, info: &SessionInfo) -> Result<(), WorktreeError> {
        self.exec_ok(info, &["worktree", "prune"]).await?;
        Ok(())
    }
}
