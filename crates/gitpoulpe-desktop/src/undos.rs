use gitpoulpe_core::{UndoDirection, UndoKind, UndoRecord, UndoState, push_undo};

use crate::engine::Command;
use crate::state::State;

#[derive(Default)]
pub struct UndoScreen {
    pub open: bool,
    pub selected: Option<u64>,
    pub busy: bool,
    pub request: u64,
    pub message: Option<String>,
}

fn irreversible(action: &str) -> Option<&'static str> {
    match action {
        "push" => Some(
            "le distant a déjà reçu les commits ; revenir en arrière demanderait de réécrire l'historique distant",
        ),
        "stash-create" | "stash-apply" | "stash-pop" | "stash-drop" | "stash-apply-conflict" => {
            Some("les stashes se gèrent depuis la réserve")
        }
        "remote-add"
        | "remote-set-url"
        | "remote-set-push-url"
        | "remote-rename"
        | "remote-remove" => Some("la configuration des remotes se modifie depuis l'écran Remotes"),
        _ => None,
    }
}

impl State {
    fn undo_key(&self) -> Option<String> {
        Some(format!("undo:{}", self.repo_key()?))
    }

    pub fn undo_records(&self) -> Vec<UndoRecord> {
        self.undo_key()
            .and_then(|key| self.prefs.get(&key))
            .unwrap_or_default()
    }

    fn save_undo_records(&mut self, records: &[UndoRecord]) {
        if let Some(key) = self.undo_key() {
            let _ = self.prefs.set(&key, &records.to_vec());
        }
    }

    pub(crate) fn record_undo(
        &mut self,
        action: &str,
        before: &Option<String>,
        after: &Option<String>,
        detail: &str,
    ) {
        let reference = self
            .repo
            .as_ref()
            .and_then(|r| r.snapshot.branch.head.clone())
            .map_or_else(|| "HEAD".to_owned(), |h| format!("refs/heads/{h}"));
        let moved = |label: String, limit: Option<&str>| match (before, after) {
            (Some(b), Some(a)) if b != a => Some((
                label,
                UndoKind::RefMove {
                    reference: reference.clone(),
                    before: b.clone(),
                    after: a.clone(),
                },
                limit.map(str::to_owned),
            )),
            _ => None,
        };
        let entry = match action {
            "commit" => moved(format!("Commit : {detail}"), None),
            "amend" => moved(format!("Amendement : {detail}"), None),
            "merge" => moved(format!("Fusion de {detail}"), None),
            "cherry-pick" => moved("Cherry-pick".to_owned(), None),
            "revert" => moved("Revert".to_owned(), None),
            "rebase" => moved("Rebase interactif".to_owned(), None),
            "pull" => moved(format!("Intégration distante ({detail})"), None),
            "reset" => moved(
                format!("Reset {detail}"),
                Some(
                    "Les modifications non enregistrées effacées par un reset hard ne sont pas restaurées : elles sont sauvegardées sous refs/gitpoulpe/recovery.",
                ),
            ),
            "branch-create" | "recover-branch" => after.as_ref().map(|tip| {
                (
                    format!("Branche créée : {detail}"),
                    UndoKind::BranchCreate {
                        name: detail.to_owned(),
                        tip: tip.clone(),
                    },
                    None,
                )
            }),
            "branch-delete" => before.as_ref().map(|tip| {
                (
                    format!("Branche supprimée : {detail}"),
                    UndoKind::BranchDelete {
                        name: detail.to_owned(),
                        tip: tip.clone(),
                    },
                    None,
                )
            }),
            "branch-rename" => match (before, after) {
                (Some(old), Some(new)) => Some((
                    format!("Branche renommée : {old} → {new}"),
                    UndoKind::BranchRename {
                        old: old.clone(),
                        new: new.clone(),
                    },
                    None,
                )),
                _ => None,
            },
            "tag-create" => after.as_ref().map(|object| {
                (
                    format!("Tag créé : {detail}"),
                    UndoKind::TagCreate {
                        name: detail.to_owned(),
                        object: object.clone(),
                    },
                    None,
                )
            }),
            "tag-delete" => before.as_ref().map(|object| {
                (
                    format!("Tag supprimé : {detail}"),
                    UndoKind::TagDelete {
                        name: detail.to_owned(),
                        object: object.clone(),
                    },
                    None,
                )
            }),
            other => irreversible(other).map(|why| {
                (
                    format!("{other} {detail}").trim().to_owned(),
                    UndoKind::Irreversible {
                        why: why.to_owned(),
                    },
                    None,
                )
            }),
        };
        let Some((label, kind, limit)) = entry else {
            return;
        };
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let mut records = self.undo_records();
        push_undo(
            &mut records,
            UndoRecord {
                id: 0,
                at,
                label,
                kind,
                state: UndoState::Done,
                limit,
            },
        );
        self.save_undo_records(&records);
    }

    pub fn open_undo(&mut self) {
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
        self.close_worktrees();
        self.close_hooks();
        self.profiles_open = false;
        self.undo_screen.open = true;
        self.undo_screen.message = None;
    }

    pub fn close_undo(&mut self) {
        self.undo_screen.open = false;
    }

    pub fn select_undo(&mut self, id: u64) {
        self.undo_screen.selected = Some(id);
        self.undo_screen.message = None;
    }

    pub fn undo_run(&mut self, direction: UndoDirection) {
        if self.undo_screen.busy {
            return;
        }
        let Some(id) = self.undo_screen.selected else {
            return;
        };
        let Some(record) = self.undo_records().into_iter().find(|r| r.id == id) else {
            return;
        };
        self.undo_screen.busy = true;
        self.undo_screen.message = None;
        self.undo_screen.request += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::UndoRedo {
                request: self.undo_screen.request,
                record: Box::new(record),
                direction,
            });
        }
    }

    pub fn apply_undo_finished(
        &mut self,
        request: u64,
        id: u64,
        direction: UndoDirection,
        result: Result<(), String>,
    ) -> bool {
        if request != self.undo_screen.request {
            return false;
        }
        self.undo_screen.busy = false;
        match result {
            Ok(()) => {
                let mut records = self.undo_records();
                if let Some(record) = records.iter_mut().find(|r| r.id == id) {
                    record.state = match direction {
                        UndoDirection::Undo => UndoState::Undone,
                        UndoDirection::Redo => UndoState::Done,
                    };
                }
                self.save_undo_records(&records);
                self.undo_screen.message = Some(match direction {
                    UndoDirection::Undo => "Opération annulée.".to_owned(),
                    UndoDirection::Redo => "Opération rétablie.".to_owned(),
                });
                self.request_refresh(false);
            }
            Err(message) => self.undo_screen.message = Some(message),
        }
        true
    }
}
