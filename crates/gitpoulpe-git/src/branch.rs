use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use gitpoulpe_core::{Branch, BranchLoss, Diagnostic, LOSS_LIST_LIMIT, LostCommit, SessionInfo};

use crate::process::RunError;
use crate::stage::locked_index;
use crate::{Cancel, Git};

const FIELD: u8 = 0x1f;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BranchError {
    NoWorktree,
    InvalidName,
    InvalidStart,
    AlreadyExists,
    NoSuchBranch,
    IsCurrent,
    UsedByWorktree(PathBuf),
    NotMerged,
    Changed,
    Refused(String),
    IndexLocked(String),
    Parse(String),
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
}

impl std::fmt::Display for BranchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWorktree => f.write_str("le dépôt n'a pas de répertoire de travail"),
            Self::InvalidName => f.write_str("nom de branche invalide pour Git"),
            Self::InvalidStart => f.write_str("point de départ invalide"),
            Self::AlreadyExists => f.write_str("une branche porte déjà ce nom"),
            Self::NoSuchBranch => f.write_str("cette branche n'existe pas"),
            Self::IsCurrent => {
                f.write_str("c'est la branche courante : changez de branche d'abord")
            }
            Self::UsedByWorktree(path) => write!(
                f,
                "cette branche est déjà utilisée par le worktree {}",
                path.display()
            ),
            Self::NotMerged => f.write_str("cette branche n'est pas fusionnée"),
            Self::Changed => {
                f.write_str("la branche a bougé depuis l'affichage, rien n'a été supprimé")
            }
            Self::Refused(message) => write!(f, "Git refuse : {message}"),
            Self::IndexLocked(path) => write!(
                f,
                "un autre processus Git travaille sur ce dépôt ({path} existe) ; rien n'a été modifié"
            ),
            Self::Parse(m) => write!(f, "sortie Git inattendue : {m}"),
            Self::GitUnavailable(d) => write!(f, "Git n'a pas pu démarrer : {d}"),
            Self::GitFailed(d) => write!(f, "Git a échoué : {d}"),
        }
    }
}

impl std::error::Error for BranchError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteMode {
    Merged,
    Force { expected_tip: String },
}

fn failure(error: RunError) -> BranchError {
    match error {
        RunError::Spawn(e) => BranchError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => BranchError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Cancelled => BranchError::Refused("cancelled".to_owned()),
        RunError::Rejected(m) => BranchError::Parse(m),
    }
}

fn valid_oid(id: &str) -> bool {
    (4..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn clean_message(diagnostic: &Diagnostic) -> String {
    diagnostic
        .message
        .lines()
        .filter(|l| !l.trim_start().starts_with("hint:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

impl Git {
    async fn branch_run(
        &self,
        info: &SessionInfo,
        args: &[&OsStr],
    ) -> Result<(crate::process::Run, Vec<u8>), BranchError> {
        let worktree = info.worktree.as_deref().ok_or(BranchError::NoWorktree)?;
        self.run_collect(worktree, args, &Cancel::new())
            .await
            .map_err(failure)
    }

    async fn branch_ok(&self, info: &SessionInfo, args: &[&str]) -> Result<Vec<u8>, BranchError> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let (run, out) = self.branch_run(info, &refs).await?;
        if run.succeeded() {
            return Ok(out);
        }
        let diagnostic = Diagnostic {
            cause: None,
            ..run.diagnostic()
        };
        if let Some(path) = locked_index(&diagnostic) {
            return Err(BranchError::IndexLocked(path));
        }
        Err(BranchError::Refused(clean_message(&diagnostic)))
    }

    pub async fn valid_branch_name(&self, info: &SessionInfo, name: &str) -> bool {
        if name.is_empty()
            || name == "@"
            || name.starts_with('-')
            || name.contains(['\0', '\n', '\r'])
        {
            return false;
        }
        let full = format!("refs/heads/{name}");
        let checks: [&[&str]; 2] = [
            &["check-ref-format", "--branch", name],
            &["check-ref-format", &full],
        ];
        for args in checks {
            if self.branch_ok(info, args).await.is_err() {
                return false;
            }
        }
        true
    }

    pub async fn list_branches(&self, info: &SessionInfo) -> Result<Vec<Branch>, BranchError> {
        let out = self
            .branch_ok(
                info,
                &[
                    "for-each-ref",
                    "--format=%(refname)%1f%(objectname)%1f%(HEAD)%1f%(worktreepath)%1f%(upstream:short)%1f%(contents:subject)",
                    "refs/heads",
                ],
            )
            .await?;
        let own = info
            .worktree
            .as_deref()
            .and_then(|w| std::fs::canonicalize(w).ok());
        let mut branches = Vec::new();
        for line in out.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let mut fields = line.splitn(6, |b| *b == FIELD);
            let (Some(name), Some(oid), Some(head), Some(tree), Some(upstream), Some(subject)) = (
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
                fields.next(),
            ) else {
                return Err(BranchError::Parse("missing branch fields".to_owned()));
            };
            let text = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
            let name = text(name);
            let Some(name) = name.strip_prefix("refs/heads/") else {
                return Err(BranchError::Parse("unexpected reference".to_owned()));
            };
            let other_worktree = (!tree.is_empty())
                .then(|| PathBuf::from(OsStr::from_bytes(tree)))
                .filter(|p| {
                    own.as_ref()
                        .is_none_or(|own| std::fs::canonicalize(p).ok().as_ref() != Some(own))
                });
            branches.push(Branch {
                name: name.to_owned(),
                oid: text(oid),
                subject: text(subject),
                upstream: (!upstream.is_empty()).then(|| text(upstream)),
                current: head == b"*",
                other_worktree,
            });
        }
        branches.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(branches)
    }

    async fn find_branch(&self, info: &SessionInfo, name: &str) -> Result<Branch, BranchError> {
        self.list_branches(info)
            .await?
            .into_iter()
            .find(|b| b.name == name)
            .ok_or(BranchError::NoSuchBranch)
    }

    pub async fn create_branch(
        &self,
        info: &SessionInfo,
        name: &str,
        start: Option<&str>,
    ) -> Result<(), BranchError> {
        if !self.valid_branch_name(info, name).await {
            return Err(BranchError::InvalidName);
        }
        if self.find_branch(info, name).await.is_ok() {
            return Err(BranchError::AlreadyExists);
        }
        match start {
            None => self
                .branch_ok(info, &["branch", "--", name])
                .await
                .map(drop),
            Some(oid) => {
                if !valid_oid(oid) {
                    return Err(BranchError::InvalidStart);
                }
                let target = format!("{oid}^{{commit}}");
                if self
                    .branch_ok(info, &["rev-parse", "--verify", "--quiet", &target])
                    .await
                    .is_err()
                {
                    return Err(BranchError::InvalidStart);
                }
                self.branch_ok(info, &["branch", "--", name, oid])
                    .await
                    .map(drop)
            }
        }
    }

    pub async fn switch_branch(
        &self,
        info: &SessionInfo,
        name: &str,
    ) -> Result<(Option<String>, String), BranchError> {
        let branch = self.find_branch(info, name).await?;
        if let Some(path) = branch.other_worktree {
            return Err(BranchError::UsedByWorktree(path));
        }
        let before = self
            .branch_ok(info, &["rev-parse", "--verify", "--quiet", "HEAD"])
            .await
            .ok()
            .map(|b| String::from_utf8_lossy(&b).trim().to_owned());
        self.branch_ok(info, &["switch", "--no-guess", "--", name])
            .await?;
        Ok((before, branch.oid))
    }

    pub async fn rename_branch(
        &self,
        info: &SessionInfo,
        old: &str,
        new: &str,
    ) -> Result<(), BranchError> {
        self.find_branch(info, old).await?;
        if !self.valid_branch_name(info, new).await {
            return Err(BranchError::InvalidName);
        }
        if self.find_branch(info, new).await.is_ok() {
            return Err(BranchError::AlreadyExists);
        }
        self.branch_ok(info, &["branch", "-m", "--", old, new])
            .await
            .map(drop)
    }

    pub async fn branch_loss(
        &self,
        info: &SessionInfo,
        name: &str,
    ) -> Result<BranchLoss, BranchError> {
        let branch = self.find_branch(info, name).await?;
        let full = format!("refs/heads/{name}");
        let exclude = format!("--exclude={full}");
        let limit = format!("--max-count={}", LOSS_LIST_LIMIT + 1);
        let out = self
            .branch_ok(
                info,
                &[
                    "log",
                    "--format=%H%x1f%s",
                    &limit,
                    &full,
                    "--not",
                    &exclude,
                    "--all",
                ],
            )
            .await?;
        let mut commits: Vec<LostCommit> = out
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .filter_map(|line| {
                let mut f = line.splitn(2, |b| *b == FIELD);
                let oid = String::from_utf8_lossy(f.next()?).into_owned();
                let subject = String::from_utf8_lossy(f.next().unwrap_or_default()).into_owned();
                valid_oid(&oid).then_some(LostCommit { oid, subject })
            })
            .collect();
        let truncated = commits.len() > LOSS_LIST_LIMIT;
        commits.truncate(LOSS_LIST_LIMIT);
        Ok(BranchLoss {
            tip: branch.oid,
            commits,
            truncated,
        })
    }

    pub async fn delete_branch(
        &self,
        info: &SessionInfo,
        name: &str,
        mode: &DeleteMode,
    ) -> Result<String, BranchError> {
        let branch = self.find_branch(info, name).await?;
        if branch.current {
            return Err(BranchError::IsCurrent);
        }
        if let Some(path) = branch.other_worktree {
            return Err(BranchError::UsedByWorktree(path));
        }
        let flag = match mode {
            DeleteMode::Merged => "-d",
            DeleteMode::Force { expected_tip } => {
                if *expected_tip != branch.oid {
                    return Err(BranchError::Changed);
                }
                "-D"
            }
        };
        match self.branch_ok(info, &["branch", flag, "--", name]).await {
            Ok(_) => Ok(branch.oid),
            Err(BranchError::Refused(message)) if message.contains("not fully merged") => {
                Err(BranchError::NotMerged)
            }
            Err(other) => Err(other),
        }
    }
}
