use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;

use gitpoulpe_core::Diagnostic;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use crate::diagnostics::filter_diagnostic;
use crate::{Cancel, Git};

const CHUNK_BYTES: usize = 64 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_COLLECTED_BYTES: usize = 1024 * 1024;

const SCRUBBED_ENV: [&str; 7] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_EXTERNAL_DIFF",
    "GIT_PAGER",
    "GIT_DIFF_OPTS",
];

pub(crate) struct Run {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub stderr: Vec<u8>,
}

impl Run {
    pub fn succeeded(&self) -> bool {
        self.code == Some(0)
    }

    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            exit_code: self.code,
            signal: self.signal,
            message: filter_diagnostic(&self.stderr),
            cause: None,
        }
    }
}

pub(crate) enum RunError {
    Spawn(io::Error),
    Io(io::Error),
    Cancelled,
    Rejected(String),
}

impl RunError {
    pub fn spawn_diagnostic(error: &io::Error) -> Diagnostic {
        Diagnostic {
            exit_code: None,
            signal: None,
            message: filter_diagnostic(error.to_string().as_bytes()),
            cause: Some(format!(
                "{:?} (os error {:?})",
                error.kind(),
                error.raw_os_error()
            )),
        }
    }
}

const SPAWN_ATTEMPTS: u32 = 20;
const SPAWN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(25);

async fn spawn_with_retry(
    command: &mut Command,
    cancel: &Cancel,
) -> Result<tokio::process::Child, RunError> {
    let mut attempt = 1;
    loop {
        match command.spawn() {
            Ok(child) => return Ok(child),
            Err(error)
                if error.kind() == io::ErrorKind::ExecutableFileBusy
                    && attempt < SPAWN_ATTEMPTS =>
            {
                attempt += 1;
                tokio::select! {
                    () = tokio::time::sleep(SPAWN_RETRY_DELAY) => {}
                    () = cancel.cancelled() => return Err(RunError::Cancelled),
                }
            }
            Err(error) => return Err(RunError::Spawn(error)),
        }
    }
}

async fn drain_stderr(mut stream: impl AsyncRead + Unpin) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buf = vec![0u8; 8 * 1024];
    while let Ok(n) = stream.read(&mut buf).await {
        if n == 0 {
            break;
        }
        let room = MAX_STDERR_BYTES.saturating_sub(kept.len());
        kept.extend_from_slice(&buf[..n.min(room)]);
    }
    kept
}

pub(crate) type EnvPairs = [(OsString, OsString)];

impl Git {
    pub(crate) async fn run<F>(
        &self,
        dir: &Path,
        args: &[&OsStr],
        cancel: &Cancel,
        on_chunk: F,
    ) -> Result<Run, RunError>
    where
        F: FnMut(&[u8]) -> Result<(), String>,
    {
        self.run_env(dir, args, &[], cancel, on_chunk).await
    }

    pub(crate) async fn run_env<F>(
        &self,
        dir: &Path,
        args: &[&OsStr],
        env: &EnvPairs,
        cancel: &Cancel,
        mut on_chunk: F,
    ) -> Result<Run, RunError>
    where
        F: FnMut(&[u8]) -> Result<(), String>,
    {
        if cancel.is_cancelled() {
            return Err(RunError::Cancelled);
        }
        let mut command = Command::new(&self.program);
        command
            .current_dir(dir)
            .arg("--no-optional-locks")
            .arg("--no-lazy-fetch")
            .args(["-c", "core.fsmonitor=false"])
            .args(["-c", "protocol.allow=never"])
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for name in SCRUBBED_ENV {
            command.env_remove(name);
        }
        command.envs(env.iter().map(|(k, v)| (k, v)));
        let mut child = spawn_with_retry(&mut command, cancel).await?;
        let mut stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let stderr_task = tokio::spawn(drain_stderr(stderr));

        let mut buf = vec![0u8; CHUNK_BYTES];
        let outcome: Result<(), RunError> = loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => break Err(RunError::Cancelled),
                read = stdout.read(&mut buf) => match read {
                    Ok(0) => break Ok(()),
                    Ok(n) => {
                        if let Err(reason) = on_chunk(&buf[..n]) {
                            break Err(RunError::Rejected(reason));
                        }
                    }
                    Err(error) => break Err(RunError::Io(error)),
                },
            }
        };
        if outcome.is_err() {
            let _ = child.start_kill();
        }
        let status = child.wait().await;
        if outcome.is_err() {
            stderr_task.abort();
        }
        outcome?;
        let status = status.map_err(RunError::Io)?;
        let stderr = stderr_task.await.unwrap_or_default();
        Ok(Run {
            code: status.code(),
            signal: status.signal(),
            stderr,
        })
    }

    pub(crate) async fn run_collect(
        &self,
        dir: &Path,
        args: &[&OsStr],
        cancel: &Cancel,
    ) -> Result<(Run, Vec<u8>), RunError> {
        let mut out = Vec::new();
        let run = self
            .run(dir, args, cancel, |chunk| {
                if out.len() + chunk.len() > MAX_COLLECTED_BYTES {
                    return Err("output exceeds the collection limit".to_owned());
                }
                out.extend_from_slice(chunk);
                Ok(())
            })
            .await?;
        Ok((run, out))
    }
}
