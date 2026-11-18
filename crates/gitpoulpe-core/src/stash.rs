use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stash {
    pub index: usize,
    pub oid: String,
    pub branch: Option<String>,
    pub message: String,
    pub timestamp: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashFile {
    pub path: PathBuf,
    pub status: char,
    pub staged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashDetail {
    pub files: Vec<StashFile>,
    pub untracked: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StashScope {
    pub include_untracked: bool,
    pub keep_index: bool,
    pub staged_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StashApplied {
    Clean { dropped: bool },
    Conflicts { files: Vec<PathBuf> },
}
