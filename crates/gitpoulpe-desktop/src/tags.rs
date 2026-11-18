use gitpoulpe_core::Tag;
use rune::TextEdit;

use crate::engine::{BranchDone, Command, TagOp};
use crate::state::State;

pub struct TagsState {
    pub open: bool,
    pub request: u64,
    pub op_request: u64,
    pub tags: Vec<Tag>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub new_name: TextEdit,
    pub message: TextEdit,
    pub target: Option<String>,
    pub confirm_delete: Option<String>,
    pub version: u32,
}

impl Default for TagsState {
    fn default() -> Self {
        Self {
            open: false,
            request: 0,
            op_request: 0,
            tags: Vec::new(),
            loading: false,
            busy: false,
            error: None,
            selected: None,
            new_name: TextEdit::new(""),
            message: TextEdit::new(""),
            target: None,
            confirm_delete: None,
            version: 0,
        }
    }
}

impl TagsState {
    pub fn selected_tag(&self) -> Option<&Tag> {
        let name = self.selected.as_deref()?;
        self.tags.iter().find(|t| t.name == name)
    }
}

impl State {
    pub fn open_tags(&mut self, target: Option<String>) {
        if self.repo.is_none() {
            return;
        }
        self.close_history();
        self.close_ignored();
        self.close_branches();
        self.close_remotes();
        self.close_conflicts();
        self.profiles_open = false;
        self.tags.open = true;
        self.tags.target = target;
        self.reload_tags();
    }

    pub fn close_tags(&mut self) {
        self.tags.open = false;
        self.tags.confirm_delete = None;
    }

    fn reload_tags(&mut self) {
        self.tags.request += 1;
        self.tags.loading = true;
        self.tags.error = None;
        self.tags.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::TagList {
                request: self.tags.request,
            });
        }
    }

    pub fn select_tag(&mut self, name: &str) {
        self.tags.selected = Some(name.to_owned());
        self.tags.confirm_delete = None;
        self.tags.version += 1;
    }

    fn send_tag_op(&mut self, op: TagOp) {
        if self.tags.busy {
            return;
        }
        self.tags.busy = true;
        self.tags.op_request += 1;
        self.tags.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::TagOp {
                request: self.tags.op_request,
                op,
            });
        }
    }

    pub fn tag_create(&mut self, annotated: bool) {
        let name = self.tags.new_name.text.trim().to_owned();
        if name.is_empty() {
            self.notice = Some("Donnez un nom au tag.".to_owned());
            return;
        }
        let message = self.tags.message.text.trim_end().to_owned();
        if annotated && message.trim().is_empty() {
            self.notice = Some("Un tag annoté a besoin d'un message.".to_owned());
            return;
        }
        let profile = self.active_profile().cloned().map(Box::new);
        self.send_tag_op(TagOp::Create {
            name,
            target: self.tags.target.clone(),
            message: annotated.then_some(message),
            profile,
        });
    }

    pub fn tag_ask_delete(&mut self) {
        if let Some(tag) = self.tags.selected_tag() {
            self.tags.confirm_delete = Some(tag.name.clone());
            self.tags.version += 1;
        }
    }

    pub fn tag_cancel_delete(&mut self) {
        self.tags.confirm_delete = None;
        self.tags.version += 1;
    }

    pub fn tag_delete(&mut self) {
        let Some(name) = self.tags.confirm_delete.take() else {
            return;
        };
        let Some(tag) = self.tags.tags.iter().find(|t| t.name == name) else {
            return;
        };
        let expected = tag.object.clone();
        self.send_tag_op(TagOp::Delete { name, expected });
    }

    pub fn apply_tags_listed(&mut self, request: u64, result: Result<Vec<Tag>, String>) -> bool {
        if request != self.tags.request {
            return false;
        }
        self.tags.loading = false;
        match result {
            Ok(list) => {
                if self
                    .tags
                    .selected
                    .as_ref()
                    .is_some_and(|n| !list.iter().any(|t| &t.name == n))
                {
                    self.tags.selected = None;
                }
                self.tags.tags = list;
                self.tags.error = None;
            }
            Err(message) => self.tags.error = Some(message),
        }
        self.tags.version += 1;
        true
    }

    pub fn apply_tag_done(&mut self, request: u64, result: Result<BranchDone, String>) -> bool {
        if request != self.tags.op_request {
            return false;
        }
        self.tags.busy = false;
        self.tags.confirm_delete = None;
        match result {
            Ok(done) => {
                if let Some((action, before, after, detail)) = done.journal {
                    self.record_journal(&action, before, after, detail);
                }
                self.notice = Some(done.message);
                self.tags.new_name = TextEdit::new("");
                self.tags.message = TextEdit::new("");
                self.reload_tags();
            }
            Err(message) => {
                self.notice = Some(format!("Tag non modifié : {message}"));
                self.reload_tags();
            }
        }
        self.tags.version += 1;
        true
    }
}
