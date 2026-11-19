use gitpoulpe_core::{LfsFile, LfsStatus, LfsTransfer};

use crate::engine::Command;
use crate::state::State;

#[derive(Default)]
pub struct LfsState {
    pub open: bool,
    pub request: u64,
    pub status: Option<LfsStatus>,
    pub files: Vec<LfsFile>,
    pub loading: bool,
    pub running: Option<(LfsTransfer, String, u8)>,
    pub error: Option<String>,
    pub done: Option<String>,
    pub version: u32,
}

impl State {
    pub fn open_lfs(&mut self) {
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
        self.profiles_open = false;
        self.lfs.open = true;
        self.reload_lfs();
    }

    pub fn close_lfs(&mut self) {
        if self.lfs.running.is_some()
            && let Some(engine) = &self.engine
        {
            engine.send(Command::NetCancel);
        }
        self.lfs.open = false;
    }

    pub fn reload_lfs(&mut self) {
        self.lfs.request += 1;
        self.lfs.loading = true;
        self.lfs.error = None;
        self.lfs.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::LfsLoad {
                request: self.lfs.request,
            });
        }
    }

    pub fn lfs_run(&mut self, kind: LfsTransfer) {
        if self.lfs.running.is_some() {
            return;
        }
        let Some(branch) = self
            .repo
            .as_ref()
            .and_then(|r| r.snapshot.branch.head.clone())
        else {
            self.lfs.error =
                Some("Placez-vous sur une branche pour transférer des objets LFS.".to_owned());
            return;
        };
        self.lfs.request += 1;
        self.lfs.running = Some((kind, "Connexion".to_owned(), 0));
        self.lfs.error = None;
        self.lfs.done = None;
        self.lfs.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::LfsRun {
                request: self.lfs.request,
                kind,
                branch,
            });
        }
    }

    pub fn lfs_cancel(&mut self) {
        if let Some(engine) = &self.engine {
            engine.send(Command::NetCancel);
        }
    }

    pub fn apply_lfs_loaded(
        &mut self,
        request: u64,
        result: Result<Box<(LfsStatus, Vec<LfsFile>)>, String>,
    ) -> bool {
        if request != self.lfs.request {
            return false;
        }
        self.lfs.loading = false;
        match result {
            Ok(loaded) => {
                let (status, files) = *loaded;
                self.lfs.status = Some(status);
                self.lfs.files = files;
            }
            Err(message) => self.lfs.error = Some(message),
        }
        self.lfs.version += 1;
        true
    }

    pub fn apply_lfs_progress(&mut self, request: u64, phase: String, percent: u8) -> bool {
        if request != self.lfs.request {
            return false;
        }
        match self.lfs.running.as_mut() {
            Some((_, p, c)) => {
                *p = phase;
                *c = percent;
            }
            None => return false,
        }
        self.lfs.version += 1;
        true
    }

    pub fn apply_lfs_finished(&mut self, request: u64, result: Result<(), String>) -> bool {
        if request != self.lfs.request {
            return false;
        }
        let kind = self.lfs.running.take().map(|(k, _, _)| k);
        match result {
            Ok(()) => {
                self.lfs.done = kind.map(|k| format!("{} terminée.", capitalized(k.caption())));
                self.reload_lfs();
            }
            Err(message) => self.lfs.error = Some(message),
        }
        self.lfs.version += 1;
        true
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}
