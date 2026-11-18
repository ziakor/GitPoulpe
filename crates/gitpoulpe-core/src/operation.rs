use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    StashApply,
}

impl OperationKind {
    pub fn git_name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
            Self::StashApply => "stash",
        }
    }

    pub fn caption(self) -> &'static str {
        match self {
            Self::Merge => "fusion",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
            Self::StashApply => "application du stash",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operation {
    pub kind: OperationKind,
    pub ours_label: String,
    pub theirs_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    BothModified,
    BothAdded,
    DeletedByUs,
    DeletedByThem,
    AddedByUs,
    AddedByThem,
    BothDeleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictFile {
    pub path: PathBuf,
    pub kind: ConflictKind,
    pub binary: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Unresolved,
    Resolved,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub size: u64,
    pub text: Option<String>,
    pub oid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictVersions {
    pub file: ConflictFile,
    pub base: Option<Version>,
    pub ours: Option<Version>,
    pub theirs: Option<Version>,
    pub merged: Option<String>,
}
