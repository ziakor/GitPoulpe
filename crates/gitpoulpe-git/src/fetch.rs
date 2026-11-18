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
    pub updates: Vec<RefUpdate>,
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
        let remotes = self
            .list_remotes(info)
            .await
            .map_err(|e| NetError::Refused(e.to_string()))?;
        let found = remotes
            .into_iter()
            .find(|r| r.name == remote && !r.name.starts_with('-'))
            .ok_or_else(|| NetError::Refused("ce remote n'existe pas".to_owned()))?;
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
            updates: parse_porcelain(&String::from_utf8_lossy(&run.stdout)),
        })
    }
}
