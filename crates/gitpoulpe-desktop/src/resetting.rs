use gitpoulpe_core::{ResetMode, ResetOutcome, ResetPreview};

use crate::engine::Command;
use crate::state::State;

pub struct ResetState {
    pub target: String,
    pub mode: ResetMode,
    pub preview: Option<ResetPreview>,
    pub error: Option<String>,
    pub busy: bool,
    pub confirm: bool,
    pub done: Option<String>,
}

#[derive(Default)]
pub struct ResetSlot {
    pub state: Option<ResetState>,
    pub request: u64,
}

impl State {
    fn history_version_bump(&mut self) {
        self.history.version += 1;
    }

    pub fn reset_ask(&mut self, target: &str) {
        self.resetting.state = Some(ResetState {
            target: target.to_owned(),
            mode: ResetMode::Mixed,
            preview: None,
            error: None,
            busy: false,
            confirm: false,
            done: None,
        });
        self.request_reset_preview();
    }

    fn request_reset_preview(&mut self) {
        let Some(r) = self.resetting.state.as_mut() else {
            return;
        };
        r.preview = None;
        r.error = None;
        r.confirm = false;
        let (target, mode) = (r.target.clone(), r.mode);
        self.resetting.request += 1;
        self.history_version_bump();
        if let Some(engine) = &self.engine {
            engine.send(Command::ResetPreview {
                request: self.resetting.request,
                target,
                mode,
            });
        }
    }

    pub fn reset_close(&mut self) {
        self.resetting.state = None;
        self.resetting.request += 1;
        self.history_version_bump();
    }

    pub fn reset_set_mode(&mut self, mode: ResetMode) {
        let Some(r) = self.resetting.state.as_mut() else {
            return;
        };
        if r.mode == mode || r.busy {
            return;
        }
        r.mode = mode;
        self.request_reset_preview();
    }

    pub fn reset_run(&mut self) {
        let Some(r) = self.resetting.state.as_mut() else {
            return;
        };
        let Some(preview) = r.preview.as_ref() else {
            return;
        };
        if r.busy {
            return;
        }
        let destructive = !preview.would_lose.is_empty();
        if destructive && !r.confirm {
            r.confirm = true;
            self.history_version_bump();
            return;
        }
        r.busy = true;
        r.error = None;
        let command = Command::Reset {
            request: self.resetting.request,
            target: r.target.clone(),
            mode: r.mode,
            expected_head: preview.head.clone(),
            confirmed: r.confirm,
        };
        self.history_version_bump();
        if let Some(engine) = &self.engine {
            engine.send(command);
        }
    }

    pub fn reset_cancel_confirm(&mut self) {
        if let Some(r) = self.resetting.state.as_mut() {
            r.confirm = false;
            self.history_version_bump();
        }
    }

    pub fn apply_reset_previewed(
        &mut self,
        request: u64,
        result: Result<Box<ResetPreview>, String>,
    ) -> bool {
        if request != self.resetting.request {
            return false;
        }
        let Some(r) = self.resetting.state.as_mut() else {
            return false;
        };
        match result {
            Ok(preview) => r.preview = Some(*preview),
            Err(message) => r.error = Some(message),
        }
        self.history_version_bump();
        true
    }

    pub fn apply_reset_finished(
        &mut self,
        request: u64,
        result: Result<ResetOutcome, String>,
    ) -> bool {
        if request != self.resetting.request {
            return false;
        }
        let Some(r) = self.resetting.state.as_mut() else {
            return false;
        };
        r.busy = false;
        r.confirm = false;
        match result {
            Ok(outcome) => {
                let mode = match outcome.mode {
                    ResetMode::Soft => "soft",
                    ResetMode::Mixed => "mixed",
                    ResetMode::Hard => "hard",
                };
                let work = outcome.work_marker.as_ref().map_or_else(
                    String::new,
                    |m| {
                        format!(
                            " Vos modifications suivies non enregistrées sont sauvegardées sous {m} (git stash apply <id>)."
                        )
                    },
                );
                let message = format!(
                    "Reset {mode} fait. L'ancien sommet {} reste retrouvable : {} (et dans le reflog).{work}",
                    outcome.old_head.chars().take(8).collect::<String>(),
                    outcome.head_marker
                );
                r.done = Some(message.clone());
                r.preview = None;
                let (old, new) = (outcome.old_head.clone(), outcome.new_head.clone());
                self.record_journal("reset", Some(old), Some(new), mode.to_owned());
                self.notice = Some(message);
                self.request_refresh(false);
            }
            Err(message) => r.error = Some(message),
        }
        self.history_version_bump();
        true
    }
}
