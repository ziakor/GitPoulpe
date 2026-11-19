use gitpoulpe_core::{ReflogEntry, ReflogInspect};
use rune::TextEdit;

use crate::engine::{BranchDone, Command};
use crate::state::State;

pub struct ReflogState {
    pub open: bool,
    pub request: u64,
    pub inspect_request: u64,
    pub op_request: u64,
    pub branch_scope: bool,
    pub entries: Vec<ReflogEntry>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub selected: Option<usize>,
    pub inspected: Option<ReflogInspect>,
    pub name: TextEdit,
    pub version: u32,
}

impl Default for ReflogState {
    fn default() -> Self {
        Self {
            open: false,
            request: 0,
            inspect_request: 0,
            op_request: 0,
            branch_scope: false,
            entries: Vec::new(),
            loading: false,
            busy: false,
            error: None,
            selected: None,
            inspected: None,
            name: TextEdit::new(""),
            version: 0,
        }
    }
}

impl ReflogState {
    pub fn selected_entry(&self) -> Option<&ReflogEntry> {
        self.entries.get(self.selected?)
    }
}

impl State {
    pub fn open_reflog(&mut self) {
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
        self.profiles_open = false;
        self.reflog.open = true;
        self.reload_reflog();
    }

    pub fn close_reflog(&mut self) {
        self.reflog.open = false;
    }

    fn reflog_reference(&self) -> String {
        if self.reflog.branch_scope
            && let Some(head) = self
                .repo
                .as_ref()
                .and_then(|r| r.snapshot.branch.head.clone())
        {
            return format!("refs/heads/{head}");
        }
        "HEAD".to_owned()
    }

    pub fn reflog_set_scope(&mut self, branch: bool) {
        self.reflog.branch_scope = branch;
        self.reload_reflog();
    }

    pub fn reload_reflog(&mut self) {
        self.reflog.request += 1;
        self.reflog.loading = true;
        self.reflog.error = None;
        self.reflog.selected = None;
        self.reflog.inspected = None;
        self.reflog.version += 1;
        let reference = self.reflog_reference();
        if let Some(engine) = &self.engine {
            engine.send(Command::ReflogList {
                request: self.reflog.request,
                reference,
            });
        }
    }

    pub fn select_reflog(&mut self, index: usize) {
        let Some(entry) = self.reflog.entries.get(index) else {
            return;
        };
        let oid = entry.oid.clone();
        self.reflog.selected = Some(index);
        self.reflog.inspected = None;
        self.reflog.name = TextEdit::new(format!(
            "recuperation-{}",
            oid.chars().take(8).collect::<String>()
        ));
        self.reflog.inspect_request += 1;
        self.reflog.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::ReflogInspect {
                request: self.reflog.inspect_request,
                oid,
            });
        }
    }

    pub fn reflog_recover(&mut self) {
        if self.reflog.busy {
            return;
        }
        let Some(entry) = self.reflog.selected_entry() else {
            return;
        };
        let name = self.reflog.name.text.trim().to_owned();
        if name.is_empty() {
            self.notice = Some("Donnez un nom à la branche de récupération.".to_owned());
            return;
        }
        let oid = entry.oid.clone();
        self.reflog.busy = true;
        self.reflog.op_request += 1;
        self.reflog.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::ReflogRecover {
                request: self.reflog.op_request,
                oid,
                name,
            });
        }
    }

    pub fn apply_reflog_listed(
        &mut self,
        request: u64,
        result: Result<Vec<ReflogEntry>, String>,
    ) -> bool {
        if request != self.reflog.request {
            return false;
        }
        self.reflog.loading = false;
        match result {
            Ok(entries) => self.reflog.entries = entries,
            Err(message) => self.reflog.error = Some(message),
        }
        self.reflog.version += 1;
        true
    }

    pub fn apply_reflog_inspected(
        &mut self,
        request: u64,
        result: Result<ReflogInspect, String>,
    ) -> bool {
        if request != self.reflog.inspect_request {
            return false;
        }
        match result {
            Ok(inspected) => self.reflog.inspected = Some(inspected),
            Err(message) => self.reflog.error = Some(message),
        }
        self.reflog.version += 1;
        true
    }

    pub fn apply_reflog_recovered(
        &mut self,
        request: u64,
        result: Result<BranchDone, String>,
    ) -> bool {
        if request != self.reflog.op_request {
            return false;
        }
        self.reflog.busy = false;
        match result {
            Ok(done) => {
                if let Some((action, before, after, detail)) = done.journal {
                    self.record_journal(&action, before, after, detail);
                }
                self.notice = Some(done.message);
            }
            Err(message) => self.notice = Some(format!("Récupération impossible : {message}")),
        }
        self.reflog.version += 1;
        true
    }
}
