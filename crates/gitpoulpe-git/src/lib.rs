mod cache;
mod cancel;
mod commit;
mod diagnostics;
mod diff;
mod diff_parse;
mod history;
mod identity;
mod open;
mod parse;
mod process;
mod stage;
mod status;
mod watch;

pub use cache::{CacheConfig, CacheStats, DiffCache};
pub use cancel::Cancel;
pub use diagnostics::filter_diagnostic;
pub use diff::{DiffDocument, DiffError, DiffReader};
pub use diff_parse::{DiffParseError, DiffParser, DiffSummary};
pub use history::{HistoryError, HistoryStream};
pub use identity::{
    Identity, IdentityError, SigningCheck, check_signing, check_signing_with_env, profile_env,
    signing_config,
};
pub use parse::{ParseError, StatusParser};
pub use stage::stamp_of;
pub use watch::{MAX_WAIT, QUIET_PERIOD, RepoWatcher, Signal, WatchError, coalesce, is_relevant};

use std::ffi::OsString;

#[derive(Clone, Debug)]
pub struct Git {
    program: OsString,
    env: Vec<(OsString, OsString)>,
}

impl Git {
    pub fn new() -> Self {
        Self {
            program: OsString::from("git"),
            env: Vec::new(),
        }
    }

    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            env: Vec::new(),
        }
    }

    pub fn with_env(mut self, name: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((name.into(), value.into()));
        self
    }

    pub(crate) fn extra_env(&self) -> &[(OsString, OsString)] {
        &self.env
    }
}

impl Default for Git {
    fn default() -> Self {
        Self::new()
    }
}
