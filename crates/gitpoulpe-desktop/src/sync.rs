use gitpoulpe_core::Tracking;
use gitpoulpe_git::FetchTarget;

use crate::remotes::FetchStatus;
use crate::state::State;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Detached,
    Local,
    Missing(String),
    Tracked(String, Tracking),
}

impl State {
    pub fn current_branch(&self) -> Option<&str> {
        let head = self.repo.as_ref()?.snapshot.branch.head.as_deref()?;
        (head != "(detached)").then_some(head)
    }

    pub fn link(&self) -> Option<Link> {
        let branch = &self.repo.as_ref()?.snapshot.branch;
        if self.current_branch().is_none() {
            return Some(Link::Detached);
        }
        Some(match (&branch.upstream, branch.tracking) {
            (None, _) => Link::Local,
            (Some(upstream), None) => Link::Missing(upstream.clone()),
            (Some(upstream), Some(tracking)) => Link::Tracked(upstream.clone(), tracking),
        })
    }

    pub fn sync_progress(&self) -> Option<(&str, u8)> {
        match &self.remotes.fetch.as_ref()?.status {
            FetchStatus::Running { phase, percent } => Some((phase.as_str(), *percent)),
            _ => None,
        }
    }

    pub fn sync_fetch(&mut self) {
        let Some(upstream) = self
            .repo
            .as_ref()
            .and_then(|r| r.snapshot.branch.upstream.clone())
        else {
            return;
        };
        let remote = upstream
            .split_once('/')
            .map_or(upstream.as_str(), |(remote, _)| remote)
            .to_owned();
        self.start_fetch(remote, FetchTarget::Upstream(upstream));
    }

    pub fn sync_integrate(&mut self) {
        if self.open_current_branch() {
            self.pull_ask();
        }
    }

    pub fn sync_publish(&mut self) {
        if self.open_current_branch() {
            self.publish_ask();
        }
    }

    fn open_current_branch(&mut self) -> bool {
        let Some(branch) = self.current_branch().map(str::to_owned) else {
            return false;
        };
        self.close_screens();
        self.open_branches();
        self.select_branch(&branch);
        true
    }
}
