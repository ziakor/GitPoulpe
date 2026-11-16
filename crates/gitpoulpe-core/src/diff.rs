use std::ops::Range;
use std::path::PathBuf;

use crate::error::Diagnostic;
use crate::identity::{Generation, RepoId, SessionId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiffSide {
    Unstaged,
    Staged,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitRow {
    Header(usize),
    Pair {
        left: Option<usize>,
        right: Option<usize>,
    },
}

pub fn split_rows(lines: &[DiffLine]) -> Vec<SplitRow> {
    let mut rows = Vec::new();
    let mut removed: Vec<usize> = Vec::new();
    let mut added: Vec<usize> = Vec::new();
    let flush = |rows: &mut Vec<SplitRow>, removed: &mut Vec<usize>, added: &mut Vec<usize>| {
        for i in 0..removed.len().max(added.len()) {
            rows.push(SplitRow::Pair {
                left: removed.get(i).copied(),
                right: added.get(i).copied(),
            });
        }
        removed.clear();
        added.clear();
    };
    let mut previous = None;
    for (index, line) in lines.iter().enumerate() {
        match line.kind {
            DiffLineKind::Removed => removed.push(index),
            DiffLineKind::Added => added.push(index),
            DiffLineKind::NoNewlineAtEof => match previous {
                Some(DiffLineKind::Removed) => {
                    let after = removed.len();
                    flush_marker(&mut rows, &mut removed, &mut added, index, true, after);
                }
                Some(DiffLineKind::Added) => {
                    let after = added.len();
                    flush_marker(&mut rows, &mut removed, &mut added, index, false, after);
                }
                _ => {
                    flush(&mut rows, &mut removed, &mut added);
                    rows.push(SplitRow::Pair {
                        left: Some(index),
                        right: Some(index),
                    });
                }
            },
            DiffLineKind::HunkHeader => {
                flush(&mut rows, &mut removed, &mut added);
                rows.push(SplitRow::Header(index));
            }
            DiffLineKind::Context => {
                flush(&mut rows, &mut removed, &mut added);
                rows.push(SplitRow::Pair {
                    left: Some(index),
                    right: Some(index),
                });
            }
        }
        previous = Some(line.kind);
    }
    flush(&mut rows, &mut removed, &mut added);
    rows
}

fn flush_marker(
    rows: &mut Vec<SplitRow>,
    removed: &mut Vec<usize>,
    added: &mut Vec<usize>,
    marker: usize,
    left_side: bool,
    after: usize,
) {
    let (own, other) = if left_side {
        (removed, added)
    } else {
        (added, removed)
    };
    let rows_before = rows.len();
    for i in 0..own.len().max(other.len()) {
        let own_index = own.get(i).copied();
        let other_index = other.get(i).copied();
        let (left, right) = if left_side {
            (own_index, other_index)
        } else {
            (other_index, own_index)
        };
        rows.push(SplitRow::Pair { left, right });
    }
    let target = rows_before + after.saturating_sub(1);
    if let Some(SplitRow::Pair { left, right }) = rows.get_mut(target) {
        let _ = (left, right);
    }
    rows.push(if left_side {
        SplitRow::Pair {
            left: Some(marker),
            right: None,
        }
    } else {
        SplitRow::Pair {
            left: None,
            right: Some(marker),
        }
    });
    own.clear();
    other.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: DiffLineKind) -> DiffLine {
        DiffLine {
            kind,
            old_number: None,
            new_number: None,
            text: Vec::new(),
        }
    }

    #[test]
    fn removed_and_added_blocks_are_paired_in_order_with_leftovers_alone() {
        use DiffLineKind::*;
        let lines: Vec<_> = [
            HunkHeader, Context, Removed, Removed, Removed, Added, Context,
        ]
        .into_iter()
        .map(line)
        .collect();
        assert_eq!(
            split_rows(&lines),
            vec![
                SplitRow::Header(0),
                SplitRow::Pair {
                    left: Some(1),
                    right: Some(1)
                },
                SplitRow::Pair {
                    left: Some(2),
                    right: Some(5)
                },
                SplitRow::Pair {
                    left: Some(3),
                    right: None
                },
                SplitRow::Pair {
                    left: Some(4),
                    right: None
                },
                SplitRow::Pair {
                    left: Some(6),
                    right: Some(6)
                },
            ]
        );
    }

    #[test]
    fn a_missing_newline_marker_stays_on_the_side_it_belongs_to() {
        use DiffLineKind::*;
        let lines: Vec<_> = [Removed, NoNewlineAtEof, Added]
            .into_iter()
            .map(line)
            .collect();
        assert_eq!(
            split_rows(&lines),
            vec![
                SplitRow::Pair {
                    left: Some(0),
                    right: None
                },
                SplitRow::Pair {
                    left: Some(1),
                    right: None
                },
                SplitRow::Pair {
                    left: None,
                    right: Some(2)
                },
            ]
        );
    }

    #[test]
    fn every_line_appears_in_exactly_one_row() {
        use DiffLineKind::*;
        let lines: Vec<_> = [
            HunkHeader,
            Removed,
            Added,
            Added,
            NoNewlineAtEof,
            Context,
            Removed,
        ]
        .into_iter()
        .map(line)
        .collect();
        let mut seen = vec![0; lines.len()];
        for row in split_rows(&lines) {
            match row {
                SplitRow::Header(i) => seen[i] += 1,
                SplitRow::Pair { left, right } => {
                    if left == right {
                        seen[left.unwrap()] += 1;
                    } else {
                        for i in [left, right].into_iter().flatten() {
                            seen[i] += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(seen, vec![1; lines.len()]);
    }

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
        };
        assert_eq!(meta.align_to_hunks(3..5), 0..10);
        assert_eq!(meta.align_to_hunks(8..14), 0..22);
        assert_eq!(meta.align_to_hunks(25..30), 22..30);
        assert_eq!(meta.total_lines(), Some(30));
    }
}
