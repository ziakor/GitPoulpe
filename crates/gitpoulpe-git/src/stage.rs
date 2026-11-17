use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use gitpoulpe_core::{
    Diagnostic, FileStamp, Generation, HunkSpec, Refusal, SessionInfo, StageOutcome, StageTarget,
    StampKind, WriteError,
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

    async fn current_status(
        &self,
        info: &SessionInfo,
        cancel: &Cancel,
    ) -> Result<gitpoulpe_core::StatusSnapshot, WriteError> {
        self.read_status(info, Generation::default().next(), cancel)
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
            })
    }

    pub async fn stage_hunk(
        &self,
        info: &SessionInfo,
        target: &StageTarget,
        hunk: &HunkSpec,
        cancel: &Cancel,
    ) -> Result<StageOutcome, WriteError> {
        let worktree = info.worktree.as_ref().ok_or(WriteError::NoWorktree)?;
        let entry = &target.entry;
        let refuse = |reason| {
            Ok(StageOutcome::Refused {
                path: entry.path.clone(),
                reason,
            })
        };
        if entry.is_conflict() {
            return refuse(Refusal::Conflict);
        }
        if entry.kind != gitpoulpe_core::EntryKind::Ordinary
            || entry.worktree != gitpoulpe_core::StatusCode::Modified
        {
            return refuse(Refusal::NotPartial);
        }
        let stale = || {
            Ok(StageOutcome::Stale {
                paths: vec![entry.path.clone()],
            })
        };
        let Some(seen) = target.stamp else {
            return stale();
        };
        let current = self.current_status(info, cancel).await?;
        let unchanged = current.entries.iter().any(|now| {
            now.path == entry.path
                && now.kind == entry.kind
                && now.index == entry.index
                && now.worktree == entry.worktree
                && now.original_path == entry.original_path
        });
        if !unchanged || current.stamps.get(&entry.path) != Some(&seen) {
            return stale();
        }
        let filters = self
            .filter_overrides(worktree, cancel)
            .await
            .map_err(|e| match e {
                gitpoulpe_core::ReadError::Cancelled => WriteError::Cancelled,
                gitpoulpe_core::ReadError::GitUnavailable(d) => WriteError::GitUnavailable(d),
                other => WriteError::GitFailed(Diagnostic {
                    exit_code: None,
                    signal: None,
                    message: other.to_string(),
                    cause: None,
                }),
            })?;
        if !filters.is_empty() {
            return refuse(Refusal::FilterConfigured);
        }

        let mut spec = b":(literal)".to_vec();
        spec.extend_from_slice(entry.path.as_os_str().as_bytes());
        let args: Vec<OsString> = vec![
            "diff".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            "--no-color".into(),
            "--unified=3".into(),
            "--".into(),
            OsString::from(std::ffi::OsStr::from_bytes(&spec)),
        ];
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let (run, diff) = self
            .run_collect(worktree, &refs, cancel)
            .await
            .map_err(write_failure)?;
        if !run.succeeded() {
            return Err(WriteError::GitFailed(Diagnostic {
                cause: None,
                ..run.diagnostic()
            }));
        }
        let Some(patch) = single_hunk_patch(&diff, hunk) else {
            return match split_diff(&diff) {
                Some(_) => stale(),
                None => refuse(Refusal::NotPartial),
            };
        };
        let args: Vec<OsString> = ["apply", "--cached", "--recount", "--whitespace=nowarn", "-"]
            .map(OsString::from)
            .to_vec();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let run = self
            .run_input(worktree, &refs, &[], Some(patch), cancel, |_| Ok(()))
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
        Ok(StageOutcome::HunkApplied)
    }
}

struct SplitDiff<'a> {
    header: Vec<&'a [u8]>,
    hunks: Vec<Vec<&'a [u8]>>,
}

fn split_diff(diff: &[u8]) -> Option<SplitDiff<'_>> {
    let mut header = Vec::new();
    let mut hunks: Vec<Vec<&[u8]>> = Vec::new();
    for line in diff.split_inclusive(|byte| *byte == b'\n') {
        if line.starts_with(b"@@ ") {
            hunks.push(vec![line]);
        } else if let Some(current) = hunks.last_mut() {
            current.push(line);
        } else {
            let partial_blocker = [
                &b"old mode"[..],
                b"new mode",
                b"similarity ",
                b"rename ",
                b"copy ",
                b"deleted file",
                b"new file",
                b"Binary files",
                b"GIT binary patch",
            ];
            if partial_blocker
                .iter()
                .any(|prefix| line.starts_with(prefix))
            {
                return None;
            }
            header.push(line);
        }
    }
    (!hunks.is_empty()).then_some(SplitDiff { header, hunks })
}

fn parse_range(field: &[u8]) -> Option<(u32, u32)> {
    let text = std::str::from_utf8(field).ok()?;
    let text = text.trim_start_matches(['-', '+']);
    match text.split_once(',') {
        Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

fn hunk_numbers(first_line: &[u8]) -> Option<(u32, u32, u32, u32)> {
    let mut parts = first_line.split(|byte| *byte == b' ');
    parts.next()?;
    let (old_start, old_len) = parse_range(parts.next()?)?;
    let (new_start, new_len) = parse_range(parts.next()?)?;
    Some((old_start, old_len, new_start, new_len))
}

fn single_hunk_patch(diff: &[u8], wanted: &HunkSpec) -> Option<Vec<u8>> {
    let split = split_diff(diff)?;
    let chosen = split.hunks.get(wanted.index)?;
    let numbers = hunk_numbers(chosen.first()?)?;
    if numbers
        != (
            wanted.old_start,
            wanted.old_len,
            wanted.new_start,
            wanted.new_len,
        )
    {
        return None;
    }
    let mut patch = Vec::new();
    for line in split.header.iter().chain(chosen.iter()) {
        patch.extend_from_slice(line);
    }
    Some(patch)
}
