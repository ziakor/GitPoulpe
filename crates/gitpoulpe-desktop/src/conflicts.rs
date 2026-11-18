use std::path::PathBuf;

use gitpoulpe_core::{ConflictFile, ConflictVersions, Operation, OperationKind, Resolution};
use gitpoulpe_git::ConflictSide;
use rune::TextEdit;

use crate::engine::{Command, ConflictDone, ConflictOp, ConflictSnapshot};
use crate::state::State;

pub struct ConflictsState {
    pub open: bool,
    pub request: u64,
    pub versions_request: u64,
    pub op_request: u64,
    pub operation: Option<Operation>,
    pub files: Vec<ConflictFile>,
    pub states: Vec<(PathBuf, Resolution)>,
    pub remembered: Vec<PathBuf>,
    pub selected: Option<PathBuf>,
    pub versions: Option<ConflictVersions>,
    pub result: TextEdit,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub confirm_abort: bool,
    pub markers_pending: bool,
    pub reload_pending: bool,
    pub version: u32,
}

impl Default for ConflictsState {
    fn default() -> Self {
        Self {
            open: false,
            request: 0,
            versions_request: 0,
            op_request: 0,
            operation: None,
            files: Vec::new(),
            states: Vec::new(),
            remembered: Vec::new(),
            selected: None,
            versions: None,
            result: TextEdit::new(""),
            loading: false,
            busy: false,
            error: None,
            confirm_abort: false,
            markers_pending: false,
            reload_pending: false,
            version: 0,
        }
    }
}

impl ConflictsState {
    pub fn state_of(&self, path: &std::path::Path) -> Resolution {
        self.states
            .iter()
            .find(|(p, _)| p == path)
            .map_or(Resolution::Unresolved, |(_, s)| *s)
    }

    pub fn ready(&self) -> bool {
        self.operation.is_some()
            && self.files.is_empty()
            && self.states.iter().all(|(_, s)| *s == Resolution::Resolved)
    }

    pub fn rows(&self) -> Vec<PathBuf> {
        let mut rows: Vec<PathBuf> = self.files.iter().map(|f| f.path.clone()).collect();
        for (path, _) in &self.states {
            if !rows.contains(path) {
                rows.push(path.clone());
            }
        }
        rows
    }

    pub fn remaining(&self) -> usize {
        self.states
            .iter()
            .filter(|(_, s)| *s != Resolution::Resolved)
            .count()
    }
}

impl State {
    pub fn probe_operation(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.probe_request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::OperationProbe {
                request: self.probe_request,
            });
        }
    }

    pub fn apply_operation_probed(&mut self, request: u64, kind: Option<OperationKind>) -> bool {
        if request != self.probe_request {
            return false;
        }
        let changed = self.operation_kind != kind;
        self.operation_kind = kind;
        if kind.is_none() && self.conflicts.open {
            self.close_conflicts();
        }
        if kind.is_none() {
            self.conflicts.remembered.clear();
        }
        changed
    }

    pub fn open_conflicts(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.close_history();
        self.close_ignored();
        self.close_branches();
        self.close_tags();
        self.close_remotes();
        self.close_stashes();
        self.close_reflog();
        self.close_undo();
        self.close_worktrees();
        self.profiles_open = false;
        self.conflicts.open = true;
        self.reload_conflicts();
    }

    pub fn close_conflicts(&mut self) {
        self.conflicts.open = false;
        self.conflicts.confirm_abort = false;
        self.conflicts.markers_pending = false;
    }

    pub fn reload_conflicts(&mut self) {
        self.conflicts.request += 1;
        self.conflicts.loading = true;
        self.conflicts.error = None;
        self.conflicts.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::ConflictLoad {
                request: self.conflicts.request,
                remembered: self.conflicts.remembered.clone(),
            });
        }
    }

    pub fn apply_conflicts_loaded(
        &mut self,
        request: u64,
        result: Result<Box<ConflictSnapshot>, String>,
    ) -> bool {
        if request != self.conflicts.request {
            return false;
        }
        self.conflicts.loading = false;
        let again = std::mem::take(&mut self.conflicts.reload_pending);
        match result {
            Ok(snapshot) => {
                let c = &mut self.conflicts;
                if snapshot.operation.is_none() {
                    c.remembered.clear();
                }
                for file in &snapshot.files {
                    if !c.remembered.contains(&file.path) {
                        c.remembered.push(file.path.clone());
                    }
                }
                c.operation = snapshot.operation;
                c.files = snapshot.files;
                c.states = snapshot.states;
                let rows = c.rows();
                if c.selected.as_ref().is_none_or(|s| !rows.contains(s)) {
                    c.selected = c.files.first().map(|f| f.path.clone());
                    c.versions = None;
                }
                c.error = None;
                if let Some(path) = c.selected.clone()
                    && c.versions.is_none()
                {
                    self.select_conflict(path);
                }
            }
            Err(message) => self.conflicts.error = Some(message),
        }
        self.conflicts.version += 1;
        if again {
            self.reload_conflicts();
        }
        true
    }

    pub fn select_conflict(&mut self, path: PathBuf) {
        self.conflicts.selected = Some(path.clone());
        self.conflicts.versions = None;
        self.conflicts.markers_pending = false;
        self.conflicts.result = TextEdit::new("");
        self.conflicts.versions_request += 1;
        self.conflicts.version += 1;
        if self.conflicts.state_of(&path) == Resolution::Unresolved
            && let Some(engine) = &self.engine
        {
            engine.send(Command::ConflictVersions {
                request: self.conflicts.versions_request,
                path,
            });
        }
    }

    pub fn apply_conflict_versions(
        &mut self,
        request: u64,
        result: Result<Box<ConflictVersions>, String>,
    ) -> bool {
        if request != self.conflicts.versions_request {
            return false;
        }
        match result {
            Ok(versions) => {
                self.conflicts.result = TextEdit::new(versions.merged.clone().unwrap_or_default());
                self.conflicts.versions = Some(*versions);
            }
            Err(message) => self.conflicts.error = Some(message),
        }
        self.conflicts.version += 1;
        true
    }

    fn send_conflict_op(&mut self, op: ConflictOp) {
        if self.conflicts.busy {
            return;
        }
        self.conflicts.busy = true;
        self.conflicts.op_request += 1;
        self.conflicts.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::ConflictOp {
                request: self.conflicts.op_request,
                op,
                remembered: self.conflicts.remembered.clone(),
            });
        }
    }

    pub fn conflict_take(&mut self, side: ConflictSide) {
        if let Some(path) = self.conflicts.selected.clone() {
            self.send_conflict_op(ConflictOp::Side { path, side });
        }
    }

    pub fn conflict_validate_result(&mut self, allow_markers: bool) {
        if let Some(path) = self.conflicts.selected.clone() {
            self.conflicts.markers_pending = false;
            let content = self.conflicts.result.text.clone();
            self.send_conflict_op(ConflictOp::Content {
                path,
                content,
                allow_markers,
            });
        }
    }

    pub fn conflict_revalidate(&mut self) {
        if let Some(path) = self.conflicts.selected.clone() {
            self.send_conflict_op(ConflictOp::Revalidate { path });
        }
    }

    pub fn conflict_skip(&mut self) {
        self.send_conflict_op(ConflictOp::Skip);
    }

    pub fn conflict_continue(&mut self) {
        if !self.conflicts.ready() {
            return;
        }
        let profile = self.active_profile().cloned().map(Box::new);
        self.send_conflict_op(ConflictOp::Continue { profile });
    }

    pub fn conflict_ask_abort(&mut self) {
        self.conflicts.confirm_abort = true;
        self.conflicts.version += 1;
    }

    pub fn conflict_cancel_abort(&mut self) {
        self.conflicts.confirm_abort = false;
        self.conflicts.markers_pending = false;
        self.conflicts.version += 1;
    }

    pub fn conflict_abort(&mut self) {
        if !self.conflicts.confirm_abort {
            return;
        }
        self.conflicts.confirm_abort = false;
        self.send_conflict_op(ConflictOp::Abort);
    }

    pub fn apply_conflict_finished(
        &mut self,
        request: u64,
        result: Result<ConflictDone, String>,
    ) -> bool {
        if request != self.conflicts.op_request {
            return false;
        }
        self.conflicts.busy = false;
        match result {
            Ok(done) => {
                self.notice = Some(done.message);
                if done.finished {
                    self.conflicts.remembered.clear();
                    self.close_conflicts();
                } else {
                    self.reload_conflicts();
                }
                self.request_refresh(false);
                self.probe_operation();
            }
            Err(message) if message == "MARKERS" => self.conflicts.markers_pending = true,
            Err(message) => {
                self.notice = Some(format!("Conflit non modifié : {message}"));
                self.reload_conflicts();
            }
        }
        self.conflicts.version += 1;
        true
    }
}
