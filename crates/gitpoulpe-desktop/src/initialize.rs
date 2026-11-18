use std::path::PathBuf;

use rune::TextEdit;

use crate::engine::Command;
use crate::state::State;

pub struct InitState {
    pub open: bool,
    pub path: PathBuf,
    pub branch: TextEdit,
    pub busy: bool,
    pub error: Option<String>,
    pub request: u64,
}

impl Default for InitState {
    fn default() -> Self {
        Self {
            open: false,
            path: PathBuf::new(),
            branch: TextEdit::new("main"),
            busy: false,
            error: None,
            request: 0,
        }
    }
}

impl State {
    pub fn begin_init(&mut self, path: PathBuf) {
        self.profiles_open = false;
        self.init.open = true;
        self.init.path = path.clone();
        self.init.branch = TextEdit::new("main");
        self.init.busy = false;
        self.init.error = None;
        self.init.request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::InitPrepare {
                request: self.init.request,
                path,
            });
        }
    }

    pub fn cancel_init(&mut self) {
        self.init.open = false;
        self.init.busy = false;
        self.init.request += 1;
    }

    pub fn confirm_init(&mut self) {
        if self.init.busy {
            return;
        }
        let branch = self.init.branch.text.trim().to_owned();
        self.init.busy = true;
        self.init.error = None;
        self.init.request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::Init {
                request: self.init.request,
                path: self.init.path.clone(),
                branch,
            });
        }
    }

    pub fn apply_init_prepared(&mut self, request: u64, branch: String) -> bool {
        if request != self.init.request || !self.init.open || self.init.busy {
            return false;
        }
        if self.init.branch.text == "main" {
            self.init.branch = TextEdit::new(branch);
        }
        true
    }

    pub fn apply_init_done(
        &mut self,
        request: u64,
        path: PathBuf,
        completed_existing: bool,
    ) -> bool {
        if request != self.init.request || !self.init.open {
            return false;
        }
        self.init.open = false;
        self.init.busy = false;
        self.open_path(path);
        if completed_existing {
            self.notice = Some("Création interrompue : le dépôt a été complété.".to_owned());
        }
        true
    }

    pub fn apply_init_failed(&mut self, request: u64, message: String) -> bool {
        if request != self.init.request || !self.init.open {
            return false;
        }
        self.init.busy = false;
        self.init.error = Some(message);
        true
    }
}
