use gitpoulpe_core::{EmptyPolicy, SequenceKind, SequenceOutcome, SequencePreview};

use crate::engine::Command;
use crate::state::State;

pub struct SequenceState {
    pub kind: SequenceKind,
    pub queue: Vec<(String, String)>,
    pub preview: Option<SequencePreview>,
    pub mainline: Option<u32>,
    pub empty: EmptyPolicy,
    pub error: Option<String>,
    pub busy: bool,
}

#[derive(Default)]
pub struct SequenceSlot {
    pub state: Option<SequenceState>,
    pub request: u64,
}

impl State {
    pub fn sequence_add(&mut self, kind: SequenceKind, oid: &str, subject: &str) {
        let state = self.sequencing.state.get_or_insert_with(|| SequenceState {
            kind,
            queue: Vec::new(),
            preview: None,
            mainline: None,
            empty: EmptyPolicy::Drop,
            error: None,
            busy: false,
        });
        if state.busy {
            return;
        }
        if state.kind != kind {
            state.kind = kind;
            state.queue.clear();
        }
        if !state.queue.iter().any(|(o, _)| o == oid) {
            state.queue.push((oid.to_owned(), subject.to_owned()));
        }
        self.sequence_refresh();
    }

    fn sequence_refresh(&mut self) {
        let Some(state) = self.sequencing.state.as_mut() else {
            return;
        };
        state.preview = None;
        state.error = None;
        let (kind, oids) = (
            state.kind,
            state
                .queue
                .iter()
                .map(|(o, _)| o.clone())
                .collect::<Vec<_>>(),
        );
        self.sequencing.request += 1;
        if oids.is_empty() {
            return;
        }
        if let Some(engine) = &self.engine {
            engine.send(Command::SequencePreview {
                request: self.sequencing.request,
                kind,
                oids,
            });
        }
    }

    pub fn sequence_remove(&mut self, index: usize) {
        let Some(state) = self.sequencing.state.as_mut() else {
            return;
        };
        if state.busy || index >= state.queue.len() {
            return;
        }
        state.queue.remove(index);
        if state.queue.is_empty() {
            self.sequence_close();
        } else {
            self.sequence_refresh();
        }
    }

    pub fn sequence_move_up(&mut self, index: usize) {
        let Some(state) = self.sequencing.state.as_mut() else {
            return;
        };
        if state.busy || index == 0 || index >= state.queue.len() {
            return;
        }
        state.queue.swap(index - 1, index);
        self.sequence_refresh();
    }

    pub fn sequence_close(&mut self) {
        self.sequencing.state = None;
        self.sequencing.request += 1;
    }

    pub fn sequence_set_mainline(&mut self, parent: u32) {
        if let Some(state) = self.sequencing.state.as_mut() {
            state.mainline = Some(parent);
        }
    }

    pub fn sequence_set_empty(&mut self, policy: EmptyPolicy) {
        if let Some(state) = self.sequencing.state.as_mut() {
            state.empty = policy;
        }
    }

    pub fn sequence_run(&mut self) {
        let profile = self.active_profile().cloned().map(Box::new);
        let Some(state) = self.sequencing.state.as_mut() else {
            return;
        };
        let Some(preview) = state.preview.as_ref() else {
            return;
        };
        if state.busy {
            return;
        }
        state.busy = true;
        state.error = None;
        let command = Command::Sequence {
            request: self.sequencing.request,
            kind: state.kind,
            oids: state.queue.iter().map(|(o, _)| o.clone()).collect(),
            mainline: state.mainline,
            empty: state.empty,
            expected_head: preview.head.clone(),
            profile,
        };
        if let Some(engine) = &self.engine {
            engine.send(command);
        }
    }

    pub fn apply_sequence_previewed(
        &mut self,
        request: u64,
        result: Result<Box<SequencePreview>, String>,
    ) -> bool {
        if request != self.sequencing.request {
            return false;
        }
        let Some(state) = self.sequencing.state.as_mut() else {
            return false;
        };
        match result {
            Ok(preview) => state.preview = Some(*preview),
            Err(message) => state.error = Some(message),
        }
        true
    }

    pub fn apply_sequence_finished(
        &mut self,
        request: u64,
        result: Result<SequenceOutcome, String>,
    ) -> bool {
        if request != self.sequencing.request {
            return false;
        }
        let Some(state) = self.sequencing.state.as_mut() else {
            return false;
        };
        state.busy = false;
        let (kind, before) = (state.kind, state.preview.as_ref().map(|p| p.head.clone()));
        match result {
            Ok(outcome) => {
                let name = kind.git_name();
                let (message, open) = match &outcome {
                    SequenceOutcome::Done { created, .. } => (
                        format!(
                            "{name} terminé : {created} commit(s) créé(s) sur la branche courante."
                        ),
                        false,
                    ),
                    SequenceOutcome::Conflicts { files } => (
                        format!(
                            "Le {name} s'est arrêté sur un conflit ({} fichier(s)) : les commits restants sont gardés par Git, résolvez puis continuez, ou abandonnez.",
                            files.len()
                        ),
                        true,
                    ),
                    SequenceOutcome::Stopped { .. } => (
                        format!(
                            "Le {name} s'est arrêté (commit devenu vide ?) : continuez, passez ce commit ou abandonnez."
                        ),
                        true,
                    ),
                };
                if let SequenceOutcome::Done { head, .. } = &outcome {
                    self.record_journal(name, before, Some(head.clone()), String::new());
                }
                self.sequencing.state = None;
                self.notice = Some(message);
                self.request_refresh(false);
                self.probe_operation();
                if open {
                    self.open_conflicts();
                }
            }
            Err(message) => state.error = Some(message),
        }
        true
    }
}
