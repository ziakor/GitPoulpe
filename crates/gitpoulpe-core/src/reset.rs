use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

impl ResetMode {
    pub fn flag(self) -> &'static str {
        match self {
            Self::Soft => "--soft",
            Self::Mixed => "--mixed",
            Self::Hard => "--hard",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResetCommit {
    pub oid: String,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResetPreview {
    pub branch: Option<String>,
    pub head: String,
    pub target: String,
    pub leaving: Vec<ResetCommit>,
    pub leaving_truncated: bool,
    pub entering: usize,
    pub staged: Vec<PathBuf>,
    pub unstaged: Vec<PathBuf>,
    pub would_lose: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResetOutcome {
    pub mode: ResetMode,
    pub old_head: String,
    pub new_head: String,
    pub head_marker: String,
    pub work_marker: Option<String>,
}
