use std::path::PathBuf;

use crate::error::Diagnostic;
use crate::status::ChangeEntry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampKind {
    File,
    Symlink,
    Directory,
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub kind: StampKind,
    pub size: u64,
    pub modified_ns: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageTarget {
    pub entry: ChangeEntry,
    pub stamp: Option<FileStamp>,
    pub index_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HunkSpec {
    pub index: usize,
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HunkLines {
    pub hunk: HunkSpec,
    pub ranges: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineSelection {
    pub hunks: Vec<HunkLines>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StageOutcome {
    Applied { staged: usize },
    HunkApplied,
    LinesApplied { lines: usize },
    Unstaged { files: usize },
    HunkUnstaged,
    LinesUnstaged { lines: usize },
    Stale { paths: Vec<PathBuf> },
    Refused { path: PathBuf, reason: Refusal },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Conflict,
    NothingToStage,
    NotPartial,
    FilterConfigured,
    NothingSelected,
    Incompatible,
    NothingToUnstage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteJob {
    Stage(Vec<StageTarget>),
    StageHunk {
        target: StageTarget,
        hunk: HunkSpec,
    },
    StageLines {
        target: StageTarget,
        selection: LineSelection,
    },
    Unstage(Vec<StageTarget>),
    UnstageHunk {
        target: StageTarget,
        hunk: HunkSpec,
    },
    UnstageLines {
        target: StageTarget,
        selection: LineSelection,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriteError {
    NoWorktree,
    Cancelled,
    IndexLocked(String),
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWorktree => f.write_str("the repository has no working directory"),
            Self::Cancelled => f.write_str("the request was cancelled"),
            Self::IndexLocked(path) => write!(f, "the index is locked: {path}"),
            Self::GitUnavailable(d) => write!(f, "Git could not be started: {d}"),
            Self::GitFailed(d) => write!(f, "Git failed: {d}"),
        }
    }
}

impl std::error::Error for WriteError {}
