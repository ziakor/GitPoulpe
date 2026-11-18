use std::ffi::OsString;
use std::path::PathBuf;

use gitpoulpe_core::{MergeKind, MergeMode, MergeOutcome, MergePreview, Profile, SessionInfo};

use crate::Git;
use crate::conflict::ConflictError;
use crate::exec::{ExecError, ExecOut};
use crate::identity::{profile_env, signing_config};

const LOCAL_CHANGES_LIMIT: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeError {
    InvalidSource,
    OperationInProgress,
    HeadMoved,
    NothingCommitted,
    SameBranch,
    LocalChanges(Vec<PathBuf>),
    CannotFastForward,
    IdentityUnknown(String),
    Refused(String),
    Exec(ExecError),
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSource => f.write_str("cette branche n'existe pas ou n'a pas de commit"),
            Self::OperationInProgress => f.write_str(
                "une autre opération Git est en cours : terminez-la ou abandonnez-la d'abord",
            ),
            Self::HeadMoved => f.write_str(
                "la branche courante a changé depuis l'aperçu : rien n'a été fusionné, relisez l'aperçu",
            ),
            Self::NothingCommitted => f.write_str("le dépôt n'a encore aucun commit"),
            Self::SameBranch => f.write_str("on ne fusionne pas une branche avec elle-même"),
            Self::LocalChanges(files) => write!(
                f,
                "vos modifications locales seraient écrasées ({}) : enregistrez-les, mettez-les de côté ou cochez la mise de côté automatique ; rien n'a été fusionné",
                files
                    .iter()
                    .take(5)
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::CannotFastForward => f.write_str(
                "l'avance rapide est impossible : les branches ont divergé, rien n'a été fusionné",
            ),
            Self::IdentityUnknown(m) => write!(f, "identité inconnue pour le commit de fusion : {m}"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MergeError {}

impl From<ExecError> for MergeError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<ConflictError> for MergeError {
    fn from(error: ConflictError) -> Self {
        Self::Refused(error.to_string())
    }
}

fn valid_branch_ref(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('-') && !name.contains(['\0', '\n', '\r'])
}

impl Git {
    async fn head_oid(&self, info: &SessionInfo) -> Result<String, MergeError> {
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
            Err(MergeError::NothingCommitted)
        }
    }

    async fn source_oid(&self, info: &SessionInfo, source: &str) -> Result<String, MergeError> {
        if !valid_branch_ref(source) {
            return Err(MergeError::InvalidSource);
        }
        let spec = format!("refs/heads/{source}^{{commit}}");
        let out = self
            .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
            .await?;
        if out.ok() {
            Ok(out.text())
        } else {
            Err(MergeError::InvalidSource)
        }
    }

    pub(crate) async fn tracked_changes(
        &self,
        info: &SessionInfo,
    ) -> Result<Vec<PathBuf>, MergeError> {
        let out = self
            .exec_ok(
                info,
                &["status", "--porcelain", "-z", "--untracked-files=no"],
            )
            .await?;
        let mut paths = Vec::new();
        let mut records = out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty());
        while let Some(record) = records.next() {
            let text = String::from_utf8_lossy(record).into_owned();
            if text.len() < 4 {
                continue;
            }
            if text.as_bytes()[0] == b'R' || text.as_bytes()[0] == b'C' {
                records.next();
            }
            if paths.len() < LOCAL_CHANGES_LIMIT {
                paths.push(PathBuf::from(&text[3..]));
            }
        }
        Ok(paths)
    }

    pub async fn merge_preview(
        &self,
        info: &SessionInfo,
        source: &str,
    ) -> Result<MergePreview, MergeError> {
        let head = self.head_oid(info).await?;
        let source_oid = self.source_oid(info, source).await?;
        let current = self
            .exec(info, &["symbolic-ref", "--short", "-q", "HEAD"], &[], None)
            .await?;
        let current = current.ok().then(|| current.text());
        if current.as_deref() == Some(source) {
            return Err(MergeError::SameBranch);
        }
        let counts = self
            .exec_ok(
                info,
                &[
                    "rev-list",
                    "--left-right",
                    "--count",
                    &format!("{head}...{source_oid}"),
                ],
            )
            .await?
            .text();
        let mut numbers = counts
            .split_whitespace()
            .map(|n| n.parse::<usize>().unwrap_or(0));
        let (only_in_head, only_in_source) =
            (numbers.next().unwrap_or(0), numbers.next().unwrap_or(0));
        let kind = if only_in_source == 0 {
            MergeKind::UpToDate
        } else if only_in_head == 0 {
            MergeKind::FastForward
        } else {
            MergeKind::TrueMerge
        };
        let predicted_conflicts = if kind == MergeKind::TrueMerge {
            let out = self
                .exec(
                    info,
                    &[
                        "merge-tree",
                        "--write-tree",
                        "--name-only",
                        "--no-messages",
                        &head,
                        &source_oid,
                    ],
                    &[],
                    None,
                )
                .await?;
            match out.code {
                Some(0) => Some(Vec::new()),
                Some(1) => Some(
                    String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .skip(1)
                        .take_while(|l| !l.is_empty())
                        .map(PathBuf::from)
                        .collect(),
                ),
                _ => None,
            }
        } else {
            None
        };
        Ok(MergePreview {
            current,
            head,
            source: source.to_owned(),
            source_oid,
            kind,
            only_in_head,
            only_in_source,
            predicted_conflicts,
            local_changes: self.tracked_changes(info).await?,
        })
    }

    pub async fn merge_branch(
        &self,
        info: &SessionInfo,
        source: &str,
        mode: MergeMode,
        expected_head: &str,
        autostash: bool,
        profile: Option<&Profile>,
    ) -> Result<MergeOutcome, MergeError> {
        if self.operation_state(info).await?.is_some() {
            return Err(MergeError::OperationInProgress);
        }
        let head = self.head_oid(info).await?;
        if head != expected_head {
            return Err(MergeError::HeadMoved);
        }
        let source_oid = self.source_oid(info, source).await?;
        let stashes_before = self.stash_count(info).await?;
        let mut env: Vec<(OsString, OsString)> = profile.map(profile_env).unwrap_or_default();
        env.push(("GIT_EDITOR".into(), "true".into()));
        let mut args: Vec<String> = profile
            .map(signing_config)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|(k, v)| ["-c".to_owned(), format!("{k}={v}")])
            .collect();
        args.extend(["merge".to_owned(), "--no-edit".to_owned()]);
        args.push(
            match mode {
                MergeMode::Auto => "--ff",
                MergeMode::NoFastForward => "--no-ff",
                MergeMode::FastForwardOnly => "--ff-only",
            }
            .to_owned(),
        );
        if autostash {
            args.push("--autostash".to_owned());
        }
        let into = self
            .exec(info, &["symbolic-ref", "--short", "-q", "HEAD"], &[], None)
            .await?;
        let message = match into.ok().then(|| into.text()) {
            Some(name) if name != "main" && name != "master" => {
                format!("Merge branch '{source}' into {name}")
            }
            _ => format!("Merge branch '{source}'"),
        };
        args.push("-m".to_owned());
        args.push(message);
        args.push("--".to_owned());
        args.push(format!("refs/heads/{source}"));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.exec(info, &refs, &env, None).await?;
        if !out.ok() {
            return self.merge_failure(info, &out).await;
        }
        let after = self.head_oid(info).await?;
        let stash_kept = self.stash_count(info).await? > stashes_before;
        if after == head {
            return Ok(MergeOutcome::UpToDate);
        }
        if after == source_oid {
            return Ok(MergeOutcome::FastForward {
                from: head,
                to: after,
            });
        }
        Ok(MergeOutcome::Merged {
            from: head,
            to: after,
            stash_kept,
        })
    }

    async fn stash_count(&self, info: &SessionInfo) -> Result<usize, MergeError> {
        let out = self.exec(info, &["stash", "list"], &[], None).await?;
        Ok(if out.ok() {
            out.text().lines().count()
        } else {
            0
        })
    }

    async fn merge_failure(
        &self,
        info: &SessionInfo,
        out: &ExecOut,
    ) -> Result<MergeOutcome, MergeError> {
        if self.operation_state(info).await?.is_some() {
            let files = self
                .list_conflicts(info)
                .await?
                .into_iter()
                .map(|f| f.path)
                .collect();
            return Ok(MergeOutcome::Conflicts { files });
        }
        let message = out.stderr.clone();
        if message.contains("overwritten by merge") || message.contains("Your local changes") {
            let files: Vec<PathBuf> = message
                .lines()
                .filter_map(|l| l.strip_prefix('\t'))
                .map(|l| PathBuf::from(l.trim()))
                .collect();
            return Err(MergeError::LocalChanges(files));
        }
        if message.contains("Not possible to fast-forward") {
            return Err(MergeError::CannotFastForward);
        }
        if message.contains("Please tell me who you are")
            || message.contains("unable to auto-detect email")
            || message.contains("empty ident name")
        {
            return Err(MergeError::IdentityUnknown(message));
        }
        Err(MergeError::Refused(message))
    }
}
