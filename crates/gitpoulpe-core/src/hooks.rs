use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookInfo {
    pub name: String,
    pub path: PathBuf,
    pub known: bool,
    pub executable: bool,
    pub symlink: bool,
    pub size: u64,
    pub interpreter: Option<String>,
}

impl HookInfo {
    pub fn active(&self) -> bool {
        self.known && self.executable
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HooksView {
    pub dir: PathBuf,
    pub configured_path: Option<String>,
    pub configured_scope: Option<String>,
    pub shared: bool,
    pub in_worktree: bool,
    pub hooks: Vec<HookInfo>,
}
