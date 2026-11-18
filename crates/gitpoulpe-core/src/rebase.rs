use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebaseAction {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}

impl RebaseAction {
    pub fn caption(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reword => "reword",
            Self::Edit => "edit",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
        }
    }

    pub fn melds(self) -> bool {
        matches!(self, Self::Squash | Self::Fixup)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebaseCommit {
    pub oid: String,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebasePlanInfo {
    pub branch: Option<String>,
    pub head: String,
    pub base: String,
    pub commits: Vec<RebaseCommit>,
    pub local_changes: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItem {
    pub oid: String,
    pub action: RebaseAction,
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebaseOutcome {
    Done { head: String, marker: String },
    Conflicts { files: Vec<PathBuf>, marker: String },
    Paused { marker: String },
}
