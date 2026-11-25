use gitpoulpe_core::{PlanItem, RebaseAction, RebaseOutcome, RebasePlanInfo};
use gitpoulpe_git::validate_plan;
use rune::TextEdit;

use crate::engine::Command;
use crate::state::State;

pub struct RebaseUi {
    pub base: String,
    pub info: Option<RebasePlanInfo>,
    pub items: Vec<PlanItem>,
    pub selected: Option<usize>,
    pub message: TextEdit,
    pub autostash: bool,
    pub error: Option<String>,
    pub busy: bool,
}

#[derive(Default)]
pub struct RebaseSlot {
    pub state: Option<RebaseUi>,
    pub request: u64,
}

const CYCLE: [RebaseAction; 6] = [
    RebaseAction::Pick,
    RebaseAction::Reword,
    RebaseAction::Edit,
    RebaseAction::Squash,
    RebaseAction::Fixup,
    RebaseAction::Drop,
];

impl RebaseUi {
    pub fn subject_of(&self, oid: &str) -> String {
        self.info
            .as_ref()
            .and_then(|i| i.commits.iter().find(|c| c.oid == oid))
            .map(|c| c.subject.clone())
            .unwrap_or_default()
    }

    fn sync_message(&mut self) {
        if let Some(index) = self.selected
            && let Some(item) = self.items.get_mut(index)
            && item.action == RebaseAction::Reword
        {
            item.message = Some(self.message.text.clone());
        }
    }

    pub fn problem(&self) -> Option<String> {
        let info = self.info.as_ref()?;
        validate_plan(&self.items, &info.commits)
            .err()
            .map(|e| e.to_string())
    }
}

impl State {
    pub fn rebase_ask(&mut self, base: &str) {
        self.rebasing.state = Some(RebaseUi {
            base: base.to_owned(),
            info: None,
            items: Vec::new(),
            selected: None,
            message: TextEdit::new(""),
            autostash: false,
            error: None,
            busy: false,
        });
        self.rebasing.request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::RebasePlan {
                request: self.rebasing.request,
                base: base.to_owned(),
            });
        }
    }

    pub fn rebase_close(&mut self) {
        self.rebasing.state = None;
        self.rebasing.request += 1;
    }

    pub fn rebase_select(&mut self, index: usize) {
        let Some(ui) = self.rebasing.state.as_mut() else {
            return;
        };
        ui.sync_message();
        if index >= ui.items.len() {
            return;
        }
        ui.selected = Some(index);
        let item = &ui.items[index];
        ui.message = TextEdit::new(
            item.message
                .clone()
                .unwrap_or_else(|| ui.subject_of(&item.oid)),
        );
    }

    pub fn rebase_cycle(&mut self, index: usize) {
        self.rebase_select(index);
        let Some(ui) = self.rebasing.state.as_mut() else {
            return;
        };
        if ui.busy {
            return;
        }
        let subject = ui.items.get(index).map(|i| ui.subject_of(&i.oid));
        let Some(item) = ui.items.get_mut(index) else {
            return;
        };
        let position = CYCLE.iter().position(|a| *a == item.action).unwrap_or(0);
        item.action = CYCLE[(position + 1) % CYCLE.len()];
        if item.action == RebaseAction::Reword {
            item.message = Some(subject.unwrap_or_default());
            ui.message = TextEdit::new(item.message.clone().unwrap_or_default());
        } else {
            item.message = None;
        }
    }

    pub fn rebase_move_up(&mut self, index: usize) {
        let Some(ui) = self.rebasing.state.as_mut() else {
            return;
        };
        if ui.busy || index == 0 || index >= ui.items.len() {
            return;
        }
        ui.sync_message();
        ui.items.swap(index - 1, index);
        ui.selected = Some(index - 1);
    }

    pub fn rebase_toggle_autostash(&mut self) {
        if let Some(ui) = self.rebasing.state.as_mut() {
            ui.autostash = !ui.autostash;
        }
    }

    pub fn rebase_run(&mut self) {
        let profile = self.active_profile().cloned().map(Box::new);
        let Some(ui) = self.rebasing.state.as_mut() else {
            return;
        };
        ui.sync_message();
        if ui.busy || ui.problem().is_some() {
            return;
        }
        let Some(info) = ui.info.as_ref() else {
            return;
        };
        ui.busy = true;
        ui.error = None;
        let command = Command::Rebase {
            request: self.rebasing.request,
            base: ui.base.clone(),
            items: ui.items.clone(),
            expected_head: info.head.clone(),
            autostash: ui.autostash,
            profile,
        };
        if let Some(engine) = &self.engine {
            engine.send(command);
        }
    }

    pub fn apply_rebase_planned(
        &mut self,
        request: u64,
        result: Result<Box<RebasePlanInfo>, String>,
    ) -> bool {
        if request != self.rebasing.request {
            return false;
        }
        let Some(ui) = self.rebasing.state.as_mut() else {
            return false;
        };
        match result {
            Ok(info) => {
                ui.items = info
                    .commits
                    .iter()
                    .map(|c| PlanItem {
                        oid: c.oid.clone(),
                        action: RebaseAction::Pick,
                        message: None,
                    })
                    .collect();
                ui.info = Some(*info);
            }
            Err(message) => ui.error = Some(message),
        }
        true
    }

    pub fn apply_rebase_finished(
        &mut self,
        request: u64,
        result: Result<RebaseOutcome, String>,
    ) -> bool {
        if request != self.rebasing.request {
            return false;
        }
        let Some(ui) = self.rebasing.state.as_mut() else {
            return false;
        };
        ui.busy = false;
        let before = ui.info.as_ref().map(|i| i.head.clone());
        match result {
            Ok(outcome) => {
                let (message, open) = match &outcome {
                    RebaseOutcome::Done { head, marker } => {
                        self.record_journal("rebase", before, Some(head.clone()), String::new());
                        (
                            format!(
                                "Rebase terminé. L'ancien sommet reste retrouvable sous {marker} (et dans le reflog)."
                            ),
                            false,
                        )
                    }
                    RebaseOutcome::Conflicts { files, marker } => (
                        format!(
                            "Le rebase s'est arrêté sur un conflit ({}). L'ancien sommet est gardé sous {marker} ; résolvez puis continuez, ou abandonnez.",
                            crate::widgets::counted(files.len(), "fichier", "fichiers")
                        ),
                        true,
                    ),
                    RebaseOutcome::Paused { marker } => (
                        format!(
                            "Le rebase est en pause (étape « edit »). L'ancien sommet est gardé sous {marker} ; modifiez le commit puis continuez, ou abandonnez."
                        ),
                        true,
                    ),
                };
                self.rebasing.state = None;
                self.notice = Some(message);
                self.request_refresh(false);
                self.probe_operation();
                if open {
                    self.open_conflicts();
                }
            }
            Err(message) => ui.error = Some(message),
        }
        true
    }
}
