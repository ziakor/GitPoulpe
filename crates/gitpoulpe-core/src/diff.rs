use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::Diagnostic;
use crate::identity::{Generation, RepoId, SessionId};
use crate::split::SplitIndex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiffSide {
    Unstaged,
    Staged,
    Commit,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommitRange {
    pub oid: String,
    pub base: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileRef {
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiffKey {
    pub repo: RepoId,
    pub session: SessionId,
    pub generation: Generation,
    pub side: DiffSide,
    pub file: FileRef,
    pub range: Option<CommitRange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    HunkHeader,
    Context,
    Added,
    Removed,
    NoNewlineAtEof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_number: Option<u32>,
    pub new_number: Option<u32>,
    pub text: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub first_line: u64,
    pub line_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentKind {
    Text,
    Binary,
    Conflict,
    ModeChange { old: String, new: String },
    RenameOnly { from: PathBuf, to: PathBuf },
    NoChanges,
    Unreadable(Diagnostic),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildState {
    Building,
    Complete,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentMeta {
    pub state: BuildState,
    pub kind: Option<DocumentKind>,
    pub lines: u64,
    pub added: u64,
    pub removed: u64,
    pub hunks: Vec<Hunk>,
    pub type_changed: bool,
    pub split: Option<Arc<SplitIndex>>,
}

impl DocumentMeta {
    pub fn total_lines(&self) -> Option<u64> {
        (self.state == BuildState::Complete).then_some(self.lines)
    }

    pub fn align_to_hunks(&self, range: Range<u64>) -> Range<u64> {
        let start = self
            .hunks
            .iter()
            .rev()
            .find(|h| h.first_line <= range.start)
            .map_or(range.start, |h| h.first_line);
        let end = self
            .hunks
            .iter()
            .find(|h| h.first_line + h.line_count >= range.end)
            .map_or(range.end, |h| h.first_line + h.line_count);
        start..end.max(start)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeData {
    pub start: u64,
    pub lines: Vec<DiffLine>,
    pub total_lines: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeError {
    Cancelled,
    OutOfRange { requested: u64, total: u64 },
    NotReadable(DocumentKind),
    Stale,
    Storage(String),
}

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("the request was cancelled"),
            Self::OutOfRange { requested, total } => {
                write!(
                    f,
                    "line {requested} is past the end of the document ({total} lines)"
                )
            }
            Self::NotReadable(kind) => write!(f, "the document has no text lines: {kind:?}"),
            Self::Stale => f.write_str("the file changed since this document was built; reload it"),
            Self::Storage(message) => write!(f, "diff storage failed: {message}"),
        }
    }
}

impl std::error::Error for RangeError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_aligned_to_whole_hunks() {
        let hunk = |first_line, line_count| Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 1,
            first_line,
            line_count,
        };
        let meta = DocumentMeta {
            state: BuildState::Complete,
            kind: Some(DocumentKind::Text),
            lines: 30,
            added: 0,
            removed: 0,
            hunks: vec![hunk(0, 10), hunk(10, 12), hunk(22, 8)],
            type_changed: false,
            split: None,
        };
        assert_eq!(meta.align_to_hunks(3..5), 0..10);
        assert_eq!(meta.align_to_hunks(8..14), 0..22);
        assert_eq!(meta.align_to_hunks(25..30), 22..30);
        assert_eq!(meta.total_lines(), Some(30));
    }
}
