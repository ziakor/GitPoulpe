use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::profile::Profile;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitRequest {
    pub message: String,
    pub staged: Vec<(PathBuf, String)>,
    pub profile: Option<Profile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmendInfo {
    pub head: String,
    pub message: String,
    pub published_on: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmendRequest {
    pub commit: CommitRequest,
    pub head: String,
    pub confirmed_published: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitRefusal {
    EmptyMessage,
    EmptyIndex,
    Conflicts,
    Stale,
    NoCommit,
    NothingToAmend,
    Published,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitFailureKind {
    IdentityUnknown,
    SigningUnavailable,
    Hook,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitOutcome {
    Created {
        oid: String,
        subject: String,
        before: Option<String>,
        signed: bool,
    },
    Refused(CommitRefusal),
    Failed {
        kind: CommitFailureKind,
        output: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub at: u64,
    pub action: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub detail: String,
}

pub const JOURNAL_LIMIT: usize = 50;

pub fn push_journal(journal: &mut Vec<JournalEntry>, entry: JournalEntry) {
    journal.push(entry);
    if journal.len() > JOURNAL_LIMIT {
        let excess = journal.len() - JOURNAL_LIMIT;
        journal.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journal_keeps_only_the_most_recent_entries() {
        let mut journal = Vec::new();
        for n in 0..(JOURNAL_LIMIT + 7) {
            push_journal(
                &mut journal,
                JournalEntry {
                    at: n as u64,
                    action: "commit".into(),
                    before: None,
                    after: Some(format!("{n}")),
                    detail: String::new(),
                },
            );
        }
        assert_eq!(journal.len(), JOURNAL_LIMIT);
        assert_eq!(journal.first().unwrap().at, 7);
        assert_eq!(journal.last().unwrap().at, (JOURNAL_LIMIT + 6) as u64);
    }
}
