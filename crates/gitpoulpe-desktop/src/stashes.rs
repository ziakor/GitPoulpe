use gitpoulpe_core::{Stash, StashDetail, StashScope};
use rune::TextEdit;

use crate::engine::{Command, StashDone, StashOp};
use crate::state::State;

pub struct StashesState {
    pub open: bool,
    pub request: u64,
    pub op_request: u64,
    pub detail_request: u64,
    pub stashes: Vec<Stash>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub detail: Option<StashDetail>,
    pub message: TextEdit,
    pub include_untracked: bool,
    pub keep_index: bool,
    pub staged_only: bool,
    pub restore_index: bool,
    pub confirm_drop: Option<String>,
    pub version: u32,
}

impl Default for StashesState {
    fn default() -> Self {
        Self {
            open: false,
            request: 0,
            op_request: 0,
            detail_request: 0,
            stashes: Vec::new(),
            loading: false,
            busy: false,
            error: None,
            selected: None,
            detail: None,
            message: TextEdit::new(""),
            include_untracked: false,
            keep_index: false,
            staged_only: false,
            restore_index: false,
            confirm_drop: None,
            version: 0,
        }
    }
}

impl StashesState {
    pub fn selected_stash(&self) -> Option<&Stash> {
        let oid = self.selected.as_deref()?;
        self.stashes.iter().find(|s| s.oid == oid)
    }
}

impl State {
    pub fn open_stashes(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.close_history();
        self.close_ignored();
        self.close_branches();
        self.close_tags();
        self.close_remotes();
        self.close_conflicts();
        self.close_reflog();
        self.close_undo();
        self.close_worktrees();
        self.close_hooks();
        self.close_lfs();
        self.close_submodules();
        self.close_gitflow();
        self.close_pulls();
        self.close_file_history();
        self.close_file_edit();
        self.close_tools();
        self.close_terminal();
        self.close_shortcuts();
        self.close_workspaces();
        self.close_favorites();
        self.close_overview();
        self.close_notices();
        self.profiles_open = false;
        self.stashes.open = true;
        self.reload_stashes();
    }

    pub fn close_stashes(&mut self) {
        self.stashes.open = false;
        self.stashes.confirm_drop = None;
    }

    pub fn reload_stashes(&mut self) {
        self.stashes.request += 1;
        self.stashes.loading = true;
        self.stashes.error = None;
        self.stashes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::StashList {
                request: self.stashes.request,
            });
        }
    }

    pub fn select_stash(&mut self, oid: &str) {
        self.stashes.selected = Some(oid.to_owned());
        self.stashes.detail = None;
        self.stashes.confirm_drop = None;
        self.stashes.detail_request += 1;
        self.stashes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::StashDetail {
                request: self.stashes.detail_request,
                oid: oid.to_owned(),
            });
        }
    }

    fn send_stash_op(&mut self, op: StashOp) {
        if self.stashes.busy {
            return;
        }
        self.stashes.busy = true;
        self.stashes.op_request += 1;
        self.stashes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::StashOp {
                request: self.stashes.op_request,
                op,
            });
        }
    }

    pub fn stash_toggle(&mut self, which: &str) {
        let s = &mut self.stashes;
        match which {
            "untracked" => {
                s.include_untracked = !s.include_untracked;
                if s.include_untracked {
                    s.staged_only = false;
                }
            }
            "keep" => {
                s.keep_index = !s.keep_index;
                if s.keep_index {
                    s.staged_only = false;
                }
            }
            "staged" => {
                s.staged_only = !s.staged_only;
                if s.staged_only {
                    s.include_untracked = false;
                    s.keep_index = false;
                }
            }
            "restore-index" => s.restore_index = !s.restore_index,
            _ => {}
        }
        s.version += 1;
    }

    pub fn stash_create(&mut self) {
        let profile = self.active_profile().cloned().map(Box::new);
        let scope = StashScope {
            include_untracked: self.stashes.include_untracked,
            keep_index: self.stashes.keep_index,
            staged_only: self.stashes.staged_only,
        };
        let message = self.stashes.message.text.trim().to_owned();
        self.send_stash_op(StashOp::Create {
            message: (!message.is_empty()).then_some(message),
            scope,
            profile,
        });
    }

    pub fn stash_apply(&mut self, pop: bool) {
        let Some(stash) = self.stashes.selected_stash() else {
            return;
        };
        let oid = stash.oid.clone();
        let restore_index = self.stashes.restore_index;
        self.send_stash_op(StashOp::Apply {
            oid,
            pop,
            restore_index,
        });
    }

    pub fn stash_ask_drop(&mut self) {
        if let Some(stash) = self.stashes.selected_stash() {
            self.stashes.confirm_drop = Some(stash.oid.clone());
            self.stashes.version += 1;
        }
    }

    pub fn stash_cancel_drop(&mut self) {
        self.stashes.confirm_drop = None;
        self.stashes.version += 1;
    }

    pub fn stash_drop(&mut self) {
        let Some(oid) = self.stashes.confirm_drop.take() else {
            return;
        };
        self.send_stash_op(StashOp::Drop { oid });
    }

    pub fn apply_stashes_listed(
        &mut self,
        request: u64,
        result: Result<Vec<Stash>, String>,
    ) -> bool {
        if request != self.stashes.request {
            return false;
        }
        self.stashes.loading = false;
        match result {
            Ok(list) => {
                if self
                    .stashes
                    .selected
                    .as_ref()
                    .is_some_and(|o| !list.iter().any(|s| &s.oid == o))
                {
                    self.stashes.selected = None;
                    self.stashes.detail = None;
                }
                self.stashes.stashes = list;
                self.stashes.error = None;
            }
            Err(message) => self.stashes.error = Some(message),
        }
        self.stashes.version += 1;
        true
    }

    pub fn apply_stash_detailed(
        &mut self,
        request: u64,
        oid: &str,
        result: Result<StashDetail, String>,
    ) -> bool {
        if request != self.stashes.detail_request || self.stashes.selected.as_deref() != Some(oid) {
            return false;
        }
        match result {
            Ok(detail) => self.stashes.detail = Some(detail),
            Err(message) => self.stashes.error = Some(message),
        }
        self.stashes.version += 1;
        true
    }

    pub fn apply_stash_finished(
        &mut self,
        request: u64,
        result: Result<StashDone, String>,
    ) -> bool {
        if request != self.stashes.op_request {
            return false;
        }
        self.stashes.busy = false;
        self.stashes.confirm_drop = None;
        match result {
            Ok(done) => {
                if let Some((action, before, after, detail)) = done.journal {
                    self.record_journal(&action, before, after, detail);
                }
                self.notice = Some(done.message);
                self.stashes.message = TextEdit::new("");
                self.reload_stashes();
                self.request_refresh(false);
                self.probe_operation();
                if done.conflicts {
                    self.open_conflicts();
                }
            }
            Err(message) => {
                self.notice = Some(format!("Stash non modifié : {message}"));
                self.reload_stashes();
            }
        }
        self.stashes.version += 1;
        true
    }
}
