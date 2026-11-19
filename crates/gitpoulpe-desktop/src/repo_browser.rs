use gitpoulpe_hosting::{
    ForkOutcome, Listing, Organization, RepoSummary, create_fork, list_organizations,
    list_repositories,
};
use rune::TextEdit;

use crate::engine::Event;
use crate::state::State;

pub struct BrowserState {
    pub open: bool,
    pub connection: Option<String>,
    pub scope: Option<String>,
    pub orgs: Vec<Organization>,
    pub orgs_error: Option<String>,
    pub repos: Vec<RepoSummary>,
    pub loading: bool,
    pub error: Option<String>,
    pub truncated: bool,
    pub request: u64,
    pub orgs_request: u64,
    pub filter: TextEdit,
    pub selected: Option<String>,
    pub version: u32,
    pub fork_busy: bool,
    pub fork_request: u64,
    pub fork_error: Option<String>,
    pub fork_done: Option<String>,
    pub forked: Option<RepoSummary>,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self {
            open: false,
            connection: None,
            scope: None,
            orgs: Vec::new(),
            orgs_error: None,
            repos: Vec::new(),
            loading: false,
            error: None,
            truncated: false,
            request: 0,
            orgs_request: 0,
            filter: TextEdit::new(""),
            selected: None,
            version: 0,
            fork_busy: false,
            fork_request: 0,
            fork_error: None,
            fork_done: None,
            forked: None,
        }
    }
}

impl BrowserState {
    pub fn visible(&self) -> Vec<usize> {
        let needle = self.filter.text.trim().to_lowercase();
        self.repos
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                needle.is_empty()
                    || r.full_name().to_lowercase().contains(&needle)
                    || r.description
                        .as_deref()
                        .is_some_and(|d| d.to_lowercase().contains(&needle))
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selected_repo(&self) -> Option<&RepoSummary> {
        let name = self.selected.as_ref()?;
        self.repos.iter().find(|r| &r.full_name() == name)
    }
}

impl State {
    pub fn open_browser(&mut self, connection_id: &str) {
        if self.accounts.book.get(connection_id).is_none() {
            return;
        }
        let request = self.accounts.browser.request + 1;
        let orgs_request = self.accounts.browser.orgs_request + 1;
        let fork_request = self.accounts.browser.fork_request + 1;
        self.accounts.browser = BrowserState {
            open: true,
            connection: Some(connection_id.to_owned()),
            request,
            orgs_request,
            fork_request,
            ..BrowserState::default()
        };
        self.load_browser_orgs();
        self.load_browser_repos();
    }

    pub fn close_browser(&mut self) {
        let browser = &mut self.accounts.browser;
        browser.open = false;
        browser.loading = false;
        browser.request += 1;
        browser.orgs_request += 1;
        browser.fork_request += 1;
        browser.fork_busy = false;
        browser.version += 1;
    }

    pub fn browse_scope(&mut self, scope: Option<String>) {
        if self.accounts.browser.scope == scope {
            return;
        }
        self.accounts.browser.scope = scope;
        self.accounts.browser.selected = None;
        self.load_browser_repos();
    }

    pub fn refresh_browser(&mut self) {
        self.load_browser_orgs();
        self.load_browser_repos();
    }

    fn browser_connection(&self) -> Option<gitpoulpe_core::Connection> {
        let id = self.accounts.browser.connection.as_ref()?;
        self.accounts.book.get(id).cloned()
    }

    fn load_browser_orgs(&mut self) {
        let Some(connection) = self.browser_connection() else {
            return;
        };
        let browser = &mut self.accounts.browser;
        browser.orgs_request += 1;
        browser.orgs_error = None;
        let request = browser.orgs_request;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = list_organizations(secrets.as_ref(), &connection)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::BrowserOrgs { request, result });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    fn load_browser_repos(&mut self) {
        let Some(connection) = self.browser_connection() else {
            return;
        };
        let browser = &mut self.accounts.browser;
        browser.request += 1;
        browser.loading = true;
        browser.error = None;
        browser.repos.clear();
        browser.truncated = false;
        browser.version += 1;
        let request = browser.request;
        let scope = browser.scope.clone();
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = list_repositories(secrets.as_ref(), &connection, scope.as_deref())
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::BrowserRepos { request, result });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_browser_orgs(
        &mut self,
        request: u64,
        result: Result<Listing<Organization>, String>,
    ) -> bool {
        let browser = &mut self.accounts.browser;
        if !browser.open || request != browser.orgs_request {
            return false;
        }
        match result {
            Ok(listing) => browser.orgs = listing.items,
            Err(message) => {
                browser.orgs.clear();
                browser.orgs_error = Some(message);
            }
        }
        browser.version += 1;
        true
    }

    pub fn apply_browser_repos(
        &mut self,
        request: u64,
        result: Result<Listing<RepoSummary>, String>,
    ) -> bool {
        let browser = &mut self.accounts.browser;
        if !browser.open || request != browser.request {
            return false;
        }
        browser.loading = false;
        match result {
            Ok(listing) => {
                browser.repos = listing.items;
                browser.truncated = listing.truncated;
            }
            Err(message) => browser.error = Some(message),
        }
        browser.version += 1;
        true
    }

    pub fn select_browser_repo(&mut self, full_name: String) {
        let browser = &mut self.accounts.browser;
        browser.selected = Some(full_name);
        browser.fork_request += 1;
        browser.fork_busy = false;
        browser.fork_error = None;
        browser.fork_done = None;
        browser.forked = None;
        self.accounts.browser.version += 1;
    }

    pub fn fork_selected_repo(&mut self, destination: Option<String>) {
        let Some(connection) = self.browser_connection() else {
            return;
        };
        if connection.provider != gitpoulpe_core::Provider::Github {
            return;
        }
        let browser = &mut self.accounts.browser;
        if browser.fork_busy {
            return;
        }
        let Some(source) = browser.selected_repo().cloned() else {
            return;
        };
        browser.fork_request += 1;
        browser.fork_busy = true;
        browser.fork_error = None;
        browser.fork_done = None;
        browser.forked = None;
        browser.version += 1;
        let request = browser.fork_request;
        let second_ms = self.accounts.second_ms;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = create_fork(
                secrets.as_ref(),
                &connection,
                &source.owner,
                &source.name,
                destination.as_deref(),
                |d| tokio::time::sleep(std::time::Duration::from_millis(d.as_secs() * second_ms)),
            )
            .await
            .map_err(|e| e.to_string());
            let _ = sender.send(Event::BrowserFork { request, result });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_browser_fork(
        &mut self,
        request: u64,
        result: Result<ForkOutcome, String>,
    ) -> bool {
        let browser = &mut self.accounts.browser;
        if !browser.open || request != browser.fork_request {
            return false;
        }
        browser.fork_busy = false;
        match result {
            Ok(outcome) => {
                let origin = outcome.repo.parent.clone().unwrap_or_default();
                browser.fork_done = Some(if outcome.existed {
                    format!(
                        "Ce fork existe déjà : {} (fork de {origin}). Rien n'a été créé.",
                        outcome.repo.full_name()
                    )
                } else {
                    format!(
                        "Fork créé : {} (fork de {origin}).",
                        outcome.repo.full_name()
                    )
                });
                browser.forked = Some(outcome.repo);
            }
            Err(message) => browser.fork_error = Some(message),
        }
        browser.version += 1;
        true
    }

    pub fn clone_forked_repo(&mut self) {
        let Some(url) = self
            .accounts
            .browser
            .forked
            .as_ref()
            .map(|r| r.clone_url.clone())
        else {
            return;
        };
        self.close_browser();
        self.close_accounts();
        self.begin_clone();
        self.clone.url = TextEdit::new(url);
    }

    pub fn clone_selected_repo(&mut self) {
        let Some(url) = self
            .accounts
            .browser
            .selected_repo()
            .map(|r| r.clone_url.clone())
        else {
            return;
        };
        self.close_browser();
        self.close_accounts();
        self.begin_clone();
        self.clone.url = TextEdit::new(url);
    }
}
