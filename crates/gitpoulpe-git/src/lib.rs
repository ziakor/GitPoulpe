mod blame;
mod branch;
mod cache;
mod cancel;
mod clone;
mod commit;
mod conflict;
mod diagnostics;
mod diff;
mod diff_parse;
mod exec;
mod fetch;
mod file_history;
mod gitflow;
mod history;
mod hooks;
mod identity;
mod ignore;
mod init;
mod inspect;
mod lfs;
mod merge;
mod net;
mod open;
mod parse;
mod process;
mod pull;
mod pull_range;
mod push;
mod rebase;
mod reflog;
mod remote;
mod reset;
mod sequence;
mod ssh;
mod stage;
mod stash;
mod status;
mod submodule;
mod tag;
mod undo;
mod watch;
mod worktree;
mod worktree_file;

pub use blame::{Blame, BlameLine};
pub use branch::{BranchError, DeleteMode};
pub use cache::{CacheConfig, CacheStats, DiffCache};
pub use cancel::Cancel;
pub use clone::{CloneError, repository_name_from_url, valid_directory_name};
pub use conflict::{AbortReport, ConflictError, ConflictSide, ContinueOutcome};
pub use diagnostics::filter_diagnostic;
pub use diff::{DiffDocument, DiffError, DiffReader};
pub use diff_parse::{DiffParseError, DiffParser, DiffSummary};
pub use exec::ExecError;
pub use fetch::{FetchOutcome, RefUpdate};
pub use file_history::{FileChange, FileHistory, FileRevision, FileRevisionView};
pub use gitflow::GitflowError;
pub use history::{HistoryError, HistoryStream};
pub use hooks::{HookError, KNOWN_HOOKS};
pub use identity::{
    Identity, IdentityError, SigningCheck, check_signing, check_signing_with_env, profile_env,
    signing_config,
};
pub use ignore::{IgnoreError, IgnoredList, MAX_IGNORED_ENTRIES};
pub use init::{DEFAULT_BRANCH, InitError, Initialized};
pub use inspect::InspectError;
pub use lfs::{LfsError, not_installed_help, parse_pointer};
pub use merge::MergeError;
pub use net::{NetError, NetProgress, SshTools, parse_progress, remote_host};
pub use parse::{ParseError, StatusParser};
pub use pull::{PullError, PullRequest};
pub use pull_range::{PullRange, PullRangeError, ReviewRefs};
pub use push::{PushError, PushRejection, PushRequest};
pub use rebase::{RebaseError, validate_plan};
pub use reflog::{REFLOG_LIMIT, ReflogError};
pub use remote::{RemoteError, validate_remote_url};
pub use reset::ResetError;
pub use sequence::{SequenceError, SequenceRequest};
pub use ssh::{HostKey, RemoteCheck};
pub use stage::stamp_of;
pub use stash::StashError;
pub use submodule::SubmoduleError;
pub use tag::TagError;
pub use undo::UndoError;
pub use watch::{MAX_WAIT, QUIET_PERIOD, RepoWatcher, Signal, WatchError, coalesce, is_relevant};
pub use worktree::{BranchChoice, WorktreeError};
pub use worktree_file::{
    EDIT_LIMIT, EditError, FileToken, LineEnding, WorktreeFile, read_worktree_file,
    save_worktree_file,
};

use std::ffi::OsString;

#[derive(Clone, Debug)]
pub struct Git {
    program: OsString,
    env: Vec<(OsString, OsString)>,
    pub(crate) ssh: SshTools,
}

impl Git {
    pub fn new() -> Self {
        Self {
            program: OsString::from("git"),
            env: Vec::new(),
            ssh: SshTools::default(),
        }
    }

    pub fn with_program(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            env: Vec::new(),
            ssh: SshTools::default(),
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
