use std::ffi::{OsStr, OsString};

use gitpoulpe_core::{Diagnostic, SessionInfo};

use crate::process::RunError;
use crate::stage::locked_index;
use crate::{Cancel, Git};

const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecError {
    NoWorktree,
    IndexLocked(String),
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWorktree => f.write_str("le dépôt n'a pas de répertoire de travail"),
            Self::IndexLocked(path) => write!(
                f,
                "un autre processus Git travaille sur ce dépôt ({path} existe) ; rien n'a été modifié"
            ),
            Self::GitUnavailable(d) => write!(f, "Git n'a pas pu démarrer : {d}"),
            Self::GitFailed(d) => write!(f, "Git a échoué : {d}"),
        }
    }
}

impl std::error::Error for ExecError {}

pub struct ExecOut {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl ExecOut {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).trim().to_owned()
    }
}

fn failure(error: RunError) -> ExecError {
    match error {
        RunError::Spawn(e) => ExecError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => ExecError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Cancelled => ExecError::GitFailed(Diagnostic {
            exit_code: None,
            signal: None,
            message: "cancelled".to_owned(),
            cause: None,
        }),
        RunError::Rejected(message) => ExecError::GitFailed(Diagnostic {
            exit_code: None,
            signal: None,
            message,
            cause: None,
        }),
    }
}

impl Git {
    pub(crate) async fn exec(
        &self,
        info: &SessionInfo,
        args: &[&str],
        env: &[(OsString, OsString)],
        input: Option<Vec<u8>>,
    ) -> Result<ExecOut, ExecError> {
        let worktree = info.worktree.as_deref().ok_or(ExecError::NoWorktree)?;
        let mut env = env.to_vec();
        if !env.iter().any(|(k, _)| k == "GIT_LITERAL_PATHSPECS") {
            env.push(("GIT_LITERAL_PATHSPECS".into(), "1".into()));
        }
        let env = &env[..];
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut out = Vec::new();
        let run = self
            .run_input(worktree, &refs, env, input, &Cancel::new(), |chunk| {
                if out.len() < MAX_OUTPUT_BYTES {
                    out.extend_from_slice(chunk);
                }
                Ok(())
            })
            .await
            .map_err(failure)?;
        let diagnostic = Diagnostic {
            cause: None,
            ..run.diagnostic()
        };
        if run.code != Some(0)
            && let Some(path) = locked_index(&diagnostic)
        {
            return Err(ExecError::IndexLocked(path));
        }
        Ok(ExecOut {
            code: run.code,
            stdout: out,
            stderr: diagnostic.message,
        })
    }

    pub(crate) async fn exec_ok(
        &self,
        info: &SessionInfo,
        args: &[&str],
    ) -> Result<ExecOut, ExecError> {
        let out = self.exec(info, args, &[], None).await?;
        if out.ok() {
            Ok(out)
        } else {
            Err(ExecError::GitFailed(Diagnostic {
                exit_code: out.code,
                signal: None,
                message: out.stderr,
                cause: None,
            }))
        }
    }
}
