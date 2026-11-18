use gitpoulpe_core::{Integration, PullKind, PullOutcome, PullPreview, PullStrategy};

use crate::engine::Command;
use crate::state::State;

pub struct PullUi {
    pub preview: Option<PullPreview>,
    pub strategy: PullStrategy,
    pub autostash: bool,
    pub running: Option<(String, u8)>,
    pub error: Option<String>,
    pub done: Option<String>,
}

#[derive(Default)]
pub struct PullSlot {
    pub state: Option<PullUi>,
    pub request: u64,
}

impl State {
    pub fn pull_ask(&mut self) {
        self.pulling.state = Some(PullUi {
            preview: None,
            strategy: PullStrategy::FastForwardOnly,
            autostash: false,
            running: None,
            error: None,
            done: None,
        });
        self.pull_refresh();
        self.branches.version += 1;
    }

    fn pull_refresh(&mut self) {
        self.pulling.request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::PullPreview {
                request: self.pulling.request,
            });
        }
    }

    pub fn pull_close(&mut self) {
        if self
            .pulling
            .state
            .as_ref()
            .is_some_and(|p| p.running.is_some())
            && let Some(engine) = &self.engine
        {
            engine.send(Command::NetCancel);
        }
        self.pulling.state = None;
        self.pulling.request += 1;
        self.branches.version += 1;
    }

    pub fn pull_set_strategy(&mut self, strategy: PullStrategy) {
        if let Some(p) = self.pulling.state.as_mut()
            && p.running.is_none()
        {
            p.strategy = strategy;
            self.branches.version += 1;
        }
    }

    pub fn pull_toggle_autostash(&mut self) {
        if let Some(p) = self.pulling.state.as_mut() {
            p.autostash = !p.autostash;
            self.branches.version += 1;
        }
    }

    pub fn pull_cancel_running(&mut self) {
        if let Some(engine) = &self.engine {
            engine.send(Command::NetCancel);
        }
    }

    pub fn pull_run(&mut self) {
        let profile = self.active_profile().cloned().map(Box::new);
        let Some(p) = self.pulling.state.as_mut() else {
            return;
        };
        let Some(preview) = p.preview.as_ref() else {
            return;
        };
        if p.running.is_some() {
            return;
        }
        p.running = Some(("Connexion".to_owned(), 0));
        p.error = None;
        p.done = None;
        let command = Command::Pull {
            request: self.pulling.request,
            strategy: p.strategy,
            expected_head: preview.head.clone(),
            autostash: p.autostash,
            profile,
        };
        self.branches.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(command);
        }
    }

    pub fn apply_pull_previewed(
        &mut self,
        request: u64,
        result: Result<Box<PullPreview>, String>,
    ) -> bool {
        if request != self.pulling.request {
            return false;
        }
        let Some(p) = self.pulling.state.as_mut() else {
            return false;
        };
        match result {
            Ok(preview) => p.preview = Some(*preview),
            Err(message) => p.error = Some(message),
        }
        self.branches.version += 1;
        true
    }

    pub fn apply_pull_progress(&mut self, request: u64, phase: String, percent: u8) -> bool {
        if request != self.pulling.request {
            return false;
        }
        match self.pulling.state.as_mut() {
            Some(p) if p.running.is_some() => p.running = Some((phase, percent)),
            _ => return false,
        }
        self.branches.version += 1;
        true
    }

    pub fn apply_pull_finished(
        &mut self,
        request: u64,
        result: Result<PullOutcome, String>,
    ) -> bool {
        if request != self.pulling.request {
            return false;
        }
        let Some(p) = self.pulling.state.as_mut() else {
            return false;
        };
        p.running = None;
        let strategy = p.strategy;
        let before = p.preview.as_ref().map(|v| v.head.clone());
        match result {
            Ok(outcome) => {
                let fetched = outcome.fetched_updates;
                let (message, moved, open) = match &outcome.integration {
                    Integration::UpToDate => (
                        format!("Déjà à jour ({fetched} référence(s) distante(s) mise(s) à jour)."),
                        None,
                        false,
                    ),
                    Integration::FastForward { to, .. } => (
                        "Intégré par avance rapide.".to_owned(),
                        Some(to.clone()),
                        false,
                    ),
                    Integration::Merged { to, .. } => (
                        "Intégré par une fusion.".to_owned(),
                        Some(to.clone()),
                        false,
                    ),
                    Integration::Rebased { to, marker, .. } => (
                        format!("Intégré par rebase. L'ancien sommet reste sous {marker}."),
                        Some(to.clone()),
                        false,
                    ),
                    Integration::Conflicts { files } => (
                        format!(
                            "L'intégration ({}) a produit des conflits dans {} fichier(s) : résolvez-les ou abandonnez.",
                            strategy.caption(),
                            files.len()
                        ),
                        None,
                        true,
                    ),
                    Integration::Paused => (
                        "L'intégration est en pause : continuez ou abandonnez.".to_owned(),
                        None,
                        true,
                    ),
                };
                if let Some(after) = moved {
                    self.record_journal("pull", before, Some(after), strategy.caption().to_owned());
                }
                self.notice = Some(message.clone());
                if let Some(p) = self.pulling.state.as_mut() {
                    p.done = Some(message);
                }
                self.request_refresh(false);
                self.probe_operation();
                self.reload_branches_public();
                if open {
                    self.pulling.state = None;
                    self.open_conflicts();
                } else {
                    self.pull_refresh();
                }
            }
            Err(message) => p.error = Some(message),
        }
        self.branches.version += 1;
        true
    }
}

pub fn kind_caption(kind: PullKind) -> &'static str {
    match kind {
        PullKind::UpToDate => "à jour",
        PullKind::Behind => "en retard",
        PullKind::Ahead => "en avance",
        PullKind::Diverged => "divergée",
    }
}
