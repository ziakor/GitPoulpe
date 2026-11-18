use std::ffi::OsString;
use std::path::PathBuf;

use gitpoulpe_core::{
    EmptyPolicy, Profile, SequenceCommit, SequenceKind, SequenceOutcome, SequencePreview,
    SessionInfo,
};

use crate::Git;
use crate::exec::ExecError;
use crate::identity::{profile_env, signing_config};

const MAX_COMMITS: usize = 200;

#[derive(Clone, Copy, Debug)]
pub struct SequenceRequest<'a> {
    pub kind: SequenceKind,
    pub oids: &'a [String],
    pub mainline: Option<u32>,
    pub empty: EmptyPolicy,
    pub expected_head: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SequenceError {
    NothingSelected,
    TooMany,
    InvalidCommit(String),
    NothingCommitted,
    OperationInProgress,
    HeadMoved,
    MergeNeedsParent,
    MergesMustStandAlone,
    BadMainline,
    LocalChanges(Vec<PathBuf>),
    IdentityUnknown(String),
    Refused(String),
    Exec(ExecError),
}

impl std::fmt::Display for SequenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingSelected => f.write_str("aucun commit n'est sélectionné"),
            Self::TooMany => write!(f, "trop de commits (maximum {MAX_COMMITS})"),
            Self::InvalidCommit(oid) => write!(f, "{oid} n'est pas un commit de ce dépôt"),
            Self::NothingCommitted => f.write_str("le dépôt n'a encore aucun commit"),
            Self::OperationInProgress => f.write_str(
                "une autre opération Git est en cours ou des fichiers sont en conflit : terminez-la d'abord",
            ),
            Self::HeadMoved => f.write_str(
                "la branche courante a changé depuis l'aperçu : rien n'a été appliqué, relisez l'aperçu",
            ),
            Self::MergeNeedsParent => f.write_str(
                "un commit de fusion a plusieurs parents : choisissez explicitement le parent à garder",
            ),
            Self::MergesMustStandAlone => f.write_str(
                "les commits de fusion ne se mélangent pas avec d'autres commits dans une même séquence : faites-les séparément",
            ),
            Self::BadMainline => f.write_str("ce parent n'existe pas pour ce commit de fusion"),
            Self::LocalChanges(files) => write!(
                f,
                "vos modifications locales seraient écrasées ({}) : enregistrez-les ou mettez-les de côté ; rien n'a été appliqué",
                files.iter().take(5).map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            ),
            Self::IdentityUnknown(m) => write!(f, "identité inconnue pour créer les commits : {m}"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SequenceError {}

impl From<ExecError> for SequenceError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<crate::conflict::ConflictError> for SequenceError {
    fn from(error: crate::conflict::ConflictError) -> Self {
        Self::Refused(error.to_string())
    }
}

fn valid_oid(id: &str) -> bool {
    (40..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

impl Git {
    async fn sequence_commits(
        &self,
        info: &SessionInfo,
        oids: &[String],
    ) -> Result<Vec<SequenceCommit>, SequenceError> {
        if oids.is_empty() {
            return Err(SequenceError::NothingSelected);
        }
        if oids.len() > MAX_COMMITS {
            return Err(SequenceError::TooMany);
        }
        let mut commits = Vec::new();
        for oid in oids {
            if !valid_oid(oid) {
                return Err(SequenceError::InvalidCommit(oid.chars().take(12).collect()));
            }
            let spec = format!("{oid}^{{commit}}");
            let out = self
                .exec(
                    info,
                    &["show", "-s", "--format=%H%x1f%P%x1f%s", &spec, "--"],
                    &[],
                    None,
                )
                .await?;
            if !out.ok() {
                return Err(SequenceError::InvalidCommit(oid.chars().take(12).collect()));
            }
            let text = out.text();
            let mut fields = text.splitn(3, '\u{1f}');
            let (full, parents, subject) = (
                fields.next().unwrap_or_default(),
                fields.next().unwrap_or_default(),
                fields.next().unwrap_or_default(),
            );
            commits.push(SequenceCommit {
                oid: full.to_owned(),
                subject: subject.to_owned(),
                parents: parents.split_whitespace().count(),
            });
        }
        Ok(commits)
    }

    pub async fn sequence_preview(
        &self,
        info: &SessionInfo,
        kind: SequenceKind,
        oids: &[String],
    ) -> Result<SequencePreview, SequenceError> {
        let head = self
            .exec(
                info,
                &["rev-parse", "--verify", "-q", "HEAD^{commit}"],
                &[],
                None,
            )
            .await?;
        if !head.ok() {
            return Err(SequenceError::NothingCommitted);
        }
        let commits = self.sequence_commits(info, oids).await?;
        let branch = {
            let out = self
                .exec(info, &["symbolic-ref", "--short", "-q", "HEAD"], &[], None)
                .await?;
            out.ok().then(|| out.text())
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
        let merges = commits.iter().filter(|c| c.parents > 1).count();
        Ok(SequencePreview {
            kind,
            branch,
            head: head.text(),
            commits,
            merges,
            local_changes,
        })
    }

    pub async fn run_sequence(
        &self,
        info: &SessionInfo,
        request: &SequenceRequest<'_>,
        profile: Option<&Profile>,
    ) -> Result<SequenceOutcome, SequenceError> {
        let SequenceRequest {
            kind,
            oids,
            mainline,
            empty,
            expected_head,
        } = *request;
        if self.operation_state(info).await?.is_some() {
            return Err(SequenceError::OperationInProgress);
        }
        let preview = self.sequence_preview(info, kind, oids).await?;
        if preview.head != expected_head {
            return Err(SequenceError::HeadMoved);
        }
        if preview.merges > 0 {
            if preview.merges != preview.commits.len() {
                return Err(SequenceError::MergesMustStandAlone);
            }
            let Some(parent) = mainline else {
                return Err(SequenceError::MergeNeedsParent);
            };
            if preview
                .commits
                .iter()
                .any(|c| parent == 0 || parent as usize > c.parents)
            {
                return Err(SequenceError::BadMainline);
            }
        }
        let mut env: Vec<(OsString, OsString)> = profile.map(profile_env).unwrap_or_default();
        env.push(("GIT_EDITOR".into(), "true".into()));
        let mut args: Vec<String> = profile
            .map(signing_config)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|(k, v)| ["-c".to_owned(), format!("{k}={v}")])
            .collect();
        args.push(kind.git_name().to_owned());
        args.push("--no-edit".to_owned());
        if kind == SequenceKind::CherryPick {
            args.push(
                match empty {
                    EmptyPolicy::Drop => "--empty=drop",
                    EmptyPolicy::Keep => "--empty=keep",
                    EmptyPolicy::Stop => "--empty=stop",
                }
                .to_owned(),
            );
        }
        if preview.merges > 0
            && let Some(parent) = mainline
        {
            args.push("-m".to_owned());
            args.push(parent.to_string());
        }
        args.push("--".to_owned());
        args.extend(preview.commits.iter().map(|c| c.oid.clone()));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.exec(info, &refs, &env, None).await?;
        if self.operation_state(info).await?.is_some() {
            let files: Vec<PathBuf> = self
                .list_conflicts(info)
                .await?
                .into_iter()
                .map(|f| f.path)
                .collect();
            return Ok(if files.is_empty() {
                SequenceOutcome::Stopped { reason: out.stderr }
            } else {
                SequenceOutcome::Conflicts { files }
            });
        }
        if !out.ok() {
            let message = out.stderr.clone();
            if message.contains("would be overwritten") || message.contains("Your local changes") {
                let files = message
                    .lines()
                    .filter_map(|l| l.strip_prefix('\t'))
                    .map(|l| PathBuf::from(l.trim()))
                    .collect();
                return Err(SequenceError::LocalChanges(files));
            }
            if message.contains("Please tell me who you are")
                || message.contains("unable to auto-detect email")
                || message.contains("empty ident name")
            {
                return Err(SequenceError::IdentityUnknown(message));
            }
            return Err(SequenceError::Refused(message));
        }
        let head = self.exec_ok(info, &["rev-parse", "HEAD"]).await?.text();
        let created = self
            .exec_ok(
                info,
                &["rev-list", "--count", &format!("{expected_head}..{head}")],
            )
            .await?
            .text()
            .parse()
            .unwrap_or(0);
        Ok(SequenceOutcome::Done { created, head })
    }
}
