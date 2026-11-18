use gitpoulpe_core::{SessionInfo, UndoDirection, UndoKind, UndoRecord, UndoState};

use crate::Git;
use crate::exec::ExecError;

const ZERO_OID: &str = "0000000000000000000000000000000000000000";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoError {
    NotReversible(String),
    WrongState,
    StateChanged { expected: String, found: String },
    ObjectMissing,
    OperationInProgress,
    CheckedOutElsewhere,
    CheckedOut,
    NameTaken,
    Refused(String),
    Exec(ExecError),
}

impl std::fmt::Display for UndoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReversible(why) => write!(f, "cette opération n'est pas réversible : {why}"),
            Self::WrongState => f.write_str("cette opération est déjà dans l'état demandé"),
            Self::StateChanged { expected, found } => write!(
                f,
                "l'état a changé depuis l'opération (attendu {expected}, trouvé {found}) : une modification faite ailleurs empêche l'annulation, rien n'a été touché"
            ),
            Self::ObjectMissing => f.write_str(
                "le commit à rétablir n'est plus dans le dépôt : rien n'a été modifié",
            ),
            Self::OperationInProgress => f.write_str(
                "une opération Git est en cours ou des fichiers sont en conflit : terminez-la d'abord",
            ),
            Self::CheckedOutElsewhere => f.write_str(
                "cette branche est extraite dans un autre worktree : elle n'est pas déplacée d'ici",
            ),
            Self::CheckedOut => f.write_str(
                "cette branche est la branche courante : elle ne peut pas être supprimée ou renommée ainsi",
            ),
            Self::NameTaken => f.write_str("un autre élément porte déjà ce nom : rien n'a été modifié"),
            Self::Refused(m) => write!(f, "Git refuse : {m}"),
            Self::Exec(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for UndoError {}

impl From<ExecError> for UndoError {
    fn from(error: ExecError) -> Self {
        Self::Exec(error)
    }
}

impl From<crate::conflict::ConflictError> for UndoError {
    fn from(error: crate::conflict::ConflictError) -> Self {
        Self::Refused(error.to_string())
    }
}

fn short(oid: &str) -> String {
    oid.chars().take(10).collect()
}

impl Git {
    async fn ref_value(
        &self,
        info: &SessionInfo,
        reference: &str,
    ) -> Result<Option<String>, UndoError> {
        let spec = if reference == "HEAD" {
            "HEAD".to_owned()
        } else {
            reference.to_owned()
        };
        let out = self
            .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
            .await?;
        Ok(out.ok().then(|| out.text()))
    }

    async fn checked_out_state(
        &self,
        info: &SessionInfo,
        branch_ref: &str,
    ) -> Result<(bool, bool), UndoError> {
        let head = self
            .exec(info, &["symbolic-ref", "-q", "HEAD"], &[], None)
            .await?;
        let here = head.ok() && head.text() == branch_ref;
        let format = self
            .exec(
                info,
                &["for-each-ref", "--format=%(worktreepath)", branch_ref],
                &[],
                None,
            )
            .await?
            .text();
        let used = !format.is_empty();
        Ok((here, used && !here))
    }

    pub async fn undo_redo(
        &self,
        info: &SessionInfo,
        record: &UndoRecord,
        direction: UndoDirection,
    ) -> Result<(), UndoError> {
        if let UndoKind::Irreversible { why } = &record.kind {
            return Err(UndoError::NotReversible(why.clone()));
        }
        match (direction, record.state) {
            (UndoDirection::Undo, UndoState::Undone) | (UndoDirection::Redo, UndoState::Done) => {
                return Err(UndoError::WrongState);
            }
            _ => {}
        }
        if self.operation_state(info).await?.is_some() {
            return Err(UndoError::OperationInProgress);
        }
        let undo = direction == UndoDirection::Undo;
        match &record.kind {
            UndoKind::RefMove {
                reference,
                before,
                after,
            } => {
                let (expected, target) = if undo {
                    (after, before)
                } else {
                    (before, after)
                };
                let found = self.ref_value(info, reference).await?.unwrap_or_default();
                if &found != expected {
                    return Err(UndoError::StateChanged {
                        expected: short(expected),
                        found: if found.is_empty() {
                            "absent".to_owned()
                        } else {
                            short(&found)
                        },
                    });
                }
                let spec = format!("{target}^{{commit}}");
                if !self
                    .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
                    .await?
                    .ok()
                {
                    return Err(UndoError::ObjectMissing);
                }
                let message = format!(
                    "gitpoulpe: {} {}",
                    if undo { "undo" } else { "redo" },
                    record.label
                );
                if reference == "HEAD" {
                    self.exec_ok(info, &["reset", "-q", "--keep", target])
                        .await?;
                } else {
                    let (here, elsewhere) = self.checked_out_state(info, reference).await?;
                    if elsewhere {
                        return Err(UndoError::CheckedOutElsewhere);
                    }
                    if here {
                        self.exec_ok(info, &["reset", "-q", "--keep", target])
                            .await?;
                    } else {
                        self.exec_ok(
                            info,
                            &["update-ref", "-m", &message, reference, target, expected],
                        )
                        .await?;
                    }
                }
                Ok(())
            }
            UndoKind::BranchCreate { name, tip } => {
                self.toggle_ref(info, &format!("refs/heads/{name}"), tip, undo)
                    .await
            }
            UndoKind::BranchDelete { name, tip } => {
                self.toggle_ref(info, &format!("refs/heads/{name}"), tip, !undo)
                    .await
            }
            UndoKind::TagCreate { name, object } => {
                self.toggle_ref(info, &format!("refs/tags/{name}"), object, undo)
                    .await
            }
            UndoKind::TagDelete { name, object } => {
                self.toggle_ref(info, &format!("refs/tags/{name}"), object, !undo)
                    .await
            }
            UndoKind::BranchRename { old, new } => {
                let (from, to) = if undo { (new, old) } else { (old, new) };
                let from_ref = format!("refs/heads/{from}");
                let to_ref = format!("refs/heads/{to}");
                if self.ref_value(info, &from_ref).await?.is_none() {
                    return Err(UndoError::StateChanged {
                        expected: from.clone(),
                        found: "absent".to_owned(),
                    });
                }
                if self.ref_value(info, &to_ref).await?.is_some() {
                    return Err(UndoError::NameTaken);
                }
                let (_, elsewhere) = self.checked_out_state(info, &from_ref).await?;
                if elsewhere {
                    return Err(UndoError::CheckedOutElsewhere);
                }
                self.exec_ok(info, &["branch", "-m", "--", from, to])
                    .await?;
                Ok(())
            }
            UndoKind::Irreversible { .. } => unreachable!("handled above"),
        }
    }

    async fn toggle_ref(
        &self,
        info: &SessionInfo,
        reference: &str,
        oid: &str,
        remove: bool,
    ) -> Result<(), UndoError> {
        let found = self.ref_value(info, reference).await?;
        if remove {
            match found {
                Some(current) if current == oid => {}
                other => {
                    return Err(UndoError::StateChanged {
                        expected: short(oid),
                        found: other.map_or_else(|| "absent".to_owned(), |c| short(&c)),
                    });
                }
            }
            if reference.starts_with("refs/heads/") {
                let (here, elsewhere) = self.checked_out_state(info, reference).await?;
                if here {
                    return Err(UndoError::CheckedOut);
                }
                if elsewhere {
                    return Err(UndoError::CheckedOutElsewhere);
                }
            }
            self.exec_ok(info, &["update-ref", "-d", reference, oid])
                .await?;
        } else {
            if found.is_some() {
                return Err(UndoError::NameTaken);
            }
            let spec = format!("{oid}^{{commit}}");
            let tag_ok = reference.starts_with("refs/tags/")
                && self
                    .exec(info, &["cat-file", "-e", oid], &[], None)
                    .await?
                    .ok();
            let commit_ok = self
                .exec(info, &["rev-parse", "--verify", "-q", &spec], &[], None)
                .await?
                .ok();
            if !(tag_ok || commit_ok) {
                return Err(UndoError::ObjectMissing);
            }
            self.exec_ok(info, &["update-ref", reference, oid, ZERO_OID])
                .await?;
        }
        Ok(())
    }
}
