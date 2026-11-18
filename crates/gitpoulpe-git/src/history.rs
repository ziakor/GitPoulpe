use std::ffi::{OsStr, OsString};

use gitpoulpe_core::{Commit, Diagnostic, SessionInfo};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdout};

use crate::Git;
use crate::diagnostics::filter_diagnostic;
use crate::process::RunError;

const READ_CHUNK: usize = 64 * 1024;
const MAX_STDERR: usize = 64 * 1024;
const FIELD: u8 = 0x1f;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryError {
    NoCommits,
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
    Parse(String),
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCommits => f.write_str("the repository has no commit yet"),
            Self::GitUnavailable(d) => write!(f, "Git could not be started: {d}"),
            Self::GitFailed(d) => write!(f, "Git failed: {d}"),
            Self::Parse(m) => write!(f, "unexpected Git output: {m}"),
        }
    }
}

impl std::error::Error for HistoryError {}

pub struct HistoryStream {
    child: Child,
    stdout: ChildStdout,
    stderr: Option<tokio::task::JoinHandle<Vec<u8>>>,
    buffer: Vec<u8>,
    finished: bool,
    produced: usize,
}

fn parse_record(record: &[u8]) -> Result<Commit, HistoryError> {
    let fields: Vec<&[u8]> = record.split(|b| *b == FIELD).collect();
    if fields.len() != 6 {
        return Err(HistoryError::Parse(format!(
            "a commit record has {} fields instead of 6",
            fields.len()
        )));
    }
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let oid = text(fields[0]);
    if oid.is_empty() {
        return Err(HistoryError::Parse("a commit record has no id".to_owned()));
    }
    let timestamp = text(fields[3])
        .parse()
        .map_err(|_| HistoryError::Parse("a commit date is not a number".to_owned()))?;
    Ok(Commit {
        oid,
        parents: text(fields[1])
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
        author: text(fields[2]),
        timestamp,
        subject: text(fields[4]),
        refs: text(fields[5])
            .split(", ")
            .filter(|r| !r.is_empty())
            .map(|r| r.strip_prefix("HEAD -> ").unwrap_or(r).to_owned())
            .collect(),
    })
}

impl Git {
    pub async fn history(&self, info: &SessionInfo) -> Result<HistoryStream, HistoryError> {
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let head_resolves = {
            let args: Vec<OsString> = ["rev-parse", "--verify", "-q", "HEAD"]
                .map(OsString::from)
                .to_vec();
            let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
            self.run_collect(dir, &refs, &crate::Cancel::new())
                .await
                .is_ok_and(|(run, _)| run.succeeded())
        };
        let mut args: Vec<OsString> = [
            "-c",
            "log.showSignature=false",
            "log",
            "--topo-order",
            "--parents",
            "-z",
            "--no-show-signature",
            "--format=%H%x1f%P%x1f%an%x1f%at%x1f%s%x1f%D",
            "--branches",
            "--tags",
            "--remotes",
        ]
        .map(OsString::from)
        .to_vec();
        if head_resolves {
            args.push(OsString::from("HEAD"));
        }
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        let mut command = self.command(dir, &refs, &[], false);
        let mut child = command
            .spawn()
            .map_err(|e| HistoryError::GitUnavailable(RunError::spawn_diagnostic(&e)))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let stderr = tokio::spawn(async move {
            let mut kept = Vec::new();
            let mut chunk = vec![0u8; 8 * 1024];
            while let Ok(n) = stderr.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                let room = MAX_STDERR.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..n.min(room)]);
            }
            kept
        });
        Ok(HistoryStream {
            child,
            stdout,
            stderr: Some(stderr),
            buffer: Vec::new(),
            finished: false,
            produced: 0,
        })
    }
}

impl HistoryStream {
    pub fn is_exhausted(&self) -> bool {
        self.finished && self.buffer.is_empty()
    }

    pub async fn next_page(&mut self, limit: usize) -> Result<Vec<Commit>, HistoryError> {
        let mut page = Vec::new();
        while page.len() < limit {
            if let Some(end) = self.buffer.iter().position(|b| *b == 0) {
                let record: Vec<u8> = self.buffer.drain(..=end).collect();
                page.push(parse_record(&record[..record.len() - 1])?);
                continue;
            }
            if self.finished {
                break;
            }
            let mut chunk = vec![0u8; READ_CHUNK];
            match self.stdout.read(&mut chunk).await {
                Ok(0) => self.finished = true,
                Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
                Err(error) => {
                    return Err(HistoryError::GitFailed(RunError::spawn_diagnostic(&error)));
                }
            }
        }
        self.produced += page.len();
        if self.finished && self.buffer.is_empty() {
            let status = self.child.wait().await.ok();
            let stderr = match self.stderr.take() {
                Some(task) => task.await.unwrap_or_default(),
                None => Vec::new(),
            };
            let failed = status.is_some_and(|s| !s.success());
            if failed {
                let message = filter_diagnostic(&stderr);
                if message.contains("does not have any commits yet") && self.produced == 0 {
                    return Err(HistoryError::NoCommits);
                }
                return Err(HistoryError::GitFailed(Diagnostic {
                    exit_code: status.and_then(|s| s.code()),
                    signal: None,
                    message,
                    cause: None,
                }));
            }
            if self.produced == 0 {
                return Err(HistoryError::NoCommits);
            }
        }
        Ok(page)
    }
}
