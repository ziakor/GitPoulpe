use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullStrategy {
    FastForwardOnly,
    Merge,
    Rebase,
}

impl PullStrategy {
    pub fn caption(self) -> &'static str {
        match self {
            Self::FastForwardOnly => "avance rapide seulement",
            Self::Merge => "fusion",
            Self::Rebase => "rebase",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullKind {
    UpToDate,
    Behind,
    Ahead,
    Diverged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullPreview {
    pub branch: String,
    pub remote: String,
    pub upstream: String,
    pub head: String,
    pub tracking: Option<String>,
    pub kind: PullKind,
    pub ahead: usize,
    pub behind: usize,
    pub local_changes: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Integration {
    UpToDate,
    FastForward {
        from: String,
        to: String,
    },
    Merged {
        from: String,
        to: String,
    },
    Rebased {
        from: String,
        to: String,
        marker: String,
    },
    Conflicts {
        files: Vec<PathBuf>,
    },
    Paused,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullOutcome {
    pub fetched_updates: usize,
    pub integration: Integration,
}
