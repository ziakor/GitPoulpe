use std::path::PathBuf;

use gitpoulpe_core::{Worktree, WorktreeRemoval};
use rune::TextEdit;

use crate::engine::{BranchDone, Command, WorktreeOp};
use crate::state::State;

pub struct WorktreesState {
    pub open: bool,
    pub request: u64,
    pub op_request: u64,
    pub removal_request: u64,
    pub list: Vec<Worktree>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub selected: Option<PathBuf>,
    pub new_path: TextEdit,
    pub new_branch: TextEdit,
    pub create_branch: bool,
    pub lock_reason: TextEdit,
    pub removal: Option<WorktreeRemoval>,
    pub version: u32,
}

impl Default for WorktreesState {
    fn default() -> Self {
        Self {
            open: false,
            request: 0,
            op_request: 0,
            removal_request: 0,
            list: Vec::new(),
            loading: false,
            busy: false,
            error: None,
            selected: None,
            new_path: TextEdit::new(""),
            new_branch: TextEdit::new(""),
            create_branch: true,
            lock_reason: TextEdit::new(""),
            removal: None,
            version: 0,
        }
    }
}

impl WorktreesState {
    pub fn selected_worktree(&self) -> Option<&Worktree> {
        let path = self.selected.as_ref()?;
        self.list.iter().find(|w| &w.path == path)
    }
}

impl State {
    pub fn open_worktrees(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.close_history();
        self.close_ignored();
        self.close_branches();
        self.close_tags();
        self.close_remotes();
        self.close_conflicts();
        self.close_stashes();
        self.close_reflog();
        self.close_undo();
        self.close_hooks();
        self.close_lfs();
        self.close_submodules();
        self.close_gitflow();
        self.close_pulls();
        self.close_file_history();
        self.close_file_edit();
        self.close_tools();
        self.close_terminal();
        self.profiles_open = false;
        self.worktrees.open = true;
        self.reload_worktrees();
    }

    pub fn close_worktrees(&mut self) {
        self.worktrees.open = false;
        self.worktrees.removal = None;
    }

    pub fn reload_worktrees(&mut self) {
        self.worktrees.request += 1;
        self.worktrees.loading = true;
        self.worktrees.error = None;
        self.worktrees.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::WorktreeList {
                request: self.worktrees.request,
            });
        }
    }

    pub fn select_worktree(&mut self, path: PathBuf) {
        self.worktrees.selected = Some(path);
        self.worktrees.removal = None;
        self.worktrees.version += 1;
    }

    pub fn worktree_toggle_create(&mut self) {
        self.worktrees.create_branch = !self.worktrees.create_branch;
        self.worktrees.version += 1;
    }

    fn send_worktree_op(&mut self, op: WorktreeOp) {
        if self.worktrees.busy {
            return;
        }
        self.worktrees.busy = true;
        self.worktrees.op_request += 1;
        self.worktrees.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::WorktreeOp {
                request: self.worktrees.op_request,
                op,
            });
        }
    }

    pub fn worktree_add(&mut self) {
        let path = self.worktrees.new_path.text.trim().to_owned();
        let branch = self.worktrees.new_branch.text.trim().to_owned();
        if path.is_empty() || branch.is_empty() {
            self.notice = Some("Donnez le chemin du worktree et une branche.".to_owned());
            return;
        }
        let create = self.worktrees.create_branch;
        self.send_worktree_op(WorktreeOp::Add {
            path: PathBuf::from(path),
            branch,
            create,
        });
    }

    pub fn worktree_open(&mut self) {
        if let Some(w) = self.worktrees.selected_worktree() {
            let path = w.path.clone();
            self.open_path(path);
        }
    }

    pub fn worktree_lock(&mut self) {
        if let Some(w) = self.worktrees.selected_worktree() {
            let path = w.path.clone();
            let reason = self.worktrees.lock_reason.text.trim().to_owned();
            self.send_worktree_op(WorktreeOp::Lock { path, reason });
        }
    }

    pub fn worktree_unlock(&mut self) {
        if let Some(w) = self.worktrees.selected_worktree() {
            let path = w.path.clone();
            self.send_worktree_op(WorktreeOp::Unlock { path });
        }
    }

    pub fn worktree_prune(&mut self) {
        self.send_worktree_op(WorktreeOp::Prune);
    }

    pub fn worktree_ask_remove(&mut self) {
        let Some(w) = self.worktrees.selected_worktree() else {
            return;
        };
        let path = w.path.clone();
        self.worktrees.removal = None;
        self.worktrees.removal_request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::WorktreeRemovalPreview {
                request: self.worktrees.removal_request,
                path,
            });
        }
    }

    pub fn worktree_cancel_remove(&mut self) {
        self.worktrees.removal = None;
        self.worktrees.version += 1;
    }

    pub fn worktree_remove(&mut self, discard: bool) {
        let Some(removal) = self.worktrees.removal.take() else {
            return;
        };
        let dirty = !removal.changed.is_empty() || !removal.untracked.is_empty();
        if dirty && !discard {
            self.worktrees.removal = Some(removal);
            return;
        }
        self.send_worktree_op(WorktreeOp::Remove {
            path: removal.path,
            discard,
        });
    }

    pub fn apply_worktrees_listed(
        &mut self,
        request: u64,
        result: Result<Vec<Worktree>, String>,
    ) -> bool {
        if request != self.worktrees.request {
            return false;
        }
        self.worktrees.loading = false;
        match result {
            Ok(list) => {
                if self
                    .worktrees
                    .selected
                    .as_ref()
                    .is_some_and(|p| !list.iter().any(|w| &w.path == p))
                {
                    self.worktrees.selected = None;
                }
                self.worktrees.list = list;
                self.worktrees.error = None;
            }
            Err(message) => self.worktrees.error = Some(message),
        }
        self.worktrees.version += 1;
        true
    }

    pub fn apply_worktree_removal_previewed(
        &mut self,
        request: u64,
        result: Result<WorktreeRemoval, String>,
    ) -> bool {
        if request != self.worktrees.removal_request {
            return false;
        }
        match result {
            Ok(removal) => self.worktrees.removal = Some(removal),
            Err(message) => self.notice = Some(message),
        }
        self.worktrees.version += 1;
        true
    }

    pub fn apply_worktree_finished(
        &mut self,
        request: u64,
        result: Result<BranchDone, String>,
    ) -> bool {
        if request != self.worktrees.op_request {
            return false;
        }
        self.worktrees.busy = false;
        match result {
            Ok(done) => {
                if let Some((action, before, after, detail)) = done.journal {
                    self.record_journal(&action, before, after, detail);
                }
                self.notice = Some(done.message);
                self.worktrees.new_path = TextEdit::new("");
                self.worktrees.new_branch = TextEdit::new("");
                self.worktrees.removal = None;
                self.reload_worktrees();
                self.reload_branches_public();
            }
            Err(message) => {
                self.notice = Some(format!("Worktree non modifié : {message}"));
                self.reload_worktrees();
            }
        }
        self.worktrees.version += 1;
        true
    }
}
