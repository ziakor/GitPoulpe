use gitpoulpe_git::{FileHistory, FileRevisionView};

use crate::engine::Command;
use crate::state::State;

pub const HISTORY_LIMIT: usize = 500;

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
    pub show_content: bool,
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
        self.profiles_open = false;
        let request = self.file_history.request + 1;
        let revision_request = self.file_history.revision_request + 1;
        self.file_history = FileHistoryState {
            open: true,
            path,
            request,
            revision_request,
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

    pub fn file_history_show_content(&mut self, content: bool) {
        self.file_history.show_content = content;
        self.file_history.version += 1;
    }
}
