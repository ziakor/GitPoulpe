use gitpoulpe_hosting::{PullDetail, PullFilter, PullSummary, list_pulls, load_pull};

use crate::engine::Event;
use crate::state::State;

pub struct PullListState {
    pub open: bool,
    pub filter: PullFilter,
    pub items: Vec<PullSummary>,
    pub truncated: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub selected: Option<u64>,
    pub detail: Option<PullDetail>,
    pub detail_loading: bool,
    pub detail_error: Option<String>,
    pub notice: Option<String>,
    pub list_request: u64,
    pub detail_request: u64,
    pub version: u32,
}

impl Default for PullListState {
    fn default() -> Self {
        Self {
            open: false,
            filter: PullFilter::Open,
            items: Vec::new(),
            truncated: false,
            loading: false,
            error: None,
            selected: None,
            detail: None,
            detail_loading: false,
            detail_error: None,
            notice: None,
            list_request: 0,
            detail_request: 0,
            version: 0,
        }
    }
}

impl State {
    fn pull_list_target(&self) -> Option<(gitpoulpe_core::Connection, String, String)> {
        let candidate = self.pulls.candidates.get(self.pulls.choice)?;
        let (owner, name) = match &self.pulls.target {
            Some(target) => (
                target.target_repo.owner.clone(),
                target.target_repo.name.clone(),
            ),
            None => (
                candidate.reference.owner.clone(),
                candidate.reference.repo.clone(),
            ),
        };
        Some((candidate.connection.clone(), owner, name))
    }

    pub fn open_pull_list(&mut self) {
        if self.pull_list_target().is_none() {
            return;
        }
        let list_request = self.pulls.list.list_request + 1;
        let detail_request = self.pulls.list.detail_request + 1;
        self.pulls.list = PullListState {
            open: true,
            list_request,
            detail_request,
            ..PullListState::default()
        };
        self.reload_pull_list();
    }

    pub fn close_pull_list(&mut self) {
        let list = &mut self.pulls.list;
        list.open = false;
        list.list_request += 1;
        list.detail_request += 1;
        list.version += 1;
    }

    pub fn set_pull_filter(&mut self, filter: PullFilter) {
        if self.pulls.list.filter == filter {
            return;
        }
        self.pulls.list.filter = filter;
        self.reload_pull_list();
    }

    pub fn reload_pull_list(&mut self) {
        let Some((connection, owner, name)) = self.pull_list_target() else {
            return;
        };
        let list = &mut self.pulls.list;
        list.list_request += 1;
        list.loading = true;
        list.error = None;
        list.version += 1;
        let request = list.list_request;
        let filter = list.filter;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = list_pulls(secrets.as_ref(), &connection, &owner, &name, filter)
                .await
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::PullListed { request, result });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_pull_listed(
        &mut self,
        request: u64,
        result: Result<(Vec<PullSummary>, bool), String>,
    ) -> bool {
        let list = &mut self.pulls.list;
        if !list.open || request != list.list_request {
            return false;
        }
        list.loading = false;
        match result {
            Ok((items, truncated)) => {
                list.items = items;
                list.truncated = truncated;
            }
            Err(message) => {
                list.items.clear();
                list.error = Some(message);
            }
        }
        list.version += 1;
        true
    }

    pub fn select_pull(&mut self, number: u64) {
        let previous = self.pulls.list.selected;
        self.pulls.list.selected = Some(number);
        if previous != Some(number) {
            self.pulls.list.detail = None;
            self.pulls.list.notice = None;
        }
        self.load_pull_detail();
    }

    pub fn load_pull_detail(&mut self) {
        let Some(number) = self.pulls.list.selected else {
            return;
        };
        let Some((connection, owner, name)) = self.pull_list_target() else {
            return;
        };
        let list = &mut self.pulls.list;
        list.detail_request += 1;
        list.detail_loading = true;
        list.detail_error = None;
        list.version += 1;
        let request = list.detail_request;
        let secrets = self.hosting.secrets.clone();
        let sender = self.event_tx();
        let waker = self.waker_slot();
        self.hosting.runtime.spawn(async move {
            let result = load_pull(secrets.as_ref(), &connection, &owner, &name, number)
                .await
                .map(Box::new)
                .map_err(|e| e.to_string());
            let _ = sender.send(Event::PullLoaded {
                request,
                number,
                result,
            });
            if let Some(wake) = waker.lock().unwrap().as_ref() {
                wake();
            }
        });
    }

    pub fn apply_pull_loaded(
        &mut self,
        request: u64,
        number: u64,
        result: Result<Box<PullDetail>, String>,
    ) -> bool {
        let list = &mut self.pulls.list;
        if !list.open || request != list.detail_request || list.selected != Some(number) {
            return false;
        }
        list.detail_loading = false;
        match result {
            Ok(detail) => {
                let moved = list
                    .detail
                    .as_ref()
                    .filter(|old| old.summary.number == number)
                    .filter(|old| old.summary.head_sha != detail.summary.head_sha)
                    .map(|old| {
                        format!(
                            "La pull request a été mise à jour depuis votre lecture ({} → {}).",
                            short(&old.summary.head_sha),
                            short(&detail.summary.head_sha)
                        )
                    });
                list.notice = moved.or_else(|| {
                    detail.changed_during_load.then(|| {
                        "La pull request a changé pendant le chargement : actualisez pour relire une version cohérente.".to_owned()
                    })
                });
                list.detail = Some(*detail);
            }
            Err(message) => {
                list.detail = None;
                list.detail_error = Some(message);
            }
        }
        list.version += 1;
        true
    }
}

pub fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}
