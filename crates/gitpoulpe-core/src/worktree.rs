use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    pub bare: bool,
    pub main: bool,
    pub current: bool,
    pub locked: Option<String>,
    pub prunable: Option<String>,
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
}

impl Worktree {
    pub fn dirty(&self) -> bool {
        self.staged + self.unstaged + self.untracked > 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeRemoval {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub locked: Option<String>,
    pub changed: Vec<PathBuf>,
    pub untracked: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeRemoved {
    pub path: PathBuf,
    pub snapshot: Option<String>,
    pub lost_untracked: usize,
}
