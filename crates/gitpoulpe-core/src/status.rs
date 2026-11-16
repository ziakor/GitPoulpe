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

fn is_unsafe_for_display(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
        )
}

pub fn display_label(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| {
            if is_unsafe_for_display(c) {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_bidi_and_zero_width_characters_never_reach_the_label() {
        let spoofed = Path::new("report\u{202E}gpj.exe\n\t\u{200B}\u{2066}x\u{FEFF}");
        let label = display_label(spoofed);
        assert!(label.chars().all(|c| !is_unsafe_for_display(c)));
        assert_eq!(
            label,
            "report\u{FFFD}gpj.exe\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}x\u{FFFD}"
        );
    }

    #[test]
    fn ordinary_unicode_names_are_kept() {
        assert_eq!(
            display_label(Path::new("dossier/é à ü 日本語.txt")),
            "dossier/é à ü 日本語.txt"
        );
    }
}
