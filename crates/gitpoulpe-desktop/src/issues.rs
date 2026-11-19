use std::path::Path;

use gitpoulpe_core::Connection;
use gitpoulpe_git::{commit_exists, local_branches, remote_urls};
use gitpoulpe_hosting::{
    IssueDetail, IssueFilter, IssueSummary, list_issues, load_issue, parse_remote,
};
use rune::TextEdit;

use crate::engine::Event;
use crate::state::State;

#[derive(Clone)]
pub struct IssueTarget {
    pub connection: Connection,
    pub owner: String,
    pub name: String,
}

pub struct IssuesState {
    pub open: bool,
    pub targets: Vec<IssueTarget>,
    pub choice: usize,
    pub filter: IssueFilter,
    pub query: TextEdit,
    pub items: Vec<IssueSummary>,
    pub truncated: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub list_request: u64,
    pub selected: Option<u64>,
    pub detail: Option<IssueDetail>,
    pub detail_loading: bool,
    pub detail_error: Option<String>,
    pub detail_request: u64,
    pub branches: Vec<String>,
    pub notice: Option<String>,
    pub version: u32,
}

impl Default for IssuesState {
    fn default() -> Self {
        Self {
            open: false,
            targets: Vec::new(),
            choice: 0,
            filter: IssueFilter::Open,
            query: TextEdit::new(""),
            items: Vec::new(),
            truncated: false,
            loading: false,
            error: None,
            list_request: 0,
            selected: None,
            detail: None,
            detail_loading: false,
            detail_error: None,
            detail_request: 0,
            branches: Vec::new(),
            notice: None,
            version: 0,
        }
    }
}

pub fn mentions_number(branch: &str, number: u64) -> bool {
    let digits = number.to_string();
    let bytes = branch.as_bytes();
    branch.match_indices(&digits).any(|(at, _)| {
        let before = at.checked_sub(1).and_then(|i| bytes.get(i));
        let after = bytes.get(at + digits.len());
        !before.is_some_and(u8::is_ascii_digit) && !after.is_some_and(u8::is_ascii_digit)
    })
}

impl State {
    pub fn open_issues(&mut self) {
        let Some(path) = self.repo.as_ref().map(|r| r.path.clone()) else {
            return;
        };
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
        self.close_pulls();
        self.close_file_history();
        self.close_file_edit();
        self.close_tools();
        self.close_terminal();
        self.close_shortcuts();
        self.close_workspaces();
        self.close_favorites();
        self.close_overview();
        self.close_notices();
        self.close_patch_import();
        self.close_deep_link();
        self.accounts.open = false;
        self.profiles_open = false;
        let list_request = self.issues.list_request + 1;
        let detail_request = self.issues.detail_request + 1;
        let mut targets: Vec<IssueTarget> = Vec::new();
        for url in remote_urls(&path) {
            let Some(reference) = parse_remote(&url) else {
                continue;
            };
            for connection in &self.accounts.book.connections {
                let known = targets.iter().any(|t| {
                    t.connection.id == connection.id
                        && t.owner.eq_ignore_ascii_case(&reference.owner)
                        && t.name.eq_ignore_ascii_case(&reference.repo)
                });
                if connection.host_caption().to_lowercase() == reference.host && !known {
                    targets.push(IssueTarget {
                        connection: connection.clone(),
                        owner: reference.owner.clone(),
                        name: reference.repo.clone(),
                    });
                }
            }
        }
        self.issues = IssuesState {
            open: true,
            list_request,
            detail_request,
            error: targets.is_empty().then(|| {
                "Aucun compte connecté ne correspond aux dépôts distants de ce dépôt : connectez le compte dans Comptes fournisseurs.".to_owned()
            }),
            targets,
            ..IssuesState::default()
        };
        self.reload_issues();
    }

    pub fn close_issues(&mut self) {
        self.issues.open = false;
        self.issues.loading = false;
        self.issues.list_request += 1;
        self.issues.detail_request += 1;
    }

    fn issue_target(&self) -> Option<IssueTarget> {
        self.issues.targets.get(self.issues.choice).cloned()
    }

    pub fn issues_switch_target(&mut self) {
        if self.issues.targets.len() > 1 {
            self.issues.choice = (self.issues.choice + 1) % self.issues.targets.len();
            self.issues.selected = None;
            self.issues.detail = None;
            self.reload_issues();
        }
    }

    pub fn issues_set_filter(&mut self, filter: IssueFilter) {
        if self.issues.filter != filter {
            self.issues.filter = filter;
            self.reload_issues();
        }
    }

    pub fn reload_issues(&mut self) {
        let Some(target) = self.issue_target() else {
            return;
        };
        let issues = &mut self.issues;
        issues.list_request += 1;
        issues.loading = true;
        issues.error = None;
        issues.items.clear();
        issues.truncated = false;
        issues.version += 1;
        let request = issues.list_request;
        let (filter, query) = (issues.filter, issues.query.text.clone());
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = list_issues(
                secrets.as_ref(),
                &target.connection,
                &target.owner,
                &target.name,
                filter,
                &query,
            )
            .await
            .map_err(|e| e.to_string());
            let _ = sender.send(Event::IssuesListed { request, result });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_issues_listed(
        &mut self,
        request: u64,
        result: Result<(Vec<IssueSummary>, bool), String>,
    ) -> bool {
        let issues = &mut self.issues;
        if !issues.open || request != issues.list_request {
            return false;
        }
        issues.loading = false;
        match result {
            Ok((items, truncated)) => {
                issues.items = items;
                issues.truncated = truncated;
            }
            Err(message) => {
                issues.items.clear();
                issues.error = Some(message);
            }
        }
        issues.version += 1;
        true
    }

    pub fn select_issue(&mut self, number: u64) {
        let Some(target) = self.issue_target() else {
            return;
        };
        let issues = &mut self.issues;
        issues.selected = Some(number);
        issues.detail = None;
        issues.detail_error = None;
        issues.detail_loading = true;
        issues.notice = None;
        issues.detail_request += 1;
        issues.version += 1;
        let request = issues.detail_request;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = load_issue(
                secrets.as_ref(),
                &target.connection,
                &target.owner,
                &target.name,
                number,
            )
            .await
            .map(Box::new)
            .map_err(|e| e.to_string());
            let _ = sender.send(Event::IssueLoaded {
                request,
                number,
                result,
            });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_issue_loaded(
        &mut self,
        request: u64,
        number: u64,
        result: Result<Box<IssueDetail>, String>,
    ) -> bool {
        let repo = self.repo.as_ref().map(|r| r.path.clone());
        let issues = &mut self.issues;
        if !issues.open || request != issues.detail_request || issues.selected != Some(number) {
            return false;
        }
        issues.detail_loading = false;
        match result {
            Ok(detail) => {
                issues.branches = repo
                    .map(|p| {
                        local_branches(&p)
                            .into_iter()
                            .filter(|b| mentions_number(b, number))
                            .collect()
                    })
                    .unwrap_or_default();
                issues.detail = Some(*detail);
            }
            Err(message) => {
                issues.detail = None;
                issues.detail_error = Some(message);
            }
        }
        issues.version += 1;
        true
    }

    pub fn issue_open_pull(&mut self, number: u64) {
        self.notices.pull_number = Some(number);
        self.open_pulls();
    }

    pub fn issue_open_commit(&mut self, sha: &str) {
        let Some(path) = self.repo.as_ref().map(|r| r.path.clone()) else {
            return;
        };
        if !commit_exists(Path::new(&path), sha) {
            self.issues.notice = Some(format!(
                "Le commit {} n'est pas dans ce dépôt local : récupérez les références distantes (Dépôts distants) pour l'obtenir.",
                sha.chars().take(10).collect::<String>()
            ));
            self.issues.version += 1;
            return;
        }
        self.close_issues();
        self.open_commit(sha.to_owned(), 0);
    }
}
