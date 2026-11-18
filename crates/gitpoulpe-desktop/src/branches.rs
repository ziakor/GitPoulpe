use gitpoulpe_core::{Branch, BranchLoss, MergeKind, MergeMode, MergeOutcome, MergePreview};
use gitpoulpe_git::DeleteMode;
use rune::TextEdit;

use crate::engine::{BranchDone, BranchOp, Command};
use crate::state::State;

pub struct MergeState {
    pub source: String,
    pub preview: Option<MergePreview>,
    pub error: Option<String>,
    pub busy: bool,
    pub no_ff: bool,
    pub autostash: bool,
}

pub struct BranchesState {
    pub open: bool,
    pub request: u64,
    pub op_request: u64,
    pub branches: Vec<Branch>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub new_name: TextEdit,
    pub rename_to: TextEdit,
    pub confirm: Option<DeleteConfirmation>,
    pub merge: Option<MergeState>,
    pub merge_request: u64,
    pub version: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteConfirmation {
    pub name: String,
    pub loss: BranchLoss,
}

impl Default for BranchesState {
    fn default() -> Self {
        Self {
            open: false,
            request: 0,
            op_request: 0,
            branches: Vec::new(),
            loading: false,
            busy: false,
            error: None,
            selected: None,
            new_name: TextEdit::new(""),
            rename_to: TextEdit::new(""),
            confirm: None,
            merge: None,
            merge_request: 0,
            version: 0,
        }
    }
}

impl BranchesState {
    pub fn selected_branch(&self) -> Option<&Branch> {
        let name = self.selected.as_deref()?;
        self.branches.iter().find(|b| b.name == name)
    }
}

impl State {
    pub fn open_branches(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.close_history();
        self.close_ignored();
        self.close_tags();
        self.close_remotes();
        self.close_conflicts();
        self.profiles_open = false;
        self.branches.open = true;
        self.reload_branches();
    }

    pub fn close_branches(&mut self) {
        self.branches.open = false;
        self.branches.confirm = None;
    }

    fn reload_branches(&mut self) {
        self.branches.request += 1;
        self.branches.loading = true;
        self.branches.error = None;
        self.branches.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::BranchList {
                request: self.branches.request,
            });
        }
    }

    pub fn merge_ask(&mut self) {
        let Some(branch) = self.branches.selected_branch() else {
            return;
        };
        let source = branch.name.clone();
        self.branches.merge = Some(MergeState {
            source: source.clone(),
            preview: None,
            error: None,
            busy: false,
            no_ff: false,
            autostash: false,
        });
        self.branches.merge_request += 1;
        self.branches.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::MergePreview {
                request: self.branches.merge_request,
                source,
            });
        }
    }

    pub fn merge_cancel(&mut self) {
        self.branches.merge = None;
        self.branches.merge_request += 1;
        self.branches.version += 1;
    }

    pub fn merge_toggle_no_ff(&mut self) {
        if let Some(m) = self.branches.merge.as_mut() {
            m.no_ff = !m.no_ff;
            self.branches.version += 1;
        }
    }

    pub fn merge_toggle_autostash(&mut self) {
        if let Some(m) = self.branches.merge.as_mut() {
            m.autostash = !m.autostash;
            self.branches.version += 1;
        }
    }

    pub fn merge_confirm(&mut self) {
        let profile = self.active_profile().cloned().map(Box::new);
        let Some(m) = self.branches.merge.as_mut() else {
            return;
        };
        let Some(preview) = m.preview.as_ref() else {
            return;
        };
        if m.busy || preview.kind == MergeKind::UpToDate {
            return;
        }
        m.busy = true;
        m.error = None;
        let command = Command::Merge {
            request: self.branches.merge_request,
            source: m.source.clone(),
            mode: if m.no_ff {
                MergeMode::NoFastForward
            } else {
                MergeMode::Auto
            },
            expected_head: preview.head.clone(),
            autostash: m.autostash,
            profile,
        };
        self.branches.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(command);
        }
    }

    pub fn apply_merge_previewed(
        &mut self,
        request: u64,
        result: Result<Box<MergePreview>, String>,
    ) -> bool {
        if request != self.branches.merge_request {
            return false;
        }
        let Some(m) = self.branches.merge.as_mut() else {
            return false;
        };
        match result {
            Ok(preview) => m.preview = Some(*preview),
            Err(message) => m.error = Some(message),
        }
        self.branches.version += 1;
        true
    }

    pub fn apply_merge_finished(
        &mut self,
        request: u64,
        result: Result<MergeOutcome, String>,
    ) -> bool {
        if request != self.branches.merge_request {
            return false;
        }
        let Some(m) = self.branches.merge.as_mut() else {
            return false;
        };
        m.busy = false;
        let source = m.source.clone();
        match result {
            Ok(outcome) => {
                self.branches.merge = None;
                let (message, journal) = match outcome {
                    MergeOutcome::UpToDate => (
                        format!("« {source} » est déjà dans la branche courante."),
                        None,
                    ),
                    MergeOutcome::FastForward { from, to } => (
                        format!(
                            "« {source} » a été avancée par avance rapide : aucun commit de fusion."
                        ),
                        Some((from, to)),
                    ),
                    MergeOutcome::Merged {
                        from,
                        to,
                        stash_kept,
                    } => (
                        if stash_kept {
                            format!(
                                "« {source} » fusionnée. Votre travail local est resté dans la réserve (git stash list) : la réapplication a produit des conflits."
                            )
                        } else {
                            format!("« {source} » fusionnée dans la branche courante.")
                        },
                        Some((from, to)),
                    ),
                    MergeOutcome::Conflicts { files } => (
                        format!(
                            "La fusion de « {source} » a produit des conflits dans {} fichier(s) : résolvez-les ou abandonnez la fusion.",
                            files.len()
                        ),
                        None,
                    ),
                };
                let conflicted = message.contains("a produit des conflits");
                if let Some((from, to)) = journal {
                    self.record_journal("merge", Some(from), Some(to), source);
                }
                self.notice = Some(message);
                self.request_refresh(false);
                self.probe_operation();
                if conflicted {
                    self.open_conflicts();
                } else {
                    self.reload_branches();
                }
            }
            Err(message) => {
                m.error = Some(message);
            }
        }
        self.branches.version += 1;
        true
    }

    pub fn select_branch(&mut self, name: &str) {
        self.branches.merge = None;
        self.branches.selected = Some(name.to_owned());
        self.branches.rename_to = TextEdit::new(name);
        self.branches.confirm = None;
        self.branches.version += 1;
    }

    fn send_branch_op(&mut self, op: BranchOp) {
        if self.branches.busy {
            return;
        }
        self.branches.busy = true;
        self.branches.op_request += 1;
        self.branches.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::BranchOp {
                request: self.branches.op_request,
                op,
            });
        }
    }

    pub fn branch_create(&mut self) {
        let name = self.branches.new_name.text.trim().to_owned();
        if name.is_empty() {
            self.notice = Some("Donnez un nom à la nouvelle branche.".to_owned());
            return;
        }
        self.send_branch_op(BranchOp::Create { name });
    }

    pub fn branch_switch(&mut self) {
        let Some(branch) = self.branches.selected_branch().filter(|b| !b.current) else {
            return;
        };
        let name = branch.name.clone();
        self.send_branch_op(BranchOp::Switch { name });
    }

    pub fn branch_rename(&mut self) {
        let Some(old) = self.branches.selected.clone() else {
            return;
        };
        let new = self.branches.rename_to.text.trim().to_owned();
        if new.is_empty() || new == old {
            self.notice = Some("Saisissez un nouveau nom différent de l'actuel.".to_owned());
            return;
        }
        self.send_branch_op(BranchOp::Rename { old, new });
    }

    pub fn branch_delete(&mut self) {
        let Some(branch) = self.branches.selected_branch().filter(|b| !b.current) else {
            return;
        };
        let name = branch.name.clone();
        self.send_branch_op(BranchOp::Delete {
            name,
            mode: DeleteMode::Merged,
        });
    }

    pub fn branch_delete_confirmed(&mut self) {
        let Some(confirm) = self.branches.confirm.take() else {
            return;
        };
        self.send_branch_op(BranchOp::Delete {
            name: confirm.name,
            mode: DeleteMode::Force {
                expected_tip: confirm.loss.tip,
            },
        });
    }

    pub fn branch_cancel_delete(&mut self) {
        self.branches.confirm = None;
        self.branches.version += 1;
    }

    pub fn apply_branches_listed(
        &mut self,
        request: u64,
        result: Result<Vec<Branch>, String>,
    ) -> bool {
        if request != self.branches.request {
            return false;
        }
        self.branches.loading = false;
        match result {
            Ok(list) => {
                if self
                    .branches
                    .selected
                    .as_ref()
                    .is_some_and(|n| !list.iter().any(|b| &b.name == n))
                {
                    self.branches.selected = None;
                }
                self.branches.branches = list;
                self.branches.error = None;
            }
            Err(message) => self.branches.error = Some(message),
        }
        self.branches.version += 1;
        true
    }

    pub fn apply_branch_done(&mut self, request: u64, result: Result<BranchDone, String>) -> bool {
        if request != self.branches.op_request {
            return false;
        }
        self.branches.busy = false;
        self.branches.confirm = None;
        match result {
            Ok(done) => {
                if let Some((action, before, after, detail)) = done.journal {
                    self.record_journal(&action, before, after, detail);
                }
                self.notice = Some(done.message);
                self.branches.new_name = TextEdit::new("");
                self.request_refresh(false);
                self.reload_branches();
            }
            Err(message) => {
                self.notice = Some(format!("Branche non modifiée : {message}"));
                self.reload_branches();
            }
        }
        self.branches.version += 1;
        true
    }

    pub fn apply_branch_needs_confirmation(
        &mut self,
        request: u64,
        name: String,
        loss: BranchLoss,
    ) -> bool {
        if request != self.branches.op_request {
            return false;
        }
        self.branches.busy = false;
        self.branches.confirm = Some(DeleteConfirmation { name, loss });
        self.branches.version += 1;
        true
    }
}
