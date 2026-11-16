use std::path::{Path, PathBuf};

use crate::identity::{Generation, RepoId, SessionId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusCode {
    Unmodified,
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    Ignored,
}

impl StatusCode {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            b'.' => Self::Unmodified,
            b'M' => Self::Modified,
            b'T' => Self::TypeChanged,
            b'A' => Self::Added,
            b'D' => Self::Deleted,
            b'R' => Self::Renamed,
            b'C' => Self::Copied,
            b'U' => Self::Unmerged,
            b'?' => Self::Untracked,
            b'!' => Self::Ignored,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Ordinary,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    Ignored,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeEntry {
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub kind: EntryKind,
    pub index: StatusCode,
    pub worktree: StatusCode,
}

impl ChangeEntry {
    pub fn is_unstaged(&self) -> bool {
        match self.kind {
            EntryKind::Untracked => true,
            EntryKind::Unmerged | EntryKind::Ignored => false,
            _ => self.worktree != StatusCode::Unmodified,
        }
    }

    pub fn is_staged(&self) -> bool {
        match self.kind {
            EntryKind::Untracked | EntryKind::Unmerged | EntryKind::Ignored => false,
            _ => self.index != StatusCode::Unmodified,
        }
    }

    pub fn is_conflict(&self) -> bool {
        self.kind == EntryKind::Unmerged
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchInfo {
    pub oid: Option<String>,
    pub head: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusSnapshot {
    pub repo: RepoId,
    pub session: SessionId,
    pub generation: Generation,
    pub branch: BranchInfo,
    pub entries: Vec<ChangeEntry>,
}

impl StatusSnapshot {
    pub fn unstaged(&self) -> impl Iterator<Item = &ChangeEntry> {
        self.entries.iter().filter(|e| e.is_unstaged())
    }

    pub fn staged(&self) -> impl Iterator<Item = &ChangeEntry> {
        self.entries.iter().filter(|e| e.is_staged())
    }

    pub fn conflicts(&self) -> impl Iterator<Item = &ChangeEntry> {
        self.entries.iter().filter(|e| e.is_conflict())
    }
}

pub fn display_label(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
        .collect()
}
