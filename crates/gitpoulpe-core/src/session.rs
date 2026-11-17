use std::path::PathBuf;
use std::sync::Arc;

use crate::identity::{Generation, RepoId, SessionId};
use crate::status::StatusSnapshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoKind {
    Ordinary,
    LinkedWorktree,
    Bare,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: SessionId,
    pub repo: RepoId,
    pub kind: RepoKind,
    pub worktree: Option<PathBuf>,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub has_commits: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaleSnapshot {
    pub received: Generation,
    pub expected: Generation,
}

#[derive(Debug)]
pub struct SessionState {
    info: SessionInfo,
    requested: Generation,
    current: Option<Arc<StatusSnapshot>>,
}

impl SessionState {
    pub fn new(info: SessionInfo) -> Self {
        Self {
            info,
            requested: Generation::default(),
            current: None,
        }
    }

    pub fn info(&self) -> &SessionInfo {
        &self.info
    }

    pub fn current(&self) -> Option<&Arc<StatusSnapshot>> {
        self.current.as_ref()
    }

    pub fn begin_read(&mut self) -> Generation {
        self.requested = self.requested.next();
        self.requested
    }

    pub fn accept(&mut self, snapshot: StatusSnapshot) -> Result<(), StaleSnapshot> {
        let foreign = snapshot.repo != self.info.repo || snapshot.session != self.info.id;
        if foreign || snapshot.generation != self.requested {
            return Err(StaleSnapshot {
                received: snapshot.generation,
                expected: self.requested,
            });
        }
        self.current = Some(Arc::new(snapshot));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::status::BranchInfo;

    fn info(common: &str) -> SessionInfo {
        SessionInfo {
            id: SessionId::next(),
            repo: RepoId::from_common_dir(Path::new(common)),
            kind: RepoKind::Ordinary,
            worktree: Some(PathBuf::from("/w")),
            git_dir: PathBuf::from(common),
            common_dir: PathBuf::from(common),
            has_commits: true,
        }
    }

    fn snapshot(info: &SessionInfo, generation: Generation) -> StatusSnapshot {
        StatusSnapshot {
            repo: info.repo.clone(),
            session: info.id,
            generation,
            branch: BranchInfo::default(),
            entries: Vec::new(),
            stamps: Default::default(),
            index_ids: Default::default(),
        }
    }

    #[test]
    fn only_the_latest_requested_generation_replaces_the_current_state() {
        let mut state = SessionState::new(info("/nonexistent/a/.git"));
        let first = state.begin_read();
        let second = state.begin_read();
        let template = state.info().clone();

        let rejected = state.accept(snapshot(&template, first)).unwrap_err();
        assert_eq!(
            rejected,
            StaleSnapshot {
                received: first,
                expected: second
            }
        );
        assert!(state.current().is_none());

        state.accept(snapshot(&template, second)).unwrap();
        assert_eq!(state.current().unwrap().generation, second);

        let third = state.begin_read();
        assert!(state.accept(snapshot(&template, second)).is_err());
        assert_eq!(state.current().unwrap().generation, second);
        assert!(third > second);
    }

    #[test]
    fn a_snapshot_from_another_session_is_rejected_even_with_the_right_generation() {
        let mut state = SessionState::new(info("/nonexistent/a/.git"));
        let other = info("/nonexistent/b/.git");
        let generation = state.begin_read();
        assert!(state.accept(snapshot(&other, generation)).is_err());
        assert!(state.current().is_none());
    }
}
