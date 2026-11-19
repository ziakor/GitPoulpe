use std::collections::HashSet;

use gitpoulpe_core::{Diagnostic, SessionInfo};

use crate::Git;
use crate::inspect::InspectError;

const RECORD: u8 = 0x1e;
const FIELD: u8 = 0x1f;
const CONTENT_LIMIT: u64 = 1024 * 1024;
const DIFF_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChange {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRevision {
    pub oid: String,
    pub timestamp: i64,
    pub author: String,
    pub subject: String,
    pub change: FileChange,
    pub path: String,
    pub old_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHistory {
    pub revisions: Vec<FileRevision>,
    pub incomplete: bool,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileRevisionView {
    pub oid: String,
    pub path: String,
    pub deleted: bool,
    pub binary: bool,
    pub content: Option<String>,
    pub content_too_large: bool,
    pub diff: String,
    pub diff_truncated: bool,
}

fn valid_id(id: &str) -> bool {
    (4..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn valid_path(path: &str) -> bool {
    !path.is_empty() && !path.contains('\0') && !path.starts_with('/') && path.len() <= 4096
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn change_of(code: &[u8]) -> Option<FileChange> {
    Some(match code.first()? {
        b'A' => FileChange::Added,
        b'M' => FileChange::Modified,
        b'D' => FileChange::Deleted,
        b'R' => FileChange::Renamed,
        b'C' => FileChange::Copied,
        b'T' => FileChange::TypeChanged,
        _ => return None,
    })
}

fn parse_log(out: &[u8]) -> Result<Vec<FileRevision>, InspectError> {
    let mut revisions = Vec::new();
    for record in out.split(|b| *b == RECORD).filter(|r| !r.is_empty()) {
        let mut tokens = record.split(|b| *b == 0);
        let header = tokens.next().unwrap_or_default();
        let mut fields = header.splitn(4, |b| *b == FIELD);
        let (Some(oid), Some(time), Some(author), Some(subject)) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(InspectError::Parse("missing history fields".to_owned()));
        };
        let code = tokens.next().unwrap_or_default();
        let code = code.strip_prefix(b"\n").unwrap_or(code);
        let Some(change) = change_of(code) else {
            continue;
        };
        let first = tokens.next().unwrap_or_default();
        let (old_path, path) = if matches!(change, FileChange::Renamed | FileChange::Copied) {
            (Some(text(first)), text(tokens.next().unwrap_or_default()))
        } else {
            (None, text(first))
        };
        revisions.push(FileRevision {
            oid: text(oid),
            timestamp: text(time)
                .trim()
                .parse()
                .map_err(|_| InspectError::Parse("the date is not a number".to_owned()))?,
            author: text(author),
            subject: text(subject),
            change,
            path,
            old_path,
        });
    }
    Ok(revisions)
}

impl Git {
    async fn shallow_boundaries(&self, info: &SessionInfo) -> HashSet<String> {
        let Ok((run, out)) = self
            .collect_in(info, &["rev-parse", "--is-shallow-repository"])
            .await
        else {
            return HashSet::new();
        };
        if !run.succeeded() || text(&out).trim() != "true" {
            return HashSet::new();
        }
        let Ok((run, out)) = self
            .collect_in(info, &["rev-parse", "--git-path", "shallow"])
            .await
        else {
            return HashSet::new();
        };
        if !run.succeeded() {
            return HashSet::new();
        }
        let relative = text(&out).trim().to_owned();
        let dir = info.worktree.as_deref().unwrap_or(&info.git_dir);
        std::fs::read_to_string(dir.join(relative))
            .map(|content| content.lines().map(str::to_owned).collect())
            .unwrap_or_else(|_| HashSet::from([String::new()]))
    }

    pub async fn file_history(
        &self,
        info: &SessionInfo,
        path: &str,
        limit: usize,
    ) -> Result<FileHistory, InspectError> {
        if !valid_path(path) || limit == 0 {
            return Err(InspectError::InvalidId);
        }
        let count = format!("-n{}", limit + 1);
        let (run, out) = self
            .collect_in(
                info,
                &[
                    "log",
                    "--follow",
                    "-z",
                    "--name-status",
                    "-M",
                    "--no-show-signature",
                    &count,
                    "--format=%x1e%H%x1f%at%x1f%an%x1f%s",
                    "--",
                    path,
                ],
            )
            .await?;
        if !run.succeeded() {
            return Err(InspectError::GitFailed(Diagnostic {
                cause: None,
                ..run.diagnostic()
            }));
        }
        let mut revisions = parse_log(&out)?;
        let truncated = revisions.len() > limit;
        revisions.truncate(limit);
        let boundaries = self.shallow_boundaries(info).await;
        let incomplete = !boundaries.is_empty()
            && (revisions.is_empty()
                || revisions
                    .last()
                    .is_some_and(|r| boundaries.contains(&r.oid) || boundaries.contains("")));
        Ok(FileHistory {
            revisions,
            incomplete,
            truncated,
        })
    }

    pub async fn file_revision(
        &self,
        info: &SessionInfo,
        oid: &str,
        path: &str,
        old_path: Option<&str>,
    ) -> Result<FileRevisionView, InspectError> {
        if !valid_id(oid) || !valid_path(path) || old_path.is_some_and(|p| !valid_path(p)) {
            return Err(InspectError::InvalidId);
        }
        let spec = format!("{oid}:{path}");
        let (size_run, size_out) = self.collect_in(info, &["cat-file", "-s", &spec]).await?;
        let (deleted, mut content, mut too_large, mut binary) =
            (!size_run.succeeded(), None, false, false);
        if size_run.succeeded() {
            let size: u64 = text(&size_out).trim().parse().unwrap_or(u64::MAX);
            if size > CONTENT_LIMIT {
                too_large = true;
            } else {
                let (run, bytes) = self.collect_in(info, &["cat-file", "blob", &spec]).await?;
                if !run.succeeded() {
                    return Err(InspectError::GitFailed(Diagnostic {
                        cause: None,
                        ..run.diagnostic()
                    }));
                }
                binary = bytes.iter().take(8000).any(|b| *b == 0);
                if !binary {
                    content = Some(text(&bytes));
                }
            }
        }
        let mut args = vec![
            "show",
            "--format=",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "-M",
            "-U3",
            oid,
            "--",
        ];
        if let Some(old) = old_path {
            args.push(old);
        }
        args.push(path);
        let (diff, diff_truncated) = match self.collect_in(info, &args).await {
            Ok((run, out)) if run.succeeded() => {
                let cut = out.len() > DIFF_LIMIT;
                (text(if cut { &out[..DIFF_LIMIT] } else { &out[..] }), cut)
            }
            Ok((run, _)) => {
                return Err(InspectError::GitFailed(Diagnostic {
                    cause: None,
                    ..run.diagnostic()
                }));
            }
            Err(InspectError::Parse(message)) if message.contains("collection limit") => {
                (String::new(), true)
            }
            Err(other) => return Err(other),
        };
        Ok(FileRevisionView {
            oid: oid.to_owned(),
            path: path.to_owned(),
            deleted,
            binary,
            content,
            content_too_large: too_large,
            diff,
            diff_truncated,
        })
    }
}
