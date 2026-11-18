use std::ffi::OsString;
use std::path::PathBuf;

use gitpoulpe_core::{
    Profile, SessionInfo, Stash, StashApplied, StashDetail, StashFile, StashScope,
};

use crate::Git;
use crate::exec::{ExecError, ExecOut};
use crate::identity::profile_env;

const FIELD: u8 = 0x1f;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StashError {
    NothingToStash,
    InvalidScope,
    NoSuchStash,
    IdentityUnknown(String),
    OperationInProgress,
    Refused(String),
    Parse(String),
    Exec(ExecError),
}

impl std::fmt::Display for StashError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingToStash => f.write_str("aucun changement à mettre de côté pour ce périmètre"),
            Self::InvalidScope => f.write_str(
                "« seulement l'index » ne se combine ni avec les fichiers non suivis ni avec « garder l'index »",
            ),
            Self::NoSuchStash => f.write_str(
                "ce stash n'existe plus à cet emplacement : la liste a changé, rien n'a été modifié",
            ),
            Self::IdentityUnknown(m) => write!(f, "identité inconnue pour créer le stash : {m}"),
            Self::OperationInProgress => f.write_str(
                "une autre opération Git est en cours ou des fichiers sont en conflit : terminez-la d'abord",
            ),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Parse(m) => write!(f, "sortie Git inattendue : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StashError {}

impl From<ExecError> for StashError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<crate::conflict::ConflictError> for StashError {
    fn from(error: crate::conflict::ConflictError) -> Self {
        Self::Refused(error.to_string())
    }
}

fn valid_oid(id: &str) -> bool {
    (40..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn refused(out: &ExecOut) -> StashError {
    let message = out.stderr.clone();
    if message.contains("Please tell me who you are")
        || message.contains("unable to auto-detect email")
        || message.contains("empty ident name")
    {
        StashError::IdentityUnknown(message)
    } else {
        StashError::Refused(message)
    }
}

impl Git {
    pub async fn list_stashes(&self, info: &SessionInfo) -> Result<Vec<Stash>, StashError> {
        let out = self
            .exec_ok(info, &["stash", "list", "--format=%H%x1f%ct%x1f%gs%x00"])
            .await?;
        let mut stashes = Vec::new();
        for (index, record) in out
            .stdout
            .split(|b| *b == 0)
            .map(|r| r.strip_prefix(b"\n").unwrap_or(r))
            .filter(|r| !r.is_empty())
            .enumerate()
        {
            let mut fields = record.splitn(3, |b| *b == FIELD);
            let (Some(oid), Some(time), Some(subject)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(StashError::Parse("missing stash fields".to_owned()));
            };
            let subject = String::from_utf8_lossy(subject).into_owned();
            let (branch, message) = match subject
                .strip_prefix("WIP on ")
                .or_else(|| subject.strip_prefix("On "))
                .and_then(|rest| rest.split_once(": "))
            {
                Some((branch, message)) => (Some(branch.to_owned()), message.to_owned()),
                None => (None, subject.clone()),
            };
            stashes.push(Stash {
                index,
                oid: String::from_utf8_lossy(oid).into_owned(),
                branch,
                message,
                timestamp: String::from_utf8_lossy(time).parse().unwrap_or(0),
            });
        }
        Ok(stashes)
    }

    async fn stash_index(&self, info: &SessionInfo, oid: &str) -> Result<usize, StashError> {
        if !valid_oid(oid) {
            return Err(StashError::NoSuchStash);
        }
        self.list_stashes(info)
            .await?
            .into_iter()
            .find(|s| s.oid == oid)
            .map(|s| s.index)
            .ok_or(StashError::NoSuchStash)
    }

    pub async fn stash_detail(
        &self,
        info: &SessionInfo,
        oid: &str,
    ) -> Result<StashDetail, StashError> {
        self.stash_index(info, oid).await?;
        let base = format!("{oid}^1");
        let index_commit = format!("{oid}^2");
        let staged: Vec<PathBuf> = self
            .exec_ok(info, &["diff", "--name-only", "-z", &base, &index_commit])
            .await?
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned()))
            .collect();
        let changes = self
            .exec_ok(
                info,
                &["diff", "--name-status", "-z", "--no-renames", &base, oid],
            )
            .await?;
        let mut files = Vec::new();
        let mut parts = changes.stdout.split(|b| *b == 0).filter(|p| !p.is_empty());
        while let (Some(status), Some(path)) = (parts.next(), parts.next()) {
            let path = PathBuf::from(String::from_utf8_lossy(path).into_owned());
            files.push(StashFile {
                status: status.first().map_or('M', |b| *b as char),
                staged: staged.contains(&path),
                path,
            });
        }
        let third = format!("{oid}^3");
        let has_third = self
            .exec(info, &["rev-parse", "--verify", "-q", &third], &[], None)
            .await?
            .ok();
        let untracked = if has_third {
            self.exec_ok(info, &["ls-tree", "-r", "--name-only", "-z", &third])
                .await?
                .stdout
                .split(|b| *b == 0)
                .filter(|p| !p.is_empty())
                .map(|p| PathBuf::from(String::from_utf8_lossy(p).into_owned()))
                .collect()
        } else {
            Vec::new()
        };
        Ok(StashDetail { files, untracked })
    }

    pub async fn create_stash(
        &self,
        info: &SessionInfo,
        message: Option<&str>,
        scope: StashScope,
        profile: Option<&Profile>,
    ) -> Result<Stash, StashError> {
        if scope.staged_only && (scope.include_untracked || scope.keep_index) {
            return Err(StashError::InvalidScope);
        }
        if self.operation_state(info).await?.is_some() {
            return Err(StashError::OperationInProgress);
        }
        let before = self.list_stashes(info).await?;
        let mut args: Vec<String> = vec!["stash".into(), "push".into()];
        if scope.staged_only {
            args.push("--staged".into());
        }
        if scope.include_untracked {
            args.push("--include-untracked".into());
        }
        if scope.keep_index {
            args.push("--keep-index".into());
        }
        if let Some(message) = message.map(str::trim).filter(|m| !m.is_empty()) {
            args.push("-m".into());
            args.push(message.to_owned());
        }
        let mut env: Vec<(OsString, OsString)> = profile.map(profile_env).unwrap_or_default();
        env.push(("GIT_LITERAL_PATHSPECS".into(), "0".into()));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self.exec(info, &refs, &env, None).await?;
        if !out.ok() {
            return Err(refused(&out));
        }
        let after = self.list_stashes(info).await?;
        if after.len() <= before.len() {
            return Err(StashError::NothingToStash);
        }
        after.into_iter().next().ok_or(StashError::NothingToStash)
    }

    pub async fn apply_stash(
        &self,
        info: &SessionInfo,
        oid: &str,
        pop: bool,
        restore_index: bool,
    ) -> Result<StashApplied, StashError> {
        let index = self.stash_index(info, oid).await?;
        if self.operation_state(info).await?.is_some() {
            return Err(StashError::OperationInProgress);
        }
        let name = format!("stash@{{{index}}}");
        let mut args: Vec<&str> = vec!["stash", if pop { "pop" } else { "apply" }];
        if restore_index {
            args.push("--index");
        }
        args.push(&name);
        let out = self.exec(info, &args, &[], None).await?;
        let conflicts = self.list_conflicts(info).await?;
        if !conflicts.is_empty() {
            return Ok(StashApplied::Conflicts {
                files: conflicts.into_iter().map(|f| f.path).collect(),
            });
        }
        if !out.ok() {
            return Err(refused(&out));
        }
        Ok(StashApplied::Clean {
            dropped: pop && self.stash_index(info, oid).await.is_err(),
        })
    }

    pub async fn drop_stash(&self, info: &SessionInfo, oid: &str) -> Result<Stash, StashError> {
        let index = self.stash_index(info, oid).await?;
        let stash = self
            .list_stashes(info)
            .await?
            .into_iter()
            .find(|s| s.oid == oid)
            .ok_or(StashError::NoSuchStash)?;
        let name = format!("stash@{{{index}}}");
        let out = self
            .exec(info, &["stash", "drop", &name], &[], None)
            .await?;
        if !out.ok() {
            return Err(refused(&out));
        }
        Ok(stash)
    }
}
