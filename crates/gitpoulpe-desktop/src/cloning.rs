use std::path::PathBuf;

use gitpoulpe_git::{CloneError, repository_name_from_url};
use rune::TextEdit;

use crate::engine::Command;
use crate::state::State;

pub struct CloneState {
    pub open: bool,
    pub url: TextEdit,
    pub name: TextEdit,
    pub parent: Option<PathBuf>,
    pub busy: bool,
    pub progress: Option<(String, u8)>,
    pub error: Option<String>,
    pub request: u64,
}

impl Default for CloneState {
    fn default() -> Self {
        Self {
            open: false,
            url: TextEdit::new(""),
            name: TextEdit::new(""),
            parent: None,
            busy: false,
            progress: None,
            error: None,
            request: 0,
        }
    }
}

impl CloneState {
    pub fn proposed_name(&self) -> String {
        repository_name_from_url(&self.url.text)
    }
}

impl State {
    pub fn begin_clone(&mut self) {
        self.profiles_open = false;
        self.init.open = false;
        let parent = self.clone.parent.take();
        self.clone = CloneState {
            open: true,
            parent,
            ..CloneState::default()
        };
    }

    pub fn close_clone(&mut self) {
        if self.clone.busy {
            self.cancel_clone();
        }
        self.clone.open = false;
        self.clone.request += 1;
    }

    pub fn set_clone_parent(&mut self, path: PathBuf) {
        self.clone.parent = Some(path);
    }

    pub fn cancel_clone(&mut self) {
        if let Some(engine) = &self.engine {
            engine.send(Command::NetCancel);
        }
    }

    pub fn start_clone(&mut self) {
        if self.clone.busy {
            return;
        }
        let url = self.clone.url.text.trim().to_owned();
        if url.is_empty() {
            self.clone.error = Some("Saisissez l'URL du dépôt à cloner.".to_owned());
            return;
        }
        let Some(parent) = self.clone.parent.clone() else {
            self.clone.error = Some("Choisissez le dossier de destination.".to_owned());
            return;
        };
        let typed = self.clone.name.text.trim().to_owned();
        let name = if typed.is_empty() {
            self.clone.proposed_name()
        } else {
            typed
        };
        self.clone.busy = true;
        self.clone.error = None;
        self.clone.progress = Some(("Connexion".to_owned(), 0));
        self.clone.request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::Clone {
                request: self.clone.request,
                url,
                parent,
                name,
            });
        }
    }

    pub fn apply_clone_progress(&mut self, request: u64, phase: String, percent: u8) -> bool {
        if request != self.clone.request || !self.clone.busy {
            return false;
        }
        self.clone.progress = Some((phase, percent));
        true
    }

    pub fn apply_clone_done(&mut self, request: u64, result: Result<PathBuf, CloneError>) -> bool {
        if request != self.clone.request || !self.clone.open {
            return false;
        }
        self.clone.busy = false;
        self.clone.progress = None;
        match result {
            Ok(path) => {
                self.clone.open = false;
                self.open_path(path);
                self.notice = Some("Dépôt cloné et ouvert.".to_owned());
            }
            Err(error) => self.clone.error = Some(error.to_string()),
        }
        true
    }
}
