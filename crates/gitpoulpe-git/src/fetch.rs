use gitpoulpe_core::SessionInfo;

use crate::net::{NetError, NetProgress};
use crate::{Cancel, Git};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefUpdate {
    pub flag: char,
    pub old: String,
    pub new: String,
    pub name: String,
}

impl RefUpdate {
    pub fn is_new(&self) -> bool {
        self.flag == '*'
    }
    pub fn is_pruned(&self) -> bool {
        self.flag == '-'
    }
    pub fn is_rejected(&self) -> bool {
        self.flag == '!'
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchOutcome {
    pub remote: String,
    pub updates: Vec<RefUpdate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchTarget {
    Remote(String),
    Upstream(String),
}

fn parse_porcelain(text: &str) -> Vec<RefUpdate> {
    let mut updates = Vec::new();
    for line in text.lines() {
        let mut chars = line.chars();
        let Some(flag) = chars.next() else { continue };
        let rest = chars.as_str().trim_start();
        let mut fields = rest.splitn(3, ' ');
        let (Some(old), Some(new), Some(name)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if flag == '=' {
            continue;
        }
        updates.push(RefUpdate {
            flag,
            old: old.to_owned(),
            new: new.to_owned(),
            name: name.to_owned(),
        });
    }
    updates
}

impl Git {
    pub async fn fetch_remote(
        &self,
        info: &SessionInfo,
        remote: &str,
        cancel: &Cancel,
        on_progress: impl FnMut(NetProgress),
    ) -> Result<FetchOutcome, NetError> {
        self.fetch(
            info,
            &FetchTarget::Remote(remote.to_owned()),
            cancel,
            on_progress,
        )
        .await
    }

    pub async fn fetch(
        &self,
        info: &SessionInfo,
        target: &FetchTarget,
        cancel: &Cancel,
        on_progress: impl FnMut(NetProgress),
    ) -> Result<FetchOutcome, NetError> {
        let remotes = self
            .list_remotes(info)
            .await
            .map_err(|e| NetError::Refused(e.to_string()))?;
        let mut usable = remotes.into_iter().filter(|r| !r.name.starts_with('-'));
        let found = match target {
            FetchTarget::Remote(name) => usable
                .find(|r| &r.name == name)
                .ok_or_else(|| NetError::Refused("ce remote n'existe pas".to_owned()))?,
            FetchTarget::Upstream(upstream) => usable
                .filter(|r| {
                    upstream
                        .strip_prefix(r.name.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
                })
                .max_by_key(|r| r.name.len())
                .ok_or_else(|| {
                    NetError::Refused(
                        "la branche suivie n'appartient à aucun dépôt distant".to_owned(),
                    )
                })?,
        };
        let remote = found.name.as_str();
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let run = self
            .network_ok(
                dir,
                &[
                    "fetch",
                    "--progress",
                    "--porcelain",
                    "--no-auto-maintenance",
                    "--recurse-submodules=no",
                    "--",
                    remote,
                ],
                found.fetch_urls.first().map(String::as_str),
                cancel,
                on_progress,
            )
            .await?;
        Ok(FetchOutcome {
            remote: found.name.clone(),
            updates: parse_porcelain(&String::from_utf8_lossy(&run.stdout)),
        })
    }
}
