use std::ffi::OsString;
use std::path::PathBuf;

use gitpoulpe_core::{
    Integration, Profile, PullKind, PullOutcome, PullPreview, PullStrategy, SessionInfo,
};

use crate::exec::ExecError;
use crate::identity::{profile_env, signing_config};
use crate::net::{NetError, NetProgress};
use crate::{Cancel, Git};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullError {
    NoBranch,
    NoUpstream,
    NothingCommitted,
    OperationInProgress,
    HeadMoved,
    Fetch(NetError),
    NotFastForward,
    LocalChanges,
    IdentityUnknown(String),
    Refused(String),
    Exec(ExecError),
}

impl std::fmt::Display for PullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBranch => f.write_str("HEAD est détaché : placez-vous sur une branche pour intégrer des changements distants"),
            Self::NoUpstream => f.write_str(
                "cette branche ne suit aucune branche distante : publiez-la avec le suivi activé d'abord",
            ),
            Self::NothingCommitted => f.write_str("le dépôt n'a encore aucun commit"),
            Self::OperationInProgress => f.write_str(
                "une autre opération Git est en cours ou des fichiers sont en conflit : terminez-la d'abord",
            ),
            Self::HeadMoved => f.write_str(
                "la branche courante a changé depuis l'aperçu : rien n'a été intégré, relisez l'aperçu",
            ),
            Self::Fetch(e) => write!(
                f,
                "la récupération a échoué ({e}) : rien n'a été intégré et votre travail local est intact"
            ),
            Self::NotFastForward => f.write_str(
                "l'avance rapide est impossible : votre branche et la branche distante ont divergé. Les références distantes ont été mises à jour ; choisissez la fusion ou le rebase. Rien n'a été intégré",
            ),
            Self::LocalChanges => f.write_str(
                "vos modifications locales seraient écrasées : enregistrez-les, mettez-les de côté ou cochez la mise de côté automatique ; rien n'a été intégré",
            ),
            Self::IdentityUnknown(m) => write!(f, "identité inconnue pour créer les commits : {m}"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PullError {}

impl From<ExecError> for PullError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<crate::conflict::ConflictError> for PullError {
    fn from(error: crate::conflict::ConflictError) -> Self {
        Self::Refused(error.to_string())
    }
}

struct Tracking {
    branch: String,
    remote: String,
    upstream: String,
    reference: String,
}

#[derive(Clone, Copy, Debug)]
pub struct PullRequest<'a> {
    pub strategy: PullStrategy,
    pub expected_head: &'a str,
    pub autostash: bool,
}

impl Git {
    async fn tracking(&self, info: &SessionInfo) -> Result<Tracking, PullError> {
        let head = self
            .exec(info, &["symbolic-ref", "--short", "-q", "HEAD"], &[], None)
            .await?;
        if !head.ok() {
            return Err(PullError::NoBranch);
        }
        let branch = head.text();
        let out = self
            .exec_ok(
                info,
                &[
                    "for-each-ref",
                    "--format=%(upstream)%1f%(upstream:short)%1f%(upstream:remotename)",
                    &format!("refs/heads/{branch}"),
                ],
            )
            .await?;
        let text = out.text();
        let mut fields = text.split('\u{1f}');
        let (reference, upstream, remote) = (
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
        );
        if reference.is_empty() || remote.is_empty() || remote == "." {
            return Err(PullError::NoUpstream);
        }
        Ok(Tracking {
            branch,
            remote: remote.to_owned(),
            upstream: upstream.to_owned(),
            reference: reference.to_owned(),
        })
    }

    pub async fn pull_preview(&self, info: &SessionInfo) -> Result<PullPreview, PullError> {
        let head = self
            .exec(
                info,
                &["rev-parse", "--verify", "-q", "HEAD^{commit}"],
                &[],
                None,
            )
            .await?;
        if !head.ok() {
            return Err(PullError::NothingCommitted);
        }
        let head = head.text();
        let tracking = self.tracking(info).await?;
        let known = self
            .exec(
                info,
                &[
                    "rev-parse",
                    "--verify",
                    "-q",
                    &format!("{}^{{commit}}", tracking.reference),
                ],
                &[],
                None,
            )
            .await?;
        let tracking_oid = known.ok().then(|| known.text());
        let (mut ahead, mut behind) = (0, 0);
        let kind = match &tracking_oid {
            None => PullKind::UpToDate,
            Some(oid) => {
                let counts = self
                    .exec_ok(
                        info,
                        &[
                            "rev-list",
                            "--left-right",
                            "--count",
                            &format!("{head}...{oid}"),
                        ],
                    )
                    .await?
                    .text();
                let mut numbers = counts
                    .split_whitespace()
                    .map(|n| n.parse::<usize>().unwrap_or(0));
                (ahead, behind) = (numbers.next().unwrap_or(0), numbers.next().unwrap_or(0));
                match (ahead, behind) {
                    (_, 0) if ahead == 0 => PullKind::UpToDate,
                    (_, 0) => PullKind::Ahead,
                    (0, _) => PullKind::Behind,
                    _ => PullKind::Diverged,
                }
            }
        };
        let status = self
            .exec_ok(
                info,
                &["status", "--porcelain", "-z", "--untracked-files=no"],
            )
            .await?;
        let local_changes = status
            .stdout
            .split(|b| *b == 0)
            .filter(|r| r.len() > 3)
            .map(|r| PathBuf::from(String::from_utf8_lossy(&r[3..]).into_owned()))
            .take(50)
            .collect();
        Ok(PullPreview {
            branch: tracking.branch,
            remote: tracking.remote,
            upstream: tracking.upstream,
            head,
            tracking: tracking_oid,
            kind,
            ahead,
            behind,
            local_changes,
        })
    }

    pub async fn pull(
        &self,
        info: &SessionInfo,
        request: &PullRequest<'_>,
        profile: Option<&Profile>,
        cancel: &Cancel,
        on_progress: impl FnMut(NetProgress),
    ) -> Result<PullOutcome, PullError> {
        if self.operation_state(info).await?.is_some() {
            return Err(PullError::OperationInProgress);
        }
        let before = self.pull_preview(info).await?;
        if before.head != request.expected_head {
            return Err(PullError::HeadMoved);
        }
        let tracking = self.tracking(info).await?;
        let fetched = self
            .fetch_remote(info, &tracking.remote, cancel, on_progress)
            .await
            .map_err(PullError::Fetch)?;
        let fetched_updates = fetched.updates.len();
        let after = self.pull_preview(info).await?;
        let Some(target) = after.tracking.clone() else {
            return Ok(PullOutcome {
                fetched_updates,
                integration: Integration::UpToDate,
            });
        };
        if after.kind == PullKind::UpToDate || after.kind == PullKind::Ahead {
            return Ok(PullOutcome {
                fetched_updates,
                integration: Integration::UpToDate,
            });
        }
        let integration = match request.strategy {
            PullStrategy::FastForwardOnly => {
                if after.kind == PullKind::Diverged {
                    return Err(PullError::NotFastForward);
                }
                let out = self
                    .exec(
                        info,
                        &["merge", "--ff-only", "--no-edit", &target],
                        &[],
                        None,
                    )
                    .await?;
                if !out.ok() {
                    return Err(self.integration_failure(&out.stderr));
                }
                Integration::FastForward {
                    from: after.head.clone(),
                    to: target,
                }
            }
            PullStrategy::Merge => {
                let mut env: Vec<(OsString, OsString)> =
                    profile.map(profile_env).unwrap_or_default();
                env.push(("GIT_EDITOR".into(), "true".into()));
                let mut args: Vec<String> = profile
                    .map(signing_config)
                    .unwrap_or_default()
                    .into_iter()
                    .flat_map(|(k, v)| ["-c".to_owned(), format!("{k}={v}")])
                    .collect();
                args.extend([
                    "merge".to_owned(),
                    "--no-edit".to_owned(),
                    "--ff".to_owned(),
                ]);
                if request.autostash {
                    args.push("--autostash".to_owned());
                }
                args.push("-m".to_owned());
                args.push(format!(
                    "Merge remote-tracking branch '{}'",
                    tracking.upstream
                ));
                args.push(target.clone());
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let out = self.exec(info, &refs, &env, None).await?;
                if self.operation_state(info).await?.is_some() {
                    return Ok(PullOutcome {
                        fetched_updates,
                        integration: self.conflict_integration(info).await?,
                    });
                }
                if !out.ok() {
                    return Err(self.integration_failure(&out.stderr));
                }
                let head = self.exec_ok(info, &["rev-parse", "HEAD"]).await?.text();
                if head == target {
                    Integration::FastForward {
                        from: after.head.clone(),
                        to: head,
                    }
                } else {
                    Integration::Merged {
                        from: after.head.clone(),
                        to: head,
                    }
                }
            }
            PullStrategy::Rebase => {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let short: String = after.head.chars().take(8).collect();
                let marker = format!("refs/gitpoulpe/recovery/{stamp}-{short}-pull");
                self.exec_ok(info, &["update-ref", &marker, &after.head])
                    .await?;
                let mut env: Vec<(OsString, OsString)> =
                    profile.map(profile_env).unwrap_or_default();
                env.push(("GIT_EDITOR".into(), "true".into()));
                let mut args: Vec<String> = profile
                    .map(signing_config)
                    .unwrap_or_default()
                    .into_iter()
                    .flat_map(|(k, v)| ["-c".to_owned(), format!("{k}={v}")])
                    .collect();
                args.push("rebase".to_owned());
                if request.autostash {
                    args.push("--autostash".to_owned());
                }
                args.push(target.clone());
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let out = self.exec(info, &refs, &env, None).await?;
                if self.operation_state(info).await?.is_some() {
                    return Ok(PullOutcome {
                        fetched_updates,
                        integration: self.conflict_integration(info).await?,
                    });
                }
                if !out.ok() {
                    let _ = self
                        .exec(info, &["update-ref", "-d", &marker], &[], None)
                        .await;
                    return Err(self.integration_failure(&out.stderr));
                }
                let head = self.exec_ok(info, &["rev-parse", "HEAD"]).await?.text();
                Integration::Rebased {
                    from: after.head.clone(),
                    to: head,
                    marker,
                }
            }
        };
        Ok(PullOutcome {
            fetched_updates,
            integration,
        })
    }

    async fn conflict_integration(&self, info: &SessionInfo) -> Result<Integration, PullError> {
        let files: Vec<PathBuf> = self
            .list_conflicts(info)
            .await?
            .into_iter()
            .map(|f| f.path)
            .collect();
        Ok(if files.is_empty() {
            Integration::Paused
        } else {
            Integration::Conflicts { files }
        })
    }

    fn integration_failure(&self, message: &str) -> PullError {
        if message.contains("overwritten")
            || message.contains("local changes")
            || message.contains("unstaged changes")
            || message.contains("uncommitted changes")
        {
            PullError::LocalChanges
        } else if message.contains("Not possible to fast-forward") {
            PullError::NotFastForward
        } else if message.contains("Please tell me who you are")
            || message.contains("unable to auto-detect email")
        {
            PullError::IdentityUnknown(message.to_owned())
        } else {
            PullError::Refused(message.to_owned())
        }
    }
}
