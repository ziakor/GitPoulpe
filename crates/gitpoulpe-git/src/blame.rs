use std::collections::HashMap;

use gitpoulpe_core::{Diagnostic, SessionInfo};

use crate::Git;
use crate::inspect::InspectError;

const SIZE_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub line: u64,
    pub oid: Option<String>,
    pub author: String,
    pub timestamp: i64,
    pub summary: String,
    pub path: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blame {
    pub path: String,
    pub revision: Option<String>,
    pub lines: Vec<BlameLine>,
}

#[derive(Clone, Default)]
struct Meta {
    author: String,
    timestamp: i64,
    summary: String,
    path: String,
}

fn valid_id(id: &str) -> bool {
    (4..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit())
}

fn valid_path(path: &str) -> bool {
    !path.is_empty() && !path.contains('\0') && !path.starts_with('/') && path.len() <= 4096
}

fn header(line: &[u8]) -> Option<(String, u64)> {
    let text = std::str::from_utf8(line).ok()?;
    let mut parts = text.split(' ');
    let oid = parts.next()?;
    if !valid_id(oid) || oid.len() < 40 {
        return None;
    }
    let _original: u64 = parts.next()?.parse().ok()?;
    let line_no: u64 = parts.next()?.parse().ok()?;
    Some((oid.to_owned(), line_no))
}

fn parse(out: &[u8]) -> Result<Vec<BlameLine>, InspectError> {
    let mut metas: HashMap<String, Meta> = HashMap::new();
    let mut lines = Vec::new();
    let mut current: Option<(String, u64)> = None;
    for raw in out.split(|b| *b == b'\n') {
        if let Some(content) = raw.strip_prefix(b"\t") {
            let Some((oid, line)) = current.take() else {
                return Err(InspectError::Parse("content without a header".to_owned()));
            };
            let meta = metas.get(&oid).cloned().unwrap_or_default();
            let zero = oid.chars().all(|c| c == '0');
            lines.push(BlameLine {
                line,
                oid: (!zero).then_some(oid),
                author: meta.author,
                timestamp: meta.timestamp,
                summary: meta.summary,
                path: meta.path,
                text: String::from_utf8_lossy(content).into_owned(),
            });
            continue;
        }
        if current.is_none() {
            if let Some(found) = header(raw) {
                metas.entry(found.0.clone()).or_default();
                current = Some(found);
            }
            continue;
        }
        let Some((oid, _)) = &current else { continue };
        let text = String::from_utf8_lossy(raw);
        let meta = metas.entry(oid.clone()).or_default();
        if let Some(value) = text.strip_prefix("author ") {
            meta.author = value.to_owned();
        } else if let Some(value) = text.strip_prefix("author-time ") {
            meta.timestamp = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = text.strip_prefix("summary ") {
            meta.summary = value.to_owned();
        } else if let Some(value) = text.strip_prefix("filename ") {
            meta.path = value.to_owned();
        }
    }
    Ok(lines)
}

impl Git {
    pub async fn blame(
        &self,
        info: &SessionInfo,
        revision: Option<&str>,
        path: &str,
    ) -> Result<Blame, InspectError> {
        if !valid_path(path) || revision.is_some_and(|r| !valid_id(r)) {
            return Err(InspectError::InvalidId);
        }
        let size = match revision {
            Some(rev) => {
                let spec = format!("{rev}:{path}");
                let (run, out) = self.collect_in(info, &["cat-file", "-s", &spec]).await?;
                if !run.succeeded() {
                    return Err(InspectError::NotACommit);
                }
                String::from_utf8_lossy(&out)
                    .trim()
                    .parse()
                    .unwrap_or(u64::MAX)
            }
            None => info
                .worktree
                .as_deref()
                .and_then(|dir| std::fs::metadata(dir.join(path)).ok())
                .map_or(0, |m| m.len()),
        };
        if size > SIZE_LIMIT {
            return Err(InspectError::TooLarge);
        }
        let mut args = vec!["blame", "--porcelain"];
        if let Some(rev) = revision {
            args.push(rev);
        }
        args.push("--");
        args.push(path);
        let (run, out) = self.collect_in(info, &args).await?;
        if !run.succeeded() {
            return Err(InspectError::GitFailed(Diagnostic {
                cause: None,
                ..run.diagnostic()
            }));
        }
        Ok(Blame {
            path: path.to_owned(),
            revision: revision.map(str::to_owned),
            lines: parse(&out)?,
        })
    }
}
