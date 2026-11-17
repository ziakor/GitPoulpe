mod cache;
mod cancel;
mod diagnostics;
mod diff;
mod diff_parse;
mod open;
mod parse;
mod process;
mod status;
mod watch;

pub use cache::{CacheConfig, CacheStats, DiffCache};
pub use cancel::Cancel;
pub use diagnostics::filter_diagnostic;
pub use diff::{DiffDocument, DiffError, DiffReader};
pub use diff_parse::{DiffParseError, DiffParser, DiffSummary};
pub use parse::{ParseError, StatusParser};
pub use watch::{MAX_WAIT, QUIET_PERIOD, RepoWatcher, Signal, WatchError, coalesce, is_relevant};

use std::ffi::OsString;

#[derive(Clone, Debug)]
pub struct Git {
    program: OsString,
}

impl Git {
    pub fn new() -> Self {
        Self {
            program: OsString::from("git"),
        }
    }

    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
        }
    }
}

impl Default for Git {
    fn default() -> Self {
        Self::new()
    }
}
