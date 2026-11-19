use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Editor,
    Diff,
    Merge,
    Shell,
}

impl ToolKind {
    pub const ALL: [Self; 4] = [Self::Editor, Self::Diff, Self::Merge, Self::Shell];

    pub fn caption(self) -> &'static str {
        match self {
            Self::Editor => "Éditeur",
            Self::Diff => "Outil de comparaison",
            Self::Merge => "Outil de fusion",
            Self::Shell => "Terminal (shell)",
        }
    }

    pub fn variables(self) -> &'static [&'static str] {
        match self {
            Self::Editor => &["file", "repo"],
            Self::Diff => &["left", "right", "file", "repo"],
            Self::Merge => &["base", "ours", "theirs", "merged", "repo"],
            Self::Shell => &["repo"],
        }
    }

    pub fn example(self) -> &'static str {
        match self {
            Self::Editor => "--wait {file}",
            Self::Diff => "--diff {left} {right}",
            Self::Merge => "--merge {base} {ours} {theirs} {merged}",
            Self::Shell => "-l",
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
#[serde(default)]
pub struct ToolBook {
    pub editor: ToolConfig,
    pub diff: ToolConfig,
    pub merge: ToolConfig,
    pub shell: ToolConfig,
}

impl ToolBook {
    pub fn get(&self, kind: ToolKind) -> &ToolConfig {
        match kind {
            ToolKind::Editor => &self.editor,
            ToolKind::Diff => &self.diff,
            ToolKind::Merge => &self.merge,
            ToolKind::Shell => &self.shell,
        }
    }

    pub fn set(&mut self, kind: ToolKind, config: ToolConfig) {
        match kind {
            ToolKind::Editor => self.editor = config,
            ToolKind::Diff => self.diff = config,
            ToolKind::Merge => self.merge = config,
            ToolKind::Shell => self.shell = config,
        }
    }
}
