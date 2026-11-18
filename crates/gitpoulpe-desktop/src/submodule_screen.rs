use std::path::PathBuf;

use gitpoulpe_core::Submodule;
use gitpoulpe_git::SubmoduleError;

use crate::engine::Command;
use crate::state::State;

#[derive(Default)]
pub struct SubmodulesState {
    pub open: bool,
    pub request: u64,
    pub root: Option<PathBuf>,
    pub list: Vec<Submodule>,
    pub loading: bool,
    pub running: Option<(String, u8)>,
    pub error: Option<String>,
    pub done: Option<String>,
    pub selected: Option<PathBuf>,
    pub needs_decision: Option<PathBuf>,
    pub version: u32,
}

impl State {
    pub fn open_submodules(&mut self) {
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
        self.close_worktrees();
        self.close_hooks();
        self.close_lfs();
        self.close_gitflow();
        self.profiles_open = false;
        self.submodules.open = true;
        self.reload_submodules();
    }

    pub fn close_submodules(&mut self) {
        if self.submodules.running.is_some()
            && let Some(engine) = &self.engine
        {
            engine.send(Command::NetCancel);
        }
        self.submodules.open = false;
    }

    pub fn reload_submodules(&mut self) {
        self.submodules.request += 1;
        self.submodules.loading = true;
        self.submodules.error = None;
        self.submodules.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::SubmoduleList {
                request: self.submodules.request,
            });
        }
    }

    pub fn select_submodule(&mut self, path: PathBuf) {
        self.submodules.selected = Some(path);
        self.submodules.needs_decision = None;
        self.submodules.version += 1;
    }

    pub fn submodule_run(&mut self, init_only: bool, confirm: bool) {
        if self.submodules.running.is_some() {
            return;
        }
        let Some(path) = self.submodules.selected.clone() else {
            return;
        };
        self.submodules.request += 1;
        self.submodules.running = Some(("Préparation".to_owned(), 0));
        self.submodules.error = None;
        self.submodules.done = None;
        self.submodules.needs_decision = None;
        self.submodules.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::SubmoduleRun {
                request: self.submodules.request,
                path,
                init_only,
                confirm,
            });
        }
    }

    pub fn submodule_cancel(&mut self) {
        if let Some(engine) = &self.engine {
            engine.send(Command::NetCancel);
        }
    }

    pub fn submodule_open(&mut self) {
        let (Some(path), Some(root)) = (
            self.submodules.selected.clone(),
            self.submodules.root.clone(),
        ) else {
            return;
        };
        self.open_path(root.join(path));
    }

    pub fn apply_submodules_listed(
        &mut self,
        request: u64,
        result: Result<(PathBuf, Vec<Submodule>), String>,
    ) -> bool {
        if request != self.submodules.request {
            return false;
        }
        self.submodules.loading = false;
        match result {
            Ok((root, list)) => {
                self.submodules.root = Some(root);
                if self
                    .submodules
                    .selected
                    .as_ref()
                    .is_some_and(|p| !list.iter().any(|s| &s.path == p))
                {
                    self.submodules.selected = None;
                }
                self.submodules.list = list;
            }
            Err(message) => self.submodules.error = Some(message),
        }
        self.submodules.version += 1;
        true
    }

    pub fn apply_submodule_progress(&mut self, request: u64, phase: String, percent: u8) -> bool {
        if request != self.submodules.request || self.submodules.running.is_none() {
            return false;
        }
        self.submodules.running = Some((phase, percent));
        self.submodules.version += 1;
        true
    }

    pub fn apply_submodule_finished(
        &mut self,
        request: u64,
        result: Result<String, SubmoduleError>,
    ) -> bool {
        if request != self.submodules.request {
            return false;
        }
        self.submodules.running = None;
        match result {
            Ok(message) => {
                self.submodules.done = Some(message);
                self.request_refresh(false);
                self.reload_submodules();
            }
            Err(SubmoduleError::UnsavedCommits(commits)) => {
                self.submodules.needs_decision = self.submodules.selected.clone();
                self.submodules.error = Some(SubmoduleError::UnsavedCommits(commits).to_string());
            }
            Err(error) => self.submodules.error = Some(error.to_string()),
        }
        self.submodules.version += 1;
        true
    }
}
