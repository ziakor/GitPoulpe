use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeKind {
    UpToDate,
    FastForward,
    TrueMerge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMode {
    Auto,
    NoFastForward,
    FastForwardOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergePreview {
    pub current: Option<String>,
    pub head: String,
    pub source: String,
    pub source_oid: String,
    pub kind: MergeKind,
    pub only_in_head: usize,
    pub only_in_source: usize,
    pub predicted_conflicts: Option<Vec<PathBuf>>,
    pub local_changes: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    UpToDate,
    FastForward {
        from: String,
        to: String,
    },
    Merged {
        from: String,
        to: String,
        stash_kept: bool,
    },
    Conflicts {
        files: Vec<PathBuf>,
    },
}
