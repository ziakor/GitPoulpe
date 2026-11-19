use std::ffi::{OsStr, OsString};

use gitpoulpe_core::SessionInfo;

use crate::Git;
use crate::cancel::Cancel;
use crate::net::{NetError, NetProgress};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRange {
    pub head: String,
    pub merge_base: String,
    pub fetched: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullRangeError {
    InvalidInput,
    Net(NetError),
    Inaccessible(String),
    NoCommonAncestor,
    Git(String),
}

impl std::fmt::Display for PullRangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput => f.write_str("révision, branche ou numéro de pull request invalide"),
            Self::Net(e) => write!(f, "{e}"),
            Self::Inaccessible(what) => write!(
                f,
                "{what} est introuvable sur le dépôt distant : la pull request a peut-être été mise à jour ou supprimée ; la pull request n'a pas été chargée en entier"
            ),
            Self::NoCommonAncestor => f.write_str(
                "la pull request et sa branche cible n'ont aucun ancêtre commun : impossible de calculer ses changements",
            ),
            Self::Git(message) => write!(f, "git a échoué : {message}"),
        }
    }
}

impl std::error::Error for PullRangeError {}

fn valid_sha(id: &str) -> bool {
    (7..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn valid_branch(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 250
        && !branch.starts_with(['/', '-'])
        && !branch.contains("//")
        && !branch.contains("..")
        && branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

impl Git {
    async fn git_out(
        &self,
        info: &SessionInfo,
        args: &[&str],
    ) -> Result<(bool, String), PullRangeError> {
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let (run, out) = self
            .run_collect(dir, &refs, &Cancel::new())
            .await
            .map_err(|e| {
                PullRangeError::Git(match e {
                    crate::process::RunError::Spawn(e) | crate::process::RunError::Io(e) => {
                        e.to_string()
                    }
                    crate::process::RunError::Cancelled => "opération annulée".to_owned(),
                    crate::process::RunError::Rejected(why) => why,
                })
            })?;
        Ok((
            run.succeeded(),
            String::from_utf8_lossy(&out).trim().to_owned(),
        ))
    }

    async fn has_commit(&self, info: &SessionInfo, sha: &str) -> Result<bool, PullRangeError> {
        let spec = format!("{sha}^{{commit}}");
        Ok(self.git_out(info, &["cat-file", "-e", &spec]).await?.0)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_pull_range(
        &self,
        info: &SessionInfo,
        remote: &str,
        number: u64,
        base_ref: &str,
        base_sha: &str,
        head_sha: &str,
        cancel: &Cancel,
        mut on_progress: impl FnMut(NetProgress),
    ) -> Result<PullRange, PullRangeError> {
        if !valid_sha(base_sha)
            || !valid_sha(head_sha)
            || !valid_branch(base_ref)
            || remote.starts_with('-')
            || number == 0
        {
            return Err(PullRangeError::InvalidInput);
        }
        let remotes = self
            .list_remotes(info)
            .await
            .map_err(|e| PullRangeError::Net(NetError::Refused(e.to_string())))?;
        let found = remotes
            .into_iter()
            .find(|r| r.name == remote)
            .ok_or_else(|| {
                PullRangeError::Net(NetError::Refused("ce remote n'existe pas".to_owned()))
            })?;
        let url = found.fetch_urls.first().cloned();
        let dir = info
            .worktree
            .as_deref()
            .unwrap_or(&info.git_dir)
            .to_path_buf();
        let mut fetched = false;
        if !self.has_commit(info, head_sha).await? {
            let spec = format!("+refs/pull/{number}/head:refs/gitpoulpe/pull/{number}/head");
            self.network_ok(
                &dir,
                &[
                    "fetch",
                    "--progress",
                    "--no-tags",
                    "--no-auto-maintenance",
                    "--",
                    remote,
                    &spec,
                ],
                url.as_deref(),
                cancel,
                &mut on_progress,
            )
            .await
            .map_err(PullRangeError::Net)?;
            fetched = true;
            if !self.has_commit(info, head_sha).await? {
                return Err(PullRangeError::Inaccessible(format!(
                    "la révision {}",
                    &head_sha[..7]
                )));
            }
        }
        if !self.has_commit(info, base_sha).await? {
            let spec = format!("+refs/heads/{base_ref}:refs/gitpoulpe/pull/{number}/base");
            self.network_ok(
                &dir,
                &[
                    "fetch",
                    "--progress",
                    "--no-tags",
                    "--no-auto-maintenance",
                    "--",
                    remote,
                    &spec,
                ],
                url.as_deref(),
                cancel,
                &mut on_progress,
            )
            .await
            .map_err(PullRangeError::Net)?;
            fetched = true;
            if !self.has_commit(info, base_sha).await? {
                return Err(PullRangeError::Inaccessible(format!(
                    "la révision de base {}",
                    &base_sha[..7]
                )));
            }
        }
        let (ok, out) = self
            .git_out(info, &["merge-base", base_sha, head_sha])
            .await?;
        if !ok || out.is_empty() {
            return Err(PullRangeError::NoCommonAncestor);
        }
        let (_, head) = self
            .git_out(info, &["rev-parse", &format!("{head_sha}^{{commit}}")])
            .await?;
        Ok(PullRange {
            head,
            merge_base: out,
            fetched,
        })
    }
}
