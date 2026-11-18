use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    pub oid: String,
    pub subject: String,
    pub upstream: Option<String>,
    pub current: bool,
    pub other_worktree: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LostCommit {
    pub oid: String,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchLoss {
    pub tip: String,
    pub commits: Vec<LostCommit>,
    pub truncated: bool,
}

pub const LOSS_LIST_LIMIT: usize = 100;
