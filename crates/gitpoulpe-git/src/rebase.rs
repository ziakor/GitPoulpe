use std::ffi::OsString;
use std::path::PathBuf;

use gitpoulpe_core::{
    PlanItem, Profile, RebaseAction, RebaseCommit, RebaseOutcome, RebasePlanInfo, SessionInfo,
};

use crate::Git;
use crate::exec::ExecError;
use crate::identity::{profile_env, signing_config};

const MAX_COMMITS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebaseError {
    NothingCommitted,
    InvalidBase,
    BaseNotAncestor,
    NothingToRebase,
    TooMany,
    HasMerges,
    OperationInProgress,
    HeadMoved,
    PlanMismatch,
    FirstCannotMeld,
    EmptyMessage(String),
    NothingKept,
    LocalChanges,
    IdentityUnknown(String),
    Refused(String),
    Exec(ExecError),
}

impl std::fmt::Display for RebaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NothingCommitted => f.write_str("le dépôt n'a encore aucun commit"),
            Self::InvalidBase => f.write_str("la base n'est pas un commit complet de ce dépôt"),
            Self::BaseNotAncestor => {
                f.write_str("la base n'est pas un ancêtre de la branche courante")
            }
            Self::NothingToRebase => f.write_str("aucun commit entre la base et la branche courante"),
            Self::TooMany => write!(f, "trop de commits (maximum {MAX_COMMITS})"),
            Self::HasMerges => f.write_str(
                "cette plage contient un commit de fusion : le rebase interactif des fusions n'est pas pris en charge",
            ),
            Self::OperationInProgress => f.write_str(
                "une autre opération Git est en cours ou des fichiers sont en conflit : terminez-la d'abord",
            ),
            Self::HeadMoved => f.write_str(
                "la branche courante a changé depuis l'aperçu : rien n'a été réécrit, relisez le plan",
            ),
            Self::PlanMismatch => f.write_str(
                "le plan ne contient pas exactement les commits de la plage, chacun une fois (utilisez « drop » pour en retirer un)",
            ),
            Self::FirstCannotMeld => f.write_str(
                "le premier commit conservé ne peut pas être « squash » ou « fixup » : il n'y a rien avant lui pour l'absorber",
            ),
            Self::EmptyMessage(oid) => write!(
                f,
                "le commit {} doit être reformulé (reword) mais son nouveau message est vide",
                oid.chars().take(8).collect::<String>()
            ),
            Self::NothingKept => f.write_str("le plan retire tous les commits : aucun ne serait conservé"),
            Self::LocalChanges => f.write_str(
                "des modifications locales empêchent le rebase : enregistrez-les ou mettez-les de côté ; rien n'a été réécrit",
            ),
            Self::IdentityUnknown(m) => write!(f, "identité inconnue pour réécrire les commits : {m}"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RebaseError {}

impl From<ExecError> for RebaseError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<crate::conflict::ConflictError> for RebaseError {
    fn from(error: crate::conflict::ConflictError) -> Self {
        Self::Refused(error.to_string())
    }
}

fn valid_oid(id: &str) -> bool {
    (40..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn validate_plan(plan: &[PlanItem], commits: &[RebaseCommit]) -> Result<(), RebaseError> {
    let mut expected: Vec<&str> = commits.iter().map(|c| c.oid.as_str()).collect();
    let mut given: Vec<&str> = plan.iter().map(|p| p.oid.as_str()).collect();
    expected.sort_unstable();
    given.sort_unstable();
    if expected != given {
        return Err(RebaseError::PlanMismatch);
    }
    let mut kept = plan.iter().filter(|p| p.action != RebaseAction::Drop);
    match kept.next() {
        None => return Err(RebaseError::NothingKept),
        Some(first) if first.action.melds() => return Err(RebaseError::FirstCannotMeld),
        Some(_) => {}
    }
    for item in plan {
        if item.action == RebaseAction::Reword
            && item.message.as_deref().is_none_or(|m| m.trim().is_empty())
        {
            return Err(RebaseError::EmptyMessage(item.oid.clone()));
        }
    }
    Ok(())
}

impl Git {
    pub async fn rebase_plan(
        &self,
        info: &SessionInfo,
        base: &str,
    ) -> Result<RebasePlanInfo, RebaseError> {
        let head = self
            .exec(
                info,
                &["rev-parse", "--verify", "-q", "HEAD^{commit}"],
                &[],
                None,
            )
            .await?;
        if !head.ok() {
            return Err(RebaseError::NothingCommitted);
        }
        let head = head.text();
        if !valid_oid(base) {
            return Err(RebaseError::InvalidBase);
        }
        let spec = format!("{base}^{{commit}}");
        let base_oid = self
            .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
            .await?;
        if !base_oid.ok() {
            return Err(RebaseError::InvalidBase);
        }
        let base_oid = base_oid.text();
        if !self
            .exec(
                info,
                &["merge-base", "--is-ancestor", &base_oid, &head],
                &[],
                None,
            )
            .await?
            .ok()
        {
            return Err(RebaseError::BaseNotAncestor);
        }
        let range = format!("{base_oid}..{head}");
        let log = self
            .exec_ok(
                info,
                &["rev-list", "--reverse", "--parents", "--format=%s", &range],
            )
            .await?;
        let text = String::from_utf8_lossy(&log.stdout).into_owned();
        let mut commits = Vec::new();
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            let Some(rest) = line.strip_prefix("commit ") else {
                continue;
            };
            let mut ids = rest.split_whitespace();
            let oid = ids.next().unwrap_or_default().to_owned();
            if ids.count() > 1 {
                return Err(RebaseError::HasMerges);
            }
            let subject = lines.next().unwrap_or_default().to_owned();
            commits.push(RebaseCommit { oid, subject });
        }
        if commits.is_empty() {
            return Err(RebaseError::NothingToRebase);
        }
        if commits.len() > MAX_COMMITS {
            return Err(RebaseError::TooMany);
        }
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
        Ok(RebasePlanInfo {
            branch,
            head,
            base: base_oid,
            commits,
            local_changes,
        })
    }

    pub async fn start_rebase(
        &self,
        info: &SessionInfo,
        base: &str,
        plan: &[PlanItem],
        expected_head: &str,
        autostash: bool,
        profile: Option<&Profile>,
    ) -> Result<RebaseOutcome, RebaseError> {
        if self.operation_state(info).await?.is_some() {
            return Err(RebaseError::OperationInProgress);
        }
        let details = self.rebase_plan(info, base).await?;
        if details.head != expected_head {
            return Err(RebaseError::HeadMoved);
        }
        validate_plan(plan, &details.commits)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let short: String = details.head.chars().take(8).collect();
        let marker = format!("refs/gitpoulpe/recovery/{stamp}-{short}-rebase");
        self.exec_ok(info, &["update-ref", &marker, &details.head])
            .await?;
        let work = tempfile::Builder::new()
            .prefix("gitpoulpe-rebase-")
            .tempdir()
            .map_err(|e| RebaseError::Refused(e.to_string()))?;
        let mut todo = String::new();
        for (index, item) in plan.iter().enumerate() {
            match item.action {
                RebaseAction::Reword => {
                    let file = work.path().join(format!("message-{index}"));
                    std::fs::write(&file, item.message.as_deref().unwrap_or_default())
                        .map_err(|e| RebaseError::Refused(e.to_string()))?;
                    todo.push_str(&format!("pick {}\n", item.oid));
                    todo.push_str(&format!(
                        "exec git commit --amend -q -F '{}'\n",
                        file.display()
                    ));
                }
                other => todo.push_str(&format!("{} {}\n", other.caption(), item.oid)),
            }
        }
        let todo_file = work.path().join("todo");
        std::fs::write(&todo_file, todo).map_err(|e| RebaseError::Refused(e.to_string()))?;
        let mut env: Vec<(OsString, OsString)> = profile.map(profile_env).unwrap_or_default();
        env.push(("GIT_EDITOR".into(), "true".into()));
        env.push((
            "GIT_SEQUENCE_EDITOR".into(),
            format!("cp '{}'", todo_file.display()).into(),
        ));
        let mut args: Vec<String> = profile
            .map(signing_config)
            .unwrap_or_default()
            .into_iter()
            .flat_map(|(k, v)| ["-c".to_owned(), format!("{k}={v}")])
            .collect();
        args.extend(["rebase".to_owned(), "-i".to_owned()]);
        if autostash {
            args.push("--autostash".to_owned());
        }
        args.push(details.base.clone());
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
                RebaseOutcome::Paused { marker }
            } else {
                RebaseOutcome::Conflicts { files, marker }
            });
        }
        if !out.ok() {
            let message = out.stderr.clone();
            let _ = self
                .exec(info, &["update-ref", "-d", &marker], &[], None)
                .await;
            if message.contains("unstaged changes")
                || message.contains("uncommitted changes")
                || message.contains("Your local changes")
            {
                return Err(RebaseError::LocalChanges);
            }
            if message.contains("Please tell me who you are")
                || message.contains("unable to auto-detect email")
            {
                return Err(RebaseError::IdentityUnknown(message));
            }
            return Err(RebaseError::Refused(message));
        }
        let head = self.exec_ok(info, &["rev-parse", "HEAD"]).await?.text();
        Ok(RebaseOutcome::Done { head, marker })
    }
}
