use std::ffi::OsString;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use gitpoulpe_core::{Diagnostic, OpenError, RepoId, RepoKind, SessionId, SessionInfo};

use crate::process::{Run, RunError};
use crate::{Cancel, Git};

fn open_failure(error: RunError) -> OpenError {
    match error {
        RunError::Cancelled => OpenError::Cancelled,
        RunError::Spawn(e) => OpenError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => OpenError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Rejected(message) => OpenError::GitFailed(Diagnostic {
            exit_code: None,
            signal: None,
            message,
            cause: None,
        }),
    }
}

fn classify(run: &Run) -> OpenError {
    let message = String::from_utf8_lossy(&run.stderr);
    if message.contains("not a git repository") {
        OpenError::NotARepository
    } else if message.contains("dubious ownership") || message.contains("Permission denied") {
        OpenError::PermissionDenied
    } else {
        OpenError::GitFailed(run.diagnostic())
    }
}

fn single_line(mut output: Vec<u8>) -> Vec<u8> {
    if output.last() == Some(&b'\n') {
        output.pop();
    }
    output
}

impl Git {
    pub async fn open(&self, path: &Path, cancel: &Cancel) -> Result<SessionInfo, OpenError> {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(OpenError::NotADirectory),
            Err(e) if e.kind() == ErrorKind::NotFound => return Err(OpenError::NotFound),
            Err(e) if e.kind() == ErrorKind::PermissionDenied => {
                return Err(OpenError::PermissionDenied);
            }
            Err(e) => {
                return Err(OpenError::GitFailed(RunError::spawn_diagnostic(&e)));
            }
        }
        let git_dir = self.rev_parse_value(path, "--git-dir", cancel).await?;
        let common_dir = self
            .rev_parse_value(path, "--git-common-dir", cancel)
            .await?;
        let bare = self
            .rev_parse_value(path, "--is-bare-repository", cancel)
            .await?
            == Path::new("true");
        let worktree = if bare {
            None
        } else {
            Some(
                self.rev_parse_value(path, "--show-toplevel", cancel)
                    .await?,
            )
        };
        let has_commits = self.head_resolves(path, cancel).await?;
        let kind = if bare {
            RepoKind::Bare
        } else if std::fs::canonicalize(&git_dir).ok() != std::fs::canonicalize(&common_dir).ok() {
            RepoKind::LinkedWorktree
        } else {
            RepoKind::Ordinary
        };
        Ok(SessionInfo {
            id: SessionId::next(),
            repo: RepoId::from_common_dir(&common_dir),
            kind,
            worktree,
            git_dir,
            common_dir,
            has_commits,
        })
    }

    async fn rev_parse_value(
        &self,
        dir: &Path,
        option: &str,
        cancel: &Cancel,
    ) -> Result<PathBuf, OpenError> {
        let args = ["rev-parse", "--path-format=absolute", option].map(OsString::from);
        let refs: Vec<&std::ffi::OsStr> = args.iter().map(OsString::as_os_str).collect();
        let (run, output) = self
            .run_collect(dir, &refs, cancel)
            .await
            .map_err(open_failure)?;
        if !run.succeeded() {
            return Err(classify(&run));
        }
        Ok(PathBuf::from(OsString::from_vec(single_line(output))))
    }

    async fn head_resolves(&self, dir: &Path, cancel: &Cancel) -> Result<bool, OpenError> {
        let args = ["rev-parse", "--verify", "-q", "HEAD"].map(OsString::from);
        let refs: Vec<&std::ffi::OsStr> = args.iter().map(OsString::as_os_str).collect();
        let (run, _) = self
            .run_collect(dir, &refs, cancel)
            .await
            .map_err(open_failure)?;
        match run.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(classify(&run)),
        }
    }
}
