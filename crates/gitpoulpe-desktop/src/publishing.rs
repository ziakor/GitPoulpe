use gitpoulpe_core::{PushKind, PushMode, PushOutcome, PushPreview};

use crate::engine::Command;
use crate::state::State;

pub struct PublishState {
    pub branch: String,
    pub remotes: Vec<String>,
    pub remote: Option<String>,
    pub preview: Option<PushPreview>,
    pub error: Option<String>,
    pub running: Option<(String, u8)>,
    pub set_upstream: bool,
    pub confirm_force: bool,
    pub done: Option<String>,
}

#[derive(Default)]
pub struct PublishSlot {
    pub state: Option<PublishState>,
    pub request: u64,
}

impl State {
    pub fn publish_ask(&mut self) {
        let Some(branch) = self.branches.selected.clone() else {
            return;
        };
        self.publishing.request += 1;
        self.publishing.state = Some(PublishState {
            branch: branch.clone(),
            remotes: Vec::new(),
            remote: None,
            preview: None,
            error: None,
            running: None,
            set_upstream: true,
            confirm_force: false,
            done: None,
        });
        self.branches.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::PublishOpen {
                request: self.publishing.request,
                branch,
            });
        }
    }

    pub fn publish_close(&mut self) {
        if self
            .publishing
            .state
            .as_ref()
            .is_some_and(|p| p.running.is_some())
            && let Some(engine) = &self.engine
        {
            engine.send(Command::NetCancel);
        }
        self.publishing.request += 1;
        self.publishing.state = None;
        self.branches.version += 1;
    }

    pub fn publish_choose_remote(&mut self, remote: &str) {
        let Some(p) = self.publishing.state.as_mut() else {
            return;
        };
        if p.running.is_some() || p.remote.as_deref() == Some(remote) {
            return;
        }
        p.remote = Some(remote.to_owned());
        p.preview = None;
        p.error = None;
        p.confirm_force = false;
        let branch = p.branch.clone();
        self.publishing.request += 1;
        self.branches.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::PushPreview {
                request: self.publishing.request,
                branch,
                remote: remote.to_owned(),
            });
        }
    }

    pub fn publish_toggle_upstream(&mut self) {
        if let Some(p) = self.publishing.state.as_mut() {
            p.set_upstream = !p.set_upstream;
            self.branches.version += 1;
        }
    }

    pub fn publish_ask_force(&mut self) {
        if let Some(p) = self.publishing.state.as_mut() {
            p.confirm_force = true;
            self.branches.version += 1;
        }
    }

    pub fn publish_cancel_force(&mut self) {
        if let Some(p) = self.publishing.state.as_mut() {
            p.confirm_force = false;
            self.branches.version += 1;
        }
    }

    pub fn publish_cancel_running(&mut self) {
        if let Some(engine) = &self.engine {
            engine.send(Command::NetCancel);
        }
    }

    pub fn publish_run(&mut self, protected: bool) {
        let Some(p) = self.publishing.state.as_mut() else {
            return;
        };
        let (Some(remote), Some(preview)) = (p.remote.clone(), p.preview.as_ref()) else {
            return;
        };
        if p.running.is_some() {
            return;
        }
        if protected && !p.confirm_force {
            return;
        }
        if !protected && matches!(preview.kind, PushKind::UpToDate | PushKind::Behind) {
            return;
        }
        let expected = protected
            .then(|| preview.known_remote_oid.clone())
            .flatten();
        p.running = Some(("Connexion".to_owned(), 0));
        p.error = None;
        p.done = None;
        p.confirm_force = false;
        let command = Command::Push {
            request: self.publishing.request,
            branch: p.branch.clone(),
            remote,
            mode: if protected {
                PushMode::ForceWithLease
            } else {
                PushMode::Normal
            },
            expected,
            set_upstream: p.set_upstream && preview.upstream.is_none(),
        };
        self.branches.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(command);
        }
    }

    pub fn apply_publish_prepared(
        &mut self,
        request: u64,
        remotes: Vec<String>,
        preview: Result<Box<PushPreview>, String>,
    ) -> bool {
        if request != self.publishing.request {
            return false;
        }
        let Some(p) = self.publishing.state.as_mut() else {
            return false;
        };
        p.remotes = remotes;
        match preview {
            Ok(preview) => {
                p.remote = Some(preview.remote.clone());
                p.preview = Some(*preview);
            }
            Err(message) => p.error = Some(message),
        }
        self.branches.version += 1;
        true
    }

    pub fn apply_push_previewed(
        &mut self,
        request: u64,
        result: Result<Box<PushPreview>, String>,
    ) -> bool {
        if request != self.publishing.request {
            return false;
        }
        let Some(p) = self.publishing.state.as_mut() else {
            return false;
        };
        match result {
            Ok(preview) => p.preview = Some(*preview),
            Err(message) => p.error = Some(message),
        }
        self.branches.version += 1;
        true
    }

    pub fn apply_push_progress(&mut self, request: u64, phase: String, percent: u8) -> bool {
        if request != self.publishing.request {
            return false;
        }
        match self.publishing.state.as_mut() {
            Some(p) if p.running.is_some() => p.running = Some((phase, percent)),
            _ => return false,
        }
        self.branches.version += 1;
        true
    }

    pub fn apply_push_finished(
        &mut self,
        request: u64,
        result: Result<PushOutcome, String>,
    ) -> bool {
        if request != self.publishing.request {
            return false;
        }
        let Some(p) = self.publishing.state.as_mut() else {
            return false;
        };
        p.running = None;
        let before = p.preview.as_ref().and_then(|v| v.known_remote_oid.clone());
        match result {
            Ok(outcome) => {
                p.done = Some(if outcome.up_to_date {
                    format!(
                        "{} est déjà à jour sur {} ({}).",
                        p.branch, outcome.remote, outcome.destination
                    )
                } else if outcome.verified {
                    format!(
                        "{} publiée sur {} ({}) ; la référence distante a été relue et correspond.",
                        p.branch, outcome.remote, outcome.destination
                    )
                } else {
                    format!(
                        "{} envoyée vers {} mais la référence distante relue ne correspond pas : le distant a modifié le résultat (hook ?).",
                        p.branch, outcome.remote
                    )
                });
                p.preview = None;
                let (remote, branch, pushed) = (
                    outcome.remote.clone(),
                    p.branch.clone(),
                    outcome.pushed_oid.clone(),
                );
                self.record_journal("push", before, Some(pushed), format!("{remote} {branch}"));
                self.notice = self.publishing.state.as_ref().and_then(|p| p.done.clone());
                self.reload_branches_public();
                self.request_refresh(false);
                if let (Some(p), Some(engine)) = (self.publishing.state.as_ref(), &self.engine)
                    && let Some(remote) = p.remote.clone()
                {
                    engine.send(Command::PushPreview {
                        request: self.publishing.request,
                        branch: p.branch.clone(),
                        remote,
                    });
                }
            }
            Err(message) => p.error = Some(message),
        }
        self.branches.version += 1;
        true
    }
}
