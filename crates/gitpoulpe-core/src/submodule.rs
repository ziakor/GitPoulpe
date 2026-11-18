use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmoduleState {
    NotInitialized,
    Initialized,
    UpToDate,
    Differs,
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submodule {
    pub name: String,
    pub path: PathBuf,
    pub url: Option<String>,
    pub url_refused: Option<String>,
    pub expected: String,
    pub current: Option<String>,
    pub state: SubmoduleState,
    pub ahead: usize,
    pub behind: usize,
    pub expected_available: bool,
    pub modified: usize,
    pub untracked: usize,
    pub depth: usize,
}

impl Submodule {
    pub fn dirty(&self) -> bool {
        self.modified + self.untracked > 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmoduleUpdated {
    pub path: PathBuf,
    pub from: Option<String>,
    pub to: String,
    pub marker: Option<String>,
}
