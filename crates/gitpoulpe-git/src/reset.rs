use std::path::PathBuf;

use gitpoulpe_core::{ResetCommit, ResetMode, ResetOutcome, ResetPreview, SessionInfo};

use crate::Git;
use crate::exec::ExecError;

const LEAVING_LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResetError {
    NothingCommitted,
    InvalidTarget,
    HeadMoved,
    OperationInProgress,
    NeedsConfirmation(Vec<PathBuf>),
    Refused(String),
    Exec(ExecError),
}

impl std::fmt::Display for ResetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingCommitted => f.write_str("le dépôt n'a encore aucun commit"),
            Self::InvalidTarget => f.write_str("la cible n'est pas un commit du dépôt"),
            Self::HeadMoved => f.write_str(
                "la branche courante a changé depuis l'aperçu : rien n'a été déplacé, relisez l'aperçu",
            ),
            Self::OperationInProgress => f.write_str(
                "une opération Git est en cours ou des fichiers sont en conflit : terminez-la d'abord",
            ),
            Self::NeedsConfirmation(files) => write!(
                f,
                "ce reset effacerait des modifications non enregistrées ({} fichier(s)) : une confirmation explicite est requise",
                files.len()
            ),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ResetError {}

impl From<ExecError> for ResetError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<crate::conflict::ConflictError> for ResetError {
    fn from(error: crate::conflict::ConflictError) -> Self {
        Self::Refused(error.to_string())
    }
}

fn valid_oid(id: &str) -> bool {
    (40..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn paths(out: &crate::exec::ExecOut) -> Vec<PathBuf> {
    out.stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned()))
        .collect()
}

impl Git {
    async fn commit_oid(&self, info: &SessionInfo, rev: &str) -> Result<String, ResetError> {
        if !valid_oid(rev) {
            return Err(ResetError::InvalidTarget);
        }
        let spec = format!("{rev}^{{commit}}");
        let out = self
            .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
            .await?;
        if out.ok() {
            Ok(out.text())
        } else {
            Err(ResetError::InvalidTarget)
        }
    }

    async fn current_head(&self, info: &SessionInfo) -> Result<String, ResetError> {
        let out = self
            .exec(
                info,
                &["rev-parse", "--verify", "-q", "HEAD^{commit}"],
                &[],
                None,
            )
            .await?;
        if out.ok() {
            Ok(out.text())
        } else {
            Err(ResetError::NothingCommitted)
        }
    }

    pub async fn reset_preview(
        &self,
        info: &SessionInfo,
        target: &str,
        mode: ResetMode,
    ) -> Result<ResetPreview, ResetError> {
        let head = self.current_head(info).await?;
        let target = self.commit_oid(info, target).await?;
        let branch = {
            let out = self
                .exec(info, &["symbolic-ref", "--short", "-q", "HEAD"], &[], None)
                .await?;
            out.ok().then(|| out.text())
        };
        let limit = format!("--max-count={}", LEAVING_LIMIT + 1);
        let log = self
            .exec_ok(
                info,
                &[
                    "log",
                    &limit,
                    "--format=%H%x1f%s",
                    &format!("{target}..{head}"),
                    "--",
                ],
            )
            .await?;
        let mut leaving: Vec<ResetCommit> = String::from_utf8_lossy(&log.stdout)
            .lines()
            .filter_map(|line| {
                let (oid, subject) = line.split_once('\u{1f}')?;
                Some(ResetCommit {
                    oid: oid.to_owned(),
                    subject: subject.to_owned(),
                })
            })
            .collect();
        let leaving_truncated = leaving.len() > LEAVING_LIMIT;
        leaving.truncate(LEAVING_LIMIT);
        let entering = self
            .exec_ok(info, &["rev-list", "--count", &format!("{head}..{target}")])
            .await?
            .text()
            .parse()
            .unwrap_or(0);
        let staged = paths(
            &self
                .exec_ok(
                    info,
                    &["diff", "--cached", "--name-only", "-z", "--no-renames"],
                )
                .await?,
        );
        let unstaged = paths(
            &self
                .exec_ok(info, &["diff", "--name-only", "-z", "--no-renames"])
                .await?,
        );
        let would_lose = if mode == ResetMode::Hard {
            let mut lost = paths(
                &self
                    .exec_ok(
                        info,
                        &["diff", "--name-only", "-z", "--no-renames", &target],
                    )
                    .await?,
            );
            for path in paths(
                &self
                    .exec_ok(
                        info,
                        &[
                            "diff",
                            "--cached",
                            "--name-only",
                            "-z",
                            "--no-renames",
                            &target,
                        ],
                    )
                    .await?,
            ) {
                if !lost.contains(&path) {
                    lost.push(path);
                }
            }
            let dirty: Vec<PathBuf> = staged.iter().chain(&unstaged).cloned().collect();
            lost.retain(|p| dirty.contains(p));
            lost.sort();
            lost
        } else {
            Vec::new()
        };
        Ok(ResetPreview {
            branch,
            head,
            target,
            leaving,
            leaving_truncated,
            entering,
            staged,
            unstaged,
            would_lose,
        })
    }

    pub async fn reset_branch(
        &self,
        info: &SessionInfo,
        target: &str,
        mode: ResetMode,
        expected_head: &str,
        discard_confirmed: bool,
    ) -> Result<ResetOutcome, ResetError> {
        if self.operation_state(info).await?.is_some() {
            return Err(ResetError::OperationInProgress);
        }
        let head = self.current_head(info).await?;
        if head != expected_head {
            return Err(ResetError::HeadMoved);
        }
        let preview = self.reset_preview(info, target, mode).await?;
        if !preview.would_lose.is_empty() && !discard_confirmed {
            return Err(ResetError::NeedsConfirmation(preview.would_lose));
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let short: String = head.chars().take(8).collect();
        let head_marker = format!("refs/gitpoulpe/recovery/{stamp}-{short}-head");
        self.exec_ok(info, &["update-ref", &head_marker, &head])
            .await?;
        let mut work_marker = None;
        if mode == ResetMode::Hard && !preview.would_lose.is_empty() {
            let snapshot = self.exec(info, &["stash", "create"], &[], None).await?;
            let oid = snapshot.text();
            if snapshot.ok() && !oid.is_empty() {
                let marker = format!("refs/gitpoulpe/recovery/{stamp}-{short}-work");
                self.exec_ok(info, &["update-ref", &marker, &oid]).await?;
                work_marker = Some(marker);
            }
        }
        let out = self
            .exec(
                info,
                &["reset", "-q", mode.flag(), &preview.target],
                &[],
                None,
            )
            .await?;
        if !out.ok() {
            return Err(ResetError::Refused(out.stderr));
        }
        Ok(ResetOutcome {
            mode,
            old_head: head,
            new_head: preview.target,
            head_marker,
            work_marker,
        })
    }
}
