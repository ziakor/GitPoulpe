use gitpoulpe_core::{Remote, RemoteImpact};
use rune::TextEdit;

use crate::engine::{BranchDone, Command, RemoteOp};
use crate::state::State;

pub struct RemotesState {
    pub open: bool,
    pub request: u64,
    pub op_request: u64,
    pub impact_request: u64,
    pub remotes: Vec<Remote>,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub selected: Option<String>,
    pub new_name: TextEdit,
    pub new_url: TextEdit,
    pub url: TextEdit,
    pub push_url: TextEdit,
    pub rename: TextEdit,
    pub confirm_remove: Option<(String, Option<RemoteImpact>)>,
    pub version: u32,
}

impl Default for RemotesState {
    fn default() -> Self {
        Self {
            open: false,
            request: 0,
            op_request: 0,
            impact_request: 0,
            remotes: Vec::new(),
            loading: false,
            busy: false,
            error: None,
            selected: None,
            new_name: TextEdit::new(""),
            new_url: TextEdit::new(""),
            url: TextEdit::new(""),
            push_url: TextEdit::new(""),
            rename: TextEdit::new(""),
            confirm_remove: None,
            version: 0,
        }
    }
}

impl RemotesState {
    pub fn selected_remote(&self) -> Option<&Remote> {
        let name = self.selected.as_deref()?;
        self.remotes.iter().find(|r| r.name == name)
    }

    fn clear_forms(&mut self) {
        self.url = TextEdit::new("");
        self.push_url = TextEdit::new("");
        self.rename = TextEdit::new("");
        self.confirm_remove = None;
    }
}

impl State {
    pub fn open_remotes(&mut self) {
        if self.repo.is_none() {
            return;
        }
        self.close_history();
        self.close_ignored();
        self.close_branches();
        self.close_tags();
        self.profiles_open = false;
        self.remotes.open = true;
        self.reload_remotes();
    }

    pub fn close_remotes(&mut self) {
        self.remotes.open = false;
        self.remotes.confirm_remove = None;
    }

    fn reload_remotes(&mut self) {
        self.remotes.request += 1;
        self.remotes.loading = true;
        self.remotes.error = None;
        self.remotes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::RemoteList {
                request: self.remotes.request,
            });
        }
    }

    pub fn select_remote(&mut self, name: &str) {
        self.remotes.selected = Some(name.to_owned());
        self.remotes.clear_forms();
        self.remotes.version += 1;
    }

    fn send_remote_op(&mut self, op: RemoteOp) {
        if self.remotes.busy {
            return;
        }
        self.remotes.busy = true;
        self.remotes.op_request += 1;
        self.remotes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::RemoteOp {
                request: self.remotes.op_request,
                op,
            });
        }
    }

    pub fn remote_add(&mut self) {
        let name = self.remotes.new_name.text.trim().to_owned();
        let url = self.remotes.new_url.text.trim().to_owned();
        if name.is_empty() || url.is_empty() {
            self.notice = Some("Donnez un nom et une URL au remote.".to_owned());
            return;
        }
        self.send_remote_op(RemoteOp::Add { name, url });
    }

    pub fn remote_set_url(&mut self) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        let url = self.remotes.url.text.trim().to_owned();
        if url.is_empty() {
            self.notice = Some("Saisissez la nouvelle URL de lecture.".to_owned());
            return;
        }
        let (name, expected) = (remote.name.clone(), remote.fetch_urls.clone());
        self.send_remote_op(RemoteOp::SetUrl {
            name,
            url,
            expected,
        });
    }

    pub fn remote_set_push_url(&mut self, clear: bool) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        let typed = self.remotes.push_url.text.trim().to_owned();
        if !clear && typed.is_empty() {
            self.notice = Some("Saisissez la nouvelle URL d'envoi.".to_owned());
            return;
        }
        let (name, expected) = (remote.name.clone(), remote.push_urls.clone());
        self.send_remote_op(RemoteOp::SetPushUrl {
            name,
            url: (!clear).then_some(typed),
            expected,
        });
    }

    pub fn remote_rename(&mut self) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        let new = self.remotes.rename.text.trim().to_owned();
        if new.is_empty() {
            self.notice = Some("Saisissez le nouveau nom.".to_owned());
            return;
        }
        let old = remote.name.clone();
        self.send_remote_op(RemoteOp::Rename { old, new });
    }

    pub fn remote_ask_remove(&mut self) {
        let Some(remote) = self.remotes.selected_remote() else {
            return;
        };
        let name = remote.name.clone();
        self.remotes.confirm_remove = Some((name.clone(), None));
        self.remotes.impact_request += 1;
        self.remotes.version += 1;
        if let Some(engine) = &self.engine {
            engine.send(Command::RemoteImpact {
                request: self.remotes.impact_request,
                name,
            });
        }
    }

    pub fn remote_cancel_remove(&mut self) {
        self.remotes.confirm_remove = None;
        self.remotes.version += 1;
    }

    pub fn remote_remove(&mut self) {
        let Some((name, Some(_))) = self.remotes.confirm_remove.take() else {
            return;
        };
        let Some(remote) = self.remotes.remotes.iter().find(|r| r.name == name) else {
            return;
        };
        let expected = remote.fetch_urls.clone();
        self.send_remote_op(RemoteOp::Remove { name, expected });
    }

    pub fn apply_remotes_listed(
        &mut self,
        request: u64,
        result: Result<Vec<Remote>, String>,
    ) -> bool {
        if request != self.remotes.request {
            return false;
        }
        self.remotes.loading = false;
        match result {
            Ok(list) => {
                if self
                    .remotes
                    .selected
                    .as_ref()
                    .is_some_and(|n| !list.iter().any(|r| &r.name == n))
                {
                    self.remotes.selected = None;
                }
                self.remotes.remotes = list;
                self.remotes.error = None;
            }
            Err(message) => self.remotes.error = Some(message),
        }
        self.remotes.version += 1;
        true
    }

    pub fn apply_remote_impacted(
        &mut self,
        request: u64,
        name: String,
        result: Result<RemoteImpact, String>,
    ) -> bool {
        if request != self.remotes.impact_request {
            return false;
        }
        match (&mut self.remotes.confirm_remove, result) {
            (Some((pending, slot)), Ok(impact)) if *pending == name => *slot = Some(impact),
            (_, Err(message)) => {
                self.remotes.confirm_remove = None;
                self.notice = Some(format!("Impact du retrait illisible : {message}"));
            }
            _ => return false,
        }
        self.remotes.version += 1;
        true
    }

    pub fn apply_remote_done(&mut self, request: u64, result: Result<BranchDone, String>) -> bool {
        if request != self.remotes.op_request {
            return false;
        }
        self.remotes.busy = false;
        self.remotes.confirm_remove = None;
        match result {
            Ok(done) => {
                if let Some((action, before, after, detail)) = done.journal {
                    self.record_journal(&action, before, after, detail);
                }
                self.notice = Some(done.message);
                self.remotes.new_name = TextEdit::new("");
                self.remotes.new_url = TextEdit::new("");
                self.remotes.clear_forms();
                self.reload_remotes();
                self.request_refresh(false);
            }
            Err(message) => {
                self.notice = Some(format!("Remote non modifié : {message}"));
                self.reload_remotes();
            }
        }
        self.remotes.version += 1;
        true
    }
}
