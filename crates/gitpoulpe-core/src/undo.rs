use serde::{Deserialize, Serialize};

pub const UNDO_LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndoKind {
    RefMove {
        reference: String,
        before: String,
        after: String,
    },
    BranchCreate {
        name: String,
        tip: String,
    },
    BranchDelete {
        name: String,
        tip: String,
    },
    BranchRename {
        old: String,
        new: String,
    },
    TagCreate {
        name: String,
        object: String,
    },
    TagDelete {
        name: String,
        object: String,
    },
    Irreversible {
        why: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UndoState {
    Done,
    Undone,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoRecord {
    pub id: u64,
    pub at: u64,
    pub label: String,
    pub kind: UndoKind,
    pub state: UndoState,
    pub limit: Option<String>,
}

impl UndoRecord {
    pub fn reversible(&self) -> bool {
        !matches!(self.kind, UndoKind::Irreversible { .. })
    }
}

pub fn push_undo(stack: &mut Vec<UndoRecord>, mut record: UndoRecord) {
    record.id = stack.iter().map(|r| r.id).max().unwrap_or(0) + 1;
    stack.push(record);
    if stack.len() > UNDO_LIMIT {
        let excess = stack.len() - UNDO_LIMIT;
        stack.drain(..excess);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoDirection {
    Undo,
    Redo,
}
