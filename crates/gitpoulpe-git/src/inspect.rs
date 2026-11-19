use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use gitpoulpe_core::{
    ChangeEntry, Commit, CommitDetail, Diagnostic, EntryKind, SessionInfo, StatusCode,
};

use crate::process::RunError;
use crate::{Cancel, Git};

const FIELD: u8 = 0x1f;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InspectError {
    InvalidId,
    NotACommit,
    TooLarge,
    GitUnavailable(Diagnostic),
    GitFailed(Diagnostic),
    Parse(String),
}

impl std::fmt::Display for InspectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidId => f.write_str("the commit id is not valid"),
            Self::NotACommit => f.write_str("no such commit"),
            Self::TooLarge => f.write_str("the file is too large to attribute"),
            Self::GitUnavailable(d) => write!(f, "Git could not be started: {d}"),
            Self::GitFailed(d) => write!(f, "Git failed: {d}"),
            Self::Parse(m) => write!(f, "unexpected Git output: {m}"),
        }
    }
}

impl std::error::Error for InspectError {}

fn valid_id(id: &str) -> bool {
    (4..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn failure(error: RunError) -> InspectError {
    match error {
        RunError::Spawn(e) => InspectError::GitUnavailable(RunError::spawn_diagnostic(&e)),
        RunError::Io(e) => InspectError::GitFailed(RunError::spawn_diagnostic(&e)),
        RunError::Cancelled => InspectError::GitFailed(Diagnostic {
            exit_code: None,
            signal: None,
            message: "cancelled".to_owned(),
            cause: None,
        }),
        RunError::Rejected(message) => InspectError::Parse(message),
    }
}

fn status_code(letter: u8) -> Option<StatusCode> {
    Some(match letter {
        b'A' => StatusCode::Added,
        b'M' => StatusCode::Modified,
        b'D' => StatusCode::Deleted,
        b'T' => StatusCode::TypeChanged,
        b'R' => StatusCode::Renamed,
        b'C' => StatusCode::Copied,
        _ => return None,
    })
}

fn parse_name_status(output: &[u8]) -> Result<Vec<ChangeEntry>, InspectError> {
    let mut tokens = output.split(|b| *b == 0).filter(|t| !t.is_empty());
    let mut entries = Vec::new();
    while let Some(status) = tokens.next() {
        let code = status_code(status[0])
            .ok_or_else(|| InspectError::Parse("unknown change status".to_owned()))?;
        let path = |bytes: &[u8]| PathBuf::from(OsString::from_vec(bytes.to_vec()));
        let (path_new, original, kind) = match code {
            StatusCode::Renamed | StatusCode::Copied => {
                let old = tokens
                    .next()
                    .ok_or_else(|| InspectError::Parse("a rename has no source".to_owned()))?;
                let new = tokens
                    .next()
                    .ok_or_else(|| InspectError::Parse("a rename has no target".to_owned()))?;
                let kind = if code == StatusCode::Renamed {
                    EntryKind::Renamed
                } else {
                    EntryKind::Copied
                };
                (path(new), Some(path(old)), kind)
            }
            _ => {
                let only = tokens
                    .next()
                    .ok_or_else(|| InspectError::Parse("a change has no path".to_owned()))?;
                (path(only), None, EntryKind::Ordinary)
            }
        };
        entries.push(ChangeEntry {
            path: path_new,
            original_path: original,
            kind,
            index: code,
            worktree: StatusCode::Unmodified,
        });
    }
    Ok(entries)
}

impl Git {
    pub(crate) async fn collect_in(
        &self,
        info: &SessionInfo,
        args: &[&str],
    ) -> Result<(crate::process::Run, Vec<u8>), InspectError> {
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let refs: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
        self.run_collect(dir, &refs, &Cancel::new())
            .await
            .map_err(failure)
    }

    pub async fn empty_tree(&self, info: &SessionInfo) -> Result<String, InspectError> {
        let (run, out) = self
            .collect_in(info, &["hash-object", "-t", "tree", "--stdin"])
            .await?;
        let id = String::from_utf8_lossy(&out).trim().to_owned();
        if run.succeeded() && valid_id(&id) {
            Ok(id)
        } else {
            Err(InspectError::GitFailed(Diagnostic {
                cause: None,
                ..run.diagnostic()
            }))
        }
    }

    pub async fn commit_detail(
        &self,
        info: &SessionInfo,
        oid: &str,
    ) -> Result<CommitDetail, InspectError> {
        if !valid_id(oid) {
            return Err(InspectError::InvalidId);
        }
        let (run, out) = self
            .collect_in(
                info,
                &[
                    "show",
                    "-s",
                    "--no-show-signature",
                    "--format=%H%x1f%P%x1f%at%x1f%D%x1f%an%x1f%ae%x1f%cn%x1f%B",
                    oid,
                    "--",
                ],
            )
            .await?;
        if !run.succeeded() {
            let diagnostic = Diagnostic {
                cause: None,
                ..run.diagnostic()
            };
            return Err(
                if diagnostic.message.contains("bad object")
                    || diagnostic.message.contains("unknown revision")
                    || diagnostic.message.contains("ambiguous argument")
                {
                    InspectError::NotACommit
                } else {
                    InspectError::GitFailed(diagnostic)
                },
            );
        }
        let mut head = out.splitn(5, |b| *b == FIELD);
        let (Some(id), Some(parents), Some(time), Some(refs)) =
            (head.next(), head.next(), head.next(), head.next())
        else {
            return Err(InspectError::Parse("missing leading fields".to_owned()));
        };
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let rest = head.next().unwrap_or_default();
        let mut tail = rest.splitn(4, |b| *b == FIELD);
        let author = text(tail.next().unwrap_or_default());
        let author_email = text(tail.next().unwrap_or_default());
        let committer = text(tail.next().unwrap_or_default());
        let message = text(tail.next().unwrap_or_default());
        let message = message.trim_end().to_owned();
        let id = text(id);
        if !valid_id(&id) {
            return Err(InspectError::Parse("the commit id is not valid".to_owned()));
        }
        let subject = message.lines().next().unwrap_or_default().to_owned();
        Ok(CommitDetail {
            commit: Commit {
                oid: id,
                parents: text(parents)
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
                author,
                timestamp: text(time)
                    .trim()
                    .parse()
                    .map_err(|_| InspectError::Parse("the date is not a number".to_owned()))?,
                subject,
                refs: text(refs)
                    .split(", ")
                    .filter(|r| !r.is_empty())
                    .map(|r| r.strip_prefix("HEAD -> ").unwrap_or(r).to_owned())
                    .collect(),
            },
            author_email,
            committer,
            message,
        })
    }

    pub async fn commit_files(
        &self,
        info: &SessionInfo,
        oid: &str,
        base: &str,
    ) -> Result<Vec<ChangeEntry>, InspectError> {
        if !valid_id(oid) || !valid_id(base) {
            return Err(InspectError::InvalidId);
        }
        let (run, out) = self
            .collect_in(
                info,
                &[
                    "diff-tree",
                    "-r",
                    "-z",
                    "--name-status",
                    "-M",
                    base,
                    oid,
                    "--",
                ],
            )
            .await?;
        if !run.succeeded() {
            return Err(InspectError::GitFailed(Diagnostic {
                cause: None,
                ..run.diagnostic()
            }));
        }
        parse_name_status(&out)
    }
}
