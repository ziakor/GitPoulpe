use std::ffi::OsString;

use gitpoulpe_core::{Diagnostic, Generation, ReadError, SessionInfo, StatusSnapshot};

use crate::parse::StatusParser;
use crate::process::RunError;
use crate::{Cancel, Git};

fn read_failure(error: RunError) -> ReadError {
    match error {
        RunError::Cancelled => ReadError::Cancelled,
        RunError::Spawn(e) => ReadError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => ReadError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Rejected(message) => ReadError::Parse(message),
    }
}

impl Git {
    pub async fn read_status(
        &self,
        info: &SessionInfo,
        generation: Generation,
        cancel: &Cancel,
    ) -> Result<StatusSnapshot, ReadError> {
        let worktree = info.worktree.as_ref().ok_or(ReadError::NoWorktree)?;
        let args = [
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
            "--no-ahead-behind",
        ]
        .map(OsString::from);
        let refs: Vec<&std::ffi::OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut parser = StatusParser::new();
        let run = self
            .run(worktree, &refs, cancel, |chunk| {
                parser.push(chunk).map_err(|e| e.to_string())
            })
            .await
            .map_err(read_failure)?;
        if !run.succeeded() {
            return Err(ReadError::GitFailed(Diagnostic {
                cause: None,
                ..run.diagnostic()
            }));
        }
        parser
            .finish(info.repo.clone(), info.id, generation)
            .map_err(|e| ReadError::Parse(e.to_string()))
    }
}
