use std::path::PathBuf;

use gitpoulpe_core::{ReflogEntry, ReflogInspect, SessionInfo};

use crate::exec::ExecError;
use crate::{BranchError, Git};

pub const REFLOG_LIMIT: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReflogError {
    InvalidReference,
    ObjectUnavailable,
    NotACommit,
    Unreadable(String),
    Branch(BranchError),
    Exec(ExecError),
}

impl std::fmt::Display for ReflogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidReference => f.write_str("référence invalide"),
            Self::ObjectUnavailable => f.write_str(
                "cet objet n'est plus disponible dans le dépôt (expiré ou nettoyé) : rien n'a été récupéré",
            ),
            Self::NotACommit => f.write_str("cet objet n'est pas un commit"),
            Self::Unreadable(m) => write!(f, "reflog illisible : {m}"),
            Self::Branch(e) => write!(f, "{e}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReflogError {}

impl From<ExecError> for ReflogError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<BranchError> for ReflogError {
    fn from(error: BranchError) -> Self {
        Self::Branch(error)
    }
}

fn valid_reference(reference: &str) -> bool {
    reference == "HEAD"
        || reference.strip_prefix("refs/heads/").is_some_and(|name| {
            !name.is_empty() && !name.contains(['\0', '\n', '\r']) && !name.starts_with('-')
        })
}

fn valid_oid(id: &str) -> bool {
    (40..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

impl Git {
    async fn reflog_path(
        &self,
        info: &SessionInfo,
        reference: &str,
    ) -> Result<PathBuf, ReflogError> {
        if !valid_reference(reference) {
            return Err(ReflogError::InvalidReference);
        }
        let out = self
            .exec_ok(
                info,
                &[
                    "rev-parse",
                    "--path-format=absolute",
                    "--git-path",
                    &format!("logs/{reference}"),
                ],
            )
            .await?;
        Ok(PathBuf::from(out.text()))
    }

    pub async fn list_reflog(
        &self,
        info: &SessionInfo,
        reference: &str,
    ) -> Result<Vec<ReflogEntry>, ReflogError> {
        let path = self.reflog_path(info, reference).await?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(ReflogError::Unreadable(e.to_string())),
        };
        let mut raw: Vec<(String, String, String, i64)> = Vec::new();
        for line in text.lines() {
            let Some((head, message)) = line.split_once('\t') else {
                continue;
            };
            let mut fields = head.split(' ');
            let (Some(previous), Some(new)) = (fields.next(), fields.next()) else {
                continue;
            };
            if !valid_oid(previous) || !valid_oid(new) {
                continue;
            }
            let timestamp = head
                .rsplit(' ')
                .nth(1)
                .and_then(|t| t.parse::<i64>().ok())
                .unwrap_or(0);
            raw.push((
                previous.to_owned(),
                new.to_owned(),
                message.to_owned(),
                timestamp,
            ));
        }
        raw.reverse();
        raw.truncate(REFLOG_LIMIT);
        let oids: Vec<&str> = raw.iter().map(|(_, new, _, _)| new.as_str()).collect();
        let present = self.present_objects(info, &oids).await?;
        let prefix = if reference == "HEAD" {
            "HEAD".to_owned()
        } else {
            reference.trim_start_matches("refs/heads/").to_owned()
        };
        Ok(raw
            .into_iter()
            .enumerate()
            .map(|(index, (previous, new, message, timestamp))| {
                let action = message.split(':').next().unwrap_or_default().to_owned();
                ReflogEntry {
                    index,
                    selector: format!("{prefix}@{{{index}}}"),
                    available: present.contains(&new),
                    oid: new,
                    previous_oid: previous,
                    action,
                    message,
                    timestamp,
                }
            })
            .collect())
    }

    async fn present_objects(
        &self,
        info: &SessionInfo,
        oids: &[&str],
    ) -> Result<std::collections::HashSet<String>, ReflogError> {
        let mut present = std::collections::HashSet::new();
        if oids.is_empty() {
            return Ok(present);
        }
        let input = oids.iter().map(|o| format!("{o}\n")).collect::<String>();
        let out = self
            .exec(
                info,
                &["cat-file", "--batch-check"],
                &[],
                Some(input.into_bytes()),
            )
            .await?;
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut fields = line.split(' ');
            if let (Some(oid), Some(kind)) = (fields.next(), fields.next())
                && kind != "missing"
            {
                present.insert(oid.to_owned());
            }
        }
        Ok(present)
    }

    pub async fn inspect_reflog_entry(
        &self,
        info: &SessionInfo,
        oid: &str,
    ) -> Result<ReflogInspect, ReflogError> {
        if !valid_oid(oid) {
            return Err(ReflogError::InvalidReference);
        }
        let present = self.present_objects(info, &[oid]).await?;
        if !present.contains(oid) {
            return Ok(ReflogInspect {
                oid: oid.to_owned(),
                available: false,
                subject: String::new(),
                author: String::new(),
                timestamp: 0,
                parents: Vec::new(),
                reachable_from: Vec::new(),
            });
        }
        let spec = format!("{oid}^{{commit}}");
        let shown = self
            .exec(
                info,
                &[
                    "show",
                    "-s",
                    "--format=%an <%ae>%x1f%ct%x1f%P%x1f%s",
                    &spec,
                    "--",
                ],
                &[],
                None,
            )
            .await?;
        if !shown.ok() {
            return Err(ReflogError::NotACommit);
        }
        let text = shown.text();
        let mut fields = text.splitn(4, '\u{1f}');
        let (author, time, parents, subject) = (
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
            fields.next().unwrap_or_default(),
        );
        let contains = self
            .exec(
                info,
                &[
                    "for-each-ref",
                    "--contains",
                    oid,
                    "--format=%(refname:short)",
                    "refs/heads",
                    "refs/remotes",
                    "refs/tags",
                ],
                &[],
                None,
            )
            .await?;
        Ok(ReflogInspect {
            oid: oid.to_owned(),
            available: true,
            subject: subject.to_owned(),
            author: author.to_owned(),
            timestamp: time.parse().unwrap_or(0),
            parents: parents.split_whitespace().map(str::to_owned).collect(),
            reachable_from: contains.text().lines().map(str::to_owned).collect(),
        })
    }

    pub async fn create_recovery_branch(
        &self,
        info: &SessionInfo,
        oid: &str,
        name: &str,
    ) -> Result<(), ReflogError> {
        if !valid_oid(oid) {
            return Err(ReflogError::InvalidReference);
        }
        if !self.present_objects(info, &[oid]).await?.contains(oid) {
            return Err(ReflogError::ObjectUnavailable);
        }
        let spec = format!("{oid}^{{commit}}");
        let check = self
            .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
            .await?;
        if !check.ok() {
            return Err(ReflogError::NotACommit);
        }
        self.create_branch(info, name, Some(oid)).await?;
        Ok(())
    }
}
