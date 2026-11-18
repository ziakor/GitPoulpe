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
const MAX_RECORD: usize = 1024 * 1024;

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

#[derive(Clone, Debug)]
struct Query {
    text: String,
    hex: bool,
}

pub struct HistoryStream {
    query: Option<Query>,
    child: Child,
    stdout: ChildStdout,
    stderr: Option<tokio::task::JoinHandle<Vec<u8>>>,
    buffer: Vec<u8>,
    finished: bool,
    produced: usize,
    scan: usize,
    discarding: bool,
    truncated: bool,
}

fn parse_record(
    record: &[u8],
    with_body: bool,
    _truncated: bool,
) -> Result<(Commit, String), HistoryError> {
    let mut head = record.splitn(5, |b| *b == FIELD);
    let (Some(oid), Some(parents), Some(timestamp), Some(refs)) =
        (head.next(), head.next(), head.next(), head.next())
    else {
        return Err(HistoryError::Parse(
            "a commit record is missing its leading fields".to_owned(),
        ));
    };
    let rest = head.next().unwrap_or_default();
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let oid = text(oid);
    if oid.is_empty() || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(HistoryError::Parse(
            "a commit record has no valid id".to_owned(),
        ));
    }
    let timestamp = text(timestamp)
        .parse()
        .map_err(|_| HistoryError::Parse("a commit date is not a number".to_owned()))?;
    let mut tail = rest.splitn(3, |b| *b == FIELD);
    let author = text(tail.next().unwrap_or_default());
    let subject = text(tail.next().unwrap_or_default());
    let body = if with_body {
        text(tail.next().unwrap_or_default())
    } else {
        String::new()
    };
    let commit = Commit {
        oid,
        parents: text(parents)
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
        author,
        timestamp,
        subject,
        refs: text(refs)
            .split(", ")
            .filter(|r| !r.is_empty())
            .map(|r| r.strip_prefix("HEAD -> ").unwrap_or(r).to_owned())
            .collect(),
    };
    Ok((commit, body))
}

fn matches(query: &Query, commit: &Commit, body: &str) -> bool {
    let needle = &query.text;
    (query.hex && needle.len() >= 4 && commit.oid.to_lowercase().starts_with(needle.as_str()))
        || commit.author.to_lowercase().contains(needle.as_str())
        || commit.subject.to_lowercase().contains(needle.as_str())
        || body.to_lowercase().contains(needle.as_str())
}

impl Git {
    pub async fn history(&self, info: &SessionInfo) -> Result<HistoryStream, HistoryError> {
        self.open_log(info, None).await
    }

    pub async fn search(
        &self,
        info: &SessionInfo,
        text: &str,
    ) -> Result<HistoryStream, HistoryError> {
        let text = text.trim().to_lowercase();
        let hex = !text.is_empty() && text.chars().all(|c| c.is_ascii_hexdigit());
        self.open_log(info, Some(Query { text, hex })).await
    }

    async fn open_log(
        &self,
        info: &SessionInfo,
        query: Option<Query>,
    ) -> Result<HistoryStream, HistoryError> {
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
            if query.is_some() {
                "--format=%H%x1f%P%x1f%at%x1f%D%x1f%an%x1f%s%x1f%b"
            } else {
                "--format=%H%x1f%P%x1f%at%x1f%D%x1f%an%x1f%s"
            },
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
            query,
            child,
            stdout,
            stderr: Some(stderr),
            buffer: Vec::new(),
            finished: false,
            produced: 0,
            scan: 0,
            discarding: false,
            truncated: false,
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
            let found = self.buffer[self.scan..]
                .iter()
                .position(|b| *b == 0)
                .map(|at| at + self.scan);
            if let Some(end) = found {
                let record: Vec<u8> = self.buffer.drain(..=end).collect();
                self.scan = 0;
                let truncated = std::mem::take(&mut self.truncated);
                let (commit, body) =
                    parse_record(&record[..record.len() - 1], self.query.is_some(), truncated)?;
                if self
                    .query
                    .as_ref()
                    .is_none_or(|q| matches(q, &commit, &body))
                {
                    page.push(commit);
                }
                continue;
            }
            self.scan = self.buffer.len();
            if self.buffer.len() > MAX_RECORD && !self.discarding {
                self.buffer.truncate(MAX_RECORD);
                self.scan = MAX_RECORD;
                self.discarding = true;
                self.truncated = true;
            }
            if self.finished {
                break;
            }
            let mut chunk = vec![0u8; READ_CHUNK];
            match self.stdout.read(&mut chunk).await {
                Ok(0) => self.finished = true,
                Ok(n) if self.discarding => {
                    if let Some(end) = chunk[..n].iter().position(|b| *b == 0) {
                        self.buffer.push(0);
                        self.buffer.extend_from_slice(&chunk[end + 1..n]);
                        self.discarding = false;
                    }
                }
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
                if message.contains("does not have any commits yet")
                    && self.produced == 0
                    && self.query.is_none()
                {
                    return Err(HistoryError::NoCommits);
                }
                return Err(HistoryError::GitFailed(Diagnostic {
                    exit_code: status.and_then(|s| s.code()),
                    signal: None,
                    message,
                    cause: None,
                }));
            }
            if self.produced == 0 && self.query.is_none() {
                return Err(HistoryError::NoCommits);
            }
        }
        Ok(page)
    }
}
