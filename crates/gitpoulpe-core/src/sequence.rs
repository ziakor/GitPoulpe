use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceKind {
    CherryPick,
    Revert,
}

impl SequenceKind {
    pub fn git_name(self) -> &'static str {
        match self {
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmptyPolicy {
    Drop,
    Keep,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequenceCommit {
    pub oid: String,
    pub subject: String,
    pub parents: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequencePreview {
    pub kind: SequenceKind,
    pub branch: Option<String>,
    pub head: String,
    pub commits: Vec<SequenceCommit>,
    pub merges: usize,
    pub local_changes: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SequenceOutcome {
    Done { created: usize, head: String },
    Conflicts { files: Vec<PathBuf> },
    Stopped { reason: String },
}
