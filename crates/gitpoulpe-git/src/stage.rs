use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use gitpoulpe_core::{
    Diagnostic, FileStamp, Generation, Refusal, SessionInfo, StageOutcome, StageTarget, StampKind,
    WriteError,
};

use crate::process::RunError;
use crate::{Cancel, Git};

pub fn stamp_of(path: &Path) -> FileStamp {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return FileStamp {
            kind: StampKind::Missing,
            size: 0,
            modified_ns: 0,
        };
    };
    let kind = if meta.file_type().is_symlink() {
        StampKind::Symlink
    } else if meta.is_dir() {
        StampKind::Directory
    } else {
        StampKind::File
    };
    let modified_ns = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| i128::try_from(d.as_nanos()).unwrap_or(0));
    FileStamp {
        kind,
        size: meta.len(),
        modified_ns,
    }
}

fn write_failure(error: RunError) -> WriteError {
    match error {
        RunError::Cancelled => WriteError::Cancelled,
        RunError::Spawn(e) => WriteError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => WriteError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Rejected(message) => WriteError::GitFailed(Diagnostic {
            exit_code: None,
            signal: None,
            message,
            cause: None,
        }),
    }
}

fn locked_index(diagnostic: &Diagnostic) -> Option<String> {
    let message = &diagnostic.message;
    let start = message.find("Unable to create '")? + "Unable to create '".len();
    let rest = &message[start..];
    let end = rest.find("index.lock")? + "index.lock".len();
    rest[end..]
        .starts_with("': File exists")
        .then(|| rest[..end].to_owned())
}

fn nul_separated(paths: &[&Path]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for path in paths {
        bytes.extend_from_slice(path.as_os_str().as_bytes());
        bytes.push(0);
    }
    bytes
}

impl Git {
    pub async fn stage(
        &self,
        info: &SessionInfo,
        targets: &[StageTarget],
        cancel: &Cancel,
    ) -> Result<StageOutcome, WriteError> {
        let worktree = info.worktree.as_ref().ok_or(WriteError::NoWorktree)?;
        if let Some(conflict) = targets.iter().find(|t| t.entry.is_conflict()) {
            return Ok(StageOutcome::Refused {
                path: conflict.entry.path.clone(),
                reason: Refusal::Conflict,
            });
        }
        if let Some(none) = targets.iter().find(|t| !t.entry.is_unstaged()) {
            return Ok(StageOutcome::Refused {
                path: none.entry.path.clone(),
                reason: Refusal::NothingToStage,
            });
        }
        if targets.is_empty() {
            return Ok(StageOutcome::Applied { staged: 0 });
        }
        let current = self
            .read_status(info, Generation::default().next(), cancel)
            .await
            .map_err(|e| match e {
                gitpoulpe_core::ReadError::Cancelled => WriteError::Cancelled,
                gitpoulpe_core::ReadError::NoWorktree => WriteError::NoWorktree,
                gitpoulpe_core::ReadError::GitUnavailable(d) => WriteError::GitUnavailable(d),
                gitpoulpe_core::ReadError::GitFailed(d) => WriteError::GitFailed(d),
                gitpoulpe_core::ReadError::Parse(message) => WriteError::GitFailed(Diagnostic {
                    exit_code: None,
                    signal: None,
                    message,
                    cause: None,
                }),
            })?;
        let by_path: HashMap<&Path, _> = current
            .entries
            .iter()
            .map(|e| (e.path.as_path(), e))
            .collect();
        let stale: Vec<PathBuf> = targets
            .iter()
            .filter(|target| {
                let unchanged = by_path.get(target.entry.path.as_path()).is_some_and(|now| {
                    now.kind == target.entry.kind
                        && now.index == target.entry.index
                        && now.worktree == target.entry.worktree
                        && now.original_path == target.entry.original_path
                });
                let same_file = target
                    .stamp
                    .is_none_or(|seen| current.stamps.get(&target.entry.path) == Some(&seen));
                !(unchanged && same_file)
            })
            .map(|target| target.entry.path.clone())
            .collect();
        if !stale.is_empty() {
            return Ok(StageOutcome::Stale { paths: stale });
        }

        let paths: Vec<&Path> = targets.iter().map(|t| t.entry.path.as_path()).collect();
        let args: Vec<OsString> = ["add", "--pathspec-from-file=-", "--pathspec-file-nul"]
            .map(OsString::from)
            .to_vec();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let env = [(OsString::from("GIT_LITERAL_PATHSPECS"), OsString::from("1"))];
        let run = self
            .run_input(
                worktree,
                &refs,
                &env,
                Some(nul_separated(&paths)),
                cancel,
                |_| Ok(()),
            )
            .await
            .map_err(write_failure)?;
        if !run.succeeded() {
            let diagnostic = Diagnostic {
                cause: None,
                ..run.diagnostic()
            };
            return Err(match locked_index(&diagnostic) {
                Some(path) => WriteError::IndexLocked(path),
                None => WriteError::GitFailed(diagnostic),
            });
        }
        Ok(StageOutcome::Applied {
            staged: targets.len(),
        })
    }
}
