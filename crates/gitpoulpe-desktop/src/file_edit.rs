use gitpoulpe_git::WorktreeFile;
use rune::TextEdit;

use crate::engine::Command;
use crate::state::State;

#[derive(Default)]
pub struct FileEditState {
    pub open: bool,
    pub path: String,
    pub request: u64,
    pub loading: bool,
    pub load_error: Option<String>,
    pub original: Option<WorktreeFile>,
    pub text: TextEdit,
    pub saving: bool,
    pub save_error: Option<String>,
    pub external: bool,
    pub notice: Option<String>,
    pub confirm_discard: bool,
    pub version: u32,
}

impl FileEditState {
    pub fn dirty(&self) -> bool {
        self.original
            .as_ref()
            .is_some_and(|o| o.text != self.text.text)
    }
}

impl State {
    pub fn open_file_edit(&mut self, path: String) {
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
        self.close_file_history();
        self.close_tools();
        self.close_terminal();
        self.close_shortcuts();
        self.close_workspaces();
        self.close_favorites();
        self.close_overview();
        self.close_notices();
        self.profiles_open = false;
        let request = self.file_edit.request + 1;
        self.file_edit = FileEditState {
            open: true,
            path,
            request,
            ..FileEditState::default()
        };
        self.load_file_edit();
    }

    pub fn load_file_edit(&mut self) {
        let screen = &mut self.file_edit;
        screen.request += 1;
        screen.loading = true;
        screen.load_error = None;
        screen.save_error = None;
        screen.external = false;
        screen.version += 1;
        let request = screen.request;
        let path = screen.path.clone();
        if let Some(engine) = &self.engine {
            engine.send(Command::FileEditRead { request, path });
        }
    }

    pub fn apply_file_edit_loaded(
        &mut self,
        request: u64,
        result: Result<WorktreeFile, String>,
    ) -> bool {
        let screen = &mut self.file_edit;
        if !screen.open || request != screen.request {
            return false;
        }
        screen.loading = false;
        match result {
            Ok(file) => {
                screen.text = TextEdit::new(file.text.clone());
                screen.original = Some(file);
            }
            Err(message) => {
                screen.original = None;
                screen.load_error = Some(message);
            }
        }
        screen.version += 1;
        true
    }

    pub fn file_edit_save(&mut self, overwrite: bool) {
        let screen = &mut self.file_edit;
        if screen.saving || screen.loading {
            return;
        }
        let Some(original) = screen.original.clone() else {
            return;
        };
        screen.request += 1;
        screen.saving = true;
        screen.save_error = None;
        screen.notice = None;
        screen.external = false;
        screen.version += 1;
        let request = screen.request;
        let text = screen.text.text.clone();
        if let Some(engine) = &self.engine {
            engine.send(Command::FileEditSave {
                request,
                original: Box::new(original),
                text,
                overwrite,
            });
        }
    }

    pub fn apply_file_edit_saved(
        &mut self,
        request: u64,
        result: Result<WorktreeFile, (String, bool)>,
    ) -> bool {
        let screen = &mut self.file_edit;
        if !screen.open || request != screen.request {
            return false;
        }
        screen.saving = false;
        let saved = match result {
            Ok(file) => {
                screen.notice = Some(format!("{} enregistré.", file.path));
                screen.original = Some(file);
                true
            }
            Err((message, external)) => {
                screen.save_error = Some(message);
                screen.external = external;
                false
            }
        };
        screen.version += 1;
        if saved {
            self.request_refresh(false);
        }
        true
    }

    pub fn file_edit_reload_from_disk(&mut self) {
        self.load_file_edit();
    }

    pub fn file_edit_keep_mine(&mut self) {
        self.file_edit.external = false;
        self.file_edit.save_error = None;
        self.file_edit.version += 1;
    }

    pub fn file_edit_close_request(&mut self) {
        if self.file_edit.dirty() && !self.file_edit.confirm_discard {
            self.file_edit.confirm_discard = true;
            self.file_edit.version += 1;
            return;
        }
        self.close_file_edit();
    }

    pub fn file_edit_cancel_close(&mut self) {
        self.file_edit.confirm_discard = false;
        self.file_edit.version += 1;
    }

    pub fn close_file_edit(&mut self) {
        let screen = &mut self.file_edit;
        screen.open = false;
        screen.loading = false;
        screen.saving = false;
        screen.confirm_discard = false;
        screen.request += 1;
    }
}
