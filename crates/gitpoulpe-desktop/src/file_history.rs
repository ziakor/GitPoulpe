use gitpoulpe_git::{Blame, FileHistory, FileRevisionView};

use crate::engine::Command;
use crate::state::State;

pub const HISTORY_LIMIT: usize = 500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PanelMode {
    #[default]
    Diff,
    Content,
    Blame,
}

#[derive(Default)]
pub struct FileHistoryState {
    pub open: bool,
    pub path: String,
    pub request: u64,
    pub revision_request: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub history: Option<FileHistory>,
    pub selected: Option<String>,
    pub revision: Option<FileRevisionView>,
    pub revision_loading: bool,
    pub revision_error: Option<String>,
    pub mode: PanelMode,
    pub blame: Option<Blame>,
    pub blame_loading: bool,
    pub blame_error: Option<String>,
    pub blame_notice: Option<String>,
    pub blame_request: u64,
    pub version: u32,
}

impl State {
    pub fn open_file_history(&mut self, path: String) {
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
        self.close_submodules();
        self.close_gitflow();
        self.close_pulls();
        self.close_file_edit();
        self.close_tools();
        self.close_terminal();
        self.profiles_open = false;
        let request = self.file_history.request + 1;
        let revision_request = self.file_history.revision_request + 1;
        let blame_request = self.file_history.blame_request + 1;
        self.file_history = FileHistoryState {
            open: true,
            path,
            request,
            revision_request,
            blame_request,
            ..FileHistoryState::default()
        };
        self.reload_file_history();
    }

    pub fn close_file_history(&mut self) {
        let screen = &mut self.file_history;
        screen.open = false;
        screen.loading = false;
        screen.revision_loading = false;
        screen.request += 1;
        screen.revision_request += 1;
        screen.blame_request += 1;
        screen.blame_loading = false;
    }

    pub fn reload_file_history(&mut self) {
        let screen = &mut self.file_history;
        screen.request += 1;
        screen.loading = true;
        screen.error = None;
        screen.notice = None;
        screen.version += 1;
        let request = screen.request;
        let path = screen.path.clone();
        if let Some(engine) = &self.engine {
            engine.send(Command::FileHistoryLoad {
                request,
                path,
                limit: HISTORY_LIMIT,
            });
        }
    }

    pub fn apply_file_history(
        &mut self,
        request: u64,
        result: Result<FileHistory, String>,
    ) -> bool {
        let screen = &mut self.file_history;
        if !screen.open || request != screen.request {
            return false;
        }
        screen.loading = false;
        match result {
            Ok(history) => {
                let kept = screen
                    .selected
                    .as_ref()
                    .is_some_and(|oid| history.revisions.iter().any(|r| &r.oid == oid));
                if screen.selected.is_some() && !kept {
                    screen.notice = Some(
                        "La révision sélectionnée n'est plus dans l'historique de ce fichier."
                            .to_owned(),
                    );
                    screen.selected = None;
                    screen.revision = None;
                    screen.revision_error = None;
                    screen.revision_loading = false;
                    screen.revision_request += 1;
                }
                screen.history = Some(history);
            }
            Err(message) => screen.error = Some(message),
        }
        screen.version += 1;
        true
    }

    pub fn select_file_revision(&mut self, oid: String) {
        let screen = &mut self.file_history;
        let Some(revision) = screen
            .history
            .as_ref()
            .and_then(|h| h.revisions.iter().find(|r| r.oid == oid))
            .cloned()
        else {
            return;
        };
        screen.selected = Some(oid.clone());
        screen.revision = None;
        screen.revision_error = None;
        screen.revision_loading = true;
        screen.revision_request += 1;
        screen.version += 1;
        let request = screen.revision_request;
        if let Some(engine) = &self.engine {
            engine.send(Command::FileRevisionLoad {
                request,
                oid,
                path: revision.path,
                old_path: revision.old_path,
            });
        }
        if self.file_history.mode == PanelMode::Blame {
            self.load_file_blame();
        }
    }

    pub fn apply_file_revision(
        &mut self,
        request: u64,
        result: Result<FileRevisionView, String>,
    ) -> bool {
        let screen = &mut self.file_history;
        if !screen.open || request != screen.revision_request {
            return false;
        }
        screen.revision_loading = false;
        match result {
            Ok(view) => screen.revision = Some(view),
            Err(message) => screen.revision_error = Some(message),
        }
        screen.version += 1;
        true
    }

    pub fn set_file_history_mode(&mut self, mode: PanelMode) {
        self.file_history.mode = mode;
        self.file_history.version += 1;
        if mode == PanelMode::Blame {
            self.load_file_blame();
        }
    }

    pub fn file_blame_of_working_copy(&mut self) {
        let screen = &mut self.file_history;
        screen.selected = None;
        screen.revision = None;
        screen.revision_error = None;
        screen.revision_loading = false;
        screen.revision_request += 1;
        self.set_file_history_mode(PanelMode::Blame);
    }

    pub fn load_file_blame(&mut self) {
        let screen = &mut self.file_history;
        let revision = screen
            .selected
            .as_ref()
            .and_then(|oid| {
                screen
                    .history
                    .as_ref()?
                    .revisions
                    .iter()
                    .find(|r| &r.oid == oid)
            })
            .map(|r| (r.oid.clone(), r.path.clone()));
        let (rev, path) = match revision {
            Some((oid, path)) => (Some(oid), path),
            None => (None, screen.path.clone()),
        };
        screen.blame_request += 1;
        screen.blame_loading = true;
        screen.blame_error = None;
        screen.blame_notice = None;
        screen.blame = None;
        screen.version += 1;
        let request = screen.blame_request;
        if let Some(engine) = &self.engine {
            engine.send(Command::FileBlameLoad {
                request,
                revision: rev,
                path,
            });
        }
    }

    pub fn apply_file_blame(&mut self, request: u64, result: Result<Blame, String>) -> bool {
        let screen = &mut self.file_history;
        if !screen.open || request != screen.blame_request {
            return false;
        }
        screen.blame_loading = false;
        match result {
            Ok(blame) => screen.blame = Some(blame),
            Err(message) => screen.blame_error = Some(message),
        }
        screen.version += 1;
        true
    }

    pub fn blame_open_commit(&mut self, oid: String) {
        let known = self
            .file_history
            .history
            .as_ref()
            .is_some_and(|h| h.revisions.iter().any(|r| r.oid == oid));
        if !known {
            self.file_history.blame_notice = Some(format!(
                "Le commit {} n'est pas dans la liste chargée de l'historique de ce fichier.",
                oid.chars().take(8).collect::<String>()
            ));
            self.file_history.version += 1;
            return;
        }
        self.file_history.mode = PanelMode::Diff;
        self.select_file_revision(oid);
    }

    pub fn file_history_external_diff(&mut self) {
        let screen = &self.file_history;
        let revision = screen
            .selected
            .as_ref()
            .and_then(|oid| {
                screen
                    .history
                    .as_ref()?
                    .revisions
                    .iter()
                    .find(|r| &r.oid == oid)
            })
            .cloned();
        match revision {
            Some(revision) => {
                self.tool_open_diff(Some(revision.oid), revision.path, revision.old_path)
            }
            None => {
                let path = screen.path.clone();
                self.tool_open_diff(None, path, None);
            }
        }
    }
}
