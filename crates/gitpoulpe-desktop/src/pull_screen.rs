use gitpoulpe_core::Connection;
use gitpoulpe_hosting::{
    HostedPull, PullDraft, PullTarget, RemoteRef, create_pull_request, parse_remote, prepare_pull,
};
use rune::TextEdit;

use crate::engine::Event;
use crate::state::State;

#[derive(Clone)]
pub struct Candidate {
    pub connection: Connection,
    pub remote: String,
    pub reference: RemoteRef,
}

pub struct PullsState {
    pub open: bool,
    pub awaiting_remotes: bool,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub branch: Option<String>,
    pub candidates: Vec<Candidate>,
    pub choice: usize,
    pub to_parent: bool,
    pub target: Option<PullTarget>,
    pub created: Option<HostedPull>,
    pub title: TextEdit,
    pub body: TextEdit,
    pub base: TextEdit,
    pub draft: bool,
    pub request: u64,
    pub version: u32,
    pub list: crate::pull_list::PullListState,
}

impl Default for PullsState {
    fn default() -> Self {
        Self {
            open: false,
            awaiting_remotes: false,
            loading: false,
            busy: false,
            error: None,
            notice: None,
            branch: None,
            candidates: Vec::new(),
            choice: 0,
            to_parent: false,
            target: None,
            created: None,
            title: TextEdit::new(""),
            body: TextEdit::new(""),
            base: TextEdit::new(""),
            draft: false,
            request: 0,
            version: 0,
            list: crate::pull_list::PullListState::default(),
        }
    }
}

fn humanize(branch: &str) -> String {
    let name = branch.rsplit('/').next().unwrap_or(branch);
    let mut text: String = name
        .chars()
        .map(|c| if matches!(c, '-' | '_') { ' ' } else { c })
        .collect();
    if let Some(first) = text.get(..1) {
        text = first.to_uppercase() + &text[1..];
    }
    text
}

impl State {
    pub fn open_pulls(&mut self) {
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
        self.close_undo();
        self.close_worktrees();
        self.close_hooks();
        self.close_lfs();
        self.close_submodules();
        self.close_gitflow();
        self.close_file_history();
        self.close_file_edit();
        self.close_tools();
        self.close_terminal();
        self.close_shortcuts();
        self.close_workspaces();
        self.profiles_open = false;
        let request = self.pulls.request + 1;
        let list = std::mem::take(&mut self.pulls.list);
        self.pulls = PullsState {
            list: crate::pull_list::PullListState {
                open: false,
                list_request: list.list_request + 1,
                detail_request: list.detail_request + 1,
                ..crate::pull_list::PullListState::default()
            },
            open: true,
            awaiting_remotes: true,
            loading: true,
            request,
            branch: self
                .repo
                .as_ref()
                .and_then(|r| r.snapshot.branch.head.clone())
                .filter(|head| head != "(detached)"),
            ..PullsState::default()
        };
        self.reload_remotes();
    }

    pub fn close_pulls(&mut self) {
        self.close_pull_list();
        self.pulls.open = false;
        self.pulls.request += 1;
        self.pulls.awaiting_remotes = false;
    }

    pub fn pulls_remotes_ready(&mut self) {
        if !self.pulls.awaiting_remotes {
            return;
        }
        self.pulls.awaiting_remotes = false;
        let mut remotes = self.remotes.remotes.clone();
        remotes.sort_by_key(|r| r.name != "origin");
        let mut candidates = Vec::new();
        for remote in &remotes {
            let Some(url) = remote.fetch_urls.first() else {
                continue;
            };
            let Some(reference) = parse_remote(url) else {
                continue;
            };
            for connection in &self.accounts.book.connections {
                if connection.host_caption().to_lowercase() == reference.host {
                    candidates.push(Candidate {
                        connection: connection.clone(),
                        remote: remote.name.clone(),
                        reference: reference.clone(),
                    });
                }
            }
        }
        self.pulls.version += 1;
        if self.pulls.branch.is_none() {
            self.pulls.loading = false;
            self.pulls.error = Some(
                "HEAD est détachée : placez-vous sur une branche pour proposer une pull request."
                    .to_owned(),
            );
            return;
        }
        if candidates.is_empty() {
            self.pulls.loading = false;
            self.pulls.error = Some(if remotes.is_empty() {
                "Ce dépôt n'a aucun dépôt distant : ajoutez-en un, publiez la branche, puis revenez ici.".to_owned()
            } else {
                "Aucun compte connecté ne correspond aux dépôts distants de ce dépôt : connectez le compte dans Comptes fournisseurs.".to_owned()
            });
            return;
        }
        self.pulls.candidates = candidates;
        self.load_pull_preview();
    }

    pub fn load_pull_preview(&mut self) {
        let Some(candidate) = self.pulls.candidates.get(self.pulls.choice).cloned() else {
            return;
        };
        let Some(branch) = self.pulls.branch.clone() else {
            return;
        };
        self.pulls.request += 1;
        self.pulls.loading = true;
        self.pulls.error = None;
        self.pulls.version += 1;
        let request = self.pulls.request;
        let to_parent = self.pulls.to_parent;
        let base = self.pulls.base.text.trim().to_owned();
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = prepare_pull(
                secrets.as_ref(),
                &candidate.connection,
                &candidate.reference.owner,
                &candidate.reference.repo,
                &branch,
                to_parent,
                (!base.is_empty()).then_some(base.as_str()),
            )
            .await
            .map(Box::new)
            .map_err(|e| e.to_string());
            let _ = sender.send(Event::PullPrepared { request, result });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_pull_prepared(
        &mut self,
        request: u64,
        result: Result<Box<PullTarget>, String>,
    ) -> bool {
        let pulls = &mut self.pulls;
        if !pulls.open || request != pulls.request {
            return false;
        }
        pulls.loading = false;
        match result {
            Ok(target) => {
                if pulls.base.text.trim().is_empty() {
                    pulls.base = TextEdit::new(&target.base);
                }
                if pulls.title.text.trim().is_empty() {
                    pulls.title = TextEdit::new(humanize(&target.branch));
                }
                pulls.target = Some(*target);
            }
            Err(message) => {
                pulls.target = None;
                pulls.error = Some(message);
            }
        }
        pulls.version += 1;
        true
    }

    pub fn pulls_next_account(&mut self) {
        if self.pulls.candidates.len() < 2 {
            return;
        }
        self.pulls.choice = (self.pulls.choice + 1) % self.pulls.candidates.len();
        self.pulls.to_parent = false;
        self.pulls.base = TextEdit::new("");
        self.pulls.target = None;
        self.pulls.created = None;
        self.pulls.notice = None;
        self.load_pull_preview();
    }

    pub fn pulls_toggle_parent(&mut self) {
        self.pulls.to_parent = !self.pulls.to_parent;
        self.pulls.base = TextEdit::new("");
        self.pulls.created = None;
        self.pulls.notice = None;
        self.load_pull_preview();
    }

    pub fn pulls_toggle_draft(&mut self) {
        self.pulls.draft = !self.pulls.draft;
        self.pulls.version += 1;
    }

    pub fn pulls_refresh(&mut self) {
        self.pulls.notice = None;
        self.load_pull_preview();
    }

    pub fn create_pull(&mut self) {
        let Some(candidate) = self.pulls.candidates.get(self.pulls.choice).cloned() else {
            return;
        };
        let Some(target) = self.pulls.target.clone() else {
            return;
        };
        if self.pulls.busy || self.pulls.created.is_some() {
            return;
        }
        let mut target = target;
        let base = self.pulls.base.text.trim();
        if !base.is_empty() {
            target.base = base.to_owned();
        }
        self.pulls.request += 1;
        self.pulls.busy = true;
        self.pulls.error = None;
        self.pulls.notice = None;
        self.pulls.version += 1;
        let request = self.pulls.request;
        let draft = PullDraft {
            title: self.pulls.title.text.clone(),
            body: self.pulls.body.text.clone(),
            draft: self.pulls.draft,
        };
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result =
                create_pull_request(secrets.as_ref(), &candidate.connection, &target, &draft)
                    .await
                    .map_err(|e| e.to_string());
            let _ = sender.send(Event::PullCreated { request, result });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_pull_created(&mut self, request: u64, result: Result<HostedPull, String>) -> bool {
        let noun = if self.pull_is_gitlab() {
            "Merge request"
        } else {
            "Pull request"
        };
        let pulls = &mut self.pulls;
        if !pulls.open || request != pulls.request {
            return false;
        }
        pulls.busy = false;
        match result {
            Ok(pr) => {
                pulls.notice = Some(format!(
                    "{} #{} créée{} : {}",
                    noun,
                    pr.number,
                    if pr.draft { " (brouillon)" } else { "" },
                    pr.url
                ));
                if let Some(target) = pulls.target.as_mut() {
                    target.existing = Some(pr.clone());
                }
                pulls.created = Some(pr);
            }
            Err(message) => pulls.error = Some(message),
        }
        pulls.version += 1;
        true
    }
}
