use gitpoulpe_core::{Branch, BranchLoss};
use gitpoulpe_git::DeleteMode;
use rune::TextEdit;

use crate::engine::{BranchDone, BranchOp, Command};
use crate::state::State;

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

    pub fn select_branch(&mut self, name: &str) {
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
