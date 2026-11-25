use gitpoulpe_core::{LostRemoteCommit, PushKind, PushMode, PushOutcome, PushPreview, SessionInfo};

use crate::exec::ExecError;
use crate::net::{NetError, NetProgress};
use crate::{Cancel, Git};

const DISCARD_LIST_LIMIT: usize = 50;

#[derive(Clone, Copy, Debug)]
pub struct PushRequest<'a> {
    pub branch: &'a str,
    pub remote: &'a str,
    pub mode: PushMode,
    pub expected_remote_oid: Option<&'a str>,
    pub set_upstream: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushRejection {
    NonFastForward,
    FetchFirst,
    StaleLease,
    Hook(String),
    Permission(String),
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushError {
    InvalidBranch,
    NoSuchRemote,
    NothingKnown,
    Rejected(PushRejection),
    Net(NetError),
    Exec(ExecError),
}

impl std::fmt::Display for PushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBranch => f.write_str("cette branche n'existe pas ou n'a pas de commit"),
            Self::NoSuchRemote => f.write_str("ce dépôt distant n'existe pas"),
            Self::NothingKnown => f.write_str(
                "la protection de l'historique réécrit a besoin de l'état distant connu : récupérez d'abord ce dépôt distant",
            ),
            Self::Rejected(PushRejection::NonFastForward) => f.write_str(
                "refusé : le distant a des commits que votre branche n'a pas (non fast-forward). Votre travail est intact ; récupérez puis intégrez les changements distants, ou, si vous avez réécrit l'historique, utilisez la publication protégée",
            ),
            Self::Rejected(PushRejection::FetchFirst) => f.write_str(
                "refusé : le distant contient des commits que vous n'avez pas encore récupérés ; récupérez d'abord, votre travail est intact",
            ),
            Self::Rejected(PushRejection::StaleLease) => f.write_str(
                "refusé par la protection : quelqu'un a publié sur cette branche depuis l'état attendu, rien n'a été écrasé ; récupérez et examinez ses commits",
            ),
            Self::Rejected(PushRejection::Hook(m)) => {
                write!(f, "refusé par un hook du distant : {m}")
            }
            Self::Rejected(PushRejection::Permission(m)) => write!(
                f,
                "refusé : vous n'avez pas le droit d'écrire sur ce distant ({m}) ; votre travail est intact"
            ),
            Self::Rejected(PushRejection::Other(m)) => write!(f, "refusé par le distant : {m}"),
            Self::Net(e) => write!(f, "{e}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PushError {}

impl From<ExecError> for PushError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('-') && !name.contains(['\0', '\n', '\r'])
}

fn rejection_of(summary: &str) -> PushRejection {
    let lower = summary.to_lowercase();
    if lower.contains("stale info") {
        PushRejection::StaleLease
    } else if lower.contains("fetch first") {
        PushRejection::FetchFirst
    } else if lower.contains("non-fast-forward") {
        PushRejection::NonFastForward
    } else if lower.contains("permission") || lower.contains("denied") && !lower.contains("hook") {
        PushRejection::Permission(summary.trim().to_owned())
    } else if lower.contains("hook") {
        PushRejection::Hook(summary.trim().to_owned())
    } else {
        PushRejection::Other(summary.trim().to_owned())
    }
}

impl Git {
    async fn local_branch_oid(
        &self,
        info: &SessionInfo,
        branch: &str,
    ) -> Result<String, PushError> {
        if !valid_name(branch) {
            return Err(PushError::InvalidBranch);
        }
        let spec = format!("refs/heads/{branch}^{{commit}}");
        let out = self
            .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
            .await?;
        if out.ok() {
            Ok(out.text())
        } else {
            Err(PushError::InvalidBranch)
        }
    }

    async fn known_remote(&self, info: &SessionInfo, remote: &str) -> Result<(), PushError> {
        if !valid_name(remote) {
            return Err(PushError::NoSuchRemote);
        }
        let remotes = self
            .list_remotes(info)
            .await
            .map_err(|e| PushError::Net(NetError::Refused(e.to_string())))?;
        if remotes.iter().any(|r| r.name == remote) {
            Ok(())
        } else {
            Err(PushError::NoSuchRemote)
        }
    }

    pub async fn push_preview(
        &self,
        info: &SessionInfo,
        branch: &str,
        remote: &str,
    ) -> Result<PushPreview, PushError> {
        let local_oid = self.local_branch_oid(info, branch).await?;
        self.known_remote(info, remote).await?;
        let tracking = format!("refs/remotes/{remote}/{branch}");
        let known = self
            .exec(
                info,
                &[
                    "rev-parse",
                    "--verify",
                    "-q",
                    &format!("{tracking}^{{commit}}"),
                ],
                &[],
                None,
            )
            .await?;
        let known_remote_oid = known.ok().then(|| known.text());
        let upstream = {
            let out = self
                .exec(
                    info,
                    &[
                        "for-each-ref",
                        "--format=%(upstream:short)",
                        &format!("refs/heads/{branch}"),
                    ],
                    &[],
                    None,
                )
                .await?;
            Some(out.text()).filter(|u| !u.is_empty())
        };
        let (mut ahead, mut behind) = (0, 0);
        let mut would_discard = Vec::new();
        let kind = match &known_remote_oid {
            None => PushKind::NewBranch,
            Some(remote_oid) => {
                let counts = self
                    .exec_ok(
                        info,
                        &[
                            "rev-list",
                            "--left-right",
                            "--count",
                            &format!("{local_oid}...{remote_oid}"),
                        ],
                    )
                    .await?
                    .text();
                let mut numbers = counts
                    .split_whitespace()
                    .map(|n| n.parse::<usize>().unwrap_or(0));
                (ahead, behind) = (numbers.next().unwrap_or(0), numbers.next().unwrap_or(0));
                if behind > 0 {
                    let limit = format!("--max-count={DISCARD_LIST_LIMIT}");
                    let log = self
                        .exec_ok(
                            info,
                            &[
                                "log",
                                &limit,
                                "--format=%H%x1f%s",
                                &format!("{local_oid}..{remote_oid}"),
                                "--",
                            ],
                        )
                        .await?;
                    for line in String::from_utf8_lossy(&log.stdout).lines() {
                        if let Some((oid, subject)) = line.split_once('\u{1f}') {
                            would_discard.push(LostRemoteCommit {
                                oid: oid.to_owned(),
                                subject: subject.to_owned(),
                            });
                        }
                    }
                }
                match (ahead, behind) {
                    (0, 0) => PushKind::UpToDate,
                    (_, 0) => PushKind::FastForward,
                    (0, _) => PushKind::Behind,
                    _ => PushKind::Diverged,
                }
            }
        };
        Ok(PushPreview {
            branch: branch.to_owned(),
            remote: remote.to_owned(),
            destination: format!("refs/heads/{branch}"),
            local_oid,
            known_remote_oid,
            upstream,
            ahead,
            behind,
            kind,
            would_discard,
        })
    }

    pub async fn push_branch(
        &self,
        info: &SessionInfo,
        request: &PushRequest<'_>,
        cancel: &Cancel,
        on_progress: impl FnMut(NetProgress),
    ) -> Result<PushOutcome, PushError> {
        let PushRequest {
            branch,
            remote,
            mode,
            expected_remote_oid,
            set_upstream,
        } = *request;
        let local_oid = self.local_branch_oid(info, branch).await?;
        self.known_remote(info, remote).await?;
        let destination = format!("refs/heads/{branch}");
        let refspec = format!("{destination}:{destination}");
        let mut args: Vec<String> = vec!["push".into(), "--porcelain".into(), "--progress".into()];
        match (mode, expected_remote_oid) {
            (PushMode::Normal, _) => {}
            (PushMode::ForceWithLease, Some(expected))
                if expected.len() >= 40 && expected.chars().all(|c| c.is_ascii_hexdigit()) =>
            {
                args.push(format!("--force-with-lease={destination}:{expected}"));
            }
            (PushMode::ForceWithLease, _) => return Err(PushError::NothingKnown),
        }
        if set_upstream {
            args.push("--set-upstream".into());
        }
        args.extend(["--".into(), remote.to_owned(), refspec]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let url = self
            .list_remotes(info)
            .await
            .ok()
            .and_then(|all| all.into_iter().find(|r| r.name == remote))
            .and_then(|r| r.push_targets().first().cloned());
        let run = self
            .run_network(dir, &refs, cancel, on_progress)
            .await
            .map_err(PushError::Net)?;
        let text = String::from_utf8_lossy(&run.stdout).into_owned();
        let mut up_to_date = false;
        for line in text.lines() {
            let mut fields = line.splitn(3, '\t');
            let (Some(flag), Some(_), Some(summary)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            match flag {
                "!" => return Err(PushError::Rejected(rejection_of(summary))),
                "=" => up_to_date = true,
                _ => {}
            }
        }
        if run.code != Some(0) {
            return Err(PushError::Net(
                self.classify_net_failure(&run.stderr, url.as_deref()).await,
            ));
        }
        let verified = self
            .remote_branch_oid(info, remote, &destination, cancel)
            .await
            .is_ok_and(|oid| oid.as_deref() == Some(local_oid.as_str()));
        Ok(PushOutcome {
            remote: remote.to_owned(),
            destination,
            pushed_oid: local_oid,
            verified,
            up_to_date,
            set_upstream,
        })
    }

    async fn remote_branch_oid(
        &self,
        info: &SessionInfo,
        remote: &str,
        reference: &str,
        cancel: &Cancel,
    ) -> Result<Option<String>, NetError> {
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let run = self
            .network_ok(
                dir,
                &["ls-remote", "--", remote, reference],
                None,
                cancel,
                |_| {},
            )
            .await?;
        Ok(String::from_utf8_lossy(&run.stdout).lines().find_map(|l| {
            let (oid, name) = l.split_once('\t')?;
            (name == reference).then(|| oid.to_owned())
        }))
    }
}
