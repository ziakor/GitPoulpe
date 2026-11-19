use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Editor,
    Diff,
    Merge,
}

impl ToolKind {
    pub const ALL: [Self; 3] = [Self::Editor, Self::Diff, Self::Merge];

    pub fn caption(self) -> &'static str {
        match self {
            Self::Editor => "Éditeur",
            Self::Diff => "Outil de comparaison",
            Self::Merge => "Outil de fusion",
        }
    }

    pub fn variables(self) -> &'static [&'static str] {
        match self {
            Self::Editor => &["file", "repo"],
            Self::Diff => &["left", "right", "file", "repo"],
            Self::Merge => &["base", "ours", "theirs", "merged", "repo"],
        }
    }

    pub fn example(self) -> &'static str {
        match self {
            Self::Editor => "--wait {file}",
            Self::Diff => "--diff {left} {right}",
            Self::Merge => "--merge {base} {ours} {theirs} {merged}",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolConfig {
    pub program: String,
    pub args: String,
}

impl ToolConfig {
    pub fn configured(&self) -> bool {
        !self.program.trim().is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolBook {
    pub editor: ToolConfig,
    pub diff: ToolConfig,
    pub merge: ToolConfig,
}

impl ToolBook {
    pub fn get(&self, kind: ToolKind) -> &ToolConfig {
        match kind {
            ToolKind::Editor => &self.editor,
            ToolKind::Diff => &self.diff,
            ToolKind::Merge => &self.merge,
        }
    }

    pub fn set(&mut self, kind: ToolKind, config: ToolConfig) {
        match kind {
            ToolKind::Editor => self.editor = config,
            ToolKind::Diff => self.diff = config,
            ToolKind::Merge => self.merge = config,
        }
    }
}
