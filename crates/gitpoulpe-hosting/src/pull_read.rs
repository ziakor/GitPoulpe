use gitpoulpe_core::Connection;
use serde_json::Value;

use crate::github::{GithubError, interpret_failure};
use crate::pulls::ChangedFile;
use crate::repos::{client, pages, valid_org};
use crate::secrets::SecretStore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullFilter {
    Open,
    Closed,
    All,
}

impl PullFilter {
    fn query(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullState {
    Open,
    Closed,
    Merged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullSummary {
    pub number: u64,
    pub title: String,
    pub state: PullState,
    pub draft: bool,
    pub author: String,
    pub head_ref: String,
    pub head_sha: String,
    pub base_ref: String,
    pub base_sha: String,
    pub url: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullCommit {
    pub sha: String,
    pub subject: String,
    pub author: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullFile {
    pub change: ChangedFile,
    pub patch: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullDetail {
    pub summary: PullSummary,
    pub body: String,
    pub commits: Vec<PullCommit>,
    pub files: Vec<PullFile>,
    pub files_truncated: bool,
    pub changed_during_load: bool,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn parse_summary(value: &Value) -> Option<PullSummary> {
    let state = match (
        text(value, "state")?.as_str(),
        value.get("merged_at").is_some_and(|m| !m.is_null())
            || value.get("merged").and_then(Value::as_bool) == Some(true),
    ) {
        (_, true) => PullState::Merged,
        ("open", false) => PullState::Open,
        _ => PullState::Closed,
    };
    let head = value.get("head")?;
    let base = value.get("base")?;
    Some(PullSummary {
        number: value.get("number")?.as_u64()?,
        title: text(value, "title").unwrap_or_default(),
        state,
        draft: value.get("draft").and_then(Value::as_bool).unwrap_or(false),
        author: value
            .get("user")
            .and_then(|u| text(u, "login"))
            .unwrap_or_default(),
        head_ref: text(head, "ref")?,
        head_sha: text(head, "sha")?,
        base_ref: text(base, "ref")?,
        base_sha: text(base, "sha")?,
        url: text(value, "html_url").unwrap_or_default(),
        updated_at: text(value, "updated_at").unwrap_or_default(),
    })
}

pub async fn list_pulls(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    filter: PullFilter,
) -> Result<(Vec<PullSummary>, bool), GithubError> {
    if !valid_org(owner) || !valid_org(name) {
        return Err(GithubError::NotFound);
    }
    let client = client(store, connection)?;
    let raw = pages(
        &client,
        format!(
            "repos/{owner}/{name}/pulls?state={}&sort=updated&direction=desc&per_page=100",
            filter.query()
        ),
    )
    .await?;
    Ok((
        raw.items.iter().filter_map(parse_summary).collect(),
        raw.truncated,
    ))
}

pub async fn load_pull(
    store: &dyn SecretStore,
    connection: &Connection,
    owner: &str,
    name: &str,
    number: u64,
) -> Result<PullDetail, GithubError> {
    if !valid_org(owner) || !valid_org(name) {
        return Err(GithubError::NotFound);
    }
    let client = client(store, connection)?;
    let base = format!("repos/{owner}/{name}/pulls/{number}");
    let response = client.get(&base).await?;
    if !response.ok() {
        return Err(interpret_failure(&response));
    }
    let value = response.json().map_err(|_| GithubError::Unreadable)?;
    let summary = parse_summary(&value).ok_or(GithubError::Unreadable)?;
    let commits_raw = pages(&client, format!("{base}/commits?per_page=100")).await?;
    let commits: Vec<PullCommit> = commits_raw
        .items
        .iter()
        .filter_map(|c| {
            Some(PullCommit {
                sha: text(c, "sha")?,
                subject: c
                    .get("commit")
                    .and_then(|m| text(m, "message"))
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_owned(),
                author: c
                    .get("author")
                    .and_then(|a| text(a, "login"))
                    .or_else(|| {
                        c.get("commit")
                            .and_then(|m| m.get("author"))
                            .and_then(|a| text(a, "name"))
                    })
                    .unwrap_or_default(),
            })
        })
        .collect();
    let files_raw = pages(&client, format!("{base}/files?per_page=100")).await?;
    let files: Vec<PullFile> = files_raw
        .items
        .iter()
        .filter_map(|f| {
            Some(PullFile {
                change: ChangedFile {
                    path: text(f, "filename")?,
                    status: text(f, "status")?,
                    additions: f.get("additions").and_then(Value::as_u64).unwrap_or(0),
                    deletions: f.get("deletions").and_then(Value::as_u64).unwrap_or(0),
                },
                patch: text(f, "patch"),
            })
        })
        .collect();
    let changed_during_load = !commits_raw.truncated
        && commits
            .last()
            .is_some_and(|last| last.sha != summary.head_sha);
    Ok(PullDetail {
        body: text(&value, "body").unwrap_or_default(),
        files_truncated: files_raw.truncated,
        summary,
        commits,
        files,
        changed_during_load,
    })
}
